# Findings & Decisions

## Requirements

- Approved product/design authority: `docs/superpowers/specs/2026-08-07-config-init-fullscreen-tui-design.md` at design checkpoint commit `8fb1ba3`.
- The implementation must replace the sequential `inquire` interactive path with fullscreen Ratatui/Crossterm while preserving canonical config construction, validation, non-interactive behavior, secret safety, and transactional commit semantics.
- `task_plan.md` is the sole executable plan for this feature. Its implementation phases use Superpowers `writing-plans` precision, but Superpowers SDD/executing-plans/per-task review orchestration is not part of this workflow.
- Review is explicit and file-backed: reviewers are read-only except for the designated review artifact; findings require Controller adjudication before repair.

## Existing Implementation

- Repository HEAD before implementation is `8fb1ba3` on `main`, matching `origin/main`; no product code has been changed during this planning/refinement pass.
- `crates/agentic-gpt/src/config_cli.rs:967-1045` owns `ConfigInitArgs`, keeps a pure `init_non_interactive`, and currently routes interactive TTY init through `InquirePromptBackend -> run_wizard -> commit_wizard_outcome`.
- `crates/agentic-gpt/src/config_wizard.rs` is 2324 lines and currently mixes terminal prompting, TTY detection, mode/profile selection, optional-section collection, validation/parsing helpers, redacted summary construction, and secure config/secret commit logic.
- `config_wizard.rs:330-620` contains the three-stream TTY predicate, prompt identifiers, `WizardOutcome`, defaults, optional-section applicability, sequential `run_wizard`, and strict current interactive applicability behavior that the new design intentionally changes to editable cross-mode seeds.
- `config_wizard.rs:1-210` defines `PromptRequest`, secret-aware `PromptAnswer`, `PromptBackend`, and the `inquire` adapter. This sequential request/answer model is the architectural limitation identified by the approved design.
- `crates/agentic-gpt/Cargo.toml` directly depends on `inquire.workspace = true`; no production Cargo manifest/source reference to `ratatui` exists yet. Crossterm is already present transitively in `Cargo.lock`, but not as the new frontend contract.
- Existing config CLI integration coverage includes `non_interactive_default_init_does_not_create_tunnel_secret_material` in `crates/agentic-gpt/tests/config_cli.rs`; the new plan preserves and expands non-interactive/non-TTY coverage.
- Existing secure commit code already validates secret targets, uses `0700` secret parents and `0600` files, writes atomically, and rolls secret state back if config persistence fails. The plan migrates this behavior rather than redesigning it.

## Contract Surface Map

| Surface | Frozen outcome | Status / evidence |
|---|---|---|
| Scope and identity | Only interactive `config init` + required setup/TUI infrastructure; unrelated consoles/features/platform work excluded. | Frozen — design spec §§1-4, 21 |
| Inputs and outputs | Interactive flags are editable seeds; non-interactive flags are direct inputs; bare non-TTY init returns actionable error; success keeps scrollback summary/pending actions. | Frozen — spec §5, §14 |
| Lifecycle/state | Dynamic Basic -> Connection (except Local) -> Optional Center -> Review -> Completion; mode drafts persist in memory; Review return target is explicit. | Frozen — spec §§6-10, 13, 17 |
| Failure/cancellation | Inline field errors; blocking system/commit errors; Esc never globally cancels; Ctrl+C globally cancels with no writes; terminal restored on error/panic. | Frozen — spec §§10-11, 15 |
| Persistence/recovery | No persistent writes before final confirmation; canonical builder validation before Review and commit; secure commit rollback retained. | Frozen — spec §§12-13, 20 |
| Security/trust | Secret content never enters review/log/debug/snapshot/error; secret input hidden; inactive-mode secrets remain process-memory only; `0700`/`0600` retained. | Frozen — spec §§8, 12-13, 20 |
| Operations | TTY gate, resize preservation, `NO_COLOR`, bounded keyboard-only first release, serial heavy Rust verification. | Frozen — spec §§5, 7, 18-19 + project convention |
| Concurrency/idempotency | No new concurrency model or background work introduced by setup; final commit is an explicit one-shot use-case. | N/A for new concurrency; one-shot commit frozen by spec §§12-13 |
| Surface parity | CLI TUI is implemented now; KMP is a sibling future frontend and is not modified. | N/A for KMP implementation — spec §4.4, §21 |
| Verification | Domain tests, TUI state/navigation tests, TestBackend layout tests, CLI/TTY behavior tests, real PTY/tmux smoke, full fmt/test/check/clippy/diff verification. | Frozen — spec §19, plan Phases 1-12 |

