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
