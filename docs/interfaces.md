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
- `POST /v1/mcp/servers`: list MCP servers configured inside one local agent.
- `POST /v1/mcp/tools`: list tools exposed by one MCP server.
- `POST /v1/mcp/callTool`: call one MCP tool through the selected local agent.

`/v1/info` intentionally returns only safe metadata: Hub version, public base URL, timeout settings, remote confirmation status, agent counts, and pending request/session counts. It must not expose secrets, confirmation callback URLs, or private config values.

`/v1/agents` returns one safe config summary per enabled local agent. When an agent is online, the summary includes coarse sandbox mode, confirmation provider, path policy roots, configured command policy rules, and builtin command policy rules. Path roots are display paths such as `workspace`, `~/Documents`, or `/tmp`; private home paths should be shortened with `~` where possible. Offline agents may return an `unknown` summary because the Hub does not persist the last local config summary. Local confirmation prompts can use English or Simplified Chinese via `confirmationLanguage` (`en` or `zh-CN`).

## ChatGPT Apps MCP endpoint

`/mcp` is the Apps-friendly MCP endpoint. It is protected by the Hub OAuth shim and forwards MCP requests to the configured local agent and local MCP server.

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
