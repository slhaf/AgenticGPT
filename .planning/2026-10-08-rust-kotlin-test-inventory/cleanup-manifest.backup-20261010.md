# 测试清理执行清单

> 状态：**执行清单已编制，尚未执行**。依据用户 2026-10-08 已确认的保留价值判断及现有盘点资料。此文件只规定清理目标与执行约束；不表示代码已被修改或测试已通过。

## 执行规则

1. 仅处理下列明确列出的 **299 个删除目标**与 **8 个合并/精简目标**；**20 个保留目标不得因本轮清理而移除**。不得把本清单外的用例自动并入删除范围。
2. 299 个删除目标**不再开展价值复审**。按文件定位对应测试函数并删除其测试声明；不要删除生产功能、变更产品行为或顺便重构实现。若清单标识已不匹配当前源码，单独记录并跳过该项。
3. 8 个合并/精简目标按逐项说明处理：允许原位精简、把独有断言迁入具名现有测试后移除旧用例；不允许为凑删除数量丢失独有的长期有效行为检查。可以保持原测试不动并记录原因。
4. 以 `AGENTS.md` 当前五档定义与长期回归价值原则为约束，完成变更后执行受影响的 Rust/Kotlin 验证，并检查测试发现结果及失败原因。按模块/批次记录实际删除、合并、保留、跳过数量和对应测试结果。
5. 涉及必须修改生产实现、边界不明或与现有长期回归保障发生冲突的项目，只暂停有问题的条目并汇报；其余清单项目可继续。不要静默扩大修改范围。

## A. 删除目标（299 个）

来源计数：290 个纯 T1–T3 清理候选（从 304 个纯低档测试排除原初 14 个保留候选）＋6 个仅含 T1–T3 的 MIXED 清理候选＋3 个本轮复审新增清理项。以下均为**精确目标**，按源码文件归组。

### `console/shared/src/androidHostTest/kotlin/work/slhaf/agentic/console/SharedLogicAndroidHostTest.kt`（1）

- [ ] `example`

### `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/SharedCommonTest.kt`（1）

- [ ] `example`

### `console/shared/src/jvmTest/kotlin/work/slhaf/agentic/console/SharedLogicDesktopTest.kt`（1）

- [ ] `example`

### `crates/agentic-apply-patch/src/lib.rs`（1）

- [ ] `parses_patch`

### `crates/agentic-browser-host/src/lib.rs`（4）

- [ ] `bridge_status_is_local_and_reports_current_clients`
- [ ] `native_frame_round_trip_uses_uint32_prefix`
- [ ] `oversized_frame_is_rejected_before_reading_payload`
- [ ] `truncated_frames_match_baseline_eof_behavior`

### `crates/agentic-gpt-hub/src/agents/dispatch_tests.rs`（3）

- [ ] `failed_request_send_removes_current_connection`
- [ ] `pending_replay_sends_reliable_envelope`
- [ ] `reporting_only_connection_is_not_a_command_target`

### `crates/agentic-gpt-hub/src/agents/lifecycle_tests.rs`（6）

- [ ] `expired_connection_cleanup_removes_only_stale_current_entries`
- [ ] `generation_expiry_rechecks_current_liveness`
- [ ] `generation_stale_heartbeat_direct_handler`
- [ ] `guarded_send_rejects_captured_target_after_replacement`
- [ ] `stale_heartbeat_is_rejected_without_touching_current_connection`
- [ ] `stale_process_update_is_rejected_without_writing_process_cache`

### `crates/agentic-gpt-hub/src/ingress/http/routes.rs`（4）

- [ ] `event_mark_body_requires_an_agent_and_event_ids`
- [ ] `offline_event_list_returns_an_error_without_cached_events`
- [ ] `process_read_keeps_domain_errors_as_http_errors`
- [ ] `unavailable_process_read_reports_only_stale_cache_metadata`

### `crates/agentic-gpt-hub/src/ingress/mcp/args.rs`（1）

- [ ] `process_exec_arguments_accept_command_cwd_and_reject_legacy_fields`

### `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs`（11）

