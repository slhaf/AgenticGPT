# Operations

This page records the minimum checks for keeping Agentic GPT reproducible after deployment.

## Local verification

From the repository root:

```bash
cargo fmt
cargo check --workspace
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
3. Run one harmless short command through `/v1/exec`.
4. Start and inspect one session if session support changed.
5. List MCP servers and tools if MCP support changed.
6. Trigger one MCP tool call that requires confirmation when confirmation policy changed.

## Safety invariants

- OpenAPI must expose only GPT Actions endpoints.
- OAuth and confirmation callback routes stay outside the GPT Actions schema.
- Safe summaries may include counts and coarse modes, but not secrets or full private path lists.
- Local confirmation denial or timeout is final; it should not silently fall back to remote confirmation.
- Long-running commands should use sessions, not `/v1/exec`.
