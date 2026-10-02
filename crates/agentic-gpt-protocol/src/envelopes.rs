use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::events::{
    EventGetRequest, EventListRequest, EventMarkRequest, EventOrigin, EventResponseDisposition,
    EventSettleRequest, EventSource,
};
use super::identity_config::{AgentConnectionMode, AgentRole, AgentRunReport, SafeConfigSummary};
use super::mcp::{McpBatchRequest, McpCallToolRequest, McpListToolsRequest};
use super::notification_tmux::{
    ConfirmationDecision, ConfirmationPayload, NotificationChannel, TmuxCapturePaneRequest,
    TmuxCloseSessionRequest, TmuxCreateSessionRequest, TmuxExecRequest, TmuxListPanesRequest,
    TmuxPasteTextRequest, UserNotifyDeliveryRequest,
};
use super::process::{
    ProcessBatchExecRequest, ProcessCancelRequest, ProcessExecRequest, ProcessInfo,
    ProcessListRequest, ProcessReadRequest,
};
use super::room::{
    RoomDiaryActiveRequest, RoomDiaryReadRequest, RoomMaintenanceStatusRequest,
    RoomMaintenanceSubmitRequest, RoomNotebookReadRequest, RoomNotebookRecentRequest,
    RoomNotebookSearchRequest, RoomStateListRequest, RoomStateReadRequest,
};
use super::skill_bootstrap::{
    BootstrapReadRequest, SkillActivationRequest, SkillInstallCancelRequest,
    SkillInstallGetRequest, SkillInstallRequest, SkillReadRequest, SkillRunRequest,
    SkillSearchRequest,
};

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HubCommand {
    #[serde(rename = "process.exec")]
    Exec {
        request_id: String,
        payload: ProcessExecRequest,
    },
    #[serde(rename = "process.batch")]
    ProcessBatch {
        request_id: String,
        payload: ProcessBatchExecRequest,
    },
    #[serde(rename = "process.list")]
    ProcessList {
        request_id: String,
        payload: ProcessListRequest,
    },
    #[serde(rename = "process.read")]
    ProcessRead {
        request_id: String,
        payload: ProcessReadRequest,
    },
    #[serde(rename = "process.cancel")]
    ProcessCancel {
        request_id: String,
        payload: ProcessCancelRequest,
    },
    #[serde(rename = "event.list")]
    EventList {
        request_id: String,
        payload: EventListRequest,
    },
    #[serde(rename = "event.get")]
    EventGet {
        request_id: String,
        payload: EventGetRequest,
    },
    #[serde(rename = "event.mark")]
    EventMark {
        request_id: String,
        payload: EventMarkRequest,
    },
    #[serde(rename = "event.settle")]
    EventSettle {
        request_id: String,
        payload: EventSettleRequest,
    },
    #[serde(rename = "event.panel")]
    EventPanel { request_id: String },
    #[serde(rename = "tmux.listSessions")]
    TmuxListSessions { request_id: String },
    #[serde(rename = "tmux.listPanes")]
    TmuxListPanes {
        request_id: String,
        payload: TmuxListPanesRequest,
    },
    #[serde(rename = "tmux.capturePane")]
    TmuxCapturePane {
        request_id: String,
        payload: TmuxCapturePaneRequest,
    },
    #[serde(rename = "tmux.pasteText")]
    TmuxPasteText {
        request_id: String,
        payload: TmuxPasteTextRequest,
    },
    #[serde(rename = "tmux.exec")]
    TmuxExec {
        request_id: String,
        payload: TmuxExecRequest,
    },
    #[serde(rename = "tmux.createSession")]
    TmuxCreateSession {
        request_id: String,
        payload: TmuxCreateSessionRequest,
    },
    #[serde(rename = "tmux.closeSession")]
    TmuxCloseSession {
        request_id: String,
        payload: TmuxCloseSessionRequest,
    },
    #[serde(rename = "mcp.listServers")]
    McpListServers {
        request_id: String,
        #[serde(default, skip_serializing_if = "is_false")]
        suppress_event_panel: bool,
    },
    #[serde(rename = "mcp.listTools")]
    McpListTools {
        request_id: String,
        payload: McpListToolsRequest,
    },
    #[serde(rename = "mcp.callTool")]
    McpCallTool {
        request_id: String,
        payload: McpCallToolRequest,
    },
    #[serde(rename = "mcp.batch")]
    McpBatch {
        request_id: String,
        payload: McpBatchRequest,
    },
    #[serde(rename = "user.notify.deliver")]
    UserNotifyDeliver {
        request_id: String,
        payload: UserNotifyDeliveryRequest,
    },
    #[serde(rename = "room.diary.active")]
    RoomDiaryActive {
        request_id: String,
        payload: RoomDiaryActiveRequest,
    },
    #[serde(rename = "room.diary.read")]
    RoomDiaryRead {
        request_id: String,
        payload: RoomDiaryReadRequest,
    },
    #[serde(rename = "room.notebook.recent")]
    RoomNotebookRecent {
        request_id: String,
        payload: RoomNotebookRecentRequest,
    },
    #[serde(rename = "room.notebook.search")]
    RoomNotebookSearch {
        request_id: String,
        payload: RoomNotebookSearchRequest,
    },
    #[serde(rename = "room.notebook.read")]
    RoomNotebookRead {
        request_id: String,
        payload: RoomNotebookReadRequest,
    },
    #[serde(rename = "room.state.list")]
    RoomStateList {
        request_id: String,
        payload: RoomStateListRequest,
    },
    #[serde(rename = "room.state.read")]
    RoomStateRead {
        request_id: String,
        payload: RoomStateReadRequest,
    },
    #[serde(rename = "room.maintenance.status")]
    RoomMaintenanceStatus {
        request_id: String,
        payload: RoomMaintenanceStatusRequest,
    },
    #[serde(rename = "room.maintenance.submit")]
    RoomMaintenanceSubmit {
        request_id: String,
        payload: RoomMaintenanceSubmitRequest,
    },
    #[serde(rename = "room.bootstrap")]
    RoomBootstrap { request_id: String },
    #[serde(rename = "room.bootstrap.read")]
    RoomBootstrapRead {
        request_id: String,
        payload: BootstrapReadRequest,
    },
    #[serde(rename = "bootstrap")]
    Bootstrap { request_id: String },
    #[serde(rename = "bootstrap.read")]
    BootstrapRead {
        request_id: String,
        payload: BootstrapReadRequest,
    },
    #[serde(rename = "skills.list")]
    SkillsList { request_id: String },
    #[serde(rename = "skills.read")]
    SkillsRead {
        request_id: String,
        payload: SkillReadRequest,
    },
    #[serde(rename = "skills.search")]
    SkillsSearch {
        request_id: String,
        payload: SkillSearchRequest,
    },
    #[serde(rename = "skills.active")]
    SkillsActive { request_id: String },
    #[serde(rename = "skills.activate")]
    SkillsActivate {
        request_id: String,
        payload: SkillActivationRequest,
    },
    #[serde(rename = "skills.deactivate")]
    SkillsDeactivate {
        request_id: String,
        payload: SkillActivationRequest,
    },
    #[serde(rename = "skills.install")]
    SkillsInstall {
        request_id: String,
        payload: SkillInstallRequest,
    },
    #[serde(rename = "skills.install.get")]
    SkillsInstallGet {
        request_id: String,
        payload: SkillInstallGetRequest,
    },
    #[serde(rename = "skills.install.cancel")]
    SkillsInstallCancel {
        request_id: String,
        payload: SkillInstallCancelRequest,
    },
    #[serde(rename = "skills.run")]
    SkillsRun {
        request_id: String,
        payload: SkillRunRequest,
    },
}

