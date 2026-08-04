# Configuration

Agentic GPT uses one local JSON configuration for Standalone, Local Unix MCP, and Hub-connected Agent modes. The default path is:

```text
~/.agentic_gpt/config.json
```

Start from:

```bash
agentic-gpt config init
agentic-gpt config show
```

[`config.example.json`](../config.example.json) is a strict v0.9 superset example. It is Standalone-first, contains no usable credentials, keeps all example downstream MCP servers disabled, and includes optional Hub fields for deployments that need them.

## Runtime-specific requirements

| Field group | Standalone | Local Unix MCP | Hub-connected Agent |
| --- | --- | --- | --- |
| Common identity/workspace/policy | Required | Required | Required |
| `tunnel` | Required | Ignored | Ignored |
| `hubUrl`, `hubTransport`, `agentSecret` | Used only for optional Hub reporting/ntfy relay | Ignored | Required |
| Public Hub/VPS | Not required | Not required | Required |
| Startup command | `run-as-standalone` | `run-as-local` | `run` / `run-as-room` |

The JSON type still contains Hub fields in every mode because one config can be moved between runtime shapes. Standalone and Local execution do not put Hub in the command path. In Standalone, Hub fields matter only when `tunnel.hubReporting.enabled` or Hub-backed `ntfy` confirmation is used.

## Standalone-first setup

```bash
agentic-gpt config init
agentic-gpt config set agentId laptop
agentic-gpt config set confirmationProvider freedesktop

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
agentic-gpt run-as-standalone --profile normal
```

Use `--profile room` for the Room surface.

## Top-level fields

| Field | Purpose |
| --- | --- |
| `agentId` | Stable local identity. It also determines the private runtime/socket path. |
| `displayName` | Human-readable machine label used in summaries/reporting. |
| `workspaceRoot` | Main writable workspace and location of `.agentic-gpt-audit.jsonl`. |
| `backupLimit` | Number of config backups retained by Agentic-managed writes. |
| `confirmationProvider` | Ordered local/remote confirmation channels. |
| `confirmationLanguage` | `en` or `zh-CN`. |
| `sandbox` | Optional bubblewrap configuration. |
| `mcpServers` | Downstream MCP servers bridged by `mcp.*`. |
| `pathPolicy` | Writable, read-only, and denied roots. |
| `policy` | Explicit allow / confirm / deny command rules. |
| `limits` | Process concurrency and total active Job capacity. |
| `skills` | Skill package/install limits and network policy. |
| `room` | Room timezone, diary boundary, and optional notebook root. |
| `tunnel` | Standalone tunnel-client source, secret reference, and optional reporting. |
| `hubUrl`, `hubTransport`, `agentSecret` | Centralized Hub connection or optional standalone Hub reporting/ntfy relay. |

Unknown top-level fields are preserved by load/write round trips. Nested strict objects such as `limits` reject removed v0.8 fields.

## Tunnel configuration

```json
{
  "tunnel": {
    "tunnelId": "tunnel_<assigned-id>",
    "apiKey": "file:/home/me/.agentic_gpt/secrets/tunnel-api-key",
    "client": {
      "version": null,
      "cacheDir": "~/.agentic_gpt/cache/tunnel-client",
      "autoDownload": true,
      "executable": null,
      "downloadUrl": null,
      "sha256": null
    },
    "hubReporting": {
      "enabled": false,
      "detail": "metadata"
    }
  }
}
```

`tunnelId` must be non-empty. `apiKey` accepts only:

- `file:/absolute/or/expanded/path`
- `env:VARIABLE_NAME`

Plaintext values are rejected. A referenced file may end with one LF or CRLF; the terminator is removed. Empty values and control characters fail startup.

Tunnel client source precedence:

1. `client.executable`: trusted local executable; optional `sha256` is checked on every start.
2. `client.downloadUrl` + required `sha256`: exact custom HTTPS archive.
3. Managed manifest/cache: the pinned official tunnel-client for the current platform.

`version: null` selects the embedded pin. `autoDownload: false` requires a verified cached artifact.

`hubReporting.enabled` is false by default. When enabled, the Hub connection is reporting-only and never accepts execution commands. `detail` is `metadata` or `full`; see [`standalone-runtime.md`](standalone-runtime.md) for the privacy boundary.

## Hub configuration

Hub mode requires:

```json
{
  "hubUrl": "https://agentic-gpt.example.com",
  "hubTransport": "websocket",
  "agentId": "laptop",
  "agentSecret": "<agent-secret>"
}
```

`hubTransport` accepts `websocket` or `sse`. `workerUrl` remains a read/set alias for `hubUrl`, but Agentic writes the canonical field.

Hub credentials are separate from the Standalone tunnel API key. Do not reuse them.

## Confirmation

Canonical form:

```json
{
  "confirmationProvider": {
    "channels": ["freedesktop"]
  },
  "confirmationLanguage": "zh-CN"
}
```

Channels:

- `freedesktop`: local desktop notification actions.
- `ntfy`: Hub-backed remote relay.

For Standalone without Hub reporting, prefer `freedesktop` only. If all configured channels are unavailable, a confirmation-required operation fails closed. Local denial or timeout never falls through to another channel.

The CLI accepts legacy labels such as `freedesktop-then-ntfy`; Agentic-managed writes serialize the canonical ordered array.

## Command and path policy

```bash
agentic-gpt config allow add bash
agentic-gpt config confirm add python -c
agentic-gpt config deny add ssh

agentic-gpt config path list
agentic-gpt config path write add ~/Projects
agentic-gpt config path readonly add /var/log
agentic-gpt config path deny add ~/.secrets
```

Configured allow rules may explicitly override builtin confirmation/deny rules. When several configured rules match, deny wins unless a more explicit configured allow override applies according to the runtime policy implementation.

`workspaceRoot` is always treated as writable. Denied roots override writable and read-only roots. Symlinks are resolved and must remain inside the effective policy boundary.

## Limits

```json
{
  "limits": {
    "maxConcurrentTasks": 2,
    "maxActiveJobs": "auto",
    "maxFileSearchContextLines": 5
  }
}
```

`maxActiveJobs` accepts a non-negative integer or `"auto"`. Auto resolves as `ceil(availableParallelism * 1.5)` clamped to 6–24. Process, skill, and MCP Jobs share this capacity.

`maxFileSearchContextLines` is the live maximum number of before/after lines that `file.search` returns for one match. It defaults to 5 and accepts an integer from 0 through 100. A search request may ask for more; the runtime clips it to the effective value and reports `requestedContextLines`, `effectiveContextLines`, `contextLinesClipped`, and a bounded warning. Negative or non-integer requests remain invalid.

v0.9 rejects `maxActiveSessions` and `sessionIdleTimeoutSecs`.

## Downstream MCP servers

```json
{
  "mcpServers": {
    "docs": {
      "enabled": false,
      "transport": "streamable-http",
      "url": "https://mcp.example.com/mcp"
    },
    "local-tool": {
      "enabled": false,
      "transport": "stdio",
      "url": "node /home/me/mcp/server.mjs"
    }
  }
}
```

Server ids are at most 64 bytes and use letters, digits, `.`, `_`, or `-`. `streamable-http` requires an absolute HTTP(S) URL. `stdio` requires a non-empty command. Keep examples disabled until their trust and confirmation policy are reviewed.

## Skills, Room, and sandbox

`skills` controls package sizes, redirects, timeouts, retry/deadline limits, install/download concurrency, and optional host allowlisting. The canonical block is top-level `skills`; legacy `room.skills` is read only when the top-level block is absent.

`room.timezone` controls Room date/time behavior. `room.diaryDayBoundaryHour` is 0–23. `room.notebookRoot` is optional.

`sandbox.enabled` activates bubblewrap. `requiredRuntimePaths` lists host paths made available inside the sandbox. Sandbox does not replace command policy, path policy, or confirmation.

## CLI-managed keys

`agentic-gpt config set` supports common scalar values including:

- `agentId`, `agentSecret`, `hubUrl`, `hubTransport`, `workspaceRoot`
- `confirmationProvider`, `confirmationLanguage`, `sandbox.enabled`
- `tunnel.tunnelId`, `tunnel.apiKey`
- all `tunnel.client.*` and `tunnel.hubReporting.*` fields
- `room.notebookRoot`, `room.timezone`, `room.diaryDayBoundaryHour`
- the documented `skills.*` scalar/list fields

Use `config allow/confirm/deny`, `config path`, and `config mcp` for structured policy/MCP changes. Complex JSON may also be edited directly while the process is stopped, followed by `agentic-gpt config show` and a smoke test.

## Live reload versus restart

Standalone and Local workers poll the config and atomically apply a valid live subset. Invalid candidates keep the last valid state.

| Configuration | Effect |
| --- | --- |
| `policy`, `pathPolicy`, `limits`, `mcpServers` | Live reload for new admissions/calls |
| Already-admitted Jobs and already-created downstream calls | Keep their original decision/config |
| `agentId`, `workspaceRoot`, profile | Restart required |
| `tunnel.*` client identity/source/secret | Restart required |
| `hubUrl`, `hubTransport`, `agentSecret`, reporting mode | Restart required for the related connection |
| Skill install concurrency/startup-owned settings | Restart required |

The Standalone supervisor emits `restart_required` when a startup identity field changes. Do not assume editing the file switched the existing child tree.

## Validation and inspection

```bash
agentic-gpt config show
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

`agent.info` exposes safe summaries rather than tunnel secrets, Hub secrets, full private paths, or MCP endpoints. The workspace audit file is:

```text
<workspaceRoot>/.agentic-gpt-audit.jsonl
```

For exact Standalone lifecycle and recovery behavior, see [`standalone-runtime.md`](standalone-runtime.md). For deployment checks, see [`operations.md`](operations.md).
