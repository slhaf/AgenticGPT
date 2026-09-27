use std::{collections::BTreeMap, fmt, path::PathBuf};

use anyhow::{anyhow, Result};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{acquire_config_mutation_lock, write_config_with_backup, Config};

#[derive(Subcommand)]
pub(crate) enum McpConfigCommand {
    List,
    Add {
        server_id: String,
        url: String,
        #[arg(long, default_value = "streamable-http")]
        transport: String,
        #[arg(long, default_value_t = true)]
        enabled: bool,
    },
    Remove {
        server_id: String,
    },
    Enable {
        server_id: String,
    },
    Disable {
        server_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpServerConfig {
    pub(crate) enabled: bool,
    pub(crate) transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) auth: Option<McpServerAuthConfig>,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum McpServerAuthConfig {
    Bearer { token: String },
}

impl fmt::Debug for McpServerAuthConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bearer { .. } => formatter
                .debug_struct("Bearer")
                .field("token", &"[REDACTED]")
                .finish(),
        }
    }
}

pub(crate) fn mutate_servers(config_path: PathBuf, command: McpConfigCommand) -> Result<()> {
    let _lock = acquire_config_mutation_lock(&config_path)?;
    let mut config = Config::load_or_default_locked(&config_path)?;
    match command {
        McpConfigCommand::List => {
            println!("{}", serde_json::to_string_pretty(&config.mcp_servers)?);
            return Ok(());
        }
        McpConfigCommand::Add {
            server_id,
            url,
            transport,
            enabled,
        } => {
            config.mcp_servers.insert(
                server_id,
                McpServerConfig {
                    enabled,
                    transport,
                    url: Some(url),
                    auth: None,
                },
            );
        }
        McpConfigCommand::Remove { server_id } => {
            config.mcp_servers.remove(&server_id);
        }
        McpConfigCommand::Enable { server_id } => {
            let server = config
                .mcp_servers
                .get_mut(&server_id)
                .ok_or_else(|| anyhow!("mcp server not found: {server_id}"))?;
            server.enabled = true;
        }
        McpConfigCommand::Disable { server_id } => {
            let server = config
                .mcp_servers
                .get_mut(&server_id)
                .ok_or_else(|| anyhow!("mcp server not found: {server_id}"))?;
            server.enabled = false;
        }
    }
    validate_server_configs(&config.mcp_servers)?;
    write_config_with_backup(&config_path, &config)
}

pub(crate) fn validate_server_configs(servers: &BTreeMap<String, McpServerConfig>) -> Result<()> {
    for (server_id, server) in servers {
        if server_id.is_empty()
            || server_id.len() > 64
            || server_id.trim() != server_id
            || !server_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(anyhow!("mcp_server_id_invalid: {server_id}"));
        }
        let raw_endpoint = server.url.as_deref().unwrap_or_default();
        let endpoint = raw_endpoint.trim();
        match server.transport.as_str() {
            "streamable-http" => {
                if endpoint.is_empty() {
                    return Err(anyhow!("mcp_server_url_missing: {server_id}"));
                }
                if endpoint != raw_endpoint {
                    return Err(anyhow!("mcp_server_url_invalid: {server_id}"));
                }
                let url = reqwest::Url::parse(endpoint)
                    .map_err(|_| anyhow!("mcp_server_url_invalid: {server_id}"))?;
                if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                    return Err(anyhow!("mcp_server_url_invalid: {server_id}"));
                }
                if let Some(McpServerAuthConfig::Bearer { token }) = &server.auth {
                    if token.is_empty()
                        || token.trim() != token
                        || token.chars().any(char::is_whitespace)
                        || token.chars().any(char::is_control)
                    {
                        return Err(anyhow!("mcp_server_auth_invalid: {server_id}"));
                    }
                }
            }
            "stdio" => {
                if endpoint.is_empty() {
                    return Err(anyhow!("mcp_server_command_missing: {server_id}"));
                }
                if endpoint != raw_endpoint || endpoint.chars().any(|character| character == '\0') {
                    return Err(anyhow!("mcp_server_command_invalid: {server_id}"));
                }
                if server.auth.is_some() {
                    return Err(anyhow!("mcp_server_auth_unsupported: {server_id}: stdio"));
                }
            }
            other => return Err(anyhow!("unsupported_mcp_transport: {server_id}: {other}")),
        }
    }
    Ok(())
}

pub(crate) fn server_config_revision(servers: &BTreeMap<String, McpServerConfig>) -> String {
    let bytes = serde_json::to_vec(servers).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}
