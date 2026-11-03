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

#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct ConfirmationReason(String);

impl ConfirmationReason {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ConfirmationReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() {
            return Err(serde::de::Error::custom(
                "acquireConfirmation reason must be at least 1 character",
            ));
        }
        if value.chars().count() > 100 {
            return Err(serde::de::Error::custom(
                "acquireConfirmation reason must be at most 100 characters",
            ));
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcquireConfirmation {
    pub reason: ConfirmationReason,
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
    reason: ConfirmationReason,
    #[serde(default)]
    method: Option<AcquireConfirmationMethod>,
}

pub fn deserialize_no_scope_confirmation<'de, D>(
    deserializer: D,
) -> Result<Option<AcquireConfirmation>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<AcquireConfirmationNoScope>::deserialize(deserializer)?;
    Ok(value.map(|value| AcquireConfirmation {
        reason: value.reason,
        method: value.method,
        read: Vec::new(),
        write: Vec::new(),
    }))
}

impl AcquireConfirmation {
    pub fn validate_scope(&self, allow_scope: bool) -> Result<(), String> {
        if !allow_scope && (!self.read.is_empty() || !self.write.is_empty()) {
            return Err(
                "acquireConfirmation read/write scopes are unsupported for this operation"
                    .to_string(),
            );
        }
        Ok(())
    }
}
