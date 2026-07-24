# Interfaces

Agentic GPT exposes two public integration surfaces from the Rust Hub.

## GPT Actions API

The GPT Actions API is described by `openapi/hub.yaml` and is protected by the Hub API key.

Core endpoints:

- `GET /v1/info`: safe Hub runtime summary.
- `GET /v1/agents`: enabled local agents with online status and safe config summaries.
- `POST /v1/exec`: short synchronous command execution. Supports optional `workingDirectory`.
- `POST /v1/batchExec`: short synchronous batch execution. Supports a batch-level `workingDirectory` and per-element overrides. Batch confirmation is all-or-nothing: if any element is denied, the whole batch is rejected; if any element needs confirmation, one confirmation decision applies to the entire batch.
- `POST /v1/sessions/start`: start a long-running command session. Supports optional `workingDirectory`.
- `GET /v1/sessions`: list sessions for one agent.
- `GET /v1/sessions/{sessionId}`: inspect one session.
- `POST /v1/sessions/{sessionId}/wait`: wait briefly for session output updates.
- `POST /v1/sessions/{sessionId}/kill`: stop a running session.
- `POST /v1/mcp/servers`: list MCP servers configured inside one local agent, or omit `agentId` to group MCP servers for all currently connected agents.
- `POST /v1/mcp/tools`: list tools exposed by one MCP server.
- `POST /v1/mcp/callTool`: call one MCP tool through the selected local agent.
- `GET /v1/runs/{runId}`: inspect persisted status and optional late result for one Hub-to-Agent command run.
- `POST /v1/room/skills/list`, `/read`, `/search`, `/active`, `/activate`, `/deactivate`: discover workspace skills through the active Room Agent and maintain local active skill state. These endpoints do not take `agentId`.
- `POST /v1/room/skills/install`: asynchronously install one skill from public GitHub, HTTPS file entries, or inline UTF-8/base64 files. The response returns an `installId` before network work begins.
- `POST /v1/room/skills/install/get`: query an installation with bounded long polling (`waitSeconds`, default 5, maximum 30); terminal responses set `pollAfterMs` to `0`.
- `POST /v1/room/skills/install/cancel`: request idempotent cooperative cancellation before atomic commit.
- `POST /v1/room/skills/run`: run an executable active workspace skill script under `scripts/`. It returns terminal session output inline when possible, otherwise the same `sessionId` used by the existing inspect/wait/kill session APIs. These endpoints do not take `agentId`.
- `POST /v1/room/bootstrap`: load the active Room Agent's repeated session entrypoint and deterministic guide manifest. It has no request body or `agentId`.
- `POST /v1/room/bootstrap/read`: read one valid bootstrap guide by its frontmatter `id`. It has no `agentId`.

`/v1/info` intentionally returns only safe metadata: Hub version, public base URL, timeout settings, remote confirmation status, agent counts, and pending request/session counts. It must not expose secrets, confirmation callback URLs, or private config values.

`/v1/agents` returns one safe config summary per enabled local agent. When an agent is online, the summary includes coarse sandbox mode, confirmation provider, path policy roots, configured command policy rules, and builtin command policy rules. Path roots are display paths such as `workspace`, `~/Documents`, or `/tmp`; private home paths should be shortened with `~` where possible. Offline agents may return an `unknown` summary because the Hub does not persist the last local config summary. Local confirmation prompts can use English or Simplified Chinese via `confirmationLanguage` (`en` or `zh-CN`).

## ChatGPT Apps MCP endpoint

`/mcp` is the Apps-friendly MCP endpoint. It is protected by the Hub OAuth shim and forwards MCP requests to the configured local agent and local MCP server.

All `/mcp` `tools/call` responses use the Hub `AgenticResult` envelope, which is directly compatible with the ChatGPT Apps / MCP tool result shape:

- `content`: model/client-visible MCP content blocks, including non-text blocks such as `image`, `audio`, `resource`, and `resource_link` when returned by downstream MCP servers.
- `structuredContent`: concise JSON visible to the model and Apps component.
- `_meta`: widget-only MCP result metadata.
- `isError`: tool-result error flag.

