mod envelopes;
mod identity_config;
mod mcp;
mod notification_tmux;
mod process_jobs;
mod room;
mod skill_bootstrap;

pub use envelopes::*;
pub use identity_config::*;
pub use mcp::*;
pub use notification_tmux::*;
pub use process_jobs::*;
pub use room::*;
pub use skill_bootstrap::*;

#[cfg(test)]
use chrono::{DateTime, Utc};

#[cfg(test)]
mod room_v2_contract_tests {
    use super::*;

    fn diary_layer(
        layer: RoomDiaryLayer,
        period: &str,
        path: &str,
        available: bool,
        content: Option<&str>,
        issue: Option<RoomDiaryLayerIssue>,
    ) -> RoomDiaryLayerResult {
        RoomDiaryLayerResult {
            layer,
            period: period.to_string(),
            path: path.to_string(),
            available,
            content: content.map(str::to_string),
            issue,
        }
    }

    #[test]
    fn diary_v2_shapes_keep_periods_semantic_and_missing_layers_explicit() {
        let request: RoomDiaryReadRequest = serde_json::from_value(serde_json::json!({
            "layer": "weekly",
            "period": "2026-09-07--2026-09-13"
        }))
        .unwrap();
        assert_eq!(request.layer, RoomDiaryLayer::Weekly);
        assert_eq!(request.period, "2026-09-07--2026-09-13");
        let request_value = serde_json::to_value(request).unwrap();
        assert_eq!(request_value["period"], "2026-09-07--2026-09-13");
        assert!(request_value.get("path").is_none());

        let response = RoomDiaryActiveResponse {
            daily: diary_layer(
                RoomDiaryLayer::Daily,
                "current",
                "Diary/Daily/current.md",
                true,
                Some("# Daily\n"),
                None,
            ),
            weekly: diary_layer(
                RoomDiaryLayer::Weekly,
                "current",
                "Diary/Weekly/current.md",
                false,
                None,
                Some(RoomDiaryLayerIssue::Missing),
            ),
            monthly: diary_layer(
                RoomDiaryLayer::Monthly,
                "current",
                "Diary/Monthly/current.md",
                true,
                Some("# Monthly\n"),
                None,
            ),
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["daily"]["content"], "# Daily\n");
        assert_eq!(value["weekly"]["available"], false);
        assert_eq!(value["weekly"]["issue"], "missing");
        assert!(value["weekly"].get("content").is_none());
        assert_eq!(value["daily"]["path"], "Diary/Daily/current.md");
        assert_eq!(
            serde_json::from_value::<RoomDiaryActiveResponse>(value).unwrap(),
            response
        );
    }

