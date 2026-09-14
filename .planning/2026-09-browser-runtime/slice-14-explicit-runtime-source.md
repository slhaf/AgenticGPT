# Slice 14 — Explicit Runtime Source Seam

This is the last frozen V1 distribution-boundary gap from `PLAN.md`: Desktop may auto-discover the official runtime from the OpenAI registry, while non-Desktop hosts (including Orange Pi) need an explicit runtime-location/configuration seam. This slice adds that seam only. It does **not** install, download, mirror, update, or redistribute the official runtime.

## Configuration shape

Add one optional top-level `browser` configuration section. The only V1 field is an optional explicit `runtime` descriptor:

```json
{
  "browser": {
    "runtime": {
      "appVersion": "...",
      "channel": "...",
      "nodeReplPath": "/absolute/path/to/node_repl",
      "nodePath": "/absolute/path/to/node",
      "browserClientPath": "/absolute/path/to/browser-client.mjs",
      "browserServicePath": "/absolute/path/to/browser-service.mjs",
      "codexHome": "/absolute/path/to/runtime-home",
      "codexCliPath": "/absolute/path/to/codex-or-compatible-cli",
      "nodeModuleDirs": ["/absolute/path/to/node_modules"]
    }
  }
}
```

- `browser` defaults to an empty section and should be omitted by sparse serialization when unused.
- `runtime` is optional. When absent, current Desktop registry discovery remains unchanged.
- All scalar strings above are required and non-empty when `runtime` is present. `nodeModuleDirs` is optional/default empty.
- All configured paths must be absolute and UTF-8 representable. Reject relative paths with stable `browser_runtime_explicit_*` configuration errors.
- `docsRoot` and `trustedCodePaths` are **not** user fields. Derive `docs_root` from `browserClientPath` exactly as Desktop discovery does, and derive trusted-code paths from `codexHome + nodeModuleDirs` with stable ordered de-duplication.
- Do not add raw environment maps or Browser service security toggles to configuration in V1.

## Selection / startup semantics

- If `config.browser.runtime` is present, it is authoritative and selected before Desktop registry discovery.
- If the explicit descriptor is malformed/unusable, Browser capability is unavailable for that process and startup logs a bounded INFO diagnostic with `source=explicit-config`; do **not** silently fall back to a Desktop registry because that would hide a broken explicit deployment.
- If explicit runtime is absent, retain the existing Desktop registry path/discovery behavior and `source=desktop-registry` diagnostics.
- Runtime source selection happens at AppState construction. Changing Browser runtime configuration requires process restart in V1; do not add Browser manager hot-swap/reload.
- Agentic startup itself remains fail-open: unusable Browser runtime means `AppState.browser_runtime=None`, not process failure.

## Runtime descriptor boundary

- Add one source-agnostic constructor/conversion in `browser_runtime.rs` that converts the explicit config into the existing `BrowserRuntimeDescriptor`.
- Both explicit config and Desktop discovery must ultimately produce the same descriptor type and use the existing `build_node_repl_launch_spec`, `BrowserRuntimeContext`, manager, six tools, audit, and manual layer unchanged.
- Avoid duplicating docs-root/trusted-code derivation logic; share helpers where practical without redesigning the module.

## Configuration persistence

- Typed config load/save/import/sparse projection must preserve an explicit Browser runtime descriptor exactly (subject to normal canonical JSON formatting).
- Existing configs with no `browser` field remain valid and behave exactly as before.
- Existing explicit `toolsets.enabled` behavior is unchanged; configuring a runtime does not auto-enable the Browser namespace and enabling Browser does not require an explicit runtime.
- Update the checked-in config documentation/examples with a clearly optional commented/documented JSON example, but do not make the default generated config depend on local proprietary runtime paths.

## Tests

Cover at minimum:

1. Config with no `browser` section defaults empty and Desktop discovery path remains selected.
2. Explicit runtime JSON round-trips through config load/sparse write/import without loss.
3. Explicit descriptor conversion derives docs root and ordered/deduplicated trusted code paths exactly like Desktop discovery.
4. Relative/empty/malformed explicit paths/fields are rejected with stable explicit-runtime error codes.
5. Explicit source has precedence over Desktop registry and an invalid explicit source does not fall back.
6. `build_app_state` remains fail-open when explicit Browser runtime is invalid/unavailable.
7. A source-agnostic context built from a valid explicit descriptor drives the same manager/manual seams; no tool surface change.
8. Existing Browser-focused tests and full config tests stay green.

## Verification

- focused explicit-runtime/config/startup-source tests;
- `cargo test -p agentic-gpt browser_`;
- relevant config/import/sparse-projection tests;
- `cargo test -p agentic-gpt --no-run` and, after orchestrator review, full package tests;
- `cargo fmt --all -- --check`;
- `git diff --check`.

## Non-goals

- no downloader/installer/updater or managed runtime cache implementation
- no committed/vendored OpenAI runtime files
- no runtime artifact mirror or redistribution
- no Neko-specific fields/logic above the official Browser backend boundary
- no Browser manager or six-tool semantic changes
- no runtime hot reload/swap
- no new semantic Browser tools or `kernel.*` rename

## Worker rules

- Read `AGENTS.md`, frozen `PLAN.md`, this slice, current `progress.md`/`findings.md`, and the current config/runtime/AppState source before editing.
- The architecture and product decisions above are frozen. Report a real contradiction instead of widening the surface.
- This is a nontrivial multi-file implementation: retain worker state/session and do not impose a hard wall-clock cutoff.
- Update `progress.md` at meaningful checkpoints/completion; update `findings.md` only for new runtime evidence or an actual contract gap.
- Do not modify `PLAN.md` or this contract.
- Do not commit/push or destructively clean. Leave the unrelated PoC `__pycache__` untouched.