- [ ] `bootstrap_timeout_values_preserve_operation_specific_codes`
- [ ] `coordinator_profile_exposes_only_native_tools`
- [ ] `full_profile_keeps_bootstrap_aliases_and_execution_surface`
- [ ] `native_tool_values_use_agentic_result_shape`
- [ ] `offline_native_process_cache_tools_omit_events`
- [ ] `process_read_arg_schema_matches_contract_and_defaults`
- [ ] `room_mcp_input_schemas_do_not_include_agent_id`
- [ ] `skill_install_and_run_tools_are_exposed_with_stable_annotations`
- [ ] `tmux_exec_schema_exposes_snapshot_fields`
- [ ] `tmux_paste_schema_exposes_confirmation_default_field`
- [ ] `tool_read_only_hints_match_side_effect_semantics`

### `crates/agentic-gpt-hub/src/notifications/notify.rs`（5）

- [ ] `android_registered_endpoint_still_reports_delivery_not_implemented`
- [ ] `notification_channels_include_agent_ntfy_and_android_placeholders`
- [ ] `ntfy_default_placeholder_is_not_configured`
- [ ] `ntfy_health_cache_controls_listing_reason`
- [ ] `parses_notify_channel_keys`

### `crates/agentic-gpt-hub/src/room/room.rs`（11）

- [ ] `bootstrap_error_codes_map_to_frozen_http_statuses`
- [ ] `first_room_agent_becomes_active_room`
- [ ] `normal_agent_is_not_room_api_fallback`
- [ ] `read_and_maintenance_room_api_without_active_room_returns_not_active`
- [ ] `room_api_after_replacement_without_hello_returns_not_active`
- [ ] `room_api_without_active_room_returns_not_active`
- [ ] `same_agent_normal_hello_releases_old_active_room`
- [ ] `same_agent_replacement_without_hello_does_not_leave_stale_active_room`
- [ ] `same_room_agent_reconnect_replaces_old_room_connection`
- [ ] `second_different_room_agent_is_rejected`
- [ ] `stale_room_disconnect_does_not_release_new_room_connection`

### `crates/agentic-gpt-hub/src/runtime/cli.rs`（1）

- [ ] `cli_version_uses_crate_version`

### `crates/agentic-gpt-hub/src/runtime/config.rs`（1）

- [ ] `safe_default_summary_has_no_paths_or_secrets`

### `crates/agentic-gpt-hub/src/runtime/main_tests.rs`（4）

- [ ] `bootstrap_commands_have_run_types`
- [ ] `ntfy_mcp_confirmation_stays_within_three_action_limit`
- [ ] `parses_bearer_case_insensitively`
- [ ] `skills_commands_have_run_types`

### `crates/agentic-gpt-hub/src/runtime/state.rs`（2）

- [ ] `process_cache_age_generation_and_ordering_are_truthful`
- [ ] `process_cache_capacity_is_bounded`

### `crates/agentic-gpt-hub/src/storage/event_feedback.rs`（2）

- [ ] `public_flush_waits_for_active_agent_barrier`
- [ ] `queued_recovery_subsets_preserve_all_known_identities`

### `crates/agentic-gpt-hub/src/support/agentic_result.rs`（2）

- [ ] `wraps_native_json_as_structured_error_when_value_has_error`
- [ ] `wraps_native_json_as_structured_success_when_value_has_no_error`

### `crates/agentic-gpt-protocol/src/envelopes.rs`（2）

- [ ] `event_hub_commands_keep_request_identity_and_agent_scope`
- [ ] `mcp_list_servers_panel_suppression_is_internal_and_legacy_compatible`

### `crates/agentic-gpt-protocol/src/lib.rs`（14）

- [ ] `bootstrap_commands_and_enums_use_public_spellings`
- [ ] `bootstrap_resource_omits_only_absent_truncation_line`
- [ ] `bootstrap_response_and_read_request_round_trip_with_camel_case_fields`
- [ ] `current_room_commands_use_nested_payloads_and_public_names`
- [ ] `diary_v2_shapes_keep_periods_semantic_and_missing_layers_explicit`
- [ ] `hello_defaults_to_command_capable_when_generation_is_present`
- [ ] `hello_without_boot_generation_is_rejected`
- [ ] `managed_mcp_call_defaults_and_bounds_are_frozen`
- [ ] `notebook_and_state_v2_shapes_bound_previews_and_read_markdown_exactly`
- [ ] `paste_and_close_default_to_confirmation`
- [ ] `skill_read_path_is_additive_and_install_source_is_discriminated`
- [ ] `skill_wait_seconds_are_bounded_without_overflow`
- [ ] `skills_command_serde_names_are_public_interface_names`
- [ ] `tmux_exec_defaults_to_structured_non_forced_confirmation_request`

