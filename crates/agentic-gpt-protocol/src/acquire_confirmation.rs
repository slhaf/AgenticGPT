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

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquireConfirmation {
    pub reason: String,
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
    reason: String,
    #[serde(default)]
    method: Option<AcquireConfirmationMethod>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcquireConfirmationInput {
    reason: String,
    #[serde(default)]
    method: Option<AcquireConfirmationMethod>,
    #[serde(default)]
    read: Vec<String>,
    #[serde(default)]
    write: Vec<String>,
}

impl<'de> Deserialize<'de> for AcquireConfirmation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = AcquireConfirmationInput::deserialize(deserializer)?;
        let confirmation = Self {
            reason: value.reason,
            method: value.method,
            read: value.read,
            write: value.write,
        };
        confirmation.validate().map_err(serde::de::Error::custom)?;
        Ok(confirmation)
    }
}

pub fn deserialize_no_scope_confirmation<'de, D>(
    deserializer: D,
) -> Result<Option<AcquireConfirmation>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<AcquireConfirmationNoScope>::deserialize(deserializer)?
        .map(|value| {
            let confirmation = AcquireConfirmation {
                reason: value.reason,
                method: value.method,
                ..AcquireConfirmation::default()
            };
            confirmation.validate().map_err(serde::de::Error::custom)?;
            Ok(confirmation)
        })
        .transpose()
}

impl AcquireConfirmation {
    pub fn validate(&self) -> Result<(), String> {
        if self.reason.trim().is_empty() {
            return Err("acquireConfirmation reason must not be blank".to_string());
        }
        if self.reason.chars().count() > 100 {
            return Err("acquireConfirmation reason must be at most 100 characters".to_string());
        }
        Ok(())
    }

    pub fn validate_scope(&self, allow_scope: bool) -> Result<(), String> {
        self.validate()?;
        if !allow_scope && (!self.read.is_empty() || !self.write.is_empty()) {
            return Err(
                "acquireConfirmation read/write scopes are unsupported for this operation"
                    .to_string(),
            );
        }
        Ok(())
    }
}
