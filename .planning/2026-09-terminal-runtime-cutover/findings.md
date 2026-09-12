# Terminal Runtime Cutover Findings

## Sources

- Inherited plan: `/home/slhaf/Projects/AgenticGPT/.planning/2026-08-10-unified-tui-baseline/`
- 2026-09 review: `local://paste-1.md`
- 2026-09 response/refinement: `local://paste-2.md`
- Current implementation worktree: `/home/slhaf/Projects/AgenticGPT/.worktrees/terminal-block-runtime`
- Neovim reference: `/home/slhaf/Projects/neovim`
- WezTerm reference: `/home/slhaf/Projects/wezterm`

## Confirmed architecture

- The current implementation has correct Agent ownership and a useful persistent PTY/Fish semantic foundation.
- It is architecturally incorrect as a full terminal because the TUI reconstructs a screen from bounded block transcripts and owns a line editor.
- The frozen 2026-08 direction already selected an authoritative mature emulator screen plus an independent semantic observer.
- The 2026-09 Attach/Browse UX is a new refinement and must not be attributed retroactively to the 2026-08 baseline.
- Current ANSI parsing work may remain only as bounded log presentation; it is not an emulator.

## Reference implementation findings

### Neovim/libvterm

- `src/nvim/terminal.c::terminal_receive` feeds arbitrary PTY chunks to libvterm and flushes damage.
- `terminal_send_key` delegates key encoding to libvterm, so application cursor/keypad/keyboard modes remain correct.
- The VTerm screen is authoritative; the Nvim buffer is a delayed display/scrollback projection.
- Emulator geometry and PTY winsize are distinct effects that must remain synchronized.
- Alternate screen is emulator-owned and does not populate normal scrollback.

### WezTerm

- `term/src/terminal.rs::Terminal::new` owns a streaming parser and terminal state; `advance_bytes` consumes arbitrary chunks.
- `TerminalState` owns primary/alternate screens, cursor, mode-aware keyboard/mouse/paste encoding, and writes generated replies through its supplied writer.
- `Screen` provides stable-range access, dirty sequence tracking, changed-row queries, reflow, and primary scrollback.
- `mux/src/renderable.rs` is the closest cell-render adapter reference.
- `mux/src/localpane.rs` demonstrates PTY + emulator resize and input forwarding.
- WezTerm semantic zones are current screen layout metadata, not durable TerminalBlock identity.
- The local `wezterm-term` manifest enables image-related features on several dependencies unconditionally; dependency boundary needs explicit resolution.

## Current implementation gaps

1. `tui/terminal.rs` renders transcript history rather than emulator cells.
2. TUI-owned `input: String` prevents real key, paste, mouse, and terminal-mode behavior.
3. Workspace shortcuts intercept ordinary terminal keys before Attach dispatch.
4. Current console snapshot combines live display and all block transcripts; repeated full clones can exceed the 1 MiB frame limit.
5. Current input writes directly without prompt-gated scheduling or semantic human/agent arbitration.
6. Resize uses outer terminal geometry and updates only the PTY.
7. Reader EIO/EOF and child exit lifecycle derivation need explicit reconciliation.
8. No `terminal.repl` model surface exists yet.

## Reusable implementation

- `AppState` ownership and lazy `TerminalManager` lifecycle.
- portable-pty child and serialized access foundation.
- Fish OSC 7/133/1337 hooks and TerminalBlock metadata.
- Console socket authentication, permissions, framing, and reconnect scaffolding.
- Top-level Process/Terminal route and shared UI chrome.
- Master/detail layout for Browse only.

## Decision-evidence status

All pre-implementation decisions listed in `task_plan.md#Blocking decisions` now have local evidence and a frozen resolution. Independent consistency review initially failed on transport/API gaps; those corrections are tracked in `progress.md` and require a clean re-review before status changes.

## Blocking-decision evidence

### Dependency and feature boundary

- Local WezTerm HEAD is `78cd82dbba7315814bfbff40e246b8bed4b702e7`; local/raw crates.io indices have no evidence-backed release for the terminal crates.
- Pin `wezterm-term` to that git revision. Vendor same-revision `wezterm-escape-parser` and `vtparse` only because std/alloc OSC/APC and several DCS builders are otherwise unbounded.
- Narrow parser override caps accumulated string-control data at 1 MiB, discards overflow through terminator/CAN/SUB, and resumes parser Ground state. This keeps ordered raw fanout while preventing malformed-output growth.
- AgenticGPT disables Kitty but public WezTerm config cannot disable Sixel/iTerm. Sub-limit images remain internally parsed under upstream's 100 MB decoded-image check/16-entry cache; DTOs emit only unsupported placeholders/warnings.
- All relevant projects are MIT; vendored provenance/licenses must remain. WezTerm crates use edition 2024 and require Rust 1.85+; the workspace tracks stable. Parser tests plus first affected-crate build are the feasibility gate.

