# Findings: Config Schema v2

## Current architecture
- `Config` is currently a fully materialized serde struct. Most top-level fields are required during deserialization; only selected fields use serde defaults/skip rules.
- `Config::default_config()` fills Hub identity/URL/secret placeholders, workspace/path policy, confirmation, sandbox, limits, skills, room, and other defaults.
- `build_config()` starts from `Config::default_config()` and overlays Config Init selections. Consequently the current JSON writer serializes a full default snapshot.
- `write_config_with_backup(path, &Config)` is the common durable writer used by Config Init and many mutation commands; centralizing sparse projection here (or directly beneath it) prevents later mutations from re-expanding config.
- `Config::load()` already performs one dependent-default repair: when `pathPolicy` is absent it derives it from loaded `workspaceRoot`. This is a useful pattern for sparse v2 loading.
- Config Init Review currently builds a full `Config` and JSON preview from that object, so preview must be changed to use the same sparse projection as durable write.

## Current runtime entry points
- Public commands currently include `run` (Hub + Normal), `run-as-room` (Hub + Room), `run-as-standalone --profile`, and `run-as-local --profile`.
- `WorkerProfile` is currently defined in `main.rs` as a clap enum; Config Init also imports it. Persisting profile means it should move to a shared domain/config module.
- `RuntimeMode` already exists in config-template/setup code and should likewise become reusable by serde/runtime dispatch rather than remain init-specific.
- `stdio-worker` is hidden/internal and may still need explicit internal launch data; it is not part of the public-entry consolidation goal.

## Desired disk semantics
- Always explicit: `mode`, `profile`.
- Omit values equal to their reconstructable defaults and omit empty/default optional sections.
- Mode/profile-specific sections should only serialize when meaningful/legal for the selected combination.
- Dependent defaults must compare against the correct effective context (notably workspace-dependent path policy).
- Unknown flattened fields currently preserved by `Config.extra` must not be accidentally dropped by the sparse projection.
- `config show` should remain useful as an effective/full view even when the on-disk file is sparse.

## Migration stance
- Do not infer missing mode/profile from `tunnel`, Hub defaults, or other incidental fields; Hub vs Local is not reliably inferable.
- Prefer an explicit “config requires mode/profile; run config init/migrate” error for old files.

## Repository-grounded refinement (2026-08-09)
- `crates/agentic-gpt/src/config.rs` has no persisted runtime selector today. `Config` is a fully materialized serde struct and `Config::load()` deserializes directly from JSON, with only the existing `pathPolicy` and legacy `room.skills` repairs. There is no default-overlay load path for missing required fields.
- `write_config_with_backup()` currently serializes `Config` directly. It is already the common writer for config init, `config set`, policy/path-policy mutations, and MCP mutations; changing this boundary is sufficient to keep those writes sparse, but callers that create a default config during `run`/supervisor startup must also produce a v2 config or fail clearly.
- `Config::load_or_default()` is used by mutation commands and tmux helpers. A missing file must therefore default to a v2 effective config (including mode/profile) before it is written; an existing file without selectors must return the migration/init error rather than be heuristically inferred.
- Public CLI dispatch is in `main.rs`: `run` hard-codes Hub/Normal, `run-as-room` hard-codes Hub/Room, and `run-as-standalone`/`run-as-local` take public `--profile`. `stdio-worker` is hidden but supervisor command construction passes explicit profile and supervisor token. The concrete internal dependency is the worker command protocol, so keep hidden worker profile data while removing public run-as-* variants.
- Standalone startup is owned by `supervisor::run(config_path, profile)` and currently validates/reads a profile supplied by the caller. Unified dispatch must load selectors before choosing `supervisor::run`, local serving, or Hub; supervisor and hidden worker must still validate that the loaded mode/profile match their internal purpose.
- Config Init’s final preview is `SetupSession::redacted_config_json()`, which currently builds a full config, redacts only `agent_secret`, and serializes it directly. Commit calls `write_config_with_backup()` later. The preview must instead call the canonical sparse projection with an explicit redaction option; transaction-only `StandaloneDraft.secret_value` is never part of `Config`, but the projection must not introduce it.
- `Config.extra` is a flattened `BTreeMap<String, serde_json::Value>`. The sparse projection must start from/merge this map so unknown future flattened fields survive every durable rewrite. Mode/profile-inactive sections must be retained rather than pruned, and runtime validation remains the active-section boundary.
- `set_workspace_root()` rewrites a path-policy write root that matches the old workspace. Sparse omission of `pathPolicy` must compare with `default_path_policy(&effective_workspace_root)`, and changing workspace without an explicit path-policy edit must retain the derived default relationship.
- Existing tests assert the legacy public CLI commands and full default serialization/example shape; these are schema-contract tests that must be updated, while no TUI layout/interaction tests should be added.

## Implementation evidence and follow-up decisions
- A custom `workspaceRoot` must remain in the sparse file; only its dependent default `pathPolicy` is reconstructed. The projection therefore compares `workspaceRoot` to the global environment-aware default, but compares `pathPolicy` to `default_path_policy(effective_workspace_root)`.
- Strict v2 load should not carry legacy object-shape exceptions. `config import` canonicalizes legacy `confirmationProvider: {provider: ...}` to `channels` before deep overlay; normal load rejects the legacy shape.
- Mode/profile-inactive known sections are retained before overlay and in projection. This preserves explicitly configured tunnel/room data for later selector changes; runtime validation/use remains mode/profile-aware, and top-level `skills` remains shared because Normal local/standalone workers still expose skills.
- Runtime selector changes are startup-owned. Hub, Local, standalone live reload, and the standalone supervisor keep the running transport/profile and report/reject selector changes as restart-required; hidden `stdio-worker` cross-checks its explicit internal profile.
- Durable writer audit found `write_config_with_backup()` is the common production JSON write boundary. `config show`, policy/MCP display output, and test fixtures intentionally serialize effective/subsection values for read-only output; no additional production durable JSON writer required a separate projection call.

## Follow-up review findings (2026-08-09)
- The current implementation still stores Hub fields as `hubUrl`, `hubTransport`, and `agentSecret`; v2 persistence must use `hub: { url, transport, agentSecret }`, with old top-level names accepted only by an explicit import path.
- Sparse projection/load currently removes `tunnel` for non-standalone mode and `room` for non-room profile. This is destructive for inactive user configuration; selectors must affect active validation/use only.
- Public `run` still creates a default file when its selected path is absent. It must return a stable init-first error without writing.
- Config Init Preview currently redacts `tunnel.apiKey`; normal `file:` and `env:` references are durable configuration and should remain visible, while only the actual Hub secret is redacted.
- `config import` needs a seed model that carries a materialized imported base plus the TUI-managed fields. Building the final config must overlay editable TUI values onto that base so recognized fields without editors (`mcpServers`, policy/pathPolicy, limits, etc.) and safe unknown flattened fields survive.
- Import must also treat `tunnel.apiKey` as a reference-only field: invalid or plaintext source values are reported and cleared before the imported base can be rewritten, while valid `file:`/`env:` references remain visible in Preview.