Hub-native `/mcp` tools wrap their JSON payloads as `AgenticResult` with both `structuredContent` and a JSON text content block. The `mcp.callTool` tool recognizes downstream MCP tool result envelopes and passes through their top-level `content`, `structuredContent`, `_meta`, and `isError` instead of nesting them inside a Hub JSON payload.

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
`hub.run.get`, `hub.session.list`, `hub.session.get`, `user.notify.channels`,
and `user.notify.send`; it never dispatches an Agent command. See
[`standalone-runtime.md`](standalone-runtime.md) for the complete profile and
standalone Tunnel documentation.

The ntfy confirmation callback routes are intentionally not part of `openapi/hub.yaml`. They are only used by confirmation action buttons.

Room skills API stores active state in the Room Agent workspace under `state/active-skills.json`. Activating a skill does not execute it or grant permissions; stale active entries remain visible as `missing` until explicitly deactivated. The built-in `skill-installer` guide is active by default and can be explicitly deactivated.

Installation jobs are persisted under `state/skill-installs/`, retain terminal records for seven days (capped at 100), and never expose inline payloads or URL query/fragment values in public status. Existing skills are archived under `skills/.archive/<id>/` before an explicit replacement. Remote file URLs require public HTTPS and are revalidated after DNS resolution and redirects; deployments can narrow hosts with `room.skills.allowedHosts`.

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
  - room.diary.recent
  - room.diary.append
tags:
  - continuity
---

Use the Diary tools for dated records and follow the recovery rules described here.
```

Required guide fields are `id`, `kind: guide`, `title`, and `summary`. `loadPolicy` defaults to `on_demand` and accepts `startup`, `contextual`, or `on_demand`. `priority` defaults to `0` and is a signed 32-bit integer. `loadWhen`, `toolBindings`, and `tags` default to empty arrays and contain non-empty strings in authored order. Unknown fields are ignored for typed V1 behavior but remain in the raw `frontmatter` returned by `room.bootstrap.read`.

Guide metadata is generic. For example, a workspace may author guides like these without changing the runtime:

```markdown
<!-- guides/notebook.md -->
---
id: notebook
kind: guide
title: Notebook continuity
summary: Search and update durable project passages before making assumptions.
loadPolicy: contextual
toolBindings: [room.notebook.search, room.notebook.current]
tags: [project-context]
---
Keep MCP argument schemas in the tool definition; put selection and recovery rules here.
```

```markdown
<!-- guides/execution.md -->
---
id: execution
kind: guide
title: Execution and session choice
summary: Choose deterministic commands, managed sessions, or persistent panes deliberately.
loadPolicy: startup
priority: 90
toolBindings: [process.exec, session.start, tmux.exec]
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

WebSocket is the default local-agent transport. Local agents may opt into the HTTP/SSE transport with `hubTransport: "sse"` for environments where outbound HTTP/SSE is more stable than WebSocket.

SSE endpoints are agent-private and use the same `x-agent-secret` authentication as WebSocket:

```text
GET  /v1/agents/{agentId}/events?connectionId=...
POST /v1/agents/{agentId}/messages?connectionId=...
```

WebSocket and HTTP/SSE share reliable ack/replay semantics for request/response-style `HubCommand` messages. The Hub sends a command envelope containing `eventId`, `runId`, `requestId`, `commandHash`, and the original `HubCommand`. The agent writes the accepted command to its local transport ledger before sending `TransportAck`; command results include `runId` and are accepted as late results when `agentId`, `runId`, and `requestId` match, even if the original connection is stale.

For HTTP/SSE, the Hub treats only the latest `connectionId` for an agent as the current connection. Stale `Hello`, `Heartbeat`, `SessionUpdate`, and `ConfirmationRequest` messages sent to `/messages` are rejected with `409 stale_connection`; the local agent should stop the writer for that `connectionId`. Stale reliable messages (`TransportAck`, `TransportRunStatus`, and `Response`) may still be accepted when their run metadata matches an existing Hub run, preserving late-result delivery after reconnects.

`Hello`, `Heartbeat`, `HeartbeatAck`, confirmation messages, and `SessionUpdate` remain best-effort/legacy messages in V1. `session.start` itself is a reliable request/response command; later session state updates continue to use the existing session cache plus `session.inspect`/`session.wait` queries.

On agent restart, the local ledger is reconciled as follows: completed runs resend their result, accepted-but-not-started runs continue execution from the stored command, and started/running runs without a completed result report `unknown` instead of replaying side effects. The Hub also marks acked runs without a status/result as `unknown` after a timeout so callers can query a terminal state via `/v1/runs/{runId}`.
