# Slice 06 — Named Browser Session Manager

This is an orchestrator-owned implementation contract. Worker may implement it but must not change its architecture or scope.

## Goal

Add the internal `BrowserRuntimeManager` that owns multiple named persistent Browser kernels and freezes lease identity, per-lease serialization, release ordering, and idle-expiry ownership before any `browser.*` model tools or `AppState` wiring are added.

This slice consumes the already-accepted runtime/launch/kernel/bootstrap primitives. It does not add config, tool descriptors/dispatch, AppState ownership, Browser manual, reset semantics, Orange Pi/Neko adapters, or live installed-runtime smoke.

## Lease identity and creation

- Lease identity is the caller-supplied `name` string. Empty/whitespace-only names are rejected with `browser_runtime_lease_name_invalid`.
- `idle_timeout` must be non-zero; zero is rejected with `browser_runtime_idle_timeout_invalid`.
- First acquire for a name installs one initializing entry in the manager map before awaiting kernel creation. Kernel creation/bootstrap runs without holding the manager map lock.
- Concurrent first acquire of the same name must share that single initialization attempt; only one kernel factory invocation may be active for that entry.
- Different names may initialize and run independently/concurrently.
- If creation/bootstrap fails, the initializing entry is removed and all waiters are woken. No failed/half-created entry remains in the map. A later acquire may retry from scratch.
- A successful repeated acquire of an existing ready lease reuses the same kernel, refreshes activity, and adopts the newly supplied idle timeout.

The production constructor may capture an already-selected `NodeReplLaunchSpec` and browser client path. It generates opaque internal session/turn ids, spawns `NodeReplKernel`, then calls `bootstrap_browser`. These ids are never lease identity and are not model-facing.

## Per-lease operation serialization

Each lease owns a single lifecycle/operation mutex containing its kernel state. `repl`, release, and idle expiry contend on that same per-lease mutex:

- two `repl` calls on the same lease are serialized across the complete underlying `kernel.js(...).await`;
- operations on different leases do not share that mutex and may execute concurrently;
- `repl` refreshes lease activity when it starts and again when the underlying call completes (success or error), so a long active call cannot expire immediately only because its wall-clock duration exceeded the previous idle deadline.

`repl` forwards the `CallToolResult` from `NodeReplKernel::js` unchanged. Missing leases return `browser_runtime_lease_not_found`.

## Release ordering

`release(name)` is final lease cleanup for this manager slice, but still only has the low-level cleanup guarantee currently available from `NodeReplKernel::shutdown`; this slice does **not** invent `turn_ended` or tab cleanup semantics.

Ordering contract:

1. If the entry is still initializing, release waits for that initialization attempt to resolve.
2. For a ready entry, the first release owner atomically changes the entry to `closing` while holding the per-lease mutex and takes ownership of the kernel.
3. Kernel shutdown runs without the manager map lock held.
4. After bounded shutdown returns (success or error), the exact entry is removed from the manager map and waiters are notified.
5. A concurrent acquire that observes `closing` waits for removal, then creates a fresh entry/kernel under the same name. It must never reuse the closing kernel.
6. Repeated release after another release has already removed the entry is idempotent and reports that nothing was removed.

Shutdown errors are returned to the release caller **after** the entry is removed; a shutdown timeout/failure must not pin a permanently closing lease in the manager.

## Idle reaper ownership

The manager starts one lightweight background reaper for the whole map, not one timer per lease. The production scan interval is a small fixed duration; tests may use a shorter injected interval.

For each ready lease whose deadline has elapsed, the reaper attempts to take the same per-lease lifecycle mutex used by `repl`:

- if a `repl` already owns the mutex, the reaper waits; after the call completes it must re-check the refreshed activity/deadline rather than expiring stale state;
- if the reaper obtains the mutex first and the deadline is still expired, it changes the entry to `closing` and owns cleanup;
- a new `repl`/acquire cannot steal a kernel once the reaper has moved it to `closing`;
- cleanup/removal uses the same bounded shutdown + exact-entry removal path as explicit release.

The reaper task must not keep the manager alive forever; use a weak manager reference or equivalent lifecycle so it exits when the manager is dropped.

## Internal state/list contract

Expose an internal sorted snapshot sufficient for later `browser.list` adaptation. It should report at least:

- lease name;
- state: `initializing`, `ready`, or `closing`;
- configured idle timeout;
- remaining idle duration when meaningful.

Do not expose internal session/turn ids. Do not serialize this directly as the final model tool schema in this slice.

## Kernel health boundary

Add only the minimal `NodeReplKernel::is_closed()` observation needed by the manager. A repeated acquire must not claim to reuse a kernel already known closed. If a ready kernel is observed closed, take closing ownership, remove it through the same cleanup path, and retry acquire with a fresh kernel.

Do not add speculative health probes beyond rmcp `RunningService::is_closed()`.

## Factory/test seam

Keep production orchestration concrete (`NodeReplKernel`). A small internal async kernel-factory closure is acceptable so manager lifecycle can be tested with in-memory rmcp kernels without spawning the proprietary runtime. Do not introduce a public generic manager, a general browser-kernel trait hierarchy, or a new dependency-injection framework.

## Non-goals

- no `browser.reset` / `js_reset` / `turn_ended`
- no AppState field
- no tool namespace/schema/dispatch
- no config or live reload behavior
- no Browser manual/docs API
- no runtime rediscovery after manager construction
- no Desktop vs Orange Pi runtime source adapter
- no Neko/native-host compatibility
- no real installed-runtime smoke
- no stronger process/tab cleanup claims than `NodeReplKernel::shutdown`

## Tests

Cover manager-owned behavior rather than re-testing accepted lower layers:

1. concurrent same-name acquire invokes the factory once and both callers observe one ready lease;
2. different names can initialize concurrently;
3. same-lease `repl` calls serialize while different leases can overlap;
4. failed initialization leaves no stale entry and a later acquire can retry;
5. release removes the entry even when shutdown reports an error/closed service, and a subsequent acquire creates a fresh kernel;
6. idle expiry removes an inactive lease;
7. activity from a `repl` racing the reaper refreshes the deadline when the repl owns lifecycle first;
8. list snapshots are sorted and never expose session/turn ids;
9. existing `browser_kernel` and `browser_runtime` tests remain passing.

Avoid exact timing assertions where synchronization primitives/barriers can prove ordering. Keep idle timing margins generous enough for normal CI jitter.

Run:

- `cargo test -p agentic-gpt browser_manager`
- `cargo test -p agentic-gpt browser_kernel`
- `cargo test -p agentic-gpt browser_runtime`
- `cargo fmt --all -- --check`
- `git diff --check`

## Worker observation rules

- Update existing `progress.md` at start/completion.
- Append to `findings.md` only for a real contract/runtime contradiction or blocker.
- Do not modify `PLAN.md` or this slice contract during implementation.
- No commits or push by the worker.
