# Findings: Agentic Interface Repair and MCP Managed Execution

## Source of truth captured
- Room notebook scope `AgenticGPT` records the defects and feature needs from real company-project use on v0.8.0.
- Requested priority is P0 `file.batch` read bridge, P0 `file.edit` result reporting, then patch-contract clarity, MCP reload/reconnect, bounded `mcp.batch`, and reusable managed long-task execution.
- The user explicitly requires planning only in this session.

## Runtime/repository baseline
- Laptop Agent reports version `0.8.0`, profile `normal`, 23 tools, healthy Hub reporting, and managed-session capacity `24` with `0` active.
- Public MCP surface currently exposes only `mcp.list` and `mcp.callTool`.
- Active repository is `/home/slhaf/Projects/AgenticGPT` at clean `main`, commit/tag `698f43a` / `v0.8.0`, aligned with `origin/main`.
- Existing codegraph MCP configuration still points to the stale doubled path `/home/slhaf/Projects/Projects/AgenticGPT`; repository inspection must use the actual path unless that server is reconfigured.

## Prior plan evidence
- The previous scoped plan `2026-07-26-standalone-info-file-and-confirmation` owns the original `agent.info` and `file.*` implementation contracts.
- It identifies `crates/agentic-gpt/src/stdio_server.rs` as the central standalone schema/dispatch/test surface and describes direct file audit, confirmation, resource bounds, exact revisions, and mixed batch behavior.
- It froze connector-facing camelCase fields such as `startLine`, `endLine`, and `includeContent`; therefore a snake_case expectation inside batch decoding is a contract regression rather than an intended public shape.
- The previous plan explicitly required exact unified diff and bounded verification evidence, making the current misleading diff/line-count response a semantic defect.

## Direct planning-write observation
- Creating this plan through the current `file.edit` write operation produced `changedLines: {added: 1, removed: 1}` and a one-line diff although the new file contains many lines.
- This independently reproduces the broader result-reporting defect for write/create mode, not only replace/delete mode. The eventual regression matrix must cover create/write as well as replace, deletion, insertion, and patch.

## Initial architectural direction
- Treat file bridge/diff issues as isolated repair phases with connector-level tests before additive MCP capabilities.
- For long-running work, rename the internal lifecycle to `ManagedJob` and expose only `job.get/list/cancel`; do not preserve process/session lifecycle aliases merely for historical compatibility.
- Keep domain-specific creation schemas (`process.*`, `skills.run`, `mcp.*`) while centralizing lifecycle, capacity, retention, and cancellation evidence in one job registry.
- `mcp.batch` should be an Agent-level coordinator over ordinary downstream calls; it need not require downstream JSON-RPC batch support.
- Reload/reconnect must compare server definitions and preserve unchanged clients; changed-server failure must be reported explicitly rather than silently switching to a half-applied registry.
- A local integration channel should exercise the same public MCP schema/dispatch path as tunnel rather than call internal dispatch directly.

## Discovery update: file contract implementation map
- `crates/agentic-gpt/src/stdio_server.rs` defines `FileBatchArgs` with outer `#[serde(rename_all = "camelCase")]`, but the internally tagged `FileBatchOperationArgs` enum carries its own `#[serde(tag = "type", rename_all = "camelCase")]`; exact field-level behavior must be inspected because enum-level rename rules may rename variants without bridging struct-variant fields as assumed.
- `to_file_batch_operation` manually copies decoded connector arguments into `file_ops::BatchOperation`; the defect boundary is therefore likely in connector serde generation/decoding, not the core batch executor.
- `crates/agentic-gpt/src/file_ops.rs` owns both single-edit and batch-edit `changedLines`/diff response construction and later audit extraction. One repair should fix the shared diff producer rather than patching audit consumers.
- Current source search found only five `changedLines` references, all in `file_ops.rs`, which keeps the reporting repair localized.
- The live planning write reproduced the same truncated one-hunk/one-line evidence for `write` mode, so tests must cover all edit modes and not assume the bug is deletion-only.