## Contract Gaps

No blocking product decision remains. The approved design already freezes observable behavior, safety, persistence, compatibility, keyboard semantics, and scope. The 2026-08-08 planning discussion additionally froze the execution/review workflow (D-08 through D-10 in `task_plan.md`).

If implementation uncovers a choice that would change those surfaces, it is not implementation discretion: reopen refinement and mark implementation authorization `no` before changing code.

## Options and Tradeoffs

### D-08 — Superpowers planning rigor without Superpowers execution orchestration

- Chosen: write the PWF `task_plan.md` at Superpowers `writing-plans` detail level, but execute phases continuously under PWF.
- Rejected for this task: Superpowers SDD/executing-plans with a fresh implementer/reviewer/fix loop for every small task. It provides strong local gates but adds substantial latency for a multi-thousand-line TUI implementation.
- Benefit: exact interfaces/tests/commit boundaries remain frozen without forcing review orchestration at each small step.

### D-09 — Read-only file-backed reviewer

- Chosen: reviewer may read all relevant artifacts and write only its review report.
- Rejected: reviewer directly edits code or automatically launches repairs.
- Benefit: review evidence remains persistent, auditable, and independently adjudicated instead of blending diagnosis with mutation.

### D-10 — Risk-weighted review cadence

- Chosen: one early review after the load-bearing `config_setup` foundation, then continuous implementation, then a staged phase-by-phase audit of the completed implementation, followed by adjudication/batched repair/scoped re-review.
- Rejected: review after every small implementation phase or no review until the very end.
- Benefit: catches a bad foundation before downstream coupling grows while avoiding the largest per-task review overhead.

## Risks and Unknowns

- Domain extraction is load-bearing: accidental duplication of config rules or leakage of TUI types would contaminate every later phase. Phase 3 exists specifically as an early independent boundary audit.
- Terminal panic-hook/restoration behavior touches process-global state and is easy to get subtly wrong; it needs a test seam plus real PTY smoke.
- Fullscreen TTY behavior is difficult to prove with ordinary piped-process tests; TestBackend covers rendering/state, while tmux/PTY smoke covers raw/alternate-screen/secret-echo behavior.
- Review suggestions can be technically plausible but outside the frozen product contract. The explicit adjudication phase prevents suggestion drift from silently becoming scope.
- `.planning/` is currently visible to Git rather than ignored; planning artifacts must not be committed or discarded implicitly without an explicit project decision.

## Issues Encountered

| Issue | Resolution |
|---|---|
| `planning-with-files` `scripts/init-session.sh` is not executable through `skills.run` in this environment. | Ran the installed script explicitly through `sh`; scoped plan initialized successfully as `2026-08-08-config-init-fullscreen-tui`. |
| `git check-ignore` returned non-zero for the scoped planning directory. | Recorded that `.planning` is Git-visible; this is not a refinement blocker. Do not create a planning commit without explicit authorization. |
| First guarded batch edit omitted `expectedRevision`. | Retried with the known revision; no partial edit was committed. |
| A standalone Superpowers-format draft plan temporarily existed under `docs/superpowers/plans/`. | Migrated its detailed content into the scoped PWF `task_plan.md`, added explicit PWF/review semantics, and removed the duplicate draft so the scoped `task_plan.md` is the sole execution plan. |

## Readiness Result

