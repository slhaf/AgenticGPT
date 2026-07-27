# Progress: Agentic Interface Repair and MCP Managed Execution

## 2026-07-27

### Discovery and reproduction
- Read Laptop `agent.info`: v0.8.0, Normal profile, 23-tool surface, healthy Hub reporting, shared managed-session capacity 24.
- Read Room notebook current state for scope `AgenticGPT` and captured all requested defects/features.
- Read `planning-with-files`, initialized this scoped plan, and switched `.planning/.active_plan` to it.
- Located the clean repository at `/home/slhaf/Projects/AgenticGPT`; baseline is `698f43a` (`v0.8.0`), `main == origin/main`.
- Read the prior file/info/confirmation plan and its frozen contracts.
- Reproduced the public `file.batch` failure: advertised `startLine` / `endLine` are rejected while runtime expects snake_case.
- Confirmed source root cause in `FileBatchOperationArgs::Read`: missing explicit serde renames on `start_line` / `end_line`.
- Reproduced misleading `file.edit` evidence while writing planning files.
- Confirmed source root cause in `logical_lines()`: `take_while` stops at the first interior blank line for newline-terminated text.

### Architecture inspection
- Mapped `stdio_server.rs` descriptor/schema/serde/dispatch seams and current connector test gaps.
- Mapped `mcp.rs`: each list/call creates an ephemeral rmcp client; no persistent registry exists.
- Mapped standalone config watcher: only policy/pathPolicy/limits currently hot reload; `mcpServers` remains restart-required.
- Mapped `sessions.rs`: one process-shaped managed registry already owns capacity, wait, retention, cancellation flag, terminal hooks, and process/skill lifecycle.
- Mapped Protocol/Hub session caches and existing inspect/wait/kill surfaces.
- Verified Cargo resolves rmcp 1.7.0 and inspected its per-request `RequestHandle::cancel` plus whole-service cancellation behavior.
- Confirmed MCP request cancellation is advisory (`notifications/cancelled`) and has no termination acknowledgement by itself.
- After the live `file.batch` defect was established, used `process.batchExec` with bounded read-only `rg`/`sed` commands for faster repository discovery.

### Refinement round 1: contract freezing (lifecycle details later superseded)
- Read `refine-implementation-plan` and its planning-extension, decision, and handoff-readiness references.
- Froze the current-defect contracts: connector casing parity, real line-diff semantics, bounded multi-hunk evidence, and explicit patch examples/errors.
- Froze hot reload as atomic validated MCP config-map replacement; no explicit reconnect tool while clients remain ephemeral.
- The initial draft froze generic managed sessions plus compatibility wrappers; the later `ManagedJob` revision below explicitly supersedes that direction.
- Froze managed single-call, result bounds, audit redaction, aggregate confirmation, and truthful cancellation states.
- Froze `mcp.batch`: 1–16 calls, parallel/sequential, atomic capacity admission, ordered child jobs, fail-fast semantics, and no opaque parent.
- Froze retention/restart boundary: bounded memory, no replay, prior-generation loss/unknown state.
- Initially defined seven implementation phases; the later local-ingress revision expands this to eight focused phases A–H.
- Defined unit, serialization, actual rmcp connector, fake downstream MCP, failure-injection, full workspace, and live-smoke matrices.

