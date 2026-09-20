# WP1 confirmation and run lifecycle closure

## Goal
Finish the remaining Hub confirmation ownership and run/wait lifecycle seams in one coordinated implementation batch. Preserve wire/permissions and established error projections; prove changed behavior with regressions and a real isolated Hub scenario.

## Current Phase
Contract discovery — in_progress.

## Phases
1. Contract discovery: verify Hub/Agent confirmation and durable run transitions; freeze cross-slice interfaces. Commit execution contracts separately.
2. Implementation: independent confirmation/lifecycle and run/dispatch ownership slices; integration owner Main. Skip worker validation while editing; validate integrated code. Separate focused commits for the two implementation slices.
3. Joint verification: regressions and real Hub callback/reconnect/late-response smoke, targeted Agent confirmation checks where affected; record exact results and conclude WP1 status from evidence.

## Constraints
- Prior hub-control-boundary completion is accepted. Do not re-run its acceptance to challenge it.
- No propose panel. User delegated technical coordination and ordinary decisions to Main.
- No fabricated run identity for confirmation messages; no new wire/schema merely to satisfy architectural wording.
- Keep permission defaults, tool contracts, and existing error mappings unless a demonstrated bug requires a narrow change.
- Agents research separate slices; Main owns shared interfaces, integration, records, and commits.
- Do not silently expand into WP2/WP3/Room feature work.

## Verification contract
Reproduce the concrete races/state regressions before their fixes where practical; retain meaningful boundary regressions. Run affected Hub suite once after concurrent edits settle, then real isolated transport/callback/receipt scenarios. No workspace-wide suite as substitute.

## Frozen implementation contracts

### Confirmation slice
- Own Hub confirmation.rs (new), main.rs/state.rs, agents/lifecycle.rs/lifecycle_tests.rs/test_support.rs, routes.rs info count and fixture migrations in notify/mcp/room; Agent confirmation.rs/hub.rs only for demonstrated teardown gaps.
- HubState replaces pending_confirmations with confirmations: Arc<crate::confirmation::Confirmations>; new()/pending_count().await are the shared constructor/query contract. Map and pending record become private to confirmation module.
- Admission captures (agent_id, connection_id, request_id, original sender) and registers while lifecycle current guard is held. Network publication starts after that guard is released. No run_id/wire addition.
- Replace and disconnect retire exact removed generation, claim unresolved sessions, enqueue ProviderUnavailable(reason=provider_unavailable) to original sender before old Close where transport remains writable. Agent disconnect failure is still the fallback for broken transports; never infer remote job cancellation.
- All terminal paths use one claim, take/clear captured sender, and cannot send a second result or reselect a new connection. Resolved callback remains 409 until expiry, then 410; invalid token/unknown/action mappings remain existing 403/404. Disabled/config-invalid/publish-failure reasons unchanged. Confirmation expiry and MCP temporary allow capabilities unchanged.
- Extract actual confirmation business and callback handling from main, leaving composition. Do not broaden permissions or make session state durable.

### Run slice
- Own runs.rs, agents/dispatch.rs/dispatch_tests.rs, plus Agent transport reliable handler only if coordinated with confirmation worker; default no Agent source edits from run worker.
- Keep existing dispatch HTTP/MCP errors and all timeout parameters. Only new receipt status is not_sent with reason agent_offline after a proven channel-send failure; exclude from replay. No DB schema/wire change.
- mark_dispatched only created -> dispatched; never erase remotely observed progress.
- mark_timeout only created/dispatched -> timeout_waiting_result; preserve acked/started/running/failed/unknown/completed/not_sent and their observation timestamps. Caller still gets existing timeout, which does not imply remote cancellation.
- Matching ACK records receipt but only advances created/dispatched/timeout_waiting_result -> acked; duplicate/late ACK cannot lower started/running/failed/unknown/completed/not_sent or replace their reason/time.
- Transport status accepts the evidenced started/running/failed/unknown set; unknown strings return existing mismatch result without mutation. Progress is monotonic started -> running, no running -> started; failed/unknown/result-bearing rows cannot be overwritten by ACK/status. Matching stale no-op updates succeed idempotently, foreign tuple still fails. Canonical Response remains able to complete failed/unknown/timeout rows and detect duplicate/conflicting results.
- Preserve upsert_agent_report terminal/result invariants and stale-ACK cleanup; do not conflate Agent ledger duplicate outcomes with transport statuses.

### Integration / regression handoff
- Both implementation workers first write baseline-compiling wp1_* regressions using only current APIs, report ready, then wait for Main's red-run result before production edits. Workers skip formatter/linter/build/tests/commits.
- Confirmation worker owns all shared HubState fixture migrations except runs.rs, which run worker migrates to confirmations: Arc::new(crate::confirmation::Confirmations::new()). No concurrent edits to the same file.
- Main owns contracts/records, targeted red/green commands, live smoke, final integration and per-phase commits.
