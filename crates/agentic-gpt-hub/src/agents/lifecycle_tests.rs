use super::*;
use crate::agents::test_support::*;
use agentic_gpt_protocol::{
    AgentConnectionMode, AgentMessage, AgentRole, AgentRunReport, ConfirmationPayload, HubCommand,
    HubCommandEnvelope, HubMessage, ProcessState, RoomNotebookReadRequest, SafeConfigSummary,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::agents::transport::{post_agent_message, SseConnectQuery};
use crate::registry::registry_entry;
use crate::runs;
use crate::state::{AgentTransport, OutboundAgentMessage};

fn generation_config_summary(workspace_root: &str) -> SafeConfigSummary {
    let mut summary = test_config_summary();
    summary.workspace_root = workspace_root.to_string();
    summary
}

pub(crate) fn generation_hello(
    role: AgentRole,
    connection_mode: AgentConnectionMode,
    boot_generation: &str,
    workspace_root: &str,
) -> AgentMessage {
    AgentMessage::Hello {
        role,
        boot_generation: boot_generation.to_string(),
        connection_mode,
        config_summary: generation_config_summary(workspace_root),
        notification_channels: Vec::new(),
    }
}

fn generation_report(run_id: &str, request_id: &str) -> AgentMessage {
    let timestamp = chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    AgentMessage::RunReport {
        report: Box::new(AgentRunReport {
            run_id: run_id.to_string(),
            request_id: request_id.to_string(),
            tool_name: "generation.probe".to_string(),
            source: "tunnel".to_string(),
            profile: "normal".to_string(),
            detail: "metadata".to_string(),
            status: "started".to_string(),
            started_at: timestamp,
            updated_at: timestamp,
            duration_ms: None,
            process_id: None,
            exit_code: None,
            reason: None,
            arguments: None,
            result: None,
            process: None,
        }),
    }
}

fn generation_confirmation(request_id: &str, request_agent_id: &str) -> AgentMessage {
    AgentMessage::ConfirmationRequest {
        request_id: request_id.to_string(),
        agent_id: request_agent_id.to_string(),
        timeout_seconds: 5,
        payload: ConfirmationPayload {
            program: "generation-probe".to_string(),
            args: Vec::new(),
            command_preview: "generation probe".to_string(),
            risk_level: "LOW".to_string(),
            reason: "generation probe".to_string(),
            kind: None,
            server_id: None,
            tool_name: None,
        },
    }
}

async fn insert_generation_confirmation(
    state: &HubState,
    confirmation_id: &str,
    request_id: &str,
    connection_id: &str,
    token: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) {
    let sender = state
        .agents
        .snapshot_for_test()
        .await
        .get("agent")
        .expect("agent connection missing")
        .sender
        .clone();
    state
        .confirmations
        .insert_for_test(crate::confirmation::TestConfirmation {
            confirmation_id: confirmation_id.to_string(),
            request_id: request_id.to_string(),
            agent_id: "agent".to_string(),
            connection_id: connection_id.to_string(),
            token: token.to_string(),
            expires_at,
            sender,
        })
        .await;
}
async fn admit_generation_confirmation(
    state: &HubState,
    request_id: &str,
    connection_id: &str,
) -> crate::confirmation::ConfirmationPublication {
    let sender = state
        .agents
        .snapshot_for_test()
        .await
        .get("agent")
        .expect("agent connection missing")
        .sender
        .clone();
    let AgentMessage::ConfirmationRequest { payload, .. } =
        generation_confirmation(request_id, "agent")
    else {
        unreachable!();
    };
    state
        .confirmations
        .admit(crate::confirmation::ConfirmationAdmission {
            agent_id: "agent",
            connection_id,
            request_id: request_id.to_string(),
            sender,
            timeout_seconds: 5,
            provider_timeout_seconds: 45,
            payload,
        })
        .await
}

pub(crate) async fn generation_handle(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    message: AgentMessage,
) -> std::result::Result<(), String> {
    timeout(
        Duration::from_secs(5),
        handle_agent_message(state, agent_id, connection_id, message),
    )
    .await
    .expect("generation handler timed out")
}

async fn generation_fixture() -> (HubState, mpsc::UnboundedReceiver<OutboundAgentMessage>) {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut old_rx = insert_connection(
        &state,
        "agent",
        "old",
        chrono::Utc::now() - chrono::Duration::seconds(10),
    )
    .await;
    let (new_tx, new_rx) = mpsc::unbounded_channel();
    replace_agent_connection(&state, "agent", "new", AgentTransport::Sse, new_tx)
        .await
        .unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(5), old_rx.recv())
            .await
            .expect("replacement close timed out"),
        Some(OutboundAgentMessage::Close)
    ));
    generation_handle(
        &state,
        "agent",
        "new",
        generation_hello(
            AgentRole::Room,
            AgentConnectionMode::CommandCapable,
            "boot-new",
            "new",
        ),
    )
    .await
    .unwrap();
    (state, new_rx)
}
#[tokio::test]
async fn room_dispatch_keeps_validated_generation_during_replacement() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut old_rx = insert_connection(&state, "agent", "old", chrono::Utc::now()).await;
    {
        let mut current = state.agents.current.lock().await;
        current.get_mut("agent").unwrap().role = AgentRole::Room;
    }
    crate::room::control::register_connection_role(&state, "agent", "old", AgentRole::Room)
        .await
        .unwrap();

    let command = HubCommand::RoomNotebookRead {
        request_id: "room-generation-request".to_string(),
        payload: RoomNotebookReadRequest {
            path: "Notebook/topic.md".to_string(),
        },
    };
    let mut request = Box::pin(crate::room::control::request_active_room(
        &state, command, 1,
    ));
    let active_guard = state.active_room.lock().await;
    assert!(matches!(
        futures_util::poll!(request.as_mut()),
        std::task::Poll::Pending
    ));

    let (new_tx, mut new_rx) = mpsc::unbounded_channel();
    let mut replacement = Box::pin(replace_agent_connection(
        &state,
        "agent",
        "new",
        AgentTransport::WebSocket,
        new_tx,
    ));
    assert!(matches!(
        futures_util::poll!(replacement.as_mut()),
        std::task::Poll::Pending
    ));
    drop(active_guard);

    let request_poll = futures_util::poll!(request.as_mut());
    assert!(
        matches!(request_poll, std::task::Poll::Pending),
        "request poll: {request_poll:?}"
    );
    assert_eq!(
        futures_util::poll!(replacement.as_mut()),
        std::task::Poll::Ready(Ok(()))
    );

    let OutboundAgentMessage::Text(text) = old_rx.recv().await.unwrap() else {
        panic!("expected Room command on validated generation");
    };
    let envelope = serde_json::from_str::<HubCommandEnvelope>(&text).unwrap();
    let data = json!({ "current": null, "warnings": [] });
    let response = post_agent_message(
        State(state.clone()),
        Path("agent".to_string()),
        Query(SseConnectQuery::for_test(Some("old".to_string()))),
        agent_headers("secret"),
        axum::Json(AgentMessage::Response {
            run_id: Some(envelope.run_id.clone()),
            request_id: envelope.request_id.clone(),
            data: data.clone(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        timeout(Duration::from_secs(5), request.as_mut())
            .await
            .expect("Room request timed out")
            .unwrap(),
        data
    );

    generation_handle(
        &state,
        "agent",
        "new",
        generation_hello(
            AgentRole::Room,
            AgentConnectionMode::CommandCapable,
            "boot-new",
            "new",
        ),
    )
    .await
    .unwrap();
    assert!(matches!(
        new_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}
#[tokio::test]
async fn wp1_confirmation_callback_does_not_route_to_replacement_generation() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut old_rx = insert_connection(&state, "agent", "old", chrono::Utc::now()).await;
    let token = "wp1-callback-token";
    insert_generation_confirmation(
        &state,
        "wp1-confirmation",
        "wp1-confirmation-request",
        "old",
        token,
        chrono::Utc::now() + chrono::Duration::seconds(60),
    )
    .await;

    let (new_tx, mut new_rx) = mpsc::unbounded_channel();
    replace_agent_connection(&state, "agent", "new", AgentTransport::Sse, new_tx)
        .await
        .unwrap();

    let response = crate::confirmation::callback(
        State(state.clone()),
        Path(("wp1-confirmation".to_string(), "allow".to_string())),
        Query(crate::confirmation::ConfirmationCallbackQuery {
            token: token.to_string(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let old_message = timeout(Duration::from_secs(5), old_rx.recv())
        .await
        .expect("retirement response timed out")
        .expect("retirement response sender closed");
    let OutboundAgentMessage::Text(text) = old_message else {
        panic!("retirement must notify the old confirmation owner");
    };
    assert!(matches!(
        serde_json::from_str::<HubMessage>(&text).unwrap(),
        HubMessage::ConfirmationResponse {
            request_id,
            decision: agentic_gpt_protocol::ConfirmationDecision::ProviderUnavailable,
            reason,
        } if request_id == "wp1-confirmation-request" && reason == "provider_unavailable"
    ));
    assert!(matches!(
        timeout(Duration::from_secs(5), old_rx.recv())
            .await
            .expect("retirement close timed out"),
        Some(OutboundAgentMessage::Close)
    ));
    assert!(matches!(
        new_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}
#[tokio::test]
async fn wp1_confirmation_callback_claims_response_once() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut outbound = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
    let token = "wp1-single-claim-token";
    insert_generation_confirmation(
        &state,
        "wp1-single-claim",
        "wp1-single-claim-request",
        "current",
        token,
        chrono::Utc::now() + chrono::Duration::seconds(60),
    )
    .await;

    let first = crate::confirmation::callback(
        State(state.clone()),
        Path(("wp1-single-claim".to_string(), "allow".to_string())),
        Query(crate::confirmation::ConfirmationCallbackQuery {
            token: token.to_string(),
        }),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = crate::confirmation::callback(
        State(state.clone()),
        Path(("wp1-single-claim".to_string(), "allow".to_string())),
        Query(crate::confirmation::ConfirmationCallbackQuery {
            token: token.to_string(),
        }),
    )
    .await;
    assert_eq!(second.status(), StatusCode::CONFLICT);

    let Some(OutboundAgentMessage::Text(text)) = timeout(Duration::from_secs(5), outbound.recv())
        .await
        .expect("callback response timed out")
    else {
        panic!("callback response sender closed");
    };
    assert!(matches!(
        serde_json::from_str::<HubMessage>(&text).unwrap(),
        HubMessage::ConfirmationResponse {
            request_id,
            decision: agentic_gpt_protocol::ConfirmationDecision::AllowOnce,
            reason,
        } if request_id == "wp1-single-claim-request" && reason == "user_allowed"
    ));
    assert!(matches!(
        outbound.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}
#[tokio::test]
async fn wp1_confirmation_retirement_does_not_claim_reused_generation() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut old_rx = insert_connection(&state, "agent", "same", chrono::Utc::now()).await;
    insert_generation_confirmation(
        &state,
        "wp1-old-retirement",
        "wp1-old-request",
        "same",
        "wp1-old-token",
        chrono::Utc::now() + chrono::Duration::seconds(60),
    )
    .await;
    let release_retirement = state.confirmations.pause_next_retirement().await;

    let mut disconnect = Box::pin(disconnect_agent(&state, "agent", "same", None));
    assert!(matches!(
        futures_util::poll!(disconnect.as_mut()),
        std::task::Poll::Pending
    ));

    let (new_tx, mut new_rx) = mpsc::unbounded_channel();
    let mut replacement = Box::pin(replace_agent_connection(
        &state,
        "agent",
        "same",
        AgentTransport::Sse,
        new_tx,
    ));
    let replacement_ready = match futures_util::poll!(replacement.as_mut()) {
        std::task::Poll::Ready(result) => {
            assert_eq!(result, Ok(()));
            true
        }
        std::task::Poll::Pending => false,
    };
    let mut publication = if replacement_ready {
        Some(admit_generation_confirmation(&state, "wp1-new-request", "same").await)
    } else {
        None
    };

    release_retirement
        .send(())
        .expect("retirement gate receiver dropped");
    assert!(
        timeout(Duration::from_secs(5), disconnect.as_mut())
            .await
            .expect("disconnect timed out"),
        "disconnect should remove old generation"
    );
    if publication.is_none() {
        timeout(Duration::from_secs(5), replacement.as_mut())
            .await
            .expect("replacement timed out")
            .expect("replacement should register reused connection id");
        publication = Some(admit_generation_confirmation(&state, "wp1-new-request", "same").await);
    }
    let publication = publication.expect("new confirmation admission missing");

    let response = crate::confirmation::callback(
        State(state.clone()),
        Path((publication.confirmation_id.clone(), "allow".to_string())),
        Query(crate::confirmation::ConfirmationCallbackQuery {
            token: publication.token,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let Some(OutboundAgentMessage::Text(text)) = timeout(Duration::from_secs(5), new_rx.recv())
        .await
        .expect("new callback response timed out")
    else {
        panic!("new callback response sender closed");
    };
    assert!(matches!(
        serde_json::from_str::<HubMessage>(&text).unwrap(),
        HubMessage::ConfirmationResponse {
            request_id,
            decision: agentic_gpt_protocol::ConfirmationDecision::AllowOnce,
            reason,
        } if request_id == "wp1-new-request" && reason == "user_allowed"
    ));
    assert!(matches!(
        new_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    let Some(OutboundAgentMessage::Text(text)) = timeout(Duration::from_secs(5), old_rx.recv())
        .await
        .expect("old retirement response timed out")
    else {
        panic!("old retirement response sender closed");
    };
    assert!(matches!(
        serde_json::from_str::<HubMessage>(&text).unwrap(),
        HubMessage::ConfirmationResponse {
            request_id,
            decision: agentic_gpt_protocol::ConfirmationDecision::ProviderUnavailable,
            reason,
        } if request_id == "wp1-old-request" && reason == "provider_unavailable"
    ));
    assert!(matches!(
        timeout(Duration::from_secs(5), old_rx.recv())
            .await
            .expect("old close timed out"),
        Some(OutboundAgentMessage::Close)
    ));
}

pub(crate) async fn generation_snapshot(state: &HubState) -> Value {
    let connection = {
        let agents = state.agents.current.lock().await;
        let connection = agents.get("agent").unwrap();
        json!({
            "connectionId": connection.connection_id,
            "role": connection.role,
            "connectionMode": connection.connection_mode,
            "helloReceived": connection.hello_received,
            "bootGeneration": connection.boot_generation,
            "transport": match connection.transport {
                AgentTransport::WebSocket => "websocket",
                AgentTransport::Sse => "sse",
            },
            "lastSeenAt": connection.last_seen_at,
            "configSummary": connection.config_summary,
            "notificationChannels": connection.notification_channels,
        })
    };
    let registry_last_seen = registry_entry(state, "agent")
        .unwrap()
        .and_then(|entry| entry.last_seen_at);
    let boot_generation = state.boot_generations.lock().await.get("agent").cloned();
    let processes = state
        .process_cache
        .snapshots("agent")
        .await
        .into_iter()
        .map(|snapshot| {
            let process_id = snapshot.process.process_id.clone();
            (process_id, serde_json::to_value(snapshot.process).unwrap())
        })
        .collect::<serde_json::Map<_, _>>();
    let active_room = state.active_room.lock().await.as_ref().map(|active| {
        json!({
            "agentId": active.agent_id,
            "connectionId": active.connection_id,
        })
    });
    let pending_confirmations = state.confirmations.pending_count().await;
    json!({
        "connection": connection,
        "registryLastSeenAt": registry_last_seen,
        "bootGeneration": boot_generation,
        "processes": processes,
        "activeRoom": active_room,
        "pendingConfirmations": pending_confirmations,
    })
}
#[tokio::test]
async fn changed_boot_generation_marks_only_active_processes_unknown_after_restart() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let _rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
    state
        .boot_generations
        .lock()
        .await
        .insert("agent".to_string(), "boot-a".to_string());
    let running = test_running_process("process_boot-a_running");
    let mut completed = test_running_process("process_boot-a_completed");
    completed.state = ProcessState::Completed;
    completed.finished_at = Some(completed.updated_at);
    state
        .process_cache
        .insert_for_test("agent", "current", Some("boot-a"), running)
        .await;
    state
        .process_cache
        .insert_for_test("agent", "current", Some("boot-a"), completed)
        .await;

    handle_agent_message(
        &state,
        "agent",
        "current",
        AgentMessage::Hello {
            role: AgentRole::Normal,
            boot_generation: "boot-b".to_string(),
            connection_mode: AgentConnectionMode::CommandCapable,
            config_summary: test_config_summary(),
            notification_channels: Vec::new(),
        },
    )
    .await
    .unwrap();

    let running = state
        .process_cache
        .snapshot("agent", "process_boot-a_running")
        .await
        .unwrap()
        .process;
    assert_eq!(running.state, ProcessState::UnknownAfterRestart);
    assert_eq!(
        running.reject_reason.as_deref(),
        Some("unknown_after_restart")
    );
    assert!(running.finished_at.is_none());
    assert_eq!(
        state
            .process_cache
            .snapshot("agent", "process_boot-a_completed")
            .await
            .unwrap()
            .process
            .state,
        ProcessState::Completed
    );
    assert_eq!(
        state
            .boot_generations
            .lock()
            .await
            .get("agent")
            .map(String::as_str),
        Some("boot-b")
    );
    assert_eq!(
        state
            .agents
            .current
            .lock()
            .await
            .get("agent")
            .and_then(|connection| connection.boot_generation.as_deref()),
        Some("boot-b")
    );
}
#[tokio::test]
async fn stale_heartbeat_is_rejected_without_touching_current_connection() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let previous_seen = chrono::Utc::now() - chrono::Duration::seconds(10);
    let _rx = insert_connection(&state, "agent", "current", previous_seen).await;

    let response = post_agent_message(
        State(state.clone()),
        Path("agent".to_string()),
        Query(SseConnectQuery::for_test(Some("old".to_string()))),
        agent_headers("secret"),
        axum::Json(AgentMessage::Heartbeat {
            sent_at: chrono::Utc::now(),
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let agents = state.agents.current.lock().await;
    let connection = agents.get("agent").unwrap();
    assert_eq!(connection.connection_id, "current");
    assert_eq!(connection.last_seen_at, previous_seen);
}

#[tokio::test]
async fn stale_process_update_is_rejected_without_writing_process_cache() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let _rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;

    let response = post_agent_message(
        State(state.clone()),
        Path("agent".to_string()),
        Query(SseConnectQuery::for_test(Some("old".to_string()))),
        agent_headers("secret"),
        axum::Json(AgentMessage::ProcessUpdate {
            process: test_running_process("process_oldboot_123"),
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(state.process_cache.count().await, 0);
}
#[tokio::test]
async fn expired_connection_cleanup_removes_only_stale_current_entries() {
    let state = test_state();
    let old_seen = chrono::Utc::now() - chrono::Duration::seconds(120);
    let fresh_seen = chrono::Utc::now();
    let _old_rx = insert_connection(&state, "old-agent", "old", old_seen).await;
    let _fresh_rx = insert_connection(&state, "fresh-agent", "fresh", fresh_seen).await;

    cleanup_expired_agent_connections_once(&state, chrono::Utc::now()).await;

    let agents = state.agents.current.lock().await;
    assert!(!agents.contains_key("old-agent"));
    assert!(agents.contains_key("fresh-agent"));
}
#[tokio::test]
async fn generation_stale_messages_preserve_current_state() {
    for case in [
        "hello_normal",
        "hello_reporting_only",
        "hello_room_changed_boot",
        "heartbeat",
        "process_update",
        "run_report",
        "confirmation_request",
    ] {
        let (state, mut new_rx) = generation_fixture().await;
        if case == "hello_room_changed_boot" {
            let running = test_running_process("generation_active");
            let mut completed = test_running_process("generation_terminal");
            completed.state = ProcessState::Completed;
            completed.finished_at = Some(completed.updated_at);
            state
                .process_cache
                .insert_for_test("agent", "old", Some("boot-old"), running)
                .await;
            state
                .process_cache
                .insert_for_test("agent", "old", Some("boot-old"), completed)
                .await;
        }
        let before = generation_snapshot(&state).await;
        let result = match case {
            "hello_normal" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    generation_hello(
                        AgentRole::Normal,
                        AgentConnectionMode::CommandCapable,
                        "boot-old",
                        "old",
                    ),
                )
                .await
            }
            "hello_reporting_only" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    generation_hello(
                        AgentRole::Normal,
                        AgentConnectionMode::ReportingOnly,
                        "boot-old",
                        "old",
                    ),
                )
                .await
            }
            "hello_room_changed_boot" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    generation_hello(
                        AgentRole::Room,
                        AgentConnectionMode::CommandCapable,
                        "boot-old",
                        "old",
                    ),
                )
                .await
            }
            "heartbeat" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    AgentMessage::Heartbeat {
                        sent_at: chrono::Utc::now(),
                    },
                )
                .await
            }
            "process_update" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    AgentMessage::ProcessUpdate {
                        process: test_running_process("generation_stale_process"),
                    },
                )
                .await
            }
            "run_report" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    generation_report("stale-report", "stale-report-request"),
                )
                .await
            }
            "confirmation_request" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    generation_confirmation("stale-confirmation", "agent"),
                )
                .await
            }
            _ => unreachable!(),
        };
        assert_eq!(
            result,
            Err("stale_connection".to_string()),
            "stale case {case}"
        );
        assert_eq!(
            generation_snapshot(&state).await,
            before,
            "stale case {case} changed current state"
        );
        if case == "run_report" {
            assert!(runs::get_run(&state, "stale-report").unwrap().is_none());
        }
        assert!(matches!(
            new_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}
#[tokio::test]
async fn generation_current_messages_keep_existing_effects() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut outbound = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;

    generation_handle(
        &state,
        "agent",
        "current",
        generation_hello(
            AgentRole::Room,
            AgentConnectionMode::CommandCapable,
            "boot-a",
            "current",
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        state
            .active_room
            .lock()
            .await
            .as_ref()
            .map(|active| active.connection_id.as_str()),
        Some("current")
    );

    let running = test_running_process("generation_running");
    let mut completed = test_running_process("generation_completed");
    completed.state = ProcessState::Completed;
    completed.finished_at = Some(completed.updated_at);
    state
        .process_cache
        .insert_for_test("agent", "current", Some("boot-a"), running)
        .await;
    state
        .process_cache
        .insert_for_test("agent", "current", Some("boot-a"), completed)
        .await;
    generation_handle(
        &state,
        "agent",
        "current",
        generation_hello(
            AgentRole::Room,
            AgentConnectionMode::CommandCapable,
            "boot-a",
            "current",
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        state
            .process_cache
            .snapshot("agent", "generation_running")
            .await
            .unwrap()
            .process
            .state,
        ProcessState::Running
    );
    assert_eq!(
        state
            .process_cache
            .snapshot("agent", "generation_completed")
            .await
            .unwrap()
            .process
            .state,
        ProcessState::Completed
    );
    generation_handle(
        &state,
        "agent",
        "current",
        generation_hello(
            AgentRole::Room,
            AgentConnectionMode::CommandCapable,
            "boot-b",
            "current-new",
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        state
            .process_cache
            .snapshot("agent", "generation_running")
            .await
            .unwrap()
            .process
            .state,
        ProcessState::UnknownAfterRestart
    );
    assert_eq!(
        state
            .process_cache
            .snapshot("agent", "generation_completed")
            .await
            .unwrap()
            .process
            .state,
        ProcessState::Completed
    );

    let sent_at = chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    generation_handle(
        &state,
        "agent",
        "current",
        AgentMessage::Heartbeat { sent_at },
    )
    .await
    .unwrap();
    let OutboundAgentMessage::Text(text) = timeout(Duration::from_secs(5), outbound.recv())
        .await
        .expect("current heartbeat ack timed out")
        .expect("current heartbeat sender closed")
    else {
        panic!("expected heartbeat ack");
    };
    assert!(matches!(
        serde_json::from_str::<HubMessage>(&text).unwrap(),
        HubMessage::HeartbeatAck {
            sent_at: ack_sent,
            ..
        } if ack_sent == sent_at
    ));

    generation_handle(
        &state,
        "agent",
        "current",
        AgentMessage::ProcessUpdate {
            process: test_running_process("generation_current_process"),
        },
    )
    .await
    .unwrap();
    assert!(state
        .process_cache
        .snapshot("agent", "generation_current_process")
        .await
        .is_some());

    generation_handle(
        &state,
        "agent",
        "current",
        generation_report("generation-current-report", "generation-current-request"),
    )
    .await
    .unwrap();
    let report = runs::get_run(&state, "generation-current-report")
        .unwrap()
        .expect("current report should be stored");
    assert_eq!(report.status, "started");
    assert_eq!(report.agent_id, "agent");
}
#[tokio::test]
async fn generation_stale_reliable_messages_do_not_touch_current() {
    for case in ["response", "transport_ack", "transport_status"] {
        let (state, mut new_rx) = generation_fixture().await;
        let command = HubCommand::McpListServers {
            request_id: format!("generation-{case}-request"),
        };
        let run = runs::prepare_run(
            &state,
            "agent",
            &format!("generation-{case}-request"),
            &command,
        )
        .unwrap();
        let before = generation_snapshot(&state).await;
        let result = match case {
            "response" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    AgentMessage::Response {
                        run_id: Some(run.run_id.clone()),
                        request_id: run.request_id.clone(),
                        data: json!({ "servers": [] }),
                    },
                )
                .await
            }
            "transport_ack" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    AgentMessage::TransportAck {
                        event_id: "generation-event".to_string(),
                        run_id: run.run_id.clone(),
                        request_id: run.request_id.clone(),
                        command_hash: run.command_hash.clone(),
                    },
                )
                .await
            }
            "transport_status" => {
                generation_handle(
                    &state,
                    "agent",
                    "old",
                    AgentMessage::TransportRunStatus {
                        run_id: run.run_id.clone(),
                        request_id: run.request_id.clone(),
                        status: "started".to_string(),
                        reason: None,
                    },
                )
                .await
            }
            _ => unreachable!(),
        };
        assert_eq!(result, Ok(()), "reliable case {case}");
        assert_eq!(
            generation_snapshot(&state).await,
            before,
            "reliable case {case} touched current state"
        );
        let stored = runs::get_run(&state, &run.run_id).unwrap().unwrap();
        match case {
            "response" => {
                assert_eq!(stored.status, "completed");
                assert_eq!(stored.result, Some(json!({ "servers": [] })));
            }
            "transport_ack" => assert_eq!(stored.status, "acked"),
            "transport_status" => assert_eq!(stored.status, "started"),
            _ => unreachable!(),
        }
        assert!(matches!(
            new_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}
#[tokio::test]
async fn generation_process_update_and_replace_are_linearized() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let _old_rx = insert_connection(&state, "agent", "old", chrono::Utc::now()).await;
    let processes = state.process_cache.lock_for_test().await;
    let mut update = Box::pin(handle_agent_message(
        &state,
        "agent",
        "old",
        AgentMessage::ProcessUpdate {
            process: test_running_process("generation_linearized_process"),
        },
    ));
    assert!(matches!(
        futures_util::poll!(update.as_mut()),
        std::task::Poll::Pending
    ));

    let (new_tx, _new_rx) = mpsc::unbounded_channel();
    let mut replacement = Box::pin(replace_agent_connection(
        &state,
        "agent",
        "new",
        AgentTransport::Sse,
        new_tx,
    ));
    let replacement_poll = futures_util::poll!(replacement.as_mut());
    let mut replacement_result = match replacement_poll {
        std::task::Poll::Ready(result) => Some(result),
        std::task::Poll::Pending => None,
    };
    let replacement_finished = replacement_result.is_some();
    drop(processes);

    let update_result = timeout(Duration::from_secs(5), update.as_mut())
        .await
        .expect("process update timed out");
    if replacement_result.is_none() {
        replacement_result = Some(
            timeout(Duration::from_secs(5), replacement.as_mut())
                .await
                .expect("replacement timed out"),
        );
    }
    assert_eq!(replacement_result.unwrap(), Ok(()));
    if replacement_finished {
        assert_eq!(update_result, Err("stale_connection".to_string()));
        assert!(state
            .process_cache
            .snapshot("agent", "generation_linearized_process")
            .await
            .is_none());
    } else {
        assert_eq!(update_result, Ok(()));
        assert!(state
            .process_cache
            .snapshot("agent", "generation_linearized_process")
            .await
            .is_some());
    }
}
#[tokio::test]
async fn generation_replacement_preserves_new_room_on_old_disconnect() {
    let (state, _new_rx) = generation_fixture().await;
    insert_generation_confirmation(
        &state,
        "generation-confirmation",
        "generation-confirmation-request",
        "new",
        "generation-token",
        chrono::Utc::now() + chrono::Duration::seconds(60),
    )
    .await;
    let before = generation_snapshot(&state).await;

    assert!(!disconnect_agent(&state, "agent", "old", None).await);
    assert_eq!(generation_snapshot(&state).await, before);
    let pending = state
        .confirmations
        .snapshot_for_test("generation-confirmation")
        .await
        .unwrap();
    assert!(!pending.resolved);

    assert!(disconnect_agent(&state, "agent", "new", None).await);
    assert!(state.active_room.lock().await.is_none());
    let pending = state
        .confirmations
        .snapshot_for_test("generation-confirmation")
        .await
        .unwrap();
    assert!(pending.resolved);
    assert!(matches!(
        pending.decision,
        Some(agentic_gpt_protocol::ConfirmationDecision::ProviderUnavailable)
    ));
}
#[tokio::test]
async fn generation_expiry_rechecks_current_liveness() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut outbound = insert_connection(
        &state,
        "agent",
        "current",
        chrono::Utc::now() - chrono::Duration::seconds(120),
    )
    .await;
    generation_handle(
        &state,
        "agent",
        "current",
        AgentMessage::Heartbeat {
            sent_at: chrono::Utc::now(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(5), outbound.recv())
            .await
            .expect("heartbeat ack timed out"),
        Some(OutboundAgentMessage::Text(_))
    ));
    let fresh_now = chrono::Utc::now();
    assert!(!disconnect_agent(&state, "agent", "current", Some(fresh_now),).await);
    assert!(state.agents.current.lock().await.contains_key("agent"));

    state
        .agents
        .current
        .lock()
        .await
        .get_mut("agent")
        .unwrap()
        .last_seen_at = fresh_now - chrono::Duration::seconds(120);
    assert!(disconnect_agent(&state, "agent", "current", Some(fresh_now),).await);
    assert!(!state.agents.current.lock().await.contains_key("agent"));

    let (replaced_state, _new_rx) = generation_fixture().await;
    assert!(
        !disconnect_agent(
            &replaced_state,
            "agent",
            "old",
            Some(chrono::Utc::now() + chrono::Duration::seconds(120)),
        )
        .await
    );
    assert_eq!(
        replaced_state
            .agents
            .current
            .lock()
            .await
            .get("agent")
            .map(|connection| connection.connection_id.as_str()),
        Some("new")
    );
}
#[tokio::test]
async fn generation_stale_heartbeat_direct_handler() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let mut old_rx = insert_connection(
        &state,
        "agent",
        "old",
        chrono::Utc::now() - chrono::Duration::seconds(10),
    )
    .await;
    let (new_tx, new_rx) = mpsc::unbounded_channel();
    replace_agent_connection(&state, "agent", "new", AgentTransport::WebSocket, new_tx)
        .await
        .unwrap();
    assert!(matches!(
        old_rx.recv().await,
        Some(OutboundAgentMessage::Close)
    ));
    let new_seen = state
        .agents
        .current
        .lock()
        .await
        .get("agent")
        .map(|connection| connection.last_seen_at)
        .unwrap();

    let result = handle_agent_message(
        &state,
        "agent",
        "old",
        AgentMessage::Heartbeat {
            sent_at: chrono::Utc::now(),
        },
    )
    .await;

    assert_eq!(result, Err("stale_connection".to_string()));
    assert_eq!(
        state
            .agents
            .current
            .lock()
            .await
            .get("agent")
            .map(|connection| connection.last_seen_at),
        Some(new_seen)
    );
    drop(new_rx);
}