## Live connector reproduction: `file.batch` read bridge
- Calling the current public `file.batch` tool with a read operation containing connector-schema fields `startLine`, `endLine`, and `includeContent` failed before execution with JSON-RPC `-32602`: `unknown field endLine, expected ... start_line, end_line`.
- The same connector schema does not permit callers to send `start_line`/`end_line`, so the public operation is impossible to satisfy. This is a confirmed connector-to-runtime serde bridge defect on the running v0.8.0 binary, not merely a stale notebook report.
- The error also shows `includeContent` remains camelCase while only the line-range fields become snake_case.
- Source confirms the exact cause: the `Read` struct variant explicitly renames `include_content` to `includeContent`, but `start_line` and `end_line` only have `#[serde(default)]`; enum-level `rename_all = "camelCase"` does not rename struct-variant fields here. The hand-written public JSON Schema still advertises `startLine`/`endLine`, so schema and serde diverge.
- Repair should add explicit field renames (or a shared connector DTO/schema derivation strategy) and an end-to-end test that obtains the public schema and dispatches a value built from those exact property names. A serde-only test or schema-only snapshot would miss this class again.

## Discovery update: diff and patch semantics
- `bounded_diff()` computes prefix/suffix and counts from `logical_lines()`. The latter uses `take_while(|line| !(line.is_empty() && text.ends_with(newline)))`; for any newline-terminated file, iteration stops at the first blank line anywhere, not only the synthetic trailing split item. This exactly explains zero/one-line diffs and incorrect `changedLines` after edits below the first blank line.
- The repair should replace `logical_lines()` with a representation that removes only the final synthetic empty element while preserving interior blank lines, then verify CRLF/LF, final-newline/no-final-newline, empty file, blank-only file, and multiple disjoint changes.
- `bounded_diff()` currently emits one coarse hunk between common prefix/suffix rather than a minimal multi-hunk diff. That is acceptable only if documented as bounded verification evidence; correctness of content/counts is mandatory. The plan should not expand scope to a sophisticated diff engine unless tests show the coarse form violates existing contract needs.
- `apply_unified_patch()` requires each hunk to start with `@@ ` and delegates to a strict parser for old/new ranges. The public schema description only says `Exact single-file unified diff`, so contract text/examples should explicitly show standard `@@ -oldStart,oldCount +newStart,newCount @@` headers and reject bare `@@` as `file_patch_invalid`.
- A discovery search initially requested `contextLines: 8`, exceeding the public maximum 5 and returning `file_context_limit_exceeded`; the corrected bounded search succeeded. No repository state changed.

## Discovery update: `ManagedJob` reuse boundary
- `sessions.rs` is process-shaped today: `ManagedSession` owns a Tokio `Child`, stdout/stderr `TailBuffer`s, command-oriented `SessionInfo`, skill lease, and command audit context.
- Reusable pieces already exist and should be extracted rather than duplicated: global active-work capacity, registration-before-start, `starting`/`waiting_confirmation`/terminal lifecycle, bounded wait (max 30s), 24-hour/100-terminal retention, tail truncation, terminal hooks, inspection/list, and cancellation intent.
- `start_prepared_managed_batch` already performs one capacity reservation for an entire process batch and preserves per-child IDs; this is a useful model for `mcp.batch` admission and child tracking.
- Directly forcing MCP work into the current process struct would create fake `program/args/exitCode/stdout/stderr` semantics. The chosen refactor is `ManagedJob` common metadata plus kind-specific process/skill/MCP runtimes and result payloads.
- Public compatibility is intentionally not preserved: remove `process.get/list/kill`, do not add `session.*`, rename `process.batchExec` to `process.batch`, and expose only `job.get/list/cancel` for lifecycle management.
- Current capacity accounting already counts process and `skills.run` work from one map. MCP work enters the same global pool, with an additional per-MCP-server limit.
- Existing cancellation intent is sufficient for local confirmation waits and child-process termination, but MCP requires richer `cancel_requested`/`cancelled`/`detached` evidence.

