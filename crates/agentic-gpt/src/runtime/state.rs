use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use agentic_gpt_protocol::{AgentMessage, AgentRole};
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

use crate::{browser_manager::BrowserRuntimeManager, browser_runtime::BrowserRuntimeDescriptor};
use crate::{config::Config, confirmation, process, skills};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Transport {
    Hub,
    TunnelStdio,
    LocalUnix,
}

impl Transport {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Hub => "hub",
            Self::TunnelStdio => "tunnel-stdio",
            Self::LocalUnix => "local-unix",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CapabilityProfile {
    Normal,
    Room,
}

impl CapabilityProfile {
    pub(crate) fn role(self) -> AgentRole {
        match self {
            Self::Normal => AgentRole::Normal,
            Self::Room => AgentRole::Room,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Room => "room",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HubMode {
    CommandCapable,
    ReportingOnly,
    Disabled,
}

impl HubMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::CommandCapable => "command-capable",
            Self::ReportingOnly => "reporting-only",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeModel {
    pub(crate) transport: Transport,
    pub(crate) profile: CapabilityProfile,
    pub(crate) hub_mode: HubMode,
}

impl RuntimeModel {
    pub(crate) fn hub(profile: CapabilityProfile) -> Self {
        Self {
            transport: Transport::Hub,
            profile,
            hub_mode: HubMode::CommandCapable,
        }
    }

    pub(crate) fn tunnel(profile: CapabilityProfile, reporting_enabled: bool) -> Self {
        Self {
            transport: Transport::TunnelStdio,
            profile,
            hub_mode: if reporting_enabled {
                HubMode::ReportingOnly
            } else {
                HubMode::Disabled
            },
        }
    }

    pub(crate) fn local(profile: CapabilityProfile) -> Self {
        Self {
            transport: Transport::LocalUnix,
            profile,
            hub_mode: HubMode::Disabled,
        }
    }

    pub(crate) fn label(self) -> String {
        format!("{}:{}", self.transport.label(), self.profile.label())
    }

    pub(crate) fn capabilities(self) -> Capabilities {
        match (self.transport, self.profile) {
            (Transport::Hub, CapabilityProfile::Normal) => Capabilities {
                skills: false,
                bootstrap: false,
                diary: false,
                notebook: false,
                notifications: true,
            },
            (Transport::Hub, CapabilityProfile::Room)
            | (Transport::TunnelStdio, CapabilityProfile::Room)
            | (Transport::LocalUnix, CapabilityProfile::Room) => Capabilities {
                skills: true,
                bootstrap: true,
                diary: true,
                notebook: true,
                notifications: true,
            },
            (Transport::TunnelStdio, CapabilityProfile::Normal)
            | (Transport::LocalUnix, CapabilityProfile::Normal) => Capabilities {
                skills: true,
                bootstrap: true,
                diary: false,
                notebook: false,
                notifications: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Capabilities {
    pub(crate) skills: bool,
    pub(crate) bootstrap: bool,
    pub(crate) diary: bool,
    pub(crate) notebook: bool,
    pub(crate) notifications: bool,
}

#[derive(Clone)]
pub(crate) struct BrowserRuntimeContext {
    pub(crate) descriptor: BrowserRuntimeDescriptor,
    pub(crate) manager: Arc<BrowserRuntimeManager>,
}

impl BrowserRuntimeContext {
    pub(crate) fn new(
        descriptor: BrowserRuntimeDescriptor,
        launch_spec: crate::browser_runtime::NodeReplLaunchSpec,
    ) -> Arc<Self> {
        let manager =
            BrowserRuntimeManager::new(launch_spec, descriptor.browser_client_path.clone());
        Arc::new(Self {
            descriptor,
            manager,
        })
    }
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) config_path: PathBuf,
    pub(crate) config: Arc<RwLock<Config>>,
    pub(crate) private_state: crate::private_state::PrivateStatePaths,
    pub(crate) event_store: Arc<crate::event_store::EventStore>,
    #[allow(dead_code)]
    pub(crate) process_history: std::sync::Arc<crate::process_history::ProcessHistoryStore>,
    pub(crate) browser_runtime: Option<Arc<BrowserRuntimeContext>>,
    pub(crate) runtime: RuntimeModel,
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) boot_generation: String,
    pub(crate) supervised: bool,
    pub(crate) file_locks: Arc<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>>,
    pub(crate) processes: Arc<Mutex<HashMap<String, process::ManagedProcess>>>,
    pub(crate) hub_sender: Arc<Mutex<Option<mpsc::UnboundedSender<AgentMessage>>>>,
    pub(crate) reporting_sender: Arc<Mutex<Option<mpsc::Sender<AgentMessage>>>>,
    pub(crate) pending_confirmations: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    pub(crate) temporary_mcp_allows: Arc<Mutex<Vec<confirmation::TemporaryMcpAllow>>>,
    pub(crate) mcp_concurrency: Arc<process::McpConcurrency>,
    #[allow(dead_code)] // Serialized Room maintenance starts using this in Phase 3.
    pub(crate) room_repository_writes: Arc<Mutex<()>>,
    pub(crate) skills_writes: Arc<Mutex<()>>,
    pub(crate) skill_leases: Arc<skills::SkillLeaseManager>,
    pub(crate) skill_installs: Arc<crate::skill_installs::InstallManager>,
}

impl AppState {
    pub(crate) fn new_process_id(&self) -> String {
        format!(
            "process_{}_{}",
            self.boot_generation,
            uuid::Uuid::new_v4().simple()
        )
    }

    pub(crate) fn process_id_generation<'a>(&self, process_id: &'a str) -> Option<&'a str> {
        process_id
            .strip_prefix("process_")
            .and_then(|value| value.split_once('_'))
            .map(|(generation, _)| generation)
    }
}
