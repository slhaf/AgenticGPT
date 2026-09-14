# Slice 12 — Frozen Browser Tool Surface

This is an orchestrator-owned implementation contract. The accepted runtime stack already provides Desktop discovery/launch, persistent named kernels, manager lifecycle/reset/final cleanup, AppState ownership, and bounded runtime docs. This slice exposes the frozen V1 model-facing surface and nothing beyond it.

## Frozen names

Expose exactly these six Browser tools:

- `browser.manual`
- `browser.acquire`
- `browser.repl`
- `browser.reset`
- `browser.release`
- `browser.list`

Do not add semantic Browser tools (`read`, `click`, `navigate`, `tabs`, `screenshot`, etc.). `browser.repl` deliberately remains the V1 name even though its lease is a persistent arbitrary-JavaScript kernel with Browser SDK bindings.

## Tool namespace / config

- Add `ToolNamespace::Browser` with serialized name `browser`.
- Include Browser in new/default Normal and Room toolsets; Room remains the only namespace excluded from Normal.
- Update checked-in config examples/tests/expected namespace lists that encode the default surface.
- Do **not** silently rewrite/migrate an existing explicit `toolsets.enabled` list. A pre-existing config that does not contain `browser` keeps Browser disabled until the user/config flow enables it.
- Tool availability still follows the existing live toolset gate; no Browser tool may bypass it.

## Runtime absence

The Browser namespace may be enabled while `AppState.browser_runtime` is `None` (for example a host without an installed/provisioned Browser runtime). This is an expected degraded capability, not Agentic startup failure.

- `browser.list` succeeds and returns `runtimeAvailable: false` plus an empty lease list.
- the other five tools return a stable Browser structured error with code `browser_runtime_unavailable`.
- Do not rediscover/provision a runtime during a tool call in this slice.

Operational manager/manual errors keep their stable Browser error prefix/code. Convert them to Browser structured errors rather than leaking them as generic MCP invalid-params errors. Argument-shape/type validation errors remain normal MCP invalid-params errors.

## Inputs

### `browser.manual`

One tool with an `action` discriminator:

- `action: "read"`: requires `path`; optional `startLine`, `endLine`; rejects search-only fields.
- `action: "search"`: requires `query`; optional `maxResults`, `contextLines`; rejects read-only fields.

Delegate to the accepted `browser_manual` layer using the selected descriptor's `docs_root`. Preserve its bounds (read lines, maxResults 1..100, context 0..5, etc.); do not duplicate filesystem semantics in stdio dispatch.

### `browser.acquire`

- required `name`: non-empty lease name, manager remains authoritative validator.
- required `idleTimeoutSeconds`: integer 1..=86400.

On success return a compact structured object containing at least `name`, `state: "ready"`, `idleTimeoutSeconds`, and selected runtime `appVersion` / `channel` (or an equivalent compact runtime object). Do not expose official session/turn IDs.

### `browser.repl`

- required `name`.
- required `code`: non-empty JavaScript, maximum 256 KiB UTF-8 bytes.
- optional `timeoutMs`: default 20000, range 1..=120000.
- optional `title`: bounded human-readable metadata, maximum 128 Unicode scalar values; it is observability metadata only and is not injected into JavaScript or Browser SDK semantics.

Call `BrowserRuntimeManager::repl` directly. Do not route through downstream `mcp.callTool`.

### `browser.reset`

- required `name`.
- Calls manager reset and returns a compact success acknowledgement.

### `browser.release`

- required `name`.
- Calls manager release and returns `{name, released}`; absent lease remains idempotent `released: false`.

### `browser.list`

- no arguments.
- With runtime available, return selected runtime identity plus sorted manager lease snapshots.
- Lease entries expose only model-facing identity/state: `name`, lower-case state (`initializing|ready|closing`), `idleTimeoutSeconds`, optional bounded `remainingIdleSeconds`, and runtime version if useful. Never expose official session/turn IDs, kernel tokens, process IDs, or filesystem paths.

## `browser.repl` result fidelity

This is a hard contract: a successful `browser.repl` MCP call must preserve the inner official node_repl `CallToolResult` content channels as the **outer** tool result, including text/image content, `structuredContent`, `isError`, and `_meta` as faithfully as rmcp permits.

Do not wrap the node_repl result inside ordinary Agentic structured JSON, because doing so would hide `nodeRepl.emitImage()` / Browser screenshot image content from the calling model.

The smallest acceptable stdio integration is a Browser-repl-specific passthrough boundary:

- dispatch may serialize the inner `CallToolResult` to a temporary `Value` for the existing lifecycle/reporting path;
- lifecycle failure classification for this tool must honor inner `isError: true`, not only Agentic `{error: ...}` objects;
- `ServerHandler::call_tool` must reconstruct and return that `CallToolResult` for `browser.repl` instead of wrapping it with `CallToolResult::structured(...)`.

