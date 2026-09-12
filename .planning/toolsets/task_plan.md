# Optional Toolsets

## Goal
Implement namespace-based optional toolsets for standalone deployments: normal excludes room tools by default, room includes every namespace; selections are editable through init TUI, CLI, and direct config hot reload.

## Constraints
- Clean upgrade; no compatibility shim.
- Namespace granularity only.
- CLI: `agentic-gpt config toolset enable|disable <namespace>` and `agentic-gpt config toolset ls`.
- Reuse or extend general TUI components where appropriate.
- Minimal behavior-focused tests and defensive code.
- At most three implementation-verification-fix attempts.

## Shared Contract
- `ToolNamespace`: closed, serialized lower-case namespace set: `agent`, `file`, `mcp`, `process`, `job`, `skills`, `tmux`, `room`.
- `ToolsetConfig`: persisted as an explicit enabled-namespace collection. Every newly written configuration retains this field; no legacy field, alias, or migration path is introduced.
- `room` maps both `bootstrap`/`bootstrap.read` and `room.*`; the other namespace mappings are their dotted tool-name prefixes.
- Preset: `normal` enables every namespace except `room`; `room` enables every namespace.
- `AgentMcpServer` calculates list and call availability from the live `AppState.config` toolset on every request so watcher replacement affects already-running stdio/local ingress.
- CLI persists only valid namespaces with existing backup-aware config writes. `ls` prints the effective enabled namespace names.
- TUI exposes the same namespace list through `OrderedMultiSelectState`; a fresh/default draft follows the selected profile preset, while an explicitly edited selection persists across profile switches.

## Ownership
- Core/runtime: `config.rs`, `stdio_server.rs`, `main.rs`, focused core tests.
- CLI: `config_cli.rs`, focused CLI tests.
- Init TUI: `config_templates.rs`, `config_setup/**`, `config_tui/**`, focused TUI/setup tests.

## Phases
- [complete] Research current config, modes, tool registry, hot reload, CLI, and init TUI.
- [complete] Define shared toolset configuration/filtering contract and slice ownership.
- [complete] Implement independent core/runtime, CLI, and TUI slices in parallel.
- [complete] Integrate and verify CLI plus live config hot reload behavior.
- [complete] Run required Rust formatting, checks, and tests.
- [complete] Prepare isolated init TUI launch command for user acceptance.
- [complete] Cleanup temporary artifacts and update affected existing documentation.

## CLI Usability Follow-up
- [complete] Show every namespace with enabled/disabled status and a localized description.
- [complete] Print localized success feedback after enable/disable persistence.
- [complete] Update affected command documentation and verify real CLI output.

## Reviewer Defect Follow-up
- [complete] Make Room execution authorization follow live `toolsets.room`.
- [complete] Seed explicit imports without toolsets from the imported profile preset.
- [complete] Replace stale Room profile error semantics and update affected documentation.
- [complete] Run focused regressions, full workspace verification, and a real hot-enable smoke.

## Errors Encountered
| Error | Attempt | Resolution |
|---|---:|---|
| `cargo fmt --check` exposed an unclosed `ConfigCommand::Deny` delimiter from the CLI slice. | 1 | Restored the missing `},` and `Path {` variant boundary, then formatted. |
| `cargo check --workspace` reported 117 TUI/setup compilation errors from missing pre-existing imports/accessors. | 1 | Sent the exact diagnostic evidence to the TUI slice owner for a constrained restoration; no toolset behavior is being redesigned. |
| Session catchup script absent under `~/.codex`. | 1 | Ran the mounted skill script through `skill://`; it completed successfully. |
| `cargo test --workspace` failed because a focused live-reload test read `candidate.toolsets` after moving `candidate`. | 2 | Preserved a cloned expected toolset before the ownership transfer. Final verification is next; stop and report if it fails. |
| Final `cargo test --workspace` failed: `config.toolset` is absent from `cli_i18n` visible-command metadata; `stdio_server::normal_and_room_tool_sets_are_exact` still asserts the old Normal count. | 3 | Attempt cap reached. Stopped without modifying these remaining integration points, per user instruction. |
| Authorized repair initially left CLI argument metadata and a room supervisor fixture stale. | Authorized round | Added namespace argument metadata, made the fixture select the room toolset preset, restored the exact 23/35 public mapping, and completed all verification. |
| `cargo check --workspace` reported `TunnelConfig` has no direct `executable` field after the legality refactor. | Follow-up integration | Corrected the setup model round-trip to read `tunnel.client.executable`; rerun workspace verification. |

## 2026-09-12 Constrained Follow-up
- [complete] Bootstrap the Room repository on a real live `room` false-to-true transition.
- [complete] Make Room init/config legality follow final Room toolset selection.
- [complete] Update bilingual `config toolset ls` Room descriptions.
- [complete] Run the requested workspace verification without committing or touching `WATCHDOG.yml`.

## 2026-09-12 Post-review Closure
- [complete] Bootstrap hot-enabled Room from current live Room config, not restart-required candidate `room.*`.
- [complete] Cover distinct live/candidate repository roots and prove maintenance executes against the live root.
- [complete] Include `toolsets` in `agent.info` live-subset observability with a focused config-health regression.
- [complete] Document the `room.*` restart boundary and rerun focused + full workspace verification.
