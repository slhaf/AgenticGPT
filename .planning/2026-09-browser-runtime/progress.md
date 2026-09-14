# Browser Runtime V1 — Progress

Status: Slice 06 named Browser session manager contract frozen; implementation starting.

Slice 06 scope: add an internal named lease manager over the accepted persistent `NodeReplKernel` primitives. Same-name first acquire must deduplicate initialization; different names remain concurrent; one per-lease lifecycle mutex serializes REPL/release/reaper ownership; release and idle expiry always remove the exact entry even after bounded shutdown failure; failed initialization leaves no stale entry. This slice does not add AppState/tool/config/reset wiring.

Status: Slice 05 browser SDK bootstrap accepted by orchestrator.

Slice 05 implementation: added the frozen Browser SDK bootstrap primitive to the persistent node_repl kernel. It uses one 20-second `js` call, JSON-escapes the supplied UTF-8 browser client path, establishes only `globalThis.agent` and `globalThis.browser` with null guards, preserves rmcp call failures, and maps tool errors to the stable bootstrap failure. Added in-memory fake-server coverage for the call, code semantics, escaping, timeout, and error behavior. No scope, binding, timeout, result/error, or non-goal changes were made.

Slice 05 verification:
- `cargo test -p agentic-gpt browser_kernel`: 16 passed, 0 failed.
- `cargo test -p agentic-gpt browser_runtime`: 17 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- Orchestrator review confirmed the browser client path is JSON-escaped before import, only `globalThis.agent` and `globalThis.browser` are established, the `chrome` backend choice remains internal to bootstrap, and tool-level bootstrap failure is not confused with rmcp transport failure.

Status: Slice 04 node_repl process lifecycle accepted by orchestrator.

Slice 01 runtime descriptor and desktop discovery accepted by orchestrator.

Added the internal runtime-discovery module and `main.rs` module declaration. The worker implementation stayed within the frozen slice. Orchestrator review added an explicit reverse-order assertion so the `updatedAt` selection contract is locked independently of input order.

Verification after review:
- `cargo test -p agentic-gpt browser_runtime`: 8 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- Expected temporary dead-code warnings remain because this slice intentionally introduces the descriptor/discovery API before a later slice consumes it; no `allow(dead_code)` suppression was added.

Slice 02 verification:
- `cargo test -p agentic-gpt browser_runtime`: 17 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- Review confirmed unrelated caller environment is preserved, runtime-coupled keys are replaced from the selected descriptor, non-browser trusted services survive while `browser` is replaced, and no Desktop/Neko/security policy is invented by this layer.
- Expected temporary dead-code warnings remain until later slices consume the descriptor/launch APIs; no warning-suppression attributes were added.

rmcp persistent-client mechanics investigation complete. Appended and source-path-checked exact rmcp 1.7.0/client-process/factory/result/handler evidence to `findings.md`; no implementation, tests, builds, git commands, or plan changes were made.

Slice 03 implementation completed: added the typed persistent node_repl kernel facade and in-memory rmcp tests within the frozen module boundary.

Slice 03 verification:
- `cargo test -p agentic-gpt browser_kernel`: 8 passed, 0 failed.
- `cargo test -p agentic-gpt browser_runtime`: 17 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- Review confirmed the fake server is reached through real rmcp duplex initialization, sequential calls reuse one initialized service, request `_meta` is observed server-side with the exact Codex session/turn shape, mixed result fields are preserved, and `isError: true` remains a successful transport-level return.
- Expected temporary dead-code warnings remain because this slice intentionally introduces the kernel API before a later slice consumes it; no warning-suppression attributes were added.

Slice 04 implementation completed: wired the persistent kernel to rmcp's direct Tokio child-process transport with the frozen initialize and bounded shutdown contracts. No protocol, timeout, command, shutdown, scope, or non-goal changes were made. No rmcp contradiction was found, so `findings.md` was not changed.

Slice 04 verification:
- `cargo test -p agentic-gpt browser_kernel`: 13 passed, 0 failed.
- `cargo test -p agentic-gpt browser_runtime`: 17 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- Orchestrator review confirmed direct executable/cwd/env construction, explicit MCP `2025-06-18` client initialization, the 10-second initialize bound, and the 6-second low-level shutdown bound. This layer still makes no Browser tab/service cleanup claims beyond rmcp transport shutdown.
- Expected temporary dead-code warnings remain because this slice intentionally introduces the process lifecycle API before a later slice consumes it; no warning-suppression attributes were added.


