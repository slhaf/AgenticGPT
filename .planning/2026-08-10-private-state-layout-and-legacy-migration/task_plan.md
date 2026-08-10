# Task Plan: Private State Layout and Legacy Migration

## Goal
Move Agentic-owned correctness-sensitive durable state out of the Agent-writable `workspaceRoot`, establish one per-agent private state root, and safely migrate the two existing legacy consumers before Job history starts using the same boundary.

## Status
- **Stage:** complete
- **Blocking decisions:** 0
- **Implementation:** complete and verified
- **Next consumer:** `.planning/2026-08-10-job-observability-and-process-tui/`

## Completed Checklist
- [x] Freeze workspace/private-state boundary.
- [x] Establish `AppState.private_state.root` under Agentic home rather than `workspaceRoot`.
- [x] Keep short-lived sockets/PIDs under existing `runtime/`; keep durable state under `state/`.
- [x] Keep `skills/`, notebook, diary, and bootstrap workspace-visible.
- [x] Move tool-managed `active-skills.json` to private state.
- [x] Move the complete `skill-installs/` record/staging/journal/recovery tree to private state.
- [x] Run migration before `skill_installs.recover()` in Hub, Standalone, and Local startup paths.
- [x] Make migration cross-filesystem-safe with target-side temporary copy, verification, target rename, and source cleanup last.
- [x] Preserve divergent old/new copies without merge or destructive overwrite.
- [x] Self-clean identical legacy/private duplicates left by an interrupted cleanup.
- [x] Fall back to preserved legacy state for the current boot when private-root creation or migration fails, with explicit warning.
- [x] Preserve wider legacy Hub `agentId` values through a stable safe directory key rather than imposing a new Hub identity restriction.
- [x] Update docs and focused/full verification.

## Frozen Layout
For normal path-safe identities:

```text
~/.agentic_gpt/
├── runtime/
│   ├── agent/<agentId>/...
│   └── tunnel/<agentId>/...
└── state/
    └── agent/<agentId>/
        ├── active-skills.json
        ├── skill-installs/
        └── jobs.sqlite3          # next consumer; not implemented in this task

<workspaceRoot>/
├── skills/
├── notebook/
├── diary/
└── bootstrap/
```

Wider/long legacy Hub identities use a stable hashed directory key under `state/agent/`; consumers must use `AppState.private_state.root` rather than reconstructing that mapping.

## Migration Rules
- Target absent + legacy present: copy to target-side temporary path, reject symlinks/special files, verify, rename to final target, then delete legacy source.
- Target and legacy both present and identical: keep target and remove redundant legacy copy.
- Target and legacy both present but divergent: target is authoritative for the boot; retain legacy unchanged and warn; never merge automatically.
- Migration/root failure: preserve legacy and use its path for the affected consumer during that boot so startup remains available; retry naturally on a later startup.
- After successful migration there is no persistent dual-read mode.
- Remove `<workspaceRoot>/state/` only when it becomes empty; unknown contents are untouched.

## Job Handoff
The private-state prerequisite is satisfied. Job history should use:

```rust
state.private_state.root.join("jobs.sqlite3")
```

It must not duplicate Agentic-home, `agentId`, or hashed-directory-key logic.

## Verification
- `cargo clippy -p agentic-gpt --all-targets -- -D warnings` — PASS.
- `cargo test -p agentic-gpt` — PASS: 314 unit + 15 config CLI + 1 local runtime + 6 standalone supervisor integration tests.
- Private-state focused tests — 7/7 PASS.
- Skills tests — 10/10 PASS.
- Skill-install tests — 10/10 PASS.
- `git diff --check` — PASS.
