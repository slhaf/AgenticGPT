# Hub 连接代际隔离发现

## Approved inputs
- User-approved execution plan is authoritative; previous Response→durable run→run-key waiter work is baseline and not re-audited.
- Production scope centers on `crates/agentic-gpt-hub/src/agents.rs`; Room/main/routes/client are seam references.
- No wire fields, protocol version, DB schema, security default, or Agent production interface changes.

## Evidence log
- Initial worktree diff stat was clean.
- `agents.rs` currently snapshots currentness in `post_agent_message`, touches separately, and passes `touch_current`; WS passes `true` for every parsed message. `handle_agent_message` then re-locks agents for Hello metadata and sender lookup, while Room/jobs/DB side effects occur in separate lock sections.
- Current `replace_agent_connection` inserts before releasing Room and closing the old sender; it returns `()`, allows empty/duplicate IDs, and `connect_agent_sse` touches registry before replacement.
- Current `disconnect_agent` removes exact current under agents, then releases Room and discards confirmations after dropping the guard; expiry snapshots stale candidates and deletes by ID without rechecking TTL.
- `room::request_active_room` releases `active_room` before taking agents; Room register/release helpers only lock `active_room`. `discard_agent_confirmations` only locks `pending_confirmations`; `send_confirmation_response` re-locks agents and cannot run under agents.
- Existing tests use direct private handler and replacement fixtures; room helper currently discards replacement result. Existing SSE docs describe stale lifecycle rejection and reliable late acceptance but omit unified WS semantics and ID collision status.
- Existing active planning pointer referenced the completed architecture-audit plan; this task uses `.planning/2026-09-16-hub-connection-generation/`.
- Shared handler now acquires `state.agents` once, rejects stale lifecycle messages before touch, updates only matching current last-seen (SSE registry best-effort), holds the guard through Hello/Room/boot/jobs/RunReport/Heartbeat/confirmation admission, and drops it before reliable run handling or replay.
- `post_agent_message` now delegates currentness to the handler and maps only `stale_connection` to HTTP 409; WS passes its bound connection ID without a boolean.
- Pre-fix direct handler test failed with `Ok(())`; after the rewrite, Hub cargo check passed and the direct regression plus existing `agents::tests` (15) and `room::tests` (12) passed.
