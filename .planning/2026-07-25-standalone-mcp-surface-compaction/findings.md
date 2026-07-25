# Findings: Agentic Standalone MCP Surface Compaction

## Repository and Baseline
- Repository is `/home/slhaf/Projects/AgenticGPT`.
- Planning began on clean `main` at `3dd8fa2`, tagged `v0.7.0`, equal to `origin/main`.
- Previous active plan `2026-07-24-agentic-standalone-tunnel-runtime` is implementation-complete and remains unchanged.
- The new plan is a follow-up public-surface and observability refinement, not a redo of the Tunnel runtime architecture.

## Phase 3 implementation findings

- `ManagedAuditContext` now carries request source, `needConfirm`, policy decision, confirmation result, optional skill provenance, installed digest, and an optional one-shot terminal hook. The former skill-only audit branch is gone.
- The generic `start_managed_session_async` registers the session before deferred path/preflight/policy/confirmation/spawn work, and active-capacity admission plus insertion occur under one sessions-map lock.
- Terminal finalization consumes the audit context exactly once, emits a best-effort `SessionUpdate`, and invokes the hook once; repeated inspection cannot duplicate either event.
- `start_session_async` remains a compatibility-prepared Hub path: its existing synchronous path/preflight/policy rejection behavior is preserved, while the shared managed finalizer is reused after admission. Tunnel/skills will use the deferred generic entrypoint in later phases.
- `wait_for_session` is now the shared bounded polling helper used by `skills.run`; the existing maximum wait bound remains 30 seconds.

## Phase 4 implementation findings

- Tunnel `process.exec`, `process.get`, `process.kill`, and `process.list` now decode strict stdio-only request structs, inject configured `agent_id` internally, and use the shared managed lifecycle. The public process wrapper omits top-level `agentId`; nested `SessionInfo.agentId` remains protocol metadata.
- Tunnel `process.batchExec` performs per-element working-directory/policy/preflight validation, one configured-provider batch confirmation, atomic aggregate capacity admission/insertion, concurrent child starts, one shared inline deadline, and per-element session follow-up data.
- Batch admission failures create no sessions; validation failures preserve ordered rejected/skipped evidence. Post-admission spawn failures remain isolated to their own retained session while siblings continue.
- Removed Tunnel process/session aliases are rejected by the existing advertised-tool gate as method-not-found. Strict `deny_unknown_fields` rejects both `agentId` and `confirmMethod` before managed session allocation.
- Hub full/coordinator tests remain green after the Tunnel adapter bypassed Hub `Exec`/`BatchExec`; the shared Hub protocol commands were not changed.

## Current Tunnel Tool Construction
- `crates/agentic-gpt/src/stdio_server.rs` owns a manually generated descriptor table, schemas, descriptions, annotations, profile tool lists, dispatch, and run reporting.
- Current Normal advertises 29 tools; Room adds ten diary/notebook tools for 39.
- `agentId` is manually repeated across process, session, tmux, and downstream MCP schemas.
- `require_agent` checks only that caller input equals this worker's configured id; it cannot route to another Agent.
- `confirmMethod` is exposed only as a per-call provider override on process/session/batch schemas.
- Every descriptor also repeats a generic output schema and transport metadata/annotations, so tool-count reduction matters in addition to input fields.

## Exact Baseline Measurement
A temporary config with Hub reporting disabled launched the real hidden stdio worker. MCP `initialize` and `tools/list` were sent over stdio; compact JSON byte counts were calculated, and the temporary directory was removed.

- Normal: 29 tools, 15,932 tool-array bytes, 8,345 summed input-schema bytes, 890 top-level description characters.
- Room: 39 tools, 21,664 tool-array bytes, 11,380 summed input-schema bytes, 1,212 top-level description characters.
- Input schemas are the largest single measured category, but descriptor count also repeats output schemas, annotations, names, and metadata.

Projected frozen surface:
- Normal: 18 tools, approximately 10,775 bytes total and 5,843 input-schema bytes.
- Room: 30 tools, approximately 17,255 bytes total and 9,104 input-schema bytes.
- Three Normal plus one Room complete surfaces shrink approximately 19,880 raw JSON bytes (28.6%).

## Current Process and Session Split
- `process.exec` calls `exec::run_exec_task`, waits at most `EXEC_TIMEOUT_SECS=30`, kills on timeout, and returns `TaskResult`.
- `session.start` registers an asynchronous managed session and returns immediately.
- `session.inspect`, `session.wait`, `session.kill`, and `session.list` operate on the in-memory managed-session map.
- Current default `maxActiveSessions` is 4; current `maxConcurrentTasks` is 2.
- Managed terminal sessions are retained for 24 hours with a newest-100 cap.
- Existing managed tail bound is 32 KiB per stream; one-shot exec bounds are 64 KiB per stream.

