mod acquire_confirmation;
mod config_api;
mod email;
mod envelopes;
mod events;
mod identity_config;
mod mcp;
mod notification_tmux;
mod process;
mod secret;
mod skills;

pub use acquire_confirmation::*;
pub use config_api::*;
pub use email::*;
pub use envelopes::*;
pub use events::*;
pub use identity_config::*;
pub use mcp::*;
pub use notification_tmux::*;
pub use process::*;
pub use secret::*;
pub use skills::*;

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
