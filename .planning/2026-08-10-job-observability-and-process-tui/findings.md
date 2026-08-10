# Findings & Decisions

## Requirements
- Current collaboration can involve multiple concurrent callers/workstreams (for example, direct ChatGPT collaboration alongside a separate Work/Codex task). Raw command/job history can become hard to attribute to the correct workstream.
- Add a caller-provided, human-readable cross-call grouping value that can be reused across many commands/jobs belonging to the same workstream.
- The grouping value should later support natural grouping/filtering in Process TUI.
- Separately, reduce repeated Job/tool response payloads and add a bounded `job.get waitOnly` polling mode without removing retained full Job detail.

## Research Findings
- `mcp.batch` child calls already accept an optional `id`. It is validated for batch-local uniqueness and `^[A-Za-z0-9._:-]{1,64}$` form.
- That MCP child `id` is copied into `ManagedMcpSpec.batch_call_id`, retained as `JobInfo.batchCallId`, returned in `McpBatchChildResponse.id`, and written to audit metadata.
- `mcp.batch` child `id` does not become the primary Job id and does not control scheduling/cancellation/lookup; those continue to use generated `job_*` ids.
- `file.batch` operations also accept optional `id`; it is echoed in operation result/error envelopes and retained in edit-group/audit metadata as an operation correlation value. `file.batch` operations are not Managed Jobs.
- Therefore current batch `id` fields are useful precedents for readable correlation, but their semantics are local child/operation identity, not cross-call workstream grouping.
- Current `JobInfo` is deliberately rich: agent/job/batch ids, kind/state/timestamps, process command fields, stdout/stderr tails, truncation/rejection data, skill metadata, MCP metadata, cancellation/evidence fields, etc. This supports retained detail but is too heavy to repeat blindly in ordinary polling responses.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Do not reuse `batchCallId` as the cross-call grouping key | Batch child ids should differ inside a batch; workstream grouping intentionally repeats across independent calls/jobs. |
| Keep generated `jobId` as the machine identity | Grouping is organizational metadata, not a replacement for exact Job identity. |
| Treat the new grouping value as caller-provided rather than inferred from command text/path | Agentic cannot reliably know whether identical-looking commands came from direct collaboration, Work, or another concurrent task. |
| Prefer batch-level inheritance for batch tools in the first design | A shared workstream group naturally applies to all child jobs; per-child override can be deferred unless a concrete need appears. |

## Grouping Lifecycle Semantics
- The grouping value is a pure human-readable string. No opaque internal group id is required.
- The same string may be reused across independent tool calls/jobs; equality of the string defines logical group membership.
- `group` is stored directly on each Job as Job metadata. No separate durable group identity/registry is required.
- First use of a previously unseen string effectively registers it by creating/admitting a grouped Job.
- Active identifiers expire after 30 minutes without new grouped Job activity. Expiry only removes them from the active/available set; persisted historical Jobs keep their `group` field.
- Reusing an expired string creates a new Job with the same group string and therefore naturally reactivates the group and rejoins its history.
- The active group list can be derived from recent persisted/live Jobs instead of maintaining a second authoritative registry.
- Freeze the human-readable `group` value contract: trim surrounding whitespace, require a non-empty result, and cap it at 32 Unicode scalar values/characters. Allow ordinary readable Unicode, spaces, and punctuation, but reject control characters including newline, carriage return, and tab. Equality is case-sensitive and Agentic does not perform case folding or other automatic normalization.

## Job History & Restart Findings
- Current standalone/local Managed Jobs live in `AppState.jobs` as an in-memory `HashMap` and terminal Jobs are pruned after 24 hours / a bounded terminal count.
- Existing `.agentic-gpt-audit.jsonl` is not a suitable Process TUI history store. `AuditRecord` captures command metadata, hashes, terminal state and evidence, but not the full retained `JobDetail`, stdout/stderr tails, or result payload needed for interactive history/Inspector use.
- `JobState::UnknownAfterRestart` already exists in the shared protocol and is used by Hub-side recovery semantics, so restart-recovery of locally persisted Jobs can use an existing state vocabulary rather than inventing a new one.
- Durable Job history is now considered part of the Process TUI foundation: TUI history should survive Agentic restart, and persisted `group` fields make historical grouping a natural consequence rather than a separate subsystem.
- Restart recovery follows the existing Hub truthfulness model: on Agent startup, every persisted Job whose state is still active becomes `unknown_after_restart`; already-terminal Jobs are left exactly as recorded. Set `finishedAt` to the recovery time. Preserve an existing true `startedAt` if present, but never invent one; if no true start is known, do not fabricate `durationMs`.

