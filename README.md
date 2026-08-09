# Agentic GPT

[中文文档](README.zh-CN.md)

Agentic GPT connects ChatGPT to Linux machines through a per-machine Secure MCP Tunnel, an optional centralized Rust Hub, or a private local MCP socket.

For most deployments, **Secure MCP Tunnel / Standalone mode is the recommended path**. It requires no VPS, public reverse proxy, or self-hosted Hub in the command path. Each machine owns an independent tunnel and worker, so one disconnected agent does not interrupt the others.

```text
Recommended — Standalone
ChatGPT Secure MCP Tunnel
  -> official tunnel-client
  -> agentic-gpt worker
  -> policy / files / process Jobs / skills / downstream MCP / tmux

Centralized — Hub
ChatGPT Actions or Apps MCP
  -> HTTPS Rust Hub
  -> WebSocket or SSE Local Agents
  -> the same local execution services

Development — Local
local MCP client or agentic-gpt CLI
  -> owner-only Unix MCP socket
  -> the same Agent surface
```

The historical Cloudflare-only Hub has been removed from `main`; it remains on branch `legacy/cf-worker-before-removal` for archival use.

## Why Standalone first?

- No VPS, public domain, reverse proxy, Hub database, or shared command router is required.
- Every machine has an independent connection and restart boundary.
- The tunnel and owner-only Unix MCP ingress expose the same 24-tool Normal or 36-tool Room surface.
- Policy, confirmation, audit, live configuration, capacity, and managed Jobs stay local to that machine.
- A fresh stdio worker can recover a resumed tunnel request that arrives before a new MCP `initialize` handshake.

Hub mode remains useful when you need one public endpoint for many agents, Custom GPT Actions, centralized run history, Hub-native aggregation/notifications, or Hub-relayed confirmation.

## Choose a runtime

| Runtime | Best for | Public server | Failure scope | Entrypoint |
| --- | --- | --- | --- | --- |
| **Secure MCP Tunnel / Standalone** | Recommended direct deployment | Not required | One tunnel/agent | `agentic-gpt run` (config `mode=standalone`) |
| **Hub + Local Agents** | Central routing, Actions, shared history/reporting | Required | Hub is shared | `agentic-gpt-hub serve` + `agentic-gpt run` |
| **Local Unix MCP** | Development, smoke tests, local automation | Not required | One local worker | `agentic-gpt run` (config `mode=local`) |

## Features

- Unified managed Jobs for `process.exec`, `process.batch`, `skills.run`, `mcp.callTool`, and `mcp.batch`.
- `job.get`, `job.list`, and `job.cancel` for shared lifecycle control.
- Atomic batch admission and bounded confirmation boundaries.
- Configurable allow / confirm / deny command policy.
- Writable, read-only, and denied path roots.
- Local desktop confirmation and optional Hub-backed ntfy confirmation.
- Optional bubblewrap sandbox integration.
- Bounded downstream MCP arguments/results, exact request-id cancellation, and truthful detached state when remote termination cannot be proven.
- Room bootstrap, diary/notebook tools, public skill installation, managed skill execution, and tmux workspaces.
- Optional Rust Hub with Actions OpenAPI, Apps-compatible `/mcp`, OAuth shim, HTTP API, WebSocket/SSE agents, history, reporting, and notifications.

## Repository layout

- `crates/agentic-gpt`: Linux agent, standalone supervisor, local MCP runtime, and CLI.
- `crates/agentic-gpt-hub`: optional Rust Hub HTTP/WebSocket/SSE/MCP service.
- `crates/agentic-gpt-protocol`: shared JSON protocol types.
- `config.example.json`: strict v0.9 standalone-first configuration example with no usable secrets.
- `openapi/hub.yaml`: Custom GPT Actions schema for Hub mode.
- `docs/configuration.md`: runtime-specific configuration, secrets, and reload boundaries.
- `docs/standalone-runtime.md`: Tunnel/local topology, trust, recovery, reporting, and tool matrices.
- `docs/interfaces.md`: Hub HTTP, Actions, Apps MCP, and agent protocol map.
- `docs/operations.md`: verification, deployment, and smoke checks.

## Requirements

Common requirements:

- A Linux machine for `agentic-gpt`.
- A release binary for the target, or Rust stable when building from source.
- Optional `bubblewrap` for sandboxed execution.

Standalone additionally needs an assigned Secure MCP Tunnel id and API key reference. It does **not** require a VPS or inbound public port.

Hub mode additionally needs a server/VPS, HTTPS, a reverse proxy when exposed publicly, a Hub API key, and per-agent secrets.

## Installation

Release archives contain both binaries. Standalone and Local modes need only `agentic-gpt`; install `agentic-gpt-hub` only for Hub mode.

```bash
tar -xzf agentic-gpt-x86_64-unknown-linux-gnu.tar.gz
install -m 0755 agentic-gpt ~/.local/bin/
# Hub mode only:
install -m 0755 agentic-gpt-hub ~/.local/bin/
```

