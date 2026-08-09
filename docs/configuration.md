# Configuration

Agentic GPT uses one local JSON configuration for Standalone, Local Unix MCP, and Hub-connected Agent modes. The default path is:

```text
~/.agentic_gpt/config.json
```

The durable file is a sparse Config v2 projection. It always contains the authoritative top-level
`mode` (`standalone`, `hub`, or `local`) and `profile` (`normal` or `room`); omitted values are
reconstructed from effective defaults. `config show` displays the fully materialized effective
configuration, while Agentic-managed writes keep the file sparse.

Start from:

```bash
agentic-gpt config init
agentic-gpt config show
```

[`config.example.json`](../config.example.json) is a sparse Config v2 example. It is
Standalone-first, contains no usable credentials, keeps all example downstream MCP servers
disabled, and includes only meaningful Hub fields for deployments that need them.

## Fullscreen initializer behavior

`agentic-gpt config init` opens the keyboard-driven fullscreen setup UI only when stdin, stdout,
and stderr are all terminals. A pipe or redirected stream is not an implicit fallback: bare
non-TTY init returns a localized, actionable error and writes nothing. Use
`config init --non-interactive` for scripts, CI, redirected output, or any other automation. The
default mode is `standalone` and the default profile is `normal`.

Mode and profile are independent choices:

- `--mode standalone|hub|local` selects the runtime connection and configuration shape.
- `--profile normal|room` selects the capability/tool surface. Normal exposes 24 tools and Room
  exposes 36 tools; a profile does not turn a Local runtime into a Hub runtime.

For deterministic scripts, use the exact CLI grammar below and provide values that must not be
placeholders:

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

With no values supplied, the non-interactive Standalone + Normal template uses safe placeholders
such as `tunnel_replace-me` and a `file:` reference under the Agentic home. It reports pending
actions to replace the tunnel ID and provision the referenced secret; it does not create or
provision secret material automatically. Hub defaults similarly report pending Hub URL and
agent-secret actions when those values are omitted. `--agent-secret` is visible to shell history
and local process inspection, so hidden interactive input is preferred. A `file:` or `env:`
reference avoids putting a tunnel secret in the command line; plaintext tunnel API keys are
rejected.

The fullscreen flow is Basic → Connection (except for Local) → Optional settings → Review →
Completion. The command-line flags seed editable fields in interactive mode; they do not lock the
values or skip the pages. Identity/display name, workspace/path policy, confirmation/language,
limits, and sandbox are always available. Room settings are offered only for the Room profile.
Tunnel-client overrides and Hub reporting are offered only for Standalone mode. Hub and Local modes
do not show those tunnel sections. Optional sections can be revisited, and selecting none keeps
the template defaults.

The UI uses keyboard navigation: Tab/Shift+Tab and the arrow keys move focus, Enter edits or
activates the focused item, Esc backs out (and is a no-op on the root Basic page), and Ctrl+C
cancels the setup. Editing Esc only leaves editing; it does not cancel the setup. Review is
redacted, can jump back to Basic, Connection, or an optional section, and does not write config,
backup, or secret files until final confirmation. This feature documents the fullscreen keyboard
flow only; mouse, inline, dashboard, and Windows behavior are outside its contract.

`config init --language auto|zh-CN|en` selects the CLI interface language. With `auto`, locale
variables are checked in this order: `LC_ALL`, then `LC_MESSAGES`, then `LANG`, then English.
An explicit `zh-CN` or `en` wins over the environment. This interface choice is separate from
the persisted `confirmationLanguage`, which controls the language of confirmation prompts sent
by the runtime and can be set through the optional configuration section or `config set`.

The first-run setup scope deliberately excludes MCP server collections and command-policy
collections. Configure those after initialization with `config mcp` and `config allow`,
`config confirm`, or `config deny` (and use `config path` for path roots).

## Runtime-specific requirements

| Field group | Standalone | Local Unix MCP | Hub-connected Agent |
| --- | --- | --- | --- |
| Common identity/workspace/policy | Required | Required | Required |
| `tunnel` | Required | Ignored | Ignored |
| `hub` (`url`, `transport`, `agentSecret`) | Used only for optional Hub reporting/ntfy relay | Ignored | Required |
| Public Hub/VPS | Not required | Not required | Required |
| Startup command | `agentic-gpt run` | `agentic-gpt run` | `agentic-gpt run` |

The JSON type still contains a nested `hub` section in every mode because one config can be moved between runtime shapes. Standalone and Local execution do not put Hub in the command path. In Standalone, Hub fields matter only when `tunnel.hubReporting.enabled` or Hub-backed `ntfy` confirmation is used. Inactive sections are preserved when explicitly configured.

## Standalone-first setup

```bash
agentic-gpt config init
agentic-gpt config set agentId laptop
agentic-gpt config set confirmationProvider.channels '["freedesktop"]'

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
agentic-gpt run
```