## SQLite History Direction
- SQLite is the preferred backing store for Job history. Job records are mutable across lifecycle states and need indexed querying by time/state/kind/group plus pagination and cleanup; JSON/JSONL is a poor fit for that product surface.
- SQLite does not provide general automatic compression of text/JSON payloads. Its space advantage over JSON mainly comes from structured storage and avoiding repeated field names, not gzip-like compression.
- The repository already includes `rusqlite 0.32` with `bundled` and `chrono` features for the Hub, so using the same stack on the Agent side does not introduce a foreign storage technology.
- Current output bounds make an uncompressed first version reasonable: process stdout and stderr tails are each capped at 64 KiB; MCP results above 512 KiB are not retained in full and instead use bounded preview/hash metadata.
- Retention is frozen at 30 days for terminal Job history plus an approximately 512 MiB database soft cap; prune oldest terminal Jobs when either boundary is exceeded. Never prune non-terminal Jobs due to history retention.
- If observed history size later warrants compression, large tail/result payloads can be compressed separately without changing the metadata/query model; compression is not required for the initial SQLite design.
- Cleanup runs once at Agent startup, then no more than once per hour after terminal snapshots during normal operation; crossing the ~512 MiB logical soft cap may trigger an immediate cleanup check rather than waiting for the hourly interval.
- Cleanup first removes terminal Jobs older than 30 days, then—if logical usage still exceeds the cap—removes the oldest remaining terminal Jobs until usage is back under the soft cap. Retention cleanup never deletes non-terminal Jobs.
- Do not compare the soft cap to the physical SQLite file length because deleted pages remain reusable inside the file. Estimate logical usage from non-free allocated pages, e.g. `(page_count - freelist_count) * page_size` or an equivalent SQLite-backed metric.
- v1 does not enable periodic full `VACUUM` or incremental-vacuum maintenance. Free pages may remain in the physical file and be reused by later writes; physical file shrinkage is a separate future requirement.

## Contract Freeze Notes
- The field name is `group`; active-group TTL is based only on new grouped Job admission/creation activity, not continued runtime activity.
- First-pass `job.list` supports exact `group` filtering and returns a flat ordered summary page carrying `group` metadata; grouping into TUI panels is presentation-layer behavior.
- Non-terminal lifecycle transitions do not wake `job.get(waitOnly=true)` early; the bounded wait ends only on terminalization or timeout.
- The former open questions above are resolved for v1. Reopen them only if implementation uncovers a concrete contract conflict.

## `waitOnly` Semantics
- `waitOnly` is a parameter of `job.get`, not a separate tool or history operation.
- Its purpose is to suppress repeated `JobDetail` payloads only while the caller is purely waiting on an active Job.
- If the bounded wait ends and the Job is still non-terminal, return only the minimum Job identity/state fields (effectively `jobId` plus `state`/existing equivalent status), with no `changed`, `terminal`, `detailAvailable`, `pollAfterMs`, stdout/stderr/result, or other repeated metadata.
- If the Job becomes terminal during the wait, return the normal full `job.get` / `JobDetail` result immediately. Do not force a second `job.get` call merely to retrieve the final output.
- If `waitOnly=true` is called for a Job that is already terminal, return the normal full terminal result immediately; `waitOnly` has no reason to suppress data once no waiting is needed.
- This preserves the useful single-call pattern: wait efficiently while active, receive the result naturally when completion occurs.

## Tool-facing Job Response Slimming
### Frozen response conventions
- Single-Job surfaces use `state`; aggregate batch surfaces use `status`.
- Tool-specific admission surfaces omit metadata already implied by the call (`kind`) or just supplied by the caller (`group`). Generic `job.get` includes `group?` and `kind` because it must identify an arbitrary Job.
- Active responses expose `elapsedMs`; before actual execution starts it is `0`. Terminal responses expose `durationMs` only when an actual `startedAt` is known; rejected/skipped-before-start Jobs do not fabricate execution duration.
- Optional empty fields are omitted rather than serialized as null/false placeholders. Truncation metadata appears only when truncation actually occurred.
- Slim responses expose one structured `error { code, message }` when caller action is required; rich `rejectReason`/internal evidence may remain in persisted detail, but routine responses should not return both an error and a duplicate reject-reason field.