The handoff gate passes for all applicable surfaces. Goal/scope/non-goals/ownership are explicit; current implementation evidence is recorded; D-01 through D-10 are confirmed; no blocking question remains; inputs/lifecycle/failure/persistence/security/operations/verification are frozen; concurrency and KMP implementation are explicitly N/A rather than silently omitted; every approved design acceptance criterion is represented in the implementation phases and Final Acceptance Checklist; Phase 1 is the exact entry phase; implementation discretion is bounded; and product code remained unchanged during refinement.

`task_plan.md`, `findings.md`, and `progress.md` are therefore ready to transition together to `implementation_ready` / implementation authorized `yes`.

## Resources

- `docs/superpowers/specs/2026-08-07-config-init-fullscreen-tui-design.md`
- `.planning/2026-08-08-config-init-fullscreen-tui/task_plan.md`
- `crates/agentic-gpt/src/config_wizard.rs`
- `crates/agentic-gpt/src/config_cli.rs`
- `crates/agentic-gpt/src/config_templates.rs`
- `crates/agentic-gpt/src/config.rs`
- `crates/agentic-gpt/tests/config_cli.rs`
- `Cargo.toml`, `crates/agentic-gpt/Cargo.toml`, `Cargo.lock`

## Phase 1 implementation findings — 2026-08-08

- `config_templates::InitInput` is the canonical builder input and currently stores mode/profile, connection values, `SecretValue`, and all optional config structs. `build_config` already performs mode-specific canonical validation and emits stable `PendingAction` values; the new session must adapt staged drafts into this input rather than reimplementing template construction.
- `RuntimeMode`, `OptionalSection`, and `TunnelSecretSource` are crate-local enums in `config_templates`; `WorkerProfile` is crate-local in `main.rs`; `UiLanguage` is crate-local in `cli_i18n`. The Phase 1 modules can refer to these crate-local types without exposing TUI dependencies.
- Existing wizard defaults and parsers are concentrated in `config_wizard.rs`: tunnel defaults/secret-source parsing around `run_wizard`, optional-section defaults/collection around `collect_optional_section`, and stable error mappings around `required_text`, `parse_path_list`, `parse_usize`, `parse_u32`, and `parse_max_active_jobs`. These are the behavior sources to preserve in `config_setup::validation`.
- `default_path_policy` is public within the crate and derives write/read-only/deny roots from a workspace path; `Config` optional types (`PathPolicyConfig`, `LimitsConfig`, `SandboxConfig`, `RoomConfig`, `TunnelClientConfig`, `HubReportingConfig`) are available for conversion from staged drafts.
- `config_cli::ConfigInitArgs` currently owns interactive seeds and `handle_init` still calls the old `config_wizard` route. Phase 1 only adds `setup_seed_from_args`/module registration; routing remains unchanged until later phases.
- Step 2's focused compile failure is the intended red test: the new test module names `SetupSeed`, `SetupSession`, and `SetupField`, and the compiler confirms those domain contracts are not yet defined.
- The Phase 1 model now keeps invalid non-`file:`/`env:` tunnel seeds out of renderable buffers: it records only a safe static error and starts the editable path empty. This avoids treating an arbitrary CLI string as a secret-bearing UI value.
- `SetupSession::build_active_input` performs a full structured validation pass before conversion and then calls the existing `config_templates::build_config` in `validate_for_review`, so canonical mode/config validation remains authoritative. Secret copies are short-lived `SecretValue::new(secret.expose())` values; `SecretValue` itself remains non-`Clone`.
- Optional section legality delegates to the existing `config_templates::optional_section_is_legal` helper, made `pub(crate)` as the narrowest canonical-helper visibility change. No duplicate applicability table was introduced in the setup layer.
- The broad `config_` verification command includes the separate standalone supervisor integration test; its only failure was `runtime_directory_unavailable` before the worker tool call, so it does not exercise the Phase 1 setup layer. Focused setup tests, config/template/wizard unit tests, config CLI integration tests, formatting, and production check all passed.
- Phase 2 review data is built from staged domain values and safe canonical build metadata, never from `Config.agent_secret` or `SecretValue.expose()` output. The only secret-related review value is the literal `[REDACTED]`; `ReviewSecretWrite` carries path/write intent only.
- The Phase 2 outcome writer intentionally duplicates the existing wizard transaction boundary until the old wizard is removed in Phase 8; its tests currently exercise the same permission, rollback, target, and no-secret invariants before the old implementation is deleted.
- Phase 2 final verification passed: focused `config_setup` (14), legacy `config_wizard` (28), and `config_cli` integration (14) tests; `cargo fmt --all -- --check`; and `cargo check -p agentic-gpt`. The production check reports only expected transitional unused/dead-code warnings because later TUI phases have not yet consumed the new façade.

