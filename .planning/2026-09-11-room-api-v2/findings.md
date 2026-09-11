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
