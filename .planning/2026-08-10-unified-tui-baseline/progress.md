# Progress Log

## Session: 2026-08-10

### Current Status
- **Phase:** 1 - Extraction Boundary Discovery
- **Implementation:** not started; architecture split/planning only

### Actions Taken
- Confirmed Process TUI has no current app-level baseline; only shared `src/tui/` primitives and the Config-specific full application exist.
- Decided the final architecture must use one shared `TuiApp` for Config, Process, Terminal, and later screens.
- Chose the existing Config TUI as the extraction source for the unified application shell.
- Split Unified TUI baseline work out of the Managed Job planning scope.

## Session: 2026-08-26

### Current Status
- **Phase:** 2 - Extract Unified `TuiApp`
- **Implementation:** first application-shell slice implemented; Phase 1 complete

### Actions Taken
- Re-validated the August 10 plan against current source; no unified app shell had been implemented since planning.
- Moved terminal lifecycle, 100 ms draw/event loop, common runtime-error path, shared `Theme`, and Config outcome extraction into `src/tui/app.rs` as `TuiApp`.
- Kept Config wizard state/event semantics inside `ConfigTuiApp`; its renderer now receives the application-owned theme.
- Kept dispatch concrete (`TuiScreen::Config`) rather than inventing a universal screen trait before Process/Terminal exist.
- Preserved `config init` / `config import` external behavior through the existing `run_config_tui` wrapper.
- `cargo check -p agentic-gpt` passed.
- `cargo test -p agentic-gpt config_tui` passed.

### Next
- Add the first real route/screen transition and app-global key boundary, preferably with a very thin Process view, then validate switching and screen-local state retention before extracting more shared UI primitives.
- After the shell is proven with a second screen, integrate Terminal runtime/page so human attach/input can live inside the same `TuiApp`.

### 2026-08-26 Process-screen validation
- Added `tui/process.rs`, a minimal real Process screen consuming typed `JobListResponse` updates with stable selection and mixed process/skill/MCP summary rendering.
- Added top-level `agentic-gpt tui` CLI entry and bilingual CLI metadata.
- Added a persistent local-MCP Job client for the console; reconnect/error state is isolated from the TUI event loop.
- Manual tmux smoke: Process screen displayed live current Jobs from the running Laptop Agent, selection/navigation rendered correctly, and `q` exited with the tmux session closing normally.
- Manual Config parity smoke: unified-shell `config init` rendered the pre-existing wizard unchanged and Ctrl+C cancelled without creating the temporary config.
- Review correction: clippy found `TuiScreen::Config(ConfigTuiApp)` produced a large enum variant; boxed only the large Config state instead of suppressing the lint.
- Verification PASS: `cargo check -p agentic-gpt`; `cargo clippy -p agentic-gpt --all-targets -- -D warnings`; `cargo test -p agentic-gpt` (319 unit + 15 config integration + 1 local-control integration + 6 supervisor tests); `cargo fmt --all -- --check`; `git diff --check`.
- Phase 2 is complete. Phase 3 remains open only for real top-level workspace switching/state retention; Process Phase 4 has a deliberately thin but real baseline. The natural next workspace screen is Terminal, which will provide the second target needed to implement/test the planned `:` direct-jump navigation without inventing placeholder routes.

