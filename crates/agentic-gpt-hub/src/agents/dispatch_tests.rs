use super::*;
use crate::agents::test_support::*;
use agentic_gpt_protocol::{
    AgentConnectionMode, AgentMessage, ExecRequest, HubCommand, HubCommandEnvelope,
};
use axum::body::to_bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::params;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::agents::transport::{post_agent_message, SseConnectQuery};
use crate::runs;
use crate::state::OutboundAgentMessage;

async fn start_response_owner_request(
    request_id: &str,
) -> (
    HubState,
    mpsc::UnboundedReceiver<OutboundAgentMessage>,
    tokio::task::JoinHandle<std::result::Result<Value, String>>,
    HubCommandEnvelope,
) {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    register_agent(&state, "foreign", "foreign-secret");
    let mut outbound = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
    let command = HubCommand::Exec {
        request_id: request_id.to_string(),
        payload: ExecRequest {
            agent_id: "agent".to_string(),
            group: None,
            program: "printf".to_string(),
            args: vec!["ok".to_string()],
            need_confirm: false,
            confirm_method: None,
            working_directory: None,
            wait_seconds: None,
        },
    };
    let request_state = state.clone();
    let caller =
        tokio::spawn(async move { request_agent(&request_state, "agent", command, 5).await });

    let OutboundAgentMessage::Text(text) = outbound.recv().await.unwrap() else {
        panic!("expected command envelope");
    };
    let envelope = serde_json::from_str::<HubCommandEnvelope>(&text).unwrap();
    while runs::get_run(&state, &envelope.run_id)
        .unwrap()
        .unwrap()
        .status
        != "dispatched"
    {
        tokio::task::yield_now().await;
    }
    (state, outbound, caller, envelope)
}

async fn post_response(
    state: &HubState,
    routed_agent_id: &str,
    secret: &str,
    run_id: Option<String>,
    request_id: String,
    data: Value,
) -> Response {
    post_agent_message(
        State(state.clone()),
        Path(routed_agent_id.to_string()),
        Query(SseConnectQuery::for_test(Some("current".to_string()))),
        agent_headers(secret),
        axum::Json(AgentMessage::Response {
            run_id,
            request_id,
            data,
        }),
    )
    .await
}

