# Task Plan: Job Observability and Managed Job Contracts

## Goal

Improve AgenticGPT's managed-job observability and control so concurrent workstreams are easy to distinguish, polling is context-efficient, and a later unified TUI can consume a stable Job/query model without changing core execution semantics.

## Current Phase

Complete — Managed Job observability, durable history, compact tool contracts, and Hub parity delivered

## Workflow State

- **Stage:** implementation_complete
- **Current role:** completion/review
- **Implementation authorized:** complete
- **Entry phase:** n/a
- **Open blocking decisions:** 0
- **Design checkpoint:** delivered across the managed-Job checkpoint series through `55131df`; final Hub parity is the closing change set
- **Next action:** choose the next independent requirement/plan; do not auto-activate the separate TUI baseline

## Scope and constraints

- Add optional caller-provided `group` metadata to every Managed Job admission surface and carry it through live state, durable history, `job.get`, and `job.list`.
- Add durable per-agent SQLite Job history at `AppState.private_state.root.join("jobs.sqlite3")`, with truthful restart recovery, bounded retention, and fail-open persistence health.
- Replace rich routine Managed Job responses with the frozen slim shapes while retaining rich bounded Job records for history/Inspector.
- Add `job.get(waitOnly)` and cursor-based `job.list` semantics without changing the underlying execution/cancellation meaning of process, skill, or MCP Jobs.
- Keep Hub forwarding/cache/reporting behavior compatible with the updated shared protocol and local semantics.
- Additive request fields are optional for callers. Existing Managed Job tool names are updated in place; the confirmed slim response contract intentionally replaces the old rich routine response shapes rather than introducing parallel `v2` tools.
- Existing authentication/authorization and network trust boundaries are unchanged. Job history is private durable state, not workspace-visible state; raw environment values/secrets must not be added to persisted Job detail.

### Non-goals

- Building the Process/Unified TUI itself; that remains in `.planning/2026-08-10-unified-tui-baseline/`.
- Adding a durable group registry or model-facing `job.groups` tool in v1.
- Per-child `group` overrides inside batch calls.
- Turning audit JSONL into Job history or replacing the audit trail.
- Persisting unbounded stdout/stderr/MCP payloads, implementing an output spool, compression, or automatic VACUUM in v1.
- Pulling file/tmux/non-Managed-Job tools into the response refactor merely for consistency.
- Changing core Job execution semantics, adding a persistent AI loop, or making history durability a prerequisite for execution.

## Phases

### Phase 1: Requirements & Discovery

- [x] Reconstruct current Managed Job identity, response, polling, batch-correlation, retention, and TUI-consumer requirements.
- [x] Ground the design in `jobs.rs`, shared protocol types, stdio adapters, Hub forwarding/cache behavior, audit/history conventions, and private-state layout.
- [x] Resolve grouping, timing, history, recovery, retention, failure, and response-contract questions.
- **Status:** complete

### Phase 2: Contract Freeze

- [x] Freeze D-01 through D-21, including exact `group`, `waitOnly`, list/cursor, response-shape, persistence, recovery, cleanup, fail-open, and private-state contracts.
- [x] Verify the private-state prerequisite in product code and full Agent tests.
- [x] Run the final handoff readiness gate with zero open product blockers.
- **Status:** complete

### Phase 3A: Protocol & Domain Contract

**Objective:** make the shared Rust contract express the frozen Job semantics before runtime/storage code depends on them.

- [x] Add optional `group` to every Managed Job admission request/protocol path; batch children inherit parent metadata rather than accepting child overrides.
- [x] Make `startedAt` optional/actual-start and add the request/query fields needed for `waitOnly`, exact `job.list` filters, limit, and opaque cursor.
- [x] Introduce purpose-built slim response/summary types or equivalent serializers for single Job, batch, list, get, and cancel surfaces; keep rich persisted detail separate.
- [x] Preserve existing `batchCallId` / child `id` correlation semantics and generated `jobId` machine identity.
- [x] Update serde/schema tests for omitted optional fields, camelCase names, group validation boundaries, and intentional in-place response-shape changes.
- **Completion boundary:** shared protocol compiles/tests with no runtime behavior implemented by guesswork.
- **Status:** complete

### Phase 3B: Durable Job History Store

**Prerequisite:** Phase 3A protocol/domain types and existing `AppState.private_state`.

