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
    hub,
    state::AppState,
    utils::{
        command_preview, log_info, log_warn, mcp_tool_command_preview, risk_level,
        risky_file_mutation, truncate_chars, CONFIRM_TIMEOUT_SECS,
    },
};

#[derive(Clone, Debug)]
pub(crate) struct TemporaryMcpAllow {
    pub(crate) agent_id: String,
    pub(crate) server_id: String,
    pub(crate) expires_at: DateTime<Utc>,
}

fn confirmation_channels(config: &Config, method: Option<&str>) -> Vec<ConfirmationChannel> {
    match method {
        None | Some("default") => config.confirmation_provider.channels.clone(),
        Some(value) => ConfirmationProviderConfig::from_legacy(value)
            .map(|c| c.channels)
            .unwrap_or_default(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OperationAuthorization {
    pub(crate) approved: bool,
    pub(crate) result: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn authorize_operation(
    state: &AppState,
    config: &Config,
    acquisition: Option<&agentic_gpt_protocol::AcquireConfirmation>,
    default_acquire: bool,
    decision: crate::policy::PolicyDecision,
    program: &str,
    args: &[String],
    strict: bool,
) -> Result<OperationAuthorization, String> {
    authorize_operation_cancellable(
        state,
        config,
        acquisition,
        default_acquire,
        decision,
        program,
        args,
        strict,
        Arc::new(AtomicBool::new(false)),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn authorize_operation_cancellable(
    state: &AppState,
    config: &Config,
    acquisition: Option<&agentic_gpt_protocol::AcquireConfirmation>,
    default_acquire: bool,
    decision: crate::policy::PolicyDecision,
    program: &str,
    args: &[String],
    strict: bool,
    cancelled: Arc<AtomicBool>,
) -> Result<OperationAuthorization, String> {
    use crate::policy::PolicyDecision;
    if cancelled.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let acquire = acquisition
        .map(|a| a.effective_acquire(default_acquire))
        .unwrap_or(default_acquire);
    let method = acquisition
        .and_then(|a| a.method.as_ref())
        .map(|m| m.as_str());
    let mut attempted = None;
    if acquire {
        let result = request_confirmation_cancellable(
            state,
            config,
            method,
            program,
            args,
            cancelled.clone(),
        )
        .await;
        if cancelled.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        if result == "allow_once" {
            return Ok(OperationAuthorization {
                approved: true,
                result: Some(result),
            });
        }
        if strict
            || !matches!(
                result.as_str(),
                "timeout" | "expired" | "provider_unavailable"
            )
        {
            return Err(result);
        }
        attempted = Some(result);
    }
    match decision {
        PolicyDecision::Deny => Err("policy_denied".into()),
        PolicyDecision::Allow => Ok(OperationAuthorization {
            approved: false,
            result: attempted,
        }),
        PolicyDecision::Confirm => {
            if let Some(result) = attempted {
                return Err(result);
            }
            let result = request_confirmation_cancellable(
                state,
                config,
                None,
                program,
                args,
                cancelled.clone(),
            )
            .await;
            if cancelled.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            if result == "allow_once" {
                Ok(OperationAuthorization {
                    approved: true,
                    result: Some(result),
                })
            } else {
                Err(result)
            }
        }
    }
}

/// Confirmation used by an asynchronously registered process. Hub-backed
/// requests remove their pending sender when the process is cancelled, so a
/// cancelled `waiting_confirmation` process cannot leak a durable callback entry.
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
            ConfirmationChannel::Freedesktop => {
                request_freedesktop_confirmation(config, program, args, cancel_requested.clone())
                    .await
            }
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
        if result != "provider_unavailable" {
            return result;
        }
    }
    "provider_unavailable".to_string()
}

async fn request_freedesktop_confirmation(
    config: &Config,
    program: &str,
    args: &[String],
    cancelled: Arc<AtomicBool>,
) -> String {
    let supports_actions = crate::notify::freedesktop_supports_actions().await;
    if !supports_actions {
        return "provider_unavailable".to_string();
    }
    let zh = confirmation_language_is_zh(config);
    let warning = if !(config.policy.enabled && config.policy.bubblewrap)
        && risky_file_mutation(program)
    {
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
            let mut selected = "provider_unavailable".to_string();
            let waited = tokio::select! {
                result = timeout(
                    Duration::from_secs(CONFIRM_TIMEOUT_SECS),
                    handle.wait_for_action_async(|response| {
                    selected = match response {
                        notify_rust::ActionResponse::Custom("allow_once") => "allow_once",
                        notify_rust::ActionResponse::Custom("deny") => "deny",
                        notify_rust::ActionResponse::Closed(notify_rust::CloseReason::Expired) => {
                            "timeout"
                        }
                        notify_rust::ActionResponse::Closed(
                            notify_rust::CloseReason::Dismissed,
                        ) => "deny",
                        _ => "provider_unavailable",
                    }
                    .to_string();
                    }),
                ) => result.map(|_| ()),
                _ = wait_for_cancellation(cancelled) => {
                    selected = "cancelled".to_string();
                    Ok(())
                },
            };
            let _ = timeout(Duration::from_secs(2), handle.close_async()).await;
            if waited.is_err() {
                "timeout".to_string()
            } else {
                selected
            }
        }
        Err(_) => "provider_unavailable".to_string(),
    }
}

async fn request_hub_confirmation_cancellable(
    state: &AppState,
    config: &Config,
    program: &str,
    args: &[String],
    cancel_requested: Arc<AtomicBool>,
) -> String {
    let preview = command_preview(program, args);
    // The Hub's notification surface displays at most 1000 characters. Never
    // obtain authority from a preview that omits part of the operation/scope.
    if preview.chars().count() > 1000 {
        log_warn("confirmation preview exceeds notification display limit".to_string());
        return "provider_unavailable".to_string();
    }
    let payload = ConfirmationPayload {
        program: program.to_string(),
        args: args.to_vec(),
        command_preview: preview,
        risk_level: risk_level(program),
        reason: if confirmation_language_is_zh(config) {
            format!("请求批准本次操作及所示访问范围：{program}")
        } else {
            format!("Approve this operation and its displayed access scope: {program}")
        },
        kind: None,
        server_id: None,
        tool_name: None,
    };
    request_hub_confirmation_payload_cancellable(state, payload, Some(cancel_requested)).await
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
        agent_id: if payload.kind.is_none() {
            state.transport_agent_id.clone()
        } else {
            config.agent_id
        },
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

#[derive(Clone, Debug)]
pub(crate) struct McpBatchConfirmationItem {
    pub(crate) index: usize,
    pub(crate) id: Option<String>,
    pub(crate) server_id: String,
    pub(crate) tool_name: String,
    pub(crate) argument_keys: Vec<String>,
    pub(crate) argument_key_count: usize,
    pub(crate) argument_keys_truncated: bool,
    pub(crate) argument_bytes: usize,
    pub(crate) argument_sha256: String,
}

pub(crate) async fn authorize_mcp_batch_cancellable(
    state: &AppState,
    items: &[McpBatchConfirmationItem],
    temporary_server: Option<&str>,
    cancel_requested: Arc<AtomicBool>,
) -> String {
    if items.is_empty() {
        return "temporary_mcp_allow".to_string();
    }
    if cancel_requested.load(Ordering::Acquire) {
        return "cancelled".to_string();
    }
    let config = state.config.read().await.clone();
    let preview = mcp_batch_confirmation_preview(items);
    let result = request_mcp_batch_confirmation_cancellable(
        state,
        &config,
        &preview,
        temporary_server,
        cancel_requested,
    )
    .await;
    match (result.as_str(), temporary_server) {
        ("allow_mcp_server_15m", Some(server_id)) => {
            add_temporary_mcp_allow(state, server_id, 15).await
        }
        ("allow_mcp_server_30m", Some(server_id)) => {
            add_temporary_mcp_allow(state, server_id, 30).await
        }
        _ => {}
    }
    result
}

pub(crate) async fn temporary_mcp_allowed(state: &AppState, server_id: &str) -> bool {
    let agent_id = state.config.read().await.agent_id.clone();
    let now = Utc::now();
    let mut allows = state.temporary_mcp_allows.lock().await;
    allows.retain(|allow| allow.expires_at > now);
    allows
        .iter()
        .any(|allow| allow.agent_id == agent_id && allow.server_id == server_id)
}

fn mcp_batch_confirmation_preview(items: &[McpBatchConfirmationItem]) -> String {
    let mut lines = vec![format!(
        "MCP Batch Call\nCalls requiring confirmation: {}",
        items.len()
    )];
    for item in items {
        let id = item.id.as_deref().unwrap_or("-");
        lines.push(format!(
            "#{} id={} server={} tool={} keys(showing {} of {}, truncated={}): [{}] bytes={} sha256={}",
            item.index,
            id,
            item.server_id,
            item.tool_name,
            item.argument_keys.len(),
            item.argument_key_count,
            item.argument_keys_truncated,
            item.argument_keys.join(", "),
            item.argument_bytes,
            item.argument_sha256,
        ));
    }
    truncate_chars(&lines.join("\n"), 8 * 1024)
}

async fn request_mcp_batch_confirmation_cancellable(
    state: &AppState,
    config: &Config,
    preview: &str,
    temporary_server: Option<&str>,
    cancel_requested: Arc<AtomicBool>,
) -> String {
    for channel in &config.confirmation_provider.channels {
        if cancel_requested.load(Ordering::Acquire) {
            return "cancelled".to_string();
        }
        let result = match channel {
            ConfirmationChannel::Freedesktop => {
                tokio::select! {
                    result = request_freedesktop_mcp_batch_confirmation(preview, temporary_server.is_some()) => result,
                    _ = wait_for_cancellation(cancel_requested.clone()) => "cancelled".to_string(),
                }
            }
            ConfirmationChannel::Ntfy => {
                let payload = ConfirmationPayload {
                    program: "mcp.batch".to_string(),
                    args: Vec::new(),
                    command_preview: preview.to_string(),
                    risk_level: "MEDIUM".to_string(),
                    reason: "MCP batch requires one aggregate confirmation".to_string(),
                    kind: Some(if temporary_server.is_some() {
                        "mcpBatchSingleServer".to_string()
                    } else {
                        "mcpBatch".to_string()
                    }),
                    server_id: temporary_server.map(str::to_string),
                    tool_name: None,
                };
                request_hub_confirmation_payload_cancellable(
                    state,
                    payload,
                    Some(cancel_requested.clone()),
                )
                .await
            }
        };
        if result == "cancelled" || result != "provider_unavailable" {
            return result;
        }
    }
    "provider_unavailable".to_string()
}

async fn request_freedesktop_mcp_batch_confirmation(
    preview: &str,
    allow_temporary_server: bool,
) -> String {
    let supports_actions = crate::notify::freedesktop_supports_actions().await;
    if !supports_actions {
        return "provider_unavailable".to_string();
    }
    let mut notification = notify_rust::Notification::new();
    notification
        .summary("Agentic GPT MCP batch confirmation")
        .body(preview)
        .action("allow_once", "Allow once")
        .action("deny", "Deny")
        .timeout((CONFIRM_TIMEOUT_SECS * 1000) as i32);
    if allow_temporary_server {
        notification
            .action("allow_mcp_server_15m", "Allow this MCP 15m")
            .action("allow_mcp_server_30m", "Allow this MCP 30m");
    }
    match notification.show() {
        Ok(handle) => {
            let action = tokio::task::spawn_blocking(move || {
                let mut selected = "timeout".to_string();
                handle.wait_for_action(|action| selected = action.to_string());
                selected
            })
            .await
            .unwrap_or_else(|_| "timeout".to_string());
            match action.as_str() {
                "allow_once" => action,
                "allow_mcp_server_15m" | "allow_mcp_server_30m" if allow_temporary_server => action,
                _ => "deny".to_string(),
            }
        }
        Err(_) => "provider_unavailable".to_string(),
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
        if result == "cancelled" || result != "provider_unavailable" {
            return result;
        }
    }
    "provider_unavailable".to_string()
}

async fn request_freedesktop_mcp_confirmation(
    _config: &Config,
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let supports_actions = crate::notify::freedesktop_supports_actions().await;
    if !supports_actions {
        return "provider_unavailable".to_string();
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
        Err(_) => "provider_unavailable".to_string(),
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
