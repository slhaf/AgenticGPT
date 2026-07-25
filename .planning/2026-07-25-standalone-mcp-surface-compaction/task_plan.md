# Task Plan: Agentic Standalone MCP Surface Compaction

## Goal
Reduce the public MCP schema exposed by each Tunnel-backed standalone Agent without introducing a generic RPC tool or weakening device isolation. The Tunnel stdio worker will expose a compact, role-correct tool surface; ordinary process execution and managed sessions will share one durable lifecycle; Room-only bootstrap remains absent from Normal; and every standalone tool call plus Hub-reporting connection transition will produce safe local logs.

## Workflow State
- **Stage:** delivered
- **Current role:** designer
- **Implementation authorized:** no; implementation and independent verification are complete
- **Active plan:** `2026-07-25-standalone-mcp-surface-compaction`
- **Current phase:** Phase 9 — Independent Repair Verification and Delivery (complete)
- **Entry phase after handoff:** none
- **Open blocking decisions:** none
- **Implementation baseline:** `c987e32`
- **Repair commit:** `03eb501`
- **Next action:** none for this plan; push/release only when separately requested
## Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| Rust borrow conflict while finalizing after tail-buffer guards | 1 | Scoped tail-buffer reads before invoking the mutable terminal finalizer. |
| Cargo focused-test invocation rejected multiple positional filters | 1 | Use one `sessions::tests` filter for the focused Phase 3 test run. |
| Stdio regression tests still asserted the pre-compaction 29/39 tool counts and Normal legacy calls | 1 | Updated tests to the frozen 18/30 surface and `process.list`; added process lifecycle coverage. |
| Cargo focused-test invocation rejected multiple positional filters | 1 | Ran the two focused suites separately with one filter each; no test code or contract change was needed. |
| Workspace standalone supervisor test exited with `runtime_directory_unavailable` before worker startup | 1 | Reproduced with isolated HOME; runtime setup passed there. The fixture then exposed stale `agentId` in its process call, which was removed; the isolated workspace suite passed. |
| Phase 7 self-review declared delivery complete while seven frozen-contract gaps remained | 1 | Reopened the same active plan, preserved D-01–D-20, and appended focused repair plus independent verification phases. |

## Scope

### In scope
- Tunnel stdio MCP tool descriptors, dispatch, capability filtering, instructions, annotations, tests, and documentation.
- Remove caller-supplied `agentId` from every Tunnel stdio input schema and inject the current configured Agent identity internally where shared protocol types still require it.
- Remove caller-supplied `confirmMethod` from Tunnel process schemas; standalone uses the configured confirmation provider.
- Replace the Tunnel `process.exec` plus `session.*` split with one managed-process lifecycle under `process.*`.
- Rebuild Tunnel `process.batchExec` as a batch launcher over the same managed-process lifecycle used by `process.exec`; each element receives its own retained `sessionId`.
- Reuse the managed-process lifecycle and follow-up APIs for `skills.run`.
- Compact the Tunnel downstream MCP, skills, and tmux surfaces without collapsing them into an untyped `operation + arguments` RPC.
- Expose `bootstrap` and `bootstrap.read` only in Tunnel Room.
- Add safe local start/completion/failure logs for every Tunnel tool call and terminal lifecycle logs for managed processes.
- Add explicit Hub-reporting connected/disconnected/retry logs for standalone workers.
- Add exact tool-set and serialized-schema regression tests.
- Update standalone runtime documentation and connector examples.

### Out of scope
- Redesigning the Hub full or coordinator MCP surfaces.
- Removing `agentId` from Agent configuration, Hub routing, protocol persistence, audit metadata, reporting records, or result metadata.
- Removing legacy Hub `session.*`, tmux, skills, bootstrap, REST, OpenAPI, or `HubCommand` variants.
- A central fleet gateway or cross-Agent routing from a standalone connector.
- Merging tmux into the managed-process subsystem.
- Hub-reporting configuration hot reload.
- Full ChatGPT → Tunnel → Agent → Tunnel round-trip latency instrumentation.
- The Server-specific confirmation/network issue.
- KMP console work.
- A generic `agent.invoke` or free-form RPC tool.

## Repository Baseline
- Repository: `/home/slhaf/Projects/AgenticGPT`
- Baseline commit: `3dd8fa2` (`v0.7.0`)
- Baseline branch: `main`, clean and equal to `origin/main` at planning start.
- Existing standalone implementation: `crates/agentic-gpt/src/stdio_server.rs`.
- Shared local command dispatch: `crates/agentic-gpt/src/local_service.rs`.
- Existing one-shot execution: `crates/agentic-gpt/src/exec.rs`.
- Existing managed lifecycle: `crates/agentic-gpt/src/sessions.rs`.
- Shared wire types and Hub commands: `crates/agentic-gpt-protocol/src/lib.rs`.
- Hub MCP compatibility surface: `crates/agentic-gpt-hub/src/mcp_server.rs`.

## Frozen Architecture Boundary

The target of this plan is the Tunnel stdio public surface. Hub routing remains a separate compatibility surface.

| Surface | Identity routing | Result of this plan |
|---|---|---|
| Tunnel standalone connector | Connector already identifies exactly one worker | Compact surface; no input `agentId` |
| Hub full MCP | Hub routes among registered Agents | Preserve current tools and required `agentId` |
| Hub coordinator MCP | Hub-native aggregation only | Preserve exact eight-tool surface |
| Agent internal protocol/audit/reporting | Uses configured Agent identity | Preserve internal/result `agentId` metadata |

A standalone call cannot select another Agent. Supplying `agentId` or `confirmMethod` to a Tunnel tool is rejected as an unexpected argument rather than ignored or treated as routing.

## Frozen Tunnel Tool Surfaces

### Tunnel Normal — exactly 18 tools

#### Managed process
- `process.exec`
- `process.batchExec`
- `process.get`
- `process.kill`
- `process.list`

