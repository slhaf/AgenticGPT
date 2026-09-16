# Operations

This page records minimum checks for reproducible deployment. Standalone is the primary path; Hub checks are separate because Hub is optional.

## Repository verification

```bash
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 - <<'PY'
import yaml
with open('openapi/hub.yaml') as f:
    yaml.safe_load(f)
print('openapi yaml ok')
PY
```

## Local/Standalone smoke test (primary)

Set `mode=local` when tunnel credentials are unavailable; use `mode=standalone` to validate the full recommended path. Both start with `agentic-gpt run`.

```bash
agentic-gpt --version
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt run
```

From another shell:

```bash
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

Expected:

- Normal exposes 23 tools; Room exposes 34.
- `agent.info.connections.localMcp.status` is `ready`.
- Runtime directory is `0700`, socket is `0600`, and only the same UID is accepted.
- `job.*`, `process.batch`, `mcp.callTool`, and `mcp.batch` are present.
- Removed v0.8 managed lifecycle names are absent.

For Standalone, also confirm:

- tunnel-client `doctor` and loopback readiness pass;
- the ChatGPT connector can call `agent.info`;
- one disconnected machine does not affect another machine's tunnel;
- after restart, a resumed first call does not produce `expect initialized request` or restart the worker pair;
- `mcp_stdio_session_resume` / `mcp_stdio_session_resumed` appear only when recovery is needed.

## Hub smoke test (optional centralized mode)

```bash
tmp=$(mktemp -d)
cargo run -q -p agentic-gpt-hub -- --db "$tmp/hub.sqlite3" --config "$tmp/hub.json" init
AGENTIC_GPT_API_KEY=test-key \
  cargo run -q -p agentic-gpt-hub -- --db "$tmp/hub.sqlite3" --config "$tmp/hub.json" serve --bind 127.0.0.1:18787
curl -fsS -H 'Authorization: Bearer test-key' http://127.0.0.1:18787/v1/info
```

Expected JSON includes `service`, `version`, `remoteConfirmation`, `agents`, `counts`, and `generatedAt`.

## Standalone deployment checks

1. Confirm `agentic-gpt --version` is the intended release.
2. Confirm the tunnel secret is a protected `file:` or `env:` reference.
3. Confirm `agentic-gpt run` with `mode=standalone` reaches readiness and stays stable beyond the restart-budget reset interval.
4. Call `agent.info` through both ChatGPT tunnel and Local Unix MCP.
5. Run one harmless process Job and inspect it through `job.get`.
6. Restart one Agent and verify other machine connectors remain usable.
7. Confirm audit JSONL is beneath `workspaceRoot` and contains no raw tunnel/MCP secrets.

## Hub deployment checks

1. Confirm Hub and Agent binaries report the same v0.9 version.
2. Confirm `/v1/info` responds through public HTTPS.
3. Confirm `/v1/agents` shows expected command-capable Agents online.
4. Run one harmless command through `/v1/process/exec`.
5. Start and inspect one Job through `/v1/jobs/{jobId}`.
6. Validate `/mcp` and refresh Actions schema when the contract changed.
7. If Standalone reporting is enabled, confirm reporting-only connections reject Hub execution.

### Hub Response ownership cutover

Use this procedure when moving to a Hub build with the Response ownership fix:

1. Identify the exact new Hub artifact and the paired, verified Agent artifact/commit. Do not proceed unless the pair has been verified together.
2. Pause new requests and wait for in-flight requests to settle. If they cannot be drained, record their `runId` values and retain the corresponding Agent ledger entries.
3. Stop the Hub. While it is stopped, back up the actual Hub configuration and SQLite database; if present, preserve the matching `-wal` and `-shm` files with the database backup.
4. Deploy the paired artifacts, restart the Hub, and reconnect the Agent.
5. Execute one matching request, then verify `GET /v1/runs/{runId}` reports the matching `agentId`, `runId`, and `requestId`, with `status=completed` and the expected `result`.

No SQL, database/schema, configuration-format, or Room-file migration is required. A missing `runId`, previously accepted leniently, is now rejected. The rejection uses `error.code=agent_message_rejected` and one of these fixed reasons: `response_run_id_required`, `response_run_mismatch`, `response_result_conflict`, `response_result_store_failed`, or `response_waiter_owner_mismatch`.

By accepted D06 behavior, a Hub restart loses synchronous waiters, OAuth sessions, and pending sessions. Do not re-execute commands to restore HTTP waits. Durable SQLite results and the Agent ledger remain.

To roll back, pause new requests, restore only the old Hub binary, and continue using the current database and Agent ledger. Never overwrite new results with an old database backup. The old binary reopens the Response ownership defect, so rollback is not risk-free.

### Hub connection-generation cutover

Use this procedure when moving to a Hub build with generation-safe WS/SSE connection admission:

1. Pause new Hub calls and drain in-flight requests. If a request cannot be drained, record its `runId` and retain the corresponding Agent transport-ledger entry.
2. Stop the Hub. Back up the actual configuration and SQLite database, preserving matching `-wal` and `-shm` files when present.
3. Deploy the verified Hub artifact and restart it. Reconnect each Agent; every SSE reconnect must use a fresh, non-empty `connectionId`.
4. Verify `/v1/info`, `/v1/agents`, one current Heartbeat/ack path, one stale lifecycle rejection, and one matching stale reliable result. A Hub restart may lose in-memory connections, waiters, and sessions; do not re-execute commands to restore HTTP waits.

No SQL, database/schema, wire, or configuration-format migration is required. The Hub retains only the current in-memory connection per registered Agent; a replacement retires the prior Room lease and stream under the same generation boundary.

To roll back, pause new calls, stop the new Hub, replace only the Hub binary with the previous verified artifact, and continue using the latest database and Agent ledger. Never restore an older database backup over newer results. The previous binary reopens the pre-generation connection race, so rollback restores that risk.

## v0.9 acceptance checklist

```bash
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

