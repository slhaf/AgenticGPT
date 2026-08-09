# Findings & Decisions

## Requirements
- Review v2 should be a final inspection/edit surface, not merely a set of group links back into the wizard.
- Focus concrete fields row-by-row; group headings are visual structure.
- Edit supported fields inline and remain on Review after successful edits.
- Preserve final `Confirm and write`, cancel/no-side-effect behavior, secret redaction, and staged-session semantics.
- Add `/` search and `n` / `N` match navigation.
- Reuse the semantic Inspector copy completed immediately before this task.
- Do not add TDD/regression tests for style/layout/display-only behavior.

## Research Findings
- Current `ReviewItem` has only `label_key` and `value`; it cannot identify a `SetupField` or editor behavior.
- Current Review focus count is `review_group_count() + 1`, so focus is group-level plus final confirm.
- `activate_review_focus()` maps group focus to `open_review_target()`, which navigates to Basic/Connection/Optional pages.
- Current `render_review()` renders group headings with `›` focus and indented non-focusable item rows.
- `SetupSession` already owns staged drafts, validation, Review regeneration, and final outcome/commit semantics; Review v2 should not create a second source of config state.
- Existing Form Kit supports text input, enum choices, list editing, MCP compound editing, and Inspector semantics that can be reused.
- MCP Review currently serializes each server to a formatted `id · enabled · transport · endpoint` string, so Review v2 needs explicit dynamic target metadata instead of parsing that string.
- Phase 1 contract now uses `ReviewItem.field: Option<SetupField>`, `ReviewEditorKind`, and `ReviewItemTarget`; MCP rows carry `McpServer { index }` and do not pretend to be a single scalar field.
- Standalone secret-reference rows map to the active concrete field (`TunnelSecretPath` or `TunnelSecretEnvironment`) while keeping the displayed reference safe.
- `MaxActiveJobs` remains explicit `Fallback` for now because its existing UI is hybrid Auto/Custom rather than a plain scalar/choice editor.
- `ReviewModel` already redacts Agent Secret and Tunnel secret value/reference semantics; this boundary must remain intact.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Extend Review row contract with `SetupField` and editor/fallback metadata | Search, Inspector, focus, and editing can all use the same stable domain contract. |
| Keep `label_key` display-only | Prevents frontend business logic from depending on localization/display strings. |
| Treat MCP server as a compound row with explicit dynamic identity | Maintains stable editing without parsing formatted values. |
| Refresh ReviewModel after each committed inline edit | Keeps displayed values, pending actions, and validation-derived state authoritative. |
| Preserve logical focus across refresh | Avoids user losing position after editing. |
| Search only safe Review text | Secret plaintext and transaction-only buffers remain outside render/search/debug surfaces. |
| Reuse `EditState` for Review primitive editing | It already redacts Debug output and carries field/buffer/cursor; Review can use the current row as the logical anchor instead of duplicating text state. |
| Commit Optional scalar edits through a cloned section draft + `save_optional_section` | Validation happens before replacing the authoritative staged section, so invalid Review edits cannot overwrite the confirmed value. |
| Seed secret edits from `SetupSession`, never from Review display text | Review intentionally contains `[REDACTED]`; existing `connection_value` safely exposes the in-memory staged secret only to the editor buffer. |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Temporary Superpowers-style Review v2 spec was started before choosing PWF | Treat PWF files as the only active plan; remove the untracked temporary spec. |
| PWF init shell script lacks executable bit for `skills.run` | Invoke installed script with `sh`; continue using skill resources/scripts where executable. |

## Resources
- `crates/agentic-gpt/src/config_setup/review.rs`
- `crates/agentic-gpt/src/config_setup/model.rs`
- `crates/agentic-gpt/src/config_setup/validation.rs`
- `crates/agentic-gpt/src/config_tui/app.rs`
- `crates/agentic-gpt/src/config_tui/pages.rs`
- `crates/agentic-gpt/src/config_tui/input.rs`
- `crates/agentic-gpt/src/tui/widgets.rs`
