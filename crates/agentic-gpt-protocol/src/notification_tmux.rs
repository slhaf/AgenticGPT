use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationChannel {
    pub key: String,
    pub display_name: String,
    pub available: bool,
    pub kind: String,
    pub supports_actions: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserNotifySendRequest {
    pub channel_key: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserNotifySendResponse {
    pub channel_key: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserNotifyDeliveryRequest {
    pub channel_key: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserNotifyDeliveryResponse {
    pub channel_key: String,
    pub delivered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationPayload {
    pub program: String,
    pub args: Vec<String>,
    pub command_preview: String,
    pub risk_level: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxListPanesRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxCapturePaneRequest {
    pub target: String,
    #[serde(default = "default_tmux_capture_lines")]
    pub lines: u32,
}

fn default_tmux_capture_lines() -> u32 {
    160
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxPasteTextRequest {
    pub target: String,
    pub text: String,
    #[serde(default)]
    pub submit: bool,
    #[serde(default = "default_true")]
    pub need_confirm: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxExecRequest {
    pub target: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub need_confirm: bool,
    #[serde(default = "default_tmux_exec_wait_ms")]
    pub wait_ms: u64,
    #[serde(default = "default_tmux_exec_capture_lines")]
    pub capture_lines: u32,
}

fn default_tmux_exec_wait_ms() -> u64 {
    300
}

fn default_tmux_exec_capture_lines() -> u32 {
    120
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxCreateSessionRequest {
    pub name: String,
    pub cwd: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxCloseSessionRequest {
    pub name: String,
    #[serde(default = "default_true")]
    pub need_confirm: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationDecision {
    AllowOnce,
    #[serde(rename = "allow_mcp_server_15m")]
    AllowMcpServer15m,
    #[serde(rename = "allow_mcp_server_30m")]
    AllowMcpServer30m,
    Deny,
    Timeout,
    ProviderUnavailable,
    CallbackTokenInvalid,
    Expired,
}
