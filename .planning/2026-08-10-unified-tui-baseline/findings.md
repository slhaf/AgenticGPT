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

## 2026-08-26 extraction boundary confirmation
- `config_tui/mod.rs` owned the only full application loop: terminal enter/restore, 100 ms draw/event polling, runtime-error handling, and final outcome extraction. These are app-global responsibilities and have now moved behind `tui::TuiApp::run`.
- `ConfigTuiApp` remains the Config screen state owner: `SetupSession`, wizard navigation, focus/edit/list/MCP/review state, validation/commit flow, and Config-local key handling all stay screen-local.
- `Theme` moved out of `ConfigTuiApp` and is now owned by the application shell; Config rendering receives the shared theme.
- The first concrete screen lifecycle uses a deliberately non-generic enum dispatch (`TuiScreen::Config`) plus `TuiOutcome`; this leaves an obvious seam for Process/Terminal without prematurely introducing a screen trait.
- Existing `config init` behavior remains intentionally modal: it constructs the unified shell directly on the Config screen and the shell exits after commit/cancel/system error. A future long-lived top-level TUI can reuse the same shell with different startup routing.
- Global key handling, command palette/direct-jump routing, shared overlays/footer, and multi-screen state preservation are not extracted yet. They should be driven by the first non-Config screen rather than guessed from Config alone.

## 2026-08-26 first non-Config screen
- Added a real `ProcessScreen` as the second concrete `TuiScreen` variant; the shell remains enum-dispatched rather than introducing a screen trait. `ConfigTuiApp` is boxed because its wizard state is much larger than the Process screen and clippy correctly flagged the unboxed enum size skew.
- Added `agentic-gpt tui` as the first long-lived human-console entry point. It requires a real TTY and starts on Process; `q`/Esc/Ctrl+C leave cleanly through the same `TerminalSession` restoration path.
- Process consumes the already-frozen model-facing `job.list` summary contract (`jobId/group/kind/state/timestamps`) instead of reaching into Agent internals or SQLite directly. Active truth therefore remains the running Agent's live-memory+history merge.
- The console keeps one persistent local MCP client while healthy and polls `job.list` every 500 ms. On disconnect it emits a bounded degraded update, closes/reconnects, and keeps the TUI alive. This avoids opening two MCP sessions per second merely for refresh.
- Async runtime work stays out of Ratatui event/render code: a Tokio poller task sends `ProcessUpdate` snapshots over an in-process channel; the synchronous `TuiApp` drains them on normal terminal events/ticks. This seam is suitable for later Terminal/other live views without making the renderer async.
- Process v0 intentionally renders a flat bounded list only: group, kind, state, short job id, selection, live/degraded state, and exit/navigation keys. Group columns/tabs, history paging via `nextCursor`, and Inspector/detail are deliberately still Phase 4 work rather than being mixed into shell extraction.
- Manual TTY smoke proved both sides of the extraction: `agentic-gpt tui` rendered real current Jobs and exited/restored the terminal; `config ... init` rendered the existing Config wizard under `TuiApp`, Ctrl+C cancelled cleanly, and no temp config file was written.

## 2026-08-26 workspace chrome / navigation framework
- Corrected the implementation order after manual review: the first Process page was useful as a second-screen proof, but it was too screen-owned/log-view-like to count as the planned product TUI framework. Phase 3 therefore resumed before further Process work.
- Extracted the Config TUI's proven `surface_shell_areas` geometry into shared `tui::shell`: the same responsive outer margin and fixed `header -> rule -> body -> rule -> footer` structure now drives both Config pages and the long-lived workspace console. This makes Config's established visual language the framework source rather than the minimal Process prototype.
- Introduced `WorkspaceState` as the long-lived workspace container. It owns the current `WorkspaceRoute`, persistent screen instances, app-global palette state, exit state, and workspace language. `TuiApp` still owns terminal lifecycle/theme and delegates workspace chrome/state through this container. This gives future Process/Terminal switching a state-retaining home without a universal screen trait.
- Workspace chrome is now visibly app-level: current-view title/status in the shared header, shared top/bottom rules, screen-owned body only, and a shared contextual footer. Process no longer draws its own title/footer or decides application exit.
- Added the planned on-demand `:` command palette as the first real overlay layer. It owns text filtering, selection, Esc/Ctrl+C dismissal, Enter execution, `:process`, and `:quit`; later `:terminal` is an additive real route rather than a placeholder today. Palette key events are intercepted before the underlying Process screen, so navigation inside the overlay does not mutate hidden screen selection.
- Manual smoke verified that Process selection remains unchanged after opening the palette, moving within it, and closing it; `:quit` exits/restores the terminal; Config still renders identically through the shared shell helper.
- Phase 3 is intentionally not marked complete yet: actual Process <-> Terminal switching and cross-route state retention cannot be truthfully verified until Terminal requirements/runtime produce the second real workspace screen.

## 2026-08-26 reusable master/detail extraction
- Follow-up review identified the Config Review JSON-preview flow as a stronger reusable interaction seam than static Config columns alone: wide screens keep master + detail visible, while narrow screens should show the interaction-active pane full width.
- Added shared `tui::layout` primitives: `MasterDetailSpec` parameterizes master/detail constraints, gap, and collapse threshold; `PaneMode` selects the active pane when collapsed; `master_detail_layout` returns the resulting areas. Geometry and input mode remain separate so screens can reuse the layout without inheriting Config-specific key behavior.
- Config's existing `surface_columns` now delegates to this shared layout with its original `Min(40) / gap 2 / Min(24) / collapse <66` defaults. Review preview passes `PaneMode::Detail`, so on narrow terminals the final JSON preview now correctly occupies the body instead of disappearing with the collapsed right inspector; other Config pages remain master-first on narrow screens.
- Added `SurfaceCursor` as the shared bounded vertical-row allocator with configurable bottom reservation. Existing Config `next_surface_rows` now consumes this primitive while preserving its two-row action-dock reservation, avoiding a large form rewrite just to prove the seam.
- Moved `render_surface_action_dock` from Config into shared TUI widgets. Config keeps using the same visual behavior, while later Process/Terminal destructive/confirm actions can reuse the dock without copying geometry.
- Process now validates the same master/detail abstraction without adding rich Job retrieval yet: wide screens show Job list + selected-Job inspector, Enter/l switches interaction to detail, Esc returns to list, and narrow screens collapse to the active pane. Screen-specific footer hints are supplied to the shared workspace footer rather than hard-coded by the app shell.
- Real tmux smoke at 100 columns showed list + preview together; resizing the same live detail view to 60 columns produced detail-only full-width output, and Esc returned to the list-only master pane. This verifies responsive geometry and interaction mode independently of future `job.get` stdout/result work.
