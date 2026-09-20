# Interfaces

The recommended Standalone runtime exposes the Normal/Room MCP surface directly
through Secure MCP Tunnel and owner-only Unix MCP. Its transport and tool
contract is documented in [`standalone-runtime.md`](standalone-runtime.md).

This page primarily maps the optional Rust Hub surfaces: GPT Actions, Apps MCP,
Hub-native tools, and the Hub-to-Agent protocol.

The cross-surface use/non-use, conditional-input, bounds, lifecycle, and parity
matrix is maintained in [`tool-contract-matrix.md`](tool-contract-matrix.md).

## Agent ingress and operation boundary (WP2)

The Agent keeps one narrow internal admission boundary without introducing a
new framework or registry. Each adapter creates an immutable
`RequestContext { ingress, operation }`; `operation::authorize(runtime, config,
context)` checks the real ingress plus namespace/toolset and capability rules.
Descriptor annotations such as `read_only`, `destructive`, and `open_world`
remain discovery/client metadata and are never authorization.

Ingress-specific authentication, framing, and error envelopes remain distinct:
local Unix retains its UID/socket guard and `local:` source prefix; Tunnel
stdio uses `tunnel:`; worker HTTP MCP retains bearer/Host/Origin/session
handling and `http:`; Hub WS/SSE retains its protocol envelope/replay and
`hub:` source; CLI uses `localadmin:`. The CLI gate is intentionally limited
to the four existing local tmux administration operations. HTTP/MCP, Hub wire,
and local stdio/Unix result projections may differ at the transport boundary,
while shared Agent operations use the same value/error and slim Job/Skill
result layer.

Normal is not an alias for Room: a Normal runtime may use Room only when the
`room` namespace is explicitly enabled. Hub keeps its existing Room toolset,
Skills capability/profile, and notification capability rules. Policy, path,
confirmation, lease, and resource owners still decide actual side effects;
this boundary does not make Hub an executor or claim generic OS sandboxing.

Reload applies the existing live-safe subset (`policy`, `limits`, `mcpServers`,
`toolsets`, `httpMcp`; `pathPolicy` only when `workspaceRoot` is unchanged)
without reconstructing startup-derived resources. Identity/mode/profile,
workspace/runtime/socket, Browser configuration as a whole (not only
`browser.runtime`), history/install, and related resource-owner
changes require restart. Enabling Room prepares the existing live root before
the new subset is used.

`agentic-gpt local` is the Unix MCP client and uses the ordinary MCP
operation gate with `local:` provenance. It is distinct from
`agentic-gpt tmux`, whose CLI-admin gate admits only
`tmux.listSessions`, `tmux.attach`, `tmux.createSession`, and
`tmux.closeSession` and records `localadmin:` provenance. MCP
`tmux.sessions`, `tmux.panes`, `tmux.exec`, and `tmux.pasteText` are not those
four CLI-admin operations.

## GPT Actions API

The GPT Actions API is described by `openapi/hub.yaml` and is protected by the Hub API key.

Core endpoints:

