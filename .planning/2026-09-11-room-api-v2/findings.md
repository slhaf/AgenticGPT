# Room API V2 — Findings

## Source of truth reviewed

- Current `heyheris/room` checkout and maintenance workflow.
- `manual/diary.md`, `manual/notebook.md`, `manual/entity.md`.
- `Hearth/procedures/procedure__room-continuity-execution.md` and lifecycle hooks.
- Frozen old `refactor/room-markdown-repository` branch as implementation reference only.
- Current Agent `refactor/room-api-v2` after optional toolsets commit `23a46c5`.

## Key findings

- The current Room repository is no longer a JSONL notebook/diary store. Daily/Weekly/Monthly temporal Markdown, topic Notebook Markdown, and State entity Markdown are the active model.
- Normal writes no longer use `.changes` or `room.applyPatch`; five fixed maintenance request slots already form a clean semantic request boundary.
- `procedure__room-continuity-execution.md` currently requires callers to manually use generic file APIs plus Git. A dedicated `room.maintenance.submit` can safely encapsulate the mechanical repository operation without taking over semantic content decisions.
- For a permanently available Room device, requiring every maintenance request to push to GitHub, wait for Actions, and pull back before local state changes is an unnecessary remote round-trip. Room should remain fully functional and historically versioned while offline.
- The maintenance request is the semantic API boundary; local execution versus GitHub workflow is an execution strategy. These concepts should not be conflated.
- The repository-owned maintenance executor is the best validation authority. Move the canonical executor out of a GitHub-specific path to `scripts/apply_maintenance.py`; both Agentic local mode and `.github/workflows/apply-maintenance.yml` should invoke that same implementation.
- The old branch's repository-root validation, Notebook recency/search, State list/read, symlink discipline, and helper structure are reusable. Its Diary filename parser and `room.applyPatch` mutation are obsolete.
- The old branch was right to make Room an independent Git top-level repository. New/empty Room creation should go further and scaffold the complete generic Room control plane so local maintenance works immediately; this is safe because Agentic knows exactly which files it created and can commit only those files as deterministic initial history.
- Existing non-empty unversioned directories are different: `git init` is safe, but silently committing unknown existing content is not. Bootstrap must not confuse repository initialization with user-authorized baselining.
- Generic Room scaffold belongs to Agentic (`Diary`, `Notebook`, `State`, `manual`, `maintenance`, executor, workflow, schema metadata). Hearth-specific runtime documents do not; Hearth remains an upper layer installed separately.
- Remote sync is orthogonal to local maintenance. Local mode defaults to no network; optional auto-push may sync after local apply. A sync failure must be observable without reclassifying or rolling back the already-successful local semantic write.
- Current Agent config still has `room.notebookRoot`; it is legacy and should become `room.repositoryRoot` without runtime compatibility aliasing in this refactor.
- Optional toolsets are now live/dynamic. The V2 Room surface should be selected by `ToolNamespace::Room`, independent of Normal/Room profile, while existing profile/runtime compatibility remains a separate concern.
- Hub parity is intentionally excluded to avoid conflicting with the separate `refactor/hub-toolset-parity` worktree.

## Phase 1 implementation mapping

