use agentic_gpt_protocol::{
    NotificationChannel, UserNotifyDeliveryRequest, UserNotifyDeliveryResponse,
};

use crate::Config;

pub(crate) fn freedesktop_notification_channel(config: &Config) -> Option<NotificationChannel> {
    let (available, _) = detect_freedesktop_notification_support();
    if !available {
        return None;
    }
    Some(NotificationChannel {
        key: format!("agent::{}::freedesktop", config.agent_id),
        display_name: format!("{} desktop notification", config.display_name),
        available: true,
        kind: "freedesktop".to_string(),
        supports_actions: false,
        reason: None,
        agent_id: Some(config.agent_id.clone()),
    })
}

pub(crate) fn detect_freedesktop_notification_support() -> (bool, bool) {
    notify_rust::get_capabilities()
        .map(|capabilities| {
            let supports_actions = capabilities
                .iter()
                .any(|capability| capability == "actions");
            (true, supports_actions)
        })
        .unwrap_or((false, false))
}

pub(crate) async fn deliver_freedesktop_notification(
    payload: UserNotifyDeliveryRequest,
) -> UserNotifyDeliveryResponse {
    if !matches!(
        payload.channel_key.split("::").collect::<Vec<_>>().as_slice(),
        ["agent", alias, "freedesktop"] if !alias.is_empty()
    ) {
        return UserNotifyDeliveryResponse {
            channel_key: payload.channel_key,
            delivered: false,
            reason: Some("unsupported_channel".to_string()),
        };
    }
    let channel_key = payload.channel_key.clone();
    let delivered = tokio::task::spawn_blocking(move || {
        notify_rust::Notification::new()
            .summary(&payload.title)
            .body(&payload.body)
            .show()
    })
    .await;
    match delivered {
        Ok(Ok(_)) => UserNotifyDeliveryResponse {
            channel_key,
            delivered: true,
            reason: None,
        },
        Ok(Err(error)) => UserNotifyDeliveryResponse {
            channel_key,
            delivered: false,
            reason: Some(format!("notification_show_failed:{error}")),
        },
        Err(error) => UserNotifyDeliveryResponse {
            channel_key,
            delivered: false,
            reason: Some(format!("notification_provider_unavailable:{error}")),
        },
    }
}
