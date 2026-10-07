use agentic_gpt_protocol::{
    NotificationChannel, UserNotifyDeliveryRequest, UserNotifyDeliveryResponse,
};

use crate::config::Config;
use std::time::Duration;

const NOTIFICATION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

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
    zbus::blocking::connection::Builder::session()
        .and_then(probe_freedesktop_notification_support)
        .unwrap_or((false, false))
}

fn probe_freedesktop_notification_support(
    builder: zbus::blocking::connection::Builder<'_>,
) -> zbus::Result<(bool, bool)> {
    let connection = builder.method_timeout(NOTIFICATION_PROBE_TIMEOUT).build()?;
    // GetNameOwner does not activate a desktop service. Address the unique owner
    // so a disappearing service cannot trigger activation during GetCapabilities.
    let owner: String = connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &("org.freedesktop.Notifications",),
        )?
        .body()
        .deserialize()?;
    let capabilities: Vec<String> = connection
        .call_method(
            Some(owner.as_str()),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "GetCapabilities",
            &(),
        )?
        .body()
        .deserialize()?;
    Ok((
        true,
        capabilities
            .iter()
            .any(|capability| capability == "actions"),
    ))
}

pub(crate) async fn freedesktop_supports_actions() -> bool {
    tokio::task::spawn_blocking(detect_freedesktop_notification_support)
        .await
        .map(|(_, supports_actions)| supports_actions)
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    struct TestBus {
        daemon: Child,
        address: String,
    }

    impl TestBus {
        fn start() -> Self {
            let mut daemon = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("start isolated test D-Bus");
            let mut address = String::new();
            BufReader::new(daemon.stdout.take().unwrap())
                .read_line(&mut address)
                .unwrap();
            assert!(!address.trim().is_empty());
            Self {
                daemon,
                address: address.trim().to_string(),
            }
        }

        async fn probe(&self) -> (bool, bool) {
            let address = self.address.clone();
            tokio::task::spawn_blocking(move || {
                zbus::blocking::connection::Builder::address(address.as_str())
                    .and_then(probe_freedesktop_notification_support)
                    .unwrap_or((false, false))
            })
            .await
            .unwrap()
        }
    }

    impl Drop for TestBus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    struct Notifications {
        actions: bool,
        stall: bool,
    }

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Notifications {
        async fn get_capabilities(&self) -> Vec<String> {
            if self.stall {
                std::future::pending::<()>().await;
            }
            if self.actions {
                vec!["actions".to_string()]
            } else {
                vec!["body".to_string()]
            }
        }
    }

    async fn serve(bus: &TestBus, actions: bool, stall: bool) -> zbus::Connection {
        zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .name("org.freedesktop.Notifications")
            .unwrap()
            .serve_at(
                "/org/freedesktop/Notifications",
                Notifications { actions, stall },
            )
            .unwrap()
            .build()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn freedesktop_probe_missing_service_returns_unavailable() {
        let bus = TestBus::start();
        assert_eq!(bus.probe().await, (false, false));
    }

    #[tokio::test]
    async fn freedesktop_probe_reports_notification_actions() {
        let bus = TestBus::start();
        let service = serve(&bus, true, false).await;
        assert_eq!(bus.probe().await, (true, true));
        drop(service);
        let _service = serve(&bus, false, false).await;
        assert_eq!(bus.probe().await, (true, false));
    }

    #[tokio::test]
    async fn freedesktop_probe_unresponsive_service_has_short_deadline() {
        let bus = TestBus::start();
        let _service = serve(&bus, true, true).await;
        let started = Instant::now();
        let support = tokio::time::timeout(Duration::from_secs(5), bus.probe())
            .await
            .expect("unresponsive notification service must not delay fallback");
        assert_eq!(support, (false, false));
        assert!(started.elapsed() >= NOTIFICATION_PROBE_TIMEOUT);
    }
}
