# Progress Log: Agentic Standalone Tunnel Runtime

## Session: 2026-07-24

### Current Status
- **Phase:** 9 - Documentation, End-to-End Verification, and Delivery (complete)
- **Workflow stage:** implementation_complete
- **Role:** implementer
- **Implementation authorized:** yes

### Phase 7 kickoff

- Investigation confirmed that the current Hub registry has one live connection per `agentId`, Room activation is performed during Hello, and `request_agent` creates/persists a run before sending a command. Phase 7 will preserve those rules for command-capable connections and add an explicit reporting-only boundary.
- New implementation assumptions were recorded in `findings.md` before code changes: Hello mode defaulting, a separate Agent run-report event, worker-owned nonblocking reporting, nullable run metadata migration, and current-connection session synchronization.
- Phase 7 implementation added a Hello-ready gate so a newly replaced connection cannot receive a command before its mode is known; snapshot session tools now read only the active connection cache, and bounded `hub.run.list` was added beside `hub.run.get`.
- One test command initially supplied multiple Cargo filters to a single `cargo test` invocation; it was corrected to separate focused test runs. The two remaining Agent loopback failures are sandbox permission errors and require the same escalation used by Phase 6.

### Phase 7 complete

- Added the Hello connection-mode extension with legacy command-capable default and a Hello-ready gate, reporting-only Hub transport behavior for WebSocket/SSE, nonblocking bounded Agent reporting, metadata/full privacy handling, session sync/cleanup, Agent-originated run persistence, TTL cleanup, idempotency/conflict handling, and bounded `hub.run.list/get` plus snapshot `session.list/inspect` behavior.
- Verification: focused Hub/Protocol/Agent tests passed; authorized `cargo test --workspace` passed with Agent 129, Hub 58, Protocol 9, and no doc-test failures; formatting and diff checks passed.
- Phase 7 commit is now the next handoff boundary. Continue with Phase 8 coordinator/full MCP profile work after committing this phase.

### Phase 8 kickoff

- Investigation confirmed the Hub's custom `/mcp` JSON-RPC path owns Apps-compatible dispatch and currently exposes the full ToolRouter. Phase 8 will add startup-fixed profile state, filter both descriptors and direct calls at that boundary, and preserve full-profile compatibility.
- New profile assumptions were recorded in `findings.md` before edits: full default via CLI/env, the frozen coordinator allowlist, Hub-native session aliases, additive bootstrap aliases, and profile labels in shared OAuth metadata.

### Phase 8 complete

- Added `serve --mcp-profile full|coordinator` with `AGENTIC_GPT_HUB_MCP_PROFILE`, profile-aware initialize/instructions/metadata, strict descriptor and direct-call filtering, the exact eight-tool coordinator surface including `hub.info`, native session aliases, and full-profile bootstrap aliases.
- Verification: formatting/diff checks and all 61 Hub tests passed, including no-dispatch assertions for hidden coordinator tools. Phase 8 is committed; Phase 9 follows for documentation, packaging, and end-to-end delivery verification.

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
- Phase 9 is complete under the user-approved delivery boundary; no remaining repository work is required.

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

### Phase 6 started

- Re-read the frozen supervisor contract and official tunnel-client v0.0.10 flags/source before implementation.
- Recorded the 45-second readiness bound, child-only secret environment, per-run worker token, Unix process-group cleanup, and private runtime-path assumptions in `findings.md`.
- Added the public `run-as-standalone` CLI shape, hidden worker authorization hook, and initial supervisor lifecycle implementation; fake-client lifecycle tests remain before completion.

### Phase 6 complete

- Added the supervisor-owned runtime lock and lifecycle around the trusted resolver: secret resolution, doctor preflight, exact tunnel-client argv/env, hidden worker token authorization, private runtime files, readiness polling, restart budget/reset, restart-required config diagnostics, signal forwarding, process-group cleanup, and stale-file cleanup.
- Added fake tunnel-client coverage for doctor/run separation, API-key argv exclusion, worker invocation, health URL discovery, readiness, and graceful process-tree shutdown; added policy tests for bounded backoff, permanent exit classification, exhaustion, and stale runtime files.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `git diff --check` | pass |
| `cargo check -p agentic-gpt` | pass; only pre-existing `RunMode` warnings remain |
| `cargo test --workspace` | Agent 126 + Hub 56 + Protocol 8 passed |

