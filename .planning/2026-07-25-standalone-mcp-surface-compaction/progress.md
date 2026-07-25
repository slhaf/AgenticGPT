# Progress: Agentic Standalone MCP Surface Compaction

## 2026-07-25 — Planning and Refinement

### Repository discovery
- Located AgenticGPT at `/home/slhaf/Projects/AgenticGPT` after correcting the initial `/home/slhaf/Documents/Projects` assumption.
- Confirmed clean `main` at `3dd8fa2` / `v0.7.0`, equal to `origin/main`.
- Read repository guidelines, previous standalone runtime plan/progress, current tool registry, dispatch, process/session, skills, tmux, protocol, capabilities, Hub reporting, and Hub MCP compatibility surfaces.

### Runtime findings
- Confirmed `agentId` on Tunnel inputs is validation-only and cannot route.
- Confirmed `skills.run` already implements the desired default-5/max-30 inline-wait followed by managed `sessionId` behavior.
- Found that ordinary managed sessions lack the skill-only terminal audit context, so process fusion requires an internal lifecycle generalization first.
- Found Tunnel stdio lacks local call lifecycle logs and long-process terminal logs.
- Found Hub reporting lacks explicit successful connected and normal-disconnected logs.

### Schema measurement
- Launched the real hidden stdio worker with a temporary reporting-disabled config.
- Sent MCP initialize and tools/list for Normal and Room.
- Measured baseline descriptor and input-schema bytes.
- Built a pure projected descriptor model for the frozen compact surface.
- Removed temporary probe files.

| Profile | Current tools/bytes/input | Target tools/projected bytes/input |
|---|---|---|
| Normal | 29 / 15,932 / 8,345 | 18 / 10,775 / 5,843 |
| Room | 39 / 21,664 / 11,380 | 30 / 17,255 / 9,104 |

### Contract freeze
- Frozen Tunnel-only public compaction; Hub full/coordinator remain compatibility surfaces.
- Frozen managed `process.exec/get/kill/list`, retained `process.batchExec`, and public `sessionId`.
- Frozen compact MCP, skills, tmux, and Room-only bootstrap surfaces.
- Frozen no-input-agentId/no-confirmMethod behavior, local lifecycle logs, reporting connection logs, and schema budgets.
- Excluded hot reload, RTT diagnostics, fleet routing, Server network diagnosis, and KMP.

### Handoff readiness
- Goal, scope, non-goals, public names, inputs, outputs, lifecycle, limits, failure behavior, compatibility, security, audit, logging, tests, verification, schema budgets, and commits are explicit.
- Repository evidence supports each implementation phase.
- No blocking decision remains.
- Product code was not modified during planning.

### Next action
Start a later implementation request from Phase 3. Each phase must update all three planning files, run focused verification, report the diff/results, and create its specified focused Git commit. Do not redesign the Hub full/coordinator surfaces or add generic RPC compatibility aliases.

### Final handoff audit
- Exact tool-count audit: Normal 18, Room 30.
- Frozen decision audit: D-01 through D-15 present and continuous.
- Phase audit: Phases 1–2 complete; Phases 3–7 pending; entry phase is Phase 3.
- No TODO, TBD, unresolved question, pending recommendation, or implementation-authorization conflict found.
- Planning files have one final newline, no CR/trailing whitespace, and `git diff --check` passes.
- Temporary schema probes are absent.
- Git status contains only `.planning/.active_plan` and the new scoped planning directory; no product code changed.
- Handoff result: `implementation_ready`.

### Formal refinement re-entry
- Evidence inspected: active plan pointer, all three planning files, Git status/log, and both active skill contracts plus refinement references.
- Workflow transition: `implementation_ready` → `refining`; role `planner` → `designer`; implementation authorization `yes` → `no`.
- Entry phase remains candidate Phase 3 pending the formal gap and handoff-readiness audits.
- Product files changed: none.

### Formal refinement round 1: lifecycle and decoder ownership
- Evidence inspected: `sessions.rs`, stdio call/dispatch/schema generation, and bounded batch execution.
- Contract gaps found: idempotent terminal ownership, immediate-failure retention, strict unknown-field rejection, Tunnel-vs-Hub dispatch separation, and unrelated batch-cleanup scope.
- Decisions confirmed: none yet; repository facts recorded in `findings.md`.
- Maturity remains `refining`; implementation remains unauthorized.

