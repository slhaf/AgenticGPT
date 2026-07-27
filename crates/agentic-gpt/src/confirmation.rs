use agentic_gpt_protocol::{AgentMessage, ConfirmationDecision, ConfirmationPayload};
use chrono::{DateTime, Utc};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout, Duration};
use uuid::Uuid;

use crate::{
    config::{
        confirmation_language_is_zh, Config, ConfirmationChannel, ConfirmationProviderConfig,
    },
    exec::PreparedBatchElement,
    hub,
    utils::{
        command_preview, log_info, log_warn, mcp_tool_command_preview, risk_level,
        risky_file_mutation, truncate_chars, CONFIRM_TIMEOUT_SECS,
    },
    AppState,
};

#[derive(Clone, Debug)]
pub(crate) struct TemporaryMcpAllow {
    pub(crate) agent_id: String,
    pub(crate) server_id: String,
    pub(crate) expires_at: DateTime<Utc>,
}

pub(crate) fn batch_confirmation_preview(
    config: &Config,
    needs_confirmation: &[PreparedBatchElement],
    all_elements: &[PreparedBatchElement],
) -> String {
    let zh = confirmation_language_is_zh(config);
    let mut lines = vec![if zh {
        format!(
            "该批次共有 {} 条命令，其中 {} 条需要确认：",
            all_elements.len(),
            needs_confirmation.len()
        )
    } else {
        format!(
            "Batch requires confirmation for {} of {} commands:",
            needs_confirmation.len(),
            all_elements.len()
        )
    }];
    for element in needs_confirmation.iter().take(8) {
        let cwd = element
            .working_directory
            .as_ref()
            .map(|directory| {
                if zh {
                    format!("（工作目录：{directory}）")
                } else {
                    format!(" (cwd: {directory})")
                }
            })
            .unwrap_or_default();
        lines.push(format!(
            "[{}] {}{}",
            element.index,
            command_preview(&element.program, &element.args),
            cwd
        ));
    }
    if needs_confirmation.len() > 8 {
        lines.push(if zh {
            format!(
                "……另外还有 {} 条需要确认的命令",
                needs_confirmation.len() - 8
            )
        } else {
            format!(
                "... and {} more commands requiring confirmation",
                needs_confirmation.len() - 8
            )
        });
    }
    let other_count = all_elements.len().saturating_sub(needs_confirmation.len());
    if other_count > 0 {
        lines.push(if zh {
            format!("另外包含 {other_count} 条不需要确认的命令。")
        } else {
            format!("Also included: {other_count} command(s) that do not require confirmation.")
        });
    }
    lines.push(if zh {
        "是否允许整个批次执行一次？".to_string()
    } else {
        "Allow the entire batch once?".to_string()
    });
    lines.join("\n")
}

pub(crate) async fn request_batch_confirmation(
    state: &AppState,
    config: &Config,
    confirm_method: Option<&str>,
    needs_confirmation: &[PreparedBatchElement],
    all_elements: &[PreparedBatchElement],
) -> String {
    let channels = confirmation_channels(config, confirm_method);
    let preview = batch_confirmation_preview(config, needs_confirmation, all_elements);
    for channel in channels {
        let result = match channel {
            ConfirmationChannel::Freedesktop => {
                request_freedesktop_batch_confirmation(config, &preview, needs_confirmation).await
            }
            ConfirmationChannel::Ntfy => {
                request_hub_batch_confirmation(state, config, &preview, needs_confirmation).await
            }
        };
        if result != "confirmation_provider_unavailable" {
            return result;
        }
    }
    "confirmation_provider_unavailable".to_string()
}

fn confirmation_channels(
    config: &Config,
    confirm_method: Option<&str>,
) -> Vec<ConfirmationChannel> {
    let override_value = confirm_method.filter(|method| !method.trim().is_empty());
    match override_value {
        None => config.confirmation_provider.channels.clone(),
        Some("default") => config.confirmation_provider.channels.clone(),
        Some(value) => ConfirmationProviderConfig::from_legacy(value)
            .map(|provider| provider.channels)
            .unwrap_or_default(),
    }
}

