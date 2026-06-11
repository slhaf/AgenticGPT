use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agentic_gpt_protocol::AgentRole;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message;

use crate::{config::Config, confirmation, sessions};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunMode {
    Normal,
    Room,
}

impl RunMode {
    pub(crate) fn role(self) -> AgentRole {
        match self {
            RunMode::Normal => AgentRole::Normal,
            RunMode::Room => AgentRole::Room,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            RunMode::Normal => "normal",
            RunMode::Room => "room",
        }
    }
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) config_path: PathBuf,
    pub(crate) config: Arc<RwLock<Config>>,
    pub(crate) run_mode: RunMode,
    pub(crate) sessions: Arc<Mutex<HashMap<String, sessions::ManagedSession>>>,
    pub(crate) hub_sender: Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>,
    pub(crate) pending_confirmations: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    pub(crate) temporary_mcp_allows: Arc<Mutex<Vec<confirmation::TemporaryMcpAllow>>>,
    pub(crate) notebook_writes: Arc<Mutex<()>>,
}
