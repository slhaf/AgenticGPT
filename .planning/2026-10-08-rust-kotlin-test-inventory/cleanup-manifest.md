# 测试清理执行清单

> 状态：**清理已执行；Rust CI 验证通过；Kotlin 已完成目视核对，部分本地目标未运行**。依据用户确认的清理范围及当前源码逐项处理。此文件记录清单目标和执行结果；未勾选项表示尚未完成。

## 执行规则

1. 仅处理下列明确列出的 **306 个删除目标**与 **8 个合并/精简目标**。不得把本清单外的用例自动并入删除范围。
2. 306 个删除目标**不再开展价值复审**。按文件定位对应测试函数并删除其测试声明；不要删除生产功能、变更产品行为或顺便重构实现。若清单标识已不匹配当前源码，单独记录并跳过该项。
3. 8 个合并/精简目标按逐项说明处理：允许原位精简、把独有断言迁入具名现有测试后移除旧用例；不允许为凑删除数量丢失独有的长期有效行为检查。可以保持原测试不动并记录原因。
4. 以 `AGENTS.md` 当前五档定义与长期回归价值原则为约束，完成变更后执行受影响的 Rust/Kotlin 验证，并检查测试发现结果及失败原因。按模块/批次记录实际删除、合并、保留、跳过数量和对应测试结果。
5. 涉及必须修改生产实现、边界不明或与现有长期回归保障发生冲突的项目，只暂停有问题的条目并汇报；其余清单项目可继续。不要静默扩大修改范围。

## A. 删除目标（306 个）

来源计数：290 个纯 T1–T3 清理候选＋6 个仅含 T1–T3 的 MIXED 清理候选＋3 个复审新增清理项＋7 个用户新确认的低档混合清理项。以下均为**精确目标**，按源码文件归组。

### `console/shared/src/androidHostTest/kotlin/work/slhaf/agentic/console/SharedLogicAndroidHostTest.kt`（1）

- [x] `example`

### `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/SharedCommonTest.kt`（1）

- [x] `example`

### `console/shared/src/jvmTest/kotlin/work/slhaf/agentic/console/SharedLogicDesktopTest.kt`（1）

- [x] `example`

### `crates/agentic-apply-patch/src/lib.rs`（1）

- [x] `parses_patch`

### `crates/agentic-browser-host/src/lib.rs`（4）

- [x] `bridge_status_is_local_and_reports_current_clients`
- [x] `native_frame_round_trip_uses_uint32_prefix`
- [x] `oversized_frame_is_rejected_before_reading_payload`
- [x] `truncated_frames_match_baseline_eof_behavior`

### `crates/agentic-gpt-hub/src/agents/dispatch_tests.rs`（3）

- [x] `failed_request_send_removes_current_connection`
- [x] `pending_replay_sends_reliable_envelope`
- [x] `reporting_only_connection_is_not_a_command_target`

### `crates/agentic-gpt-hub/src/agents/lifecycle_tests.rs`（6）

- [x] `expired_connection_cleanup_removes_only_stale_current_entries`
- [x] `generation_expiry_rechecks_current_liveness`
- [x] `generation_stale_heartbeat_direct_handler`
- [x] `guarded_send_rejects_captured_target_after_replacement`
- [x] `stale_heartbeat_is_rejected_without_touching_current_connection`
- [x] `stale_process_update_is_rejected_without_writing_process_cache`

### `crates/agentic-gpt-hub/src/ingress/http/routes.rs`（4）

- [x] `event_mark_body_requires_an_agent_and_event_ids`
- [x] `offline_event_list_returns_an_error_without_cached_events`
- [x] `process_read_keeps_domain_errors_as_http_errors`
- [x] `unavailable_process_read_reports_only_stale_cache_metadata`

### `crates/agentic-gpt-hub/src/ingress/mcp/args.rs`（1）

- [x] `process_exec_arguments_accept_command_cwd_and_reject_legacy_fields`

### `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs`（11）

- [x] `bootstrap_timeout_values_preserve_operation_specific_codes`
- [x] `coordinator_profile_exposes_only_native_tools`
- [x] `full_profile_keeps_bootstrap_aliases_and_execution_surface`
- [x] `native_tool_values_use_agentic_result_shape`
- [x] `offline_native_process_cache_tools_omit_events`
- [x] `process_read_arg_schema_matches_contract_and_defaults`
- [x] `room_mcp_input_schemas_do_not_include_agent_id`
- [x] `skill_install_and_run_tools_are_exposed_with_stable_annotations`
- [x] `tmux_exec_schema_exposes_snapshot_fields`
- [x] `tmux_paste_schema_exposes_confirmation_default_field`
- [x] `tool_read_only_hints_match_side_effect_semantics`

