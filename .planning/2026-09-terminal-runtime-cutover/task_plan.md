# Terminal Runtime Clean Cutover

## Status

- State: `implementation_ready`
- Canonical repository: `/home/slhaf/Projects/AgenticGPT`
- Implementation worktree: `/home/slhaf/Projects/AgenticGPT/.worktrees/terminal-block-runtime`
- Inherits: `.planning/2026-08-10-unified-tui-baseline/`
- Planning date: 2026-09
- Independent planning-consistency and implementation-readiness reviews: PASS on final frozen contracts.

## Goal

Replace the transcript-backed pseudo-terminal surface with one Agent-owned persistent PTY backed by a mature VT emulator. Preserve durable shell execution semantics as an independent `TerminalBlock` history. Deliver a full-width, ordinary interactive Attach experience and a temporary Browse state for block history, folding, and inspection.

## Decision provenance

### Inherited decisions — 2026-08 baseline

These remain authoritative unless explicitly refined below:

1. The persistent PTY/shell is owned by `TerminalManager` behind `AppState`; TUI and model clients share one shell.
2. Terminal is not `process.exec`; cwd, environment, shell functions, virtual environments, and foreground programs persist.
3. The primary model interface is `terminal.repl`, accepting an ordered semantic input sequence rather than tmux-style primitive tools.
4. Shell source is scheduled by actual prompt/continuation/executing state, not split on physical newlines and not dumped as one byte blob.
5. `TerminalBlock` maps one shell command lifecycle boundary. One repl request may yield zero, one, or many blocks.
6. Shell integration events, not terminal pixels or prompt text, are the source of truth for command start/completion, exit status, cwd, and readiness.
7. Raw PTY bytes feed two independent consumers: a mature live VT emulator and a semantic shell-integration observer.
8. The emulator screen and durable block history are different models. Emulator scrollback is not block log storage; block transcript is not a terminal screen.
9. Full-screen/interactive applications retain block metadata but are not persisted as frame histories or fake stdout.
10. Ordinary terminal keys must reach the PTY; host navigation must use a non-conflicting escape mechanism.
11. Preferred implementation direction is `portable-pty + wezterm-term + wezterm-escape-parser`; do not create an Agentic-owned second terminal screen/scrollback.

### 2026-09 refinements

These refine rather than rewrite the inherited baseline:

1. The 2026-08 UI baseline used a persistent two-pane live-terminal + inspector layout. The 2026-09 refinement makes Attach full-width by default and opens the inspector only in temporary Browse state.
2. Host escape mechanism is required, but the exact chord is not a stable contract. It is `TBD/configurable`; any initial binding is provisional and must be isolated behind a keymap boundary.
3. Screen transport uses `geometry_generation` plus monotonic `screen_revision`; it does not expose WezTerm `StableRowIndex` as permanent protocol identity.
4. Resize/reflow, reconnect, or revision mismatch invalidate the delta chain and require a full screen snapshot.
5. Input semantics are frozen before protocol cutover: shell source, raw text, semantic key, paste, mouse, and resize are distinct message kinds.
6. An explicit `InputArbiter` separates immediate human foreground input, emulator protocol replies, and prompt-gated agent shell-source scheduling.
7. Inspector “full log” means the complete currently retained bounded log, fetched by block ID with paging; retention remains bounded.
8. V1 excludes graphics protocols and inline images. WezTerm image-related features should be disabled where supported; otherwise their dependency cost must be accepted explicitly or isolated by a narrow patch/fork decision before core implementation.
9. Live screen, block history, and inspector log use separate transport lifecycles.
10. The current transcript/ANSI renderer has no compatibility claim as the Attach surface. It may be retained only as a bounded log presentation helper if it remains useful.

### 2026-09 concrete closures

These are new implementation decisions produced by the September review and blocker research; they are not retroactively attributed to the August baseline:

| Closure | September source |
|---|---|
| Pinned WezTerm term plus bounded local parser overrides and accepted sub-limit image behavior | Local WezTerm/vtparse source and September memory-safety blocker research |
| Protocol-owned screen DTO, generation/revision rules, row-chunked transfer, geometry/style/cell bounds | September design response (`local://paste-1.md`), critique (`local://paste-2.md`), and local WezTerm API audit |
| Separate Browse screen paging, block metadata paging, log paging, retention and frame budgets | September critique and local console frame/history audit |
| V2 console handshake, exclusive Attach lease, resume token, mutation acknowledgement, and no input replay | September InputArbiter refinement and current reconnect-path audit |
| Exact `terminal.repl` submit/status/cancel/log operations, output paging, queue/timeouts, and terminal-source confirmation digest | September model-interface blocker research; August explicitly deferred final tool schema |
| Conservative prompt-edit/continuation state and foreground-pgrp evidence around OSC 7/133/1337 | September lifecycle audit of the Fish spike and local Fish/WezTerm semantics |

## Frozen architecture contracts

### Ownership and byte flow

```text
Agent-owned TerminalManager
  PTY reader -> ordered raw chunk fanout
    -> wezterm_term::Terminal::advance_bytes -> authoritative Screen
    -> authenticated Fish hook channel + PTY BoundaryObserver -> lifecycle + TerminalBlock store
  serialized PTY writer
    <- emulator-generated terminal replies
    <- human attach input selected by InputArbiter
    <- prompt-gated agent/system scheduler
```

- Every raw PTY output chunk reaches both consumers in original byte order.
- Emulator replies and external inputs share one serialized writer; byte-level interleaving is prohibited.
- Child wait status, PTY EOF, and reader failure remain separate facts. Child wait is authoritative for process exit.

