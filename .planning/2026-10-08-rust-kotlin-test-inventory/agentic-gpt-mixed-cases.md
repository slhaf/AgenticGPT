# Agentic GPT mixed-tier test cases

**Final MIXED declarations: 323.** Each unique test declaration is listed once, grouped by repository-relative source file. Tier components describe independently asserted behavior; MIXED is a separate bucket, not an extra single-tier count. Evidence uses source line ranges where the slice reports supplied them. Ignored and platform-gated declarations are still counted once as declared cases.

## Tier boundary and reconciliation

T5 is assigned only when a test assertion depends on production behavior against a material resource (filesystem, Git, socket, process, or persisted database state/effects). Fixture setup/cleanup and empty-store scans alone do not promote a case. In-memory SQLite counts as T5 when assertions depend on actual SQL state or effects.

The corrected empty-store classifications are: `event_api_business_errors_keep_their_code_and_include_the_panel` = T4; `browser_repl_event_decoration_preserves_inner_result_channels` = MIXED(T2+T4), with no T5; `in_process_stdio_initialize_list_and_call` = T4; `denied_process_batch_creates_no_processes` = MIXED(T4+T5), because the post-denial SQLite list asserts the no-persisted-process invariant; `in_process_room_stdio_initialize_list_and_call` = T4, because only the error flag and constructed daily-object shape are asserted and an empty/missing root still yields that object.

| Slice | Declarations | T1 | T2 | T3 | T4 | T5 | MIXED |
|---|---:|---:|---:|---:|---:|---:|---:|
| Browser/config | 202 | 3 | 28 | 50 | 24 | 3 | 94 |
| Ingress/operations | 105 | 7 | 10 | 34 | 8 | 3 | 43 |
| MCP/room | 68 | 0 | 3 | 5 | 12 | 12 | 36 |
| Storage/skills | 69 | 0 | 2 | 7 | 0 | 2 | 58 |
| UI/support/tmux/integration | 62 | 1 | 14 | 8 | 4 | 24 | 11 |
| Runtime/files/process | 148 | 0 | 16 | 30 | 11 | 10 | 81 |
| **Total** | **654** | **11** | **73** | **134** | **59** | **54** | **323** |

The six slices reconcile to 654 declarations; single-tier buckets sum to 331 and MIXED contributes 323, for 654 overall. MIXED rows below sum to 94 + 43 + 36 + 58 + 11 + 81 = 323.

## MIXED case inventory

### `crates/agentic-gpt/src/browser/browser_distribution_tests.rs`
- `acquisition_temp_cleanup_handles_materializer_success_and_failure` — MIXED (T4+T5); checks acquisition temp cleanup after production materialization succeeds or fails package-hash validation; T4: temp guard/materializer chain; T5: package/cache file effects [evidence L439]
- `active_manifest_corruption_fails_closed` — MIXED (T4+T5); installs artifact, corrupts active manifest on disk in several ways, and checks production discovery rejects it; T4: install/discovery chain; T5: manifest file reads [evidence L1089]
- `ar_parser_rejects_malformed_truncated_duplicate_and_unsupported_members` — MIXED (T3+T5); writes each archive fixture to a temp package path and production `ar_data` reads it; accepts odd member and rejects unsupported compression, duplicate control member, truncation; T3: ar-format rules; T5: production reads real temp package file [evidence L915]
- `duplicate_data_member_is_rejected` — MIXED (T3+T5); production materializer reads package file with duplicate data members and rejects it; T3: archive validation; T5: file-backed package processing [evidence L895]
- `hash_mismatch_is_rejected_before_archive_processing` — MIXED (T3+T5); production materializer reads the package file, rejects wrong digest, and leaves no staging tree; T3: hash gate; T5: file-backed materialization assertion [evidence L880]
- `identical_install_is_idempotent_and_corrupt_artifact_repairs` — MIXED (T4+T5); production install preserves manifest bytes on repeat, then repairs a removed cached file; T4: install/repair chain; T5: cache reads/writes [evidence L1063]
- `local_http_status_redirect_and_body_bounds_are_rejected` — MIXED (T3+T5); checks status/redirect and body limits through loopback HTTP; T3: response rules; T5: real socket exchange [evidence L309]
- `provisioning_lock_rechecks_cache_after_waiting` — MIXED (T4+T5); waits on target lock while another call materializes cache artifact, then checks waiter reuses it; T4: provision/lock/cache chain; T5: production lock/cache file effects [evidence L403]
- `real_package_materializes_from_explicit_fixture_path` — MIXED (T4+T5); reads supplied official package, materializes it, and checks version/channel and installed paths. `#[ignore]` at L538; requires `AGENTIC_BROWSER_REAL_DEB*`; T4: package verification/extraction/runtime descriptor chain; T5: actual package/cache files [evidence L539]
- `selected_links_special_types_and_duplicate_files_are_rejected` — MIXED (T3+T5); production materializer reads archives and rejects selected symlink/hard-link/FIFO entries and duplicate selected files; T3: archive-entry rules; T5: package/cache file effects [evidence L981]
- `streamed_package_checks_size_hash_and_temp_cleanup` — MIXED (T3+T5); streams loopback HTTP body to temp files; checks bytes, hash/size failures, and cleanup; T3: size/hash rules; T5: socket and production file writes [evidence L339]
- `symlinked_critical_parent_in_cached_artifact_fails_closed` — MIXED (T4+T5); replaces critical cached dir with symlink and checks production discovery fails closed. `#[cfg(unix)]` at L1150; T4: artifact/discovery chain; T5: symlink/filesystem effect [evidence L1152]
- `synthetic_deb_materializes_only_selected_resources` — MIXED (T4+T5); materializes synthetic archive, checks selected files/permissions/paths, and rediscovers cache runtime; T4: archive-to-runtime/discovery chain; T5: production package/cache filesystem [evidence L813]
- `target_names_are_frozen` — MIXED (T1+T3); freezes supported target-name literals and checks component/hash validation; T1: frozen names; T3: validation [evidence L801]

### `crates/agentic-gpt/src/browser/browser_kernel.rs`
- `bootstrap_sends_escaped_path_and_frozen_browser_setup_code` — MIXED (T1+T2); checks escaped path, timeout, frozen setup code/order, and browser-ID output sent as JS; T1: frozen content; T2: escaping/request forwarding [evidence L702]
- `sequential_calls_reuse_initialized_service_and_metadata` — MIXED (T2+T4); sends two JS calls; checks one initialize, protocol, and consistent metadata; T2: metadata forwarding; T4: kernel/client/service reuse [evidence L602]

### `crates/agentic-gpt/src/browser/browser_manager.rs`
- `list_is_sorted_and_contains_no_kernel_identity` — MIXED (T2+T4); checks sorted snapshots, timeout presence, and absence of session/turn IDs; T2: snapshot projection; T4: lease state [evidence L1229]

### `crates/agentic-gpt/src/browser/browser_manual.rs`
- `read_ranges_and_rejects_bad_paths` — MIXED (T3+T5); reads requested lines from fixture and rejects empty/dot/traversal/absolute paths; output omits root path; T3: bounds/path rules; T5: production file read [evidence L341]
- `read_rejects_root_component_and_final_symlinks` — MIXED (T3+T5); production path checks reject symlinked root/intermediate/final file. `#[cfg(unix)]` at L436; T3: symlink rejection rules; T5: real filesystem metadata [evidence L438]
- `read_reports_continuation_and_rejects_giant_line` — MIXED (T3+T5); reads actual fixture files and checks output continuation/cap and overlong-line rejection; T3: output rules; T5: production file reads [evidence L402]
- `search_is_literal_nested_and_sorted` — MIXED (T3+T5); production search reads nested fixture files, sorts literal matches, and returns context; T3: search rules; T5: production directory/file reads [evidence L374]
- `search_skips_symlink_and_non_utf8` — MIXED (T3+T5); production walk/read reports skipped symlink and non-UTF-8 files. `#[cfg(unix)]` at L521; T3: skip rules; T5: real filesystem traversal/read [evidence L523]

### `crates/agentic-gpt/src/browser/browser_runtime.rs`
- `absent_codex_cli_path_is_supported` — MIXED (T2+T3+T5); production reads registry, maps absent CLI to `None`, and omits launch variable; T2: optional mapping; T3: absent-field handling; T5: registry read [evidence L718]
- `absent_node_module_dirs_defaults_to_empty` — MIXED (T2+T5); production reads registry and maps missing dirs to empty vector; T2: default mapping; T5: registry file read [evidence L707]
- `client_path_without_bundle_parent_is_rejected` — MIXED (T3+T5); reads registry fixture and rejects client path with no valid bundle/docs root; T3: path rule; T5: registry read [evidence L784]
- `derives_docs_root_from_browser_client_path` — MIXED (T2+T5); production reads registry and maps client path to docs root; T2: path mapping; T5: registry file read [evidence L693]
- `derives_ordered_deduplicated_trusted_code_paths` — MIXED (T3+T5); production reads registry fixture and derives stable deduplicated trusted paths; T3: dedupe/order logic; T5: registry file read [evidence L673]
- `empty_entries_are_rejected` — MIXED (T3+T5); production reads registry fixture and rejects empty entries; T3: empty-registry rule; T5: registry file read [evidence L733]
- `explicit_descriptor_allows_missing_codex_cli_path` — MIXED (T2+T3); accepts absent CLI path and omits corresponding launch variable; T2: optional mapping; T3: absence branch [evidence L828]
- `malformed_json_is_rejected_with_browser_runtime_context` — MIXED (T3+T5); production opens malformed temp registry and returns contextual JSON error; T3: parse/error rule; T5: registry read [evidence L775]
- `malformed_latest_entry_does_not_fall_back_to_older_valid_entry` — MIXED (T3+T5); reads registry fixture and rejects malformed newest entry rather than fallback; T3: selection/error rule; T5: registry file read [evidence L759]
- `missing_or_empty_required_fields_are_rejected` — MIXED (T3+T5); production reads registry fixture and rejects missing app version/empty Node path with field errors; T3: required-field rules; T5: registry file read [evidence L739]
- `selects_greatest_updated_at_and_maps_selected_entry` — MIXED (T3+T2+T5); production reads registry file, selects newest entry, maps fields, and is input-order independent; T3: selection; T2: mapping; T5: registry file read [evidence L609]

