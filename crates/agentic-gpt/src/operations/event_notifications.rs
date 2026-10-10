use agentic_gpt_protocol::{
    EventResponseDisposition, EventSource, EventSourceKind, ProcessInfo, ProcessState,
    SkillInstallStatus, SkillInstallStatusResponse,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::state::AppState;

pub(crate) fn process_source(process_id: &str) -> EventSource {
    EventSource {
        kind: EventSourceKind::Process,
        reference: process_id.to_string(),
    }
}

pub(crate) fn skill_install_source(install_id: &str) -> EventSource {
    EventSource {
        kind: EventSourceKind::SkillInstall,
        reference: install_id.to_string(),
    }
}

pub(crate) fn process_completion_details(
    process: &ProcessInfo,
) -> Option<(String, String, DateTime<Utc>)> {
    if process.state.is_active() {
        return None;
    }
    let event_type = process_event_type(process.state);
    let message = format!(
        "Managed process {} ended with state {}.",
        process.process_id,
        process.state.as_str()
    );
    Some((event_type.to_string(), message, process.updated_at))
}
pub(crate) fn register_internal_source(
    state: &AppState,
    source: &EventSource,
    policy: &crate::event_store::InternalEventPolicy,
    origin: Option<&agentic_gpt_protocol::EventOrigin>,
) -> Result<()> {
    if let Err(error) = state.event_store.register_internal(source, policy) {
        abort_unadmitted_source(state, source, origin);
        return Err(error);
    }
    if let Err(error) = bind_origin(state, source, origin) {
        abort_unadmitted_source(state, source, origin);
        return Err(error);
    }
    Ok(())
}

fn process_event_type(state: ProcessState) -> &'static str {
    match state {
        ProcessState::Completed => "process.completed",
        ProcessState::Failed => "process.failed",
        ProcessState::Rejected => "process.rejected",
        ProcessState::Cancelled => "process.cancelled",
        ProcessState::TimedOut => "process.timed_out",
        ProcessState::Detached => "process.detached",
        ProcessState::UnknownAfterRestart => "process.unknown_after_restart",
        ProcessState::Skipped => "process.skipped",
        ProcessState::Queued
        | ProcessState::WaitingConfirmation
        | ProcessState::Starting
        | ProcessState::Running
        | ProcessState::CancelRequested => unreachable!("active process has no completion event"),
    }
}

fn install_completion_details(
    status: &SkillInstallStatusResponse,
) -> Option<(&'static str, String, DateTime<Utc>)> {
    let (event_type, status_name) = match status.status {
        SkillInstallStatus::Completed => ("skill_install.completed", "completed"),
        SkillInstallStatus::Failed => ("skill_install.failed", "failed"),
        SkillInstallStatus::Cancelled => ("skill_install.cancelled", "cancelled"),
        SkillInstallStatus::Queued | SkillInstallStatus::Running => return None,
    };
    let message = format!(
        "Skill installation {} ended with state {}.",
        status.install_id, status_name
    );
    Some((
        event_type,
        message,
        status.finished_at.unwrap_or(status.updated_at),
    ))
}

pub(crate) fn record_internal_completion(
    state: &AppState,
    source: &EventSource,
    event_type: &str,
    message: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    state
        .event_store
        .record_internal_completion(source, event_type, message, at)
}

pub(crate) fn record_process_completion(state: &AppState, process: &ProcessInfo) -> Result<()> {
    let Some((event_type, message, at)) = process_completion_details(process) else {
        return Ok(());
    };
    let source = process_source(&process.process_id);
    record_internal_completion(state, &source, &event_type, &message, at)?;
    state
        .process_history
        .acknowledge_event_completion(&process.process_id)
}

pub(crate) fn record_pending_process_completion(
    state: &AppState,
    completion: &crate::process_history::PendingProcessEventCompletion,
) -> Result<()> {
    let source = process_source(&completion.process_id);
    record_internal_completion(
        state,
        &source,
        &completion.event_type,
        &completion.message,
        completion.completed_at,
    )?;
    state
        .process_history
        .acknowledge_event_completion(&completion.process_id)
}

