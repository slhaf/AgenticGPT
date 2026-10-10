# Progress Log

## Session: 2026-10-08

### Current Status
- **Phase:** 8 - Verification & Delivery (complete; cleanup commit `519fda3`, planning closeout commit pending)
- **Started:** 2026-10-08

### Actions Taken
- Created the task-specific planning record and recorded scope, counting unit, audit boundaries, verification method, and ambiguity handling.
- Confirmed `.codegraph/` exists and used CodeGraph before code search.
- Mapped five Cargo workspace crates and the shared Gradle targets (JVM, JS, Wasm, Android host); found four Kotlin test files so far.
- `cargo test --workspace -- --list` completed with 827 Rust declarations. Tier totals and mixed-case indexes reconcile; the case/name/behavior index for single-tier Rust declarations is being persisted as a follow-up.
- `./gradlew :shared:tasks --all` completed; static source-set review reconciles to 7 Kotlin declarations. All 7 Kotlin case names and behaviors are now recorded in `kotlin-test-cases.md`; no Kotlin suite was executed.

### Test Results
| Test | Expected | Actual | Status |
|------|----------|--------|--------|
| Rust test discovery (`cargo test --workspace -- --list`) | List cases without executing tests | Command completed, listing returned | Passed |
| Gradle task discovery (`./gradlew :shared:tasks --all`) | List enabled test tasks | Command completed, task list returned | Passed |
- Checked Rust test attributes across the five crates: no feature-gated test attributes, and one ignored/manual test; Cargo list includes it.
- Kotlin audit reconciled all four Gradle-included modules and confirmed seven unique declarations; no Kotlin test suite has been run.
- Reviewed the ignored real-package Rust test: it materializes a verified official browser `.deb` into a cache and checks expected artifacts; potential tier-5 value is recorded for human review.
- Intermediate hub subset count was corrected after reviewing six `notify.rs` cases (not seven); final full-crate count is 138 within the 173-case non-`agentic-gpt` total above.
- The four non-`agentic-gpt` Rust crates reconcile to Cargo's 173 cases: T1/T2/T3/T4/T5/MIXED = 1/42/36/35/49/10. The Hub split was recomputed from the exhaustive case inventory; earlier aggregate classifications were superseded where individual SQL/resource assertions required T5.
- The other-crate technical classification contains 10 MIXED cases; the six cases containing T4/T5 remain in the mixed review catalog, while four Protocol T2+T3 cases are routed to `review-candidates.md`.
- Exhaustive other-crate case review produced 163 single-tier rows plus 10 mixed rows; the four crate counts reconcile to Hub 138, Protocol 23, Apply-patch 2, Browser-host 10.
- Independent reviewer agreed with `hub_info_reports_safe_runtime_summary` as MIXED (T4+T5), because the asserted count depends on its actual in-memory SQLite registry read.
- Hub storage case inventory: `runs.rs` has 11 T5 and one MIXED; `event_feedback.rs` has 21 single-tier T5, two single-tier T3, and three MIXED cases. This replaces earlier partial event-feedback counts.
- Adopted one explicit T5/MIXED decision rule: actual runtime resources count only when assertions depend on them; fixtures, async wrappers, and setup/cleanup alone do not promote a test. The `agentic-gpt` audit is being normalized to this boundary.
- `agentic-gpt` empty-store/shape-only correction: `event_api_business_errors_keep_their_code_and_include_the_panel`, `in_process_stdio_initialize_list_and_call`, and `in_process_room_stdio_initialize_list_and_call` are T4; the first two read an initially empty store, and the Room test asserts only success plus `daily.is_object()` (the handler produces that object even when no Room files exist). `browser_repl_event_decoration_preserves_inner_result_channels` remains MIXED T2+T4 without T5. `denied_process_batch_creates_no_processes` is MIXED T4+T5: after a denied batch it asserts no process rows through `process.list`, whose production path queries SQLite; this checks a negative persistence invariant, not merely an empty-store read.
- Main Rust audit initially classified `denied_process_batch_creates_no_processes` as T4-only. Source review found `process.list` queries `ProcessHistoryStore` and the test asserts no persisted rows after denial; final tier is MIXED T4+T5. `event_api_business_errors_keep_their_code_and_include_the_panel`, `in_process_stdio_initialize_list_and_call`, and `in_process_room_stdio_initialize_list_and_call` are T4: the first two check empty-store/error or response behavior, while Room only asserts success and the constructed `daily` object shape (even with no Room files). `browser_repl_event_decoration_preserves_inner_result_channels` remains MIXED T2+T4, without T5.
- Protocol case-label recount resolved the discrepancy: detailed labels total T2 14 / T3 5 / MIXED 4 in its 23 cases; the earlier 13/6 aggregate was stale and is excluded from workspace totals.
- Final `agentic-gpt` slice counts: browser/config 202 (3/28/50/24/3/94), ingress/operations 105 (7/10/34/8/3/43), MCP/room 68 (0/3/5/12/12/36), storage/skills 69 (0/2/7/0/2/58), UI/support/tmux/integration 62 (1/14/8/4/24/11), runtime/files/process 148 (0/16/30/11/10/81); in T1/T2/T3/T4/T5/MIXED order. Total: 654 (11/73/134/59/54/323).
- Final technical totals remain Rust 827 (T1/T2/T3/T4/T5/MIXED = 12/115/170/94/103/333); Kotlin 7 (3/0/4/0/0/0); combined 834. Review routing does not alter technical tier totals.
- The mixed-review catalogs now contain 303 cases with T4/T5: 297 `agentic-gpt` and 6 other Rust. The 30 lower-only mixed cases are individually recorded in `review-candidates.md`: 17 retention, 6 cleanup, 7 user-judgment candidates.
- 在低档清理审查范围内（304 个纯 T1–T3 加 30 个只含 T1–T3 的 MIXED），当前是 31 个保留候选、296 个清理候选、7 个待用户判断。另有 T4/T5 低价值候选单独列出，不计入此清理数量；以上均未删除测试。
- ID-by-ID static check passed: all 30 lower-only mixed case names appear exactly once in the correct disposition section (17 retain, 6 cleanup, 7 user judgment), none remain in either mixed catalog, and the catalogs contain 297+6=303 T4/T5 mixed cases.
- No engineering-consultant consultation was used for this phase; the seven uncertain value judgments are documented for the user. No Rust or Kotlin tests were run, and no source/test files were modified.