If manager/runtime fails before an official result exists, synthesize one stable `CallToolResult::structured_error` containing the Browser error object so the passthrough rule still holds.

## Structured Browser errors

Use one compact Browser error shape for non-validation failures, for example:

```json
{"error":{"code":"browser_runtime_lease_not_found","message":"browser_runtime_lease_not_found"}}
```

- `code` is the stable prefix before the first diagnostic `:` when the underlying error contains details.
- `message` may retain a bounded diagnostic string but must not dump environment values, absolute runtime paths, JavaScript source, or page contents.
- For `browser.repl`, embed the same object as the structured content of an `isError: true` `CallToolResult`.

## Annotations / policy boundary

Follow the existing Agentic toolset/policy surface; do not invent a second Browser confirmation framework in this slice.

- `browser.manual`: read-only, non-destructive, not open-world.
- `browser.list`: read-only, non-destructive, not open-world.
- `browser.acquire`: not read-only, non-destructive, not open-world.
- `browser.repl`: not read-only, mark destructive and open-world because arbitrary Browser JavaScript can mutate external web state.
- `browser.reset`: not read-only, destructive, not open-world.
- `browser.release`: not read-only, destructive, not open-world.

The official Browser runtime remains responsible for Browser-specific semantics/confirmations. Agentic owns namespace gating, annotations, lifecycle, and audit.

## Local audit

Write bounded local audit records for stateful/active Browser operations: acquire, repl, reset, release. Manual/list follow the existing convention for read-only discovery and need no local mutation audit.

Add a small Browser-specific audit record rather than logging Browser calls as fake shell/MCP jobs. At minimum include:

- time, tool, request source;
- lease name;
- selected runtime app version (if runtime exists);
- optional bounded `title` for repl;
- repl code byte count + SHA-256, **never the JavaScript source itself**;
- requested timeout/idle timeout where applicable;
- outcome, stable error code, duration.

Audit write failure should follow existing best-effort audit conventions and must not replace the Browser tool's actual result.

## Instructions / descriptions

Update the local MCP instructions and tool descriptions so callers understand:

- acquire a named persistent lease before repl;
- JS bindings survive multiple repl calls;
- reset is recovery/admin, release is final cleanup;
- Browser SDK semantics belong in JavaScript and `browser.manual` reads the selected runtime's official docs.

Keep descriptions compact enough to preserve the repository's tool-schema budget discipline.

## Tests

Cover at minimum:

1. Normal/Room default toolsets and namespace serialization include Browser; explicit filtering removes all six Browser tools.
2. Advertised Browser surface is exactly the six frozen names and no semantic Browser tools.
3. Exact Browser input schemas/required fields/conditional validation and annotations.
4. Missing runtime: list succeeds with unavailable/empty leases; the other five return stable Browser errors.
5. Manual read/search dispatch binds to descriptor docs root using temp fixtures and does not leak absolute paths.
6. List output maps manager snapshots without opaque ids/paths.
7. Release absent lease is idempotent false.
8. Browser structured error code extraction is bounded/stable.
9. `browser.repl` outer MCP result preserves a representative inner `CallToolResult` with text + image + structured content + `isError` + meta (use a controllable test seam; no live Desktop dependency).
10. Browser local audit never contains the repl JavaScript body and records its byte count/hash/request source/outcome.
11. Existing non-Browser surfaces and live toolset authorization continue to work; update schema-size budgets deliberately rather than removing the budget test.

## Verification

- focused Browser namespace/schema/dispatch/audit tests;
- `cargo test -p agentic-gpt browser_`;
- relevant config/stdio tests affected by the new namespace;
- `cargo test -p agentic-gpt --no-run`;
- `cargo fmt --all -- --check`;
- `git diff --check`.

The installed-runtime end-to-end acceptance remains the next orchestrator slice; do not auto-start Chrome or add environment-dependent CI tests here.

## Non-goals

- no semantic Browser tools beyond the six frozen names
- no rename to `kernel.*` in V1
- no runtime provisioning/download/cache implementation
- no explicit Orange Pi runtime source/config seam yet
- no Chrome auto-start or backend health polling
- no new downstream MCP Job wrapper
- no HTTP-ingress-specific Browser lifetime
- no redesign of manager/kernel lifecycle

## Worker rules

- Read `AGENTS.md`, frozen `PLAN.md`, this contract, current progress/findings, `config.rs`, `stdio_server.rs`, `audit.rs`, `state.rs`, and accepted Browser manual/manager/runtime code before editing.
- This is a nontrivial multi-file implementation: retain your session/state; do not impose a hard wall-clock implementation cutoff.
- Keep architecture/product decisions frozen; report a genuine contradiction instead of redesigning.
- Update `progress.md` at meaningful checkpoints and completion; `findings.md` only for new evidence/contract contradictions.
- Do not modify PLAN.md or this frozen contract.
- No commit/push and no destructive git cleanup. Ignore the unrelated PoC `__pycache__` artifact.