## Discovery update: current MCP lifecycle and reload gap
- `mcp.rs` creates a fresh downstream client for every `list_tools` and `call_tool`, executes one request, then calls `client.cancel()`. There is no persistent MCP client registry today, so hot reload is not a matter of swapping cached clients yet; the live config map itself is the effective routing source.
- The current call path has no inline wait bound, no output/result truncation, no retained request state, and no capacity admission. A long call holds the connector request until rmcp returns or transport fails.
- Authorization is performed before client creation via `authorize_mcp_tool_call`; each call writes command-shaped audit with server/tool and up to 1000 characters of serialized arguments. Batch/job design should preserve this authorization source but use a richer per-child audit/result envelope instead of hiding all calls behind one aggregate record.
- `watch_standalone_live_config()` validates the disk candidate but `apply_standalone_live_subset()` copies only `policy`, `path_policy`, and `limits`. `mcp_servers` remains intentionally restart-required and `agent.info` derives that mismatch by effective-vs-disk comparison.
- Because clients are currently ephemeral, the minimal safe Phase-1 reload repair can include `mcp_servers` in the validated live subset atomically: acquire the config write lock and swap the complete map only after candidate validation. Existing in-flight calls keep their cloned server config/client; new calls observe the new map.
- An explicit `mcp.reload`/`mcp.reconnect` operation becomes more valuable only if implementation later introduces persistent pooled clients. Planning should avoid inventing a registry solely to satisfy the word “reconnect”; first preserve current ephemeral-client simplicity unless batch/performance evidence requires pooling.

## Discovery update: current test gaps
- Existing `file_ops.rs` patch coverage verifies one valid standard hunk, CRLF preservation, conflict, wrong target, and multi-file rejection. It does not test a bare `@@`/missing ranges error message, zero-length range forms, interior blank lines, or diff/changed-line output.
- Existing `stdio_server.rs` tests dispatch `file.batch` with read fields that omit `startLine`/`endLine`; therefore they bypass the exact connector/runtime mismatch. They also call `dispatch()` directly with handwritten JSON rather than validating that advertised schema property names deserialize successfully.
- Existing file edit tests assert mutation/audit outcomes but do not assert full diff text or `changedLines`, which allowed the reporting bug through acceptance.
- The connector-level repair suite needs table-driven cases that derive/inspect the generated descriptor schema, then invoke the same public adapter/dispatch path with every optional camelCase field at least once.

## Discovery update: MCP confirmation and annotations
- Current `mcp.callTool` is always connector-annotated non-read-only and open-world, but not destructive. It does not project the selected downstream tool's annotations into the outer descriptor because the downstream tool is chosen dynamically at call time.
- Runtime authorization always asks for MCP confirmation unless that server has a temporary 15/30-minute allow. It does not inspect downstream `readOnly`/`destructive` annotations before deciding.
- `authorize_mcp_tool_call` is not cancellable today; cancellation-aware confirmation exists elsewhere for managed process work. MCP managed execution should add a cancellable authorization path so a job can be honestly cancelled while waiting for user confirmation.
- For `mcp.batch`, the conservative compatible rule is: pre-resolve/list tool metadata where available, but never weaken current confirmation solely because a downstream annotation says read-only. Aggregate one confirmation for all calls that would individually require it, grouped by server/tool with bounded argument summaries; temporary server allows can exempt only matching child calls.
- Each child still needs its own authorization/audit outcome. One aggregate confirmation record may reference child IDs, but a single opaque batch audit is insufficient for incident review.

## Discovery update: cancellation and persistence limits
- `process.kill` sets a cancellation flag, attempts `Child::kill`, then unconditionally reports terminal `killed` if still active. That is truthful for owned local child processes but cannot be reused verbatim for remote MCP work.
- MCP job state needs separate fields such as `cancelRequested`, `cancelCapability`, and `terminationEvidence`; terminal states should distinguish confirmed `cancelled`, local `detached`, and `unknown_remote_state` rather than mapping all requests to `killed`.
- The current managed registry is memory-only. Terminal entries survive up to 24 hours/100 records only while the Agent process remains alive. Therefore V1 must explicitly report `job_lost_after_restart` / `unknown_after_restart`; silent disappearance or side-effect replay is unacceptable.
- `AppState` currently has one concrete `sessions: HashMap<String, ManagedSession>`. A shared `ManagedJob` enum or common metadata plus kind-specific storage can preserve one capacity/retention registry and avoid a second MCP-only map, but this refactor should be isolated before MCP execution logic lands.