- `GET /v1/info`: safe Hub runtime summary.
- `GET /v1/agents`: enabled local agents with online status and safe config summaries.
- `POST /v1/process/exec`: start one managed process and wait briefly. The response is always a Job envelope and supports optional `workingDirectory` and bounded `waitSeconds`.
- `POST /v1/process/batch`: atomically admit a managed process batch with batch-level `workingDirectory`, per-element overrides, one confirmation decision, and ordered child Jobs.
- `GET /v1/jobs?agentId=...`: list active or recently retained Jobs with optional kind/state/limit filters.
- `GET /v1/jobs/{jobId}?agentId=...&waitSeconds=...`: inspect or briefly wait for one Job.
- `POST /v1/jobs/{jobId}/cancel?agentId=...`: request kind-aware cancellation and return outcome/termination evidence.
- `POST /v1/mcp/servers`: list MCP servers configured inside one local agent, or omit `agentId` to group MCP servers for all currently connected agents.
- `POST /v1/mcp/tools`: list tools exposed by one MCP server.
- `POST /v1/mcp/callTool`: start one managed downstream MCP tool Job through the selected local agent. `waitSeconds` defaults to 5 and is capped at 30; `timeoutSeconds` defaults to 300 and is capped at 900.
- `POST /v1/mcp/batch`: atomically admit 1–16 ordered downstream MCP child Jobs. It uses one aggregate confirmation, parallel or sequential mode, optional safe fail-fast scheduling, shared global/per-server concurrency limits, and a 2 MiB aggregate response budget.
- `GET /v1/runs/{runId}`: inspect persisted status and optional late result for one Hub-to-Agent command run.
- `POST /v1/room/skills/list`, `/read`, `/search`, `/active`, `/activate`, `/deactivate`: discover workspace skills through the active Room Agent and maintain local active skill state. These endpoints do not take `agentId`.
- `POST /v1/room/skills/install`: asynchronously install one skill from public GitHub, HTTPS file entries, or inline UTF-8/base64 files. The response returns an `installId` before network work begins.
- `POST /v1/room/skills/install/get`: query an installation with bounded long polling (`waitSeconds`, default 5, maximum 30); terminal responses set `pollAfterMs` to `0`.
- `POST /v1/room/skills/install/cancel`: request idempotent cooperative cancellation before atomic commit.
- `POST /v1/room/skills/run`: run an executable active workspace skill script under `scripts/`. It returns terminal Job output inline when possible, otherwise the same `jobId` used by `job.get` and `job.cancel`. These endpoints do not take `agentId`.
- `POST /v1/room/bootstrap`: load the active Room Agent's repeated session entrypoint and deterministic guide manifest. It has no request body or `agentId`.
- `POST /v1/room/bootstrap/read`: read one valid bootstrap guide by its frontmatter `id`. It has no `agentId`.

`/v1/info` intentionally returns only safe metadata: Hub version, public base URL, timeout settings, remote confirmation status, agent counts, and pending request/Job counts. It must not expose secrets, confirmation callback URLs, or private config values.

`/v1/agents` returns one safe config summary per enabled local agent. When an agent is online, the summary includes coarse sandbox mode, confirmation provider, path policy roots, configured command policy rules, and builtin command policy rules. Path roots are display paths such as `workspace`, `~/Documents`, or `/tmp`; private home paths should be shortened with `~` where possible. Offline agents may return an `unknown` summary because the Hub does not persist the last local config summary. Local confirmation prompts can use English or Simplified Chinese via `confirmationLanguage` (`en` or `zh-CN`).

### Hub Job authority, freshness, and retention

The Agent's managed Job history is the execution-side authority. Hub `JobInfo`
entries and the Hub Job cache are projections used for routing and observation;
they do not prove that a local process is still running or that a side effect
was undone. A Hub cache entry is bounded to 4,096 Jobs, expires 15 minutes
after its `observedAt`, and is classified as `stale` after 60 seconds. A
15-second sweep removes expired entries, and capacity eviction removes the
oldest observation. Evicting an active projection has no effect on the Agent
Job or the authoritative Hub run receipt.

Hub HTTP `job.list`, `job.get`, and `job.cancel`, plus the corresponding
Apps MCP Job inspection/control responses, expose top-level `freshness` and
`observedAt` metadata. Direct live Job envelopes from other command endpoints
may omit these projection fields; their Agent Job payload remains authoritative.
`live` is a response from the Agent, `cached` is a usable Hub projection within
its freshness window, `stale` is an older projection, and `unknown` means that
no usable current fact is available (including after restart reconciliation).
These fields describe the response projection; they are not fields on Agent
`JobInfo`. A cache-only `job.get` is degraded evidence, not a fresh wait, and
the Hub does not invent continuation for an Agent-issued cursor.

Hub run receipts remain the durable control-plane identity for a dispatched
command. After the 24-hour run retention window, only eligible completed
payloads are compacted: `runId`, request/agent identity, command hash, status,
and conflict/unknown/tombstone evidence remain. The identity/hash evidence
needed for replay and deduplication remains protected; unknown and conflict
records are not compacted. `AgentRun` therefore reports `resultRetained` and
`resultOmitted` separately; an omitted payload is not evidence that the command
did not run.