## Skills Run Is the Existing Model
- `skills.run` resolves an active workspace script, acquires a shared skill lease, starts it through `start_skill_session_async`, waits default 5/max 30 seconds, and returns `sessionId`, `completedInline`, `pollAfterMs`, and a session snapshot.
- This is already the desired short-inline/long-managed interaction model.
- Its follow-up documentation currently points to `session.inspect/session.wait/session.kill` and must move to `process.get/process.kill`.

## Managed Audit Gap
- One-shot exec writes a terminal audit record directly.
- Managed session terminal audit is currently attached only to an optional `SkillAuditContext`.
- Ordinary managed sessions do not carry a generic audit context.
- Simply routing `process.exec` through current sessions would therefore lose the reliable one-shot audit path.
- `skill_audit` must be generalized so every managed process has one terminal audit path, with optional skill metadata.

## Terminal Reporting and Logging Gap
- Stdio calls report initial/final tool-call events to optional Hub reporting.
- A long process call can return while the child continues; current session monitor does not emit a later terminal reporting snapshot or local completion log.
- `report_session` is currently called only when the immediate stdio result contains a session.
- Hub command legacy paths contain explicit exec/session logs, but Tunnel stdio calls do not emit equivalent local tool lifecycle logs.
- Hub reporting currently logs failures and retries but lacks a clear successful connected line and a distinct normal-disconnect line.

## Hub Compatibility Boundary
- Hub full MCP has routed process/session/tmux/MCP tools whose `agentId` is semantically required.
- Hub coordinator has an exact eight-tool Hub-native surface.
- Shared `HubCommand` variants and OpenAPI routes support existing clients.
- Compacting only Tunnel descriptors avoids a broad Hub migration and keeps multi-Agent routing explicit.
- Internal protocol payloads may continue to require `agent_id`; the stdio adapter can inject current config identity.

## Skills Merge Constraint
- `skills.list` returns valid summaries.
- `skills.search` applies text filtering and a limit.
- `skills.active` returns activation timestamps and stale/missing active entries.
- A naive `list(activeOnly=true)` over valid scanned packages would lose stale active records.
- The compact list adapter needs one stable richer response that preserves active metadata and missing/stale entries.

## tmux Merge Constraint
- Session list/create/close and pane list/capture have distinct payloads but share narrow domains.
- `tmux.sessions(action=...)` and `tmux.panes(action=...)` are small bounded tagged operations, not a generic RPC.
- Conditional fields require explicit runtime validation because the manual JSON schema does not currently encode per-action `oneOf` branches.
- Merging list/create/close makes descriptor annotations conservative: destructive/open-world because close/create exist.
- `tmux.exec` and `tmux.pasteText` must remain separate due shell detection, quoting, raw-input behavior, and confirmation differences.

## Bootstrap Profile Finding
- Current Tunnel Normal capabilities incorrectly include bootstrap because the previous runtime plan exposed bootstrap to both Normal and Room.
- User now restricts bootstrap to Room.
- Keeping manifest/read as two tools avoids an ambiguous optional-id input and conditional return type.

## Identity and Confirmation Finding
- A standalone connector already identifies one worker; input `agentId` is redundant and misleading.
- Keeping the configured Agent id in results/audit/reporting remains useful metadata and is not routing.
- Confirmation provider is an Agent deployment choice. Tunnel callers still control `needConfirm`, while local policy may force confirmation independently.
- Removing `confirmMethod` from Tunnel schemas shrinks repeated fields and avoids bypassing deployment provider selection.

## Excluded Follow-ups
- Hub-reporting enable/detail hot reload remains a separate runtime/config task.
- Full end-to-end Tunnel RTT needs protocol-stage timestamps or an echo diagnostic and is independent of schema compaction.
- Server confirmation delivery appears deployment/network-specific because other Agents receive Hub confirmation while Hub is online.
- A future fleet/coordinator execution gateway could expose one schema with a target parameter, but it would reintroduce a central execution data plane and is not justified now.

## Planning Tooling Notes
- Initial probing used `/home/slhaf/Documents/Projects`, which is not the Laptop project root and was rejected by path preflight.
- User clarified that the allowed repository root is `/home/slhaf/Projects`; all later audit and probes used `/home/slhaf/Projects/AgenticGPT`.
- Rejected probes made no changes.
- Temporary schema-probe directories were removed after each measurement.

## Formal Refine Re-entry — 2026-07-25
- Both `planning-with-files` and `refine-implementation-plan` are active in the Room skill workspace.
- The active plan and all three planning files exist and were read completely from disk.
- Repository status is clean at `9b08dcc`, with `main` ahead of `origin/main` only by the planning checkpoint; no product edits exist.
- The prior self-authored handoff is treated as a candidate contract until the formal repository-grounded readiness audit completes.

