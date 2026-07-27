# Task Plan: Agentic Interface Repair and MCP Managed Execution

## Goal
Repair the v0.8.0 standalone connector/file contract defects and add bounded MCP reload, batch, long-running execution, and a first-class local integration ingress. Internally unify process, skill-run, and MCP execution as `ManagedJob`; expose one compact `job.*` lifecycle surface with no historical process/session compatibility wrappers.

## Workflow State
- **Stage:** implementation_complete
- **Current role:** implementer
- **Implementation authorized:** yes — user explicitly started implementation on 2026-07-27
- **Active plan:** `2026-07-27-agentic-interface-repair-mcp-execution`
- **Baseline:** `698f43a` (`v0.8.0`), clean `main`, aligned with `origin/main`
- **Current phase:** complete — Implementation Phases A–H finished
- **Entry phase:** Implementation Phase A — connector contract repair
- **Open blocking decisions:** none
- **Design checkpoint:** not created; planning files remain uncommitted
- **Next action:** none inside the implementation plan; push, tag, release, deployment, and v0.8 config migration remain separate user-authorized operations

## Scope and Ownership

### P0 current-defect repair
1. Repair `file.batch` read-operation connector schema/runtime casing so the advertised `startLine` / `endLine` fields can actually be called.
2. Repair `file.edit` and batch-edit diff generation and `changedLines` semantics for create, overwrite, insert, delete, replace, and patch.
3. Clarify the accepted unified-diff dialect and add negative contract tests.
4. Add connector-level schema-to-serde-to-dispatch tests rather than relying only on internal JSON/unit tests.

### P1 additive capability
5. Make valid `mcpServers` changes hot-reloadable without restarting the Agent.
6. Add a local Unix-socket MCP ingress and CLI trigger path that shares the running worker with tunnel traffic.
7. Convert process, skill-run, and MCP execution to one internal `ManagedJob` registry and compact `job.*` lifecycle surface.
8. Add bounded `mcp.batch` for one connector round trip over multiple ordinary downstream MCP tool calls.
9. Extend connector-level testing, audit, confirmation, capacity, time, output, cancellation, restart, local-ingress, and observability coverage.

### Non-goals
- No product code, tests, configuration, generated artifacts, commit, push, deployment, tag, or release in this planning request.
- No downstream JSON-RPC batch requirement; Agentic coordinates ordinary MCP calls itself.
- No persistent pooled MCP client registry in V1. Current clients remain per-call unless later profiling proves pooling necessary.
- No claim of transactional rollback for downstream MCP side effects.
- No durable replay of an in-flight MCP call after Agent restart.
- No raw MCP arguments/results in audit logs or confirmation previews.
- No weakening of current MCP confirmation based only on untrusted downstream tool annotations.
- No loopback TCP or unauthenticated network debug API; the local ingress is Unix-domain-socket only and protected by the private runtime directory.
- No compatibility aliases for removed `process.get/list/kill`, `process.batchExec`, `session.*`, or `hub.session.*` tool names.

## Frozen Public Contracts

### 1. `file.batch` connector parity
- Public connector fields remain camelCase.
- Read operations accept `id`, `path`, `includeContent`, `startLine`, and `endLine` exactly as advertised.
- Search/edit operation fields must also have schema/runtime parity; the regression suite exercises every advertised nested optional field at least once.
- Unknown nested fields remain rejected by serde with a typed invalid-arguments response.
- The connector descriptor schema, runtime DTO, and standalone surface revision must change together.

### 2. `file.edit` diff and `changedLines`
- Return a valid bounded unified diff generated from the complete before/after text, preserving interior blank lines.
- Use true line-level edit operations; `changedLines.added` and `changedLines.removed` are computed from the full diff operation stream before output truncation.
- A created N-line file reports `added: N`, `removed: 0`; deleting N lines reports the inverse.
- Final newline differences must produce non-empty verification evidence rather than disappearing.
- Preserve LF/CRLF file content exactly; diff display may use LF separators but must remain semantically correct.
- Return multiple hunks when changes are disjoint. Diff text remains capped at 64 KiB on a UTF-8 boundary; `diffTruncated` reports clipping while change counts remain complete.
- Empty files, blank-only files, interior blank lines, and files with/without a final newline are first-class test cases.