    #[test]
    fn notebook_and_state_v2_shapes_bound_previews_and_read_markdown_exactly() {
        let preview = RoomNotebookPreview {
            path: "Notebook/topic.md".to_string(),
            title: "Topic".to_string(),
            content_preview: "bounded body".to_string(),
            truncated: true,
            effective_at: DateTime::parse_from_rfc3339("2026-09-11T01:02:03Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let response = RoomNotebookResultsResponse {
            documents: vec![preview.clone()],
            warnings: vec![],
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["documents"][0]["contentPreview"], "bounded body");
        assert_eq!(value["documents"][0]["effectiveAt"], "2026-09-11T01:02:03Z");
        assert_eq!(
            serde_json::from_value::<RoomNotebookResultsResponse>(value).unwrap(),
            response
        );

        let read: RoomNotebookReadRequest = serde_json::from_value(serde_json::json!({
            "path": "Notebook/topic.md"
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(read).unwrap()["path"],
            "Notebook/topic.md"
        );
        let read_response = RoomNotebookReadResponse {
            path: "Notebook/topic.md".to_string(),
            content: "# Topic\n\nFull Markdown body.\n".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&read_response).unwrap()["content"],
            "# Topic\n\nFull Markdown body.\n"
        );

        let state = RoomStateListResponse {
            entities: vec![RoomStateEntity {
                entity: "project".to_string(),
                path: "State/entities/project.md".to_string(),
            }],
        };
        let state_value = serde_json::to_value(&state).unwrap();
        assert_eq!(state_value["entities"][0]["entity"], "project");
        assert_eq!(
            serde_json::from_value::<RoomStateListResponse>(state_value).unwrap(),
            state
        );
        let state_request: RoomStateReadRequest =
            serde_json::from_value(serde_json::json!({ "entity": "project" })).unwrap();
        assert_eq!(state_request.entity, "project");
    }

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
mod job_contract_tests {
    use super::*;

    fn sample_tool_response() -> JobToolResponse {
        JobToolResponse {
            job_id: "job-1".to_string(),
            group: None,
            kind: None,
            state: JobState::Running,
            elapsed_ms: Some(42),
            duration_ms: None,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            result: None,
            error: None,
            result_truncated: false,
            result_bytes: None,
            result_sha256: None,
            result_preview: None,
            result_omitted: false,
        }
    }

    #[test]
    fn job_group_validation_trims_and_bounds_readable_text() {
        assert_eq!(
            normalize_job_group(Some("  direct work  ")).unwrap(),
            Some("direct work".to_string())
        );
        assert_eq!(normalize_job_group(None).unwrap(), None);
        assert_eq!(
            normalize_job_group(Some("   ")).unwrap_err(),
            JobGroupValidationError::Empty
        );
        assert_eq!(
            normalize_job_group(Some("work\tstream")).unwrap_err(),
            JobGroupValidationError::ControlCharacter
        );
        assert!(normalize_job_group(Some(&"界".repeat(JOB_GROUP_MAX_CHARS))).is_ok());
        assert_eq!(
            normalize_job_group(Some(&"界".repeat(JOB_GROUP_MAX_CHARS + 1))).unwrap_err(),
            JobGroupValidationError::TooLong
        );
        assert_eq!(JobGroupValidationError::TooLong.code(), "job_group_invalid");
    }

    #[test]
    fn managed_job_admission_group_is_additive_and_parent_scoped() {
        let exec: ExecRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "program": "true",
            "args": [],
            "needConfirm": false
        }))
        .unwrap();
        assert_eq!(exec.group, None);

        let batch: BatchExecRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "group": "chat-direct",
            "elements": [{"program": "true", "args": []}],
            "needConfirm": false
        }))
        .unwrap();
        assert_eq!(batch.group.as_deref(), Some("chat-direct"));

        let skill: SkillRunRequest = serde_json::from_value(serde_json::json!({
            "id": "demo",
            "path": "scripts/check.sh",
            "group": "chat-direct"
        }))
        .unwrap();
        assert_eq!(skill.group.as_deref(), Some("chat-direct"));

        let mcp: McpCallToolRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "serverId": "server",
            "toolName": "tool",
            "group": "chat-direct"
        }))
        .unwrap();
        assert_eq!(mcp.group.as_deref(), Some("chat-direct"));

        let mcp_batch: McpBatchRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "group": "chat-direct",
            "calls": [{"serverId": "server", "toolName": "tool"}]
        }))
        .unwrap();
        assert_eq!(mcp_batch.group.as_deref(), Some("chat-direct"));
        assert!(serde_json::to_value(&mcp_batch.calls[0])
            .unwrap()
            .get("group")
            .is_none());
    }

    #[test]
    fn job_list_and_wait_contracts_have_frozen_defaults() {
        let list: JobListRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(list.effective_limit(), 50);
        assert_eq!(list.group, None);
        assert_eq!(list.cursor, None);

        let oversized: JobListRequest = serde_json::from_value(serde_json::json!({
            "limit": 999,
            "group": "work",
            "cursor": "opaque"
        }))
        .unwrap();
        assert_eq!(oversized.effective_limit(), 100);
        assert_eq!(oversized.group.as_deref(), Some("work"));
        assert_eq!(oversized.cursor.as_deref(), Some("opaque"));

        let get: JobGetRequest = serde_json::from_value(serde_json::json!({
            "jobId": "job-1"
        }))
        .unwrap();
        assert!(!get.wait_only);
        assert!(serde_json::to_value(&get)
            .unwrap()
            .get("waitOnly")
            .is_none());

        let wait = JobWaitResponse {
            job_id: "job-1".to_string(),
            state: JobState::Running,
            elapsed_ms: 42,
        };
        assert_eq!(
            serde_json::to_value(wait).unwrap(),
            serde_json::json!({"jobId":"job-1","state":"running","elapsedMs":42})
        );
    }

    #[test]
    fn slim_job_views_omit_routine_noise_and_keep_batch_budget_semantics() {
        let active = serde_json::to_value(sample_tool_response()).unwrap();
        assert_eq!(
            active,
            serde_json::json!({"jobId":"job-1","state":"running","elapsedMs":42})
        );

        let mut omitted = sample_tool_response();
        omitted.state = JobState::Completed;
        omitted.elapsed_ms = None;
        omitted.duration_ms = Some(7);
        omitted.result_omitted = true;
        let batch = McpBatchToolResponse {
            status: McpBatchStatus::Completed,
            error: None,
            results: vec![McpBatchToolChildResponse { job: omitted }],
        };
        let value = serde_json::to_value(batch).unwrap();
        assert_eq!(value["results"][0]["resultOmitted"], true);
        assert!(value.get("completedInline").is_none());
        assert!(value.get("pollAfterMs").is_none());
        assert!(value.get("aggregateTruncated").is_none());
    }

    #[test]
    fn job_info_can_represent_not_started_without_fabricated_timestamp() {
        let now = Utc::now();
        let info = JobInfo {
            agent_id: "agent".to_string(),
            job_id: "job-1".to_string(),
            group: Some("work".to_string()),
            batch_id: None,
            batch_call_id: None,
            batch_index: None,
            kind: JobKind::Process,
            state: JobState::Queued,
            created_at: now,
            started_at: None,
            updated_at: now,
            finished_at: None,
            program: None,
            args: Vec::new(),
            working_directory: None,
            command_preview: None,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason: None,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: None,
            mcp_tool_name: None,
            cancel_requested: false,
            cancel_outcome: None,
            termination_evidence: None,
        };
        let value = serde_json::to_value(info).unwrap();
        assert_eq!(value["group"], "work");
        assert!(value.get("startedAt").is_none());
    }
}

