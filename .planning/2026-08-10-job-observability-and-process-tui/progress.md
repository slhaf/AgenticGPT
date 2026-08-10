# Progress Log

## Session: 2026-08-10

### Current Status
- **Phase:** Phase 3A complete; next is Phase 3B — Durable Job History Store
- **Started:** 2026-08-10
- **Implementation:** Phase 3A complete; stopped at the protocol/domain boundary before SQLite/runtime integration

### Actions Taken
- Recalled the previously chosen next Agentic sequence: job response slimming + `job.get waitOnly`, then Process TUI, followed later by optional toolsets and Terminal.
- Inspected existing readable-looking `id` fields in `mcp.batch` and `file.batch`.
- Traced `mcp.batch.id` into `ManagedMcpSpec.batch_call_id`, retained `JobInfo.batchCallId`, batch responses, and audit metadata.
- Confirmed `file.batch.id` is local operation correlation metadata rather than Managed Job identity.
- Clarified the new requirement: the human-readable value is primarily a reusable cross-call workstream/category marker for distinguishing concurrent collaborators/tasks and for later Process TUI grouping.
- Initialized scoped plan `.planning/2026-08-10-job-observability-and-process-tui/` and recorded the above findings.
- Froze initial grouping lifecycle semantics: pure human-readable string key; equal strings share a logical group within the current runtime; first use auto-registers; 30 minutes without new grouped calls expires the active registration; later reuse reactivates the same string.
- Initially simplified grouping to in-memory-only, then revised after considering Process TUI execution history: durable Job history and restart recovery are now in scope, while a separate durable group registry remains unnecessary.
- Chose Job-level grouping: each Job carries its own human-readable `group`; active groups can be derived from grouped Jobs seen within the 30-minute TTL.
- Inspected current persistence boundaries: local Jobs are in-memory and bounded (24h terminal retention / max terminal count), existing audit JSONL lacks full Job detail/output, and shared protocol already defines `unknown_after_restart` for recovery semantics.
- Evaluated durable history storage: preferred SQLite over JSON/JSONL; repository already uses bundled `rusqlite` in Hub. Confirmed SQLite itself is not a general compression layer; current Job payload bounds (64 KiB stdout/stderr tails, 512 KiB MCP full-result ceiling) make an uncompressed first version reasonable.
- Froze terminal-history retention at 30 days plus an approximately 512 MiB soft database cap, with oldest terminal Jobs pruned first and non-terminal Jobs excluded from retention pruning.
- Inspected current `job.get`: it accepts only `jobId + waitSeconds`, polls every 20 ms until terminal/timeout, and always returns full `JobDetail` even when the Job remains active.
- Froze the main `waitOnly` contract: it is a `job.get` parameter that suppresses repeated detail only during a pure non-terminal wait; active timeout returns minimal Job identity/state/elapsed time with no redundant helper flags, while terminal completion (or an already-terminal Job) returns the normal final result in the same call.
- Inspected current `JobResponse`/`JobDetail`/`JobInfo` layering and found substantial duplication (`status` + nested `state`, outer + nested `jobId`, helper flags, timestamps, command metadata, default lifecycle fields). Chose to separate rich persisted Job records from slim tool-facing response types.
- Corrected timing direction: current `startedAt` is populated at Job creation and never updated on actual start, so it cannot represent true execution duration. Plan now makes `startedAt` optional/actual-start and exposes derived `elapsedMs` for active Jobs plus `durationMs` for terminal Jobs.
- User confirmed the slim-response direction matches the fields that felt noisy while reading real Job logs; treat rich-record vs slim-tool-response separation as frozen pending implementation/refine details.
- Confirmed slimming affects the whole Managed Job surface: process/skill/MCP single and batch responses reuse rich Job shapes, and `job.cancel` also returns full `JobDetail`; non-Job tools remain outside this refactor.
- Froze `job.list` summary rows to `jobId/group/kind/state/createdAt/startedAt?/finishedAt?`; `kind` is required because Process TUI will mix process/skill/MCP Jobs and choose per-kind renderers, while `group` partitions them into columns/tabs.
- Froze optional `group` propagation across every Managed Job admission surface: `process.exec`, `process.batch`, `skills.run`, `mcp.callTool`, and `mcp.batch`; batch children inherit the parent group with no per-child override in v1.
- Froze `group` validation as a short human-readable key: trim surrounding whitespace, require non-empty, cap at 32 Unicode characters, allow ordinary readable Unicode/spaces/punctuation, reject control characters/newline/tab, preserve case, and perform no automatic normalization. TUI width truncation stays presentation-only.
- Confirmed current completed-Job retrieval: terminal Jobs remain available to `job.get` only while in memory, bounded by 24 hours and 100 terminal Jobs; after pruning they are not found, and restart loses local Job state. New SQLite history should make retained terminal records available to `job.get` across restart.
- Reduced the planned terminal in-memory retention to a 5-minute / 100-entry hot cache once SQLite history is available; older terminal `job.get` reads fall through to SQLite.
- Froze `job.list` query/pagination direction: optional exact `group/kind/state`, default limit 50/max 100, opaque cursor over `createdAt DESC, jobId DESC`, response `{jobs,nextCursor?}`, no offset/total/hasMore. Active-group discovery remains a separate concern rather than abusing paged history.
- Froze remaining `waitOnly` edge behavior: `waitSeconds=0` behaves as ordinary get, and non-terminal lifecycle transitions do not wake a wait early; wait continues to terminal or timeout.
- Froze SQLite write direction to admission insert + no continuous active/tail writes + terminal full snapshot; admission persistence is required so interrupted non-terminal Jobs can become `unknown_after_restart` after restart, while live TUI reads active state directly from memory.
- Inspected current TUI structure: shared `src/tui` provides terminal runtime/theme/widgets/form helpers, while `config_tui` is a config-specific application. Unified TUI construction has since been split into its own planning scope.
- Froze exact slim Managed Job response contracts for `process.exec`, `skills.run`, `mcp.callTool`, `process.batch`, `mcp.batch`, `job.get`, and `job.cancel`: tool-specific entrypoints omit implied `kind`/caller-supplied `group`, generic `job.get` restores identity metadata, process non-zero exit uses state/exitCode/tails, MCP retains result/error semantics, and cancellation becomes cancellation-focused.
- Split MCP batch result-budget semantics: per-Job oversized results use `resultTruncated` with retained preview/hash evidence, while aggregate-response-only omission uses `resultOmitted:true` and remains retrievable through `job.get`; parent aggregate truncation counters are unnecessary in the slim contract.
- Froze restart recovery: persisted active Jobs become `unknown_after_restart` at startup, terminal Jobs remain unchanged, recovery records `finishedAt`, and no missing true `startedAt`/`durationMs` is fabricated.
- Froze SQLite cleanup: run at startup and at most hourly after terminal snapshots, with immediate cleanup allowed when logical usage exceeds ~512 MiB; prune >30-day terminal Jobs then oldest terminal Jobs until under cap; never retention-prune non-terminal Jobs; use non-free page usage rather than physical file length; no automatic full/incremental vacuum in v1.
- Confirmed all remaining Phase 2 contract questions are resolved; Job planning advances to Phase 3 implementation.