### 3. Unified patch contract
- Public descriptions and examples require a standard single-file unified diff with hunk headers such as `@@ -oldStart,oldCount +newStart,newCount @@`.
- Bare `@@`, missing ranges, mismatched line counts, wrong targets, and multi-file patches return `file_patch_invalid`; content/context mismatch remains `file_patch_conflict`.
- For compatibility, the parser may continue accepting the standard abbreviated count form where an omitted count means 1; the connector examples always emit explicit counts.

### 4. `mcpServers` hot reload
- Add `mcp_servers` to the validated standalone live subset.
- Validate the complete candidate map before it becomes effective: supported transport, required non-empty endpoint/command, and structurally valid entries.
- Swap the entire map atomically under the config write lock only after candidate validation.
- Existing in-flight calls retain their cloned old server configuration; calls started after the swap use the new map.
- Added, changed, enabled, disabled, and removed servers affect new calls without Agent restart.
- Invalid disk changes leave the previous live map untouched and are surfaced through existing config health.
- Because clients are currently ephemeral, no new `mcp.reload` or `mcp.reconnect` tool is added in V1. If pooling is introduced later, reconnect semantics must be redesigned then.
- After a valid live swap, `agent.info.config.restartRequiredFields` no longer contains `mcpServers`.

### 5. `ManagedJob` core and compact public lifecycle
- Rename the internal process-shaped abstraction to `ManagedJob`, with `ManagedJobRegistry`, `JobInfo`, `JobKind`, `JobState`, and kind-specific runtime payloads.
- `JobKind` is `process | skill | mcp`; `skills.run` may reuse the process runtime internally while retaining `kind: skill` externally. Skill installation keeps its separate `installId` workflow and is not folded into this registry in this scope.
- Creation remains domain-specific so schemas stay small and unambiguous:
  - `process.exec`
  - `process.batch` (rename from `process.batchExec`)
  - `skills.run`
  - `mcp.callTool`
  - `mcp.batch`
- Lifecycle is exposed only as:
  - `job.get` with `jobId` and optional `waitSeconds` (`0..30`); omit/zero for immediate inspection, positive values for bounded waiting.
  - `job.list` with bounded optional `kind`, `state`, and `limit` filters.
  - `job.cancel`, which requests kind-aware cancellation and reports evidence rather than promising a kill.
- Remove `process.get`, `process.list`, and `process.kill`; do not add standalone `session.*` aliases.
- Hub/full surfaces remove `session.start/list/inspect/wait/kill`; `process.exec` itself handles inline-versus-deferred process execution and returns a `jobId`. Hub coordinator surfaces rename `hub.session.list/get` to `hub.job.list/get`. No compatibility aliases remain.
- Generic job fields include:
  - `jobId`, `kind`, `state`, `createdAt`, `startedAt`, `updatedAt`, and terminal timing.
  - process/skill metadata and bounded stdout/stderr tails where applicable.
  - MCP `result` or bounded result-summary fields, `resultTruncated`, `resultBytes`, `resultSha256`, server/tool/config revision/request id metadata.
  - `cancelRequested`, `cancelOutcome`, and `terminationEvidence`.
- All job-producing tools return one stable Agentic job envelope; fast work may complete inline but still returns `jobId`.
- Rename user-visible lifecycle terminology consistently: config `limits.maxActiveSessions` becomes `limits.maxActiveJobs`, `agent.info.execution.sessions` becomes `agent.info.execution.jobs`, Protocol/Hub session payloads/events become job payloads/events, and no legacy aliases are retained.
- Target standalone surfaces after this plan:
  - Normal: 24 tools.
  - Room: 36 tools.
  These counts remove three process lifecycle wrappers, rename `process.batchExec` without an alias, and add `job.get/list/cancel` plus `mcp.batch`.

### 6. Managed `mcp.callTool`
Input additions:
- `waitSeconds`: default 5, minimum 0, maximum 30
- `timeoutSeconds`: default 300, minimum 1, maximum 900