pub(crate) fn record_skill_install_completion(
    state: &AppState,
    status: &SkillInstallStatusResponse,
) -> Result<()> {
    let Some((event_type, message, at)) = install_completion_details(status) else {
        return Ok(());
    };
    let source = skill_install_source(&status.install_id);
    record_internal_completion(state, &source, event_type, &message, at)
}

pub(crate) fn initial_response_dispositions(
    operation: &str,
    value: &Value,
) -> Result<Vec<EventResponseDisposition>> {
    let mut dispositions = Vec::new();
    match operation {
        "process.exec" | "mcp.callTool" | "skills.run" => {
            if let Some(process_id) = find_string_field(value, "processId") {
                dispositions.push(EventResponseDisposition {
                    source: process_source(process_id),
                    includes_terminal: includes_process_terminal(value),
                });
            }
        }
        "process.batch" => append_batch_dispositions(value, "processes", &mut dispositions),
        "mcp.batch" => append_batch_dispositions(value, "results", &mut dispositions),
        "skills.install" => {
            if value
                .get("deduplicated")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Ok(dispositions);
            }
            if let Some(install_id) = value.get("installId").and_then(Value::as_str) {
                let includes_terminal = value
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(is_terminal_install_status);
                dispositions.push(EventResponseDisposition {
                    source: skill_install_source(install_id),
                    includes_terminal,
                });
            }
        }
        _ => {}
    }
    Ok(dispositions)
}

pub(crate) fn registered_response_dispositions(
    event_store: &crate::event_store::EventStore,
    operation: &str,
    value: &Value,
) -> Result<Vec<EventResponseDisposition>> {
    let dispositions = initial_response_dispositions(operation, value)?;
    if dispositions.is_empty() {
        return Ok(dispositions);
    }
    let registration = event_store
        .contains_internal_sources(dispositions.iter().map(|disposition| &disposition.source))?;
    let mut registered = Vec::with_capacity(dispositions.len());
    for (disposition, is_registered) in dispositions.into_iter().zip(registration) {
        if is_registered {
            registered.push(disposition);
        }
    }
    Ok(registered)
}

fn append_batch_dispositions(
    value: &Value,
    key: &str,
    dispositions: &mut Vec<EventResponseDisposition>,
) {
    let Some(items) = value.get(key).and_then(Value::as_array) else {
        return;
    };
    for item in items {
        if let Some(process_id) = find_string_field(item, "processId") {
            dispositions.push(EventResponseDisposition {
                source: process_source(process_id),
                includes_terminal: includes_process_terminal(item),
            });
        }
    }
}

