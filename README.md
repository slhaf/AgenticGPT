# Agentic GPT

Agentic GPT is a Linux local execution agent plus a Rust VPS Hub for controlled ChatGPT Actions and Apps MCP access.

The current default architecture is:

```text
ChatGPT Actions / ChatGPT Apps MCP
  -> HTTPS API on Rust VPS Hub
  -> WebSocket connection to Local Agent
  -> local process / session / confirmation / MCP bridge / sandbox
```

The old Cloudflare Worker implementation was moved out of `main`; see branch `legacy/cf-worker-before-removal` if the historical Cloudflare-only Hub is needed. New work should target the Rust Hub.

## Layout

- `crates/agentic-gpt`: Rust CLI local agent.
- `crates/agentic-gpt-hub`: Rust VPS Hub HTTP/WebSocket service.
- `crates/agentic-gpt-protocol`: Shared JSON protocol types.
- `openapi/hub.yaml`: Custom GPT Actions schema for the Rust Hub.
- `docs/interfaces.md`: Public interface map for Actions, Apps MCP, and Local Agent WebSocket.
- `docs/operations.md`: Local verification, smoke tests, deployment checks, and safety invariants.
- `dist`: release artifact output.

## Hub

```bash
cargo run -p agentic-gpt-hub -- init
cargo run -p agentic-gpt-hub -- agent add \
  --agent-id laptop \
  --display-name my-laptop \
  --secret '<agent-secret>'
AGENTIC_GPT_API_KEY='<high-entropy-api-key>' \
  cargo run -p agentic-gpt-hub -- serve --bind 127.0.0.1:8787
```

Hub state defaults to `~/.agentic_gpt/hub.sqlite3`. Put Caddy or Nginx in front of the Hub for HTTPS and WebSocket reverse proxying.

Hub config defaults to `~/.agentic_gpt/hub.json`. Remote confirmation is disabled by default; enable ntfy on the Hub, not on each Local Agent:

```json
{
  "remoteConfirmation": {
    "enabled": true,
    "provider": "ntfy",
    "timeoutSeconds": 45,
    "ntfy": {
      "serverUrl": "https://ntfy.example.com",
      "topic": "<high-entropy-topic>",
      "callbackBaseUrl": "https://agentic-gpt.example.com"
    }
  }
}
```

The ntfy callback routes are intentionally not part of the GPT Actions OpenAPI. They are called only from ntfy action buttons and require the one-time confirmation token in the callback URL.

## Local Agent

```bash
cargo run -p agentic-gpt -- config init
cargo run -p agentic-gpt -- config set hubUrl http://127.0.0.1:8787
cargo run -p agentic-gpt -- config set agentId laptop
cargo run -p agentic-gpt -- config set agentSecret '<agent-secret>'
cargo run -p agentic-gpt -- config set confirmationProvider freedesktop-then-hub
cargo run -p agentic-gpt -- config set confirmationLanguage zh-CN
cargo run -p agentic-gpt -- run
```

Config lives at `~/.agentic_gpt/config.json`; audit logs are JSONL at `~/.agentic_gpt/audit.log`.

`workerUrl` is accepted as a legacy alias when reading or setting config, but `hubUrl` is the canonical field.

`freedesktop-then-hub` first tries local desktop notification actions. It falls back to Hub-backed ntfy only when the local provider is unavailable or cannot show the notification. User denial or timeout from the local provider is final and does not fall back.

Command policy rules can be added or removed by command. `remove` matches `program` plus optional `argsPrefix`; if multiple rules match in an interactive terminal, the CLI asks which one to delete.

```bash
cargo run -p agentic-gpt -- config allow add bash
cargo run -p agentic-gpt -- config allow remove bash
cargo run -p agentic-gpt -- config confirm add python -c
cargo run -p agentic-gpt -- config confirm remove python -c
```

Path access is controlled by `pathPolicy` in the local agent config. `workspaceRoot` is always a write root; defaults also allow writes under `~/Documents`, `~/Downloads`, and `/tmp`, allow read-only access to selected system/cache paths, and deny common credential/browser/auth paths. Manage roots with:

```bash
cargo run -p agentic-gpt -- config path list
cargo run -p agentic-gpt -- config path write add ~/Projects
cargo run -p agentic-gpt -- config path readonly add /var/log
cargo run -p agentic-gpt -- config path deny add ~/.secrets
cargo run -p agentic-gpt -- config path write remove ~/Projects
```

## Interfaces

Use `openapi/hub.yaml`, replace the server URL with your VPS HTTPS domain, and configure Bearer auth with `AGENTIC_GPT_API_KEY`.

The Hub API exposes a safe runtime summary at `GET /v1/info`, agent discovery at `GET /v1/agents`, short command execution, session APIs, and MCP bridge operations. The Hub API intentionally does not expose task polling. `exec` and `batchExec` wait synchronously up to the short-command limit; long-running work should use the session APIs.

ChatGPT Apps should use the Apps-friendly MCP endpoint at `/mcp`. OAuth discovery and token exchange are implemented by the Hub OAuth shim.

See:

- `docs/interfaces.md`
- `docs/operations.md`

## Verification

```bash
cargo fmt
cargo test --workspace
cargo check --workspace
python3 -c "import yaml; yaml.safe_load(open('openapi/hub.yaml')); print('openapi yaml ok')"
```

## Release Artifacts

```bash
./scripts/dist-linux.sh
```

Artifacts are written to:

- `dist/x86_64-unknown-linux-gnu/agentic-gpt`
- `dist/x86_64-unknown-linux-gnu/agentic-gpt-hub`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt-hub`