- [x] Add Agent-side `rusqlite` using the repository's existing 0.32/bundled stack and create a private `jobs.sqlite3` history store under D-21.
- [x] Implement idempotent schema initialization, indexed query columns for `jobId/group/kind/state/createdAt`, and bounded rich detail storage suitable for `job.get`/Inspector.
- [x] Implement admission insert, terminal upsert/snapshot, startup active→`unknown_after_restart` recovery, terminal preservation, and truthful optional start/duration handling.
- [x] Implement D-20 fail-open/degraded health, bounded retry for never-persisted terminal results, corruption-only isolation, and non-destructive handling for permission/disk/I/O/lock failures.
- [x] Implement 30-day/~512 MiB logical cleanup, startup/hourly/immediate-cap checks, terminal-only pruning, and no-vacuum behavior.
- **Completion boundary:** history store is independently tested for schema, recovery, retention, and failure injection before tool responses depend on it.
- **Status:** complete

### Phase 3C: Managed Job Runtime Integration

**Prerequisite:** Phases 3A–3B.

- [x] Store validated `group` on Managed Jobs and record true admission/start/finish timestamps at the actual lifecycle boundaries.
- [x] Wire admission persistence, actual-start updates in process/skill/MCP runners, terminal snapshots, pending-persistence retention, and 5-minute/100 terminal hot-cache pruning.
- [x] Make `job.get` resolve live memory first and persisted terminal history second; persisted rows remain available across restart for the retention window.
- [x] Make `job.list` merge live memory with persisted summaries, deduplicate by `jobId` with live state winning, apply exact filters, and paginate on `(createdAt DESC, jobId DESC)`.
- [x] Keep cancellation/result truth independent from history health and preserve rich bounded cancellation/evidence data for Inspector/history.
- **Completion boundary:** runtime tests prove live/persisted merge, timing, recovery, hot-cache fallback, and degraded-history behavior.
- **Status:** complete

### Phase 3D: Slim Tool Surfaces & `waitOnly`

**Prerequisite:** Phase 3C runtime/query behavior.

- [x] Update stdio/MCP schemas and adapters for `group`, `waitOnly`, filters/cursor, and exact frozen response shapes.
- [x] Implement `waitOnly=true`: terminal-or-timeout only; timeout while active returns exactly `jobId/state/elapsedMs`; `waitSeconds=0` is ordinary get.
- [x] Slim `process.exec`, `skills.run`, `mcp.callTool`, `process.batch`, `mcp.batch`, `job.get`, and `job.cancel` according to D-15–D-19.
- [x] Preserve process non-zero exit as `failed + exitCode + tails`, structured preflight/rejection errors, MCP downstream result+error semantics, and per-Job truncation evidence.
- [x] Enforce `mcp.batch` distinction between `resultTruncated` (Job retention ceiling) and `resultOmitted` (aggregate budget only); remove obsolete aggregate/helper noise.
- **Completion boundary:** schema/adapter tests assert the exact active/terminal/batch/cancel JSON shapes and absence of removed noise.
- **Status:** complete

### Phase 3E: Hub / Reporting / Cache Parity

**Prerequisite:** frozen shared protocol and local runtime/tool adapters.

- [x] Propagate optional `group`, `waitOnly`, filters, and cursor fields through Hub full MCP/HTTP forwarding without conflating group with child correlation ids.
- [x] Update Hub cached Job fallback handling so first-page filters remain truthful, rich JobInfo does not leak through ordinary fallback, Agent-issued cursors are never fabricated by cache, and failed waits are reported as degraded cache evidence rather than fresh results.
- [x] Keep Agent/Hub outcome semantics aligned under history degradation, restart states, and Agent unavailability.
- **Completion boundary:** Hub/Agent protocol integration and Hub schema/unit tests compile and exercise updated forwarding/reporting/cache paths.
- **Status:** complete

### Phase 3F: Documentation & Full Verification

- [x] Update interface/docs for optional `group`, `waitOnly`, slim responses, durable history/recovery, private Job DB location, list cursor semantics, and truthful Hub cache degradation.
- [x] Run focused protocol/Job/stdio/Hub tests while implementing plus real local-runtime smoke coverage.
- [x] Run formatting, clippy/check, Agent/Hub unit suites, local MCP integration, and `git diff --check`; environment-sensitive full-workspace checks were supplemented by the passing crate-wide suites and real local smoke.
- [x] Review the diff for accidental secret persistence, stale rich response leakage, and internal/public correlation confusion.
- **Completion boundary:** final managed Job contract is stable for TUI/other consumers.
- **Status:** complete

