# Agentic GPT

[中文文档](README.zh-CN.md)

Agentic GPT is a Linux local execution agent and Rust Hub for connecting ChatGPT to local machines in a controlled way.

It is designed for workflows where ChatGPT needs to inspect local state, run short commands, start long-running sessions, bridge configured MCP servers, and ask for explicit confirmation before sensitive actions.

```text
ChatGPT Actions / ChatGPT Apps MCP
  -> HTTPS API on Rust Hub
  -> WebSocket connection to Local Agent
  -> local process / session / confirmation / MCP bridge / sandbox
```

The current mainline uses the Rust Hub. The older Cloudflare Worker implementation was moved out of `main`; see branch `legacy/cf-worker-before-removal` only if you need the historical Cloudflare-only Hub.

## Features

- Local command execution through a persistent agent connection.
- Short synchronous commands and long-running sessions.
- Batch command execution with all-or-nothing confirmation semantics.
- Local desktop confirmation and optional Hub-backed remote confirmation.
- Configurable command policy: allow, confirm, deny.
- Path policy with writable, read-only, and denied roots.
- Optional bubblewrap sandbox integration.
- MCP bridge from ChatGPT to MCP servers configured on the local agent.
- Room-scoped asynchronous skill installation from public GitHub/HTTPS/inline sources, plus managed execution of active skill scripts.
- Room-scoped repeated session bootstrap with a concise entrypoint and generic frontmatter-driven capability guides.
- Optional standalone Secure MCP Tunnel runtime with Normal/Room profiles and opt-in reporting-only Hub telemetry.
- ChatGPT Actions OpenAPI schema and ChatGPT Apps-friendly MCP endpoint.

## Repository layout

- `crates/agentic-gpt`: Linux local agent CLI.
- `crates/agentic-gpt-hub`: Rust Hub HTTP/WebSocket service.
- `crates/agentic-gpt-protocol`: Shared JSON protocol types.
- `openapi/hub.yaml`: Custom GPT Actions schema for the Rust Hub.
- `docs/interfaces.md`: Interface map for Actions, Apps MCP, and Local Agent WebSocket.
- `docs/standalone-runtime.md`: Standalone Tunnel topology, configuration, trust, recovery, reporting, and Hub profiles.
- `docs/operations.md`: Local verification, smoke tests, deployment checks, and safety invariants.
- `scripts/dist-linux.sh`: Multi-target Linux release build script.

## Requirements

- Linux local machine for the local agent.
- Release binaries for your target, or Rust stable if building from source.
- A server or VPS for the Hub if you want remote ChatGPT access.
- HTTPS reverse proxy such as Caddy or Nginx for public Hub access.
- Optional: `bubblewrap` for sandboxed execution.
- Optional: `ntfy` for Hub-backed remote confirmation.

## Installation

Download a release archive for your target from GitHub Releases, then extract both binaries and put them somewhere in your `PATH`:

```bash
tar -xzf agentic-gpt-x86_64-unknown-linux-gnu.tar.gz
install -m 0755 agentic-gpt agentic-gpt-hub ~/.local/bin/
```

Supported release targets:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

For building from source, CI, and release publishing, see [`docs/development.md`](docs/development.md).

## Quick start

### 1. Start the Hub

```bash
agentic-gpt-hub init
agentic-gpt-hub agent add \
  --agent-id laptop \
  --display-name my-laptop \
  --secret '<agent-secret>'
AGENTIC_GPT_API_KEY='<high-entropy-api-key>' \
  agentic-gpt-hub serve --bind 127.0.0.1:8787
```

Hub state defaults to `~/.agentic_gpt/hub.sqlite3`; Hub config defaults to `~/.agentic_gpt/hub.json`.

For public access, put Caddy or Nginx in front of the Hub and expose it over HTTPS. The Hub serves both HTTP APIs and WebSocket endpoints.

### 2. Start the Local Agent

```bash
agentic-gpt config init
agentic-gpt config set hubUrl http://127.0.0.1:8787
agentic-gpt config set agentId laptop
agentic-gpt config set agentSecret '<agent-secret>'
agentic-gpt config set confirmationProvider freedesktop-then-ntfy
agentic-gpt run
```

Local agent config lives at `~/.agentic_gpt/config.json`; audit logs are written as JSONL to `~/.agentic_gpt/audit.log`.

`workerUrl` is accepted as a legacy alias when reading or setting config, but `hubUrl` is the canonical field.

### 3. Connect ChatGPT

For Custom GPT Actions, use `openapi/hub.yaml`, replace the server URL with your HTTPS Hub URL, and configure Bearer auth with `AGENTIC_GPT_API_KEY`.

For ChatGPT Apps / MCP, use the Apps-friendly MCP endpoint:

```text
https://<your-hub-domain>/mcp
```

