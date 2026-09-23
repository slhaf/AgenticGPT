# Standalone and local MCP runtimes

Agentic has three runtime shapes. Standalone is the recommended direct
deployment: it puts the official OpenAI `tunnel-client` in front of one Agentic
worker and does not require Hub in the command path. Local integration serves
the same Agent surface over a private Unix socket without tunnel credentials.
Hub mode remains the optional centralized topology.

## Runtime topology

```text
Standalone mode (recommended):
Secure MCP Tunnel -> tunnel-client -> agentic-gpt worker
                                      |-> stdio MCP ingress
                                      |-> owner-only Unix MCP ingress
                                      \-> optional HTTP MCP ingress: http://<host>:<port>/mcp
                                          (worker-owned, enabled by httpMcp)
                                      \-> optional reporting-only Hub connection

Local integration mode:
local rmcp CLI/client -> owner-only Unix socket -> agentic-gpt worker

Hub mode (centralized):
ChatGPT -> HTTPS Hub -> WebSocket/SSE -> agentic-gpt -> local policy/services
```

The tunnel runtime is started with `agentic-gpt run` when the config has `mode=standalone`.
Agentic resolves and
verifies the tunnel client, runs its `doctor --json` preflight, creates the
worker command, supervises the tunnel/worker process tree, and keeps the
worker's stdout reserved for MCP framing. That same hidden worker owns the
owner-only Unix MCP socket and, when `httpMcp.enabled` is true, the optional
HTTP MCP listener. Do not start the hidden `stdio-worker` command directly.

The standalone HTTP endpoint is fixed at `/mcp` and supports the configured
direct bearer plus an optional standalone ChatGPT connector OAuth contract.
Without `httpMcp.publicUrl`, direct bearer use remains valid and OAuth routes
fail closed; with a valid HTTPS `publicUrl`, the listener advertises
path-specific protected-resource metadata at
`/.well-known/oauth-protected-resource/mcp`, a root-compatible alias at
`/.well-known/oauth-protected-resource`, and AS/OIDC aliases at
`/.well-known/oauth-authorization-server` and
`/.well-known/openid-configuration`. `/oauth/authorize` and `/oauth/token`
implement one authorization-code flow for `agentic:mcp`. Only
`https://chatgpt.com/connector/oauth/<suffix>` and the exact
`https://chatgpt.com/connector_platform_oauth_redirect` callbacks are
accepted. There are no refresh tokens, `offline_access`, DCR, generic
registration, or arbitrary redirects.

OAuth codes and access tokens are opaque, listener-local in-memory records with
expiry and revocation. Direct bearer content rotation updates authentication in
place, preserves existing rmcp sessions, and revokes OAuth records; listener
replacement, rebind, disable, or restart discards all listener-local state.
The standalone authorization page and tool/profile semantics are not Hub's
`Hub API key`, profile, or routing contract. The rmcp transport is stateful
Streamable HTTP/SSE, so rebind or disable still requires a new `initialize`.
See [`configuration.md`](configuration.md) for schema, allow-host, CLI, and
reload details.

`publicUrl` is the advertised external HTTPS origin only; it does not change
the local bind or route traffic, and the origin may remain loopback/private.
All sibling discovery, authorization, token, and `/mcp` routes must be exposed
through HTTPS for ChatGPT. The listener rejects malformed/missing Host and
forbids disallowed authorities on every sibling route. A reverse proxy or ESA
must put the authority it actually sends in `allowHosts`; a present `Origin`
must exactly match `publicUrl`, while absent Origin remains valid for
server-to-server requests. No permissive CORS is added.

For development without tunnel configuration or Hub reporting, set
`mode=local` and use `agentic-gpt run`. It loads the same profile-selected toolset preset,
policy, path policy, confirmation, audit, live config, and managed execution state, but serves
only the Unix MCP ingress; `httpMcp` does not add a TCP listener in Local mode.

### Six public runtime mappings

