# Hub 连接代际隔离执行计划

## Goal
在不改变 wire、DB schema、安全默认或上一批可靠 run 合同的前提下，统一 WS/SSE 入站 current-generation 准入，并让替换、断开、TTL 清理共享同一 agents 锁线性化边界。旧连接不得污染当前 metadata、Job cache、RunReport、last_seen 或 Room lease；合法可靠迟到结果继续走既有归属逻辑。

## Status
完成；两阶段提交与真实 WS/SSE smoke 均通过。

## Phases
### Preparation
- [x] Confirm current symbols and lock-order constraints
- [x] Create direct stale-heartbeat red regression
- **Status:** complete

### Stage One
- [x] Implement generation-safe shared message handler
- [x] Migrate WS and SSE entrypoints
- [x] Preserve current-only side effects atomically
- [x] Preserve reliable late-message behavior
- [x] Commit stage-one production and tests
- **Status:** complete

### Stage Two
- [x] Make replacement registration generation-safe
- [x] Make disconnect and expiry recheck ownership
- [x] Migrate replacement callers and room fixtures
- [x] Add V2 retirement and collision coverage
- [x] Record WS/SSE contract and rollout boundaries
- [x] Run full Hub regression suite
- [x] Commit stage-two source tests and docs
- **Status:** complete

### Verification
- [x] Build Hub release smoke binary
- [x] Run isolated WS/SSE real transport smoke
- [x] Record bounded verification evidence
- **Status:** complete

### Delivery
- [x] Inspect final diff and commits
- [x] Confirm no temporary artifacts remain
- **Status:** complete
### Hardening
- [x] Add route-level SSE heartbeat regression
- [x] Run post-hardening Hub verification
- [x] Amend retirement commit
- **Status:** complete

## Decisions
- `state.agents` is the sole admission,副作用, replace, and disconnect linearization boundary.
- Lock order is agents → active_room/boot_generations/jobs/pending_confirmations; no network, oneshot, task wait, or agents re-entry under the guard.
- Reliable messages are Response, TransportAck, and TransportRunStatus; stale reliable messages do not touch current state.
- Empty connection IDs and same-current duplicate IDs are rejected; no historical ID registry.

## Errors Encountered
| Error | Attempt | Resolution |
|---|---:|---|
| planning skill absolute path missing | 1 | Used skill:// instructions and created the selected plan directory manually |
| pre-fix direct stale-heartbeat regression failed with `Ok(())` | 1 | Expected red evidence; implement shared current-generation admission before rerunning |
| shared handler check lost `REQUEST_TIMEOUT_SECS` import | 1 | Restored the existing import; no behavior change |
| interactive staging via foreground PTY timed out | 1 | Restarted through supervised hub process and completed the split interactively |
| cleanup loop edit temporarily removed its `for` statement | 1 | Re-read the exact function, restored the loop, and reran Hub check |
| initial real smoke peer rejected its oversized WebSocket Hello frame | 1 | Added RFC 6455 extended-length client framing and reran the unchanged Hub binary successfully |
