use agentic_gpt_protocol::{EventOrigin, HubCommand};

use crate::{
    config::{Config, ToolNamespace},
    state::RuntimeModel,
};

/// The real ingress that admitted an Agent operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestIngress {
    TunnelStdio,
    LocalUnix,
    Http,
    Hub,
    Cli,
}

impl RequestIngress {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::TunnelStdio => "tunnel:stdio",
            Self::LocalUnix => "local:unix",
            Self::Http => "http:mcp",
            Self::Hub => "hub",
            Self::Cli => "localadmin",
        }
    }

    pub(crate) fn source(self, tool: &str) -> String {
        let prefix = match self {
            Self::TunnelStdio => "tunnel",
            Self::LocalUnix => "local",
            Self::Http => "http",
            Self::Hub => "hub",
            Self::Cli => "localadmin",
        };
        format!("{prefix}:{tool}")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestContext<'a> {
    pub(crate) ingress: RequestIngress,
    pub(crate) operation: &'a str,
    pub(crate) event_origin: Option<&'a EventOrigin>,
}

impl<'a> RequestContext<'a> {
    pub(crate) fn new(ingress: RequestIngress, operation: &'a str) -> Self {
        Self {
            ingress,
            operation,
            event_origin: None,
        }
    }

    pub(crate) fn with_event_origin(
        ingress: RequestIngress,
        operation: &'a str,
        event_origin: Option<&'a EventOrigin>,
    ) -> Self {
        Self {
            ingress,
            operation,
            event_origin,
        }
    }

    pub(crate) fn source(self) -> String {
        self.ingress.source(self.operation)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionError {
    ToolsetDisabled { namespace: ToolNamespace },
    CapabilityRequired,
    CapabilityUnavailable { operation: String },
    UnknownOperation { operation: String },
}

impl AdmissionError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::ToolsetDisabled {
                namespace: ToolNamespace::Room,
            } => "room_toolset_required",
            Self::ToolsetDisabled { .. } => "toolset_required",
            Self::CapabilityRequired => "room_agent_required",
            Self::CapabilityUnavailable { .. } => "capability_unavailable",
            Self::UnknownOperation { .. } => "unknown_operation",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::ToolsetDisabled {
                namespace: ToolNamespace::Room,
            } => "room commands require toolsets.room to be enabled".to_string(),
            Self::ToolsetDisabled { namespace } => {
                format!("operation requires toolsets.{namespace} to be enabled")
            }
            Self::CapabilityRequired => "room commands require profile=room in config".to_string(),
            Self::CapabilityUnavailable { operation } => {
                format!("{operation} is unavailable for this runtime")
            }
            Self::UnknownOperation { operation } => format!("unknown operation: {operation}"),
        }
    }
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AdmissionError {}

pub(crate) const EVENT_API_TOOL_NAMES: &[&str] = &["event.get", "event.list", "event.mark"];

pub(crate) fn is_event_api_operation(operation: &str) -> bool {
    EVENT_API_TOOL_NAMES.contains(&operation)
}

/// Existing Agent-local descriptor namespace metadata. Discovery may filter this list, but
/// admission never treats descriptor presence or annotation hints as authorization.
pub(crate) const TOOL_NAMESPACE_BY_NAME: &[(&str, ToolNamespace)] = &[
    ("agent.info", ToolNamespace::Agent),
    ("browser.acquire", ToolNamespace::Browser),
    ("browser.list", ToolNamespace::Browser),
    ("browser.manual", ToolNamespace::Browser),
    ("browser.release", ToolNamespace::Browser),
    ("browser.repl", ToolNamespace::Browser),
    ("browser.reset", ToolNamespace::Browser),
    ("bootstrap", ToolNamespace::Room),
    ("bootstrap.read", ToolNamespace::Room),
    ("file.edit", ToolNamespace::File),
    ("file.read", ToolNamespace::File),
    ("file.search", ToolNamespace::File),
    ("process.cancel", ToolNamespace::Process),
    ("process.list", ToolNamespace::Process),
    ("process.read", ToolNamespace::Process),
    ("mcp.batch", ToolNamespace::Mcp),
    ("mcp.callTool", ToolNamespace::Mcp),
    ("mcp.list", ToolNamespace::Mcp),
    ("process.batch", ToolNamespace::Process),
    ("process.exec", ToolNamespace::Process),
    ("room.diary.active", ToolNamespace::Room),
    ("room.diary.read", ToolNamespace::Room),
    ("room.maintenance.status", ToolNamespace::Room),
    ("room.maintenance.submit", ToolNamespace::Room),
    ("room.notebook.read", ToolNamespace::Room),
    ("room.notebook.recent", ToolNamespace::Room),
    ("room.notebook.search", ToolNamespace::Room),
    ("room.state.list", ToolNamespace::Room),
    ("room.state.read", ToolNamespace::Room),
    ("skills.install", ToolNamespace::Skills),
    ("skills.install.cancel", ToolNamespace::Skills),
    ("skills.install.get", ToolNamespace::Skills),
    ("skills.list", ToolNamespace::Skills),
    ("skills.read", ToolNamespace::Skills),
    ("skills.run", ToolNamespace::Skills),
    ("skills.setActive", ToolNamespace::Skills),
    ("tmux.exec", ToolNamespace::Tmux),
    ("tmux.panes", ToolNamespace::Tmux),
    ("tmux.pasteText", ToolNamespace::Tmux),
    ("tmux.sessions", ToolNamespace::Tmux),
];