### Formal refinement round 2: reporting and merged response contracts
- Evidence inspected: reporting run/session emitters, reporting-only WebSocket/SSE loops, skill list/search/active implementations and protocol types, stdio unit tests, and supervised worker smoke.
- Research conclusions: tool-call reports end when the MCP call returns; asynchronous terminal state uses one later SessionUpdate; existing stderr logger remains authoritative; merged skills output needs both `skills` and `activeSkills`.
- User-owned choices remaining: immediate failed-process retention, exact merged skills filtering semantics, and whether unrelated batch timeout cleanup belongs in this plan.
- Maturity remains `refining`; implementation remains unauthorized.

### Formal refinement round 3: partial user decisions
- Questions resolved: Q-01A and Q-02A.
- Decisions updated: D-05 and D-09.
- Plan sections updated: managed process behavior, merged skills response contract, key questions, and findings rationale.
- Remaining blocker: Q-03 only.
- Maturity remains `refining`; implementation remains unauthorized.

### Formal refinement round 4: managed batch decision
- User resolved Q-03 by including Tunnel `process.batchExec` in the managed-process lifecycle.
- D-04 was marked superseded; D-16 records the replacement contract.
- Phase 4 and acceptance criteria now require an ordered multi-session batch launcher with one shared inline wait deadline and per-session follow-up.
- Remaining blocker: Q-04 batch admission atomicity/capacity behavior.
- Maturity remains `refining`; implementation remains unauthorized.

### Formal refinement round 5: batch admission and handoff freeze
- User resolved Q-04A: all-or-reject batch admission.
- Decision confirmed: D-17.
- Phase impacts: Phase 3 now establishes the shared lifecycle/finalizer; Phase 4 performs aggregate preflight/capacity/confirmation before allocating all batch session ids.
- Readiness transition: `refining` → `implementation_ready`; implementation authorization `no` → `yes` for a later request.
- Entry phase: Phase 3.
- Open blocking decisions: none.
- Product files changed during formal refinement: none.

### Formal final handoff audit
- Exact tool-set audit passed: Normal 18, Room 30.
- Decision audit passed: D-01 through D-17 are continuous; D-04 is explicitly superseded by D-16.
- Question audit passed: Q-01 through Q-04 are all resolved.
- Workflow audit passed: `implementation_ready`, designer handoff complete, entry Phase 3, no open blockers.
- File-boundary audit passed: only the active plan's `task_plan.md`, `findings.md`, and `progress.md` changed; no product code, tests, config, generated files, or active-plan pointer changed.
- Formatting audit passed: one final newline, no CR/trailing whitespace, and `git diff --check` exit 0.
- Formal refine result: ready for a later `$planning-with-files` implementation request without `$refine-implementation-plan`.

### Formal readiness recheck: handoff reopened
- Reapplied the `handoff-readiness.md` gate line by line against all three planning files and current repository state.
- Found and corrected two researchable contradictions: single-process allocation order and Phase 3/Phase 6 terminal-log ownership.
- Strengthened D-17 with atomic aggregate capacity reservation/insertion and concurrent admission verification.
- New blocking question: Q-05, exact public response envelope for managed `process.batchExec`.
- Workflow transition: `implementation_ready` → `refining`; implementation authorization `yes` → `no`.
- Product files changed: none.

### Formal refinement round 6: managed batch response
- User resolved Q-05 with the recommended envelope, excluding public wrapper `agentId`.
- Decision confirmed: D-18.
- Updated the exact outer status values and per-element `managed|rejected|skipped` structure.
- Continued readiness audit now checks every other merged-tool response surface before restoring implementation authorization.

### Formal refinement round 7: remaining public response audit
- Inspected existing MCP, tmux, session, skill activation, and batch result implementations.
- No further user-owned output choice was required; D-19 freezes compatibility-preserving response bodies for all remaining merged tools.
- Removed the contradictory top-level `agentId` from the shared Tunnel process/skill wrapper while preserving nested `SessionInfo` metadata and Hub protocol compatibility.
- Preserved current empty-batch behavior and documented aggregate session capacity as the effective non-empty batch bound.
- Open blocking decisions are now none; final readiness gate still pending.

### Formal refinement round 8: post-admission failure gap
- Structural response audit is complete; no additional public envelope requires user selection.
- Readiness review found one remaining non-structural blocker: Q-06, sibling behavior when a post-admission OS spawn fails.
- Documented that all-or-reject admission is not transactional command execution and that process side effects cannot be rolled back.
- Maturity remains `refining`; implementation remains unauthorized.

### Formal refinement round 9: post-admission failure resolution
- User resolved Q-06A: successfully spawned siblings continue independently when another element fails to spawn.
- Decision confirmed: D-20.
- Workflow transition: `refining` → `implementation_ready`; implementation authorization `no` → `yes` for a later request.
- Entry phase: Phase 3.
- Open blocking decisions: none.
- Product files changed: none.

