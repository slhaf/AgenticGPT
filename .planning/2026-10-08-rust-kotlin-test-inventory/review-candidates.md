# Test Cases for Retention or Cleanup Review

Inventory only. These are candidates for the user's later decision; no test or implementation edits are proposed here. Tier labels describe asserted behavior, and MIXED cases count once in the separate mixed-case index.

## Lower-tier behavior worth retaining

| Case | Tier(s) | Behavior worth protecting |
|---|---|---|
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::pinned_repository_key_has_exact_fingerprint` | T1 | Freezes the trusted repository key fingerprint. |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::authenticated_inrelease_metadata_selects_exact_target` | T2 | Maps authenticated platform metadata to the exact package target. |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::cleartext_signature_verifies_and_tampering_fails` | T3 | Accepts valid signed metadata and rejects tampering. |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::fixed_repository_url_rejects_untrusted_path_forms` | T3 | Rejects traversal, foreign host, query, and fragment URL forms. |
| `crates/agentic-gpt/src/config/config.rs::process_response_bytes_defaults_validates_bounds_and_rejects_invalid_values` | T3 | Locks default, allowed bounds, and invalid value rejection for an agent-facing output budget. |
| `crates/agentic-gpt/src/config/config.rs::explicit_import_clears_plaintext_tunnel_secret_and_reports_it` | MIXED (T2+T3+T5) | Checks secret-clearing policy, warning, redaction, and actual import behavior. |
| `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::process_read_preserves_raw_byte_offsets_and_utf8_output` | MIXED (T2+T4+T5) | Protects byte cursor/offset and UTF-8/base64 projection through a real command-output path. |
| `crates/agentic-gpt/src/mcp/mcp_tests.rs::server_config_validation_is_complete_and_typed` | T3 | Covers typed MCP configuration validation branches and stable errors. |
| `crates/agentic-gpt/src/storage/event_store.rs::list_summaries_count_unicode_scalars_and_preserve_full_message_on_get` | MIXED (T3+T5) | Protects Unicode summary truncation and full-message retrieval from persisted events. |
| `crates/agentic-gpt/src/storage/process_history.rs::oversized_admission_metadata_rejects_the_entire_batch` | MIXED (T3+T5) | Verifies size rejection is atomic and no partial SQLite batch rows are stored. |
| `crates/agentic-gpt/src/files/file_ops.rs::search_rejects_invalid_patterns_and_enforces_bounds` | T3 | Exercises malformed-pattern and result/context bound errors. |
| `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs::mcp_batch_descriptor_freezes_bounds_and_side_effect_annotations` | T2 | Protects public schema bounds and side-effect metadata. |
| `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs::coordinator_rejects_hidden_execution_tools_before_dispatch` | T3 | Verifies profile gating and that hidden tools do not dispatch. |
| `crates/agentic-gpt-protocol/src/lib.rs::managed_mcp_batch_defaults_and_bounds_are_frozen` | MIXED (T2+T3) | Protects public batch wire fields plus defaults, clamping, and bounds. |
| `crates/agentic-gpt-protocol/src/process.rs::process_read_rejects_explicit_budgets_outside_shared_limits` | T3 | Keeps process-read budget validation aligned with shared limits. |
| `crates/agentic-browser-host/src/lib.rs::zero_malformed_and_non_utf8_payloads_are_errors` | T2 | Covers invalid framed payload decoding at the wire boundary. |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::overdue_restore_claim_is_trigger_once_at_due_boundary` | T3 | Protects due-time triggering and one-shot claim behavior. |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::future_restore_schedules_until_snoozed_due_boundary` | T3 | Protects Schedule-before-due and Trigger-at-due boundaries. |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::degraded_item_stays_pending_until_due_and_can_be_terminal` | T3 | Protects degraded/pending versus terminal-state behavior. |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::terminal_transition_clears_actions_and_rejects_duplicate_transition` | T3 | Protects terminal action clearing and duplicate-transition rejection. |

## Low-value or cleanup/strengthening candidates

| Case | Tier(s) | Concern |
|---|---|---|
| `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::room_profile_dispatches_room_memory_tools` | MIXED (T4+T5) | Checks JSON field presence/types but not returned Room data or operation semantics. |
| `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::compact_mcp_skills_and_tmux_adapters_preserve_result_envelopes` | MIXED (T4+T5) | Accepts either result or error for both tmux calls, so both adapters may fail and pass. |
| `crates/agentic-gpt/src/runtime/tunnel_distribution.rs::artifact_lock_serializes_concurrent_installers` | T5 | Sleeps, releases the first lock, then checks eventual acquisition; never asserts the waiter was blocked. |
| `crates/agentic-gpt/src/files/file_ops.rs::absent_commit_uses_no_replace_and_overwrite_preserves_permissions` | T5 | Calls `fs::hard_link`/`fs::rename` directly instead of exercising the production commit path. |
| `crates/agentic-gpt/src/process/managed.rs::shell_eval_preserves_dash_leading_command_text` | T5 | Checks only exit 127/Failed and no captured command/output proving the leading text was preserved. |
| `crates/agentic-gpt/tests/config_cli.rs::config_import_compatible_round_trip_preserves_http_mcp_fields` | T5 | Import is expected to fail without a TTY; subsequent `show` observes the source generated by `init`, not an imported config. |
| `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs::every_advertised_tool_is_accepted_by_apps_dispatcher` | T4 | Only checks “not unknown tool”; arbitrary argument/runtime failures are accepted. |
| `crates/agentic-browser-host/src/lib.rs::identifier_free_notifications_forward_and_broadcast` | T4 | Tests pure forwarding/broadcast through in-memory buffers. |
| `crates/agentic-apply-patch/src/lib.rs::parses_patch` | T2 | Parses add/delete/update forms but asserts only three hunks, not their individual content/type. |
| Kotlin `SharedCommonTest.example`, `SharedLogicDesktopTest.example`, `SharedLogicAndroidHostTest.example` | T1 | Each asserts only the fixed arithmetic expression `1 + 2 == 3`. |

The single ignored browser package materialization case (`crates/agentic-gpt/src/browser/browser_distribution_tests.rs::real_package_materializes_from_explicit_fixture_path`) is not marked low-value: it requires verified release-package inputs and checks real package materialization and runtime layout. Three platform-sensitive Rust declarations require portability review; see `findings.md`.
