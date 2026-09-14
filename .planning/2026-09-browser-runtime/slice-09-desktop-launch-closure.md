# Slice 09 — Desktop Launch Closure

This is an orchestrator-owned implementation contract. It closes only the launch/bootstrap facts proven by the 2026-09-14 Laptop live probes before AppState/tool wiring begins.

## Goal

Make the accepted runtime descriptor + launch spec sufficient for the current Desktop extension-backed Chrome path without reading `~/.codex/config.toml`, and distinguish a healthy runtime with no currently available Chrome backend from a broken Browser bootstrap.

## Runtime descriptor / trusted code paths

The current installed runtime proved that `NODE_REPL_TRUSTED_CODE_PATHS` is mandatory for resolving the trusted Browser service. Model this as runtime data, not as a Codex-config dependency.

- Add `trusted_code_paths: Vec<PathBuf>` to `BrowserRuntimeDescriptor`.
- Desktop registry discovery derives it from the selected runtime evidence: selected `codexHome` followed by selected `nodeModuleDirs`, deduplicated while preserving order.
- Do not inspect or parse `~/.codex/config.toml`.
- Do not invent `BROWSER_USE_AVAILABLE_BACKENDS`, `BROWSER_USE_TINYSKY_ENABLED`, native-pipe timeout, Browser instruction variables, or `BROWSER_USE_SECURITY_MODE`; the live extension-backed probe succeeded without them.

`build_node_repl_launch_spec`:

- must ensure every descriptor `trusted_code_paths` entry is present in `NODE_REPL_TRUSTED_CODE_PATHS`;
- if a caller-supplied base env already has trusted code paths, preserve those additional paths and merge/deduplicate the descriptor-required paths rather than discarding caller values;
- serialize with the platform path-list convention via `split_paths` / `join_paths`;
- reject an unrepresentable merged path list with stable `browser_runtime_trusted_code_paths_invalid`;
- keep existing trusted-service, node-module-dir, security-mode, and unrelated-base-env semantics unchanged.

No upper layer may depend on the concrete `~/.codex` location; later installed/explicit/managed-cache runtime sources should only need to produce the same descriptor.

## Browser backend availability classification

The current Laptop live probe proved that node_repl + Browser SDK may be healthy while `agent.browsers.list()` has no Chrome backend because Chrome/the ChatGPT extension is not running (e.g. TTY-only host state). Starting the existing Chrome profile made the extension backend immediately appear.

Update the existing Browser bootstrap primitive without changing its role:

- after `setupBrowserRuntime()` and before `browsers.get("chrome")`, if no persistent `globalThis.browser` binding exists, query `globalThis.agent.browsers.list()`;
- if no entry with `family === "chrome"` exists, emit one Agentic-owned sentinel and do not call `get("chrome")`;
- Rust detects only that Agentic-owned sentinel and returns stable `browser_runtime_browser_unavailable`;
- do not parse or depend on the official human error string `Browser is not available: chrome`;
- if Chrome was advertised but `get("chrome")` or another bootstrap step still returns a tool error, keep the existing generic `browser_runtime_browser_bootstrap_failed`;
- successful bootstrap still establishes `globalThis.agent` + `globalThis.browser` and reports the browser id as today.

Manager initialization already removes failed entries, so unavailable Chrome must leave no stale lease and a later acquire can retry after Chrome/extension starts. No manager state-machine redesign is needed here.

## Tests

Runtime tests must cover:

1. Desktop discovery derives ordered/deduplicated trusted code paths from `codexHome + nodeModuleDirs`.
2. Launch spec creates `NODE_REPL_TRUSTED_CODE_PATHS` when base env omits it.
3. Launch spec preserves caller trusted paths while appending missing descriptor-required paths exactly once.
4. Existing runtime-coupled overwrite, unrelated-env, trusted-services, node-module, and security-mode tests remain green.

Kernel tests must cover:

5. Bootstrap code performs Browser availability discovery before `get("chrome")` and uses an Agentic-owned sentinel.
6. A successful result remains success.
7. A sentinel result maps to exactly `browser_runtime_browser_unavailable` even though transport/tool execution itself succeeded.
8. An ordinary tool-level bootstrap error still maps to `browser_runtime_browser_bootstrap_failed`.
9. rmcp transport failure behavior remains unchanged.

Do not add an always-on environment-dependent test. The orchestrator will perform the installed-runtime live smoke while the temporary Chrome backend is available.

## Verification

- `cargo test -p agentic-gpt browser_runtime`
- `cargo test -p agentic-gpt browser_kernel`
- `cargo test -p agentic-gpt browser_manager`
- `cargo fmt --all -- --check`
- `git diff --check`

Then leave the worktree for orchestrator live smoke/review.

## Non-goals

- no AppState ownership yet
- no model-facing Browser tools or schemas
- no config CLI/TUI
- no explicit non-Desktop/Orange Pi runtime source yet
- no managed runtime download/cache implementation
- no Neko/backend adapter work
- no Chrome auto-starting
- no new Browser lease states

## Worker rules

- Read `AGENTS.md`, frozen `PLAN.md`, this contract, current `progress.md`/`findings.md`, and accepted Browser runtime/kernel/manager code before editing.
- Keep the implementation self-contained to this slice; architecture and product surface are frozen.
- Update `progress.md` at completion; update `findings.md` only for a genuine new contradiction/evidence fact.
- Do not modify PLAN.md or this frozen contract.
- No commit/push. No destructive git cleanup. Ignore the unrelated untracked PoC `__pycache__` artifact.

