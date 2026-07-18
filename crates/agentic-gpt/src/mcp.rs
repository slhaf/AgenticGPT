use std::{env, path::PathBuf};

use agentic_gpt_protocol::{McpCallToolRequest, McpListToolsRequest, McpServerSummary};
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use clap::Subcommand;
use rmcp::{
    model::{CallToolRequestParams, ClientInfo, JsonObject},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, ConfigureCommandExt,
        StreamableHttpClientTransport, TokioChildProcess,
    },
    ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::Instant;

use crate::{
    audit::{write_audit, AuditRecord},
    config::{write_config_with_backup, Config},
    confirmation, AppState,
};

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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpServerConfig {
    pub(crate) enabled: bool,
    pub(crate) transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
}

pub(crate) fn mutate_servers(config_path: PathBuf, command: McpConfigCommand) -> Result<()> {
    let mut config = Config::load_or_default(&config_path)?;
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
    write_config_with_backup(&config_path, &config)
}

pub(crate) async fn list_servers(state: &AppState) -> Value {
    let config = state.config.read().await;
    let servers = config
        .mcp_servers
        .iter()
        .map(|(id, server)| McpServerSummary {
            id: id.clone(),
            enabled: server.enabled,
            transport: server.transport.clone(),
            url: server.url.clone(),
        })
        .collect::<Vec<_>>();
    json!({ "servers": servers })
}

pub(crate) async fn list_tools(state: &AppState, payload: McpListToolsRequest) -> Result<Value> {
    let server = server_config(state, &payload.server_id).await?;
    let client = client(&server).await?;
    let tools = client.list_all_tools().await?;
    let _ = client.cancel().await;
    Ok(json!({ "tools": tools }))
}

pub(crate) async fn call_tool(state: &AppState, payload: McpCallToolRequest) -> Result<Value> {
    let started = Instant::now();
    let config = state.config.read().await.clone();
    let server = match server_config(state, &payload.server_id).await {
        Ok(server) => server,
        Err(error) => {
            audit_mcp_call(
                &config,
                &payload,
                "Deny",
                None,
                None,
                Some(error.to_string()),
                started.elapsed().as_millis(),
            );
            return Err(error);
        }
    };

    let authorization = confirmation::authorize_mcp_tool_call(
        state,
        &payload.server_id,
        &payload.tool_name,
        &payload.arguments,
    )
    .await;
    if !mcp_authorization_allows(&authorization) {
        audit_mcp_call(
            &config,
            &payload,
            "Confirm",
            Some(authorization.clone()),
            None,
            Some(format!("mcp_tool_call_rejected: {authorization}")),
            started.elapsed().as_millis(),
        );
        return Err(anyhow!("mcp_tool_call_rejected: {authorization}"));
    }

    let arguments = match tool_arguments(payload.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => {
            audit_mcp_call(
                &config,
                &payload,
                "Deny",
                Some(authorization),
                None,
                Some(error.to_string()),
                started.elapsed().as_millis(),
            );
            return Err(error);
        }
    };
    let client = match client(&server).await {
        Ok(client) => client,
        Err(error) => {
            audit_mcp_call(
                &config,
                &payload,
                "Confirm",
                Some(authorization),
                None,
                Some(error.to_string()),
                started.elapsed().as_millis(),
            );
            return Err(error);
        }
    };
    let result = client
        .call_tool(CallToolRequestParams::new(payload.tool_name.clone()).with_arguments(arguments))
        .await;
    let _ = client.cancel().await;
    match result {
        Ok(result) => {
            let value = serde_json::to_value(result)?;
            audit_mcp_call(
                &config,
                &payload,
                if authorization == "temporary_mcp_allow" {
                    "AllowTemporary"
                } else {
                    "Confirm"
                },
                Some(authorization),
                Some(0),
                None,
                started.elapsed().as_millis(),
            );
            Ok(value)
        }
        Err(error) => {
            audit_mcp_call(
                &config,
                &payload,
                "Confirm",
                Some(authorization),
                None,
                Some(error.to_string()),
                started.elapsed().as_millis(),
            );
            Err(error.into())
        }
    }
}

fn mcp_authorization_allows(value: &str) -> bool {
    matches!(
        value,
        "allow_once" | "allow_mcp_server_15m" | "allow_mcp_server_30m" | "temporary_mcp_allow"
    )
}

fn audit_mcp_call(
    config: &Config,
    payload: &McpCallToolRequest,
    policy_decision: &str,
    confirmation_result: Option<String>,
    exit_code: Option<i32>,
    reject_reason: Option<String>,
    duration_ms: u128,
) {
    let arguments = serde_json::to_string(&payload.arguments).unwrap_or_else(|_| "{}".to_string());
    let truncated = arguments.chars().count() > 1000;
    let arguments = truncate_chars(&arguments, 1000);
    let _ = write_audit(
        config,
        AuditRecord {
            task_id: None,
            session_id: None,
            time: Utc::now(),
            program: "mcp.callTool".to_string(),
            args: vec![
                payload.server_id.clone(),
                payload.tool_name.clone(),
                arguments,
            ],
            working_directory: None,
            need_confirm: confirmation_result.as_deref() != Some("temporary_mcp_allow"),
            policy_decision: policy_decision.to_string(),
            confirmation_result,
            exit_code,
            duration_ms,
            truncated,
            request_source: "hub:mcp".to_string(),
            reject_reason,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
        },
    );
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

async fn server_config(state: &AppState, server_id: &str) -> Result<McpServerConfig> {
    let config = state.config.read().await;
    let server = config
        .mcp_servers
        .get(server_id)
        .cloned()
        .ok_or_else(|| anyhow!("mcp_server_not_found: {server_id}"))?;
    if !server.enabled {
        return Err(anyhow!("mcp_server_disabled: {server_id}"));
    }
    match server.transport.as_str() {
        "streamable-http" => {
            if server.url.as_deref().unwrap_or_default().trim().is_empty() {
                return Err(anyhow!("mcp_server_url_missing: {server_id}"));
            }
        }
        "stdio" => {
            if server.url.as_deref().unwrap_or_default().trim().is_empty() {
                return Err(anyhow!("mcp_server_command_missing: {server_id}"));
            }
        }
        other => return Err(anyhow!("unsupported_mcp_transport: {other}")),
    }
    Ok(server)
}

async fn client(
    server: &McpServerConfig,
) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>> {
    match server.transport.as_str() {
        "streamable-http" => {
            let url = server.url.clone().context("mcp_server_url_missing")?;
            let transport = StreamableHttpClientTransport::from_config(
                StreamableHttpClientTransportConfig::with_uri(url),
            );
            Ok(ClientInfo::default().serve(transport).await?)
        }
        "stdio" => {
            let command = server.url.clone().context("mcp_server_command_missing")?;
            let transport =
                TokioChildProcess::new(tokio::process::Command::new("sh").configure(|cmd| {
                    cmd.arg("-lc").arg(command);
                    if let Ok(home) = env::var("HOME") {
                        let path = env::var("PATH").unwrap_or_default();
                        cmd.env("PATH", format!("{home}/.local/bin:{path}"));
                    }
                }))?;
            Ok(ClientInfo::default().serve(transport).await?)
        }
        other => Err(anyhow!("unsupported_mcp_transport: {other}")),
    }
}

fn tool_arguments(arguments: Value) -> Result<JsonObject> {
    match arguments {
        Value::Null => Ok(JsonObject::new()),
        Value::Object(map) => Ok(map),
        other => Err(anyhow!("mcp_tool_arguments_must_be_object: {other}")),
    }
}
