# Findings: Private State Layout and Legacy Migration

## Repository evidence
- Configuration docs and `default_path_policy()` explicitly treat `workspaceRoot` as the main writable workspace; `workspaceRoot` is always part of the effective write surface unless a stronger deny applies.
- Relative `file.*` paths resolve beneath `workspaceRoot`. Current reserved-path handling protects only specific runtime files such as `.agentic-gpt-audit.jsonl`, not the whole `state/` subtree.
- Workspace-visible assets currently include `skills/`, `diary/`, `notebook/`, and `bootstrap/`. These are intentionally read/maintained as workspace content and should remain there.
- `active-skills.json` currently lives at `workspaceRoot/state/active-skills.json` and is read/written by skills activation logic.
- Skill-install persistence currently lives at `workspaceRoot/state/skill-installs/` and contains terminal/non-terminal records, staging data, commit journals, pruning state, and startup recovery inputs.
- `skill_installs.recover(state)` runs during Agent startup before normal long-running service loops; the private-state migration must run before that recovery call.
- The repository already uses private per-agent runtime paths under `~/.agentic_gpt/runtime/agent/<agentId>/` for the local MCP socket and `~/.agentic_gpt/runtime/tunnel/<agentId>/` for tunnel supervisor artifacts. These directories validate agent identity and enforce private permissions.
- Durable state has a different lifecycle from runtime sockets/PIDs, so a sibling `~/.agentic_gpt/state/agent/<agentId>/` hierarchy is preferable to reusing `runtime/`.

## Frozen boundary
### Workspace-visible / collaborative
- `workspaceRoot/skills/`
- `workspaceRoot/notebook/` or configured notebook root
- `workspaceRoot/diary/`
- `workspaceRoot/bootstrap/`

### Agentic-private durable state
- `~/.agentic_gpt/state/agent/<agentId>/active-skills.json`
- `~/.agentic_gpt/state/agent/<agentId>/skill-installs/`
- future `~/.agentic_gpt/state/agent/<agentId>/jobs.sqlite3`

### Agentic-private short-lived runtime
- existing `~/.agentic_gpt/runtime/agent/<agentId>/...`
- existing `~/.agentic_gpt/runtime/tunnel/<agentId>/...`

## Implemented contract
- `private_state::prepare()` establishes `AppState.private_state` before `skill_installs.recover()` in Hub, Standalone worker, and Local startup paths. Consumers do not reconstruct private paths themselves.
- `PrivateStatePaths.root` is the authoritative durable per-agent root; current child paths are `active_skills` and `skill_installs`, and Job history can later use `root.join("jobs.sqlite3")`.
- Path-safe `agentId` values are used directly as the state directory key. Wider/long legacy Hub identities are mapped to a stable SHA-256-based key, preserving compatibility without path injection.
- Private state root creation enforces mode `0700` on Unix; migrated active-skill temp files are mode `0600` before target rename.
- Migration is cross-filesystem-safe: copy into a target-side temporary file/tree, reject symlinks/special files, verify complete bytes/tree shape, rename inside the target filesystem, then remove the legacy source last.
- If target exists and source differs, target is authoritative, source is retained, and startup warns. If they are identical, the redundant legacy source is cleaned automatically.
- If root creation or a specific migration cannot complete, the source remains intact and the affected consumer uses the legacy path for that boot with an explicit warning. A later healthy startup retries migration.
- Successful migration switches the boot immediately to private paths; there is no persistent dual-read compatibility layer.
- `workspaceRoot/state/` is removed only when empty. Unknown/unrelated legacy contents are never deleted.
- `workspaceRoot/skills/`, notebook, diary, and bootstrap remain workspace-visible and unchanged.

## Scope guard
- Job SQLite schema/history behavior is not part of this task.
- Redesigning the skills package format or activation semantics is not part of this task.
- Notebook/diary/bootstrap relocation is explicitly out of scope.