### `process.exec` / `skills.run`
- Active: `jobId`, `state`, `elapsedMs`, optional non-empty `stdoutTail`/`stderrTail`, and `truncated:true` only when process tails were clipped.
- Terminal after execution: `jobId`, `state`, optional `durationMs`, optional `exitCode`, optional non-empty stdout/stderr tails, and `truncated:true` only if applicable.
- Process/skill non-zero exit is represented by `state=failed + exitCode + tails`; do not synthesize a redundant `error` merely because the executable exited non-zero.
- Rejection/preflight/admission failure that prevented normal execution uses structured `error` instead of exposing `rejectReason` directly.

### `mcp.callTool`
- Active: `jobId`, `state`, `elapsedMs`.
- Terminal: `jobId`, `state`, optional `durationMs`, optional `result`, optional structured `error`.
- If the Job-level MCP result exceeded the retained-result ceiling, omit full `result` and return `resultTruncated:true` plus `resultBytes`, `resultSha256`, and bounded `resultPreview` when available. These metadata are omitted for normal retained results.
- Downstream `isError=true` may legitimately return both `result` and `error`; preserve both because the downstream payload can contain useful error detail.

### `job.get`
- Ordinary active get: `jobId`, optional `group`, `kind`, `state`, `elapsedMs`; process/skill additionally include bounded non-empty stdout/stderr tails and `truncated:true` when needed, while MCP has no synthetic progress fields.
- `waitOnly=true` while still active: exactly `jobId`, `state`, `elapsedMs`.
- Terminal get (live or SQLite history): `jobId`, optional `group`, `kind`, `state`, optional `durationMs`, then the same kind-specific terminal outcome fields used by the corresponding admission surface.
- No raw timestamps, command metadata, lifecycle helper flags, or Inspector/audit metadata are returned by routine `job.get`.

### `process.batch`
- Parent: `batchId`, `status`, `jobs` only.
- Each child uses the same slim process child shape as `process.exec`; array order remains the correlation mechanism, so no extra child index is required.
- Remove `completedInline` and `pollAfterMs`.

### `mcp.batch`
- Parent: `batchId`, `status`, optional aggregate-level `error`, and ordered `results`.
- Each result keeps `index`, optional caller `id`, then the same slim MCP child shape (`jobId/state/elapsedMs` while active or terminal result/error fields).
- Distinguish two different budget cases: `resultTruncated:true` means the Job itself retained only bounded preview/hash evidence because the downstream result exceeded the per-Job ceiling; `resultOmitted:true` means an otherwise retained child result was omitted only from the aggregate batch response budget and remains retrievable with `job.get(jobId)`.
- With per-child `resultOmitted`, parent `aggregateTruncated/aggregateBytes` are unnecessary and should be removed from the slim response.

### `job.cancel`
- Return only `jobId`, `state`, `cancelOutcome`, `terminationEvidence`, and optional structured `error` when cancellation delivery/termination itself failed.
- Do not return stdout/stderr/result/command/timestamps merely because cancellation was requested.

