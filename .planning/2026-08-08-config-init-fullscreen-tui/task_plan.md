# Task Plan: Config Init Fullscreen TUI

## Workflow State

- **Stage:** implementation_ready
- **Current role:** designer
- **Implementation authorized:** yes
- **Entry phase:** Phase 1
- **Open blocking decisions:** 0
- **Design checkpoint:** `8fb1ba3` (`docs: design fullscreen config init tui`) + scoped planning refinement completed 2026-08-08
- **Next action:** begin Phase 1 under `planning-with-files` without `refine-implementation-plan`; stop and reopen refinement only if a frozen contract is invalidated.

## Plan Authority and Execution Model

- This `task_plan.md` is the single executable plan and phase/status authority for this feature.
- Phase internals follow the precision of Superpowers `writing-plans`: exact files, interfaces, ordered test-first steps, verification commands, expected outcomes, and commit boundaries where useful.
- The Superpowers planning format does **not** authorize `subagent-driven-development`, `executing-plans`, per-step reviewers, or automatic fix loops.
- Implementation phases execute continuously under `planning-with-files`, updating `progress.md` and `findings.md` as work proceeds.
- Review happens only in the explicit review phases below. Reviewers are read-only with respect to product code and the plan; their only permitted write is the designated review artifact.
- Review findings are advisory until adjudicated. A reviewer must not repair findings, modify implementation, or spawn a review/fix loop.
- If a step would become too large to remain readable, the phase may reference one subordinate detail file; `task_plan.md` must still retain the step objective, contract, verification, completion boundary, and reference path.

**Goal:** Replace the sequential `inquire`-based `agentic-gpt config init` wizard with a bilingual fullscreen Ratatui setup flow whose domain state, validation, review model, and commit handoff remain reusable outside the terminal frontend.

**Architecture:** First extract the existing wizard business semantics into a frontend-neutral `config_setup` layer that owns staged drafts, applicability, structured validation, redacted review data, and final `WizardOutcome`. Then add a small config-agnostic `tui` runtime and a `config_tui` frontend that owns only pages, focus/edit buffers, navigation, rendering, and terminal events. The existing `config_templates::build_config` and secure config/secret commit path remain the canonical configuration facts; the old `PromptBackend`/`InquirePromptBackend` path is removed only after the new TUI and CLI behavior are covered.

**Tech Stack:** Rust 2021, rustc/cargo 1.97.1, clap 4.5, ratatui 0.29, crossterm 0.29, serde/serde_json, anyhow, existing `config_templates`, existing config validation and backup helpers, existing Unix secret permission semantics.

## Current Phase

Phase 1 — pending implementation after design handoff.

## Scope and Ownership

- In scope: only the interactive `agentic-gpt config init` experience, the frontend-neutral setup use-case it requires, the small reusable terminal runtime/widgets required by that experience, CLI routing changes, tests, and matching documentation.
- Existing `--non-interactive` config construction remains the stable automation surface and keeps current canonical config semantics.
- The existing secure config/secret commit behavior is preserved and moved behind the new setup boundary rather than redesigned.
- Out of scope: KMP setup/bridge work, Inline TUI, dashboard/main console, Jobs/process browser, Python REPL, PTY/terminal session APIs, mouse/panes/tabs/splits, MCP initialization, policy initialization, Windows support, and unrelated config-schema restructuring.
- Ownership boundary: `config_setup` owns setup semantics; `tui` owns config-agnostic terminal mechanics; `config_tui` owns terminal presentation/navigation only; `config_templates`/config validation remain canonical configuration facts.

## Key Questions

No blocking user-owned product questions remain. Any later discovery that changes observable behavior, safety, persistence, compatibility, or the phase structure must reopen refinement instead of being decided silently during implementation.

## Decisions Made

| ID | Area | Status | Outcome | Concise rationale | Evidence |
|---|---|---|---|---|---|
| D-01 | Interactive frontend | confirmed | Replace sequential `inquire` prompts with fullscreen `ratatui + crossterm`. | The approved design exists specifically to remove prompt-history/cancel artifacts and support page navigation/editing. | Design spec §§1-4, 21 |
| D-02 | Architecture | confirmed | Split `config_setup` (domain/application), `tui` (generic terminal runtime/widgets), and `config_tui` (terminal frontend). | Keeps setup semantics reusable and prevents TUI types from owning business rules. | Design spec §4, §17 |
| D-03 | CLI/seed behavior | confirmed | Interactive flags are editable seeds; `--non-interactive` remains direct/strict; bare non-TTY init errors actionably. | Separates human wizard intent from automation and preserves deterministic scripts. | Design spec §5 |
| D-04 | Keyboard/cancellation | confirmed | Esc only exits editing/returns one level/no-ops at root; Ctrl+C is the sole global cancel. | Required for predictable navigation and the current CapsLock→Esc environment. | Design spec §10, §15 |
| D-05 | Staging/persistence | confirmed | All edits stay in memory; real builder validates before Review and commit; persistent writes happen only after final confirmation. | Prevents partial config/secret side effects and keeps canonical validation authoritative. | Design spec §§11-13 |
| D-06 | Optional/Review UX | confirmed | Optional settings use a re-entrant center; Review is frontend-neutral/redacted and supports explicit return-to-Review editing. | Sequential prompts cannot satisfy repeated section edits and review jump-back semantics. | Design spec §§9, 13 |
| D-07 | Terminal/safety | confirmed | RAII + panic-safe restoration, secret redaction/non-echo, existing secret permissions/rollback, and `NO_COLOR` support are mandatory. | Terminal corruption or secret leakage is a release-blocking failure. | Design spec §§14-15, 18-20 |
| D-08 | Planning workflow | confirmed | `task_plan.md` is the only executable plan; its implementation detail follows Superpowers `writing-plans` rigor without inheriting Superpowers execution orchestration. | Preserves precise engineering contracts while avoiding slow per-task SDD/review loops. | 2026-08-08 planning discussion |
| D-09 | Review behavior | confirmed | Reviewers are read-only and write evidence-backed findings only to designated review artifacts; findings require Controller adjudication before repair. | Separates independent diagnosis from code mutation and prevents reviewer suggestions becoming automatic changes. | 2026-08-08 planning discussion |
| D-10 | Review cadence | confirmed | Review once after domain foundation, then run implementation continuously through hardening, then perform one staged independent review, batched repair, scoped re-review, and final verification. | Catches load-bearing boundary mistakes early while keeping later implementation fast. | 2026-08-08 planning discussion |

## Implementation Discretion

The Implementer may choose equivalent private helper decomposition, local variable names, exact test-function names/assertion wording, and minor layout mechanics that do not alter the frozen contracts. Concrete inter-phase interfaces/types explicitly named below, module ownership, CLI behavior, validation/error semantics, secret/persistence rules, key behavior, review workflow, and acceptance criteria are not implementation discretion. If a local choice would change observable behavior or a frozen boundary, reopen refinement first.

## Phase Dependencies