Wait or transport timeout is not remote cancellation. It ends the local wait
only; a late matching receipt or result can still arrive. Cancellation is
reported only from observed termination evidence, and a cache snapshot or
missing response never permits an inference that the remote Job stopped.


## ChatGPT Apps MCP endpoint

`/mcp` is the Apps-friendly MCP endpoint. It is protected by the Hub OAuth shim and forwards MCP requests to the configured local agent and local MCP server.

All `/mcp` `tools/call` responses use the Hub `AgenticResult` envelope, which is directly compatible with the ChatGPT Apps / MCP tool result shape. Hub-native JSON is exposed as `structuredContent` plus a JSON text content block; a top-level `error` makes the MCP tool result `isError=true`.

`mcp.callTool` no longer passes a downstream result envelope through at the Hub top level. It returns a managed `JobResponse`. A terminal downstream result is retained under `result`; downstream `isError=true` produces a failed Job while retaining that result. Serialized arguments are capped at 256 KiB. Serialized results up to 512 KiB are retained; larger results are omitted and replaced by `resultBytes`, `resultSha256`, and a UTF-8-safe `resultPreview`. Active calls are inspected with `job.get` and cancelled with `job.cancel`.

`mcp.batch` returns `McpBatchResponse` with child details in original input
order. Validation and capacity admission happen before confirmation and before
any child starts. Parallel mode uses the shared scheduler (eight globally, two
per server); sequential mode waits for each child terminal state. With
`failFast=true`, only not-yet-started children become `skipped`; already-started
calls are not cancelled. Single-server batches can receive temporary server
allow actions, while multi-server confirmation remains batch-scoped. Each
child is an ordinary MCP Job with `batchId`, optional `batchCallId`, and
`batchIndex`, so later inspection and cancellation use the same `job.*`
lifecycle.

Cancellation is evidence-based. Agentic sends MCP `notifications/cancelled`
with the exact downstream request id. If no downstream terminal response is
observed, the Job becomes `detached` rather than claiming cancellation
succeeded. Hub cache-only `job.get` responses set `detailAvailable=false`, and
Hub never reports a cached snapshot as a successful `job.cancel`.

This contract applies to the Apps MCP `/mcp` surface. The GPT Actions endpoints under `/v1/*` keep their OpenAPI-described JSON response shapes.

OAuth discovery routes:

- `/.well-known/oauth-protected-resource`
- `/.well-known/oauth-authorization-server`
- `/.well-known/openid-configuration`
- `/oauth/authorize`
- `/oauth/token`

The Hub MCP profile is selected at Hub startup with `--mcp-profile full|coordinator`
or `AGENTIC_GPT_HUB_MCP_PROFILE`. `full` is the default and preserves the
execution surface plus the transport-neutral `bootstrap` aliases. `coordinator`
advertises only the Hub-native tools `hub.info`, `agent.list`, `hub.run.list`,
`hub.run.get`, `hub.job.list`, `hub.job.get`, `user.notify.channels`,
and `user.notify.send`; it never dispatches an Agent command. See
[`standalone-runtime.md`](standalone-runtime.md) for the complete profile and
standalone Tunnel documentation.

The ntfy confirmation callback routes are intentionally not part of `openapi/hub.yaml`. They are only used by confirmation action buttons.

Room skill packages remain workspace-visible under `<workspaceRoot>/skills/`, while tool-managed activation state is stored as private durable state under `~/.agentic_gpt/state/agent/<agentId>/active-skills.json`. On startup, Agentic migrates an unambiguous legacy `<workspaceRoot>/state/active-skills.json`; if both old and new copies differ, the private copy remains authoritative and the legacy copy is retained with a warning. Activating a skill does not execute it or grant permissions; stale active entries remain visible as `missing` until explicitly deactivated. The built-in `skill-installer` guide is active by default and can be explicitly deactivated.

