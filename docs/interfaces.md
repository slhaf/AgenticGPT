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

`/v1/info` intentionally returns only safe metadata: Hub version, public base URL, timeout settings, remote confirmation status, agent counts, and pending request/session counts. It must not expose secrets, confirmation callback URLs, or private config values.

`/v1/agents` returns one safe config summary per enabled local agent. When an agent is online, the summary includes coarse sandbox mode, confirmation provider, path policy roots, configured command policy rules, and builtin command policy rules. Path roots are display paths such as `workspace`, `~/Documents`, or `/tmp`; private home paths should be shortened with `~` where possible. Offline agents may return an `unknown` summary because the Hub does not persist the last local config summary. Local confirmation prompts can use English or Simplified Chinese via `confirmationLanguage` (`en` or `zh-CN`).

## ChatGPT Apps MCP endpoint

`/mcp` is the Apps-friendly MCP endpoint. It is protected by the Hub OAuth shim and forwards MCP requests to the configured local agent and local MCP server.

All `/mcp` `tools/call` responses use the Hub `AgenticResult` envelope, which is directly compatible with the ChatGPT Apps / MCP tool result shape:

- `content`: model/client-visible MCP content blocks, including non-text blocks such as `image`, `audio`, `resource`, and `resource_link` when returned by downstream MCP servers.
- `structuredContent`: concise JSON visible to the model and Apps component.
- `_meta`: widget-only MCP result metadata.
- `isError`: tool-result error flag.

Hub-native `/mcp` tools wrap their JSON payloads as `AgenticResult` with both `structuredContent` and a JSON text content block. The `mcpCallTool` tool recognizes downstream MCP tool result envelopes and passes through their top-level `content`, `structuredContent`, `_meta`, and `isError` instead of nesting them inside a Hub JSON payload.

This contract applies to the Apps MCP `/mcp` surface. The GPT Actions endpoints under `/v1/*` keep their OpenAPI-described JSON response shapes.

OAuth discovery routes:

- `/.well-known/oauth-protected-resource`
- `/.well-known/oauth-authorization-server`
- `/.well-known/openid-configuration`
- `/oauth/authorize`
- `/oauth/token`

The ntfy confirmation callback routes are intentionally not part of `openapi/hub.yaml`. They are only used by confirmation action buttons.

## Local Agent WebSocket

Local agents connect to:

```text
GET /v1/agents/{agentId}/connect
```

The Hub sends command messages over this WebSocket. The local agent sends hello, heartbeat, session updates, command responses, and confirmation requests back to the Hub.

## Local Agent HTTP/SSE transport

Local agents may opt into the HTTP/SSE transport with `hubTransport: "sse"`. WebSocket remains the default transport.

SSE endpoints are agent-private and use the same `x-agent-secret` authentication as WebSocket:

```text
GET  /v1/agents/{agentId}/events?connectionId=...
POST /v1/agents/{agentId}/messages?connectionId=...
```

The SSE transport gives reliable ack/replay semantics to request/response-style `HubCommand` messages. The Hub sends a command envelope containing `eventId`, `runId`, `requestId`, `commandHash`, and the original `HubCommand`. The agent writes the accepted command to its local transport ledger before sending `TransportAck`; command results include `runId` and are accepted as late results when `agentId`, `runId`, and `requestId` match, even if the original SSE connection is stale.

`Hello`, `Heartbeat`, `HeartbeatAck`, confirmation messages, and `SessionUpdate` remain best-effort/legacy messages in V1. `StartSession` itself is a reliable request/response command; later session state updates continue to use the existing session cache plus `inspectSession`/`waitSession` queries.

On agent restart, the local ledger is reconciled as follows: completed runs resend their result, accepted-but-not-started runs continue execution from the stored command, and started/running runs without a completed result report `unknown` instead of replaying side effects. The Hub also marks acked runs without a status/result as `unknown` after a timeout so callers can query a terminal state via `/v1/runs/{runId}`.