### `crates/agentic-gpt-hub/src/notifications/notify.rs`（5）

- [x] `android_registered_endpoint_still_reports_delivery_not_implemented`
- [x] `notification_channels_include_agent_ntfy_and_android_placeholders`
- [x] `ntfy_default_placeholder_is_not_configured`
- [x] `ntfy_health_cache_controls_listing_reason`
- [x] `parses_notify_channel_keys`

### `crates/agentic-gpt-hub/src/room/room.rs`（11）

- [x] `bootstrap_error_codes_map_to_frozen_http_statuses`
- [x] `first_room_agent_becomes_active_room`
- [x] `normal_agent_is_not_room_api_fallback`
- [x] `read_and_maintenance_room_api_without_active_room_returns_not_active`
- [x] `room_api_after_replacement_without_hello_returns_not_active`
- [x] `room_api_without_active_room_returns_not_active`
- [x] `same_agent_normal_hello_releases_old_active_room`
- [x] `same_agent_replacement_without_hello_does_not_leave_stale_active_room`
- [x] `same_room_agent_reconnect_replaces_old_room_connection`
- [x] `second_different_room_agent_is_rejected`
- [x] `stale_room_disconnect_does_not_release_new_room_connection`

### `crates/agentic-gpt-hub/src/runtime/cli.rs`（1）

- [x] `cli_version_uses_crate_version`

### `crates/agentic-gpt-hub/src/runtime/config.rs`（1）

- [x] `safe_default_summary_has_no_paths_or_secrets`

### `crates/agentic-gpt-hub/src/runtime/main_tests.rs`（4）

- [x] `bootstrap_commands_have_run_types`
- [x] `ntfy_mcp_confirmation_stays_within_three_action_limit`
- [x] `parses_bearer_case_insensitively`
- [x] `skills_commands_have_run_types`

### `crates/agentic-gpt-hub/src/runtime/state.rs`（2）

- [x] `process_cache_age_generation_and_ordering_are_truthful`
- [x] `process_cache_capacity_is_bounded`

### `crates/agentic-gpt-hub/src/storage/event_feedback.rs`（2）

- [x] `public_flush_waits_for_active_agent_barrier`
- [x] `queued_recovery_subsets_preserve_all_known_identities`

### `crates/agentic-gpt-hub/src/support/agentic_result.rs`（2）

- [x] `wraps_native_json_as_structured_error_when_value_has_error`
- [x] `wraps_native_json_as_structured_success_when_value_has_no_error`

### `crates/agentic-gpt-protocol/src/envelopes.rs`（2）

- [x] `event_hub_commands_keep_request_identity_and_agent_scope`
- [x] `mcp_list_servers_panel_suppression_is_internal_and_legacy_compatible`

### `crates/agentic-gpt-protocol/src/lib.rs`（14）

- [x] `bootstrap_commands_and_enums_use_public_spellings`
- [x] `bootstrap_resource_omits_only_absent_truncation_line`
- [x] `bootstrap_response_and_read_request_round_trip_with_camel_case_fields`
- [x] `current_room_commands_use_nested_payloads_and_public_names`
- [x] `diary_v2_shapes_keep_periods_semantic_and_missing_layers_explicit`
- [x] `hello_defaults_to_command_capable_when_generation_is_present`
- [x] `hello_without_boot_generation_is_rejected`
- [x] `managed_mcp_call_defaults_and_bounds_are_frozen`
- [x] `notebook_and_state_v2_shapes_bound_previews_and_read_markdown_exactly`
- [x] `paste_and_close_default_to_confirmation`
- [x] `skill_read_path_is_additive_and_install_source_is_discriminated`
- [x] `skill_wait_seconds_are_bounded_without_overflow`
- [x] `skills_command_serde_names_are_public_interface_names`
- [x] `tmux_exec_defaults_to_structured_non_forced_confirmation_request`

### `crates/agentic-gpt-protocol/src/process.rs`（2）

- [x] `process_exec_wire_uses_command_and_cwd_and_rejects_legacy_argv_fields`
- [x] `unified_process_response_has_compact_identity_and_observation_fields`

### `crates/agentic-gpt/src/browser/browser_distribution_tests.rs`（9）