## Session: Test Cleanup Execution
- User authorized execution of the explicit `cleanup-manifest.md` scope; no test or implementation files were modified before that authorization.
- Read the development guide and planning workflow. Repository was clean at HEAD `8232dcf35953bd6db3b6f0a268f0532cedab6a43`.
- Recounted the manifest: exactly 306 deletion rows and 8 merge/simplify rows. This supersedes the stale 299 deletion count in the inventory handoff.
- CodeGraph exploration returned partial symbols (80 across five files); exact per-file target matching remains required before each edit.
- Planning now tracks Phase 7 cleanup and Phase 8 verification/delivery. No source edits or test runs yet.
- Began Phase 7 using five disjoint file-ownership batches: Hub; supporting Rust crates plus Kotlin; Agent browser; Agent config/files; remaining Agent modules. Each worker is restricted to exact manifest entries and source-only edits; they do not run tests.
- Supporting Rust/Kotlin batch returned: 26 listed declarations removed, plus `process_read_defaults_view_and_bounds_wait` merged into `process_read_rejects_explicit_budgets_outside_shared_limits` with default-view, wait-boundary, no-cursor, and max-bytes boundary assertions retained. No target mismatch; this batch ran no checks.
- Batch results reconcile exactly to 306 listed deletions: Hub 53, supporting Rust/Kotlin 26, Agent browser 43, Agent config/files 49, remaining Agent 135. All reports had zero mismatches or skips.
- Eight special targets: five merged, two simplified in place, one retained unchanged because the independent pinned-key fingerprint assertion is not exercised by available trusted-repository signature fixtures. Unique behavior assertions were reported preserved; source review and verification remain.
- First `cargo fmt --all -- --check` exposed one missing closing brace at the end of the Protocol `tmux_tests` module after removing its final test, plus excess blank lines left by declaration deletions. Restored the module delimiter; format cleanup is next.
- CI `cargo check --workspace` passed. Strict Clippy identified test-only imports/helpers and an empty `#[cfg(test)]` module made unused by the manifest deletions in Protocol; removed only those now-dead test scaffolds. Re-run formatting and Clippy before continuing.
- After dead Protocol scaffolding removal, strict Clippy reached all crates and reported remaining unused imports/test helpers caused by removed declarations (including Hub and Agent test modules), plus two private production helpers with no remaining callers (`room/bootstrap.rs::build_resource`, `runtime/supervisor.rs::backoff_delay`). Do not remove these production helpers without confirming they are not required by production behavior; inspect references before deciding.
- Review of two Clippy-flagged helpers confirmed both have `#[cfg(test)]`; `build_resource` and `backoff_delay` are test-only and now have no callers after the declared case deletions. The configured LSP reference tool was unavailable, so exact-symbol source search was used.
- Lint-only cleanup is delegated in three disjoint slices (Hub; Agent browser/config; remaining Agent); workers may remove only imports and dead `#[cfg(test)]` support code, without running verification commands.
- Hub Clippy cleanup completed: removed only now-empty test modules, unused test imports, and test-only helpers in nine assigned Hub files. No tests/build/fmt/lint run during this batch.
- Agent remaining-module Clippy cleanup completed: removed unused test imports and three unreferenced test-only helpers (`build_resource`, `backoff_delay`, `mark_zip_entry_as_unix_symlink`); preserved production helpers. No tests/build/fmt/lint run during this batch.
- Agent browser/config Clippy cleanup completed: removed unused test imports, three unused `Behavior` variants/match arms, and the unreferenced `explicit_config` helper. No tests/build/fmt/lint run during this batch.
- Strict Clippy after the first lint cleanup exposed accidentally removed imports still used by retained tests (Hub `Capabilities`/room protocol types and the browser pinned-key `KeyDetails` trait), plus other unused imports not yet handled. Sent exact compiler diagnostics to the three file owners for narrow import corrections; no test run yet.
- Corrected the retained-test import regressions from the second Clippy run (including `build_app_state`/`reload_live_config_once`, required setup imports, and Hub/browser types); retained the bootstrap success match arm and removed only variants no longer constructed. `cargo fmt --all -- --check` passed after these corrections.
- Full workspace test run initially failed only `bootstrap_escapes_import_path_and_initializes_browser_in_order`: the simplified test's fake `Behavior::Success` had been routed through `expected_result()` with `is_error = true`. Restored the original success response (`CallToolResult::default()`), kept `Preserve` on `expected_result()`, and the targeted test passed (1 passed, 423 filtered across 5 suites).
- The first full workspace run then reached the Hub batch test and found its optional integer schema expected as a scalar; actual schema is `["integer", "null"]`. Updated the public-schema assertion and the focused Hub test passed (1 passed, 84 filtered).
- Final pinned-toolchain Rust CI checks passed: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` (519 passed, 1 ignored). The full test run includes 13 test suites.
- The separate latest-stable Clippy workflow passed: `cargo +stable clippy --workspace --all-targets -- -D warnings`. CI binary build passed for Agent and Hub.
- Live contract parity passed, including OpenAPI/schema validation and real Agent/Hub interactions; one supplemental recovery interleaving remains explicitly an inference.
- Android host Gradle task remains unverified: with `ANDROID_HOME` set, the SDK requires acceptance of Build-Tools 36 and Android 36 licenses. No license acceptance was performed.
- Combined JVM/JS/Wasm Gradle invocation compiled JS and Wasm test sources, and `:shared:jvmTest` completed, but failed at `:kotlinStoreYarnLock` because `console/build/js/yarn.lock` was missing during concurrent Yarn setup. The user directed not to run every Kotlin target. An isolated `:shared:jvmTest` then passed in 2 seconds; JS/Wasm test runners were not completed. Remaining Kotlin changes were visually reviewed, not represented as executed tests.
- Removed the generated untracked `console/kotlin-js-store/` artifact after Gradle finished. Current manifest records 306 exact deletions and all 8 B outcomes; no mismatch or skip.
- Final Rust recheck passed: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace -- --list` (520 listed cases).
### Errors
| Error | Resolution |
|-------|------------|

