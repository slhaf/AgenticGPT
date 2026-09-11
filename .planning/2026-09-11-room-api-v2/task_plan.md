# Room API V2 — Task Plan

## Goal

Replace the legacy JSONL Room API and obsolete `room.applyPatch` design with a **local-first, Git-backed Room repository API** that matches the current Hearth/Room world model: deterministic semantic reads plus semantic maintenance requests whose execution backend may be local or workflow-based.

## Current contract

### Repository

- Effective root: `room.repositoryRoot` when configured, otherwise `<workspaceRoot>/room`.
- Room is intrinsically Git-backed. `ensure_repository()` must idempotently make the effective root an independent Git top-level repository; a new Room must never silently degrade into an unversioned directory.
- When Agentic creates a missing or empty Room root, it scaffolds a complete standalone Room control plane from bundled templates, initializes `main`, and creates one deterministic initial commit containing only that scaffold.
- A default scaffold contains:

```text
room.json
Diary/Daily/current.md
Diary/Weekly/current.md
Diary/Monthly/current.md
Notebook/
State/entities/
manual/diary.md
manual/notebook.md
manual/entity.md
maintenance/diary/daily/
maintenance/diary/weekly/
maintenance/diary/monthly/
maintenance/notebook/
maintenance/entity/
scripts/apply_maintenance.py
.github/workflows/apply-maintenance.yml
```

- Empty tracked directories use repository-owned placeholders where needed. `Daily/current.md` is initialized for the current `Asia/Shanghai` logical day; Weekly/Monthly use canonical empty `Summary + Entries` skeletons.
- `room.json` carries the Room repository/control-plane schema version. Initial V2 uses `schemaVersion: 1`.
- `Hearth/` is **not** part of the generic Agentic Room scaffold. Hearth remains an upper-layer project/runtime that may install its own canonical documents into a Room repository.
- Existing files are never silently overwritten by bootstrap. An existing Git Room is validated as-is; missing/outdated control-plane files are reported by status rather than silently upgraded.
- For a non-empty, non-Git existing root, Agentic may `git init -b main` so future changes are versionable, but must not automatically `git add .` or create a baseline commit that captures unknown pre-existing files. Status reports the incomplete/unborn state until it is explicitly baselined.
- Read operations require only the relevant semantic repository content. Local maintenance requires a clean usable local control plane, but does **not** require a remote.
- Symlink/path escape is rejected at semantic boundaries.

### Maintenance execution configuration

Room maintenance is local-first. Configuration gains:

```text
room.maintenance.mode = "local" | "workflow"   # default: local
room.maintenance.autoPush = false               # default: false
```

- `mode=local`: semantic maintenance is validated and applied against the local Room checkout immediately; the resulting Markdown change is committed locally before `room.maintenance.submit` returns.
- `mode=workflow`: the request is committed and pushed, and the remote repository workflow becomes the writer; this mode requires `origin` and the workflow control plane.
- `autoPush` applies to successful **local** execution only. If enabled and a usable remote exists, Agentic synchronizes/pushes the resulting local history. A push/sync failure does not roll back an already-successful local maintenance; the response reports local `applied` plus remote sync failure separately.
- `workflow` mode inherently requires push, so `autoPush` does not govern it.
- The submit tool may optionally override the configured execution mode for one request; omitting it uses config.

### Public Room tool surface

The `room` optional toolset advertises the following Room data tools, in addition to existing Room bootstrap tools:

```text
room.diary.active
room.diary.read
room.notebook.recent
room.notebook.search
room.notebook.read
room.state.list
room.state.read
room.maintenance.status
room.maintenance.submit
```

Legacy JSONL CRUD and `room.applyPatch` are removed from the advertised Agent surface.

### Diary reads

`room.diary.active` returns the exact active temporal files in one call:

- `Diary/Daily/current.md`
- `Diary/Weekly/current.md`
- `Diary/Monthly/current.md`

