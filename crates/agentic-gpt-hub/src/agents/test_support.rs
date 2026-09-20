use agentic_gpt_protocol::{
    AgentConnectionMode, AgentRole, Capabilities, JobInfo, JobKind, JobState, SafeConfigSummary,
};
use axum::http::{HeaderMap, HeaderValue};
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, Mutex};

use super::super::state::{AgentConnection, AgentTransport, HubState, OutboundAgentMessage};
use crate::db::init_db;
use crate::utils::sha256_hex;
use crate::{HubConfig, McpProfile, NtfyConfig, RemoteConfirmationConfig};

pub(super) fn test_state() -> HubState {
    let conn = Connection::open_in_memory().unwrap();
    init_db(&conn).unwrap();
    HubState {
        api_key: "test-api-key".to_string(),
        db: Arc::new(StdMutex::new(conn)),
        config: Arc::new(HubConfig {
            remote_confirmation: RemoteConfirmationConfig {
                enabled: false,
                provider: "none".to_string(),
                timeout_seconds: 45,
                ntfy: NtfyConfig {
                    server_url: String::new(),
                    topic: String::new(),
                    callback_base_url: String::new(),
                },
            },
        }),
        mcp_profile: McpProfile::Full,
        agents: Arc::new(Mutex::new(HashMap::new())),
        dispatch: Arc::new(crate::agents::dispatch::Dispatch::new()),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        jobs: Arc::new(Mutex::new(HashMap::new())),
        boot_generations: Arc::new(Mutex::new(HashMap::new())),
        active_room: Arc::new(Mutex::new(None)),
        http: reqwest::Client::new(),
        public_base_url: None,
        oauth_codes: Arc::new(Mutex::new(HashMap::new())),
        oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
        ntfy_health: Arc::new(Mutex::new(None)),
    }
}

pub(super) fn register_agent(state: &HubState, agent_id: &str, secret: &str) {
    let conn = state.db.lock().unwrap();
    let capabilities = Capabilities {
        jobs: true,
        confirmation: true,
        notification_actions: true,
    };
    conn.execute(
        "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
         values (?1, null, ?1, 1, ?2, null, ?3)",
        params![
            agent_id,
            sha256_hex(secret),
            serde_json::to_string(&capabilities).unwrap()
        ],
    )
    .unwrap();
}

pub(super) fn test_running_job(job_id: &str) -> JobInfo {
    let now = chrono::Utc::now();
    JobInfo {
        agent_id: "agent".to_string(),
        job_id: job_id.to_string(),
        group: None,
        batch_id: None,
        batch_call_id: None,
        batch_index: None,
        kind: JobKind::Process,
        state: JobState::Running,
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        finished_at: None,
        program: Some("sleep".to_string()),
        args: vec!["10".to_string()],
        working_directory: None,
        command_preview: Some("sleep 10".to_string()),
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
        skill_id: None,
        skill_path: None,
        installed_digest: None,
        mcp_server_id: None,
        mcp_tool_name: None,
        cancel_requested: false,
        cancel_outcome: None,
        termination_evidence: None,
    }
}

pub(super) fn test_config_summary() -> SafeConfigSummary {
    serde_json::from_value(json!({
        "workspaceRoot": "configured",
        "sandbox": {"enabled": false, "mode": "disabled"},
        "pathPolicy": {
            "writeRootCount": 1,
            "readOnlyRootCount": 0,
            "denyRootCount": 0,
            "writeRoots": [{"path": "workspace", "source": "workspace"}],
            "readOnlyRoots": [],
            "denyRoots": []
        },
        "policyRuleCounts": {"allow": 0, "confirm": 0, "deny": 0},
        "policyRules": {
            "allow": [],
            "confirm": [],
            "deny": [],
            "builtins": {"confirm": [], "deny": []}
        },
        "confirmationProvider": "none"
    }))
    .unwrap()
}

pub(super) fn agent_headers(secret: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-agent-secret", HeaderValue::from_str(secret).unwrap());
    headers
}

pub(super) async fn insert_connection(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    last_seen_at: chrono::DateTime<chrono::Utc>,
) -> mpsc::UnboundedReceiver<OutboundAgentMessage> {
    let (tx, rx) = mpsc::unbounded_channel();
    state.agents.lock().await.insert(
        agent_id.to_string(),
        AgentConnection {
            connection_id: connection_id.to_string(),
            sender: tx,
            last_seen_at,
            role: AgentRole::Normal,
            connection_mode: AgentConnectionMode::CommandCapable,
            hello_received: true,
            boot_generation: Some("testboot".to_string()),
            transport: AgentTransport::Sse,
            config_summary: None,
            notification_channels: Vec::new(),
        },
    );
    rx
}
