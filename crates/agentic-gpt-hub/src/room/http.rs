use agentic_gpt_protocol::{
    BootstrapReadRequest, HubCommand, RoomDiaryActiveRequest, RoomDiaryReadRequest,
    RoomMaintenanceStatusRequest, RoomMaintenanceSubmitRequest, RoomNotebookReadRequest,
    RoomNotebookRecentRequest, RoomNotebookSearchRequest, RoomStateListRequest,
    RoomStateReadRequest, SkillActivationRequest, SkillInstallCancelRequest,
    SkillInstallGetRequest, SkillInstallRequest, SkillReadRequest, SkillRunRequest,
    SkillSearchRequest,
};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;

use super::control::{request_active_room, RoomRouteError};
use crate::routes::{api_error, require_action_auth};
use crate::state::HubState;
use crate::utils::random_id;
use crate::REQUEST_TIMEOUT_SECS;

const ROOM_TRANSPORT_MARGIN_SECS: u64 = 5;

pub(crate) async fn room_email_list(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<agentic_gpt_protocol::RoomEmailListRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomEmailList {
            request_id: random_id("req"),
            payload,
        },
        "room_email_list_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_email_read(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<agentic_gpt_protocol::RoomEmailReadRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomEmailRead {
            request_id: random_id("req"),
            payload,
        },
        "room_email_read_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_email_search(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<agentic_gpt_protocol::RoomEmailSearchRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomEmailSearch {
            request_id: random_id("req"),
            payload,
        },
        "room_email_search_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_email_attachment(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<agentic_gpt_protocol::RoomEmailAttachmentRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomEmailAttachment {
            request_id: random_id("req"),
            payload,
        },
        "room_email_attachment_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_email_send(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<agentic_gpt_protocol::RoomEmailSendRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomEmailSend {
            request_id: random_id("req"),
            payload,
        },
        "room_email_send_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_email_operate(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<agentic_gpt_protocol::RoomEmailOperateRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomEmailOperate {
            request_id: random_id("req"),
            payload,
        },
        "room_email_operate_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_diary_active(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<RoomDiaryActiveRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomDiaryActive {
            request_id: random_id("req"),
            payload,
        },
        "room_diary_active_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_diary_read(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<RoomDiaryReadRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomDiaryRead {
            request_id: random_id("req"),
            payload,
        },
        "room_diary_read_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_notebook_recent(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<RoomNotebookRecentRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookRecent {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_recent_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_notebook_search(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<RoomNotebookSearchRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookSearch {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_search_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_notebook_read(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<RoomNotebookReadRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookRead {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_read_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_state_list(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<RoomStateListRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomStateList {
            request_id: random_id("req"),
            payload,
        },
        "room_state_list_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_state_read(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<RoomStateReadRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomStateRead {
            request_id: random_id("req"),
            payload,
        },
        "room_state_read_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_maintenance_status(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<RoomMaintenanceStatusRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomMaintenanceStatus {
            request_id: random_id("req"),
            payload,
        },
        "room_maintenance_status_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_maintenance_submit(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<RoomMaintenanceSubmitRequest>,
) -> Response {
    let timeout_secs = REQUEST_TIMEOUT_SECS
        .max(u64::from(payload.effective_wait_seconds()) + ROOM_TRANSPORT_MARGIN_SECS);
    forward_room_command(
        state,
        headers,
        HubCommand::RoomMaintenanceSubmit {
            request_id: random_id("req"),
            payload,
        },
        "room_maintenance_submit_timeout",
        timeout_secs,
    )
    .await
}

