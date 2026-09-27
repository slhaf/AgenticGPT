use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::identity_config::{AgentConnectionMode, AgentRole, AgentRunReport, SafeConfigSummary};
use super::mcp::{McpBatchRequest, McpCallToolRequest, McpListToolsRequest};
use super::notification_tmux::{
    ConfirmationDecision, ConfirmationPayload, NotificationChannel, TmuxCapturePaneRequest,
    TmuxCloseSessionRequest, TmuxCreateSessionRequest, TmuxExecRequest, TmuxListPanesRequest,
    TmuxPasteTextRequest, UserNotifyDeliveryRequest,
};
use super::process_jobs::{
    BatchExecRequest, ExecRequest, JobCancelRequest, JobGetRequest, JobInfo, JobListRequest,
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
        payload: ExecRequest,
    },
    #[serde(rename = "process.batch")]
    ProcessBatch {
        request_id: String,
        payload: BatchExecRequest,
    },
    #[serde(rename = "job.list")]
    JobList {
        request_id: String,
        payload: JobListRequest,
    },
    #[serde(rename = "job.get")]
    JobGet {
        request_id: String,
        payload: JobGetRequest,
    },
    #[serde(rename = "job.cancel")]
    JobCancel {
        request_id: String,
        payload: JobCancelRequest,
    },
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
    McpListServers { request_id: String },
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
            | Self::JobList { request_id, .. }
            | Self::JobGet { request_id, .. }
            | Self::JobCancel { request_id, .. }
            | Self::TmuxListSessions { request_id }
            | Self::TmuxListPanes { request_id, .. }
            | Self::TmuxCapturePane { request_id, .. }
            | Self::TmuxPasteText { request_id, .. }
            | Self::TmuxExec { request_id, .. }
            | Self::TmuxCreateSession { request_id, .. }
            | Self::TmuxCloseSession { request_id, .. }
            | Self::McpListServers { request_id }
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
    JobUpdate {
        job: JobInfo,
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