### Phase 4: Integration Verification & TUI Handoff

- [x] Verify concurrent callers/workstreams remain distinguishable through persisted `group` metadata across process/skill/MCP Jobs.
- [x] Verify compact polling keeps stable `jobId` lookup into retained terminal history and that restart/history semantics remain truthful.
- [x] Record the final Job/query contract in this plan/findings for later TUI consumption; the separate untracked `.planning/2026-08-10-unified-tui-baseline/` remains untouched.
- **Status:** complete

## Key Questions

- **Open blocking questions:** none.
- Historical contract questions are resolved into D-01 through D-21. Q-01 (history location/isolation) resolved as D-21; Q-02 (persistence failure behavior) resolved as D-20. Reopen design only if implementation finds a concrete conflict with a frozen contract.

## Decisions Made

| ID | Area | Status | Frozen outcome |
| --- | --- | --- | --- |
| D-01 | Identity/grouping | confirmed | Generated `jobId` remains machine identity. Optional caller `group` is a human-readable cross-call workstream key; exact equal strings mean the same logical group and no opaque group id is added. |
| D-02 | Group validation | confirmed | Trim outer whitespace; require non-empty; max 32 Unicode scalar values; allow readable Unicode/spaces/punctuation; reject CR/LF/tab/control characters; compare case-sensitively with no normalization/casefold. |
| D-03 | Admission coverage | confirmed | `process.exec`, `process.batch`, `skills.run`, `mcp.callTool`, and `mcp.batch` accept optional `group`; batch children inherit the parent and have no v1 override. |
| D-04 | Group lifecycle | confirmed | Active group availability expires 30 minutes after the most recent **new grouped Job admission**; runtime activity does not refresh TTL. History keeps `group`, later reuse reactivates the same string, and no durable group registry is created. |
| D-05 | Existing correlation ids | confirmed | `mcp.batch.id`/`batchCallId` and file-batch operation ids remain local correlation fields and never become `group` or Job identity. |
| D-06 | Durable history | confirmed | Add SQLite Job history separate from audit JSONL so retained terminal Jobs/grouping survive restart and `job.get` can fall back to durable records. |
| D-07 | Retention/cleanup | confirmed | Retain terminal history 30 days with ~512 MiB **logical** per-agent soft cap; prune old/oldest terminal rows only, never non-terminal rows; cleanup at startup and at most hourly after terminal writes, with immediate cap checks allowed. |
| D-08 | Storage size policy | confirmed | No compression or automatic full/incremental VACUUM in v1; reuse free pages and keep process tails/result retention bounded by the existing ceilings. |
| D-09 | Persistence write policy | confirmed | Persist admission once, do not continuously persist live state/tails, and upsert one complete terminal snapshot; live TUI/runtime state stays in memory. |
| D-10 | Restart recovery | confirmed | Persisted active Jobs become `unknown_after_restart`; terminal rows stay unchanged. Recovery sets `finishedAt` to recovery time, preserves true `startedAt` when known, and never fabricates start/duration. |
| D-11 | In-memory retention | confirmed | Once durable history exists, terminal Managed Jobs are a 5-minute / max-100 hot cache; older retained reads fall through to SQLite. Never-persisted terminal results follow D-20 rather than ordinary eviction. |
| D-12 | Time semantics | confirmed | `createdAt` = admission, optional `startedAt` = actual execution start, `finishedAt` = terminal transition; active surfaces derive `elapsedMs` (0 before start), terminal surfaces expose `durationMs` only when a true start exists. |
| D-13 | `job.list` | confirmed | Summary row: `jobId, group?, kind, state, createdAt, startedAt?, finishedAt?`. Exact optional `group/kind/state` filters, default limit 50/max 100, opaque cursor over `createdAt DESC, jobId DESC`, response `{jobs,nextCursor?}` only; live rows override persisted duplicates. |
| D-14 | `job.get waitOnly` | confirmed | Optional `waitOnly`; `waitOnly=true` with positive wait waits through non-terminal transitions and wakes only on terminal or timeout. Active timeout returns **exactly** `jobId,state,elapsedMs`; terminal/already-terminal returns the normal terminal get; `waitSeconds=0` is ordinary get. |
| D-15 | Rich-vs-slim contract | confirmed | Keep rich bounded durable Job records for history/Inspector but use purpose-built slim routine responses. Single Job uses `state`, aggregate parent uses `status`; omit empty/null/false noise, raw timestamps/commands/helpers, and duplicate rejection fields. Existing tool names update in place; optional request additions are backward-compatible while the slim response shape intentionally replaces the old rich routine shape. |
| D-16 | Process/skill responses | confirmed | Active process/skill: `jobId,state,elapsedMs` + non-empty bounded tails and `truncated:true` only when clipped. Terminal: `jobId,state,durationMs?,exitCode?,tails?,truncated?`. Non-zero exit is `failed + exitCode + tails`; preflight/policy/admission rejection uses structured `{code,message}` error. |
| D-17 | MCP single responses | confirmed | Active MCP: `jobId,state,elapsedMs`. Terminal: `jobId,state,durationMs?,result?,error?`; downstream `isError` may keep both result+error. Result above 512 KiB uses `resultTruncated:true,resultBytes,resultSha256,resultPreview?` instead of full result. |
| D-18 | Batch responses | confirmed/final | `process.batch` => `batchId,status,jobs`. Model-facing `mcp.batch` => `status,error?,results`; child order is the public correlation mechanism, while batch/call ids and indexes remain internal for audit/runtime correlation. `resultTruncated` means the Job retention ceiling; `resultOmitted` means aggregate-budget omission while `job.get` still has the retained result. Remove routine `completedInline/pollAfter` and aggregate truncation counters. |
| D-19 | Cancel/detail/security boundary | confirmed | `job.cancel` returns only `jobId,state,cancelOutcome,terminationEvidence,error?`. Persisted detail keeps bounded provenance (process program/args/cwd; skill id/path/digest; MCP server/tool), cancellation/evidence and rich errors, but no raw environment values/secrets or unbounded streams; future output spool/outputRef is out of scope. |
| D-20 | Persistence failure | confirmed | Job-history persistence is fail-open and outcome-independent: execution continues in explicit degraded/live-only history health; terminal results pending persistence receive bounded retry/retention; later terminal upsert may repair missed admission. Only diagnosed corruption may be isolated/replaced; permission/disk/I/O/lock failures are non-destructive. |
| D-21 | History location | confirmed | Use `AppState.private_state.root.join("jobs.sqlite3")`, normally `~/.agentic_gpt/state/agent/<agentId>/jobs.sqlite3`; consume the implemented private-state mapping rather than reconstructing `agentId` paths. Retention/cap are per-agent DB. |

