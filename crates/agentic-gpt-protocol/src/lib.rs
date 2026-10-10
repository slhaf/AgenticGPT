mod envelopes;
mod events;
mod identity_config;
mod mcp;
mod notification_tmux;
mod process;
mod room;
mod skill_bootstrap;

pub use envelopes::*;
pub use events::*;
pub use identity_config::*;
pub use mcp::*;
pub use notification_tmux::*;
pub use process::*;
pub use room::*;
pub use skill_bootstrap::*;

#[cfg(test)]
mod room_v2_contract_tests {
    use super::*;

    #[test]
    fn maintenance_v2_shapes_close_slots_bound_wait_and_separate_sync() {
        let slots: Vec<serde_json::Value> = RoomMaintenanceSlot::ALL
            .iter()
            .map(|slot| serde_json::to_value(slot).unwrap())
            .collect();
        assert_eq!(
            slots,
            vec![
                serde_json::json!("diary.daily"),
                serde_json::json!("diary.weekly"),
                serde_json::json!("diary.monthly"),
                serde_json::json!("notebook"),
                serde_json::json!("entity"),
            ]
        );
        assert!(
            serde_json::from_value::<RoomMaintenanceSlot>(serde_json::json!("diary.other"))
                .is_err()
        );

        let request: RoomMaintenanceSubmitRequest = serde_json::from_value(serde_json::json!({
            "items": [{
                "slot": "entity",
                "payload": {"entity": "project", "operation": "refresh"}
            }],
            "mode": "workflow",
            "waitSeconds": 31
        }))
        .unwrap();
        assert!(request.has_valid_item_count());
        assert_eq!(request.effective_wait_seconds(), 30);
        assert_eq!(
            serde_json::to_value(&request).unwrap()["items"][0]["payload"]["operation"],
            "refresh"
        );

        let response = RoomMaintenanceSubmitResponse {
            mode: RoomMaintenanceExecutionMode::Local,
            state: RoomMaintenanceSubmissionState::Applied,
            local_applied: true,
            sync: RoomMaintenanceSyncOutcome::Failed,
            revision: Some("abc123".to_string()),
        };
        let response_value = serde_json::to_value(&response).unwrap();
        assert_eq!(response_value["localApplied"], true);
        assert_eq!(response_value["state"], "applied");
        assert_eq!(response_value["sync"], "failed");
        assert_eq!(
            serde_json::from_value::<RoomMaintenanceSubmitResponse>(response_value).unwrap(),
            response
        );

        let status = RoomMaintenanceStatusResponse {
            repository: RoomMaintenanceRepositoryStatus {
                root: "/workspace/room".to_string(),
                initialized: true,
                top_level: true,
                branch: Some("main".to_string()),
                head: Some("abc123".to_string()),
                clean: Some(true),
            },
            schema: RoomMaintenanceSchemaStatus {
                schema_version: Some(1),
                supported: true,
                ready: true,
            },
            scaffold: RoomMaintenanceScaffoldStatus {
                ready: false,
                missing_paths: vec!["manual/entity.md".to_string()],
            },
            local_executor: RoomMaintenanceLocalExecutorStatus { ready: true },
            configured_mode: RoomMaintenanceExecutionMode::Local,
            auto_push: false,
            workflow: RoomMaintenanceWorkflowStatus {
                available: true,
                ready: false,
            },
            remote: RoomMaintenanceRemoteStatus {
                configured: true,
                available: true,
            },
            sync: RoomMaintenanceSyncStatus {
                upstream_available: true,
                in_sync: Some(false),
                local_head: Some("abc123".to_string()),
                upstream_head: Some("def456".to_string()),
            },
            slots: RoomMaintenanceSlot::ALL
                .iter()
                .map(|slot| RoomMaintenanceSlotStatus {
                    slot: *slot,
                    occupied: false,
                })
                .collect(),
        };
        let status_value = serde_json::to_value(&status).unwrap();
        assert_eq!(status_value["schema"]["schemaVersion"], 1);
        assert_eq!(status_value["schema"]["ready"], true);
        assert_eq!(status_value["scaffold"]["ready"], false);
        assert_eq!(status_value["localExecutor"]["ready"], true);
        assert_eq!(status_value["workflow"]["ready"], false);
        assert_eq!(status_value["remote"]["available"], true);
        assert_eq!(status_value["sync"]["inSync"], false);
        assert_eq!(status_value["slots"].as_array().unwrap().len(), 5);
        assert_eq!(
            serde_json::from_value::<RoomMaintenanceStatusResponse>(status_value).unwrap(),
            status
        );
    }
}

#[cfg(test)]
mod tmux_tests {
    use super::*;

    #[test]
    fn install_and_run_protocol_defaults_and_command_names_are_stable() {
        let get: SkillInstallGetRequest = serde_json::from_value(serde_json::json!({
            "installId": "install-1"
        }))
        .unwrap();
        assert_eq!(get.effective_wait_seconds(), 5);

        let run: SkillRunRequest = serde_json::from_value(serde_json::json!({
            "id": "demo",
            "path": "scripts/check.sh"
        }))
        .unwrap();
        assert_eq!(run.effective_wait_seconds(), 5);
        assert_eq!(run.args, None);

        let command = HubCommand::SkillsRun {
            request_id: "req".to_string(),
            payload: run,
        };
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["type"], "skills.run");
        assert_eq!(value["requestId"], "req");
        assert_eq!(value["payload"]["waitSeconds"], serde_json::Value::Null);

        let install = HubCommand::SkillsInstall {
            request_id: "req-install".to_string(),
            payload: SkillInstallRequest {
                id: "demo".to_string(),
                source: SkillInstallSource::Files { files: vec![] },
                replace_existing: false,
                activate_after_install: None,
                idempotency_key: None,
            },
        };
        assert_eq!(
            serde_json::to_value(install).unwrap()["type"],
            "skills.install"
        );
    }
    #[test]
    fn managed_mcp_batch_defaults_bounds_and_wire_type_are_frozen() {
        let defaults: McpBatchRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "calls": [{
                "id": "first",
                "serverId": "server",
                "toolName": "tool",
                "arguments": {}
            }]
        }))
        .unwrap();
        assert_eq!(defaults.mode, McpBatchMode::Parallel);
        assert!(!defaults.fail_fast);
        assert_eq!(defaults.effective_wait_seconds(), 5);
        assert_eq!(defaults.effective_timeout_seconds(), 300);
        assert_eq!(McpBatchRequest::MIN_CALLS, 1);
        assert_eq!(McpBatchRequest::MAX_CALLS, 16);
        assert_eq!(
            McpBatchRequest::MAX_AGGREGATE_ARGUMENT_BYTES,
            2 * 1024 * 1024
        );
        assert_eq!(McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES, 2 * 1024 * 1024);

        let bounded: McpBatchRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "calls": [{"serverId": "server", "toolName": "tool"}],
            "mode": "sequential",
            "failFast": true,
            "waitSeconds": 999,
            "timeoutSeconds": 9999
        }))
        .unwrap();
        assert_eq!(bounded.mode, McpBatchMode::Sequential);
        assert!(bounded.fail_fast);
        assert_eq!(bounded.effective_wait_seconds(), 30);
        assert_eq!(bounded.effective_timeout_seconds(), 900);

        let command = HubCommand::McpBatch {
            request_id: "req-batch".to_string(),
            payload: defaults,
        };
        assert_eq!(command.request_id(), "req-batch");
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["type"], "mcp.batch");
        assert_eq!(value["requestId"], "req-batch");
        assert_eq!(value["payload"]["calls"][0]["id"], "first");
    }
}
