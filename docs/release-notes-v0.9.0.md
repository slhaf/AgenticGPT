# Agentic GPT v0.9.0 release notes

v0.9.0 unifies managed execution around kind-aware Jobs and adds a local integration ingress plus bounded downstream MCP execution. It is intentionally breaking; read [`migration-v0.9.md`](migration-v0.9.md) before deployment.

## Highlights

- Secure MCP Tunnel / Standalone is the recommended direct deployment; Hub remains an optional centralized mode.
- One `ManagedJob` lifecycle for process, skill, and MCP work.
- `job.get`, `job.list`, and `job.cancel` replace process/session lifecycle wrappers.
- Owner-only Unix MCP ingress, `run-as-local`, and real local rmcp CLI clients.
- Atomic MCP config hot reload shared by Hub, tunnel, and local ingress.
- Managed `mcp.callTool` with bounded arguments/results, exact request-id cancellation, timeout evidence, and safe audit metadata.
- `mcp.batch` with 1–16 atomic child Jobs, one aggregate confirmation, parallel/sequential modes, safe fail-fast, global 8 / per-server 2 concurrency, ordered results, and 2 MiB aggregate budgets.
- Boot-generation-aware Hub cache reconciliation; active Jobs become `unknown_after_restart` without replaying side effects.
- Normal/Room standalone surfaces are fixed at 24/36 tools.
- Tunnel stdio workers recover when a restarted child receives a resumed tool request before a fresh MCP `initialize`; the private recovery handshake is never exposed to the tunnel stream.
- Actions OpenAPI, Apps MCP, HTTP, reliable protocol, local/tunnel descriptors, documentation, and tests share the same contract.

## Breaking changes

- Crates and binaries are versioned `0.9.0`.
- `maxActiveSessions` becomes `maxActiveJobs`; remove `sessionIdleTimeoutSecs`.
- Managed `session.*`, `process.get/list/kill`, and `process.batchExec` are removed without aliases.
- HTTP execution routes move beneath `/v1/process/*` and `/v1/jobs/*`.
- `mcp.callTool` returns `JobResponse`, not a raw downstream MCP result.
- Agent `Hello` requires `bootGeneration`.

## Verification baseline

The implementation was accepted with workspace formatting/check/tests, strict OpenAPI parsing/reference validation, clippy with warnings denied, real local Unix MCP E2E, hidden standalone worker E2E including stale logical-session recovery before the first tool call, Hub Full/Coordinator MCP tests, deterministic rmcp downstream tests, and live connector availability checks. No tag, push, deployment, or GitHub Release is created by the implementation commits themselves.
