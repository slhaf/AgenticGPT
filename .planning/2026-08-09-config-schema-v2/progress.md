# Progress: Config Schema v2

## 2026-08-09
- Opened a new scoped planning session after completing Config Init Review v2.
- Inspected current `Config`, `Config::default_config()`, `Config::load()`, `build_config()`, `write_config_with_backup()`, config mutation write sites, current CLI startup commands, and `WorkerProfile` ownership.
- Frozen direction: persist authoritative mode/profile, converge public runtime startup on `agentic-gpt run`, centralize sparse disk projection, load sparse files into full effective Config, keep all mutation paths sparse, and make TUI preview equal the redacted durable projection.
- User requested implementation be handed off to Codex Luna Max after planning is ready.
- Repository refinement completed: the shared selector types will live in `config.rs`; sparse writes will be centralized in `write_config_with_backup()` plus a reusable projection API; load will deep-overlay sparse JSON onto environment-aware defaults and reject existing files missing `mode`/`profile`; unified `run` will dispatch after selector-aware validation; hidden `stdio-worker` keeps explicit supervisor profile data and cross-checks it.
- Phase 1 and all implementation phases are complete; final workspace-level verification passed. Existing public-command tests/docs and local/standalone integration launchers were updated to the unified `run` contract.

## Implementation and verification update
- Added shared serde/clap `RuntimeMode` and `WorkerProfile` selectors to effective `Config`, required them on existing files, and added stable migration/config-init errors for missing or invalid selectors.
- Implemented deep sparse load/projection in `config.rs`: environment-aware defaults, workspace-dependent path-policy repair, unknown flattened-field retention, strict canonical v2 shapes, inactive tunnel/room retention, secret redaction, and durable backup writes through one projection boundary.
- Unified public startup on `agentic-gpt run`; mode dispatches Hub/Standalone/Local, profile comes from Config, public `run-as-*` commands/help entries were removed, and hidden `stdio-worker` remains an explicit supervisor protocol with selector cross-checks. Running selector changes are restart-required.
- Updated Config Init templates, CLI `config set mode/profile`, sparse TUI preview, checked-in example, docs, and stable config/runtime/transaction tests. No TUI presentation or interaction tests were added.
- `cargo fmt --all -- --check` and `cargo check -p agentic-gpt` pass. The agentic-gpt unit binary suite passes 299 tests. With a short writable `HOME=/tmp/h` (and existing Rust/Cargo homes), `config_cli` (14), `local_control` (1), and `standalone_supervisor` (6) integration tests pass.
- A first integration attempt with `HOME=/tmp/agentic-gpt-test-home` failed only because the generated Unix socket path exceeded the platform path-length limit; rerunning with `/tmp/h` passed. An earlier multi-filter `cargo test` invocation was invalid Cargo CLI usage and was replaced with valid single-filter/target commands.
- Final verification passed with `HOME=/tmp/h RUSTUP_HOME=/home/slhaf/.rustup CARGO_HOME=/home/slhaf/.cargo cargo test --workspace`: 299 agentic-gpt unit tests, config CLI (14), Local runtime (1), standalone supervisor (6), Hub (59), protocol (12), and doc tests. `cargo check --workspace`, formatter check, and `git diff --check` also pass.
- Representative Config Init projections for all six mode/profile combinations were inspected. Standalone writes selectors plus tunnel fields; Hub writes selectors plus Hub fields; Local writes selectors while explicitly configured inactive Hub/tunnel/room data remains durable.
- The CLI round-trip contract now explicitly verifies that the on-disk file stays sparse while `config show` reconstructs effective defaults.

## Focused follow-up correction pass
- Re-read the active plan and current uncommitted diff before editing. The corrections retain the existing Config Init/TUI and transaction implementation; they narrow changes to schema shape, persistence semantics, explicit migration, runtime missing-file behavior, and stable contract tests.
- Changed `Config` to persist `hub: { url, transport, agentSecret }`; strict load rejects old top-level Hub names and explicit import maps them with warnings for fields that cannot be materialized. Added nested Hub key-registry entries and updated runtime consumers.
- Removed mode/profile-based deletion of inactive `hub`, `tunnel`, and `room` data from load/projection. Added round-trip tests for inactive sections, unknown flattened fields, legacy Hub import, invalid recognized import fields, and the imported-base/TUI overlay.
- Added `config import [SOURCE]` (SOURCE defaults to the selected `--config` path), which seeds the existing interactive Config Init TUI and commits through the existing backup/secret transaction. Imported optional values seed Review while non-editor fields remain on the imported base.
- Changed public `run` and the standalone supervisor to return `config_missing: run config init first` without creating a config file. Added an integration regression test.
- Preview now redacts only `hub.agentSecret`; normal `file:`/`env:` tunnel API-key references remain visible. Added a stable sparse-projection assertion.
- Import now reports and clears invalid/plaintext `tunnel.apiKey` values before the imported base reaches the TUI, so an inactive-section rewrite cannot carry secret material into JSON.
- Tightened the final v2 boundary after review: normal load rejects legacy `confirmationProvider.provider` and `room.skills`; `config import` canonicalizes both, uses Init defaults for missing selectors instead of heuristic inference, and preserves inactive imported data.
- Fixed sparse projection for inactive saved tunnels so their reconstructable `client`/`hubReporting` defaults do not expand on unrelated mode changes.
- Canonicalized confirmation configuration on ordered `confirmationProvider.channels`: legacy provider labels remain import/compatibility inputs only, `config set` exposes the ordered array directly, and Config Init/Review now use a reusable ordered multi-select Form Kit component whose selection order is fallback priority.
- Updated checked-in example, README/configuration docs, interfaces/runtime references, CLI localization, and config CLI tests for nested Hub/import behavior.
- Final verification: `cargo fmt --all -- --check`, `git diff --check`, `cargo check --workspace`, and `HOME=/tmp/h RUSTUP_HOME=/home/slhaf/.rustup CARGO_HOME=/home/slhaf/.cargo cargo test --workspace` all pass. The full run covers 307 agentic-gpt unit tests, 15 config CLI tests, Local runtime, standalone supervisor, Hub, protocol, and doc tests.
