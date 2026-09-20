# Hub 连接代际隔离进度

## 2026-09-16
- Approved plan loaded from inline prompt and durable local handoff.
- Created dedicated planning files; no production edits yet.
- Initial `git diff --stat` was clean.
- Planning skill resolver path from fallback was absent; manual selected-plan initialization used.
- Source mapping complete: current handler has a TOCTOU gap; replacement/disconnect/expiry callers are confined to agents.rs plus room test helper.
- LSP confirmed four handler references, five replacement references, and four disconnect references.
- First post-rewrite `cargo check -p agentic-gpt-hub` found a removed `REQUEST_TIMEOUT_SECS` import; restored it before continuing.
- Stage-one handler rewrite completed: agents guard now serializes lifecycle admission and side effects; reliable branches retain their existing run contracts after guard release.
- `cargo check -p agentic-gpt-hub` passed after restoring the existing timeout import.
- Pre-fix direct regression reproduced the old `Ok(())`; post-fix direct regression passed.
- Existing `agents::tests` passed (15) and `room::tests` passed (12).
- Stage-one production changes committed as `775b17b` (`fix(hub): isolate stale agent generations`); staged tests remain with the Stage Two API/test bundle because their fixtures exercise replacement retirement.
- Interactive staging initially timed out in a foreground PTY; the supervised `git-stage` process completed the split. A cleanup-loop edit briefly dropped its `for`; exact-function reread restored it and Hub check passed.
- `cargo fmt --all -- --check` passes after formatting the affected Hub source/tests.
- Generation-focused suite passes: 9 tests; full `cargo test -p agentic-gpt-hub` passes: 73 tests.
- Release binary build passes via `cargo build -p agentic-gpt-hub --release`.
- Real isolated transport smoke passes against the release binary: `generation-smoke: ws=ok sse=ok stale=409 late-result=completed`. The first peer attempt exposed only an incomplete temporary WebSocket framing helper; extended-length framing fixed it without changing Hub code.
- Smoke workspace and peer script removed; final tracked tree is clean, with only the intentional durable `.planning/2026-09-16-hub-connection-generation/` directory untracked.
- Added the permanent route-level SSE regression: current `Heartbeat` POST returns `200`, emits `HeartbeatAck`, and advances registry `last_seen_at`; post-hardening Hub suite passes 74 tests.
- Rechecked the advisory lock path: `request_active_room` clones and drops its `active_room` guard before awaiting `agents`; no deadlock blocker or production reorder is required.
- Amended `8fa47ad` (`fix(hub): retire replaced agent connections`) to include the route-level regression while retaining the two-commit implementation history.