### Emulator ownership

- `wezterm-term::Screen` is the only live terminal screen and owns primary scrollback, alternate screen, reflow, cursor, terminal modes, wide cells, and dirty-row sequence state.
- AgenticGPT does not maintain a second live cell grid or scrollback copy.
- `TerminalBlock` identity never derives from screen rows. Semantic zones may project a block onto current/reflowed rows but are disposable layout data.

### Dependency boundary

- Pin `wezterm-term` to git revision `78cd82dbba7315814bfbff40e246b8bed4b702e7`; commit the resulting lockfile.
- Vendor only `wezterm-escape-parser` and `vtparse` from that same revision under `vendor/wezterm-parser/`, preserving MIT license/provenance, and override the git packages through Cargo patch entries.
- The narrow parser patch caps accumulated OSC, APC, short-DCS, Sixel, and XTGETTCAP data at 1 MiB per control sequence. Overflow discards that action through its legal terminator/CAN/SUB, emits `terminal_control_sequence_truncated`, and resumes Ground parsing; split/unterminated recovery has focused tests.
- `wezterm-escape-parser` uses only its `std` feature. `wezterm-term` uses `default-features = false`, although upstream still enables image dependencies internally.
- AgenticGPT's own `TerminalConfiguration` disables Kitty graphics and `TERM=xterm-256color` does not advertise Sixel/iTerm. Sub-1-MiB unsolicited Sixel/iTerm may still parse under WezTerm's 100 MB decoded-image check/16-entry cache, but no graphics bytes/types cross Agentic DTOs; image cells project as `unsupported_graphics`.
- Parser caps make malformed control output memory-bounded without altering ordinary raw PTY fanout. Strictly rejecting every image action or lowering decoded-image bounds would require a wider `wezterm-term` fork and remains post-V1 hardening.
- Do not depend on WezTerm mux/GUI crates, termwiz surface helpers, or serde features. Adapter boundary is `Terminal`, active `Screen`, terminal input methods, and protocol-owned DTOs.

### Screen transport

```text
FullScreen {
  geometry_generation,
  screen_revision,
  cols,
  rows,
  viewport,
  lines: [ScreenRow],
  cursor,
  title,
  bell,
}

ScreenDelta {
  geometry_generation,
  base_revision,
  screen_revision,
  changed_rows,
  cursor,
  title?,
  bell?,
}
```

- `geometry_generation` changes on resize/reflow, reset, or primary/alternate active-buffer switch.
- `screen_revision` is monotonic within a generation. A sparse Delta advances exactly one revision and is valid only when generation matches and `base_revision + 1 == screen_revision`.
- Reconnect, mismatch, dropped delta, resize, reflow, reset, or active-buffer switch triggers a full screen transfer.
- Screen rows are protocol DTOs, not serialized WezTerm internal types. They preserve cell text, width, foreground/background, and supported attributes.
- Normal updates send dirty rows only. Snapshot size and delta size are independently bounded.

#### Concrete screen DTO and bounded transfer

- Styles are protocol-owned values: resolved default/RGB foreground, background, and underline color; exact intensity `normal|bold|dim`, underline `none|single|double|curly|dotted|dashed`, and blink `none|slow|rapid` enums; plus italic, reverse, invisible, strikethrough, and overline booleans. Ratatui 0.29 maps unsupported underline/blink variants to its nearest modifier and cannot draw overline; these explicit renderer degradations do not lose exact wire state.
- Each physical viewport row is top-to-bottom and contains runs using `ScreenStyleRef::Table(u16) | Inline(ScreenStyle)`. Runs preserve visible cell text/width; gaps/trailing columns are explicit blanks. Adapter iteration clips to `[0, cols)`: an overhanging final-column wide cell becomes U+FFFD with remaining width and `text_truncated = true`. Summed widths equal `cols`.
- Wire shapes are `ScreenRun { style: ScreenStyleRef, cells }` and `ScreenCell { text, width, text_truncated, unsupported_graphics }`. An image-backed cell sets `unsupported_graphics = true`, carries no image identifier/bytes, and renders a deterministic replacement glyph.
- Cursor carries `row`, `col = min(public_x, cols - 1)`, visibility, and shape; WezTerm's private wrap-pending bit and distinction between `x == cols` and a clamped final-column cursor are intentionally not represented. Bell is a monotonic counter. Title is UTF-8-safe and at most 4 KiB.
- Logical Full and Delta transfers are atomic but may use bounded wire chunks:

```text
FullBegin { generation, revision, geometry, viewport, styles, cursor, title, bell, chunk_count }
FullChunk { generation, revision, chunk_index, first_row, lines }
DeltaBegin { generation, base_revision, revision, style_additions, metadata, chunk_count }
DeltaChunk { generation, base_revision, revision, chunk_index, changed_rows }
ScreenTransferEnd { kind, generation, base_revision?, revision, chunk_count }
```