#[cfg(test)]
mod tmux_tests {
    use super::*;

    #[test]
    fn paste_and_close_default_to_confirmation() {
        let paste: TmuxPasteTextRequest = serde_json::from_value(serde_json::json!({
            "target": "%0",
            "text": "status"
        }))
        .unwrap();
        let close: TmuxCloseSessionRequest =
            serde_json::from_value(serde_json::json!({ "name": "agentic" })).unwrap();
        assert!(paste.need_confirm);
        assert!(close.need_confirm);
        assert!(!paste.submit);
    }

    #[test]
    fn tmux_exec_defaults_to_structured_non_forced_confirmation_request() {
        let request: TmuxExecRequest = serde_json::from_value(serde_json::json!({
            "target": "%0",
            "program": "git",
            "args": ["status"]
        }))
        .unwrap();
        assert_eq!(request.program, "git");
        assert_eq!(request.args, ["status"]);
        assert!(!request.need_confirm);
        assert_eq!(request.wait_ms, 300);
        assert_eq!(request.capture_lines, 120);
    }

    #[test]
    fn skills_command_serde_names_are_public_interface_names() {
        let command = HubCommand::SkillsRead {
            request_id: "req".to_string(),
            payload: SkillReadRequest {
                id: "demo".to_string(),
                path: None,
            },
        };
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["type"], "skills.read");
        assert_eq!(value["requestId"], "req");
        assert_eq!(value["payload"]["id"], "demo");

