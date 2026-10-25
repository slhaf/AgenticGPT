use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub type ConfigPath = Vec<String>;

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_true(value: &bool) -> bool {
    *value
}

fn default_true() -> bool {
    true
}

fn present_value<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
pub enum ConfigChange {
    Set { path: ConfigPath, value: Value },
    Unset { path: ConfigPath },
    Add { path: ConfigPath, value: Value },
    Remove { path: ConfigPath, value: Value },
}

impl ConfigChange {
    pub fn path(&self) -> &[String] {
        match self {
            Self::Set { path, .. }
            | Self::Unset { path }
            | Self::Add { path, .. }
            | Self::Remove { path, .. } => path,
        }
    }

    pub fn value(&self) -> Option<&Value> {
        match self {
            Self::Set { value, .. } | Self::Add { value, .. } | Self::Remove { value, .. } => {
                Some(value)
            }
            Self::Unset { .. } => None,
        }
    }

    pub fn op_name(&self) -> &str {
        match self {
            Self::Set { .. } => "set",
            Self::Unset { .. } => "unset",
            Self::Add { .. } => "add",
            Self::Remove { .. } => "remove",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum ConfigManualDetail {
    Summary,
    #[default]
    Full,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigManualRequest {
    /// 可选章节标识；省略且不检索时返回章节目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// 可选检索文字；与 section 同用时限定章节。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// 文档详细程度，默认 full。
    #[serde(default)]
    pub detail: ConfigManualDetail,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigGetRequest {
    /// 配置路径段数组；含点的事件名和服务器 ID 各占一个段，不支持数组下标。
    pub paths: Vec<ConfigPath>,
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigUpdateRequest {
    /// 按序作用于最新持久配置；整批最终校验后保存一次。
    pub changes: Vec<ConfigChange>,
    /// true 只预览本次修改；后续实写重新以届时最新配置为基础。
    #[serde(default)]
    pub dry_run: bool,
}

impl std::fmt::Debug for ConfigUpdateRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfigUpdateRequest")
            .field("change_count", &self.changes.len())
            .field("dry_run", &self.dry_run)
            .finish()
    }
}

/// Hub 路由字段与 Agent 原生请求共用的薄封套。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
#[serde(rename_all = "camelCase")]
pub struct ConfigAgentRequest<T> {
    /// Hub 中已启用的目标 Agent；配置不属于 Hub 服务端。
    pub agent_id: String,
    #[serde(flatten)]
    pub request: T,
}

// Deserialize the routing field separately so unknown request fields are still
// rejected by T. Serde's flatten buffering must not swallow that validation.
impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for ConfigAgentRequest<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut object = serde_json::Map::<String, Value>::deserialize(deserializer)?;
        let agent_id = object
            .remove("agentId")
            .ok_or_else(|| serde::de::Error::missing_field("agentId"))?;
        let agent_id = serde_json::from_value(agent_id).map_err(serde::de::Error::custom)?;
        let request =
            serde_json::from_value(Value::Object(object)).map_err(serde::de::Error::custom)?;
        Ok(Self { agent_id, request })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ConfigApplyStatus {
    Applied,
    RestartRequired,
    Applying,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ConfigApplyEffect {
    LiveUpdate,
    ComponentReconfigure,
    RestartRequired,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigApiError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<ConfigPath>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigItem {
    pub path: ConfigPath,
    pub value: Value,
    pub explicit: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub exists: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub redacted: bool,
    #[serde(
        default,
        deserialize_with = "present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub applied_value: Option<Value>,
    pub apply_status: ConfigApplyStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ConfigApiError>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigPreviewItem {
    pub path: ConfigPath,
    pub before: Value,
    pub after: Value,
    pub expected_effect: ConfigApplyEffect,
    #[serde(default, skip_serializing_if = "is_false")]
    pub redacted: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum ConfigUpdateChange {
    Applied(ConfigItem),
    Preview(ConfigPreviewItem),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigManualResponse {
    pub content: String,
    pub matches: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigGetResponse {
    pub items: Vec<ConfigItem>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigUpdateResponse {
    pub saved: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dry_run: bool,
    pub changes: Vec<ConfigUpdateChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ConfigApiError>,
}