| Command | Command transport | Capability profile | Hub connection |
| --- | --- | --- | --- |
| `agentic-gpt run` (`mode=standalone`, `profile=normal`) | Tunnel stdio + local Unix MCP + optional HTTP MCP | Normal | disabled by default; reporting-only when enabled |
| `agentic-gpt run` (`mode=standalone`, `profile=room`) | Tunnel stdio + local Unix MCP + optional HTTP MCP | Room | disabled by default; reporting-only when enabled |
| `agentic-gpt run` (`mode=local`, `profile=normal`) | Local Unix MCP | Normal | disabled |
| `agentic-gpt run` (`mode=local`, `profile=room`) | Local Unix MCP | Room | disabled |
| `agentic-gpt run` (`mode=hub`, `profile=normal`) | Hub | Normal | command-capable |
| `agentic-gpt run` (`mode=hub`, `profile=room`) | Hub | Room | command-capable |

Transport does not change local policy. Tunnel, HTTP, and local Unix ingress use the same
Agent tool surface and policy boundaries for a profile-selected toolset set. The normal preset
excludes the logical `room` namespace by default; the room preset enables it. Explicit
`toolsets.enabled` selection is authoritative. Calls entering one worker share the same live
config, confirmation state, audit, capacity, and managed execution registry.
Room bootstrap, diary, and notebook execution follows the live `room` namespace rather than the
startup profile. A Normal-profile worker can therefore enable `room` without restart; direct
Room dispatch while it is disabled returns `room_toolset_required`.

## Local Unix MCP control channel

The socket path is derived from the configured identity:

```text
~/.agentic_gpt/runtime/agent/<agentId>/mcp.sock
```

The runtime directory is mode `0700`, the socket is mode `0600`, and accepted
connections must report the same local UID. Agentic never opens a TCP debug
port. Startup rejects an active socket, safely removes only a proven stale
owned socket, and uses the existing per-config `.run.lock`, so tunnel-backed
and local-only runtimes cannot own the same configuration simultaneously.
`agent.info.connections.localMcp` reports `ready`/`unavailable` and the exact
socket path.

Use the built-in real rmcp client to inspect or call the running surface:

```bash
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info --config ~/.agentic_gpt/config.json --arguments '{}'
printf '%s' '{"path":"README.md"}' | \
  agentic-gpt local call file.read --config ~/.agentic_gpt/config.json \
  --arguments-file -
```

The `agentic-gpt local` command is the owner-only Unix MCP client; its calls
retain `local:` audit provenance. It is distinct from the `agentic-gpt tmux`
local-admin CLI, which exposes exactly four commands: `list`, `attach`,
`create`, and `close` (request-context operation names
`tmux.listSessions`, `tmux.attach`, `tmux.createSession`, and
`tmux.closeSession`). Those CLI calls use `localadmin:` provenance, do not add
remote approval semantics, and do not fabricate AppState. The other ingress
prefixes remain distinct: `tunnel:` for the tunnel, `http:` for HTTP, and
`hub:` for Hub.

`--arguments` and `--arguments-file PATH|-` accept one JSON object, capped at
2 MiB. Structured MCP results are written to stdout; logs and typed connection
errors are written to stderr. A stopped/restarting runtime returns
`local_mcp_unavailable`; clients may reconnect but must not replay side effects.

## Tunnel, HTTP, and local tool surfaces

The V2 advertised surface contains 29 Normal names and 40 Room names. Profiles select
namespace presets rather than fixing the final runtime surface: normal enables `agent`, `file`,
`mcp`, `process`, `job`, `skills`, `tmux`, and `browser`; room enables all of those plus `room`.
An explicit `toolsets.enabled` selection is authoritative. The logical `room` namespace contains
`bootstrap`, `bootstrap.read`, the semantic read tools, and maintenance status/submit. These
filters only remove names from the advertised surface; they never expose dispatch-only aliases.

Start with `agent.info` to inspect the active profile and enabled namespaces, bounded path policy,
capacity, confirmation availability, and reporting state:

```text
mcp.list, mcp.callTool, mcp.batch
process.exec, process.batch, job.get, job.list, job.cancel
skills.list, skills.read, skills.setActive, skills.install,
skills.install.get, skills.install.cancel, skills.run
tmux.sessions, tmux.panes, tmux.exec, tmux.pasteText
browser.manual, browser.acquire, browser.repl, browser.reset, browser.release, browser.list
agent.info, file.read, file.search, file.edit
```

When the logical `room` namespace is enabled, the additional advertised names are:

```text
bootstrap, bootstrap.read
room.diary.active, room.diary.read
room.notebook.recent, room.notebook.search, room.notebook.read
room.state.list, room.state.read
room.maintenance.status, room.maintenance.submit
```

The same nine semantic Room names are the current contract for local Unix MCP,
Tunnel stdio, and Hub Full MCP/HTTP. Hub Full forwards
`room.diary.active/read`, `room.notebook.recent/search/read`,
`room.state.list/read`, and `room.maintenance.status/submit` through the
captured active Room lease; the matching HTTP routes are
`POST /v1/room/<namespace>/<action>` and accept no `agentId`. Coordinator
advertises and dispatches none of these Room operations.

Room reads remain bounded and Agent-owned: Notebook `limit` defaults to 20 and
is 1–100, search queries are non-empty and at most 256 Unicode characters,
Diary periods are semantic layer/date values, and Notebook/State Markdown reads
reject content above 512 KiB. `room.notebook.recent` and
`room.notebook.search` use current Markdown previews/results despite retaining
their public names; they are not passage/JSONL operations.

`room.maintenance.status` is read-only. `room.maintenance.submit` accepts one
to five unique slots, optional `local`/`workflow` mode, and `waitSeconds` from
0 through 30 (default 0). A workflow wait timeout only ends the wait and does
not cancel maintenance. Existing Agent path, lock, clean-tree, expected-change,
executor, and Git controls remain authoritative; no separate wait API or new
confirmation gate is added.

Hub owns only authentication, active-lease routing, and bounded run receipts.
A generic receipt may retain a bounded operation result, but it is not a Room
content authority or replica. Retired JSONL append/update/remove and
passage/date-selection callers are not silently mapped to maintenance; migrate
them to explicit semantic slot/payload requests or remove them. Historical
release/migration records are history, not an active error contract.

Managed `mcp.callTool` uses the same Job registry and capacity limit as
process and skill Jobs. Its `waitSeconds` defaults to 5 and is capped at 30;
`timeoutSeconds` is an absolute confirmation/connect/request deadline that
defaults to 300 and is capped at 900. Arguments must be a JSON object and their
serialized size is capped at 256 KiB. Results up to 512 KiB are retained in
the flat `JobToolResponse.result`; larger results set `resultTruncated=true` and
retain only byte count, SHA-256, and an 8 KiB UTF-8-safe preview. A downstream
`isError=true` result is retained while the Job state becomes `failed`.

Skill installation lookup and skill execution use the same bounded wait
contract: `waitSeconds` defaults to 5 and is capped at 30. A wait timeout only
ends the local wait; it does not implicitly cancel an installation or Job.
Use `skills.install.cancel` or `job.cancel` explicitly when cancellation is
required.

`mcp.batch` accepts 1–16 ordered calls. Every call is fully validated before
capacity admission or confirmation; invalid input and insufficient shared Job
capacity create no child Jobs and start no downstream side effects. Admission
is atomic. The batch then requests one aggregate confirmation after excluding
servers already covered by temporary allow state. A single-server batch may
receive a 15- or 30-minute server grant; a multi-server batch only supports one
batch-scoped allow or deny.

Parallel mode is the default; sequential mode waits for each child to become
terminal before starting the next. The shared scheduler permits at most eight
active MCP children globally and two per server, and `agent.info` reports the
limits plus active/queued counts. `failFast=true` only prevents children that
have not started from beginning after a hard failure; already-started calls are
never cancelled. Child results remain in input order. Per-call arguments and
results keep the 256 KiB / 512 KiB bounds, while aggregate arguments and the
serialized batch response are each capped at 2 MiB. If the response budget is
exceeded, later child result bodies are removed first while hashes, sizes,
previews, states, and Job ids remain available.

