use agentic_gpt_protocol::{
    AgentConnectionMode, AgentRole, ConfirmationDecision, JobInfo, NotificationChannel,
    SafeConfigSummary,
};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::{oauth, room, HubConfig};

#[derive(Clone)]
pub(crate) struct HubState {
    pub(crate) api_key: String,
    pub(crate) db: Arc<StdMutex<Connection>>,
    pub(crate) config: Arc<HubConfig>,
    pub(crate) mcp_profile: McpProfile,
    pub(crate) agents: Arc<Mutex<HashMap<String, AgentConnection>>>,
    pub(crate) pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    pub(crate) pending_confirmations: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
    pub(crate) jobs: Arc<Mutex<HashMap<String, HashMap<String, JobInfo>>>>,
    pub(crate) boot_generations: Arc<Mutex<HashMap<String, String>>>,
    pub(crate) active_room: Arc<Mutex<Option<room::ActiveRoomConnection>>>,
    pub(crate) http: reqwest::Client,
    pub(crate) public_base_url: Option<String>,
    pub(crate) oauth_codes: Arc<Mutex<HashMap<String, oauth::OAuthAuthorizationCode>>>,
    pub(crate) oauth_tokens: Arc<Mutex<HashMap<String, oauth::OAuthAccessToken>>>,
    pub(crate) ntfy_health: Arc<Mutex<Option<crate::notify::NtfyHealthCache>>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub(crate) enum McpProfile {
    #[default]
    Full,
    Coordinator,
}

impl McpProfile {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Coordinator => "coordinator",
        }
    }
}

#[derive(Clone)]
pub(crate) struct AgentConnection {
    pub(crate) connection_id: String,
    pub(crate) sender: mpsc::UnboundedSender<OutboundAgentMessage>,
    pub(crate) last_seen_at: DateTime<Utc>,
    pub(crate) role: AgentRole,
    pub(crate) connection_mode: AgentConnectionMode,
    pub(crate) hello_received: bool,
    pub(crate) boot_generation: Option<String>,
    pub(crate) transport: AgentTransport,
    pub(crate) config_summary: Option<SafeConfigSummary>,
    pub(crate) notification_channels: Vec<NotificationChannel>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentTransport {
    WebSocket,
    Sse,
}

#[derive(Clone, Debug)]
pub(crate) enum OutboundAgentMessage {
    Text(String),
    Close,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingConfirmation {
    pub(crate) confirmation_id: String,
    pub(crate) request_id: String,
    pub(crate) agent_id: String,
    pub(crate) token_hash: String,
    pub(crate) command_preview: String,
    pub(crate) risk_level: String,
    pub(crate) reason: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) resolved: bool,
    pub(crate) decision: Option<ConfirmationDecision>,
}