## Acceptance criteria

- [x] All five Managed Job admission surfaces accept valid optional `group`; invalid values fail with a stable structured error and batch children inherit the parent exactly.
- [x] `group` survives live execution, terminal persistence, restart, `job.get`, and `job.list`; existing child correlation ids remain unchanged and distinct.
- [x] `createdAt/startedAt/finishedAt/elapsedMs/durationMs` reflect D-12, including queued/rejected-before-start cases with no fabricated duration.
- [x] `job.get(waitOnly=true)` matches D-14 exactly for active timeout, terminalization, already-terminal Jobs, zero wait, not-found/error paths, and state refresh immediately before serialization.
- [x] Routine process/skill/MCP/batch/get/cancel JSON matches D-15–D-19 and no removed rich/noise fields leak back through adapters or Hub fallback.
- [x] Process non-zero exit, preflight rejection, MCP downstream error, per-Job result truncation, and aggregate-only result omission remain observably distinguishable.
- [x] Job history is created only under D-21 private state and does not reserve/write `workspaceRoot/state/jobs*`.
- [x] Admission + terminal snapshot policy supports restart mapping to `unknown_after_restart`, preserves terminal records, and never fabricates unknown active output/timestamps.
- [x] History init/write/cleanup failures obey D-20: underlying Job outcome remains truthful, degraded health is inspectable, retries/memory retention are bounded, and non-corruption failures never rename/delete a healthy DB.
- [x] Normal persisted terminals leave the 5-minute/100 hot cache and remain retrievable through `job.get` for the durable retention window.
- [x] `job.list` merges live + persisted state without duplicates, live wins same `jobId`, exact filters work, cursor pagination is stable under new admissions, and default/max limits are 50/100.
- [x] Cleanup uses logical non-free-page usage, prunes terminal rows in the frozen order, never retention-prunes non-terminal rows, and does not auto-vacuum.
- [x] Persisted detail stays within frozen tail/result bounds and adds no raw environment values or new secret material.
- [x] Hub forwarding/report/cache code preserves `group`, optional timing, restart state, and slim response semantics under the coordinated shared-protocol update.
- [x] No `job.groups`, child-group override, compression/output spool, or Process TUI implementation is introduced by this plan. A later user-authorized non-Job tool-surface compaction was completed as a separate follow-up and does not alter these Managed Job semantics.
- [x] Final `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `git diff --check` pass.

## Implementation Discretion

The Implementer may choose the following private details so long as no frozen observable contract changes:

- SQLite table/column split versus a bounded `detail_json`, schema-version helper naming, indexes, prepared-statement/transaction organization, and connection synchronization strategy. Queryable sort/filter fields must remain indexed/available.
- Opaque cursor encoding and validation details, provided the stable sort tuple and error behavior are deterministic and cursors do not expose a new public semantic.
- Exact bounded retry/backoff schedule, degraded terminal-memory cap, and history-health diagnostic placement, provided D-20 remains bounded, inspectable, and fail-open.
- Module/helper/type names, internal serializer factoring, hot-cache data structures, and whether common views are traits/functions/structs.
- Test module layout, fixtures, temporary DB helpers, and assertion wording.
- Whether Agent-side `rusqlite` is declared directly or promoted to a workspace dependency, provided the repository stays on the existing compatible 0.32/bundled stack.

## Implementation Handoff

- **Plan maturity:** delivered
- **Design phase:** complete
- **Implementation authorized:** complete
- **Entry phase:** n/a
- **Frozen decisions:** D-01 through D-21
- **Open blocking decisions:** none
- **Implementation discretion:** see `Implementation Discretion`
- **Verification convention:** focused tests per phase; final `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `git diff --check`
- **Commit convention:** no automatic commits; report phase diff/tests and commit only when the user authorizes a focused checkpoint
- **Design checkpoint:** not set; planning remains local/untracked, private-state prerequisite is product commit `a50655f`
- **Next invocation:** select or create the next independent plan; this plan requires no further implementation.