pub(crate) async fn settle_initial_response(
    state: &AppState,
    operation: &str,
    value: &Value,
) -> Result<()> {
    for disposition in initial_response_dispositions(operation, value)? {
        match state
            .event_store
            .settle_response(&disposition.source, disposition.includes_terminal)
        {
            Ok(()) => {}
            Err(error) if error.to_string() == "event_internal_source_not_registered" => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn includes_process_terminal(value: &Value) -> bool {
    value
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(is_terminal_process_state)
        || value.get("process").is_some_and(includes_process_terminal)
}

fn is_terminal_process_state(state: &str) -> bool {
    matches!(
        state,
        "completed"
            | "failed"
            | "rejected"
            | "cancelled"
            | "timed_out"
            | "detached"
            | "unknown_after_restart"
            | "skipped"
    )
}

fn is_terminal_install_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

fn find_string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    if let Some(value) = value.get(field).and_then(Value::as_str) {
        return Some(value);
    }
    match value {
        Value::Object(object) => object
            .values()
            .find_map(|value| find_string_field(value, field)),
        Value::Array(items) => items
            .iter()
            .find_map(|value| find_string_field(value, field)),
        _ => None,
    }
}

pub(crate) async fn drain_completion_notifications(state: &AppState) -> Result<()> {
    let mut failures = Vec::new();
    match state.process_history.pending_event_completions() {
        Ok(completions) => {
            for completion in completions {
                if let Err(error) = record_pending_process_completion(state, &completion) {
                    failures.push(format!("process completion delivery failed: {error}"));
                }
            }
        }
        Err(error) => failures.push(format!("process completion recovery failed: {error}")),
    }

    match state.skill_installs.pending_event_completions().await {
        Ok(records) => {
            for record in records {
                match record_skill_install_completion(state, &record.status) {
                    Ok(()) => {
                        if let Err(error) = state
                            .skill_installs
                            .acknowledge_event_completion(state, &record.install_id)
                            .await
                        {
                            failures.push(format!(
                                "skill install completion acknowledgement failed: {error}"
                            ));
                        }
                    }
                    Err(error) => {
                        failures.push(format!("skill install completion delivery failed: {error}"))
                    }
                }
            }
        }
        Err(error) => failures.push(format!("skill install completion recovery failed: {error}")),
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(failures.join("; ")))
    }
}

pub(crate) async fn recover_event_notifications(state: &AppState) -> Result<()> {
    if let Err(error) = drain_completion_notifications(state).await {
        crate::utils::log_warn(format!(
            "event completion recovery deferred; notification evidence remains durable: {error}"
        ));
    }
    for source in state.event_store.pending_internal_sources()? {
        if state.event_store.remote_origin(&source)?.is_some() {
            continue;
        }
        state.event_store.settle_response(&source, false)?;
    }
    Ok(())
}

pub(crate) fn abort_unadmitted_source(
    state: &AppState,
    source: &EventSource,
    origin: Option<&agentic_gpt_protocol::EventOrigin>,
) {
    let _ = if let Some(origin) = origin {
        state
            .event_store
            .settle_remote_response(source, origin, true)
    } else {
        state.event_store.settle_response(source, true)
    };
}

pub(crate) fn bind_origin(
    state: &AppState,
    source: &EventSource,
    origin: Option<&agentic_gpt_protocol::EventOrigin>,
) -> Result<()> {
    if let Some(origin) = origin {
        state.event_store.bind_origin(source, origin)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[tokio::test]
    async fn response_feedback_requires_registered_sources_and_keeps_off_policy_quiet() {
        let (state, root) = test_state();
        let capacity_rejection = json!({
            "processId": "process-capacity-rejected",
            "state": "rejected",
            "terminationEvidence": "not_started"
        });
        assert!(registered_response_dispositions(
            &state.event_store,
            "process.exec",
            &capacity_rejection,
        )
        .unwrap()
        .is_empty());

        let off_policy = crate::event_store::InternalEventPolicy {
            low_ttl_seconds: 24 * 60 * 60,
            overrides: std::collections::BTreeMap::from([("process.completed".to_string(), None)]),
        };
        let terminal_source = process_source("process-completed-leader");
        let terminal_origin = agentic_gpt_protocol::EventOrigin {
            run_id: "run-completed-leader".to_string(),
            request_id: "request-completed-leader".to_string(),
            command_hash: "hash-completed-leader".to_string(),
        };
        register_internal_source(
            &state,
            &terminal_source,
            &off_policy,
            Some(&terminal_origin),
        )
        .unwrap();
        state
            .event_store
            .record_internal_completion(
                &terminal_source,
                "process.completed",
                "completed leader with live descendant",
                Utc::now(),
            )
            .unwrap();
        let terminal_response = json!({
            "processId": "process-completed-leader",
            "state": "completed",
            "captureStatus": "incomplete",
            "output": {"hasMore": true}
        });
        let terminal_dispositions = registered_response_dispositions(
            &state.event_store,
            "process.exec",
            &terminal_response,
        )
        .unwrap();
        assert_eq!(
            terminal_dispositions,
            vec![EventResponseDisposition {
                source: terminal_source.clone(),
                includes_terminal: true,
            }]
        );
        state
            .event_store
            .settle_remote_response(
                &terminal_source,
                &terminal_origin,
                terminal_dispositions[0].includes_terminal,
            )
            .unwrap();

        let active_source = process_source("process-off-policy-async");
        let active_origin = agentic_gpt_protocol::EventOrigin {
            run_id: "run-off-policy-async".to_string(),
            request_id: "request-off-policy-async".to_string(),
            command_hash: "hash-off-policy-async".to_string(),
        };
        register_internal_source(&state, &active_source, &off_policy, Some(&active_origin))
            .unwrap();
        let active_response = json!({
            "processId": "process-off-policy-async",
            "state": "running"
        });
        let active_dispositions =
            registered_response_dispositions(&state.event_store, "process.exec", &active_response)
                .unwrap();
        assert_eq!(
            active_dispositions,
            vec![EventResponseDisposition {
                source: active_source.clone(),
                includes_terminal: false,
            }]
        );
        state
            .event_store
            .record_internal_completion(
                &active_source,
                "process.completed",
                "completion suppressed by off policy",
                Utc::now(),
            )
            .unwrap();
        state
            .event_store
            .settle_remote_response(
                &active_source,
                &active_origin,
                active_dispositions[0].includes_terminal,
            )
            .unwrap();
        assert!(state.event_store.panel().unwrap().new.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    fn test_state() -> (AppState, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agentic-event-recovery-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut config = crate::config::Config::default_config().unwrap();
        config.workspace_root = root.clone();
        let private_state =
            crate::private_state::PrivateStatePaths::for_test(root.join("private-state"));
        let state = AppState {
            config_path: root.join("config.json"),
            config: Arc::new(tokio::sync::RwLock::new(config)),
            private_state: private_state.clone(),
            event_store: crate::event_store::EventStore::open(&private_state).unwrap(),
            process_history: crate::process_history::ProcessHistoryStore::open(&private_state),
            browser_runtime: None,
            runtime: crate::state::RuntimeModel::hub(crate::state::CapabilityProfile::Room),
            started_at: Utc::now(),
            boot_generation: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            supervised: false,
            file_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            hub_sender: Arc::new(tokio::sync::Mutex::new(None)),
            reporting_sender: Arc::new(tokio::sync::Mutex::new(None)),
            pending_confirmations: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::process::McpConcurrency::new()),
            room_repository_writes: Arc::new(tokio::sync::Mutex::new(())),
            skills_writes: Arc::new(tokio::sync::Mutex::new(())),
            skill_leases: Arc::new(crate::skills::SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::for_test(
                private_state.skill_installs.clone(),
            )),
        };
        (state, root)
    }

    #[tokio::test]
    async fn recovery_does_not_locally_settle_remote_awaiting_source() {
        let (state, root) = test_state();
        let source = process_source("process-remote-awaiting");
        let origin = agentic_gpt_protocol::EventOrigin {
            run_id: "run-remote".to_string(),
            request_id: "request-remote".to_string(),
            command_hash: "command-hash".to_string(),
        };
        let policy = state.config.read().await.events.internal_policy();
        state
            .event_store
            .register_internal(&source, &policy)
            .unwrap();
        state.event_store.bind_origin(&source, &origin).unwrap();
        state
            .event_store
            .record_internal_completion(
                &source,
                "process.completed",
                "completed remotely",
                Utc::now(),
            )
            .unwrap();

        recover_event_notifications(&state).await.unwrap();

        assert_eq!(
            state.event_store.remote_origin(&source).unwrap(),
            Some(origin)
        );
        let pending = state.event_store.pending_internal_sources().unwrap();
        assert!(pending.iter().any(|pending| pending == &source));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn process_history_recovery_failure_does_not_block_event_access() {
        let (state, root) = test_state();
        let event = state
            .event_store
            .inject(
                &agentic_gpt_protocol::EventInjectRequest {
                    message: "external inbox event".to_string(),
                    severity: None,
                    reference: "external-recovery".to_string(),
                },
                24 * 60 * 60,
            )
            .unwrap();
        let state = AppState {
            process_history: crate::process_history::ProcessHistoryStore::disabled(
                root.join("unavailable-process-history.sqlite3"),
            ),
            ..state
        };

        recover_event_notifications(&state).await.unwrap();

        assert_eq!(
            state.event_store.get(&event.event_id).unwrap().message,
            "external inbox event"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