### `crates/agentic-gpt-protocol/src/process.rs`（2）

- [ ] `process_exec_wire_uses_command_and_cwd_and_rejects_legacy_argv_fields`
- [ ] `unified_process_response_has_compact_identity_and_observation_fields`

### `crates/agentic-gpt/src/browser/browser_distribution_tests.rs`（9）

- [ ] `authenticated_inrelease_metadata_rejects_duplicates_and_contradictions`
- [ ] `cleartext_signature_hash_policy_is_sha2_only`
- [ ] `injected_fetcher_rejects_status_and_metadata_bounds`
- [ ] `package_selector_rejects_ambiguity_duplicate_fields_and_unsafe_names`
- [ ] `package_selector_validates_required_identity_fields`
- [ ] `packages_digest_and_size_are_verified_before_selection`
- [ ] `release_date_parser_accepts_rfc2822_timestamp`
- [ ] `selected_entry_and_size_bounds_are_enforced_without_large_fixtures`
- [ ] `selected_paths_reject_traversal_and_empty_aliases`

### `crates/agentic-gpt/src/browser/browser_kernel.rs`（19）

- [ ] `bootstrap_converts_tool_error_to_stable_failure`
- [ ] `bootstrap_preserves_rmcp_service_failure_prefix`
- [ ] `bootstrap_sentinel_maps_to_browser_unavailable`
- [ ] `command_helper_uses_direct_program_cwd_and_env_overrides`
- [ ] `constructor_rejects_empty_or_whitespace_session_ids`
- [ ] `constructor_rejects_empty_or_whitespace_turn_ids`
- [ ] `js_attaches_exact_codex_turn_metadata`
- [ ] `js_keeps_tool_error_result_as_successful_transport_return`
- [ ] `js_prefixes_rmcp_service_failures`
- [ ] `js_preserves_mixed_result_fields_unchanged`
- [ ] `js_sends_exact_tool_name_and_arguments`
- [ ] `node_repl_client_info_uses_the_frozen_initialize_contract`
- [ ] `reset_js_converts_tool_error_to_stable_failure`
- [ ] `reset_js_prefixes_rmcp_service_failures`
- [ ] `reset_js_sends_exact_tool_name_and_empty_arguments`
- [ ] `spawn_rejects_invalid_ids_before_attempting_process_creation`
- [ ] `turn_ended_converts_tool_error_to_stable_failure`
- [ ] `turn_ended_prefixes_rmcp_service_failures`
- [ ] `turn_ended_sends_exact_tool_name_and_arguments`

### `crates/agentic-gpt/src/browser/browser_manual.rs`（1）

- [ ] `search_rejects_query_and_context_bounds`

### `crates/agentic-gpt/src/browser/browser_runtime.rs`（13）

- [ ] `explicit_descriptor_derives_shared_docs_and_trusted_paths`
- [ ] `explicit_descriptor_rejects_empty_and_relative_fields`
- [ ] `launch_spec_creates_browser_only_trusted_services_when_absent`
- [ ] `launch_spec_creates_trusted_code_paths_when_base_omits_them`
- [ ] `launch_spec_derives_program_and_cwd_from_descriptor`
- [ ] `launch_spec_does_not_create_empty_node_module_dirs`
- [ ] `launch_spec_empty_node_module_dirs_preserves_base_value`
- [ ] `launch_spec_merges_and_deduplicates_trusted_code_paths`
- [ ] `launch_spec_overwrites_stale_runtime_coupled_values`
- [ ] `launch_spec_preserves_security_mode_without_inventing_it`
- [ ] `launch_spec_preserves_unrelated_base_environment`
- [ ] `launch_spec_rejects_invalid_trusted_services`
- [ ] `launch_spec_replaces_browser_trusted_service_and_preserves_others`

### `crates/agentic-gpt/src/config/config_cli.rs`（5）

- [ ] `interactive_init_requires_all_three_terminals_and_no_non_interactive_flag`
- [ ] `registry_applies_new_scalar_and_list_keys`
- [ ] `registry_keys_are_unique_and_have_bilingual_metadata`
- [ ] `setup_seed_conversion_preserves_editable_flags_and_redacts_agent_secret`
- [ ] `toolset_listing_describes_all_namespaces_and_feedback_is_localized`

### `crates/agentic-gpt/src/config/config_templates.rs`（7）