### Design revision: `ManagedJob` and local integration ingress
- Confirmed the internal lifecycle rename to `ManagedJob`, with one registry for process, `skills.run`, and MCP work.
- Removed the prior compatibility-wrapper direction. The frozen lifecycle surface is only `job.get`, `job.list`, and `job.cancel`; `process.get/list/kill` and all `session.*` aliases are removed.
- Renamed `process.batchExec` to `process.batch` without an alias. Hub full and coordinator session names also migrate to job terminology.
- Extended the breaking rename consistently to `JobInfo`, job protocol events/caches, config `limits.maxActiveJobs`, and `agent.info.execution.jobs`.
- Recalculated exact standalone targets: Normal 24 tools and Room 36 tools.
- Inspected the CLI, standalone supervisor, hidden stdio worker, `local_service`, runtime model, instance lock, existing E2E tests, and rmcp 1.7 transport implementations.
- Confirmed that the hidden `stdio-worker` is not itself a suitable local contract: it is supervisor-token protected, its stdio belongs to the tunnel client, and a one-shot direct-dispatch CLI would not share retained jobs or test the public MCP schema path.
- Confirmed rmcp accepts arbitrary Tokio `AsyncRead + AsyncWrite`, making a same-worker Unix-domain-socket MCP ingress feasible without a custom protocol or HTTP debug server.
- Froze the local design: tunnel workers serve stdio and owner-only Unix MCP concurrently over the same `AppState`; `run-as-local` serves the Unix ingress without tunnel dependencies; `local list-tools` and `local call` are real rmcp clients.
- Froze security/operations boundaries: transport-neutral private runtime path, `0700` directory, owner-only socket, same-UID peer verification where supported, no TCP, typed unavailable/restart errors, cautious stale cleanup, and per-ingress audit attribution.
- Added Implementation Phase D for the local ingress before `ManagedJob`/managed MCP work; later phases shifted to E–H so the new channel can be used for realistic local contract testing.
- Implementation remains unauthorized; no product code, test code, config, commit, push, deployment, tag, or release was created.

### Implementation Phase A completed
- Added explicit `startLine` / `endLine` serde bridges for nested `file.batch` read operations; public camelCase now reaches runtime fields.
- Audited all read/search/edit nested connector properties and encoded their observable defaults in the advertised schema.
- Added a descriptor-to-runtime parity test covering every advertised nested optional field, defaults, runtime mapping, and snake_case rejection.
- Added a real in-process rmcp initialize/list/call test that reads the advertised `file.batch` schema, performs a ranged read with camelCase fields, and confirms snake_case returns MCP INVALID_PARAMS.
- Confirmed the standalone surface revision advances from the running v0.8.0 revision while Normal/Room tool counts and schema budgets remain valid.
- Verification passed: focused `file_batch` tests 5/5; exact tool surface and schema-budget tests; full Agent unit suite 194/194; standalone supervisor integration suite 5/5; `cargo fmt --all --check`; `git diff --check`.
- Existing warnings about unused `RunMode::Room` / `RunMode::role` remained unchanged.
- Focused commit created: `504fb8b fix(agent): align file batch connector contract`.
- No push, deployment, tag, or release was performed.
- Phase B is now active.

### Implementation Phase B completed
- Replaced the coarse prefix/suffix diff with `similar 3.1.1` line diff operations and bounded multi-hunk unified output.
- `changedLines` now counts the complete insert/delete stream before 64 KiB UTF-8-safe output truncation.
- Fixed logical line handling so interior blank lines, empty/blank-only content, CRLF, and final-newline differences remain visible.
- Tightened patch parsing: standard zero-length ranges, abbreviated count=1, exact old/new range consistency, newline markers, safe invalid-line handling, and malformed/extra range rejection.
- Added generated-diff-to-patch-parser round-trip coverage for disjoint hunks and final-newline changes.
- Updated public `file.edit` and nested `file.batch` patch descriptions plus standalone documentation with explicit `@@ -1,2 +1,2 @@` syntax and bare-`@@` rejection.
- Added connector assertions for replace, dry-run, no-op, create, overwrite, patch, and batch edit diff/count responses.
- Verification passed: focused bounded-diff, unified-patch, file-edit, file-batch, and schema tests; full Agent unit suite 201/201; standalone supervisor integration suite 5/5; `cargo fmt --all --check`; `git diff --check`.
- Existing warnings about unused `RunMode::Room` / `RunMode::role` remained unchanged.
- Focused commit created: `3cd6571 fix(agent): repair file edit change evidence`.
- No push, deployment, tag, or release was performed.
- Phase C is now active.

