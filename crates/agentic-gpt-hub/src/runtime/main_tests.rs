use super::*;
use crate::config::RemoteConfirmationConfig;
use crate::state::{HubState, McpProfile};
use chrono::Utc;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

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
        mcp_profile: McpProfile::Full,
        agents: Arc::new(agents::lifecycle::Connections::new()),
        dispatch: Arc::new(agents::dispatch::Dispatch::new()),
        confirmations: Arc::new(confirmation::Confirmations::new()),
        process_cache: Arc::new(state::ProcessCache::new()),
        boot_generations: Arc::new(Mutex::new(HashMap::new())),
        active_room: Arc::new(Mutex::new(None)),
        http: reqwest::Client::new(),
        public_base_url: Some("https://hub.example.invalid".to_string()),
        oauth_codes: Arc::new(Mutex::new(HashMap::new())),
        oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
        ntfy_health: Arc::new(Mutex::new(Some(notify::NtfyHealthCache {
            server_url: "https://ntfy.example.invalid".to_string(),
            checked_at: Utc::now(),
            result: notify::NtfyHealthStatus::Healthy,
        }))),
    }
}

#[tokio::test]
async fn hub_info_reports_safe_runtime_summary() {
    let state = test_state();
    let response = state::projection::build_hub_info_response(&state)
        .await
        .unwrap();
    let value = serde_json::to_value(response).unwrap();
    let text = serde_json::to_string(&value).unwrap();

    assert_eq!(value["service"], "agentic-gpt-hub");
    assert_eq!(value["publicBaseUrl"], "https://hub.example.invalid");
    assert_eq!(value["remoteConfirmation"]["enabled"], true);
    assert_eq!(value["remoteConfirmation"]["provider"], "ntfy");
    assert_eq!(value["remoteConfirmation"]["ntfyConfigured"], true);
    assert_eq!(value["agents"]["registeredCount"], 0);
    assert_eq!(value["agents"]["onlineCount"], 0);
    assert_eq!(value["counts"]["pendingRequestCount"], 0);
    assert!(!text.contains("secret-topic-for-test"));
    assert!(!text.contains("callback.example.invalid"));
    assert!(!text.contains("test-api-key"));
}