MCP cancellation uses the exact rmcp request id. `job.cancel` and execution
timeouts send `notifications/cancelled`; if the transport does not provide a
terminal cancellation response, Agentic reports `detached` with bounded
termination evidence rather than claiming `cancelled`. Child audit records
carry `batchId`, optional `batchCallId`, and `batchIndex`; one aggregate audit
records mode, fail-fast, confirmation outcome, child Job ids, final outcome,
and clipping. Confirmation/audit records contain server/tool names, a bounded
argument-key subset plus total count, byte counts and hashes, config revision,
result size/hash, and terminal evidence, but never raw arguments or raw results.

The standalone worker intentionally has no Tunnel `agentId` or
`confirmMethod` input fields. The worker supplies its configured local agent
identity internally; unexpected legacy fields are rejected. `bootstrap` is
Room-only. Managed Job admission tools (`process.exec`, `process.batch`,
`skills.run`, `mcp.callTool`, and `mcp.batch`) accept an optional validated
human-readable `group`; batch children inherit their parent group. Routine Job
responses are compact and keep `jobId` as the stable handle for later lookup.
Rich bounded provenance remains internal/durable rather than being repeated in
every response.

Terminal Job history is retained in the per-agent private `jobs.sqlite3` store
for 30 days subject to the logical soft cap, while a short live hot cache serves
recent results. `job.get` falls back to retained history by `jobId`; its
`waitSeconds` defaults to 0 and is capped at 30, and `waitOnly=true` suppresses
active intermediate detail while a bounded wait is in progress and returns
normal detail once terminal. `job.list` has a default limit of 50, capped at
100, and supports exact `group`/kind/state filters plus stable opaque cursor
pagination ordered by `createdAt DESC, jobId DESC`. Hub full and HTTP forwarding
preserve those fields while the Agent is available.
group/kind/state, but it explicitly refuses to invent continuation for an
Agent-issued cursor and reports cached `job.get` data only as degraded evidence,
not as a fresh wait result. Batch admission still rejects the whole batch before
starting any child when preflight, policy, confirmation, or capacity fails.

### Storage authority and recovery boundaries

The private Agent Job database (`jobs.sqlite3`) retains terminal history for
30 days subject to its logical soft cap. Its version-1 migration adopts
unambiguous legacy rows transactionally and refuses a future schema version
without quarantining or rewriting the database. Before migration it creates a
private `<jobs.sqlite3>.pre-migration.bak`; a newly created database is mode
`0600` under a private mode-`0700` parent. The live Job registry and hot cache
are projections; after an Agent restart, active Jobs are represented as
`unknown_after_restart` and are not replayed for side effects. History
retention or a Hub cache eviction does not roll back a process, MCP call, or
other external effect.

Process and MCP batch admission rows commit in one SQLite transaction before
their children enter the live registry. A persistence failure leaves no partial
batch admissions and starts no child execution; existing history is preserved.
This is admission atomicity, not transactional execution or rollback of external
effects after an admitted batch starts.

If acquiring the migration write lock fails after a snapshot has been staged,
the Agent removes that staging snapshot without replacing the previous recovery
backup. A later attempt can retry migration after the writer releases its lock.
This handled-error cleanup does not claim cleanup after an abrupt process kill
or hardware failure.

Reliable Hub commands use the Agent transport ledger as their local
deduplication and result authority. Each claim is file-locked and carries an
explicit owner; a record owned by another Agent is rejected. Unowned legacy
records remain `LegacyUnowned`: they are not automatically reconciled,
executed, or used to disclose a result. Recovery is an operator-led review of
the preserved raw record and newer owner-bound evidence; never delete or
replace deduplication evidence to bypass a corruption or ownership error.
Malformed JSON or a torn final line fails closed and preserves the raw bytes in
a private `.recovery` sidecar. Compaction may retain the prior raw ledger in a
private `.backup`; do not delete or replace these artifacts to bypass a
corruption error.