### Implementation Phase C completed
- Added complete MCP server-map semantic validation for server ids, supported transports, absolute HTTP(S) endpoints, non-empty stdio commands, and surrounding-whitespace/control rejection.
- MCP config CLI mutations validate the complete candidate map before writing; invalid additions leave the disk file unchanged.
- Standalone startup, hidden worker startup, Hub startup, and both Hub/standalone config watchers now reject invalid MCP maps and retain the last valid live state.
- Standalone live reload atomically swaps the complete `mcpServers` map together with policy/path/limits only after full candidate validation.
- Verified already-cloned in-flight server definitions retain the old endpoint while new calls read the new map.
- `agent.info` now treats `mcpServers` as live, removes it from restart-required fields, reports a deterministic config revision plus configured/enabled counts and per-call lifecycle, and exposes no endpoint values.
- Extended the real hidden-worker watcher E2E to cover MCP add, endpoint change, disable, remove, and semantically invalid transport with last-good retention.
- Updated standalone documentation: live MCP validation, atomic behavior, current per-call client lifecycle, and why no reconnect command exists.
- Verification passed: focused MCP/config/reload/info tests; real hidden-worker reload E2E; full Agent unit suite 207/207; standalone supervisor integration suite 5/5; `cargo fmt --all --check`; `git diff --check`.
- Actual Laptop server ids/transports (`chrome`, `codegraph`, `idea`) satisfy the new validation contract; endpoint values were not emitted.
- Existing warnings about unused `RunMode::Room` / `RunMode::role` remained unchanged.
- Focused commit created: `f9deb24 feat(agent): hot reload mcp server configuration`.
- No push, deployment, tag, or release was performed.
- Phase D is now active.

### Implementation Phase D completed
- Refactored the standalone MCP adapter into a transport-neutral `AgentMcpServer` with per-connection `RequestIngress` (`tunnel:stdio` / `local:unix`) while retaining one shared `AppState` and tool surface.
- Added an owner-only Unix MCP listener at `~/.agentic_gpt/runtime/agent/<agentId>/mcp.sock`, with `0700` runtime directory, `0600` socket, same-UID peer checks, 16-connection bound, path-length bound, active/stale socket discrimination, inode-safe cleanup, symlink-directory rejection, and empty-directory cleanup.
- Hidden tunnel workers now serve stdio and local Unix MCP concurrently; `run-as-local` serves the same Normal/Room capabilities without tunnel credentials, tunnel binary, or Hub reporting.
- Added real rmcp CLI commands: `local list-tools` and `local call`, with `--config` before/after subcommands, bounded inline/file/stdin JSON object arguments, structured stdout, and typed stderr errors.
- Added LocalUnix runtime identity/capabilities, `agent.info.connections.localMcp` readiness/path, shared live reload, SIGINT/SIGTERM shutdown, and same config run-lock exclusion.
- Parameterized process/batch/skill/MCP audit and lifecycle ingress sources so local calls no longer appear as tunnel or Hub calls.
- Added unit/security tests, tunnel/local descriptor parity, local audit attribution, real local-only CLI E2E, local MCP hot reload, hidden tunnel-worker local peer E2E, permission/lock/stale/symlink/restart/cleanup tests, and documentation.
- Verification passed: full Agent unit suite 216/216; local CLI E2E 1/1; standalone supervisor E2E 5/5; `cargo fmt --all --check`; `git diff --check`.
- Existing warnings about unused `RunMode::Room` / `RunMode::role` remained unchanged.
- Focused commit created: `ea19b42 feat(agent): add local mcp control ingress`.
- No push, deployment, tag, or release was performed.
- Phase E is now active.

