# Phase 10 Independent Staged Review

- Reviewed HEAD: `f62bef7fe11efcfc037829625c880e1a87065dee`
- Reviewed product range: `1721ec9..f62bef7`
- Per-phase ranges: Phase 4 `6a1ee0a..1721ec9`; Phase 5 `1721ec9..ac5cab1`; Phase 6 `ac5cab1..73fc5de`; Phase 7 `73fc5de..8d820e4`; Phase 8 `8d820e4..80daae3`; Phase 9 `80daae3..f62bef7`.
- Review date: 2026-08-08
- Review mode: read-only. No product code, tests, documentation, plan authority file, or existing review artifact was changed during this review. This file is the only permitted write for Phase 10.

## Review evidence

- Read the approved design at `docs/superpowers/specs/2026-08-07-config-init-fullscreen-tui-design.md`, the frozen `task_plan.md`, `progress.md`, `findings.md`, and the Phase 3 artifact.
- Verified the phase commit boundaries and inspected the complete Phase 4-9 diff, current source, tests, and documentation.
- Recorded verification evidence from the implementation pass: focused TUI/domain tests, 310 binary unit tests, 14 `config_cli` integration tests, workspace check, formatting, clippy with `-D warnings`, diff checks, coupling guards, and disposable tmux/PTY smoke. The full crate test command remains environment-blocked at the known `local_control` socket prerequisite; this is recorded in `progress.md` and is not treated as a product finding here.
- Coupling searches remain clean: `config_setup` contains no Ratatui/Crossterm/TUI navigation types; production code contains no `inquire`/`PromptBackend` path; `config_tui` contains no direct config parsing/build calls outside its `config_setup` use-case boundary.
- The Phase 3 advisory `R3-001` (direct pending-action regression coverage) remains relevant to the later Phase 11 disposition, but it is not duplicated as a new Phase 10 product finding.

## Phase 4 — TUI runtime/widgets

### R10-001 — Partial terminal-entry failure can leave alternate screen active

- Reviewed phase: Phase 4 (`6a1ee0a..1721ec9`)
- Severity: high
- Category: terminal safety / error-path restoration
- Evidence: `crates/agentic-gpt/src/tui/runtime.rs:37-41` enables raw mode and executes `EnterAlternateScreen, Hide` together. If that command reports an error after the alternate-screen escape has already been processed, the error path calls only `disable_raw_mode()` and never attempts `LeaveAlternateScreen`. `TUI_ACTIVE` is set only at line 56, after `Terminal::new` and `terminal.clear()`; therefore setup failures before that line also bypass the panic hook's `restore_primitives()` branch.
- Expected contract: design §15.3-15.4 requires best-effort restoration on every propagated setup/error path and panic cleanup of raw mode, alternate screen, and cursor.
- Impact: a partial write failure during setup can strand the user in the alternate screen or leave cursor state inconsistent even though `TerminalSession::enter()` returns an error. This is exactly the class of terminal corruption the runtime boundary is meant to prevent.
- Suggested direction: make the setup guard active immediately after raw-mode/alternate-screen entry, route every partial-entry error through the same full primitive restoration (including `LeaveAlternateScreen` and `Show`), and add an injectable/focused failure-seam test for partial setup.

## Phase 5 — navigation/editing/basic/connection

### R10-002 — Standalone Secret source selector renders no selected state and can retain a hidden invalid provision flag

- Reviewed phase: Phase 5 (`1721ec9..ac5cab1`)
- Severity: medium
- Category: connection-flow correctness / hidden staged state
- Evidence: `crates/agentic-gpt/src/config_tui/pages.rs:152-163` renders both `TunnelSecretSource` and `ProvisionTunnelSecret` through `render_radio_row` with `selected = value == "true"`. The source values are `"file"` or `"env"`, so neither source is ever shown selected. In `crates/agentic-gpt/src/config_tui/app.rs:428-442`, switching File → Environment changes only `secret_source`; it leaves `provision_secret_now` unchanged. The dynamic field list at `pages.rs:192-205` then hides the provision controls, while `validation.rs:276-292` still rejects `provision_secret_now == true` for an environment source and reports a missing secret value.
- Expected contract: the active source must be visibly selected, dependent fields must remain semantically valid when the source changes, and a hidden field must not silently make the page impossible to finish.
- Impact: users cannot tell which source is active; a valid File+provision draft can become an invalid Environment draft with no visible way to disable the now-hidden provision option. Completion requires an unintuitive switch-back sequence and risks confusing validation errors.
- Suggested direction: render source selection using `value == "file"` (or explicit source labels), and when switching to Environment either clear/drop the file-only provision draft or keep a visible, valid dependent control. Add a regression test for source toggling with provision enabled.

