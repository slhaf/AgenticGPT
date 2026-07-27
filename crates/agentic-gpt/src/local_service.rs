use agentic_gpt_protocol::HubCommand;
use anyhow::Result;

use crate::{bootstrap, diary, jobs, mcp, notebook, notify, skills, tmux, AppState};

/// Value-returning local operation layer shared by transport adapters.
///
/// The Hub adapter owns request/response envelopes and transport acknowledgements. This module
/// owns the operation result and error shape so the stdio adapter can call the same code later.
pub(crate) async fn dispatch(state: AppState, command: HubCommand) -> Result<serde_json::Value> {
    match dispatch_inner(state, command).await {
        Err(error) if error.to_string() == "room_agent_required" => Ok(room_agent_required_error()),
        result => result,
    }
}

async fn dispatch_inner(state: AppState, command: HubCommand) -> Result<serde_json::Value> {
    match command {
        HubCommand::Exec { payload, .. } => Ok(serde_json::to_value(
            jobs::start_and_wait_process(
                state,
                payload,
                jobs::ManagedJobOptions::for_source("hub:process.exec"),
            )
            .await,
        )?),
        HubCommand::ProcessBatch { payload, .. } => {
            match jobs::start_process_batch(state, payload, "hub:process.batch".to_string(), None)
                .await
            {
                Ok(response) => Ok(serde_json::to_value(response)?),
                Err(reason) => Ok(serde_json::json!({
                    "status": "rejected",
                    "completedInline": true,
                    "pollAfterMs": 0,
                    "error": {"code": "process_batch_rejected", "message": reason}
                })),
            }
        }
        HubCommand::JobList { payload, .. } => {
            Ok(serde_json::json!({ "jobs": jobs::list_jobs(&state, payload).await }))
        }
        HubCommand::JobGet { payload, .. } => match jobs::get_job_detail(
            &state,
            &payload.job_id,
            payload.wait_seconds.unwrap_or(0).min(30),
        )
        .await
        {
            Ok(job) => Ok(serde_json::to_value(job)?),
            Err(reason) => Ok(serde_json::json!({
                "error": {"code": reason, "message": reason}
            })),
        },
        HubCommand::JobCancel { payload, .. } => {
            match jobs::cancel_job(&state, &payload.job_id).await {
                Ok(job) => Ok(serde_json::to_value(job)?),
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
        HubCommand::McpCallTool { payload, .. } => {
            match mcp::call_tool(&state, payload, "hub:mcp", None).await {
                Ok(result) => Ok(result),
                Err(error) => Ok(serde_json::json!({
                    "error": { "code": "mcp_call_tool_failed", "message": error.to_string() }
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
        HubCommand::RoomNotebookAppend { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.notebook)?;
            map_result(
                notebook::append(&state, payload).await,
                "room_notebook_append_failed",
            )
        }
        HubCommand::RoomNotebookRecent { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.notebook)?;
            map_result(
                notebook::recent(&state, payload).await,
                "room_notebook_recent_failed",
            )
        }
        HubCommand::RoomNotebookSelectExact { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.notebook)?;
            map_result(
                notebook::select_exact(&state, payload).await,
                "room_notebook_select_exact_failed",
            )
        }
        HubCommand::RoomNotebookSearch { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.notebook)?;
            map_result(
                notebook::search(&state, payload).await,
                "room_notebook_search_failed",
            )
        }
        HubCommand::RoomNotebookCurrent { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.notebook)?;
            map_result(
                notebook::current(&state, payload).await,
                "room_notebook_current_failed",
            )
        }
        HubCommand::RoomNotebookUpdate { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.notebook)?;
            map_notebook_result(
                notebook::update(&state, payload).await,
                "room_notebook_update_failed",
            )
        }
        HubCommand::RoomNotebookRemove { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.notebook)?;
            map_notebook_result(
                notebook::remove(&state, payload).await,
                "room_notebook_remove_failed",
            )
        }
        HubCommand::RoomDiaryAppend { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.diary)?;
            map_result(
                diary::append(&state, payload).await,
                "room_diary_append_failed",
            )
        }
        HubCommand::RoomDiaryRecent { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.diary)?;
            map_result(
                diary::recent(&state, payload).await,
                "room_diary_recent_failed",
            )
        }
        HubCommand::RoomDiarySelectExact { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.diary)?;
            map_result(
                diary::select_exact(&state, payload).await,
                "room_diary_select_exact_failed",
            )
        }
        HubCommand::RoomBootstrap { .. } | HubCommand::Bootstrap { .. } => {
            require_capability(&state, |capabilities| capabilities.bootstrap)?;
            map_bootstrap_result(bootstrap::load(&state).await, "bootstrap_read_failed")
        }
        HubCommand::RoomBootstrapRead { payload, .. }
        | HubCommand::BootstrapRead { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.bootstrap)?;
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
        HubCommand::SkillsRun { payload, .. } => {
            require_capability(&state, |capabilities| capabilities.skills)?;
            Ok(crate::hub::run_skill(&state, payload).await)
        }
    }
}

fn room_agent_required_error() -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": "room_agent_required",
            "message": "room commands require run-as-room"
        }
    })
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

fn map_result<T: serde::Serialize>(
    result: std::result::Result<T, anyhow::Error>,
    default_code: &str,
) -> Result<serde_json::Value> {
    Ok(match result {
        Ok(result) => serde_json::to_value(result)?,
        Err(error) => serde_json::json!({
            "error": { "code": default_code, "message": error.to_string() }
        }),
    })
}

fn map_notebook_result<T: serde::Serialize>(
    result: std::result::Result<T, anyhow::Error>,
    default_code: &str,
) -> Result<serde_json::Value> {
    Ok(match result {
        Ok(result) => serde_json::to_value(result)?,
        Err(error) => notebook_command_error(default_code, error),
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

fn notebook_command_error(default_code: &str, error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = if message == "not_found" {
        "not_found"
    } else if message.starts_with("validation_error")
        || message.ends_with("_required")
        || message.ends_with("_too_long")
    {
        "validation_error"
    } else {
        default_code
    };
    serde_json::json!({
        "error": {
            "code": code,
            "message": if code == "not_found" { "passage not found" } else { &message }
        }
    })
}