impl HubCommand {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Exec { request_id, .. }
            | Self::ProcessBatch { request_id, .. }
            | Self::ProcessList { request_id, .. }
            | Self::ProcessRead { request_id, .. }
            | Self::ProcessCancel { request_id, .. }
            | Self::EventList { request_id, .. }
            | Self::EventGet { request_id, .. }
            | Self::EventMark { request_id, .. }
            | Self::EventSettle { request_id, .. }
            | Self::EventPanel { request_id }
            | Self::TmuxListSessions { request_id }
            | Self::TmuxListPanes { request_id, .. }
            | Self::TmuxCapturePane { request_id, .. }
            | Self::TmuxPasteText { request_id, .. }
            | Self::TmuxExec { request_id, .. }
            | Self::TmuxCreateSession { request_id, .. }
            | Self::TmuxCloseSession { request_id, .. }
            | Self::McpListServers { request_id, .. }
            | Self::McpListTools { request_id, .. }
            | Self::McpCallTool { request_id, .. }
            | Self::McpBatch { request_id, .. }
            | Self::UserNotifyDeliver { request_id, .. }
            | Self::RoomDiaryActive { request_id, .. }
            | Self::RoomDiaryRead { request_id, .. }
            | Self::RoomNotebookRecent { request_id, .. }
            | Self::RoomNotebookSearch { request_id, .. }
            | Self::RoomNotebookRead { request_id, .. }
            | Self::RoomStateList { request_id, .. }
            | Self::RoomStateRead { request_id, .. }
            | Self::RoomMaintenanceStatus { request_id, .. }
            | Self::RoomMaintenanceSubmit { request_id, .. }
            | Self::RoomBootstrap { request_id }
            | Self::RoomBootstrapRead { request_id, .. }
            | Self::Bootstrap { request_id }
            | Self::BootstrapRead { request_id, .. }
            | Self::SkillsList { request_id }
            | Self::SkillsRead { request_id, .. }
            | Self::SkillsSearch { request_id, .. }
            | Self::SkillsActive { request_id }
            | Self::SkillsActivate { request_id, .. }
            | Self::SkillsDeactivate { request_id, .. }
            | Self::SkillsInstall { request_id, .. }
            | Self::SkillsInstallGet { request_id, .. }
            | Self::SkillsInstallCancel { request_id, .. }
            | Self::SkillsRun { request_id, .. } => request_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubCommandEnvelope {
    pub event_id: String,
    pub run_id: String,
    pub request_id: String,
    pub command_hash: String,
    pub command: HubCommand,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    Hello {
        role: AgentRole,
        #[serde(rename = "bootGeneration")]
        boot_generation: String,
        #[serde(default, rename = "connectionMode")]
        connection_mode: AgentConnectionMode,
        #[serde(rename = "configSummary")]
        config_summary: SafeConfigSummary,
        #[serde(default, rename = "notificationChannels")]
        notification_channels: Vec<NotificationChannel>,
    },
    Heartbeat {
        #[serde(rename = "sentAt")]
        sent_at: DateTime<Utc>,
    },
    #[serde(rename = "process.update")]
    ProcessUpdate {
        process: ProcessInfo,
    },
    RunReport {
        report: Box<AgentRunReport>,
    },
    Response {
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "runId")]
        run_id: Option<String>,
        #[serde(rename = "requestId")]
        request_id: String,
        data: serde_json::Value,
        #[serde(
            default,
            skip_serializing_if = "Vec::is_empty",
            rename = "eventSources"
        )]
        event_sources: Vec<EventResponseDisposition>,
    },
    #[serde(rename = "event.sources")]
    EventSources {
        origin: EventOrigin,
        sources: Vec<EventSource>,
    },
    TransportAck {
        #[serde(rename = "eventId")]
        event_id: String,
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "commandHash")]
        command_hash: String,
    },
    TransportRunStatus {
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(rename = "requestId")]
        request_id: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    ConfirmationRequest {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "timeoutSeconds")]
        timeout_seconds: u64,
        payload: ConfirmationPayload,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HubMessage {
    HeartbeatAck {
        #[serde(rename = "sentAt")]
        sent_at: DateTime<Utc>,
        #[serde(rename = "receivedAt")]
        received_at: DateTime<Utc>,
    },
    ConfirmationResponse {
        #[serde(rename = "requestId")]
        request_id: String,
        decision: ConfirmationDecision,
        reason: String,
    },
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_hub_commands_keep_request_identity_and_agent_scope() {
        let list = HubCommand::EventList {
            request_id: "req-list".to_string(),
            payload: EventListRequest {
                agent_id: "agent-b".to_string(),
                status: None,
                severity: None,
                limit: Some(7),
                cursor: None,
            },
        };
        let get = HubCommand::EventGet {
            request_id: "req-get".to_string(),
            payload: EventGetRequest {
                agent_id: "agent-b".to_string(),
                event_id: "event-1".to_string(),
            },
        };
        let mark = HubCommand::EventMark {
            request_id: "req-mark".to_string(),
            payload: EventMarkRequest {
                agent_id: "agent-b".to_string(),
                event_ids: vec!["event-1".to_string()],
            },
        };
        let settle = HubCommand::EventSettle {
            request_id: "req-settle".to_string(),
            payload: EventSettleRequest {
                origin: EventOrigin {
                    run_id: "run-1".to_string(),
                    request_id: "req-create".to_string(),
                    command_hash: "hash-1".to_string(),
                },
                dispositions: vec![EventResponseDisposition {
                    source: super::super::events::EventSource {
                        kind: super::super::events::EventSourceKind::Process,
                        reference: "process-1".to_string(),
                    },
                    includes_terminal: false,
                }],
            },
        };
        let panel = HubCommand::EventPanel {
            request_id: "req-panel".to_string(),
        };

        for (command, expected_type, expected_request_id) in [
            (&list, "event.list", "req-list"),
            (&get, "event.get", "req-get"),
            (&mark, "event.mark", "req-mark"),
            (&settle, "event.settle", "req-settle"),
            (&panel, "event.panel", "req-panel"),
        ] {
            let value = serde_json::to_value(command).unwrap();
            assert_eq!(value["type"], expected_type);
            assert_eq!(value["requestId"], expected_request_id);
            assert_eq!(command.request_id(), expected_request_id);
        }
        let value = serde_json::to_value(&list).unwrap();
        assert_eq!(value["payload"]["agentId"], "agent-b");
        assert_eq!(value["payload"]["limit"], 7);
        let value = serde_json::to_value(&get).unwrap();
        assert_eq!(value["payload"]["eventId"], "event-1");
        let value = serde_json::to_value(&mark).unwrap();
        assert_eq!(value["payload"]["eventIds"], serde_json::json!(["event-1"]));
        assert!(serde_json::to_value(&panel)
            .unwrap()
            .get("payload")
            .is_none());
        let settle_value = serde_json::to_value(&settle).unwrap();
        assert_eq!(settle_value["payload"]["origin"]["runId"], "run-1");
        assert_eq!(
            settle_value["payload"]["dispositions"][0]["source"]["ref"],
            "process-1"
        );

        let response = AgentMessage::Response {
            run_id: Some("run-1".to_string()),
            request_id: "req-create".to_string(),
            data: serde_json::json!({ "state": "completed" }),
            event_sources: match &settle {
                HubCommand::EventSettle { payload, .. } => payload.dispositions.clone(),
                _ => unreachable!(),
            },
        };
        let response_value = serde_json::to_value(&response).unwrap();
        assert_eq!(response_value["eventSources"][0]["includesTerminal"], false);
        let legacy_response: AgentMessage = serde_json::from_value(serde_json::json!({
            "type": "response",
            "runId": "run-1",
            "requestId": "req-create",
            "data": {}
        }))
        .unwrap();
        assert!(matches!(
            legacy_response,
            AgentMessage::Response { event_sources, .. } if event_sources.is_empty()
        ));

        let sources = AgentMessage::EventSources {
            origin: match &settle {
                HubCommand::EventSettle { payload, .. } => payload.origin.clone(),
                _ => unreachable!(),
            },
            sources: match &settle {
                HubCommand::EventSettle { payload, .. } => payload
                    .dispositions
                    .iter()
                    .map(|disposition| disposition.source.clone())
                    .collect(),
                _ => unreachable!(),
            },
        };
        assert_eq!(
            serde_json::to_value(sources).unwrap()["type"],
            "event.sources"
        );
    }

    #[test]
    fn mcp_list_servers_panel_suppression_is_internal_and_legacy_compatible() {
        let normal = HubCommand::McpListServers {
            request_id: "req-normal".to_string(),
            suppress_event_panel: false,
        };
        let normal_value = serde_json::to_value(&normal).unwrap();
        assert_eq!(
            normal_value,
            serde_json::json!({ "type": "mcp.listServers", "requestId": "req-normal" })
        );

        let legacy: HubCommand = serde_json::from_value(serde_json::json!({
            "type": "mcp.listServers",
            "requestId": "req-legacy"
        }))
        .unwrap();
        assert!(matches!(
            legacy,
            HubCommand::McpListServers {
                suppress_event_panel: false,
                ..
            }
        ));

        let aggregate = HubCommand::McpListServers {
            request_id: "req-aggregate".to_string(),
            suppress_event_panel: true,
        };
        assert_eq!(
            serde_json::to_value(aggregate).unwrap(),
            serde_json::json!({
                "type": "mcp.listServers",
                "requestId": "req-aggregate",
                "suppressEventPanel": true
            })
        );
    }
}
