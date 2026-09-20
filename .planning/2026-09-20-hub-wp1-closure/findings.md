# WP1 findings

- User confirms hub-control-boundary implementation complete. Current agents module split exists; lifecycle owns current connections and dispatch owns pending waiters.
- Initial tracked git diff --stat is empty. Previous batch ledger records 75 Hub tests and real WS/SSE/tool/Room smoke; these are prior evidence, not current execution.
- Current confirmation seam read before this batch: lifecycle spawns handle_confirmation_request without connection identity; main stores agent/request only; callback reselects current sender by agent_id; replacement closes old sender without retiring corresponding confirmations. Agent disconnect behavior must be checked before claiming an unresolved local waiter.
- Confirmation and run/dispatch investigations run independently. No production edits yet.

## Verified closure decisions
- Hub confirmation claim must bind captured sender and connection generation before spawn; current replacement does not retire confirmations at all. Invalid/disabled config and publish-error paths must not reselect a current sender.
- Agent currently has request-id keyed waits and disconnect drains; scout's suggestion of an SSE gap is unverified until caller orchestration is traced.
- runs::mark_status unconditionally updates status/reason even when result exists; mark_dispatched/timeout share a generic updater that can lower acknowledged remote progress. Clearly failed send leaves created replay-eligible.
- Receipt status describes best-known remote/transport observation, not an immutable history of caller waiting. Local wait completion is observable in the already-returned timeout and removed pending waiter; a later matched ACK must advance timeout_waiting_result -> acked without recreating any waiter. It is not a reversal/cancellation of the caller's timeout. This intentionally rejects the advisory to freeze the receipt at timeout forever: doing so obscures new ACK knowledge and bypasses existing acked->unknown cleanup. No separate persistence axis is introduced in this batch.
- Two implementation workers have disjoint files and first add baseline-compatible failing regressions. Main will run them before releasing production edits.