- Phase 1 -> Phase 2 -> Phase 3 are sequential because the domain/outcome boundary is load-bearing.
- Phase 3 may block Phase 4 only for an adjudicated high-severity foundation finding; ordinary findings are recorded for later Phase 11 adjudication.
- Phase 4 -> Phase 5 -> Phase 6 -> Phase 7 -> Phase 8 -> Phase 9 execute sequentially under the same frozen contract, with each phase running its own focused verification and recording base/HEAD commits in `progress.md`.
- Phase 10 starts only after Phases 4-9 complete and reviews their recorded ranges in one independent staged pass.
- Phase 11 starts after Phase 10 and owns all finding dispositions, accepted repairs, and scoped repair re-review.
- Phase 12 starts only after Phase 11 has no unresolved accepted finding and is the final release/acceptance gate.

## Implementation Handoff

- **Plan maturity:** implementation_ready
- **Design phase:** complete
- **Implementation authorized:** yes
- **Entry phase:** Phase 1
- **Frozen decisions:** D-01 through D-10
- **Open blocking decisions:** none
- **Implementation discretion:** see `Implementation Discretion`
- **Verification convention:** test-first focused checks per implementation phase; heavy Rust commands serially with `--test-threads=1`; TestBackend plus disposable tmux/PTY smoke for terminal behavior; final fmt/test/check/clippy/diff gate in Phase 12
- **Commit convention:** use the focused commit boundary specified by each implementation phase; record each phase BASE..HEAD range in `progress.md`; review/planning artifacts are not auto-committed
- **Design checkpoint:** product design commit `8fb1ba3`; scoped planning artifacts are intentionally uncommitted pending explicit repository-history choice
- **Next invocation:** `$planning-with-files` without `$refine-implementation-plan`

## Global Constraints

- The approved design is `docs/superpowers/specs/2026-08-07-config-init-fullscreen-tui-design.md`; do not broaden scope beyond it.
- Default initialization mode remains exactly `standalone`; default capability profile remains exactly `normal`.
- `config_setup` must not depend on Ratatui, Crossterm, terminal key events, page indexes, cursor state, terminal size, or TUI navigation types.
- `config_tui` must not duplicate mode applicability, optional-section legality, field/config validation, review construction, or final config construction.
- Interactive flags are seeds/prefill values and remain editable; only `--non-interactive` treats flags as direct execution parameters.
- In interactive mode, seeds for inactive modes are retained in memory rather than rejected; only the active mode participates in build/review/commit.
- `agentic-gpt config init --non-interactive ...` keeps existing deterministic behavior and must never initialize Ratatui/Crossterm.
- Bare non-TTY `agentic-gpt config init` must not write a config or wait on stdin; it returns a localized actionable error instructing the caller to use `--non-interactive`.
- `Esc` in editing exits editing only; `Esc` on a child page returns one level; `Esc` on the wizard root is a no-op; `Ctrl+C` is the only global cancel key.
- All wizard edits remain staged in memory until final confirmation; optional-section “save” never writes config, backup, or secret files.
- Before entering Review and again before commit, call the real config builder/canonical validation rather than relying only on UI-side field checks.
- Secret values never enter `Debug`, `Display`, logs, snapshots, review text, error text, or normal test failure output.
- Secret directories remain mode `0700`; secret files remain mode `0600`; existing transactional rollback behavior is preserved.
- `NO_COLOR` disables hue-based state distinction without breaking focus/status readability.
- The first public TUI is keyboard-only; do not add mouse, inline TUI, dashboard, Jobs browser, Python REPL, PTY session, pane/tab/split management, MCP initialization, or policy initialization.
- The existing KMP `console/` frontend is not modified in this feature.
- Run Rust test commands serially with `--test-threads=1`; do not launch multiple heavy Cargo test/check/clippy commands in parallel on the laptop.
- Every behavior change begins with a failing focused test and ends with a focused commit.

---

## File Structure

### Files to create

- `crates/agentic-gpt/src/config_setup/mod.rs` — public crate-local façade for setup domain types/use-cases.
- `crates/agentic-gpt/src/config_setup/model.rs` — `SetupSeed`, `SetupSession`, per-mode drafts, optional-section drafts/status, and domain field identifiers.
- `crates/agentic-gpt/src/config_setup/validation.rs` — structured field/section validation and conversion from staged drafts to active `InitInput`.
- `crates/agentic-gpt/src/config_setup/review.rs` — redacted frontend-neutral `ReviewModel` and review groups/items.
- `crates/agentic-gpt/src/config_setup/outcome.rs` — `WizardOutcome`, explicit final-build handoff, and existing secure config/secret commit implementation.
- `crates/agentic-gpt/src/tui/mod.rs` — config-agnostic TUI façade.
- `crates/agentic-gpt/src/tui/runtime.rs` — alternate-screen/raw-mode/cursor lifecycle, panic-safe restoration, event polling, resize handling.
- `crates/agentic-gpt/src/tui/theme.rs` — normal/accent/dim/success/warning/error/disabled style tokens with `NO_COLOR` handling.
- `crates/agentic-gpt/src/tui/widgets.rs` — minimal shared text-input, radio/menu, action, inline-error, header/progress, footer helpers.
- `crates/agentic-gpt/src/config_tui/mod.rs` — `run_config_tui` entry point.
- `crates/agentic-gpt/src/config_tui/navigation.rs` — page stack, review return target, focus order, dynamic main-flow calculation.
- `crates/agentic-gpt/src/config_tui/input.rs` — UI-only edit buffer/cursor handling and key-to-action mapping.
- `crates/agentic-gpt/src/config_tui/app.rs` — `TuiState`, event reduction, domain use-case calls, cancellation/completion/system-error state.
- `crates/agentic-gpt/src/config_tui/pages.rs` — Basic, mode-specific Connection, Optional Center/section, Review, Completion, and system-error rendering.

### Files to modify

- `Cargo.toml` — replace workspace `inquire` dependency with `ratatui = "0.29"` and `crossterm = "0.29"` after migration is complete.
- `crates/agentic-gpt/Cargo.toml` — consume Ratatui/Crossterm; retain `inquire.workspace = true` only until Phase 8 removes the old path.
- `Cargo.lock` — lock the new terminal frontend dependency graph and remove `inquire`-only graph entries when no longer referenced.
- `crates/agentic-gpt/src/main.rs` — register `config_setup`, `tui`, and `config_tui`; remove `config_wizard` after migration.
- `crates/agentic-gpt/src/config_cli.rs` — convert `ConfigInitArgs` into `SetupSeed`, route TTY init to `config_tui`, keep non-interactive init unchanged, implement explicit non-TTY error.
- `crates/agentic-gpt/src/config_wizard.rs` — temporary source of characterized behavior; progressively emptied and deleted after domain/commit tests migrate.
- `crates/agentic-gpt/tests/config_cli.rs` — black-box non-TTY/non-interactive behavior and no-side-effect cancellation/error coverage.
- `README.md`, `README.zh-CN.md` — replace sequential wizard wording with fullscreen setup behavior and key semantics.
- `docs/configuration.md`, `docs/configuration.zh-CN.md` — document TTY/non-TTY split, editable seeds, optional center, Review, cancellation, and secret handling.

### Files reviewed but not expected to change

