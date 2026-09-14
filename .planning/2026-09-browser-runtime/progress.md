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

Slice 07 reset lifecycle contract frozen by orchestrator after a read-only check of the current installed official node_repl resources. `tools/list` confirms `js_reset` has an empty input schema and `turn_ended` requires `hook_event_name/session_id/turn_id`, with repeated same session/turn notifications ignored; the official Chrome plugin invokes it from the `Stop` hook. No local official evidence requires id rotation after reset, so healthy in-place reset retains opaque ids while a manager respawn naturally receives fresh ids. Implementation is next; AppState/tool/config wiring remains out of scope.
Slice 07 start checkpoint: re-read the frozen PLAN.md and Slice 07 contract plus accepted kernel/manager source. Local CodeGraph was initialized because the shared index pointed at another worktree. The planning session-catchup helper advertised by the skill is unavailable at its documented path; this is tooling-only and does not change the slice contract. Implementation is split by file ownership: kernel reset primitives in `browser_kernel.rs`, manager reset/state-machine integration in `browser_manager.rs`; the manager consumes `turn_ended(&mut self)`, `reset_js(&mut self)`, and existing `bootstrap_browser(&mut self, &Path)` without changing the frozen lifecycle boundary.
Slice 07 kernel checkpoint: `NodeReplKernel::turn_ended` now sends the exact `turn_ended` Stop/session/turn payload and `reset_js` sends `js_reset` with `{}`. Both preserve the existing rmcp transport error prefix and convert `isError: true` results to stable lifecycle failures. The delegated kernel tests cover wire shape and both failure classes; manager integration remains in progress.
Slice 07 manager checkpoint: `BrowserRuntimeManager::reset` now serializes in-place reset/recovery through the existing per-entry lifecycle mutex, stores the selected browser client path for bootstrap, preserves exact-entry identity on recovery, and removes failed recovery entries. `ManagedKernel`/`FakeKernel` forward and record reset primitives. Focused manager tests cover healthy order, concurrency, initialization, closed-kernel recovery, each in-place failure fallback, failed recovery retry, reaper refresh, and closing/removed behavior. No AppState/tool/config/runtime-adapter changes were made.
Slice 07 manager focused verification: `cargo test -p agentic-gpt browser_manager` passed (18 tests, 0 failures; 30 warnings). The warnings are the expected temporary dead-code warnings for the intentionally unwired internal runtime/manager APIs. Kernel and runtime focused suites plus formatting and diff checks remain.
Slice 07 verification checkpoint: focused suites passed: `cargo test -p agentic-gpt browser_kernel` (22 tests, 0 failures) and `cargo test -p agentic-gpt browser_runtime` (17 tests, 0 failures). The first `cargo fmt --all -- --check` reported rustfmt-only layout differences in the two touched Rust files; `cargo fmt --all` applied those mechanical changes. `git diff --check` was rerun after formatting and is still required, along with the final fmt check.
Slice 07 final verification: after rustfmt, `cargo test -p agentic-gpt browser_manager` passed (18 tests), `cargo test -p agentic-gpt browser_kernel` passed (22 tests), and `cargo test -p agentic-gpt browser_runtime` passed (17 tests), all with 0 failures. `cargo fmt --all -- --check` passed and `git diff --check` passed. No findings entry was needed: implementation matched the frozen runtime evidence and contract. No AppState, model-facing tools, config, runtime adapters, Neko changes, or contract-file edits were made.
Slice 07 review follow-up: strengthened `reset_respawns_known_closed_kernel_in_same_entry` to assert the lease entry identity is preserved while the fake kernel token changes. The helper now uses the fake kernel's stable `closed` allocation as its kernel identity rather than the fixed `Option` field address. Re-ran all three focused suites after formatting: manager 18 passed, kernel 22 passed, runtime 17 passed; final fmt and diff checks passed.
Slice 07 latest verification: the post-strengthening command completed successfully after the kernel-token assertion/helper edit: `cargo test -p agentic-gpt browser_manager` 18 passed, `browser_kernel` 22 passed, and `browser_runtime` 17 passed, all with 0 failures. The subsequent `cargo fmt --all -- --check && git diff --check` also passed with no output.
Slice 07 orchestrator review: independently reviewed the reset state machine and reran the three focused suites plus fmt/diff checks; all remained green (manager 18, kernel 22, runtime 17). Recovery transitions to `initializing` before dropping the per-lease lifecycle mutex, performs old-kernel shutdown/factory work without the manager map lock, and either installs the fresh kernel into the same exact entry or removes the unusable entry on recovery failure. Release/acquire/repl/reaper behavior remains serialized through the accepted state machine. A broader `cargo test -p agentic-gpt` run reached 411/412 unit tests but failed the unrelated pre-existing `stdio_server::tests::stdio_resumes_stale_logical_session_before_first_tool_call` timeout; rerunning that exact unchanged test alone reproduced the same timeout, while Browser focused suites stayed green. Review also promoted the verified node-repl same-session/turn `turn_ended` deduplication into `findings.md`; repeated in-place reset ID semantics remain a later installed-runtime smoke item rather than an invented rotation rule.