- Current `JobResponse` duplicates `status`/`jobId` outside a flattened `JobDetail` whose nested `JobInfo` already contains `state`/`jobId`, and also returns `completedInline`, `pollAfterMs`, `detailAvailable`, timestamps, command metadata, and default lifecycle flags on routine calls.
- Split responsibilities instead of using one rich `JobInfo` shape everywhere: durable Job/history records remain rich for SQLite/restart recovery/Inspector, while tool-facing responses use purpose-built slim views.
- Active ordinary responses should center on `jobId`, `state`, `elapsedMs`, plus bounded stdout/stderr tails only when the caller requested ordinary detail (initial exec or `job.get(waitOnly=false)`). `waitOnly` omits tails and returns only identity/state/elapsed time.
- Terminal responses should center on `jobId`, `state`, `durationMs`, and kind-specific outcome data: process `exitCode` + stdout/stderr tails; MCP/skill `result` or `error`; truncation metadata only when truncation actually occurred.
- Remove routine response duplication/noise: `agentId`, `createdAt`, `updatedAt`, `finishedAt`, `completedInline`, `pollAfterMs`, `detailAvailable`, duplicate outer `status`, default `cancelRequested:false`, and repeated command forms (`program` + `args` + `commandPreview`) should not all be emitted on ordinary calls.
- `group` is useful on ordinary `job.get`/history/list responses but is unnecessary in `waitOnly`; `kind` is primarily useful in unified list/history/TUI summaries rather than a tool response whose surface already implies the kind.
- Keep raw timestamps and richer provenance/cancel/evidence metadata in durable records and Inspector/detail paths rather than making every tool call pay their token cost.
- Fix time semantics as part of this work: `startedAt` becomes optional and is set only when execution actually starts; `createdAt` remains admission time; `finishedAt` marks terminal transition. Derive `elapsedMs` for active Jobs and `durationMs` for terminal Jobs from true execution timestamps.
- Slimming applies across the Managed Job family, not just `process.exec`: `skills.run` reuses `JobResponse`; `mcp.callTool` reuses `mcp_job_response -> JobResponse`; `process.batch` returns a full `JobInfo` per child; `mcp.batch` returns a flattened full `JobDetail` per child; `job.cancel` currently returns full `JobDetail`. File/tmux/non-Job tools should not be pulled into this refactor merely for consistency.
- `job.cancel` should become cancellation-focused rather than detail-focused: preserve `jobId`, `state`, cancellation outcome/evidence and relevant error, without returning unrelated Job detail fields.

## `job.list` Direction
- `job.list` should use a purpose-built summary row rather than `JobInfo`.
- Minimum useful row: `jobId`, optional `group`, `state`, `createdAt`, optional `startedAt`, optional `finishedAt`. `state` is required because a null `finishedAt` cannot distinguish queued/waiting/running/cancel-requested states.
- `kind` is required in `job.list` summaries because the planned Process TUI will mix process/skill/MCP Jobs in one view and select a different renderer per kind, while `group` partitions the same unified stream into columns/tabs.
- The field name is frozen as `group`.
- Every Managed Job admission surface accepts optional `group`: `process.exec`, `process.batch`, `skills.run`, `mcp.callTool`, and `mcp.batch`. Batch-level `group` is inherited by every child Job; per-child group overrides are out of scope for the first version.
- `job.get`, `job.list`, and `job.cancel` do not accept `group` because the Job already owns the metadata; `job.list` may filter by group separately when query semantics are finalized.
- No stdout/stderr/result, command arguments, audit/provenance, cancellation evidence, or other Inspector fields belong in `job.list` rows.
- Freeze first-pass query parameters to optional single-value `group`, `kind`, `state`, plus `limit` and opaque `cursor`. Keep filtering exact and simple; multi-value filters can be added later only if a concrete caller needs them.
- Use cursor pagination rather than offset pagination because new Jobs can arrive while the TUI is scrolling. Order rows by `createdAt DESC, jobId DESC`; encode the last row's stable sort tuple in an opaque cursor.
- Default `limit` to 50 and cap it at 100.
- Return `{ jobs, nextCursor? }`; omit `nextCursor` at end of results. Do not add redundant `hasMore` or expensive/low-value `total` counts.
- Active-group discovery is a separate concern from history pagination. Do not overload `job.list` for it; if model-facing active-group discovery is needed, expose a dedicated small surface (for example `job.groups`) or bounded diagnostics elsewhere.

## Persistence Write Policy
- SQLite cannot be terminal-only in the literal sense because restart recovery needs evidence that a non-terminal Job existed. Insert a durable row at Job admission, then avoid continuous active-state/tail writes, and write the complete terminal snapshot once on terminal transition.
- Live Process TUI reads active Job state/output directly from in-memory `ManagedJob` state; SQLite is not the live-refresh transport.
- If the Agent restarts before terminalization, any persisted admitted-but-nonterminal row is converted to `unknown_after_restart`. Exact intermediate active state/output before the crash is not required in v1.
- This intentionally avoids periodic stdout/stderr persistence and write amplification while preserving restart truthfulness and final history.

## Process TUI Baseline Reality
- The repository has reusable TUI primitives in `src/tui/`: `TerminalSession`/tick+key+resize runtime, theme, surface/footer/inspector widgets, and form-kit helpers.
- `src/config_tui/` is a real application-level event/render loop with navigation/state, but it is tightly coupled to the config wizard and should be used as a pattern/reference rather than treated as a generic application shell.
- There is currently no Process TUI application baseline: no Process-specific app/state, group tabs/columns, mixed-kind list renderer, stable selection model, live refresh/query bridge, history cursor loading, or Inspector coordination.
- Phase 4 therefore starts by establishing a Process TUI application skeleton. Reuse existing primitives directly; do not prematurely extract a universal TUI app framework unless Process work demonstrates a genuinely shared abstraction.

