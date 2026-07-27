# Migrating from v0.8 to v0.9

v0.9 intentionally removes the process-shaped managed session surface and replaces it with one kind-aware Job lifecycle. Upgrade the Hub and every Local Agent together; the v0.9 protocol requires `Hello.bootGeneration` and is not wire-compatible with v0.8.

## Before upgrading

1. Stop the Hub and Local Agents.
2. Back up the Hub database/config and each Local Agent config/audit file.
3. Compare the Local Agent config with [`config.example.json`](../config.example.json).
4. Do not start a v0.9 binary with the old limits object.

## Required configuration migration

Change:

```json
{
  "limits": {
    "maxConcurrentTasks": 2,
    "maxActiveSessions": 6,
    "sessionIdleTimeoutSecs": 3600
  }
}
```

To:

```json
{
  "limits": {
    "maxConcurrentTasks": 2,
    "maxActiveJobs": 6
  }
}
```

`maxActiveJobs` also accepts `"auto"`, which resolves at startup/reload using the documented bounded CPU formula. `sessionIdleTimeoutSecs` never controlled runtime behavior and must be removed. The v0.9 limits object rejects both removed fields instead of silently ignoring them.

## MCP and tool renames

| v0.8 | v0.9 |
| --- | --- |
| `process.batchExec` | `process.batch` |
| `process.get` | `job.get` |
| `process.list` | `job.list` |
| `process.kill` | `job.cancel` |
| `session.start` | `process.exec` or the domain creator |
| `session.list` | `job.list` |
| `session.inspect` / `session.wait` | `job.get` with optional `waitSeconds` |
| `session.kill` | `job.cancel` |
| `hub.session.list/get` | `hub.job.list/get` |

There are no compatibility aliases. tmux session tools are unrelated and unchanged.

## HTTP route migration

| v0.8 | v0.9 |
| --- | --- |
| `POST /v1/exec` | `POST /v1/process/exec` |
| `POST /v1/batchExec` | `POST /v1/process/batch` |
| `/v1/sessions/*` | `/v1/jobs/*` |
| raw `POST /v1/mcp/callTool` result | managed `JobResponse` |
| — | `POST /v1/mcp/batch` |

Replace the Actions schema with the v0.9 [`openapi/hub.yaml`](../openapi/hub.yaml); do not keep a cached v0.8 schema in the Custom GPT.

## Response migration

- `process.exec`, `skills.run`, and `mcp.callTool` return `JobResponse`.
- `job.get` and `job.cancel` return `JobDetail`.
- `job.list` returns lightweight `JobInfo` values and never carries retained large results.
- `mcp.callTool` retains the downstream MCP result under `result`; it no longer passes the downstream result envelope through at the Hub top level.
- `mcp.batch` returns ordered child Job details. Each child has `batchId`, optional `batchCallId`, and `batchIndex`.
- Hub cache-only reads set `detailAvailable=false`.

## Deployment order

Because v0.9 is wire-breaking, use a coordinated restart:

1. Stop v0.8 Local Agents.
2. Stop the v0.8 Hub.
3. Migrate every Local Agent config.
4. Replace Hub and Local Agent binaries with the same v0.9 build.
5. Start the Hub, then Local Agents.
6. Reinstall/refresh the Actions schema and reconnect Apps MCP clients.

## Acceptance checks

```bash
agentic-gpt --version
agentic-gpt-hub --version
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info \
  --config ~/.agentic_gpt/config.json \
  --arguments '{}'
```

Expected:

- both binaries report `0.9.0`;
- Normal local/tunnel surfaces have 24 tools and Room has 36;
- `job.get`, `job.list`, `job.cancel`, and `mcp.batch` are present;
- removed managed session/process lifecycle names are absent;
- `agent.info.execution.jobs` and `agent.info.mcp.concurrency` are present;
- the local socket is owner-only and Hub/local/tunnel descriptor revisions match.