#### Downstream MCP
- `mcp.list`
- `mcp.callTool`

#### Skills
- `skills.list`
- `skills.read`
- `skills.setActive`
- `skills.install`
- `skills.install.get`
- `skills.install.cancel`
- `skills.run`

#### tmux
- `tmux.sessions`
- `tmux.panes`
- `tmux.exec`
- `tmux.pasteText`

### Tunnel Room — exactly 30 tools
Tunnel Normal plus:
- `bootstrap`
- `bootstrap.read`
- Existing ten `room.diary.*` and `room.notebook.*` tools unchanged.

### Removed from Tunnel `tools/list`
- `session.start`, `session.list`, `session.inspect`, `session.wait`, `session.kill`
- `mcp.listServers`, `mcp.listTools`
- `skills.search`, `skills.active`, `skills.activate`, `skills.deactivate`
- `tmux.listSessions`, `tmux.listPanes`, `tmux.capturePane`, `tmux.createSession`, `tmux.closeSession`
- `bootstrap`, `bootstrap.read` from Normal only

No hidden generic dispatch tool replaces these names. Hub full compatibility tools remain advertised and callable through the Hub connector.

## Managed Process Contract

### `process.exec`
Input:
- `program` — required executable name/path.
- `args` — optional direct argument vector, default empty.
- `workingDirectory` — optional working directory subject to current path policy.
- `needConfirm` — optional, default false; normal policy may still require confirmation.
- `waitSeconds` — optional inline wait, default 5, maximum 30.

Behavior:
1. Strictly decode and validate the Tunnel request shape. Unknown fields, including legacy `agentId` and `confirmMethod`, return invalid params before id allocation.
2. Allocate a managed `sessionId` and register a `starting` lifecycle record. Registration and active-capacity admission occur atomically so concurrent starts cannot oversubscribe `limits.maxActiveSessions`.
3. Resolve working directory, path policy, execution preflight, and policy decision exactly once against that registered record. Rejection becomes a retained terminal session.
4. If confirmation is required, transition to `waiting_confirmation`; denial/timeout/cancellation finalizes the same retained session.
5. Spawn and attach the child, or finalize the retained session on spawn failure.
6. Wait up to `waitSeconds` for a terminal state.
7. When terminal within the wait, return `completedInline=true` and final bounded output.
8. When still `starting`, `waiting_confirmation`, or `running`, return `completedInline=false`, `pollAfterMs=1000`, `sessionId`, and the current session snapshot.
9. Reaching the inline wait never kills the process and is not reported as an execution timeout.
10. Active process capacity uses the existing `limits.maxActiveSessions` boundary; default remains 4. Terminal records retain the existing 24-hour / newest-100 policy.

Public Tunnel response wrapper is shared with `skills.run`:
- `sessionId`
- `completedInline`
- `pollAfterMs`
- `session` — current/final `SessionInfo`.

The wrapper does not repeat `agentId`. The nested existing `SessionInfo.agentId` remains compatibility metadata and is not caller routing. Tunnel `skills.run` uses the same wrapper even if the preserved Hub protocol response type still contains a top-level `agentId`.

The public identifier remains `sessionId` because it is already established by `skills.run`, Hub protocol, audit, reporting, and retained session state. No second `processId` vocabulary is introduced.

### `process.get`
Input:
- `sessionId` — required.
- `waitSeconds` — optional, default 0, maximum 30.

Behavior:
- Replaces both inspect and wait.
- Returns the `SessionInfo` object directly, matching existing inspect/wait result semantics; it does not add a second `{session: ...}` wrapper.
- Returns immediately at zero wait.
- With positive wait, returns when terminal or at the bounded deadline.
- Unknown ids return `{error: {code: "session_not_found", message: ...}}`.

### `process.kill`
- Requires `sessionId`.
- Cancels waiting confirmation or kills the active child.
- Idempotent terminal behavior follows the current session implementation; unknown ids return not found.
- Returns the resulting `SessionInfo` directly.
- Releases skill leases and writes terminal audit/log state exactly once.

### `process.list`
- Takes no arguments.
- Returns `{sessions: [SessionInfo, ...]}` to preserve the established session vocabulary and current Tunnel list envelope.
- Includes active `starting`, `waiting_confirmation`, and `running` managed processes only, matching existing `session.list` behavior.
- Retained terminal processes remain addressable through `process.get` by known id.

### `process.batchExec`
- Is a batch launcher over the same managed-process primitive as `process.exec`, not a separate one-shot executor and not one batch-owned child process.
- Every element receives its own `sessionId`, retained terminal state, audit record, terminal log, and optional terminal `SessionUpdate`.
- Input keeps `elements`, `workingDirectory`, and `needConfirm`; removes `agentId` and `confirmMethod`; adds one optional batch `waitSeconds` with the same default 5 / maximum 30 bound. Empty `elements` preserves current behavior: return `completed` with an empty `results` array and create no sessions. Any non-empty batch larger than currently available aggregate session capacity is rejected before id allocation.
- Elements are started concurrently subject to the standalone `maxActiveSessions` capacity contract; the legacy Hub full `BatchExec` path and `maxConcurrentTasks` behavior remain unchanged.
- The batch waits against one shared deadline, then returns ordered per-element process responses. A running element is never killed merely because the batch inline wait expires.
- There is no `process.batchGet` or batch-owned lifecycle. Follow-up inspection and cancellation use each element's `sessionId` through `process.get` and `process.kill`. `batchId` is correlation metadata only.
- Admission is all-or-reject: normalize defaults and working directories, validate every element, evaluate policy, and then reserve aggregate active-session capacity plus insert all `starting` records atomically before any child is spawned. No concurrent single or batch start may consume the reserved slots between check and insertion.
- If any element fails validation/policy or aggregate capacity is insufficient, no managed session is created and the batch returns ordered per-element rejection evidence.
- When confirmation is required, request one batch confirmation after successful preflight and before id allocation; denial/timeout starts nothing.
- All-or-reject describes admission only. After records are inserted, operating-system spawn and runtime execution are not transactional. A sibling spawn failure finalizes only that element as retained `spawn_failed`; already spawned siblings continue independently and are not rolled back.

