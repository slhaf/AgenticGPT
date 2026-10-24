use std::fmt;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EmailPolicy {
    Allow,
    #[default]
    Confirm,
    Deny,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EmailTls {
    #[default]
    Implicit,
    Starttls,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EmailAccountConfig {
    pub(crate) imap_host: String,
    #[serde(default = "default_imap_port")]
    pub(crate) imap_port: u16,
    pub(crate) smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub(crate) smtp_port: u16,
    #[serde(default)]
    pub(crate) imap_tls: EmailTls,
    #[serde(default)]
    pub(crate) smtp_tls: EmailTls,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) from: String,
}

impl fmt::Debug for EmailAccountConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmailAccountConfig")
            .field("imap_host", &self.imap_host)
            .field("imap_port", &self.imap_port)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("imap_tls", &self.imap_tls)
            .field("smtp_tls", &self.smtp_tls)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("from", &self.from)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub(crate) struct EmailConfig {
    pub(crate) enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account: Option<EmailAccountConfig>,
    pub(crate) sync_interval_seconds: u64,
    pub(crate) retention_days: u64,
    pub(crate) max_storage_bytes: u64,
    pub(crate) send_policy: EmailPolicy,
    pub(crate) trash_policy: EmailPolicy,
}

fn default_imap_port() -> u16 {
    993
}
fn default_smtp_port() -> u16 {
    465
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account: None,
            sync_interval_seconds: 60,
            retention_days: 90,
            max_storage_bytes: 2 * 1024 * 1024 * 1024,
            send_policy: EmailPolicy::Confirm,
            trash_policy: EmailPolicy::Confirm,
        }
    }
}

impl EmailConfig {
    pub(crate) fn validate(&self) -> Result<()> {
        if !(10..=86400).contains(&self.sync_interval_seconds) {
            return Err(anyhow!("email_sync_interval_seconds_invalid"));
        }
        if !(1..=36500).contains(&self.retention_days) {
            return Err(anyhow!("email_retention_days_invalid"));
        }
        if self.max_storage_bytes < 1024 * 1024 {
            return Err(anyhow!("email_max_storage_bytes_invalid"));
        }
        if self.enabled && self.account.is_none() {
            return Err(anyhow!("email_account_required"));
        }
        if let Some(account) = &self.account {
            for host in [&account.imap_host, &account.smtp_host] {
                if host.is_empty()
                    || host.chars().any(|c| c.is_whitespace() || c.is_control())
                    || host.contains(['/', '@', '?', '#'])
                {
                    return Err(anyhow!("email_account_host_invalid"));
                }
            }
            if account.imap_port == 0 || account.smtp_port == 0 {
                return Err(anyhow!("email_account_port_invalid"));
            }
            if account.username.trim().is_empty() || account.username.chars().any(char::is_control)
            {
                return Err(anyhow!("email_account_username_invalid"));
            }
            let from = &account.from;
            if from.trim() != from
                || !from.contains('@')
                || from.chars().any(|c| c.is_whitespace() || c.is_control())
                || from.contains(['<', '>', ',', ';'])
            {
                return Err(anyhow!("email_account_from_invalid"));
            }
            super::validate_secret_reference(&account.password)
                .map_err(|_| anyhow!("email_account_password_reference_invalid"))?;
        }
        Ok(())
    }
}
