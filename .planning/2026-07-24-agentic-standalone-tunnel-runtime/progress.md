# Progress Log: Agentic Standalone Tunnel Runtime

## Session: 2026-07-24

### Current Status
- **Phase:** 2 - Contract Refinement and Handoff Freeze (complete)
- **Workflow stage:** implementation_ready
- **Role:** designer
- **Implementation authorized:** yes

### Actions Taken
- Verified the official Secure MCP Tunnel path using tunnel-client's embedded MCP stub and a real ChatGPT tool call.
- Discussed coexistence of direct Tunnel command routing, existing centralized Hub routing, optional Hub aggregation/reporting, and future KMP needs.
- Agreed on a public `run-as-standalone` entry that internally supervises official tunnel-client.
- Agreed that tunnel-client should launch an internal Agentic stdio MCP worker.
- Agreed that tunnel id, API-key reference, cache location, and download URL belong in Agentic configuration.
- Agreed that Standalone should expose skills and bootstrap while diary and notebook remain Room-only.
- Inspected current CLI, `RunMode`, local state construction, unconditional Hub connection, Room gating, Hub MCP tool surface, agent registration, run storage, and notifications.
- Read the active `planning-with-files` and `refine-implementation-plan` skills and confirmed their required sequence.
- Initialized `.planning/2026-07-24-agentic-standalone-tunnel-runtime/` and selected it through `.planning/.active_plan`.
- Replaced the generic templates with a repository-grounded initial plan, findings record, decisions, scope, and candidate phases.

### Test / Validation Results

| Validation | Expected | Actual | Status |
|---|---|---|---|
| Tunnel stub connector discovery | Connector tools visible in ChatGPT | `server_info`, `echo`, `uppercase` visible | pass |
| Tunnel stub tool call | Response traverses full tunnel path | `server_info` returned stub metadata | pass |
| Planning skills available | Both base and refinement skills active | Both active and readable | pass |
| New scoped plan initialization | Three files plus active-plan pointer | Created successfully on laptop | pass |

### Errors

| Error | Resolution |
|---|---|
| `skills.run` returned `invalid_working_directory` for the laptop repository. | The Room skill runner is constrained to its workspace. Used laptop `process.exec` to invoke the same installed `planning-with-files` initializer. |
| A parallel official-source inspection read before the sibling clone completed. | Re-ran the inspection serially after clone completion. |

### Next Step
- Begin Phase 3 in a later invocation using `planning-with-files` without `refine-implementation-plan`.

### Implementation session: Phase 3 started

- Re-read the active plan, findings, and progress after the design-freeze commit; worktree is clean and Phase 1–2 remain complete.
- Confirmed the implementation boundary: `RunMode` is currently overloaded across role, policy, and Room capability gates; `room.skills` is the only current skill configuration location; Hub dispatch owns nearly all local operation result conversion.
- Recorded the implementation assumptions in `task_plan.md` and `findings.md`: keep `RunMode` as a compatibility adapter, make top-level `skills` canonical with legacy deserialization, add bootstrap aliases additively, and defer binary/supervisor/reporting behavior to later phases.
- Next action: implement the runtime model and config contract before extracting the shared dispatcher.

### Phase 3 validation finding

- Focused Agent and protocol suites passed (`104` and `8` tests), and formatting passed before workspace validation.
- Workspace validation found four expected additive protocol integration points in `agentic-gpt-hub`: one `SafeConfigSummary` constructor and three exhaustive `HubCommand` matches. These are recorded as a Phase 3 error and will be fixed before the phase commit.


### Refinement round 1: runtime foundations

- Evidence inspected: complete active planning files; config/default/migration behavior; stderr logging; instance locking; Hub WebSocket/SSE lifecycle; local audit and Hub transport ledger; Agentic release targets.
- Questions asked: none yet; repository facts were researched first.
- Decisions confirmed: none beyond existing U-01 through U-09.
- Plan sections updated: workflow stage, Phase 1 discovery checklist, repository findings.
- Maturity transition: `exploring` → `refining`.
- Remaining blockers: 13 candidate decision areas, pending further repository and official tunnel-client inspection.


### Refinement round 2: official client and stdio feasibility

- Evidence inspected: official v0.0.10 release assets/checksums and configuration contract; rmcp 1.7.0 server/stdio features; local Hub-command dispatch and policy coupling.
- Questions asked: none yet; narrowed researchable facts before asking user-owned choices.
- Decisions confirmed: official binary + stdio direction remains feasible; no new product decision inferred.
- Plan sections updated: findings for release trust, secret references, health/log behavior, rmcp server support, and dispatch refactor constraints.
- Maturity transition: remains `refining`.
- Remaining blockers: scope of Hub reporting/coordinator work, public bootstrap naming, platform matrix, secret acceptance, binary trust/update policy, and supervisor failure semantics.