### Implementation Phase E completed
- Replaced the process-shaped session registry with `ManagedJob`, `ManagedJobRuntime::Process`, `JobKind`, `JobState`, `JobInfo`, and boot-generation-prefixed `job_*` identifiers.
- Unified `process.exec`, `process.batch`, and `skills.run` on one Job creation/lifecycle path with bounded inline waits, shared capacity, retention, audit, confirmation, output tails, skill leases, and terminal hooks.
- Exposed only `job.get`, `job.list`, and `job.cancel`; renamed `process.batchExec` to `process.batch`; removed `process.get/list/kill`, managed `session.*`, `hub.session.*`, old HubCommand variants, and unused `TaskResult`/`BatchExecResult` protocol types without aliases.
- Migrated config and diagnostics to `limits.maxActiveJobs` and `agent.info.execution.jobs`; strict limits deserialization rejects `maxActiveSessions` and the historical never-functional `sessionIdleTimeoutSecs`. Removed the dead idle setting and unused activity bookkeeping instead of renaming them.
- Migrated Hub cache, connection state, reliable commands, Full/Coordinator MCP surfaces, HTTP routes, run history, database columns, OpenAPI paths/schemas, docs, and skill guidance to Job terminology.
- Added required `Hello.bootGeneration`; generation changes mark only active cached Jobs as `unknown_after_restart`, retain terminal Jobs, and never replay side effects. Fixed a real Hello reconnect deadlock by releasing the Agent registry lock before pending replay lookup.
- Agent command responses now publish initial Job snapshots before the reliable response so active work immediately enters Hub cache; `job.list` has one stable `{jobs:[...]}` envelope and filtered cached fallback across standalone, Hub MCP, and HTTP.
- Made process cancellation truthful and non-blocking for the registry: process kill occurs outside the Job lock and reports `cancelled`, `cancelled_before_start`, `already_terminal`, or `cancel_failed` with bounded termination evidence. Process exit/cancel races no longer become false failures.
- Updated static Actions OpenAPI to `/v1/process/*` and `/v1/jobs/*`; all local `$ref` values resolve, old execution/session paths and schemas are absent, and batch rejection is modeled as a valid response branch.
- Verified exact standalone surfaces remain Normal 23 / Room 35 before Phase G adds `mcp.batch`.
- Verification passed: Agent unit suite 210/210; local Unix CLI E2E 1/1; standalone supervisor E2E 5/5; Hub suite 62/62; Protocol suite 10/10; `cargo fmt --all --check`; `git diff --check`; OpenAPI YAML/reference validation.
- Current Laptop config remains v0.8-shaped (`maxActiveSessions` plus `sessionIdleTimeoutSecs`) and must be explicitly migrated before deploying the new binary; this phase did not modify user configuration.
- Focused commit created: `d2c7bea feat(agent): replace sessions with managed jobs`.
- No push, deployment, tag, or release was performed.
- Phase F is now active.

### Implementation Phase F completed
- Added `ManagedJobRuntime::Mcp` and converted single `mcp.callTool` execution into the shared Job registry, capacity limit, retention, lifecycle, audit, terminal hooks, Hub cache, and `job.get/list/cancel` surface.
- Extended `McpCallToolRequest` with bounded `waitSeconds` (default 5, max 30) and absolute `timeoutSeconds` (default 300, range 1..900).
- Added the lightweight/detail split: `JobInfo` remains safe for lists and Hub cache; `JobDetail` retains a bounded result/error and explicitly reports `detailAvailable`; all process, skill, and MCP creation paths now return one generic `JobResponse`.
- Removed the redundant `SkillRunResponse` protocol type and the Hub raw downstream MCP result passthrough path.
- Registered selected-server validation failures as audited `rejected` MCP Jobs rather than returning pre-registry errors.
- Used rmcp `send_cancellable_request` to retain the exact downstream request id and peer. Timeouts and `job.cancel` send `notifications/cancelled` for that request id; absence of downstream terminal proof produces `detached` rather than a false cancellation claim.
- Bound confirmation, downstream connect/request allocation, notification delivery, response grace, and client cleanup. Network operations do not run while holding the Job registry lock; cleanup/notification grace is capped at 2 seconds.
- Added cancellable Hub MCP confirmation and verified cancellation removes the pending confirmation sender before any downstream request starts.
- Added argument/result limits: arguments must be objects and serialize to at most 256 KiB; results up to 512 KiB are retained, while larger results keep byte count, SHA-256, and an 8 KiB UTF-8-safe preview.
- Bounded MCP metadata further: confirmation/audit include only a sorted subset of at most 32 argument keys, each capped at 80 characters, plus total count, truncation flag, bytes, and SHA-256. Raw argument values and raw results never enter confirmation messages, audit, reporting, or Hub cache.
- Bounded Job error messages to 8 KiB UTF-8 safely and prevented late MCP results/errors from overwriting terminal Jobs.
- Added safe MCP audit fields: server/tool, bounded key metadata, argument/result bytes and hashes, MCP config revision, terminal state, request source, and termination evidence.
- Updated standalone/Hub MCP tools, HTTP routes, Apps dispatch, docs, and strict Actions OpenAPI. `mcp.callTool` now returns `JobResponse`; `job.get/cancel` return `JobDetail`; cache-only reads set `detailAvailable=false`; unavailable cancellation is explicit rather than stale-cache success.
- Added real rmcp duplex client/server tests covering inline and deferred results, `isError`, oversized results, exact-id timeout cancellation, user cancellation, detached outcomes, confirmation cancellation cleanup, shared capacity, oversized arguments, audit redaction, bounded key summaries, and bounded UTF-8 errors.
- Verification passed: Agent suite 221/221; local Unix CLI E2E 1/1; standalone supervisor E2E 5/5; Hub suite 56/56; Protocol suite 11/11; doc tests; `cargo fmt --all --check`; `git diff --check`; strict OpenAPI schema and local-reference validation.
- Focused commit created: `aafdad9 feat(agent): manage downstream mcp calls as jobs`.
- No push, deployment, tag, or release was performed.
- Phase G is now active.

