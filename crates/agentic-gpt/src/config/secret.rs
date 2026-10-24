use std::fmt;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SecretPolicy {
    Allow,
    #[default]
    Confirm,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub(crate) struct RoomSecretConfig {
    pub(crate) enabled: bool,
    pub(crate) server_url: String,
    pub(crate) account: String,
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    pub(crate) master_password: String,
    pub(crate) get_policy: SecretPolicy,
    pub(crate) upsert_policy: SecretPolicy,
    pub(crate) delete_policy: SecretPolicy,
    pub(crate) sync_wait_timeout_ms: u64,
}

impl Default for RoomSecretConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_url: String::new(),
            account: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            master_password: String::new(),
            get_policy: SecretPolicy::Allow,
            upsert_policy: SecretPolicy::Confirm,
            delete_policy: SecretPolicy::Confirm,
            sync_wait_timeout_ms: 3000,
        }
    }
}

impl fmt::Debug for RoomSecretConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomSecretConfig")
            .field("enabled", &self.enabled)
            .field("server_url", &"[REDACTED]")
            .field("account", &"[REDACTED]")
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .field("master_password", &"[REDACTED]")
            .field("get_policy", &self.get_policy)
            .field("upsert_policy", &self.upsert_policy)
            .field("delete_policy", &self.delete_policy)
            .field("sync_wait_timeout_ms", &self.sync_wait_timeout_ms)
            .finish()
    }
}

impl RoomSecretConfig {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.sync_wait_timeout_ms > 30_000 {
            return Err(anyhow!("room_secret_sync_wait_timeout_ms_invalid"));
        }
        if self.enabled || !self.server_url.is_empty() {
            let url = url::Url::parse(&self.server_url)
                .map_err(|_| anyhow!("room_secret_server_url_invalid"))?;
            if self.server_url.len() > 4096
                || self
                    .server_url
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
                || !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(anyhow!("room_secret_server_url_invalid"));
            }
        }
        if (self.enabled && self.account.is_empty())
            || self.account.len() > 1024
            || self
                .account
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(anyhow!("room_secret_account_invalid"));
        }
        for (reference, code) in [
            (&self.client_id, "room_secret_client_id_reference_invalid"),
            (
                &self.client_secret,
                "room_secret_client_secret_reference_invalid",
            ),
            (
                &self.master_password,
                "room_secret_master_password_reference_invalid",
            ),
        ] {
            if self.enabled || !reference.is_empty() {
                if reference.len() > 4096 || reference.chars().any(char::is_control) {
                    return Err(anyhow!(code));
                }
                super::validate_secret_reference(reference).map_err(|_| anyhow!(code))?;
            }
        }
        Ok(())
    }
}