- `viewport` is `{ active_buffer: primary|alternate, retained_rows }`; only current physical viewport rows cross the live-screen wire. Primary `retained_rows` reports scrollback+visible rows; alternate reports visible rows.
- The generation-local style table is a dedup optimization capped at 1024 entries and 256 KiB encoded. FullBegin carries only that bounded table; DeltaBegin additions are append-only until either cap fills; every further exact style is inline in its run. Table exhaustion never quantizes colors, bumps generation, or causes retry.
- Each frame is at most 960 KiB application payload under the 1 MiB hard frame limit. Frames from one screen transaction are contiguous and never interleaved with another screen transaction.
- Safety bounds: 1..512 columns, 1..256 rows, 131072 cells, 1024 shared styles plus bounded inline run styles, 32 UTF-8 bytes per cell, 64 MiB maximum staged logical transfer, and 256 chunks. Overlong cell text becomes U+FFFD with `text_truncated = true` while preserving width; titles truncate on a UTF-8 boundary with an explicit flag.
- A transaction whose actual encoded form exceeds 64 MiB returns `screen_payload_unsupported` and a non-reconnecting error surface; valid styles are never silently quantized.
- Client stages a transfer by generation/base/revision and commits atomically only after all contiguous chunks and `ScreenTransferEnd` validate. Duplicate/missing/mismatched chunks discard staging and request Full.
- Updates arriving during a transfer coalesce to the latest state. After commit, the server sends a valid retained next delta or starts a fresh Full; it never interleaves or applies a partial transaction.
- Deltas apply only to the committed matching base generation/revision. Reconnect always starts a new Full transfer.

#### Browse screen paging

- Browse scrollback is a read-only screen projection, not TerminalBlock log and not a second live grid.
- `BrowseScreenRequest { request_id, generation, anchor, row_count }` selects `newest` or an opaque before/after cursor; `row_count` is 1..256.
- `BrowseScreenPage { request_id, generation, screen_revision, active_buffer, first_cursor, last_cursor, has_older, has_newer, styles, lines }` uses the same bounded row/style DTO and row-chunk framing as Full.
- The opaque cursor encodes generation plus adapter-internal stable-row position; raw WezTerm `StableRowIndex` is never serialized. Reflow/reset/active-buffer change returns `browse_cursor_expired` with a fresh newest cursor.
- Primary Browse pages the configured bounded 3500-row WezTerm scrollback plus visible rows. While alternate is active, the only valid page is its current visible viewport; inactive primary access is neither promised nor copied.
- If ongoing output evicts the requested primary row, the server returns cursor-expired rather than a shifted/misidentified row. Browse may refresh explicitly; it never changes the live Attach viewport or PTY size.

### History transport

```text
BlockHistoryDelta  -> append/update metadata only
BlockPage          -> bounded metadata pagination
BlockLogPage       -> bounded content by block_id + byte/line cursor
```

- No message repeatedly clones every block transcript.
- Per-block log and total history retention are bounded.
- Interactive/alternate-screen blocks do not persist VT frame history.
- Right inspector pages through the retained log; “full” never means unbounded storage.

#### Concrete history retention and paging

- Retain at most 256 blocks and 8 MiB aggregate raw log tails.
- Normal completed/running blocks retain a UTF-8-safe 64 KiB tail. Interactive/alternate-screen blocks retain metadata only and never VT frame history.
- On pressure, evict oldest completed blocks until both limits fit; never evict the active block.
- Bound command/source to 4 KiB, each cwd to 1 KiB, IDs to 64 ASCII bytes, and one encoded metadata record to 8 KiB. Expose truncation and total-seen counters.
- `BlockPage` is metadata-only, default 50/max 100, with an opaque descending `(startedAt,id)` cursor and 896 KiB encoded response budget. A 100-record page of individually bounded 8 KiB metadata records remains below this budget including its envelope.
- `BlockLogPage` is at most 32 KiB and 256 lines, addressed by block ID plus opaque log revision/byte/line cursor.
- A cursor preceding the retained tail returns `log_cursor_expired` with the fresh retained start. An evicted block returns `block_not_retained`.
- History append/update deltas never contain log bytes. Delta loss yields `history_resync_required`; reconnect explicitly queries pages.

### Private console session protocol

- Protocol V2 is a strict tagged client/server envelope. Unknown versions, variants, fields, and invalid bounds receive a structured error and close without mutating Terminal state.
- Connection sequence is `ClientHello { version, resumeLeaseToken? }` -> `ServerHello { version, connectionId, leaseState, lifecycle }` -> `Subscribe { mode, rows?, cols? }`.
- An Attach subscription first acquires/resumes control and applies its supported body geometry atomically to emulator+PTY, then the server begins Full screen transfer. A denied lease yields read-only Browse; a Browse subscription receives current screen without resizing.
- Server assigns a random connection ID and, on lease grant, an opaque 256-bit resume token plus monotonic `leaseRevision`. Every mutating Human request carries connection ID, lease revision, and a per-connection monotonic request ID.
- Mutating requests receive `InputAccepted` or `InputRejected` correlated by request ID. Duplicate/out-of-order IDs are rejected. Acceptance means the event entered the serialized runtime path, not that the foreground application consumed it.
- Console client never replays unacknowledged input/resize/lease mutation after a write failure. Only idempotent Full/page/status requests may retry after a new handshake.
- Unexpected holder disconnect preserves the lease only for the two-second grace and only a matching resume token can reclaim it. Explicit Release, Browse, or clean close releases immediately. Grace expiry increments lease revision and wakes the scheduler.
- Server envelopes keep `ScreenTransfer`, `Lifecycle`, `History`, `LogPage`, `Lease`, `Response`, and `Error` as distinct variants. Screen updates coalesce, history/lifecycle loss emits a resync-required event, and responses/lease changes are never silently dropped.
- Socket same-UID verification, 0700 runtime directory, 0600 socket, 1 MiB hard framing, and 960 KiB pre-write application budget remain mandatory.

### Input model

Distinct semantic variants are required:

