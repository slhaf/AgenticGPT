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
- `--profile normal|room` selects the default toolset preset. The normal preset enables every
  namespace except `room`; the room preset enables every namespace. A profile preset does not
  by itself fix the final runtime surface or count, because an explicit `toolsets.enabled`
  selection is authoritative.

The available namespaces are `agent`, `file`, `mcp`, `process`, `job`, `skills`, `tmux`,
`browser`, and `room`. The logical `room` namespace includes Room bootstrap (`bootstrap` and
`bootstrap.read`) and every `room.*` tool. Selection filters the pre-existing advertised Normal/Room names only;
it never exposes dispatch-only aliases.

Pin a selection with the following exact commands:

```bash
agentic-gpt config toolset ls
agentic-gpt config toolset enable <namespace>
agentic-gpt config toolset disable <namespace>
```
`ls` prints all namespaces, their enabled/disabled state, and a concise description of the tools
they contain. Successful `enable` and `disable` mutations print an explicit confirmation.

The same `toolsets.enabled` array may be edited directly in JSON. Valid toolset changes are
hot-reloaded while the worker is running and take effect for subsequent tool discovery and
calls; restarting is not required. An invalid candidate keeps the last valid live selection.
Room bootstrap, diary, and notebook authorization follows the live `room` namespace, not the
startup profile. A direct Room command while that namespace is disabled returns
`room_toolset_required`.

For example, an explicit normal selection is represented as:

```json
{
  "profile": "normal",
  "toolsets": {
    "enabled": ["agent", "file", "mcp", "process", "job", "skills", "tmux", "browser"]
  }
}
```

This later explicit selection remains authoritative if the profile is changed.

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
limits, sandbox, and the optional Toolsets section are available. Toolsets starts from the
profile preset; once explicitly edited, its namespace selection is authoritative. Room settings
are offered only for the Room profile. Tunnel-client overrides and Hub reporting are offered only
for Standalone mode. Hub and Local modes do not show those tunnel sections. Optional sections can
be revisited, and selecting none keeps the template defaults.


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
| `httpMcp` | Optional, active only in Standalone | Ignored | Ignored |
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

Set `profile` to `room` for the all-namespace Room preset (for example, `agentic-gpt config set
profile room`). An explicit `toolsets.enabled` selection remains authoritative.

## Top-level fields

| Field | Purpose |
| --- | --- |
| `mode` | Authoritative runtime dispatch: `standalone`, `hub`, or `local`. |
| `profile` | Default capability/toolset preset: `normal` or `room`. |
| `toolsets` | Enabled tool namespaces; an explicit `enabled` list overrides the profile preset. |
| `agentId` | Stable local identity. It also determines the private runtime/socket path and per-agent durable state root under `~/.agentic_gpt/state/agent/<agentId>/`. |
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
| `room` | Room repository root, timezone, diary boundary, maintenance mode, and auto-push policy. |
| `tunnel` | Standalone tunnel-client source, secret reference, and optional reporting. |
| `browser` | Optional advanced explicit Browser runtime override; ordinary runtime discovery/provisioning is otherwise automatic. |
| `hub` | Centralized Hub connection or optional standalone Hub reporting/ntfy relay. |
| `httpMcp` | Optional Standalone worker-owned inbound Streamable HTTP MCP endpoint. |

## Standalone HTTP MCP endpoint

Standalone can expose an optional inbound MCP endpoint from the hidden worker:

```text
http://<host>:<port>/mcp
```

It is disabled by default and is independent of both the tunnel transport and Hub. The
configuration shape and defaults are:

```json
{
  "httpMcp": {
    "enabled": false,
    "host": "127.0.0.1",
    "port": 8765,
    "publicUrl": null,
    "bearerToken": "",
    "allowHosts": ["localhost", "127.0.0.1", "::1"]
  }
}
```

The path is fixed at `/mcp`; it cannot be changed through configuration. When
`publicUrl` is absent, the endpoint accepts the configured
`Authorization: Bearer ...` credential for direct local use. When `publicUrl`
is present, it additionally exposes the standalone ChatGPT connector OAuth
contract:

- `GET /.well-known/oauth-protected-resource/mcp` is the canonical
  path-specific protected-resource metadata; `GET
  /.well-known/oauth-protected-resource` is a root-compatible alias.
- `GET /.well-known/oauth-authorization-server` and `GET
  /.well-known/openid-configuration` are authorization-server/OpenID aliases.