## Formal Refine Evidence — Managed Lifecycle and Strict Decoding
- `sessions.rs` currently has three independent terminal paths: `finish_pending_session`, `refresh_session`, and `kill_session`. Audit/lease cleanup exists only in some branches; a killed child may remain attached long enough for a later refresh to overwrite `killed` with `exited`/`failed`. Phase 3 needs one idempotent terminal-finalization owner rather than scattered `skill_audit.take()` logic.
- `start_session_async_inner` returns early for invalid working directory, preflight denial, policy denial, and active-capacity rejection before inserting a managed record. If the compact `process.exec` contract promises a retained `sessionId`, those immediate failure paths need an explicit retention rule rather than accidental behavior.
- `stdio_server.rs` uses `serde_json::from_value` into request types that do not deny unknown fields. Removing `agentId`/`confirmMethod` from descriptors alone would silently accept and ignore them. The Tunnel adapter needs strict stdio-only request types or equivalent explicit key validation. Shared Hub protocol request structs must remain compatibility-oriented.
- `HubCommand::Exec` is the Hub full one-shot execution primitive. The new Tunnel managed `process.exec` must call a shared internal managed-process helper directly or use a new private adapter path; repurposing the existing Hub command would violate D-01.
- `process.batchExec` currently aborts Tokio join tasks at the global deadline. The child process is owned inside each aborted future and `tokio::process::Command` does not visibly enable `kill_on_drop`; a broad orphan/process-tree repair is not required by schema compaction and should not be pulled in without an explicit scope decision.

## Formal Refine Evidence — Reporting, Skills Envelope, and Test Seams
- Tunnel tool-call reporting already emits one `RunReport(started)` and one `RunReport(completed|failed)` using a generated `runId/requestId`. `SessionUpdate` is a separate message without run correlation. The proportional design is to finish the tool-call report when the MCP call returns, carry the initiating run id only for local terminal logging, and emit one later terminal `SessionUpdate`; do not invent a second terminal `RunReport`.
- Reporting metadata mode already redacts program, args, working directory, command preview, and output tails. The new local logs should use the existing stderr `log_info/log_warn` key-value convention rather than introduce a logging framework or JSON protocol on stdout.
- `skills.list` and `skills.search` both return `{skills, warnings}` while `skills.active` returns `{activeSkills, warnings}`. `ActiveSkill` is the only type that can represent deleted/stale active entries (`summary: null`). A merged tool needs a frozen stable envelope that preserves both arrays rather than forcing a lossy union.
- Existing stdio tests cover exact tool sets, profile gating, in-process initialize/list/call, and a real supervised worker smoke. They are the natural regression seams for exact 18/30 sets, strict old-field rejection, Room-only bootstrap, protocol-only stdout, and the updated no-`agentId` end-to-end call.
- Reporting connection functions establish both WebSocket and SSE reporting senders and send Hello/current session snapshots, but do not log successful reporting-only connection or normal stream close. This can be added locally without changing reconnect or command-channel behavior.

## Decision Rationale — Q-01 / D-05
- User selected retained terminal sessions after id allocation. This makes every returned `sessionId` queryable through `process.get` and avoids an id that disappears immediately. Invalid JSON shape and forbidden legacy fields remain pre-allocation MCP parameter errors.

## Decision Rationale — Q-02 / D-09
- User selected the stable `{skills, activeSkills, warnings}` envelope. This preserves ordinary list/search results and the separate activation ledger needed to represent deleted/stale active skills without a mode-dependent output union.

## Decision Rationale — Q-03 / D-16
- User clarified that `process.batchExec` belongs in the same managed-process refactor. The Tunnel batch should be implemented as multiple starts of the shared `process.exec` primitive, not retained as the old one-shot batch executor.
- This removes the earlier orphan-cleanup side question for the Tunnel path: the batch deadline becomes only an inline response deadline, while each child remains owned by its managed session and can be inspected or killed later.
- Hub full compatibility still uses the existing `HubCommand::BatchExec` and one-shot batch implementation, so changing the Tunnel adapter does not alter routed Hub clients.
- No batch lifecycle tool is added because the frozen 18-tool surface already provides per-process `get`, `kill`, and `list`; `batchId` remains correlation metadata.
- One external semantic remains open: whether validation/policy/capacity admission is all-or-reject before any child starts, or whether elements start independently and may partially succeed.

## Decision Rationale — Q-04 / D-17
- User selected all-or-reject admission even though it is intentionally strict. Batch semantics represent one coordinated submission; allowing earlier elements to start before a later validation/capacity failure would create partial side effects and a harder recovery contract.
- The launcher can still return all session ids together because it allocates them only after every element passes normalization, path/preflight/policy checks, aggregate capacity is available, and any single batch confirmation is allowed.
- Aggregate capacity is checked against the number of elements before starting. This avoids a batch consuming only part of the remaining slots and makes rejection deterministic.