- [ ] `explicit_confirmation_language_wins_and_normal_room_override_requires_room_toolset`
- [ ] `local_mode_ignores_tunnel_inputs`
- [ ] `mcp_servers_flow_into_built_config`
- [ ] `normal_profile_accepts_room_override_with_explicit_room_toolset`
- [ ] `partial_hub_template_rejects_invalid_url_and_transport_with_stable_errors`
- [ ] `pending_actions_are_deterministic_and_unique`
- [ ] `secret_debug_output_is_redacted`

### `crates/agentic-gpt/src/config/config.rs`（14）

- [ ] `auto_max_active_processes_uses_the_frozen_formula`
- [ ] `confirmation_provider_disk_shape_rejects_legacy_provider`
- [ ] `confirmation_provider_rejects_duplicate_or_unknown_channels`
- [ ] `empty_browser_section_is_omitted_from_sparse_defaults`
- [ ] `file_search_context_limit_defaults_and_rejects_invalid_values`
- [ ] `http_bearer_file_references_require_absolute_paths_without_tightening_tunnel_refs`
- [ ] `hub_validation_rejects_invalid_url_and_transport_with_stable_errors`
- [ ] `legacy_confirmation_labels_map_to_canonical_fallback_order`
- [ ] `limits_reject_retired_max_active_jobs_field`
- [ ] `managed_browser_policy_defaults_are_enabled_without_auto_provisioning` — 复审新增：锁定当前 managed browser 默认启用、默认不自动安装的产品策略；未来变更并不天然属于回归。原候选清单的文件路径误写为 browser/browser_runtime.rs。
- [ ] `managed_browser_policy_rejects_unknown_fields`
- [ ] `max_active_processes_supports_auto_and_explicit_round_trips`
- [ ] `mcp_server_semantics_are_validated_before_standalone_use`
- [ ] `shell_init_file_serde_preserves_default_disabled_and_explicit_paths`

### `crates/agentic-gpt/src/config/setup/model.rs`（5）

- [ ] `malformed_tunnel_secret_reference_is_reported_as_a_field_error`
- [ ] `optional_status_and_drafts_survive_mode_and_profile_changes`
- [ ] `room_availability_uses_profile_preset_without_explicit_toolset_selection`
- [ ] `setup_defaults_to_standalone_normal_and_preserves_inactive_mode_seeds`
- [ ] `tunnel_secret_reference_seeds_are_parsed_without_exposing_secret_text`

### `crates/agentic-gpt/src/config/setup/review.rs`（3）

- [ ] `review_reports_default_and_configured_optional_statuses`
- [ ] `review_rows_expose_stable_edit_contract_without_secret_material`
- [ ] `shell_review_shows_path_only_for_explicit_path_mode`

### `crates/agentic-gpt/src/config/setup/validation.rs`（5）

- [ ] `hub_connection_transport_and_secret_are_structured`
- [ ] `normal_profile_with_explicit_room_disabled_toolset_hides_and_rejects_room`
- [ ] `optional_validation_covers_paths_numbers_runtime_paths_and_reporting`
- [ ] `process_response_bytes_limit_accepts_protocol_bounds_and_rejects_invalid_values`
- [ ] `required_connection_fields_report_concrete_domain_fields`

### `crates/agentic-gpt/src/files/file_ops.rs`（5）

- [ ] `apply_patch_parser_handles_add_delete_update_and_move_without_fs_access`
- [ ] `bounded_diff_counts_create_delete_crlf_and_final_newline`
- [ ] `bounded_diff_preserves_blank_lines_and_emits_disjoint_hunks`
- [ ] `bounded_diff_truncates_utf8_after_computing_complete_counts`
- [ ] `rejects_duplicate_and_ancestor_patch_paths`

### `crates/agentic-gpt/src/ingress/http_oauth.rs`（9）

- [ ] `chatgpt_redirect_families_are_exact`
- [ ] `expired_oauth_tokens_are_pruned_and_rejected`
- [ ] `host_and_origin_matching_follow_rmcp_policy`
- [ ] `oauth_error_and_html_responses_are_uncacheable`
- [ ] `oauth_tokens_reject_wrong_resource_binding`
- [ ] `pkce_rfc7636_s256_vector_matches`
- [ ] `public_url_normalization_uses_config_contract`
- [ ] `standalone_page_escapes_hidden_fields_and_avoids_hub_copy`
- [ ] `token_rotation_revokes_oauth_state_but_direct_bearer_changes`