- `GET|POST /oauth/authorize` and `POST /oauth/token` implement the
  authorization-code flow with the single `agentic:mcp` scope.

The configured value must be a non-empty HTTPS origin with no userinfo, path
other than empty or `/`, query, or fragment. A trailing slash is normalized.
It is an advertised external origin, not a routing or proxy override: `host`
and `port` remain the local bind coordinates, and the origin may remain
loopback/private. `publicUrl` is non-secret and is never masked in `config
show`, Review, diagnostics, or the TUI; `bearerToken` remains a secret
reference and its resolved value is never exposed.

The connector accepts only ChatGPT callback URIs in these exact families:
`https://chatgpt.com/connector/oauth/<suffix>` or
`https://chatgpt.com/connector_platform_oauth_redirect`. It accepts only
`agentic:mcp`; there are no refresh tokens, `offline_access`, dynamic client
registration, generic registration, or arbitrary redirects. Authorization codes
and access tokens are opaque, listener-local in-memory records with expiry,
one-use code consumption, and revocation on listener replacement or bearer
content rotation. The standalone authorization page and tool/profile surface
are standalone semantics; they do not use Hub's `Hub API key`, profile, or
routing contract.

OAuth routes and `/mcp` share listener Host protection. Missing or malformed
Host is rejected, and a disallowed authority is forbidden; an allowlist entry
without a port matches any port, while a port-bearing entry matches exactly.
`null` and exactly `["*"]` remain explicit allow-all values. If `Origin` is
present it must exactly match the configured `publicUrl`; absent Origin remains
valid for server-to-server requests, and no permissive CORS is added. With no
`publicUrl`, failed direct bearer authentication keeps a plain Bearer
challenge; with one configured, the MCP challenge points to the
path-specific protected-resource metadata URL.

The rmcp transport is stateful Streamable HTTP/SSE: clients must initialize a
session, and a listener rebind or disable closes its sessions so the client
must initialize again. Direct bearer content rotation updates authentication
in place and preserves existing sessions while atomically revoking OAuth
records. Invalid candidates retain the last-good listener and its state.

`allowHosts` is the DNS-rebinding protection applied by the HTTP MCP transport:

- The default list permits only `localhost`, `127.0.0.1`, and `::1`.
- A non-empty list of valid host/authority entries restricts requests to those entries.
- `null` or exactly `["*"]` explicitly allows every Host value. The wildcard cannot be
  mixed with another entry.
- An empty array is rejected; it is not an implicit allow-all value.
- Malformed authority entries, wildcard mixtures, and other invalid values are rejected.

For a non-loopback listener, choose an explicit authority allowlist or deliberately use
one of the two full-allow values. `host` must be a non-empty bindable host without
whitespace or control characters, and `port` must be in `1..=65535`.

`bearerToken` is a secret reference, never a literal credential. It must be empty only
when the endpoint is disabled; an enabled endpoint requires one of:

- `file:/absolute/path` (one trailing LF or CRLF is removed);
- `env:VARIABLE_NAME`.

The referenced value must be available, non-empty, and free of control characters.
Configuration validation rejects plaintext, malformed references, and an enabled
endpoint without a reference. Resolved token content is kept in memory only. Config
show, review, diagnostics, logs, and `agent.info` redact both the reference and the
resolved value; provisioning and rotating the underlying file/environment remains an
external secret-management operation.

### Configure HTTP MCP with the CLI

The controlled registry is exposed under the `http-mcp` section:

```bash
agentic-gpt config keys --section http-mcp
agentic-gpt config set httpMcp.bearerToken env:AGENTIC_HTTP_MCP_TOKEN
agentic-gpt config set httpMcp.host 127.0.0.1
agentic-gpt config set httpMcp.port 8765
agentic-gpt config set httpMcp.publicUrl https://mcp.example.com
agentic-gpt config set httpMcp.allowHosts '["mcp.example.com"]'
agentic-gpt config set httpMcp.enabled true
```

To explicitly disable Host filtering, use either JSON `null` or `["*"]`; to
make the OAuth routes fail closed and return to direct-bearer local use,
clear the optional origin:

```bash
agentic-gpt config set httpMcp.allowHosts null
agentic-gpt config set httpMcp.allowHosts '["*"]'
agentic-gpt config set httpMcp.publicUrl null
```

