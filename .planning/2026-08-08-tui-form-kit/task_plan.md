# Task Plan: Reusable TUI Form Kit

## Goal

Extract the frozen config-init form language into reusable `tui` components, then migrate the affected config pages onto those components without changing `config_setup` business contracts.

## Frozen UX rules

- `❯` means keyboard focus only.
- `` is used only for committed selection in a real enum/choice list.
- Boolean rows never show ``/check marks; the rendered value (`开`/`关`, `on`/`off`) is the state.
- Text input uses the demo long-form grammar: `◆ label` on one line, then `❯ [value]` on the next line.
- Empty text inputs render as an empty box/space, never `•`.
- The transient `█` cursor is allowed only while actively editing and disappears after commit/cancel.
- Internal subsections use a Nerd Font diamond heading (`◆`) plus vertical spacing; only the outer section keeps the Labeled horizontal rule.
- Enum choices such as tunnel secret source and Limits Auto/Custom are visible lists, not single-row Enter-to-cycle controls.
- Workspace path arrays are edited as real item lists; JSON remains an adapter/storage detail and is never typed by the user.
- Existing config defaults remain authoritative. Do not invent values for optional Tunnel Client fields.
- Review, Completion, SystemError, and `config_setup` contracts are out of scope.

## Architecture

`tui/forms` owns frontend-neutral form rendering/state mechanics. It must not know `SetupField`, `OptionalSection`, Tunnel, Workspace, or Limits.

`config_tui` owns adapters between existing setup drafts and the generic form components. Serialization such as Workspace path arrays and `max_active_jobs = "auto" | number` stays here.

## Phases

### Phase 1 — Form Kit primitives

Create `crates/agentic-gpt/src/tui/forms/` with reusable primitives and focused TestBackend tests:

- subsection heading (`◆`)
- long-form text input with dynamic width and transient edit cursor
- boolean value row with no selected marker
- choice-list row/list with separate focus and committed selection
- editable list state/rendering primitives suitable for string/path lists
- shared action/footer integration where useful

Keep the existing Surface palette and `NO_COLOR` behavior. Do not migrate config pages in this phase except tiny compile-compatible exports.

**Commit:** `feat(tui): add reusable form primitives`

### Phase 2 — Connection controls

Migrate Standalone/Hub Connection fields to the Form Kit:

- Tunnel secret source becomes a visible `file` / `env` choice list.
- Provision secret remains a boolean row and has no check/selection marker.
- Text inputs use the long-form field grammar; empty values have no symbol placeholder.
- Preserve dynamic field visibility, secret masking, validation, staged values, and literal h/j/k/l while editing.

**Commit:** `refactor(config-tui): use form kit for connection`

### Phase 3 — Workspace list editor

Replace JSON text entry for `write_roots`, `read_only_roots`, and `deny_roots` with a generic list editor:

- `j/k` moves between visible items in browse mode.
- Enter edits the focused item.
- `a` adds an item and enters editing.
- `d` deletes the focused item.
- Empty list is visually empty/actionable rather than displaying JSON.
- Adapter parses existing JSON draft strings into items and serializes items back to the unchanged draft fields.
- Invalid pre-existing serialized data must surface through existing validation/error handling rather than panic.

Workspace root remains an ordinary long-form input.

**Commit:** `feat(config-tui): add workspace path list editing`

### Phase 4 — Optional form grammar and Limits

Migrate the remaining Optional forms to the frozen grammar:

- Use `◆` subsection headings and meaningful spacing instead of nested horizontal rules.
- Limits follows the demo: ordinary numeric input for max concurrent tasks; visible Auto/Custom choice for max active jobs with custom numeric input; separate Search subsection and context-lines input.
- Sandbox enabled, Tunnel auto-download, Hub reporting enabled, and other booleans use marker-free boolean rows.
- Confirmation/other real enums use visible choice lists where the domain has a bounded choice set; do not disguise enums as text fields or cycle rows.
- Tunnel Client preserves real defaults: cache dir and auto-download are populated from current setup defaults; version/executable/download URL/SHA-256 remain empty when the model says empty.
- Room and other text/numeric fields use the shared long-form input.