```text
ShellSource { request_id, source, origin }
TextInput   { target_block_id?, text, origin }
KeyInput    { target_block_id?, key, modifiers, state, origin }
PasteInput  { target_block_id?, text, origin }
MouseInput  { target_block_id?, kind, button, modifiers, row, col, origin }
Resize      { rows, cols, origin }
```
- Human Attach sends text/key/paste/mouse events, not a TUI-owned submitted line.
- Semantic key/paste/mouse events are encoded against current emulator modes by the Agent-owned emulator before bytes reach the PTY.
- Emulator protocol replies bypass shell scheduling and are written with highest correctness priority.
- `ShellSource` enters the prompt-gated scheduler; it never writes directly to an active foreground program.

#### terminal.repl public schema

- `TerminalReplArgs` is a strict camelCase tagged union; unknown variants/fields are denied:

```text
Submit  { operation: "submit", inputs: [TerminalInput; 1..64], waitSeconds?, timeoutSeconds? }
Status  { operation: "status", requestId, cursor?, waitSeconds? }
Cancel  { operation: "cancel", requestId }
LogPage { operation: "logPage", blockId, cursor? }
```

- `Submit` assigns a bounded opaque `terminalRequestId`. Each text/source input is at most 64 KiB and aggregate serialized arguments are at most 256 KiB. `waitSeconds` defaults to 5 and is bounded 0..30; `timeoutSeconds` defaults to 300 and is bounded 1..900.
- Public input is a second strict tagged union:

```text
{ type: "shellSource", source }
{ type: "text", targetBlockId, text }
{ type: "key", targetBlockId, key, modifiers, state }
{ type: "paste", targetBlockId, text }
{ type: "mouse", targetBlockId, kind, button?, modifiers, row, col }
{ type: "resize", rows, cols }
```

- Origin is never a public request field. The server assigns Agent origin to public inputs, Human origin to authenticated private Attach events, and System origin to emulator replies.
- `key` is a tagged protocol enum covering Unicode text, Enter, Tab, Backspace, Escape, arrows, Home/End, PageUp/PageDown, Insert/Delete, and F1..F24. Modifiers are a deduplicated list of shift/control/alt/super; state is press/repeat/release.
- Mouse kind is press/release/move/drag/scroll, coordinates are physical viewport cells, and button is left/middle/right or wheel direction where applicable.
- V1 supports X10/VT200/SGR cell-coordinate mouse reporting. If the active application requests SGR-Pixels, clients without pixel-offset events receive `mouse_pixels_unsupported` and no fabricated coordinate is sent.
- Every scalar enum denies unknown values. Text is valid UTF-8; ShellSource additionally rejects NUL, ESC, CR, DEL, and C0 controls other than LF/Tab so it cannot terminate its bracketed-paste envelope. Request/block IDs use bounded opaque types, rows/cols satisfy screen geometry bounds, and all other unknown fields are denied.
- Every operation returns `TerminalReplResult { operation, requestId?, status, shellState, activeBlockId?, blocks, nextCursor?, logPage?, screenGeneration, screenRevision, deadlineAt?, warnings }`.
- Status is completed, active, cancelled, rejected, timed_out, or error. `blocks` is an ordered append-only request-scoped page of at most 50 `TerminalBlockResult { metadata, outputTail, outputTruncated, nextLogCursor? }`; per-block inline output is at most 16 KiB and total serialized MCP result is at most 240 KiB. `nextCursor` retrieves later block results through `Status`.
- `LogPage` exposes the same retained per-block log used by the inspector, at most 32 KiB/256 lines per call, with explicit `block_not_retained` and `log_cursor_expired` results. Thus command output is model-readable without screen inspection.
- Terminal source preflight recognizes one command of literal words with no expansion, substitution, glob, redirection, control operator, or assignment. It strips only syntactic `builtin`/`command` prefixes, resolves explicit executable/operand paths against cwd, then requires the existing program+argument policy decision to be Allow and all path checks to pass. The additional terminal-safe forms are: explicit `builtin` with `cd|pwd|set|export|echo|printf|true|false`, or `command` with an absolute/relative executable. Existing interpreter/wrapper rules still apply to the underlying program+args, so `fish|sh|python -c`, `env`, `xargs`, `sudo`, `exec`, `eval`, `source`, `.`, non-allowlisted builtins, bare PATH lookup, functions/aliases, compound source, and ambiguous effects remain Confirm/Deny unless the user has an exact overriding rule.
- Add a terminal-source confirmation kind bound to SHA-256 over length-prefixed immutable complete source, current cwd, profile, terminal runtime ID, and request/input identity. UI may show a bounded preview, but approval carries/verifies the full digest and becomes stale if request state/cwd changes.
- Structural validation and queue-capacity reservation are all-or-nothing for the full Submit batch. The actor assigns sequence at accepted enqueue; requests and their inputs never overtake an earlier request awaiting policy/confirmation.
- Each ShellSource input receives its own just-in-time policy/confirmation decision. Denial, rejected confirmation, timeout, or cancellation stops that request and removes all its later undispatched inputs; already completed/running blocks remain in its result.
- Once admitted, dispatch is FIFO after emulator replies and current Human lease input; batches never interleave.
- `Status` with optional wait observes later completion after an earlier active result. `Cancel` removes undispatched inputs but never sends a signal or force-kills a running foreground process; the active block remains returned/observable.
- The server retains at most 256 request records, never evicts an active request, and stores only bounded input state plus block IDs/status. Expired IDs return `terminal_request_not_retained`.
- The request deadline starts when the validated batch is enqueued and includes lease, scheduler, policy, and confirmation wait. On expiry the actor removes undispatched inputs; running input continues. Lifecycle evidence timestamped at or before the deadline wins over the timer; otherwise timeout wins in the single actor order.
- MCP transport cancellation stops only that call's wait; it does not cancel the Agent-owned terminal request. Explicit `Cancel` is the sole public runtime cancellation operation.
- Public policy admission reuses existing MCP concurrency, rule evaluation, allowed-root/path resolution, confirmation, and structured error paths through a terminal-specific source preflight. It intentionally does not inherit `process.exec`'s `requires_tty_not_supported` rejection because this runtime owns a PTY.
- Agent-origin foreground input is admitted only for a live matching foreground block. If queued behind a lease, it is delivered only if that same block remains foreground after release; otherwise it returns `terminal_input_stale`/`no_foreground` and never reaches a later prompt.
- Agent-origin Resize participates in the same FIFO batch, never bypasses a Human lease, and applies emulator+PTY geometry only after lease release; it does not require PromptReady.
- The ordered Agent input queue is bounded to 256 events and 1 MiB aggregate payload; overflow returns `terminal_queue_full`.

