# Findings

- Worktree-local CodeGraph index is absent; the available index targets another worktree and cannot be trusted for branch-specific source. Raw repository search/read is required.
- Tool descriptors are assembled in `crates/agentic-gpt/src/stdio_server.rs`; room-only names are currently selected by `CapabilityProfile::Room`.
- `Config` in `crates/agentic-gpt/src/config.rs` is persisted JSON and already held behind `AppState.config: Arc<RwLock<Config>>`.
- CLI dispatch is `main.rs` → `config_cli::handle_config`; existing MCP config mutation offers a relevant list/enable/disable persistence pattern.
- Initialization flows through `config_templates::InitInput`, `config_setup::SetupSession`, and `config_tui`; `OrderedMultiSelectState` is the existing reusable checklist state.
- No Rust language server is configured, so reference discovery must use CodeGraph where trustworthy and targeted repository search otherwise.
- Current tool availability is frozen in `AgentMcpServer::with_ingress`: `NORMAL_TOOLS` plus room arrays are copied into `Arc<Vec<Tool>>`; `list_tools`, `get_tool`, and dispatch authorization all consult that fixed vector. Hot reload therefore requires dynamic selection from `state.config`, not only config watcher support.
- Tool namespaces are the prefix before the first dot for every current tool except bare `bootstrap`; the room preset must treat `bootstrap`/`bootstrap.read` and `room.*` as one logical room namespace or formally define a namespace table rather than infer ad hoc.
- `Config::load` merges serialized JSON over mode/profile-adjusted defaults. A new field can have profile-aware defaults if defaults are recomputed after reading `profile`; sparse persistence must compare against the same profile preset.
- Runtime config watchers already replace `AppState.config` after validated file changes while rejecting mode/profile changes. The optional toolset field should remain live-reloadable and must not be added to restart-required comparisons.
- Existing MCP config commands demonstrate the requested clap shape and backup-aware persistence.
- Standalone has two watchers: supervisor watches startup identity and only requests restart when identity changes; worker/local runtime `watch_standalone_live_config` applies an approved live subset to `AppState.config`. Toolsets belong in that live subset.
- `config_templates::build_config` starts from `Config::default_config`, then overwrites mode/profile. Therefore profile-dependent toolset presets must be assigned after profile selection unless `default_config` accepts a profile.
- Init TUI optional sections are round-tripped through `OptionalSection`, `OptionalSectionDraft`, `OptionalDrafts`, validation/save/build/review, and generic page rendering. Adding a Toolsets optional section can reuse `OrderedMultiSelectState`.
- Current setup model preserves explicit imported settings; fresh sessions use default drafts. Profile changes need to update the default toolset preset only while the toolset section remains unconfigured, otherwise a user's explicit toggles should survive.
- The exact live reload cutover is `main.rs::apply_standalone_live_subset`; it currently copies policy/path/limits/MCP only and must copy toolsets.
- Local mode is the repository’s intended credential-free smoke surface and runs the same `watch_standalone_live_config` path as the standalone worker. It can verify live toolset reload with `agentic-gpt run` plus `agentic-gpt local list-tools`.
- Existing docs assert fixed 24/36 profile counts and a fixed Normal/Room matrix. Documentation must instead state profile presets plus user-selectable toolsets; `README`, both configuration documents, `standalone-runtime.md`, and `tool-contract-matrix.md` are directly affected.
- `config.example.json` must add an explicit normal-preset `toolsets` object because strict loading will require it.
- Configuration docs need exact JSON shape and the three `config toolset` commands, and must replace fixed profile tool counts with defaults: normal excludes `room`, room enables all; the initializer optional-settings list must mention Toolsets.
- `standalone-runtime.md` and `tool-contract-matrix.md` remain useful as per-tool contract references, but their fixed-surface claims must change to say exposure follows the enabled namespaces; the room namespace includes bootstrap plus Room memory tools.
- `stdio_server` has dispatch-only aliases absent from the advertised `NORMAL_TOOLS`/room arrays. Decision: optional toolsets filter exactly the pre-existing advertised surface; they must not expand it by exposing aliases.

- 2026-09-12 seam audit: `build_app_state()` bootstraps Room only at startup; `apply_standalone_live_subset()` copies `toolsets` but has no false-to-true repository transition.
- `config_templates::optional_section_is_legal()` and setup session availability currently gate `OptionalSection::Room` on `WorkerProfile::Room`, so a Normal profile with an explicit Room-enabled `ToolsetConfig` cannot carry Room draft values through init.
- The TUI Room help still says it is available only for the Room profile. `config_cli::toolset_description()` and its bilingual listing test still describe only bootstrap/diary/notebook, omitting state and maintenance.
- The mounted planning catch-up script path `~/.codex/skills/planning-with-files/scripts/session-catchup.py` is absent in this environment; this session continues from the checked-in planning files.

- 2026-09-12 follow-up implementation: live reload now bootstraps the Room repository only on a real disabled-to-enabled transition, before applying the candidate config; bootstrap failure leaves the previous live subset intact.
- Init legality now receives the finalized `ToolsetConfig`; explicit Room enablement works with `profile=normal`, while an explicit Room-disabled selection hides and rejects the Room section even if the profile changes to Room.
- Room CLI descriptions now name bootstrap, diary, notebook, state, and maintenance in both English and Chinese; the listing behavior test asserts both localized strings.
- Verification after integration: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test --workspace` (457 passed across 9 suites), and `git diff --check` all passed. The first check failure was a stale `tunnel.executable` access; it was corrected to `tunnel.client.executable`.

- 2026-09-12 regression hardening: the live-reload test now explicitly asserts the initial Normal state has Room disabled and no repository root before writing the disk candidate; the targeted bootstrap/maintenance test passed.

- 2026-09-12 final verification rerun after the last `main.rs` regression assertion: `cargo fmt --all -- --check`, `cargo check --workspace`, and `cargo test --workspace` (457 passed across 9 suites) all passed.
- `WATCHDOG.yml` was present in the earlier session status but is now absent from the target worktree. Current status/stat, expected parent/worktree paths, home/project glob searches, Git history, and shell-history inspection found no copy or deletion command. It was not recreated because its contents are unavailable; no current diff/status entry is attributable to it.

- 2026-09-12 post-review found one remaining live/restart boundary defect: the first hot-enable implementation called `ensure_repository(&candidate)`. Because `room.*` remains restart-required while only `toolsets.enabled` is live, a disk edit that changed `room.repositoryRoot` together with `room: false -> true` could bootstrap candidate root B while live Room tools still used root A. The transition now bootstraps from the current live config before applying the candidate toolset, and the regression uses distinct live/candidate roots to prove B is untouched and maintenance executes against A.
- The same review found `agent_info::live_subset()` still omitted `toolsets`, so disk/live toolset drift could incorrectly report `liveSubsetMatchesDisk=true`. Toolsets are now included in the observable live subset with a direct `config_health` regression; this avoids the unrelated notification probes in full `agent.info` collection.
- Documentation now states explicitly that `room.*` settings are restart-required even though the `room` namespace itself can be hot-enabled. Full verification after both fixes passed: fmt, diff-check, workspace check, and workspace tests with 458 tests across the same 9 suites. Existing integration tests were unusually slow in this environment (`agent_info` MCP subset, Local Unix smoke, and one supervisor reload test) but completed successfully.
