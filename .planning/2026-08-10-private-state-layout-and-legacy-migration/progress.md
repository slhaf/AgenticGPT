# Progress: Private State Layout and Legacy Migration

## 2026-08-10 — Planning start
- Created a separate migration plan rather than expanding the Job Observability plan.
- Confirmed the architectural distinction between writable workspace assets and Agentic-owned private durable state.
- Froze the new durable root shape as `~/.agentic_gpt/state/agent/<agentId>/`.
- Froze migration scope to `active-skills.json` and `skill-installs/`; `skills/`, notebook, diary, and bootstrap remain workspace-visible.
- Recorded the need for a small shared private-state path helper and startup migration before skill-install recovery/tool ingress.
- Recorded cross-filesystem-safe copy/verify/cleanup as the migration direction; exact conflict/failure/compatibility behavior remains to be frozen.
- No product source files changed.

## 2026-08-10 — Implementation complete
- Added `crates/agentic-gpt/src/private_state.rs` and `AppState.private_state` as the small authoritative private-state path boundary.
- Migrated tool-managed `active-skills.json` and the complete `skill-installs/` transaction/recovery tree out of the writable workspace while leaving skill packages and other collaborative assets in place.
- Startup now prepares/migrates private state before skill-install recovery and normal ingress for Hub, Standalone, and Local modes.
- Frozen conservative migration behavior: divergent target wins without deleting legacy; identical old/new copies self-clean; migration/root failure preserves and uses legacy state for that boot; no long-term dual-read.
- Preserved wider legacy Hub `agentId` compatibility through a stable hashed state-directory key while path-safe IDs retain the direct `<agentId>` layout.
- Updated English/Chinese configuration docs and skills interface documentation.
- No generic state framework or workspace `file.*` reserved-path exception was introduced.

## Verification
- `cargo clippy -p agentic-gpt --all-targets -- -D warnings` — PASS.
- `cargo test -p agentic-gpt` — PASS: 314 unit tests + 15 config CLI + 1 local runtime + 6 standalone supervisor integration tests.
- Private-state focused tests — 7/7 PASS, including migration, idempotency, divergent conflict preservation, identical-copy cleanup, symlink rejection/fallback, wide `agentId` mapping, and Unix `0700` root permissions.
- Skills tests — 10/10 PASS; skill packages remain workspace-visible while active state uses private state.
- Skill-install tests — 10/10 PASS; records/staging/journals use the selected private records root.
- `git diff --check` — PASS.

## Runtime smoke test with copied real state
- Copied the live laptop workspace's legacy `state/active-skills.json` and `state/skill-installs/` into an isolated temporary HOME/workspace; the real environment was not modified.
- First startup attempt used an intentionally isolated but overly long HOME path: startup reached and completed migration, then correctly failed later on `local_mcp_socket_path_too_long`. Post-attempt target hashes exactly matched the copied legacy files and the legacy state directory was cleaned.
- Repeated the test under short `/tmp/agps-*` paths. The real `agentic-gpt run --config ...` local Agent started successfully, exposed its Unix MCP socket, and `agentic-gpt local ... list-tools` connected successfully.
- A second startup of the already-migrated sandbox also succeeded, exercising migration idempotency. `skills.list` returned the activation records copied from the real legacy `active-skills.json`, proving the running consumer reads the migrated private state. Non-copied workspace skill packages correctly appeared as `missing`; the built-in installer remained active.
- The sandbox Agent was stopped cleanly after verification.

## Current blockers
None. The Job Observability private-state prerequisite is satisfied.