agentic-gpt --version
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info \
  --config ~/.agentic_gpt/config.json \
  --arguments '{}'
```

Expected contract:

- `agentic-gpt` reports `0.9.0`; Hub mode also requires `agentic-gpt-hub 0.9.0`.
- Normal local/tunnel surfaces expose 23 tools and Room exposes 34.
- `mcp.batch`, `mcp.callTool`, `job.get`, `job.list`, and `job.cancel` are present.
- `process.batchExec`, managed `session.*`, and `process.get/list/kill` are absent.
- `agent.info.execution.jobs` and `agent.info.mcp.concurrency` are present.
- MCP concurrency reports global limit 8 and per-server limit 2.
- Local Unix MCP uses a `0700` runtime directory and `0600` socket.
- Local Unix and tunnel descriptor/schema revisions match.
- A fresh hidden worker recovers a resumed call before `initialize`, preserves the original id, and remains alive.
- `config.example.json` parses strictly, validates for Standalone, and contains no usable credentials.

A no-side-effect `mcp.batch` smoke can use duplicate call ids. It must fail before confirmation/downstream connection and write one aggregate `validation_rejected` audit with no child calls:

```bash
agentic-gpt local call mcp.batch \
  --config ~/.agentic_gpt/config.json \
  --arguments '{
    "calls": [
      {"id":"dup","serverId":"configured-server","toolName":"probe","arguments":{}},
      {"id":"dup","serverId":"configured-server","toolName":"probe","arguments":{}}
    ],
    "waitSeconds": 0
  }'
```

Expected error: `mcp_batch_failed` with message prefix `mcp_batch_call_id_duplicate`.

Before a Hub release tag, validate [`openapi/hub.yaml`](../openapi/hub.yaml) with the intended Actions importer. Tagging, deployment, migration, and connector restart remain separate actions.

## Safety invariants

- Tunnel, Hub, Agent, and ntfy credentials never appear in argv, safe summaries, reports, or audit payloads.
- OpenAPI exposes only GPT Actions endpoints; OAuth and confirmation callbacks stay outside it.
- Safe summaries contain counts/coarse modes, not secrets or complete private path lists.
- Local confirmation denial or timeout is final.
- Long work uses managed Jobs and bounded waits.
- Standalone reporting is optional and reporting-only, never a hidden shared command dependency.
- Invalid live config keeps the last valid subset; startup identity changes require restart.