### Refinement round 1: repository-grounded handoff check
- Evidence inspected: active plan pointer, full planning set, clean product working tree, `AppState`, `jobs.rs`, shared protocol Job/request types, stdio adapters/schemas, Hub MCP/HTTP forwarding/cache behavior, SQLite/audit path conventions, existing Job tests and workspace verification command.
- Questions pending: Q-01 history DB location/isolation; Q-02 persistence failure behavior.
- Decisions confirmed: previously frozen group, waitOnly, slim response, timing, list, retention, recovery, cleanup, and no-vacuum contracts remain compatible with repository evidence.
- Plan sections updated: Workflow State, refinement repository evidence, remaining contract gaps.
- Maturity transition: contract_frozen discussion -> refining handoff details; implementation remains unauthorized until Q-01/Q-02 and readiness checks are closed.
- Remaining blockers: 2.

### Refinement round 2: persistence failure behavior
- Evidence inspected: workspace `state/` conventions, current file reserved-path behavior, skill-install recovery behavior, and Job finalization/audit failure handling.
- Questions answered: Q-02.
- Decisions confirmed: D-20 fail-open Job history with explicit degraded health; Job outcome is independent from persistence outcome; non-durable terminal results stay in memory pending bounded retry; later terminal upsert may repair a missed admission; only clearly diagnosed corruption may be isolated/replaced.
- Plan sections updated: Workflow State blocker count, Decisions Made, Remaining contract gaps, D-20 rationale.
- Maturity transition: still refining; one blocker remains (Q-01 storage location/isolation).
- Remaining blockers: 1.

### Private-state prerequisite completed
- Split correctness-sensitive durable state out of writable `workspaceRoot` and implemented the prerequisite plan in `.planning/2026-08-10-private-state-layout-and-legacy-migration/`.
- `AppState.private_state.root` is now the authoritative per-agent durable root; active-skill state and skill-install recovery data were migrated and verified against the full Agent test suite.
- D-21 is fully resolved: Job SQLite should be placed at `state.private_state.root.join("jobs.sqlite3")`; do not reconstruct Agentic-home or `agentId` path mapping inside Job code.
- Prerequisite verification: clippy `-D warnings` PASS; `cargo test -p agentic-gpt` PASS with 314 + 15 + 1 + 6 tests; `git diff --check` PASS.
- Job refinement resumes with zero open product blockers; only final implementation-sequence/handoff readiness remains.

