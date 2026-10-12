use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AcquireConfirmationMethod {
    Ntfy,
    Freedesktop,
}

impl AcquireConfirmationMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ntfy => "ntfy",
            Self::Freedesktop => "freedesktop",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcquireConfirmation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquire: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<AcquireConfirmationMethod>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcquireConfirmationNoScope {
    #[serde(default)]
    acquire: Option<bool>,
    #[serde(default)]
    method: Option<AcquireConfirmationMethod>,
}

pub fn deserialize_no_scope_confirmation<'de, D>(
    deserializer: D,
) -> Result<Option<AcquireConfirmation>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<AcquireConfirmationNoScope>::deserialize(deserializer).map(|value| {
        value.map(|value| AcquireConfirmation {
            acquire: value.acquire,
            method: value.method,
            ..AcquireConfirmation::default()
        })
    })
}

impl AcquireConfirmation {
    pub fn effective_acquire(&self, default: bool) -> bool {
        self.acquire.unwrap_or(default)
    }

    pub fn validate_scope(&self, allow_scope: bool) -> Result<(), String> {
        if self.read.is_empty() && self.write.is_empty() {
            return Ok(());
        }
        if !allow_scope {
            return Err(
                "acquireConfirmation read/write scopes are unsupported for this operation"
                    .to_string(),
            );
        }
        if self.acquire != Some(true) {
            return Err("acquireConfirmation read/write scopes require acquire=true".to_string());
        }
        Ok(())
    }
}