## Phase 3 review findings — 2026-08-08

- Review artifact: `.planning/2026-08-08-config-init-fullscreen-tui/reviews/phase-03-domain-foundation.md`, reviewed range `ba038e7..6a1ee0a`.
- No high-severity foundation issue was found. One low-severity test-gap was recorded: `ReviewModel.pending_actions` transformation is implemented but not directly asserted by the review tests. This is advisory and deferred to later hardening/adjudication.

## Phase 4 implementation findings — 2026-08-08

- Context7 confirmed the Ratatui 0.29 `Terminal::draw`/`Frame`/`TestBackend` APIs and the Crossterm event-poll pattern before implementation. Ratatui 0.29 resolves its backend through Crossterm 0.28.1, while the workspace's explicit Crossterm 0.29 dependency is also present; Cargo therefore records both compatible versions.
- The initial test run needed approved escalation because the sandbox cannot write the shared Cargo registry. After dependency download, the expected red compile failure was observed for the not-yet-implemented runtime/theme/widgets symbols.
- The TUI runtime installs one process-global panic hook through `Once`, preserves/chains the previous hook, marks the active guard only after setup succeeds, and uses a restoration seam to verify show-cursor -> leave-alternate-screen -> disable-raw-mode ordering.
- Phase 4 completed with exactly one product commit, `6a1ee0a..1721ec9` (`feat: add reusable fullscreen tui runtime`). The old `inquire` dependency and route remain untouched for the later migration phases.

## Phase 5 implementation findings — 2026-08-08

- `config_tui::navigation` owns the mode-specific main flow: Standalone/Hub use `Basic -> Connection -> OptionalCenter -> Review`, while Local omits Connection and reports a shorter progress total. Changing mode rebuilds only the future flow and retains both mode drafts in `SetupSession`.
- `config_tui::input::EditState` is deliberately UI-only. Character/editing keys change the buffer and cursor; Enter commits through the app's domain-field setter path, while Esc drops the buffer. Its custom `Debug` implementation redacts the buffer to prevent secret leakage in diagnostics.
- Basic and Connection pages delegate validation to `SetupSession`; the frontend stores only field-keyed safe error codes and focuses the first returned `SetupField`. Connection rendering uses masked text inputs for tunnel/agent secrets and does not put the secret marker into TestBackend content.
- Standalone connection fields are derived from the staged secret source and provision toggle: file mode exposes path/provision controls, environment mode exposes only the environment name, and the secret value editor appears only while provisioning is enabled. Non-editing Enter activates a focused field/toggle and advances only from the action row.
- Progress is passed from `Navigation::progress()` into page rendering instead of hard-coding a four-page total, so Local's reduced flow is visible in the header and survives resize re-rendering.
- Phase 5 focused verification passed with 15 `config_tui` tests, crate check, formatting, and diff checks. The production binary is intentionally not routed to this frontend until Phase 8; the current dead-code warnings are therefore expected transitional warnings.
- After the Enter/secret-field boundary fixes, the final focused verification passed with 15 tests. Phase 5 was committed exactly once as `1721ec9..ac5cab1` (`feat: add config tui core flow`); planning artifacts remain outside that product commit.

## Phase 6 implementation findings — 2026-08-08

