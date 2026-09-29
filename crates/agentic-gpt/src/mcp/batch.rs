use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use agentic_gpt_protocol::{
    McpBatchChildResponse, McpBatchMode, McpBatchRequest, McpBatchResponse, McpBatchStatus,
    McpCallToolRequest, ProcessError, ProcessState,
};
use anyhow::{anyhow, Result};
use rmcp::model::JsonObject;
use sha2::{Digest, Sha256};
use tokio::{
    task::JoinSet,
    time::{sleep, Duration, Instant},
};

use crate::{
    audit::{write_mcp_batch_audit, McpBatchAuditRecord},
    config::mcp_servers::McpServerConfig,
    confirmation::{self, McpBatchConfirmationItem},
    process,
    process::{ManagedMcpSpec, TerminalEventHook},
    state::AppState,
    utils::bounded_mcp_argument_keys,
};

use super::{
    mcp_authorization_allows, production_client_factory, run_managed_call, server_config_snapshot,
    tool_arguments, validate_tool_name, McpClientFactory,
};

#[derive(Clone)]
struct PreparedMcpBatchCall {
    index: usize,
    id: Option<String>,
    payload: McpCallToolRequest,
    arguments: JsonObject,
    server: McpServerConfig,
    config_revision: String,
    argument_keys: Vec<String>,
    argument_key_count: usize,
    argument_keys_truncated: bool,
    argument_bytes: usize,
    argument_sha256: String,
    temporary_allowed: bool,
}

pub(crate) async fn batch(
    state: &AppState,
    payload: McpBatchRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
) -> Result<McpBatchResponse> {
    start_managed_batch_with_factory(
        state,
        payload,
        request_source,
        terminal_event_hook,
        production_client_factory(),
    )
    .await
}

pub(crate) async fn batch_slim(
    state: &AppState,
    payload: McpBatchRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
) -> Result<McpBatchResponse> {
    start_managed_batch_without_aggregate_budget(
        state,
        payload,
        request_source,
        terminal_event_hook,
        production_client_factory(),
    )
    .await
}

pub(super) async fn start_managed_batch_with_factory(
    state: &AppState,
    payload: McpBatchRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
    client_factory: McpClientFactory,
) -> Result<McpBatchResponse> {
    start_managed_batch_with_factory_budget(
        state,
        payload,
        request_source,
        terminal_event_hook,
        client_factory,
        true,
    )
    .await
}

async fn start_managed_batch_without_aggregate_budget(
    state: &AppState,
    payload: McpBatchRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
    client_factory: McpClientFactory,
) -> Result<McpBatchResponse> {
    start_managed_batch_with_factory_budget(
        state,
        payload,
        request_source,
        terminal_event_hook,
        client_factory,
        false,
    )
    .await
}