### Refinement round 3: stdio child lifecycle and decision map

- Evidence inspected: official v0.0.10 command parser, stdio child transport, signal forwarding, worker-exit shutdown, Agentic config reload, confirmation fallback, and shared-skill config coupling.
- Questions prepared: Q-01 through Q-06.
- Decisions confirmed: none in this round.
- Plan sections updated: Key Questions and official lifecycle findings.
- Maturity transition: remains `refining`; first user decision batch ready.
- Remaining blockers: Q-01 through Q-06, then supervisor lifecycle and acceptance details.


### Refinement round 4: first user decisions

- Evidence inspected: prior decision batch and user clarification that both Normal and Room Agents should support Tunnel transport.
- Questions resolved: Q-01, Q-03, Q-04, Q-05, Q-06.
- Decisions confirmed: D-01 through D-05 and U-10.
- Plan sections updated: workflow blockers, scope, capability/transport model, Key Questions, Decisions, and findings rationale.
- Maturity transition: remains `refining`.
- Remaining blockers: Q-02 workload decision, public command naming for Room-over-Tunnel, and supervisor lifecycle/defaults.


### Refinement round 5: coordinator profile estimate

- Evidence inspected: Hub MCP router/Apps dispatch, all 43 tool registrations, Hub CLI/config, existing agent/run/session/notification read surfaces.
- Questions reconsidered: Q-02.
- Decisions confirmed: none; recommendation changed from defer to a bounded coordinator profile.
- Plan sections updated: Q-02 recommendation, Phase 9, and detailed workload estimate.
- Maturity transition: remains `refining`.
- Remaining blockers: user confirmation of Q-02B, public Normal/Room Tunnel command naming, and lifecycle/default decisions.


### Refinement round 6: coordinator, profile naming, and lifecycle

- Evidence inspected: Q-02 workload estimate and official stdio child lifecycle.
- Questions resolved: Q-02 and Q-07 through Q-10.
- Decisions confirmed: D-06 through D-10.
- Plan sections updated: Key Questions, Decisions, workflow blockers, Phase 9, and lifecycle rationale.
- Maturity transition: remains `refining`.
- Remaining blockers: final configuration/defaults, platform support, health/log exposure, and acceptance criteria.


### Refinement round 7: reporting persistence and final contract gaps

- Evidence inspected: Hub run schema/TTL, Agent message handling, session lifecycle/retention, config migration patterns, CLI shape, dependencies, and supported release platforms.
- Questions prepared: Q-11 through Q-13.
- Decisions confirmed from repository evidence: Linux amd64/arm64 V1 boundary; canonical top-level skills config with legacy fallback; direct argv launch and private health runtime files.
- Plan sections updated: draft public configuration contract, Key Questions, and final repository findings.
- Maturity transition: remains `refining`.
- Remaining blockers: Q-11 through Q-13 only, followed by acceptance/handoff readiness review.


### Refinement round 8: reporting privacy/defaults and handoff freeze

- Evidence inspected: complete active planning files, handoff-readiness checklist, official doctor JSON/exit semantics, and final repository status.
- Questions resolved: Q-11, Q-12, Q-13.
- Decisions confirmed: D-11, D-12, D-13.
- Clarification frozen: default restart policy means five restart attempts after the initial launch (1/2/4/8/16 seconds); 30 seconds is only the cap for a larger configured budget.
- Plan rebuilt: frozen runtime/config/tool/reporting/trust/lifecycle contracts; detailed Phases 3–9; cross-phase acceptance; implementation discretion; readiness gate; canonical handoff block.
- Maturity transition: `refining` → `implementation_ready`.
- Entry phase: Phase 3 - Runtime model, configuration, and shared local tool service.
- Open blockers: none.
- Product changes during refinement: none.
- Design checkpoint commit: recommended, not created because authorization has not been given.

### Phase 3 complete

- Added `RuntimeModel` with independent `Transport`, `CapabilityProfile`, and `HubMode`; preserved `RunMode` only as a public-entry/test compatibility converter and preserved serialized `AgentRole` values.
- Added canonical top-level `skills`, legacy `room.skills` fallback with top-level precedence warning, flattened unknown-field preservation, optional Tunnel configuration, safe tunnel summaries, strict API-key reference validation, reporting detail defaults, and `config set` keys.
- Added transport-neutral `bootstrap`/`bootstrap.read` protocol variants while retaining Hub `room.bootstrap*` aliases.
- Added `local_service::dispatch` as the value-returning operation layer and changed the active Hub handler to envelope/session-update/transport-response adaptation; direct-vs-Hub capability parity is tested.
- Updated Hub-side protocol matches and safe-summary construction after workspace validation found the additive integration points.