## Formal Refine Contract-Surface Result
- Scope/identity: Tunnel-only surface and configured worker identity are frozen; Hub routed identity is preserved.
- Inputs/outputs: exact 18/30 tools, strict unknown-field rejection, merged skills envelope, process/batch responses, defaults, and bounds are frozen.
- Lifecycle/failure: retained post-allocation failures, exactly-once finalization, cancellation, shared wait deadlines, all-or-reject batch admission, and no kill-on-inline-expiry are frozen.
- Persistence/recovery: in-memory managed retention remains 24 hours/newest 100; no DB/config migration applies.
- Security/trust: path policy, policy, confirmation provider ownership, skill path/lease rules, downstream MCP checks, and Room capability boundaries remain authoritative.
- Operations/observability: active-session capacity, bounded tails, safe stderr logs, best-effort reporting, and schema budgets are frozen.
- N/A: durable database migration, cross-device fleet routing, UI/CLI work, reporting hot reload, and full RTT instrumentation are explicitly out of scope.

## Readiness Recheck — Contract Gaps Found
- The prior `process.exec` sequence said path/policy/preflight happened before session allocation while Q-01/D-05 required those failures to produce retained sessions after allocation. The executable order has been corrected to strict decode first, atomic admission/registration second, then path/policy/preflight/confirmation/spawn against the retained record.
- Phase 3 and Phase 6 both claimed ownership of the terminal stderr log. The boundary is now Phase 3 = exactly-once finalizer plus transport-neutral terminal-event hook; Phase 6 = safe stderr rendering, fields, and leak/duplication tests.
- D-17 requires aggregate batch capacity reservation to be race-free. The plan now requires capacity reservation and insertion of all `starting` records in one atomic admission section before spawning any child, plus concurrent admission tests.
- The managed batch public output remains underspecified. Existing `BatchExecResult` wraps one-shot `TaskResult`, which cannot express session ids or active managed states. Q-05 must freeze the replacement envelope before handoff.

## Decision Rationale — Q-05 / D-18
- User accepted the proposed managed batch envelope but removed `agentId` from the public response. The standalone connector already identifies the worker, so repeating identity at both batch and embedded-process levels would add schema/output weight without routing value.
- `SessionInfo` may retain its existing internal/result identity metadata for compatibility, but the new adapter-owned batch and single-process wrapper fields do not add another `agentId`.
- The outer status and ordered input echo preserve the useful shape of the old batch response, while `outcome` and optional embedded managed response make active sessions and later `process.get/kill` follow-up explicit.

## Repository-Derived Response Contracts — D-19
- Existing `session.inspect`, `session.wait`, and `session.kill` expose `SessionInfo` directly; the compact `process.get/kill` inherit that body. Existing Tunnel `session.list` wraps the active array as `{sessions}`; `process.list` keeps it to avoid a needless rename.
- `mcp::list_servers` already returns `{servers}` and `mcp::list_tools` returns `{tools}`. The optional `serverId` selects the mode, so an additional tagged union or generic result envelope is unnecessary.
- tmux list/create/close currently return `{sessions}`, `{session,cwd,created}`, and `{session,closed}`. Pane list/capture return `{session,panes}` and `{target,lines,capture}`. The merged action tools preserve those bodies and use strict action-dependent input validation.
- Current batch code accepts an empty element list and returns a completed empty result. The managed Tunnel adapter preserves this edge behavior; aggregate active-session capacity provides the effective upper bound for non-empty batches.
- The earlier plan line retaining a top-level `agentId` in the shared process wrapper conflicted with Q-05 and standalone compaction. It is removed for Tunnel `process.exec`, embedded batch process results, and Tunnel `skills.run`; nested legacy `SessionInfo.agentId` remains unchanged.

## Remaining Behavioral Gap — Q-06
- Atomic batch admission can guarantee that validation, policy, confirmation, capacity reservation, id allocation, and starting-record insertion succeed as one decision before any child spawn. It cannot make arbitrary operating-system process creation or command side effects transactional.
- After admission, one child may spawn and begin executing before a sibling spawn call fails. Killing already spawned siblings is only best effort and cannot undo filesystem/network side effects.
- The Implementer must not silently choose between independent sibling continuation and best-effort sibling cancellation; this observable failure behavior is the final user-owned decision.
- Restart recovery is separately classified as N/A for this plan: managed state remains in-memory, no persistent reattachment is introduced, and existing worker-shutdown behavior is preserved.

## Decision Rationale — Q-06 / D-20
- User selected independent continuation after admission. A sibling spawn failure finalizes only its own retained session as `spawn_failed`; already spawned siblings continue and remain controllable through `process.get/kill`.
- This keeps managed batch semantics aligned with “launch multiple independent `process.exec` calls” and avoids pretending arbitrary command side effects can be rolled back transactionally.