Public response:
- Outer fields: `batchId`, `status`, `results`, `startedAt`, `updatedAt`. No `agentId`.
- `status`: `running` when at least one managed element remains active at the shared wait deadline; `completed` when all managed elements exit successfully; `partial_failed` when all managed elements are terminal and at least one failed or was killed; `rejected` when admission creates no sessions.
- Each ordered result contains `index`, `program`, `args`, optional `workingDirectory`, `outcome`, optional `process`, and optional `rejectReason`.
- `outcome=managed` requires `process`; the embedded process response contains `sessionId`, `completedInline`, `pollAfterMs`, and `session`, but no duplicate `agentId`.
- `outcome=rejected` identifies the element whose validation/policy/capacity/confirmation evidence caused all-or-reject admission failure.
- `outcome=skipped` identifies an otherwise admissible element that was not started because another element rejected the batch.

## Managed Lifecycle, Audit, and Output Contract

The current `ManagedSession` implementation is generalized into a transport-neutral managed-process lifecycle rather than copied into stdio dispatch.

Required properties:
- Generic audit context for ordinary process runs and skill runs; no skill-only audit branch.
- Exactly one terminal audit record for success, failure, rejection, cancellation, and spawn failure.
- Request source distinguishes at least `tunnel:process.exec`, `tunnel:skills.run`, and preserved Hub sources.
- Policy decision, confirmation result, exit code, bounded duration, truncation, and reject reason remain captured.
- Managed process terminal transition is emitted once even if multiple `process.get` calls refresh the same record.
- Managed records remain in-memory only. This plan introduces no durable restart recovery, database persistence, or cross-worker session reattachment; worker restart invalidates retained ids and preserves existing shutdown semantics.
- Hub reporting receives the initial snapshot and a later terminal snapshot when reporting is enabled; reporting failure never changes local execution.
- Do not reduce the old one-shot `process.exec` 64 KiB-per-stream bound. The implementer may raise the shared managed tail bound to 64 KiB or support source-specific bounded tails.
- `process.exec` quick completion, long execution, confirmation wait, and `skills.run` all share one response helper and one wait helper.

The old one-shot executor remains available for Hub `process.exec` and bounded `process.batchExec` compatibility unless an internal refactor can preserve those public contracts exactly.

## Downstream MCP Contract

### `mcp.list`
- Optional `serverId`.
- Omitted: return the existing `{servers: [McpServerSummary, ...]}` shape.
- Present: return the existing `{tools: [Tool, ...]}` shape for that configured server; do not add a mode tag or extra wrapper.
- No `agentId`.
- Invalid/disabled server behavior preserves existing structured errors.

### `mcp.callTool`
- Keeps required `serverId` and `toolName`, optional object `arguments`.
- No `agentId`.
- Existing confirmation, temporary allow, bounded result, and error behavior remain.

## Skills Contract

### `skills.list`
Optional input:
- `query`
- `limit`
- `activeOnly`

Behavior:
- No query: list all valid skills.
- Nonblank query: current case-insensitive search semantics and bounds.
- `activeOnly=true`: preserve current active-state semantics, including stale/missing active entries and activation metadata rather than silently dropping them.
- One stable response envelope is used across all modes: `{ skills, activeSkills, warnings }`.
- `skills` contains valid scanned skill summaries after optional query filtering; with `activeOnly=true`, it contains only valid active summaries.
- `activeSkills` always contains the complete activation-record view, including stale/missing active entries with `summary=null`; it is not query-filtered.

### `skills.setActive`
- Required `id` and boolean `active`.
- Returns the existing activation result shape `{id, active, changed, activatedAt?}`.
- `active=true` preserves current activate validation and default-skill tombstone removal.
- `active=false` preserves idempotent deactivate and default-skill tombstone behavior.

### Preserved skills tools
- `skills.read`
- `skills.install`
- `skills.install.get`
- `skills.install.cancel`
- `skills.run`

`skills.run` keeps skill path validation, activation requirement, update lease, installed digest audit, and optional `waitSeconds`; returned `sessionId` is managed through `process.get`, `process.kill`, and `process.list`.

## tmux Contract

### `tmux.sessions`
Input:
- `action` — required enum `list | create | close`.
- `name` — required by create/close.
- `cwd` — required by create.
- `needConfirm` — close-only, default true.

Behavior and output:
- `list` delegates to current list-sessions behavior and returns `{sessions: [...]}`.
- `create` retains path policy, idempotent existing-session result, and no implicit shell command execution; returns `{session, cwd, created}`.
- `close` retains confirmation and audit behavior; returns `{session, closed}`.
- Do not add a generic `result` field or action-tagged response envelope; the requested action already determines the preserved result shape.
- Action-incompatible or missing fields return a structured validation error; they are never silently ignored.
- Tool annotations are conservative because one tool includes destructive close: read-only false, destructive true, open-world true.

### `tmux.panes`
Input:
- `action` — required enum `list | capture`.
- `session` — optional list filter.
- `target` — required by capture.
- `lines` — capture-only, default 160 and existing maximum.

Both operations are read-only. Conditional fields are validated explicitly.
- `list` returns the existing `{session, panes}` shape.
- `capture` returns the existing `{target, lines, capture}` shape.
- Do not add an action tag or generic result wrapper.

### `tmux.exec` and `tmux.pasteText`
- Keep current separate tools and safety distinction.
- Remove only `agentId` from Tunnel input schemas.
- Preserve shell-pane vs non-shell-pane checks, structured argv, quoting, path policy, confirmation, bounded post-submit snapshot, and audit.

## Bootstrap and Capability Contract

