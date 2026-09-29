# Agentic GPT

[Chinese README](README.zh-CN.md)

Agentic GPT connects ChatGPT to Linux machines through a per-machine Secure MCP Tunnel, an optional centralized Rust Hub, or a local-only Unix MCP socket.

For most deployments, **start with Secure MCP Tunnel / Standalone mode**. It needs no VPS, public reverse proxy, or self-hosted Hub in the command execution path. Each machine has its own tunnel and worker, so one Agent disconnecting does not affect the others.

```text
Recommended — Standalone
ChatGPT Secure MCP Tunnel
  -> official tunnel-client
  -> agentic-gpt worker
  -> stdio MCP + owner-only Unix MCP
  -> optional worker-owned HTTP MCP endpoint: http://<host>:<port>/mcp
  -> policy / files / managed processes / skills / downstream MCP / tmux / Browser runtime

Centralized — Hub
ChatGPT Actions or Apps MCP
  -> HTTPS Rust Hub
  -> local Agent over WebSocket or SSE
  -> the same local execution capabilities

Development — Local
Local MCP client or agentic-gpt CLI
  -> owner-only Unix MCP socket
  -> the same Agent tool surface
```

The historical Cloudflare-only Hub has been removed from `main`; it remains archived on the `legacy/cf-worker-before-removal` branch.

## Why start with Standalone?

- No VPS, public domain, reverse proxy, Hub database, or shared command router.
- Independent connection and restart boundaries for each machine.
- Tools exposed through Tunnel, HTTP, and owner-only Unix MCP ingress are selected by toolset. Normal/Room profiles supply defaults, but explicit `toolsets.enabled` takes precedence. Live discovery with the default profiles exposes 31 tools for Normal and 42 for Room; explicit selections can change those counts.
- Standalone can also expose a worker-owned Streamable HTTP MCP endpoint at the fixed `/mcp` path. It is disabled by default and supports direct bearer-token authentication or a ChatGPT connector OAuth flow.
- Policy, confirmation, audit, live configuration, capacity, and managed process state stay on the machine.
- A newly started stdio worker can restore a resumed tunnel session even when its first request arrives before a fresh MCP `initialize` handshake.

Choose Hub mode when you need one public entry point for multiple Agents, Custom GPT Actions, centralized run history, Hub-native aggregation/notifications, or Hub-relayed confirmation.

## Choose a runtime mode

| Mode | Use case | Public server required? | Failure scope | Start command |
| --- | --- | --- | --- | --- |
| **Secure MCP Tunnel / Standalone** | Recommended direct deployment; optional worker-owned HTTP MCP | No | One tunnel/Agent | `agentic-gpt run` (configure `mode=standalone`) |
| **Hub + Local Agent** | Central routing, Actions, shared history/reporting | Yes | Hub is a shared dependency | `agentic-gpt-hub serve` + `agentic-gpt run` |
| **Local Unix MCP** | Development, smoke checks, local automation | No | One local worker | `agentic-gpt run` (configure `mode=local`) |

## Capabilities

- `process.exec`, `process.batch`, `skills.run`, `mcp.callTool`, and `mcp.batch` can create managed processes.
- Use `process.status`, `process.list`, `process.output`, `process.result`, and `process.cancel` to inspect or control a process by `processId`.
- Batch admission is atomic, with bounded confirmation boundaries.
- Configurable allow / confirm / deny command policy.
- Writable, read-only, and denied path roots.
- Local desktop confirmation and optional Hub-relayed ntfy confirmation.
- Optional bubblewrap sandbox integration.
- Bounded downstream MCP arguments/results and exact request-ID cancellation; `detached` is reported when remote termination cannot be proven.
- Room bootstrap, diary/notebook tools, public-source Skill installation, managed Skill execution, and tmux workspaces.
- An optional Rust Hub with Actions OpenAPI, an Apps-compatible `/mcp`, OAuth shim, HTTP API, WebSocket/SSE Agents, history, reporting, and notifications.

## Repository layout