Configuration replacement stages and syncs a private temporary file, then
renames it into place. Replacing an existing config first stores a private
backup under its `backups/` directory, bounded by `backupLimit`. Setup of
secret references also uses a private setup journal. If hashes, file types, or
the journal state do not match an expected before/after pair, recovery fails
closed with a conflict rather than choosing a side or overwriting user data.
Stop the owning Agent before manually restoring a config or secret; never
paste secret values into commands, logs, or support output.

Workspace audit JSONL is bounded at 8 MiB. Rotation keeps the current file and
one `.1` backup and is best effort; audit loss is observable logging loss, not
proof that an operation or result did not occur. A wait timeout, missing
receipt, or cache omission likewise cannot be interpreted as remote
cancellation or effect rollback.


Tunnel, HTTP, and local Unix ingress do not expose Hub aggregation or notification tools. They use
the same local policy, path-policy, confirmation, audit, and ManagedJob lifecycle as Hub execution
while keeping the Hub out of the command path. The top-level `mcpServers` block is different: it
is the downstream registry consumed by `mcp.*` calls, not an inbound listener definition.

The checked-in [public tool contract matrix](tool-contract-matrix.md) records
use/non-use guidance, conditional fields, bounds, lifecycle/failure semantics,
and standalone/Hub parity for every Normal, Room, and Hub profile tool.

### Standalone file tools

`file.read` and `file.search` are bounded UTF-8 operations. They accept paths
relative to `workspaceRoot` (or absolute paths authorized by `pathPolicy`),
resolve symlinks before policy checks, and never invoke a shell or external
search process. Reads return content by default, optionally attach metadata with
`metadata: true`, support inclusive line ranges, and stop at the last complete
line before the 256 KiB response bound. A truncated read returns only
`nextStartLine`; a single line larger than the bound is rejected. Inside Git
repositories, search honors Git ignore
rules by default and caps its returned match/context payload at 256 KiB while
also bounding scanned files and bytes.

`contextLines` is a non-negative integer with a default of 0. The live maximum
is `limits.maxFileSearchContextLines` (default 5, configurable from 0 through
100 and visible as `agent.info.execution.fileSearch.maxContextLines`). Requests
above that maximum are clipped rather than discarded. Normal search responses
return only `matches`; clipping adds the effective `contextLines` and a bounded
warning, while truncation/skipped-file evidence appears only when it occurs.
Negative or non-integer values fail argument validation.

`file.read` and `file.search` also accept an ordered `requests` array of up to
32 per-request shapes. Flat and batch forms are mutually exclusive. Batch
results preserve input order, isolate failures, and trim result detail before
collapsing envelopes at the approximately 1 MiB aggregate response bound.
Search batches retain the 20,000-file and 128 MiB aggregate scan limits in
addition to each search's ordinary limits.

`file.edit` accepts only `patch` and optional `needConfirm`.
The patch uses Codex apply-patch syntax and may add, update, delete, or move
multiple files. Every source and destination is resolved through path policy,
locked deterministically, checked as UTF-8 and at most 8 MiB, staged and
validated before one optional confirmation. Source snapshots are revalidated
immediately before commit. Normal success responses contain only committed
requested paths and actions; partial failures retain ordered status/error
evidence. Diffs, resolved paths, changed-line counts, and revisions stay
internal for confirmation and audit.

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
- `hub.job.list`, `hub.job.get`
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

The local configuration is normally `~/.agentic_gpt/config.json`. Standalone
requires the `tunnel` block; Hub and Local modes do not. The complete
cross-runtime field reference is [`configuration.md`](configuration.md). The
canonical skill block is top-level `skills`. A legacy `room.skills`
block is read only when top-level `skills` is absent; when both are present,
top-level values win and a later config write serializes the canonical block.

A minimal standalone configuration uses references, not secret values:

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

Confirmation channels are serialized canonically as an ordered array:

```json
"confirmationProvider": {
  "channels": ["freedesktop", "ntfy"]
}
```