### Refinement round 3: final readiness gate and implementation handoff
- Evidence inspected: full active planning set, refine extension/readiness contracts, current HEAD/status, protocol Job/request types, `jobs.rs` runtime symbols, stdio Job dispatchers, Agent private-state wiring, and existing Hub `rusqlite` usage.
- Questions asked: none; no unresolved user-owned product choice remained. User accepted proceeding with final refine/handoff under the already frozen contract.
- Decisions frozen: consolidated stable D-01 through D-21; D-20 fail-open and D-21 private-state location retained unchanged.
- Plan sections updated: Scope/non-goals, executable Phase 3A–3F sequence, stable Decisions Made index, Acceptance criteria, Implementation Discretion, Implementation Handoff, final readiness evidence.
- Maturity transition: `refining` → `implementation_ready`; implementation authorization `no` → `yes`; exact entry set to Phase 3A.
- Repository safety check: HEAD `a50655f`; `git diff --name-only -- crates Cargo.toml Cargo.lock docs` empty; only planning paths were dirty/untracked before this write-through.
- Verification convention frozen: focused phase tests, then `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `git diff --check`.
- Commit convention frozen: no automatic commit; ask/receive user authorization for any focused implementation checkpoint.
- Remaining blockers: 0.
- Next invocation: `$planning-with-files` without `$refine-implementation-plan`; begin Phase 3A as Implementer.

### Implementation Phase 3A: protocol/domain entry
- Session catchup reported no unsynced prior implementation work; product tree remained clean before Phase 3A.
- Re-read the shared protocol Job/request area. `BatchExecRequest`, `SkillRunRequest`, `McpCallToolRequest`, `McpBatchRequest`, `JobInfo`, Job query types, and current rich response types all live in `agentic-gpt-protocol/src/lib.rs`, confirming protocol-first implementation is the correct boundary.
- Current `JobInfo.started_at` is non-optional; `JobListRequest` lacks `group/cursor`; `JobGetRequest` lacks `waitOnly`; current `JobResponse`, batch responses, and `JobDetail` are rich/flattened shapes that 3A must separate without yet changing runtime behavior.
- First planning write attempt was rejected at `file.batch` preflight because existing-file edits omitted `expectedRevision`; no file was changed. Retried with revisions instead of repeating the invalid call.
- Phase 3A marked `in_progress`; no SQLite/runtime source touched yet.

### Phase 3A verification notes
- Initial protocol implementation added all five admission `group` fields, D-02 group normalization/validation, optional `JobInfo.group/startedAt`, `job.list` group/cursor/default-limit contract, `job.get.waitOnly`, and purpose-built slim tool/list/wait/cancel/batch view types while retaining legacy rich response structs for later adapter migration.
- `cargo test -p agentic-gpt-protocol`: PASS, 17/17 tests including 5 new Job contract tests.
- `cargo check -p agentic-gpt-protocol`: PASS.
- `cargo fmt --all -- --check`: FAIL only on two rustfmt line-wrap diffs in the modified protocol file; no semantic/test failure. Resolution: run rustfmt, then re-run checks rather than changing logic.

- After rustfmt, `cargo check --workspace --all-targets` failed only on expected protocol compile fallout: missing new optional request/query fields in Rust struct literals, `JobInfo.started_at` constructors still using bare `DateTime`, and two duration calculations subtracting the new `Option<DateTime>`. No additional design/runtime conflict surfaced.
- Compatibility resolution for Phase 3A: mechanically set new request/query fields to `None`/`false` at existing callers, wrap existing admission timestamps in `Some(...)`, and make legacy duration calculations Option-aware without yet changing when Jobs truly start. Group propagation/actual-start semantics remain assigned to later runtime phases.

- First mechanical-fallout rewrite script aborted before any file writes because the `ExecRequest` regex also matched the suffix of `BatchExecRequest`, causing the subsequent expected batch count to be zero. Resolution: keep the precompute-before-write safety model and add an identifier boundary before request type names; no product file was partially mutated by the failed attempt.
- A follow-up `file.batch` planning-log write was rejected by the RPC layer after `expectedMatches: 1` was decoded as a floating-point value; switched to single-file `file.edit` rather than retrying the same batch shape.

- Second safe rewrite attempt also aborted before writes: after adding the identifier boundary, the true `ExecRequest` literal count is 10 rather than the earlier 13; the three extras in the first dry-count were exactly the three `BatchExecRequest` literals whose type names end in `ExecRequest`. Corrected the expected count to 10; no product mutation occurred.

- Third safe rewrite attempt aborted before writes because one Hub `JobGetRequest.wait_seconds` is a multi-line expression, while the script expected a single-line field. Inspected both JobGet literals; resolution is to insert `wait_only: false` immediately after the single-line `job_id` field instead of parsing the wait expression. No product file was partially mutated.

- Corrected mechanical rewrite passed all pre-write count assertions and updated 9 Agent/Hub compatibility files: 10 Exec, 3 process-batch, 4 MCP-call, 3 MCP-batch, 5 skill-run request literals; 6 JobInfo group/start literals; 5 JobList and 2 JobGet literals; plus the two legacy duration consumers and one test assignment.
- `cargo fmt --all` completed, then `cargo check --workspace --all-targets`: PASS. The shared protocol contract can therefore coexist with the unchanged current runtime behavior until later phases implement real group propagation/start timing.

### Phase 3A completion
- Diff review PASS: protocol owns the new contracts; nine compatibility files contain only mechanical default/Option changes, with no premature group propagation, actual-start transition, history store, or slim-adapter switch.
- Final Phase 3A checks PASS: `cargo fmt --all -- --check`; `cargo clippy -p agentic-gpt-protocol --all-targets -- -D warnings`; `cargo test -p agentic-gpt-protocol` (17/17); `git diff --check`.
- Workspace compatibility had already been re-established with `cargo check --workspace --all-targets` PASS after the mechanical caller updates.
- Phase 3A marked complete. No commit created. Implementation intentionally stops here; next phase is 3B durable Job history.

### Test Results
| Test | Expected | Actual | Status |
|------|----------|--------|--------|
| Repository trace of `mcp.batch.id` | Determine whether it affects job identity/runtime semantics | It is retained correlation metadata; generated `jobId` remains runtime identity | PASS |
| Repository trace of `file.batch.id` | Determine whether it creates/labels Managed Jobs | It only correlates batch operations/results/audit; no Managed Job is created for each operation | PASS |

### Errors
| Error | Resolution |
|-------|------------|
| None | — |

### Phase 3B implementation and verification

- Added `crates/agentic-gpt/src/job_history.rs` and Agent-side bundled `rusqlite 0.32`; `AppState` now owns the startup-opened per-agent history handle without wiring admission/finalization/runtime lifecycle hooks.
- Implemented schema/index initialization, bounded rich snapshots, admission insert, terminal upsert, get/list with exact filters and opaque cursor, startup active-row recovery, terminal preservation, logical retention/cap cleanup, health/degraded state, bounded pending-terminal retry, corruption-only isolation, and non-destructive failure behavior.
- Focused store tests: 9/9 PASS (`job_history::tests`), covering schema idempotency/indexes, admission+terminal upsert, ordering/filter/cursor, group/startedAt recovery round-trip, restart recovery, age/cap retention, nonterminal preservation, bounds, degraded failure, and corruption isolation.
- `cargo check --workspace --all-targets`: PASS.
- `cargo clippy -p agentic-gpt --all-targets -- -D warnings`: PASS.
- `cargo fmt --all -- --check`: PASS.
- `cargo test -p agentic-gpt`: unit/in-process suite passed (323 tests plus 15 config integration tests); the existing `tests/local_control.rs` runtime test failed only because the sandbox run could not make its Unix socket ready (`local socket did not become ready`), an environment-bound local socket condition rather than a Job-history assertion.
- `git diff --check`: PASS. No commit created. No Phase 3C lifecycle wiring, slim response adapter changes, waitOnly behavior, Hub semantics, TUI work, or non-Managed-Job tool changes were made.

### Phase 3B review corrections
- Independent review found three related D-20 gaps in the initial implementation: retry memory was bounded but retry attempts/backoff were not; dropping a never-persisted terminal only incremented a counter without the frozen high-signal warning; and a successful unrelated history write could mark health healthy while pending terminal snapshots still existed.
- Corrected pending persistence to a 100-entry queue with a finite 5-attempt budget and 2/4/8/16-second backoff. Exhausted retry budget and queue-capacity eviction both increment dropped-terminal diagnostics and emit a bounded warning containing only the Job id/reason.
- Corrected health semantics and mutex lock order: pending state is inspected before the health lock, avoiding the prior opposite lock ordering, and health remains degraded until pending persistence is fully drained.
- Added two focused D-20 regression tests and strengthened the queue-bound test; `job_history::tests` now passes 11/11.
- Fresh post-review verification: `cargo fmt --all -- --check` PASS; `cargo check --workspace --all-targets` PASS; `cargo clippy -p agentic-gpt --all-targets -- -D warnings` PASS; focused Job history tests 11/11 PASS; `git diff --check` PASS.
