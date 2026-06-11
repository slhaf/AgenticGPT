use agentic_gpt_protocol::{
    HubCommand, NotificationChannel, UserNotifyDeliveryRequest, UserNotifySendRequest,
    UserNotifySendResponse,
};
use anyhow::Result;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::time::{timeout, Duration};

// Temporary dependencies until agents/routes/utils modules take ownership.
use crate::agents::request_agent;
use crate::registry::registry_entries;
use crate::routes::{api_error, require_action_auth};
use crate::state::HubState;
use crate::utils::{random_id, random_token, sha256_hex};
use crate::{NtfyConfig, REQUEST_TIMEOUT_SECS};

#[derive(Clone, Debug)]
pub(crate) struct NtfyHealthCache {
    pub(crate) server_url: String,
    pub(crate) checked_at: DateTime<Utc>,
    pub(crate) result: NtfyHealthStatus,
}

#[derive(Clone, Debug)]
pub(crate) enum NtfyHealthStatus {
    Healthy,
    Unhealthy,
    Failed,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserNotifySendBody {
    channel: String,
    title: String,
    body: String,
    #[serde(default)]
    actions: Vec<agentic_gpt_protocol::NotificationAction>,
    #[serde(default)]
    priority: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AndroidRegisterRequest {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    capabilities: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NotifyChannelKey {
    AgentFreedesktop { alias: String },
    HubNtfy,
    AndroidNotice,
    AndroidAlarm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AndroidEndpointState {
    NotConnected,
    DeliveryNotImplemented,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NotifyRouteError {
    InvalidChannel(String),
    AgentNotFound(String),
    ChannelUnavailable {
        channel_key: String,
        reason: &'static str,
    },
    DeliveryFailed {
        channel_key: String,
        reason: String,
    },
    Db(String),
}

pub(crate) async fn notify_channels(State(state): State<HubState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    match notification_channels(&state).await {
        Ok(channels) => Json(json!({ "channels": channels })).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    }
}

pub(crate) async fn notify_send(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<UserNotifySendBody>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    let request = UserNotifySendRequest {
        channel_key: payload.channel,
        title: payload.title,
        body: payload.body,
        actions: payload.actions,
        priority: payload.priority,
    };
    match send_user_notification(&state, request).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => notify_route_error_response(error),
    }
}

pub(crate) async fn android_notify_register(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<AndroidRegisterRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    match register_android_endpoint(&state, payload) {
        Ok(value) => Json(value).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    }
}

pub(crate) async fn notification_channels(state: &HubState) -> Result<Vec<NotificationChannel>> {
    let entries = registry_entries(state)?;
    let by_id = entries
        .into_iter()
        .map(|entry| (entry.agent_id.clone(), entry))
        .collect::<HashMap<_, _>>();
    let online = state.agents.lock().await;
    let mut channels = Vec::new();
    for (agent_id, connection) in online.iter() {
        let Some(entry) = by_id.get(agent_id) else {
            continue;
        };
        if !entry.enabled {
            continue;
        }
        let alias = entry.alias.as_deref().unwrap_or(&entry.agent_id);
        for channel in &connection.notification_channels {
            if channel.kind == "freedesktop" {
                channels.push(NotificationChannel {
                    key: format!("agent::{alias}::freedesktop"),
                    display_name: format!("{} desktop notification", entry.display_name),
                    available: true,
                    kind: "freedesktop".to_string(),
                    supports_actions: channel.supports_actions,
                    reason: None,
                    agent_id: Some(entry.agent_id.clone()),
                });
            }
        }
    }

    let ntfy_state = ntfy_channel_state(state).await;
    channels.push(NotificationChannel {
        key: "hub::ntfy".to_string(),
        display_name: "ntfy notification".to_string(),
        available: ntfy_state.reason.is_none(),
        kind: "ntfy".to_string(),
        supports_actions: false,
        reason: ntfy_state.reason.map(str::to_string),
        agent_id: None,
    });

    let android_reason = match android_endpoint_state(state) {
        AndroidEndpointState::NotConnected => "android_endpoint_not_connected",
        AndroidEndpointState::DeliveryNotImplemented => "android_delivery_not_implemented",
    };
    channels.push(NotificationChannel {
        key: "hub::android::notice".to_string(),
        display_name: "Android normal notification".to_string(),
        available: false,
        kind: "android_notice".to_string(),
        supports_actions: true,
        reason: Some(android_reason.to_string()),
        agent_id: None,
    });
    channels.push(NotificationChannel {
        key: "hub::android::alarm".to_string(),
        display_name: "Android alarm notification".to_string(),
        available: false,
        kind: "android_alarm".to_string(),
        supports_actions: true,
        reason: Some(android_reason.to_string()),
        agent_id: None,
    });
    Ok(channels)
}

pub(crate) async fn send_user_notification(
    state: &HubState,
    request: UserNotifySendRequest,
) -> std::result::Result<UserNotifySendResponse, NotifyRouteError> {
    match parse_notify_channel_key(&request.channel_key)? {
        NotifyChannelKey::AgentFreedesktop { alias } => {
            let agent_id = resolve_agent_alias(state, &alias)?;
            let delivery = UserNotifyDeliveryRequest {
                channel_key: request.channel_key.clone(),
                title: request.title.clone(),
                body: request.body.clone(),
                actions: request.actions.clone(),
                priority: request.priority.clone(),
            };
            let command = HubCommand::UserNotifyDeliver {
                request_id: random_id("req"),
                payload: delivery,
            };
            match request_agent(state, &agent_id, command, REQUEST_TIMEOUT_SECS).await {
                Ok(value) => {
                    if value.get("error").is_some() {
                        return Err(NotifyRouteError::DeliveryFailed {
                            channel_key: request.channel_key,
                            reason: value.to_string(),
                        });
                    }
                    Ok(UserNotifySendResponse {
                        channel_key: request.channel_key,
                        accepted: value
                            .get("delivered")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        delivery_id: None,
                        reason: value
                            .get("reason")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                }
                Err(reason) => Err(NotifyRouteError::DeliveryFailed {
                    channel_key: request.channel_key,
                    reason,
                }),
            }
        }
        NotifyChannelKey::HubNtfy => {
            if ntfy_not_configured(&state.config.remote_confirmation.ntfy) {
                return Err(NotifyRouteError::ChannelUnavailable {
                    channel_key: request.channel_key,
                    reason: "ntfy_not_configured",
                });
            }
            publish_ntfy_notification(state, &request)
                .await
                .map_err(|error| NotifyRouteError::DeliveryFailed {
                    channel_key: request.channel_key.clone(),
                    reason: error.to_string(),
                })?;
            Ok(UserNotifySendResponse {
                channel_key: request.channel_key,
                accepted: true,
                delivery_id: None,
                reason: None,
            })
        }
        NotifyChannelKey::AndroidNotice | NotifyChannelKey::AndroidAlarm => {
            let reason = match android_endpoint_state(state) {
                AndroidEndpointState::NotConnected => "android_endpoint_not_connected",
                AndroidEndpointState::DeliveryNotImplemented => "android_delivery_not_implemented",
            };
            Err(NotifyRouteError::ChannelUnavailable {
                channel_key: request.channel_key,
                reason,
            })
        }
    }
}

pub(crate) fn parse_notify_channel_key(
    key: &str,
) -> std::result::Result<NotifyChannelKey, NotifyRouteError> {
    let parts = key.split("::").collect::<Vec<_>>();
    match parts.as_slice() {
        ["agent", alias, "freedesktop"] if !alias.trim().is_empty() => {
            Ok(NotifyChannelKey::AgentFreedesktop {
                alias: alias.to_string(),
            })
        }
        ["hub", "ntfy"] => Ok(NotifyChannelKey::HubNtfy),
        ["hub", "android", "notice"] => Ok(NotifyChannelKey::AndroidNotice),
        ["hub", "android", "alarm"] => Ok(NotifyChannelKey::AndroidAlarm),
        _ => Err(NotifyRouteError::InvalidChannel(key.to_string())),
    }
}

pub(crate) fn resolve_agent_alias(
    state: &HubState,
    alias: &str,
) -> std::result::Result<String, NotifyRouteError> {
    let entries =
        registry_entries(state).map_err(|error| NotifyRouteError::Db(error.to_string()))?;
    entries
        .into_iter()
        .find(|entry| {
            entry.enabled
                && (entry.alias.as_deref() == Some(alias)
                    || (entry.alias.is_none() && entry.agent_id == alias))
        })
        .map(|entry| entry.agent_id)
        .ok_or_else(|| NotifyRouteError::AgentNotFound(alias.to_string()))
}

pub(crate) async fn publish_ntfy_notification(
    state: &HubState,
    request: &UserNotifySendRequest,
) -> Result<()> {
    let ntfy = &state.config.remote_confirmation.ntfy;
    let server_url = ntfy.server_url.trim_end_matches('/');
    let body = json!({
        "topic": ntfy.topic,
        "title": request.title,
        "message": request.body,
        "priority": ntfy_priority(request.priority.as_deref()),
    });
    let response = state.http.post(server_url).json(&body).send().await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("ntfy returned {}", response.status()))
    }
}

pub(crate) fn android_endpoint_state(state: &HubState) -> AndroidEndpointState {
    if android_endpoint_exists(state).unwrap_or(false) {
        AndroidEndpointState::DeliveryNotImplemented
    } else {
        AndroidEndpointState::NotConnected
    }
}

struct NtfyChannelState {
    reason: Option<&'static str>,
}

async fn ntfy_channel_state(state: &HubState) -> NtfyChannelState {
    let ntfy = &state.config.remote_confirmation.ntfy;
    if ntfy_not_configured(ntfy) {
        return NtfyChannelState {
            reason: Some("ntfy_not_configured"),
        };
    }
    match cached_ntfy_health(state).await {
        NtfyHealthStatus::Healthy => NtfyChannelState { reason: None },
        NtfyHealthStatus::Unhealthy => NtfyChannelState {
            reason: Some("ntfy_unhealthy"),
        },
        NtfyHealthStatus::Failed => NtfyChannelState {
            reason: Some("ntfy_health_check_failed"),
        },
    }
}

async fn cached_ntfy_health(state: &HubState) -> NtfyHealthStatus {
    const NTFY_HEALTH_CACHE_TTL_SECS: i64 = 45;

    let server_url = state
        .config
        .remote_confirmation
        .ntfy
        .server_url
        .trim_end_matches('/')
        .to_string();
    let now = Utc::now();
    if let Some(cached) = state.ntfy_health.lock().await.clone() {
        if cached.server_url == server_url
            && now.signed_duration_since(cached.checked_at).num_seconds()
                < NTFY_HEALTH_CACHE_TTL_SECS
        {
            return cached.result;
        }
    }

    let result = check_ntfy_health(state, &server_url).await;
    *state.ntfy_health.lock().await = Some(NtfyHealthCache {
        server_url,
        checked_at: now,
        result: result.clone(),
    });
    result
}

async fn check_ntfy_health(state: &HubState, server_url: &str) -> NtfyHealthStatus {
    let url = format!("{server_url}/v1/health");
    let response = match timeout(Duration::from_secs(3), state.http.get(url).send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => return NtfyHealthStatus::Failed,
        Err(_) => return NtfyHealthStatus::Failed,
    };
    if !response.status().is_success() {
        return NtfyHealthStatus::Failed;
    }
    match response.json::<Value>().await {
        Ok(value) if value.get("healthy").and_then(Value::as_bool) == Some(true) => {
            NtfyHealthStatus::Healthy
        }
        Ok(_) => NtfyHealthStatus::Unhealthy,
        Err(_) => NtfyHealthStatus::Failed,
    }
}

pub(crate) fn ntfy_not_configured(ntfy: &NtfyConfig) -> bool {
    let server_url = ntfy.server_url.trim().trim_end_matches('/');
    let topic = ntfy.topic.trim();
    server_url.is_empty()
        || topic.is_empty()
        || (server_url == "https://ntfy.example.invalid" && topic == "change-me-high-entropy-topic")
}

fn ntfy_priority(priority: Option<&str>) -> i32 {
    match priority {
        Some("min") | Some("low") => 2,
        Some("high") => 4,
        Some("urgent") | Some("alarm") => 5,
        _ => 3,
    }
}

fn android_endpoint_exists(state: &HubState) -> Result<bool> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "select exists(select 1 from notification_endpoints where kind = 'android' and enabled = 1)",
    )?;
    let exists: i64 = stmt.query_row([], |row| row.get(0))?;
    Ok(exists != 0)
}

fn register_android_endpoint(state: &HubState, payload: AndroidRegisterRequest) -> Result<Value> {
    let endpoint_id = random_id("android");
    let token = random_token();
    let token_hash = sha256_hex(&token);
    let now = Utc::now().to_rfc3339();
    let display_name = payload
        .display_name
        .unwrap_or_else(|| "Android endpoint".to_string());
    let capabilities = if payload.capabilities.is_null() {
        json!({})
    } else {
        payload.capabilities
    };
    let conn = state.db.lock().unwrap();
    conn.execute(
        "insert into notification_endpoints(endpoint_id, kind, display_name, capabilities_json, token_hash, enabled, last_seen_at, created_at)
         values (?1, 'android', ?2, ?3, ?4, 1, ?5, ?5)",
        params![
            endpoint_id,
            display_name,
            serde_json::to_string(&capabilities)?,
            token_hash,
            now
        ],
    )?;
    Ok(json!({
        "endpointId": endpoint_id,
        "token": token,
        "status": "registered",
        "deliveryImplemented": false
    }))
}

fn notify_route_error_response(error: NotifyRouteError) -> Response {
    match error {
        NotifyRouteError::InvalidChannel(channel) => api_error(
            StatusCode::BAD_REQUEST,
            "invalid_notify_channel",
            format!("Invalid notification channel: {channel}"),
        ),
        NotifyRouteError::AgentNotFound(alias) => api_error(
            StatusCode::NOT_FOUND,
            "agent_alias_not_found",
            format!("No enabled agent found for alias: {alias}"),
        ),
        NotifyRouteError::ChannelUnavailable {
            channel_key,
            reason,
        } => api_error(
            StatusCode::BAD_REQUEST,
            reason,
            format!("Notification channel {channel_key} is unavailable: {reason}"),
        ),
        NotifyRouteError::DeliveryFailed {
            channel_key,
            reason,
        } => api_error(
            StatusCode::BAD_GATEWAY,
            "notify_delivery_failed",
            format!("Notification delivery failed for {channel_key}: {reason}"),
        ),
        NotifyRouteError::Db(reason) => {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_gpt_protocol::{AgentRole, Capabilities};
    use axum::extract::ws::Message;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::{mpsc, Mutex};

    use crate::agents::command_request_id;
    use crate::db::init_db;
    use crate::state::AgentConnection;
    use crate::{HubConfig, RemoteConfirmationConfig};

    fn test_hub_config() -> HubConfig {
        HubConfig {
            remote_confirmation: RemoteConfirmationConfig {
                enabled: true,
                provider: "ntfy".to_string(),
                timeout_seconds: 45,
                ntfy: NtfyConfig {
                    server_url: "https://ntfy.example.invalid".to_string(),
                    topic: "secret-topic-for-test".to_string(),
                    callback_base_url: "https://callback.example.invalid".to_string(),
                },
            },
        }
    }

    fn test_state() -> HubState {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        HubState {
            api_key: "test-api-key".to_string(),
            db: Arc::new(StdMutex::new(conn)),
            config: Arc::new(test_hub_config()),
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            active_room: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: Some("https://hub.example.invalid".to_string()),
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(Mutex::new(Some(NtfyHealthCache {
                server_url: "https://ntfy.example.invalid".to_string(),
                checked_at: Utc::now(),
                result: NtfyHealthStatus::Healthy,
            }))),
        }
    }

    async fn insert_connection(
        state: &HubState,
        agent_id: &str,
        connection_id: &str,
        role: AgentRole,
    ) -> mpsc::UnboundedReceiver<Message> {
        let (tx, rx) = mpsc::unbounded_channel();
        state.agents.lock().await.insert(
            agent_id.to_string(),
            AgentConnection {
                connection_id: connection_id.to_string(),
                sender: tx,
                last_seen_at: Utc::now(),
                role,
                config_summary: None,
                notification_channels: Vec::new(),
            },
        );
        rx
    }

    #[test]
    fn parses_notify_channel_keys() {
        assert_eq!(
            parse_notify_channel_key("agent::laptop::freedesktop").unwrap(),
            NotifyChannelKey::AgentFreedesktop {
                alias: "laptop".to_string()
            }
        );
        assert_eq!(
            parse_notify_channel_key("hub::ntfy").unwrap(),
            NotifyChannelKey::HubNtfy
        );
        assert_eq!(
            parse_notify_channel_key("hub::android::notice").unwrap(),
            NotifyChannelKey::AndroidNotice
        );
        assert!(parse_notify_channel_key("agent::::freedesktop").is_err());
        assert!(parse_notify_channel_key("room::notify").is_err());
    }

    #[tokio::test]
    async fn notification_channels_include_agent_ntfy_and_android_placeholders() {
        let state = test_state();
        {
            let conn = state.db.lock().unwrap();
            let capabilities = serde_json::to_string(&Capabilities {
                sessions: true,
                confirmation: true,
                notification_actions: false,
            })
            .unwrap();
            conn.execute(
                "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
                 values ('agentic-gpt-slhaf-laptop', 'laptop', 'Laptop', 1, 'hash', null, ?1)",
                params![capabilities],
            )
            .unwrap();
        }
        let _rx = insert_connection(
            &state,
            "agentic-gpt-slhaf-laptop",
            "conn1",
            AgentRole::Normal,
        )
        .await;
        state
            .agents
            .lock()
            .await
            .get_mut("agentic-gpt-slhaf-laptop")
            .unwrap()
            .notification_channels
            .push(NotificationChannel {
                key: "agent::agentic-gpt-slhaf-laptop::freedesktop".to_string(),
                display_name: "Desktop".to_string(),
                available: true,
                kind: "freedesktop".to_string(),
                supports_actions: false,
                reason: None,
                agent_id: Some("agentic-gpt-slhaf-laptop".to_string()),
            });

        let channels = notification_channels(&state).await.unwrap();
        assert!(channels
            .iter()
            .any(|channel| channel.key == "agent::laptop::freedesktop"));
        assert!(channels.iter().any(|channel| channel.key == "hub::ntfy"));
        let android = channels
            .iter()
            .find(|channel| channel.key == "hub::android::notice")
            .unwrap();
        assert!(!android.available);
        assert_eq!(
            android.reason.as_deref(),
            Some("android_endpoint_not_connected")
        );
    }

    #[tokio::test]
    async fn ntfy_default_placeholder_is_not_configured() {
        let mut state = test_state();
        state.config = Arc::new(HubConfig::default_config());
        let channels = notification_channels(&state).await.unwrap();
        let ntfy = channels
            .iter()
            .find(|channel| channel.key == "hub::ntfy")
            .unwrap();
        assert!(!ntfy.available);
        assert_eq!(ntfy.reason.as_deref(), Some("ntfy_not_configured"));
    }

    #[tokio::test]
    async fn ntfy_health_cache_controls_listing_reason() {
        let state = test_state();
        *state.ntfy_health.lock().await = Some(NtfyHealthCache {
            server_url: "https://ntfy.example.invalid".to_string(),
            checked_at: Utc::now(),
            result: NtfyHealthStatus::Unhealthy,
        });
        let channels = notification_channels(&state).await.unwrap();
        let ntfy = channels
            .iter()
            .find(|channel| channel.key == "hub::ntfy")
            .unwrap();
        assert!(!ntfy.available);
        assert_eq!(ntfy.reason.as_deref(), Some("ntfy_unhealthy"));

        *state.ntfy_health.lock().await = Some(NtfyHealthCache {
            server_url: "https://ntfy.example.invalid".to_string(),
            checked_at: Utc::now(),
            result: NtfyHealthStatus::Failed,
        });
        let channels = notification_channels(&state).await.unwrap();
        let ntfy = channels
            .iter()
            .find(|channel| channel.key == "hub::ntfy")
            .unwrap();
        assert!(!ntfy.available);
        assert_eq!(ntfy.reason.as_deref(), Some("ntfy_health_check_failed"));
    }

    #[tokio::test]
    async fn android_registered_endpoint_still_reports_delivery_not_implemented() {
        let state = test_state();
        register_android_endpoint(
            &state,
            AndroidRegisterRequest {
                display_name: Some("Phone".to_string()),
                capabilities: json!({ "channels": ["notice", "alarm"] }),
            },
        )
        .unwrap();
        let channels = notification_channels(&state).await.unwrap();
        let android = channels
            .iter()
            .find(|channel| channel.key == "hub::android::alarm")
            .unwrap();
        assert!(!android.available);
        assert_eq!(
            android.reason.as_deref(),
            Some("android_delivery_not_implemented")
        );
        let error = send_user_notification(
            &state,
            UserNotifySendRequest {
                channel_key: "hub::android::alarm".to_string(),
                title: "Wake".to_string(),
                body: "Up".to_string(),
                actions: Vec::new(),
                priority: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            NotifyRouteError::ChannelUnavailable {
                channel_key: "hub::android::alarm".to_string(),
                reason: "android_delivery_not_implemented"
            }
        );
    }

    #[tokio::test]
    async fn user_notify_send_routes_agent_channel_by_alias() {
        let state = test_state();
        {
            let conn = state.db.lock().unwrap();
            let capabilities = serde_json::to_string(&Capabilities {
                sessions: true,
                confirmation: true,
                notification_actions: false,
            })
            .unwrap();
            conn.execute(
                "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
                 values ('agentic-gpt-slhaf-laptop', 'laptop', 'Laptop', 1, 'hash', null, ?1)",
                params![capabilities],
            )
            .unwrap();
        }
        let mut rx = insert_connection(
            &state,
            "agentic-gpt-slhaf-laptop",
            "conn1",
            AgentRole::Normal,
        )
        .await;
        let request_state = state.clone();
        let task = tokio::spawn(async move {
            send_user_notification(
                &request_state,
                UserNotifySendRequest {
                    channel_key: "agent::laptop::freedesktop".to_string(),
                    title: "Hello".to_string(),
                    body: "World".to_string(),
                    actions: Vec::new(),
                    priority: None,
                },
            )
            .await
            .unwrap()
        });
        let Message::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected command");
        };
        let command = serde_json::from_str::<HubCommand>(&text).unwrap();
        let request_id = command_request_id(&command).to_string();
        assert!(matches!(command, HubCommand::UserNotifyDeliver { .. }));
        let sender = state.pending.lock().await.remove(&request_id).unwrap();
        sender
            .send(json!({
                "channelKey": "agent::laptop::freedesktop",
                "delivered": true
            }))
            .unwrap();
        let response = task.await.unwrap();
        assert!(response.accepted);
        assert_eq!(response.channel_key, "agent::laptop::freedesktop");
    }
}