## Discovery update: connector-contract test seam
- `tool_descriptor()` and `tool_schema()` build the exact public MCP descriptor in `stdio_server.rs`; `dispatch_with_lifecycle()` validates top-level arguments and then serde-decodes the connector DTOs. Module tests can exercise both without adding a separate external test binary.
- Nested `file.batch.operations` fields are not checked by the top-level validator; serde `deny_unknown_fields` is the authoritative nested bridge. Therefore the regression test should read the `operations.items.oneOf` schema, assert advertised property names, and dispatch representative JSON containing every advertised nested field.
- `standalone_surface()` already hashes descriptor schema/annotations for stale-surface detection. The repair test should also assert its revision changes when the connector schema changes, but revision hashing alone cannot prove runtime deserialization parity.
- Lifecycle helpers currently recognize active/failure states only through process-shaped `session` and batch `process` fields. The Job migration must update these helpers so human terminal reporting and run-event completion do not falsely mark an active MCP job as completed.

## Discovery update: public `job.*` surface and tool counts
- Hub currently exposes `session.start/list/inspect/wait/kill`, while standalone exposes `process.get/list/kill`; both are historical projections of the same internal process-shaped map.
- Keeping both families would permanently consume tool-description context and force the model to choose among equivalent lifecycle routes. The revised design removes both families rather than layering compatibility wrappers.
- Creation remains domain-specific: `process.exec`, `process.batch`, `skills.run`, `mcp.callTool`, and `mcp.batch`. Lifecycle is only `job.get`, `job.list`, and `job.cancel`.
- `job.get` merges inspection and bounded wait through optional `waitSeconds`, avoiding separate inspect/wait tools.
- Hub full surfaces remove session tools; coordinator `hub.session.list/get` becomes `hub.job.list/get`. Protocol `SessionInfo`/session commands become `JobInfo`/job commands as an intentional v0.9 breaking rename.
- Based on the current 23-tool Normal surface: remove three process lifecycle tools, rename batch without an alias, add three job tools and `mcp.batch`, yielding exactly 24 Normal tools. Room adds its existing 12 tools, yielding 36.
- Skill installation remains a distinct install workflow with `installId`; only `skills.run` participates in `ManagedJob` in this scope.

## Discovery update: capacity and result bounds
- Existing `limits.maxActiveSessions` already supplies the correct shared admission semantics, but the v0.9 Job migration should rename it to `limits.maxActiveJobs` without a legacy alias. Existing `maxConcurrentTasks` is used for short batch execution and should not silently become a per-server MCP policy because its semantics/config name are broader and currently default to 2.
- Add explicit bounded MCP constants/config only where operational tuning is necessary. Recommended V1 contract: maximum 16 batch calls; at most 8 concurrent child starts globally; at most 2 active calls per server; 30-second maximum inline wait; 512 KiB retained result per child; 2 MiB aggregate batch response/result budget; 15-minute maximum tracked runtime; shared 24-hour/100-terminal retention.
- Global active managed jobs still cannot exceed `maxActiveSessions`; `mcp.batch` must reserve capacity atomically for children that promote to managed jobs, and reject or run inline within remaining capacity rather than oversubscribe during promotion races.
- Existing process sessions retain 64 KiB stdout and stderr tails. MCP results are structured JSON, so they need prefix-safe structured truncation metadata rather than reuse of text tail buffers.
- Protocol/Hub code assumes command-shaped sessions in descriptions and caches. Because compatibility is not a goal, v0.9 should replace these with `JobInfo`, job commands/events/caches, kind-specific payloads, `maxActiveJobs`, and `agent.info.execution.jobs` rather than retaining fake process fields or dual names.