### Input arbitration

- At most one console client holds the human Attach control lease. Other clients remain read-only/Browse until acquiring control.
- Human Attach bytes are delivered immediately to the current foreground program while the lease is held.
- Agent/system shell-source and targeted foreground-input requests may queue while a human Attach lease exists but are not injected.
- Releasing Attach control, entering Browse, disconnecting, or explicit yield releases the human lease. A short reconnect grace may retain the lease but must be bounded.
- Agent scheduler dispatch requires: no human Attach lease, shell lifecycle is safe, and the previous scheduled command has reached its required boundary.
- Human takeover during an agent foreground command may send raw foreground input, but no additional agent shell source dispatches until the shell returns to a safe state.
- The initial implementation does not use an inactivity timeout to guess whether a partially typed human command is complete.

- Attach lease reconnect grace is 2 seconds. Grace blocks agent source dispatch; expiry releases the lease deterministically.
- The initial host keymap uses a provisional implementation binding selected and recorded by direct PTY smoke. The binding is isolated behind `TerminalHostKeymap` and is not a stable configuration/API decision.
- TUI Terminal workspace state is explicit: `mode = Attach|Browse` and `lease = Acquiring|Held { revision, resume_token }|Denied|Lost`. Only `Attach + Held` forwards terminal events.
- Host keymap evaluation precedes terminal forwarding for its exact configured chord only. Entering Browse sends explicit Release before enabling host navigation; leaving Browse sends Acquire with current body geometry and forwards nothing until Held.
- Lease denial/loss forces read-only Browse and a visible status. Human events carrying a stale connection/lease revision are rejected rather than reassigned.
- Runtime capture enables bracketed-paste and mouse events and preserves Key/Paste/Mouse/Resize as distinct events. Attach forwards them; Browse consumes navigation locally.

### Shell lifecycle and scheduler

Required lifecycle states:

```text
Starting
PromptReady { cwd }
PromptEditingOrContinuation { origin }
AwaitingCommandStart { terminal_request_id, input_index, origin }
Executing { block_id, origin }
ForegroundInteractive { block_id, origin, foreground_pgrp }
AwaitingPrompt { block_id, exit_code }
Exited { child_status }
Error { code }
```

- Fish emits standard OSC 7 cwd and OSC 133 `A/C/D;<status>` for emulator presentation/semantic zones, but raw PTY OSC is never lifecycle authority.
- Agent creates a 0600 FIFO inside its 0700 runtime directory plus a random 256-bit token. Unexported Fish hook state sends a bounded control record `{ version, token, sequence, kind, cwd?, status?, sourcePrefix? }` through the FIFO from `fish_prompt`, `fish_preexec`, and `fish_postexec` without replacing the user's prompt.
- Each hook also emits a custom OSC 1337 `SetUserVar=AGENTICGPT_BOUNDARY=<base64(token,sequence,kind)>` at the exact PTY byte boundary. Lifecycle accepts an event only after matching the FIFO record and PTY boundary by token+strictly increasing sequence; standard/app-emitted OSC 7/133 is presentation-only.
- BEL or ST terminators and arbitrary PTY/FIFO chunk splits are accepted. Records are capped at 16 KiB and unmatched records/boundaries at 16 entries; malformed, replayed, overflowed, or unpaired data never changes readiness and yields a bounded diagnostic.
- Fish loads the user's normal configuration. Child environment advertises `TERM=xterm-256color`, `COLORTERM=truecolor`, and `TERM_PROGRAM=AgenticGPT`; the token/FIFO path are not exported to child processes.
- `PromptReady` is published only from a matched authenticated prompt record carrying cwd. Any accepted Human input there conservatively enters `PromptEditingOrContinuation`; only a later matched preexec or prompt record leaves it. No idle timer guesses completion.
- Agent `ShellSource` is checked with Fish's no-execute parser as complete source, then delivered at PromptReady as one bracketed paste plus Enter under the serialized writer. Dispatch requires emulator-observed bracketed-paste mode; otherwise it fails closed. A new matched prompt without intervening matched preexec completes that input with zero blocks and `source_not_executed`.
- A matched authenticated preexec pair is the sole normal block-start event. Runtime correlation assigns Agent request/input origin only when awaiting that scheduled source; otherwise the authenticated Human path owns origin. Hook source is bounded metadata, never origin authority.
- PTY bytes between matched authenticated preexec and postexec boundaries append only to that active block's bounded raw log after integration marker removal. The complete original ordered bytes still reach the emulator unchanged.
- Matched postexec records exit code and enters `AwaitingPrompt`; the next matched prompt record supplies cwd-after and permits the next scheduler item. Child exit finalizes a still-active block when no prompt can occur.
- Linux foreground process-group observation (`tcgetpgrp`, compared with the known shell pgrp) upgrades `Executing` to `ForegroundInteractive`; it is lifecycle evidence, not a timing or screen heuristic. Raw interactive input never starts a block.
- `TerminalLifecycleSnapshot { revision, state, cwd, active_block_id?, prompt_count, pty_eof, child_exit?, error? }` is sent on connect/resync. `TerminalLifecycleDelta { base_revision, revision, event }` advances exactly one revision; mismatch requests a lifecycle snapshot independently of screen/history.
- Approved code executing inside the same mutable Fish process can intentionally inspect/replace hooks, so this is accidental-output isolation rather than a sandbox boundary. Missing/tampered hooks stall dispatch and time out closed; hostile same-shell isolation would require a separate privileged shell-control process and is a documented post-V1 hardening limit.
- Lifecycle events are prompt-ready, prompt-editing, command-started, foreground-changed, command-completed, cwd-changed, PTY-EOF, child-exited, and runtime-error. Child wait is exit truth; normal Linux PTY EIO after slave closure records PTY EOF and is not independently fatal.
- One scheduler input completes from the actual correlated lifecycle events it causes, not screen inspection. Later source stays queued through editing, awaiting-start, executing, foreground-interactive, and awaiting-prompt states.