- The Optional Center derives its visible order from one local eight-section list, while focus traversal uses `SetupSession::available_optional_sections()`. This keeps NotApplicable rows visible for status context without making them actionable.
- Optional form edits are held in `ConfigTuiApp::section_draft`, separate from `SetupSession::optional_drafts`. `validate_optional_draft` is a read-only domain operation; `save_optional_section` validates again before replacing the staged canonical draft, so invalid local edits cannot leak into Review or later sections.
- Re-entry clones the current staged draft (or domain default) and preserves saved values across mode/profile changes. Section forms expose exactly the frozen field set and route numeric, JSON-path, URL, and applicability rules through `config_setup` rather than parsing in the TUI.
- TestBackend and state tests cover dimmed/non-focusable NotApplicable rows, configured status/re-entry, Workspace path-policy fields with inline validation, and discard-on-Esc. Phase 6 product changes were committed exactly once as `ac5cab1..73fc5de` (`feat: add optional config tui center`).

## Phase 7 implementation findings — 2026-08-08

- Review now carries a Basic `ReviewGroup` alongside the active connection and optional groups. The TUI derives editable targets from the frontend-neutral model and filters inactive connection/optional sections without exposing their values.
- Review editors use `ReturnTarget::Review` explicitly. Basic and Connection validation remains domain-owned; Optional Center saves continue to validate the staged candidate before replacing the canonical draft. Returning to Review always rebuilds the redacted model, so mode changes and optional edits are reflected immediately.
- Confirmation validates the staged session before consuming it, then creates one `WizardOutcome` and calls the injected committer exactly once. The production committer delegates to the existing secure `commit_wizard_outcome` transaction; no writes occur while entering, editing, or cancelling Review.
- Completion shows only the real `agentic-gpt config show` next step and omits progress totals. Commit failures are represented by stable safe codes/localized copy in a blocking SystemError page; the underlying error string is never rendered, including when it contains a secret marker.
- Phase 7 focused verification passed: `config_setup` 15 tests, `config_tui` 22 tests, `cargo check -p agentic-gpt`, formatting, and diff checks. Product changes were committed exactly once as `73fc5de..8d820e4` (`feat: add config tui review and commit`).

## Phase 8 implementation findings — 2026-08-08

- `config_cli::handle_init` now has an explicit three-way branch: `--non-interactive` stays on the unchanged direct builder/write path; a process with TTY stdin, stdout, and stderr enters `run_config_tui`; every other interactive attempt returns a localized error that names `--non-interactive` and performs no writes.
- Interactive command-line flags are passed as `SetupSeed` values into the staged TUI session. The TUI owns the final commit and returns only after terminal restoration, so the existing scrollback summary is printed outside the alternate screen. Ctrl+C is translated from the stable `config_init_cancelled` code to the existing bilingual CLI cancellation text.
- The legacy `config_wizard.rs` prompt/request/backend layer and `inquire` dependency were deleted after the frontend-neutral setup/outcome and TUI tests covered their required behavior. Cargo lock cleanup removed only the no-longer-reachable inquire packages.
- Existing integration fixtures that intentionally initialize a config under piped stdio now pass `--non-interactive`; the new bare non-TTY black-box test covers English and Chinese actionable errors plus no config side effect.
- Phase 8 focused verification passed: 306 binary unit tests, 14 config CLI integration tests, `cargo check -p agentic-gpt`, formatting, diff checks, and stale-reference search. Product changes were committed exactly once as `8d820e4..80daae3` (`feat: replace config init with fullscreen tui`).

## Phase 9 implementation findings — 2026-08-08

- TestBackend coverage now exercises Basic, Standalone/Hub Connection, Local's skipped Connection, Optional Center, Workspace, Review, Completion, and SystemError surfaces. The small-terminal matrix keeps each primary action visible at 36x12 without panics.
- Test-first rendering coverage exposed that editing state was not reaching the view layer. `render_text_input_with_cursor` now takes the staged edit buffer and cursor, inserts a visible block cursor for ordinary fields, and keeps secret values as bullets; the test and real Hub PTY capture contain no secret marker.
- Test-first event coverage exposed that `handle_key(Enter)` treated Optional Center like a generic page and skipped section entry. Optional Center now dispatches Enter through `Activate`, so focused sections open and the action row still advances to Review; this is covered by app-state tests and tmux capture.
- Resize events remain intentionally state-neutral: Ratatui redraws against the new terminal area while `SetupSession`, page, focus, and editing buffer remain owned by the app. The runtime now discards only the dimensions after classifying the event.
- Clippy cleanup removed unused façade re-exports and marked intentionally reserved UI state/actions explicitly; `cargo clippy --workspace --all-targets -- -D warnings` passes without changing domain semantics.
- Disposable smoke evidence covered alternate-screen restoration, Basic Esc no-op, editing Esc, Optional-section Esc, Ctrl+C side-effect freedom and shell recovery, resize preservation, masked secret input, and successful config/secret commit (`0600` secret; JSON parsed successfully). CapsLock→Esc is represented by the same `KeyCode::Esc` path; the tmux environment cannot synthesize a physical keyd remap independently.
- Phase 9 product commit: `80daae3..f62bef7` (`docs: finish fullscreen config setup`).