### R10-003 — Form errors are not inline, deterministic, or attached to the focused field

- Reviewed phase: Phase 5 (`1721ec9..ac5cab1`)
- Severity: medium
- Category: validation UX
- Evidence: `ConfigTuiApp::record_errors` at `crates/agentic-gpt/src/config_tui/app.rs:527-535` focuses the first structured error but stores all messages in a `HashMap`. Connection and optional pages at `crates/agentic-gpt/src/config_tui/pages.rs:180-182` and `391-393` render only `errors.iter().next()` once in the final body row. HashMap iteration order is not the validation order, and the row is not the field that received focus.
- Expected contract: design §11 requires ordinary validation errors to be inline, to leave the page in place, and to focus the first error field with an immediately understandable local message.
- Impact: when multiple fields are invalid, the UI can show a message for a different field than the focused one, below unrelated controls, or omit other errors entirely. This makes correction especially difficult on the dynamic connection and multi-field optional forms.
- Suggested direction: render an error row associated with each field (or a deterministic first-error row immediately under the focused field), preserve validation order, and test multiple simultaneous errors plus focus/error alignment.

### R10-004 — Basic mode/profile choices are hidden behind enum-cycling rows

- Reviewed phase: Phase 5 (`1721ec9..ac5cab1`)
- Severity: low
- Category: basic-page usability
- Evidence: `crates/agentic-gpt/src/config_tui/pages.rs:84-106` renders one `Runtime mode {current:?}` row and one current profile row, both always marked selected. `ConfigTuiApp::activate_focus` at `crates/agentic-gpt/src/config_tui/app.rs:410-426` cycles the closed enums on Enter, but the page never presents the available Standalone/Hub/Local or Normal/Room choices or their short explanations required by design §8.1.
- Expected contract: Basic should be a grouped mode/profile choice surface where the available choices and current selection are legible without knowing an undocumented cycling order.
- Impact: users can change the value only by repeatedly pressing Enter and cannot see the alternatives before committing to a different mode/profile. This weakens the central page's discoverability and makes the dynamic flow feel opaque.
- Suggested direction: render explicit option rows (or an equally clear compact choice list) and map focus/activation to those options; add TestBackend assertions for all available choices and the selected marker.

## Phase 6 — optional configuration center

No additional phase-local finding was identified beyond the shared form/error and localization findings R10-003 and R10-010. The center's applicable-section filtering, disabled NotApplicable rows, re-entry, staged save/discard, and mode/profile status behavior were covered by focused state and TestBackend tests. The localization and review-overflow findings below still affect optional-center users.

## Phase 7 — Review/return editing/commit/completion/errors

### R10-005 — Review content is clipped with no scroll or focus visibility for lower groups

- Reviewed phase: Phase 7 (`73fc5de..8d820e4`)
- Severity: medium
- Category: Review navigation / usability
- Evidence: `TuiState.scroll` is an unused field at `crates/agentic-gpt/src/config_tui/app.rs:20-32`; no event path updates it. `render_review` at `crates/agentic-gpt/src/config_tui/pages.rs:661-760` builds every item for every applicable optional group into one `Paragraph` with a fixed body area and no scroll offset. At normal 90x28 dimensions, a Standalone Review with connection, pending actions, and the seven applicable optional groups produces roughly forty lines while the body has about twenty-three rows. `review_targets` still cycles through the clipped groups, but their focus marker is not visible.
- Expected contract: Review must be a navigable page that can jump to every applicable group and return directly to editing; small-terminal layout may clip text, but it must not make valid targets undiscoverable.
- Impact: lower Limits/Sandbox/Tunnel/Hub groups are not inspectable or visibly focusable in common terminal sizes. Users may press Enter blindly or miss required Review edits, undermining the core Review return-editing feature.
- Suggested direction: either implement a focused-group-aware scroll offset (including keyboard navigation) or render a compact, scrollable group summary; add a normal-size TestBackend test that focuses every group and verifies its marker/label remains visible.

