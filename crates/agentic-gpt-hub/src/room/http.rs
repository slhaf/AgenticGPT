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
    timeout_code: &'static str,
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
        Err(RoomRouteError::Timeout(reason)) => {
            api_error(StatusCode::GATEWAY_TIMEOUT, timeout_code, reason)
        }
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
        "target_exists" | "idempotency_conflict" | "room_state_conflict" => StatusCode::CONFLICT,
        "not_found"
        | "room_notebook_not_found"
        | "room_state_entity_not_found"
        | "skill_not_found"
        | "install_not_found"
        | "bootstrap_not_found"
        | "guide_not_found"
        | "room_not_active" => StatusCode::NOT_FOUND,
        "bootstrap_read_failed" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(value)).into_response()
}
