# Room API V2 — Progress

- 2026-09-11: optional toolsets completed, independently reviewed/fixed, verified with 433 workspace tests, and committed as `23a46c5 feat: add configurable optional toolsets`.
- Reviewed current Room repository manuals/workflow and the complete Hearth hook/procedure set.
- Reviewed frozen `refactor/room-markdown-repository` implementation to identify reusable repository-read code versus obsolete old-world mutation/Diary semantics.
- Refined V2 after architecture discussion: Room is local-first. New/empty roots receive a complete versioned generic scaffold and deterministic initial commit; existing unknown content is never silently baselined. `room.maintenance.submit` remains the semantic request boundary, while execution is configurable as local (default) or workflow. Local execution applies/commits immediately and remote auto-push is optional; workflow execution uses push + remote writer. Remote failure never rolls back a successful local apply.
- Canonical maintenance executor should live at `scripts/apply_maintenance.py` and be shared by local Agentic execution and the GitHub workflow; `.github` is transport, not the semantic implementation.
- Next: hand the frozen plan to OMP for Phase 1 implementation; no product code has been modified in this V2 workstream yet.
- 2026-09-11 Phase 1 implementation slices landed: config/init/TUI/docs, Room repository/scaffold/runtime mutex, and compile-safe V2 protocol types.
- Integration attempt 1: `cargo fmt --all` found an unclosed `safe_path_display` delimiter left by the config slice. Restored the pre-existing home-relative path formatting body; no contract change.
- Integration attempt 2: `cargo check --workspace` found three compile errors: ambiguous `File::by_ref`, non-static `anyhow!(code)`, and an incomplete Room setup match omitting MCP/toolsets. Python syntax validation for the scaffold executor passed. Sent each compiler diagnostic to its owning slice for a minimal repair.
- Focused protocol tests passed (3/3). The Room-focused Agent suite passed 29/30; the sole failure was the pre-existing import precedence contract (`top-level skills` should beat legacy `room.skills`) returning default `maxFiles=256` instead of explicit `11`. Investigating the config slice merge before further verification.
- Focused failure root cause was an accidentally dropped `#[serde(rename_all = "camelCase")]` on `RoomSkillsConfig`; restored it. The exact failed test then passed and the full Room-focused Agent set passed 30/30.
- Cleanup edit initially inserted a duplicate `mod room_repository` and placed a dead-code annotation on `skills_writes` because stale anchors were remapped; immediately re-read both files and corrected the duplicate/annotation placement before validation.
- Phase 1 real smoke used a Normal-profile Local runtime with the `room` toolset explicitly enabled. Startup created `<workspaceRoot>/room` as an independent clean `main` repository with exactly one commit and exactly the 16 frozen scaffold files, no `Hearth/`.
- Smoke metadata was `schemaVersion: 1`; `Diary/Daily/current.md` carried logical date `2026-09-11` and the Asia/Shanghai skeleton. Running the scaffolded repository-owned `scripts/apply_maintenance.py` with no occupied slots completed successfully without changing the checkout.
- Focused verification: Room V2 protocol serde tests passed 3/3; Agent Room/config/repository focused tests passed 30/30.
- Final verification after review fixes: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test --workspace` (448 passed across 9 suites), Python executor syntax compilation, and `git diff --check` passed without warnings/errors.
- Runtime bootstrap now follows the live Room toolset rather than profile, so a Normal-profile worker with `room` enabled receives the V2 repository foundation; disabling the namespace avoids bootstrap.
- Temporary runtimes, Room repositories, isolated HOME directories, and Python bytecode artifacts were removed.
- Independent pre-commit review found two real executor path-confinement defects and one readiness symlink false-positive. Fixed all three without changing the frozen contract; added a repository readiness regression.
- Executor behavior smoke now proves `Notebook/topic.md` is written only below `Notebook/`, consumes its request, and rejects a symlinked `Notebook` ancestor with exit code 2. All associated temporary fixtures were removed.
- Two later line-anchored edits landed at stale positions in the concurrently changing executor/repository files; each tool warning was followed by an immediate targeted read and correction before validation. No malformed intermediate source was tested or committed.
- Independent reviewer re-read the corrected source and marked all eight frozen Phase 1 items pass with no remaining blocking contract defect.
- Final actual smoke rebuilt the binary, started a Normal-profile Local worker with only the live `room` toolset enabled, observed one clean initial commit, executed the repository-owned maintenance engine, and confirmed the scaffold workflow has both manual dispatch and `main` maintenance-request push triggers.
- Phase 2 read surface landed: semantic Diary active/exact reads, recursive Notebook recent/search/read, and deterministic State list/read are implemented against the validated Room repository layout.
- Stdio descriptors and local dispatch now expose only the V2 Room read surface when the Room namespace is enabled; legacy Room JSONL names are not advertised.
- Focused checks passed: Room read tests (4/4, including lexical-vs-recency ordering and dotted State list→read round-trip), live toolset authorization test (1/1), and standalone supervisor tests (6/6).
- Full verification passed after the read-surface corrections: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test --workspace` (452 passed across 9 suites), and `git diff --check`.
- Disposable Local Unix smoke passed without restart: enabling `room` exposed `room.diary.active`, `room.notebook.search`, and `room.state.read`; disabling it removed the Room tools and rejected a Room call; re-enabling restored them and a notebook search returned the expected document. Temporary runtime/config/fixture files were removed.
- Notebook recency now uses Git commit time for clean tracked files and working-tree mtime for dirty/untracked files, with mtime fallback when Git metadata is unavailable.
- 2026-09-11: Phase 3 maintenance status/submit landed with five-slot mapping, serialized repository/occupancy preconditions, disposable executor preflight, local semantic commit/remote outcome separation, workflow request submission, and bounded consumption wait/pull.
- Local and workflow Git mutations now suppress repository hooks; Git/executor subprocesses use bounded wall-clock waits, process-group cleanup, bounded/discarded output, and cleanup fallback for every disposable-checkout path.
- Strict nested maintenance request decoding and deterministic public descriptor cases were added. Idempotent local payloads still produce the required bounded maintenance commit via `--allow-empty`.
- Focused verification passed: maintenance behavior tests (7 passed), deterministic public dispatch corpus (1 passed), frozen descriptor test (1 passed), and strict nested decoding test (1 passed).
- Full verification passed: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test --workspace` (459 passed across 9 suites), and `git diff --check`.
- Integration note: the initial delegated core slice stalled and left two compile regressions in the repository helper; the constant and schema arm were restored before verification, with no contract change.
- Final post-review recheck after filter-free preflight: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test --workspace` (459 passed across 9 suites), and `git diff --check` passed.