### R10-006 — Commit/rollback error classification drops the rollback-failure signal

- Reviewed phase: Phase 7 (`73fc5de..8d820e4`)
- Severity: medium
- Category: system-error semantics / recovery safety
- Evidence: `commit_wizard_outcome` returns `"config_init_config_write_failed: config_init_secret_rollback_failed"` when config writing and secret rollback both fail (`crates/agentic-gpt/src/config_setup/outcome.rs:147-161`). `ConfigTuiApp::set_system_error` at `crates/agentic-gpt/src/config_tui/app.rs:734-754` first splits at `:` and maps only the first token, so the combined error is classified as `config_init_config_write_failed`. Although `system_error_message` contains a dedicated rollback message at `app.rs:824-849`, that branch is unreachable for the compound error.
- Expected contract: design §15.2 requires a blocking safe error surface that distinguishes unrecovered rollback/system failure from an ordinary config-write failure without exposing secret bytes.
- Impact: the user is told only that config writing failed and is not warned that secret rollback may have failed, even though the filesystem may require manual inspection. Re-running or assuming the old secret state was restored can be unsafe.
- Suggested direction: preserve a structured error code (or match the rollback marker before the first-colon split), render the rollback-safe message, and add a focused test for the compound error mapping and post-error return code.

### R10-007 — Standalone Review omits the selected secret path/environment reference when no immediate write is planned

- Reviewed phase: Phase 7 (`73fc5de..8d820e4`)
- Severity: medium
- Category: Review completeness / secret-safe transparency
- Evidence: `crates/agentic-gpt/src/config_setup/review.rs:236-280` includes only `tunnel_id` and `tunnel_secret_source` for Standalone. The path is added to `ReviewSecretWrite` only when `provision_secret_now` is true (`review.rs:185-201`). A deferred `file:` reference or `env:NAME` therefore appears in Review only as `file`/`env`, with neither the path nor environment-variable name available for verification.
- Expected contract: design §13.2 requires current-mode key connection information, while still redacting secret contents; a reference/path is safe metadata and is shown in the design's Review example.
- Impact: users cannot catch a typo in the secret file path or environment variable before final confirmation. The resulting config can point to the wrong secret source while the Review appears complete.
- Suggested direction: expose a non-secret redacted reference (`file:<path>` or `env:<name>`) in the frontend-neutral `ReviewModel` for all active Standalone setups, and add assertions for deferred file/env references while keeping secret bytes absent.

### R10-008 — Terminal setup/draw/event errors bypass the blocking system-error page

- Reviewed phase: Phase 7 (`73fc5de..8d820e4`) and Phase 4 runtime integration
- Severity: medium
- Category: system-error handling
- Evidence: `crates/agentic-gpt/src/config_tui/mod.rs:26-42` propagates `TerminalSession::enter`, `draw`, and `next_event` failures directly with `?`. Only commit failures reach `ConfigTuiApp::set_system_error` (`app.rs:713-722`). A terminal setup/clear/event failure therefore never renders the localized blocking error page and may return raw `tui_*`/I/O text after restoration (or before the app has a drawable surface).
- Expected contract: design §15.2 lists terminal setup/restore anomalies among blocking system errors and requires safe, non-secret user-facing handling. For pre-screen failures, the fallback must at least be safe/localized and fully restore the terminal; post-entry failures should use the same blocking surface where drawing remains possible.
- Impact: terminal failures have inconsistent UX and can expose implementation-level error text instead of the intended localized error code/message. The user cannot distinguish a recoverable setup issue from a config validation failure through the same error contract.
- Suggested direction: add a safe terminal-error mapping/fallback around setup and event-loop failures, keep full restoration on all paths, and test both pre-entry and post-entry failure seams without requiring a real broken terminal.

## Phase 8 — CLI migration and `inquire` removal

No additional finding was identified. The three-stream TTY gate, explicit `--non-interactive` route, editable seed conversion, no-wait bare non-TTY error, removal of `inquire`/PromptBackend production references, and updated repository automation call sites match the frozen CLI contract. The known runtime-directory/socket integration failures are environment prerequisites recorded in `progress.md`, not regressions in this phase.

## Phase 9 — terminal smoke, documentation, hardening, and verification

