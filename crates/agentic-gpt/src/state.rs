use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use agentic_gpt_protocol::{AgentMessage, AgentRole};
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

use crate::{config::Config, confirmation, sessions};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunMode {
    Normal,
    Room,
}

impl RunMode {
    pub(crate) fn profile(self) -> CapabilityProfile {
        match self {
            RunMode::Normal => CapabilityProfile::Normal,
            RunMode::Room => CapabilityProfile::Room,
        }
    }

    pub(crate) fn role(self) -> AgentRole {
        self.profile().role()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Transport {
    Hub,
    TunnelStdio,
}

impl Transport {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Hub => "hub",
            Self::TunnelStdio => "tunnel-stdio",
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
            | (Transport::TunnelStdio, CapabilityProfile::Room) => Capabilities {
                skills: true,
                bootstrap: true,
                diary: true,
                notebook: true,
                notifications: true,
            },
            (Transport::TunnelStdio, CapabilityProfile::Normal) => Capabilities {
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
pub(crate) struct AppState {
    pub(crate) config_path: PathBuf,
    pub(crate) config: Arc<RwLock<Config>>,
    pub(crate) runtime: RuntimeModel,
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) supervised: bool,
    pub(crate) file_locks: Arc<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>>,
    pub(crate) sessions: Arc<Mutex<HashMap<String, sessions::ManagedSession>>>,
    pub(crate) hub_sender: Arc<Mutex<Option<mpsc::UnboundedSender<AgentMessage>>>>,
    pub(crate) reporting_sender: Arc<Mutex<Option<mpsc::Sender<AgentMessage>>>>,
    pub(crate) pending_confirmations: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    pub(crate) temporary_mcp_allows: Arc<Mutex<Vec<confirmation::TemporaryMcpAllow>>>,
    pub(crate) notebook_writes: Arc<Mutex<()>>,
    pub(crate) skills_writes: Arc<Mutex<()>>,
    pub(crate) skill_leases: Arc<sessions::SkillLeaseManager>,
    pub(crate) skill_installs: Arc<crate::skill_installs::InstallManager>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_capabilities_follow_transport_and_profile() {
        let hub_normal = RuntimeModel::hub(CapabilityProfile::Normal).capabilities();
        assert!(!hub_normal.skills);
        assert!(!hub_normal.bootstrap);
        assert!(!hub_normal.diary);
        assert!(!hub_normal.notebook);
        assert!(hub_normal.notifications);

        let tunnel_normal = RuntimeModel::tunnel(CapabilityProfile::Normal, false).capabilities();
        assert!(tunnel_normal.skills);
        assert!(tunnel_normal.bootstrap);
        assert!(!tunnel_normal.diary);
        assert!(!tunnel_normal.notebook);
        assert!(!tunnel_normal.notifications);

        let tunnel_room = RuntimeModel::tunnel(CapabilityProfile::Room, true);
        assert_eq!(tunnel_room.hub_mode, HubMode::ReportingOnly);
        assert!(tunnel_room.capabilities().skills);
        assert!(tunnel_room.capabilities().bootstrap);
        assert!(tunnel_room.capabilities().diary);
        assert!(tunnel_room.capabilities().notebook);
        assert_eq!(tunnel_room.profile.role(), AgentRole::Room);
    }
}