### `crates/agentic-gpt/src/config/config.rs`
- `checked_in_v09_config_example_is_strict_and_safe_to_copy` — MIXED (T1+T4+T5); loads checked-in example via temp config, validates MCP/Standalone settings, checks placeholders/defaults and no secret material; T1: example content; T4: load/validation; T5: production config-file read [evidence L2474]
- `config_load_rejects_missing_toolsets` — MIXED (T3+T5); production loads temp config without toolsets and rejects it; T3: strict-field rule; T5: config-file read [evidence L2697]
- `durable_writer_uses_sparse_projection_and_preserves_unknown_fields` — MIXED (T2+T4+T5); production writer writes sparse config; checks future field retained and defaults omitted; T2: sparse projection; T4: writer/projection chain; T5: production file write/read [evidence L3381]
- `explicit_browser_runtime_round_trips_through_sparse_write_and_import` — MIXED (T2+T4+T5); writes explicit browser config, production loads/sparsely projects/imports and checks round trip; T2: browser JSON mapping; T4: write/load/import chain; T5: file operations [evidence L3399]
- `explicit_import_clears_invalid_http_public_url_and_keeps_other_fields` — MIXED (T2+T3+T5); production import warns/removes invalid public URL while retaining other HTTP MCP values; T2: import mapping; T3: URL rule; T5: file read [evidence L3350]
- `explicit_import_clears_plaintext_tunnel_secret_and_reports_it` — MIXED (T2+T3+T5); production import warns/clears plaintext secret, preserves other field, excludes marker from serialized config; T2: import/redaction mapping; T3: secret policy; T5: file read [evidence L3323]
- `explicit_import_maps_legacy_hub_and_preserves_recognized_and_unknown_fields` — MIXED (T2+T4+T5); production import reads legacy JSON, maps Hub/config fields and preserves future field; T2: legacy field mapping; T4: import/reconstruction chain; T5: file read [evidence L3131]
- `explicit_import_prefers_top_level_skills_over_legacy_room_skills` — MIXED (T3+T5); production imports temp config, keeps top-level skills, emits legacy-room warning; T3: precedence/warning rule; T5: import file read [evidence L2794]
- `explicit_import_preserves_shell_init_file_tristate` — MIXED (T2+T3+T5); production import maps absent/null/explicit shell values to Default/Disabled/Path; T2: JSON mapping; T3: tri-state; T5: import file read [evidence L3198]
- `explicit_import_preserves_toolsets_that_omit_a_room_profile_namespace` — MIXED (T3+T5); production import retains explicit toolset list rather than filling it from Room profile; T3: import/preservation rule; T5: file read [evidence L3266]
- `explicit_import_rejects_invalid_toolsets` — MIXED (T3+T5); production import reads file and rejects unknown toolset; T3: validation; T5: file read [evidence L3281]
- `explicit_import_reports_unimportable_recognized_fields_and_keeps_other_values` — MIXED (T2+T3+T5); production import warns for unsupported fields while preserving valid display/unknown fields/defaults; T2: import mapping; T3: field warning rules; T5: file read [evidence L3296]
- `explicit_import_uses_normal_toolset_preset_when_toolsets_are_omitted` — MIXED (T2+T5); production import reads Normal profile config and selects Normal preset; T2: preset mapping; T5: file read [evidence L3252]
- `explicit_import_uses_room_toolset_preset_when_toolsets_are_omitted` — MIXED (T2+T5); production import reads Room profile config and selects all toolsets; T2: preset mapping; T5: file read [evidence L3238]
- `explicit_limit_stays_numeric_after_config_load_and_write` — MIXED (T2+T5); production loads temp config, serializes numeric limit, preserves future field; T2: numeric/extra mapping; T5: production config-file read [evidence L2737]
- `loading_a_legacy_file_without_selectors_returns_migration_error` — MIXED (T3+T5); production reads config lacking mode/profile and returns migration guidance; T3: migration rule; T5: file read [evidence L3097]
- `managed_browser_policy_defaults_are_enabled_without_auto_provisioning` — MIXED (T1+T2+T3); checks fixed defaults, empty serialization/parsing, and sparse omission; T1: fixed defaults; T2: serde mapping; T3: sparse/default rules [evidence L3437]
- `managed_browser_policy_round_trips_non_default_values` — MIXED (T2+T4+T5); production writer/load/import round-trip nondefault browser policy; T2: serde mapping; T4: writer/load/import chain; T5: file operations [evidence L3452]
- `max_active_processes_supports_auto_and_explicit_round_trips` — MIXED (T2+T3); maps `auto`/numeric JSON to variants and back; rejects negative/uppercase forms; T2: serde mapping; T3: invalid-input rules [evidence L2530]
- `new_default_config_serializes_auto_limit` — MIXED (T1+T2); checks fixed default JSON process limit, search context, and confirmation channels; T1: fixed defaults; T2: serialization [evidence L2637]
- `old_config_without_process_response_bytes_uses_default` — MIXED (T2+T5); production loads temp file missing field and checks default; T2: default mapping; T5: config-file read [evidence L2512]
- `room_repository_and_maintenance_use_the_v2_json_shape` — MIXED (T2+T3); checks v2 room JSON and rejects legacy `notebookRoot`; T2: JSON mapping; T3: legacy rejection [evidence L2756]
- `sparse_load_preserves_inactive_mode_and_profile_sections` — MIXED (T2+T5); production loads sparse file and preserves inactive tunnel and Room values; T2: field mapping; T5: config-file read [evidence L3018]
- `sparse_load_reconstructs_workspace_dependent_path_policy_and_unknown_fields` — MIXED (T2+T4+T5); production loads temp sparse JSON, derives path policy, keeps unknown field; T2: field/extra mapping; T4: load/default-policy chain; T5: file read [evidence L2995]
- `sparse_projection_always_keeps_selectors_and_omits_reconstructable_defaults` — MIXED (T2+T3); keeps mode/profile/toolsets and omits reconstructable default fields; T2: projection; T3: omission rules [evidence L2902]
- `sparse_projection_keeps_inactive_sections_and_redacts_config_secrets` — MIXED (T2+T3); preserves inactive tunnel/ref and redacts Hub/MCP secrets; T2: projection; T3: secret/inactive rules [evidence L2956]
- `sparse_projection_preserves_custom_workspace_root_but_reconstructs_its_path_defaults` — MIXED (T2+T4+T5); projects custom workspace root, omits default path policy, production load rebuilds it; T2: projection; T4: project/load reconstruction; T5: config file read [evidence L2934]
- `sparse_projection_preserves_explicit_inactive_hub_tunnel_and_room_data` — MIXED (T2+T4+T5); projects inactive data then production loads file and checks preserved Hub/tunnel/Room/future fields; T2: projection; T4: projection/load chain; T5: file read [evidence L3043]
- `strict_load_rejects_legacy_confirmation_provider_shape` — MIXED (T3+T5); production reads temp file and rejects old provider shape with import guidance; T3: migration rule; T5: file read [evidence L3080]
- `strict_load_rejects_legacy_room_skills` — MIXED (T3+T5); production reads temp config and rejects missing top-level skills when legacy room skills appear; T3: strict migration rule; T5: file read [evidence L2780]
- `strict_load_rejects_legacy_top_level_hub_fields_even_with_selectors` — MIXED (T3+T5); production reads temp file and rejects legacy top-level Hub fields; T3: strict schema rule; T5: file read [evidence L3112]
- `toolset_profiles_are_closed_and_deterministic` — MIXED (T1+T3); checks fixed namespace/profile lists plus enable/disable/parser behavior; T1: frozen namespace/order; T3: profile/mutation rules [evidence L2651]
- `tunnel_secret_references_are_strict_and_safe_summary_is_redacted` — MIXED (T3+T2); checks env/file ref policy and summary redaction; T3: reference policy; T2: summary projection [evidence L2832]

### `crates/agentic-gpt/src/config/config_cli.rs`
- `registry_applies_new_scalar_and_list_keys` — MIXED (T2+T3); maps registry strings into display, backup, path-list, and limit fields; T2: mapping; T3: typed parsing [evidence L596]
- `registry_updates_room_repository_and_maintenance_settings` — MIXED (T2+T3); maps Room settings, clears optional path, rejects invalid mode/retired key; T2: mapping; T3: validation [evidence L618]
- `toolset_commands_dispatch_and_reject_unknown_namespaces` — MIXED (T2+T3); parses list/enable/disable into variants and rejects unknown namespace; T2: argument mapping; T3: rejection [evidence L459]
- `toolset_enable_and_disable_persist_across_config_loads` — MIXED (T4+T5); invokes handlers and production writer/loader, checks persisted toolset state; T4: handler/write/load chain; T5: config file operations [evidence L506]
- `toolset_listing_describes_all_namespaces_and_feedback_is_localized` — MIXED (T1+T2); checks fixed English/Chinese listing, states, rows, and messages; T1: fixed text; T2: rendering/localization [evidence L547]

### `crates/agentic-gpt/src/config/config_templates.rs`
- `default_template_is_standalone_normal_with_safe_placeholders` — MIXED (T1+T3+T4); checks defaults/placeholders/pending actions and Standalone validation; T1: fixed placeholders; T3: template rules; T4: builder/validator [evidence L425]
- `hub_template_uses_supplied_connection_values` — MIXED (T2+T4); builds Hub config from supplied values/language and validates it; T2: supplied mapping; T4: builder/validator [evidence L452]
- `imported_base_survives_tui_managed_field_overlay` — MIXED (T2+T4); overlays managed input while preserving imported fields/MCP/unknown extra; T2: field preservation; T4: import/template chain [evidence L507]
- `local_template_omits_tunnel_and_validates_locally` — MIXED (T3+T4); builds Local config without tunnel and validates; T3: mode branch; T4: builder/validator [evidence L440]

### `crates/agentic-gpt/src/config/setup/model.rs`
- `imported_base_seeds_reviewable_fields_without_requiring_an_editor_for_every_field` — MIXED (T2+T4); seeds identity/limits/MCP drafts and includes unknown field in preview; T2: draft mapping; T4: imported-config/session/preview chain [evidence L1118]
- `mcp_server_draft_defaults_empty_and_saves_as_configured` — MIXED (T2+T3); checks empty default, save/status transition, token round-trip/debug redaction; T2: field mapping; T3: session/status/redaction [evidence L1183]
- `preview_is_the_redacted_sparse_projection_without_transaction_secret_material` — MIXED (T2+T4); compares preview with sparse built config, preserves ref, excludes transaction secret; T2: projection/redaction; T4: active-input/build/preview chain [evidence L1086]
- `tunnel_secret_reference_seeds_are_parsed_without_exposing_secret_text` — MIXED (T2+T3); maps file/env refs into draft fields and checks Hub secret debug redaction; T2: reference mapping; T3: secret handling [evidence L1038]