- `crates/agentic-gpt`: Linux Agent, Standalone supervisor, local MCP runtime, and CLI.
- `crates/agentic-gpt-hub`: optional Rust Hub HTTP/WebSocket/SSE/MCP service.
- `crates/agentic-browser-host`: optional Linux Native Messaging bridge for a self-hosted ChatGPT Chrome extension Browser backend.
- `crates/agentic-gpt-protocol`: shared JSON protocol types.
- `config.example.json`: strict v0.9, Standalone-first example without usable secrets.
- `openapi/hub.yaml`: Custom GPT Actions schema for Hub mode.
- `docs/configuration.md`: configuration, secrets, and reload boundaries by runtime.
- `docs/standalone-runtime.md`: Tunnel/local topology, trust, recovery, reporting, and tool matrix.
- `docs/interfaces.md`: index of Hub HTTP, Actions, Apps MCP, and Agent protocols.
- `docs/operations.md`: verification, deployment, and smoke checks.

## Requirements

All modes:

- Run `agentic-gpt` on a Linux machine.
- Use a release binary for your architecture or build from source with stable Rust.
- Optionally install `bubblewrap` for sandboxed execution.

Standalone also needs an assigned Secure MCP Tunnel ID and API-key reference, but **does not need a VPS or public inbound port**.

Hub mode additionally needs a server/VPS, HTTPS, a reverse proxy for public deployments, a Hub API key, and an individual secret for each Agent.

## Install

Release archives contain Agent, Hub, and Browser-host binaries. Standalone and Local need only `agentic-gpt`; install `agentic-gpt-hub` only for Hub mode, and `agentic-browser-host` only for a self-hosted Chrome extension Browser backend.

```bash
tar -xzf agentic-gpt-x86_64-unknown-linux-gnu.tar.gz
install -m 0755 agentic-gpt ~/.local/bin/
# Hub mode only:
install -m 0755 agentic-gpt-hub ~/.local/bin/
# Self-hosted Browser backend only:
install -m 0755 agentic-browser-host ~/.local/bin/
```

Supported targets:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

See the [development guide](docs/development/README.md) before adding features or modules or changing configuration, state ownership, or public contracts. See the [release guide](docs/development/releasing.md) for packaging and publishing.
See the [self-hosted Browser guide](docs/browser-self-hosted.md) for extension/Neko deployment.

## Quick start: Secure MCP Tunnel (recommended)

### 1. Initialize local configuration

Run `agentic-gpt config init` in a terminal to open the keyboard-driven full-screen configuration UI. It defaults to Standalone mode and the Normal toolset preset. Full-screen mode requires stdin, stdout, and stderr to be connected to a terminal; a pipe or redirect produces an actionable error without writing anything. Scripts and CI must explicitly use `--non-interactive` for deterministic configuration construction.

```bash
agentic-gpt config init
agentic-gpt config set agentId laptop
agentic-gpt config set confirmationProvider.channels '["freedesktop"]'
```

In scripts, supply the values you need to fix explicitly and add `--non-interactive`:

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

The first command writes a safe Standalone + Normal placeholder configuration and reports the outstanding tunnel ID and secret-reference steps; it does not configure a secret automatically. `--mode` selects the runtime transport (Standalone, Hub, or Local); `--profile` selects a default toolset preset (normal enables every namespace except `room`; room enables all namespaces). Explicit `toolsets.enabled` overrides the profile preset. In full-screen mode, these options only prefill editable fields. The flow is Basic → Connection (except Local) → Optional settings → Review → Completion. You can revisit optional settings, Review masks secrets, and you can return to earlier pages to edit them. Use Tab/Shift+Tab and arrow keys to move focus, Enter to edit or act, Esc to go back (a no-op on the root Basic page), and Ctrl+C to cancel. Nothing is written to configuration, backup, or secret files before final confirmation on Review. `--agent-secret` is a command-line argument: passing it exposes the secret to local process inspection; typing a literal secret in a shell command also records it in shell history. Prefer the full-screen UI's masked input. Use `file:`/`env:` references for the Tunnel API key. This guide describes only the keyboard-driven full-screen flow; it makes no claim about mouse controls, inline/dashboard modes, or Windows behavior.