- [x] `authenticated_inrelease_metadata_rejects_duplicates_and_contradictions`
- [x] `cleartext_signature_hash_policy_is_sha2_only`
- [x] `injected_fetcher_rejects_status_and_metadata_bounds`
- [x] `package_selector_rejects_ambiguity_duplicate_fields_and_unsafe_names`
- [x] `package_selector_validates_required_identity_fields`
- [x] `packages_digest_and_size_are_verified_before_selection`
- [x] `release_date_parser_accepts_rfc2822_timestamp`
- [x] `selected_entry_and_size_bounds_are_enforced_without_large_fixtures`
- [x] `selected_paths_reject_traversal_and_empty_aliases`

### `crates/agentic-gpt/src/browser/browser_kernel.rs`（19）

- [x] `bootstrap_converts_tool_error_to_stable_failure`
- [x] `bootstrap_preserves_rmcp_service_failure_prefix`
- [x] `bootstrap_sentinel_maps_to_browser_unavailable`
- [x] `command_helper_uses_direct_program_cwd_and_env_overrides`
- [x] `constructor_rejects_empty_or_whitespace_session_ids`
- [x] `constructor_rejects_empty_or_whitespace_turn_ids`
- [x] `js_attaches_exact_codex_turn_metadata`
- [x] `js_keeps_tool_error_result_as_successful_transport_return`
- [x] `js_prefixes_rmcp_service_failures`
- [x] `js_preserves_mixed_result_fields_unchanged`
- [x] `js_sends_exact_tool_name_and_arguments`
- [x] `node_repl_client_info_uses_the_frozen_initialize_contract`
- [x] `reset_js_converts_tool_error_to_stable_failure`
- [x] `reset_js_prefixes_rmcp_service_failures`
- [x] `reset_js_sends_exact_tool_name_and_empty_arguments`
- [x] `spawn_rejects_invalid_ids_before_attempting_process_creation`
- [x] `turn_ended_converts_tool_error_to_stable_failure`
- [x] `turn_ended_prefixes_rmcp_service_failures`
- [x] `turn_ended_sends_exact_tool_name_and_arguments`

### `crates/agentic-gpt/src/browser/browser_manual.rs`（1）

- [x] `search_rejects_query_and_context_bounds`

### `crates/agentic-gpt/src/browser/browser_runtime.rs`（14）

- [x] `explicit_descriptor_derives_shared_docs_and_trusted_paths`
- [x] `explicit_descriptor_rejects_empty_and_relative_fields`
- [x] `launch_spec_creates_browser_only_trusted_services_when_absent`
- [x] `launch_spec_creates_trusted_code_paths_when_base_omits_them`
- [x] `launch_spec_derives_program_and_cwd_from_descriptor`
- [x] `launch_spec_does_not_create_empty_node_module_dirs`
- [x] `launch_spec_empty_node_module_dirs_preserves_base_value`
- [x] `launch_spec_merges_and_deduplicates_trusted_code_paths`
- [x] `launch_spec_overwrites_stale_runtime_coupled_values`
- [x] `launch_spec_preserves_security_mode_without_inventing_it`
- [x] `launch_spec_preserves_unrelated_base_environment`
- [x] `launch_spec_rejects_invalid_trusted_services`
- [x] `launch_spec_replaces_browser_trusted_service_and_preserves_others`
- [x] `explicit_descriptor_allows_missing_codex_cli_path`

### `crates/agentic-gpt/src/config/config_cli.rs`（7）

- [x] `interactive_init_requires_all_three_terminals_and_no_non_interactive_flag`
- [x] `registry_applies_new_scalar_and_list_keys`
- [x] `registry_keys_are_unique_and_have_bilingual_metadata`
- [x] `setup_seed_conversion_preserves_editable_flags_and_redacts_agent_secret`
- [x] `toolset_listing_describes_all_namespaces_and_feedback_is_localized`
- [x] `registry_updates_room_repository_and_maintenance_settings`
- [x] `toolset_commands_dispatch_and_reject_unknown_namespaces`

### `crates/agentic-gpt/src/config/config_templates.rs`（7）

- [x] `explicit_confirmation_language_wins_and_normal_room_override_requires_room_toolset`
- [x] `local_mode_ignores_tunnel_inputs`
- [x] `mcp_servers_flow_into_built_config`
- [x] `normal_profile_accepts_room_override_with_explicit_room_toolset`
- [x] `partial_hub_template_rejects_invalid_url_and_transport_with_stable_errors`
- [x] `pending_actions_are_deterministic_and_unique`
- [x] `secret_debug_output_is_redacted`

### `crates/agentic-gpt/src/config/config.rs`（16）

