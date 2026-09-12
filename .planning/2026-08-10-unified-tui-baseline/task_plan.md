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
- [x] Define Terminal screen integration on the same `TuiApp`
- [ ] Fold future Skill/MCP-oriented views into the same navigation model where justified
- [ ] Consolidate shared widgets/state only after repeated usage demonstrates the seam
- **Status:** pending

#### Frozen Terminal V1 baseline
- The persistent shell/PTY is Agent-owned runtime state, not TUI-owned state. Add a dedicated `TerminalManager` behind `AppState` so tunnel MCP, local MCP, and the human TUI all operate on the same shell state and Terminal lifetime does not depend on the console process.
- The shell is genuinely persistent across Agent and human turns: `cd`, environment changes, sourced virtual environments, shell functions, and foreground interactive programs all affect the same PTY/session.
- The primary model-facing operation is `terminal.repl`; do not recreate tmux-style `create/list/screen/write/resize` as the main public abstraction. `terminal.repl` accepts an ordered semantic input sequence whose normal path is shell source and whose exceptional interactive path can express text/key input.
- Shell source is sequenced by actual shell execution boundaries rather than split on physical newlines. The runtime must not blindly write an entire multi-command source buffer into the PTY because later input can become stdin for an active foreground program. Shell continuation / command lifecycle signals decide when the next logical command may be sent.
- Shell integration is the semantic source of truth for command boundaries, cwd changes, exit code, and prompt-ready/continuation state. Prefer a protocol in the OSC 133 / semantic-prompt family over prompt-string scraping or dirty sentinel output.
- A `TerminalBlock` represents one shell execution boundary, not one `terminal.repl` call. One repl call may yield zero, one, or multiple blocks depending on what the shell actually executes.
- Blocks retain stable structured history for normal shell commands: command/source, origin (`human` / `agent` / `system`), cwd before/after, state, timestamps/duration, exit code, and bounded transcript/output. Interactive/full-screen applications retain block metadata but are not flattened into fake stdout transcripts or full VT screen dumps by default.
- The terminal emulator and the block timeline are separate layers over one PTY. PTY bytes feed an emulator core for current interactive rendering while shell-integration events build the semantic block timeline.
- Terminal V1 should use a mature emulator core rather than implement ANSI/VT behavior in AgenticGPT. eDEX-UI/xterm.js, Neovim/libvterm, and WezTerm's terminal core are architecture references; the Rust implementation choice remains an implementation-time decision.

#### Frozen TerminalScreen interaction
- TerminalScreen is a responsive two-pane master/detail surface, reusing the existing shared `master_detail_layout` behavior.
- Left/master pane is always the normal interactive terminal experience. It renders the live emulator/scrollback and overlays Warp-like block boundaries/status/origin from semantic metadata without injecting decoration text into the underlying PTY stream.
- Right/detail pane is read-only block Preview/Inspector only. Selecting a block shows stable structured metadata and transcript; it is never the live attach surface and never receives PTY keyboard input.
- There is no separate attach page or attach-before/after visual transition. Human terminal interaction remains in the left pane at all times, including `vim`, `htop`, REPLs, SSH, and other alternate-screen/interactive applications.
- Block selection/navigation must not steal ordinary terminal keys. Use Agentic-owned modified shortcuts/commands for block navigation; exact bindings remain smoke-test UX work. The right Preview follows the selected block and may pin historical selection while the live terminal continues independently.
- Top-level Process/Terminal switching remains app-global through `:` direct-jump commands (for example `:process` / `:terminal`) and optional non-conflicting global shortcuts. `Tab` remains available for screen-local focus behavior.
- Narrow layouts collapse using the existing master/detail semantics: the interactive terminal remains the primary pane; Preview is drill-down/secondary detail rather than a third terminal mode.

#### Frozen policy / confirmation boundary
- Shell-source execution must not silently bypass the existing policy/path-policy model merely because it runs inside a PTY.
- When shell content can be resolved into concrete commands safely, evaluate the command set using the same batch-style policy/confirmation semantics as managed process execution. Dynamic or uncertain shell constructs must fail closed into a broader source-level confirmation rather than being treated as automatically safe.
- Whether the TUI is open changes only confirmation presentation/transport, not the underlying policy decision. A single Agent-owned pending confirmation remains the source of truth.
- With no active TUI console, existing notification/confirmation channels such as freedesktop/ntfy remain usable.
- With an active TUI console, the same pending confirmation may additionally surface in the global workspace status, emit a desktop notification, and be answered from the Terminal screen. Closing/disconnecting the TUI must not lose or implicitly resolve a pending request.
- TUI confirmation presence therefore belongs to a future local-console presence/subscription path, not to direct Agent manipulation of the TUI process.

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
| Keep Terminal runtime Agent-owned | The TUI is a separate local client process; PTY/shell state must survive TUI exit and must be identical for model and human callers. |
| Separate VT rendering from semantic Blocks | Emulator state is needed for real terminal applications, while stable command-level history needs shell-integration metadata and transcripts that should not depend on current emulator scrollback/reflow. |
| Keep TerminalScreen two-pane | The left pane stays a real terminal with Warp-like block decoration; the right pane is a pure block Inspector. This preserves normal terminal ergonomics and reuses the established master/detail interaction. |
| Keep policy truth independent of UI presence | TUI presence may add a confirmation frontend but must not alter which action requires confirmation or become the durable owner of the request. |

## Notes
- This plan is intentionally separate from the Managed Job contract/history work.
- The existing Job planning folder keeps its historical name but no longer owns TUI application construction.