### Resize

- TUI derives rows/columns from the actual Attach body rectangle after header/footer/overlay layout.
- Initial Attach acquisition sends an immediate size.
- The Agent updates emulator geometry and PTY winsize as one serialized runtime operation, then publishes a new geometry generation/full snapshot.
- Opening Browse or the inspector must not resize the child to inspector geometry; the live Attach viewport geometry remains authoritative until Attach layout changes.
- Supported bodies are 1..512 columns, 1..256 rows, and at most 131072 cells. An unsupported initial body remains read-only with `screen_geometry_unsupported`; an oversized later resize retains/renders the prior coherent viewport with a visible error.

### Attach UX

- Default Terminal state is full-width Attach with no inspector.
- The visible surface is a direct projection of emulator cells, including real cursor and alternate screen.
- No TUI-owned input line exists.
- Ordinary keys, Esc, Tab, arrows, function keys, Ctrl-C, paste, and reported mouse events belong to the terminal.
- A host escape mechanism exists behind a dedicated configurable keymap boundary. Its exact binding is intentionally not frozen here.
- There is no separate attach page or attach-before-use ritual.

### Browse UX

- Browse is a temporary host interaction state; only Browse opens the right read-only inspector.
- Left Browse pane is a projection of the authoritative active emulator screen: primary scrollback plus visible rows while primary is active, or the read-only alternate viewport while alternate is active. Public WezTerm APIs do not expose the inactive primary screen.
- Completed normal-screen blocks may be visually folded by omitting projected primary rows and inserting host decoration. Running blocks and alternate-screen content are never folded.
- Browse navigation does not mutate emulator screen state or inject text into the PTY.
- Exiting Browse returns to full-width Attach and restores terminal input routing.

## Non-goals

1. Implementing a custom ANSI/VT emulator.
2. Preserving the current transcript-backed Attach surface or old console DTOs as compatibility aliases.
3. Graphics protocols, inline images, sixel/iTerm image rendering, ligature shaping, or GPU rendering in V1.
4. Persisting shell/emulator state across Agent process restart.
5. Multi-controller simultaneous human input.
6. Unlimited block history or output retention.
7. VT frame recording/replay for interactive programs.
8. Deriving command lifecycle or durable block identity from screen pixels/rows.
9. Stabilizing a permanent host escape chord in this cutover.
10. Replacing `process.exec` with Terminal or routing managed Jobs through the PTY.

## Clean migration boundaries

### Boundary A — emulator/runtime core

- Add the selected WezTerm terminal dependencies and a narrow adapter.
- Feed PTY bytes to emulator and independent semantic observer.
- Introduce explicit lifecycle, serialized writer, and synchronized emulator/PTTY resize.
- Preserve Agent ownership, child process, Fish hooks, and block metadata.

### Boundary B — protocol and manager cutover

- Replace unified `TerminalSnapshot { blocks }` with strict V2 screen/Browse/lifecycle/history/log/lease/response envelopes in `console.rs` and manager APIs.
- Replace string line input with semantic variants, request IDs, lease revision validation, acknowledgements, and no-replay reconnect behavior.
- Add Attach lease/InputArbiter, scheduler/request state, retention stores, and independent resync paths behind `TerminalManager`.
- Migrate console server, client, `main.rs` TUI connection caller, and old protocol tests atomically; no old DTO or translation path remains.

### Boundary C — full Attach cutover

- Replace transcript rendering/local editor with emulator cell rendering, cursor placement, and Attach/Browse/lease state.
- Preserve crossterm Key/Paste/Mouse/Resize events, enable/restore bracketed-paste and mouse capture, and route only the host-keymap chord before Attach forwarding.
- Compute actual supported body geometry and propagate it only through lease-validated resize. Keep existing top-level route/chrome and reconnect entry point.

### Boundary D — Browse/history cutover