- [x] `auto_max_active_processes_uses_the_frozen_formula`
- [x] `confirmation_provider_disk_shape_rejects_legacy_provider`
- [x] `confirmation_provider_rejects_duplicate_or_unknown_channels`
- [x] `empty_browser_section_is_omitted_from_sparse_defaults`
- [x] `file_search_context_limit_defaults_and_rejects_invalid_values`
- [x] `http_bearer_file_references_require_absolute_paths_without_tightening_tunnel_refs`
- [x] `hub_validation_rejects_invalid_url_and_transport_with_stable_errors`
- [x] `legacy_confirmation_labels_map_to_canonical_fallback_order`
- [x] `limits_reject_retired_max_active_jobs_field`
- [x] `managed_browser_policy_defaults_are_enabled_without_auto_provisioning` — 复审新增：锁定当前 managed browser 默认启用、默认不自动安装的产品策略；未来变更并不天然属于回归。原候选清单的文件路径误写为 browser/browser_runtime.rs。
- [x] `managed_browser_policy_rejects_unknown_fields`
- [x] `max_active_processes_supports_auto_and_explicit_round_trips`
- [x] `mcp_server_semantics_are_validated_before_standalone_use`
- [x] `shell_init_file_serde_preserves_default_disabled_and_explicit_paths`
- [x] `new_default_config_serializes_auto_limit`
- [x] `toolset_profiles_are_closed_and_deterministic`

### `crates/agentic-gpt/src/config/setup/model.rs`（5）

- [x] `malformed_tunnel_secret_reference_is_reported_as_a_field_error`
- [x] `optional_status_and_drafts_survive_mode_and_profile_changes`
- [x] `room_availability_uses_profile_preset_without_explicit_toolset_selection`
- [x] `setup_defaults_to_standalone_normal_and_preserves_inactive_mode_seeds`
- [x] `tunnel_secret_reference_seeds_are_parsed_without_exposing_secret_text`

### `crates/agentic-gpt/src/config/setup/review.rs`（4）

- [x] `review_reports_default_and_configured_optional_statuses`
- [x] `review_rows_expose_stable_edit_contract_without_secret_material`
- [x] `shell_review_shows_path_only_for_explicit_path_mode`
- [x] `event_review_preserves_values_and_offers_every_override_choice`

### `crates/agentic-gpt/src/config/setup/validation.rs`（5）

- [x] `hub_connection_transport_and_secret_are_structured`
- [x] `normal_profile_with_explicit_room_disabled_toolset_hides_and_rejects_room`
- [x] `optional_validation_covers_paths_numbers_runtime_paths_and_reporting`
- [x] `process_response_bytes_limit_accepts_protocol_bounds_and_rejects_invalid_values`
- [x] `required_connection_fields_report_concrete_domain_fields`

### `crates/agentic-gpt/src/files/file_ops.rs`（5）

- [x] `apply_patch_parser_handles_add_delete_update_and_move_without_fs_access`
- [x] `bounded_diff_counts_create_delete_crlf_and_final_newline`
- [x] `bounded_diff_preserves_blank_lines_and_emits_disjoint_hunks`
- [x] `bounded_diff_truncates_utf8_after_computing_complete_counts`
- [x] `rejects_duplicate_and_ancestor_patch_paths`

### `crates/agentic-gpt/src/ingress/http_oauth.rs`（9）

- [x] `chatgpt_redirect_families_are_exact`
- [x] `expired_oauth_tokens_are_pruned_and_rejected`
- [x] `host_and_origin_matching_follow_rmcp_policy`
- [x] `oauth_error_and_html_responses_are_uncacheable`
- [x] `oauth_tokens_reject_wrong_resource_binding`
- [x] `pkce_rfc7636_s256_vector_matches`
- [x] `public_url_normalization_uses_config_contract`
- [x] `standalone_page_escapes_hidden_fields_and_avoids_hub_copy`
- [x] `token_rotation_revokes_oauth_state_but_direct_bearer_changes`

### `crates/agentic-gpt/src/ingress/hub.rs`（1）

- [x] `oversized_report_json_becomes_a_hash_record`

### `crates/agentic-gpt/src/ingress/local_control.rs`（1）

- [x] `socket_path_rejects_invalid_identity_and_oversized_path`

### `crates/agentic-gpt/src/ingress/stdio_server_tests.rs`（19）

