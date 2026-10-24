//! These integration tests protect the confidential, non-replayable dispatch boundary.
//! Ordinary dispatch tests exercise durable receipts; secrets must bypass that machinery.

use agentic_gpt_protocol::{
    AgentConnectionMode, AgentMessage, AgentRole, HubCommand, HubMessage, RoomSecretDeleteRequest,
    RoomSecretGetRequest, RoomSecretListRequest, RoomSecretType, RoomSecretUpsertRequest,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use super::dispatch::{request_room, send_pending_replays, RequestError};
use super::lifecycle::handle_agent_message;
use super::test_support::*;
use super::transport::{post_agent_message, SseConnectQuery};
use crate::room::control::RoomRouteError;
use crate::state::{HubState, OutboundAgentMessage};

async fn active_room() -> (HubState, mpsc::UnboundedReceiver<OutboundAgentMessage>) {
    let state = test_state();
    register_agent(&state, "room", "room-secret");
    let outbound = insert_connection(&state, "room", "current", chrono::Utc::now()).await;
    handle_agent_message(
        &state,
        "room",
        "current",
        AgentMessage::Hello {
            role: AgentRole::Room,
            boot_generation: "secret-test-boot".to_owned(),
            connection_mode: AgentConnectionMode::CommandCapable,
            config_summary: test_config_summary(),
            notification_channels: vec![],
        },
    )
    .await
    .unwrap();
    (state, outbound)
}

fn get_command(request_id: &str) -> HubCommand {
    HubCommand::RoomSecretGet {
        request_id: request_id.to_owned(),
        payload: RoomSecretGetRequest {
            site: "example.invalid".to_owned(),
            account: "test-account".to_owned(),
            secret_type: RoomSecretType::Password,
        },
    }
}

async fn receive_secret(
    outbound: &mut mpsc::UnboundedReceiver<OutboundAgentMessage>,
) -> (String, HubCommand) {
    let message = timeout(Duration::from_secs(5), outbound.recv())
        .await
        .expect("secret dispatch did not reach the active connection")
        .expect("active connection closed");
    let OutboundAgentMessage::Text(text) = message else {
        panic!("secret dispatch closed the active connection");
    };
    match serde_json::from_str::<HubMessage>(&text).unwrap() {
        HubMessage::SecretCommand {
            dispatch_id,
            command,
        } => (dispatch_id, *command),
        _ => panic!("secret used the wrong transport message"),
    }
}

async fn respond(
    state: &HubState,
    agent_id: &str,
    secret: &str,
    connection_id: &str,
    dispatch_id: &str,
    request_id: &str,
    data: Value,
) -> StatusCode {
    post_agent_message(
        State(state.clone()),
        Path(agent_id.to_owned()),
        Query(SseConnectQuery::for_test(Some(connection_id.to_owned()))),
        agent_headers(secret),
        axum::Json(AgentMessage::SecretResponse {
            dispatch_id: dispatch_id.to_owned(),
            request_id: request_id.to_owned(),
            data,
        }),
    )
    .await
    .status()
}

async fn assert_no_durable_secret(state: &HubState) {
    {
        let conn = state.db.lock().unwrap();
        let runs: i64 = conn
            .query_row("select count(*) from agent_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(runs, 0, "secret dispatch created a durable receipt");
        let tables = conn
            .prepare("select name from sqlite_schema where type = 'table'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for table in tables {
            let mut statement = conn
                .prepare(&format!("select * from \"{}\"", table.replace('"', "\"\"")))
                .unwrap();
            let mut rows = statement.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                for index in 0..row.as_ref().column_count() {
                    if let rusqlite::types::ValueRef::Text(bytes) = row.get_ref(index).unwrap() {
                        for secret in [
                            "fixture-password-content",
                            "fixture-totp-seed",
                            "fixture-returned-value",
                        ] {
                            assert!(
                                !bytes
                                    .windows(secret.len())
                                    .any(|window| window == secret.as_bytes()),
                                "secret content entered a database table"
                            );
                        }
                    }
                }
            }
        }
    }
    assert_eq!(state.dispatch.pending_count().await, 0);
    assert!(crate::runs::pending_unacked(state, "room")
        .unwrap()
        .is_empty());
    let (sender, mut replay) = mpsc::unbounded_channel();
    send_pending_replays(state, "room", &sender).await;
    assert!(matches!(
        replay.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn secret_room_dispatch_returns_values_without_durable_receipts_or_replay() {
    let (state, mut outbound) = active_room().await;
    let commands = [
        (
            HubCommand::RoomSecretUpsert {
                request_id: "secret-upsert".to_owned(),
                payload: RoomSecretUpsertRequest {
                    site: "example.invalid".to_owned(),
                    account: "test-account".to_owned(),
                    password: Some("fixture-password-content".to_owned()),
                    totp: Some("fixture-totp-seed".to_owned()),
                },
            },
            json!({"created": true}),
        ),
        (
            get_command("secret-get"),
            json!({"value": "fixture-returned-value"}),
        ),
        (
            HubCommand::RoomSecretList {
                request_id: "secret-list".to_owned(),
                payload: RoomSecretListRequest::default(),
            },
            json!({"items": []}),
        ),
        (
            HubCommand::RoomSecretDelete {
                request_id: "secret-delete".to_owned(),
                payload: RoomSecretDeleteRequest {
                    site: "example.invalid".to_owned(),
                    account: "test-account".to_owned(),
                },
            },
            json!({"deleted": true}),
        ),
    ];
    for (command, result) in commands {
        let request_state = state.clone();
        let caller = tokio::spawn(async move { request_room(&request_state, command, 5).await });
        let (dispatch_id, command) = receive_secret(&mut outbound).await;
        assert_eq!(state.dispatch.secret_pending_count(), 1);
        assert_no_durable_secret(&state).await;
        assert_eq!(
            respond(
                &state,
                "room",
                "room-secret",
                "current",
                &dispatch_id,
                command.request_id(),
                result.clone()
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(caller.await.unwrap().unwrap(), result);
        assert_eq!(state.dispatch.secret_pending_count(), 0);
        assert_no_durable_secret(&state).await;
    }
    assert!(matches!(
        outbound.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn secret_response_requires_the_dispatch_agent_connection_and_request() {
    let (state, mut outbound) = active_room().await;
    register_agent(&state, "foreign", "foreign-secret");
    let _foreign =
        insert_connection(&state, "foreign", "foreign-current", chrono::Utc::now()).await;
    let request_state = state.clone();
    let caller =
        tokio::spawn(
            async move { request_room(&request_state, get_command("owner-request"), 5).await },
        );
    let (dispatch_id, _) = receive_secret(&mut outbound).await;
    for (agent_id, secret, connection_id, request_id, expected) in [
        (
            "foreign",
            "foreign-secret",
            "foreign-current",
            "owner-request",
            StatusCode::BAD_REQUEST,
        ),
        (
            "room",
            "room-secret",
            "stale-connection",
            "owner-request",
            StatusCode::CONFLICT,
        ),
        (
            "room",
            "room-secret",
            "current",
            "wrong-request",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(
            respond(
                &state,
                agent_id,
                secret,
                connection_id,
                &dispatch_id,
                request_id,
                json!({"value": "untrusted"})
            )
            .await,
            expected
        );
        assert_eq!(state.dispatch.secret_pending_count(), 1);
        assert!(!caller.is_finished());
    }
    let value = json!({"value": "fixture-returned-value"});
    assert_eq!(
        respond(
            &state,
            "room",
            "room-secret",
            "current",
            &dispatch_id,
            "owner-request",
            value.clone()
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(caller.await.unwrap().unwrap(), value);
    assert_no_durable_secret(&state).await;
}

fn request_error(error: RoomRouteError) -> RequestError {
    match error {
        RoomRouteError::Dispatch(error) => error,
        error => panic!("unexpected Room routing failure: {error:?}"),
    }
}

#[tokio::test]
async fn secret_timeout_and_caller_cancellation_discard_waiters_without_replay() {
    let (state, mut outbound) = active_room().await;
    for (command, expected) in [
        (get_command("timed-out-read"), "agent_request_timeout"),
        (
            HubCommand::RoomSecretDelete {
                request_id: "timed-out-write".to_owned(),
                payload: RoomSecretDeleteRequest {
                    site: "example.invalid".to_owned(),
                    account: "test-account".to_owned(),
                },
            },
            "secret_write_outcome_unknown",
        ),
    ] {
        let error = request_room(&state, command, 0).await.unwrap_err();
        let (dispatch_id, command) = receive_secret(&mut outbound).await;
        assert_eq!(request_error(error).code, expected);
        assert_eq!(state.dispatch.secret_pending_count(), 0);
        // Late plaintext is discarded, without creating a replayable receipt.
        assert_eq!(
            respond(
                &state,
                "room",
                "room-secret",
                "current",
                &dispatch_id,
                command.request_id(),
                json!({"value": "fixture-returned-value"})
            )
            .await,
            StatusCode::OK
        );
        assert_no_durable_secret(&state).await;
    }
    let request_state = state.clone();
    let canceled = tokio::spawn(async move {
        request_room(&request_state, get_command("reused-request"), 60).await
    });
    let (old_dispatch_id, _) = receive_secret(&mut outbound).await;
    canceled.abort();
    assert!(canceled.await.unwrap_err().is_cancelled());
    assert_eq!(state.dispatch.secret_pending_count(), 0);

    let request_state = state.clone();
    let current = tokio::spawn(async move {
        request_room(&request_state, get_command("reused-request"), 5).await
    });
    let (new_dispatch_id, _) = receive_secret(&mut outbound).await;
    assert_ne!(old_dispatch_id, new_dispatch_id);
    assert_eq!(
        respond(
            &state,
            "room",
            "room-secret",
            "current",
            &old_dispatch_id,
            "reused-request",
            json!({"value": "old-value"})
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(state.dispatch.secret_pending_count(), 1);
    assert!(!current.is_finished());
    let fresh = json!({"value": "fresh-value"});
    assert_eq!(
        respond(
            &state,
            "room",
            "room-secret",
            "current",
            &new_dispatch_id,
            "reused-request",
            fresh.clone()
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(current.await.unwrap().unwrap(), fresh);
    assert_eq!(state.dispatch.secret_pending_count(), 0);
    assert_no_durable_secret(&state).await;
}