- Worktree starts clean at `d1daa6c`, immediately after optional toolsets `23a46c5`; neither commit requires rewriting.
- No worktree-local `.codegraph/` index exists, so Phase 1 discovery uses targeted repository search/read.
- Current `RoomConfig` still carries `notebook_root`, timezone, diary boundary, and mirrored skills. Phase 1 must replace only the root field and add nested maintenance configuration without inheriting the legacy JSONL Diary/Notebook storage behavior.
- Config integration spans `config.rs`, `config_cli.rs`, `config_templates.rs`, `config_setup/{model,validation,review}.rs`, `config_tui/{app,pages}.rs`, example config, and bilingual configuration documentation.
- Existing `AppState.notebook_writes` is the legacy write mutex. The V2 foundation needs a repository-wide Room mutex because all future semantic maintenance shares one Git repository transaction boundary.
- Existing Agent protocol still exposes legacy Room commands. Phase 1 may add compile-safe V2 request/response types, but advertised/dispatch cutover remains frozen for Phase 2/4.
- Current toolset namespace mapping remains the pre-V2 surface until Phase 2; Phase 1 must not expose dispatch-only aliases or partially advertise V2 commands.
- The reference branch has a single `room_repository.rs`. Reusable pieces are effective-root expansion, independent top-level Git validation, missing-ancestor canonicalization, symlink-safe layout walking, Markdown enumeration, bounded previews, Git recency, and repository-relative paths.
- The reference branch's `parse_diary_filename`, JSONL-era request types, and entire `apply_patch` implementation are explicitly excluded. Phase 1 repository bootstrap must instead create the frozen `Daily/Weekly/Monthly current.md` scaffold plus repository-owned maintenance control plane.
- No canonical Room checkout containing `manual/diary.md` or `scripts/apply_maintenance.py` is present under `/home/slhaf/Projects`; bundled Phase 1 assets must implement the frozen generic contract directly rather than copying an unavailable external checkout.
- `RoomConfig` currently defaults legacy `notebook_root=None`; `AppState` currently has only `notebook_writes`. Phase 1 can add `repository_root`, nested defaulted maintenance config, and a separate `room_repository_writes` mutex without removing legacy runtime fields/modules before Phase 4.
- `AppState` is constructed directly by many focused test fixtures outside `main.rs`; adding the frozen repository mutex therefore requires mechanical initialization updates in those files. This is an implementation ownership expansion, not a contract conflict; no fixture behavior needs to change.
- Integrated repository foundation exposes effective root/path/read helpers, exact scaffold path staging, fixed local Git identity/date/signing controls, and independent readiness fields. New/empty roots scaffold; existing Git roots are inspection-only; non-empty non-Git roots are initialized without staging user content.
- The bundled scaffold is 16 tracked files including scoped `.gitkeep` placeholders. The runtime checks workflow readiness by verifying that the workflow references `scripts/apply_maintenance.py`; public dispatch remains unchanged for Phase 1.
- No Rust language server is configured in this worktree, so symbol diagnostics/refactors must be verified through compiler/test coverage and targeted repository searches.
- Scaffold manuals define bounded closed payloads: diary summary plus up to 128 text/tag entries, one bounded Notebook path/title/body document, and one bounded entity/content document. Daily current carries the injected Asia/Shanghai logical date; Weekly/Monthly use the required Summary + Entries skeleton.
- The bundled workflow invokes only `python3 scripts/apply_maintenance.py`, then stages `Diary`, `Notebook`, `State`, and consumed `maintenance` files. This preserves one repository-owned semantic implementation for future local/workflow backends.
- The first focused Agent failure is in generic import field isolation, not the frozen Room contract: after normalizing legacy `room.skills`, the per-key validation loop reuses an `original` object and may classify top-level `skills` as unusable when another new Room field makes the isolated candidate invalid. The fix must preserve the existing top-level-skills precedence rather than weakening Room config validation.
- Integration review found repository bootstrap was initially keyed to `CapabilityProfile::Room`. That would violate the already-frozen optional-toolset independence when a Normal profile enables the live `room` namespace. Bootstrap now keys exclusively to `config.toolsets.is_enabled(Room)`; profile remains unrelated.
- Pre-commit review found and fixed scaffold executor confinement defects: Notebook prefix stripping wrote at repository root, and diary/entity/maintenance paths checked only the leaf while allowing symlinked ancestors. The shared helper now retains/walks the full semantic prefix, rejects backslashes, and all fixed semantic paths use it.
- Readiness inspection also now routes local executor and workflow paths through the repository path/symlink validator; symlinked ancestors report `Unavailable` rather than a false `Ready`.
- The review's concern that `git -C <nested> init -b main` might reinitialize a parent repository was disproven by a disposable nested-Git experiment: Git created `<nested>/.git` and reported the nested directory as top level.
- Phase 3 starts with protocol request/response structs already present; no HubCommand variant is required for the frozen Agent-side direct stdio surface.
- Existing `RepositoryStatus` already separates repository, schema, scaffold/control-plane, local executor, workflow, remote, and sync readiness; Phase 3 maps these to status output and derives missing scaffold paths.
- The bundled executor accepts exactly one `maintenance.json` per known slot, validates bounded payloads, writes semantic Markdown, and removes consumed requests. Local and workflow paths must invoke this same script.
- `room_repository_writes` is already present on `AppState` and intentionally marked for Phase 3; full submit serialization must hold it across preflight, apply, commit, and any configured sync.
- The frozen workflow asset currently triggers on maintenance request pushes and invokes `scripts/apply_maintenance.py`; Agent workflow mode must synchronize `main` fast-forward-only and never duplicate semantic validation.
- Review hardening: preflight now uses `git worktree add --no-checkout`, filter-free `git archive` extraction, and bounded `tar` materialization, so repository smudge filters cannot perform offline-local network work.
- Review hardening also bounds Git/executor process groups, cleans partial request writes and failed workflow staging, refreshes repository readiness after fast-forward sync, and records idempotent local applications with an empty semantic commit.

## Phase 4 legacy boundary

- The standalone Agent now has no legacy Room JSONL modules or execution path: `diary.rs`, `notebook.rs`, their write lock, and the old local operation handlers were removed.
- Legacy `HubCommand` protocol variants and the Hub HTTP/MCP/OpenAPI forwarding surface remain only as explicit compatibility residue for the separate Hub parity workstream; the Agent accepts those inbound variants only to return `room_legacy_surface_removed`.
- The advertised standalone Room preset is V2-only: 23 Normal names and 34 Room names, with nine semantic Room tools plus the two bootstrap aliases.
- Standalone/runtime, README, interface, configuration, and contract-matrix documentation now describe V2 reads and maintenance submission. The matrix retains legacy names only under the Hub compatibility section.
- Phase 4 focused verification passed: `cargo fmt --all`, `cargo check -p agentic-gpt`, all 45 `cargo test -p agentic-gpt room_` tests, the exact Room surface contract test, and no remaining `notebook_writes` or Agent legacy-module references.

## Phase 5 verification

- Repository coverage now proves empty-root bootstrap creates the exact scaffold, two independent bootstraps share the deterministic initial commit, schema metadata distinguishes outdated from invalid, and workflow configuration points at the repository-owned executor.
- Read coverage proves active/exact Diary results are repeatable, missing layers report stable unavailable details without mutation, State listing is sorted, and oversized Markdown is rejected.
- Maintenance coverage exercises occupied slots, dirty/local preconditions, auto-push disabled/enabled, absent and failed remotes after local success, workflow timeout preservation, and a worker-applied workflow request.
- The disposable Local Unix smoke toggles the live `room` namespace without restart, exposes V2 tools only while enabled, rejects legacy JSONL names, and executes `room.diary.active` against the bootstrapped repository.
- The workflow asset and repository test share `python3 scripts/apply_maintenance.py`; no second semantic executor is introduced.
- Phase 5 verification passed: focused repository/read/maintenance suites (15/7/11 tests), Local Unix integration (1 test), workspace check, 452 workspace tests across 9 suites, rustfmt check, and `git diff --check`.