- [x] `active_response_precedes_terminal_for_both_serial_orderings`
- [x] `batch_lifecycle_detection_reads_process_envelopes`
- [x] `browser_conditional_arguments_and_bounds_are_validated`
- [x] `browser_descriptors_and_annotations_are_frozen`
- [x] `browser_reporting_and_error_helpers_do_not_retain_source`
- [x] `concurrent_check_clear_enqueue_interleaving_is_linearizable`
- [x] `event_api_schemas_preserve_defaults_and_empty_mark_boundary`
- [x] `every_room_adapter_rejects_unknown_identity_fields`
- [x] `file_lock_registry_prunes_released_paths`
- [x] `file_surface_schema_is_exact`
- [x] `inline_terminal_tracker_discards_pending_terminal_event`
- [x] `managed_terminal_event_includes_duration`
- [x] `mcp_batch_projection_preserves_compact_observation_and_child_identity`
- [x] `mutating_tool_annotations_do_not_promise_read_only_or_additive_effects`
- [x] `process_read_input_schema_advertises_wait_view_and_response_budget`
- [x] `room_maintenance_descriptors_are_frozen`
- [x] `room_maintenance_submit_rejects_unknown_nested_fields`
- [x] `tmux_actions_reject_incompatible_fields`
- [x] `tunnel_local_and_http_ingress_advertise_identical_surface` — 复审新增：把 Tunnel、Local 与 HTTP 三入口强制绑定为完全相同的工具表；未来合理分化会造成误报。执行前确认入口实际授权与契约测试仍覆盖可见性和来源标识。

### `crates/agentic-gpt/src/mcp/mcp_tests.rs`（2）

- [x] `server_config_revision_is_deterministic_and_content_sensitive`
- [x] `streamable_http_bearer_auth_is_validated_and_injected`

### `crates/agentic-gpt/src/operations/confirmation.rs`（1）

- [x] `batch_confirmation_shows_the_original_script_and_working_directory`

### `crates/agentic-gpt/src/operations/event_notifications.rs`（4）

- [x] `active_process_response_remains_eligible_for_async_completion`
- [x] `deduplicated_install_response_does_not_settle_original_source`
- [x] `mixed_batch_classifies_each_process_from_its_own_terminal_state`
- [x] `terminal_process_observation_suppresses_completion_with_incomplete_capture`

### `crates/agentic-gpt/src/operations/operation.rs`（4）

- [x] `cli_admission_is_limited_to_local_admin_tmux_operations`
- [x] `normal_hub_cannot_acquire_room_capability_by_operation_name`
- [x] `process_read_keeps_process_authorization_across_agent_profiles_and_hub`
- [x] `same_prefix_unknown_operations_fail_closed`

### `crates/agentic-gpt/src/operations/policy.rs`（8）

- [x] `dynamic_words_redirects_and_complex_scripts_never_use_partial_allows`
- [x] `escaped_crlf_cannot_hide_a_default_denied_command`
- [x] `escaped_word_separator_requires_confirmation_instead_of_partial_matching`
- [x] `known_deny_wins_inside_unsupported_or_incomplete_scripts`
- [x] `shell_confirmation_cannot_be_removed_by_an_allow_rule`
- [x] `shell_requires_allow_for_every_literal_command`
- [x] `shell_supports_literal_quotes_concatenation_and_supported_operators`
- [x] `terminal_dollar_in_a_quoted_argument_is_not_truncated`

### `crates/agentic-gpt/src/operations/shell_parser.rs`（5）

- [x] `dynamic_and_expanding_words_are_not_literal`
- [x] `escaped_crlf_does_not_hide_the_following_command`
- [x] `escaped_word_separator_does_not_preserve_truncated_arguments`
- [x] `extracts_quoted_words_and_literal_concatenation`
- [x] `quoted_terminal_dollars_and_newlines_are_kept`

### `crates/agentic-gpt/src/process/exec.rs`（1）

- [x] `shell_argument_limit_counts_exact_bootstrap_and_keeps_boundary`

### `crates/agentic-gpt/src/process/managed.rs`（5）

- [x] `failed_process_reasons_are_projected_without_fabricating_exit_errors`
- [x] `output_overflow_reports_exact_retained_gap`
- [x] `process_error_messages_are_utf8_safe_and_bounded`
- [x] `process_read_rejects_output_pages_that_cannot_advance`
- [x] `skill_leases_still_block_updates`

### `crates/agentic-gpt/src/room/bootstrap.rs`（2）

- [x] `guide_directory_entry_errors_are_reported_without_dropping_readable_entries`
- [x] `resource_truncation_is_line_aware_and_utf8_safe`

### `crates/agentic-gpt/src/room/room_reads.rs`（2）