- Move selection, folding, and inspector into Browse-only state.
- Add generation-bound Browse screen pages plus block metadata/log pages.
- Remove obsolete transcript-screen snapshots, local editor, and representation-pinning tests. Retain the ANSI-aware line projector only if it is referenced exclusively by bounded inspector/log rendering; otherwise delete it.

### Boundary E — model interface

- Define shared terminal request/result/input/block/log/confirmation DTOs in `agentic-gpt-protocol`; local Agent MCP uses the frozen schema without an agent selector.
- Register/dispatch `terminal.repl` in Agent `stdio_server` through `AppState::terminal`; add `HubCommand::TerminalRepl` and Agent `local_service` dispatch to the same manager methods.
- Register Hub `terminal.repl` with required `agentId` beside the same operation fields, forward the shared request over the existing Agent channel, and return the same bounded result.
- Extend confirmation payload/state with terminal-source kind and immutable digest; reuse current profile rules, confirmation UI, concurrency guards, and structured errors without changing `process.exec`.

## Acceptance criteria

### Runtime/emulator

1. Persistent cwd, environment, functions, and foreground state survive TUI disconnect/reconnect while Agent lives.
2. Split ANSI/OSC/UTF-8 chunks produce the same screen and semantic events as contiguous input.
3. Cursor move/erase, wide Unicode including final-column overflow, colors/style variants, primary scrollback, resize/reflow, and alternate-screen enter/exit render correctly.
4. Emulator-generated terminal replies reach the same PTY writer without byte interleaving.
5. PTY EOF, normal EIO, child exit, and reader/wait failures produce the specified independent lifecycle facts.
6. Kitty is disabled; over-limit unterminated OSC/APC/DCS/Sixel input is bounded/discarded with parser recovery, while sub-limit Sixel/iTerm serializes no image data and produces the explicit unsupported warning/projection.

### Attach

7. Terminal opens full width without inspector or artificial input line.
8. Printable input, arrows, Tab, Esc, Ctrl-C, function keys, bracketed paste, and X10/VT200/SGR-cell mouse reporting reach foreground applications with mode-correct encoding; unsupported SGR-Pixels is rejected.
9. `vim` or an equivalent alternate-screen app opens, resizes, accepts input, exits, and restores primary screen/scrollback.
10. For supported body sizes, actual Attach dimensions equal emulator and PTY dimensions initially and after resize.
11. Unsupported initial or later geometry yields `screen_geometry_unsupported`, sends no mismatched PTY resize, and retains/displays only a coherent prior viewport.
12. Reconnect receives a full current screen/cursor without replaying block transcripts or any unacknowledged Human input.
13. Two clients cannot both hold Attach; denial/loss is read-only, token reconnect succeeds only within two seconds, and grace expiry unblocks queued Agent source.
14. The provisional host escape enters Browse, is not sent to the PTY, explicitly releases the lease, and returning to Attach waits for reacquisition before forwarding ordinary keys.

### Browse/history

15. Browse alone opens the inspector and consumes host navigation keys.
16. Returning to Attach restores full width and ordinary key forwarding.
17. Primary scrollback pages by opaque generation-bound cursor without changing PTY size; expired/reflowed cursors resync, and alternate-screen Browse exposes only the current alternate viewport.
18. Completed long normal-screen blocks can fold in Browse without changing emulator state; running/alternate-screen blocks do not fold.
19. Block metadata updates append/incrementally update without full-history retransmission.
20. Inspector pages through the complete retained bounded log by block ID.
21. More than sixteen 64 KiB historical blocks cannot overflow screen transport or cause reconnect loops.

### Semantics/arbitration

22. `false` yields one completed block with exit code 1; consecutive submitted sources remain separate blocks with no output leakage.
23. Multiline Fish source reaches Fish as one bracketed semantic submission without physical-newline splitting or continuation guessing.
24. Python REPL/foreground interactive input remains one running block until shell return; targeted expressions create no fake shell blocks.
25. Agent source/resize/foreground input queue while Human Attach is held and dispatch FIFO only after release plus their respective safe-state/target checks.
26. Human input, Agent source, and emulator replies never byte-interleave; stale targeted foreground input never reaches a later prompt.
27. One Submit may produce zero/one/many actual block results; an active result is later observable through Status.
28. Normal command output is available in bounded inline results and LogPage without screen inspection; block/request pagination retrieves every retained result.
29. Per-input policy denial/confirmation, full-source digest staleness, request Cancel, and deadline races stop only undispatched work and never force-kill a running foreground process.
30. A foreground program emitting fake OSC 7/133/1337 cannot change lifecycle, cwd, block boundaries, or release queued scheduler input without a matching authenticated FIFO+boundary pair.

### Quality/security

31. Console endpoint preserves same-UID validation, private directory/socket/FIFO permissions, version negotiation, lease/request IDs, acknowledgements, no input replay, and frame bounds.
32. Screen/Browse/history/lifecycle/log payloads have explicit size/retention bounds and independent malformed/revision-mismatch recovery.
33. Focused tests, affected crate checks, strict clippy, rustfmt, diff check, and direct TTY smoke pass.

## Blocking decisions

