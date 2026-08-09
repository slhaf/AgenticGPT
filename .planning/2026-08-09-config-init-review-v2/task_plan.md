# Task Plan: Config Init Review v2

## Goal
Upgrade `config init` Review from group-level jump-back navigation to a row-level, inspectable and editable final review while preserving staged config, secret redaction, and single-commit semantics.

## Current Phase
Phase 10

## Phases

### Phase 1: Freeze Review Row Contract
- [x] Extend frontend-neutral Review rows with stable `SetupField` identity and explicit editor/fallback metadata.
- [x] Represent dynamic MCP rows without parsing formatted display strings.
- [x] Keep all secret material redacted and out of searchable Review data.
- [x] Preserve existing Review build/validation semantics.
- **Status:** complete

### Phase 2: Row-Level Review Navigation & Layout
- [x] Flatten eligible Review rows into stable focus targets plus the final Confirm action.
- [x] Render section headings as structure, not ordinary focus targets.
- [x] Keep focused row visible while scrolling.
- [x] Add the existing semantic Inspector to Review using row `SetupField` identity.
- [x] Preserve group-page fallback for rows explicitly marked as fallback-only.
- **Status:** complete

### Phase 3: Inline Editing for Primitive Fields
- [x] Add Review-local editor state without changing `ConfigPage::Review`.
- [x] Reuse existing text/secret/choice primitives for scalar fields.
- [x] Enter commits validated staged changes and refreshes Review in place.
- [x] Esc discards only the active edit buffer.
- [x] Validation failure keeps the editor open and leaves the confirmed staged value unchanged.
- **Status:** complete

### Phase 4: Complex Editing & Search
- [x] Reuse list editing for Workspace/path collections and runtime paths.
- [x] Reuse MCP compound editing with stable server identity/index and add/remove behavior.
- [x] Implement `/` search plus `n` / `N` cycling over section labels, field labels, and safe display values.
- [x] Ensure secret plaintext and transaction-only buffers never enter the search corpus.
- **Status:** complete

### Phase 5: Verification & Cleanup
- [x] Run focused behavior tests for Review contract, editing, fallback, search, redaction, cancel, and single commit.
- [x] Run `cargo fmt --all -- --check`, `cargo check -p agentic-gpt`, relevant config setup/TUI behavior tests, and `git diff --check`.
- [x] Do not add dedicated tests for pure spacing, colors, placeholder text, or other presentation-only details.
- [x] Remove obsolete group-level Review machinery that no longer carries a real contract.
- [x] Leave working tree ready for user visual inspection before final commit.
- **Status:** complete

### Phase 6: Visual Inspection Polish
- [x] Match Review section diamonds to the existing Form Kit subsection heading color.
- [x] Keep blue for the active nested child focus; render an entered parent row in neutral emphasis instead.
- [x] Edit scalar text and binary/choice fields directly in the Review row without a second-line editor.
- [x] Keep list values expanded by default and remove explicit `Add path` / `Save changes` action rows.
- [x] Apply list mutations directly to staged config while preserving final disk write only at `Confirm and write`.
- [x] Give Review value boxes a shared default width and the existing truncation/fade behavior.
- [x] Render effective built-in defaults instead of empty boxes where a real default exists.
- [x] Re-run meaningful behavior tests and full verification; no dedicated visual regression tests.
- **Status:** complete

### Phase 7: Review Density & Content Status
- [x] Derive Review optional-section status from current content versus the section default, not merely whether the section was explicitly saved.
- [x] While a complex local draft is open, reflect that draft's content status in the visible section heading.
- [x] Collapse list rows by default; Enter expands the current list and reuses the Phase 6 nested interaction.
- [x] Remove MCP `Delete server` / `Save changes` child rows; use `d` for delete and stage valid edits automatically.
- [x] Keep new invalid MCP drafts local until they become valid; Esc may discard a still-local new server.
- [x] Re-run meaningful Review behavior tests and full verification; no visual regression tests.
- **Status:** complete

