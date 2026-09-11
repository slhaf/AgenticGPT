use agentic_gpt_protocol::{normalize_job_group, HubCommand};
use anyhow::Result;

use crate::{bootstrap, config::ToolNamespace, jobs, mcp, notify, skills, tmux, AppState};

/// Value-returning local operation layer shared by transport adapters.
///
/// The Hub adapter owns request/response envelopes and transport acknowledgements. This module
/// owns the operation result and error shape so the stdio adapter can call the same code later.
pub(crate) async fn dispatch(state: AppState, command: HubCommand) -> Result<serde_json::Value> {
    match dispatch_inner(state, command).await {
        Err(error) if error.to_string() == "room_toolset_required" => {
            Ok(room_toolset_required_error())
        }
        Err(error) if error.to_string() == "room_agent_required" => {
            Ok(profile_capability_required_error())
        }
        result => result,
    }
}

async fn dispatch_inner(state: AppState, command: HubCommand) -> Result<serde_json::Value> {
    match command {
        HubCommand::Exec { mut payload, .. } => {
            payload.group = match normalize_hub_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let response = jobs::start_and_wait_process(
                state,
                payload,
                jobs::ManagedJobOptions::for_source("hub:process.exec"),
            )
            .await;
            crate::stdio_server::slim_process_response(serde_json::to_value(response)?)
        }
        HubCommand::ProcessBatch { mut payload, .. } => {
            payload.group = match normalize_hub_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            match jobs::start_process_batch(state, payload, "hub:process.batch".to_string(), None)
                .await
            {
                Ok(response) => crate::stdio_server::slim_process_batch_response(response),
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
                Ok(page) => crate::stdio_server::slim_job_list_response(page),
                Err(reason) => Ok(serde_json::json!({
                    "error": { "code": reason.clone(), "message": reason }
                })),
            }
        }
        HubCommand::JobGet { payload, .. } => {
            let wait_seconds = payload.wait_seconds.unwrap_or(0).min(30);
            match jobs::get_job_detail(&state, &payload.job_id, wait_seconds).await {
                Ok(job) => {
                    crate::stdio_server::slim_job_get_response(job, payload.wait_only, wait_seconds)
                }
                Err(reason) => Ok(serde_json::json!({
                    "error": {"code": reason.clone(), "message": reason}
                })),
            }
        }
        HubCommand::JobCancel { payload, .. } => {
            match jobs::cancel_job(&state, &payload.job_id).await {
                Ok(job) => crate::stdio_server::slim_cancel_response(job),
                Err(reason) => Ok(serde_json::json!({
                    "error": {"code": reason, "message": reason}
                })),
            }
        }
        HubCommand::TmuxListSessions { .. } => Ok(tmux::list_sessions().await),
        HubCommand::TmuxListPanes { payload, .. } => Ok(tmux::list_panes(payload).await),
        HubCommand::TmuxCapturePane { payload, .. } => Ok(tmux::capture_pane(payload).await),
        HubCommand::TmuxPasteText { payload, .. } => Ok(tmux::paste_text(&state, payload).await),
        HubCommand::TmuxExec { payload, .. } => Ok(tmux::exec(&state, payload).await),
        HubCommand::TmuxCreateSession { payload, .. } => {
            Ok(tmux::create_session(&state, payload).await)
        }
        HubCommand::TmuxCloseSession { payload, .. } => {
            Ok(tmux::close_session(&state, payload).await)
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
            match mcp::call_tool(&state, payload, "hub:mcp", None).await {
                Ok(result) => crate::stdio_server::slim_mcp_response(result),
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
            match mcp::batch(&state, payload, "hub:mcp.batch", None).await {
                Ok(result) => crate::stdio_server::slim_mcp_batch_response(result),
                Err(error) => Ok(serde_json::json!({
                    "error": { "code": "mcp_batch_failed", "message": error.to_string() }
                })),
            }
        }
        HubCommand::UserNotifyDeliver { payload, .. } => {
            if !state.runtime.capabilities().notifications {
                return Ok(capability_error("user.notify.deliver"));
            }
            Ok(serde_json::to_value(
                notify::deliver_freedesktop_notification(payload).await,
            )?)
        }
        HubCommand::RoomNotebookAppend { .. }
        | HubCommand::RoomNotebookRecent { .. }
        | HubCommand::RoomNotebookSelectExact { .. }
        | HubCommand::RoomNotebookSearch { .. }
        | HubCommand::RoomNotebookCurrent { .. }
        | HubCommand::RoomNotebookUpdate { .. }
        | HubCommand::RoomNotebookRemove { .. }
        | HubCommand::RoomDiaryAppend { .. }
        | HubCommand::RoomDiaryRecent { .. }
        | HubCommand::RoomDiarySelectExact { .. } => Ok(legacy_room_surface_removed_error()),
        HubCommand::RoomBootstrap { .. } | HubCommand::Bootstrap { .. } => {
            require_room_toolset(&state).await?;
            map_bootstrap_result(bootstrap::load(&state).await, "bootstrap_read_failed")
        }
        HubCommand::RoomBootstrapRead { payload, .. }
        | HubCommand::BootstrapRead { payload, .. } => {
            require_room_toolset(&state).await?;
            map_bootstrap_result(
                bootstrap::read(&state, payload).await,
                "bootstrap_read_failed",
            )
        }
        HubCommand::SkillsList { .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            map_skills_result(skills::list(&state).await, "skills_list_failed")
        }
        HubCommand::SkillsRead { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            map_skills_result(skills::read(&state, payload).await, "skills_read_failed")
        }
        HubCommand::SkillsSearch { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            map_skills_result(
                skills::search(&state, payload).await,
                "skills_search_failed",
            )
        }
        HubCommand::SkillsActive { .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            map_skills_result(skills::active(&state).await, "skills_active_failed")
        }
        HubCommand::SkillsActivate { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            map_skills_result(
                skills::activate(&state, payload).await,
                "skills_activate_failed",
            )
        }
        HubCommand::SkillsDeactivate { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            map_skills_result(
                skills::deactivate(&state, payload).await,
                "skills_deactivate_failed",
            )
        }
        HubCommand::SkillsInstall { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            map_install_result(state.skill_installs.start(state.clone(), payload).await)
        }
        HubCommand::SkillsInstallGet { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            map_install_result(state.skill_installs.get(&state, payload).await)
        }
        HubCommand::SkillsInstallCancel { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            map_install_result(state.skill_installs.cancel(&state, payload).await)
        }
        HubCommand::SkillsRun { mut payload, .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            payload.group = match normalize_hub_group(payload.group) {
                Ok(group) => group,
                Err(error) => return Ok(error),
            };
            let value = crate::hub::run_skill(&state, payload).await;
            if value.get("error").is_some() {
                Ok(value)
            } else {
                crate::stdio_server::slim_process_response(value)
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

fn room_toolset_required_error() -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": "room_toolset_required",
            "message": "room commands require toolsets.room to be enabled"
        }
    })
}

fn profile_capability_required_error() -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": "room_agent_required",
            "message": "room commands require profile=room in config"
        }
    })
}

async fn require_room_toolset(state: &AppState) -> Result<()> {
    let enabled = {
        let config = state.config.read().await;
        config.toolsets.is_enabled(ToolNamespace::Room)
    };
    if enabled {
        Ok(())
    } else {
        Err(anyhow::anyhow!("room_toolset_required"))
    }
}

fn require_capability(
    state: &AppState,
    predicate: impl FnOnce(crate::state::Capabilities) -> bool,
) -> Result<()> {
    if predicate(state.runtime.capabilities()) {
        Ok(())
    } else {
        Err(anyhow::anyhow!("room_agent_required"))
    }
}

fn capability_error(name: &str) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": "capability_unavailable",
            "message": format!("{name} is unavailable for this runtime")
        }
    })
}

fn legacy_room_surface_removed_error() -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": "room_legacy_surface_removed",
            "message": "legacy Room JSONL commands are reserved for Hub parity and are not available on the Agent"
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