### `crates/agentic-gpt/src/ingress/hub.rs`（1）

- [ ] `oversized_report_json_becomes_a_hash_record`

### `crates/agentic-gpt/src/ingress/local_control.rs`（1）

- [ ] `socket_path_rejects_invalid_identity_and_oversized_path`

### `crates/agentic-gpt/src/ingress/stdio_server_tests.rs`（19）

- [ ] `active_response_precedes_terminal_for_both_serial_orderings`
- [ ] `batch_lifecycle_detection_reads_process_envelopes`
- [ ] `browser_conditional_arguments_and_bounds_are_validated`
- [ ] `browser_descriptors_and_annotations_are_frozen`
- [ ] `browser_reporting_and_error_helpers_do_not_retain_source`
- [ ] `concurrent_check_clear_enqueue_interleaving_is_linearizable`
- [ ] `event_api_schemas_preserve_defaults_and_empty_mark_boundary`
- [ ] `every_room_adapter_rejects_unknown_identity_fields`
- [ ] `file_lock_registry_prunes_released_paths`
- [ ] `file_surface_schema_is_exact`
- [ ] `inline_terminal_tracker_discards_pending_terminal_event`
- [ ] `managed_terminal_event_includes_duration`
- [ ] `mcp_batch_projection_preserves_compact_observation_and_child_identity`
- [ ] `mutating_tool_annotations_do_not_promise_read_only_or_additive_effects`
- [ ] `process_read_input_schema_advertises_wait_view_and_response_budget`
- [ ] `room_maintenance_descriptors_are_frozen`
- [ ] `room_maintenance_submit_rejects_unknown_nested_fields`
- [ ] `tmux_actions_reject_incompatible_fields`
- [ ] `tunnel_local_and_http_ingress_advertise_identical_surface` — 复审新增：把 Tunnel、Local 与 HTTP 三入口强制绑定为完全相同的工具表；未来合理分化会造成误报。执行前确认入口实际授权与契约测试仍覆盖可见性和来源标识。

### `crates/agentic-gpt/src/mcp/mcp_tests.rs`（2）

- [ ] `server_config_revision_is_deterministic_and_content_sensitive`
- [ ] `streamable_http_bearer_auth_is_validated_and_injected`

### `crates/agentic-gpt/src/operations/confirmation.rs`（1）

- [ ] `batch_confirmation_shows_the_original_script_and_working_directory`

### `crates/agentic-gpt/src/operations/event_notifications.rs`（4）

- [ ] `active_process_response_remains_eligible_for_async_completion`
- [ ] `deduplicated_install_response_does_not_settle_original_source`
- [ ] `mixed_batch_classifies_each_process_from_its_own_terminal_state`
- [ ] `terminal_process_observation_suppresses_completion_with_incomplete_capture`

### `crates/agentic-gpt/src/operations/operation.rs`（4）

- [ ] `cli_admission_is_limited_to_local_admin_tmux_operations`
- [ ] `normal_hub_cannot_acquire_room_capability_by_operation_name`
- [ ] `process_read_keeps_process_authorization_across_agent_profiles_and_hub`
- [ ] `same_prefix_unknown_operations_fail_closed`

### `crates/agentic-gpt/src/operations/policy.rs`（8）

- [ ] `dynamic_words_redirects_and_complex_scripts_never_use_partial_allows`
- [ ] `escaped_crlf_cannot_hide_a_default_denied_command`
- [ ] `escaped_word_separator_requires_confirmation_instead_of_partial_matching`
- [ ] `known_deny_wins_inside_unsupported_or_incomplete_scripts`
- [ ] `shell_confirmation_cannot_be_removed_by_an_allow_rule`
- [ ] `shell_requires_allow_for_every_literal_command`
- [ ] `shell_supports_literal_quotes_concatenation_and_supported_operators`
- [ ] `terminal_dollar_in_a_quoted_argument_is_not_truncated`

### `crates/agentic-gpt/src/operations/shell_parser.rs`（5）

- [ ] `dynamic_and_expanding_words_are_not_literal`
- [ ] `escaped_crlf_does_not_hide_the_following_command`
- [ ] `escaped_word_separator_does_not_preserve_truncated_arguments`
- [ ] `extracts_quoted_words_and_literal_concatenation`
- [ ] `quoted_terminal_dollars_and_newlines_are_kept`