### `crates/agentic-gpt/src/config/setup/outcome.rs`
- `aliased_config_and_secret_paths_are_rejected_before_secret_write` — MIXED (T3+T5); rejects config/secret path alias and checks existing config unchanged/no marker in backup; T3: alias validation; T5: filesystem path/effect assertion [evidence L905]
- `commit_creates_secret_parent_0700_file_0600_and_config` — MIXED (T4+T5); commits secret/config and checks permissions/reference; T4: outcome/secret/config chain; T5: production writes [evidence L702]
- `commit_replacement_restores_existing_mode_and_bytes_on_config_failure` — MIXED (T4+T5); config-write failure restores previous secret bytes/mode; T4: commit/rollback; T5: file operations [evidence L734]
- `config_failure_removes_new_secret_and_invalid_target_has_no_side_effect` — MIXED (T4+T5); failed config write removes new secret; invalid target creates no files; T4: rollback chain; T5: file operations [evidence L761]
- `config_load_does_not_recover_setup_while_mutation_lock_is_held` — MIXED (T4+T5); blocks production load behind lock, checks journal state, then recovery restores secret/mode and removes evidence; T4: lock/load/recovery; T5: thread/lock/filesystem [evidence L658]
- `imported_config_review_edit_commits_events_and_keeps_backup` — MIXED (T4+T5); production import/review/commit edits events and checks output/preserved values/backup; T4: import/session/review/commit chain; T5: file operations [evidence L809]
- `no_secret_outcome_writes_config_without_secret_material` — MIXED (T4+T5); builds Local no-secret outcome, commits config, checks file exists; T4: session/outcome/commit; T5: production write [evidence L789]
- `outcome_handoff_revalidates_canonical_connection_before_any_write_plan` — MIXED (T3+T4); rejects invalid Hub URL before write plan; no filesystem call/material file operation; T3: validation; T4: setup/outcome handoff [evidence L978]
- `recovery_restores_crash_state_and_retains_conflicting_evidence` — MIXED (T4+T5); recovery restores old secret/mode and removes journal/backup; conflict retains secret/evidence; T4: recovery chain; T5: actual file operations [evidence L681]
- `symlink_parent_alias_to_nonexistent_target_is_rejected_before_secret_write` — MIXED (T3+T5); rejects symlink-parent alias and checks no target/secret-bearing backup written; T3: canonical path validation; T5: filesystem operation [evidence L938]

### `crates/agentic-gpt/src/config/setup/review.rs`
- `event_review_preserves_values_and_offers_every_override_choice` — MIXED (T1+T2); projects event values and checks fixed choices; T1: fixed choices; T2: draft/review mapping [evidence L189]
- `review_is_redacted_active_mode_only_and_reports_secret_write_intent` — MIXED (T2+T3); checks active-mode projection, inactive Hub exclusion, write intent/redaction, Room applicability; T2: projection; T3: mode/status/redaction [evidence L16]
- `review_preserves_pending_actions_and_redacted_standalone_reference` — MIXED (T2+T3); checks deferred/immediate write intent, pending actions, and reference; T2: projection; T3: action rules [evidence L224]

### `crates/agentic-gpt/src/config/setup/validation.rs`
- `active_input_ignores_inactive_connection_secrets_and_restores_staged_drafts` — MIXED (T2+T3); filters inactive credentials and restores them on mode switch; T2: active-input projection; T3: state behavior [evidence L219]
- `shell_init_file_modes_preserve_default_disabled_and_explicit_path` — MIXED (T2+T4); maps shell draft modes through config building to enum values; T2: mapping; T4: save/build chain [evidence L145]

### `crates/agentic-gpt/src/files/file_ops.rs`
- `binary_content_is_rejected_with_or_without_metadata` — MIXED (T2+T5); reads invalid UTF-8 bytes in both metadata modes and checks error ; T2: invalid-content/error mapping; T5: production file read. Evidence L2765–L2783 [evidence L2765–L2783]
- `bounded_reader_never_returns_more_than_the_requested_limit` — MIXED (T2+T5); checks real file exceeds at limit four and is complete at five ; T2: bounded-read result mapping; T5: production file read. Evidence L2813–L2830 [evidence L2813–L2830]
- `deny_and_readonly_policy_precedence_is_enforced` — MIXED (T3+T5); reads read-only file, rejects write there, and rejects denied-file read ; T3: policy precedence; T5: actual file read/path resolution. Evidence L2719–L2761 [evidence L2719–L2761]
- `large_files_are_rejected_with_or_without_metadata` — MIXED (T3+T5); checks an over-limit real file is rejected in both modes ; T3: size-limit decision; T5: production size/read check. Evidence L2787–L2809 [evidence L2787–L2809]
- `move_source_removal_failure_compensates_destination` — MIXED (T4+T5); injects source-removal failure during move and checks source survives/destination absent ; T4: commit/compensation chain; T5: actual filesystem effects. Evidence L3179–L3212 [evidence L3179–L3212]
- `ranges_are_bounded_and_utf8_safe` — MIXED (T2+T5); reads selected multibyte line and checks range/content/count ; T2: line-range/UTF-8 mapping; T5: production file read. Evidence L2704–L2715 [evidence L2704–L2715]
- `reads_metadata_without_exposing_revision_and_preserves_newlines` — MIXED (T2+T5); reads real CRLF/LF content and checks metadata/omitted revision ; T2: content/metadata response mapping; T5: production file read. Evidence L2688–L2700 [evidence L2688–L2700]
- `search_streams_file_byte_and_output_limits_without_overshoot` — MIXED (T3+T5); checks file/byte scanning limits and output cap over real files ; T3: scan/output budgeting; T5: production traversal/reads. Evidence L2932–L3008 [evidence L2932–L3008]
- `search_supports_literal_regex_glob_context_and_skip_accounting` — MIXED (T3+T5); searches fixture tree and checks literal/regex/glob matching, context, offsets, non-UTF8 skip, and result truncation ; T3: search/matching/context logic; T5: production traversal/file reads. Evidence L2865–L2928 [evidence L2865–L2928]
- `symlinks_are_allowed_only_when_the_canonical_target_stays_inside_policy` — MIXED (T3+T5); accepts in-root symlink and rejects outside target ; T3: path-policy decision; T5: real Unix symlink/canonical lookup. Evidence L2834–L2861 (`#[cfg(unix)]`) [evidence L2834–L2861 (`#[cfg(unix)]`)]

### `crates/agentic-gpt/src/ingress/hub.rs`
- `recovery_replays_completed_results_but_never_executes_retired_argv_commands` — MIXED (T4+T5); T4 recovery reconciles retired/current commands with response channels and preserves completed results; T5 reads/writes the temporary transport ledger and verifies the retired argv never creates a marker file [evidence L1431–L1575]