## Phase 10 review findings — 2026-08-08

- Review artifact: `.planning/2026-08-08-config-init-fullscreen-tui/reviews/phase-10-staged-review.md`, reviewed HEAD `f62bef7fe11efcfc037829625c880e1a87065dee`, product range `1721ec9..f62bef7`.
- The read-only review identified `R10-001` partial terminal-entry restoration risk; `R10-002` Standalone Secret source/provision hidden-state error; `R10-003` non-inline/nondeterministic field errors; `R10-004` opaque Basic enum cycling; `R10-005` clipped/unscrollable Review; `R10-006` rollback-error code loss; `R10-007` missing deferred Standalone secret reference in Review; `R10-008` terminal errors bypassing the blocking system-error surface; `R10-009` syntactically aliased config/secret collision; and `R10-010` incomplete bilingual localization.
- The review also records advisory carry-over `R3-001` for direct pending-action assertions and the CapsLock physical-key smoke limitation. No findings were repaired during Phase 10.

## Phase 11 adjudication start — 2026-08-08

- Workflow moved to `repair` / `controller/implementer`; implementation is authorized only for accepted, frozen-contract repairs after each Phase 3/10 item receives an explicit disposition.
- Next: adjudicate the Phase 3 and Phase 10 artifacts against source/spec evidence, then create the bounded test-first repair checklist and preserve the one-commit-per-implementation-phase rule.

## Phase 11 dispositions — 2026-08-08

| Finding | Disposition | Rationale / repair boundary |
|---|---|---|
| `R3-001` | accepted | Add direct review pending-action assertions for deferred, immediate-secret, and Hub placeholder cases; this is a narrow regression-test repair in the existing domain review module. |
| `R10-001` | accepted | Terminal restoration is a frozen safety contract. Repair the setup guard and failure seam in `tui/runtime.rs`; no new terminal feature is authorized. |
| `R10-002` | accepted | Source selection and dependent provisioning are part of the frozen Standalone connection flow. Repair rendering/state normalization and add app regression coverage. |
| `R10-003` | accepted | Inline, deterministic field errors are explicitly required by design §11. Repair only error presentation/state mapping; validation ownership remains in `config_setup`. |
| `R10-004` | accepted | Design §8.1 requires discoverable mode/profile choices. Add an explicit compact choice presentation without changing the closed domain enums or flow semantics. |
| `R10-005` | accepted | Review return-editing is a core acceptance criterion. Add bounded focus-aware scrolling/visibility; do not add a general dashboard or pane system. |
| `R10-006` | accepted | Rollback uncertainty is a system-error safety distinction. Preserve a structured safe code through the existing blocking page. |
| `R10-007` | accepted | Active Standalone reference metadata is required in Review and can be shown without secret bytes. Extend the frontend-neutral redacted model and tests. |
| `R10-008` | accepted | System/terminal errors need one safe blocking/fallback contract. Map post-entry loop errors to the existing page and keep pre-entry errors safe/localized after full restoration. |
| `R10-009` | accepted | The exact-path collision check is an actionable secret-safety hole. Normalize/compare config and secret identities before any write and add an alias regression test. |
| `R10-010` | accepted | Bilingual fullscreen copy is the primary goal. Add a small language-keyed copy boundary for pages/errors and Chinese rendering tests; do not change persistent confirmation-language semantics. |

