# Task Plan: Config Schema v2 — Persisted Runtime Selection & Sparse Storage

## Goal
Make the config file authoritative for runtime mode/profile and store only meaningful configuration, while keeping the in-memory `Config` fully materialized for runtime use. Collapse public startup to `agentic-gpt run`, keep all config mutation paths sparse, and make the TUI final preview match the exact redacted JSON that will be written.

## Current Phase
Focused follow-up correction pass — complete

## Phases

### Phase 1: Freeze Config v2 Contract
- [x] Define persisted `mode` (`hub|standalone|local`) and `profile` (`normal|room`) as required top-level config fields.
- [x] Move runtime/profile enums out of CLI-only ownership so serde, config init, runtime dispatch, and clap can share one domain representation.
- [x] Define sparse-storage rules and the boundary between disk config and effective in-memory `Config`.
- [x] Decide old config behavior: v2 files require `mode/profile`; missing fields produce an explicit migration/init error rather than heuristic mode inference.
- **Status:** complete

### Phase 1 refinement decisions (frozen after repository inspection)
- Put `RuntimeMode` and `WorkerProfile` in the shared config/domain layer (not `main.rs` or `config_templates.rs`) with serde/clap-compatible lowercase spellings and helpers for `CapabilityProfile` conversion.
- Add `mode` and `profile` to the effective `Config`; the persisted projection always emits them. `Config::load()` will parse a small sparse/disk representation by overlaying onto `Config::default_config()`, then repair dependent defaults. Existing files missing either selector fail with a stable migration/init error. `load_or_default()` may return a fully materialized v2 default only when the file does not exist.
- Keep one projection API in `config.rs` (effective `Config` plus selected mode/profile, with a redacted variant for preview). `write_config_with_backup()` serializes that projection; callers must not serialize `Config` directly for durable JSON. Unknown flattened `extra` fields are merged into the projection and retained.
- Mode/profile select active validation/use, not storage ownership: explicitly configured `hub`, `tunnel`, and `room` data remain durable even when inactive. Empty maps/lists and values equal to defaults are omitted when reconstructable. `mode`/`profile` are the only unconditional fields.
- Public `run` loads the effective config, validates the selected mode, and dispatches Hub, standalone supervisor, or local runtime. Remove the public `run-as-*` subcommands and their i18n/help/tests. Retain hidden `stdio-worker --profile` as the supervisor protocol’s explicit internal launch data, but make it cross-check the loaded Config selector.
- `config show` remains effective/full JSON (using the loaded materialized `Config`); only durable writes and Config Init preview use the sparse projection.
- `workspaceRoot` is compared with the environment-aware global default and is retained when custom; only `pathPolicy` is compared with `default_path_policy(effective_workspace_root)`. Strict v2 load accepts only canonical `confirmationProvider.channels`; legacy `confirmationProvider.provider` is normalized only by explicit `config import`.
- Mode/profile-inactive `tunnel` and `room` sections are retained through sparse overlay and effective load; runtime validation/use only examines the active section. Top-level `skills` remains a shared runtime section; legacy `room.skills` is rejected by strict v2 load and migrated only by explicit `config import`.
- Selector changes are restart-required for an already-running runtime. Hub/local/standalone reload paths reject mode/profile changes, and the hidden worker cross-checks its explicit profile against Config.

### Phase 2: Sparse Disk Projection & Effective Load
- [x] Introduce one canonical disk projection used by every config write path.
- [x] Always persist `mode` and `profile`; omit unchanged defaults and empty optional sections where semantics are recoverable.
- [x] Handle dependent defaults correctly (especially `pathPolicy` derived from `workspaceRoot`).
- [x] Load sparse JSON by overlaying it onto environment-aware defaults, producing a complete runtime `Config`.
- [x] Preserve unknown/future flattened fields where current compatibility behavior requires it.
- **Status:** complete

### Phase 3: Unified Runtime Entry
- [x] Make public `agentic-gpt run [--config ...]` load `mode/profile` from config and dispatch Hub / Standalone / Local accordingly.
- [x] Make `profile` come from config for public runtime startup.
- [x] Remove or retire public `run-as-room`, `run-as-standalone`, and `run-as-local` entry points; keep `stdio-worker` hidden/internal and pass explicit internal data only where required by the supervisor protocol.
- [x] Ensure runtime validation is mode-aware after config load.
- **Status:** complete

### Phase 4: Keep Every Mutation Path Sparse
- [x] Route `config init`, `config set`, MCP add/remove/update, policy/path-policy mutations, and any other durable config writes through the same sparse serializer.
- [x] Ensure a post-init mutation never expands the file back to a full default snapshot.
- [x] Preserve backup behavior and existing secret-write/rollback transaction semantics.
- [x] Keep `config show` as effective/full runtime config unless repository constraints require otherwise.
- **Status:** complete

### Phase 5: Config Init / TUI Integration
- [x] Make Config Init persist the selected mode/profile into Config v2.
- [x] Make the final “Preview config” pane render the same sparse disk projection that will be committed, with config secrets redacted and transaction-only Tunnel secret material absent.
- [x] Ensure switching mode/profile before commit does not leave irrelevant mode/profile-only sections in the serialized file.
- [x] Update checked-in examples/help/schema-facing copy as needed.
- **Status:** complete

### Phase 6: Compatibility & Verification
- [x] Add/adjust only stable config/runtime contract tests (disk round-trip, sparse projection, unified dispatch, mutation persistence, secret/backup correctness); do not add TUI presentation or interaction regression tests.
- [x] Verify existing config mutation commands and runtime startup paths against Config v2.
- [x] Run fmt/check/full relevant suite and inspect representative generated JSON for Hub/Standalone/Local × Normal/Room.
- **Status:** complete

## Frozen Decisions
- `mode` and `profile` are authoritative config data and are always explicit on disk, even when equal to defaults.
- Public startup should converge on `agentic-gpt run`; mode/profile are not public startup flags in the v2 model.
- Runtime code consumes a fully materialized `Config`; sparse/optional semantics belong at the persistence boundary, not throughout runtime code.
- Sparse serialization is centralized; no caller hand-builds its own reduced JSON.
- `config show` represents effective configuration; the file itself represents explicit durable choices.
- Missing v2 `mode/profile` is not guessed from incidental fields. Return a clear migration/init error.
- `pathPolicy` omission must be evaluated against the default derived from the effective `workspaceRoot`, not only against global defaults.
- TUI preview and durable write must share the same disk projection. Preview additionally redacts secret values.
- Do not add TUI style/layout/interaction tests. Keep tests only for stable persistence/runtime/transaction contracts.

## Scope Boundaries
- Do not redesign unrelated TUI layout or optional-section UX.
- Do not change Tunnel secret transaction semantics beyond adapting config serialization.
- Do not add a general schema-version migration framework unless needed to make this v2 boundary explicit and maintainable.
- Do not preserve legacy public startup commands by default merely for compatibility; prefer a clean 0.9.x schema/runtime break unless implementation evidence shows a concrete dependency.

## Follow-up corrections (current working tree)
- [x] Keep normal v2 load strict and add an explicit `config import` flow for legacy/external JSON.
- [x] Persist Hub data under nested `hub` and preserve explicitly configured inactive `hub`, `tunnel`, and `room` sections.
- [x] Make `run` fail without creating a config when the selected file is absent.
- [x] Show valid `file:`/`env:` tunnel API-key references in redacted Preview while redacting only the Hub secret.
- [x] Add only stable import/persistence/runtime/transaction contract coverage and rerun the full verification suite.