- [x] `diary_periods_use_strict_direct_layer_paths`
- [x] `semantic_paths_reject_arbitrary_repository_files`

### `crates/agentic-gpt/src/room/room_repository.rs`（1）

- [x] `logical_day_respects_the_configured_shanghai_boundary`

### `crates/agentic-gpt/src/runtime/main_tests.rs`（23）

- [x] `batch_confirmation_preview_supports_chinese`
- [x] `cli_version_uses_crate_version`
- [x] `configured_allow_overrides_builtin_confirm`
- [x] `configured_allow_overrides_builtin_deny`
- [x] `configured_allow_overrides_need_confirm`
- [x] `configured_deny_wins_when_multiple_config_rules_match`
- [x] `configured_room_repository_root_overrides_default`
- [x] `explicit_runtime_precedes_desktop_discovery_and_invalid_does_not_fallback`
- [x] `local_cli_accepts_config_before_or_after_subcommand`
- [x] `notification_delivery_rejects_unsupported_channel`
- [x] `public_run_has_only_a_config_path_and_no_profile_override`
- [x] `remove_rule_matches_command_and_args_prefix`
- [x] `remove_rule_matches_command_without_uuid`
- [x] `remove_rule_refuses_ambiguous_non_interactive_match`
- [x] `room_policy_keeps_high_risk_commands_restricted`
- [x] `room_policy_overlay_differs_from_normal_policy`
- [x] `rule_matches_program_and_args_prefix_structurally`
- [x] `run_as_room_uses_workspace_default_repository_root`
- [x] `safe_summary_includes_path_roots_and_policy_rules`
- [x] `sse_post_status_classification_stops_on_stale_connection`
- [x] `standalone_reload_replaces_the_frozen_live_subset`
- [x] `sudo_requires_credentials`
- [x] `room_timezone_defaults_and_can_be_overridden`

### `crates/agentic-gpt/src/runtime/state.rs`（1）

- [x] `runtime_capabilities_follow_transport_and_profile`

### `crates/agentic-gpt/src/runtime/supervisor.rs`（9）

- [x] `doctor_diagnostic_output_is_bounded_and_redacted`
- [x] `forwarded_child_lines_preserve_known_severity_and_strip_timestamp`
- [x] `forwarded_journal_lines_preserve_untimestamped_severity_after_redaction`
- [x] `mcp_binding_preserves_worker_tokenization`
- [x] `restart_decision_covers_retry_permanent_and_exhausted`
- [x] `restart_identity_warning_compares_to_immutable_runtime_and_warns_once`
- [x] `retry_schedule_is_bounded_and_exponential`
- [x] `runtime_paths_reject_path_injection`
- [x] `worker_command_quotes_paths_and_never_contains_api_key`

### `crates/agentic-gpt/src/runtime/tunnel_distribution.rs`（5）

- [x] `archive_hash_mismatch_is_checked_before_install`
- [x] `archive_rejects_traversal_symlinks_duplicates_and_extra_files`
- [x] `artifact_selection_requires_pinned_or_explicit_trust`
- [x] `download_url_requires_https_outside_tests`
- [x] `manifest_and_platforms_are_pinned` — 复审新增：锁死阶段性的分发版本、URL、SHA-256；其他测试覆盖制品选择和哈希拒绝。但若团队明确要求第二处独立固定值审计，则应重新评估。

### `crates/agentic-gpt/src/skills/skill_installs.rs`（7）

- [x] `github_sources_resolve_structured_and_convenience_forms`
- [x] `idempotency_conflict_is_rejected_before_creating_a_second_job`
- [x] `idempotency_retries_return_the_original_install`
- [x] `package_paths_reject_case_conflicts_and_file_directory_collisions`
- [x] `queued_cancel_is_idempotent_and_status_is_terminal`
- [x] `remote_policy_rejects_private_and_reserved_addresses`
- [x] `transient_download_errors_are_the_only_retryable_materialization_failures`

### `crates/agentic-gpt/src/storage/event_store.rs`（1）

- [x] `low_ttl_out_of_range_returns_an_error_without_panicking`

### `crates/agentic-gpt/src/storage/private_state.rs`（1）

- [x] `legacy_wide_agent_id_uses_stable_safe_state_key`

### `crates/agentic-gpt/src/support/utils.rs`（4）

- [x] `compact_id_has_a_stable_twelve_hex_digit_body`
- [x] `journal_rendering_omits_inner_timestamp_only_in_journal_mode`
- [x] `mcp_argument_key_summary_is_counted_and_bounded`
- [x] `mcp_confirmation_preview_is_sorted_bounded_metadata_without_values`