- `crates/agentic-gpt/src/config_templates.rs` — remains canonical mode/template construction; if an existing private validator is needed by `config_setup`, make only that existing helper `pub(crate)` rather than reimplementing its logic.
- `crates/agentic-gpt/src/config.rs` — remains canonical config validation and backup writer.
- `console/` — intentionally untouched.

---

### Phase 1: Extract Frontend-Neutral Setup Session and Structured Validation

- **Status:** pending

**Files:**
- Create: `crates/agentic-gpt/src/config_setup/mod.rs`
- Create: `crates/agentic-gpt/src/config_setup/model.rs`
- Create: `crates/agentic-gpt/src/config_setup/validation.rs`
- Modify: `crates/agentic-gpt/src/main.rs:1-20`
- Modify: `crates/agentic-gpt/src/config_cli.rs:967-1035`
- Test: module tests under `config_setup/model.rs` and `config_setup/validation.rs`

**Interfaces:**
- Produces `pub(crate) struct SetupSeed` with optional `mode`, `profile`, `tunnel_id`, `tunnel_api_key`, `hub_url`, `hub_transport`, `agent_id`, and secret-aware `agent_secret`.
- Produces `pub(crate) struct SetupSession` with `selected_mode`, `selected_profile`, independent `StandaloneDraft` and `HubDraft`, optional-section drafts, `UiLanguage`, and `config_path`.
- Produces `pub(crate) enum SetupField` covering every editable setup field; this is a domain field identifier, not a page/focus identifier.
- Produces `pub(crate) struct ValidationError { pub field: SetupField, pub code: &'static str }` and `pub(crate) type ValidationErrors = Vec<ValidationError>`.
- Produces `pub(crate) enum OptionalSectionDraft { Identity(IdentityDraft), Workspace(WorkspaceDraft), Confirmation(ConfirmationDraft), Limits(LimitsDraft), Sandbox(SandboxDraft), Room(RoomDraft), TunnelClient(TunnelClientDraft), HubReporting(HubReportingDraft) }`.
- Produces `SetupSession::new(seed: SetupSeed, language: UiLanguage, config_path: PathBuf) -> Self`, `set_mode`, `set_profile`, `available_optional_sections`, `section_status`, `validate_basic`, `validate_connection`, `validate_field`, `save_optional_section`, and `validate_for_review`.
- `save_optional_section(&mut self, draft: OptionalSectionDraft) -> Result<(), ValidationErrors>` validates the candidate and mutates staged optional state only when validation succeeds.
- `config_cli` owns `fn setup_seed_from_args(args: &ConfigInitArgs) -> SetupSeed`; `config_setup` does not depend on clap/`ConfigInitArgs`.

- [ ] **Step 1: Add failing setup-session tests for defaults, editable seeds, and mode-draft retention**

Add this minimal test first, then extend the same test module with the seed cases listed below:

```rust
#[test]
fn setup_defaults_to_standalone_normal_and_preserves_inactive_mode_seeds() {
    let seed = SetupSeed {
        mode: Some(RuntimeMode::Hub),
        tunnel_id: Some("tunnel_seed".into()),
        hub_url: Some("https://hub.example.com".into()),
        ..SetupSeed::default()
    };
    let mut session = SetupSession::new(seed, UiLanguage::En, PathBuf::from("/tmp/config.json"));

    assert_eq!(session.selected_mode(), RuntimeMode::Hub);
    assert_eq!(session.selected_profile(), WorkerProfile::Normal);
    assert_eq!(session.standalone().tunnel_id, "tunnel_seed");
    assert_eq!(session.hub().hub_url, "https://hub.example.com");

    session.set_mode(RuntimeMode::Standalone);
    assert_eq!(session.standalone().tunnel_id, "tunnel_seed");
    session.set_mode(RuntimeMode::Hub);
    assert_eq!(session.hub().hub_url, "https://hub.example.com");
}
```

Also prove a `file:PATH`/`env:NAME` tunnel secret seed is parsed into the corresponding standalone secret source/path-or-name draft, and malformed tunnel secret references produce a field validation error rather than a panic or secret disclosure.

- [ ] **Step 2: Run the focused tests and verify they fail because `config_setup` does not exist**

Run:

```bash
cargo test -p agentic-gpt config_setup:: -- --test-threads=1
```

Expected: compile/test failure for missing `config_setup` types.

- [ ] **Step 3: Implement focused staged draft types without terminal dependencies**

Use these exact field names and shapes:

```rust
pub(crate) struct StandaloneDraft {
    pub tunnel_id: String,
    pub secret_source: TunnelSecretSource,
    pub secret_path: String,
    pub secret_environment: String,
    pub provision_secret_now: bool,
    pub secret_value: Option<SecretValue>,
}

pub(crate) struct HubDraft {
    pub hub_url: String,
    pub hub_transport: String,
    pub agent_id: String,
    pub agent_secret: Option<SecretValue>,
}

pub(crate) struct OptionalDrafts {
    pub identity: Option<IdentityDraft>,
    pub workspace: Option<WorkspaceDraft>,
    pub confirmation: Option<ConfirmationDraft>,
    pub limits: Option<LimitsDraft>,
    pub sandbox: Option<SandboxDraft>,
    pub room: Option<RoomDraft>,
    pub tunnel_client: Option<TunnelClientDraft>,
    pub hub_reporting: Option<HubReportingDraft>,
}
```

Use `Option<SectionDraft>` to encode `Default` versus `Configured`; compute `NotApplicable` from selected mode/profile rather than storing a third mutable state. Initialize section draft defaults from the current prompt defaults: display name `AgenticGPT agent`, workspace root `~/.agentic_gpt/workspace` plus `default_path_policy`, limits `2/auto/5`, sandbox `false/bwrap/["/usr","/bin","/lib","/lib64","/etc/ssl"]`, Room `Asia/Shanghai/5`, tunnel cache `~/.agentic_gpt/cache/tunnel-client` with auto-download true, and Hub reporting `false/metadata`.

- [ ] **Step 4: Add failing validation tests for required fields, transport, numbers, path lists, secrets, and section legality**

Cover at least:

```rust
assert_eq!(
    session.validate_connection().unwrap_err()[0].field,
    SetupField::TunnelId
);
```

and cases for Hub URL/transport, empty Hub secret, JSON path lists, `max_concurrent_tasks`, `max_active_jobs`, diary boundary hour, sandbox runtime-path JSON, reporting detail, and a Room/TunnelClient/HubReporting section that is not applicable to the current mode/profile.

- [ ] **Step 5: Implement structured validators by reusing current parsing/config helpers**

Move/adapt `required_text`, `parse_path_list`, `parse_usize`, `parse_u32`, `parse_max_active_jobs`, secret-reference parsing, and section legality into `config_setup::validation`. Preserve existing stable error codes such as `config_init_secret_empty`, `config_init_path_policy_write_roots_invalid`, and `config_init_number_invalid: ...`; map them to `ValidationError` with a concrete `SetupField`.

Do not perform file writes. Validation may parse/normalize draft text and may call canonical validators, but it must not mutate persistent config.

- [ ] **Step 6: Build active `InitInput` from staged state and prove inactive drafts are ignored**

