# Slice 08 — Final Lease Cleanup

This is an orchestrator-owned implementation contract. Keep the slice limited to closing the already-frozen final-cleanup gap now that Slice 07 provides the official `turn_ended` primitive.

## Goal

Make explicit release and idle expiry perform the Browser V1 final-cleanup sequence without changing lease identity, reset behavior, AppState, config, or model-facing tools.

The frozen product contract requires final release to signal the current official Browser turn and then terminate the persistent node_repl process. Slice 06 intentionally implemented only low-level shutdown because `turn_ended` did not exist yet. Slice 07 now provides `ManagedKernel::turn_ended()` / `NodeReplKernel::turn_ended()`.

## Shared final-cleanup primitive

Add one internal manager-side helper for a kernel that has already been taken under closing ownership.

- If the kernel is not already known closed, call `turn_ended()` first.
- Regardless of `turn_ended` success/failure, always call the existing bounded `shutdown()` afterward.
- If the kernel is already known closed, skip `turn_ended` and still invoke bounded `shutdown()` as the existing process/transport cleanup attempt.
- Cleanup must never hold the manager map lock or a lease lifecycle mutex across these awaits.
- No tab-close semantics are invented; this remains official end-of-turn signaling plus rmcp/process shutdown only.

Error behavior:

- success only when both attempted stages succeed;
- if only one stage fails, preserve that stage's existing stable error;
- if both `turn_ended` and shutdown fail, return one stable `browser_runtime_final_cleanup_failed` error containing bounded diagnostic text for both failures;
- callers that intentionally ignore cleanup errors (the idle reaper) may continue to do so, but cleanup/removal ordering must still happen.

## Explicit release

Keep all accepted Slice 06 ordering semantics:

- wait for initialization;
- one owner moves ready -> closing and takes the kernel;
- cleanup occurs outside locks;
- exact entry is removed and waiters notified even when cleanup fails;
- acquire observing closing waits for removal and creates a fresh lease;
- repeated release after removal remains idempotent.

Replace the shutdown-only operation with the shared final-cleanup primitive. Return its error only after exact-entry removal.

## Idle reaper

When the reaper wins lifecycle ownership for an expired ready lease, use the same final-cleanup primitive before exact-entry removal. It continues to ignore cleanup errors because no caller exists to receive them.

The existing repl/reaper refreshed-deadline race semantics remain unchanged.

## Other cleanup paths

Do **not** blindly replace every shutdown call:

- reset recovery is not final release; its old kernel may already have received `turn_ended`, so keep reset recovery's shutdown-only behavior;
- acquire discovering an already-closed kernel cannot reliably signal `turn_ended`; keep its closed-kernel cleanup behavior unchanged;
- production factory bootstrap failure remains shutdown-only.

## Tests

Cover only the new final-cleanup semantics:

1. explicit release orders `turn_ended -> shutdown` for a healthy kernel;
2. `turn_ended` failure still performs shutdown and removes the entry;
3. shutdown failure after successful `turn_ended` still removes the entry and preserves the shutdown error;
4. dual failure returns the stable combined final-cleanup error and still removes the entry;
5. idle expiry uses `turn_ended -> shutdown` before removal;
6. reset-recovery tests continue to prove reset does not gain an extra final-cleanup signal;
7. accepted manager/kernel/runtime suites remain green.

Avoid duplicating lower-level `turn_ended` wire-shape tests already owned by `browser_kernel`.

Run:

- `cargo test -p agentic-gpt browser_manager`
- `cargo test -p agentic-gpt browser_kernel`
- `cargo test -p agentic-gpt browser_runtime`
- `cargo fmt --all -- --check`
- `git diff --check`

## Non-goals

- no AppState/config/tool-surface wiring
- no reset-ID policy changes
- no Browser manual
- no runtime discovery/environment changes
- no Neko/backend work
- no model-facing error schema
- no tab cleanup claims

## Worker rules

- Read PLAN.md, this contract, current progress/findings, and accepted manager/kernel code before editing.
- Update progress.md at completion.
- Append findings.md only for a genuine runtime/contract contradiction.
- Do not modify PLAN.md or frozen slice contracts.
- No commit/push; orchestrator reviews and accepts.

