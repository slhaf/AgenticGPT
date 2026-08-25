# Findings: Unified TUI Baseline

## Existing reusable foundation
- `src/tui/runtime.rs` already owns terminal setup/restoration and emits `Key`, `Resize`, and `Tick` events through `TerminalSession`.
- `src/tui/theme.rs`, `src/tui/widgets.rs`, and `src/tui/forms/` provide shared visual/form primitives.
- `src/config_tui/mod.rs` currently contains the only full draw/event loop and drives `ConfigTuiApp` at a 100 ms tick cadence.
- `ConfigTuiApp` is application-like in size/structure but strongly couples Config wizard pages, editing, review state, validation, commit behavior, and navigation.

## Architectural direction
- The final product needs one `TuiApp`, not separate Config/Process/Terminal application loops.
- Extraction should start from Config because it is the existing proven app-level implementation.
- The extraction seam should separate app-global terminal/routing/global UI concerns from Config-specific wizard state.
- Process will be the first additional screen used to validate the shell; Terminal follows on the same application.

## Tentative visual/navigation baseline
- Keep application chrome thin. Do not add a persistent top navigation row listing `Process / Terminal / Config`; it competes with the work surface and does not match the existing Config TUI visual language.
- Global shell should keep a single-line current-view title/breadcrumb plus small global status, a screen-owned body, and a contextual footer.
- Prefer an on-demand `:` command palette/direct-jump model for top-level view switching (for example `:process`, `:terminal`, `:config`) instead of permanent navigation chrome.
- Keep `Tab` available for focus movement inside a screen rather than top-level view switching.
- Process is currently best modeled as a persistent multi-panel workspace with stable spatial roles: `Groups -> Jobs -> Output/Detail`.
- On narrow terminals, use priority collapse / drill-down instead of squeezing every Process panel into narrow columns. Exact breakpoints and dimensions are intentionally deferred until implementation/manual review.
- Config should preserve the main body structure and visual language of the existing Config TUI when moved under the unified shell rather than being redesigned merely to fit the shell.
- Keep footer help contextual and limited to actions available in the current screen/focus; full help can remain behind `?`.
- Treat this as a baseline direction, not final cell geometry. Revisit visual details against real Ratatui output when the shell and Process screen are implemented.

## Open questions
- Exact top-level route/screen enum and whether screen dispatch remains concrete enum matching initially or needs a small screen trait after Config + Process exist.
- Whether `config init` launches the unified shell directly into Config and exits after commit, or returns to a top-level TUI route when invoked from inside the main TUI.
- Which footer/overlay/error handling is truly global versus screen-local.
- How global refresh/tick scheduling should coexist with screen-specific refresh needs such as Process live Jobs.