Supported targets:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

Source builds, CI, and release publishing are documented in [`docs/development.md`](docs/development.md).

## Quick start: Secure MCP Tunnel (recommended)

### 1. Initialize the local configuration

On a terminal, `agentic-gpt config init` opens the keyboard-driven fullscreen setup UI. Its
defaults are Standalone mode and the Normal capability profile. Fullscreen mode requires stdin,
stdout, and stderr to all be terminals; a pipe or redirected stream returns an actionable error
and writes nothing. Scripts and CI must choose the deterministic builder explicitly with
`--non-interactive`.

```bash
agentic-gpt config init
agentic-gpt config set agentId laptop
agentic-gpt config set confirmationProvider.channels '["freedesktop"]'
```

For scripts, pass every value that must be deterministic and add `--non-interactive`:

```bash
agentic-gpt config init --non-interactive
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt config init \
  --mode standalone \
  --profile room \
  --tunnel-id tunnel_<assigned-id> \
  --tunnel-api-key file:"$HOME/.agentic_gpt/secrets/tunnel-api-key" \
  --non-interactive
```

The first command writes safe Standalone + Normal placeholders and reports the tunnel ID and
secret-reference actions still pending; it does not provision a secret automatically. `--mode`
selects the runtime transport (Standalone, Hub, or Local), while `--profile` selects the tool
surface (Normal or Room). In fullscreen mode these flags seed editable fields rather than
locking them. The flow is Basic → Connection (except Local) → Optional settings → Review →
Completion; optional sections can be revisited, and Review redacts secret values and can jump
back to an earlier section. Tab/Shift+Tab and arrow keys move focus, Enter edits or activates,
Esc backs out (and is a no-op on the root Basic page), and Ctrl+C cancels. In fullscreen mode, no
config, backup, or secret file is written until final Review confirmation. `--agent-secret` is visible to shell
history and local process inspection; prefer hidden input in the fullscreen UI. For tunnel API
keys, use a `file:`/`env:` reference. This feature documents keyboard fullscreen setup only; it
does not promise mouse, inline, dashboard, or Windows behavior.

The default config path is `~/.agentic_gpt/config.json`. Review [`config.example.json`](config.example.json) and [`docs/configuration.md`](docs/configuration.md) before exposing write roots or enabling MCP servers.
Use `agentic-gpt config keys [--section <SECTION>] [--json]` to inspect the controlled `config set` registry.

### 2. Store the tunnel secret by reference

```bash
install -d -m 700 "$HOME/.agentic_gpt/secrets"
touch "$HOME/.agentic_gpt/secrets/tunnel-api-key"
chmod 600 "$HOME/.agentic_gpt/secrets/tunnel-api-key"
read -rsp "Tunnel API key: " AGENTIC_TUNNEL_API_KEY
printf '\n'
printf '%s' "$AGENTIC_TUNNEL_API_KEY" > "$HOME/.agentic_gpt/secrets/tunnel-api-key"
unset AGENTIC_TUNNEL_API_KEY

agentic-gpt config set tunnel.tunnelId tunnel_<assigned-id>
agentic-gpt config set tunnel.apiKey file:"$HOME/.agentic_gpt/secrets/tunnel-api-key"
agentic-gpt config set tunnel.client.autoDownload true
```

`tunnel.apiKey` accepts only `file:PATH` or `env:NAME`; plaintext secrets are rejected.

### 3. Start the configured runtime

```bash
agentic-gpt run
```

The config file’s `mode` and `profile` select Standalone/Hub/Local and Normal/Room; for example, use
`agentic-gpt config set profile room` before starting for the 36-tool Room surface. The same worker
also exposes an owner-only Unix MCP socket for local inspection:

```bash
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

Connect ChatGPT through the Secure MCP Tunnel assigned to this agent. Each machine is configured and started independently.

Complete tunnel-client trust, cache, recovery, reporting, and service-manager guidance is in [`docs/standalone-runtime.md`](docs/standalone-runtime.md).

## Local-only development

No tunnel credentials are needed:

```bash
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt run
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

Local mode uses the same policy, path policy, confirmation, audit, live config, and Job implementation as Standalone mode, but serves only the owner-only Unix socket.

## Centralized Hub mode

Choose Hub mode when the shared endpoint and centralized capabilities are worth the extra infrastructure.

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

Put Caddy or Nginx in front of the Hub and expose it over HTTPS. Hub state defaults to `~/.agentic_gpt/hub.sqlite3`; Hub config defaults to `~/.agentic_gpt/hub.json`.

### 2. Start a Hub-connected agent

```bash
agentic-gpt config init
agentic-gpt config set hub.url https://agentic-gpt.example.com
agentic-gpt config set hub.transport websocket
agentic-gpt config set agentId laptop
agentic-gpt config set hub.agentSecret '<agent-secret>'
agentic-gpt run
```

