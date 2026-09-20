# Hub control-boundary findings

## Approved scope
- Split existing `crates/agentic-gpt-hub/src/agents.rs` into agents/{mod,transport,lifecycle,dispatch,test_support} modules.
- Stage two wraps connection map and pins Room dispatch target; preserve existing contracts.

## Evidence
- Worktree branch: `refactor/architecture-cleanup`; source changes are intentional; prior generation planning directory remains separate.
- CodeGraph exists but still indexes the old `agents.rs` path after LSP rename; current reads/searches are authoritative.
- Stage-one ownership now matches the plan: transport owns route/auth/framing; lifecycle owns currentness/admission/replacement/disconnect/cleanup and calls dispatch reliable handling/replay; dispatch owns pending waiter, request/reliable run path, replay encoding, cached jobs, and all-agent MCP aggregation.
- `HubState.agents` remains `Arc<Mutex<HashMap<String, AgentConnection>>>` for stage one. `HubState.pending` and `state::PendingResponse` are gone; dispatch pending is private.
- Main route registrations use `agents::transport`; cleanup uses `agents::lifecycle`; routes/mcp/notify/room use `agents::dispatch`; all in-crate HubState constructors have a Dispatch owner.
- WS auth now uses the same `require_agent_secret` implementation as SSE/POST. Existing response/error strings and upgrade timing are unchanged.
- `request_agent` borrows `command.request_id()` for prepare_run, then moves the original command into the envelope; no duplicate request-ID getter/setter remains.
- Existing tests are physically split into owner files. Dispatch waiter hash tests access `state.dispatch.pending` only from dispatch child tests; generation fixtures remain in lifecycle tests; transport tests use `SseConnectQuery::for_test`.
- Production `cargo check -p agentic-gpt-hub` and test-target compile `cargo test -p agentic-gpt-hub --no-run` pass. Remaining warnings are unused test/import cleanup only.

## Tool errors
- Tried reading `crates/agentic-gpt-hub/src/lib.rs`; file does not exist because declarations live in main.rs.
- Initial `agents/mod.rs` skeleton write happened before extracting the renamed source; restored exact HEAD content through `git show` before moving blocks. No behavioral work was based on the truncated file.

## Next
- Run Stage One Hub tests, clean warnings, inspect old boundary strings, then commit the stage-one refactor before beginning Connections/Room target work.