- Tunnel Normal capabilities set `bootstrap=false`.
- Tunnel Normal does not advertise or directly dispatch `bootstrap` or `bootstrap.read`.
- Tunnel Room advertises both existing tools unchanged.
- The two bootstrap tools remain separate: manifest and guide-read have different required inputs and result shapes; merging them saves one descriptor but introduces ambiguous conditional I/O.
- Hub Room and Hub full compatibility aliases remain unchanged.
- Tunnel skills remain available in both Normal and Room.

## Standalone Identity and Confirmation Contract

- No Tunnel stdio input schema contains `agentId` or `agent_id`.
- `stdio_server` removes `require_agent` and `require_optional_agent` as public routing checks.
- The adapter reads the configured Agent id and injects it only when constructing shared protocol requests/results.
- Passing `agentId` to a Tunnel call fails as an unexpected argument.
- No Tunnel process schema contains `confirmMethod`.
- Passing `confirmMethod` fails as an unexpected argument.
- Standalone confirmation provider selection comes from Agent configuration; `needConfirm` and policy still determine whether confirmation is required.
- Hub full retains caller-selected `agentId` and current confirmation override compatibility.

## Local Logging Contract

Every Tunnel stdio `tools/call` emits safe stderr lifecycle logs:

- `tool_call_started`
- `tool_call_completed`
- `tool_call_failed`

Minimum fields:
- `runId`
- `tool`
- `profile`
- `status`
- `durationMs` on terminal call log
- `sessionId` when returned
- `exitCode` when present
- bounded structured `errorCode` when present

Default logs never include:
- full arguments or results
- command preview
- working directory
- stdout/stderr
- skill content
- downstream MCP arguments
- secrets or confirmation tokens

For a managed process that outlives the initiating call, emit one later process-terminal log with `sessionId`, terminal state, duration, exit code/reject code, and source. Do not duplicate terminal logs during repeated inspection.

Hub-reporting logging:
- Log a successful reporting connection after transport establishment and Hello setup, including transport and Agent id but no secret.
- Log disconnection/normal close and failures distinctly.
- Preserve bounded retry logs.
- Reporting logs are absent when reporting is disabled.

Worker stdout remains exclusively MCP protocol output.

## Schema Footprint Budget

Measured from the real v0.7.0 hidden stdio worker using compact JSON serialization of `tools/list`:

| Profile | Current tools | Current tool-array bytes | Current summed input-schema bytes |
|---|---:|---:|---:|
| Normal | 29 | 15,932 | 8,345 |
| Room | 39 | 21,664 | 11,380 |

Projection using the frozen target schemas:

| Profile | Target tools | Projected tool-array bytes | Projected input-schema bytes |
|---|---:|---:|---:|
| Normal | 18 | 10,775 | 5,843 |
| Room | 30 | 17,255 | 9,104 |

Projected reductions:
- Normal: 37.9% fewer tools, 32.4% fewer total descriptor bytes, 30.0% fewer input-schema bytes.
- Room: 23.1% fewer tools, 20.4% fewer total descriptor bytes, 20.0% fewer input-schema bytes.
- Three Normal connectors plus one Room connector: approximately 69,460 → 49,580 raw descriptor bytes, a 28.6% reduction when all four complete surfaces are expanded.

Regression ceilings with implementation headroom:
- Normal: exactly 18 tools; compact tool array ≤ 11,500 bytes; summed input schemas ≤ 6,200 bytes.
- Room: exactly 30 tools; compact tool array ≤ 18,000 bytes; summed input schemas ≤ 9,600 bytes.

These are MCP serialization budgets, not a promise of an exact model-token ratio. Connector discovery may render or cache schemas differently.

## Compatibility and Delivery Contract

- This is an intentional breaking change to the Tunnel stdio public tool surface.
- Do not advertise legacy Tunnel aliases merely to preserve old schema names; that would defeat the compaction goal.
- After Agent restart, the connector must rediscover the target tool list.
- Hub full/coordinator exact surfaces and behavior remain regression-protected.
- Existing `HubCommand` variants may remain as internal and Hub compatibility primitives.
- Documentation must show new Tunnel names and explicitly distinguish them from Hub full names.
- No configuration migration is required.
- No database migration is required.

## Key Questions

| ID | Question | Blocking | Status | Resolution |
|---|---|---:|---|---|
| Q-01 | Retention for immediate managed-process failures | yes | resolved | Use retained terminal sessions after id allocation; strict decode failures create no session. |
| Q-02 | Stable response shape for merged `skills.list` | yes | resolved | Always return `skills`, `activeSkills`, and `warnings`. |
| Q-03 | Include `process.batchExec` in the managed-process lifecycle | yes | resolved | Yes. Batch becomes a launcher for multiple managed process executions; the old one-shot Tunnel batch path is replaced. |
| Q-04 | Batch admission atomicity and capacity behavior | yes | resolved | Preserve all-or-reject admission: validate every element, then atomically reserve capacity and insert all starting records before spawning any child. |
| Q-05 | Public response envelope for managed `process.batchExec` | yes | resolved | Preserve the old batch correlation/timing shape without `agentId`; each ordered element is tagged `managed`, `rejected`, or `skipped`, and managed elements embed the single-process response without a duplicate `agentId`. |
| Q-06 | Post-admission spawn failure and rollback behavior | yes | resolved | Successfully spawned siblings continue independently; the failed element becomes a retained `spawn_failed` terminal session. No sibling rollback is attempted. |

## Frozen Decisions

