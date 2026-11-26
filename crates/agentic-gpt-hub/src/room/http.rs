use agentic_gpt_protocol::{
    HubCommand, SkillActivationRequest, SkillInstallCancelRequest, SkillInstallGetRequest,
    SkillInstallRequest, SkillReadRequest, SkillRunRequest, SkillSearchRequest,
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

pub(crate) async fn room_secret_list(
    State(state): State<HubState>,
    headers: HeaderMap,
    payload: Result<
        Json<agentic_gpt_protocol::RoomSecretListRequest>,
        axum::extract::rejection::JsonRejection,
    >,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_params",
            "invalid secret request JSON",
        );
    };
    forward_room_command(
        state,
        headers,
        HubCommand::RoomSecretList {
            request_id: random_id("req"),
            payload,
        },
    )
    .await
}

pub(crate) async fn room_secret_get(
    State(state): State<HubState>,
    headers: HeaderMap,
    payload: Result<
        Json<agentic_gpt_protocol::RoomSecretGetRequest>,
        axum::extract::rejection::JsonRejection,
    >,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_params",
            "invalid secret request JSON",
        );
    };
    forward_room_command(
        state,
        headers,
        HubCommand::RoomSecretGet {
            request_id: random_id("req"),
            payload,
        },
    )
    .await
}

pub(crate) async fn room_secret_upsert(
    State(state): State<HubState>,
    headers: HeaderMap,
    payload: Result<
        Json<agentic_gpt_protocol::RoomSecretUpsertRequest>,
        axum::extract::rejection::JsonRejection,
    >,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_params",
            "invalid secret request JSON",
        );
    };
    forward_room_command(
        state,
        headers,
        HubCommand::RoomSecretUpsert {
            request_id: random_id("req"),
            payload,
        },
    )
    .await
}

pub(crate) async fn room_secret_delete(
    State(state): State<HubState>,
    headers: HeaderMap,
    payload: Result<
        Json<agentic_gpt_protocol::RoomSecretDeleteRequest>,
        axum::extract::rejection::JsonRejection,
    >,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_params",
            "invalid secret request JSON",
        );
    };
    forward_room_command(
        state,
        headers,
        HubCommand::RoomSecretDelete {
            request_id: random_id("req"),
            payload,
        },
    )
    .await
}

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
    )
    .await
}

async fn forward_room_command(
    state: HubState,
    headers: HeaderMap,
    command: HubCommand,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    match request_active_room(&state, command, REQUEST_TIMEOUT_SECS).await {
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
        "toolset_required" | "room_agent_required" => StatusCode::FORBIDDEN,
        "secret_not_found" => StatusCode::NOT_FOUND,
        "secret_confirmation_rejected" | "secret_confirmation_unavailable" => StatusCode::FORBIDDEN,
        "ambiguous_secret"
        | "secret_presence_unknown"
        | "secret_value_missing"
        | "secret_write_outcome_unknown" => StatusCode::CONFLICT,
        "secret_invalid_totp" | "secret_result_too_large" => StatusCode::BAD_REQUEST,
        "secret_not_configured"
        | "secret_authentication_failed"
        | "secret_vault_locked"
        | "secret_provider_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        "email_not_found" | "email_attachment_not_found" => StatusCode::NOT_FOUND,
        "email_policy_denied" => StatusCode::FORBIDDEN,
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
        "target_exists" | "idempotency_conflict" | "room_state_conflict" => StatusCode::CONFLICT,
        "not_found" | "skill_not_found" | "resource_not_found" | "install_not_found"
        | "room_not_active" => StatusCode::NOT_FOUND,
        "skills_read_failed"
        | "skills_write_failed"
        | "skills_list_failed"
        | "skills_search_failed"
        | "skills_active_failed"
        | "skills_activate_failed"
        | "skills_deactivate_failed"
        | "skills_install_failed" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(value)).into_response()
}