- Phase 4 legacy Agent cleanup landed: removed `diary.rs`/`notebook.rs`, the obsolete `AppState.notebook_writes` mutex and initializers, and all Agent execution of legacy `HubCommand::RoomDiary*`/`RoomNotebook*` JSONL operations.
- Legacy protocol variants plus Hub HTTP/MCP/OpenAPI forwarding remain as explicit compatibility residue for the separate Hub parity workstream; Agent inbound legacy commands now return `room_legacy_surface_removed`.
- Standalone Room docs and surface tests now reflect the V2-only 23 Normal / 34 Room surface and the nine semantic tools plus bootstrap aliases; mutations are documented through `room.maintenance.submit`.
- Phase 4 verification passed: `cargo fmt --all`, `cargo check -p agentic-gpt`, 45 Room-focused Agent tests, and the exact standalone Room surface test. The planning catch-up helper path was unavailable (`session-catchup.py` not found); planning continued from the checked-in files.

- 2026-09-11: Phase 5 verification coverage landed: repository bootstrap/path/schema tests (15), deterministic read tests (7), and maintenance backend tests (11), including remote push outcomes and workflow worker consumption.
- 2026-09-11: Real Local Unix integration passed (1 test): the running Agent exposed V2 Room tools only while `room` was enabled, rejected legacy JSONL names, and executed `room.diary.active` without restart.
- 2026-09-11: Final verification passed: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test --workspace` (452 passed across 9 suites), focused Phase 5 suites, and `git diff --check`.

- 2026-09-11: Post-verification docs audit corrected the current-runtime Room counts in `README.zh-CN.md` and `docs/operations.md` to 23 Normal / 34 Room; versioned v0.9 migration/release notes remain historical.

- 2026-09-11: Rechecked `normal_and_room_tool_sets_follow_fixed_surface_contract`: 23 Normal names plus 11 Room additions (two bootstrap entrypoints and nine semantic Room tools) produce 34 Room names; planning and current-runtime docs are aligned.