| ID | Decision | Status |
|---|---|---|
| D-01 | Scope compaction to Tunnel stdio public tools; preserve Hub full/coordinator compatibility surfaces. | confirmed |
| D-02 | Remove caller `agentId` from all Tunnel schemas and reject it when supplied. | confirmed |
| D-03 | Remove Tunnel `confirmMethod`; configured provider remains authoritative. | confirmed |
| D-04 | Replace Tunnel `session.*` while keeping Tunnel `process.batchExec` as the old bounded one-shot batch. | superseded by D-16 |
| D-05 | `process.exec` starts managed immediately, waits default 5/max 30 seconds, and never kills on inline wait expiry. Strict decode failures create no session; after a session id is allocated, path/policy/capacity/spawn failures become retained terminal sessions. | confirmed |
| D-06 | Retain public `sessionId` rather than introduce `processId`. | confirmed |
| D-07 | `skills.run` uses the same managed lifecycle and is controlled by `process.*`. | confirmed |
| D-08 | Merge downstream list-servers/list-tools into `mcp.list(serverId?)`. | confirmed |
| D-09 | Merge skills list/search/active and activate/deactivate while preserving stale-active semantics through a stable `{skills, activeSkills, warnings}` envelope. | confirmed |
| D-10 | Compact tmux to sessions, panes, exec, and pasteText; do not merge tmux into process. | confirmed |
| D-11 | Keep bootstrap/read separate but expose them only in Tunnel Room. | confirmed |
| D-12 | Add safe local tool-call, managed-process terminal, and Hub-reporting connection logs. | confirmed |
| D-13 | Enforce exact tool counts and bounded serialized schema size in tests. | confirmed |
| D-14 | Keep generic RPC/fleet routing, reporting hot reload, and full RTT diagnostics out of this plan. | confirmed |
| D-15 | No legacy Tunnel aliases are advertised after the change. | confirmed |
| D-16 | Tunnel `process.batchExec` launches multiple managed process executions, returning one retained `sessionId` per element and using `process.get/kill/list` for follow-up. | confirmed |
| D-17 | Managed batch admission is all-or-reject: normalize and validate all elements, resolve one batch confirmation, then atomically reserve capacity and insert all starting records before spawning children. | confirmed |
| D-18 | Managed `process.batchExec` preserves `batchId/status/results/startedAt/updatedAt` but omits `agentId`; each ordered result has input identity, `outcome=managed|rejected|skipped`, an optional embedded single-process response without duplicate `agentId`, and optional `rejectReason`. | confirmed |
| D-19 | New merged adapters inherit existing result bodies instead of inventing generic envelopes: direct `SessionInfo` for get/kill, `{sessions}` for list, `{servers}`/`{tools}` for MCP list modes, and existing action-specific tmux result objects. | confirmed |
| D-20 | Managed batch all-or-reject applies to admission only. After admission, each spawned process is independent; a sibling spawn failure finalizes only that element as `spawn_failed` and does not cancel already spawned siblings. | confirmed |
| D-21 | Reopen this delivered plan rather than create a separate repair plan. The public contract in D-01–D-20 is unchanged; only implementation conformance and verification are reopened. | confirmed |
| D-22 | Repair exactly the seven independently verified contract gaps and add direct regression tests for each before declaring delivery again. Do not redesign the 18/30 surface or broaden Hub scope. | confirmed |
| D-23 | Use one focused Phase 8 repair commit after targeted and full verification; Phase 9 is an independent review/delivery gate and creates another commit only when new evidence requires repair. | confirmed |

## Implementation Phases

### Phase 1: Repository and Runtime Audit
**Objective:** Establish exact current tool surfaces, shared execution lifecycle, schema footprint, profile capabilities, logging, and Hub compatibility boundaries.

- [x] Confirm clean `v0.7.0` baseline and completed previous runtime plan.
- [x] Inspect `stdio_server`, `local_service`, one-shot exec, managed sessions, skills run, tmux, runtime capabilities, Hub reporting, Hub MCP, and protocol types.
- [x] Measure real Normal/Room `tools/list` descriptor sizes through the hidden stdio worker.
- [x] Identify generic managed-session audit/reporting gaps before process fusion.
- **Status:** complete

### Phase 2: Contract Refinement and Handoff Freeze
**Objective:** Freeze the compact public surface and prevent schema reduction from erasing safety, audit, or compatibility semantics.

- [x] Freeze exact Normal and Room tool sets.
- [x] Freeze process wait, lifecycle, id, capacity, retention, output, audit, and response behavior.
- [x] Freeze MCP, skills, tmux, bootstrap, identity, confirmation, logging, schema-budget, and compatibility contracts.
- [x] Bound excluded follow-up work.
- [x] Pass handoff readiness audit.
- **Status:** complete

### Phase 3: Generalize the Managed-Process Lifecycle
**Objective / visible outcome:** One internal managed lifecycle safely supports ordinary Tunnel process execution and skills, with terminal audit/log/report behavior exactly once.

**Prerequisites:** Frozen D-01–D-03 and D-05–D-20; no product changes since the planning checkpoint.

**Relevant implementation:** `crates/agentic-gpt/src/sessions.rs`, managed-process helpers called by `hub.rs` skills execution and later by `stdio_server.rs`; shared audit/log/report helpers in `audit.rs` and `hub.rs`.

**Work boundaries:**
1. Generalize skill-only audit context in `sessions.rs` into a managed-process audit context carrying source, confirmation/policy data, and optional skill metadata.
2. Add one shared start-and-bounded-wait helper used by process and skills.
3. Preserve path/policy/preflight and asynchronous confirmation behavior.
4. Preserve bounded tails without reducing old process exec output capacity.
5. Ensure terminal transition audit, skill lease release, reporting snapshot, and one transport-neutral terminal-event hook happen once. Phase 3 does not freeze or implement the final stderr rendering format.
6. Preserve current Hub session behavior and protocol variants.

**Tests / acceptance:**
- Quick exit, nonzero exit, spawn failure, preflight rejection, confirmation allow/deny/timeout, cancellation before spawn, active-limit rejection, concurrent admission without oversubscription, long-running terminal transition, audit exactly once, terminal-event hook exactly once, session reporting best effort, skill lease release.