Behavior:
1. Validate server/tool/argument bounds and shared capacity.
2. Register one `ManagedJob` before authorization/connection so `job.get` / `job.cancel` work while waiting.
3. Reuse cancellable MCP confirmation.
4. Create the downstream client, retain the rmcp per-request `RequestHandle`, and execute under the configured deadline.
5. Wait inline for at most `waitSeconds`.
6. Return a stable Agentic envelope whether completion is inline or deferred.

Response envelope:
```json
{
  "status": "completed|running|failed|rejected|timed_out|cancel_requested|cancelled|detached",
  "completedInline": true,
  "jobId": "job_...",
  "pollAfterMs": 500,
  "job": {},
  "result": {},
  "error": null
}
```
- `jobId` is always returned after successful registration, including inline completion.
- This intentionally replaces the raw downstream-result-only response and must be documented as the v0.9 managed-execution contract across tunnel stdio, local Unix MCP, Hub MCP, and HTTP routes.
- Downstream result content remains unchanged when within bounds; oversized results use the bounded summary contract below.

### 7. Bounded MCP results
- Per-call serialized argument object: maximum 256 KiB.
- Aggregate `mcp.batch` argument bytes: maximum 2 MiB.
- Retained structured result per child: maximum 512 KiB.
- Aggregate batch response/result material: maximum 2 MiB.
- Never return invalid truncated JSON.
- If a result exceeds the per-child bound, omit the full `result` and return a valid summary containing `resultTruncated: true`, original byte count, SHA-256, and a bounded UTF-8 serialized preview string.
- Aggregate clipping replaces later/large child result bodies with summaries while preserving every child id, index, state, error, and job id.

### 8. `mcp.batch`
Input:
```json
{
  "calls": [
    {"id":"symbols","serverId":"idea","toolName":"...","arguments":{}}
  ],
  "mode": "parallel|sequential",
  "failFast": false,
  "waitSeconds": 5,
  "timeoutSeconds": 300
}
```

Rules:
- `calls` contains 1–16 entries.
- Optional ids use `[A-Za-z0-9._-]`, 1–64 characters, unique within the batch.
- `mode` defaults to `parallel`.
- `failFast` defaults to false.
- `waitSeconds` and `timeoutSeconds` use the single-call bounds.
- Results are always returned in input order with child job ids.
- All children use the same `ManagedJobRegistry` and global capacity pool.
- The whole batch reserves required global capacity atomically before confirmation/start; capacity failure rejects the batch without starting children.
- Global MCP start concurrency defaults to 8; per-server active-call concurrency defaults to 2. Additional admitted children remain `queued` and count as active managed capacity.
- Parallel mode starts calls subject to both concurrency limits.
- Sequential mode starts the next child only after the previous child reaches a terminal state; a background coordinator continues after the connector response returns.
- No parent job is required in V1. `batchId` is an audit/correlation id; each child remains independently inspectable/cancellable through `job.*`.
- `failFast` in sequential mode marks remaining queued children `skipped` after the first hard failure.
- `failFast` in parallel mode prevents not-yet-started queued children from starting; it does not cancel already-running calls and cannot roll back side effects.
- Batch status is `completed`, `running`, `completed_with_errors`, or `rejected`.

### 9. Confirmation semantics
- Preserve the current default that MCP calls require confirmation unless the matching server has a live temporary allow.
- `mcp.callTool` uses cancellable confirmation while the managed job is `waiting_confirmation`.
- `mcp.batch` performs one bounded aggregate confirmation for all children that are not covered by temporary allows.
- If all confirmable children target one server, the confirmation may offer 15/30-minute temporary server allow.
- A multi-server aggregate confirmation offers only allow-once or deny; it must not silently grant temporary access to multiple servers.
- Denial/unavailability starts no downstream calls and records a per-child rejected audit outcome.
- Downstream annotations may enrich the preview/audit but do not weaken confirmation in V1.

### 10. Truthful cancellation and timeout
- Use rmcp 1.7 `RequestHandle::cancel(reason)` to send `notifications/cancelled` for the exact MCP request id.
- Sending that notification proves only that cancellation was requested; it does not prove the remote task terminated.
- State transitions:
  - `cancel_requested`: notification accepted for transport delivery; awaiting evidence.
  - `cancelled`: downstream response/error explicitly indicates cancellation; `terminationEvidence=remote_response`.
  - `detached`: no terminal evidence arrives during a bounded grace period; Agent closes/detaches its client and reports remote state unknown.
  - `timed_out`: execution deadline expired; cancellation is requested, but remote state remains unknown unless later evidence is received.