Installation jobs are persisted under `~/.agentic_gpt/state/agent/<agentId>/skill-installs/`; the legacy `<workspaceRoot>/state/skill-installs/` tree is migrated with the same preserve-on-conflict behavior before install recovery runs. Installation records retain terminal state for seven days (capped at 100) and never expose inline payloads or URL query/fragment values in public status. Existing skills are archived under `skills/.archive/<id>/` before an explicit replacement. Remote file URLs require public HTTPS and are revalidated after DNS resolution and redirects; deployments can narrow hosts with `room.skills.allowedHosts`.

## Room session bootstrap package

The Room Agent reads a repeated session bootstrap package directly from the configured `workspaceRoot` on every call. Reads do not create files, install defaults, cache an index, or require a reload. The fixed layout is:

```text
<workspaceRoot>/bootstrap/
├── bootstrap.md
└── guides/
    ├── diary.md
    ├── notebook.md
    └── ...
```

`bootstrap.md` is required. `guides/` is optional. Only direct, regular, non-hidden files with a lowercase `.md` extension are considered guides; nested directories, hidden entries, and other extensions are ignored. The bootstrap root and entrypoint may not be symlinks. A missing package is a normal 404 (`bootstrap_not_found`); the service does not auto-create or personalize one.

The entrypoint starts with a closed YAML object. Its required fields are:

```markdown
---
id: room
kind: entrypoint
name: Room Bootstrap
description: Session initialization and guide routing.
schemaVersion: 1
---

At the start of a Room session, read the relevant guides listed below.
```

`id` uses the conservative ASCII grammar `[A-Za-z0-9_.-]+`; `.` and `..` are not valid IDs. `kind` must be `entrypoint`, `name` and `description` must be non-empty strings, and `schemaVersion` must be the integer `1`. The raw frontmatter is retained in the entrypoint response. Invalid entrypoint metadata fails the package with `bootstrap_invalid`.

Every guide uses the same closed-frontmatter convention:

```markdown
---
id: diary
kind: guide
title: Diary conventions
summary: Preserve continuity without replacing the Diary tool schema.
loadPolicy: contextual
priority: 80
loadWhen:
  - The session continues prior personal or project context.
toolBindings:
  - room.diary.active
  - room.diary.read
  - room.maintenance.submit
tags:
  - continuity
---

Use semantic Diary reads for current or exact documents; route mutations through the
maintenance submission contract.
```

Required guide fields are `id`, `kind: guide`, `title`, and `summary`. `loadPolicy` defaults to `on_demand` and accepts `startup`, `contextual`, or `on_demand`. `priority` defaults to `0` and is a signed 32-bit integer. `loadWhen`, `toolBindings`, and `tags` default to empty arrays and contain non-empty strings in authored order. Unknown fields are ignored for typed V1 behavior but remain in the raw `frontmatter` returned by `room.bootstrap.read`.

Guide metadata is generic. For example, a workspace may author guides like these without changing the runtime:

```markdown
<!-- guides/notebook.md -->
---
id: notebook
kind: guide
title: Notebook continuity
summary: Search and read durable project passages before making assumptions.
loadPolicy: contextual
toolBindings: [room.notebook.search, room.notebook.read, room.maintenance.submit]
tags: [project-context]
---
Keep MCP argument schemas in the tool definition; use maintenance submission for Notebook changes.
```

```markdown
<!-- guides/execution.md -->
---
id: execution
kind: guide
title: Execution and Job choice
summary: Choose managed Jobs or persistent panes deliberately.
loadPolicy: startup
priority: 90
toolBindings: [process.exec, process.batch, job.get, job.cancel, tmux.exec]
tags: [operations, safety]
---
Use the tool schema for arguments and this guide for workflow, confirmation, and recovery.
```

```markdown
<!-- guides/skills.md -->
---
id: skills
kind: guide
title: Skill selection
summary: Discover and read relevant skills before using an installed workflow.
loadPolicy: on_demand
toolBindings: [skills.list, skills.read, skills.run]
tags: [workflows]
---
Treat toolBindings as descriptive routing hints, not permission grants or availability claims.
```