### `crates/agentic-gpt/src/tmux/tmux.rs`（4）

- [x] `cd_requires_one_explicit_path`
- [x] `parses_session_and_pane_metadata`
- [x] `session_scoped_pane_listing_does_not_request_all_panes`
- [x] `shell_detection_and_argument_quoting_are_structural`

### `crates/agentic-gpt/src/ui/cli_i18n.rs`（5）

- [x] `every_catalog_entry_is_non_empty_for_each_language`
- [x] `explicit_language_overrides_locale_environment`
- [x] `locale_precedence_is_lc_all_then_lc_messages_then_lang`
- [x] `localized_command_tree_has_complete_visible_metadata`
- [x] `prescan_accepts_equals_and_split_forms_anywhere`

### `crates/agentic-gpt/src/ui/config_tui/pages.rs`（1）

- [x] `shell_path_field_is_focusable_only_for_explicit_path_mode`

### `crates/agentic-gpt/src/ui/tui/forms/state.rs`（2）

- [x] `empty_list_can_add_first_item_and_delete_back_to_empty`
- [x] `list_state_adds_edits_deletes_and_keeps_focus_valid`

### `crates/agentic-gpt/src/ui/tui/layout.rs`（2）

- [x] `master_detail_collapses_to_active_pane`
- [x] `surface_cursor_honors_reserved_bottom_rows`

### `crates/agentic-gpt/src/ui/tui/process.rs`（1）

- [x] `clipping_and_short_id_are_bounded`

### `crates/agentic-gpt/src/ui/tui/runtime.rs`（1）

- [x] `restoration_seam_records_cleanup_in_reverse_setup_order`

### `crates/agentic-gpt/src/ui/tui/shell.rs`（2）

- [x] `overlay_stays_inside_small_terminal`
- [x] `shell_uses_config_tui_margin_and_fixed_chrome_rows`

### `crates/agentic-gpt/src/ui/tui/workspace.rs`（1）

- [x] `command_filter_is_case_insensitive_and_keeps_real_routes_only`

## B. 合并／精简目标（8 个）

以下八项均已按逐项结果处理。`合并` 可使原测试函数消失；`精简` 保留原测试函数；原样保留的项目须记录理由。

1. [x] `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::pinned_repository_key_has_exact_fingerprint`：原样保留。可用签名验证 fixture 不使用 pinned repository key，删除会失去唯一将固定公钥材料与预期 fingerprint 绑定的断言。
2. [x] `crates/agentic-gpt/src/files/file_ops.rs::search_rejects_invalid_patterns_and_enforces_bounds`：实际只检验非法 Regex、Glob，未在此验证 bounds；保留无效模式拒绝断言，并入搜索负例/边界相关测试。
3. [x] `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs::mcp_batch_descriptor_freezes_bounds_and_side_effect_annotations`：保留公开 schema、实际资源上限及副作用 annotations 一致性验证；减少把当前数字硬编码成孤立快照。
4. [x] `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::target_names_are_frozen`：当前目标名称无需单独冻结；非法路径组件和非法 hash 仍是有效失败模式，合并到真实消费者的输入校验测试。
5. [x] `crates/agentic-gpt/src/browser/browser_kernel.rs::bootstrap_sends_escaped_path_and_frozen_browser_setup_code`：保留 import 路径转义、初始化关键顺序和结果语义；精简对整段生成 JavaScript 文本的锁定，优先原位改写。
6. [x] `crates/agentic-gpt/src/config/config.rs::sparse_projection_always_keeps_selectors_and_omits_reconstructable_defaults`：优先用保存→加载的行为验证必需 selector 和默认值可重建，合并至现有 durable writer/sparse load 测试；减少只判字段是否出现的快照断言。
7. [x] `crates/agentic-gpt/src/config/setup/review.rs::review_preserves_pending_actions_and_redacted_standalone_reference`：pending actions 与密钥脱敏有保留意义；可合并到 review_is_redacted_active_mode_only_and_reports_secret_write_intent 等相邻用例，但不得丢失 pending-action 负例。
8. [x] `crates/agentic-gpt-protocol/src/process.rs::process_read_defaults_view_and_bounds_wait`：与同模块预算边界测试集中核对默认 view、wait 上限和无 cursor 等行为；合并前确认所有独有断言已迁移。


### 执行结果

306 个删除目标已按源码逐项匹配并删除；无名称不匹配或跳过。8 个合并／精简目标均已处理：5 项合并、2 项原位精简、1 项原样保留并记录理由。

