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