The default configuration path is `~/.agentic_gpt/config.json`. Before opening writable roots or enabling MCP servers, review [`config.example.json`](config.example.json) and the [configuration guide](docs/configuration.md).
Room settings live under `room`: `repositoryRoot` is optional and defaults to `<workspaceRoot>/room`; `maintenance.mode` accepts `local` or `workflow` and defaults to `local`; `maintenance.autoPush` defaults to `false`. You can also set these with `agentic-gpt config set room.repositoryRoot null`, `agentic-gpt config set room.maintenance.mode local`, and `agentic-gpt config set room.maintenance.autoPush false`.
Use `agentic-gpt config keys [--section <SECTION>] [--json]` to inspect the controlled `config set` registry.
Manage tool namespaces with:

```bash
agentic-gpt config toolset ls
agentic-gpt config toolset enable <namespace>
agentic-gpt config toolset disable <namespace>
```

`ls` displays each namespace's enabled state and short description; successful `enable` and `disable` operations report the namespace and its resulting state.

Available namespaces are `agent`, `file`, `mcp`, `process`, `skills`, `tmux`, `browser`, and `room`. The logical `room` namespace includes `bootstrap`, `bootstrap.read`, and all `room.*` tools. You can also edit `toolsets.enabled` in JSON directly; valid edits reload without a restart, while an invalid candidate leaves the last valid selection in place.

### 2. Store the tunnel key by reference

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

The configuration's `mode` selects Standalone/Hub/Local, and `profile` selects a default toolset preset. By default, normal enables all namespaces except `room`, and room enables all namespaces; explicit `toolsets.enabled` takes precedence. The same worker also exposes an owner-only Unix MCP socket for local inspection:

```bash
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

Connect ChatGPT to the Secure MCP Tunnel assigned to this Agent. Configure and start each machine independently.

Optional Standalone HTTP MCP is configured independently of the tunnel:

```bash
agentic-gpt config set httpMcp.bearerToken env:AGENTIC_HTTP_MCP_TOKEN
agentic-gpt config set httpMcp.host 127.0.0.1
agentic-gpt config set httpMcp.port 8765
agentic-gpt config set httpMcp.publicUrl https://mcp.example.com
agentic-gpt config set httpMcp.allowHosts '["mcp.example.com"]'
agentic-gpt config set httpMcp.enabled true
```

The endpoint is always `http://<host>:<port>/mcp`. Direct bearer-token authentication uses a `file:` or `env:` secret reference and works without `publicUrl`. The optional, non-secret HTTPS `publicUrl` advertises the external origin for ChatGPT connector OAuth; it is shown and editable in clear text in the full-screen TUI, via `config init --non-interactive --http-mcp-public-url ...`, or via `config set httpMcp.publicUrl ...` (set `null` to clear it). It does not change the local bind host or port.

With `publicUrl` configured, Standalone exposes OAuth resource discovery at `/.well-known/oauth-protected-resource/mcp` and a root-path compatibility alias at `/.well-known/oauth-protected-resource`. Authorization-server/OpenID Connect (AS/OIDC) metadata is available at `/.well-known/oauth-authorization-server` and `/.well-known/openid-configuration`. ChatGPT OAuth uses `/oauth/authorize`, `/oauth/token`, and the sole scope `agentic:mcp`. Accepted redirect URLs are limited to `https://chatgpt.com/connector/oauth/<suffix>` and the exact `https://chatgpt.com/connector_platform_oauth_redirect`. Authorization codes and access tokens live only in the current listener's memory and can expire or be revoked; there are no refresh tokens, `offline_access`, dynamic client registration, general registration, or arbitrary redirects. OAuth does not replace direct bearer-token authentication. Standalone pages, tools, and `profile` semantics are separate from Hub API-key, profile, and routing contracts.