The `/mcp` `tools/call` response uses the Hub `AgenticResult` envelope, which is compatible with ChatGPT Apps / MCP tool results. Hub-native tools return `content`, `structuredContent`, and `isError`; `mcp.callTool` passes through downstream MCP result envelopes, including non-text content blocks and `_meta`.

OAuth discovery and token exchange are implemented by the Hub OAuth shim.

For a direct Secure MCP Tunnel deployment, configure `tunnel.tunnelId` and a
secret reference (`file:PATH` or `env:NAME`), then run
`agentic-gpt run-as-standalone`. Use `--profile room` for the Room surface.
The Tunnel worker exposes a compact 23-tool Normal surface or a 35-tool Room
surface; it supplies its local identity internally and rejects legacy
`agentId`/`confirmMethod` inputs. The complete topology, tool matrix, pinned
client assets, reporting privacy modes, and recovery procedure are in
[`docs/standalone-runtime.md`](docs/standalone-runtime.md).

Room skills are exposed without an input `agentId`: use `skills.install` and poll `skills.install.get` (or the matching `/v1/room/skills/*` Actions routes), then use `skills.run` for executable files beneath an active skill's `scripts/` directory. Installation starts asynchronously for Apps-compatible bounded requests; terminal status includes redacted source provenance and `pollAfterMs` guidance.

## Confirmation

The local agent can request confirmation before commands that match confirm policy rules.

```bash
agentic-gpt config set confirmationProvider freedesktop-then-ntfy
agentic-gpt config set confirmationLanguage zh-CN
```

Supported confirmation channels:

- `freedesktop`: local desktop notification actions.
- `ntfy`: Hub-relayed remote confirmation using the existing ntfy callback path.
- `freedesktop-then-ntfy`: try local desktop confirmation first; fall back to the Hub-relayed ntfy channel only when the local provider is unavailable.

New and rewritten configuration uses the canonical ordered form
`{"channels":["freedesktop","ntfy"]}`. Legacy `hub`,
`freedesktop-then-hub`, `freedesktopThenHub`, `default`, and the object form
`{"provider":"..."}` remain readable for compatibility.

A local denial or timeout is final and does not fall back to Hub.

Supported confirmation languages:

- `en`
- `zh-CN`

Remote confirmation is disabled by default. Enable it on the Hub, not on each Local Agent:

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

## Command policy

Command policy rules can be added or removed by command. `remove` matches `program` plus optional `argsPrefix`; if multiple rules match in an interactive terminal, the CLI asks which one to delete.

```bash
agentic-gpt config allow add bash
agentic-gpt config allow remove bash
agentic-gpt config confirm add python -c
agentic-gpt config confirm remove python -c
agentic-gpt config deny add ssh
```

Policy precedence is intentionally conservative. Builtin deny rules still apply unless explicitly overridden by configured allow rules.

## Path policy

Path access is controlled by `pathPolicy` in the local agent config.

`workspaceRoot` is always a write root. Defaults also allow writes under `~/Documents`, `~/Downloads`, `~/Projects`, and `/tmp`, allow read-only access to selected system/cache paths, and deny common credential, browser, auth, and cloud config paths.

Manage roots with:

```bash
agentic-gpt config path list
agentic-gpt config path write add ~/Projects
agentic-gpt config path readonly add /var/log
agentic-gpt config path deny add ~/.secrets
agentic-gpt config path write remove ~/Projects
```

`process.exec`, `process.batchExec`, and `session.start` also support `workingDirectory`. The resolved directory must exist, must be inside writable roots, and must not be inside denied roots.

## Interfaces

The Hub exposes:

- `GET /v1/info`: safe runtime summary.
- `GET /v1/agents`: ag## More documentation

- [`docs/interfaces.md`](docs/interfaces.md): API, Actions, Apps MCP, and Local Agent WebSocket interface map.
- [`docs/operations.md`](docs/operations.md): deployment checks, smoke tests, and safety invariants.
- [`docs/development.md`](docs/development.md): source development, verification, CI, and release publishing.

gentic-gpt`
- `dist/x86_64-unknown-linux-gnu/agentic-gpt-hub`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt-hub`

Pushing a version tag builds Linux release archives and publishes a GitHub Release:

```bash
git tag v0.1.0
git push origin v0.1.0
```

Release archives contain both binaries for one target:

- `agentic-gpt-x86_64-unknown-linux-gnu.tar.gz`
- `agentic-gpt-aarch64-unknown-linux-gnu.tar.gz`
- `SHA256SUMS`

## Security notes

Agentic GPT is designed to make local execution explicit and auditable, not risk-free. Treat the Hub API key, agent secrets, and ntfy topics as sensitive credentials.

Recommended defaults:

- Use HTTPS in front of the Hub.
- Keep high-entropy Hub API keys and agent secrets.
- Keep credential directories in denied roots.
- Prefer confirmation for shell interpreters and network tools.
- Use sessions for long-running commands instead of forcing short command timeouts.
- Review `~/.agentic_gpt/audit.log` when debugging or tightening policy.

## License

MIT