### `crates/agentic-gpt/src/ingress/stdio_server_tests.rs`
- `browser_manual_dispatch_uses_selected_runtime_docs_root` — MIXED (T4+T5); T4 exercises manual read/search dispatch and path rejection; T5 reads actual temporary documentation files [evidence L783–L822]
- `browser_missing_runtime_has_stable_degraded_results` — MIXED (T4+T5); T4 checks Browser adapter degraded responses; T5 reads the produced audit file and verifies a hashed/redacted failed-call record [evidence L707–L752]
- `browser_repl_event_decoration_preserves_inner_result_channels` — MIXED (T2+T4); T2 verifies result content/structured/error/meta channels survive decoration; T4 sends a canned fake-kernel Browser result through MCP stdio and checks one panel. The empty EventStore read is excluded from T5 [evidence L876–L955]
- `changing_live_toolsets_updates_surface_and_authorization` — MIXED (T1+T4+T5); T1 checks changing advertised surface; T4 verifies calls are denied/allowed as config toggles; T5 successful bootstrap reads the seeded temporary bootstrap file [evidence L1189–L1232]
- `compact_mcp_skills_and_tmux_adapters_preserve_result_envelopes` — MIXED (T4+T5); T4 dispatches MCP/skills/tmux and checks basic envelopes; T5 attempts real external tmux list commands [evidence L3669–L3686]
- `denied_process_batch_creates_no_processes` — MIXED (T4+T5); T4: simulates Hub confirmation denial and verifies the batch refusal; T5: verifies the post-denial ProcessHistory SQLite list remains empty (asserted no-persisted-process side effect). [evidence L2268–2309]
- `deterministic_tool_contract_corpus_exercises_public_dispatch` — MIXED (T1+T4+T5); T1 checks descriptor required fields; T4 dispatches positive/negative cases; T5 reads/searches real temporary fixture files [evidence L1013–L1164]
- `external_injection_waits_for_the_next_public_panel_exposure` — MIXED (T4+T5); T4 exercises LocalUnix-only injection, panel exposure/clearing, and Http denial; T5 asserts persisted event-store visibility and shown-count changes [evidence L260–L323]
- `file_edit_add_creates_nested_parents_after_whole_patch_preflight` — MIXED (T4+T5); T4 checks whole-patch preflight, confirmation, external-change and commit-failure outcomes; T5 asserts real directory/file contents and no-write guarantees [evidence L3310–L3393]
- `file_edit_add_keeps_path_policy_for_missing_parents_and_symlinks` — MIXED (T4+T5); T4 checks denied-root/path-policy decisions; T5 creates real symlinks and verifies no denied directories are written [evidence L3421–L3453]
- `file_edit_add_reserves_audit_path_under_symlinked_workspace` — MIXED (T4+T5); T4 checks reserved-path rejection; T5 creates a real workspace symlink and verifies the audit path remains a file [evidence L3394–L3420]
- `file_edit_apply_patch_context_mismatch_and_confirmation_do_not_write` — MIXED (T4+T5); T4 checks patch-context and confirmation failure paths; T5 verifies original file remains unchanged [evidence L3563–L3596]
- `file_edit_apply_patch_revalidates_external_change_before_commit` — MIXED (T4+T5); T4 checks revision revalidation/conflict behavior; T5 verifies the externally changed file contents remain on disk [evidence L3598–L3614]
- `file_edit_apply_patch_supports_multi_file_changes_and_slim_response` — MIXED (T2+T4+T5); T2 checks compact per-file change projection; T4 applies a multi-operation patch; T5 verifies real add/update/delete/move effects [evidence L3455–L3500]
- `file_edit_external_changes_remain_isolated_by_path` — MIXED (T4+T5); T4 checks independent revision-conflict handling across server states; T5 verifies two real workspace files preserve their distinct contents [evidence L3616–L3653]
- `file_edit_later_commit_failure_reports_prior_and_skipped_changes` — MIXED (T2+T4+T5); T2 checks committed/failed/skipped response and audit mapping; T4 tests ordered partial commit behavior; T5 asserts real file and audit-log effects under injected commit failures [evidence L3502–L3561]
- `file_image_response_adds_event_panel_text_once` — MIXED (T2+T4+T5); T2 checks image/text/panel channel projection without duplication; T4 exercises the stdio decoration chain; T5 reads a PNG fixture and a seeded EventStore event, checking shown count [evidence L2737–L2822]
- `file_read_and_search_batches_preserve_order_and_isolate_failures` — MIXED (T4+T5); T4 checks batch order, per-item failure isolation, and invalid single/batch combinations; T5 reads/searches real temporary files [evidence L3245–L3308]
- `file_read_dispatch_supports_content_and_metadata_modes` — MIXED (T4+T5); T4 checks content/metadata/error response paths; T5 reads a real temporary file and verifies its content/line metadata [evidence L2667–L2691]
- `file_search_dispatch_supports_literal_and_regex_queries` — MIXED (T4+T5); T4 checks configured context clipping and literal/regex search behavior; T5 searches a real temporary file [evidence L3177–L3243]
- `in_process_stdio_file_read_batch_descriptor_and_call_contract` — MIXED (T1+T4+T5); T1 checks file.read batch descriptor contract; T4 calls via MCP duplex and verifies ordered partial results; T5 reads real temporary files [evidence L2693–L2735]
- `in_process_stdio_file_read_enforces_image_pixel_and_response_bounds` — MIXED (T2+T4+T5); T2 checks error/budget projections and batch recovery; T4 drives file reads through stdio; T5 reads malformed/oversized image and text fixtures and asserts actual response bounds [evidence L3013–L3175]
- `in_process_stdio_file_read_projects_static_and_gif_images` — MIXED (T2+T4+T5); T2 checks image bytes/MIME/frame and batch-result projection; T4 exercises MCP file-read/batch output; T5 reads and decodes real image fixtures [evidence L2824–L3011]
- `local_mcp_call_audit_uses_local_request_source` — MIXED (T4+T5); T4 checks rejected MCP operation and terminal audit evidence; T5 reads the written audit file [evidence L2540–L2566]
- `local_skill_audit_uses_local_request_source` — MIXED (T4+T5); T4 activates/runs a skill through LocalUnix dispatch and checks request-source audit mapping; T5 creates and executes a real temporary shell script and reads the audit file [evidence L2504–L2538]
- `managed_batch_uses_one_confirmation_for_all_elements` — MIXED (T4+T5); T4 simulates a Hub confirmation and checks one confirmation serves the batch; T5 executes two commands and reads their audit records from disk [evidence L2210–L2266]
- `normal_and_room_tool_sets_follow_fixed_surface_contract` — MIXED (T1+T4); T1 asserts exact Normal/Room tool surface/schema; T4 changes namespace configuration and verifies advertised tools and call authorization update [evidence L100–L258]
- `process_creation_and_batch_responses_obey_response_budget` — MIXED (T2+T4+T5); T2 checks compact response fields/serialized byte budget; T4 checks single/batch response flow; T5 runs commands that produce large output [evidence L1840–L1892]
- `process_creation_read_cancel_and_batch_use_process_api` — MIXED (T4+T5); T4 checks process create/read/list/cancel/batch state transitions and batch preflight; T5 spawns true/sleep/false commands and asserts actual outcomes/termination [evidence L1490–L1623]
- `process_read_omitted_wait_but_zero_is_nonblocking` — MIXED (T4+T5); T4 compares process-read wait semantics; T5 runs a real sleep process and observes it active/completed [evidence L1625–L1668]
- `process_read_preserves_mcp_result_states_and_complete_values` — MIXED (T2+T4+T5); T2 checks included/deferred/unavailable/not-retained result projection; T4 seeds ProcessHistory and reads via dispatch; T5 asserts persisted SQLite result data and retry [evidence L2050–L2208]
- `process_read_preserves_raw_byte_offsets_and_utf8_output` — MIXED (T2+T4+T5); T2 verifies byte-offset/cursor and UTF-8/base64 mapping; T4 chains process creation with paginated reads; T5 executes printf and reads captured output [evidence L1768–L1838]
- `process_shapes_are_compact_and_keep_group_filters` — MIXED (T4+T5); T4 checks read/list projections, group filtering, and rejection state; T5 executes true/sleep/false commands [evidence L1670–L1766]
- `process_tools_reject_legacy_identity_and_confirmation_fields` — MIXED (T1+T4); T1 checks current/retired process API names and accepted schema shape; T4 calls the server to verify legacy fields/aliases and invalid range are rejected [evidence L1371–L1488]
- `room_profile_dispatches_room_memory_tools` — MIXED (T4+T5); T4 dispatches bootstrap/skills/notebook/diary/state adapters; T5 bootstrap reads the seeded temporary workspace file [evidence L2619–L2633]
- `targetless_hub_listing_keeps_events_for_targeted_calls` — MIXED (T4+T5); T4 drives suppressed then targeted Hub responses; T5 asserts event records and shown counts in the persistent EventStore [evidence L350–L428]
- `terminal_result_is_not_suppressed_when_panel_preparation_fails` — MIXED (T4+T5); T4 tests process-result/event-panel recovery after decoration failure; T5 executes `true` and induces/checks a real SQLite trigger failure and retained event [evidence L430–L479]
- `tunnel_local_and_http_ingress_advertise_identical_surface` — MIXED (T1+T2); T1 checks identical advertised tool descriptors; T2 checks ingress label/source string mapping [evidence L2483–L2502]
- `tunnel_skill_audit_uses_tunnel_request_source` — MIXED (T4+T5); T4 checks skill completion and tunnel-source audit mapping; T5 creates/executes a temporary shell script and reads audit output [evidence L2568–L2600]

### `crates/agentic-gpt/src/mcp/mcp_tests.rs`
- `config_cli_rejects_invalid_server_without_writing_and_accepts_valid_server` — MIXED (T3+T5); **T3:** checks config validation for invalid and valid server additions. **T5:** verifies the real config file stays unchanged on rejection and persists/reloads a valid addition. [evidence L1837–1878]
- `managed_mcp_fast_result_uses_real_rmcp_transport` — MIXED (T4+T5); **T4:** managed call/process flow over the in-process RMCP test server. **T5:** verifies the persisted audit JSONL’s metadata and secret redaction. [evidence L348–400]
- `mcp_batch_clips_late_results_to_the_aggregate_budget` — MIXED (T2+T4); **T2:** projects omitted retained results as deferred. **T4:** exercises batch result clipping and process-read recovery. [evidence L1192–1295]
- `mcp_batch_impossible_response_budget_rejects_before_registration_or_effects` — MIXED (T4+T5); **T4:** checks rejection before batch/process/confirmation effects. **T5:** checks the persisted response-budget audit record. [evidence L973–1026]
- `mcp_batch_preflight_and_capacity_fail_atomically_before_confirmation` — MIXED (T4+T5); **T4:** checks batch validation and capacity rejection before registration, confirmation, or tool calls. **T5:** checks persisted rejection/capacity audit records. [evidence L792–969]
- `mcp_batch_public_projection_obeys_exact_json_budget_and_keeps_deferred_result_readable` — MIXED (T2+T4); **T2:** checks projection at minimum, exact-fit, and one-byte-short JSON budgets. **T4:** exercises batch results and process-read recovery. [evidence L1298–1452]
- `mcp_batch_sequential_fail_fast_preserves_order_and_audit_correlation` — MIXED (T4+T5); **T4:** first tool error skips later calls and preserves child ordering/correlation. **T5:** checks persisted batch/child audit records. [evidence L1030–1126]

### `crates/agentic-gpt/src/operations/event_notifications.rs`
- `process_history_recovery_failure_does_not_block_event_access` — MIXED (T4+T5); T4 tests recovery handling a disabled process-history store without blocking event access; T5 retrieves a previously inserted event from the temporary EventStore [evidence L634–L662]
- `recovery_does_not_locally_settle_remote_awaiting_source` — MIXED (T4+T5); T4 tests recovery preserving a remote-awaiting source/origin; T5 asserts the registered/bound/completed source remains in the persisted EventStore [evidence L598–L632]
- `response_feedback_requires_registered_sources_and_keeps_off_policy_quiet` — MIXED (T4+T5); T4 tests source registration, completion settlement, and off-policy suppression; T5 writes/reads registered completion state through the temporary EventStore database [evidence L448–L559]

### `crates/agentic-gpt/src/process/exec.rs`
- `losing_startup_channel_before_ready_prevents_command_execution` — MIXED (T4+T5); closes startup channel while init is blocked and checks command marker remains absent after release ; T4: startup-channel/command-gating chain; T5: actual child/FD/filesystem behavior. Evidence L803–L878; Unix test module [evidence L803–L878]
- `relative_shell_init_file_is_not_resolved_through_path` — MIXED (T3+T5); launches shell with duplicate init name in cwd and PATH; checks cwd source/output and startup markers ; T3: relative-init resolution rule; T5: actual shell and file reads. Evidence L719–L802; Unix test module [evidence L719–L802]