Missing layers are represented explicitly rather than failing the entire response.

`room.diary.read` performs deterministic exact temporal reads:

- `layer=daily`, `period=current | YYYY-MM-DD`
- `layer=weekly|monthly`, `period=current | YYYY-MM-DD--YYYY-MM-DD`

Archive paths are derived mechanically from the period and layer; arbitrary paths are not accepted.

### Notebook reads

- `room.notebook.recent`: bounded previews ordered by effective Git/working-tree recency.
- `room.notebook.search`: bounded case-insensitive substring search over path, H1 title, and body.
- `room.notebook.read`: exact full Markdown read of one path returned/discovered under `Notebook/`; no arbitrary repository path.

### State reads

- `room.state.list`: deterministic `State/entities/*.md` discovery.
- `room.state.read`: exact full Markdown read by entity filename stem.

### Maintenance control plane

`room.maintenance.status` reports repository/slot state without mutating it: root, branch/head, cleanliness, and the five known maintenance slots.

It also reports the operational capabilities that matter to callers: repository schema version, scaffold/control-plane readiness, local executor readiness, configured/default maintenance mode, `autoPush`, remote/workflow availability, and local/remote sync state when it can be determined without destructive action.

`room.maintenance.submit` accepts 1..5 unique requests. Each item has:

```text
slot: diary.daily | diary.weekly | diary.monthly | notebook | entity
payload: JSON value defined by the Room repository's current manual/workflow
```

Optional `waitSeconds` is bounded to 0..30.

`room.maintenance.submit` is a semantic request API, not a synonym for GitHub submission. Both backends consume the same five maintenance slot payloads and the same repository-owned maintenance engine.

Common preflight:

1. serialize through one Agent-owned Room repository write lock;
2. require a clean, usable local Room repository and a supported `room.json` schema;
3. fail if any requested maintenance slot is occupied;
4. stage the exact request set in a disposable detached worktree/copy and execute that checkout's `scripts/apply_maintenance.py` as validation/preflight; cleanup occurs on every path.

Local backend:

5. if `autoPush=true` and a usable upstream exists, fetch and require fast-forwardable local `main` before applying; if auto-push is disabled, local execution performs no network access;
6. write the requested `maintenance/**/maintenance.json` files to the real checkout and run `scripts/apply_maintenance.py` locally;
7. the executor consumes the request files and updates semantic Markdown; commit the resulting repository diff as one bounded Room-maintenance commit;
8. return local `state=applied` immediately after the commit succeeds;
9. when auto-push is enabled, attempt the remote push and report sync outcome independently. Remote failure never rewinds the successful local commit.

Workflow backend:

5. require `origin`, `main`, and workflow readiness; fetch/pull `origin/main` with fast-forward-only semantics before submission;
6. write only the requested maintenance files, commit the request, and push it to `origin main`;
7. if `waitSeconds > 0`, poll `origin/main` until the submitted maintenance files are consumed by the workflow; when observed, fast-forward the local checkout and return `state=applied`; timeout returns `state=submitted`, not a false failure.

The Agentic API does not duplicate the Room repository's semantic validation rules. Both local execution and remote workflow execution use the repository-owned `scripts/apply_maintenance.py`; `manual/` + executor remain the mechanical authority, while `.github/workflows/apply-maintenance.yml` is only one execution transport.

## Explicit non-goals

- No JSONL migration or compatibility layer.
- No `room.applyPatch` compatibility path.
- No direct Diary/Notebook/State CRUD mutation tools.
- No generic Room file reader; generic files remain the responsibility of `file.*`.
- No silent control-plane upgrades of an existing Room repository. New/empty Room creation does scaffold the V2 control plane by design.
- No automatic import/baseline commit of unknown files from an existing non-empty unversioned directory.
- No Hub HTTP/OpenAPI surface migration in this workstream; Hub parity is a separate branch/workstream after optional toolsets.
- No automatic Project Sources/Hearth mirror synchronization.