The legacy scalar/object forms (`hub`, `freedesktop-then-hub`,
`freedesktopThenHub`, `default`, and `{ "provider": "..." }`) remain readable
and preserve behavior; Agentic-managed writes emit the canonical `channels`
form. `ntfy` is the truthful channel name, while notification publication,
callback tokens, pending state, and decision relay remain owned by the Hub.

The active-Job limit accepts either the adaptive value or an explicit
integer:

```json
"limits": {
  "maxActiveJobs": "auto"
}
```

`auto` resolves at worker startup and after each valid live limits reload as
`clamp(ceil(availableParallelism * 1.5), 6, 24)`. Existing numeric values stay
explicit and are not migrated. Capacity rejection keeps the
`max_active_jobs_reached` code and includes bounded `active`, `requested`,
and `limit` details; batch admission remains atomic and all-or-reject.

This is a breaking v0.9 migration. Before starting the new binary, replace
`limits.maxActiveSessions` with `limits.maxActiveJobs` and remove the historical
`sessionIdleTimeoutSecs` field, which never controlled runtime behavior. The
strict limits object rejects both removed fields. The managed execution tool
and HTTP aliases are also removed rather than wrapped: use `process.batch`,
`job.get`, `job.list`, `job.cancel`, `/v1/process/*`, and `/v1/jobs/*`; old
`process.batchExec`, `process.get/list/kill`, managed `session.*`, `/v1/exec`,
`/v1/batchExec`, and `/v1/sessions/*` calls fail explicitly. tmux session names
and tmux session APIs are unchanged.

The current multi-file mutation boundary is documented in the file contract
matrix: one complete apply-patch request is staged and validated before its
optional confirmation and commit.

While a Standalone, Local, or Hub-connected Agent worker is running, edits to
`policy`, `limits`, `mcpServers`, `toolsets.enabled`, and (when `workspaceRoot`
is unchanged) `pathPolicy` are polled, fully validated, and applied atomically to
new admissions, calls, and tool discovery. MCP server ids use `A-Z`, `a-z`,
`0-9`, `.`, `_`, or `-` (maximum 64 bytes); `streamable-http` requires an
absolute HTTP(S) URL and may optionally use structured Bearer auth; `stdio`
requires a non-empty command and rejects HTTP auth. Invalid config versions keep
the last valid live subset. Already admitted Jobs and already-created downstream
MCP clients retain their original decision/server definition and are not cancelled
or rerouted by a reload. Because downstream clients are currently created per call,
no separate reload or reconnect command is needed.

Startup-owned identity, workspace root, Room settings, Browser configuration,
tunnel/client, reporting connection, and skill-install concurrency changes
remain restart-required. The shared watcher logs
`config changes require restart; fields=...`; the Standalone supervisor
additionally emits `restart_required`, while Hub mode has no supervisor event.
If `workspaceRoot` changes, the previous workspace root and `pathPolicy` remain
an atomic live pair until restart; a path-policy-only edit can reload only while
the root is unchanged. Enabling the `room` namespace live bootstraps against the
current live Room configuration; restart-required `room.*` edits on disk do not
change that runtime root until restart. `agent.info.mcp` reports only the
effective config revision, configured/enabled counts, and client lifecycle; it
does not expose endpoints.

The Standalone worker also watches and reconciles the `httpMcp` subset. Enabling
or disabling the endpoint, changing the bearer-token reference or resolved
content, or changing `host`, `port`, `publicUrl`, or `allowHosts` is handled
without restarting the worker. A public-origin or listener replacement is a new
listener identity: it closes stateful HTTP sessions and discards listener-local
OAuth codes and tokens. When the replacement bind address differs, it binds the
replacement before retiring the old listener. A same-address `allowHosts` or
`publicUrl` change must retire the old listener before binding the replacement;
if that replacement bind fails, the old listener is already gone and a later
retry starts fresh. A bind conflict on a different address keeps a working old
listener when the old address remains usable and is retried without interrupting
tunnel or Unix execution. Changing the token reference, or the content resolved
from it, updates the in-memory direct authenticator without rebinding, preserves
existing sessions, and atomically revokes OAuth records. If the reference cannot
be resolved, HTTP authentication fails closed: the listener stops listening and
accepting requests until the credential becomes available again. Invalid
candidates retain the last-good live configuration and state, and no reference,
token, code, or access-token value is logged or included in summaries.

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