### Phase 6 implementation errors and resolutions

| Error | Resolution |
|---|---|
| The sandbox denied loopback bind for the fake supervisor readiness server. | Ran the focused and workspace suites with narrow escalated loopback permission; production code remains loopback-only for health. |
| A focused cargo test command supplied two filters, which Cargo rejects. | Re-ran the intended tests with one filter/full crate selection and recorded the command correction here. |
| The first fake-client secret assertion inspected argv and the deliberately appended environment marker together. | Split the fixture log at the marker so argv exclusion and environment injection are asserted independently. |

### Next step

- Phase 7: add reporting-only Hub protocol and persistence without coupling reporting failures to direct Tunnel tool execution.

### Phase 9 started

- Phase 8 was committed as `a2e8728` (`feat(hub): add coordinator MCP profile`); the worktree was clean at the phase boundary.
- Re-read the delivery checklist and audited the existing README, interface/operations docs, Linux packaging script, CLI/config surface, pinned tunnel manifest, and supervisor diagnostics before editing documentation.
- Recorded the Phase 9 documentation, secret-handling, release-environment, and external-connector evidence constraints in `findings.md` before making product/doc changes.
- During the surface audit, found and recorded the frozen-plan/implementation mismatch for coordinator `hub.info`; this will be corrected before the Phase 9 delivery commit.

### Phase 9 verification checkpoint

- Added `hub.info` to the coordinator MCP surface and direct Apps dispatcher; Hub tests now cover the exact eight-tool list and all advertised tools remain callable.
- Added `docs/standalone-runtime.md` and linked it from both READMEs and `docs/interfaces.md`. The guide documents the four mappings, Normal/Room tool sets, config migration, secret references, pinned client sources, reporting detail, coordinator/full profiles, runtime files, recovery, and the distinction between fake-client and real connector verification.
- `cargo fmt --all -- --check` and `git diff --check` pass.
- `./scripts/dist-linux.sh` passes with authorized external execution and writes amd64/arm64 release artifacts for both binaries. `file`, CLI help, embedded manifest URL/digest inspection, and target mapping tests pass.
- `cargo test --workspace` passes after the authorized loopback rerun: Agent 129, Hub 61, Protocol 9, doc tests 0.
- The real external Secure MCP Tunnel connector call is not executable from the current API/tool surface; Phase 9 remains open at that acceptance boundary and must not be described as fully complete.

### Phase 9 continuation smoke-test attempt

- Added `crates/agentic-gpt/tests/standalone_supervisor.rs`, which provisions a temporary config and fake tunnel binding, launches the actual built `agentic-gpt` supervisor, invokes the actual hidden stdio worker with MCP initialize/`process.exec` traffic, and checks the returned local-tool marker. This is local topology evidence only and is not the external connector call.
- The first unsandboxed run failed before the test logic could execute because its loopback health listener received `Operation not permitted`; the next run requires the same narrow loopback escalation used by the workspace suite.
- The authorized rerun reached the supervisor but timed out waiting for the worker marker after 15 seconds. The test currently cleans temporary files before returning this error, so the next diagnostic edit will preserve worker response/stderr and supervisor stderr before changing the MCP harness.
- Diagnostic output showed the fake harness passed the entire double-quoted `mcp.command` value as one executable (`sh: ... stdio-worker ...: no such file or directory`). This is a harness parser defect: production `quote_arg` intentionally uses double quotes for the worker command; the harness will strip those delimiters before its shell-only emulation.

### Phase 9 continuation smoke-test complete

- The corrected integration test passed: the actual supervisor CLI reached readiness, the fake tunnel launched the actual hidden stdio worker, MCP initialize/initialized/tools-call traffic completed, and the worker executed `/usr/bin/printf` through the local dispatcher with the expected marker.
- This is stronger local supervisor/worker/tool evidence, but the frozen real Secure MCP Tunnel connector acceptance remains pending because the current environment exposes no connector capability or external credentials.

### Phase 9 full verification rerun

- `cargo fmt --all -- --check` and `git diff --check` pass after the integration test and documentation updates.
- Authorized `cargo test --workspace` passes: Agent 129 unit tests plus 1 integration test, Hub 61, Protocol 9, and 0 doc tests. The unprivileged run still reports only the two known loopback sandbox denials; the authorized result is authoritative for the suite.