async fn start_managed_batch_with_factory_budget(
    state: &AppState,
    payload: McpBatchRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
    client_factory: McpClientFactory,
    enforce_aggregate_budget: bool,
) -> Result<McpBatchResponse> {
    let started = Instant::now();
    let batch_id = format!("batch_{}", uuid::Uuid::new_v4().simple());
    let prepared = match prepare_mcp_batch(state, &payload).await {
        Ok(prepared) => prepared,
        Err(error) => {
            write_batch_rejection_audit(
                state,
                &batch_id,
                request_source,
                &payload,
                "validation_rejected",
                &batch_error_code(&error.to_string()),
                started,
            )
            .await;
            return Err(error);
        }
    };
    let server_count = prepared
        .iter()
        .map(|call| call.payload.server_id.as_str())
        .collect::<HashSet<_>>()
        .len();
    let specs = prepared
        .iter()
        .map(|call| ManagedMcpSpec {
            agent_id: payload.agent_id.clone(),
            group: payload.group.clone(),
            batch_id: Some(batch_id.clone()),
            batch_call_id: call.id.clone(),
            batch_index: Some(call.index),
            server_id: call.payload.server_id.clone(),
            tool_name: call.payload.tool_name.clone(),
            request_source: request_source.to_string(),
            argument_keys: call.argument_keys.clone(),
            argument_key_count: call.argument_key_count,
            argument_keys_truncated: call.argument_keys_truncated,
            argument_bytes: call.argument_bytes,
            argument_sha256: call.argument_sha256.clone(),
            config_revision: call.config_revision.clone(),
            terminal_event_hook: terminal_event_hook.clone(),
        })
        .collect::<Vec<_>>();
    let registrations = match process::register_mcp_batch(state, specs).await {
        Ok(registrations) => registrations,
        Err(reason) => {
            write_batch_rejection_audit(
                state,
                &batch_id,
                request_source,
                &payload,
                "capacity_rejected",
                &batch_error_code(&reason),
                started,
            )
            .await;
            return Err(anyhow!(reason));
        }
    };
    let child_refs = registrations
        .iter()
        .enumerate()
        .map(|(index, registration)| {
            (
                index,
                prepared[index].id.clone(),
                registration.info.process_id.clone(),
            )
        })
        .collect::<Vec<_>>();

    let confirmation_items = prepared
        .iter()
        .filter(|call| !call.temporary_allowed)
        .map(|call| McpBatchConfirmationItem {
            index: call.index,
            id: call.id.clone(),
            server_id: call.payload.server_id.clone(),
            tool_name: call.payload.tool_name.clone(),
            argument_keys: call.argument_keys.clone(),
            argument_key_count: call.argument_key_count,
            argument_keys_truncated: call.argument_keys_truncated,
            argument_bytes: call.argument_bytes,
            argument_sha256: call.argument_sha256.clone(),
        })
        .collect::<Vec<_>>();
    let confirmation_servers = confirmation_items
        .iter()
        .map(|item| item.server_id.as_str())
        .collect::<HashSet<_>>();
    let temporary_server = (confirmation_servers.len() == 1)
        .then(|| {
            confirmation_items
                .first()
                .map(|item| item.server_id.as_str())
        })
        .flatten();
    let batch_cancel = Arc::new(AtomicBool::new(false));
    let watcher = tokio::spawn(watch_batch_cancellation(
        registrations
            .iter()
            .map(|registration| registration.cancel_requested.clone())
            .collect(),
        batch_cancel.clone(),
    ));
    let confirmation_result = confirmation::authorize_mcp_batch_cancellable(
        state,
        &confirmation_items,
        temporary_server,
        batch_cancel,
    )
    .await;
    watcher.abort();
    for (index, registration) in registrations.iter().enumerate() {
        let child_authorization = if prepared[index].temporary_allowed {
            "temporary_mcp_allow"
        } else {
            confirmation_result.as_str()
        };
        let _ = process::set_mcp_authorization(
            state,
            &registration.info.process_id,
            child_authorization,
        )
        .await;
    }

    if !mcp_authorization_allows(&confirmation_result) {
        let terminal = if confirmation_result == "cancelled" {
            ProcessState::Cancelled
        } else {
            ProcessState::Rejected
        };
        for registration in &registrations {
            let _ = process::finish_mcp_error(
                state,
                &registration.info.process_id,
                terminal,
                "mcp_batch_rejected",
                format!("MCP batch did not start: {confirmation_result}"),
                (terminal == ProcessState::Cancelled).then_some("batch_cancelled_before_start"),
                Some("aggregate_authorization_decision"),
            )
            .await;
        }
        let mut response = build_mcp_batch_response(
            state,
            &batch_id,
            &child_refs,
            0,
            Some(McpBatchStatus::Rejected),
            enforce_aggregate_budget,
        )
        .await?;
        response.error = Some(ProcessError {
            code: "mcp_batch_rejected".to_string(),
            message: format!("MCP batch did not start: {confirmation_result}"),
        });
        if enforce_aggregate_budget {
            apply_batch_result_budget(&mut response)?;
        }
        write_batch_audit(
            state,
            &batch_id,
            request_source,
            &payload,
            server_count,
            confirmation_items.len(),
            Some(confirmation_result),
            &child_refs,
            &response,
            started,
        )
        .await;
        return Ok(response);
    }

    let mode = payload.mode;
    let fail_fast = payload.fail_fast;
    let timeout_seconds = payload.effective_timeout_seconds();
    let coordinator_state = state.clone();
    let coordinator_batch_id = batch_id.clone();
    let coordinator_source = request_source.to_string();
    let coordinator_payload = payload.clone();
    let coordinator_refs = child_refs.clone();
    let coordinator_confirmation = confirmation_result.clone();
    tokio::spawn(async move {
        run_mcp_batch_coordinator(
            coordinator_state.clone(),
            prepared,
            registrations,
            mode,
            fail_fast,
            timeout_seconds,
            client_factory,
            coordinator_confirmation.clone(),
        )
        .await;
        if let Ok(response) = build_mcp_batch_response(
            &coordinator_state,
            &coordinator_batch_id,
            &coordinator_refs,
            0,
            None,
            enforce_aggregate_budget,
        )
        .await
        {
            write_batch_audit(
                &coordinator_state,
                &coordinator_batch_id,
                &coordinator_source,
                &coordinator_payload,
                server_count,
                confirmation_items.len(),
                Some(coordinator_confirmation),
                &coordinator_refs,
                &response,
                started,
            )
            .await;
        }
    });

    build_mcp_batch_response(
        state,
        &batch_id,
        &child_refs,
        payload.effective_wait_seconds(),
        None,
        enforce_aggregate_budget,
    )
    .await
}