pub(crate) fn tool_namespace(name: &str) -> Option<ToolNamespace> {
    TOOL_NAMESPACE_BY_NAME
        .iter()
        .find(|(tool, _)| *tool == name)
        .map(|(_, namespace)| *namespace)
}

pub(crate) fn tool_is_read_only(name: &str) -> bool {
    !matches!(
        name,
        "process.exec"
            | "browser.acquire"
            | "browser.repl"
            | "browser.reset"
            | "browser.release"
            | "process.batch"
            | "process.cancel"
            | "tmux.sessions"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "mcp.batch"
            | "mcp.callTool"
            | "skills.activate"
            | "skills.deactivate"
            | "skills.setActive"
            | "skills.install"
            | "skills.install.cancel"
            | "room.maintenance.submit"
            | "skills.run"
            | "event.mark"
            | "file.edit"
    )
}

pub(crate) fn tool_is_destructive(name: &str) -> bool {
    matches!(
        name,
        "file.edit"
            | "browser.repl"
            | "browser.reset"
            | "browser.release"
            | "process.cancel"
            | "tmux.sessions"
            | "tmux.closeSession"
            | "process.exec"
            | "process.batch"
            | "mcp.batch"
            | "mcp.callTool"
            | "tmux.exec"
            | "tmux.pasteText"
            | "skills.deactivate"
            | "skills.install"
            | "skills.install.cancel"
            | "room.maintenance.submit"
            | "skills.setActive"
            | "skills.run"
            | "event.mark"
    )
}

pub(crate) fn tool_is_open_world(name: &str) -> bool {
    matches!(
        name,
        "process.exec"
            | "browser.repl"
            | "process.batch"
            | "tmux.sessions"
            | "mcp.batch"
            | "mcp.callTool"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "skills.install"
            | "skills.run"
    )
}

fn operation_namespace(operation: &str) -> Option<ToolNamespace> {
    if let Some(namespace) = tool_namespace(operation) {
        return Some(namespace);
    }
    match operation {
        "bootstrap" | "bootstrap.read" | "room.bootstrap" | "room.bootstrap.read" => {
            Some(ToolNamespace::Room)
        }
        "tmux.listSessions" | "tmux.listPanes" | "tmux.capturePane" | "tmux.createSession"
        | "tmux.closeSession" => Some(ToolNamespace::Tmux),
        "mcp.listServers" | "mcp.listTools" => Some(ToolNamespace::Mcp),
        "skills.active" | "skills.activate" | "skills.deactivate" | "skills.search" => {
            Some(ToolNamespace::Skills)
        }
        _ => None,
    }
}

fn is_current_room_operation(operation: &str) -> bool {
    matches!(
        operation,
        "room.diary.active"
            | "room.diary.read"
            | "room.notebook.recent"
            | "room.notebook.search"
            | "room.notebook.read"
            | "room.state.list"
            | "room.state.read"
            | "room.maintenance.status"
            | "room.maintenance.submit"
    )
}

fn is_hub_room_toolset_operation(operation: &str) -> bool {
    matches!(
        operation,
        "room.bootstrap" | "room.bootstrap.read" | "bootstrap" | "bootstrap.read"
    )
}