Add `pub(crate) fn build_active_input(&self) -> Result<InitInput, ValidationErrors>` that includes only selected mode/profile and applicable configured optional sections. Create explicit short-lived secret copies with `SecretValue::new(secret.expose())`; do not add `Clone` to `SecretValue`.

Test that Hub secrets/tunnel data do not appear when Standalone is active and vice versa, while switching back restores the staged draft.

- [ ] **Step 7: Run focused setup tests**

Run:

```bash
cargo test -p agentic-gpt config_setup:: -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 8: Commit the extraction**

```bash
git add crates/agentic-gpt/src/config_setup crates/agentic-gpt/src/config_cli.rs crates/agentic-gpt/src/main.rs
git commit -m "refactor: extract config setup session"
```

---

### Phase 2: Move Redacted Review and Secure Outcome/Commit Behind `config_setup`

- **Status:** pending

**Files:**
- Create: `crates/agentic-gpt/src/config_setup/review.rs`
- Create: `crates/agentic-gpt/src/config_setup/outcome.rs`
- Modify: `crates/agentic-gpt/src/config_setup/mod.rs`
- Modify: `crates/agentic-gpt/src/config_wizard.rs:1023-1514,2110-2324`
- Test: module tests under `config_setup/review.rs` and `config_setup/outcome.rs`

**Interfaces:**
- Produces `pub(crate) struct ReviewModel { mode, profile, connection, optional_sections, config_path, will_backup_existing_config, pending_actions, secret_write }`.
- Produces `pub(crate) enum ReviewTarget { Basic, Connection, OptionalCenter, OptionalSection(OptionalSection) }` for domain review grouping only; it must not reference TUI pages or navigation stacks.
- Produces redacted `ReviewItem { label_key: &'static str, value: String }`/`ReviewGroup` data; labels are keys/raw domain concepts and localization remains in the frontend.
- Produces `SetupSession::review_model(&self) -> Result<ReviewModel, ValidationErrors>` which calls `build_config` on the active staged input and never exposes secret contents.
- Produces `SetupSession::into_wizard_outcome(self) -> Result<WizardOutcome, ValidationErrors>` and `pub(crate) fn commit_wizard_outcome(config_path: &Path, outcome: WizardOutcome) -> Result<InitSummary>`.

- [ ] **Step 1: Add failing review tests that assert redaction, active-mode-only data, pending actions, and section status**

Use a unique secret marker and assert it is absent from every renderable/debuggable review field:

```rust
let marker = "review-secret-marker-4f2e";
let review = session.review_model().unwrap();
let rendered = format!("{review:?}");
assert!(!rendered.contains(marker));
assert!(review.secret_write.is_some());
```

Derive `Debug` for `ReviewModel`; this is safe only because the model contains no `SecretValue` or secret text. Test that inactive mode drafts are absent and that each section reports exactly `Default`, `Configured`, or `NotApplicable` from its current staged/applicability state.

- [ ] **Step 2: Run the focused tests and verify failure**

```bash
cargo test -p agentic-gpt config_setup::review -- --test-threads=1
```

Expected: FAIL before review types exist.

- [ ] **Step 3: Implement `ReviewModel` from canonical build output plus staged section status**

Call `build_config` using the active `InitInput`; represent secrets only as source/reference metadata and booleans such as `will_write`, never secret bytes. Determine `will_backup_existing_config` with a metadata/existence check against `config_path` without writing anything.

Review must include mode/profile, current connection values, optional statuses/summary values, config path, backup intent, pending actions, and optional secret-write path.

- [ ] **Step 4: Port the existing secure `WizardOutcome` and commit implementation unchanged in behavior**

Move the current transactional secret/config writer (including `PriorSecretState`, temporary secret file guard, `0700` parent, `0600` file, atomic rename, config-write rollback, symlink/target checks, and stable error codes) from `config_wizard.rs` into `config_setup/outcome.rs`.

`WizardOutcome` must not derive `Debug`. `SetupSession::into_wizard_outcome` performs canonical build validation again immediately before handing off to commit.

- [ ] **Step 5: Move the existing commit tests and verify all permission/rollback cases still pass**

Port the tests for:

- secret parent/file permissions;
- replacement of an existing secret;
- config failure restoring an existing secret;
- config failure removing a newly created secret;
- invalid secret target rejection;
- no-secret config write;
- final refusal/cancellation producing no write plan.

Run:

```bash
cargo test -p agentic-gpt config_setup::outcome -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 6: Run the full existing config-wizard/config-CLI regression before touching the frontend**

```bash
cargo test -p agentic-gpt config_ -- --test-threads=1
```

Expected: PASS; the old `inquire` route still works at this checkpoint.

- [ ] **Step 7: Commit the review/outcome migration**

```bash
git add crates/agentic-gpt/src/config_setup crates/agentic-gpt/src/config_wizard.rs
git commit -m "refactor: isolate config setup review and commit"
```

---

### Phase 3: Independent Domain-Foundation Review

- **Status:** pending
- **Type:** read-only review

**Objective:** Independently verify that Phases 1-2 established the load-bearing frontend-neutral setup boundary without changing behavior, safety, or persistence semantics before later TUI phases build on it.

**Inputs:**
- Approved design: `docs/superpowers/specs/2026-08-07-config-init-fullscreen-tui-design.md`.
- This plan, especially Global Constraints and Phases 1-2.
- Phase 1-2 commit range recorded in `progress.md`.
- Relevant source/tests at the reviewed HEAD.

**Reviewer contract:**
- Read-only with respect to product code, tests, configuration, documentation, `task_plan.md`, `findings.md`, and `progress.md`.
- The only permitted write is `.planning/2026-08-08-config-init-fullscreen-tui/reviews/phase-03-domain-foundation.md` (creating `reviews/` if absent).
- Do not repair findings, rewrite implementation, change scope, or start another reviewer/fix loop.
- Do not manufacture findings. `No findings.` is a valid complete result.
- Every finding must cite concrete repository evidence and distinguish a frozen-contract violation from a discretionary improvement suggestion.

**Review criteria, in order:**
1. Spec compliance: `config_setup` is frontend-neutral and preserves staged/mode/optional/review/outcome contracts.
2. Correctness: active/inactive mode drafts, structured validation, canonical builder handoff, and pending-action semantics are correct.
3. Safety/data integrity: secret redaction, `0700`/`0600`, transactional rollback, target validation, and no pre-confirmation writes remain intact.
4. Test adequacy: failure paths and boundary contracts are meaningfully exercised, not merely compile-covered.
5. Maintainability: no duplicated business rules or TUI/CLI coupling has leaked into the new domain layer.

**Finding format:**

```markdown
## R3-001 — concise title
- Severity: high | medium | low | suggestion
- Category: spec-compliance | correctness | safety-data-integrity | test-gap | code-quality | suggestion
- Claim: one falsifiable statement
- Evidence: `path:line` / symbol / failing or missing test
- Expected contract: exact plan/spec requirement
- Impact: concrete consequence
- Suggested direction: repair direction only, not a patch
```

**Verification:**
- Review artifact exists and names the exact reviewed commit range.
- Every non-suggestion finding has concrete evidence and an expected contract/correctness basis.
- Reviewer made no product-code or plan changes.

**Completion boundary:** Phase 3 completes when the artifact is written. Findings are not automatically accepted; implementation may continue unless a high-severity load-bearing foundation finding is identified, in which case the Controller adjudicates it before Phase 4.

---

### Phase 4: Add the Config-Agnostic Fullscreen TUI Runtime and Shared Widgets

- **Status:** pending

**Files:**
- Create: `crates/agentic-gpt/src/tui/mod.rs`
- Create: `crates/agentic-gpt/src/tui/runtime.rs`
- Create: `crates/agentic-gpt/src/tui/theme.rs`
- Create: `crates/agentic-gpt/src/tui/widgets.rs`
- Modify: `Cargo.toml`
- Modify: `crates/agentic-gpt/Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/agentic-gpt/src/main.rs`
- Test: module tests under `tui/theme.rs`, `tui/widgets.rs`, and runtime restoration seams

**Interfaces:**
- Produces `pub(crate) struct TerminalSession` that enters raw mode + alternate screen, hides the cursor, exposes a Ratatui `Terminal<CrosstermBackend<Stdout>>`, and restores cursor/alternate screen/raw mode on `Drop`.
- Produces `pub(crate) enum TerminalEvent { Key(KeyEvent), Resize(u16, u16), Tick }` and `TerminalSession::next_event(timeout: Duration) -> Result<TerminalEvent>`.
- Produces `pub(crate) struct Theme` with accent/focus, normal, dim/help, success, warning, error, disabled styles and a `Theme::from_env()` `NO_COLOR` path.
- Produces rendering helpers for header/progress, radio/menu rows, text/secret input, inline error, action button, and contextual footer; these helpers accept generic labels/state and know nothing about config types.

- [ ] **Step 1: Add Ratatui/Crossterm dependencies while retaining `inquire` temporarily**

In workspace dependencies add:

```toml
ratatui = "0.29"
crossterm = "0.29"
```

and consume them in `crates/agentic-gpt/Cargo.toml`. Do not remove `inquire` until Phase 8.

- [ ] **Step 2: Add failing theme/widget buffer tests**

Using `ratatui::backend::TestBackend`, assert that a 70-column frame renders the header title/progress, focused input prefix `›`, radio markers `●`/`○`, inline error text, action label, and footer without panicking. Add a narrow-frame case (for example 36x16) that only asserts rendering completes and the action/footer remain present.

- [ ] **Step 3: Implement minimal style tokens and `NO_COLOR` behavior**

`Theme::from_env()` must inspect `NO_COLOR`; when present, use modifiers/symbols instead of hue to distinguish focus/status. Keep all concrete colors inside `theme.rs`.

- [ ] **Step 4: Implement `TerminalSession` with RAII restoration and panic cleanup chaining**

On enter: enable raw mode, enter alternate screen, hide cursor, create Terminal, clear. On normal/error `Drop`: show cursor, leave alternate screen, disable raw mode, best-effort terminal cursor restore.

Install one process-global panic hook through `std::sync::Once`; preserve the previously installed hook and always chain to it. `TerminalSession` marks a process-global TUI-active guard true only after raw mode/alternate-screen setup succeeds and clears it during normal `Drop`. When a panic occurs while the guard is active, the hook performs best-effort cursor show, alternate-screen leave, and raw-mode disable before chaining to the prior hook. Do not swap panic hooks on every session and never silently replace the prior hook.

- [ ] **Step 5: Add a restoration-seam unit test and compile check**

Factor terminal side effects behind tiny functions/guards so a test can prove cleanup ordering without requiring a real TTY. Then run:

```bash
cargo test -p agentic-gpt tui:: -- --test-threads=1
cargo check -p agentic-gpt
```

Expected: PASS.

- [ ] **Step 6: Commit the runtime**

```bash
git add Cargo.toml Cargo.lock crates/agentic-gpt/Cargo.toml crates/agentic-gpt/src/tui crates/agentic-gpt/src/main.rs
git commit -m "feat: add reusable fullscreen tui runtime"
```

---

### Phase 5: Build `config_tui` Navigation, Editing, Basic, and Connection Pages

- **Status:** pending

**Files:**
- Create: `crates/agentic-gpt/src/config_tui/mod.rs`
- Create: `crates/agentic-gpt/src/config_tui/navigation.rs`
- Create: `crates/agentic-gpt/src/config_tui/input.rs`
- Create: `crates/agentic-gpt/src/config_tui/app.rs`
- Create: `crates/agentic-gpt/src/config_tui/pages.rs`
- Modify: `crates/agentic-gpt/src/main.rs`
- Test: module tests under `config_tui/navigation.rs`, `input.rs`, `app.rs`, `pages.rs`

**Interfaces:**
- Produces `pub(crate) enum ConfigPage { Basic, Connection, OptionalCenter, Optional(OptionalSection), Review, Completion }`.
- Produces `pub(crate) enum ReturnTarget { MainFlow, Review }` and explicit page-stack behavior.
- Produces `pub(crate) struct EditState { field: SetupField, buffer: String, cursor: usize }` and `pub(crate) struct TuiState { page, return_target, focus, editing, scroll, modal, cancelled, committed_summary }`.
- Produces `pub(crate) enum TuiAction` for navigation/edit/select/activate/back/cancel and `ConfigTuiApp::handle_event`.
- Produces `pub(crate) fn run_config_tui(config_path: &Path, seed: SetupSeed, language: UiLanguage) -> Result<InitSummary>` only after Phase 7 wires Review/commit; until then expose a testable `ConfigTuiApp::new`/`render` without routing CLI traffic to it.

- [ ] **Step 1: Write failing state tests for dynamic flow and key semantics**

Cover:

- Standalone/Hub: `Basic -> Connection -> OptionalCenter -> Review`;
- Local: `Basic -> OptionalCenter -> Review` with total progress reduced;
- mode changes recalculate future flow without destroying mode drafts;
- focused text `Enter` creates `EditState` seeded from confirmed domain value;
- editing `Esc` drops edit buffer and stays on page;
- child-page `Esc` returns one level;
- Basic/root `Esc` no-op;
- `Ctrl+C` from navigation or editing sets cancelled state;
- `Tab`/`Shift+Tab` and arrows never focus disabled/not-applicable rows.

- [ ] **Step 2: Run focused tests and verify failure**

```bash
cargo test -p agentic-gpt config_tui:: -- --test-threads=1
```

Expected: compile/test failure for missing frontend types.

- [ ] **Step 3: Implement UI-only navigation/edit state and key mapping**

Keep all terminal key interpretation in `config_tui::input/app`. Ordinary character keys only mutate `EditState.buffer`; committing an edit with Enter calls a `SetupSession` setter and clears/refreshes the field error. Esc while editing discards the uncommitted buffer and returns to the already-confirmed setup value.

- [ ] **Step 4: Implement Basic page rendering and domain updates**

Render Runtime Mode and Profile on one page with descriptions, radio selection, one accent/focus state, progress header, and contextual footer. Mode/profile changes call `SetupSession::set_mode/set_profile`; never compute optional legality in the page.

- [ ] **Step 5: Implement Standalone and Hub connection pages plus Local skip**

Standalone fields: Tunnel ID, secret source, file path or environment name, provision-now toggle when file source, and hidden secret input only when provisioning now.

Hub fields: Hub URL, transport, Agent ID, hidden Agent Secret.

Local has no Connection page. Secret widgets display bullets/placeholder only; their render/debug state must never contain the actual secret string.

- [ ] **Step 6: Wire page-level validation on Next**

Basic/Connection Next calls the corresponding `SetupSession` validator. On errors stay on the page, set inline errors keyed by `SetupField`, and focus the first invalid field. After a previously-invalid field is edited and confirmed, revalidate that field and clear the error if valid.

- [ ] **Step 7: Add Ratatui `TestBackend` snapshots-by-assertion for Basic and both connection variants**

Assert key text/focus markers/conditional fields rather than exact color cells. Include a resize re-render that preserves domain and UI state.

- [ ] **Step 8: Run focused tests/check and commit**

```bash
cargo test -p agentic-gpt config_tui:: -- --test-threads=1
cargo check -p agentic-gpt

git add crates/agentic-gpt/src/config_tui crates/agentic-gpt/src/main.rs
git commit -m "feat: add config tui core flow"
```

---

### Phase 6: Implement the Optional Configuration Center and Re-entrant Section Forms

- **Status:** pending

**Files:**
- Modify: `crates/agentic-gpt/src/config_tui/app.rs`
- Modify: `crates/agentic-gpt/src/config_tui/navigation.rs`
- Modify: `crates/agentic-gpt/src/config_tui/pages.rs`
- Modify: `crates/agentic-gpt/src/config_setup/model.rs`
- Modify: `crates/agentic-gpt/src/config_setup/validation.rs`
- Test: `config_tui` and `config_setup` module tests

**Interfaces:**
- Optional Center obtains all eight known sections from `SetupSession`, with computed `Default`/`Configured`/`NotApplicable` status.
- Not-applicable sections remain visible, dimmed, and non-focusable.
- Entering an applicable section opens `ConfigPage::Optional(section)` and loads fields from the existing staged `Option<SectionDraft>` or section defaults.
- “Save and return” validates the whole section, writes only the staged draft, and returns to Optional Center; Esc returns without saving uncommitted edit buffer changes.

- [ ] **Step 1: Add failing navigation/domain tests for section availability, status, repeated entry, and mode/profile changes**

Prove:

```rust
assert_eq!(session.section_status(OptionalSection::Identity), SectionStatus::Default);
// save identity
assert_eq!(session.section_status(OptionalSection::Identity), SectionStatus::Configured);
// switch Local
assert_eq!(session.section_status(OptionalSection::TunnelClient), SectionStatus::NotApplicable);
// switch Standalone again; staged tunnel-client draft, if configured earlier, is restored
```

Also prove a Room draft remains staged when profile switches away from Room and becomes active again when profile returns to Room.

- [ ] **Step 2: Implement Optional Center rendering/focus rules**

Render Identity, Workspace, Confirmation, Limits, Sandbox, Room, Tunnel client overrides, Hub reporting, plus `完成并继续`/`Finish and continue`. Do not remove inapplicable rows; display them as `不适用`/`Not applicable` and skip them in focus traversal.

- [ ] **Step 3: Implement all section forms using existing field semantics**

The forms must cover exactly:

- Identity: display name.
- Workspace: workspace root, write roots JSON list, read-only roots JSON list, deny roots JSON list.
- Confirmation: provider and language.
- Limits: max concurrent tasks, max active jobs (`auto` or integer), max file-search context lines.
- Sandbox: enabled, bubblewrap path, required runtime paths JSON list.
- Room: timezone, diary boundary hour, optional notebook root.
- Tunnel client: optional version, cache dir, auto-download, optional executable/download URL/SHA256.
- Hub reporting: enabled and detail (`metadata`/`full`).

Do not invent MCP/policy/extra runtime settings.

- [ ] **Step 4: Validate sections only through `config_setup`**

On Save, construct the matching `OptionalSectionDraft` and call `SetupSession::save_optional_section(candidate)`. The method validates first and updates staged state only on success. Map returned `SetupField` errors to UI focus/inline text. Pages may localize error codes but must not re-parse numeric/path/config semantics themselves.

- [ ] **Step 5: Add TestBackend coverage for Optional Center and one complex section**

At minimum assert:

- NotApplicable rows are visible but not focusable;
- configured row status changes after save;
- Workspace shows all path-policy fields and an inline JSON error;
- re-entering Workspace displays the previously saved values.

- [ ] **Step 6: Run focused tests and commit**

```bash
cargo test -p agentic-gpt config_setup:: -- --test-threads=1
cargo test -p agentic-gpt config_tui:: -- --test-threads=1

git add crates/agentic-gpt/src/config_setup crates/agentic-gpt/src/config_tui
git commit -m "feat: add optional config tui center"
```


---

### Phase 7: Add Review Return-Editing, Final Commit, Completion, and System Errors

- **Status:** pending

**Files:**
- Modify: `crates/agentic-gpt/src/config_tui/app.rs`
- Modify: `crates/agentic-gpt/src/config_tui/navigation.rs`
- Modify: `crates/agentic-gpt/src/config_tui/pages.rs`
- Modify: `crates/agentic-gpt/src/config_tui/mod.rs`
- Modify: `crates/agentic-gpt/src/config_setup/review.rs`
- Modify: `crates/agentic-gpt/src/config_setup/outcome.rs`
- Test: `config_tui` and `config_setup` module tests

**Interfaces:**
- Finishing Optional Center calls canonical `SetupSession::review_model`; validation failure routes to the owning Basic/Connection/Optional page and first invalid field rather than showing a generic modal.
- Review groups carry `ReviewTarget`; Enter records `ReturnTarget::Review`, opens the target editor, and successful Next/Save returns directly to Review.
- Confirm-and-write consumes the session with `into_wizard_outcome`, canonical-validates again, then calls `commit_wizard_outcome` exactly once.
- Successful commit records `InitSummary`, switches to Completion, and the final `[完成]/[Done]` exits fullscreen.
- System/commit/terminal errors use a blocking modal/error page with a safe localized message/code and no secret.

- [ ] **Step 1: Add failing review-navigation tests**

Cover Basic -> Review return, Connection -> Review return, Optional section -> Review return, and mode change from Review editing that causes the connection review group to update to the newly selected mode.

- [ ] **Step 2: Add failing no-side-effect and one-shot commit tests**

Use a temporary config/secret path and a test commit seam/counter to prove:

- entering/re-entering Review writes nothing;
- Ctrl+C from Review writes nothing;
- validation failure before Review writes nothing;
- Confirm-and-write calls commit once;
- secret marker never appears in rendered Review/Completion/system-error buffers.

- [ ] **Step 3: Render the frontend-neutral Review model**

Show mode/profile, active connection values, optional section statuses/key summaries, config path, backup intent, pending actions, and whether/where a secret will be written. Never format secret content.

- [ ] **Step 4: Implement explicit review return targets**

Do not rely on fixed numeric page order. `ReturnTarget::Review` survives the editor page until successful Next/Save, at which point Review is rebuilt from the updated `SetupSession`.

- [ ] **Step 5: Implement final commit and Completion**

After successful `commit_wizard_outcome`, show config path and only real existing next-step CLI guidance. Do not add a Dashboard/main-TUI command that does not exist. Completion has no `n / n` progress indicator.

- [ ] **Step 6: Implement safe system-error modal/page**

Field validation remains inline. Config/secret/backup/terminal failures become a blocking safe error surface; use stable code/localized safe copy, and never include underlying secret-bearing debug data.

- [ ] **Step 7: Run focused tests and commit**

```bash
cargo test -p agentic-gpt config_setup:: -- --test-threads=1
cargo test -p agentic-gpt config_tui:: -- --test-threads=1

git add crates/agentic-gpt/src/config_setup crates/agentic-gpt/src/config_tui
git commit -m "feat: add config tui review and commit"
```

---

### Phase 8: Switch CLI Routing to Fullscreen TUI and Remove the `inquire` Wizard

- **Status:** pending

**Files:**
- Modify: `crates/agentic-gpt/src/config_cli.rs:967-1045`
- Modify: `crates/agentic-gpt/src/main.rs:1-20`
- Delete: `crates/agentic-gpt/src/config_wizard.rs`
- Modify: `Cargo.toml`
- Modify: `crates/agentic-gpt/Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/agentic-gpt/tests/config_cli.rs`

**Interfaces:**
- `handle_init` branches explicitly into three states: `--non-interactive`, all-three-streams TTY, and bare non-TTY.
- `--non-interactive` calls the unchanged `init_non_interactive` path.
- all-three-streams TTY calls `run_config_tui(config_path, setup_seed_from_args(&args), language)`.
- bare non-TTY returns a localized actionable error and performs no writes.
- Old `PromptRequest`, `PromptAnswer`, `PromptBackend`, `InquirePromptBackend`, sequential `run_wizard`, and `inquire` dependency are absent after this task.

- [ ] **Step 1: Add/adjust black-box tests for the new non-TTY contract before changing routing**

In `crates/agentic-gpt/tests/config_cli.rs`, spawn the binary with piped stdio and bare `config init`; assert non-zero exit, localized actionable text mentioning `--non-interactive`, and absence of config/secret files.

Keep/extend existing tests proving `config init --non-interactive` still succeeds under piped stdio with the same config semantics.

- [ ] **Step 2: Add a pure three-stream TTY decision helper test**

Keep this exact pure helper signature/behavior:

```rust
fn should_use_interactive_init(
    non_interactive: bool,
    stdin_tty: bool,
    stdout_tty: bool,
    stderr_tty: bool,
) -> bool {
    !non_interactive && stdin_tty && stdout_tty && stderr_tty
}
```

Test every false-stream case plus the non-interactive override.

- [ ] **Step 3: Route TTY init to `config_tui` and map Ctrl+C to existing localized cancellation semantics**

Do not print the normal CLI success summary while alternate screen is still active. `run_config_tui` returns only after completion/cancel/error and terminal restoration; after successful completion, preserve the existing scrollback-friendly initialized/config-path line plus pending actions.

- [ ] **Step 4: Remove `config_wizard.rs` and `inquire`**

After all migrated tests are green, delete the module registration and file, remove `inquire.workspace = true` from the crate, remove workspace `inquire = "0.9.4"`, run `cargo update`/normal Cargo resolution as needed, and verify no `inquire`, `PromptBackend`, or `<canceled>` references remain outside historical docs/specs/plans.

Search:

```bash
rg -n "InquirePromptBackend|PromptBackend|PromptRequest|inquire::|<canceled>" crates/agentic-gpt/src Cargo.toml crates/agentic-gpt/Cargo.toml
```

Expected: no matches.

- [ ] **Step 5: Run config CLI regression and crate tests**

```bash
cargo test -p agentic-gpt --test config_cli -- --test-threads=1
cargo test -p agentic-gpt -- --test-threads=1
cargo check -p agentic-gpt
```

Expected: PASS.

- [ ] **Step 6: Commit the migration**

```bash
git add -A Cargo.toml Cargo.lock crates/agentic-gpt
git commit -m "feat: replace config init with fullscreen tui"
```

---

### Phase 9: Harden Layout/Terminal Smoke and Documentation Before Independent Review

- **Status:** pending

**Files:**
- Modify: `crates/agentic-gpt/src/config_tui/pages.rs`
- Modify: `crates/agentic-gpt/src/tui/runtime.rs`
- Modify: `crates/agentic-gpt/src/tui/widgets.rs`
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/configuration.md`
- Modify: `docs/configuration.zh-CN.md`
- Modify: `docs/superpowers/specs/2026-08-07-config-init-fullscreen-tui-design.md` only if implementation exposes a factual correction; do not restyle approved decisions.

**Interfaces:**
- No new product semantics. This task proves the approved behavior survives real terminal operation and documents only implemented commands/keys.

- [ ] **Step 1: Expand TestBackend coverage to every acceptance-critical page/state**

Cover Basic, Standalone Connection, Hub Connection, Optional Center, Workspace (or another complex section), Review, Completion, and system-error surface at a normal size; keep one small-terminal regression ensuring no panic/overflow and accessible primary action.

- [ ] **Step 2: Run a real tmux/PTY smoke on the built binary**

Build first:

```bash
cargo build -p agentic-gpt
```

Then use a disposable HOME/config path inside a real tmux pane and verify manually/through pane capture:

- alternate screen enters/exits cleanly;
- normal Esc on Basic is no-op;
- CapsLock→Esc behaves identically to Esc in the current keyd environment;
- Esc while editing exits editing only;
- Esc on a section returns to Optional Center;
- Ctrl+C cancels from multiple pages and leaves no config/secret side effects;
- terminal accepts normal shell input after cancel/error;
- resize does not lose staged/session/navigation/edit state;
- secret typing is never visible in pane capture;
- successful commit writes valid config and correct secret permissions.

Use a temporary config path and disposable secret path; do not overwrite the live `~/.agentic_gpt/config.json` during smoke.

- [ ] **Step 3: Update English/Chinese documentation**

Document:

- `config init` requires interactive stdin/stdout/stderr for fullscreen mode;
- scripts/CI must use `--non-interactive`;
- interactive flags seed fields but remain editable;
- flow is Basic -> Connection (except Local) -> Optional Center -> Review -> Completion;
- key semantics for Tab/arrows, Enter, Esc, Ctrl+C;
- optional sections can be revisited;
- Review is redacted and can jump back to edit;
- config/secret files are not written until final confirmation;
- no claim of mouse/inline/dashboard/Windows behavior from this feature.

- [ ] **Step 4: Run formatting, tests, check, clippy, and diff validation serially**

```bash
cargo fmt --all -- --check
cargo test -p agentic-gpt -- --test-threads=1
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Expected: all PASS.

- [ ] **Step 5: Perform plan/spec coverage review before the final commit**

Explicitly verify all acceptance criteria 1-15 in `docs/superpowers/specs/2026-08-07-config-init-fullscreen-tui-design.md` map to code/tests. Search production code for forbidden coupling:

```bash
rg -n "ratatui|crossterm|KeyEvent|ConfigPage|TuiState" crates/agentic-gpt/src/config_setup
```

Expected: no matches.

Search TUI pages for canonical config construction/validation duplication:

```bash
rg -n "build_config|validate_standalone|validate_hub|serde_json::from_str|parse::<usize>|parse::<u32>" crates/agentic-gpt/src/config_tui
```

Expected: no business-validation/build calls except the explicit setup use-case boundary; if direct parsing appears in pages, move it back to `config_setup`.

- [ ] **Step 6: Commit documentation/hardening**

```bash
git add README.md README.zh-CN.md docs/configuration.md docs/configuration.zh-CN.md crates/agentic-gpt/src/tui crates/agentic-gpt/src/config_tui
git commit -m "docs: finish fullscreen config setup"
```

---

### Phase 10: Independent Staged Review of the Completed Implementation

- **Status:** pending
- **Type:** read-only review

**Objective:** Review the completed implementation phase-by-phase after Phases 4-9 are finished, preserving the speed of continuous implementation while still obtaining an independent Superpowers-style evidence-based audit.

**Inputs:**
- Approved design spec and this frozen plan.
- Commit ranges for Phases 4-9 from `progress.md`.
- Phase 3 review artifact and any recorded adjudication.
- Current source, tests, documentation, and verification output.

**Reviewer contract:**
- Same read-only rules as Phase 3.
- Only permitted write: `.planning/2026-08-08-config-init-fullscreen-tui/reviews/phase-10-staged-review.md`.
- Review each implementation phase as a distinct section so findings retain provenance even though the review is performed in one pass.
- Do not modify code, tests, docs, plan state, or existing review artifacts; do not repair findings or launch follow-up agents.

**Required review sections:**
- Phase 4 — TUI runtime/widgets.
- Phase 5 — navigation/editing/basic/connection.
- Phase 6 — optional configuration center.
- Phase 7 — Review/return editing/commit/completion/errors.
- Phase 8 — CLI migration and `inquire` removal.
- Phase 9 — terminal smoke, documentation, hardening, and verification.
- Cross-phase integration — coupling, secret safety, terminal restoration, compatibility, stale/dead paths, and regression risk.

**Finding format:** use the Phase 3 format with IDs `R10-001`, `R10-002`, ... and include `Reviewed phase:` for each finding. Suggestions remain explicitly categorized and are not repair requirements by default.

**Completion boundary:** Phase 10 completes when the staged review artifact exists, records the reviewed HEAD/ranges, and either lists evidence-backed findings or explicitly states `No findings.`

---

### Phase 11: Controller Adjudication and Batched Repair

- **Status:** pending

**Objective:** Decide which reviewer findings are real under the frozen contract, then repair only accepted findings as one bounded implementation pass instead of allowing reviewers to mutate the code directly.

**Steps:**
- [ ] Read Phase 3 and Phase 10 review artifacts alongside the exact cited code/spec/plan evidence.
- [ ] For every finding record one disposition in `findings.md`: `accepted`, `rejected`, `duplicate`, `defer`, or `out-of-scope`, with concise rationale.
- [ ] Build a repair checklist only from `accepted` findings; suggestions require an explicit acceptance decision before entering the checklist.
- [ ] If an accepted finding exposes a genuinely unfrozen product decision, stop repair, set workflow stage back to `refining`, implementation authorization to `no`, and re-enter `refine-implementation-plan` before changing code.
- [ ] Otherwise implement the accepted repair batch test-first, run focused verification for each repaired contract, and record commits/results in `progress.md`.
- [ ] Run one scoped re-review limited to accepted repaired findings; write it to `.planning/2026-08-08-config-init-fullscreen-tui/reviews/phase-11-repair-rereview.md`. The re-review remains read-only and may report only whether each accepted finding is resolved or still evidenced.

**Completion boundary:** all review findings have dispositions; all accepted non-deferred findings are repaired and scoped re-review shows no unresolved accepted finding.

---

### Phase 12: Final Cumulative Verification

- **Status:** pending

**Objective:** Prove the repaired final tree still satisfies the complete approved design and repository quality gates.

- [ ] **Step 1: Re-run full deterministic verification serially**

```bash
cargo fmt --all -- --check
cargo test -p agentic-gpt -- --test-threads=1
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Expected: all PASS.

- [ ] **Step 2: Re-run architecture/coupling guards**

```bash
rg -n "ratatui|crossterm|KeyEvent|ConfigPage|TuiState" crates/agentic-gpt/src/config_setup
rg -n "InquirePromptBackend|PromptBackend|PromptRequest|inquire::|<canceled>" crates/agentic-gpt/src Cargo.toml crates/agentic-gpt/Cargo.toml
```

Expected: no matches.

For business parsing/build calls in `config_tui`, inspect any match and require it to be only an explicit `config_setup` use-case boundary; pages must not contain duplicated business parsing/validation.

- [ ] **Step 3: Re-run real disposable-terminal smoke for acceptance-critical terminal behavior**

Verify alternate-screen/raw-mode restoration, Esc/Ctrl+C semantics, resize preservation, secret non-echo, no-side-effect cancellation, and successful config/secret commit against disposable paths. Do not use the live AgenticGPT config/secret files.

- [ ] **Step 4: Close review and plan state**

Confirm every accepted reviewer finding is resolved, every deferred/out-of-scope/rejected finding has rationale, all Final Acceptance Checklist items are satisfied, and update `task_plan.md`, `findings.md`, and `progress.md` consistently.

**Completion boundary:** all final verification passes, review disposition is complete, and no acceptance criterion remains unchecked.

---

## Final Acceptance Checklist

- [ ] Interactive TTY `config init` opens fullscreen TUI with no sequential prompt history or `<canceled>` output.
- [ ] Esc/Ctrl+C semantics match the approved design exactly.
- [ ] Basic groups mode/profile; Connection groups mode-specific fields; Local has no empty Connection page.
- [ ] focused/editing are distinct: focused text fields show `›`, and editing text fields additionally show the live cursor.
- [ ] Optional Center is re-entrant and keeps inapplicable sections visible but disabled.
- [ ] All persistent writes happen only after final Review confirmation.
- [ ] Review can return-edit Basic/Connection/sections and then return directly to Review.
- [ ] Secret contents are absent from review, logs, snapshots, errors, debug, and terminal capture.
- [ ] Interactive flags are editable seeds; non-interactive mode remains strict/direct.
- [ ] Bare non-TTY init errors actionably and writes nothing.
- [ ] Resize, Ctrl+C, ordinary errors, and panic cleanup restore the terminal.
- [ ] `config_setup` contains no Ratatui/Crossterm/TUI navigation dependencies.
- [ ] `config_tui` does not duplicate setup applicability/validation/review/build rules.
- [ ] `inquire` and old PromptBackend flow are removed from production dependencies/code.
- [ ] KMP/Inline TUI/Jobs/Python/Terminal Session remain outside this implementation.