| 清单目标 | 结果 | 保留的长期有效行为 |
| --- | --- | --- |
| `browser_distribution_tests.rs::pinned_repository_key_has_exact_fingerprint` | 原样保留 | 没有真实 pinned-repository 签名验证 fixture；保留固定公钥 fingerprint 断言，避免失去唯一信任材料绑定检查。 |
| `file_ops.rs::search_rejects_invalid_patterns_and_enforces_bounds` | 合并至 `search_enforces_stream_bounds_and_rejects_invalid_patterns` | 保留 Regex/Glob 无效模式拒绝；独立 scan/output 上限断言仍验证资源边界。 |
| `mcp_server.rs::mcp_batch_descriptor_freezes_bounds_and_side_effect_annotations` | 原位精简并改名为 `mcp_batch_descriptor_preserves_schema_limits_and_side_effect_annotations` | 保留公开 schema、`calls` 1–16 项资源上限及副作用 annotations；删除孤立 wait/timeout 数值快照。 |
| `browser_distribution_tests.rs::target_names_are_frozen` | 合并至 `package_consumers_reject_invalid_path_components_and_hashes` | 在实际消费者路径保留非法制品路径组件与非法 hash 拒绝。 |
| `browser_kernel.rs::bootstrap_sends_escaped_path_and_frozen_browser_setup_code` | 原位精简并改名为 `bootstrap_escapes_import_path_and_initializes_browser_in_order` | 保留导入路径转义、初始化顺序、浏览器不可用 sentinel 与结果语义；去掉整段生成 JavaScript 文本快照。 |
| `config.rs::sparse_projection_always_keeps_selectors_and_omits_reconstructable_defaults` | 合并至 `durable_writer_uses_sparse_projection_and_preserves_unknown_fields` | 通过写入→加载保留 mode/profile/toolsets 和可重建默认值；继续验证未知字段保留。 |
| `review.rs::review_preserves_pending_actions_and_redacted_standalone_reference` | 合并至 `review_is_redacted_active_mode_only_and_reports_secret_write_intent` | 保留待处理动作、立即写入负例、secret reference 与活动/非活动密钥脱敏。 |
| `process.rs::process_read_defaults_view_and_bounds_wait` | 合并至 `process_read_rejects_explicit_budgets_outside_shared_limits` | 保留默认 view/wait/no-cursor/max-bytes 以及 wait 上限和溢出输入行为。 |
- Rust 测试发现：`cargo test --workspace -- --list` 报告 workspace 共 520 个测试；`cargo test --workspace` 通过 519 个，另有 1 个既有手动测试被忽略。独立源码扫描确认 306 个 A 项均已消失，其中 Rust 303 项、Kotlin 3 项，与原始清单相符。
- 固定工具链下的 Rust CI 均通过：`cargo fmt --all -- --check`、`cargo check --workspace`、严格 Clippy、`cargo test --workspace`、Agent/Hub 二进制构建及 `scripts/check_contract_parity.py`。最新 stable 工具链严格 Clippy 也通过。
- Kotlin：三份被清理的测试源文件在移除唯一测试后只剩空类，已一并删除。`:shared:jvmTest` 单独运行通过（2 秒，Gradle 报告 3 项执行、14 项 up-to-date）。合并的 JVM/JS/Wasm 调用中 JS 与 Wasm 测试源码编译通过，但执行阶段因并发 Yarn 初始化后缺少 `console/build/js/yarn.lock` 而失败；按用户要求不再逐个重跑 JS/Wasm。Android host 未运行：SDK 需要接受 Build-Tools 36 与 Android 36 licenses；未代用户接受。Kotlin 其余目标仅作目视源码核对，未运行不记作通过。
## C. 核验与交付

- 执行开始前记录实际 HEAD；每轮按文件定位并核对名称，避免因源码移动误删。
- 对 Rust 运行相应 crate 的测试发现和受影响测试；对 Kotlin 执行适用的 shared 测试任务。执行失败应写明命令、失败用例与原因，不可把未执行写成通过。
- 完成后核对本文件所有复选项与仓库剩余测试声明，记录删除/合并/精简/跳过的实际结果和测试数变化，提交代码变更与简明报告。
- 盘点来源：[`agentic-gpt-single-tier-cases.md`](agentic-gpt-single-tier-cases.md)、[`other-rust-single-tier-cases.md`](other-rust-single-tier-cases.md)、[`kotlin-test-cases.md`](kotlin-test-cases.md)、[`review-candidates.md`](review-candidates.md)、[`cleanup-review.md`](cleanup-review.md)。