## Errors Encountered

| Error | Resolution |
|-------|------------|
| Initial focused Cargo command supplied multiple test filters | Re-ran with the supported `jobs::tests::` module filter; all 10 focused runtime tests passed. |
| Full Agent suite had five `Operation not permitted` listener/download failures | Recorded as sandbox/environment-bound and unrelated to Job/history code; 323 other Agent tests passed. |

## Notes

- Final handoff readiness passed on 2026-08-10 after the user accepted proceeding with the already consolidated/frozen contract; no new product choice was introduced by the handoff rewrite.
- Contract freeze is complete; Phase 3 implementation should treat D-01 through D-21 and the acceptance criteria as constraints rather than reopen them without a concrete conflict.
- Unified TUI application architecture and Process screen work are split into `.planning/2026-08-10-unified-tui-baseline/`; this plan only defines the Job/runtime/query contracts that TUI will consume.
- Phase 3C completed on 2026-08-13: Agent runtime now persists admission/start/terminal lifecycle state, retains bounded degraded terminals, falls back from live memory to durable history, and exposes merged stable-cursor list paging. Phase 3D remains the next scoped phase for slim adapters and `waitOnly`.
- Verification handoff: workspace all-targets check, Agent clippy, rustfmt check, diff check, Job/history/MCP focused tests all pass; full Agent tests are 323 pass / 5 sandbox-bound failures listed above. No commit was created.
- Do not let TUI presentation requirements silently redefine core managed-job execution semantics.
- 2026-08-18 completion review found the remaining Phase 3E gap in Hub full/HTTP forwarding (`group`, `job.list cursor`, `job.get waitOnly`) and cache fallback semantics; the gap was closed before marking this plan complete.
- Real `agentic-gpt run` + `agentic-gpt local` smoke testing also exposed and fixed terminal `finishedAt`/`durationMs` drift caused by repeated refresh of an already-terminal process.
- Later public-tool compaction removed model-facing `mcp.batch` correlation bookkeeping while retaining it internally; D-18 above records the final observable contract.
- Existing full Job records should remain authoritative even if ordinary tool responses become much smaller.
