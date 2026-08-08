# Phase 11 Repair Scoped Re-review

- Reviewed HEAD: `5d019775d55e0767288a5933b15f689698b8cc83`
- Repair range: `f62bef7..5d01977`
- Review date: 2026-08-08
- Review mode: read-only scoped re-review. No product source, tests, or existing review artifact was changed by this review.

## Verification evidence

- `cargo test -p agentic-gpt --bin agentic-gpt -- --test-threads=1`: 317 passed.
- `cargo test -p agentic-gpt --test config_cli -- --test-threads=1`: 14 passed.
- Configuration/TUI focused tests (`cargo test -p agentic-gpt config_ -- --test-threads=1`): all configuration and TUI tests passed; the command also reached the known standalone-supervisor runtime-directory prerequisite and was blocked there.
- `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `git diff --check`: passed before the commit.
- Coupling guards remain clean: no Ratatui/Crossterm imports in `config_setup`, no production `inquire`/`PromptBackend`, and no direct config parsing/build calls in `config_tui`.

## Accepted finding dispositions

| Finding | Disposition | Evidence in repair range |
|---|---|---|
| R3-001 | Resolved | `config_setup::review::tests::review_preserves_pending_actions_and_redacted_standalone_reference` covers deferred/immediate standalone provisioning and Hub pending actions. |
| R10-001 | Resolved | `tui/runtime.rs` marks `TUI_ACTIVE` immediately after raw-mode entry and routes partial entry, terminal construction, and clear failures through full primitive/terminal restoration. |
| R10-002 | Resolved | The source row is visibly selected; switching File to Environment clears file-only provisioning and the staged secret, with a regression test covering the transition. |
| R10-003 | Resolved | Connection and optional forms allocate an error row beside every invalid field, render localized safe messages, and retain first-error focus; the validation placement test covers the focused field. |
| R10-004 | Resolved | Basic now exposes the available mode/profile choices in compact hint rows while retaining the current selection and cycling behavior. |
| R10-005 | Resolved | Review records group line positions and applies a focused-group-aware bounded vertical scroll; the lower-group visibility test verifies the marker remains visible. |
| R10-006 | Resolved | `set_system_error` recognizes the rollback marker before splitting compound errors and preserves `config_init_secret_rollback_failed`; a regression test covers the compound message. |
| R10-007 | Resolved | Standalone Review always includes a normalized `file:<path>` or `env:<name>` reference, while secret values remain absent; the domain review test covers deferred references. |
| R10-008 | Resolved | Terminal entry/draw/event/handler failures map to a localized terminal-error fallback, set the blocking SystemError state when the surface remains drawable, and avoid returning implementation-level I/O text. |
| R10-009 | Partially resolved | The repair covers lexical aliases and existing-file canonical identities, but the follow-up review found that a symlinked parent with a nonexistent final target still bypasses the whole-path canonicalization check. |
| R10-010 | Resolved | Basic, Connection, Optional Center/forms, Review, Completion, placeholder, and SystemError copy route through the language selector; validation codes are rendered as localized safe messages. Chinese Basic-surface coverage is present. |

## Remaining advisory limitations

- The Phase 9 smoke limitation for synthesizing a physical CapsLock key remains environmental; source-level Esc semantics and focused tests cover the intended cancellation behavior.
- Review still renders the complete summary and scrolls it; a future compact-summary mode could improve density but is not required for this repair acceptance.
- The known standalone-supervisor integration prerequisite (`runtime_directory_unavailable`) remains external to this repair and does not indicate a product regression.

## Conclusion

All accepted Phase 10 findings except the newly reproduced R10-009 symlink-parent variant have a corresponding repair and focused regression evidence in `5d01977`. R10-009 remains open for the bounded follow-up repair; this artifact must not be read as a full resolution of that finding.

## Follow-up review note — 2026-08-08

An independent re-review at the same HEAD reproduced the missing-parent-identity case:

- `<root>/alias -> real/`;
- config path `<root>/alias/config.json`;
- secret path `<root>/real/config.json`;
- both final files initially absent.

The complete-path `fs::canonicalize()` calls fail before either file exists. The exact regression test was added and observed red before the follow-up implementation. The required repair is to canonicalize the deepest existing ancestor and append the missing components before comparing identities.