### `crates/agentic-gpt/src/process/managed.rs`
- `aborting_cancellation_releases_stale_terminal_process_pin` — MIXED (T4+T5); aborts cancellation after pin acquisition, kills TERM-resistant child group, and checks pruning succeeds ; T4: cancellation-pin/pruning chain; T5: actual group/task effects. Evidence L6378–L6469 [evidence L6378–L6469]
- `active_process_capacity_and_cancel_are_truthful` — MIXED (T4+T5); checks capacity rejection with live sleep, then cancellation and observed process-group evidence ; T4: capacity/cancel response state; T5: real child/process-group signal. Evidence L6048–L6078 [evidence L6048–L6078]
- `admitted_process_keeps_policy_snapshot_across_reload` — MIXED (T4+T5); changes policy after admitting printf and checks admitted child still completes with output ; T4: admission/config-snapshot behavior; T5: real child. Evidence L5751–L5775 [evidence L5751–L5775]
- `cancellation_reaches_background_descendant_after_leader_exit` — MIXED (T4+T5); leader exits while background sleep remains; cancellation kills group and settles capture ; T4: managed leader/group/capture lifecycle; T5: real descendant process group. Evidence L6080–L6113 [evidence L6080–L6113]
- `cancelling_a_terminal_process_reports_already_terminal_without_rewriting_state` — MIXED (T4+T5); after true/history completion, checks cancellation idempotence and live/history fields unchanged ; T4: terminal-state invariant; T5: actual child and history DB. Evidence L6471–L6525 [evidence L6471–L6525]
- `cancelling_aged_terminal_process_returns_detail_after_cache_pruning` — MIXED (T4+T5); ages cached terminal process with background group, cancels, and checks terminal detail/group termination ; T4: aged-cache cancellation behavior; T5: actual group termination. Evidence L6332–L6376 [evidence L6332–L6376]
- `completed_processes_release_capacity_and_keep_output` — MIXED (T4+T5); completes true, admits printf, checks stale-state refresh and captured output/EOF ; T4: capacity-release/state/capture chain; T5: real children. Evidence L4947–L4972 [evidence L4947–L4972]
- `exec_preserves_rejection_when_process_was_not_admitted` — MIXED (T4+T5); fills capacity with sleep and checks second command returns rejected/not-started response ; T4: admission-to-response behavior; T5: running child/capacity contention. Evidence L4913–L4945 [evidence L4913–L4945]
- `exec_waits_for_terminal_state_after_early_output` — MIXED (T4+T5); runs printf then sleep and waits for terminal response ; T4: wait/output/state chain; T5: actual child. Evidence L4899–L4910 [evidence L4899–L4910]
- `mcp_batch_admission_failure_is_atomic_before_registration` — MIXED (T4+T5); SQLite trigger rejects second registration, verifies no partial registration/history, then retries ; T4: MCP registration atomicity; T5: actual history database. Evidence L5860–L5883 [evidence L5860–L5883]
- `output_pages_base64_invalid_utf8_without_loss` — MIXED (T2+T5); child emits invalid bytes; checks Base64 representation round-trips bytes and respects response budget ; T2: binary-to-Base64 response mapping; T5: real child output. Evidence L5396–L5433 [evidence L5396–L5433]
- `oversized_process_admission_fails_before_command_effect` — MIXED (T4+T5); Linux oversized commands fail admission, with no marker, process registry entry, or history record ; T4: size-check/admission chain; T5: real filesystem/history no-effect checks. Evidence L5660–L5711 (`#[cfg(target_os = "linux")]`) [evidence L5660–L5711 (`#[cfg(target_os = "linux")]`)]
- `oversized_process_batch_is_rejected_before_any_admission` — MIXED (T4+T5); rejects 100-element batch and checks empty process/history state and no marker files ; T4: batch-level budget/admission logic; T5: real history/filesystem no-effect checks. Evidence L5712–L5748 [evidence L5712–L5748]
- `process_batch_admission_failure_is_atomic_before_spawn` — MIXED (T4+T5); SQLite trigger fails second admission, checks no partial history/process/marker, then retries real children ; T4: atomic batch-admission chain; T5: history DB and child execution. Evidence L5777–L5858 [evidence L5777–L5858]
- `process_batch_respects_max_concurrent_tasks_without_blocking_batch_return` — MIXED (T4+T5); checks a batch returns with one sleep running and one queued, then cancels them ; T4: scheduler/queue behavior; T5: real children/cancellation. Evidence L5885–L5955 [evidence L5885–L5955]
- `process_batch_response_obeys_whole_body_budget_and_keeps_identities` — MIXED (T2+T5); runs two large-output children and checks whole-body budget, batch identities/indexes, and pages ; T2: identity/response-budget mapping; T5: real batch children. Evidence L5580–L5659 [evidence L5580–L5659]
- `process_batch_wait_wakes_for_later_child` — MIXED (T4+T5); queues true and short sleep at concurrency one; checks batch wait returns after both complete ; T4: queued-child wakeup/state chain; T5: real children. Evidence L5957–L6000 [evidence L5957–L6000]
- `process_filters_and_restart_loss_are_explicit` — MIXED (T2+T5); runs true, checks completed filter and old-boot versus unknown-id errors ; T2: filter/id-to-error mapping; T5: actual child. Evidence L6527–L6558 [evidence L6527–L6558]
- `process_list_merges_live_wins_and_uses_global_cursor_order` — MIXED (T3+T5); inserts persisted rows/live duplicate and checks live-wins state, filters, and global cursor ordering ; T3: merge/filter/cursor logic; T5: SQLite history read/write. Evidence L6629–L6737 [evidence L6629–L6737]
- `process_read_compacts_output_to_configured_response_budget` — MIXED (T2+T5); runs large output and checks response fits configured budget with continuation page ; T2: response fitting; T5: real child output. Evidence L5477–L5496 [evidence L5477–L5496]
- `process_read_cursors_are_replayable_and_page_raw_offsets` — MIXED (T2+T5); checks bounded replayable pages, offsets, reconstruction, and malformed/status/ahead/foreign cursor errors over child output ; T2: cursor/page mapping; T5: real process output. Evidence L5267–L5394 [evidence L5267–L5394]
- `reader_eof_is_visible_while_the_child_keeps_running` — MIXED (T4+T5); checks active child can have completed output capture/EOF before process completion ; T4: reader/process-state coordination; T5: real child/pipe. Evidence L5111–L5154 [evidence L5111–L5154]
- `revoked_group_is_not_reused_by_capacity_reader_or_cancellation` — MIXED (T3+T5); on Unix revokes real PGID, checks no reused-id probe and exercises reader/cancel/prune paths ; T3: revoked-ID observation logic; T5: actual process-group lifecycle. Evidence L6191–L6330 (`#[cfg(unix)]`) [evidence L6191–L6330 (`#[cfg(unix)]`)]
- `runtime_history_tracks_group_timestamps_and_hot_cache_fallback` — MIXED (T4+T5); runs grouped printf, checks timestamps/history/output, prunes cache, then retrieves durable record/output ; T4: live/history fallback chain; T5: real child and history DB. Evidence L6560–L6627 [evidence L6560–L6627]
- `shell_init_failure_blocks_command_and_reports_init_error` — MIXED (T4+T5); init returns 23; verifies command did not create marker and managed error reports init failure ; T4: startup-failure-to-detail chain; T5: real child/file. Evidence L5007–L5032 [evidence L5007–L5032]
- `shell_init_fd3_changes_preserve_startup_channel` — MIXED (T4+T5); closes/rebinds FD3 during init and checks commands run and output bytes ; T4: startup-channel handling; T5: real shell/descriptors/files. Evidence L5034–L5078 [evidence L5034–L5078]
- `shell_init_runs_at_top_level_before_working_directory_is_restored` — MIXED (T4+T5); init sets variable/marker and changes cwd; command sees variable but uses configured cwd ; T4: startup/command sequencing; T5: real shell/filesystem. Evidence L4974–L5005 [evidence L4974–L5005]
- `terminal_events_precede_inherited_pipe_eof_and_history_keeps_final_output` — MIXED (T4+T5); checks terminal event/hook precedes EOF, later inherited output is captured, and history is durable ; T4: event/reader/history ordering; T5: real child/event/history resources. Evidence L5156–L5265 [evidence L5156–L5265]
- `terminal_leader_keeps_capacity_after_eof_until_group_retirement` — MIXED (T4+T5); checks terminal leader retains capacity through group retirement, then new child is admitted ; T4: capacity/group-retirement chain; T5: real background process group. Evidence L6115–L6188 [evidence L6115–L6188]

### `crates/agentic-gpt/src/room/bootstrap.rs`
- `flat_discovery_ignores_hidden_non_markdown_and_nested_entries` — MIXED (T3+T5); **T3:** checks guide discovery/filter rules. **T5:** applies them to actual directory entries/files. [evidence L908–925]
- `frontmatter_scan_limit_accepts_exact_boundary_and_rejects_overflow` — MIXED (T3+T5); **T3:** checks exact-boundary acceptance and overflow rejection. **T5:** reads actual entrypoint/guide files. [evidence L1149–1189]
- `guides_sort_by_priority_then_id_and_manifest_caps_at_64_but_read_keeps_all` — MIXED (T3+T5); **T3:** checks priority sorting, manifest cap, and read selection. **T5:** discovers/reads actual guide files. [evidence L1020–1051]
- `invalid_and_valid_guides_have_different_revision_membership` — MIXED (T3+T5); **T3:** checks how valid versus invalid guide content affects revision. **T5:** mutates and reloads actual files. [evidence L1054–1082]
- `invalid_guides_are_excluded_and_duplicate_ids_are_order_independent` — MIXED (T3+T5); **T3:** checks duplicate-ID, metadata, and UTF-8 validation/warning behavior. **T5:** loads actual guide files. [evidence L928–968]
- `missing_package_and_invalid_entrypoint_are_fail_closed` — MIXED (T2+T5); **T2:** checks error mapping to `bootstrap_not_found` versus `bootstrap_invalid`. **T5:** loader checks the actual filesystem package/entrypoint. [evidence L861–878]
- `mixed_validity_duplicate_ids_exclude_every_candidate` — MIXED (T3+T5); **T3:** checks duplicate-ID exclusion for valid and invalid candidates. **T5:** loads the candidates from actual files. [evidence L971–1001]
- `oversized_files_keep_full_metadata_with_bounded_prefixes` — MIXED (T2+T5); **T2:** checks size/hash metadata and bounded-prefix projection. **T5:** reads oversized entrypoint and guide files. [evidence L1106–1146]
- `utf8_validation_accepts_multibyte_codepoint_split_across_scan_chunks` — MIXED (T3+T5); **T3:** checks chunk-boundary UTF-8 validation, digest, and line count. **T5:** scans an actual file. [evidence L1192–1212]
- `valid_entrypoint_defaults_and_crlf_are_supported` — MIXED (T3+T5); **T3:** checks parsed CRLF metadata and default fields. **T5:** loads actual entrypoint and guide files. [evidence L882–905]

### `crates/agentic-gpt/src/room/room_maintenance.rs`
- `local_submit_preflights_and_commits_one_semantic_change` — MIXED (T2+T5); **T2:** checks payload-to-Markdown mapping. **T5:** verifies the resulting file, commits, and worktree state in a real repository. [evidence L1458–1497]
- `preflight_rejects_invalid_payload_without_mutating_room` — MIXED (T3+T5); **T3:** checks invalid-payload validation. **T5:** verifies actual repository files and commit count remain unchanged. [evidence L1500–1534]
- `status_reports_capabilities_and_empty_slots` — MIXED (T3+T5); **T3:** checks status/capability/slot mapping. **T5:** obtains status after initializing an actual repository. [evidence L1429–1455]
- `submit_rejects_duplicate_slots_and_dirty_repository` — MIXED (T3+T5); **T3:** checks duplicate-slot rejection. **T5:** checks rejection based on actual dirty Git state. [evidence L1537–1567]
- `workflow_wait_timeout_preserves_submitted_request` — MIXED (T2+T5); **T2:** checks request serialization/forwarding. **T5:** verifies the submitted request in the real local Git repo and bare origin after wait timeout. [evidence L1321–1363]