- `job.cancel` while waiting for confirmation cancels the local confirmation wait and can terminate as `cancelled` with local evidence because no downstream call started.
- Closing a streamable HTTP connection or owned stdio transport is not, by itself, proof that downstream side effects stopped.

### 11. Retention and restart boundary
- Reuse the existing bounded in-memory retention: terminal jobs up to 24 hours and at most 100 retained entries across all job kinds.
- MCP jobs/results are not replayed after Agent restart.
- Job ids include a boot-generation component so `job.get` / `job.cancel` can return `job_lost_after_restart` for recognized prior-generation ids rather than a misleading active result.
- On a new Agent connection/start generation, Hub marks previously cached nonterminal jobs for that agent `unknown_after_restart`; it never reissues the MCP side effect.
- Durable terminal-result storage is out of scope for V1 because bounded retention satisfies the current delayed-next-turn requirement. It may be added later without changing cancellation truthfulness.

### 12. Audit and observability
- Write one redacted audit record per MCP child plus one aggregate batch/confirmation record.
- Child audit includes job id, batch id, server id, tool name, argument key names, argument byte count/hash, authorization result, config revision, ingress source, timing, result byte count/hash/truncation, terminal state, and cancellation evidence.
- Never store raw argument values, raw result content, or confirmation-preview content in audit.
- `agent.info` gains a bounded MCP section with effective server-config revision, configured server count, active/queued call counts, global/per-server concurrency defaults, result bounds, and timeout bound.
- Surface annotations:
  - `mcp.list`: read-only, non-destructive, open-world.
  - `mcp.callTool` and `mcp.batch`: non-read-only, conservatively destructive, open-world.
  - `job.get` / `job.list`: read-only; `job.cancel`: destructive.

### 13. Local MCP control ingress and CLI trigger channel
- Add a Unix-domain-socket MCP ingress to the same worker that currently serves tunnel stdio. It must share the exact `AppState`, tool descriptors, schema validation, confirmation, audit, config watcher, `ManagedJobRegistry`, and result bounds; it is not a second execution service.
- Serve the cloned MCP tool server over `tokio::net::UnixStream`. rmcp 1.7 already accepts any `AsyncRead + AsyncWrite` transport, so no custom JSON-RPC protocol is required.
- `run-as-standalone` exposes the local socket alongside tunnel stdio. Add `run-as-local --config ... --profile normal|room` for development without the tunnel client; it acquires the existing config run lock and exposes the same socket/surface. A tunnel-backed and local-only runtime for the same config cannot run simultaneously.
- `run-as-local` uses the same Normal/Room capabilities, path policy, execution policy, MCP config, confirmation, audit, live reload, and bounds as standalone, but does not require tunnel configuration, tunnel credentials, the tunnel binary, or Hub reporting. Split deployment mode from per-request ingress so a tunnel worker can serve both `tunnel:stdio` and `local:unix` concurrently.
- Add CLI clients:
  - `agentic-gpt local list-tools --config ...`
  - `agentic-gpt local call --config ... <tool> --arguments-file <path|->` with an optional bounded inline JSON argument form.
  The CLI connects as a real rmcp client; it must not call `local_service::dispatch` directly because that would bypass public schema/transport behavior and could not share a running job registry.
- Derive a transport-neutral socket path such as `~/.agentic_gpt/runtime/agent/<agentId>/mcp.sock`; create the directory as `0700`, the socket as owner-only, verify the connecting peer is the same local UID where supported, clean only proven-stale socket files, and never bind TCP. A missing/restarting worker returns a typed local-unavailable error.
- Parameterize per-request ingress source (`tunnel:stdio`, `local:unix`, Hub) rather than hard-coding `tunnel:*` audit strings. The current single `RuntimeModel.transport` may remain a deployment-mode field, but request ingress must be separate because one worker can accept tunnel and local calls concurrently.
- Expose local-ingress readiness/path metadata in `agent.info` without adding any MCP tool; the Normal/Room tool counts above remain unchanged.
- A tunnel worker restart may briefly drop the socket and loses in-memory jobs under the already-frozen boot-generation contract; the CLI reconnects but never replays side effects.