### Implementation Phase G completed
- Added public `mcp.batch` across standalone/local MCP, Hub Full MCP, Hub reliable commands, HTTP `/v1/mcp/batch`, Apps dispatch, protocol types, run history, docs, and strict Actions OpenAPI.
- Froze the standalone surface at Normal 24 / Room 36 tools; tunnel stdio and local Unix ingress advertise the same descriptor/schema revision.
- Added 1..16 ordered calls with optional unique bounded ids, object-only arguments, 256 KiB per-call and 2 MiB aggregate argument limits, parallel/sequential modes, safe `failFast`, bounded inline wait, and per-child execution deadlines.
- Implemented complete preflight followed by one-lock atomic shared-capacity admission. Validation/capacity failures create no child Jobs, send no confirmation, start no downstream work, and write exactly one bounded aggregate rejection audit.
- Added `batchId`, optional `batchCallId`, and `batchIndex` to child Job/audit correlation; Hub initial response extraction caches every `results[].job` snapshot.
- Added one aggregate confirmation after excluding already-temporarily-allowed servers. Single-server scope can grant 15/30-minute server allow; multi-server scope only permits batch-scoped allow/deny. Cancelling any child while confirmation is pending cancels the aggregate confirmation, clears the pending sender, closes all children, and starts no downstream call.
- Added one shared MCP scheduler used by both single and batch calls: global active limit 8, per-server limit 2, cancelable queued state, fair all-or-release permit acquisition, and `agent.info.mcp.concurrency` active/queued diagnostics.
- Parallel mode preserves input-order responses while respecting 8/2 limits. Sequential mode waits for each child terminal state. `failFast=true` only marks not-yet-started children `skipped`; already-started downstream calls are never cancelled.
- Added a strict 2 MiB serialized aggregate response budget. Later child result bodies are removed first while preserving Job ids, state/error, result byte count/hash/preview, and later `job.get` access to the retained child detail.
- Added one aggregate audit per accepted batch plus one child audit per child; aggregate audit contains safe counts/mode/fail-fast/confirmation/child ids/outcome/error code/clipping only, and child audit contains batch correlation without raw arguments/results.
- Added deterministic real-rmcp duplex tests for atomic rejection, capacity rejection, one-confirmation single/multi-server behavior, temporary allow, confirmation cancellation cleanup, sequential fail-fast, ordered results, per-server/global concurrency, two-level result budgets, aggregate/child audit correlation, and Hub child-cache extraction.
- Updated README, standalone runtime, interfaces, Hub/standalone instructions, tool annotations, exact surface tests, protocol serialization/default tests, scheduler diagnostics, and strict OpenAPI schemas/local-reference checks.
- Verification passed: Agent suite 228/228; local Unix CLI E2E 1/1; standalone supervisor E2E 5/5; Hub suite 56/56; Protocol suite 12/12; doc tests; `cargo fmt --all --check`; `git diff --check`; strict OpenAPI parsing/reference/bounds validation.
- Focused commit created: `f6ae26f feat(agent): add bounded managed mcp batch calls`.
- No push, deployment, tag, or release was performed.
- Phase H is now active.