Set `profile` to `room` for the Room surface (for example, `agentic-gpt config set profile room`).

## Top-level fields

| Field | Purpose |
| --- | --- |
| `mode` | Authoritative runtime dispatch: `standalone`, `hub`, or `local`. |
| `profile` | Authoritative capability surface: `normal` or `room`. |
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
| `hub` | Centralized Hub connection or optional standalone Hub reporting/ntfy relay. |

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
  "hub": {
    "url": "https://agentic-gpt.example.com",
    "transport": "websocket",
    "agentSecret": "<agent-secret>"
  },
  "agentId": "laptop"
}
```

`hub.transport` accepts `websocket` or `sse`. Legacy top-level `hubUrl`, `hubTransport`, `workerUrl`, and `agentSecret` are recognized only by the explicit `config import` flow; normal v2 load rejects them.

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

`maxConcurrentTasks` limits how many child Process Jobs from one `process.batch` call may run at the same time. All children are admitted together; excess children remain `queued`, so this limit does not prevent the batch call from returning after its bounded `waitSeconds`. Values below 1 have an effective minimum of 1.

`maxActiveJobs` accepts a non-negative integer or `"auto"`. Auto resolves as `ceil(availableParallelism * 1.5)` clamped to 6–24. Process, skill, and MCP Jobs share this capacity, including queued batch children.

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

`config set` is a controlled registry, not a general JSONPath editor. List the registry in the
current locale with:

```text
agentic-gpt config keys [--section <SECTION>] [--json]
```

The text form groups keys by `runtime`, `identity`, `hub`, `confirmation`, `sandbox`, `limits`, `skills`,
`room`, and `tunnel`; `--section` filters to one of those names. `--json` returns machine-readable
metadata including the value type, nullability, example, bilingual descriptions, and aliases. Only keys in this registry are accepted by
`config set`; structured policy and MCP collections use their dedicated commands.

The value is one shell argument after the registered key. JSON list values therefore need shell
quoting, and `room.notebookRoot` is nullable: use the literal JSON value `null` to clear it.

```bash
agentic-gpt config set sandbox.requiredRuntimePaths '["/usr","/opt/runtime"]'
agentic-gpt config set skills.allowedHosts '["skills.example.com"]'
agentic-gpt config set room.notebookRoot null
```

The registry includes common scalar values such as:

- `mode`, `profile`, `agentId`, `hub.url`, `hub.transport`, `hub.agentSecret`, `workspaceRoot`
- `confirmationProvider.channels`, `confirmationLanguage`, `sandbox.enabled`
- `tunnel.tunnelId`, `tunnel.apiKey`
- all `tunnel.client.*` and `tunnel.hubReporting.*` fields
- `room.notebookRoot`, `room.timezone`, `room.diaryDayBoundaryHour`
- the documented `skills.*` scalar/list fields

Use `config allow/confirm/deny`, `config path`, and `config mcp` for structured policy/MCP changes. Complex JSON may also be edited directly while the process is stopped, followed by `agentic-gpt config show` and a smoke test.

## Secret files and transactional writes

Tunnel secrets must be referenced as `file:PATH` or `env:NAME`; the `file:` path may be absolute
or use the usual home expansion, while an environment name must be a valid shell variable name.
The fullscreen setup's optional file writer creates the parent directory with mode `0700` and the
secret file with mode `0600`, writes through a temporary file, and atomically renames it into
place.
If the subsequent config write fails, it removes a newly-created secret or restores the prior
secret bytes and mode. Escape, Ctrl-C, a prompt error, or a final refusal happens before the
transaction is committed, so no config or secret file is created or modified. Summaries,
diagnostics, and errors never print secret values.

## Explicit import migration

Normal `Config::load()` is strict v2 and does not infer missing selectors or silently accept the old
Hub shape. Use `agentic-gpt config import --config PATH [SOURCE]` to migrate old or external JSON
(`--config` may be omitted for the default config path). If SOURCE is omitted, the selected
`--config` path is imported. The flow seeds the normal interactive Config
Init TUI, carries forward recognized fields without editors (including MCP servers, policy, path
policy, limits, inactive hub/tunnel/room data, and safe unknown flattened fields), reports fields
that cannot be imported, and writes through the normal backup/secret transaction.

## Live reload versus restart

Standalone and Local workers poll the config and atomically apply a valid live subset. Invalid candidates keep the last valid state.

| Configuration | Effect |
| --- | --- |
| `policy`, `pathPolicy`, `limits`, `mcpServers` | Live reload for new admissions/calls |
| Already-admitted Jobs and already-created downstream calls | Keep their original decision/config |
| `mode`, `profile`, `agentId`, `workspaceRoot` | Restart required |
| `tunnel.*` client identity/source/secret | Restart required |
| `hub`, reporting mode | Restart required for the related connection |
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