## Implementation Phases and Commit Boundaries

### Implementation Phase A — Connector contract repair (P0)
Objective: make the current public `file.batch` schema callable and prevent future hand-written schema/serde drift.

Work:
- Fix nested read casing for `startLine` / `endLine`.
- Audit all nested batch fields for exact advertised casing/default parity.
- Add table-driven descriptor-to-runtime tests.
- Add a real rmcp stdio connector test that lists tools, reads the advertised schema, and calls `file.batch` with ranged read fields.

Verification:
- Focused stdio/file contract tests.
- Exact Normal/Room surface counts and surface revision checks.
- Live v0.8 reproduction becomes a passing ranged read.

Commit boundary:
- `fix(agent): align file batch connector contract`

### Implementation Phase B — Diff evidence and patch contract repair (P0)
Objective: return truthful review evidence from every edit mode.

Work:
- Replace the broken interior-blank-line handling.
- Use a proven bounded line-diff implementation or equivalent correct algorithm.
- Generate multi-hunk unified output and full change counts.
- Tighten patch schema descriptions/examples and negative errors.
- Keep single and batch edit responses/audits on the same shared diff producer.

Verification matrix:
- create, overwrite, insert, delete single import, delete multiline method, replace, patch, no-op, dry-run.
- LF, CRLF, empty, blank-only, interior blank lines, final newline/no final newline.
- disjoint edits, diff truncation with complete counts, bare `@@`, malformed/mismatched/wrong-target/multi-file patches.
- Real temporary project edit smoke with revision verification.

Commit boundary:
- `fix(agent): repair file edit change evidence`

### Implementation Phase C — MCP config validation and hot reload (P1)
Objective: apply endpoint changes without restarting the Agent.

Work:
- Add MCP entry validation.
- Include `mcp_servers` and MCP execution limits in the atomic standalone live subset.
- Update `agent.info` health/restart logic and MCP diagnostics.
- Preserve in-flight old-config behavior and new-call new-config behavior.

Verification:
- added/changed/enabled/disabled/removed server cases.
- invalid candidate retains old map.
- concurrent in-flight call across map swap.
- `restartRequiredFields` no longer reports `mcpServers` after successful reload.
- watcher-level temp-config integration test and real local endpoint smoke.

Commit boundary:
- `feat(agent): hot reload mcp server configuration`

### Implementation Phase D — Local MCP ingress and CLI trigger path (P1)
Objective: provide a real local integration channel before the larger managed-execution phases.

Work:
- Extract a cloneable transport-neutral MCP tool server from the stdio-specific naming.
- Add private Unix-socket listener lifecycle, permissions, stale cleanup, ingress attribution, and readiness reporting.
- Add `run-as-local`, `local list-tools`, and `local call` CLI paths using rmcp client/server over Unix streams.
- Keep the same run lock, config validation/live reload, confirmation, audit, and tool surface.

Verification:
- Spawn `run-as-local` with a temp config, list exact tools, and call `agent.info`, ranged `file.batch`, and a guarded process command.
- Run local socket and tunnel stdio against the same in-process server and assert descriptor/schema/annotation parity.
- Verify owner-only directory/socket permissions, stale socket cleanup, second-instance rejection, missing/restarting worker errors, and request-source audit attribution.
- Verify the CLI accepts arguments from stdin/file without shell-quoting corruption and returns structured JSON on stdout with logs on stderr.

Commit boundary:
- `feat(agent): add local mcp control ingress`

### Implementation Phase E — `ManagedJob` core and protocol surface (P1)
Objective: replace process-shaped sessions with one compact job lifecycle before adding managed MCP execution.