### `crates/agentic-gpt/src/process/exec.rs`（1）

- [ ] `shell_argument_limit_counts_exact_bootstrap_and_keeps_boundary`

### `crates/agentic-gpt/src/process/managed.rs`（5）

- [ ] `failed_process_reasons_are_projected_without_fabricating_exit_errors`
- [ ] `output_overflow_reports_exact_retained_gap`
- [ ] `process_error_messages_are_utf8_safe_and_bounded`
- [ ] `process_read_rejects_output_pages_that_cannot_advance`
- [ ] `skill_leases_still_block_updates`

### `crates/agentic-gpt/src/room/bootstrap.rs`（2）

- [ ] `guide_directory_entry_errors_are_reported_without_dropping_readable_entries`
- [ ] `resource_truncation_is_line_aware_and_utf8_safe`

### `crates/agentic-gpt/src/room/room_reads.rs`（2）

- [ ] `diary_periods_use_strict_direct_layer_paths`
- [ ] `semantic_paths_reject_arbitrary_repository_files`

### `crates/agentic-gpt/src/room/room_repository.rs`（1）

- [ ] `logical_day_respects_the_configured_shanghai_boundary`

### `crates/agentic-gpt/src/runtime/main_tests.rs`（22）

- [ ] `batch_confirmation_preview_supports_chinese`
- [ ] `cli_version_uses_crate_version`
- [ ] `configured_allow_overrides_builtin_confirm`
- [ ] `configured_allow_overrides_builtin_deny`
- [ ] `configured_allow_overrides_need_confirm`
- [ ] `configured_deny_wins_when_multiple_config_rules_match`
- [ ] `configured_room_repository_root_overrides_default`
- [ ] `explicit_runtime_precedes_desktop_discovery_and_invalid_does_not_fallback`
- [ ] `local_cli_accepts_config_before_or_after_subcommand`
- [ ] `notification_delivery_rejects_unsupported_channel`
- [ ] `public_run_has_only_a_config_path_and_no_profile_override`
- [ ] `remove_rule_matches_command_and_args_prefix`
- [ ] `remove_rule_matches_command_without_uuid`
- [ ] `remove_rule_refuses_ambiguous_non_interactive_match`
- [ ] `room_policy_keeps_high_risk_commands_restricted`
- [ ] `room_policy_overlay_differs_from_normal_policy`
- [ ] `rule_matches_program_and_args_prefix_structurally`
- [ ] `run_as_room_uses_workspace_default_repository_root`
- [ ] `safe_summary_includes_path_roots_and_policy_rules`
- [ ] `sse_post_status_classification_stops_on_stale_connection`
- [ ] `standalone_reload_replaces_the_frozen_live_subset`
- [ ] `sudo_requires_credentials`

### `crates/agentic-gpt/src/runtime/state.rs`（1）

- [ ] `runtime_capabilities_follow_transport_and_profile`

### `crates/agentic-gpt/src/runtime/supervisor.rs`（9）

- [ ] `doctor_diagnostic_output_is_bounded_and_redacted`
- [ ] `forwarded_child_lines_preserve_known_severity_and_strip_timestamp`
- [ ] `forwarded_journal_lines_preserve_untimestamped_severity_after_redaction`
- [ ] `mcp_binding_preserves_worker_tokenization`
- [ ] `restart_decision_covers_retry_permanent_and_exhausted`
- [ ] `restart_identity_warning_compares_to_immutable_runtime_and_warns_once`
- [ ] `retry_schedule_is_bounded_and_exponential`
- [ ] `runtime_paths_reject_path_injection`
- [ ] `worker_command_quotes_paths_and_never_contains_api_key`

### `crates/agentic-gpt/src/runtime/tunnel_distribution.rs`（5）

- [ ] `archive_hash_mismatch_is_checked_before_install`
- [ ] `archive_rejects_traversal_symlinks_duplicates_and_extra_files`
- [ ] `artifact_selection_requires_pinned_or_explicit_trust`
- [ ] `download_url_requires_https_outside_tests`
- [ ] `manifest_and_platforms_are_pinned` — 复审新增：锁死阶段性的分发版本、URL、SHA-256；其他测试覆盖制品选择和哈希拒绝。但若团队明确要求第二处独立固定值审计，则应重新评估。

### `crates/agentic-gpt/src/skills/skill_installs.rs`（7）

