use agentic_gpt_protocol::{normalize_job_group, HubCommand, JobInfo};
use anyhow::Result;

use crate::{
    bootstrap, jobs, mcp, notify,
    operation::{self, AdmissionError, RequestContext},
    operation_result::{
        slim_cancel_response, slim_job_get_response, slim_job_list_response,
        slim_mcp_batch_response, slim_mcp_response, slim_process_batch_response,
        slim_process_response,
    },
    room_maintenance, room_reads, skills, tmux, AppState,
};

/// Value-returning local operation layer shared by transport adapters.
///
/// Transport adapters own their envelopes and acknowledgements. This module owns shared admission,
/// operation execution, and result/error shapes for Hub and local stdio callers.
/// When supplied by a Hub command adapter, `snapshots` receives authoritative
/// JobInfo values from the typed operation result during projection. Other ingress
/// callers pass `None` and incur no snapshot allocation or cloning.
pub(crate) async fn dispatch(
    state: AppState,
    command: HubCommand,
    context: RequestContext<'_>,
    snapshots: Option<&mut Vec<JobInfo>>,
) -> Result<serde_json::Value> {
    let admission = {
        let config = state.config.read().await;
        operation::authorize(state.runtime, &config, context)
    };
    if let Err(error) = admission {
        return Ok(admission_error_value(error));
    }
    dispatch_inner(state, command, context, snapshots).await
}

fn admission_error_value(error: AdmissionError) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": error.code(),
            "message": error.message(),
        }
    })
}

async fn dispatch_inner(
    state: AppState,
    command: HubCommand,
    context: RequestContext<'_>,
    mut snapshots: Option<&mut Vec<JobInfo>>,
) -> Result<serde_json::Value> {
    match command {
        HubCommand::Exec { mut payload, .. } => {
            payload.group = match normalize_hub_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let response = jobs::start_and_wait_process(
                state,
                payload,
                jobs::ManagedJobOptions::for_source(context.source()),
            )
            .await;
            slim_process_response(response, snapshots.as_deref_mut())
        }
        HubCommand::ProcessBatch { mut payload, .. } => {
            payload.group = match normalize_hub_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let request_source = context.source();
            match jobs::start_process_batch(state, payload, request_source, None).await {
                Ok(response) => slim_process_batch_response(response, snapshots.as_deref_mut()),
                Err(reason) => Ok(serde_json::json!({
                    "error": {"code": "process_batch_rejected", "message": reason}
                })),
            }
        }
        HubCommand::JobList { mut payload, .. } => {
            payload.group = match normalize_hub_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            match jobs::list_jobs_page(&state, payload).await {
                Ok(page) => slim_job_list_response(page),
                Err(reason) => Ok(serde_json::json!({
                    "error": { "code": reason.clone(), "message": reason }
                })),
            }
        }
        HubCommand::JobGet { payload, .. } => {
            let wait_seconds = payload.wait_seconds.unwrap_or(0).min(30);
            match jobs::get_job_detail(&state, &payload.job_id, wait_seconds).await {
                Ok(job) => slim_job_get_response(
                    job,
                    payload.wait_only,
                    wait_seconds,
                    snapshots.as_deref_mut(),
                ),
                Err(reason) => Ok(serde_json::json!({
                    "error": {"code": reason.clone(), "message": reason}
                })),
            }
        }
        HubCommand::JobCancel { payload, .. } => {
            match jobs::cancel_job(&state, &payload.job_id).await {
                Ok(job) => slim_cancel_response(job, snapshots.as_deref_mut()),
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
            payload.group = match normalize_hub_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let request_source = context.source();
            match mcp::call_tool(&state, payload, &request_source, None).await {
                Ok(response) => slim_mcp_response(response, snapshots.as_deref_mut()),
                Err(error) => Ok(serde_json::json!({
                    "error": { "code": "mcp_call_tool_failed", "message": error.to_string() }
                })),
            }
        }
        HubCommand::McpBatch { mut payload, .. } => {
            payload.group = match normalize_hub_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let request_source = context.source();
            match mcp::batch(&state, payload, &request_source, None).await {
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
        HubCommand::SkillsInstall { payload, .. } => {
            map_install_result(state.skill_installs.start(state.clone(), payload).await)
        }
        HubCommand::SkillsInstallGet { payload, .. } => {
            map_install_result(state.skill_installs.get(&state, payload).await)
        }
        HubCommand::SkillsInstallCancel { payload, .. } => {
            map_install_result(state.skill_installs.cancel(&state, payload).await)
        }
        HubCommand::SkillsRun { mut payload, .. } => {
            payload.group = match normalize_hub_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let request_source = context.source();
            match skills::run(state.clone(), payload, &request_source, None).await {
                Ok(response) => slim_process_response(response, snapshots),
                Err(error) => Ok(skills::skill_run_command_error(error)),
            }
        }
    }
}

fn normalize_hub_group(
    group: Option<String>,
) -> std::result::Result<Option<String>, serde_json::Value> {
    normalize_job_group(group.as_deref()).map_err(|error| {
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