Work:
- Rename/refactor the existing registry into common `ManagedJob` metadata plus process/skill/MCP runtime variants.
- Preserve process/skills execution behavior, audit, capacity, retention, reporting, and tails while deliberately replacing old public names.
- Replace protocol `SessionInfo`, session commands/events/caches, config `maxActiveSessions`, and `agent.info.execution.sessions` with their job equivalents; retain no legacy wire/config aliases.
- Publish only `job.get/list/cancel`; rename `process.batchExec` to `process.batch`; remove process/session compatibility wrappers across standalone, Protocol, Hub MCP, HTTP routes, run history, and coordinator surfaces.
- Add boot generation and Hub stale-job reconciliation.

Verification:
- Existing process and `skills.run` behaviors remain correct through the new names/envelopes.
- Global capacity still includes process and skill jobs.
- `job.get` immediate/bounded-wait behavior, list filters, kind-aware cancel routing, `maxActiveJobs` config validation, and `agent.info.execution.jobs` diagnostics.
- Exact Normal 24 / Room 36 tool surfaces and absence of removed aliases.
- Retention pruning, prior-generation id, Hub cache restart reconciliation, HTTP/MCP parity.

Commit boundary:
- `refactor(agent): replace sessions with managed jobs`

### Implementation Phase F — Managed single MCP execution (P1)
Objective: make one long MCP call observable and bounded.

Work:
- Implement managed authorization/connect/request/await state machine.
- Retain rmcp request handle and request id.
- Add inline wait, execution deadline, result bounding, redacted audit, cancellable confirmation, and truthful cancellation.
- Update standalone, local Unix ingress, protocol, Hub MCP, and HTTP route response envelopes.

Verification:
- fast inline completion; deferred completion then `job.get` bounded wait.
- capacity rejection; confirmation allow/deny/unavailable/cancel.
- connection failure, downstream error, disconnect, timeout.
- supported cancellation notification observed; explicit cancellation response; ignored cancellation becomes detached/unknown.
- oversized result summary and no invalid JSON.
- no raw arguments/results in audit or metadata reporting.
- identical behavior through tunnel stdio and local Unix ingress.

Commit boundary:
- `feat(agent): manage downstream mcp tool execution`

### Implementation Phase G — Bounded `mcp.batch` (P1)
Objective: remove repeated connector round-trip cost without bypassing safety bounds.

Work:
- Add standalone/local and Hub/HTTP contracts.
- Implement atomic admission, aggregate confirmation, ordered child registration, global/per-server scheduling, sequential coordinator, fail-fast semantics, and aggregate result budgeting.
- Correlate child jobs/audits with `batchId`.

Verification:
- same-server and multi-server parallel limits.
- sequential order with a long first call and continued background execution.
- partial failure, failFast queued skipping, in-flight non-cancellation.
- single/multi-server confirmation and temporary allow behavior.
- capacity rejection without partial start.
- mixed inline/deferred children, child cancellation, timeout, result/aggregate truncation.
- input order preserved under parallel completion.
- tunnel and local ingress parity.

Commit boundary:
- `feat(agent): add bounded managed mcp batch calls`

### Implementation Phase H — Connector parity, documentation, and acceptance
Objective: verify the complete feature through real connector paths and leave a reviewable clean implementation.

Work:
- Update standalone/local instructions, Hub descriptions, HTTP/interface docs, config examples, release notes, and breaking-name migration notes.
- Add schema snapshot/parity coverage for all new/changed tools and annotations.
- Add deterministic fake MCP connector E2E harness.
- Run Normal and Room tunnel plus local-only smoke and Hub full-surface smoke.
- Run Laptop/Tablet/Server/Room live smoke where available, without changing unrelated machine configuration.

Verification commands/environments:
- focused Agent tests after every phase.
- Protocol, Hub, Agent binary, supervisor, workspace tests.
- `cargo fmt --check`, `cargo clippy` where repository convention allows, `git diff --check`.
- exact tool surface counts/revision and connector-generated schema calls.
- local socket permissions/readiness, CLI output discipline, and tunnel/local parity.
- final security/output/cancellation review.

Commit boundary:
- `docs(test): finalize managed execution and local ingress contracts`

## Test Matrix Summary