async fn request_freedesktop_batch_confirmation(
    config: &Config,
    preview: &str,
    needs_confirmation: &[PreparedBatchElement],
) -> String {
    let supports_actions = tokio::task::spawn_blocking(|| {
        notify_rust::get_capabilities()
            .map(|capabilities| {
                capabilities
                    .iter()
                    .any(|capability| capability == "actions")
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !supports_actions {
        return "confirmation_provider_unavailable".to_string();
    }
    let has_risky_file_mutation = needs_confirmation
        .iter()
        .any(|element| risky_file_mutation(&element.program));
    let zh = confirmation_language_is_zh(config);
    let warning = if !config.sandbox.enabled && has_risky_file_mutation {
        if zh {
            "\n警告：bubblewrap 未启用；该批次包含文件变更命令，对宿主机的可见范围更大。"
        } else {
            "\nWARNING: bubblewrap is disabled; this batch includes file mutation commands with broader host visibility."
        }
    } else {
        ""
    };
    let body = format!("{preview}{warning}");
    let provider = notify_rust::Notification::new()
        .summary(if zh {
            "Agentic GPT 批量确认"
        } else {
            "Agentic GPT batch confirmation"
        })
        .body(&body)
        .action(
            "allow_once",
            if zh {
                "允许本批次"
            } else {
                "Allow batch once"
            },
        )
        .action("deny", if zh { "拒绝本批次" } else { "Deny batch" })
        .timeout((CONFIRM_TIMEOUT_SECS * 1000) as i32)
        .show();
    match provider {
        Ok(handle) => {
            let action = tokio::task::spawn_blocking(move || {
                let mut selected = "timeout".to_string();
                handle.wait_for_action(|action| selected = action.to_string());
                selected
            })
            .await
            .unwrap_or_else(|_| "timeout".to_string());
            if action == "allow_once" {
                action
            } else {
                "deny".to_string()
            }
        }
        Err(_) => "confirmation_provider_unavailable".to_string(),
    }
}

async fn request_hub_batch_confirmation(
    state: &AppState,
    config: &Config,
    preview: &str,
    needs_confirmation: &[PreparedBatchElement],
) -> String {
    let risk = if needs_confirmation
        .iter()
        .any(|element| risk_level(&element.program) == "HIGH")
    {
        "HIGH"
    } else {
        "MEDIUM"
    };
    let payload = ConfirmationPayload {
        program: "process.batch".to_string(),
        args: Vec::new(),
        command_preview: truncate_chars(preview, 1000),
        risk_level: risk.to_string(),
        reason: if confirmation_language_is_zh(config) {
            "批量命令中包含匹配确认策略的命令".to_string()
        } else {
            "Batch contains command(s) matching confirm policy".to_string()
        },
        kind: Some("process.batch".to_string()),
        server_id: None,
        tool_name: None,
    };
    request_hub_confirmation_payload(state, payload).await
}

pub(crate) async fn request_confirmation(
    state: &AppState,
    config: &Config,
    confirm_method: Option<&str>,
    program: &str,
    args: &[String],
) -> String {
    for channel in confirmation_channels(config, confirm_method) {
        let result = match channel {
            ConfirmationChannel::Freedesktop => {
                request_freedesktop_confirmation(config, program, args).await
            }
            ConfirmationChannel::Ntfy => {
                request_hub_confirmation(state, config, program, args).await
            }
        };
        if result != "confirmation_provider_unavailable" {
            return result;
        }
    }
    "confirmation_provider_unavailable".to_string()
}

/// Confirmation used by an asynchronously registered Job. Hub-backed
/// requests remove their pending sender when the Job is cancelled, so a
/// cancelled `waiting_confirmation` Job cannot leak a durable callback entry.
pub(crate) async fn request_confirmation_cancellable(
    state: &AppState,
    config: &Config,
    confirm_method: Option<&str>,
    program: &str,
    args: &[String],
    cancel_requested: Arc<AtomicBool>,
) -> String {
    for channel in confirmation_channels(config, confirm_method) {
        let result = match channel {
            ConfirmationChannel::Freedesktop => tokio::select! {
                result = request_freedesktop_confirmation(config, program, args) => result,
                _ = wait_for_cancellation(cancel_requested.clone()) => "cancelled".to_string(),
            },
            ConfirmationChannel::Ntfy => {
                request_hub_confirmation_cancellable(
                    state,
                    config,
                    program,
                    args,
                    cancel_requested.clone(),
                )
                .await
            }
        };
        if result != "confirmation_provider_unavailable" {
            return result;
        }
    }
    "confirmation_provider_unavailable".to_string()
}

async fn request_freedesktop_confirmation(
    config: &Config,
    program: &str,
    args: &[String],
) -> String {
    let supports_actions = tokio::task::spawn_blocking(|| {
        notify_rust::get_capabilities()
            .map(|capabilities| {
                capabilities
                    .iter()
                    .any(|capability| capability == "actions")
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !supports_actions {
        return "confirmation_provider_unavailable".to_string();
    }
    let zh = confirmation_language_is_zh(config);
    let warning = if !config.sandbox.enabled && risky_file_mutation(program) {
        if zh {
            "\n警告：bubblewrap 未启用；该文件变更命令对宿主机的可见范围更大。"
        } else {
            "\nWARNING: bubblewrap is disabled; this file mutation command has broader host visibility."
        }
    } else {
        ""
    };
    let body = format!(
        "{}{}{}",
        command_preview(program, args),
        warning,
        if zh {
            "\n是否允许本次执行？"
        } else {
            "\nAllow once?"
        }
    );
    let provider = notify_rust::Notification::new()
        .summary(if zh {
            "Agentic GPT 确认"
        } else {
            "Agentic GPT confirmation"
        })
        .body(&body)
        .action("allow_once", if zh { "允许本次" } else { "Allow once" })
        .action("deny", if zh { "拒绝" } else { "Deny" })
        .timeout((CONFIRM_TIMEOUT_SECS * 1000) as i32)
        .show();
    match provider {
        Ok(handle) => {
            let action = tokio::task::spawn_blocking(move || {
                let mut selected = "timeout".to_string();
                handle.wait_for_action(|action| selected = action.to_string());
                selected
            })
            .await
            .unwrap_or_else(|_| "timeout".to_string());
            if action == "allow_once" {
                action
            } else {
                "deny".to_string()
            }
        }
        Err(_) => "confirmation_provider_unavailable".to_string(),
    }
}

async fn request_hub_confirmation(
    state: &AppState,
    _config: &Config,
    program: &str,
    args: &[String],
) -> String {
    let payload = ConfirmationPayload {
        program: program.to_string(),
        args: args.to_vec(),
        command_preview: truncate_chars(&command_preview(program, args), 1000),
        risk_level: risk_level(program),
        reason: if confirmation_language_is_zh(_config) {
            format!("命令匹配确认策略：{program}")
        } else {
            format!("Command matched confirm policy: {program}")
        },
        kind: None,
        server_id: None,
        tool_name: None,
    };
    request_hub_confirmation_payload(state, payload).await
}

async fn request_hub_confirmation_cancellable(
    state: &AppState,
    config: &Config,
    program: &str,
    args: &[String],
    cancel_requested: Arc<AtomicBool>,
) -> String {
    let payload = ConfirmationPayload {
        program: program.to_string(),
        args: args.to_vec(),
        command_preview: truncate_chars(&command_preview(program, args), 1000),
        risk_level: risk_level(program),
        reason: if confirmation_language_is_zh(config) {
            format!("命令匹配确认策略：{program}")
        } else {
            format!("Command matched confirmation policy: {program}")
        },
        kind: None,
        server_id: None,
        tool_name: None,
    };
    request_hub_confirmation_payload_cancellable(state, payload, Some(cancel_requested)).await
}

async fn request_hub_confirmation_payload(
    state: &AppState,
    payload: ConfirmationPayload,
) -> String {
    request_hub_confirmation_payload_cancellable(state, payload, None).await
}

async fn request_hub_confirmation_payload_cancellable(
    state: &AppState,
    payload: ConfirmationPayload,
    cancel_requested: Option<Arc<AtomicBool>>,
) -> String {
    if cancel_requested
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Acquire))
    {
        return "cancelled".to_string();
    }
    let request_id = format!("confirm_req_{}", Uuid::new_v4().simple());
    let (tx, rx) = oneshot::channel();
    state
        .pending_confirmations
        .lock()
        .await
        .insert(request_id.clone(), tx);
    let config = state.config.read().await.clone();
    let message = AgentMessage::ConfirmationRequest {
        request_id: request_id.clone(),
        agent_id: config.agent_id,
        timeout_seconds: CONFIRM_TIMEOUT_SECS,
        payload,
    };
    if let Err(error) = hub::send_agent_message(state, message).await {
        state.pending_confirmations.lock().await.remove(&request_id);
        log_warn(format!("hub confirmation unavailable: {error}"));
        return "provider_unavailable".to_string();
    }
    log_info(format!(
        "hub confirmation requested; requestId={request_id}"
    ));
    let result = if let Some(cancel_requested) = cancel_requested {
        tokio::select! {
            result = timeout(Duration::from_secs(CONFIRM_TIMEOUT_SECS + 5), rx) => {
                match result {
                    Ok(Ok(decision)) => decision,
                    _ => "timeout".to_string(),
                }
            }
            _ = wait_for_cancellation(cancel_requested) => "cancelled".to_string(),
        }
    } else {
        match timeout(Duration::from_secs(CONFIRM_TIMEOUT_SECS + 5), rx).await {
            Ok(Ok(decision)) => decision,
            _ => "timeout".to_string(),
        }
    };
    state.pending_confirmations.lock().await.remove(&request_id);
    result
}

async fn wait_for_cancellation(cancel_requested: Arc<AtomicBool>) {
    while !cancel_requested.load(Ordering::Acquire) {
        sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) async fn fail_pending_confirmations(state: &AppState, reason: &str) {
    let pending = state
        .pending_confirmations
        .lock()
        .await
        .drain()
        .map(|(_, sender)| sender)
        .collect::<Vec<_>>();
    for sender in pending {
        let _ = sender.send(reason.to_string());
    }
}

pub(crate) async fn authorize_mcp_tool_call_cancellable(
    state: &AppState,
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    cancel_requested: Arc<AtomicBool>,
) -> String {
    if temporary_mcp_allowed(state, server_id).await {
        return "temporary_mcp_allow".to_string();
    }
    if cancel_requested.load(Ordering::Acquire) {
        return "cancelled".to_string();
    }

    let config = state.config.read().await.clone();
    let decision = request_mcp_tool_confirmation_cancellable(
        state,
        &config,
        server_id,
        tool_name,
        arguments,
        cancel_requested,
    )
    .await;
    match decision.as_str() {
        "allow_mcp_server_15m" => add_temporary_mcp_allow(state, server_id, 15).await,
        "allow_mcp_server_30m" => add_temporary_mcp_allow(state, server_id, 30).await,
        _ => {}
    }
    decision
}

async fn temporary_mcp_allowed(state: &AppState, server_id: &str) -> bool {
    let agent_id = state.config.read().await.agent_id.clone();
    let now = Utc::now();
    let mut allows = state.temporary_mcp_allows.lock().await;
    allows.retain(|allow| allow.expires_at > now);
    allows
        .iter()
        .any(|allow| allow.agent_id == agent_id && allow.server_id == server_id)
}

#[cfg(test)]
pub(crate) async fn allow_mcp_server_for_test(state: &AppState, server_id: &str) {
    add_temporary_mcp_allow(state, server_id, 15).await;
}

async fn add_temporary_mcp_allow(state: &AppState, server_id: &str, minutes: i64) {
    let agent_id = state.config.read().await.agent_id.clone();
    let expires_at = Utc::now() + chrono::Duration::minutes(minutes);
    let mut allows = state.temporary_mcp_allows.lock().await;
    allows.retain(|allow| allow.expires_at > Utc::now());
    allows.retain(|allow| !(allow.agent_id == agent_id && allow.server_id == server_id));
    allows.push(TemporaryMcpAllow {
        agent_id,
        server_id: server_id.to_string(),
        expires_at,
    });
}

async fn request_mcp_tool_confirmation_cancellable(
    state: &AppState,
    config: &Config,
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    cancel_requested: Arc<AtomicBool>,
) -> String {
    for channel in &config.confirmation_provider.channels {
        if cancel_requested.load(Ordering::Acquire) {
            return "cancelled".to_string();
        }
        let result = match channel {
            ConfirmationChannel::Freedesktop => {
                tokio::select! {
                    result = request_freedesktop_mcp_confirmation(config, server_id, tool_name, arguments) => result,
                    _ = wait_for_cancellation(cancel_requested.clone()) => "cancelled".to_string(),
                }
            }
            ConfirmationChannel::Ntfy => {
                request_hub_mcp_confirmation_cancellable(
                    state,
                    config,
                    server_id,
                    tool_name,
                    arguments,
                    cancel_requested.clone(),
                )
                .await
            }
        };
        if result == "cancelled" || result != "confirmation_provider_unavailable" {
            return result;
        }
    }
    "confirmation_provider_unavailable".to_string()
}

async fn request_freedesktop_mcp_confirmation(
    _config: &Config,
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let supports_actions = tokio::task::spawn_blocking(|| {
        notify_rust::get_capabilities()
            .map(|capabilities| {
                capabilities
                    .iter()
                    .any(|capability| capability == "actions")
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !supports_actions {
        return "confirmation_provider_unavailable".to_string();
    }
    let body = format!(
        "{}\n\nAllow once, or temporarily allow this MCP server?",
        mcp_tool_command_preview(server_id, tool_name, arguments)
    );
    let provider = notify_rust::Notification::new()
        .summary("Agentic GPT MCP confirmation")
        .body(&body)
        .action("allow_once", "Allow once")
        .action("allow_mcp_server_15m", "Allow this MCP 15m")
        .action("allow_mcp_server_30m", "Allow this MCP 30m")
        .action("deny", "Deny")
        .timeout((CONFIRM_TIMEOUT_SECS * 1000) as i32)
        .show();
    match provider {
        Ok(handle) => {
            let action = tokio::task::spawn_blocking(move || {
                let mut selected = "timeout".to_string();
                handle.wait_for_action(|action| selected = action.to_string());
                selected
            })
            .await
            .unwrap_or_else(|_| "timeout".to_string());
            match action.as_str() {
                "allow_once" | "allow_mcp_server_15m" | "allow_mcp_server_30m" => action,
                _ => "deny".to_string(),
            }
        }
        Err(_) => "confirmation_provider_unavailable".to_string(),
    }
}

async fn request_hub_mcp_confirmation_cancellable(
    state: &AppState,
    config: &Config,
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    cancel_requested: Arc<AtomicBool>,
) -> String {
    if cancel_requested.load(Ordering::Acquire) {
        return "cancelled".to_string();
    }
    let request_id = format!("confirm_req_{}", Uuid::new_v4().simple());
    let (tx, rx) = oneshot::channel();
    state
        .pending_confirmations
        .lock()
        .await
        .insert(request_id.clone(), tx);
    let payload = ConfirmationPayload {
        program: "mcp.callTool".to_string(),
        args: vec![server_id.to_string(), tool_name.to_string()],
        command_preview: mcp_tool_command_preview(server_id, tool_name, arguments),
        risk_level: "MEDIUM".to_string(),
        reason: "MCP tool call requires confirmation".to_string(),
        kind: Some("mcpTool".to_string()),
        server_id: Some(server_id.to_string()),
        tool_name: Some(tool_name.to_string()),
    };
    let message = AgentMessage::ConfirmationRequest {
        request_id: request_id.clone(),
        agent_id: config.agent_id.clone(),
        timeout_seconds: CONFIRM_TIMEOUT_SECS,
        payload,
    };
    if let Err(error) = hub::send_agent_message(state, message).await {
        state.pending_confirmations.lock().await.remove(&request_id);
        log_warn(format!("hub MCP confirmation unavailable: {error}"));
        return "provider_unavailable".to_string();
    }
    log_info(format!(
        "hub MCP confirmation requested; requestId={request_id}; serverId={server_id}; toolName={tool_name}"
    ));
    let result = tokio::select! {
        result = timeout(Duration::from_secs(CONFIRM_TIMEOUT_SECS + 5), rx) => {
            match result {
                Ok(Ok(decision)) => decision,
                _ => "timeout".to_string(),
            }
        }
        _ = wait_for_cancellation(cancel_requested) => "cancelled".to_string(),
    };
    state.pending_confirmations.lock().await.remove(&request_id);
    result
}

pub(crate) fn confirmation_decision_value(decision: ConfirmationDecision) -> String {
    match decision {
        ConfirmationDecision::AllowOnce => "allow_once",
        ConfirmationDecision::AllowMcpServer15m => "allow_mcp_server_15m",
        ConfirmationDecision::AllowMcpServer30m => "allow_mcp_server_30m",
        ConfirmationDecision::Deny => "deny",
        ConfirmationDecision::Timeout => "timeout",
        ConfirmationDecision::ProviderUnavailable => "provider_unavailable",
        ConfirmationDecision::CallbackTokenInvalid => "callback_token_invalid",
        ConfirmationDecision::Expired => "expired",
    }
    .to_string()
}