No new phase-local finding was identified. The smoke evidence confirms alternate-screen use, cancellation restoration, live editing, resize state retention, secret non-echo, and successful commit behavior for the exercised paths. The independent findings above show cases not covered by that smoke matrix: injected partial terminal setup failure, source-dependent hidden state, large Review rendering, localized/non-localized page copy, and syntactically aliased config/secret targets.

## Cross-phase integration — coupling, secret safety, restoration, compatibility, and regression risk

### R10-009 — Syntactically aliased config and secret paths bypass the collision guard

- Reviewed phase: Phase 7 outcome/commit handoff, exercised by Phase 8 CLI routing and Phase 9 smoke (`73fc5de..f62bef7`)
- Severity: high
- Category: secret safety / transactional write boundary
- Evidence: `crates/agentic-gpt/src/config_setup/outcome.rs:136-139` rejects only `target == config_path`. `target` comes from `exec::expand_pathbuf`, which expands `~` but does not normalize relative `./` components or make paths absolute (`crates/agentic-gpt/src/exec.rs:216-224`); the caller's `config_path` is kept in its original CLI spelling. The secret-target validator rejects `CurDir`/`ParentDir` components on the secret path, but no equivalent identity check is applied to the config path. Thus `--config ./config.json` with a provision target `config.json` compares unequal while referring to the same filesystem file.
- Expected contract: no secret may be written to the config/backup target, and final commit must preserve the existing safe secret/config boundary for all accepted path spellings.
- Impact: with an existing config, the secret write can replace that file; `write_config_with_backup("./config.json", ...)` then copies the secret bytes into the normal config backup before writing the config. This can leave secret material in a backup file and defeats the no-secret-in-config guarantee. With a new config, the secret target is still briefly used as the config target and can inherit the wrong permissions.
- Suggested direction: compare canonical/lexically normalized file identities for both config and secret targets before any write, including relative aliases and existing/non-existing paths; reject a collision before creating the secret parent or target. Add a regression test using `./config.json` versus `config.json` that asserts no config/backup contains the secret marker.

### R10-010 — The fullscreen frontend is only partially bilingual

- Reviewed phase: Phase 5-9 frontend and documentation range (`1721ec9..f62bef7`)
- Severity: medium
- Category: localization / primary UX contract
- Evidence: the frozen goal calls for a bilingual fullscreen flow, but `crates/agentic-gpt/src/config_tui/pages.rs:62-760` hardcodes English titles, field labels, statuses, action labels, Review text, and footers. `render_system_error` at `pages.rs:837-860` also hardcodes its title/code/footer. `UiLanguage` is used for completion and the system-error message payload, but `ConfigTuiApp::record_errors` at `crates/agentic-gpt/src/config_tui/app.rs:527-535` stores raw validation codes such as `config_init_number_invalid` for display. The implementation tests create `UiLanguage::ZhCn` but do not assert localized page output.
- Expected contract: design §1/§2 and the plan goal require the interactive setup surface to honor the selected English/Chinese UI language; user-facing validation and status copy should be localized/safe rather than internal codes.
- Impact: `--language zh-CN config init` presents an otherwise Chinese CLI with mostly English setup pages and raw internal validation identifiers, so the promised bilingual experience is not delivered and error correction is needlessly opaque.
- Suggested direction: route all page/field/status/footer/action copy through a small language-keyed copy table, localize structured validation codes at the frontend boundary, and add Chinese TestBackend coverage for Basic, Connection, Optional Center, Review, completion, and system-error surfaces.

## Suggestions and advisory carry-over

- `R3-001` remains an advisory test-gap: add direct pending-action assertions for deferred and immediate secret provisioning during Phase 11 adjudication.
- The Phase 9 tmux smoke could not synthesize a physical CapsLock key independently of the environment's keyd mapping; source-level Esc semantics were verified, but this remains a documented smoke limitation rather than a product defect.
- Once Review scrolling is repaired, consider a compact summary mode so the page does not need to render every optional field to support return-edit navigation.

## Review conclusion

The core separation, CLI migration, staged-write boundary, secret redaction, and exercised terminal flows are present and the cross-phase coupling guards are clean. The implementation is not ready for the final acceptance gate without adjudicating the high-severity terminal/path-safety findings and the medium-severity source-state, validation, Review, error-surface, and localization findings above. Phase 10 itself is complete as a read-only review; Phase 11 should record a disposition for every `R10-*` item before any repair.