### Deferred/advisory items

- The physical CapsLock keyd mapping cannot be independently synthesized in the current tmux smoke environment; source-level Esc behavior is already covered, so this remains deferred as an environment limitation.
- The optional compact Review summary is deferred until the accepted scrolling repair is implemented; it is a presentation optimization, not a separate frozen requirement.

### Phase 11 repair checklist

- [x] Add failing tests for accepted domain pending-action, path-alias, deferred-reference, rollback-code, source-toggle, localized-copy, inline-error, Basic-choice, Review-scroll, and terminal-failure contracts.
- [x] Implement the accepted product repairs without changing `--non-interactive` semantics or the `config_setup`/`config_tui` ownership boundary.
- [x] Run focused tests and quality checks, then create exactly one Phase 11 product commit (`5d01977`).
- [x] Write the scoped `phase-11-repair-rereview.md` artifact and verify every accepted finding is resolved.

## Phase 11 repair completion — 2026-08-08

- Product repair commit: `5d019775d55e0767288a5933b15f689698b8cc83` (`fix: harden fullscreen config setup`), with base `f62bef7`; this is the only Phase 11 product commit.
- Scoped re-review: `.planning/2026-08-08-config-init-fullscreen-tui/reviews/phase-11-repair-rereview.md`.
- All accepted findings except the newly reproduced R10-009 symlink-parent variant were resolved by the scoped re-review. R10-009 is now explicitly tracked as partially resolved pending the bounded follow-up repair; remaining non-R10-009 items are advisory/environmental only.
- Phase 12 is now the active final verification phase; no new product behavior is authorized during verification.

## Follow-up independent finding — R10-009 remains partially resolved — 2026-08-08

- The independent re-review reproduced a second high-severity variant at HEAD `5d01977`: when `config_path=<root>/alias/config.json`, `alias -> real`, and `secret_path=<root>/real/config.json`, both final files are initially absent. Whole-path `fs::canonicalize` fails for both paths, so the current identity check returns false.
- The secret is then written first through the real path. The config write through the symlinked parent sees the newly-created secret as an existing config, copies it into `real/backups/config.<timestamp>.json`, and only then overwrites the target. The backup therefore contains the secret marker.
- Exact red test added as `config_setup::outcome::tests::symlink_parent_alias_to_nonexistent_target_is_rejected_before_secret_write`; before the repair it failed because `commit_wizard_outcome` unexpectedly succeeded.
- Disposition: `R10-009` is **partially resolved**, not fully resolved. A bounded follow-up repair is accepted: canonicalize the deepest existing ancestor and append the missing components before comparing identities. Existing lexical-alias and existing-file protections remain required.
- No broader config setup redesign is authorized; Phase 11/12 remain reopened only for this secret-safety repair and its scoped re-review.

## Follow-up repair disposition — R10-009 scoped resolution — 2026-08-08

- The bounded repair was implemented in `config_setup/outcome.rs` and committed as `4a23c0c` (`fix: reject symlinked config secret aliases`), with `5d01977` as its product base. Planning and review artifacts remain intentionally uncommitted.
- `paths_refer_to_same_file()` still short-circuits lexical equality and compares fully canonicalized existing targets. For a missing target, `canonicalize_with_existing_ancestor()` walks upward on `NotFound`, canonicalizes the deepest existing ancestor (resolving symlinked parents), and appends the missing components in order before comparison.
- The exact regression `symlink_parent_alias_to_nonexistent_target_is_rejected_before_secret_write` is green and asserts `config_init_secret_path_invalid`, no config, no secret, and no backup containing the marker. Existing lexical-alias and existing-file identity tests remain green.
- Scoped re-review artifact: `.planning/2026-08-08-config-init-fullscreen-tui/reviews/phase-11-r10-009-followup-rereview.md`. Within this accepted finding's scope, the previously open symlink-parent/nonexistent-target variant is resolved at `4a23c0c`; the historical `5d01977` partial disposition above is retained as provenance rather than overwritten.
