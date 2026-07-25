# Findings: Agentic Standalone MCP Surface Compaction

## Repository and Baseline
- Repository is `/home/slhaf/Projects/AgenticGPT`.
- Planning began on clean `main` at `3dd8fa2`, tagged `v0.7.0`, equal to `origin/main`.
- Previous active plan `2026-07-24-agentic-standalone-tunnel-runtime` is implementation-complete and remains unchanged.
- The new plan is a follow-up public-surface and observability refinement, not a redo of the Tunnel runtime architecture.

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