| Area | Unit | Contract/serialization | Connector E2E | Failure injection | Live smoke |
|---|---:|---:|---:|---:|---:|
| `file.batch` casing | yes | schema → serde | actual rmcp stdio | invalid/unknown fields | ranged project read |
| edit diff/counts | yes | response semantics | actual file tool call | truncation/malformed patch | guarded temp edit |
| MCP reload | yes | config serde/health | watcher + call | invalid map/in-flight swap | endpoint change |
| `ManagedJob` / `job.*` | yes | protocol breaking rename | local + tunnel + Hub | capacity/restart/prune | process + skill |
| local ingress/CLI | yes | MCP descriptor parity | Unix socket + CLI | permissions/stale/restart | local tool call |
| managed MCP call | yes | envelope/schema | local + tunnel + fake downstream | timeout/cancel/disconnect/oversize | IDE MCP call |
| `mcp.batch` | yes | ordered envelopes | fake multi-server | partial/failFast/capacity | multi-call IDE checks |
| audit/confirmation | yes | redaction/annotations | confirmation provider harness | deny/unavailable/cancel | bounded manual confirmation |

## Acceptance Criteria
- The exact live `file.batch` ranged-read failure is fixed through the public connector path.
- Every advertised nested file-batch property deserializes or has an intentional tested rejection.
- `file.edit` diffs show the real changed content and `changedLines` is correct even after interior blank lines and under output truncation.
- Patch requirements are explicit enough that a model need not guess the hunk syntax.
- Valid `mcpServers` edits become effective without restart; invalid edits retain the prior live map.
- Process, skill-run, and MCP work share one `ManagedJobRegistry`, `maxActiveJobs` capacity, and retention system.
- Removed process/session aliases are absent; Normal exposes exactly 24 tools and Room 36.
- A local Unix-socket MCP ingress and CLI can list/call the exact running surface without tunnel latency or policy bypass.
- A fast MCP call completes inline; a long call returns a job id and can be inspected/waited without losing the result.
- Cancellation never claims remote termination without evidence.
- `mcp.batch` performs up to 16 bounded calls in one connector round trip, preserves order, respects per-server/global concurrency, and reports every child independently.
- Confirmation and audit remain per-child attributable, bounded, and free of raw argument/result values.
- Result, batch, runtime, history, and capacity limits are enforced and visible in `agent.info`.
- Agent restart never replays MCP side effects; stale active jobs are reported unknown/lost rather than completed or cancelled.
- Standalone Normal/Room, local Unix ingress, Hub MCP, and HTTP surfaces remain coherent under the new breaking job names; no compatibility wrappers remain.
- Full focused/workspace tests, formatting, diff checks, connector E2E, and live smoke pass.
- Final implementation worktree is clean after the phase commits; no push/deploy/tag/release occurs without a separate request.

## Decisions Made