## Current completed-Job retrieval behavior
- Completed/terminal Jobs remain retrievable through `job.get` only while retained in the in-memory `AppState.jobs` map.
- Current pruning keeps terminal Jobs for at most 24 hours and at most 100 terminal entries, pruning the oldest/expired terminal Jobs.
- Once pruned, `job.get` returns `job_not_found` for a same-boot id; an id recognized as belonging to a previous boot returns `job_lost_after_restart` because local Job state is not currently restored after restart.
- Under the new SQLite history design, `job.get` should transparently fall back to persisted Job history so retained terminal results remain retrievable across restart for the frozen history-retention window.
- Once SQLite history is authoritative, terminal Jobs only need a short in-memory hot-cache lifetime. Freeze the target at 5 minutes plus the existing 100-terminal-entry cap; this comfortably exceeds the 30-second bounded wait/batch aggregation window while releasing terminal `ManagedJob` runtime state quickly. Older terminal reads fall through to SQLite.

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Initial interpretation treated the readable identifier mainly as a per-Job display label | User clarified the actual purpose is cross-command classification across concurrent collaboration/workstreams; planning now uses grouping semantics. |

## Refinement repository evidence
- The active plan pointer resolves to this Job plan. At refinement entry, `git status --short` showed only `.planning/.active_plan` plus the two local planning directories; no product source/test/config files were modified.
- `AppState.jobs` is the live in-memory authority. `jobs.rs` owns ManagedJob admission, refresh, wait, detail, cancellation, finalization, and the current 24h/100 terminal-memory prune behavior.
- `JobInfo`, `JobDetail`, `JobResponse`, `JobBatchResponse`, `JobListRequest`, `JobGetRequest`, process/MCP/skill request types, and HubCommand variants are shared through `agentic-gpt-protocol`; changing timing/group/list/response contracts therefore has cross-surface impact.
- `stdio_server.rs` exposes local/tunnel MCP schemas and adapters for `process.exec/batch`, `skills.run`, `mcp.callTool/batch`, and `job.get/list/cancel`. Hub MCP/HTTP routes forward the shared protocol types to the Agent and maintain a separate in-memory cached Job snapshot fallback.
- Hub persistence already uses bundled `rusqlite`; the Agent crate does not currently depend on `rusqlite`, so Phase 3 must add that dependency or a shared workspace dependency before introducing local Job history.
- Current local Job finalization ignores audit-write failures (`let _ = write_audit(...)`), so existing observability evidence is best-effort and does not currently block core process completion. This is relevant but not by itself sufficient to decide the stronger durable-history failure contract.
- Live-plus-history listing cannot be implemented as a SQLite-only query because active rows are intentionally not continuously updated. External `job.list` semantics require live in-memory state to override the persisted admission snapshot for the same `jobId`, with no duplicate rows.
- Repository path conventions are mixed: security/audit JSONL lives under `workspaceRoot`, while Hub's SQLite database lives under `~/.agentic_gpt/hub.sqlite3`. The Agent supports custom `--config` paths and therefore may have multiple local identities/configurations on one machine; Job-history location/isolation is an operational decision rather than a repository fact.

## Remaining contract gaps
- **Q-01 (resolved by D-21): Job history location/isolation.** Private-state migration is now implemented and verified. Job history uses `AppState.private_state.root.join("jobs.sqlite3")`, giving one private durable DB per Agent identity/state root; the ~512 MiB soft cap is therefore per-agent DB.
- **Q-02 (resolved by D-20): persistence failure behavior.** History persistence is fail-open: inability to initialize or write durable Job history does not change the real process/skill/MCP Job outcome and does not block execution solely for observability durability.
- The private-state prerequisite guarantees the DB is outside `workspaceRoot`; path-safe IDs map directly to `<agentId>`, while wider legacy Hub identities use a stable safe directory key. Job code must consume `AppState.private_state.root` rather than reimplement this mapping.
- Invalid `group`/cursor handling, SQLite schema layout, cursor encoding, indexes, transaction/helper structure, and exact private type names can use recommended defaults or implementation discretion because they do not require a new product choice.