The `env:NAME` form is suitable for a service manager's protected environment
or an injected secret, for example `env:AGENTIC_TUNNEL_API_KEY`; do not use a
literal `AGENTIC_TUNNEL_API_KEY=<secret> ...` command in a shell transcript.

Supported `config set` keys include:

- `tunnel.tunnelId`, `tunnel.apiKey`.
- `tunnel.client.version`, `tunnel.client.cacheDir`,
  `tunnel.client.autoDownload`, `tunnel.client.executable`,
  `tunnel.client.downloadUrl`, `tunnel.client.sha256`.
- `tunnel.hubReporting.enabled`, `tunnel.hubReporting.detail`.
- `httpMcp.enabled`, `httpMcp.host`, `httpMcp.port`, `httpMcp.publicUrl`,
  `httpMcp.bearerToken`, `httpMcp.allowHosts`.

The tunnel identity, secret reference, client source/version/hash/cache, Browser
configuration, and CLI profile are startup identity. Editing one while the
Standalone supervisor is running logs `restart_required` with changed field
names only; Browser changes use the `browser` field name. It does not switch
the existing child tree, and secret values are never printed.
`toolsets.enabled` is live configuration and does not require a restart.

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
needed for an Agent connection (`hub.url`, `hub.transport`, `agentId`, and
`hub.agentSecret`):

```bash
agentic-gpt config set tunnel.hubReporting.enabled true
agentic-gpt config set tunnel.hubReporting.detail metadata
```

The reporting connection identifies itself as `reporting-only`. It can send
hello/heartbeat, direct-run lifecycle events, Job snapshots, and the
existing confirmation traffic, but it never accepts Hub execution envelopes.
Hub requests reject reporting-only agents before creating a run. Reporting
disconnects, queue drops, and Hub unavailability never delay or change the
local MCP result.

`metadata` records tool/source/profile/status/timestamps/duration, identifiers,
exit code, and bounded failure reason. It omits arguments, results, program
argv, working directories, and stdout/stderr. `full` additionally stores
bounded JSON arguments/results and bounded existing Job snapshots; an
oversized value becomes a byte-count/SHA-256 truncation record rather than a
partial JSON fragment. Direct-run records remain in Hub storage for 24 hours.

The worker also writes bounded lifecycle records to stderr for each tool call
and managed process terminal event. These records contain the run/tool/profile
and status, with duration and safe 12-hex run/Job identifiers when
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

A tunnel control-plane logical connection can outlive a restarted stdio child.
When the fresh worker receives a non-ping request before a new MCP `initialize`,
the tunnel-only stdio transport restores rmcp's local initialization state with
a private handshake, suppresses that private response, and then replays the
original request with its original id. Pre-initialize notifications from the
stale logical connection are ignored. Ordinary client-led initialization is
passed through unchanged, and the owner-only Local Unix ingress does not use
this recovery shim. Successful recovery emits bounded diagnostics
`mcp_stdio_session_resume` and `mcp_stdio_session_resumed`; neither log contains
request arguments or results.

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
run history and `hub.job.list`/`hub.job.get` for current snapshots.

## Optional centralized Hub mode

Hub mode remains available when centralized routing, Actions, history, or
reporting is worth the shared infrastructure:

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
smoke. It also launches the hidden worker with a stale initialized notification
and a tool call as the first request, proving restart recovery keeps the worker
alive, hides the private handshake, and accepts a follow-up call. The in-process
tests cover the corresponding Room profile. Together they verify that stdout
remains MCP-only and that the compact Normal/Room surfaces are callable. These
checks are still distinct from a real Secure MCP
Tunnel control-plane call: the latter must invoke an actual external connector
and return a local Agentic tool result, not merely pass `/healthz`, `doctor`, or
a local fake handoff. Record external credentials/environment prerequisites
separately from repository tests.