Slice 08 final-cleanup contract frozen: now that Slice 07 provides `turn_ended`, explicit release and idle expiry can close the deferred product-contract gap by sharing `turn_ended -> bounded shutdown -> exact-entry removal`. Reset recovery, already-closed acquire cleanup, and bootstrap-failure cleanup deliberately remain shutdown-only because they are not equivalent final-release paths. AppState/tool/config/manual work remains out of scope for this slice.

Slice 08 implementation completed: added one manager-side final-cleanup helper that conditionally signals `turn_ended`, always attempts bounded shutdown, preserves single-stage errors, and emits the bounded `browser_runtime_final_cleanup_failed` diagnostic for dual failure. Explicit release and idle expiry now use it outside manager/lifecycle locks; reset recovery, closed-kernel acquire cleanup, and bootstrap-failure cleanup remain shutdown-only. Added focused release/reaper ordering, failure, removal, and reset-regression coverage. No findings contradiction was identified.

Slice 08 verification:
- `cargo test -p agentic-gpt browser_manager`: 21 passed, 0 failed.
- `cargo test -p agentic-gpt browser_kernel`: 22 passed, 0 failed.
- `cargo test -p agentic-gpt browser_runtime`: 17 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
Slice 08 orchestrator review found that the first implementation still awaited `turn_ended` without a bound, so a stuck graceful signal could violate the frozen bounded-cleanup contract before the already-bounded kernel shutdown was reached. Final cleanup now gives `turn_ended` its own 6-second production timeout and still proceeds to shutdown after timeout; an injected short-timeout test proves the stalled signal path reaches shutdown. After that fix and a small unused-mut cleanup, final verification passed: `browser_manager` 22, `browser_kernel` 22, `browser_runtime` 17, plus `cargo fmt --all -- --check` and `git diff --check`.

Slice 09 desktop-launch contract frozen after live evidence: the TTY-era `Browser is not available: chrome` result was host/backend absence, not a broken node_repl launch. Starting the existing default Chrome profile on temporary Xvfb/VNC made the ChatGPT extension backend appear; a reduced registry-derived environment plus `NODE_REPL_TRUSTED_CODE_PATHS` then completed `setupBrowserRuntime -> browsers.list -> browsers.get("chrome")` successfully while omitting the other Codex-config-only Browser variables. Slice 09 implementation completed: added ordered/deduplicated `trusted_code_paths` derived from Desktop `codexHome + nodeModuleDirs`, merged caller and descriptor path lists with platform path-list helpers, and preserved existing launch overrides. Browser bootstrap now lists advertised backends before requesting Chrome, emits an Agentic-owned sentinel when Chrome is unavailable, and maps only that sentinel to `browser_runtime_browser_unavailable`; ordinary tool and rmcp failures retain existing classifications. Added focused runtime/kernel regression coverage. No Codex TOML/config, extra Browser environment variables, AppState/tool wiring, backend adapters, or Chrome auto-start were added. Verification: `cargo test -p agentic-gpt browser_runtime` 20 passed, `browser_kernel` 23 passed, `browser_manager` 22 passed; `cargo fmt --all -- --check` and `git diff --check` passed. Orchestrator review replaced sentinel detection by whole-Content JSON equality with rmcp's text accessor so optional annotations/meta cannot hide the Agentic-owned marker, reran the same focused suites successfully, and then repeated the installed-runtime smoke using the exact reduced environment and Slice 09 bootstrap sequence against the temporary Chrome extension backend; it returned `browserId: "1"` with `isError: false`.

Slice 10 AppState ownership contract frozen: production startup may opportunistically resolve the current Desktop runtime into one shared descriptor+manager context, but Browser runtime absence must not make Agentic startup fail. AppState/upper layers remain source-agnostic so a later explicit path or Agentic-managed runtime cache can replace/add discovery without changing Browser lifecycle or tool consumers. This slice does not expose Browser tools or auto-start/probe Chrome.

Slice 10 implementation completed: AppState now owns an optional clone-cheap descriptor plus shared BrowserRuntimeManager context. Production build_app_state uses the current Desktop registry, an empty Browser-specific base environment, and logs one bounded generic warning while continuing startup when discovery or launch-spec construction is unavailable. All ordinary test AppState literals use None; no host Desktop discovery is used by tests.

Slice 10 verification:
- `cargo test -p agentic-gpt browser_`: 65 passed, 0 failed.
- `cargo test -p agentic-gpt`: 419 passed, 1 failed on the pre-existing `stdio_server::tests::stdio_resumes_stale_logical_session_before_first_tool_call` deadline timeout.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
Slice 10 orchestrator review kept the source-agnostic optional context shape and changed missing Desktop runtime from a generic warning into a bounded informational diagnostic with `source`, construction `stage`, and the stable error text; absence is an expected degraded capability, not an Agentic health fault. Reverification passed: all 65 Browser-focused tests, `cargo test -p agentic-gpt --no-run` for the full package compile surface, fmt check, and diff check. The temporary dead-code warnings are expected until the next tool-surface slice consumes the AppState context.

Slice 11 runtime-manual contract frozen: the selected official docs tree is small but remains external runtime data, so Browser gets its own bounded docs-root-relative read/literal-search layer with traversal/symlink/UTF-8/scan/output limits. This internal slice stays source-agnostic and does not expose `browser.manual` or change toolsets/AppState; the later surface layer will bind it to `state.browser_runtime.descriptor.docs_root`.