### Phase 3 verification evidence

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo test -p agentic-gpt` | 104 passed |
| `cargo test -p agentic-gpt-protocol` | 8 passed |
| `cargo test --workspace` | Agent 104 + Hub 56 + Protocol 8 passed |

### Next step

- Phase 4: add the hidden capability-aware rmcp stdio worker over the shared local dispatcher; no Tunnel child/supervisor is started until later phases.

### Phase 4 started

- Confirmed rmcp 1.7 supports a server over `transport::stdio()` with `ServerHandler`; the worker can use a dynamic descriptor table while dispatching through the shared `local_service`.
- Recorded the schema/identity and hidden-worker assumptions in `findings.md` before product edits.
- Phase 4 implementation is now in progress; no Phase 4 product commit has been made yet.

### Phase 4 complete

- Enabled rmcp server/macros/`transport-io` features and added the hidden `stdio-worker --config ... --profile normal|room` entry. The worker loads one config, does not acquire the runtime lock, recovers skill-install records, and serves native rmcp stdio with logs remaining on stderr.
- Added a capability-filtered descriptor/dispatch adapter over `local_service`: Tunnel Normal exposes the frozen 29 tools; Tunnel Room adds the 10 diary/notebook tools. `user.notify.deliver` and all other Hub-only tools stay absent. Existing overlapping Hub argument shapes, annotations, bounded limits, session envelopes, and structured error values are preserved.
- Added protocol-level tests for exact tool sets, absent Room-only direct calls, in-process initialize/list/call over duplex stdio, skills/bootstrap, and Room diary/notebook dispatch.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo test -p agentic-gpt --bin agentic-gpt` | 109 passed |
| `cargo test --workspace` | Agent 109 + Hub 56 + Protocol 8 passed |

### Phase 4 implementation errors and resolutions

| Error | Resolution |
|---|---|
| rmcp `CallToolResult` is non-exhaustive and cannot be constructed with a struct literal. | Used rmcp's `structured`/`structured_error` constructors so structured content and `isError` remain protocol-native. |
| The first stdio session-list test observed the raw dispatcher vector instead of the existing MCP `{sessions: [...]}` envelope. | Kept the dispatcher transport-neutral and restored the established envelope in the stdio adapter, alongside `session.start` and not-found normalization. |

### Next step

- Phase 5: implement trusted Tunnel-client distribution and verification; no download or supervisor lifecycle is part of the Phase 4 commit.

### Phase 5 started

- Re-read the frozen Phase 5 contract and confirmed the repository has no existing distribution module or ZIP dependency.
- Recorded the Rust ZIP-reader and atomic cache-install assumptions in `findings.md` before implementation.
- Official v0.0.10 release checksum lookup remains a required research input before finalizing the embedded manifest.

### Phase 5 research checkpoint

- Confirmed the official v0.0.10 Linux asset digests from GitHub release metadata and independently checked the amd64 ZIP. The archive has one regular root entry, `tunnel-client`; no wrapper directory is needed for the built-in extraction path.

### Phase 5 implementation constraint checkpoint

- Recorded bounded archive, extraction, and entry limits before implementing the downloader and ZIP extractor.

### Phase 5 complete

- Added `tunnel_distribution.rs` and the ZIP dependency with the pinned Linux manifest, trusted override/cache resolution, HTTPS-only bounded downloads, async artifact locks, safe extraction, staged atomic cache replacement, executable permissions, cleanup, and deterministic redacted errors.
- Added ten focused tests covering manifest/platform/checksum fixtures, HTTPS policy, executable overrides, archive traversal/symlink/duplicate/layout rejection, archive hash mismatch, cache repair/replacement, offline and `autoDownload=false` behavior, concurrent locks, redirects, response truncation, and size bounds.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo check -p agentic-gpt` | pass; existing unused-code warnings remain until Phase 6 wires the resolver into the supervisor |
| `cargo test -p agentic-gpt tunnel_distribution::tests` | 10 passed |

### Phase 5 implementation errors and resolutions

| Error | Resolution |
|---|---|
| The sandbox denied loopback bind for the local HTTP test server. | Requested the narrow test command with escalated local-network permission; production code still rejects HTTP. |
| The ZIP writer test helper masks high POSIX file-type bits and could not emit a symlink entry through `unix_permissions`. | Patched the test fixture's central-directory metadata to represent a Unix symlink, then verified the extractor rejects it. |

### Next step

- Phase 6: wire the resolver into the standalone supervisor and own the tunnel-client/stdio-worker lifecycle; this phase will eliminate the current distribution-module dead-code warnings.