The MCP schemas remain the source of truth for tool availability and arguments. Guides provide selection, sequencing, conventions, safety, examples, and recovery behavior; they do not duplicate complete MCP schemas, grant authorization, or assert that every named binding is currently exposed.

`room.bootstrap` returns the entrypoint inline, a flat manifest, a package `revision`, counts, and warnings. Valid guides are ordered by descending `priority`, then ascending `id`; at most 64 summaries are returned. `totalGuides` counts all valid, duplicate-free guides, so a valid guide beyond the 64-item ceiling is still readable through `room.bootstrap.read` and still affects `revision`. Duplicate IDs exclude every colliding guide. Invalid optional guides are excluded with warnings rather than failing the package.

`room.bootstrap.read` accepts `{ "id": "diary" }`, validates the entrypoint and guide package again, and returns the selected summary, raw guide frontmatter, bounded Markdown resource, and relevant warnings. It is not a generic path reader. Unknown, invalid, duplicate-excluded, or otherwise unavailable IDs return `guide_not_found`.

The complete leading YAML frontmatter block, including both `---` delimiters, must end within the first 1,048,576 bytes (1 MiB); a closing delimiter exactly at the bound is accepted. This metadata bound is separate from returned-content truncation. An over-limit entrypoint returns `bootstrap_invalid`; an over-limit optional guide is excluded with `guide_frontmatter_invalid`. The complete file is still streamed for UTF-8 validation, line counting, SHA-256, and package revision.

Each text resource is UTF-8 Markdown with `mediaType: text/markdown`. `sizeBytes` and `sha256` describe the complete original file; `returnedSizeBytes` describes the returned content. Entrypoints are capped at 65,536 bytes and guides at 262,144 bytes. Oversized valid documents return a prefix with `truncated: true` and an `entrypoint_truncated` or `guide_truncated` warning. Truncation prefers the last complete newline within the bound; otherwise it ends at a valid UTF-8 boundary. `totalLines` and `returnedThroughLine` are one-based logical counts, `omittedFromLine` identifies the first omitted line, and `lastLineComplete` distinguishes a complete-line prefix from a partial-line prefix.

The stable warning prefixes include `entrypoint_truncated`, `guide_truncated`, `guides_truncated`, `guides_dir_symlink_ignored`, `guide_dir_entry_unreadable`, `guide_symlink_ignored`, `guide_unreadable`, `guide_non_utf8`, `guide_frontmatter_invalid`, `guide_metadata_invalid`, and `guide_duplicate_id`. Package-level failures use `bootstrap_not_found`, `bootstrap_invalid`, or `bootstrap_read_failed`; Room routing adds `room_not_active`, `room_state_conflict`, `room_bootstrap_timeout`, and `room_bootstrap_read_timeout`. These operations are read-only, retry-safe, non-destructive, and non-consequential.

The MCP tools are `room.bootstrap` and `room.bootstrap.read`. The matching GPT Actions routes are `POST /v1/room/bootstrap` and `POST /v1/room/bootstrap/read`, with operation IDs `roomBootstrap` and `roomBootstrapRead`. Both surfaces are Room-scoped and omit `agentId`.

## Local Agent transports

Local agents connect to:

```text
GET /v1/agents/{agentId}/connect
```

WebSocket is the default local-agent transport. Local agents may opt into the HTTP/SSE transport with `hub.transport: "sse"` for environments where outbound HTTP/SSE is more stable than WebSocket.

SSE endpoints are agent-private and use the same `x-agent-secret` authentication as WebSocket:

```text
GET  /v1/agents/{agentId}/events?connectionId=...
POST /v1/agents/{agentId}/messages?connectionId=...
```