### History and frame bounds

- Retain at most 256 blocks and 8 MiB aggregate retained log tails.
- Retain a UTF-8-safe 64 KiB tail per normal block; interactive/alternate-screen blocks retain metadata only.
- Evict oldest completed blocks under count/aggregate pressure; never evict the active block.
- Metadata limits: command/source 4 KiB, each cwd 1 KiB, ID 64 ASCII bytes, encoded record at most 8 KiB.
- `BlockPage`: metadata-only, default 50/max 100, opaque descending `(startedAt,id)` cursor, encoded payload budget 896 KiB so 100 individually bounded 8 KiB records plus envelope fit beneath the 960 KiB application budget.
- `BlockLogPage`: at most 32 KiB and 256 lines, cursor contains block/log revision plus byte/line offset. Expired tail cursors return `log_cursor_expired` with a fresh retained start.
- Console hard frame remains 1 MiB; application payload budget is 960 KiB and is measured before writing.
- Reconnect sends screen+lifecycle only. History/log are explicit queries; missed history deltas produce a bounded resync marker.

### terminal.repl and arbitration

- Public MCP owns submit/status/cancel/log-page request/result operations. Attach lease, live screen/Browse pages, history fanout, and lifecycle fanout remain private console protocol.
- Submit contains 1..64 ordered inputs, each string at most 64 KiB and aggregate serialized input at most 256 KiB. Status/cancel use a server request ID; log-page uses block ID.
- Public MCP input origin is always Agent; callers cannot spoof human/system origin. Non-shell foreground input must name its target block.
- Default wait is 5 seconds with 0..30 bound; operation timeout defaults to 300 seconds with 1..900 bound and never force-kills a running foreground process.
- Results expose bounded inline output (16 KiB/block, 240 KiB total), 50 block results/page, request-scoped continuation, and 32 KiB/256-line per-block log paging. Model output never derives from the screen.
- Static terminal-source auto-Allow is limited to explicit literal builtin/path commands; ambiguous PATH/function/alias/compound source requires per-input confirmation bound to the full immutable source digest.
- Submit batches reserve capacity atomically and dispatch FIFO without interleaving. Denial/cancel/timeout stops later undispatched inputs in that request.
- Attach control is exclusive. Agent shell source, targeted foreground input, and resize queue while the lease is held; no human inactivity heuristic is used. A queued foreground event is discarded stale unless its target block remains active after release.
- Agent input queue is bounded to 256 events and 1 MiB; terminal request records are bounded to 256 and never evict active requests.

### Screen DTO direction

- WezTerm `SequenceNo` and stable rows are internal damage/paging signals only; no `StableRowIndex` crosses the wire.
- Styles preserve RGB/default/underline colors plus exact intensity/underline/blink variants and booleans. A 1024-entry table is only deduplication; exact overflow styles inline per run. Ratatui degrades unsupported underline/blink/overline display while wire state remains exact.
- Rows clip overhanging wide cells, preserve width, and use explicit text-truncation/unsupported-graphics markers. Cursor projection intentionally cannot expose private WezTerm wrap-pending state.
- Delta application requires matching geometry generation and adjacent base revision; any mismatch requests a full snapshot without reconnecting.
- Screen transport uses atomic row-chunked logical transfers: 960 KiB per frame, 1..512x1..256/131072 cells, 1024 shared styles plus exact inline overflow, 64 MiB staged total, and 256 chunks. Unsupported geometry/payload yields a coherent error surface instead of reconnect looping.
- Browse pages current primary scrollback through opaque generation-bound cursors; while alternate is active only the current alternate viewport is accessible because pinned public APIs do not expose inactive primary state.

### Lifecycle trust boundary

- Raw OSC 7/133 from PTY output cannot authorize scheduler state because foreground programs can emit them.
- Frozen design pairs each Fish prompt/preexec/postexec event over a private 0600 FIFO with an exact custom OSC 1337 boundary carrying a random token and monotonic sequence. Lifecycle changes only when both channels match.
- Standard OSC 7/133 remains useful to the emulator for presentation/semantic zones but is ignored as durable command authority.
- Agent source uses Fish no-execute validation plus one bracketed paste/Enter and requires emulator-observed paste mode; it is never split or dumped as raw newline-separated input.
- This prevents accidental foreground-output spoofing. Approved source running inside the mutable shell can still inspect/tamper with integration state; missing/tampered events stall/timeout closed. Hostile same-shell isolation remains post-V1 hardening.