- [ ] `github_sources_resolve_structured_and_convenience_forms`
- [ ] `idempotency_conflict_is_rejected_before_creating_a_second_job`
- [ ] `idempotency_retries_return_the_original_install`
- [ ] `package_paths_reject_case_conflicts_and_file_directory_collisions`
- [ ] `queued_cancel_is_idempotent_and_status_is_terminal`
- [ ] `remote_policy_rejects_private_and_reserved_addresses`
- [ ] `transient_download_errors_are_the_only_retryable_materialization_failures`

### `crates/agentic-gpt/src/storage/event_store.rs`（1）

- [ ] `low_ttl_out_of_range_returns_an_error_without_panicking`

### `crates/agentic-gpt/src/storage/private_state.rs`（1）

- [ ] `legacy_wide_agent_id_uses_stable_safe_state_key`

### `crates/agentic-gpt/src/support/utils.rs`（4）

- [ ] `compact_id_has_a_stable_twelve_hex_digit_body`
- [ ] `journal_rendering_omits_inner_timestamp_only_in_journal_mode`
- [ ] `mcp_argument_key_summary_is_counted_and_bounded`
- [ ] `mcp_confirmation_preview_is_sorted_bounded_metadata_without_values`

### `crates/agentic-gpt/src/tmux/tmux.rs`（4）

- [ ] `cd_requires_one_explicit_path`
- [ ] `parses_session_and_pane_metadata`
- [ ] `session_scoped_pane_listing_does_not_request_all_panes`
- [ ] `shell_detection_and_argument_quoting_are_structural`

### `crates/agentic-gpt/src/ui/cli_i18n.rs`（5）

- [ ] `every_catalog_entry_is_non_empty_for_each_language`
- [ ] `explicit_language_overrides_locale_environment`
- [ ] `locale_precedence_is_lc_all_then_lc_messages_then_lang`
- [ ] `localized_command_tree_has_complete_visible_metadata`
- [ ] `prescan_accepts_equals_and_split_forms_anywhere`

### `crates/agentic-gpt/src/ui/config_tui/pages.rs`（1）

- [ ] `shell_path_field_is_focusable_only_for_explicit_path_mode`

### `crates/agentic-gpt/src/ui/tui/forms/state.rs`（2）

- [ ] `empty_list_can_add_first_item_and_delete_back_to_empty`
- [ ] `list_state_adds_edits_deletes_and_keeps_focus_valid`

### `crates/agentic-gpt/src/ui/tui/layout.rs`（2）

- [ ] `master_detail_collapses_to_active_pane`
- [ ] `surface_cursor_honors_reserved_bottom_rows`

### `crates/agentic-gpt/src/ui/tui/process.rs`（1）

- [ ] `clipping_and_short_id_are_bounded`

### `crates/agentic-gpt/src/ui/tui/runtime.rs`（1）

- [ ] `restoration_seam_records_cleanup_in_reverse_setup_order`

### `crates/agentic-gpt/src/ui/tui/shell.rs`（2）

- [ ] `overlay_stays_inside_small_terminal`
- [ ] `shell_uses_config_tui_margin_and_fixed_chrome_rows`

### `crates/agentic-gpt/src/ui/tui/workspace.rs`（1）

- [ ] `command_filter_is_case_insensitive_and_keeps_real_routes_only`

## B. 合并／精简目标（8 个）

以下八项的处理方向已确定，但必须先保护其原有的独有有效断言。`合并` 可以使原测试函数消失；`精简` 可以保留原测试函数，不能一律视为应删。