**Verification:**
- `cargo fmt --all -- --check`
- Focused Agent managed-process tests.
- `cargo test -p agentic-gpt --bin agentic-gpt`

**Commit:** `refactor(agent): unify managed process lifecycle`
- **Status:** complete

### Phase 4: Replace the Tunnel Process/Session Surface
**Objective / visible outcome:** Tunnel Normal exposes `process.exec/batchExec/get/kill/list`, with no `session.*`, input `agentId`, or `confirmMethod`.

**Prerequisites:** Phase 3 complete and committed.

**Relevant implementation:** `crates/agentic-gpt/src/stdio_server.rs`, `local_service.rs`, `exec.rs`, `sessions.rs`, and stdio/supervisor tests. Do not repurpose Hub `HubCommand::Exec` or `HubCommand::BatchExec`.

**Work boundaries:**
1. Add stdio-only compact request decoding and inject configured Agent identity internally.
2. Change Tunnel `process.exec` to the managed start/wait response contract.
3. Implement `process.get`, `process.kill`, and `process.list` over the shared lifecycle.
4. Replace the Tunnel one-shot batch dispatch with an all-or-reject managed batch launcher: preflight every element, perform at most one batch confirmation, then atomically verify/reserve aggregate capacity and insert all allocated `starting` records before spawning through the same managed primitive as `process.exec`; preserve the separate Hub full legacy batch path.
5. Remove direct Tunnel agent identity validation and reject unexpected routing/provider fields.
6. Keep Hub full process/session behavior unchanged.

**Tests / acceptance:**
- Exact process tool list and schemas.
- Quick inline, wait-expired running, later get, bounded get wait, kill, list, not found, caller `agentId` rejection, caller `confirmMethod` rejection, configured confirmation provider, ordered multi-session batch launch, concurrent atomic admission, shared wait deadline, post-admission sibling spawn failure with independent continuation, and per-element follow-up.
- Hub full exact process/session regression.

**Verification:** focused stdio protocol tests and Agent/Hub tests.

**Commit:** `feat(agent): compact standalone process tools`
- **Status:** complete

### Phase 5: Compact MCP, Skills, tmux, and Profile Capabilities
**Objective / visible outcome:** Complete the exact 18-tool Normal and 30-tool Room surfaces while preserving existing behavior behind fewer descriptors.

**Prerequisites:** Phase 4 complete and committed.

**Relevant implementation:** `stdio_server.rs`, `skills.rs`, tmux handlers, runtime capability model, shared protocol adapters, and exact Hub MCP surface tests.

**Work boundaries:**
1. Implement `mcp.list(serverId?)`; preserve `mcp.callTool` without input `agentId`.
2. Implement `skills.list(query?, limit?, activeOnly?)` with a stable response and stale-active semantics.
3. Implement `skills.setActive(id, active)` and point `skills.run` follow-up guidance to `process.*`.
4. Implement conditionally validated `tmux.sessions` and `tmux.panes`; keep exec/pasteText separate.
5. Move bootstrap descriptors from Normal into Room only and update runtime capability tests.
6. Update read-only/destructive/open-world annotations conservatively.
7. Do not change Hub full/coordinator tools.

**Tests / acceptance:**
- Exact tool names/counts per profile.
- Removed Tunnel names return method-not-found.
- MCP list modes, skills query/active/stale transitions, skill run via process management, tmux action validation/behavior, Normal bootstrap absence, Room bootstrap presence.
- Hub exact surfaces unchanged.

**Verification:** focused protocol tests, `cargo test --workspace`.

**Commit:** `feat(agent): compact standalone MCP surface`
- **Status:** complete

### Phase 6: Local Observability, Documentation, and Schema Budgets
**Objective / visible outcome:** Standalone execution is visible in local logs, reporting connection state is explicit, and schema compaction is permanently regression-tested and documented.

**Prerequisites:** Phases 3–5 complete and committed.

**Relevant implementation:** `stdio_server.rs`, managed lifecycle finalizer, reporting-only WebSocket/SSE loops in `hub.rs`, standalone supervisor smoke, and runtime documentation.

**Work boundaries:**
1. Add safe stdio tool-call start/completion/failure logs.
2. Bind the Phase 3 terminal-event hook to one safe stderr managed-process terminal log for asynchronous completions.
3. Add Hub-reporting connected/disconnected/failure/retry logs for WebSocket and SSE.
4. Add descriptor serialization budget tests and no-`agentId`/no-`confirmMethod` schema tests.
5. Update stdio server instructions, standalone runtime docs, README references, and examples.
6. Build and exercise real hidden worker `initialize/tools/list/tools/call` for Normal and Room.

**Tests / acceptance:**
- Logs appear on stderr only, contain required metadata, omit sentinel arguments/results/secrets, and do not duplicate terminal events.
- Exact descriptor ceilings pass.
- Hidden worker stdout remains valid MCP only.
- Documentation has no stale Tunnel `session.*`, old MCP/skills/tmux names, or Normal bootstrap claim.

**Verification:**
- `cargo fmt --all -- --check`
- `git diff --check`
- `cargo check --workspace`
- `cargo test --workspace`
- Hidden stdio Normal/Room smoke probe and measured descriptor report.

**Commit:** `docs(agent): document compact standalone tools`
- **Status:** complete

### Phase 7: Independent Review and Delivery Check
**Objective / visible outcome:** A reviewer confirms the public reduction did not weaken safety, audit, Hub compatibility, or process lifecycle correctness.

**Review boundaries:**
- Compare baseline and final `tools/list` byte metrics.
- Inspect all removed/merged tool mappings and conditional validation.
- Verify no caller identity/provider override remains in Tunnel schemas.
- Verify Hub full/coordinator exact surfaces and OpenAPI remain compatible.
- Stress quick/long/concurrent/confirmed/killed process paths and skill leases.
- Inspect local logs and audit for duplication or sensitive content.
- Run final full suite and repository status.

