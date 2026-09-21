# WP3 findings

## Confirmed input
- Read docs/architecture/decisions.md D01–D08 and full WP3 scope, staged commits, acceptance and stop conditions in refactoring-plan.md377–454.
- D06 explicitly separates correctness-critical identity/dedup/results, retainable history, best-effort audit/report and restart-ephemeral Hub state. No blanket durable-everything requirement.
- D05 actual user deployment includes Neko, containers, shared directories and Unix sockets. Topology/UID/GID must be established before choosing protection; do not replace this with an owner-only assumption.
- Current WP1/WP2 completion and structural comparison are accepted. Git diff --stat was empty before initializing this independent plan.
- User asks for useful deduplication/design, not mandatory LOC reduction. Planning files may be committed; temporary probes/raw snapshots/secrets are excluded.

## Evidence status
Roadmap storage/cache gaps are investigation inputs, not automatically current bugs. Concrete values, APIs and code changes will be selected only after source/caller review.

## Additional guidance and probe capabilities
- Read target-architecture §5 authority/identity matrix and engineering R04–R10 ownership, lifecycle, result conflict and durability rules.
- Available binaries:python3, docker, podman, unshare, setpriv, runuser, tmux, sqlite3.
- Docker is reachable, but currently running containers are two existing buildkit builders, not Neko. Leave them untouched. Cached images are githubyumao/mcsmanager-daemon:latest and moby/buildkit:buildx-stable-1; actual Neko runtime topology is not yet observed locally.
- User authorized KDE Connect reminder for an unanswered material confirmation: `kdeconnect-cli -n slhaf-mobile --ping-msg <content>`. Do not send routine updates, test pings or secrets.

## Baseline findings before implementation
- Agent transport ledger is a shared `~/.agentic_gpt/transport-runs.jsonl`, not a per-Agent file. Its records preserve run/request/hash/command/result but do not identify an Agent owner. Do not silently relocate or reassign legacy records.
- `transport_ledger.rs` currently skips malformed JSON lines, performs unlocked read/append transitions and does not sync appends. This can lose deduplication evidence; compaction must not erase unknown or conflicting records.
- `hub.rs::handle_reliable_envelope` already aborts execution on `mark_started` failure. `send_response` already propagates completion persistence failure before sending the response.
- A separate ordering gap remains: `handle_hub_command` sends JobUpdate with `?` before calling `send_response`; a disconnected Hub can therefore prevent a completed operation from reaching ledger persistence. Fix persistence ordering rather than treating notification delivery as authoritative completion.
- Execution claims need an atomic accepted-to-started transition, not merely a lock around acceptance: reconciliation reads a snapshot and may race another claimant.
- LSP references for HubState.jobs remain stale after rust-analyzer reload (lifecycle positions point at unrelated current lines); reported tool issue. Use current-source/CodeGraph evidence to supplement, not stale reference positions.

## Resolved implementation details
- Final review rejected the initially considered narrow legacy-adoption exception: a command's explicit target plus exact envelope match does not establish which Agent actually executed it under the old global reconciliation behavior. Every ownerless record therefore remains LegacyUnowned, never autoexecuted or disclosed; original evidence is retained for operator-led recovery.
- Hub cache is explicitly disposable projection: its hard capacity may evict active-looking snapshots without changing remote execution or authoritative history. Protecting active/unknown evidence applies to storage, not retaining every cache entry forever.
- Current remote deployment inspection established host-root and systemd-nspawn-root Agent access to the same bridge socket inode as Neko Chromium/native host UID/GID1000:1000. Directory0755/socket0660 have no ACL xattrs. Read-only bridge status succeeded; isolated real browser-host shared-volume tests confirmed same UID/shared GID access and different UID/GID EACCES, plus stdin-close socket cleanup. No production Browser permissions/authentication changes were made.
- Both SQLite migration owners stage private consistent snapshots, then re-read user_version under an IMMEDIATE write transaction before publishing migration backup or changing schema. Actual concurrent-future-version probes proved refusal without downgrading data or overwriting existing recovery evidence.

## Post-delivery advisor review (baseline e8b531a; corrected below)
- P1 — batch admission atomicity: jobs.rs:543-551 (MCP) and1040-1048 (process) remove only in-memory registrations after a later insert_admission failure, leaving earlier committed SQLite rows. Actual local process.batch with a SQLite trigger rejecting the second insert returned process_batch_rejected, left one queued row, and converted it to unknown_after_restart on restart; neither touch marker was created. MCP manifestation is source-confirmed, not independently runtime-exercised. Recommended correction: one durable transaction for all admissions before in-memory publication, not unreliable compensating deletes after database failure.
- P2 — job_history.rs:739 propagates IMMEDIATE transaction acquisition failure after staging without cleanup; StagedMigrationSnapshot has no Drop. Actual unmodified Agent binary was paused at the read-only snapshot fsync after VACUUM, then resumed while a competing writer held its lock through startup completion. One32768-byte snapshot remained (directory0700/file0600); original schema/data stayed unchanged. This proves a private-file/resource leak, not cross-user disclosure or loss of canonical data. Match the Hub's explicit acquisition-error cleanup.
- Closed/stale: capacity now returns Rejected and history failure Failed; not_started truthfully describes both no-effect outcomes while reject_reason distinguishes cause. Empty-cache Job get includes unknown freshness. Legacy/future migration regressions and663-test final evidence exist; the alleged empty-table query panic is no longer present. Removed Room API400, missing guidance Bootstrap404 and job_not_found for non-admitted failures are expected, not reasons to restore/write records.
- Earlier failed race probes do not negate the later successful staging-directory barrier proof recorded in progress.md. That proof covers future-version rejection, not the separate sustained-lock-timeout cleanup branch.
- Next package is WP4-A (contract parity/docs/CI); WP4-B is optional later module organization. WP-R remains a separate core package. WP4-A is not architecturally dependent on WP3, but correcting these new WP3 defects first is the recommended delivery order.

## Repair decisions
- Admission API takes borrowed JobInfo references and commits all rows in one SQLite transaction; callers publish staged in-memory registrations only after success. No compensating deletes, extra compatibility entry point or cloned batch vector.
- The additional Hub publish concern is distinct: after successful rename the snapshot is already the final backup, not a sensitive file remaining in staging. A subsequent directory-removal failure is returned explicitly and can leave an empty directory; sync_parent runs only after directory removal. No evidence supports expanding this repair into a second Hub sensitive-snapshot leak.
- Corrections implemented: the shared borrowed-iterator admission API uses one IMMEDIATE transaction; both process and MCP callers stage registrations and publish only after commit. The migration commit helper explicitly cleans staging on transaction-acquisition failure and retains prior version/backup guards.
- Four permanent regressions cover storage rollback preserving existing rows, process/MCP caller failure without visible or durable orphans, healthy retries, and busy acquisition after an already-created snapshot. A separate read-only review found no remaining defect within these repaired boundaries.
- Real Agent verification passed: second process-batch INSERT failure preserved exact preexisting history across restart; removing the trigger allowed two Jobs to complete at capacity2. Post-VACUUM lock contention left no staging directory and kept original DB/backup bytes; releasing the lock allowed migration and execution to retry successfully. MCP is covered by regression at its registration boundary, not a new full external-server runtime scenario.