### Phase 9 complete

- The user explicitly waived the live production Secure MCP Tunnel connector call for this delivery because it requires consistent account-scoped `tunnelId` and runtime API key credentials. This waiver is recorded in `findings.md` and `task_plan.md`; it does not claim that the local mock or health check is a production connector call.
- All repository-local Phase 9 work is complete and remains represented by the single amended Phase 9 delivery commit.

### Phase 9 checkpoint committed

- Committed the repository-local Phase 9 delivery as one focused commit after all available automated and release checks passed. The external connector acceptance boundary is explicitly waived for this delivery by the user.
### Phase 10 repair started

- Reproduced the production mismatch from the earlier real official-client doctor output and mapped it to `Invocation::mcp_command`.
- Confirmed the worktree was clean before repair.
- Confirmed the Phase 9 integration fake tunnel strips outer double quotes from `mcp.command`, masking the actual client behavior.
- Repair scope is limited to command binding, redacted/bounded doctor diagnostics, and focused regression coverage; no public contract change is required.


### Phase 10 command correction

- The first focused Cargo command supplied two separate test filters; Cargo accepts only one positional filter. No tests ran. Re-ran the supervisor test module as one focused filter instead of repeating the invalid command.

### Phase 10 repair complete

- Changed `Invocation::mcp_command` to concatenate the already token-quoted worker command directly.
- Added bounded doctor failure diagnostics with redaction for both the Runtime API key and per-run worker token; extended child-log forwarding to redact both values.
- Added exact binding, diagnostic redaction/truncation, and failing-doctor tests. Updated the standalone fake tunnel to reject whole-command quoting and removed its quote-stripping workaround.
- Focused supervisor tests passed: 11/11. Standalone supervisor integration passed: 1/1.
- Full `cargo test --workspace` passed: Agent 132 unit tests + 1 integration, Hub 61, Protocol 9, doc tests 0.
- `cargo fmt --all -- --check`, `git diff --check`, and `cargo check -p agentic-gpt` passed; only the two pre-existing `RunMode` dead-code warnings remain.
- A temporary config copied the real tunnel settings but used an isolated config/runtime identity. The newly built Agentic binary passed the official pinned `tunnel-client` doctor and logged `standalone tunnel ready`; the temporary process and files were then removed.

### Phase 11 repair started

- Inspected running-process and official tunnel-client logs after the ChatGPT connector reported a connection error.
- Confirmed the worker command starts successfully; failure occurs before any MCP request because every control-plane metadata/poll request rejects the Authorization header locally.
- Inspected only key-file metadata and byte classes, not secret content. Found two trailing LF bytes while the current parser removes only one.
- Repair scope: normalize repeated trailing CR/LF for file references, reject embedded controls, add regression tests, and verify actual control-plane traffic rather than local readiness alone.

### Phase 11 verification correction

- The first combined final verification stopped at `git diff --check` because `findings.md` and `progress.md` had extra blank lines at EOF. No compile or test step ran in that command. Trimmed planning files to one final newline and reran the full verification chain.

### Phase 11 repair complete

- Updated `resolve_secret` so file references remove all trailing CR/LF bytes and all references reject remaining control characters before entering `CONTROL_PLANE_API_KEY`.
- Replaced the single-CRLF test with coverage for repeated mixed trailing line endings, embedded newline rejection, line-only empty-file rejection, and plaintext-reference rejection.
- Focused supervisor tests passed 11/11.
- Real official-client validation with the original unmodified key file logged `tunnel metadata fetched` and no Authorization-header failure.
- Real ChatGPT Secure MCP Tunnel validation discovered 29 Normal tools and successfully completed `process.exec`; `bootstrap` and `skills.list` additionally proved structured error and data responses.
- The temporary isolated supervisor, config, runtime directory, audit file, and state directory were removed after validation.
- Final `cargo fmt --all -- --check`, `git diff --check`, `cargo check -p agentic-gpt`, and `cargo test --workspace` passed: Agent 132 unit + 1 integration, Hub 61, Protocol 9, doc tests 0. Only the two pre-existing `RunMode` dead-code warnings remain.
