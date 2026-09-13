# Browser Runtime V1 — Frozen Contract

## Goal

Add a first-class `browser` toolset to AgenticGPT that exposes the official OpenAI Browser runtime through persistent named JavaScript kernels. Agentic owns orchestration/lifecycle/policy; it does not reimplement browser semantics.

The implementation target is the existing AgenticGPT standalone/local worker architecture at `main@40de102`. This work is independent from the separate standalone HTTP endpoint branch.

## Product surface — frozen

The model-facing V1 tools are exactly:

- `browser.manual`
- `browser.acquire`
- `browser.repl`
- `browser.reset`
- `browser.release`
- `browser.list`

Do not add semantic browser tools such as `browser.read`, `browser.click`, `browser.navigate`, `browser.tabs`, or `browser.screenshot`. Browser semantics stay in the official Browser SDK and are driven through JavaScript passed to `browser.repl`.

### `browser.acquire`

- Input includes caller-chosen `name` and `idleTimeoutSeconds`.
- Idempotent by `name`: reuse an existing healthy lease with the same name.
- A lease owns one persistent official `node_repl` MCP process/kernel.
- Bootstrap the official Browser SDK with `setupBrowserRuntime()` and expose convenient persistent bindings (`agent`, `browser` or the equivalent verified current-runtime objects) in that JS kernel.
- Different names are isolated. Calls within one lease are serialized.

### `browser.repl`

- Input identifies the lease and carries arbitrary JavaScript plus optional execution timeout/title metadata.
- Forward the JavaScript to the official node_repl `js` tool; do not parse or translate Browser SDK semantics in Rust.
- Return the official result/content/image metadata as faithfully as practical through Agentic's tool result conventions.
- Activity refreshes the lease idle deadline.

### `browser.reset`

- Recovery/admin operation, not normal per-call cleanup.
- Keep the lease name.
- End the current official browser turn (`turn_ended`), reset JS state (`js_reset`) or respawn if the kernel is unhealthy, then bootstrap a fresh usable turn/kernel state.
- Do not delete the lease.

### `browser.release`

- Final cleanup: `turn_ended`, close/kill the node_repl process, remove the lease.
- Cleanup must be bounded; failure to get a graceful response must not leave an unbounded wait.

### `browser.list`

- Report lease names and bounded operational state useful for recovery/debugging (e.g. state, idle timeout/deadline/activity, runtime version). Do not expose official `session_id` / `turn_id` as model-facing identity.

### `browser.manual`

- Read/search the documentation bundled with the *currently selected official runtime version*.
- Do not hardcode copied docs into AgenticGPT and do not use web documentation as the runtime authority.

## Runtime facts already verified — do not re-derive product design

On the Laptop, the official runtime registry is:

`~/.local/state/openai-codex/chrome-native-hosts-v2.json`

The latest registered runtime currently exposes explicit paths for `nodeReplPath`, `nodePath`, `browserClientPath`, `browserServicePath`, `resourcesPath`, etc. The registry entry/version is the preferred evidence for installed Desktop runtime discovery; do not hardcode today's version/path.

The existing proof-of-concept is:

`/home/slhaf/Projects/AgenticGPT/experimental/chrome-control-poc/browser_bridge.py`

Read it before implementing the node_repl client. It already proves the current initialize/initialized/tools-call shape, per-call `_meta` turn metadata, and persistent JS-kernel behavior. Treat it as evidence, not production architecture.

The current official Browser bundle is reachable through the registry's `browserClientPath`; adjacent runtime docs include `docs/api.json`, `docs/accessibility.md`, tab lifecycle docs, browser safety/troubleshooting docs, etc. Read only the pieces needed for the implementation. Inspect current `browser-client.mjs` / `browser-service.mjs` when a runtime contract detail is required.

Verified model-facing node_repl MCP tools are `js`, `js_add_node_module_dir`, `js_reset`, and `turn_ended`. Browser-specific operations are Browser SDK/internal service operations, not separate MCP tools.