async fn prepare_mcp_batch(
    state: &AppState,
    payload: &McpBatchRequest,
) -> Result<Vec<PreparedMcpBatchCall>> {
    if !(McpBatchRequest::MIN_CALLS..=McpBatchRequest::MAX_CALLS).contains(&payload.calls.len()) {
        return Err(anyhow!(
            "mcp_batch_call_count_invalid: calls={}; min={}; max={}",
            payload.calls.len(),
            McpBatchRequest::MIN_CALLS,
            McpBatchRequest::MAX_CALLS
        ));
    }
    let mut ids = HashSet::new();
    let mut aggregate_bytes = 0usize;
    let mut prepared = Vec::with_capacity(payload.calls.len());
    for (index, call) in payload.calls.iter().enumerate() {
        if let Some(id) = call.id.as_deref() {
            validate_batch_call_id(id)?;
            if !ids.insert(id.to_string()) {
                return Err(anyhow!("mcp_batch_call_id_duplicate: {id}"));
            }
        }
        validate_tool_name(&call.tool_name)?;
        let arguments = tool_arguments(call.arguments.clone())?;
        let encoded = serde_json::to_vec(&call.arguments)?;
        if encoded.len() > process::MAX_MCP_ARGUMENT_BYTES {
            return Err(anyhow!(
                "mcp_tool_arguments_too_large: index={index}; bytes={}; max={}",
                encoded.len(),
                process::MAX_MCP_ARGUMENT_BYTES
            ));
        }
        aggregate_bytes = aggregate_bytes.saturating_add(encoded.len());
        if aggregate_bytes > McpBatchRequest::MAX_AGGREGATE_ARGUMENT_BYTES {
            return Err(anyhow!(
                "mcp_batch_arguments_too_large: bytes={aggregate_bytes}; max={}",
                McpBatchRequest::MAX_AGGREGATE_ARGUMENT_BYTES
            ));
        }
        let argument_sha256 = format!("sha256:{:x}", Sha256::digest(&encoded));
        let (argument_keys, argument_key_count, argument_keys_truncated) =
            bounded_mcp_argument_keys(&call.arguments);
        let (config_revision, server) = server_config_snapshot(state, &call.server_id).await;
        let server = server.map_err(|reason| anyhow!(reason))?;
        let temporary_allowed = confirmation::temporary_mcp_allowed(state, &call.server_id).await;
        prepared.push(PreparedMcpBatchCall {
            index,
            id: call.id.clone(),
            payload: McpCallToolRequest {
                agent_id: payload.agent_id.clone(),
                group: payload.group.clone(),
                server_id: call.server_id.clone(),
                tool_name: call.tool_name.clone(),
                arguments: call.arguments.clone(),
                wait_seconds: Some(0),
                timeout_seconds: payload.timeout_seconds,
            },
            arguments,
            server,
            config_revision,
            argument_keys,
            argument_key_count,
            argument_keys_truncated,
            argument_bytes: encoded.len(),
            argument_sha256,
            temporary_allowed,
        });
    }
    Ok(prepared)
}