### Final state-field reconciliation
- The final gate caught stale textual workflow markers left from the readiness recheck; no contract decision was missing.
- Reconciled both Workflow State and Implementation Handoff to `implementation_ready`, Phase 3 entry, D-01 through D-20, and no blockers.
- Expanded Phase 3 prerequisites through D-20 and made Phase 4 atomic insertion plus post-admission sibling-spawn behavior explicit in acceptance tests.

### Final refine handoff gate — passed
- Workflow state and handoff state are both `implementation_ready`; entry phase is Phase 3 and no blocker remains.
- Exact surface audit passed: Normal 18 tools, Room 30 tools.
- Question audit passed: Q-01 through Q-06 resolved.
- Decision audit passed: D-01 through D-20 continuous, with D-04 explicitly superseded by D-16.
- Public contract audit passed for process wrappers, managed batch, MCP list modes, skills merge, tmux action tools, strict legacy-field rejection, and Room-only bootstrap.
- Lifecycle/failure audit passed for atomic admission, retained post-allocation failures, exactly-once finalization, independent sibling continuation after spawn failure, in-memory restart semantics, Hub compatibility, audit/report/log ownership, and bounded output.
- Phase audit passed: Phases 1–2 complete; Phases 3–7 pending with explicit prerequisites, implementation seams, acceptance tests, verification, and commit boundaries.
- File-boundary and hygiene audit passed: only the three active planning files changed; `git diff --check` exit 0; no product code changed.
- Result: ready for a planning-only checkpoint and handoff to Luna using `$planning-with-files` from Phase 3.

### Planning checkpoint
- Formal refine checkpoint created: `7f2fc27` (`docs(planning): refine standalone MCP handoff`).
- This checkpoint contains only the three active planning files and is the clean implementation baseline for Luna.

## 2026-07-25 — Phase 3 implementation

### Lifecycle generalization
- Replaced the skill-only audit context with a generic managed audit context carrying request source, confirmation/policy metadata, optional skill provenance, installed digest, and one-shot terminal hook.
- Added shared managed start and bounded-wait entrypoints; `skills.run` now uses the shared wait helper.
- Registered asynchronous sessions and checked active capacity in one sessions-map critical section before spawning.
- Retained capacity, preflight, policy, confirmation, cancellation, and spawn failures as terminal managed sessions after registration.
- Added one terminal finalizer that writes audit, emits a best-effort reporting session update, releases skill leases, and invokes the terminal hook exactly once.
- Added regression tests for ordinary managed audit exactly once and concurrent active-capacity admission.

### Verification notes
- `cargo check -p agentic-gpt --bin agentic-gpt` passed after fixing a Rust borrow-lifetime error in terminal tail collection.
- First focused test command was invalid because Cargo accepts one test filter per invocation; this is recorded as a command error and will not be repeated. The replacement is the single `sessions::tests` filter.
- Focused `cargo test -p agentic-gpt --bin agentic-gpt sessions::tests` passed: 7 tests.
- Full `cargo test -p agentic-gpt --bin agentic-gpt` passed: 135 tests.
- `cargo fmt --all` completed and the Phase 3 diff has no formatting or whitespace errors.
- Hub compatibility review identified and preserved the old synchronous `session.start` preflight/rejection behavior; only the new Tunnel/skills managed entrypoint defers those checks after registration.

### Phase 4 start
- Phase 3 committed as `eb76367` (`refactor(agent): unify managed process lifecycle`).
- Phase 4 is now in progress. The implementation boundary is the Tunnel stdio adapter and the new managed entrypoint; Hub full/coordinator dispatch and legacy protocol variants remain untouched.
- Initial stdio test run exposed only stale 29/39 assertions and Normal calls to removed `session.list`/`bootstrap`; tests were updated to the frozen 18/30 surface and `process.list`.

### Phase 4 verification
- Added strict Tunnel process argument structs and direct managed dispatch for single-process lifecycle operations.
- Added managed batch admission/launch path with all-or-reject validation, configured confirmation, atomic capacity reservation/insertion, shared deadline, and independent post-admission sibling behavior.
- `cargo test -p agentic-gpt --bin agentic-gpt stdio_server::tests` passed: 7 tests.
- `cargo test -p agentic-gpt --bin agentic-gpt` passed: 137 tests, including Hub compatibility and all existing Agent tests.
- Added tests for exact 18/30 counts, Normal bootstrap absence, removed alias rejection, strict `agentId`/`confirmMethod` rejection, quick/long/get/kill/list, managed batch success, and batch admission rejection.
- Initial `cargo test --workspace` passed the 138 Agent tests but the standalone supervisor fixture failed before worker startup with `runtime_directory_unavailable` under the existing HOME; an isolated HOME confirmed the runtime code and then exposed the fixture's stale `agentId`.
- Updated `tests/standalone_supervisor.rs` to send the frozen Tunnel process shape without `agentId`; isolated `standalone_supervisor` now passes.

