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

## Verified implementation and review
- Agent outer WS/SSE/reporting teardown calls the existing pending-confirmation drain with provider_unavailable; no Agent production change was warranted. Captured Hub sender delivery remains best-effort on broken transports.
- Integrated regressions establish canonical-result precedence, monotonic remote progress, invalid-status rejection, proven-send-failure replay exclusion, callback single claim and original-sender retirement. Six real Hub HTTP/SSE scenarios additionally exercised callbacks, provider failure, timeout and late receipts using simulated protocol peers.
- Review found a distinct reusable-connection-ID retirement race in the initial implementation: removing/replacing a connection and later claiming confirmations outside the current-connection guard leaves an admission window. The selected correction retains lock order current connections -> confirmations and completes retirement under the existing guard; it introduces no new ID scheme, protocol field or network await.
- Real Agent execution/ledger restart, external ntfy delivery, active Room and OAuth restart behavior were not exercised by this batch. The isolated Hub restart only proves persisted receipt reads and empty transient counters.