Host filtering defaults to loopback authorities. `null` or exactly `["*"]` explicitly allows any Host; an empty array or a wildcard mixed with other entries is rejected. Allow the Host authority actually received (including rewritten Host headers from a reverse proxy or ESA). If an `Origin` header is present, it must exactly match `publicUrl`; server-to-server requests without `Origin` remain allowed. ChatGPT OAuth deployments must forward all discovery, authorization, token, and `/mcp` routes over HTTPS. The origin can be loopback or a private-network address; `publicUrl` does not route traffic, so include the authority actually sent by the proxy in `allowHosts`. Rebinding or disabling the listener closes stateful sessions and discards its local OAuth `state`; rotating direct token content updates authentication in place and revokes OAuth codes/tokens.
Local mode still exposes only Unix ingress. Hub's `/mcp` has a separate OAuth/Hub contract; `mcpServers` is the downstream registry used by `mcp.*`, not this inbound listener.

See the [configuration guide](docs/configuration.md) for the full schema, init/import editing flow, redaction, and live-reload/last-good behavior.

See the [Standalone runtime guide](docs/standalone-runtime.md) for tunnel-client trust, caching, recovery, reporting, and service-manager details.

## Local-only development

No tunnel credentials are needed:

```bash
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt run
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

Local and Standalone share the same command and path policy, confirmation, audit, live configuration, and process implementation; Local exposes only an owner-only Unix socket.

## Centralized Hub mode

Choose Hub mode when a shared entry point and centralized capabilities justify the extra infrastructure.

### 1. Start the Hub

```bash
agentic-gpt-hub init
read -rsp "Agent secret: " AGENT_SECRET
printf '\n'
agentic-gpt-hub agent add \
  --agent-id laptop \
  --display-name my-laptop \
  --secret "$AGENT_SECRET"
unset AGENT_SECRET

read -rsp "Hub API key: " AGENTIC_GPT_API_KEY
printf '\n'
export AGENTIC_GPT_API_KEY
agentic-gpt-hub serve --bind 127.0.0.1:8787
unset AGENTIC_GPT_API_KEY
```

Hub's `agent add` has no masked interactive prompt; `--secret` is required on the command line. The `read -s` example keeps the literal secret out of shell history, but shell expansion still exposes it to local process inspection. Run it on a trusted machine and clear the variable afterward. The Hub API key is passed through the environment, so protect the process environment as well. For public deployment, place Caddy or Nginx in front of the Hub and expose it over HTTPS. Hub state defaults to `~/.agentic_gpt/hub.sqlite3`, and configuration to `~/.agentic_gpt/hub.json`.

### 2. Start an Agent connected to the Hub

```bash
agentic-gpt config init --mode hub
agentic-gpt config set hub.url https://agentic-gpt.example.com
agentic-gpt config set hub.transport websocket
agentic-gpt config set agentId laptop
agentic-gpt run
```

Set `profile` to `room` and use `agentic-gpt run` to start a Room-profile Agent. Explicit `toolsets.enabled` takes precedence; `hub.transport` accepts `websocket` or `sse`. `config init --mode hub` opens the full-screen TUI with Hub mode prefilled. Enter the same Agent secret used for Hub registration through the masked input on the Connection page. Do not pass it via `config set hub.agentSecret` or `config init --agent-secret`: argument values are exposed to local process inspection, and a literal secret typed into a shell command also enters shell history.

For an existing v0.9 configuration or external JSON, use the explicit migration flow `agentic-gpt config import --config PATH [SOURCE]` (`--config` is optional for the default path). Omitting `SOURCE` imports the selected configuration path. This loads values into the standard Config Init TUI and writes the current nested Hub schema through the usual backup transaction.

### 3. Connect ChatGPT to the Hub

- Custom GPT Actions: import [`openapi/hub.yaml`](openapi/hub.yaml) and authenticate with `AGENTIC_GPT_API_KEY` as a Bearer token.
- ChatGPT Apps MCP: connect to `https://<your-hub-domain>/mcp`.

Hub-native tools and forwarded execution use the same process-lifecycle projection. Use `process.status` or `process.list` to inspect work, `process.output` and `process.result` to retrieve output/results, and `process.cancel` to cancel, passing the returned `processId`.

## Managed processes and safety boundaries