WebSocket and HTTP/SSE share reliable ack/replay semantics for request/response-style `HubCommand` messages. The Hub sends a command envelope containing `eventId`, `runId`, `requestId`, `commandHash`, and the original `HubCommand`. The agent writes the accepted command to its local transport ledger before sending `TransportAck`; command results include `runId` and are accepted as late results when `agentId`, `runId`, and `requestId` match, even if the original connection is stale.
ConfirmationRequest is not a run-bearing command and carries no `runId`. Hub confirmation ownership is the captured `(agentId, connectionId, requestId, sender)` tuple: a callback resolves one claim against that connection/request and the captured delivery target; it never reselects a sender by bare Agent id or invents a run identity.
When the current connection is replaced or removed, an unresolved confirmation makes one terminal `ProviderUnavailable` claim with reason `provider_unavailable` and targets the captured sender before the old stream is closed when that transport is writable. Callback, confirmation timeout, publish failure, replacement, and disconnect race for the same claim, so none can emit a second terminal decision or reselect a sender. If the captured sender is already broken, Agent disconnect-drain behavior remains the fallback; Hub does not promise delivery over a failed transport.

A bounded Hub/HTTP/MCP wait timeout ends only the local waiter. It does not cancel remote execution. A matching late `TransportAck`, `TransportRunStatus`, or `Response` may still advance the persisted run receipt after the caller has received its timeout, even when no waiter remains. `not_sent` is reserved for a proven channel-send failure and is excluded from replay; a timeout or missing ACK is not `not_sent`.

Hub transport receipt statuses covered by this contract include `created`, `dispatched`, `acked`, `started`, `running`, `failed`, `unknown`, `completed`, `timeout_waiting_result`, and `not_sent`; this is not an exhaustive list of AgentReport or Job statuses. Terms such as `sent_no_ack`, `acked_running`, `wait_expired`, `remote_unknown`, and `cancel_requested` are conceptual vocabulary for describing observations or caller state, not additional wire or persisted statuses. No `cancel_requested` transport state is introduced.

Both transports apply the same current-generation admission rule. `Hello`, `Heartbeat`, `JobUpdate`, `RunReport`, and `ConfirmationRequest` are current-only lifecycle messages: only the latest connection for an agent may update metadata, the Job cache, reports, last-seen state, confirmation admission, or the Room lease. A stale WebSocket message is rejected by the handler and the retired stream receives `Close`; a stale HTTP/SSE message is rejected with `409 stale_connection`. The local agent should stop the writer for that generation.

Stale reliable messages (`TransportAck`, `TransportRunStatus`, and `Response`) may still be accepted when their run metadata matches an existing Hub run, preserving late-result delivery after reconnects. They do not refresh or otherwise modify the current connection's lifecycle state. `RunReport` is not a reliable replay message and remains current-only.

Each SSE connection must use a fresh, non-empty `connectionId`; the same current ID cannot name a second stream. Omitting `connectionId` lets the Hub generate a fresh ID. An explicitly empty ID returns `400 invalid_connection_id`; reusing the current ID returns `409 connection_id_in_use`. A successful replacement closes the previous stream. These IDs identify a connection generation after agent-secret authentication; they are not a separate peer-authentication mechanism.

`Hello`, `Heartbeat`, `HeartbeatAck`, confirmation messages, and `JobUpdate` remain best-effort lifecycle messages in V1. `process.exec`, `process.batch`, `job.get`, and `job.cancel` are reliable request/response commands. `Hello.bootGeneration` changes cause active cached Jobs to become `unknown_after_restart`; terminal Jobs remain retained and side effects are never replayed.

On Agent restart, the transport ledger is the durable command/result authority:
owner-bound completed records can resend their matching result, and
owner-bound accepted records can resume from the stored command; started/running
records without a completed result become `unknown` rather than replaying a side
effect. Claims
are locked and bound to their explicit `agentId`; a foreign owner cannot adopt
the record. Legacy unowned records remain `LegacyUnowned`: they are not
auto-reconciled, executed, or used to disclose a result. Recovery is an
operator-led review of the preserved raw record and newer owner-bound
evidence; never delete deduplication evidence to make startup pass.

Ledger parsing or a torn final line fails closed. The raw offending bytes are
preserved in a private `.recovery` sidecar, and compaction keeps a private
`.backup`; neither is a reason to delete or reset the ledger. The Hub marks
acked runs without a status/result as `unknown` after its timeout, but neither
that state nor a missing cache proves that an Agent stopped.
