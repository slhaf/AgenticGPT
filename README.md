# Agentic GPT

[中文](README.zh-CN.md)

Agentic GPT connects ChatGPT to Linux machines for file operations, command execution, skills, browser work, and local events.

**Standalone** is the recommended mode: each machine connects through its own Secure MCP Tunnel, without a self-hosted Hub, VPS, or inbound port.

| Runtime mode | Use case |
| --- | --- |
| Standalone | Connect directly to a machine through Secure MCP Tunnel; HTTP MCP ingress is also available. |
| Local | Develop, debug, and automate through a local Unix MCP socket. |
| Hub | Provide one entry point for multiple machines, Custom GPT Actions, or centralized run records. |

## Capabilities

- Read, search, and edit files, including images and patches.
- Run commands and batches, inspect output and history, and cancel running processes.
- Install and run skills, and call downstream MCP services.
- Use tmux workspaces and the Browser runtime.
- Persist local events and inspect or handle reminders through the event inbox.
- Manage Diary, Notebook, and maintenance tasks through Room.
- Configure command and path policies, desktop or ntfy confirmation, and optional bubblewrap sandboxing.

## Install and requirements

The Agent runs on Linux. Release archives support `x86_64` and `aarch64`; download the archive for your architecture from the [releases page](https://github.com/slhaf/AgenticGPT/releases). For `x86_64`:

```bash
tar -xzf agentic-gpt-x86_64-unknown-linux-gnu.tar.gz
install -d ~/.local/bin
install -m 0755 agentic-gpt ~/.local/bin/
```

Ensure `~/.local/bin` is in `PATH`. Archives also contain `agentic-gpt-hub` and `agentic-browser-host` for Hub deployments and a self-hosted Browser backend respectively; install them as needed.

**Before upgrading an older deployment to 0.10.0, read the [release notes and upgrade steps](docs/release-notes-v0.10.0.md).**

## Quick start: Standalone

Have the Secure MCP Tunnel ID and API key assigned to this machine ready.

### 1. Initialize configuration

```bash
agentic-gpt config init
```

Choose Standalone in the configuration UI and set the Agent identity and Tunnel connection details. The default config file is `~/.agentic_gpt/config.json`. Scripts can use `config init --non-interactive`; see the [configuration guide](docs/configuration.md) for options.

### 2. Store Tunnel credentials

The following stores the API key in a private file and configures a file reference. Replace `tunnel_<assigned-id>` with the assigned Tunnel ID.

```bash
install -d -m 700 "$HOME/.agentic_gpt/secrets"
touch "$HOME/.agentic_gpt/secrets/tunnel-api-key"
chmod 600 "$HOME/.agentic_gpt/secrets/tunnel-api-key"
read -rsp "Tunnel API key: " AGENTIC_TUNNEL_API_KEY
printf '\n'
printf '%s' "$AGENTIC_TUNNEL_API_KEY" > "$HOME/.agentic_gpt/secrets/tunnel-api-key"
unset AGENTIC_TUNNEL_API_KEY

agentic-gpt config set tunnel.tunnelId 'tunnel_<assigned-id>'
agentic-gpt config set tunnel.apiKey file:"$HOME/.agentic_gpt/secrets/tunnel-api-key"
```

Tunnel API keys use `file:` or `env:` references; plaintext configuration is rejected. Adjust the working directory, accessible paths, and confirmation settings in the initialization UI or config file.

### 3. Start and connect

```bash
agentic-gpt run
```

Connect ChatGPT to the Secure MCP Tunnel assigned to this machine. Inspect the local tools and runtime status with:

```bash
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

See the [configuration guide](docs/configuration.md#standalone-入站-http-mcp-端点) for HTTP MCP ingress and authentication, and the [Standalone runtime guide](docs/standalone-runtime.md) for service management and troubleshooting.

## Other runtime modes

### Local

Use the Agent through a local Unix MCP socket without Tunnel credentials:

```bash
agentic-gpt config init --mode local --non-interactive
agentic-gpt run
```

Use the `local list-tools` and `local call` commands above to access the Agent; stdio MCP clients can use `agentic-gpt local stdio`. See the [local MCP control channel](docs/standalone-runtime.md).

### Hub

Install `agentic-gpt-hub` from the release archive, register the Agent with the Hub, then configure its connection with `agentic-gpt config init --mode hub`. Public deployments require HTTPS.

- Custom GPT Actions use [`openapi/hub.yaml`](openapi/hub.yaml).
- ChatGPT Apps MCP connects to the Hub's `/mcp` endpoint.

See the [Hub runtime guide](docs/standalone-runtime.md) and [operations guide](docs/operations.md#hub-部署检查) for deployment steps, credentials, and proxy configuration.

## Documentation

| Topic | Documentation |
| --- | --- |
| Configuration, access policies, confirmation, Toolsets | [Configuration](docs/configuration.md) · [Example config](config.example.json) |
| Tunnel, local MCP, service management | [Standalone runtime](docs/standalone-runtime.md) |
| Tools and HTTP API | [Interface reference](docs/interfaces.md) · [Tool matrix](docs/tool-contract-matrix.md) |
| Process API and older clients | [Process cutover](docs/process-cutover.md) |
| Browser and self-hosted deployment | [Browser deployment](docs/browser-self-hosted.md) |
| Upgrades and operations | [0.10.0 release notes](docs/release-notes-v0.10.0.md) · [Operations](docs/operations.md) |
| Source development, builds, and releases | [Development](docs/development/README.md) · [Releasing](docs/development/releasing.md) |

## License

Original AgenticGPT code and the main codebase use the MIT License.

`crates/agentic-apply-patch` contains code derived from OpenAI Codex under the Apache License 2.0. See its [LICENSE](crates/agentic-apply-patch/LICENSE) and [NOTICE](crates/agentic-apply-patch/NOTICE).