## Implementation phases

### Phase 1 — Contract/config/runtime foundation

- [x] Replace `room.notebookRoot` with optional `room.repositoryRoot` in config, CLI/init/review/docs.
- [x] Add `room.maintenance.mode` (`local` default / `workflow`) and `room.maintenance.autoPush` (`false` default) to config, CLI/init/review/docs.
- [x] Add versioned Room scaffold assets (`room.json`, temporal skeletons, manuals, maintenance layout, executor, workflow) and a deterministic new/empty-root bootstrap path.
- [x] Add `room_repository` runtime module and repository bootstrap/validation/read helpers, including independent `git init -b main`; reuse reviewed code from the frozen old branch where semantics still match.
- [x] For Agentic-created new/empty repositories, commit only the deterministic scaffold as the initial history. Never baseline unknown pre-existing files automatically.
- [x] Keep repository initialization, local-executor readiness, workflow readiness, and remote-sync readiness as distinct status concepts.
- [x] Add Agent-owned Room repository write mutex.
- [x] Add protocol request/response types required by the new Agent-side commands while keeping Hub compatibility compile-safe.

### Phase 2 — Read surface

- [ ] Implement active/exact temporal reads for Daily/Weekly/Monthly current layout.
- [ ] Implement Notebook recent/search/read.
- [ ] Implement State list/read.
- [ ] Replace advertised stdio tool descriptors/schemas/descriptions and local dispatch.
- [ ] Ensure live optional-toolset enable/disable exposes/removes the complete V2 surface without restart.

### Phase 3 — Maintenance status/submit

- [ ] Implement status and slot mapping.
- [ ] Implement serialized clean-repository and occupancy checks.
- [ ] Implement repository-owned `scripts/apply_maintenance.py` preflight in a disposable checkout with cleanup on all paths.
- [ ] Implement local backend: write request → local executor apply/consume → one local semantic commit → optional best-effort/explicit remote sync.
- [ ] Implement workflow backend: fast-forward sync → request commit/push → bounded workflow-consumption wait/pull.
- [ ] Ensure local apply success and remote push status are represented independently; never roll back a successful local semantic commit solely because remote sync failed.
- [ ] Add safe structured responses/errors; never include secrets or unbounded Git stderr.

### Phase 4 — Legacy Agent cleanup

- [ ] Remove legacy Room JSONL tool advertisements and Agent-side dispatch paths.
- [ ] Remove now-unused `diary.rs` / `notebook.rs` runtime dependencies when no Agent path references them.
- [ ] Keep only the minimum protocol/Hub compatibility residue required for the separate Hub parity branch; document it explicitly.
- [ ] Update Room toolset descriptions/count/surface tests and standalone/config docs.

### Phase 5 — Verification

- [ ] Unit tests for repository root/path/symlink/period parsing and deterministic reads.
- [ ] Bootstrap tests: absent root, empty root, existing Git root, non-empty non-Git root, no overwrite, deterministic initial scaffold commit, schema/version detection.
- [ ] Maintenance preflight invalid payload, occupied slot, dirty repo, local apply, local auto-push off/on, missing remote, push failure after local success, workflow missing origin, workflow wait timeout/applied paths.
- [ ] Real disposable local Room repository smoke using the scaffolded repository-owned maintenance executor.
- [ ] Workflow smoke verifies `.github/workflows/apply-maintenance.yml` invokes the same repository-owned executor path rather than a separate implementation.
- [ ] Live toolset toggle smoke on one running Local Unix Agent.
- [ ] `cargo fmt --all -- --check`, `cargo check --workspace`, focused tests, full workspace tests, `git diff --check`.

## Completion boundary

Implementation is complete when the Agent-side Room optional toolset exposes only the V2 semantic surface, all mutations flow through maintenance submission, real current-layout Room fixtures pass read/submit smoke tests, and no advertised Agent tool depends on the legacy JSONL world model or `room.applyPatch`.
