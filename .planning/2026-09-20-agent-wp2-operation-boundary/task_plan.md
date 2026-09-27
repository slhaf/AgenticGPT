# WP2 Agent operation boundary

## Goal
Implement the existing roadmap WP2 ingress/application boundary and safe config reload, using current code seams rather than imposing design patterns. Preserve D01-D08, public transport projections and permission defaults. Keep resource execution/results in Agent.

## Current Phase
Complete — scoped implementation, regression/build checks, real runtime proof, documentation and temporary-resource cleanup are finished.

## Next Step
WP2 is closed. Start a subsequent work package only when selected, first reading its relevant decisions, target, engineering rules, roadmap and current-code evidence.

## Phases
1. Architecture/current-code contract: completed and committed as `c41a551`.
2. Integrated operation/config implementation: completed as `d95efd3`. Shared admission, local/direct ingress, Hub adapter, Skill runner, neutral projections, tmux context and safe reload shared Main's module/CLI/config composition, so their coherent implementation commit is atomic.
3. Verification and evidence: completed with this record. Final Agent tests/builds and real Local/stdio/HTTP/Hub/CLI probes pass; operational/config and architecture/interface docs state the actual contracts and external-service limits.

## Required scope
- Minimum RequestContext + operation admission, not a registry/framework/new crate.
- Every existing executable ingress/direct resource route uses the shared application boundary; ingress auth/framing/error projection remains distinct.
- Discovery metadata/annotations never stand in for authorization; share only evidenced stable metadata.
- Startup-only/live-safe/restart-required config categories align with actual derived resources.
- Describe actual process/MCP/Browser/tmux/tunnel trust boundaries; preserve sandbox default and policy override semantics.
- WP-R remote Room contract completion, WP3 durability overhaul, Console and Browser-host topology redesign remain separate.

- User clarified this is the workflow for every subsequent WP: read relevant whole-repository architecture decisions/direction/dependencies/acceptance first, then verify current code. Main selects implementation, steps, architecture and patterns; docs are direction, not mandatory stale implementation.
## Constraints
- Existing docs/architecture/decisions.md D01-D08 is authoritative product direction; target/engineering/roadmap are implementation guidance, not proof of current code or a compulsory directory layout.
- Prior WP1 and hub-control-boundary are complete; no reimplementation or Plan Review UI.
- Main owns decomposition, shared interfaces, planning files, integration, validation and per-phase commits. Workers skip validation during concurrent edits.
- Use CodeGraph to locate/trace code before text search; LSP references before exported-symbol mutation.
- No extra auth identities or context fields without actual producer/consumer. No duplicate operation executor, adapter-to-adapter DTO calls, compatibility shim or new wire state.
- If evidence reveals a real permission/behavior choice not resolved by D01-D08, enumerate it before changing defaults.

## Verification status
Baseline red: two concrete WP2 regressions failed (`artifact://109`). Final integrated `cargo test -p agentic-gpt`: 526 passed, one ignored; Agent/Hub builds and fmt check passed (`artifact://147`), with five Browser distribution dead-code warnings. Real supervised Hub/CLI and Local/stdio/HTTP probes both exited 0. Exact scenarios, fixture-only repairs and unexercised external-service limits are recorded in progress.md. Owned temporary fixtures and drivers were removed after both supervisors exited.

## Frozen implementation contracts
- Admission lives in a narrow Agent `operation` module, not a new dispatch executor. Reuse/move the existing RequestIngress and name/namespace/annotation facts; add Hub and Cli variants. RequestContext only carries real ingress and borrowed operation name; RuntimeModel/config come from their existing owners. No invented principal/run/connection fields.
- API: `RequestContext<'a> { ingress: RequestIngress, operation: &'a str }`, `new(ingress, operation)`, `source() -> String`; existing ingress `label()`/`source(tool)` remain semantically intact. `authorize(runtime: RuntimeModel, config: &Config, context: RequestContext<'_>) -> Result<(), AdmissionError>` is synchronous, so the caller can borrow the existing config guard without cloning it.
- Authority remains explicit: Agent-local MCP checks namespace membership; explicit Normal + Room toolset remains supported. Hub retains its existing Room toolset, Skills capability and notification capability rules; no universal Agent-local-toolset restriction is silently imposed on the distinct Hub surface. CLI tmux is explicit local-admin invocation, not fabricated remote authorization.
- All current direct MCP routes enter this gate before resource calls, including Room/Skill/Browser/tmux. Move existing gate logic rather than stacking a second inconsistent authorization system. Unknown/local-disabled tools retain MCP method-not-found; Hub error projections remain existing structured codes. Domain policy/confirmation/path/lease controls remain in domain owners.
- Shared Skill execution: `skills::run(state: AppState, request: SkillRunRequest, request_source: &str, terminal_event_hook: Option<jobs::TerminalEventHook>) -> anyhow::Result<JobResponse>` plus moved skill error mapper. Both stdio and local_service consume it; hub loses run_skill/error implementation but keeps envelope/ledger/ACK/JobUpdate/Response.
- Pure shared slim result projections move out of stdio into a neutral Agent operation_result module; migrate all callers/tests without root aliases. Outer HTTP/MCP/wire schemas remain distinct.
- tmux mutations accept a trailing RequestContext and use its source for audit. CLI create/close reuse config-backed authorized operation bodies and audit without constructing a fake AppState/Hub connection or requiring a new daemon. Explicit local-admin close remains the approval act (no new interactive prompt); remote request confirmation flags and policy stay unchanged. Startup default-session creation remains bootstrap, not user admission.
- Config uses one existing live-subset algorithm for Hub/Local/Standalone; only policy, limits, MCP, toolsets, and existing HTTP options apply. pathPolicy applies only if workspaceRoot matches live. Startup fields remain unchanged; Room enabling prepares the live repository; restart drift remains observable. No resource reconstruction.

## File ownership
- Admission integration owner: operation.rs, operation_result.rs, stdio_server.rs, local_control.rs/http_server.rs import migration. Hub cutover owner: local_service.rs and hub.rs. Main owns final composition and integration repairs.
- Skill owner: skills.rs only; implement frozen shared execution API, no adapter edits.
- Tmux owner: tmux.rs only; implement context/source and shared CLI bodies, report exact main caller patch to Main.
- Config owner: main.rs reload functions/watchers/tests only; owns all main.rs edits while running. Main later adds module declarations and CLI adapter wiring serially.
- Main owns plan/docs, red/green validation, actual runtime smoke and commits. Workers skip all validation and coordinate any unexpected shared mutation before editing.