The Browser SDK is the abstraction boundary. The official client currently uses trusted `globalThis.nodeRepl.rpc("browser", ...)` internally; Agentic should not reproduce that service protocol.

## Runtime/distribution boundary

- Official/proprietary runtime files remain external resources and must never be committed into AgenticGPT.
- Laptop may auto-discover an installed Desktop runtime through the current registry.
- Non-Desktop hosts (including Orange Pi) need an explicit runtime-location/configuration seam; do not invent a package installer/distributor in this V1.
- If a portable runtime descriptor cannot be derived confidently from the current code/runtime evidence, record the exact gap in `findings.md` rather than guessing.
- Neko/native-host compatibility is a lower backend boundary. Do not spread Neko-specific compatibility logic through `BrowserRuntimeManager`.
- The previously observed existing-user-tab claim anomaly is out of scope. V1 automation may prefer agent-owned `browser.tabs.new()` paths.

## Agentic architecture boundary

- `AppState` should own a shared long-lived `BrowserRuntimeManager` (or repository-conventional equivalent).
- The manager owns multiple named leases/kernels.
- A Browser kernel is a long-lived runtime resource, not itself a Job.
- Individual tool calls still use Agentic's normal tool dispatch, policy, audit, structured error, timeout/cancellation conventions where applicable.
- Do **not** route `browser.repl` through the existing downstream `mcp.callTool` implementation if that lifecycle reconnects and closes the downstream client per invocation; persistence is required.
- Reuse existing rmcp/process/runtime patterns where they actually fit, but keep one persistent stdio MCP client/process per lease.
- Do not couple Browser runtime lifetime to HTTP ingress or any one transport.

## Security boundary

The official Browser runtime may require its local-testing security mode in this non-Codex host. If current runtime evidence proves this is required, confine that launch configuration to the Browser runtime process and rely on Agentic policy/audit as the outer control boundary. Do not silently generalize it to unrelated processes. Record any unresolved production-security question in `findings.md`.

## Implementation rules

1. Read this contract first; it is authoritative for product scope.
2. Inspect repository anchors only as needed to place the implementation correctly: `AppState` construction, tool namespace registration/dispatch, config, policy/audit, rmcp stdio client usage, structured results/errors, and tests.
3. Do not redesign the six-tool surface or rewrite this contract to match implementation convenience.
4. Maintain `progress.md` as a short current execution ledger and `findings.md` for evidence, blockers, and contract-vs-reality gaps. Update them at meaningful checkpoints, replacing stale statements rather than dumping logs.
5. If required information is unavailable from the repository, official runtime, docs, registry, or PoC, do not infer a fake contract. Record the gap and stop that dependent part.
6. No commits and no push. Leave the worktree for orchestrator review.
7. Do not use `git restore`, `git reset`, `git clean`, bulk deletion, or other destructive cleanup.
8. Run focused tests while implementing, then repository-appropriate formatting/check/tests for the touched scope. Record exact results in `progress.md`.

## Acceptance

At minimum, orchestrator review must be able to verify:

- Browser namespace/tool descriptors expose only the six frozen tools.
- Two named leases can exist independently; same-lease REPL calls are serialized.
- JS bindings survive multiple `browser.repl` calls in one lease.
- `reset` preserves lease identity while clearing/rebootstrapping kernel state; `release` removes it.
- Idle expiry is bounded and cleans up the lease/process.
- Runtime discovery does not pin today's version/path and external runtime files are not copied into the repo.
- `browser.manual` reads/searches docs from the selected runtime.
- Tool policy/audit/error handling follows existing Agentic conventions.
- Focused tests cover manager lifecycle plus tool dispatch/schema behavior; integration coverage uses a controllable fake node_repl/runtime fixture where necessary and may include an opt-in live smoke against the installed runtime if repository conventions allow it.

