# Task Plan: Unified TUI Baseline

## Goal
Establish one application-level TUI shell for AgenticGPT by extracting the proven app/runtime structure from the existing Config TUI, then host Config, Process, Terminal, and future screens inside the same `TuiApp` rather than building independent TUI applications.

## Current Phase
Phase 3 — Screen Model & Navigation Baseline

## Phases

### Phase 1: Extraction Boundary Discovery
- [x] Map `ConfigTuiApp` responsibilities into app-global versus Config-screen-specific state/behavior
- [x] Map reusable `src/tui/` runtime/theme/widget/form primitives and identify missing app-level primitives
- [x] Freeze the first unified navigation/screen lifecycle contract and startup/exit semantics
- [x] Decide how existing `config init` enters/exits the unified TUI without regressing current behavior
- **Status:** complete

### Phase 2: Extract Unified `TuiApp`
- [x] Introduce the application-level `TuiApp` shell from the existing Config TUI event/render loop
- [x] Move terminal lifecycle, shared theme, screen dispatch, runtime-error handling, and outcome extraction into the shell; defer command-palette/global-overlay policy to Phase 3 where multiple workspace screens can drive it
- [x] Keep Config wizard navigation/editing/validation/commit state inside a Config screen/module
- [x] Migrate current Config TUI behavior onto the unified shell with unit-suite and manual TTY parity verification
- **Status:** complete

### Phase 3: Screen Model & Navigation Baseline
- [x] Establish concrete Config/Process screen dispatch and screen-local event/render/update boundaries inside one `TuiApp`
- [ ] Support switching among workspace screens without duplicating terminal/session ownership; the app-global `:` command palette/direct-jump layer is implemented, while actual cross-screen switching awaits the first real second workspace route (Terminal)
- [ ] Preserve screen-local selection/scroll/edit state across top-level navigation; `WorkspaceState` owns route + screen instances, Process selection/detail mode are stable across refresh/palette use, but cross-route retention still needs Terminal to verify
- [x] Avoid unnecessary trait/generic abstraction beyond Config + Process; concrete enum dispatch remains sufficient
- **Status:** in_progress

### Phase 4: Process Screen Baseline
- [x] Build Process as the first non-Config screen against the frozen `job.list` contract, exposed through `agentic-gpt tui`
- [ ] Support group tabs/columns, richer mixed-kind renderers, history cursor loading, and Inspector coordination *(stable selection + live refresh + reusable master/detail preview skeleton are implemented; rich `job.get` input/output detail remains pending)*
- [x] Validate that the extracted shell is genuinely reusable before further abstraction via real local-MCP and tmux TTY smoke tests
- **Status:** in_progress

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