**Commit:** focused repair commit(s) only when evidenced.
- **Original review conclusion:** superseded by the independent acceptance review at `c987e32`; see Phases 8–9.
- **Status:** complete


### Phase 8: Focused Contract Repair
**Objective / visible outcome:** Close the seven independently reproduced gaps without changing the frozen Tunnel tool surface, Hub compatibility boundary, or public response contracts.

**Prerequisites:** Phases 3–7 implementation at `c987e32`; D-01–D-23 frozen; worktree contains no unrelated product changes.

**Relevant implementation:** `crates/agentic-gpt/src/stdio_server.rs`, `crates/agentic-gpt/src/sessions.rs`, `crates/agentic-gpt/src/utils.rs`, focused Agent tests, and real standalone supervisor probes. Hub full/coordinator code and protocol public types remain compatibility boundaries, not redesign targets.

**Work boundaries:**
1. Make Tunnel `process.batchExec` perform at most one configured-provider confirmation for the whole admitted batch. A successful batch confirmation must suppress per-element confirmation while preserving each element's effective policy and confirmation result in terminal audit.
2. Retain a `skills.run` lease-admission failure such as `skill_update_pending` as the same queryable terminal managed session returned to the caller. It must be visible through `process.get`, finalize once, release resources, write one audit record, and invoke the terminal hook once.
3. Preserve at least 64 KiB per stream for Tunnel `process.exec` and managed `process.batchExec` elements. The Implementer may raise the shared managed tail bound or use source-specific bounds; Hub one-shot limits and public truncation semantics must not regress. `skills.run` may inherit the larger shared bound but must remain bounded.
4. Add `durationMs` to the one later managed-process terminal log, computed from the retained session timestamps and emitted exactly once without raw arguments, paths, output, or secrets.
5. Distinguish Tunnel skill execution in terminal audit with `requestSource=tunnel:skills.run`; preserve the existing Hub skill source for Hub-routed execution.
6. Enforce action-dependent input validation for `tmux.sessions` and `tmux.panes`. Missing and action-incompatible fields return invalid params/structured validation failure; no supplied field may be silently ignored.
7. Enforce strict unknown-field rejection for every advertised Tunnel tool, including Room diary/notebook adapters. Legacy `agentId`, `agent_id`, and `confirmMethod` supplied to any Tunnel tool must fail before side effects or id allocation.
8. Keep removed alias dispatch code cleanup optional. It may be deleted when safely unreachable, but cleanup must not alter Hub full aliases or expand the repair diff.

**Required regression tests:**
- A confirmation-provider spy/counter proves one batch confirmation and zero per-element confirmations after allow.
- Batch denial/timeout still creates no sessions; allowed elements retain the batch confirmation result in audit.
- `skills.run` lease failure returns a retained id; repeated `process.get` remains terminal and audit/hook counts stay exactly one.
- A process emitting more than 32 KiB and at most 64 KiB per stream is not truncated before the 64 KiB boundary; output beyond the bound is truncated deterministically.
- Managed terminal log includes `durationMs` and excludes sentinel arguments, paths, stdout/stderr, and secrets.
- Tunnel and Hub `skills.run` audits preserve distinct request sources.
- Every `tmux.sessions` and `tmux.panes` action rejects incompatible fields and accepts its valid field set.
- Representative Normal and every Room diary/notebook adapter reject unknown fields; add a table-driven all-advertised-tool legacy-field test where practical.
- Exact 18/30 tools, schema budgets, Normal bootstrap absence, Room bootstrap presence, Hub full/coordinator surfaces, and supervisor smoke remain green.

**Verification:**
- `cargo fmt --all -- --check`
- `git diff --check`
- focused Agent tests covering all seven gaps
- isolated `cargo check --workspace`
- isolated `cargo test --workspace`
- real hidden stdio Normal/Room initialize, tools/list, and targeted tools/call probes

**Completion boundary:** All seven gaps have direct regression evidence, no frozen public contract changes, and the Phase 8 diff is committed as one focused repair.

**Phase 8 verification evidence:**
- Direct regression suites passed: `sessions::tests` 9/9 and `stdio_server::tests` 15/15; the full Agent binary suite passed 147/147.
- Isolated `cargo check --workspace` and `cargo test --workspace` passed: Agent 147, standalone supervisor 2, Hub 61, protocol 9, doc tests 0.
- `cargo fmt --all -- --check` and `git diff --check` passed.
- The hidden-worker Normal/Room initialize, tools/list, and targeted tools/call smoke passed within the 2 standalone supervisor tests.
- The repair diff is limited to `sessions.rs`, `stdio_server.rs`, and `utils.rs`; Hub full/coordinator and protocol public files are unchanged.

**Commit:** `fix(agent): close standalone MCP contract gaps`
- **Status:** complete; created as the single focused Phase 8 repair commit after this planning update

### Phase 9: Independent Repair Verification and Delivery
**Objective / visible outcome:** A fresh reviewer verifies the Phase 8 repair against the frozen plan and raw code/test evidence before restoring delivered status.

**Prerequisites:** Phase 8 complete and committed; reviewer starts from the frozen planning files and the `c987e32..HEAD` diff rather than the Implementer's completion summary.

**Review boundaries:**
1. Reproduce or inspect direct proof for all seven original gaps; do not infer correctness from the full suite alone.
2. Trace batch confirmation from stdio admission through managed child start and audit to prove there is exactly one provider interaction.
3. Trace every terminal path for ordinary process, batch element, and skills run, including lease failure, cancellation, spawn failure, and repeated inspection.
4. Inspect strict decoding at the public call boundary for all 18/30 advertised tools, with special attention to Room protocol request adapters and action-dependent tmux fields.
5. Measure actual process output bounds and verify terminal log/audit fields and redaction.
6. Confirm Hub full/coordinator, protocol, documentation, schema budgets, and hidden-worker stdout remain unchanged or compatible.
7. Run formatting, checks, full isolated workspace tests, real Normal/Room probes, and final clean-status inspection.

