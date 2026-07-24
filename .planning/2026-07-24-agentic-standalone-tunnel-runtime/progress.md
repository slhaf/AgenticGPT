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