Set `profile` to `room` before `agentic-gpt run` for the Room profile. `hub.transport` may be `websocket` or `sse`.

For an existing v0.9 or external JSON file, use the explicit migration flow:
`agentic-gpt config import --config PATH [SOURCE]` (omit `--config` for the default path). If
`SOURCE` is omitted, the selected config path is imported. It seeds the normal Config Init TUI and
writes the current nested Hub schema with the usual backup transaction.

### 3. Connect ChatGPT to the Hub

- Custom GPT Actions: import [`openapi/hub.yaml`](openapi/hub.yaml) and use `AGENTIC_GPT_API_KEY` as Bearer auth.
- ChatGPT Apps MCP: connect to `https://<your-hub-domain>/mcp`.

Hub-native and forwarded execution use the same managed Job envelopes. Active work is inspected with `job.get` and cancelled with `job.cancel`.

## Managed Jobs and safety boundaries

- Normal exposes 24 tools; Room exposes 36.
- `process.exec`, `skills.run`, and `mcp.callTool` return `JobResponse`.
- `mcp.batch` accepts 1–16 ordered calls, uses one aggregate confirmation, and enforces global/per-server concurrency.
- MCP arguments are JSON objects capped at 256 KiB per call; retained results are capped at 512 KiB; aggregate batch arguments/results are capped at 2 MiB.
- Audit records contain bounded metadata, hashes, states, and termination evidence rather than raw MCP arguments/results.
- Use `agent.info` before execution to inspect the active profile, path policy, capacity, confirmation, MCP configuration summary, and connection state.

## Confirmation, command policy, and path policy

```bash
agentic-gpt config set confirmationProvider.channels '["freedesktop"]'
agentic-gpt config set confirmationLanguage zh-CN

agentic-gpt config allow add bash
agentic-gpt config confirm add python -c
agentic-gpt config deny add ssh

agentic-gpt config path list
agentic-gpt config path write add ~/Projects
agentic-gpt config path readonly add /var/log
agentic-gpt config path deny add ~/.secrets
```

Hub-backed `ntfy` is optional and is useful only when Hub mode or standalone Hub reporting/confirmation relay is configured. Local denial or timeout is final.

Detailed field definitions and live-reload behavior are in [`docs/configuration.md`](docs/configuration.md).

## Upgrade to v0.9

v0.9 is intentionally breaking:

- `limits.maxActiveSessions` becomes `limits.maxActiveJobs`.
- Remove `sessionIdleTimeoutSecs`.
- Managed `session.*` and `process.get/list/kill` become `job.get/list/cancel`.
- `process.batchExec` becomes `process.batch`.
- `mcp.callTool` returns a managed `JobResponse`.

Standalone/Local deployments upgrade `agentic-gpt` independently. Hub deployments must upgrade Hub and connected agents together because the v0.9 protocol is not wire-compatible with v0.8. See [`docs/migration-v0.9.md`](docs/migration-v0.9.md).

## More documentation

- [`docs/configuration.md`](docs/configuration.md): runtime selection, all major config sections, secret references, and reload/restart boundaries.
- [`docs/standalone-runtime.md`](docs/standalone-runtime.md): standalone/local operation, tunnel-client trust, recovery, reporting, and exact tool matrices.
- [`docs/interfaces.md`](docs/interfaces.md): Hub HTTP, Actions, Apps MCP, protocol, and direct MCP surface references.
- [`docs/tool-contract-matrix.md`](docs/tool-contract-matrix.md): checked-in Normal/Room/Hub tool contract and parity matrix.
- [`docs/operations.md`](docs/operations.md): local verification, Standalone-first deployment checks, Hub checks, and safety invariants.
- [`docs/migration-v0.9.md`](docs/migration-v0.9.md): v0.8 to v0.9 migration by runtime.
- [`docs/migration-v0.10.md`](docs/migration-v0.10.md): file.batch per-file group and partial-success migration.
- [`docs/release-notes-v0.9.1.md`](docs/release-notes-v0.9.1.md): v0.9.1 release boundary and verification summary.
- [`docs/release-notes-v0.9.0.md`](docs/release-notes-v0.9.0.md): v0.9.0 feature and verification summary.
- [`docs/development.md`](docs/development.md): development, CI, and release publishing.

## Build and release

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git tag v0.9.0
git push origin v0.9.0
```

Creating or pushing a tag is a separate release action; normal commits do not publish anything.

## Security notes

- Treat tunnel API keys, Hub API keys, agent secrets, and ntfy topics as credentials.
- Prefer `file:` or protected `env:` secret references; never store a tunnel key as plaintext config.
- Keep credential, browser, cloud, and SSH directories in denied roots.
- Prefer confirmation for shells, network tools, and unfamiliar MCP servers.
- Use bounded Job waits instead of long blocking HTTP/MCP requests.
- Hub mode must use HTTPS when exposed publicly.
- Do not start v0.9 with an unmigrated v0.8 limits object.

## License

MIT