### `crates/agentic-gpt/src/room/room_reads.rs`
- `diary_active_and_exact_reads_are_deterministic_for_current_layout` — MIXED (T2+T5); **T2:** checks diary layer/period/path mapping. **T5:** reads actual current and exact-period diary files. [evidence L624–758]
- `diary_active_reports_missing_layer_without_mutating_repository` — MIXED (T3+T5); **T3:** checks missing-layer response and non-mutation behavior. **T5:** verifies against an actual partial diary tree. [evidence L761–807]
- `notebook_recent_orders_by_recency_before_limit` — MIXED (T3+T5); **T3:** checks recency-before-limit ordering and deterministic tie-breaking. **T5:** discovers actual notebook paths and constructs documents from files. [evidence L873–920]
- `state_list_entities_round_trip_for_dotted_stem` — MIXED (T2+T5); **T2:** checks dotted filename/entity mapping. **T5:** lists and reads an actual state file. [evidence L592–621]
- `state_list_is_sorted_and_state_read_rejects_oversized_markdown` — MIXED (T3+T5); **T3:** checks sorting and size-limit rules. **T5:** lists/reads actual Markdown files, including the oversized file. [evidence L810–870]

### `crates/agentic-gpt/src/room/room_repository.rs`
- `absent_root_bootstraps_exact_scaffold_and_one_main_commit` — MIXED (T1+T5); **T1:** checks exact scaffold path inventory. **T5:** initializes and inspects an actual Git repository. The body does not assert one-commit count despite the name. [evidence L1102–1124]
- `bounded_markdown_helper_caps_bytes_and_rejects_symlinks` — MIXED (T3+T5); **T3:** checks byte-bound and extension behavior. **T5:** reads an actual file. The body does not create/test a symlink despite the name. [evidence L1365–1375]
- `empty_root_bootstraps_exact_scaffold` — MIXED (T1+T5); **T1:** checks exact scaffold inventory. **T5:** verifies actual Git branch/tree and one-commit count. [evidence L1127–1155]
- `independent_bootstraps_have_the_same_deterministic_initial_commit` — MIXED (T3+T5); **T3:** checks deterministic initial-bootstrap output. **T5:** compares two actual Git commits and their files. [evidence L1158–1180]
- `invalid_boundary_does_not_partially_initialize_repository` — MIXED (T3+T5); **T3:** checks invalid boundary rejection. **T5:** verifies the real repository root was not created. [evidence L1092–1099]
- `path_validation_rejects_escape_and_symlink_root` — MIXED (T2+T5); **T2:** checks path/traversal validation. **T5:** on Unix, verifies a real symlinked repository root is rejected. [evidence L1255–1276]
- `schema_version_detection_distinguishes_outdated_and_invalid_metadata` — MIXED (T3+T5); **T3:** checks Outdated versus Missing readiness classification. **T5:** inspects mutated metadata in an actual repository. [evidence L1233–1252]
- `status_keeps_repository_schema_executor_workflow_remote_and_sync_distinct` — MIXED (T3+T5); **T3:** checks readiness-state classification. **T5:** compares states before and after actual Git/bootstrap operations. [evidence L1294–1313]
- `workflow_uses_the_repository_owned_executor` — MIXED (T1+T5); **T1:** checks fixed generated workflow commands/path list. **T5:** bootstraps and reads the actual repository workflow file. [evidence L1316–1333]

### `crates/agentic-gpt/src/runtime/agent_info.rs`
- `config_health_ignores_path_policy_drift_with_workspace_restart` — MIXED (T3+T5); compares live and disk workspace/path roots; requires restart fields without live-subset issue ; T3: config-health comparison logic; T5: actual config-file read and real path resolution. Evidence L537–L578 [evidence L537–L578]
- `info_reports_invalid_config_and_capacity_exhaustion_without_secrets` — MIXED (T4+T5); checks invalid disk JSON, degraded capacity health, and secret omission ; T4: aggregate health reporting; T5: actual invalid-config file read. Evidence L695–L730 [evidence L695–L730]
- `info_reports_mcp_live_subset_revision_without_restart_requirement` — MIXED (T4+T5); checks MCP counts/revision and invalid-disk reporting before/after changing live MCP config ; T4: info/config-health plus live MCP-state chain; T5: actual disk-config reads/writes. Evidence L581–L660 [evidence L581–L660]
- `info_reports_restart_differences_and_current_ntfy_relay` — MIXED (T4+T5); checks restart fields/current ntfy state against effective and disk config ; T4: runtime info/config-health aggregation; T5: actual temporary config-file read. Evidence L492–L534 [evidence L492–L534]
- `info_reports_toolset_live_subset_difference_without_restart_requirement` — MIXED (T3+T5); compares effective and disk toolsets, then checks live-subset match after application ; T3: live-subset/restart classification; T5: actual config-file read. Evidence L663–L692 [evidence L663–L692]

### `crates/agentic-gpt/src/runtime/main_tests.rs`
- `batch_confirmation_preview_supports_chinese` — MIXED (T1+T2); renders Chinese preview and checks localized wording/cwd/escaping ; T1: fixed localized strings; T2: batch-element-to-preview rendering. Evidence L1107–L1124 [evidence L1107–L1124]
- `cli_version_uses_crate_version` — MIXED (T1+T2); parses `--version` and checks display-version kind and rendered crate version ; T1: fixed version text; T2: CLI version rendering/forwarding. Evidence L309–L318 [evidence L309–L318]
- `deny_roots_override_read_and_write` — MIXED (T3+T5); checks both read and write commands are denied under an actual denied root ; T3: deny precedence; T5: real filesystem path resolution. Evidence L1040–L1075 [evidence L1040–L1075]
- `hub_panel_failure_emits_live_event_source_without_terminal_response` — MIXED (T4+T5); forces SQLite panel-update failure after a hub `true` command and checks event-source handoff/no response ; T4: hub/event sequencing; T5: child execution and SQLite trigger/store operations. Evidence L561–L652 [evidence L561–L652]
- `load_old_config_without_path_policy_adds_defaults` — MIXED (T2+T5); writes legacy config without `pathPolicy`, reloads, and checks defaults ; T2: legacy-field-to-default mapping; T5: production config-file read. Evidence L1239–L1250 [evidence L1239–L1250]
- `load_partial_path_policy_uses_workspace_derived_defaults_for_missing_lists` — MIXED (T2+T5); loads only write roots and checks other lists use workspace defaults ; T2: partial-config mapping; T5: production config-file read. Evidence L1254–L1266 [evidence L1254–L1266]
- `local_arguments_are_bounded_objects_from_inline_or_file` — MIXED (T2+T5); checks inline/default/invalid/oversized JSON and reads a temporary argument file ; T2: JSON object and size handling; T5: production file read. Evidence L383–L410 [evidence L383–L410]
- `managed_cache_precedes_desktop_and_preserves_descriptor_fields` — MIXED (T3+T5); injected providers make managed cache win and checks descriptor/call counts plus Unix codex-home mode ; T3: provider selection and descriptor behavior; T5: production directory permission assertion. Evidence L146–L189 [evidence L146–L189]
- `normal_runtime_follows_live_room_toolset_for_bootstrap_dispatch` — MIXED (T4+T5); files provide bootstrap/guide data; dispatch rejects before live toolset enable and returns guide after enable ; T4: live-config/dispatch chain; T5: production reads of workspace files. Evidence L488–L532 [evidence L488–L532]
- `old_rule_ids_are_ignored_when_loading_config` — MIXED (T2+T5); loads legacy rule id, checks rule survives and serialized form omits id ; T2: legacy-field mapping; T5: config-file round trip. Evidence L1271–L1290 [evidence L1271–L1290]
- `path_policy_allows_write_root_and_blocks_readonly_write` — MIXED (T3+T5); preflights touch/du against configured write/read-only roots ; T3: command access classification/policy; T5: production resolution of real temporary roots. Evidence L996–L1036 [evidence L996–L1036]
- `path_root_remove_matches_expanded_equivalent_path` — MIXED (T3+T5); adds a root and removes it using `target/../target` ; T3: root mutation/equivalent-path behavior; T5: existing-root path normalization. Evidence L1341–L1362 [evidence L1341–L1362]
- `read_only_system_file_is_allowed` — MIXED (T3+T5); checks preflight allows `cat /proc/meminfo` and `df /` ; T3: read-only command/path policy; T5: checks against actual host paths. Evidence L983–L993; **REVIEW** retained: `/proc/meminfo` is platform/environment-specific and ungated [evidence L983–L993]
- `relative_path_arguments_are_resolved_from_working_directory` — MIXED (T3+T5); checks a real relative target fails from workspace root and succeeds from its containing cwd ; T3: cwd-relative argument resolution; T5: production path lookup. Evidence L1178–L1203 [evidence L1178–L1203]
- `room_mode_dispatches_bootstrap_manifest_and_read` — MIXED (T4+T5); dispatches manifest/read commands and checks parsed data plus exact guide content ; T4: hub command/response chain; T5: production fixture-file reads. Evidence L655–L702 [evidence L655–L702]
- `room_mode_dispatches_current_diary_command` — MIXED (T4+T5); dispatches active-diary request and checks missing diary result ; T4: hub command/response behavior; T5: lookup of the real absent repository resource. Evidence L726–L744 [evidence L726–L744]
- `room_timezone_defaults_and_can_be_overridden` — MIXED (T1+T2); checks fixed defaults and retained custom timezone/day-boundary values ; T1: default values; T2: config-field update/readback. Evidence L420–L427 [evidence L420–L427]
- `standalone_live_reload_applies_valid_mcp_map_and_rejects_invalid_candidate` — MIXED (T4+T5); checks valid MCP/limits/shell updates then atomic rejection of invalid transport ; T4: validation and live-state update chain; T5: actual candidate-config file reads. Evidence L1559–L1682 [evidence L1559–L1682]
- `standalone_live_reload_bootstraps_room_repository_before_maintenance_submit` — MIXED (T4+T5); reloads Room settings, checks live-root scaffold, submits maintenance, and reads created notebook ; T4: reload/repository-maintenance chain; T5: real repository/filesystem effects. Evidence L1685–L1756 [evidence L1685–L1756]
- `symlink_to_denied_path_is_rejected` — MIXED (T3+T5); on Unix creates denied-target symlink and checks preflight rejection ; T3: deny policy; T5: symlink/canonical-path lookup. Evidence L1207–L1235; **REVIEW** retained: assertion is compiled out off Unix [evidence L1207–L1235]
- `unknown_program_defaults_to_write_access` — MIXED (T3+T5); checks unknown program targeting read-only root is treated as a write ; T3: unknown-program access classification; T5: real path resolution. Evidence L1079–L1102 [evidence L1079–L1102]
- `working_directory_must_be_existing_writable_directory` — MIXED (T3+T5); checks valid directories and rejects file/missing/denied/outside roots ; T3: working-directory policy outcomes; T5: production filesystem metadata/canonicalization. Evidence L1128–L1173 [evidence L1128–L1173]
- `wp2_hub_live_reload_preserves_restart_fields` — MIXED (T4+T5); checks reload updates live allow rule but retains restart-required hub/workspace/path fields ; T4: hub reload behavior; T5: actual config-file read. Evidence L1507–L1555 [evidence L1507–L1555]
- `wp2_reload_preserves_live_path_policy_when_workspace_changes` — MIXED (T4+T5); reloads disk config and checks live workspace/path policy remain while allow rule updates ; T4: live reload/state update chain; T5: actual config-file read. Evidence L1437–L1504 [evidence L1437–L1504]

