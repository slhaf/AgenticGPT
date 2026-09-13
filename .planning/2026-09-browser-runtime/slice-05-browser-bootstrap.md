# Slice 05 — Browser SDK Bootstrap

This is an orchestrator-owned implementation contract. Worker may implement it but must not change its architecture or scope.

## Goal

Add the internal bootstrap step that turns an initialized persistent node_repl kernel into a Browser-SDK-ready kernel by importing the selected runtime's official `browser-client.mjs` and establishing stable convenience bindings for later arbitrary `browser.repl` JavaScript.

This slice does not add named leases, AppState, model tools, reset/release, runtime config sources, or a live installed-runtime smoke.

## Stable bindings

The V1 internal REPL environment after successful bootstrap owns exactly these convenience globals:

- `globalThis.agent`: the object returned by official `setupBrowserRuntime()`;
- `globalThis.browser`: the Browser object returned by `agent.browsers.get("chrome")`.

Do not create a stable `globalThis.chrome` alias in Agentic V1. The external Browser object is intentionally named `browser`; the selected current backend remains `"chrome"` inside bootstrap.

## API

Extend `NodeReplKernel` with:

```rust
pub(crate) async fn bootstrap_browser(
    &mut self,
    browser_client_path: &Path,
) -> Result<()>;
```

The supplied path comes from the already-selected `BrowserRuntimeDescriptor`; this method performs no registry/config discovery.

## Bootstrap JavaScript contract

Convert `browser_client_path` to a UTF-8 string or return:

`browser_runtime_path_not_utf8:browser_client_path`

Encode the path as a proper JavaScript string literal using JSON serialization; never interpolate an unescaped raw path.

Execute one `js` call with timeout `20_000` ms whose semantics are:

```js
if (globalThis.agent == null) {
  const { setupBrowserRuntime } = await import(<JSON-escaped browser client path>);
  globalThis.agent = await setupBrowserRuntime();
}
if (globalThis.browser == null) {
  globalThis.browser = await globalThis.agent.browsers.get("chrome");
}
nodeRepl.write(JSON.stringify({ browserId: globalThis.browser.browserId }));
```

Minor formatting differences are fine; semantics are not.

The null checks make repeated bootstrap calls in the same JS kernel idempotent: do not re-import/recreate bindings when both already exist.

## Result/error contract

- If the underlying `js` call returns an rmcp/service `Err`, preserve its existing `browser_runtime_node_repl_call_failed:...` error; do not wrap it a second time.
- If `js` returns `CallToolResult` with `is_error == Some(true)`, return stable `browser_runtime_browser_bootstrap_failed`.
- Any other result is considered successful; bootstrap output content is diagnostic and is not exposed by this API.

Do not parse `browserId` in Rust in this slice. Browser object validity beyond the official bootstrap call is left to the Browser SDK/runtime itself and later live smoke.

## Why bootstrap is separate from spawn

Do not merge bootstrap into `NodeReplKernel::spawn` in this slice. Process/MCP startup and Browser SDK readiness remain separate primitives so later reset/recovery can re-bootstrap an existing or respawned kernel explicitly.

## Non-goals

- no registry/config discovery
- no Desktop/Orange Pi env source adapter
- no `js_reset` or `turn_ended`
- no reset/release semantics
- no session/turn rotation
- no named lease / idle timeout
- no AppState manager
- no `browser.*` model tools
- no Neko/backend compatibility logic
- no real installed-runtime smoke test yet

## Tests

Reuse the in-memory fake rmcp server in `browser_kernel.rs`; do not introduce a JavaScript interpreter. Tests inspect the exact JS code sent to the fake `js` tool and returned result behavior.

Cover at least:

1. bootstrap sends one `js` call with `timeout_ms = 20_000`;
2. generated code imports the JSON-escaped supplied browser client path and calls `setupBrowserRuntime()`;
3. code establishes `globalThis.agent` and `globalThis.browser` with `browsers.get("chrome")`;
4. code contains null/idempotency guards for both bindings;
5. a path containing quotes/backslashes is safely JSON-escaped rather than raw-interpolated;
6. tool result `isError: true` becomes `browser_runtime_browser_bootstrap_failed`;
7. rmcp/service failure preserves the existing `browser_runtime_node_repl_call_failed:...` prefix;
8. existing kernel/runtime tests remain passing.

Avoid testing JavaScript formatting character-for-character; assert required semantic substrings/values so harmless formatting refactors do not create maintenance churn.

Run:

- `cargo test -p agentic-gpt browser_kernel`
- `cargo test -p agentic-gpt browser_runtime`
- `cargo fmt --all -- --check`
- `git diff --check`

## Worker observation rules

- Update existing `progress.md` at start/completion.
- Append to `findings.md` only for a real contract/runtime contradiction or blocker.
- Do not modify `PLAN.md` or any slice contract.
- No commits or push.