### Phase 8: Eliminate Remaining Avoidable Fallbacks
- [x] Remove the duplicate runtime-mode row from the Connection Review group.
- [x] Make Basic Mode and Profile cycle in place from Review; mode changes may alter visible connection/optional rows without leaving Review.
- [x] Allow ReviewModel to represent incomplete-but-safe staged connection state; keep strict validation at final confirm/write.
- [x] Make Tunnel secret source toggle file/env in place while preserving the existing provision-secret cleanup rule.
- [x] Replace Max active jobs fallback with the Limits-page Auto/Custom nested interaction, including inline custom numeric editing.
- [x] Verify no remaining fallback exists; all Review-editable fields now stay on Review or use a nested Review interaction.
- [x] Re-run meaningful Review behavior tests and full verification; no visual regression tests.
- **Status:** complete

### Phase 9: Review Metadata Cleanup
- [x] Remove `Pending action` from the Review header; keep pending actions internal for outcome/commit logic.
- [x] Remove the redundant top-level `Secret write` summary; the connection rows already expose provision intent/path safely.
- [x] Show a backup path only when an existing config will actually be backed up; use the real backup directory and timestamped filename pattern.
- [x] Keep config backup, secret write, and rollback transaction semantics unchanged.
- [x] Run fmt/check/full tests/diff-check; no presentation regression tests.
- **Status:** complete

### Phase 10: Final Preview & Closing Flow
- [x] Clarify Confirmation language copy, use CPU core count wording in max-active auto semantics, fix max-active heading formatting, and rename Limits `Search` subsection to `File search`.
- [x] First Review confirm validates staged config and opens a redacted final JSON preview in the right pane instead of writing immediately.
- [x] While preview is active, freeze left Review navigation; route `j/k` and arrow keys to right-pane vertical scrolling.
- [x] Support `/` search plus `n` / `N` match cycling inside the JSON preview; Esc closes search first, then exits preview back to editable Review.
- [x] Second Enter from preview performs the only disk commit.
- [x] Remove the Completion TUI page; after commit, exit TUI and let the ordinary CLI print completion plus config path.
- [x] Replace the permanent `Add MCP server [+]` Review row with root-level `a` add-MCP interaction and a temporary nested server editor.
- [x] Keep JSON preview secret-safe: redact config secret values and never include transaction-only Tunnel secret plaintext.
- [x] Remove presentation/interaction regression tests that would freeze the evolving TUI; retain only the final single-commit side-effect boundary in Review.
- [x] Run full verification.
- **Status:** complete

## Frozen Interaction Decisions
- Review focus is row-level; section headings are structural only.
- The Review final action first opens a redacted JSON preview; only the second Enter in preview writes to disk.
- Enter edits supported rows inline; structured fields use nested Review interactions rather than navigating back to wizard pages.
- Successful inline edits refresh Review and retain the same logical row when possible.
- In JSON preview, scrolling and `/` search belong to the right pane; Esc returns to editable Review.
- `/` enters Review search outside preview; `n` / `N` cycle matches.
- Inspector reuses existing field semantic copy; Review does not create a second semantic-description source.
- Review/UI code must not infer business fields or edit behavior from `label_key` or formatted display text.

## Scope Boundaries
- No canonical config schema changes.
- No mouse support, undo/redo, diff/history view, or arbitrary JSON editor.
- No change to config backup / secret provision / rollback transaction semantics.
- No visual TDD or dedicated visual regression tests.

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Use PWF only for this change | Scope is medium and bounded; a second planning/spec framework would duplicate state. |
| Put field/editor identity in `config_setup::review` | Keeps Review frontend-neutral and prevents TUI string inference. |
| Reuse existing Form Kit/list/MCP primitives | Avoids a parallel editor implementation and preserves established interaction semantics. |
| Eliminate Review page fallback after nested editors stabilized | Review is now a complete final-edit surface; structured cases stay explicit through typed nested interactions instead of wizard navigation. |

## Errors Encountered
| Error | Resolution |
|-------|------------|
| `skills.run` rejected `scripts/init-session.sh` as `script_not_executable` | Ran the same installed skill script through `sh`; scoped PWF plan initialized successfully. |