### `crates/agentic-gpt/src/runtime/supervisor.rs`
- `doctor_failure_surfaces_redacted_stdout_stderr_and_exit_code` — MIXED (T2+T5); runs failing script and checks exit/output diagnostics redact secrets ; T2: diagnostic redaction/projection; T5: subprocess and captured streams. Evidence L1216–L1246 [evidence L1216–L1246]
- `doctor_spawn_failure_preserves_os_error_kind` — MIXED (T2+T5); attempts nonexistent executable and checks mapped OS error details ; T2: spawn-error projection; T5: real OS spawn attempt. Evidence L1249–L1269 [evidence L1249–L1269]
- `fake_tunnel_verifies_args_environment_health_and_shutdown` — MIXED (T4+T5); runs fake tunnel/local health server and checks args, secret environment, readiness, termination ; T4: supervisor lifecycle/argument sequencing; T5: real child and socket. Evidence L1272–L1338 [evidence L1272–L1338]
- `health_probe_disables_configured_proxy` — MIXED (T3+T5); succeeds against local health server despite configured dead proxy ; T3: proxy-bypass client behavior; T5: local TCP health exchange. Evidence L1120–L1137 [evidence L1120–L1137]
- `health_url_accepts_only_local_http_endpoints` — MIXED (T3+T5); reads URL file and accepts loopback HTTP while rejecting HTTPS/non-local HTTP ; T3: endpoint validation; T5: actual health-file read. Evidence L1141–L1153 [evidence L1141–L1153]
- `secret_reference_normalizes_trailing_line_endings_and_rejects_controls` — MIXED (T3+T5); checks secret-file trimming/control rejection, empty-file failure, and plaintext rejection ; T3: secret-reference/content validation; T5: actual file reads. Evidence L1181–L1213 [evidence L1181–L1213]

### `crates/agentic-gpt/src/runtime/tunnel_distribution.rs`
- `bounded_local_download_handles_redirect_and_size_limit` — MIXED (T3+T5); checks local redirect download, size cap, and short-body failure ; T3: redirect/size/length handling; T5: local HTTP exchange and destination-file write. Evidence L934–L959 [evidence L934–L959]
- `cache_revalidates_archive_and_repairs_binary` — MIXED (T3+T5); installs artifact, corrupts it, checks repair state, reinstalls and reads replacement ; T3: cache validity/repair decisions; T5: actual cache-file operations. Evidence L830–L859 [evidence L830–L859]
- `executable_override_checks_permissions_and_optional_hash` — MIXED (T3+T5); checks file mode/hash and rejects mismatch/symlink ; T3: executable/hash policy; T5: real file metadata/hash/symlink. Evidence L770–L791; **REVIEW** retained: uses Unix symlink API without a Unix gate [evidence L770–L791]
- `manifest_and_platforms_are_pinned` — MIXED (T1+T2); checks platform map, manifest count, pinned URL/version and digests ; T1: fixed manifest content; T2: platform mapping. Evidence L711–L729 [evidence L711–L729]
- `offline_cache_and_auto_download_false_are_deterministic` — MIXED (T3+T5); resolves prepopulated cache offline and checks missing-cache result with downloads disabled ; T3: cache/download policy; T5: actual cache lookup. Evidence L864–L899 [evidence L864–L899]

### `crates/agentic-gpt/src/skills/skill_installs.rs`
- `commit_journal_recovery_rejects_paths_outside_skills_root` — MIXED (T3+T5); rejects an on-disk journal path outside the skills root and verifies the outside filesystem path remains [evidence L2167-L2192]
- `commit_journal_recovery_restores_archive_without_destroying_precommit_target` — MIXED (T3+T5); reconciles real journal/archive/target files and checks restore/removal plus preservation of the prior target [evidence L2112-L2164]
- `inline_install_is_persisted_and_completes_atomically` — MIXED (T3+T5); completes an install and checks actual installed files and persisted install record alongside result/status behavior [evidence L1948-L1982]
- `live_completion_drain_retries_after_event_store_becomes_writable` — MIXED (T4+T5); T4: chains install-manager completion and EventStore notification draining. T5: assertions depend on a real SQLite lock/rollback/retry and persisted pending/event state [evidence L2263-L2341]
- `terminal_install_notification_survives_recovery_until_acknowledged` — MIXED (T3+T5); writes/reloads a persisted install record, checks pending notification recovery, then checks acknowledgement updates the record [evidence L2194-L2260]

### `crates/agentic-gpt/src/skills/skills.rs`
- `activation_is_idempotent_and_state_file_only_saves_id_and_time` — MIXED (T3+T5); checks activation semantics against actual active-state file contents [evidence L995-L1024]
- `active_marks_deleted_skill_stale_without_summary_and_deactivate_cleans_it` — MIXED (T3+T5); activates a file-backed skill, deletes its file, then checks stale status and persisted deactivation [evidence L961-L992]
- `builtin_installer_is_default_active_and_deactivation_survives_restart` — MIXED (T3+T5); reads/searches built-in content, writes active-state changes, and verifies workspace shadowing is ignored [evidence L1027-L1112]
- `invalid_id_and_missing_skill_return_clear_errors` — MIXED (T3+T5); checks invalid-ID validation and missing-skill lookup against the workspace filesystem [evidence L1303-L1330]
- `list_reads_only_valid_first_level_skills_sorted_with_active` — MIXED (T3+T5); reads actual skill files and active state; checks first-level filtering, sorting, metadata, and built-in entry [evidence L860-L896]
- `read_rejects_resource_escape_directories_and_symlinks` — MIXED (T3+T5); filesystem traversal/directory/symlink cases return path errors; symlink setup is Unix-conditional [evidence L1159-L1186]
- `read_returns_frontmatter_package_summary_and_warnings` — MIXED (T3+T5); reads the skill file/package directory and checks parsed metadata, assets, and warnings [evidence L899-L923]
- `read_supports_bounded_utf8_and_base64_package_resources` — MIXED (T3+T5); reads actual text/binary files and checks encoding/content/size transformation [evidence L1115-L1156]
- `run_resolution_requires_active_workspace_executable_under_scripts` — MIXED (T3+T5); checks active/path/executable policy against a real script file and its filesystem permissions [evidence L1189-L1243]
- `run_waits_for_real_skill_process_and_returns_completed_output` — MIXED (T4+T5); Activates a file-backed skill, launches its real shell script, waits for terminal completion, and checks completed state plus decoded `skill-output\n` stdout. [evidence L1245–1300]
- `search_matches_id_frontmatter_tags_and_body_case_insensitively_with_limit` — MIXED (T3+T5); searches files created in the workspace and checks matching/limit/blank-query behavior [evidence L926-L958]

### `crates/agentic-gpt/src/storage/audit.rs`
- `audit_rotation_keeps_one_bounded_backup` — MIXED (T3+T5); exercises rotation policy and verifies backup/current audit file contents and size on disk [evidence L262-L271]
- `concurrent_audit_appends_remain_complete_json_lines` — MIXED (T3+T5); concurrent writers append to the audit file; checks 128 complete JSON lines [evidence L274-L297]
- `oversized_audit_record_is_rejected_without_truncating_existing_data` — MIXED (T3+T5); rejects an over-limit append and verifies pre-existing audit bytes remain unchanged [evidence L250-L259]

### `crates/agentic-gpt/src/storage/event_store.rs`
- `awaiting_completion_survives_recovery_and_source_cannot_be_forged` — MIXED (T3+T5); checks persisted completion recovery/settlement and source provenance plus forged-source rejection [evidence L1745-L1781]
- `completion_order_off_suppression_and_later_reads_preserve_original_settlement` — MIXED (T3+T5); persists internal completion/settlement decisions and checks ordering, suppression, and later reads do not alter them [evidence L1679-L1742]
- `concurrent_panels_expose_a_low_event_exactly_once` — MIXED (T3+T5); concurrent calls through two store handles contend over persisted exposure state; checks the event is shown once [evidence L1621-L1643]
- `concurrent_v1_openers_migrate_the_schema_once` — MIXED (T3+T5); concurrently opens a real temporary v1 SQLite database and asserts both opens succeed [evidence L1854-L1900]
- `duplicate_completion_cannot_reset_or_resurrect_a_terminal_event` — MIXED (T3+T5); checks persisted event status/message/severity after duplicate completion and retention cleanup/replay [evidence L2096-L2160]
- `event_database_rejects_a_different_configured_agent_identity` — MIXED (T3+T5); checks SQLite owner mismatch rejection and subsequent retrieval by the original owner [evidence L2044-L2068]
- `expiry_boundary_and_seven_day_history_retention_are_applied_before_reads` — MIXED (T3+T5); checks persisted event expiration and deletion/absence of old handled history from list/get [evidence L1461-L1515]
- `list_cursor_is_stable_and_bound_to_agent_scope_and_filters` — MIXED (T3+T5); pages SQLite-backed results and rejects cursor reuse under different agent/severity filters [evidence L1534-L1571]
- `list_defaults_to_twenty_pending_and_can_filter_handled_history` — MIXED (T3+T5); checks default paging and status filters against persisted event rows [evidence L1574-L1618]
- `list_summaries_count_unicode_scalars_and_preserve_full_message_on_get` — MIXED (T3+T5); inserts/list-reads SQLite events; verifies Unicode summary truncation and full message retrieval [evidence L1357-L1394]
- `low_one_medium_three_high_unlimited_exposure_and_hidden_counting` — MIXED (T3+T5); panel reads update persisted shown counts; checks severity caps and final counts [evidence L1433-L1458]
- `marking_is_idempotent_and_unknown_ids_remain_not_found` — MIXED (T3+T5); marks a persisted event twice and checks handled/not-found results [evidence L1518-L1531]
- `panel_orders_severity_caps_five_and_counts_hidden_pending_events` — MIXED (T3+T5); checks severity/age ordering and counts from persisted event rows, including a hidden row [evidence L1397-L1430]
- `remote_settlement_requires_matching_origin_and_stays_sticky` — MIXED (T3+T5); checks persisted origin/settlement rules, mismatch rejection, sticky decisions, and state after reopening the database [evidence L1903-L2041]
- `reopen_does_not_replace_existing_policy_snapshot_or_completion` — MIXED (T3+T5); persists registration/completion and checks a later registration does not replace the original policy snapshot [evidence L2071-L2093]
- `reopen_restores_events_and_internal_response_arbitration` — MIXED (T3+T5); closes/reopens the SQLite store, then checks event and internal response state after settlement [evidence L1646-L1676]
- `unrepresentable_rfc3339_ttl_is_rejected_before_event_insertion` — MIXED (T3+T5); rejects an unrepresentable expiry and verifies the DB list remains empty; also checks a representable expiry round-trips [evidence L1807-L1851]

