# Slice 03 — Persistent node_repl Kernel Facade

This is an orchestrator-owned implementation contract. Worker may implement it but must not change its architecture or scope.

## Goal

Add the smallest typed facade around one already-initialized rmcp client so Agentic can preserve one node_repl MCP session across multiple JavaScript calls and attach the required Codex turn metadata without hand-rolling JSON-RPC.

This slice deliberately does **not** spawn a real process, choose an MCP protocol version, implement outer request timeout/cancellation, bootstrap the Browser SDK, or implement reset/release. Those are later slices.

## Module boundary

Add `crates/agentic-gpt/src/browser_kernel.rs` and register it from `main.rs` with:

```rust
mod browser_kernel;
```

Do not move runtime discovery/launch-spec code out of `browser_runtime.rs`.

## Kernel type

The module owns one internal persistent rmcp client:

```rust
pub(crate) struct NodeReplKernel {
    client: RunningService<RoleClient, ClientInfo>,
    session_id: String,
    turn_id: String,
}
```

The exact import paths may follow rmcp 1.7.0 conventions, but the semantic fields above are fixed.

Provide a **private** constructor used by this module's tests and by a later production spawn function added in the same module:

```rust
fn from_initialized_client(
    client: RunningService<RoleClient, ClientInfo>,
    session_id: String,
    turn_id: String,
) -> Result<NodeReplKernel>;
```

Reject empty/whitespace-only session or turn ids with stable errors:

- `browser_runtime_session_id_invalid`
- `browser_runtime_turn_id_invalid`

Do not generate IDs in this slice. Lease/turn identity policy belongs to the later manager/reset layer.

## JavaScript call API

Expose only:

```rust
pub(crate) async fn js(
    &mut self,
    code: &str,
    timeout_ms: u64,
) -> Result<CallToolResult>;
```

Behavior is fixed:

1. Build `CallToolRequestParams` for tool name `js`.
2. Arguments are exactly the JSON object:

```json
{
  "code": "<caller code>",
  "timeout_ms": <u64>
}
```

3. Attach request `_meta` through rmcp's typed `Meta` / request-meta API, not by manually serializing JSON-RPC. Metadata is exactly:

```json
{
  "x-codex-turn-metadata": {
    "session_id": "<kernel session id>",
    "turn_id": "<kernel turn id>"
  }
}
```

4. Call the persistent client's typed `call_tool` API.
5. Return the resulting rmcp `CallToolResult` unchanged, including `content`, images, `structuredContent`, `isError`, and result `_meta`. `isError == true` is still a successful transport-level return and must **not** be converted into `Err` here.
6. Convert only rmcp transport/service failures into an anyhow error prefixed `browser_runtime_node_repl_call_failed:`.

`timeout_ms` in this slice is the official `js` tool argument only. Do not add an outer Tokio timeout and do not send cancellation notifications yet; later lifecycle code will own the outer bound/cancellation policy.

## Persistence/serialization boundary

- Reuse the same `RunningService` stored in the kernel for every `js` call; do not reconnect or close between calls.
- `js` takes `&mut self`. This makes the lowest-level kernel API sequential by construction; the later named-lease manager will place the kernel behind the appropriate async mutex for cross-task serialization.
- Do not expose the underlying `Peer`, transport, or client from this facade.

## Non-goals

- no `TokioChildProcess` / process spawn
- no MCP initialize/protocol-version selection
- no process close/kill/release
- no outer call timeout/cancellation
- no `js_reset`
- no `turn_ended`
- no turn-id rotation
- no `setupBrowserRuntime()` / Browser SDK bootstrap
- no AppState / manager / named lease
- no tool namespace or browser tools
- no config/policy/audit/Neko work

## Tests

Tests stay in `browser_kernel.rs` and use an in-memory rmcp duplex/fake server (following the existing `mcp.rs` test pattern), never a real node_repl process.

Cover at least:

1. private constructor rejects empty/whitespace-only session id;
2. private constructor rejects empty/whitespace-only turn id;
3. one `js` call sends exact tool name, `code`, and numeric `timeout_ms`;
4. request `_meta` contains exact nested `x-codex-turn-metadata` session/turn values;
5. two sequential `js` calls use the same initialized service and same metadata without reconnect/reinitialize;
6. a `CallToolResult` containing mixed content (at least text + image if convenient), `structuredContent`, `isError`, and result `_meta` is preserved through the facade;
7. a tool result with `isError: true` remains `Ok(CallToolResult)` rather than becoming an anyhow error;
8. an rmcp/service failure becomes `browser_runtime_node_repl_call_failed:...`.

The fake server/test harness may record initialize count to prove persistence, but do not add a general production dependency-injection framework in this slice.

Run:

- `cargo test -p agentic-gpt browser_kernel`
- `cargo test -p agentic-gpt browser_runtime`
- `cargo fmt --all -- --check`
- `git diff --check`

## Worker observation rules

- Update existing `progress.md` at start/completion.
- Append to `findings.md` only if rmcp 1.7.0 or repository reality contradicts this contract.
- Do not modify `PLAN.md`, Slice 01/02, or this slice contract.
- No commits or push.