- Every managed process has a `processId` and a status reflecting its actual lifecycle. `process.status` returns metadata only; use `process.output` for bounded output and `process.result` for retained results.
- The Worker HTTP API exposes `GET /v1/process` (list), `GET /v1/process/{processId}` (status), `GET /v1/process/{processId}/output`, `GET /v1/process/{processId}/result`, and `POST /v1/process/{processId}/cancel`. Hub MCP exposes `hub.process.status` and `hub.process.list`.
- `process.exec`, `skills.run`, and `mcp.callTool` start managed processes. `process.batch` and `mcp.batch` return ordered child-process projections. `mcp.batch` accepts 1–16 calls with one aggregate confirmation and global/per-server concurrency limits.
- Creation responses inline at most 8 KiB of output; larger initial output uses a shared preview capped at 2 KiB. The default `process.output` cursor window is 8 KiB; the maximum is 32 KiB.
- MCP call arguments must be JSON objects and are limited to 256 KiB; retained process results are limited to 512 KiB; aggregate batch arguments and results are each limited to 2 MiB.
- Worker process state is stored in `process.sqlite3`. Initializing the process store does not migrate or modify old `jobs.sqlite3` data.
- Audit records contain bounded metadata, hashes, status, and termination evidence, not raw MCP arguments/results.
- Check `agent.info` for the current profile, path policy, capacity, confirmation, MCP configuration summary, and connection status before execution.

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

Hub-relayed `ntfy` is optional and useful only in Hub mode or with Standalone Hub reporting/confirmation relay configured. A local denial or timeout is final.

See the [configuration guide](docs/configuration.md) for field definitions and live-reload behavior.

## More documentation

- [Configuration](docs/configuration.md): runtime selection, major configuration sections, secret references, and live-reload/restart boundaries.
- [Standalone runtime](docs/standalone-runtime.md): Standalone/Local operation, tunnel-client trust, recovery, reporting, and the precise tool matrix.
- [Interface index](docs/interfaces.md): Hub HTTP, Actions, Apps MCP, protocol, and direct MCP interfaces.
- [Tool contract matrix](docs/tool-contract-matrix.md): Normal/Room/Hub tool contracts and cross-surface parity.
- [Operations](docs/operations.md): local verification, Standalone-first deployment checks, Hub checks, and safety invariants.
- [Development](docs/development/README.md): read before implementing features/modules or reviewing changes to ownership, permissions, configuration, or contracts; includes verification and completion criteria.
- [Releases](docs/development/releasing.md): read before release preflight, packaging, or pushing a release tag.

## Build and release

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
rust_version="$(cargo metadata --no-deps --format-version=1 | python3 -c 'import json,sys; packages={p["name"]:p["version"] for p in json.load(sys.stdin)["packages"]}; print(packages["agentic-gpt"])')"
git tag "v${rust_version}"
git push origin "v${rust_version}"
```

The tag version is derived from the `agentic-gpt` Cargo package version; `agentic-gpt-hub` must use the same version, and release preflight checks that they match. Creating or pushing a tag is a separate release action; an ordinary commit publishes nothing.

## Security notes

- Treat Tunnel API keys, Hub API keys, Agent secrets, and ntfy topics as credentials.
- Prefer `file:` or protected `env:` secret references; never write a plaintext tunnel key into configuration.
- Keep credentials, browser, cloud-platform, and SSH directories in denied roots.
- Prefer confirmation for shell commands, network tools, and unfamiliar MCP servers.
- Retrieve results through bounded process output/result rather than leaving HTTP/MCP requests blocked for a long time.
- Public Hub deployments require HTTPS.
- Do not let v0.9 read an unmigrated v0.8 limits object.

## License

Original AgenticGPT code and the main codebase use the MIT License.

The `crates/agentic-apply-patch` subcrate contains code derived from OpenAI Codex under the Apache License 2.0 license. See [`crates/agentic-apply-patch/LICENSE`](crates/agentic-apply-patch/LICENSE) and [`crates/agentic-apply-patch/NOTICE`](crates/agentic-apply-patch/NOTICE).