async fn rejected_reason(response: Response) -> String {
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["error"]["code"], "agent_message_rejected");
    value["error"]["message"].as_str().unwrap().to_string()
}
#[tokio::test]
async fn pending_replay_sends_reliable_envelope() {
    let state = test_state();
    let command = HubCommand::Exec {
        request_id: "req_replay".to_string(),
        payload: ExecRequest {
            agent_id: "agent".to_string(),
            group: None,
            program: "printf".to_string(),
            args: vec!["ok".to_string()],
            need_confirm: false,
            confirm_method: None,
            working_directory: None,
            wait_seconds: None,
        },
    };
    let run = runs::prepare_run(&state, "agent", "req_replay", &command).unwrap();
    runs::mark_dispatched(&state, &run.run_id).unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();

    send_pending_replays(&state, "agent", &tx).await;

    let OutboundAgentMessage::Text(text) = rx.recv().await.unwrap() else {
        panic!("expected replay envelope");
    };
    let envelope = serde_json::from_str::<HubCommandEnvelope>(&text).unwrap();
    assert_eq!(envelope.run_id, run.run_id);
    assert_eq!(envelope.request_id, "req_replay");
    assert_eq!(envelope.command_hash, run.command_hash);
    assert!(matches!(envelope.command, HubCommand::Exec { .. }));
    assert!(rx.try_recv().is_err());
}
#[tokio::test]
async fn stale_response_with_matching_run_is_accepted() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let _rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
    let command = HubCommand::Exec {
        request_id: "req_late".to_string(),
        payload: ExecRequest {
            agent_id: "agent".to_string(),
            group: None,
            program: "printf".to_string(),
            args: vec!["ok".to_string()],
            need_confirm: false,
            confirm_method: None,
            working_directory: None,
            wait_seconds: None,
        },
    };
    let run = runs::prepare_run(&state, "agent", "req_late", &command).unwrap();

    let response = post_agent_message(
        State(state.clone()),
        Path("agent".to_string()),
        Query(SseConnectQuery::for_test(Some("old".to_string()))),
        agent_headers("secret"),
        axum::Json(AgentMessage::Response {
            run_id: Some(run.run_id.clone()),
            request_id: "req_late".to_string(),
            data: json!({ "ok": true }),
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let stored = runs::get_run(&state, &run.run_id).unwrap().unwrap();
    assert_eq!(stored.status, "completed");
    assert_eq!(stored.result, Some(json!({ "ok": true })));
}
#[tokio::test]
async fn response_owner_rejects_unmatched_without_consuming_waiter() {
    for case in ["nonexistent_run", "wrong_request", "missing_run_id"] {
        let request_id = format!("req_response_owner_unmatched_{case}");
        let (state, _outbound, caller, envelope) = start_response_owner_request(&request_id).await;
        let (routed_agent_id, secret, invalid_run_id, invalid_request_id, expected_reason) =
            match case {
                "nonexistent_run" => (
                    "foreign",
                    "foreign-secret",
                    Some(format!("{}-missing", envelope.run_id)),
                    envelope.request_id.clone(),
                    "response_run_mismatch",
                ),
                "wrong_request" => (
                    "agent",
                    "secret",
                    Some(envelope.run_id.clone()),
                    format!("{}-wrong", envelope.request_id),
                    "response_run_mismatch",
                ),
                "missing_run_id" => (
                    "agent",
                    "secret",
                    None,
                    envelope.request_id.clone(),
                    "response_run_id_required",
                ),
                _ => unreachable!(),
            };

        let response = post_response(
            &state,
            routed_agent_id,
            secret,
            invalid_run_id,
            invalid_request_id,
            json!({ "servers": ["mismatched"] }),
        )
        .await;
        assert_eq!(rejected_reason(response).await, expected_reason);
        assert!(!caller.is_finished());
        assert!(runs::get_run(&state, &envelope.run_id)
            .unwrap()
            .unwrap()
            .result
            .is_none());

        let valid_data = json!({ "servers": [] });
        let response = post_response(
            &state,
            "agent",
            "secret",
            Some(envelope.run_id.clone()),
            envelope.request_id.clone(),
            valid_data.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(caller.await.unwrap().unwrap(), valid_data);

        let stored = runs::get_run(&state, &envelope.run_id).unwrap().unwrap();
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.result, Some(valid_data));
    }
}
#[tokio::test]
async fn response_owner_isolates_runs_sharing_request_id() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut outbound = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
    let request_id = "req_response_owner_shared_request".to_string();
    let make_command = || HubCommand::Exec {
        request_id: request_id.clone(),
        payload: ExecRequest {
            agent_id: "agent".to_string(),
            group: None,
            program: "printf".to_string(),
            args: vec!["ok".to_string()],
            need_confirm: false,
            confirm_method: None,
            working_directory: None,
            wait_seconds: None,
        },
    };

    let first_command = make_command();
    let second_command = make_command();
    let first_state = state.clone();
    let first_caller =
        tokio::spawn(async move { request_agent(&first_state, "agent", first_command, 5).await });
    let OutboundAgentMessage::Text(first_text) = outbound.recv().await.unwrap() else {
        panic!("expected first command envelope");
    };
    let first_envelope = serde_json::from_str::<HubCommandEnvelope>(&first_text).unwrap();

    let second_state = state.clone();
    let second_caller =
        tokio::spawn(async move { request_agent(&second_state, "agent", second_command, 5).await });
    let OutboundAgentMessage::Text(second_text) = outbound.recv().await.unwrap() else {
        panic!("expected second command envelope");
    };
    let second_envelope = serde_json::from_str::<HubCommandEnvelope>(&second_text).unwrap();

    assert_eq!(first_envelope.request_id, request_id);
    assert_eq!(second_envelope.request_id, request_id);
    assert_ne!(first_envelope.run_id, second_envelope.run_id);
    assert!(!first_envelope.event_id.is_empty());
    assert!(!second_envelope.event_id.is_empty());
    assert!(!first_envelope.command_hash.is_empty());
    assert!(!second_envelope.command_hash.is_empty());

    let first_data = json!({ "runId": first_envelope.run_id.clone() });
    let second_data = json!({ "runId": second_envelope.run_id.clone() });

    let second_response = post_response(
        &state,
        "agent",
        "secret",
        Some(second_envelope.run_id.clone()),
        second_envelope.request_id.clone(),
        second_data.clone(),
    )
    .await;
    assert_eq!(second_response.status(), StatusCode::OK);

    let first_response = post_response(
        &state,
        "agent",
        "secret",
        Some(first_envelope.run_id.clone()),
        first_envelope.request_id.clone(),
        first_data.clone(),
    )
    .await;
    assert_eq!(first_response.status(), StatusCode::OK);

    let (first_joined, second_joined) = tokio::join!(first_caller, second_caller);
    let first_result = first_joined.unwrap().unwrap();
    let second_result = second_joined.unwrap().unwrap();
    assert_eq!(first_result, first_data);
    assert_eq!(second_result, second_data);

    let first_run = runs::get_run(&state, &first_envelope.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(first_run.status, "completed");
    assert_eq!(first_run.result, Some(first_data));

    let second_run = runs::get_run(&state, &second_envelope.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(second_run.status, "completed");
    assert_eq!(second_run.result, Some(second_data));
}
#[tokio::test]
async fn response_owner_timeout_preserves_other_run_and_accepts_late_result() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut outbound = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
    let request_id = "req_response_owner_timeout_shared_request".to_string();
    let make_command = || HubCommand::Exec {
        request_id: request_id.clone(),
        payload: ExecRequest {
            agent_id: "agent".to_string(),
            group: None,
            program: "printf".to_string(),
            args: vec!["ok".to_string()],
            need_confirm: false,
            confirm_method: None,
            working_directory: None,
            wait_seconds: None,
        },
    };
    let long_command = make_command();
    let short_command = make_command();
    let long_state = state.clone();
    let long_caller =
        tokio::spawn(async move { request_agent(&long_state, "agent", long_command, 5).await });
    let OutboundAgentMessage::Text(long_text) = outbound.recv().await.unwrap() else {
        panic!("expected long-running command envelope");
    };
    let long_envelope = serde_json::from_str::<HubCommandEnvelope>(&long_text).unwrap();

    let short_state = state.clone();
    let short_caller =
        tokio::spawn(async move { request_agent(&short_state, "agent", short_command, 0).await });
    let OutboundAgentMessage::Text(short_text) = outbound.recv().await.unwrap() else {
        panic!("expected short-running command envelope");
    };
    let short_envelope = serde_json::from_str::<HubCommandEnvelope>(&short_text).unwrap();
    assert_eq!(long_envelope.request_id, request_id);
    assert_eq!(short_envelope.request_id, request_id);
    assert_ne!(long_envelope.run_id, short_envelope.run_id);

    let short_error = short_caller.await.unwrap().unwrap_err();
    assert_eq!(
        short_error,
        format!("process_exec_timeout; runId={}", short_envelope.run_id)
    );
    assert!(!long_caller.is_finished());

    let short_data = json!({ "runId": short_envelope.run_id.clone() });
    let response = post_response(
        &state,
        "agent",
        "secret",
        Some(short_envelope.run_id.clone()),
        short_envelope.request_id.clone(),
        short_data.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!long_caller.is_finished());
    let short_run = runs::get_run(&state, &short_envelope.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(short_run.status, "completed");
    assert_eq!(short_run.result, Some(short_data));

    let long_data = json!({ "runId": long_envelope.run_id.clone() });
    let response = post_response(
        &state,
        "agent",
        "secret",
        Some(long_envelope.run_id.clone()),
        long_envelope.request_id.clone(),
        long_data.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(long_caller.await.unwrap().unwrap(), long_data.clone());
    let long_run = runs::get_run(&state, &long_envelope.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(long_run.status, "completed");
    assert_eq!(long_run.result, Some(long_data));
}
#[tokio::test]
async fn response_owner_checks_pending_hash_against_durable_run() {
    let (state, _outbound, caller, envelope) =
        start_response_owner_request("req_response_owner_pending_hash").await;
    state
        .dispatch
        .pending
        .lock()
        .await
        .get_mut(&envelope.run_id)
        .expect("request waiter should be present")
        .command_hash = "tampered-command-hash".to_string();

    let data = json!({ "servers": ["canonical"] });
    let response = post_response(
        &state,
        "agent",
        "secret",
        Some(envelope.run_id.clone()),
        envelope.request_id.clone(),
        data.clone(),
    )
    .await;
    assert_eq!(
        rejected_reason(response).await,
        "response_waiter_owner_mismatch"
    );
    assert!(!caller.is_finished());
    let stored = runs::get_run(&state, &envelope.run_id).unwrap().unwrap();
    assert_eq!(stored.status, "completed");
    assert_eq!(stored.result, Some(data.clone()));

    state
        .dispatch
        .pending
        .lock()
        .await
        .get_mut(&envelope.run_id)
        .expect("mismatched waiter should be retained")
        .command_hash = envelope.command_hash.clone();
    let response = post_response(
        &state,
        "agent",
        "secret",
        Some(envelope.run_id.clone()),
        envelope.request_id.clone(),
        data.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(caller.await.unwrap().unwrap(), data);
}
#[tokio::test]
async fn response_owner_rejects_conflict_and_accepts_duplicate() {
    let (state, _outbound, caller, envelope) =
        start_response_owner_request("req_response_owner_conflict").await;
    let canonical = json!({ "servers": ["canonical"] });
    assert!(matches!(
        runs::store_result(
            &state,
            "agent",
            &envelope.run_id,
            &envelope.request_id,
            &canonical,
        )
        .unwrap(),
        runs::StoreResultOutcome::Stored { .. }
    ));

    let conflict = json!({ "servers": ["conflict"] });
    let response = post_response(
        &state,
        "agent",
        "secret",
        Some(envelope.run_id.clone()),
        envelope.request_id.clone(),
        conflict.clone(),
    )
    .await;
    assert_eq!(rejected_reason(response).await, "response_result_conflict");
    assert!(!caller.is_finished());

    let stored = runs::get_run(&state, &envelope.run_id).unwrap().unwrap();
    assert_eq!(stored.result, Some(canonical.clone()));
    let conflict_json: String = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "select conflict_json from agent_runs where run_id = ?1",
            params![envelope.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&conflict_json).unwrap(),
        conflict
    );

    let response = post_response(
        &state,
        "agent",
        "secret",
        Some(envelope.run_id.clone()),
        envelope.request_id.clone(),
        canonical.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(caller.await.unwrap().unwrap(), canonical);
}
#[tokio::test]
async fn response_owner_keeps_waiter_on_store_failure() {
    let (state, _outbound, caller, envelope) =
        start_response_owner_request("req_response_owner_store_failure").await;
    state
        .db
        .lock()
        .unwrap()
        .execute_batch(
            "create trigger force_result_write_failure
                 before update of result_json on agent_runs
                 begin
                     select raise(abort, 'forced_result_write_failure');
                 end;",
        )
        .unwrap();

    let data = json!({ "servers": ["retry"] });
    let response = post_response(
        &state,
        "agent",
        "secret",
        Some(envelope.run_id.clone()),
        envelope.request_id.clone(),
        data.clone(),
    )
    .await;
    assert_eq!(
        rejected_reason(response).await,
        "response_result_store_failed"
    );
    assert!(!caller.is_finished());
    assert!(runs::get_run(&state, &envelope.run_id)
        .unwrap()
        .unwrap()
        .result
        .is_none());

    state
        .db
        .lock()
        .unwrap()
        .execute_batch("drop trigger force_result_write_failure;")
        .unwrap();
    let response = post_response(
        &state,
        "agent",
        "secret",
        Some(envelope.run_id.clone()),
        envelope.request_id.clone(),
        data.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(caller.await.unwrap().unwrap(), data.clone());
    let stored = runs::get_run(&state, &envelope.run_id).unwrap().unwrap();
    assert_eq!(stored.status, "completed");
    assert_eq!(stored.result, Some(data));
}
#[tokio::test]
async fn failed_request_send_removes_current_connection() {
    let state = test_state();
    let rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
    drop(rx);
    let command = HubCommand::Exec {
        request_id: "req_send_failed".to_string(),
        payload: ExecRequest {
            agent_id: "agent".to_string(),
            group: None,
            program: "printf".to_string(),
            args: vec!["ok".to_string()],
            need_confirm: false,
            confirm_method: None,
            working_directory: None,
            wait_seconds: None,
        },
    };

    let result = request_agent(&state, "agent", command, 1).await;

    assert_eq!(result.unwrap_err(), "agent_offline");
    assert!(!state.agents.snapshot_for_test().await.contains_key("agent"));
    let run_id: String = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "select run_id from agent_runs where request_id = ?1",
            params!["req_send_failed"],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!state.dispatch.pending.lock().await.contains_key(&run_id));
}
#[tokio::test]
async fn reporting_only_connection_is_not_a_command_target() {
    let state = test_state();
    let _rx = insert_connection(&state, "agent", "reporting", chrono::Utc::now()).await;
    let mut connection = state
        .agents
        .snapshot_for_test()
        .await
        .remove("agent")
        .unwrap();
    connection.connection_mode = AgentConnectionMode::ReportingOnly;
    state.agents.insert_for_test("agent", connection).await;
    let command = HubCommand::Exec {
        request_id: "req_reporting_only".to_string(),
        payload: ExecRequest {
            agent_id: "agent".to_string(),
            group: None,
            program: "printf".to_string(),
            args: vec!["blocked".to_string()],
            need_confirm: false,
            confirm_method: None,
            working_directory: None,
            wait_seconds: None,
        },
    };

    let result = request_agent(&state, "agent", command, 1).await;

    assert_eq!(result.unwrap_err(), "agent_reporting_only");
    let run_count: i64 = state
        .db
        .lock()
        .unwrap()
        .query_row("select count(*) from agent_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(run_count, 0);
}
#[tokio::test]
async fn wp1_send_failure_marks_not_sent_and_excludes_replay() {
    let state = test_state();
    let rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
    drop(rx);
    let command = HubCommand::Exec {
        request_id: "req_wp1_send_failure".to_string(),
        payload: ExecRequest {
            agent_id: "agent".to_string(),
            group: None,
            program: "printf".to_string(),
            args: vec!["offline".to_string()],
            need_confirm: false,
            confirm_method: None,
            working_directory: None,
            wait_seconds: None,
        },
    };

    let result = request_agent(&state, "agent", command, 1).await;
    assert_eq!(result.unwrap_err(), "agent_offline");

    let run_id: String = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "select run_id from agent_runs where request_id = ?1",
            params!["req_wp1_send_failure"],
            |row| row.get(0),
        )
        .unwrap();
    let stored = runs::get_run(&state, &run_id).unwrap().unwrap();
    assert_eq!(stored.status, "not_sent");
    assert_eq!(stored.reason.as_deref(), Some("agent_offline"));

    let (tx, mut replay_rx) = mpsc::unbounded_channel();
    send_pending_replays(&state, "agent", &tx).await;
    assert!(matches!(
        replay_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn wp1_unknown_transport_status_is_mismatch_without_mutation() {
    let state = test_state();
    let command = HubCommand::McpListServers {
        request_id: "req_wp1_unknown_status".to_string(),
    };
    let run = runs::prepare_run(&state, "agent", command.request_id(), &command).unwrap();
    runs::mark_dispatched(&state, &run.run_id).unwrap();
    let before = runs::get_run(&state, &run.run_id).unwrap().unwrap();

    let error = handle_reliable_message(
        &state,
        "agent",
        AgentMessage::TransportRunStatus {
            run_id: run.run_id.clone(),
            request_id: run.request_id.clone(),
            status: "completed".to_string(),
            reason: Some("wire_completed".to_string()),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error, "transport_status_run_mismatch");

    let after = runs::get_run(&state, &run.run_id).unwrap().unwrap();
    assert_eq!(after.status, before.status);
    assert_eq!(after.reason, before.reason);
    assert_eq!(after.updated_at, before.updated_at);
}