`allowHosts` is a JSON array or `null`, not a comma-delimited string.
`publicUrl` must be an HTTPS origin; `config set` validates the complete
candidate before writing. A rejected value leaves both the configuration and
its backup unchanged. `config mcp` remains reserved for the downstream
`mcpServers` registry and does not configure this inbound listener.

For deterministic provisioning, `config init --non-interactive` accepts all endpoint
fields as flags:

```bash
agentic-gpt config init --non-interactive \
  --mode standalone \
  --http-mcp-enabled true \
  --http-mcp-host 127.0.0.1 \
  --http-mcp-port 8765 \
  --http-mcp-public-url https://mcp.example.com \
  --http-mcp-bearer-token env:AGENTIC_HTTP_MCP_TOKEN \
  --http-mcp-allow-hosts '["mcp.example.com"]'
```

The flags must still obey the HTTPS-origin, secret-reference, and allow-host
rules; enabling without a token reference fails before any config or backup is
written. In interactive `config init`, the same flags seed editable Connection
fields. The HTTP MCP enabled toggle, bind host/port, non-secret public-origin
editor, secret-reference editor, and JSON array/`null` allow-host editor can be
changed before Review; an empty public-origin value clears it. `config import
--config PATH [SOURCE]` recognizes an existing `httpMcp` object, seeds these
fields into the same interactive editor, and lets the operator correct or
disable it before the single final commit. Review shows the bearer reference as
`[REDACTED]` but displays `publicUrl`; cancellation or validation failure writes
nothing.

Path-safe `agentId` values map directly to the private state directory name. Wider legacy Hub identities remain supported and use a stable hashed directory key instead of becoming a filesystem path component.

Unknown top-level fields are preserved by load/write round trips. Nested strict objects such as `limits` reject removed v0.8 fields.

### Browser runtime override

Browser is not enabled by configuring a runtime; `toolsets.enabled` remains authoritative. With no
`browser` section (or with `browser: {}`), normal runtime discovery is unchanged. The explicit
descriptor is an advanced override for development, unusual deployments, or recovery; it is not
intended to be the normal managed-runtime installation path. `codexCliPath` is optional because the
official Browser launcher only exports `CODEX_CLI_PATH` when one is available. Other scalar values
are required when `runtime` is present, and every configured path must be absolute;
`nodeModuleDirs` defaults to an empty list:

```json
{
  "browser": {
    "runtime": {
      "appVersion": "<official-runtime-version>",
      "channel": "<runtime-channel>",
      "nodeReplPath": "/absolute/path/to/node_repl",
      "nodePath": "/absolute/path/to/node",
      "browserClientPath": "/absolute/path/to/browser-client.mjs",
      "browserServicePath": "/absolute/path/to/browser-service.mjs",
      "codexHome": "/absolute/path/to/runtime-home",
      "codexCliPath": "/optional/absolute/path/to/codex-or-compatible-cli",
      "nodeModuleDirs": ["/absolute/path/to/node_modules"]
    }
  }
}
```

