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


Slice 06 preparation checkpoint: read the repository guidance, frozen PLAN.md, slice contract, current progress, and findings. The local worktree has no .codegraph index; the planning session-catchup helper path advertised by the skill is unavailable in this environment. No contract/runtime contradiction was found and no findings entry was added. Source implementation and focused lifecycle tests are next.
Slice 06 implementation checkpoint: the minimal `NodeReplKernel::is_closed()` observation is in place. The delegated manager worker was stopped after producing no source edits; manager implementation is now being completed directly within the frozen slice boundary. No findings contradiction was identified.
Slice 06 verification checkpoint: the manager focused suite passes (9 tests), and the accepted kernel/runtime suites pass (16 and 17 tests). The first manager test compile exposed only a test-closure ownership error, which was corrected; `cargo fmt --all` applied the resulting rustfmt-only layout, then `cargo fmt --all -- --check` and `git diff --check` passed. Expected dead-code warnings remain because manager/runtime APIs are intentionally not wired to AppState or tools in this slice.
Slice 06 orchestrator review: reviewed the full manager implementation against the frozen contract and found one real waiter race: an entry could be removed and `notify_waiters()` fire after map lookup but before the waiter registered, leaving `acquire` / `repl` / `release` waiting forever on a stale entry. The wait paths now register first and re-check exact map ownership before awaiting, closing that lost-notification window. Review also added manager-level coverage for acquire racing an in-progress closing release and for repeated release idempotence. Final focused verification: `browser_manager` 10 passed, `browser_kernel` 16 passed, `browser_runtime` 17 passed; `cargo fmt --all -- --check` and `git diff --check` passed. Scope remains internal manager + minimal `NodeReplKernel::is_closed()` only; no AppState/tool/config/reset wiring was added.
