# Slice 07 — Browser Reset Lifecycle

This is an orchestrator-owned implementation contract. Worker may implement it but must not redesign the Browser V1 surface or broaden this slice into AppState/tool/config wiring.

## Goal

Add the internal reset lifecycle needed by the already-frozen `browser.reset` product contract. Reset preserves the caller-visible lease name and serializes against `repl`, release, and the idle reaper. A healthy kernel is reset in place through the official node_repl control tools; an unhealthy or failed in-place reset is recovered by replacing the kernel inside the same manager entry and re-bootstraping it.

This slice consumes the accepted `BrowserRuntimeManager` from Slice 06. It does not add model-facing `browser.*` tools yet.

## Verified node_repl evidence

Current installed node_repl `tools/list` reports:

- `js_reset`: empty object input schema; resets the JavaScript kernel and clears bindings.
- `turn_ended`: required `{hook_event_name, session_id, turn_id}`; repeated notifications for the same session/turn are ignored.
- the official Chrome plugin invokes `turn_ended` from the `Stop` hook and passes the current session/turn ids.

Current official `js-reset.md` says reset discards JavaScript bindings and the next JS call initializes a fresh runtime; it explicitly does not close browser tabs/native apps or erase their state.

No inspected official source defines mandatory session/turn-id rotation after `turn_ended` or `js_reset`. Do not invent one.

## Kernel primitives

Add only the low-level node_repl operations required by reset:

- `NodeReplKernel::turn_ended()` calls MCP tool `turn_ended` with:
  - `hook_event_name = "Stop"`;
  - the kernel's stored opaque `session_id`;
  - the kernel's stored opaque `turn_id`.
- `NodeReplKernel::reset_js()` calls MCP tool `js_reset` with an empty object.
- tool transport failures retain stable browser-runtime error context; a tool result with `isError: true` is treated as reset failure rather than a successful lifecycle step.
- a healthy in-place reset sequence is exactly:
  1. `turn_ended()`;
  2. `reset_js()`;
  3. `bootstrap_browser(browser_client_path)`.

Do not claim that `turn_ended` or `js_reset` closes tabs, shuts down the browser service, clears native/browser state, or rotates ids.

## ID policy

- Lease identity remains the caller-supplied lease name.
- For a healthy in-place reset, retain the existing opaque session/turn ids. The evidence does not justify rotating either merely because reset occurred.
- When reset recovery respawns a kernel through the manager factory, generate fresh opaque session/turn ids exactly as normal kernel creation already does. This is an Agentic process/kernel-lifecycle choice, not a claim about official reset semantics.
- Session/turn ids remain internal and are never exposed in list snapshots or later tool schemas.

## Manager reset ordering

Add `BrowserRuntimeManager::reset(name)` with the existing per-lease lifecycle mutex as the sole operation-ownership boundary.

- Invalid/blank names retain the existing validation error.
- Missing lease returns `browser_runtime_lease_not_found`.
- If the entry is still initializing, reset waits for that initialization attempt to resolve and then acts on the ready lease if the exact entry still exists.
- If the entry is closing or is removed while reset waits, reset returns `browser_runtime_lease_not_found`; it must not revive a closing lease.
- Reset serializes across the complete operation with same-lease `repl`, explicit release, and idle expiry. Other leases remain concurrent.
- Refresh activity when reset begins and after it successfully establishes a usable kernel.

### Healthy in-place path

While owning the lease lifecycle mutex, if the kernel is not known closed:

1. run the exact in-place sequence `turn_ended -> js_reset -> bootstrap`;
2. if all three succeed, keep the same kernel object, lease entry, idle timeout, and opaque ids;
3. return success.

The lifecycle mutex may be held across these awaited kernel calls because it is intentionally the per-lease serialization boundary; the manager map lock must not be held.

### Recovery / respawn path

If the kernel is already known closed, or any in-place reset step fails:

1. transition the exact lease entry to `initializing` while holding its lifecycle mutex;
2. take the old kernel out of the lifecycle state;
3. release the lifecycle mutex;
4. best-effort bounded shutdown of the old kernel if one exists; shutdown failure does not prevent recovery;
5. create one fresh fully bootstrapped kernel through the existing manager factory, outside the manager map lock and lifecycle mutex;
6. on success, install that fresh kernel back into the same exact lease entry, mark it ready, refresh activity, and notify waiters;
7. on factory/bootstrap failure, remove the exact broken entry and notify waiters, then return the recovery error.

Successful reset therefore never deletes/recreates the lease entry. Unrecoverable reset failure is allowed to remove the unusable exact entry rather than pin a zombie lease forever; a later `acquire(name, ...)` can recreate it.

Only one reset/recovery owner may operate on a lease at a time because all reset/repl/release/reaper ownership goes through the same lifecycle mutex/state machine.

## Idle reaper interaction

- A same-lease reaper waiting behind an active reset must re-check the refreshed deadline after reset completes.
- During respawn recovery the entry is `initializing`, so the reaper must not expire or steal it.
- A reset that starts after the reaper has already moved the lease to `closing` must not revive it; it returns not-found once exact-entry ownership is gone.

## Internal list semantics

Do not add a new public/model-facing state solely for reset.

- healthy in-place reset is serialized by the lifecycle mutex and need not expose a transient snapshot state;
- respawn recovery uses the existing internal `initializing` state;
- ready/closing semantics from Slice 06 remain unchanged.

## Non-goals

- no AppState field or construction wiring
- no model-facing `browser.reset` descriptor/dispatch yet
- no `browser.manual`, `browser.acquire`, `browser.repl`, `browser.release`, or `browser.list` tool wiring
- no config/live reload
- no Desktop/Orange Pi runtime source adapter
- no Neko/native-host compatibility changes
- no installed-runtime end-to-end smoke
- no tab cleanup claims
- no mandatory session/turn-id rotation rule
- no broader kernel trait/DI framework

## Tests

Cover only reset-owned behavior plus accepted regressions:

1. kernel `turn_ended` sends exact tool name and `{hook_event_name:"Stop", session_id, turn_id}` arguments;
2. kernel `reset_js` sends `js_reset` with empty arguments;
3. tool-level `isError:true` and rmcp failures become reset failures with stable context;
4. healthy reset performs `turn_ended -> js_reset -> bootstrap` in order and preserves the same managed kernel/lease identity;
5. same-lease `repl` and reset serialize; different leases can still operate independently;
6. known-closed kernel skips in-place reset and respawns once;
7. failure during any in-place reset step falls back to one fresh factory/bootstrap attempt;
8. successful respawn recovery keeps the same lease entry/name and idle timeout while replacing the kernel;
9. failed respawn removes the exact unusable entry and allows later acquire retry;
10. reset racing the idle reaper refreshes activity when reset owns lifecycle first;
11. reset observing a closing/removed lease does not revive it;
12. existing manager/kernel/runtime focused suites remain passing.

Avoid duplicating accepted registry/env/metadata/result-preservation tests unless reset specifically changes that contract.

Run:

- `cargo test -p agentic-gpt browser_manager`
- `cargo test -p agentic-gpt browser_kernel`
- `cargo test -p agentic-gpt browser_runtime`
- `cargo fmt --all -- --check`
- `git diff --check`

## Worker observation rules

- Update existing `progress.md` at meaningful start/completion checkpoints.
- Append `findings.md` only for a real contract/runtime contradiction or newly verified runtime fact that materially changes this slice.
- Do not modify `PLAN.md`, Slice 06 contract, or this Slice 07 contract.
- No commit or push by the worker.

