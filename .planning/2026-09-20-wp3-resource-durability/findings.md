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

## Current-source findings
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