| ID | Area | Status | Outcome | Rationale |
|---|---|---|---|---|
| D-01 | Priority | confirmed | Complete P0 file repairs before additive MCP work. | Separates current regressions from larger architecture changes. |
| D-02 | Connector casing | confirmed | Public camelCase is authoritative; test descriptor-to-runtime parity. | The running schema and serde currently contradict each other. |
| D-03 | Diff semantics | recommended | Use true line-diff operations and multi-hunk bounded unified output. | Fixes both missing and inflated change evidence. |
| D-04 | Patch syntax | recommended | Explicit full-range examples; retain standard omitted-count compatibility. | Clear model contract without unnecessary breakage. |
| D-05 | Reload | recommended | Atomically hot-swap validated MCP config; no explicit reconnect tool while clients are ephemeral. | Smallest correct solution for current architecture. |
| D-06 | Lifecycle | confirmed | Rename the internal core to `ManagedJob` and keep one registry for process, skill-run, and MCP work. | Avoids fake process fields and a duplicate MCP lifecycle. |
| D-07 | Public job API | confirmed | Expose only `job.get/list/cancel`; remove process/session wrappers, rename `process.batchExec` to `process.batch`, and migrate public config/info/protocol terminology to jobs without aliases. | Historical aliases consume tool context and add routing ambiguity in a fast-moving Agentic surface. |
| D-08 | MCP response | recommended | Use one stable managed-job envelope and version/document the response change. | Raw-only results cannot represent deferred work. |
| D-09 | Batch | recommended | 1–16 calls, parallel/sequential, ordered child jobs, no opaque parent. | Solves connector latency while keeping each call observable. |
| D-10 | Capacity | recommended | Shared global job capacity plus 8 global/2 per-server MCP concurrency. | Prevents MCP from bypassing existing resource control. |
| D-11 | Output bounds | recommended | 512 KiB child and 2 MiB aggregate valid-JSON summaries. | Bounded retention without corrupting structured results. |
| D-12 | Confirmation/audit | recommended | One aggregate confirmation, per-child redacted audit, no raw values. | Preserves safety and incident attribution. |
| D-13 | Cancellation | confirmed by requirement | Forward per-request cancellation but distinguish requested/cancelled/detached. | rmcp notification has no termination acknowledgement. |
| D-14 | Restart | recommended | Bounded memory retention, no replay, typed prior-generation loss/unknown state. | Honest V1 behavior without a second durable execution system. |
| D-15 | Delivery | confirmed | One focused commit per phase; stop after planning in this request. | Reviewable and rollback-friendly. |
| D-16 | Local integration need | confirmed | Provide a command-triggerable local ingress at the same execution layer as tunnel. | Enables realistic local debugging without network round trips or bypassing Agent policy. |
| D-17 | Local ingress design | recommended | Unix-domain-socket MCP on the same worker plus `run-as-local` and local CLI clients; no TCP/debug-only dispatch API. | rmcp already supports AsyncRead/AsyncWrite, and sharing AppState preserves jobs, audit, confirmation, and schema parity. |

## Implementation Discretion
- Exact line-diff crate/algorithm, provided full counts and bounded valid unified output meet the contract.
- Private `ManagedJob` enum/trait/helper layout.
- Semaphore and scheduler data structures.
- Exact typed error names where not frozen above, provided error categories remain stable and tests/documentation agree.
- Fake MCP server implementation, Unix-socket accept-loop layout, and assertion wording.
- Whether MCP execution limits are fixed constants or a backward-compatible defaulted config subsection; observable defaults and `agent.info` output may not change.

## Implementation Handoff
- **Plan maturity:** implementation_ready
- **Design phase:** complete
- **Implementation authorized:** no in the current request; begin only after a later explicit instruction
- **Entry phase:** Implementation Phase A — Connector contract repair
- **Frozen/recommended decisions:** D-01 through D-17
- **Open blocking decisions:** none
- **Verification convention:** focused tests per phase, then full workspace and real connector acceptance
- **Commit convention:** one focused commit for each Phase A through H
- **Design checkpoint:** not set; do not commit without a separate request
- **Next invocation:** `planning-with-files` only; do not re-run refinement unless a frozen contract changes

## Errors Encountered
| Error | Attempt | Resolution |
|---|---:|---|
| Historical codegraph path `/home/slhaf/Projects/Projects/AgenticGPT` did not exist. | 1 | Located the clean active repository at `/home/slhaf/Projects/AgenticGPT`. |
| Live `file.batch` ranged read rejected camelCase fields while expecting snake_case. | 1 | Preserved as the P0 reproduction; used independent reads and `process.batchExec` for planning discovery. |
| A discovery `file.search` requested `contextLines: 8`, above the bound of 5. | 1 | Reissued with the valid bound and recorded the error; no repository state changed. |
| Current `file.edit` reports false diff/line counts while updating planning files. | repeated observation | Verified resulting file revisions/content separately; made the reporting bug a P0 repair with regression coverage. |

## Phase I — v0.9 pre-release tunnel stdio restart recovery

**Status:** complete

- Diagnose v0.8 restart failures where a fresh hidden stdio worker receives a resumed `tools/call` before MCP `initialize`.
- Add a tunnel-only resumable stdio transport that privately restores rmcp initialization state, suppresses the private response, and replays the original request id unchanged.
- Ignore stale pre-initialize notifications without exposing request arguments or results in diagnostics.
- Preserve ordinary client-led initialize behavior and leave Local Unix ingress unchanged.
- Cover in-process stale-session recovery, normal/Room initialize paths, and an actual hidden-worker process E2E.
- Update v0.9 release, runtime, operations, and migration documentation.
