# Slice 04 — node_repl Process Lifecycle

This is an orchestrator-owned implementation contract. Worker may implement it but must not change its architecture or scope.

## Goal

Connect `NodeReplKernel` to a real rmcp `TokioChildProcess` using the already-built `NodeReplLaunchSpec`, with a known-good MCP initialize contract and bounded low-level shutdown.

This slice is only the process/MCP lifecycle underneath a kernel. It does not implement Browser SDK bootstrap, reset/release semantics, named leases, AppState, or model tools.

## Client initialize contract

Add a private helper in `browser_kernel.rs` that builds the rmcp `ClientInfo` used for node_repl.

It must use:

- MCP protocol version exactly `2025-06-18` using rmcp 1.7.0's supported `ProtocolVersion` representation;
- default/empty client capabilities;
- implementation name `agentic-browser-runtime`;
- implementation version `env!("CARGO_PKG_VERSION")`.

Do not use `ClientInfo::default()` for production node_repl startup because rmcp 1.7.0 currently defaults to a newer protocol than the locally verified PoC.

## Command construction

Add a private helper that turns `&NodeReplLaunchSpec` into a direct `tokio::process::Command`:

- executable: `spec.program` directly; no shell and no `sh -lc`;
- current directory: `spec.cwd`;
- inherit the current process environment normally;
- apply all `spec.env_overrides` on top via command env overrides;
- do not independently add Browser/Neko/security/auth variables here;
- leave stdin/stdout ownership to `TokioChildProcess`;
- keep stderr inherited unless rmcp's construction API requires spelling that explicitly.

## Spawn API

Add:

```rust
pub(crate) async fn spawn(
    spec: &NodeReplLaunchSpec,
    session_id: String,
    turn_id: String,
) -> Result<NodeReplKernel>;
```

Behavior:

1. Validate session/turn ids **before** spawning a process, using the same rules/errors as Slice 03.
2. Build the direct Tokio command from the launch spec.
3. Create `TokioChildProcess` and serve it with the explicit node_repl `ClientInfo` above.
4. Bound the initialize/serve handshake to **10 seconds** (matching the proven PoC's initialization bound).
5. On success, construct the kernel around that initialized persistent client and the supplied ids.

Stable error prefixes:

- process/transport construction failure: `browser_runtime_node_repl_spawn_failed:`
- initialize/serve failure: `browser_runtime_node_repl_initialize_failed:`
- initialize timeout: `browser_runtime_node_repl_initialize_timeout`

If initialize times out/fails after a child was created, do not add an ad-hoc PID-kill subsystem in this slice; let rmcp/TokioChildProcess ownership/drop cleanup run. A later live smoke will verify actual orphan behavior.

## Low-level shutdown API

Add:

```rust
pub(crate) async fn shutdown(self) -> Result<()>;
```

This is transport/kernel shutdown only. It is **not** Browser `release` and must not call `turn_ended`, close tabs, rotate ids, or claim stronger Browser cleanup semantics.

Implementation contract:

- consume the kernel, so no later JS calls are possible;
- call rmcp `RunningService::cancel()` / equivalent cancellation path that reaches transport close;
- bound the whole awaited shutdown to **6 seconds**;
- success returns `Ok(())` regardless of the rmcp quit-reason value if the service joined successfully;
- rmcp/service join failure returns `browser_runtime_node_repl_shutdown_failed:...`;
- outer timeout returns `browser_runtime_node_repl_shutdown_timeout`.

The 6-second bound intentionally exceeds the inspected rmcp cancellation drain + child graceful wait window. If the outer bound fires, returning promptly is more important than claiming synchronous child-death proof; rmcp/service drop cleanup owns the fallback kill attempt.

## Refactor allowed inside existing kernel

Refactor Slice 03's id validation into a private helper so both `spawn` and the private in-memory constructor validate before use. Do not change JS call semantics.

## Tests

Keep tests in `browser_kernel.rs`; no real official node_repl is required yet.

Cover at least:

1. node_repl `ClientInfo` advertises exactly protocol `2025-06-18`, expected implementation name/version, and default capabilities;
2. command helper uses direct executable, exact cwd, and applies env overrides without a shell wrapper;
3. invalid session/turn id fails before any spawn path is required;
4. impossible executable path returns `browser_runtime_node_repl_spawn_failed:...` rather than hanging;
5. existing in-memory fake server initialization sees the explicit `2025-06-18` client protocol rather than rmcp default;
6. `shutdown` cleanly terminates an in-memory running service;
7. Slice 03 JS/metadata/result tests remain unchanged and passing.

Do not build a test-only production dependency-injection framework. A real installed-runtime smoke test belongs to a later slice after base-env/runtime source wiring exists.

Run:

- `cargo test -p agentic-gpt browser_kernel`
- `cargo test -p agentic-gpt browser_runtime`
- `cargo fmt --all -- --check`
- `git diff --check`

## Worker observation rules

- Update existing `progress.md` at start/completion.
- Append to `findings.md` only if rmcp 1.7.0 contradicts or blocks this contract.
- Do not modify `PLAN.md` or any slice contract.
- No commits or push.
