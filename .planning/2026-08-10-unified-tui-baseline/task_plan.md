# Task Plan: Unified TUI Baseline

## Goal
Establish one application-level TUI shell for AgenticGPT by extracting the proven app/runtime structure from the existing Config TUI, then host Config, Process, Terminal, and future screens inside the same `TuiApp` rather than building independent TUI applications.

## Current Phase
Phase 1 — Extraction Boundary Discovery

## Phases

### Phase 1: Extraction Boundary Discovery
- [ ] Map `ConfigTuiApp` responsibilities into app-global versus Config-screen-specific state/behavior
- [ ] Map reusable `src/tui/` runtime/theme/widget/form primitives and identify missing app-level primitives
- [ ] Freeze the first unified navigation/screen lifecycle contract and startup/exit semantics
- [ ] Decide how existing `config init` enters/exits the unified TUI without regressing current behavior
- **Status:** in_progress

### Phase 2: Extract Unified `TuiApp`
- [ ] Introduce the application-level `TuiApp` shell from the existing Config TUI event/render loop
- [ ] Move global terminal lifecycle, route/screen selection, theme, global key handling, overlays/footer, and shared app context into the shell
- [ ] Keep Config wizard navigation/editing/validation/commit state inside a Config screen/module
- [ ] Migrate current Config TUI behavior onto the unified shell with parity verification
- **Status:** pending

### Phase 3: Screen Model & Navigation Baseline
- [ ] Establish concrete screen routing and screen-local event/render/update boundaries
- [ ] Support switching among top-level screens without duplicating terminal/session ownership
- [ ] Preserve screen-local selection/scroll/edit state across reasonable navigation where useful
- [ ] Avoid unnecessary trait/generic abstraction beyond what Config + first additional screen actually require
- **Status:** pending

### Phase 4: Process Screen Baseline
- [ ] Build Process as the first non-Config screen against the frozen Managed Job/query contracts
- [ ] Support group tabs/columns, mixed-kind renderers, stable selection, live-memory refresh, history cursor loading, and Inspector coordination
- [ ] Validate that the extracted shell is genuinely reusable before further abstraction
- **Status:** pending

### Phase 5: Terminal / Further Screen Integration
- [ ] Define Terminal screen integration on the same `TuiApp`
- [ ] Fold future Skill/MCP-oriented views into the same navigation model where justified
- [ ] Consolidate shared widgets/state only after repeated usage demonstrates the seam
- **Status:** pending

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| One application-level `TuiApp` is required | Config, Process, Terminal, and future screens are intended to live inside one product TUI, so independent app loops would create migration debt. |
| Extract the shell from the existing Config TUI | Config is the only complete, proven TUI application today; its working event/render/state loop is the best source for the baseline. |
| Extract `TuiApp`, not a genericized `ConfigTuiApp` | App-global lifecycle/routing belongs in the shell, while Config wizard state and navigation remain screen-local. |
| Reuse current `src/tui/` primitives | `TerminalSession`, `Theme`, widgets, and form helpers already provide lower-level building blocks. |
| Do not pre-build an elaborate universal screen framework | A unified app shell is necessary, but trait/generic abstractions should be driven by Config plus the first additional screen rather than speculation. |
| Keep global visual chrome thin | Avoid a permanent `Process / Terminal / Config` navbar; prefer a current-view title/status line, screen-owned body, contextual footer, and an on-demand `:` view switcher/direct-jump model. |
| Start Process with stable `Groups -> Jobs -> Output/Detail` spatial roles | A persistent multi-panel workspace gives group/job/output locations stable meaning; narrow terminals should collapse/drill down rather than compress all panels. Exact geometry remains implementation-time UX work. |

## Notes
- This plan is intentionally separate from the Managed Job contract/history work.
- The existing Job planning folder keeps its historical name but no longer owns TUI application construction.
