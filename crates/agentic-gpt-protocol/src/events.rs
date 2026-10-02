use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EventSeverity {
    Low,
    Medium,
    High,
}

impl EventSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EventStatus {
    Pending,
    Handled,
    Expired,
}

impl EventStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Handled => "handled",
            Self::Expired => "expired",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSourceKind {
    Process,
    SkillInstall,
    External,
}

impl EventSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::SkillInstall => "skill_install",
            Self::External => "external",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSource {
    pub kind: EventSourceKind,
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventOrigin {
    pub run_id: String,
    pub request_id: String,
    pub command_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventResponseDisposition {
    pub source: EventSource,
    pub includes_terminal: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSettleRequest {
    pub origin: EventOrigin,
    pub dispositions: Vec<EventResponseDisposition>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRecord {
    pub event_id: String,
    pub message: String,
    pub severity: EventSeverity,
    pub created_at: DateTime<Utc>,
    pub status: EventStatus,
    pub source: EventSource,
    pub shown_count: u32,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventListItem {
    pub event_id: String,
    pub summary: String,
    pub severity: EventSeverity,
    pub created_at: DateTime<Utc>,
    pub status: EventStatus,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventListRequest {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub status: Option<EventStatus>,
    #[serde(default)]
    pub severity: Option<EventSeverity>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventGetRequest {
    #[serde(default)]
    pub agent_id: String,
    pub event_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventMarkRequest {
    #[serde(default)]
    pub agent_id: String,
    pub event_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventListResponse {
    pub items: Vec<EventListItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventMarkResponse {
    pub handled_ids: Vec<String>,
    pub not_found_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPanel {
    pub current: String,
    pub new: Vec<BTreeMap<String, String>>,
}

impl Default for EventPanel {
    fn default() -> Self {
        Self {
            current: "low: 0 | medium: 0 | high: 0".to_string(),
            new: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventInjectRequest {
    pub message: String,
    #[serde(default)]
    pub severity: Option<EventSeverity>,
    #[serde(rename = "ref")]
    pub reference: String,
}