pub(crate) async fn skills_list(State(state): State<HubState>, headers: HeaderMap) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsList {
            request_id: random_id("req"),
        },
        "skills_list_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_bootstrap(State(state): State<HubState>, headers: HeaderMap) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomBootstrap {
            request_id: random_id("req"),
        },
        "room_bootstrap_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn room_bootstrap_read(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<BootstrapReadRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomBootstrapRead {
            request_id: random_id("req"),
            payload,
        },
        "room_bootstrap_read_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn skills_read(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillReadRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsRead {
            request_id: random_id("req"),
            payload,
        },
        "skills_read_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn skills_search(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillSearchRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsSearch {
            request_id: random_id("req"),
            payload,
        },
        "skills_search_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn skills_active(State(state): State<HubState>, headers: HeaderMap) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsActive {
            request_id: random_id("req"),
        },
        "skills_active_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn skills_activate(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillActivationRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsActivate {
            request_id: random_id("req"),
            payload,
        },
        "skills_activate_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn skills_deactivate(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillActivationRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsDeactivate {
            request_id: random_id("req"),
            payload,
        },
        "skills_deactivate_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn skills_install(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillInstallRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsInstall {
            request_id: random_id("req"),
            payload,
        },
        "skills_install_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn skills_install_get(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillInstallGetRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsInstallGet {
            request_id: random_id("req"),
            payload,
        },
        "skills_install_get_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn skills_install_cancel(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillInstallCancelRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsInstallCancel {
            request_id: random_id("req"),
            payload,
        },
        "skills_install_cancel_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

pub(crate) async fn skills_run(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillRunRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsRun {
            request_id: random_id("req"),
            payload,
        },
        "skills_run_timeout",
        REQUEST_TIMEOUT_SECS,
    )
    .await
}

async fn forward_room_command(
    state: HubState,
    headers: HeaderMap,
    command: HubCommand,
    _timeout_code: &'static str,
    timeout_secs: u64,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    match request_active_room(&state, command, timeout_secs).await {
        Ok(value) => room_value_response(value),
        Err(RoomRouteError::NotActive) => api_error(
            StatusCode::NOT_FOUND,
            "room_not_active",
            "no active room agent",
        ),
        Err(RoomRouteError::StateConflict) => api_error(
            StatusCode::CONFLICT,
            "room_state_conflict",
            "active room state is inconsistent",
        ),
        Err(RoomRouteError::Dispatch(error)) => api_error(error.status, error.code, error),
    }
}

pub(super) fn room_value_response(value: Value) -> Response {
    let Some(code) = value
        .get("error")
        .and_then(Value::as_object)
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
    else {
        return Json(value).into_response();
    };
    let status = match code {
        "email_not_found" | "email_attachment_not_found" => StatusCode::NOT_FOUND,
        "email_disabled" | "email_policy_denied" => StatusCode::FORBIDDEN,
        code if code.starts_with("email_confirmation_") => StatusCode::FORBIDDEN,
        "email_identity_recovery_required"
        | "email_remote_message_unavailable"
        | "email_remote_operation_unsupported"
        | "email_content_expired"
        | "email_render_cache_expired" => StatusCode::CONFLICT,
        "email_renderer_unavailable" | "email_renderer_busy" | "email_password_unavailable" => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        code if code.starts_with("email_imap_") => StatusCode::SERVICE_UNAVAILABLE,
        "email_renderer_timeout" => StatusCode::GATEWAY_TIMEOUT,
        "email_worker_failed"
        | "email_operation_failed"
        | "email_renderer_worker_failed"
        | "email_renderer_io_failed"
        | "email_renderer_response_invalid" => StatusCode::INTERNAL_SERVER_ERROR,
        "target_exists"
        | "idempotency_conflict"
        | "room_state_conflict"
        | "room_maintenance_slot_occupied"
        | "room_maintenance_repository_dirty" => StatusCode::CONFLICT,
        "not_found"
        | "room_notebook_not_found"
        | "room_state_entity_not_found"
        | "skill_not_found"
        | "resource_not_found"
        | "install_not_found"
        | "bootstrap_not_found"
        | "guide_not_found"
        | "room_not_active" => StatusCode::NOT_FOUND,
        "bootstrap_read_failed"
        | "room_maintenance_failed"
        | "room_notebook_read_failed"
        | "room_notebook_write_failed"
        | "room_state_read_failed"
        | "room_state_write_failed"
        | "skills_read_failed"
        | "skills_write_failed"
        | "room_diary_active_failed"
        | "room_diary_read_failed"
        | "room_notebook_recent_failed"
        | "room_notebook_search_failed"
        | "room_state_list_failed"
        | "skills_list_failed"
        | "skills_search_failed"
        | "skills_active_failed"
        | "skills_activate_failed"
        | "skills_deactivate_failed"
        | "skills_install_failed" => StatusCode::INTERNAL_SERVER_ERROR,
        code if code.starts_with("room_maintenance_") && code.ends_with("_failed") => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(value)).into_response()
}