pub(crate) fn authorize(
    runtime: RuntimeModel,
    config: &Config,
    context: RequestContext<'_>,
) -> Result<(), AdmissionError> {
    let operation = context.operation;
    if context.ingress == RequestIngress::Cli {
        if matches!(
            operation,
            "tmux.listSessions" | "tmux.attach" | "tmux.createSession" | "tmux.closeSession"
        ) {
            return Ok(());
        }
        return Err(AdmissionError::UnknownOperation {
            operation: operation.to_string(),
        });
    }
    if is_event_api_operation(operation) {
        return Ok(());
    }
    if operation == "privateevent.inject" {
        return if context.ingress == RequestIngress::LocalUnix {
            Ok(())
        } else {
            Err(AdmissionError::UnknownOperation {
                operation: operation.to_string(),
            })
        };
    }
    if matches!(operation, "event.panel" | "event.settle") {
        return if context.ingress == RequestIngress::Hub {
            Ok(())
        } else {
            Err(AdmissionError::UnknownOperation {
                operation: operation.to_string(),
            })
        };
    }

    if operation == "user.notify.deliver" {
        if runtime.capabilities().notifications {
            return Ok(());
        }
        return Err(AdmissionError::CapabilityUnavailable {
            operation: operation.to_string(),
        });
    }

    if context.ingress == RequestIngress::Hub {
        if operation_namespace(operation).is_none() {
            return Err(AdmissionError::UnknownOperation {
                operation: operation.to_string(),
            });
        }
        if is_current_room_operation(operation) {
            if runtime.profile != crate::state::CapabilityProfile::Room {
                return Err(AdmissionError::CapabilityRequired);
            }
            if !config.toolsets.is_enabled(ToolNamespace::Room) {
                return Err(AdmissionError::ToolsetDisabled {
                    namespace: ToolNamespace::Room,
                });
            }
        }
        if is_hub_room_toolset_operation(operation)
            && !config.toolsets.is_enabled(ToolNamespace::Room)
        {
            return Err(AdmissionError::ToolsetDisabled {
                namespace: ToolNamespace::Room,
            });
        }
        if operation_namespace(operation) == Some(ToolNamespace::Skills)
            && !runtime.capabilities().skills
        {
            return Err(AdmissionError::CapabilityRequired);
        }
        return Ok(());
    }

    let Some(namespace) = operation_namespace(operation) else {
        return Err(AdmissionError::UnknownOperation {
            operation: operation.to_string(),
        });
    };
    if !config.toolsets.is_enabled(namespace) {
        return Err(AdmissionError::ToolsetDisabled { namespace });
    }
    if namespace == ToolNamespace::Skills && !runtime.capabilities().skills {
        return Err(AdmissionError::CapabilityRequired);
    }
    Ok(())
}

pub(crate) fn hub_command_name(command: &HubCommand) -> &'static str {
    match command {
        HubCommand::Exec { .. } => "process.exec",
        HubCommand::ProcessBatch { .. } => "process.batch",
        HubCommand::ProcessList { .. } => "process.list",
        HubCommand::ProcessRead { .. } => "process.read",
        HubCommand::EventList { .. } => "event.list",
        HubCommand::EventGet { .. } => "event.get",
        HubCommand::EventMark { .. } => "event.mark",
        HubCommand::EventSettle { .. } => "event.settle",
        HubCommand::EventPanel { .. } => "event.panel",
        HubCommand::ProcessCancel { .. } => "process.cancel",
        HubCommand::TmuxListSessions { .. } => "tmux.listSessions",
        HubCommand::TmuxListPanes { .. } => "tmux.listPanes",
        HubCommand::TmuxCapturePane { .. } => "tmux.capturePane",
        HubCommand::TmuxPasteText { .. } => "tmux.pasteText",
        HubCommand::TmuxExec { .. } => "tmux.exec",
        HubCommand::TmuxCreateSession { .. } => "tmux.createSession",
        HubCommand::TmuxCloseSession { .. } => "tmux.closeSession",
        HubCommand::McpListServers { .. } => "mcp.listServers",
        HubCommand::McpListTools { .. } => "mcp.listTools",
        HubCommand::McpCallTool { .. } => "mcp.callTool",
        HubCommand::McpBatch { .. } => "mcp.batch",
        HubCommand::UserNotifyDeliver { .. } => "user.notify.deliver",
        HubCommand::RoomDiaryActive { .. } => "room.diary.active",
        HubCommand::RoomDiaryRead { .. } => "room.diary.read",
        HubCommand::RoomNotebookRecent { .. } => "room.notebook.recent",
        HubCommand::RoomNotebookSearch { .. } => "room.notebook.search",
        HubCommand::RoomNotebookRead { .. } => "room.notebook.read",
        HubCommand::RoomStateList { .. } => "room.state.list",
        HubCommand::RoomStateRead { .. } => "room.state.read",
        HubCommand::RoomMaintenanceStatus { .. } => "room.maintenance.status",
        HubCommand::RoomMaintenanceSubmit { .. } => "room.maintenance.submit",
        HubCommand::RoomBootstrap { .. } => "room.bootstrap",
        HubCommand::RoomBootstrapRead { .. } => "room.bootstrap.read",
        HubCommand::Bootstrap { .. } => "bootstrap",
        HubCommand::BootstrapRead { .. } => "bootstrap.read",
        HubCommand::SkillsList { .. } => "skills.list",
        HubCommand::SkillsRead { .. } => "skills.read",
        HubCommand::SkillsSearch { .. } => "skills.search",
        HubCommand::SkillsActive { .. } => "skills.active",
        HubCommand::SkillsActivate { .. } => "skills.activate",
        HubCommand::SkillsDeactivate { .. } => "skills.deactivate",
        HubCommand::SkillsInstall { .. } => "skills.install",
        HubCommand::SkillsInstallGet { .. } => "skills.install.get",
        HubCommand::SkillsInstallCancel { .. } => "skills.install.cancel",
        HubCommand::SkillsRun { .. } => "skills.run",
    }
}
