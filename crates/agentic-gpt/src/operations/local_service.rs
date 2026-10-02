use std::collections::HashSet;

use agentic_gpt_protocol::{
    normalize_process_group, EventResponseDisposition, EventSettleRequest, EventSourceKind,
    HubCommand, ProcessBatchExecRequest, ProcessExecRequest, ProcessInfo,
};
use anyhow::Result;

use crate::{
    bootstrap,
    config::Config,
    mcp, notify,
    operation::{self, AdmissionError, RequestContext},
    operation_result::{
        slim_mcp_batch_response, slim_mcp_response, slim_process_batch_response,
        slim_process_cancel_response, slim_process_list_response, slim_process_response,
        slim_process_status_response,
    },
    process, room_maintenance, room_reads, skills,
    state::AppState,
    tmux,
};
fn settle_remote_response(
    state: &AppState,
    payload: &EventSettleRequest,
) -> Result<serde_json::Value> {
    let mut seen = HashSet::with_capacity(payload.dispositions.len());
    for disposition in &payload.dispositions {
        if disposition.source.kind == EventSourceKind::External {
            return Err(anyhow::anyhow!("event_settle_external_source"));
        }
        if !seen.insert((
            disposition.source.kind.as_str(),
            disposition.source.reference.as_str(),
        )) {
            return Err(anyhow::anyhow!("event_settle_duplicate_source"));
        }
    }
    for EventResponseDisposition {
        source,
        includes_terminal,
    } in &payload.dispositions
    {
        state
            .event_store
            .settle_remote_response(source, &payload.origin, *includes_terminal)?;
    }
    Ok(serde_json::json!({"status": "settled"}))
}

pub(crate) enum ProcessCall {
    Exec {
        request: ProcessExecRequest,
        terminal_event_hook: Option<process::TerminalEventHook>,
    },
    Batch {
        request: ProcessBatchExecRequest,
        terminal_event_hook: Option<process::TerminalEventHook>,
    },
}

pub(crate) async fn dispatch_process<F>(
    state: AppState,
    context: RequestContext<'_>,
    mut snapshots: Option<&mut Vec<ProcessInfo>>,
    build: F,
) -> Result<serde_json::Value>
where
    F: FnOnce(&Config) -> Result<ProcessCall>,
{
    let call = {
        let config = state.config.read().await;
        if let Err(error) = operation::authorize(state.runtime, &config, context) {
            return Ok(admission_error_value(error));
        }
        build(&config)?
    };
    let request_source = context.source();
    match call {
        ProcessCall::Exec {
            mut request,
            terminal_event_hook,
        } => {
            request.group = match normalize_group(request.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let response = process::start_and_wait_process(
                state,
                request,
                process::ProcessOptions {
                    terminal_event_hook,
                    event_origin: context.event_origin.cloned(),
                    ..process::ProcessOptions::for_source(request_source)
                },
            )
            .await;
            slim_process_response(response, snapshots.as_deref_mut())
        }
        ProcessCall::Batch {
            mut request,
            terminal_event_hook,
        } => {
            request.group = match normalize_group(request.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            match process::start_process_batch(
                state,
                request,
                request_source,
                terminal_event_hook,
                context.event_origin.cloned(),
            )
            .await
            {
                Ok(response) => slim_process_batch_response(response, snapshots),
                Err(reason) => Ok(serde_json::json!({
                    "error": {"code": "process_batch_rejected", "message": reason}
                })),
            }
        }
    }
}

/// Value-returning local operation layer shared by transport adapters.
///
/// Transport adapters own their envelopes and acknowledgements. This module owns shared admission,
/// operation execution, and result/error shapes for Hub and local stdio callers.
/// ProcessInfo values from typed operation results during projection. Other ingress
/// callers pass `None` and incur no snapshot allocation or cloning.
pub(crate) async fn dispatch(
    state: AppState,
    command: HubCommand,
    context: RequestContext<'_>,
    snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<serde_json::Value> {
    match command {
        HubCommand::Exec { payload, .. } => {
            dispatch_process(state, context, snapshots, move |_| {
                Ok(ProcessCall::Exec {
                    request: payload,
                    terminal_event_hook: None,
                })
            })
            .await
        }
        HubCommand::ProcessBatch { payload, .. } => {
            dispatch_process(state, context, snapshots, move |_| {
                Ok(ProcessCall::Batch {
                    request: payload,
                    terminal_event_hook: None,
                })
            })
            .await
        }
        command => {
            let admission = {
                let config = state.config.read().await;
                operation::authorize(state.runtime, &config, context)
            };
            if let Err(error) = admission {
                return Ok(admission_error_value(error));
            }
            dispatch_inner(state, command, context, snapshots).await
        }
    }
}

fn admission_error_value(error: AdmissionError) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": error.code(),
            "message": error.message(),
        }
    })
}
async fn bind_current_agent_id(state: &AppState, requested: &mut String) -> bool {
    let current = state.config.read().await.agent_id.clone();
    if requested.is_empty() {
        *requested = current;
        true
    } else {
        *requested == current
    }
}