#[allow(clippy::too_many_arguments)]
async fn run_mcp_batch_coordinator(
    state: AppState,
    prepared: Vec<PreparedMcpBatchCall>,
    registrations: Vec<process::ManagedMcpRegistration>,
    mode: McpBatchMode,
    fail_fast: bool,
    timeout_seconds: u64,
    client_factory: McpClientFactory,
    confirmation_result: String,
) {
    let stop = Arc::new(AtomicBool::new(false));
    match mode {
        McpBatchMode::Sequential => {
            for (call, registration) in prepared.into_iter().zip(registrations) {
                if fail_fast && stop.load(Ordering::Acquire) {
                    mark_batch_child_skipped(&state, &registration.info.process_id).await;
                    continue;
                }
                run_managed_call(
                    state.clone(),
                    call.payload,
                    call.arguments,
                    call.server,
                    registration.cancel_requested,
                    timeout_seconds,
                    registration.info.process_id.clone(),
                    client_factory.clone(),
                    Some(if call.temporary_allowed {
                        "temporary_mcp_allow".to_string()
                    } else {
                        confirmation_result.clone()
                    }),
                    fail_fast.then(|| stop.clone()),
                )
                .await;
                if fail_fast
                    && process::get_process_detail(&state, &registration.info.process_id, 0)
                        .await
                        .is_ok_and(|process| hard_batch_failure(process.process.state))
                {
                    stop.store(true, Ordering::Release);
                }
            }
        }
        McpBatchMode::Parallel => {
            let mut tasks = JoinSet::new();
            for (call, registration) in prepared.into_iter().zip(registrations) {
                let task_state = state.clone();
                let task_factory = client_factory.clone();
                let task_stop = stop.clone();
                let task_confirmation = confirmation_result.clone();
                tasks.spawn(async move {
                    run_managed_call(
                        task_state.clone(),
                        call.payload,
                        call.arguments,
                        call.server,
                        registration.cancel_requested,
                        timeout_seconds,
                        registration.info.process_id.clone(),
                        task_factory,
                        Some(if call.temporary_allowed {
                            "temporary_mcp_allow".to_string()
                        } else {
                            task_confirmation
                        }),
                        fail_fast.then(|| task_stop.clone()),
                    )
                    .await;
                    if fail_fast
                        && process::get_process_detail(
                            &task_state,
                            &registration.info.process_id,
                            0,
                        )
                        .await
                        .is_ok_and(|process| hard_batch_failure(process.process.state))
                    {
                        task_stop.store(true, Ordering::Release);
                    }
                });
            }
            while tasks.join_next().await.is_some() {}
        }
    }
}

async fn mark_batch_child_skipped(state: &AppState, process_id: &str) {
    let _ = process::finish_mcp_error(
        state,
        process_id,
        ProcessState::Skipped,
        "mcp_batch_fail_fast_skipped",
        "MCP batch fail-fast prevented this queued child from starting",
        None,
        Some("fail_fast_before_downstream_start"),
    )
    .await;
}

fn hard_batch_failure(state: ProcessState) -> bool {
    matches!(
        state,
        ProcessState::Failed
            | ProcessState::Rejected
            | ProcessState::Cancelled
            | ProcessState::TimedOut
            | ProcessState::Detached
            | ProcessState::UnknownAfterRestart
    )
}