### `crates/agentic-gpt/src/storage/private_state.rs`
- `identical_target_and_legacy_source_cleanup_is_self_healing` — MIXED (T3+T5); compares on-disk source/target content and checks cleanup of identical legacy data [evidence L534-L551]
- `migration_rejects_symlink_and_falls_back_to_legacy` — MIXED (T3+T5); **`cfg(unix)`**; rejects a symlink in the actual source tree and verifies fallback path/warning [evidence L611-L638]
- `migration_rejects_target_symlink_and_preserves_legacy_source` — MIXED (T3+T5); **`cfg(unix)`**; rejects an on-disk destination symlink and verifies fallback data/outside file contents [evidence L581-L608]
- `prepare_is_idempotent_after_successful_migration` — MIXED (T3+T5); repeats filesystem migration/preparation and checks stable paths and preserved file bytes [evidence L515-L531]
- `prepare_migrates_known_legacy_state_and_cleans_empty_root` — MIXED (T3+T5); moves actual legacy state files/directories into the private root and removes the old tree [evidence L474-L512]
- `target_conflict_keeps_target_and_retains_legacy_source` — MIXED (T3+T5); verifies real conflicting files remain and that a warning is returned [evidence L554-L578]

### `crates/agentic-gpt/src/storage/process_history.rs`
- `admitted_metadata_reserves_terminal_error_space` — MIXED (T3+T5); persists admission/running/terminal snapshots and verifies large terminal error fields survive DB retrieval [evidence L1388-L1428]
- `early_terminal_evidence_survives_full_snapshot_and_restart_recovery` — MIXED (T3+T5); persists early completion evidence, later snapshot, and restart recovery; checks recovered event details [evidence L1300-L1367]
- `oversized_admission_metadata_rejects_the_entire_batch` — MIXED (T3+T5); oversized admission is rejected and SQLite queries confirm neither batch row was stored [evidence L1370-L1385]
- `process_list_filters_and_cursor_rejects_malformed_values` — MIXED (T3+T5); checks filtered SQLite paging/cursor creation and malformed-cursor rejection [evidence L1431-L1455]
- `terminal_event_outbox_survives_reopen_until_acknowledged` — MIXED (T3+T5); checks a completion row survives DB reopen and is removed from pending state on acknowledgement [evidence L1267-L1297]
- `terminal_output_and_offsets_survive_reopen_atomically` — MIXED (T3+T5); persists terminal state/output in SQLite, attempts a replacement, and checks original state/bytes/offsets/detail after reopen [evidence L1226-L1264]

### `crates/agentic-gpt/src/storage/transport_ledger.rs`
- `claim_concurrency_allows_one_started_owner` — MIXED (T3+T5); concurrent claims append/read the real ledger and yield exactly one owner [evidence L923-L968]
- `compaction_collapses_completed_transition_history_and_retains_evidence` — MIXED (T3+T5); compacts actual JSONL, verifies retained results/evidence and reads the recovery backup [evidence L1152-L1278]
- `conflicting_completion_preserves_canonical_result_and_evidence` — MIXED (T3+T5); checks canonical result and conflict evidence in actual ledger content [evidence L993-L1040]
- `conflicting_identity_is_rejected_with_evidence` — MIXED (T3+T5); rejects conflicting identity and checks the mismatch record in the ledger file [evidence L1043-L1061]
- `malformed_transport_identity_and_owner_rows_fail_closed` — MIXED (T3+T5); malformed ledger rows fail parsing; checks unchanged ledger bytes and recovery evidence on disk [evidence L1824-L1861]
- `missing_identity_is_rejected_without_fabricating_hash` — MIXED (T3+T5); rejects empty identity/hash and checks no ledger file was created [evidence L1130-L1149]
- `retired_process_commands_survive_mixed_ledger_recovery_and_compaction` — MIXED (T3+T4+T5); T3: recovers/compacts legacy/current command records and preserves results. T4: builds a response using `EventStore`. T5: reads/writes the ledger and recovery backup plus the EventStore database [evidence L1281-L1489]
- `stored_legacy_argv_commands_keep_identity_results_and_current_recovery` — MIXED (T3+T4+T5); T3: checks legacy/current command recovery, validation, and compaction. T4: forms a completed response through `EventStore`. T5: asserts ledger/backup state and uses the EventStore database [evidence L1492-L1821]
- `torn_corruption_fails_closed_and_deduplicates_recovery_evidence` — MIXED (T3+T5); corrupt on-disk JSONL blocks reads/acceptance and creates one recovery-evidence file entry [evidence L971-L990]
- `unowned_legacy_is_blocked_including_explicit_agent_target` — MIXED (T3+T5); writes legacy ledger records, rejects adoption, and checks unowned agent state remains unchanged [evidence L1064-L1127]

### `crates/agentic-gpt/tests/config_cli.rs`
- `bare_non_tty_init_requires_explicit_non_interactive_mode` — MIXED (T4+T5); T4 non-TTY init guard/error guidance; T5 real CLI process status/output and asserted absence of the config file. [evidence L700–734]
- `config_help_is_fully_localized_without_changing_tokens` — MIXED (T2+T5); T2 localized help/token mapping; T5 real CLI subprocess stdout/status. [evidence L14–42]
- `config_keys_http_mcp_section_exposes_editable_contract` — MIXED (T2+T5); T2 HTTP MCP registry metadata-to-JSON mapping; T5 real CLI subprocess output/status. [evidence L905–955]
- `config_keys_json_lists_registry` — MIXED (T2+T5); T2 registry metadata-to-JSON mapping; T5 real CLI subprocess output/status. [evidence L371–428]
- `every_visible_command_has_help` — MIXED (T2+T5); T2 visible command metadata/help descriptions; T5 real CLI subprocess outputs/statuses for the listed commands. [evidence L44–122]
- `invalid_mode_is_localized_without_changing_valid_tokens` — MIXED (T4+T5); T4 parser-to-localized-error behavior; T5 real CLI exit status and stderr. [evidence L174–200]
- `invalid_owned_parse_errors_keep_stream_and_tokens` — MIXED (T4+T5); T4 parser/error-rendering behavior for three failures; T5 real CLI exit status and stderr. [evidence L202–249]
- `language_auto_detection_obeys_locale_precedence` — MIXED (T2+T5); T2 locale/environment precedence and explicit override; T5 real CLI subprocess outputs/status. [evidence L140–172]
- `language_flag_is_equivalent_before_and_after_subcommand` — MIXED (T2+T5); T2 argv flag-placement/selection behavior; T5 real CLI subprocess outputs/status. [evidence L124–138]
- `plaintext_tunnel_api_key_is_rejected_without_writing_config` — MIXED (T3+T5); T3 plaintext-key rejection/no-leak behavior; T5 real CLI process output/status and asserted config-file absence. [evidence L797–829]

### `crates/agentic-gpt/tests/standalone_supervisor.rs`
- `supervised_journal_mode_omits_agentic_inner_timestamp` — MIXED (T2+T5); T2: verifies forwarded child-log timestamp stripping and journal-line rendering; T5: runs the real supervisor/worker against temporary config/files, fake tunnel, and local health socket, then checks worker output and forwarded stderr. [evidence L53–58; helper L161–246]

## Lower-tier cases worth retaining

- `crates/agentic-gpt/src/browser/browser_distribution_tests.rs` — `pinned_repository_key_has_exact_fingerprint` (T1): freezes a security trust-root fingerprint; `authenticated_inrelease_metadata_selects_exact_target` (T2): maps authenticated metadata to the exact target; `cleartext_signature_verifies_and_tampering_fails` (T3): exercises signature verification and tamper rejection.
- `crates/agentic-gpt/src/ingress/stdio_server_tests.rs` — `normal_and_room_tool_sets_follow_fixed_surface_contract` (MIXED T1+T4): protects exact API surface/schema; `process_read_preserves_raw_byte_offsets_and_utf8_output` (MIXED T2+T4+T5): protects byte-offset and UTF-8/base64 projection.
- `crates/agentic-gpt/src/mcp/mcp_tests.rs::server_config_validation_is_complete_and_typed` (T3): directly validates valid and invalid typed MCP config variants.
- `crates/agentic-gpt/src/storage/event_store.rs::list_summaries_count_unicode_scalars_and_preserve_full_message_on_get` (MIXED T3+T5): checks Unicode-scalar summary logic and full-message retrieval from SQLite.
- `crates/agentic-gpt/src/files/file_ops.rs::search_rejects_invalid_patterns_and_enforces_bounds` (T3): exercises independent pattern/bounds logic.
- `crates/agentic-gpt/src/runtime/main_tests.rs::cli_version_uses_crate_version` (T1+T2): protects fixed version text and CLI rendering.

## Concrete low-value T4/T5 cases

- `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::room_profile_dispatches_room_memory_tools` (MIXED T4+T5): asserts field presence/types only, not Room data or semantic behavior.
- `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::compact_mcp_skills_and_tmux_adapters_preserve_result_envelopes` (MIXED T4+T5): accepts either result or error, so both tmux adapters may fail while the test passes.
- `crates/agentic-gpt/src/runtime/tunnel_distribution.rs::artifact_lock_serializes_concurrent_installers` (T5): checks eventual acquisition after sleep/release but never asserts that the second waiter is blocked while the first lock is held.
- `crates/agentic-gpt/src/files/file_ops.rs::absent_commit_uses_no_replace_and_overwrite_preserves_permissions` (T5): exercises `hard_link`/`rename` directly, bypassing the production commit path implied by the name.
- `crates/agentic-gpt/src/process/managed.rs::shell_eval_preserves_dash_leading_command_text` (T5): checks only Failed/127 for `--`; no output proves the leading command text was preserved.
- `crates/agentic-gpt/tests/config_cli.rs::config_import_compatible_round_trip_preserves_http_mcp_fields` (T5): expected import fails without a TTY; later `show` reads init-generated source rather than imported config.

## Review-needed cases

- `crates/agentic-gpt/src/runtime/main_tests.rs::read_only_system_file_is_allowed`: reads ungated `/proc/meminfo`, which is platform/environment-specific.
- `crates/agentic-gpt/src/runtime/main_tests.rs::symlink_to_denied_path_is_rejected`: its assertion is compiled out on non-Unix.
- `crates/agentic-gpt/src/runtime/tunnel_distribution.rs::executable_override_checks_permissions_and_optional_hash`: uses Unix symlink APIs without a Unix cfg gate.
