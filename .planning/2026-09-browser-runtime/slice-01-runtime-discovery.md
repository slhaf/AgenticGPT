# Slice 01 — Browser Runtime Descriptor & Desktop Discovery

This is an orchestrator-owned implementation contract. Worker may implement it but must not change its architecture or scope.

## Goal

Add a small internal Browser runtime discovery module that turns the installed OpenAI Desktop runtime registry into one validated, self-consistent runtime descriptor. This slice does **not** launch `node_repl`, construct its environment, add Browser tools, or add any AppState-owned manager.

## Exact module/API

Add `crates/agentic-gpt/src/browser_runtime.rs` and register it from `main.rs` with `mod browser_runtime;`.

The module owns:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BrowserRuntimeDescriptor {
    pub(crate) app_version: String,
    pub(crate) channel: String,
    pub(crate) node_repl_path: PathBuf,
    pub(crate) node_path: PathBuf,
    pub(crate) browser_client_path: PathBuf,
    pub(crate) browser_service_path: PathBuf,
    pub(crate) codex_home: PathBuf,
    pub(crate) codex_cli_path: PathBuf,
    pub(crate) node_module_dirs: Vec<PathBuf>,
    pub(crate) docs_root: PathBuf,
}
```

and two discovery entry points:

```rust
pub(crate) fn default_desktop_registry_path() -> Result<PathBuf>;
pub(crate) fn discover_desktop_runtime(registry_path: &Path) -> Result<BrowserRuntimeDescriptor>;
```

`default_desktop_registry_path()` resolves the current user's home directory using the repository's existing Rust dependencies/conventions and returns:

`~/.local/state/openai-codex/chrome-native-hosts-v2.json`

It must not read or mutate the registry.

`discover_desktop_runtime()` reads only the supplied registry path, parses schema v2 data needed by this slice, chooses the entry with the lexicographically greatest non-empty `updatedAt` (matching the proven PoC), and builds the descriptor from that single selected entry.

## Required registry fields

For the selected entry require non-empty:

- `appVersion`
- `channel`
- `updatedAt`
- `paths.nodeReplPath`
- `paths.nodePath`
- `paths.browserClientPath`
- `paths.browserServicePath`
- `paths.codexHome`
- `paths.codexCliPath`

`paths.nodeModuleDirs` is optional and defaults to an empty vector.

Do not add `resourcesPath`, extension/native-host metadata, auth fields, backend fields, or Neko-specific fields to the descriptor in this slice.

## Derived field

`docs_root` is derived from the selected `browser_client_path` exactly as the verified current bundle layout supports:

`browser_client_path.parent().and_then(parent).join("docs")`

For `.../<bundle>/scripts/browser-client.mjs`, this yields `.../<bundle>/docs`.

If the client path has fewer than two parent components, discovery returns a stable error rather than inventing a path.

## Validation boundary

This slice validates **registry structure and selected-entry field presence only**. It does not require every referenced file to exist on disk. Runtime filesystem readiness belongs to the later launch/runtime layer, because registry fixtures and explicit future descriptors must remain testable without installing proprietary runtime files.

Malformed JSON, missing/empty `entries`, no entry with usable non-empty `updatedAt`, missing/empty required selected-entry fields, malformed `paths`, or an under-parented browser client path must return stable, specific `anyhow` error strings prefixed with `browser_runtime_...`.

Do not silently fall back to an older entry if the selected latest entry is structurally incomplete; selecting current and validating current are separate steps. A broken current registry entry should be visible as an error.

## Explicit non-goals

- no `BrowserConfig` yet
- no launch env / `NODE_REPL_TRUSTED_SERVICES`
- no `node_repl` process/client
- no AppState changes
- no tool namespace / schemas / dispatch
- no policy/audit integration
- no reset/release/session/turn logic
- no runtime installer/bootstrap/distribution
- no Orange Pi/Neko compatibility
- no config CLI/TUI or live reload changes

## Tests

Unit tests stay next to `browser_runtime.rs` and must use temporary registry JSON fixtures without depending on the user's real registry.

Cover at least:

1. chooses greatest `updatedAt` independent of input order;
2. maps all descriptor fields from the selected entry;
3. derives `docs_root` from `browserClientPath`;
4. absent `nodeModuleDirs` becomes empty;
5. empty entries rejected;
6. missing/empty required selected-entry field rejected;
7. latest entry malformed does not fall back to an older valid entry;
8. malformed JSON rejected with browser-runtime error context;
9. client path unable to produce bundle/docs root rejected.

Run only the focused test(s) for this module plus `cargo fmt --all -- --check` for this slice. Do not broaden into workspace-wide fixes.

## Worker observation rules

- Update existing `.planning/2026-09-browser-runtime/progress.md` at start and completion.
- Append implementation-specific evidence/problems to existing `findings.md` only if something in the repository contradicts this contract or blocks it.
- Do not modify `PLAN.md` or this slice contract.
- No commits or push.