**Delivery rule:** Mark the plan `delivered` only when every Phase 8 acceptance item has independent evidence. Any discovered defect reopens Phase 8 or creates one narrowly scoped repair item; do not waive a frozen criterion because tests are otherwise green.

**Phase 9 independent verification evidence:**
- Reviewed the raw `f7f09a4..03eb501` diff rather than relying on the Implementer summary; product changes are limited to `sessions.rs`, `stdio_server.rs`, and `utils.rs`, with Hub/protocol/docs compatibility files unchanged.
- Traced all seven original findings through implementation and terminal paths. Batch confirmation is one provider interaction with preserved per-element audit evidence; lease admission failure is retained and finalized once; managed tails are 64 KiB; tmux conditional fields reject; terminal logs include bounded duration; Tunnel and Hub skill audit sources remain distinct; and public stdio argument validation covers every advertised tool.
- Re-ran eight direct repair tests independently; all passed.
- Ran a raw hidden-worker stdio probe for Normal and Room. Actual surfaces are 18/30; every advertised tool rejected each of `agentId`, `agent_id`, and `confirmMethod`; 40 KiB output was preserved, 72 KiB was deterministically truncated to 64 KiB; tmux incompatible fields rejected; managed terminal stderr included `durationMs`; Tunnel skill audit used `tunnel:skills.run`; every worker stdout line remained valid JSON-RPC.
- Re-ran Hub skill provenance, exact tool-set, schema-budget, formatting, and diff-hygiene checks; all passed.
- Isolated `cargo check --workspace` and `cargo test --workspace` passed: Agent 147, standalone supervisor 2, Hub 61, protocol 9, doc tests 0. Only the pre-existing `RunMode::Room` and `RunMode::role` dead-code warnings remain.
- No new defect, contract change, or product repair was required during Phase 9.

**Commit:** no Phase 9 product commit; one planning-only delivery checkpoint records the independent acceptance and restores a clean worktree.
- **Status:** complete

## Acceptance Criteria

1. Tunnel Normal advertises exactly 18 frozen tools; Tunnel Room advertises exactly 30.
2. No Tunnel input schema contains `agentId`, `agent_id`, or `confirmMethod`; supplied values are rejected.
3. Hub full/coordinator public tools, routing identity, and compatibility behavior remain unchanged.
4. `process.exec` returns inline for quick commands and a live `sessionId` for longer/confirmation-waiting commands without killing them.
5. `process.get/kill/list` fully replace Tunnel `session.*`, and `skills.run` sessions are managed through the same tools.
6. Managed processes write exactly one bounded terminal audit and one terminal lifecycle log; reporting is best effort.
7. `process.batchExec` returns ordered per-element managed responses, never kills running elements when its inline wait expires, and exposes follow-up only through the returned session ids.
8. MCP, skills, tmux, and Room bootstrap compaction preserves the frozen behavior and validation contracts.
9. Default local logs reveal lifecycle metadata but no raw arguments/results/stdout/stderr/paths/secrets.
10. Normal descriptor bytes are ≤11,500 and input schemas ≤6,200; Room descriptor bytes are ≤18,000 and input schemas ≤9,600.
11. Hidden stdio worker stdout remains protocol-only; Normal and Room initialize/list/call smoke tests pass.
12. Full workspace tests, formatting, checks, documentation inspection, and clean Git status pass before delivery.
13. An allowed `process.batchExec` invokes the confirmation provider exactly once for the whole batch and never reconfirms individual elements; denied/timed-out confirmation creates no sessions.
14. Every returned `skills.run` session id, including `skill_update_pending`, is retained and queryable through `process.get`, with exactly one terminal audit and terminal hook.
15. Tunnel `process.exec` and managed batch elements preserve a 64 KiB-per-stream bound and deterministic truncation beyond it.
16. `tmux.sessions` and `tmux.panes` reject every action-incompatible supplied field rather than ignoring it.
17. Every advertised Tunnel adapter strictly rejects unknown fields, including legacy identity/provider fields on Room diary/notebook tools, before side effects or id allocation.
18. Managed terminal logs include bounded `durationMs`; Tunnel `skills.run` audit records use `tunnel:skills.run`, while Hub skill execution preserves its existing source.
19. Each repaired gap has a direct regression test or real probe that would fail against `c987e32`; full green suites alone are not sufficient evidence.

**Acceptance status:** all 19 criteria passed independently against repair commit `03eb501`.

## Implementation Discretion

The implementer may choose private Rust type names, module splits, helper ownership, and whether the shared response is a new protocol type or an adapter-only struct, provided:
- the public Tunnel shapes and exact tool names remain frozen;
- Hub public compatibility remains unchanged;
- safety, audit, output bounds, retention, and logging constraints remain met;
- no generic RPC or duplicate compatibility descriptors are introduced.

## Implementation Handoff

- **Plan maturity:** delivered
- **Design phase:** complete
- **Implementation authorized:** no; no implementation phase remains
- **Entry phase:** none
- **Frozen decisions:** D-01 through D-23 (D-04 superseded by D-16; D-01–D-20 otherwise unchanged)
- **Open blocking decisions:** none
- **Implementation discretion:** exhausted for this delivery; future requirement changes must reopen the affected contract
- **Verification convention:** direct evidence for all seven repair findings, raw hidden-worker Normal/Room probes, and isolated full workspace verification completed
- **Commit convention:** Phase 8 product repair is `03eb501`; Phase 9 required no product repair and is recorded by a planning-only delivery checkpoint
- **Design checkpoint:** `f7f09a4`
- **Delivery:** accepted. Phases 1–9 are complete, all 19 acceptance criteria pass, and no further implementation or verification work remains in this plan.
- **Next invocation:** none for this plan; reopen with both planning skills only if a frozen requirement changes
