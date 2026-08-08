# Phase 3 Domain-Foundation Review

- Reviewed commit range: `ba038e7..6a1ee0a`
- Reviewed HEAD: `6a1ee0acb8fd24fdc3368bccd700d85b46c18347`
- Scope: Phase 1-2 `config_setup` extraction, structured validation, redacted review model, and secure outcome/commit handoff.
- Review mode: read-only. No product code, tests, configuration, documentation, or planning authority file was changed during this review.

## Review evidence

- `config_setup` contains no Ratatui, Crossterm, terminal-event, page-index, cursor, or navigation dependency.
- `SetupSession` retains independent Standalone and Hub drafts, filters optional sections through the canonical `optional_section_is_legal` helper, and builds only the selected mode into `InitInput`.
- Review construction calls the staged active-input builder and canonical `build_config`, emits only redacted/reference metadata, excludes inactive connection data, and computes optional-section status from applicability plus staged presence.
- `SetupSession::into_wizard_outcome` revalidates before constructing the write plan. The outcome writer retains the existing parent/file permissions, atomic temporary-file rename, symlink/target checks, config-write rollback, and stable error-code behavior.
- Verification at the reviewed HEAD passed: focused `config_setup` tests (14), legacy `config_wizard` tests (28), `config_cli` integration tests (14), `cargo fmt --all -- --check`, and `cargo check -p agentic-gpt`.

## Findings

### R3-001 — Review pending-action semantics lack a direct regression assertion

- Severity: low
- Category: test-gap
- Claim: The review test suite does not assert that `ReviewModel.pending_actions` preserves canonical pending actions or removes `ProvisionTunnelSecret` when an immediate secret write plan is present.
- Evidence: `crates/agentic-gpt/src/config_setup/review.rs:174-180` clones `built.pending` and conditionally removes `PendingAction::ProvisionTunnelSecret`, but the tests at `crates/agentic-gpt/src/config_setup/review.rs:12-35` assert redaction and `secret_write` only; they never inspect `review.pending_actions`. The Phase 2 contract explicitly requires pending-action coverage in `task_plan.md:307-319` and the review model contract requires pending actions in `task_plan.md:329-333`.
- Expected contract: Review must expose canonical pending actions, with an immediate secret write represented by `secret_write` rather than a contradictory deferred-provision action.
- Impact: A later refactor could drop, reorder, or incorrectly retain pending actions while the current review tests still pass, causing the frontend to report an incomplete or contradictory completion plan.
- Suggested direction: Add focused assertions for (1) default/deferred standalone setup retaining `ProvisionTunnelSecret`, (2) immediate secret provisioning removing that action while exposing `ReviewSecretWrite`, and (3) Hub placeholder actions remaining visible.

## Disposition

No high-severity load-bearing foundation finding was identified. The single low-severity test-gap is advisory and does not block Phase 4; it should be considered during the later hardening/adjudication pass.