### 2026-08-26 Phase 3 workspace-shell continuation
- Re-centered work on the original Phase 3 plan after user review correctly noted that the earlier Process prototype did not yet expose the planned reusable TUI framework. No further Process feature work was added.
- Moved Config's responsive five-row surface-shell geometry into `tui/shell.rs` and reused it from both Config and workspace rendering.
- Added `WorkspaceState` with persistent route/screen ownership and app-global command-palette/exit state; Process is now a body-only screen under shared chrome.
- Added visible shared workspace header/rules/footer and the `:` overlay/direct-jump framework. Current real commands are `:process` and `:quit`; Terminal is not represented by a fake route before its requirements are defined.
- Fixed an interaction bug found during smoke review: while the palette is open, arrow keys are now intercepted and no longer move Process selection underneath the overlay.
- Manual tmux checks: shared workspace chrome matches Config's shell language; palette renders/filters/exits correctly; Process selection survives palette navigation; Config-init parity remains unchanged.
- Verification PASS: `cargo check -p agentic-gpt`; Agent clippy all targets with `-D warnings`; focused `tui::` tests (8/8 before final additions); full `cargo test -p agentic-gpt` (322 + 15 + 1 + 6); `cargo fmt --all -- --check`; `git diff --check`.
- Phase 3 framework work is now at the honest stopping point before Terminal: shell/chrome/overlay/direct-jump infrastructure exists, but cross-workspace route switching/state-retention acceptance remains open until a real Terminal screen exists.

### 2026-08-26 reusable layout checkpoint
- Extracted parameterized master/detail layout + narrow-pane mode into `tui/layout.rs`; Config and Process both consume it.
- Extracted `SurfaceCursor` and shared surface action dock; Config retained behavior while losing duplicated layout implementation.
- Config Review JSON preview now has truthful narrow-screen behavior: preview/detail becomes the visible full-width pane below the collapse threshold.
- Process uses the reusable layout only as a preview skeleton: selected Job summary on detail side, Enter/l detail mode, Esc master mode; actual stdout/stderr/result retrieval remains deliberately deferred to Process Phase 4.
- Manual responsive smoke passed at 100-column split view and 60-column collapsed master/detail switching.
- Verification PASS after the extraction: Agent check + all-target clippy `-D warnings`; focused TUI suite; full `cargo test -p agentic-gpt` (324 + 15 + 1 + 6); rustfmt check; `git diff --check`.

### 2026-08-26 targeted TUI regression pass
- Ran manual Config regression at 100x28 and 60x24: Basic and Connection pages retained the established split/collapsed rendering, keyboard movement, contextual footer, and bottom action dock; Local-mode flow reached Optional Center and Review without write side effects.
- Exercised Review -> Preview final JSON on the real Local flow. Wide view retained master + JSON detail; resizing the live preview to 60 columns produced detail-only JSON; Esc returned to the Review master pane. Temporary config targets remained absent because the flow never confirmed write.
- Exercised Process selection, palette open/navigation/close, Enter detail, Esc detail-back, and application exit.
- Found one real regression during the pass: moving Esc handling out of the workspace shell had removed the old "Esc exits Process from the master list" behavior. Fixed the boundary so Esc returns from an active detail pane, while Esc from the master list exits as before; q continues to exit from either mode.
- Post-fix verification PASS: `cargo check -p agentic-gpt`, all-target clippy with `-D warnings`, focused `tui::` suite (10/10), and `git diff --check`.

## Session: 2026-08-27

### Current Status
- **Phase:** Terminal V1 design freeze before implementation
- **Implementation:** no Terminal code written yet; runtime/screen/policy seams frozen against the current clean `3cf7260` checkpoint