The explicit source is selected at process startup and is authoritative: an invalid descriptor
closes Browser capability rather than falling back to Desktop discovery, while Agentic startup
continues. Changing it requires a process restart. `docsRoot` and `trustedCodePaths` are derived
internally and are not configuration fields. No installer, downloader, or runtime cache is managed
by this setting.

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
      "url": "https://mcp.example.com/mcp",
      "auth": {
        "type": "bearer",
        "token": "replace-me"
      }
    },
    "local-tool": {
      "enabled": false,
      "transport": "stdio",
      "url": "node /home/me/mcp/server.mjs"
    }
  }
}
```

Server ids are at most 64 bytes and use letters, digits, `.`, `_`, or `-`. `streamable-http` requires an absolute HTTP(S) URL and may use `auth: {"type":"bearer","token":"..."}`; the runtime sends the token as `Authorization: Bearer <token>`. Bearer auth is rejected for `stdio`, which requires a non-empty command. The TUI masks Bearer tokens and redacts them from its final JSON preview. Keep examples disabled until their trust and confirmation policy are reviewed.

## Skills, Room, and sandbox

`skills` controls package sizes, redirects, timeouts, retry/deadline limits, install/download concurrency, and optional host allowlisting. The canonical block is top-level `skills`; legacy `room.skills` is read only when the top-level block is absent.

`room.timezone` is retained Room metadata; V2 reads use repository paths rather than the
legacy JSONL date partitioning. `room.diaryDayBoundaryHour` is 0–23 and controls the logical
date written into a newly bootstrapped Daily scaffold.
`room.repositoryRoot` is optional and defaults to `<workspaceRoot>/room`. The nested
`room.maintenance.mode` is `local` or `workflow` and defaults to `local`; `room.maintenance.autoPush`
defaults to `false`. The standalone Room toolset exposes semantic reads plus
`room.maintenance.status` and `room.maintenance.submit`; all mutations use the latter.
Legacy JSONL Room commands remain only in the protocol and Hub HTTP/MCP compatibility surface
for the separate Hub parity workstream and are not advertised or executed by the Agent.

`sandbox.enabled` activates bubblewrap. `requiredRuntimePaths` lists host paths made available inside the sandbox. Sandbox does not replace command policy, path policy, or confirmation.

## CLI-managed keys

`config set` is a controlled registry, not a general JSONPath editor. List the registry in the
current locale with:

```text
agentic-gpt config keys [--section <SECTION>] [--json]
```

The text form groups keys by `runtime`, `identity`, `hub`, `confirmation`, `sandbox`, `limits`, `skills`,
`room`, `tunnel`, and `http-mcp`; `--section` filters to one of those names. `--json` returns machine-readable
metadata including the value type, nullability, example, bilingual descriptions, and aliases. Only keys in this
registry are accepted by `config set`; structured policy and MCP collections use their dedicated commands.

The value is one shell argument after the registered key. JSON list values therefore need shell
quoting. `room.repositoryRoot` is nullable: use the literal JSON value `null` to clear it and
return to the workspace default.

```bash
agentic-gpt config set sandbox.requiredRuntimePaths '["/usr","/opt/runtime"]'
agentic-gpt config set skills.allowedHosts '["skills.example.com"]'
agentic-gpt config set room.repositoryRoot null
agentic-gpt config set room.maintenance.mode local
agentic-gpt config set room.maintenance.autoPush false
```

The registry includes common scalar values such as:

- `mode`, `profile`, `agentId`, `hub.url`, `hub.transport`, `hub.agentSecret`, `workspaceRoot`
- `confirmationProvider.channels`, `confirmationLanguage`, `sandbox.enabled`
- `tunnel.tunnelId`, `tunnel.apiKey`
- all `tunnel.client.*` and `tunnel.hubReporting.*` fields
- `room.repositoryRoot`, `room.timezone`, `room.diaryDayBoundaryHour`
- `room.maintenance.mode`, `room.maintenance.autoPush`
- the documented `skills.*` scalar/list fields
- `httpMcp.enabled`, `httpMcp.host`, `httpMcp.port`, `httpMcp.publicUrl`, `httpMcp.bearerToken`, `httpMcp.allowHosts`

Use `config allow/confirm/deny`, `config path`, and `config mcp` for structured policy/MCP changes.
The exact `config toolset` commands above manage namespace selection. Complex JSON, including
`toolsets.enabled`, may also be edited directly; a valid edit hot-reloads without restarting the
worker, while an invalid candidate leaves the last valid live state in place. Follow with
`agentic-gpt config show` and a smoke test.

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
| `policy`, `pathPolicy`, `limits`, `mcpServers`, `toolsets.enabled` | Live reload for new admissions/calls and tool discovery |
| `httpMcp.enabled`, `host`, `port`, `publicUrl`, `allowHosts` | Standalone live rebind; listener identity changes close stateful sessions and discard listener-local OAuth state, so clients must initialize again |
| `httpMcp.bearerToken` reference or referenced content | Standalone live authentication update without rebind; existing sessions remain valid while the resolved credential is available |
| Already-admitted Jobs and already-created downstream calls | Keep their original decision/config |
| `mode`, `profile`, `agentId`, `workspaceRoot` | Restart required |
| `room.*` repository, timezone, diary-boundary, and maintenance settings | Restart required; `toolsets.enabled` may expose Room live using the current live Room settings |
| `tunnel.*` client identity/source/secret | Restart required |
| `hub`, reporting mode | Restart required for the related connection |
| Skill install concurrency/startup-owned settings | Restart required |

An unavailable HTTP MCP credential fails closed: the endpoint stops accepting requests and stops
listening until the reference resolves again. A syntactically or semantically invalid candidate is
rejected by the watcher and leaves the last-good live configuration in place. A bind conflict is
also kept isolated from tunnel and Unix execution; fix the endpoint configuration and let the watcher
retry. These outcomes never print the reference or token.

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
