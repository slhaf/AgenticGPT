# Progress Log

## Session: 2026-10-08

### Current Status
- **Phase:** 5 - Exhaustive Case Index
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
- The other-crate case inventory records all 10 MIXED cases, lower-tier behaviors worth retaining, and selected low-value candidates with concrete reasons.
- Exhaustive other-crate case review produced 163 single-tier rows plus 10 mixed rows; the four crate counts reconcile to Hub 138, Protocol 23, Apply-patch 2, Browser-host 10.
- Independent reviewer agreed with `hub_info_reports_safe_runtime_summary` as MIXED (T4+T5), because the asserted count depends on its actual in-memory SQLite registry read.
- Hub storage case inventory: `runs.rs` has 11 T5 and one MIXED; `event_feedback.rs` has 21 single-tier T5, two single-tier T3, and three MIXED cases. This replaces earlier partial event-feedback counts.
- Adopted one explicit T5/MIXED decision rule: actual runtime resources count only when assertions depend on them; fixtures, async wrappers, and setup/cleanup alone do not promote a test. The `agentic-gpt` audit is being normalized to this boundary.
- `agentic-gpt` empty-store/shape-only correction: `event_api_business_errors_keep_their_code_and_include_the_panel`, `in_process_stdio_initialize_list_and_call`, and `in_process_room_stdio_initialize_list_and_call` are T4; the first two read an initially empty store, and the Room test asserts only success plus `daily.is_object()` (the handler produces that object even when no Room files exist). `browser_repl_event_decoration_preserves_inner_result_channels` remains MIXED T2+T4 without T5. `denied_process_batch_creates_no_processes` is MIXED T4+T5: after a denied batch it asserts no process rows through `process.list`, whose production path queries SQLite; this checks a negative persistence invariant, not merely an empty-store read.
- Main Rust audit initially classified `denied_process_batch_creates_no_processes` as T4-only. Source review found `process.list` queries `ProcessHistoryStore` and the test asserts no persisted rows after denial; final tier is MIXED T4+T5. `event_api_business_errors_keep_their_code_and_include_the_panel`, `in_process_stdio_initialize_list_and_call`, and `in_process_room_stdio_initialize_list_and_call` are T4: the first two check empty-store/error or response behavior, while Room only asserts success and the constructed `daily` object shape (even with no Room files). `browser_repl_event_decoration_preserves_inner_result_channels` remains MIXED T2+T4, without T5.
- Protocol case-label recount resolved the discrepancy: detailed labels total T2 14 / T3 5 / MIXED 4 in its 23 cases; the earlier 13/6 aggregate was stale and is excluded from workspace totals.
- Final `agentic-gpt` slice counts: browser/config 202 (3/28/50/24/3/94), ingress/operations 105 (7/10/34/8/3/43), MCP/room 68 (0/3/5/12/12/36), storage/skills 69 (0/2/7/0/2/58), UI/support/tmux/integration 62 (1/14/8/4/24/11), runtime/files/process 148 (0/16/30/11/10/81); in T1/T2/T3/T4/T5/MIXED order. Total: 654 (11/73/134/59/54/323).
- Final reconciled totals: Rust 827 (T1/T2/T3/T4/T5/MIXED = 12/115/170/94/103/333); Kotlin 7 (3/0/4/0/0/0); combined 834. Mixed indexes contain 323 `agentic-gpt` and 10 other Rust cases.
- Review candidates are recorded in `findings.md` and `review-candidates.md`. Only discovery commands were run; no Rust or Kotlin test suite was executed.
- Exhaustive case-index follow-up complete: the four Rust index files contain 827 case entries; a multiset comparison against `cargo test --workspace -- --list` found no missing or extra test names. The two repeated Rust function names match declarations in separate crate source paths. The Kotlin index lists seven declarations across common/JVM/androidHost source sets; common cases are counted once. No Rust or Kotlin test suite was executed.
### Errors
| Error | Resolution |
|-------|------------|

