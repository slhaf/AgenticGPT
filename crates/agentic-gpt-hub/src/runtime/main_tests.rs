use super::*;
use crate::config::RemoteConfirmationConfig;
use crate::routes::parse_bearer_token;
use crate::state::{HubState, McpProfile};
use agentic_gpt_protocol::HubCommand;
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
        job_cache: Arc::new(state::JobCache::new()),
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

#[test]
fn parses_bearer_case_insensitively() {
    assert_eq!(parse_bearer_token("bearer  abc ").as_deref(), Some("abc"));
    assert_eq!(parse_bearer_token("Basic abc"), None);
}

#[test]
fn ntfy_mcp_confirmation_stays_within_three_action_limit() {
    let actions = confirmation::ntfy_confirmation_actions(
        "https://hub.example",
        "confirm-1",
        "token-1",
        Some("mcpTool"),
    );
    let actions = actions.as_array().unwrap();
    assert_eq!(actions.len(), 3);
    assert_eq!(actions[0]["label"], "Allow once");
    assert_eq!(actions[1]["label"], "Allow MCP 30m");
    assert_eq!(actions[2]["label"], "Deny");
}

#[test]
fn skills_commands_have_run_types() {
    let commands = [
        HubCommand::SkillsList {
            request_id: "req-list".to_string(),
        },
        HubCommand::SkillsRead {
            request_id: "req-read".to_string(),
            payload: agentic_gpt_protocol::SkillReadRequest {
                id: "demo".to_string(),
                path: None,
            },
        },
        HubCommand::SkillsSearch {
            request_id: "req-search".to_string(),
            payload: agentic_gpt_protocol::SkillSearchRequest {
                query: "demo".to_string(),
                limit: None,
            },
        },
        HubCommand::SkillsActive {
            request_id: "req-active".to_string(),
        },
        HubCommand::SkillsActivate {
            request_id: "req-activate".to_string(),
            payload: agentic_gpt_protocol::SkillActivationRequest {
                id: "demo".to_string(),
            },
        },
        HubCommand::SkillsDeactivate {
            request_id: "req-deactivate".to_string(),
            payload: agentic_gpt_protocol::SkillActivationRequest {
                id: "demo".to_string(),
            },
        },
        HubCommand::SkillsInstall {
            request_id: "req-install".to_string(),
            payload: agentic_gpt_protocol::SkillInstallRequest {
                id: "demo".to_string(),
                source: agentic_gpt_protocol::SkillInstallSource::Files { files: vec![] },
                replace_existing: false,
                activate_after_install: None,
                idempotency_key: None,
            },
        },
        HubCommand::SkillsInstallGet {
            request_id: "req-install-get".to_string(),
            payload: agentic_gpt_protocol::SkillInstallGetRequest {
                install_id: "install-1".to_string(),
                wait_seconds: Some(0),
            },
        },
        HubCommand::SkillsInstallCancel {
            request_id: "req-install-cancel".to_string(),
            payload: agentic_gpt_protocol::SkillInstallCancelRequest {
                install_id: "install-1".to_string(),
            },
        },
        HubCommand::SkillsRun {
            request_id: "req-run".to_string(),
            payload: agentic_gpt_protocol::SkillRunRequest {
                id: "demo".to_string(),
                path: "scripts/check.sh".to_string(),
                group: None,
                args: None,
                working_directory: None,
                wait_seconds: Some(0),
            },
        },
    ];
    let expected = [
        "skills.list",
        "skills.read",
        "skills.search",
        "skills.active",
        "skills.activate",
        "skills.deactivate",
        "skills.install",
        "skills.install.get",
        "skills.install.cancel",
        "skills.run",
    ];

    for (index, command) in commands.iter().enumerate() {
        let expected_type = expected[index];
        assert_eq!(crate::runs::command_type(command), expected_type);
    }
}

#[test]
fn bootstrap_commands_have_run_types() {
    let commands = [
        HubCommand::RoomBootstrap {
            request_id: "req-bootstrap".to_string(),
        },
        HubCommand::RoomBootstrapRead {
            request_id: "req-bootstrap-read".to_string(),
            payload: agentic_gpt_protocol::BootstrapReadRequest {
                id: "diary".to_string(),
            },
        },
    ];
    let expected = ["room.bootstrap", "room.bootstrap.read"];
    for (index, command) in commands.iter().enumerate() {
        assert_eq!(crate::runs::command_type(command), expected[index]);
    }
}