### Phase 5 start
- Phase 4 committed as `75909ee` (`feat(agent): compact standalone process tools`).
- Phase 5 now owns the remaining advertised compact names: `mcp.list`, merged skills list/activation, merged tmux actions, and Room-only bootstrap descriptors/dispatch. Hub full/coordinator compatibility remains out of scope.

### Phase 5 verification
- Added `mcp.list(serverId?)`, strict `mcp.callTool`, merged `skills.list`/`skills.setActive`, strict preserved skill adapters, merged `tmux.sessions`/`tmux.panes`, and strict tmux exec/paste adapters.
- Moved bootstrap descriptors and strict bootstrap dispatch to Room only; Normal remains method-not-found for bootstrap.
- `cargo test --workspace` with isolated HOME/runtime state passed: Agent 138 tests, standalone supervisor 1 test, Hub 61 tests, protocol 9 tests, and doc tests.
- Phase 5 acceptance tests cover exact 18/30 names, no Tunnel `agentId`/`confirmMethod` schema fields, merged result envelopes, Room capability presence, and preserved Hub full/coordinator tests.

### Phase 6 start
- Phase 5 committed as `02f025d` (`feat(agent): compact standalone MCP surface`).
- Phase 6 now owns safe stdio lifecycle logs, managed terminal hook rendering, reporting connection transitions, schema byte ceilings, standalone docs, and real hidden-worker Normal/Room smoke.

### Phase 6 verification
- Added stderr-only bounded stdio lifecycle records for every standalone tool call, including run id, tool, profile, status, duration, safe session/exit metadata, and bounded error codes; raw arguments, results, paths, secrets, and process output are not rendered.
- Bound the shared managed terminal hook to Tunnel process and skill starts, including each managed batch element, so asynchronous terminal completion is logged once by the existing finalizer.
- Added explicit successful connected and normal disconnected logs for both Hub reporting WebSocket and SSE transports; retry/failure behavior remains unchanged.
- Added serialized Normal/Room descriptor and summed input-schema ceilings of 11,500/6,200 and 18,000/9,600 bytes, alongside the exact no-`agentId`/no-`confirmMethod` schema assertions.
- Updated `docs/standalone-runtime.md` with the exact 18/30 matrix, strict Tunnel input behavior, managed response/batch semantics, local log privacy, and Normal/Room hidden-worker verification scope.
- Extended the real supervisor smoke to Normal and Room; both initialize, list, and call through the hidden worker successfully under isolated runtime HOME.
- Verification passed: isolated `cargo test -p agentic-gpt` (139 unit tests plus 2 standalone supervisor tests), `cargo fmt --all`, and `git diff --check`. The same supervisor test is known to fail before startup with `runtime_directory_unavailable` under the host HOME; isolated HOME is the supported deterministic gate.
- Phase 6 committed product, test, documentation, and planning changes are pending the final Phase 7 review commit boundary.

### Phase 7 review and delivery
- Reviewed the final diff against D-01 through D-20: Tunnel-only compaction is isolated from Hub full/coordinator routing and compatibility surfaces; no generic RPC or cross-Agent selection was introduced.
- Verified the final advertised sets remain exactly Normal 18 and Room 30, with Room-only bootstrap and strict rejection of Tunnel `agentId`/`confirmMethod` inputs.
- Verified the managed process wrapper, per-element batch sessions, shared waits, retained failures, terminal hook ownership, and direct Tunnel `skills.run` adapter preserve the frozen lifecycle contract.
- Verified documentation has no stale Tunnel `session.*`, old compacted MCP/skills/tmux names, or Normal bootstrap claim; the remaining `session.start`/`hub.session.*` references are explicitly Hub/general path-policy material.
- Final verification passed: `cargo fmt --all -- --check`, `git diff --check`, isolated `cargo check --workspace`, and isolated `cargo test --workspace` (Agent 139, supervisor 2, Hub 61, protocol 9, doc tests 0).
- Phase 6 delivery commit: the focused `docs(agent): document compact standalone tools` commit (amended once for final plan-state reconciliation). Final worktree is clean; no repair commit was needed.
- Phase 7 is complete. The active plan is delivered with no open blockers or remaining implementation work.