### Actions Taken
- Re-read the unified TUI plan and current `TuiApp` / `WorkspaceState` / `ProcessScreen` implementation. Confirmed the working tree remains clean and `main` remains one commit ahead of `origin/main` at `3cf7260 feat: establish unified TUI workspace shell`.
- Confirmed Terminal cannot be TUI-owned: the TUI is already a separate process using the Agent's local Unix MCP ingress, so only Agent-owned `AppState` runtime state can guarantee that human input and model `terminal.repl` operate on one persistent shell and that the terminal survives TUI exit.
- Froze a dedicated `TerminalManager` direction behind `AppState`, with one persistent PTY/shell, shell-integration semantic events, a PTY-aware sequential input scheduler, terminal-emulator state, and structured `TerminalBlock` history.
- Froze `terminal.repl` as the primary Agent-facing surface rather than a tmux-like collection of screen/write/resize tools. Ordered shell-source/text/key input remains the intended shape; actual source schema is deferred until the runtime spike proves the sequencer.
- Froze command-boundary semantics: one repl call is not one block; actual shell command lifecycle events define zero-to-many blocks. Multi-line/multi-command input must be fed according to continuation/command-ready state rather than dumped into the PTY or split naively on newlines.
- Refined the TerminalScreen UX after reviewing eDEX-UI and Neovim-style embedded terminals. Final baseline is two-pane: left is always the live interactive terminal with Warp-like Block decoration over emulator/scrollback; right is a read-only Preview/Inspector for the selected block. There is no separate attach page and no visual attach transition.
- Reused the existing master/detail mental model: terminal remains master/primary; Preview is detail/secondary and can collapse on narrow layouts. Process/Terminal top-level switching stays in the `:` palette (`:process` / future `:terminal`) with optional global shortcuts chosen later by smoke testing.
- Froze the policy distinction: TUI presence changes confirmation presentation only. Shell source should use batch-like policy checks when safely resolvable; dynamic/uncertain constructs conservatively escalate to broader confirmation. The pending request remains Agent-owned whether answered by freedesktop/ntfy or by a future TUI confirmation frontend/status indicator.
- Identified implementation references rather than dependencies to copy blindly: eDEX-UI for PTY/frontend layering, Neovim/libvterm for embedded terminal lifecycle/alternate-screen/input behavior, LazyVim/Snacks for focus UX, and WezTerm terminal core as a possible Rust emulator implementation reference.

### Next
- Finish the final focused source-review pass in WezTerm `mux::LocalPane`: PTY reader thread/lifecycle, writer ownership, resize flow, `Terminal` locking, and output notification/damage propagation.
- Then implement a minimal Agent-owned PTY/runtime spike before any fake Terminal route using the now-preferred `portable-pty + wezterm-term + wezterm-escape-parser` stack. Prove persistent cwd/environment state, shell integration command boundaries/exit status/cwd, interactive foreground behavior, resize, and emulator rendering.
- After the runtime spike is credible, add the real `WorkspaceRoute::Terminal`, persistent `TerminalScreen`, and `:terminal` command to close Phase 3 cross-route/state-retention acceptance.
- Add TUI confirmation presence/status only after the shared Agent-owned pending-confirmation path is explicit; do not make the TUI the durable owner of confirmations.

### 2026-08-27 source-review checkpoint
- Cloned `/home/slhaf/Projects/neovim` and `/home/slhaf/Projects/wezterm`, initialized CodeGraph for both, and registered both repositories with `cg-watch` for local source navigation.
- Neovim review established the host-terminal invariants: PTY bytes feed the emulator model; keyboard is encoded against terminal modes; alternate screen remains emulator state; host UI determines resize; refreshes are event-driven/batched rather than coarse polling.
- WezTerm review established that its reusable terminal core already owns primary scrollback, alternate screen, reflow, semantic cell attributes, dirty-row sequence tracking, and terminal-aware input encoding. This removes the earlier need for an extra Agentic `TerminalSurface`/scrollback mirror.
- Froze the semantic split: live display comes from `wezterm-term::Screen`; durable command history remains Agentic `TerminalBlock`; current Prompt/Input/Output placement is recomputed as semantic zones after resize/reflow rather than used as permanent block identity.
- Froze the PTY-output fanout design: one byte stream feeds both `wezterm_term::Terminal::advance_bytes` and an independent streaming `wezterm_escape_parser::Parser` used by `SemanticTracker`. The latter observes OSC 133 lifecycle/status, OSC 7 cwd, and OSC 1337 command/user-var state without requiring a fork of `wezterm-term`.
- Current stop point: source review is effectively complete except for one final WezTerm `LocalPane` wiring pass. No Terminal implementation code has been started yet.
