# Migrating from v0.8 to v0.9

v0.9 replaces process-shaped managed sessions with one kind-aware Job lifecycle. The upgrade boundary depends on the runtime:

- **Standalone / Local Unix MCP:** upgrade `agentic-gpt` on each machine independently. Hub is not in the command path.
- **Hub + Local Agents:** upgrade the Hub and every command-capable Agent together. v0.9 requires `Hello.bootGeneration` and is not wire-compatible with v0.8.

## Before upgrading

For every Agent:

1. Stop that Agent runtime.
2. Back up its config, workspace audit, and locally managed state.
3. Compare the config with [`config.example.json`](../config.example.json) and [`configuration.md`](configuration.md).
4. Migrate the limits object before starting v0.9.

For Hub mode, also stop the Hub and back up its database/config before replacing either side.

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

`maxActiveJobs` also accepts `"auto"`. `sessionIdleTimeoutSecs` never controlled runtime behavior and must be removed. v0.9 rejects both removed fields instead of silently ignoring them.

Standalone users moving from an older Hub-only config must also add a valid `tunnel` block. Tunnel secrets must use `file:PATH` or `env:NAME`; plaintext values are rejected.

## Tool renames

| v0.8 | v0.9 |
| --- | --- |
| `process.batchExec` | `process.batch` |
| `process.get/list/kill` | `job.get/list/cancel` |
| `session.start` | `process.exec` or the domain creator |
| `session.list` | `job.list` |
| `session.inspect/wait` | `job.get` with optional `waitSeconds` |
| `session.kill` | `job.cancel` |
| `hub.session.list/get` | `hub.job.list/get` |

There are no compatibility aliases. tmux session tools and names are unchanged.

## Hub HTTP route migration

These changes apply only to Hub/Actions callers:

| v0.8 | v0.9 |
| --- | --- |
| `POST /v1/exec` | `POST /v1/process/exec` |
| `POST /v1/batchExec` | `POST /v1/process/batch` |
| `/v1/sessions/*` | `/v1/jobs/*` |
| raw `POST /v1/mcp/callTool` result | managed `JobResponse` |
| — | `POST /v1/mcp/batch` |

Replace the Actions schema with the v0.9 [`openapi/hub.yaml`](../openapi/hub.yaml); do not keep a cached v0.8 schema.

## Response migration

- `process.exec`, `skills.run`, and `mcp.callTool` return `JobResponse`.
- `job.get` and `job.cancel` return `JobDetail`.
- `job.list` returns lightweight `JobInfo` and never carries retained large results.
- `mcp.callTool` retains the downstream MCP result under `result`.
- `mcp.batch` returns ordered child Jobs with `batchId`, optional `batchCallId`, and `batchIndex`.
- Hub cache-only reads set `detailAvailable=false`.

## Deployment order: Standalone or Local

Each machine can be upgraded independently:

1. Stop its v0.8 runtime.
2. Migrate its local config.
3. Replace `agentic-gpt` with v0.9.
4. Set the desired `mode`/`profile` in Config v2 and start `agentic-gpt run`.
5. Verify the local socket and tool surface.
6. Reconnect or retry the corresponding ChatGPT tunnel if needed.

Other machines remain available during this rollout.

## Deployment order: Hub mode

Because the Hub protocol is wire-breaking, use a coordinated restart:

1. Stop v0.8 Local Agents.
2. Stop the v0.8 Hub.
3. Migrate every Local Agent config.
4. Replace Hub and Agent binaries with the same v0.9 build.
5. Start the Hub, then Local Agents.
6. Refresh the Actions schema and reconnect Apps MCP clients.

Do not mix a v0.8 Hub with v0.9 Agents or the reverse.

## Acceptance checks

Common Agent checks:

```bash
agentic-gpt --version
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info \
  --config ~/.agentic_gpt/config.json \
  --arguments '{}'
```

Expected:

- `agentic-gpt` reports `0.9.0`;
- Normal local/tunnel surfaces have 24 tools and Room has 36;
- `job.get`, `job.list`, `job.cancel`, and `mcp.batch` are present;
- removed managed lifecycle names are absent;
- `agent.info.execution.jobs` and `agent.info.mcp.concurrency` are present;
- the local socket is owner-only;
- a restarted Standalone worker does not exit with `expect initialized request` when a resumed call arrives before a fresh initialize.

Hub deployments additionally check:

```bash
agentic-gpt-hub --version
curl -fsS -H "Authorization: Bearer $AGENTIC_GPT_API_KEY" \
  https://<hub-domain>/v1/info
```

Confirm Hub and Agent versions match, Agents are online, and refreshed Actions/Apps MCP contracts expose the v0.9 surface.