## Decision rationale: D-20 — fail-open Job history
- Startup/open/schema/history failures put Job history into an explicit degraded/live-only health state with bounded warning diagnostics; the Agent remains able to execute Jobs.
- A successful/failed/cancelled Job remains that actual execution outcome even if the terminal history snapshot cannot be written. Persistence failure is separate observability state, not Job failure.
- A terminal Job that has not been durably snapshotted must not enter the ordinary 5-minute hot-cache eviction path. Keep the rich in-memory result pending persistence and retry on a bounded/backoff basis; after persistence succeeds, normal hot-cache retention applies.
- Degraded retention still needs a hard memory bound. If persistent storage remains unavailable long enough that never-persisted terminal Jobs must be dropped, emit an explicit high-signal warning/error event rather than treating it as ordinary retention cleanup. Exact private retry schedule and degraded-memory policy are implementation discretion so long as they are bounded and tested.
- If admission INSERT fails but execution proceeds, a later terminal write may upsert the complete terminal snapshot so history can recover when storage becomes available. If the Agent crashes before any durable admission exists, restart recovery cannot reconstruct that Job; this is an accepted fail-open durability tradeoff.
- Only errors clearly diagnosed as database corruption may trigger isolation of the damaged DB (for example timestamped rename) followed by a clean replacement. Permission denied, disk full, generic I/O errors, or transient lock errors must not rename/delete the database because the file may be healthy.
- History health should be externally inspectable through bounded diagnostics/TUI status so degraded persistence is not silent. Exact field placement can follow existing `agent.info`/future TUI conventions without changing Job outcome shapes.

## Final Handoff Readiness
- **Scope/ownership:** PASS. This plan owns Managed Job grouping, history, query/polling, slim response and Hub parity contracts; Process TUI implementation and non-Job tools are explicitly out of scope.
- **Repository evidence:** PASS. Current runtime authority is `AppState.jobs`/`jobs.rs`; shared contracts live in `agentic-gpt-protocol`; stdio adapters expose the Managed Job surfaces; Hub forwards/caches shared Job snapshots; Agent-side `rusqlite` is new while Hub already uses 0.32 bundled; `AppState.private_state.root` is implemented by `a50655f`.
- **Open decisions:** PASS, none. Q-01→D-21 and Q-02→D-20 closed the last blockers; all earlier group/list/wait/response/recovery/retention choices were user-confirmed during the design rounds.
- **Inputs/outputs/defaults/versioning:** PASS. D-01–D-05 and D-13–D-19 freeze request fields, validation, defaults, pagination and exact routine response behavior. Existing tool names update in place; request additions are optional, and the confirmed slim output contract intentionally replaces the old rich routine shapes.
- **Lifecycle/concurrency/idempotency/cancellation/failure:** PASS. D-09–D-14 and D-19–D-20 freeze admission/start/finish time, wait behavior, memory-vs-history authority, restart truthfulness, cancellation evidence, and outcome-independent persistence failure.
- **Persistence/migration/retention/cleanup/recovery/rollback:** PASS. D-06–D-11 and D-20–D-21 freeze SQLite placement, new-store initialization (no legacy Job-history migration exists), write policy, recovery, retention, degraded mode, corruption isolation and no-vacuum behavior.
- **Security/trust:** PASS. No new network/auth surface is introduced; DB is private per-agent state, group validation rejects controls, and persisted detail must not add raw env values/secrets. Existing bounded command/result provenance remains private-state data.
- **Operations/observability:** PASS. Retention/cap/cleanup cadence and history-degraded health are frozen; exact private retry schedule/diagnostic placement is delegated but must remain bounded and externally inspectable.
- **Requirement→phase/acceptance mapping:** PASS. Phase 3A–3F orders protocol, persistence, runtime, adapters, Hub parity and verification; `task_plan.md` now carries observable acceptance criteria for each frozen surface.
- **Implementation discretion:** PASS. Only locally equivalent schema/helper/retry/cursor/test organization choices are delegated; none may alter D-01–D-21.
- **Planning consistency:** PASS after final write-through: all three files now state zero blockers and `implementation_ready`.
- **No product code during final refinement:** PASS. At handoff check `git diff --name-only -- crates Cargo.toml Cargo.lock docs` was empty; HEAD was `a50655f`, with only planning paths dirty/untracked.
- **N/A:** legacy Job-history data migration (feature is new); new authentication/authorization/network policy; physical SQLite shrink guarantees; output spool/compression; Process TUI construction.