1. [ ] `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::pinned_repository_key_has_exact_fingerprint`：已由生产函数按同一指纹常量校验；宜改为真正独立的固定公钥/信任材料验证，或并入可信仓库验证测试。未证实完全重复前不直接删除。
2. [ ] `crates/agentic-gpt/src/files/file_ops.rs::search_rejects_invalid_patterns_and_enforces_bounds`：实际只检验非法 Regex、Glob，未在此验证 bounds；保留无效模式拒绝断言，并入搜索负例/边界相关测试。
3. [ ] `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs::mcp_batch_descriptor_freezes_bounds_and_side_effect_annotations`：保留公开 schema、实际资源上限及副作用 annotations 一致性验证；减少把当前数字硬编码成孤立快照。
4. [ ] `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::target_names_are_frozen`：当前目标名称无需单独冻结；非法路径组件和非法 hash 仍是有效失败模式，合并到真实消费者的输入校验测试。
5. [ ] `crates/agentic-gpt/src/browser/browser_kernel.rs::bootstrap_sends_escaped_path_and_frozen_browser_setup_code`：保留 import 路径转义、初始化关键顺序和结果语义；精简对整段生成 JavaScript 文本的锁定，优先原位改写。
6. [ ] `crates/agentic-gpt/src/config/config.rs::sparse_projection_always_keeps_selectors_and_omits_reconstructable_defaults`：优先用保存→加载的行为验证必需 selector 和默认值可重建，合并至现有 durable writer/sparse load 测试；减少只判字段是否出现的快照断言。
7. [ ] `crates/agentic-gpt/src/config/setup/review.rs::review_preserves_pending_actions_and_redacted_standalone_reference`：pending actions 与密钥脱敏有保留意义；可合并到 review_is_redacted_active_mode_only_and_reports_secret_write_intent 等相邻用例，但不得丢失 pending-action 负例。
8. [ ] `crates/agentic-gpt-protocol/src/process.rs::process_read_defaults_view_and_bounds_wait`：与同模块预算边界测试集中核对默认 view、wait 上限和无 cursor 等行为；合并前确认所有独有断言已迁移。

## C. 保留保护名单（20 个）

以下用例继续保留；如实施合并，应避免破坏它们及其必要断言。

- `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::authenticated_inrelease_metadata_selects_exact_target`
- `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::cleartext_signature_verifies_and_tampering_fails`
- `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::fixed_repository_url_rejects_untrusted_path_forms`
- `crates/agentic-gpt/src/config/config.rs::process_response_bytes_defaults_validates_bounds_and_rejects_invalid_values`
- `crates/agentic-gpt/src/mcp/mcp_tests.rs::server_config_validation_is_complete_and_typed`
- `crates/agentic-gpt-protocol/src/lib.rs::managed_mcp_batch_defaults_bounds_and_wire_type_are_frozen`
- `crates/agentic-gpt-protocol/src/process.rs::process_read_rejects_explicit_budgets_outside_shared_limits`
- `crates/agentic-browser-host/src/lib.rs::zero_malformed_and_non_utf8_payloads_are_errors`
- `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::overdue_restore_claim_is_trigger_once_at_due_boundary`
- `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::future_restore_schedules_until_snoozed_due_boundary`
- `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::degraded_item_stays_pending_until_due_and_can_be_terminal`
- `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::terminal_transition_clears_actions_and_rejects_duplicate_transition`
- `crates/agentic-gpt/src/config/config.rs::room_repository_and_maintenance_use_the_v2_json_shape`
- `crates/agentic-gpt/src/config/config.rs::sparse_projection_keeps_inactive_sections_and_redacts_config_secrets`
- `crates/agentic-gpt/src/config/config.rs::tunnel_secret_references_are_strict_and_safe_summary_is_redacted`
- `crates/agentic-gpt/src/config/setup/model.rs::mcp_server_draft_defaults_empty_and_saves_as_configured`
- `crates/agentic-gpt/src/config/setup/review.rs::review_is_redacted_active_mode_only_and_reports_secret_write_intent`
- `crates/agentic-gpt/src/config/setup/validation.rs::active_input_ignores_inactive_connection_secrets_and_restores_staged_drafts`
- `crates/agentic-gpt-protocol/src/lib.rs::maintenance_v2_shapes_close_slots_bound_wait_and_separate_sync`
- `crates/agentic-gpt-protocol/src/lib.rs::install_and_run_protocol_defaults_and_command_names_are_stable`

## D. 核验与交付

- 执行开始前记录实际 HEAD；每轮按文件定位并核对名称，避免因源码移动误删。
- 对 Rust 运行相应 crate 的测试发现和受影响测试；对 Kotlin 执行适用的 shared 测试任务。执行失败应写明命令、失败用例与原因，不可把未执行写成通过。
- 完成后核对本文件所有复选项与仓库剩余测试声明，记录删除/合并/精简/跳过的实际结果和测试数变化，提交代码变更与简明报告。
- 盘点来源：[`agentic-gpt-single-tier-cases.md`](agentic-gpt-single-tier-cases.md)、[`other-rust-single-tier-cases.md`](other-rust-single-tier-cases.md)、[`kotlin-test-cases.md`](kotlin-test-cases.md)、[`review-candidates.md`](review-candidates.md)、[`cleanup-review.md`](cleanup-review.md)。