| Decision | State | Resolution |
|---|---|---|
| WezTerm dependency/toolchain/parser memory | closed | Pin term rev `78cd82db…`; vendor same-rev parser crates with 1 MiB string-control caps; commit lock; require stable Rust 1.85+ and first-build proof. |
| Graphics/image feature boundary | closed | Disable Kitty; cap input sequences; accept sub-limit internal Sixel/iTerm parsing; emit no image DTO/bytes and project unsupported cells. |
| Screen DTO/color/attribute representation | closed | Exact table-or-inline styles/cells, explicit Ratatui degradation, and atomic row-chunk transfers. |
| Host escape binding | closed | Smoke-select a provisional binding behind `TerminalHostKeymap`; binding itself remains intentionally non-stable. |
| Attach session/lease/reconnect | closed | Strict V2 hello, connection/request IDs, lease revision+resume token, two-second grace, acknowledgements, and no mutation replay. |
| Block retention/page sizes | closed | 256 blocks, 8 MiB aggregate, 64 KiB/block, 50/100 metadata pages, 32 KiB/256-line log pages. |
| Browse inactive-primary limitation | closed | Page primary only while active; alternate Browse exposes only current alternate viewport; never copy inactive screen. |
| `terminal.repl` schema/policy/results | closed | Submit/status/cancel/log operations, bounded output/results, FIFO requests, static allowlist, digest-bound confirmation, local+Hub ingress. |
| Screen frame/geometry/style bounds | closed | 960 KiB frames; 512x256/131072 cells, 1024-entry dedup table with exact inline overflow, 64 MiB staged transfer. |

## Implementation dependency graph

### Completed planning gate

1. Blocker evidence and contracts are frozen in this directory.
2. Independent consistency/readiness review must pass before changing State from `planning` to `implementation_ready`.

### Runtime foundation — serial gate

3. Add pinned `wezterm-term`, vendor the same-revision `wezterm-escape-parser`/`vtparse` with only the frozen 1 MiB accumulation/recovery patch, preserve licenses, add Cargo overrides, commit lock resolution, run parser recovery tests, and immediately build the affected crate for Rust/toolchain/dependency proof.
4. Add `terminal_screen.rs` as the narrow WezTerm adapter: Agent configuration, response-writer channel, screen extraction, table-or-inline exact styles, Full/Delta chunk building, Browse pages, mode-aware input, and synchronized resize. Its tests own split chunks, style/color/wide-cell/cursor/title/graphics bounds, alt screen, reflow, and malformed transfer cases.
5. Rewrite `terminal_runtime.rs` as the sole PTY/child/raw-byte owner: Fish environment/hooks, ordered emulator+semantic fanout, marker slicing, serialized writes, independent EOF/EIO/child-wait facts, foreground-pgrp observation, and emulator+PTY resize.
6. Rewrite `terminal_manager.rs` as the sole actor/state owner: lifecycle reducer, TerminalBlock/log retention, request store, just-in-time admission/confirmation, InputArbiter/FIFO scheduler, Attach lease/grace, and screen/history/lifecycle publication. Extend `TerminalOrigin` with Agent and migrate every match.

Steps 4 and 5 may be implemented concurrently only after their event/writer/resize interface is fixed; step 6 integrates both and is the serialization point.

### Private console cutover — atomic boundary

7. Replace private DTOs and framing in `console.rs` with V2 hello/subscribe/screen/Browse/lifecycle/history/log/lease/response envelopes and 960 KiB pre-write checks.
8. Replace reconnect write buffering with explicit idempotent retry versus non-replayed mutation handling; implement request acknowledgements, lease resume, and independent resync.
9. Migrate `main.rs` client creation plus every console test/caller in the same boundary. Delete old Snapshot/Input/Resize DTOs; no adapter or alias survives.

Steps 7–9 have one integration owner because partial server/client migration cannot run.

### Attach and Browse UI

10. In `tui/runtime.rs`, preserve Key/Paste/Mouse/Resize, enable and reverse bracketed-paste/mouse capture, and keep panic-safe terminal restoration.
11. In `tui/terminal.rs`, replace local line input/transcript surface with staged screen cache, atomic Full/Delta apply, ratatui cell/cursor renderer, Attach lease state, Browse screen/history/log caches, and bounded inspector rendering.
12. In `tui/workspace.rs`, centralize `TerminalHostKeymap`, make Attach/Browse routing explicit, release/reacquire lease around Browse, and prevent global Process shortcuts from stealing ordinary Attach keys.
13. Delete obsolete transcript-screen snapshots/renderer code/tests unless the ANSI projector remains exclusively used by bounded inspector logs. Run direct non-tmux and tmux TTY smokes before continuing.

Steps 10 and the screen-cache portion of 11 may proceed concurrently; step 12 and final event routing serialize their integration.

### Public model interface

14. In `agentic-gpt-protocol`, add shared terminal input/request/result/block/log DTOs, `HubCommand::TerminalRepl`, and digest-bearing terminal confirmation fields with strict serde bounds.
15. After step 14, implement Agent local MCP registration/dispatch in `stdio_server.rs` and Hub-command dispatch in `local_service.rs` to the same `AppState::terminal` methods.
16. Concurrently after step 14, implement Hub `terminal.repl` registration/schema/forwarding in `agentic-gpt-hub/src/mcp_server.rs`; Hub requires `agentId`, local MCP does not.
17. Integrate terminal-source confirmation UI/state in `confirmation.rs` and any existing confirmation consumers without changing `process.exec` semantics.

### Verification and cleanup

18. Run all 33 acceptance scenarios with focused unit/integration tests and real Fish/PTY/TUI smokes; preserve exact evidence in `progress.md`.
19. Run affected crate checks/tests, strict clippy, rustfmt, and diff check once after integration.
20. Remove superseded code, tests, temporary harnesses, and planning-only smoke artifacts; report remaining V1 limitations exactly.
