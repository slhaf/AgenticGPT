# Slice 10 — AppState Runtime Ownership

This is an orchestrator-owned implementation contract. Browser runtime discovery/launch, persistent kernel lifecycle, reset/final cleanup, and Desktop extension availability are already accepted. This slice only gives the running Agentic process one shared Browser runtime owner; it does not expose Browser tools yet.

## Goal

Give `AppState` an optional long-lived Browser runtime context containing the selected runtime descriptor and one shared `BrowserRuntimeManager`.

The runtime source is deliberately below this context boundary. Today production may auto-discover the installed Desktop registry. A future explicit-path or Agentic-managed-cache source must be able to produce the same descriptor/context without changing AppState consumers, manager semantics, or model-facing tools.

## State shape

Add a cloneable repository-internal context type adjacent to `AppState` (or an equivalently conventional location) containing exactly the runtime-level data later Browser tools need:

- selected `BrowserRuntimeDescriptor` (for runtime version/docs root and future diagnostics);
- shared `Arc<BrowserRuntimeManager>`.

`AppState` owns `Option<Arc<...context...>>` (or an equivalent clone-cheap optional owner). Do not add separate Codex-path fields to `AppState`.

Test state constructors that do not exercise Browser behavior should use `None`; do not make their unit tests discover the host Desktop runtime.

## Production construction

In the existing `build_app_state` path:

1. attempt the current Desktop source only through `browser_runtime::default_desktop_registry_path()` + `discover_desktop_runtime()`;
2. build the accepted launch spec from that descriptor with an empty Browser-specific base env (do not read Codex TOML/process Browser env as a hidden dependency);
3. construct one `BrowserRuntimeManager::new(spec, descriptor.browser_client_path.clone())`;
4. store descriptor + manager in the shared runtime context.

Desktop runtime discovery/construction failure is **not Agentic startup failure**. Store no Browser context and continue startup. Log one bounded non-secret informational/warning diagnostic using the repository's existing logging convention; do not expose filesystem contents or environment values.

This distinction is important:

- no installed/usable runtime source at process startup -> Browser context absent;
- runtime context present but Chrome/extension later absent -> manager acquire returns the accepted `browser_runtime_browser_unavailable` and can be retried after Chrome starts.

Do not auto-start Chrome and do not probe a Browser backend during AppState construction. Manager construction itself must remain lazy with respect to lease/kernel creation.

## Lifetime / reload

The Browser runtime context is process-lifetime V1 state, like other long-lived managers. Existing standalone live config reload must not recreate/drop it in this slice. Runtime-source hot reload and installing a managed runtime while the process is already running are later concerns; restart is acceptable for V1 source changes.

## Tests / verification

- Update every `AppState` literal/test helper required by the new field, using `None` unless the test explicitly targets Browser state.
- Add a small focused test seam around Browser-context construction if needed to prove that a valid descriptor produces one shared manager without spawning a lease; do not make tests depend on the developer's real Desktop registry.
- Ensure ordinary test AppState creation has no host Browser/Codex dependency.
- Existing Browser runtime/kernel/manager suites remain green.
- Run repository tests naturally affected by AppState shape (at minimum the main/state-related unit compile set; if the full package suite hits the already-recorded unrelated stale-stdio timeout, record rather than changing it here).
- `cargo fmt --all -- --check`
- `git diff --check`

## Non-goals

- no Browser tool namespace/descriptors/dispatch
- no `browser.manual` implementation yet
- no config schema/CLI/TUI Browser runtime source
- no explicit Orange Pi/non-Desktop source yet
- no managed runtime cache/downloader
- no Chrome auto-start
- no backend health polling
- no runtime-source live reload
- no manager lifecycle changes

## Worker rules

- Read `AGENTS.md`, frozen `PLAN.md`, this contract, current progress/findings, `state.rs`, `main.rs`, and accepted Browser runtime/manager code first.
- Keep the product surface/architecture frozen and scope this to ownership/construction only.
- Update `progress.md` at completion; `findings.md` only for genuine evidence/contract contradictions.
- Do not modify PLAN.md or this frozen contract.
- No commit/push and no destructive git cleanup. Ignore the unrelated PoC `__pycache__` artifact.