fn event_agent_mismatch() -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": "event_agent_mismatch",
            "message": "event request must target the connected Agent"
        }
    })
}

fn event_store_error_value(error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = message
        .split([':', ';'])
        .next()
        .filter(|code| {
            !code.is_empty()
                && code
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
        .unwrap_or("event_operation_failed");
    serde_json::json!({"error": {"code": code, "message": message}})
}

async fn dispatch_inner(
    state: AppState,
    command: HubCommand,
    context: RequestContext<'_>,
    mut snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<serde_json::Value> {
    match command {
        HubCommand::ProcessOutput { payload, .. } => {
            match process::get_process_output(&state, payload).await {
                Ok(response) => Ok(serde_json::to_value(response)?),
                Err(reason) => Ok(serde_json::json!({
                    "error": {"code": reason.clone(), "message": reason}
                })),
            }
        }
        HubCommand::ProcessResult { payload, .. } => {
            match process::get_process_result(&state, payload).await {
                Ok(response) => Ok(serde_json::to_value(response)?),
                Err(reason) => Ok(serde_json::json!({
                    "error": {"code": reason.clone(), "message": reason}
                })),
            }
        }
        HubCommand::EventList { mut payload, .. } => {
            if !bind_current_agent_id(&state, &mut payload.agent_id).await {
                return Ok(event_agent_mismatch());
            }
            match state.event_store.list(&payload) {
                Ok(response) => Ok(serde_json::to_value(response)?),
                Err(error) => Ok(event_store_error_value(error)),
            }
        }
        HubCommand::EventGet { mut payload, .. } => {
            if !bind_current_agent_id(&state, &mut payload.agent_id).await {
                return Ok(event_agent_mismatch());
            }
            match state.event_store.get(&payload.event_id) {
                Ok(record) => Ok(serde_json::to_value(record)?),
                Err(error) => Ok(event_store_error_value(error)),
            }
        }
        HubCommand::EventMark { mut payload, .. } => {
            if !bind_current_agent_id(&state, &mut payload.agent_id).await {
                return Ok(event_agent_mismatch());
            }
            match state.event_store.mark(&payload.event_ids) {
                Ok(response) => Ok(serde_json::to_value(response)?),
                Err(error) => Ok(event_store_error_value(error)),
            }
        }
        HubCommand::EventPanel { .. } => Ok(serde_json::to_value(state.event_store.panel()?)?),
        HubCommand::EventSettle { payload, .. } => settle_remote_response(&state, &payload),

        HubCommand::ProcessList { mut payload, .. } => {
            payload.group = match normalize_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            match process::get_process_list(&state, payload).await {
                Ok(page) => slim_process_list_response(page),
                Err(reason) => Ok(serde_json::json!({
                    "error": { "code": reason.clone(), "message": reason }
                })),
            }
        }
        HubCommand::ProcessStatus { payload, .. } => {
            match process::get_process_status(&state, payload).await {
                Ok(status) => slim_process_status_response(status, snapshots.as_deref_mut()),
                Err(reason) => Ok(serde_json::json!({
                    "error": {"code": reason.clone(), "message": reason}
                })),
            }
        }
        HubCommand::ProcessCancel { payload, .. } => {
            match process::cancel_process(&state, &payload.process_id).await {
                Ok(response) => slim_process_cancel_response(response),
                Err(reason) => Ok(serde_json::json!({
                    "error": {"code": reason, "message": reason}
                })),
            }
        }

        HubCommand::TmuxListSessions { .. } => Ok(tmux::list_sessions().await),
        HubCommand::TmuxListPanes { payload, .. } => Ok(tmux::list_panes(payload).await),
        HubCommand::TmuxCapturePane { payload, .. } => Ok(tmux::capture_pane(payload).await),
        HubCommand::TmuxPasteText { payload, .. } => {
            Ok(tmux::paste_text(&state, payload, context).await)
        }
        HubCommand::TmuxExec { payload, .. } => Ok(tmux::exec(&state, payload, context).await),
        HubCommand::TmuxCreateSession { payload, .. } => {
            Ok(tmux::create_session(&state, payload, context).await)
        }
        HubCommand::TmuxCloseSession { payload, .. } => {
            Ok(tmux::close_session(&state, payload, context).await)
        }
        HubCommand::McpListServers { .. } => Ok(mcp::list_servers(&state).await),
        HubCommand::McpListTools { payload, .. } => match mcp::list_tools(&state, payload).await {
            Ok(result) => Ok(result),
            Err(error) => Ok(serde_json::json!({
                "error": { "code": "mcp_list_tools_failed", "message": error.to_string() }
            })),
        },
        HubCommand::McpCallTool { mut payload, .. } => {
            payload.group = match normalize_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let request_source = context.source();
            match mcp::call_tool(
                &state,
                payload,
                &request_source,
                None,
                context.event_origin.cloned(),
            )
            .await
            {
                Ok(response) => slim_mcp_response(response, snapshots.as_deref_mut()),
                Err(error) => Ok(serde_json::json!({
                    "error": { "code": "mcp_call_tool_failed", "message": error.to_string() }
                })),
            }
        }
        HubCommand::McpBatch { mut payload, .. } => {
            payload.group = match normalize_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let request_source = context.source();
            match mcp::batch::batch(
                &state,
                payload,
                &request_source,
                None,
                context.event_origin.cloned(),
            )
            .await
            {
                Ok(response) => slim_mcp_batch_response(response, snapshots.as_deref_mut()),
                Err(error) => Ok(serde_json::json!({
                    "error": { "code": "mcp_batch_failed", "message": error.to_string() }
                })),
            }
        }
        HubCommand::UserNotifyDeliver { payload, .. } => Ok(serde_json::to_value(
            notify::deliver_freedesktop_notification(payload).await,
        )?),
        HubCommand::RoomDiaryActive { payload, .. } => map_room_read_result(
            room_reads::diary_active(&state, payload).await,
            "room_diary_active_failed",
        ),
        HubCommand::RoomDiaryRead { payload, .. } => map_room_read_result(
            room_reads::diary_read(&state, payload).await,
            "room_diary_read_failed",
        ),
        HubCommand::RoomNotebookRecent { payload, .. } => map_room_read_result(
            room_reads::notebook_recent(&state, payload).await,
            "room_notebook_recent_failed",
        ),
        HubCommand::RoomNotebookSearch { payload, .. } => map_room_read_result(
            room_reads::notebook_search(&state, payload).await,
            "room_notebook_search_failed",
        ),
        HubCommand::RoomNotebookRead { payload, .. } => map_room_read_result(
            room_reads::notebook_read(&state, payload).await,
            "room_notebook_read_failed",
        ),
        HubCommand::RoomStateList { payload, .. } => map_room_read_result(
            room_reads::state_list(&state, payload).await,
            "room_state_list_failed",
        ),
        HubCommand::RoomStateRead { payload, .. } => map_room_read_result(
            room_reads::state_read(&state, payload).await,
            "room_state_read_failed",
        ),
        HubCommand::RoomMaintenanceStatus { payload, .. } => {
            map_room_maintenance_result(room_maintenance::status(&state, payload).await, "status")
        }
        HubCommand::RoomMaintenanceSubmit { payload, .. } => {
            map_room_maintenance_result(room_maintenance::submit(&state, payload).await, "submit")
        }
        HubCommand::RoomBootstrap { .. } | HubCommand::Bootstrap { .. } => {
            map_bootstrap_result(bootstrap::load(&state).await, "bootstrap_read_failed")
        }
        HubCommand::RoomBootstrapRead { payload, .. }
        | HubCommand::BootstrapRead { payload, .. } => map_bootstrap_result(
            bootstrap::read(&state, payload).await,
            "bootstrap_read_failed",
        ),
        HubCommand::SkillsList { .. } => {
            map_skills_result(skills::list(&state).await, "skills_list_failed")
        }
        HubCommand::SkillsRead { payload, .. } => {
            map_skills_result(skills::read(&state, payload).await, "skills_read_failed")
        }
        HubCommand::SkillsSearch { payload, .. } => map_skills_result(
            skills::search(&state, payload).await,
            "skills_search_failed",
        ),
        HubCommand::SkillsActive { .. } => {
            map_skills_result(skills::active(&state).await, "skills_active_failed")
        }
        HubCommand::SkillsActivate { payload, .. } => map_skills_result(
            skills::activate(&state, payload).await,
            "skills_activate_failed",
        ),
        HubCommand::SkillsDeactivate { payload, .. } => map_skills_result(
            skills::deactivate(&state, payload).await,
            "skills_deactivate_failed",
        ),
        HubCommand::SkillsInstall { payload, .. } => map_install_result(
            state
                .skill_installs
                .start_with_origin(state.clone(), payload, context.event_origin.cloned())
                .await,
        ),
        HubCommand::SkillsInstallGet { payload, .. } => {
            map_install_result(state.skill_installs.get(&state, payload).await)
        }
        HubCommand::SkillsInstallCancel { payload, .. } => {
            map_install_result(state.skill_installs.cancel(&state, payload).await)
        }
        HubCommand::SkillsRun { mut payload, .. } => {
            payload.group = match normalize_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let request_source = context.source();
            match skills::run(
                state.clone(),
                payload,
                &request_source,
                None,
                context.event_origin.cloned(),
            )
            .await
            {
                Ok(response) => slim_process_response(response, snapshots),
                Err(error) => Ok(skills::skill_run_command_error(error)),
            }
        }
        HubCommand::Exec { .. } | HubCommand::ProcessBatch { .. } => {
            unreachable!("process commands are dispatched before dispatch_inner")
        }
    }
}

fn normalize_group(
    group: Option<String>,
) -> std::result::Result<Option<String>, serde_json::Value> {
    normalize_process_group(group.as_deref()).map_err(|error| {
        serde_json::json!({
            "error": {"code": error.code(), "message": error.message()}
        })
    })
}
fn map_room_read_result<T: serde::Serialize>(
    result: std::result::Result<T, anyhow::Error>,
    default_code: &str,
) -> Result<serde_json::Value> {
    Ok(match result {
        Ok(result) => serde_json::to_value(result)?,
        Err(error) => room_read_error(default_code, error),
    })
}

fn room_read_error(default_code: &str, error: anyhow::Error) -> serde_json::Value {
    let reason = error.to_string();
    let code = reason
        .split([':', ';'])
        .next()
        .filter(|value| {
            value.starts_with("room_")
                && value.len() <= 128
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
        .unwrap_or(default_code);
    let message = reason.chars().take(512).collect::<String>();
    serde_json::json!({ "error": { "code": code, "message": message } })
}

fn map_room_maintenance_result<T: serde::Serialize>(
    result: std::result::Result<T, anyhow::Error>,
    operation: &str,
) -> Result<serde_json::Value> {
    Ok(match result {
        Ok(result) => serde_json::to_value(result)?,
        Err(error) => room_maintenance_error(operation, error),
    })
}

pub(crate) fn room_maintenance_error(
    operation: &str,
    error: impl std::fmt::Display,
) -> serde_json::Value {
    let reason = error.to_string();
    let detail = reason.chars().take(384).collect::<String>();
    let message = format!("room maintenance {operation} failed: {detail}")
        .chars()
        .take(512)
        .collect::<String>();
    serde_json::json!({
        "error": {
            "code": "room_maintenance_failed",
            "message": message
        }
    })
}

fn map_bootstrap_result<T: serde::Serialize>(
    result: std::result::Result<T, anyhow::Error>,
    default_code: &str,
) -> Result<serde_json::Value> {
    Ok(match result {
        Ok(result) => serde_json::to_value(result)?,
        Err(error) => bootstrap_command_error(default_code, error),
    })
}

fn map_skills_result<T: serde::Serialize>(
    result: std::result::Result<T, anyhow::Error>,
    default_code: &str,
) -> Result<serde_json::Value> {
    Ok(match result {
        Ok(result) => serde_json::to_value(result)?,
        Err(error) => skills_command_error(default_code, error),
    })
}

fn map_install_result<T: serde::Serialize>(
    result: std::result::Result<T, anyhow::Error>,
) -> Result<serde_json::Value> {
    Ok(match result {
        Ok(result) => serde_json::to_value(result)?,
        Err(error) => install_command_error(error),
    })
}

fn skills_command_error(default_code: &str, error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = match message.as_str() {
        "invalid_id" | "query_required" => "validation_error",
        "not_found" => "not_found",
        _ => default_code,
    };
    serde_json::json!({
        "error": {
            "code": code,
            "message": if code == "not_found" { "skill not found" } else { &message }
        }
    })
}

fn bootstrap_command_error(default_code: &str, error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = match message.as_str() {
        "bootstrap_not_found"
        | "guide_not_found"
        | "bootstrap_invalid"
        | "bootstrap_read_failed" => message.as_str(),
        _ => default_code,
    };
    serde_json::json!({ "error": { "code": code, "message": message } })
}

fn install_command_error(error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = match message.as_str() {
        "install_not_found" => "install_not_found",
        "target_exists" => "target_exists",
        "idempotency_conflict" => "idempotency_conflict",
        "reserved_id" => "reserved_id",
        "invalid_id"
        | "invalid_files"
        | "invalid_file_source"
        | "invalid_path"
        | "duplicate_path"
        | "invalid_base64"
        | "package_limit_exceeded"
        | "download_blocked"
        | "invalid_github_source"
        | "invalid_github_repository"
        | "invalid_github_url"
        | "unsupported_github_host"
        | "ambiguous_github_url"
        | "invalid_idempotency_key" => "validation_error",
        _ => "skills_install_failed",
    };
    serde_json::json!({ "error": { "code": code, "message": message } })
}
