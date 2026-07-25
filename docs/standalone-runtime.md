# Standalone Tunnel runtime

Agentic has two ways to expose a local machine to ChatGPT. The existing Hub
runtime keeps the Hub in the command path. The standalone runtime puts the
official OpenAI `tunnel-client` in front of an Agentic stdio MCP worker; the
Hub is optional and, when enabled, is reporting-only.

## Runtime topology

```text
Hub mode:
ChatGPT -> HTTPS Hub -> WebSocket/SSE -> agentic-gpt -> local policy/services

Standalone mode:
Secure MCP Tunnel -> tunnel-client -> agentic-gpt stdio-worker
                                      \-> optional reporting-only Hub connection
```

The public entrypoint is `agentic-gpt run-as-standalone`. Agentic resolves and
verifies the tunnel client, runs its `doctor --json` preflight, creates the
worker command, supervises the tunnel/worker process tree, and keeps the
worker's stdout reserved for MCP framing. Do not start the hidden
`stdio-worker` command directly.

### Four public runtime mappings

| Command | Command transport | Capability profile | Hub connection |
| --- | --- | --- | --- |
| `agentic-gpt run` | Hub | Normal | command-capable |
| `agentic-gpt run-as-room` | Hub | Room | command-capable |
| `agentic-gpt run-as-standalone` | Tunnel stdio | Normal | disabled by default; reporting-only when enabled |
| `agentic-gpt run-as-standalone --profile room` | Tunnel stdio | Room | disabled by default; reporting-only when enabled |

Transport does not change local policy. Tunnel Normal and Tunnel Room use the
same policy boundaries as their corresponding local profiles; Room adds diary
and notebook, while Normal does not.

## Tunnel tool surfaces

Tunnel Normal advertises exactly 18 MCP tools:

```text
mcp.list, mcp.callTool
process.exec, process.batchExec, process.get, process.kill, process.list
skills.list, skills.read, skills.setActive, skills.install,
skills.install.get, skills.install.cancel, skills.run
tmux.sessions, tmux.panes, tmux.exec, tmux.pasteText
```

Tunnel Room advertises exactly 30 tools: the 18 Normal tools, `bootstrap` and
`bootstrap.read`, plus these ten Room memory tools:

```text
room.diary.append, room.diary.recent, room.diary.selectExact
room.notebook.append, room.notebook.current, room.notebook.recent,
room.notebook.remove, room.notebook.search, room.notebook.selectExact,
room.notebook.update
```

The standalone worker intentionally has no Tunnel `agentId` or
`confirmMethod` input fields. The worker supplies its configured local agent
identity internally; unexpected legacy fields are rejected. `bootstrap` is
Room-only. `process.exec` and `skills.run` return a stable managed-session
wrapper with `sessionId`, `completedInline`, `pollAfterMs`, and nested
`session`; the nested session retains its `agentId` for reporting and local
inspection. Batch execution returns one such wrapper per admitted element and
rejects the whole batch before admission when any element fails preflight,
policy, confirmation, or capacity checks.

Tunnel surfaces do not expose Hub aggregation or notification tools. They use
the same local policy, path-policy, confirmation, audit, and managed-session
lifecycle as Hub execution while keeping the Hub out of the command path.

## Hub MCP profiles

`agentic-gpt-hub serve` defaults to the backward-compatible `full` profile.
Choose the profile at startup; it is not hot-switched:

```text
agentic-gpt-hub serve --mcp-profile full
agentic-gpt-hub serve --mcp-profile coordinator
```

`AGENTIC_GPT_HUB_MCP_PROFILE` is the equivalent environment setting. The
coordinator profile exposes exactly these eight Hub-native tools:

- `hub.info`
- `agent.list`
- `hub.run.list`, `hub.run.get`
- `hub.session.list`, `hub.session.get`
- `user.notify.channels`, `user.notify.send`

Coordinator calls never dispatch an Agent command. Session queries read only
current/recent snapshots held for the active connection; retained run records
are the durable history. Hidden execution, session-control, tmux, downstream
MCP, skills, bootstrap, diary, and notebook tools are both absent from
`tools/list` and rejected by `tools/call`.

The full profile keeps the existing Hub execution surface, adds the
transport-neutral `bootstrap` and `bootstrap.read` names, retains
`room.bootstrap` and `room.bootstrap.read` compatibility aliases, and includes
the Hub-native aggregation/notification tools. OAuth discovery metadata
identifies the selected profile without advertising hidden tools.

## Configuration

The local configuration is normally `~/.agentic_gpt/config.json`. The tunnel
block is additive; old configurations without it continue to work for Hub
mode. The canonical skill block is top-level `skills`. A legacy `room.skills`
block is read only when top-level `skills` is absent; when both are present,
top-level values win and a later config write serializes the canonical block.

A minimal standalone configuration uses references, not secret values:

