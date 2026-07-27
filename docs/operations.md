# Operations

This page records the minimum checks for keeping Agentic GPT reproducible after deployment.

## Local verification

From the repository root:

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

## Runtime smoke test

Use a temporary Hub database and config, start the Hub on a local port, and query the info endpoint:

```bash
tmp=$(mktemp -d)
cargo run -q -p agentic-gpt-hub -- --db "$tmp/hub.sqlite3" --config "$tmp/hub.json" init
AGENTIC_GPT_API_KEY=test-key \
  cargo run -q -p agentic-gpt-hub -- --db "$tmp/hub.sqlite3" --config "$tmp/hub.json" serve --bind 127.0.0.1:18787
curl -fsS -H 'Authorization: Bearer test-key' http://127.0.0.1:18787/v1/info
```

Expected result: JSON containing `service`, `version`, `remoteConfirmation`, `agents`, `counts`, and `generatedAt`.

## Deployment checks

After deploying a new Hub or local agent build:

1. Confirm `/v1/info` responds through the public HTTPS domain.
2. Confirm `/v1/agents` shows the expected agents online.
3. Run one harmless command through `/v1/process/exec`.
4. Start and inspect one Job through `/v1/jobs/{jobId}` if Job support changed.
5. List MCP servers and tools if MCP support changed.
6. Trigger one MCP tool call that requires confirmation when confirmation policy changed.

## v0.9 acceptance checklist

Run these checks from the same commit that will be deployed:

```bash
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

agentic-gpt --version
agentic-gpt-hub --version
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info \
  --config ~/.agentic_gpt/config.json \
  --arguments '{}'
```

Expected contract:

- both binaries report `0.9.0`;
- Normal local/tunnel surfaces expose 24 tools and Room exposes 36;
- `mcp.batch`, `mcp.callTool`, `job.get`, `job.list`, and `job.cancel` are present;
- `process.batchExec`, managed `session.*`, and `process.get/list/kill` are absent;
- `agent.info.execution.jobs` is present;
- `agent.info.mcp.concurrency` reports global limit 8, per-server limit 2, and bounded active/queued counts;
- Local Unix MCP uses an owner-only `0700` runtime directory and `0600` socket;
- Local Unix and tunnel stdio descriptor/schema revisions match for the same profile;
- `config.example.json` parses under the strict v0.9 config type and contains no usable credentials.

A no-side-effect `mcp.batch` dispatch smoke can use duplicate call ids. It must fail before confirmation or downstream connection and write one aggregate `validation_rejected` audit with no child `mcp.callTool` records:

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

The expected structured error is `mcp_batch_failed` with a message beginning `mcp_batch_call_id_duplicate`. This validates schema → serde → dispatch → aggregate audit without starting downstream work.

Before a release tag, also validate [`openapi/hub.yaml`](../openapi/hub.yaml) with the intended Actions importer and refresh the Custom GPT schema. Tagging and deployment remain separate actions.

## Safety invariants

- OpenAPI must expose only GPT Actions endpoints.
- OAuth and confirmation callback routes stay outside the GPT Actions schema.
- Safe summaries may include counts and coarse modes, but not secrets or full private path lists.
- Local confirmation denial or timeout is final; it should not silently fall back to remote confirmation.
- Long-running commands should use the Job envelope and bounded `/v1/jobs/{jobId}` inspection, not an unbounded HTTP request.
