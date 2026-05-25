# Agentic GPT

Agentic GPT is a Linux local execution agent plus a VPS Hub for controlled GPT Actions command execution.

The current default architecture is:

```text
Custom GPT Actions
  -> HTTPS API on VPS Hub
  -> WebSocket connection to Local Agent
  -> local process / session / confirmation / sandbox
```

The old Cloudflare Worker implementation remains under `apps/worker` as legacy code, but new work should target the Rust Hub.

## Layout

- `crates/agentic-gpt`: Rust CLI local agent.
- `crates/agentic-gpt-hub`: Rust VPS Hub HTTP/WebSocket service.
- `crates/agentic-gpt-protocol`: Shared JSON protocol types.
- `openapi/hub.yaml`: Custom GPT Actions schema for the VPS Hub.
- `apps/worker`: Legacy Cloudflare Worker implementation.
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
cargo run -p agentic-gpt -- run
```

Config lives at `~/.agentic_gpt/config.json`; audit logs are JSONL at `~/.agentic_gpt/audit.log`.

`workerUrl` is accepted as a legacy alias when reading or setting config, but `hubUrl` is the canonical field.

`freedesktop-then-hub` first tries local desktop notification actions. It falls back to Hub-backed ntfy only when the local provider is unavailable or cannot show the notification. User denial or timeout from the local provider is final and does not fall back.

Path access is controlled by `pathPolicy` in the local agent config. `workspaceRoot` is always a write root; defaults also allow writes under `~/Documents`, `~/Downloads`, and `/tmp`, allow read-only access to selected system/cache paths, and deny common credential/browser/auth paths. Manage roots with:

```bash
cargo run -p agentic-gpt -- config path list
cargo run -p agentic-gpt -- config path write add ~/Projects
cargo run -p agentic-gpt -- config path readonly add /var/log
cargo run -p agentic-gpt -- config path deny add ~/.secrets
cargo run -p agentic-gpt -- config path write remove ~/Projects
```

## GPT Actions

Use `openapi/hub.yaml`, replace the server URL with your VPS HTTPS domain, and configure Bearer auth with `AGENTIC_GPT_API_KEY`.

The Hub API intentionally does not expose task polling. `exec` and `batchExec` wait synchronously up to the short-command limit; long-running work should use the session APIs.

## Verification

```bash
cargo test --workspace
pnpm --filter worker test
cargo check --workspace
```

## Release Artifacts

```bash
pnpm dist:linux
```

Artifacts are written to:

- `dist/x86_64-unknown-linux-gnu/agentic-gpt`
- `dist/x86_64-unknown-linux-gnu/agentic-gpt-hub`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt-hub`