        let active = ActiveSkill {
            id: "missing".to_string(),
            activated_at: Utc::now(),
            status: "missing".to_string(),
            stale: true,
            summary: None,
        };
        let serialized = serde_json::to_string(&active).unwrap();
        assert!(!serialized.contains("summary"));
    }

    #[test]
    fn skill_read_path_is_additive_and_install_source_is_discriminated() {
        let request: SkillReadRequest = serde_json::from_value(serde_json::json!({
            "id": "demo"
        }))
        .unwrap();
        assert_eq!(request.path, None);

        let source = SkillInstallSource::Github {
            repository: Some("owner/repo".to_string()),
            url: None,
            ref_name: Some("release/v1".to_string()),
            path: Some("skills/demo".to_string()),
        };
        let serialized = serde_json::to_value(source).unwrap();
        assert_eq!(serialized["type"], "github");
        assert_eq!(serialized["repository"], "owner/repo");
        assert_eq!(serialized["ref"], "release/v1");
        assert_eq!(serialized["path"], "skills/demo");

        let legacy_response = serde_json::json!({
            "skill": {
                "id": "demo",
                "skillMd": "# Demo",
                "frontmatter": {},
                "tags": [],
                "active": true,
                "packageSummary": {
                    "hasAssets": false,
                    "hasScripts": false,
                    "hasReferences": false
                },
                "warnings": []
            }
        });
        let response: SkillReadResponse = serde_json::from_value(legacy_response).unwrap();
        assert!(response.resource.is_none());
        assert!(!serde_json::to_string(&response)
            .unwrap()
            .contains("resource"));
    }

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
        assert!(value.get("jobId").is_none());
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
    fn skill_wait_seconds_are_bounded_without_overflow() {
        let wait_values = [0, 30, 31, u64::MAX];
        let expected_values = [0, 30, 30, 30];

        for (&wait_seconds, &expected) in wait_values.iter().zip(expected_values.iter()) {
            let get: SkillInstallGetRequest = serde_json::from_value(serde_json::json!({
                "installId": "install-1",
                "waitSeconds": wait_seconds
            }))
            .unwrap();
            assert_eq!(get.effective_wait_seconds(), expected);

            let run: SkillRunRequest = serde_json::from_value(serde_json::json!({
                "id": "demo",
                "path": "scripts/check.sh",
                "waitSeconds": wait_seconds
            }))
            .unwrap();
            assert_eq!(run.effective_wait_seconds(), expected);
        }
    }

    #[test]
    fn bootstrap_commands_and_enums_use_public_spellings() {
        let bootstrap = HubCommand::RoomBootstrap {
            request_id: "req-bootstrap".to_string(),
        };
        let bootstrap_value = serde_json::to_value(bootstrap).unwrap();
        assert_eq!(bootstrap_value["type"], "room.bootstrap");
        assert_eq!(bootstrap_value["requestId"], "req-bootstrap");

        let read = HubCommand::RoomBootstrapRead {
            request_id: "req-read".to_string(),
            payload: BootstrapReadRequest {
                id: "diary".to_string(),
            },
        };
        let read_value = serde_json::to_value(read).unwrap();
        assert_eq!(read_value["type"], "room.bootstrap.read");
        assert_eq!(read_value["requestId"], "req-read");
        assert_eq!(read_value["payload"]["id"], "diary");

        assert_eq!(
            serde_json::to_value(BootstrapDocumentKind::Entrypoint).unwrap(),
            "entrypoint"
        );
        assert_eq!(
            serde_json::to_value(BootstrapDocumentKind::Guide).unwrap(),
            "guide"
        );
        assert_eq!(
            serde_json::to_value(BootstrapLoadPolicy::OnDemand).unwrap(),
            "on_demand"
        );
        assert_eq!(
            serde_json::to_value(BootstrapEncoding::Utf8).unwrap(),
            "utf8"
        );

        let neutral = HubCommand::Bootstrap {
            request_id: "req-neutral".to_string(),
        };
        assert_eq!(neutral.request_id(), "req-neutral");
        assert_eq!(serde_json::to_value(neutral).unwrap()["type"], "bootstrap");

        let neutral_read = HubCommand::BootstrapRead {
            request_id: "req-neutral-read".to_string(),
            payload: BootstrapReadRequest {
                id: "guide".to_string(),
            },
        };
        assert_eq!(neutral_read.request_id(), "req-neutral-read");
        assert_eq!(
            serde_json::to_value(neutral_read).unwrap()["type"],
            "bootstrap.read"
        );
    }
    #[test]
    fn current_room_commands_use_nested_payloads_and_public_names() {
        let commands = [
            (
                HubCommand::RoomDiaryActive {
                    request_id: "diary-active".to_string(),
                    payload: RoomDiaryActiveRequest::default(),
                },
                "room.diary.active",
            ),
            (
                HubCommand::RoomDiaryRead {
                    request_id: "diary-read".to_string(),
                    payload: RoomDiaryReadRequest {
                        layer: RoomDiaryLayer::Daily,
                        period: "current".to_string(),
                    },
                },
                "room.diary.read",
            ),
            (
                HubCommand::RoomNotebookRecent {
                    request_id: "notebook-recent".to_string(),
                    payload: RoomNotebookRecentRequest::default(),
                },
                "room.notebook.recent",
            ),
            (
                HubCommand::RoomNotebookSearch {
                    request_id: "notebook-search".to_string(),
                    payload: RoomNotebookSearchRequest {
                        query: "room".to_string(),
                        limit: Some(3),
                    },
                },
                "room.notebook.search",
            ),
            (
                HubCommand::RoomNotebookRead {
                    request_id: "notebook-read".to_string(),
                    payload: RoomNotebookReadRequest {
                        path: "Notebook/topic.md".to_string(),
                    },
                },
                "room.notebook.read",
            ),
            (
                HubCommand::RoomStateList {
                    request_id: "state-list".to_string(),
                    payload: RoomStateListRequest::default(),
                },
                "room.state.list",
            ),
            (
                HubCommand::RoomStateRead {
                    request_id: "state-read".to_string(),
                    payload: RoomStateReadRequest {
                        entity: "project".to_string(),
                    },
                },
                "room.state.read",
            ),
            (
                HubCommand::RoomMaintenanceStatus {
                    request_id: "maintenance-status".to_string(),
                    payload: RoomMaintenanceStatusRequest::default(),
                },
                "room.maintenance.status",
            ),
            (
                HubCommand::RoomMaintenanceSubmit {
                    request_id: "maintenance-submit".to_string(),
                    payload: RoomMaintenanceSubmitRequest {
                        items: vec![RoomMaintenanceRequestItem {
                            slot: RoomMaintenanceSlot::Entity,
                            payload: serde_json::json!({
                                "entity": "project",
                                "content": "body"
                            }),
                        }],
                        mode: Some(RoomMaintenanceExecutionMode::Local),
                        wait_seconds: Some(0),
                    },
                },
                "room.maintenance.submit",
            ),
        ];

        for (command, name) in commands {
            let request_id = command.request_id().to_string();
            let value = serde_json::to_value(command).unwrap();
            assert_eq!(value["type"], name);
            assert_eq!(value["requestId"], request_id);
            assert!(value["payload"].is_object());
        }
    }

    #[test]
    fn bootstrap_resource_omits_only_absent_truncation_line() {
        let resource = BootstrapTextResource {
            path: "bootstrap.md".to_string(),
            encoding: BootstrapEncoding::Utf8,
            content: "---\nid: room\n---\n".to_string(),
            media_type: "text/markdown".to_string(),
            size_bytes: 18,
            returned_size_bytes: 18,
            total_lines: 3,
            returned_through_line: 3,
            omitted_from_line: None,
            truncated: false,
            last_line_complete: true,
            sha256: "a".repeat(64),
        };
        let value = serde_json::to_value(resource).unwrap();
        assert_eq!(value["mediaType"], "text/markdown");
        assert_eq!(value["sizeBytes"], 18);
        assert_eq!(value["returnedSizeBytes"], 18);
        assert_eq!(value["totalLines"], 3);
        assert_eq!(value["returnedThroughLine"], 3);
        assert_eq!(value["truncated"], false);
        assert_eq!(value["lastLineComplete"], true);
        assert!(value.get("omittedFromLine").is_none());

        let truncated: BootstrapTextResource = serde_json::from_value(serde_json::json!({
            "path": "guides/diary.md",
            "encoding": "utf8",
            "content": "line 1\n",
            "mediaType": "text/markdown",
            "sizeBytes": 100,
            "returnedSizeBytes": 7,
            "totalLines": 20,
            "returnedThroughLine": 1,
            "omittedFromLine": 2,
            "truncated": true,
            "lastLineComplete": true,
            "sha256": "b".repeat(64)
        }))
        .unwrap();
        assert_eq!(truncated.omitted_from_line, Some(2));
    }

    #[test]
    fn bootstrap_response_and_read_request_round_trip_with_camel_case_fields() {
        let response = BootstrapResponse {
            schema_version: 1,
            revision: "r".repeat(64),
            entrypoint: BootstrapEntrypoint {
                id: "room".to_string(),
                kind: BootstrapDocumentKind::Entrypoint,
                name: "Room Bootstrap".to_string(),
                description: "Route startup guides".to_string(),
                frontmatter: serde_json::json!({"schemaVersion": 1, "future": true}),
                resource: BootstrapTextResource {
                    path: "bootstrap.md".to_string(),
                    encoding: BootstrapEncoding::Utf8,
                    content: "content\n".to_string(),
                    media_type: "text/markdown".to_string(),
                    size_bytes: 8,
                    returned_size_bytes: 8,
                    total_lines: 1,
                    returned_through_line: 1,
                    omitted_from_line: None,
                    truncated: false,
                    last_line_complete: true,
                    sha256: "a".repeat(64),
                },
            },
            guides: vec![BootstrapGuideSummary {
                id: "diary".to_string(),
                kind: BootstrapDocumentKind::Guide,
                title: "Diary conventions".to_string(),
                summary: "Keep continuity".to_string(),
                load_policy: BootstrapLoadPolicy::Contextual,
                priority: 80,
                load_when: vec!["continued context".to_string()],
                tool_bindings: vec!["room.diary.active".to_string()],
                tags: vec!["continuity".to_string()],
                path: "guides/diary.md".to_string(),
                size_bytes: 10,
                total_lines: 2,
                sha256: "d".repeat(64),
            }],
            total_guides: 1,
            returned_guides: 1,
            warnings: vec![],
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(
            value["entrypoint"]["resource"]["mediaType"],
            "text/markdown"
        );
        assert_eq!(value["guides"][0]["loadPolicy"], "contextual");
        assert_eq!(value["guides"][0]["toolBindings"][0], "room.diary.active");
        assert_eq!(value["totalGuides"], 1);
        assert_eq!(value["returnedGuides"], 1);

        let round_trip: BootstrapResponse = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, response);

        let request: BootstrapReadRequest = serde_json::from_value(serde_json::json!({
            "id": "diary"
        }))
        .unwrap();
        assert_eq!(request.id, "diary");
        assert_eq!(serde_json::to_value(request).unwrap()["id"], "diary");
    }

    #[test]
    fn managed_mcp_call_defaults_and_bounds_are_frozen() {
        let defaults: McpCallToolRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "serverId": "server",
            "toolName": "tool",
            "arguments": {}
        }))
        .unwrap();
        assert_eq!(defaults.effective_wait_seconds(), 5);
        assert_eq!(defaults.effective_timeout_seconds(), 300);

        let bounded: McpCallToolRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "serverId": "server",
            "toolName": "tool",
            "arguments": {},
            "waitSeconds": 999,
            "timeoutSeconds": 9999
        }))
        .unwrap();
        assert_eq!(bounded.effective_wait_seconds(), 30);
        assert_eq!(bounded.effective_timeout_seconds(), 900);

        let minimum: McpCallToolRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "serverId": "server",
            "toolName": "tool",
            "arguments": {},
            "waitSeconds": 0,
            "timeoutSeconds": 0
        }))
        .unwrap();
        assert_eq!(minimum.effective_wait_seconds(), 0);
        assert_eq!(minimum.effective_timeout_seconds(), 1);
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

    #[test]
    fn hello_defaults_to_command_capable_when_generation_is_present() {
        let message: AgentMessage = serde_json::from_value(serde_json::json!({
            "type": "hello",
            "role": "normal",
            "bootGeneration": "boot-test",
            "configSummary": {
                "workspaceRoot": "/workspace",
                "sandbox": {"enabled": false, "mode": "disabled"},
                "pathPolicy": {
                    "writeRootCount": 0,
                    "readOnlyRootCount": 0,
                    "denyRootCount": 0,
                    "writeRoots": [],
                    "readOnlyRoots": [],
                    "denyRoots": []
                },
                "policyRuleCounts": {"allow": 0, "confirm": 0, "deny": 0},
                "policyRules": {
                    "allow": [], "confirm": [], "deny": [],
                    "builtins": {"confirm": [], "deny": []}
                },
                "confirmationProvider": "none"
            }
        }))
        .unwrap();
        assert!(matches!(
            message,
            AgentMessage::Hello {
                connection_mode: AgentConnectionMode::CommandCapable,
                ..
            }
        ));
    }

    #[test]
    fn hello_without_boot_generation_is_rejected() {
        let error = serde_json::from_value::<AgentMessage>(serde_json::json!({
            "type": "hello",
            "role": "normal",
            "configSummary": {
                "workspaceRoot": "/workspace",
                "sandbox": {"enabled": false, "mode": "disabled"},
                "pathPolicy": {
                    "writeRootCount": 0,
                    "readOnlyRootCount": 0,
                    "denyRootCount": 0,
                    "writeRoots": [],
                    "readOnlyRoots": [],
                    "denyRoots": []
                },
                "policyRuleCounts": {"allow": 0, "confirm": 0, "deny": 0},
                "policyRules": {
                    "allow": [], "confirm": [], "deny": [],
                    "builtins": {"confirm": [], "deny": []}
                },
                "confirmationProvider": "none"
            }
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains("bootGeneration"));
    }
}