```json
{
  "tunnel": {
    "tunnelId": "tunnel_<assigned-id>",
    "apiKey": "file:/home/me/.config/agentic-gpt/tunnel-api-key",
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

The active-session limit accepts either the adaptive value or an explicit
integer:

```json
"limits": {
  "maxActiveSessions": "auto"
}
```

`auto` resolves at worker startup and after each valid live limits reload as
`clamp(ceil(availableParallelism * 1.5), 6, 24)`. Existing numeric values stay
explicit and are not migrated. Capacity rejection keeps the
`max_active_sessions_reached` code and includes bounded `active`, `requested`,
and `limit` details; batch admission remains atomic and all-or-reject.

While the standalone worker is running, edits to `policy`, `pathPolicy`, and
`limits` are polled and applied atomically to new admissions. Invalid config
versions keep the last valid live subset. Already admitted sessions continue
with their original admission decision and are not cancelled by a reload.
Startup-owned identity, workspace, tunnel/client, reporting connection, and
skill-install concurrency changes remain restart-required and are reported by
the supervisor.

`apiKey` accepts only `env:NAME` and `file:PATH`. The resolved value is
injected into the tunnel-client child environment as
`CONTROL_PLANE_API_KEY`; it is not placed in argv, logs, config summaries,
reports, generated runtime files, or backups. A file reference may end in one
LF or CRLF, which is removed. Empty values and plaintext references fail
startup.

Provision the secret with a secret manager or a separately protected file. For
example, configure the path without putting the key in shell history or
argv:

```bash
install -d -m 700 "$HOME/.config/agentic-gpt"
# Write the value using your secret manager; do not paste it into this command.
chmod 600 "$HOME/.config/agentic-gpt/tunnel-api-key"
agentic-gpt config set tunnel.tunnelId tunnel_<assigned-id>
agentic-gpt config set tunnel.apiKey file:"$HOME/.config/agentic-gpt/tunnel-api-key"
agentic-gpt config set tunnel.client.autoDownload true
```

The `env:NAME` form is suitable for a service manager's protected environment
or an injected secret, for example `env:AGENTIC_TUNNEL_API_KEY`; do not use a
literal `AGENTIC_TUNNEL_API_KEY=<secret> ...` command in a shell transcript.

Supported `config set` keys include:

- `tunnel.tunnelId`, `tunnel.apiKey`.
- `tunnel.client.version`, `tunnel.client.cacheDir`,
  `tunnel.client.autoDownload`, `tunnel.client.executable`,
  `tunnel.client.downloadUrl`, `tunnel.client.sha256`.
- `tunnel.hubReporting.enabled`, `tunnel.hubReporting.detail`.

The tunnel identity, secret reference, client source/version/hash/cache, and
CLI profile are startup identity. Editing one while the supervisor is running
logs `restart_required`; it does not switch the existing child tree.

## Tunnel client trust and source selection

With no executable override, the release manifest pins OpenAI tunnel-client
`v0.0.10` for the supported Linux targets:

| Platform | Asset | Archive SHA-256 |
| --- | --- | --- |
| `linux-amd64` | `tunnel-client-v0.0.10-linux-amd64.zip` | `b9e0388a343f2d7adeff3992f411a0bd3d916a64bc56534aac5fd15ac1b20cd5` |
| `linux-arm64` | `tunnel-client-v0.0.10-linux-arm64.zip` | `b842a9b2352eebd80514cf01a1fbb1c0d400a7d24a4015e85a7ea5f1aeaa5b30` |

`version: null` selects the embedded pin. An explicit version must be in the
manifest. Unsupported platforms fail before network access. Agentic verifies
the archive before extraction, accepts only one regular file named
`tunnel-client`, rejects traversal/symlink/device/duplicate layouts, and
installs through a private cache and atomic replacement.

Source precedence is:

1. `client.executable`: use a local trusted executable; an optional `sha256`
   is checked on every startup.
2. `client.downloadUrl` plus `client.sha256`: use an exact HTTPS archive URL
   with a required archive digest and bounded HTTPS redirects.
3. Managed manifest/cache: use the pinned URL and digest. `autoDownload: false`
   requires a verified cache artifact to already exist.

Managed identities include version, platform, and archive digest, so custom and
official artifacts cannot collide. The default cache is
`~/.agentic_gpt/cache/tunnel-client`.

## Optional Hub reporting

Reporting is disabled by default and is independent of Tunnel command
execution. Enable it only when the local config already has the Hub identity
needed for an Agent connection (`hubUrl`, `hubTransport`, `agentId`, and
`agentSecret`):

```bash
agentic-gpt config set tunnel.hubReporting.enabled true
agentic-gpt config set tunnel.hubReporting.detail metadata
```

The reporting connection identifies itself as `reporting-only`. It can send
hello/heartbeat, direct-run lifecycle events, session snapshots, and the
existing confirmation traffic, but it never accepts Hub execution envelopes.
Hub requests reject reporting-only agents before creating a run. Reporting
disconnects, queue drops, and Hub unavailability never delay or change the
local MCP result.

`metadata` records tool/source/profile/status/timestamps/duration, identifiers,
exit code, and bounded failure reason. It omits arguments, results, program
argv, working directories, and stdout/stderr. `full` additionally stores
bounded JSON arguments/results and bounded existing session snapshots; an
oversized value becomes a byte-count/SHA-256 truncation record rather than a
partial JSON fragment. Direct-run records remain in Hub storage for 24 hours.

The worker also writes bounded lifecycle records to stderr for each tool call
and managed process terminal event. These records contain the run/tool/profile
and status, with duration and safe 12-hex run/session identifiers when
available; inline terminal calls emit one final record, while calls that
return active emit one response record and one later terminal record. They
never contain arguments, results, paths, secrets, or process output. Reporting
connection transitions are logged separately as connected/disconnected with
the selected transport.

## Health, logs, restart, and recovery

For agent id `laptop`, the supervisor uses the private runtime directory:

```text
~/.agentic_gpt/runtime/tunnel/laptop/
├── health.url          # transient loopback readiness URL
├── tunnel-client.log   # structured child log, retained for diagnostics
└── tunnel-client.pid   # transient child pid marker
```

The supervisor runs `doctor --json` before the first child, waits up to 45
seconds for loopback readiness, and forwards child output to Agentic stderr
with component prefixes, secret redaction, and the child's known INFO/WARN/
ERROR severity. Unknown child stdout is informational and unknown child
stderr is warning-level. Under journald Agentic omits its own inner timestamp;
foreground logs retain one self-contained timestamp. The health URL and pid
marker are removed on normal or failed cleanup; the structured log is
retained.

Unexpected child exits and readiness failures use at most five retries with
1/2/4/8/16-second delays. Sixty seconds continuously ready resets the failure
counter. Configuration/reference errors, unsupported platforms, missing local
executables, checksum failures, and tunnel authentication/authorization
failures are permanent startup failures. SIGINT/SIGTERM stops the tunnel
process group and worker, then uses a bounded kill fallback.

Recovery checklist:

1. Read the Agentic stderr diagnostic and the retained
   `tunnel-client.log`; never print the API key while inspecting logs.
2. Check `agentic-gpt config show` for the reference and non-secret source
   summary, not the resolved value.
3. For a managed client, check the cache path and leave
   `autoDownload: true` unless offline provisioning is intentional.
4. For an override, verify the file is a regular executable and that the
   configured optional digest matches the intended binary.
5. Check the tunnel/control-plane status with the provider. Restart after
   changing identity, profile, or client distribution settings.
6. If reporting is the only failure, disable
   `tunnel.hubReporting.enabled` temporarily; local Tunnel execution remains
   independent.

For Hub mode, use `GET /v1/info`, `GET /v1/agents`, `hub.info`, and
`agent.list` for safe diagnostics. Use `hub.run.list`/`hub.run.get` for retained
run history and `hub.session.list`/`hub.session.get` for current snapshots.

## Existing centralized Hub mode

Standalone mode is additive. Existing centralized deployments continue to use:

```bash
agentic-gpt-hub init
agentic-gpt-hub agent add --agent-id laptop --display-name my-laptop --secret '<agent-secret>'
agentic-gpt run
```

The full Hub MCP profile remains the default. Use coordinator only on a Hub
instance intended for status/history/notification access:

```text
agentic-gpt-hub serve --mcp-profile coordinator
```

Keep the Hub behind HTTPS when it is reachable from ChatGPT. The existing
Actions routes and WebSocket/SSE agent transports remain available; the
standalone reporting connection is an additional reporting-only mode, not a
replacement for command-capable Hub agents.

## Verification map

Automated checks cover config/reference validation, exact worker tool lists,
secret argv/environment separation, trusted asset/cache behavior, supervisor
restart and process-tree cleanup, reporting privacy/idempotency, and the full
versus coordinator MCP surfaces. The repository's multi-target release script
is [`scripts/dist-linux.sh`](../scripts/dist-linux.sh); it builds both Linux
targets when `cross` and the corresponding toolchains are installed.

`crates/agentic-gpt/tests/standalone_supervisor.rs` launches the actual Agentic
supervisor and hidden stdio worker for a Normal-profile initialize/list/call
smoke. The in-process tests cover the corresponding Room profile. Together
they verify that stdout remains MCP-only and that the compact Normal/Room
surfaces are callable. These checks are still distinct from a real Secure MCP
Tunnel control-plane call: the latter must invoke an actual external connector
and return a local Agentic tool result, not merely pass `/healthz`, `doctor`, or
a local fake handoff. Record external credentials/environment prerequisites
separately from repository tests.