async fn watch_batch_cancellation(flags: Vec<Arc<AtomicBool>>, batch_cancel: Arc<AtomicBool>) {
    loop {
        if flags.iter().any(|flag| flag.load(Ordering::Acquire)) {
            batch_cancel.store(true, Ordering::Release);
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
}

async fn build_mcp_batch_response(
    state: &AppState,
    batch_id: &str,
    child_refs: &[(usize, Option<String>, String)],
    wait_seconds: u64,
    forced_status: Option<McpBatchStatus>,
    enforce_aggregate_budget: bool,
) -> Result<McpBatchResponse> {
    let deadline = Instant::now() + Duration::from_secs(wait_seconds.min(30));
    let mut details = Vec::new();
    loop {
        details.clear();
        let mut all_terminal = true;
        for (index, id, process_id) in child_refs {
            let detail = process::get_process_detail(state, process_id, 0)
                .await
                .map_err(|reason| anyhow!(reason))?;
            all_terminal &= detail.process.state.is_terminal();
            details.push(McpBatchChildResponse {
                index: *index,
                id: id.clone(),
                result_omitted: false,
                process: detail,
            });
        }
        if all_terminal || wait_seconds == 0 || Instant::now() >= deadline {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    let completed_inline = details
        .iter()
        .all(|result| result.process.process.state.is_terminal());
    let status = forced_status.unwrap_or_else(|| {
        if !completed_inline {
            McpBatchStatus::Running
        } else if details
            .iter()
            .all(|result| result.process.process.state == ProcessState::Completed)
        {
            McpBatchStatus::Completed
        } else {
            McpBatchStatus::CompletedWithErrors
        }
    });
    let mut response = McpBatchResponse {
        batch_id: batch_id.to_string(),
        status,
        completed_inline,
        poll_after_ms: if completed_inline { 0 } else { 1_000 },
        results: details,
        aggregate_truncated: false,
        aggregate_bytes: None,
        error: None,
    };
    if enforce_aggregate_budget {
        apply_batch_result_budget(&mut response)?;
    }
    Ok(response)
}

fn apply_batch_result_budget(response: &mut McpBatchResponse) -> Result<()> {
    let mut bytes = serde_json::to_vec(response)?.len();
    if bytes > McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES {
        response.aggregate_truncated = true;
        for index in (0..response.results.len()).rev() {
            let removed = {
                let child = &mut response.results[index];
                if child.process.result.take().is_some() {
                    child.result_omitted = true;
                    true
                } else {
                    false
                }
            };
            if removed {
                bytes = serde_json::to_vec(response)?.len();
                if bytes <= McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES {
                    break;
                }
            }
        }
    }
    let mut previous = None;
    for _ in 0..4 {
        let current = serde_json::to_vec(response)?.len();
        response.aggregate_bytes = Some(current);
        if previous == Some(current) {
            break;
        }
        previous = Some(current);
    }
    let final_bytes = serde_json::to_vec(response)?.len();
    response.aggregate_bytes = Some(final_bytes);
    if final_bytes > McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES {
        return Err(anyhow!(
            "mcp_batch_result_too_large_after_clipping: bytes={final_bytes}; max={}",
            McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES
        ));
    }
    Ok(())
}

async fn write_batch_rejection_audit(
    state: &AppState,
    batch_id: &str,
    request_source: &str,
    payload: &McpBatchRequest,
    outcome: &str,
    error_code: &str,
    started: Instant,
) {
    let config = state.config.read().await.clone();
    let server_count = payload
        .calls
        .iter()
        .map(|call| call.server_id.as_str())
        .collect::<HashSet<_>>()
        .len();
    let _ = write_mcp_batch_audit(
        &config,
        McpBatchAuditRecord {
            time: chrono::Utc::now(),
            tool: "mcp.batch".to_string(),
            batch_id: batch_id.to_string(),
            request_source: request_source.to_string(),
            call_count: payload.calls.len(),
            server_count,
            mode: match payload.mode {
                McpBatchMode::Parallel => "parallel",
                McpBatchMode::Sequential => "sequential",
            }
            .to_string(),
            fail_fast: payload.fail_fast,
            confirmation_required_count: 0,
            confirmation_result: None,
            child_process_ids: Vec::new(),
            outcome: outcome.to_string(),
            error_code: Some(error_code.to_string()),
            duration_ms: started.elapsed().as_millis(),
            truncated: false,
        },
    );
}

fn batch_error_code(message: &str) -> String {
    let end = message.find([':', ';']).unwrap_or(message.len());
    message[..end].chars().take(128).collect()
}

#[allow(clippy::too_many_arguments)]
async fn write_batch_audit(
    state: &AppState,
    batch_id: &str,
    request_source: &str,
    payload: &McpBatchRequest,
    server_count: usize,
    confirmation_required_count: usize,
    confirmation_result: Option<String>,
    child_refs: &[(usize, Option<String>, String)],
    response: &McpBatchResponse,
    started: Instant,
) {
    let config = state.config.read().await.clone();
    let _ = write_mcp_batch_audit(
        &config,
        McpBatchAuditRecord {
            time: chrono::Utc::now(),
            tool: "mcp.batch".to_string(),
            batch_id: batch_id.to_string(),
            request_source: request_source.to_string(),
            call_count: payload.calls.len(),
            server_count,
            mode: match payload.mode {
                McpBatchMode::Parallel => "parallel",
                McpBatchMode::Sequential => "sequential",
            }
            .to_string(),
            fail_fast: payload.fail_fast,
            confirmation_required_count,
            confirmation_result,
            child_process_ids: child_refs
                .iter()
                .map(|(_, _, process_id)| process_id.clone())
                .collect(),
            outcome: match response.status {
                McpBatchStatus::Running => "running",
                McpBatchStatus::Completed => "completed",
                McpBatchStatus::CompletedWithErrors => "completed_with_errors",
                McpBatchStatus::Rejected => "rejected",
            }
            .to_string(),
            error_code: response.error.as_ref().map(|error| error.code.clone()),
            duration_ms: started.elapsed().as_millis(),
            truncated: response.aggregate_truncated,
        },
    );
}

fn validate_batch_call_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
    {
        return Err(anyhow!("mcp_batch_call_id_invalid: {id}"));
    }
    Ok(())
}