## Implementation sequence evidence
- **3A Protocol/domain first:** shared `JobInfo`/request/response types affect Agent stdio and Hub, so downstream code should not invent parallel temporary semantics.
- **3B History store second:** runtime/list/get behavior depends on a tested durable store and D-20 degraded mode; isolate those mechanics before response adapters.
- **3C Runtime third:** lifecycle timing, persistence hooks, hot cache and live+persisted merge become one coherent authority before serializers consume them.
- **3D Tool adapters fourth:** once runtime semantics are real, exact slim JSON and `waitOnly` can be asserted without mocks encoding the wrong model.
- **3E Hub parity fifth:** update forwarding/cache/reporting after shared/local shapes stabilize, then catch cross-crate compatibility in integration tests.
- **3F Verification last:** docs + workspace-wide lint/test/diff review freeze the consumer contract before the separate TUI plan starts.

## Phase 3A implementation findings
- `ExecRequest`, `BatchExecRequest`, `SkillRunRequest`, `McpCallToolRequest`, and `McpBatchRequest` are the five shared admission payloads that should receive parent-level optional `group`; `ExecElement` and `McpBatchCall` intentionally remain without child overrides.
- Keep request `group` as `Option<String>` rather than a custom rejecting Deserialize newtype. A protocol-level normalizer/validator can enforce D-02 while letting stdio/Hub adapters later map failures to the frozen structured `{code,message}` behavior instead of losing the error behind generic serde invalid-parameter handling.
- Phase 3A should add purpose-built slim view/query response types while retaining the current rich `JobDetail`/`JobResponse` structs temporarily. Deleting/replacing the old response structs now would force adapter/runtime changes before Phases 3C/3D; adding the target types first preserves the planned dependency order and still lets protocol serde tests freeze the eventual JSON shapes.
- `JobInfo` remains the rich retained domain record and therefore should gain optional `group` plus optional true `startedAt`; runtime will correct assignment semantics in Phase 3C.
- `JobListRequest` needs exact `group/kind/state`, `limit`, and opaque `cursor`; a dedicated summary/response type prevents `job.list` from reusing rich `JobInfo`.
- Workspace compile fallout is bounded enough to keep Phase 3A green: request structs have a few dozen Rust literals that can mechanically receive `group: None`, while `JobInfo.started_at` has only two production duration calculations plus constructors/tests that need Option-aware compatibility edits. This does not require implementing history/runtime behavior early.
- Special literal inspection confirmed the intended compatibility policy: current MCP/process JobInfo constructors still represent admission-time start and should use `started_at: Some(now)` until Phase 3C changes lifecycle timing; MCP batch tests using struct update syntax inherit the parent request's new group field and need no child override; direct request helpers/tests simply add `group: None`.
- The two existing duration consumers are audit-record duration in `jobs.rs` (`u128`) and terminal-event text in `stdio_server.rs` (`i64`). For Phase 3A compatibility they should map optional `started_at` and fall back to zero when absent; Phase 3D will later expose frozen response `durationMs` only when a true start exists rather than using this legacy display/audit fallback.

## Phase 3A diff review
- Full diff review confirms the intended boundary: `agentic-gpt-protocol` contains the new request/domain/slim-view contracts and tests; the other nine Agent/Hub files contain only compile-compatibility `group: None`, `cursor: None`, `wait_only: false`, `started_at: Some(now)`, and Option-aware legacy duration handling.
- No existing caller propagates a real group yet, no Job execution start transition was moved yet, and no old rich routine response adapter was replaced yet. Those observable behavior changes remain assigned to Phases 3C/3D as planned.
- Existing rich `JobDetail`/`JobResponse`/legacy batch response types remain intact alongside the new target slim types, preventing Phase 3A from forcing premature adapter migration.

## Resources
- `crates/agentic-gpt-protocol/src/lib.rs` — `JobInfo`, `McpBatchChildResponse`, protocol batch call ids.
- `crates/agentic-gpt/src/mcp.rs` — MCP batch preparation, registration, response construction, batch call id validation.
- `crates/agentic-gpt/src/jobs.rs` — managed MCP/job retention of `batch_call_id` and Job detail.
- `crates/agentic-gpt/src/file_ops.rs` — `file.batch` operation id echo/group/audit behavior.