## Discovery update: rmcp request cancellation evidence
- Cargo.lock resolves `rmcp 1.7.0`.
- rmcp exposes a per-request `RequestHandle` containing the MCP `RequestId`; `RequestHandle::cancel(reason)` sends `notifications/cancelled` for that request. Its own documentation says the peer SHOULD cease work, not that termination is acknowledged.
- `RunningService::cancel()` closes the whole client/service and transport; it is connection cancellation, not proof that one remote tool stopped.
- The implementation therefore can forward a standards-compliant per-request cancellation notification, but the public contract must remain `cancel_requested` until downstream response/error supplies stronger evidence. If no such evidence arrives within a bounded grace period, close/detach the client and report `detached` / remote state unknown rather than `cancelled`.
- High-level `Peer<RoleClient>::call_tool()` awaits the response directly. Managed MCP execution must use the lower-level request handle path so it can retain request id, cancel, await, and audit independently.

## Discovery update: configuration and parity
- `agent.info` currently derives `mcpServers` as restart-required because `apply_standalone_live_subset()` omits that map. Its tests already cover restart-field derivation, so reload acceptance can directly assert that a valid live swap removes `mcpServers` from `restartRequiredFields`.
- `validate_standalone()` currently validates tunnel identity/secrets only; MCP server entries need focused validation before the map becomes live-reloadable.
- Hub and standalone both route through the same `mcp.rs` functions, while Hub has a fixed 35-second request timeout. Managed calls must return a job envelope well before that timeout and update Hub/tunnel/local surfaces together.
- Hub currently caches `SessionInfo` by agent/session and exposes inspect/wait/kill. The revised contract deliberately replaces that cache/protocol/tool terminology with jobs and requires regression coverage for the breaking migration.

## Discovery update: local integration ingress feasibility
- The hidden `stdio-worker` already hosts the exact standalone MCP tool server and routes through `local_service::dispatch`, but it is supervisor-token protected and its stdio is owned by the tunnel client. Manually invoking it is therefore not a suitable public local-debug contract.
- A one-shot CLI that directly constructs `HubCommand` and calls `local_service::dispatch` would be easy but incorrect for this goal: it would bypass advertised schema validation/annotations, create a separate in-memory registry, and lose long-running jobs when the CLI process exits.
- rmcp 1.7 implements `IntoTransport` for any Tokio `AsyncRead + AsyncWrite`; `tokio::net::UnixStream` can therefore carry the same MCP protocol without a custom transport or a network HTTP server.
- The recommended design is a Unix-domain-socket MCP listener inside the same worker, serving a clone of the same MCP server/AppState concurrently with tunnel stdio. This is a true peer ingress: local and tunnel calls share live config, confirmation, audit, capacity, `ManagedJob` state, and results.
- `run-as-standalone` should expose that socket automatically. A new `run-as-local` mode should acquire the existing config run lock and expose the same socket without launching the tunnel client, enabling CI and local development.
- `agentic-gpt local list-tools` and `agentic-gpt local call` should be real rmcp clients over the socket. Arguments should support stdin/file input to avoid shell-quoting damage; structured result goes to stdout and logs to stderr.
- Security can remain local-user scoped: private runtime directory (`0700`), owner-only Unix socket, no TCP binding, typed unavailable/restart errors, and cautious stale-socket cleanup. The existing supervisor remains the run-lock owner in tunnel mode.
- One worker can now have multiple ingress sources, so audit/request source must be per connection (`tunnel:stdio`, `local:unix`, Hub) rather than inferred only from `RuntimeModel.transport` or hard-coded `tunnel:*` strings.
- This local ingress adds no MCP tool and does not change the 24/36 target counts. It is best implemented before `ManagedJob`/managed MCP phases so later work has a fast real transport-level test channel.

## Workflow efficiency note
- Because the live `file.batch` read bridge is the defect under investigation, later repository discovery uses `process.batchExec` with bounded read-only `rg`/`sed` commands. This avoids relying on the broken API and materially reduces connector round trips.

## Contract-surface map

