# Browser Runtime V1 — Progress

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