### Implementation Phase H completed
- Bumped all three crates and the lockfile from `0.8.0` to `0.9.0`; enabled and tested standard `--version` output for both binaries.
- Rebuilt the damaged English README tail and rewrote the Chinese README against the final v0.9 Job/local/tunnel/MCP contracts.
- Added a strict non-secret `config.example.json`, English/Chinese v0.9 migration guides, v0.9.0 release notes, and a deployment acceptance checklist.
- Documented the coordinated Hub/Agent upgrade boundary, removed-tool/route/config migrations, 24/36 surfaces, managed MCP envelopes, safe duplicate-id batch smoke, and explicit no-auto-release behavior.
- Added strict config-example parsing/safety coverage; the example enables no MCP server and contains no real credential material.
- Added real connector-path `mcp.batch` smoke through Local Unix MCP and hidden tunnel stdio. Both validate schema -> serde -> dispatch -> typed `mcp_batch_failed` -> one `validation_rejected` aggregate audit with no child downstream call.
- Added Hub Apps `mcp.batch` descriptor coverage for 1..16 calls, 0..30 inline wait, 1..900 timeout, required fields, and non-read-only/open-world/non-destructive annotations.
- Added clippy to CI and made the entire workspace/all-targets clean under `-D warnings`. Mechanical fixes were applied directly; intentional orchestration arity and Axum `Err(Response)` helpers use narrow function-local lint allowances.
- Boxed the large `AgentMessage::RunReport` payload to satisfy enum-size lint without changing the serialized protocol shape.
- Added explicit lock-file open semantics, deterministic sort helpers, derived defaults, bounded test cleanups, and other behavior-preserving lint repairs.
- Verified all relative documentation links and every local OpenAPI `$ref`; the v0.9 Actions batch route remains non-consequential at the HTTP approval layer while its MCP tool annotation remains side-effecting/open-world.
- Live read-only connector smoke succeeded for Laptop, Tablet, Server, and Room: all four were `health=ready`, had valid live config, and had Hub reporting connected. They remain intentionally undeployed on v0.8.0 (Normal 23 / Room 35), so no v0.9 runtime claim or config migration was made.
- Final verification passed: Agent suite 230/230; Local Unix CLI E2E 1/1; standalone supervisor E2E 5/5; Hub suite 58/58; Protocol suite 12/12; doc tests; `cargo fmt --all --check`; `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `git diff --check`; strict OpenAPI parse/reference checks; documentation link checks; both binaries report `0.9.0`.
- Focused commit created: `7069c97 chore(release): finalize v0.9 interface acceptance`.
- No push, deployment, user-config migration, tag, GitHub Release, or restart was performed.
- All implementation phases A–H are complete.

### Initial planning maturity transition (historical)
- Updated `task_plan.md` to `implementation_ready`.
- Implementation remains explicitly unauthorized in this request; entry phase is Phase A after a later implementation instruction.
- Open blocking decisions: none.
- No product source, test, config, generated artifact, commit, push, deployment, tag, or release was created.

## Errors Encountered
| Error | Attempt | Resolution |
|---|---:|---|
| Historical doubled repository path did not exist. | 1 | Located the actual clean repository path. |
| `file.batch` ranged read rejected public camelCase fields. | 1 | Preserved as P0 reproduction and stopped using the broken operation for discovery. |
| `file.search` contextLines 8 exceeded the public maximum 5. | 1 | Reissued with 5; no state change. |
| `file.edit` returned false diff/changedLines while planning files were correctly written. | repeated | Verified file revisions/content independently and planned the shared diff repair. |
| A direct Python planning-write command entered confirmation and was cancelled. | 1 | Reissued the planning-only edit through the already-allowed bounded shell path; no partial write occurred. |
| A consistency script expected a duplicate line caused only by overlapping display ranges. | 1 | The script aborted before writing; re-ran against the actual single line and completed successfully. |
| The first rmcp regression test accessed `.code` directly on `ServiceError`. | 1 | Matched `ServiceError::McpError` and asserted the wrapped MCP error code. |
| The ranged-read test expected an `endLine` response field that the stable read contract does not expose. | 1 | Asserted `returnedThroughLine` together with content and `startLine`; production behavior was already correct. |
| Initial Phase B source injection produced two escaped character-literal syntax errors. | 2 | Corrected the generated Rust literals; no semantic code was executed before successful compilation. |
| A Phase B test-injection script matched a pre-rustfmt code block and aborted before writing tests. | 1 | Re-read the formatted source, applied smaller exact edits, and verified no partial test insertion occurred. |
| An MCP validation test used the literal characters `\\0` instead of an actual NUL. | 1 | Corrected the test input to a real NUL; production validation was unchanged. |
| Two Phase C exact-edit scripts aborted on rustfmt-adjusted anchors. | 2 | Inspected the actual source, confirmed only earlier completed edits had landed, and applied the remaining changes in smaller idempotent steps. |
| Two manual local E2E scripts used outdated `agent.info` field paths. | 2 | The rmcp calls/socket were successful and cleanup traps ran; corrected the parser to `identity` and completed the E2E. |
| The first empty-runtime-directory cleanup test wrote into a parent directory that the new guard had correctly removed. | 1 | Recreated the test directory before the regular-file safety case; production cleanup behavior remained correct. |
| Laptop Agentic returned `Session terminated` during Phase E. | 1 | User restarted the Laptop Agentic worker; verified `ea19b42`, the full uncommitted migration, and planning state were intact before continuing. |
| The first boot-generation Hello test hung. | 1 | Found and fixed a real reconnect deadlock: Hello held the Agent registry mutex while pending replay reacquired it. |
| Final OpenAPI diff check found one trailing blank line. | 1 | Normalized the YAML to one final newline and reran parser/reference/tests. |
| The old protocol Hello test expected missing `bootGeneration` compatibility. | 1 | Updated the breaking contract test: connection mode may default, but boot generation is required and missing values are rejected. |
| A focused `cargo test` command supplied two filters. | 1 | Used a shared `cancel` filter; no code issue was involved. |
| Immediate cancellation returned `cancelled_before_start` instead of the process-kill outcome expected by the test. | 1 | Preserved the truthful branch and made the process-kill test wait until `running`; added terminal cancellation coverage. |
| The first managed MCP test command appeared to hang. | 1 | It was compiling and then exposed stale `JobDetail` test field access; no fake downstream test had been partially written. Migrated the assertions and continued in smaller test increments. |
| Fake downstream cancellation returned transport closure rather than a terminal cancellation response. | 2 | Verified rmcp exact-id cancellation cancels the request context/task; froze truthful `detached` semantics when no downstream terminal proof is observable. |
| Strict OpenAPI `JobResponse` initially used `allOf` with `JobDetail.additionalProperties=false`. | 1 | Replaced it with one explicit strict object schema so Actions validators do not reject envelope fields. |
| Full regression found one stale confirmation-preview assertion after redaction hardening. | 1 | Updated the assertion to the bounded `showing N of M` key-summary format; production behavior was correct. |
| Static review found raw MCP argument values in confirmation previews. | 1 | Replaced them with bounded key subset/count, serialized bytes, and SHA-256; added Hub-confirmation redaction and pending-sender cleanup tests. |
| A Phase G broad regex treated a function parameter as a `JobInfo` field and temporarily corrupted one signature. | 1 | Compiler localized the change; restored the signature and switched to exact `JobInfo` initializer edits before any tests ran. |
| One Agentic `process.batch` verification call placed `waitSeconds` inside a child element. | 1 | The wrapper rejected the malformed request before execution; reissued the intended single command and made no repository change. |
| The aggregate clipping test used a child result large enough to hit the 512 KiB child limit first. | 1 | Reduced the fake payload so each child stays retained while the combined response exceeds 2 MiB, proving the intended second-level budget. |
| Child cancellation during aggregate confirmation preserved direct local-cancel evidence instead of the test’s aggregate evidence. | 1 | Kept the truthful terminal evidence and asserted direct-cancel and aggregate-closure children separately. |
| Capacity audit initially stored the full semicolon-delimited rejection detail as `errorCode`. | 1 | Normalized both `:` and `;` delimiters so audit keeps stable `max_active_jobs_reached` while the public error retains details. |
| Wide static `sed` ranges visually duplicated several Phase G lines. | 1 | Verified exact numbered ranges and multiline duplicate patterns; source contained no duplicate statements. |
| Phase H temporary config-init smoke used the wrong CLI argument order. | 1 | The command exited before writing; used the checked-in strict config type and dedicated example test instead. |
| Two Phase H Agentic batch-wrapper calls placed `waitSeconds` inside a child operation. | 2 | The wrapper rejected both before execution; switched the remaining acceptance commands to single managed process calls. |
| Hub descriptor test found `mcp.batch.waitSeconds.maximum` missing from generated rmcp schema. | 1 | Added the exact schemars range to the batch DTO and locked 0..30 plus 1..900 timeout bounds in the descriptor test. |
| Operations checklist insertion targeted `Security invariants` instead of the actual `Safety invariants` heading. | 1 | Used the exact heading; only the documentation insertion had been skipped. |
| First deny-warnings clippy run exposed historical and new mechanical lints. | 1 | Fixed all behavior-preserving lints, boxed the large run-report variant without wire changes, and used narrow function-local allowances for intentional orchestration/API shapes. |
| The first version acceptance run showed both CLIs rejected `--version`. | 1 | Enabled Clap crate-version metadata, added parser regressions, and verified both binaries print `0.9.0`. |
| Initial version parser tests used `unwrap_err`, which required `Cli: Debug`. | 1 | Matched the parse result explicitly instead of adding an unrelated Debug requirement. |
| Live connectors reported v0.8.0 and the old 23/35 surfaces. | 1 | Recorded this as the expected undeployed baseline; all connectors were healthy and connected, and no deployment/restart was performed. |

## Current State
- Implementation Phases A–H are complete; Phase H product commit is `7069c97` and planning artifacts remain uncommitted.
- Open blocking decisions: none.
- Laptop, Tablet, Server, and Room remain on the healthy undeployed v0.8.0 baseline; coordinated config migration and deployment are intentionally outside this implementation task.
- No push, deployment, user-config migration, restart, tag, or release has been performed.

## 2026-07-27 — Phase I complete

Implemented the v0.9 pre-release tunnel stdio restart repair:

- Added `ResumableStdioTransport` around the tunnel stdio transport.
- Stale pre-initialize notifications are ignored with bounded metadata-only diagnostics.
- A resumed non-ping request triggers a private MCP initialize; its response is suppressed and the original request is replayed unchanged.
- Added `mcp_stdio_session_resume` / `mcp_stdio_session_resumed` diagnostics without arguments or results.
- Added duplex regression coverage and a real `stdio-worker` process E2E proving first-call recovery, private-id non-leakage, follow-up-call liveness, and absence of `expect initialized request`.
- Updated standalone runtime, operations, v0.9 release notes, and Chinese migration acceptance criteria.

Verification:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
  - Agent unit tests: 231/231
  - Local Unix E2E: 1/1
  - Standalone supervisor/worker E2E: 6/6
  - Hub: 58/58
  - Protocol: 12/12
- `git diff --check`

No push, deployment, tag, release, configuration migration, or device restart was performed.