| Surface | Status | Repository-grounded outcome |
|---|---|---|
| Scope/ownership | covered | P0 file defects are isolated before P1 MCP architecture; one `ManagedJob` subsystem serves process, skill-run, and MCP work. |
| Inputs/outputs | covered | CamelCase parity, managed-job envelope, bounded batch schema, valid result summaries, breaking job names, and local CLI arguments are frozen. |
| Lifecycle/concurrency | covered | One `ManagedJobRegistry`, queued/running/waiting/terminal states, atomic batch admission, global/per-server limits, and sequential coordinator are specified. |
| Failure/cancellation | covered | Typed file failures, partial batch outcomes, request-level cancellation notification, unconfirmed detach, and timeout truthfulness are explicit. |
| Data/retention/restart | covered | 24-hour/100 terminal bounded memory, no replay, boot-generation loss signal, and Hub stale-cache reconciliation are explicit. |
| Security/trust | covered | Current confirmation remains conservative; audit and previews exclude raw MCP values/results. |
| Operations/observability | covered | Hot reload, local Unix ingress readiness, per-ingress audit source, `agent.info` diagnostics, and time/capacity/output bounds are explicit. |
| Surface parity | covered | Tunnel stdio, local Unix MCP, Protocol, Hub MCP, HTTP, breaking `job.*` names, annotations, and versioned response change are mapped. |
| Verification | covered | Unit, schema/serde contract, tunnel/local rmcp parity, Unix-socket/CLI E2E, fake downstream server, failure injection, full workspace, and live smoke are mapped. |

## Decision rationale summary
- D-03 chooses a real line-diff stream rather than only fixing `logical_lines()`: the current coarse prefix/suffix algorithm can overcount unchanged middle lines across disjoint edits even after the blank-line bug is fixed.
- D-05 omits an explicit reconnect tool because every current call creates a new client. Atomic config-map hot reload already changes the endpoint used by the next call; a reconnect API would have no additional truthful object to reconnect.
- D-07 deliberately removes process/session lifecycle aliases. `job.cancel` can still report strong local process termination evidence while retaining advisory semantics for MCP cancellation.
- D-08 accepts a documented v0.9 response-envelope change because a raw downstream result cannot represent a deferred job, result bounds, or cancellation state without an unstable union.
- D-09 does not require a parent job. Child jobs are sufficient for inspection/cancellation, while a bounded background coordinator can advance sequential queued children by batch correlation id.
- D-14 chooses bounded memory over a durable journal for V1. The restart contract forbids replay and exposes loss/unknown state, satisfying delayed-turn retrieval without introducing a second recovery system.

## Remaining implementation discretion
- Exact private `ManagedJob` type decomposition and the line-diff crate/algorithm, provided the frozen public behavior and bounds are met.
- Exact semaphore/container choices for global and per-server MCP concurrency.
- Exact fake MCP server harness and Unix-socket accept-loop implementation for deterministic delay, cancellation, failure, permission, restart, and oversized-result tests.

## 2026-07-27 — tunnel stdio initialization recovery

- OpenAI tunnel-client v0.0.10 skips startup MCP probing for stdio, reuses one shared stdio connection, and forwards polled commands directly. After the child is restarted, the control plane can therefore continue an already-initialized logical flow against a fresh rmcp server.
- rmcp 1.7 treats the first non-ping request being anything other than `initialize` as a fatal server-initialization error. The hidden worker exits, tunnel-client shuts down on child EOF, and the Agentic supervisor restarts the pair.
- Merely returning a pre-initialize error would stop the crash but could leave the logical connector unable to recover because it may never resend `initialize`.
- The selected repair is a tunnel-only transport shim: stage the first resumed request, inject a private initialize with bounded empty client capabilities, suppress only its matching response, then replay the staged request. The original request id and response remain the only pair visible to tunnel-client.
- `AgentMcpServer` does not consume peer capabilities or client identity from `RequestContext`, so the private initialization does not alter confirmation, notification, cancellation, or tool execution behavior.
- Local Unix MCP retains its ordinary rmcp client-led handshake and does not use the recovery shim.
