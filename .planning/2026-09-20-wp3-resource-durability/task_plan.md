# WP3 resource ownership, retention and durability

## Goal
Execute the existing roadmap WP3 end to end on completed WP1/WP2. Read relevant architecture docs first, then choose concrete implementation from actual code. User allows planning records committed with each phase. Deduplicate real repeated flows where useful; LOC reduction and design patterns are not forced acceptance targets.

## Current phase
Post-delivery review completed; WP3 is reopened for two concrete corrections before the recommended transition to WP4-A: atomic durable batch admission (process and MCP paths) and Agent migration snapshot cleanup on write-lock acquisition failure. This review changed no production code. Prior 663-test and successful runtime evidence remain valid for their exercised paths, but did not cover these failures. WP4 has not started.

## Required scope
1. Authority matrix: run receipts, Job history/cache, transport ledger, audit, config/secrets, Room files, Browser leases, notification endpoints and Console local attention. Record owner/source vs projection, sensitivity, retention, allowed loss and recovery.
2. Bound Hub Job projection by TTL/capacity and expose truthful live/cached/stale/unknown semantics without deleting authoritative runs/history or cancelling execution.
3. Apply D06 durability layers: correctness-critical identity/dedup/results protected; history/confirmation outcomes retained where appropriate; audit/report explicitly best-effort; ephemeral Hub sessions may expire on restart.
4. Address evidenced config atomic replacement/backup/fsync, SQLite migration markers, JSONL locking/rotation/compaction/corruption recovery and sensitive projections. Protect identity/config/run before observation data.
5. Retention/cleanup must preserve unfinished/unknown records and owner/hash/conflict/replay evidence; preserve existing user data and migration/rollback paths.
6. Map actual external MCP/Browser/tmux/tunnel/browser-host resource ownership and Neko/container/shared-volume/UID/GID/socket topology before any authentication or permission changes. No forced owner-only deployment or new remote exposure.

## Phases and commits
1. Read docs/current code; freeze authority/durability/retention matrix and concrete cross-slice contracts. Commit records.
2. Implement Hub bounded Job projection/freshness with migration of all consumers. Commit coherent source+proof.
3. Implement critical write/migration/recovery changes and retention/compaction in owned storage slices, only after interfaces are frozen. Keep intermediate commits buildable; atomic shared composition when necessary.
4. Exercise actual isolated Hub/Agent restart, cache eviction, late/conflict, interrupted/concurrent/corrupt writes, state permissions and Room ownership. Verify browser-host same-user/different-user/shared-container access where actual topology is available; report unreachable prerequisites precisely rather than claiming proof.
5. Record actual external-effect guarantees, final docs/evidence and phase commits. Cleanup is appended only after smoke proof.
6. Review delayed advisor concerns against final source and actual verification chronology; reproduce remaining error paths, record dispositions and determine whether WP3 needs correction before proceeding to WP4-A. This review does not initiate WP4 or restore retired Room APIs.

## Frozen constraints
- D01–D08 apply. No new generic storage framework, global durability, sandbox-default/policy-override change, protocol compatibility shims or remote Browser bridge.
- Prior Hub control-boundary, WP1 and WP2 are completed; do not repeat their implementation or reopen historical plans.
- Main owns scope/decomposition, shared interfaces, planning files, integration, runtime verification and phase commits. Workers skip builds/tests/formatters/linters during edits; one owner per shared mutation boundary.
- CodeGraph first for code discovery; Main supplies LSP references before exported API edits. Read-only scouts may lack Git/shell tools; request concrete facts from Main rather than reconstructing them from old plans.
- User-visible todo updates use direct todo calls, not nested eval calls (prior UI refresh issue).
- No new retention value, auth mechanism or migration is considered selected until backed by current callers/deployment facts. Ask only when remaining tradeoffs actually require user choice.

## Acceptance proof
- Actual Hub/Agent restart differentiates retained authoritative facts from lost projections, including completed/active/timeout/unknown/late/conflict.
- Cache upper bound and TTL can be observed without deleting runs/history or terminating jobs.
- Fault scenarios cover config, run receipt, transport ledger and audit according to their explicit durability layer; unsupported scenarios remain unverified.
- Private state/runtime/socket, Hub DB/config, audit and Room owner/mode/backup/retention are checked without touching user data.
- Shared browser-host socket preserves supported Neko/container collaboration; no automatic owner-only or network exposure.
- External call/audit evidence never claims downstream rollback or complete isolation.

## User confirmation notification
If a material decision requires user confirmation and the user has not replied in time, notify the user's phone with `kdeconnect-cli -n slhaf-mobile --ping-msg <content>`. Send a concise, non-sensitive description of the pending decision, not credentials/config values or routine progress. Do not ping merely to test notification delivery.

## Implementation contracts
- Hub projection owner: 4096 global entries; evict after 15 minutes since Hub observation; stale after 60 seconds or connection loss/change. These are lossy-cache limits, not result retention. Preserve Agent JobInfo timestamps, current-connection write gate and lock order. Label response provenance without extending Agent JobInfo.
- Hub SQLite: explicit versioned transactional migration and recoverable pre-migration backup; preserve existing 24-hour completed history window but never delete active/unknown/conflict/replay identity. Prefer retained tombstones over destructive pruning; result omission must be explicit.
- Ledger: keep global legacy path, add explicit owner to new records; one locked conditional claim, sync before acknowledgement/effect/result delivery. Never manufacture missing command hashes; preserve original and conflicting evidence. Corruption is fail-closed, not permission to replay; preserve raw recovery evidence. No time-based identity expiry.
- Final legacy migration rule: every ownerless record remains LegacyUnowned, including an exact incoming envelope whose command declares the current Agent. Old global reconciliation could run a command under another Agent, so payload target is not proof of historical execution ownership. Preserve evidence; neither startup nor incoming-envelope handling may execute or disclose an unowned result.
- Agent history: version1 transactionally adopts legacy schema without changing existing rows, preserves a private consistent pre-migration snapshot, and refuses future versions without quarantine/reset; new DB0600 inside private0700 root. Keep 30-day/512-MiB policy for ordinary completed history; protect unknown/active and terminal snapshots not yet persisted. Audit separately remains best-effort with locked bounded rotation.
- Config: narrow atomic/backup/sync changes, not a generic storage framework; setup recovery is a specific multi-file transaction, not claimed atomicity from independent renames. Private-state migration retains source until verified durable target.
- Ownership: Hub cache worker owns cache owner, consumers and Hub main cache wiring; Hub storage worker owns db/runs and coordinates any main.rs config-only edit. Agent ledger worker owns transport_ledger/hub. Agent config worker owns config/setup/config CLI policy/MCP mutation edges as required. Agent history/audit worker owns job_history/jobs/audit/private_state/skills. No worker changes protocol or another owner's file without Main contract update.
- All writing workers skip builds/tests/format/lint while concurrent. Main integrates, runs validation and actual smoke, then commits each coherent phase.