**Commit:** `refactor(config-tui): align optional forms with form kit`

### Phase 5 — Hardening and visual regression coverage

- Run `cargo fmt --all -- --check`.
- Run `cargo check -p agentic-gpt`.
- Run focused `config_tui::` and `tui::` tests serially.
- Run `NO_COLOR=1` focused form/theme tests.
- Run `cargo test -p agentic-gpt --bin agentic-gpt -- --test-threads=1`.
- Run `git diff --check`.
- Remove dead legacy form helpers only when no remaining page uses them.
- Human visual pass: Connection, Workspace, Limits, Sandbox, Tunnel Client in zh-CN at a comfortable terminal size plus one narrow-terminal smoke pass.

**Commit:** only if hardening requires code/test cleanup; otherwise Phase 4 is the final implementation commit.

## Execution status

- Phase 1 complete: `5afa1ea feat(tui): add reusable form primitives`.
- Phase 2 complete: `9873774 refactor(config-tui): use form kit for connection`.
- Phase 3 complete: `226b817 feat(config-tui): add workspace path list editing`.
- Phase 4 complete: `4618efb refactor(config-tui): align optional forms with form kit`.
- Phase 5 automated hardening complete; human visual acceptance remains pending.

Automated Phase 5 evidence:

- `cargo fmt --all -- --check`: pass.
- `cargo check -p agentic-gpt`: pass, no warnings.
- `cargo test -p agentic-gpt config_tui:: --bin agentic-gpt -- --test-threads=1`: 40 passed.
- `cargo test -p agentic-gpt tui:: --bin agentic-gpt -- --test-threads=1`: 54 passed.
- `NO_COLOR=1 cargo test -p agentic-gpt tui:: --bin agentic-gpt -- --test-threads=1`: 54 passed.
- `cargo test -p agentic-gpt --bin agentic-gpt -- --test-threads=1`: 336 passed.
- `git diff --check`: pass.
- `config_setup`, Review, Completion, and SystemError contracts remain untouched by the form-kit migration.

Phase 5 adds regression coverage for inline Auto/Custom focus/selection/editing semantics and for keeping the primary action visible across all applicable Optional forms in a narrow terminal.

### Human-pass follow-up

The first human visual pass identified four polish issues and they were addressed without changing `config_setup` contracts:

- Optional sections keep an entry snapshot. An unchanged section uses `Return` / `返回` and leaves a default section at `Default`; a changed section uses `Save and return` / `保存并返回`.
- Returning to Optional Center restores the entry that opened the section instead of resetting focus to the first row.
- Optional forms use a fixed bottom action dock separated from the scrolling body by a light structural rule, so long lists cannot visually absorb the primary action.
- Numeric controls use compact five-cell centered inputs; Limits Auto/Custom columns are aligned, and other numeric long-form fields use the same centered numeric grammar where applicable.

Follow-up verification:

- `cargo check -p agentic-gpt`: pass.
- `cargo test -p agentic-gpt config_tui:: --bin agentic-gpt -- --test-threads=1`: 40 passed.
- `cargo test -p agentic-gpt tui:: --bin agentic-gpt -- --test-threads=1`: 54 passed.
- `NO_COLOR=1 cargo test -p agentic-gpt tui:: --bin agentic-gpt -- --test-threads=1`: 54 passed.
- `cargo test -p agentic-gpt --bin agentic-gpt -- --test-threads=1`: 336 passed.
- `git diff --check`: pass.

## Commit convention

Every plan/phase commit uses the repository author and adds:

`Co-authored-by: Heris <heris@slhaf.work>`

Root `task_plan.md`, `findings.md`, and `progress.md` created by the earlier Luna run remain excluded from commits.
