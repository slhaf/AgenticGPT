# Public tool contract matrix

This matrix describes the current public tool contracts; the live descriptors
and typed request objects remain authoritative. “No use” means the nearest
tempting operation that this tool deliberately does not perform. Bounds are
inclusive unless stated otherwise.

The deterministic Agent corpus and `scripts/check_contract_parity.py` are the
runtime/schema authorities for current behavior. This matrix is descriptive:
the prediction-shape probe at `scripts/evaluate_tool_contracts.py` cannot prove
schema validity or dispatch.

For current remote Room callers, coordinate the paired Hub and Agent artifacts
and refresh the imported schema from [`../openapi/hub.yaml`](../openapi/hub.yaml).
The nine current Room operations use the existing semantic Agent DTOs and
camelCase JSON bodies: Diary active/read, Notebook recent/search/read, State
list/read, and maintenance status/submit. Empty requests use `{}`; no remote
Room operation accepts `agentId`. `room.notebook.recent` and
`room.notebook.search` now return current Markdown previews/results, not the
retired passage-oriented JSONL shape.

## Agent admission and effect boundary (WP2)

This matrix describes public tool contracts; admission is enforced by the
Agent's internal `RequestContext`/operation gate, not by this document or by
descriptor annotations. The context records the real ingress and borrowed
operation name. Namespace/toolset selection and RuntimeModel capability are
separate checks, and `read_only`/`destructive`/`open_world` remain discovery
metadata rather than authorization.

Local Unix, Tunnel stdio, worker HTTP MCP, Hub command, and CLI paths retain
distinct framing/auth/error envelopes. Their Agent audit source prefixes are
`local:`, `tunnel:`, `http:`, `hub:`, and `localadmin:` respectively. The CLI
surface is only the four existing local tmux administration operations:
`tmux.listSessions`, `tmux.attach`, `tmux.createSession`, and
`tmux.closeSession`; MCP `tmux.sessions`, `tmux.panes`, `tmux.exec`, and
`tmux.pasteText` are separate MCP operations, not CLI aliases. Shared Skill
execution and Process result projections remain owned by the Agent
operation/resource layers.

Normal does not imply Room: Normal may use Room only with an explicit
`toolsets.room` namespace enablement. Hub retains its existing Room toolset,
Skills capability/profile, and notifications capability behavior. Actual
effects remain with policy, path, confirmation, lease, and resource owners;
external MCP, Browser JavaScript, tmux, tunnel children, and browser-host
effects are not thereby claimed to have generic OS sandbox coverage.

## Standalone advertised surface and namespace presets

The standalone advertised surface has Normal and Room presets. The normal
preset enables every namespace except `room`; the room preset enables every
namespace. Available namespaces are `agent`, `file`, `mcp`, `process`, `skills`,
`tmux`, `browser`, and `room`. The logical `room` namespace includes
`bootstrap`, `bootstrap.read`, semantic read tools, and maintenance
status/submit. An explicit `toolsets.enabled` selection is authoritative
regardless of profile, and filters only these advertised names; it never
exposes dispatch-only aliases.

Tunnel stdio and local Unix MCP use the same descriptors, schemas,
confirmation, path policy, audit, and Process registry. Standalone calls do
not accept Hub-only `agentId` or `confirmMethod` fields.

| Public name | Use / no use | Required or conditional inputs | Defaults and bounds | Failure / lifecycle | Surface parity |
|---|---|---|---|---|---|
| `agent.info` | Inspect local runtime; no execution or mutation. | No required fields. | Bounded diagnostics and safe config summary. | Read-only snapshot; live Process/config state may change after return. | Normal + Room; Tunnel = local Unix. |
| `browser.manual` | Read or search the selected runtime's official Browser documentation; no semantic Browser operation. | `action`; `read` requires `path` with optional `startLine`/`endLine`, while `search` requires `query` with optional `maxResults`/`contextLines`; action-incompatible fields are rejected. | Docs-root-relative bounded reads; search `maxResults` is 1–100 and `contextLines` is 0–5. | Runtime absence and manual bounds/path errors are stable Browser errors; read-only and non-destructive. | Normal + Room; uses the selected runtime docs root. |
| `browser.acquire` | Acquire or reuse one named persistent Browser JavaScript lease; no semantic Browser call. | `name`, `idleTimeoutSeconds`. | Idle timeout is 1–86,400 seconds; same name is idempotent. | Missing runtime or manager failures are stable Browser errors; lease state is ready on success. | Normal + Room; same manager in both ingress paths. |
| `browser.repl` | Run arbitrary JavaScript in a persistent Browser lease; no Rust-side Browser semantic translation. | `name`, non-empty `code`; optional `timeoutMs` and observability `title`. | Code ≤256 KiB UTF-8 bytes; timeout default 20,000 ms and bounded to 1–120,000; title ≤128 Unicode scalars. | Official text/image/structured content, error state, and metadata pass through; use acquire first. Destructive and open-world. | Normal + Room; same persistent lease manager in both ingress paths. |
| `browser.reset` | Recover/admin-reset one Browser lease; no normal per-call cleanup. | `name`. | Preserves lease identity while resetting/rebootstrapping kernel state. | Missing runtime, lease, or recovery failures are stable Browser errors; destructive but not open-world. | Normal + Room; same manager in both ingress paths. |
| `browser.release` | Perform final cleanup for one Browser lease; no implicit reuse after release. | `name`. | Bounded turn-ending/shutdown cleanup; absent names return `released: false`. | Cleanup failures remain stable Browser errors while the lease is removed; destructive but not open-world. | Normal + Room; same manager in both ingress paths. |
| `browser.list` | Discover bounded Browser runtime/lease state; no mutation. | No fields. | Reports runtime availability, version/channel, sorted lease names, lower-case state, idle timeout, and bounded remaining idle seconds. | Missing runtime succeeds with `runtimeAvailable: false` and no leases; read-only and non-destructive. | Normal + Room; opaque IDs and paths remain hidden. |
| `file.read` | Read UTF-8 text or supported raster images and optional metadata; no shell, search process, or write. | Flat `path` form or ordered `requests` of the same shape (1–32), mutually exclusive; optional `metadata`, inclusive `startLine`/`endLine` for text. | Text behavior and bounds are unchanged. PNG/JPEG/WebP/GIF images return top-level MCP image Content blocks plus JSON `structuredContent` metadata (no base64 duplication). Static `image` metadata includes detected MIME and dimensions; GIF metadata has canvas dimensions and ordered PNG `frames` with source timestamps. Batch `results` retain input order with zero-based `index`; top-level content follows item order, then frame order. Per image/frame decode limit: 16 Mi pixels; GIF traversal: 64 Mi cumulative pixels; serialized image payload: 8 MiB per `tools/call`. GIF returns at most 8 duration-uniform samples including playback endpoints, with source playback frame-start timestamps in milliseconds. | Typed path/UTF-8/size/oversized-line and image decode/limit errors; ordered batch partial-error semantics; retry-safe and non-destructive. | Normal + Room; same file schema in both ingress paths. |
| `file.search` | In-process literal/regex search; no shell or external search fallback. | Flat `path`/`query` form or ordered `requests` of the same shape (1–32), mutually exclusive; optional mode/globs/context/limits. | Normal success returns matches only; clipping/truncation/skipped-file evidence is conditional. Per-search limits plus 20k-file/128 MiB aggregate scan and ~1 MiB response bounds apply. | Invalid regex/glob/path or typed argument errors; read-only and bounded. | Normal + Room; same search schema in both ingress paths. |
| `file.edit` | Apply a complete Codex apply-patch patch across UTF-8 files; no model-supplied revision guards. | `patch` plus optional `needConfirm`; patch supports Add/Delete/Update/Move across multiple files. | One complete preflight, deterministic locks, one confirmation, internal source revalidation, bounded diff, and atomic/temp commits. Add File may create missing parent directories only within the existing path policy; Move semantics are unchanged. | Path/context/UTF-8/size/race/confirmation failures write no file contents before commit; Add-created empty parent directories are cleaned up on preflight rejection or confirmation failure. Once physical commit begins, partial failures are reported in order without cross-file rollback; audit retains internal revisions without exposing them in the response. | Normal + Room; standalone only. |
| `process.exec` | Start one policy-controlled local process; no direct unbounded shell API. | `program`; optional `group`, direct `args`, `workingDirectory`, `waitSeconds`, `needConfirm`. | Creation input is inline up to 8 KiB; larger creation inputs use a shared preview capped at 2 KiB. Wait is bounded at 30 seconds; `group` is a validated human-readable workstream key. | Policy/confirmation/capacity/spawn/exit outcomes are retained in Process history; use `process.status`, `process.output`, and `process.result` to inspect. | Normal + Room; Hub full `process.exec` mirrors. |
| `process.batch` | Admit multiple managed processes; no implicit sibling rollback after admission. | `elements` (each requires `program`); optional parent `group`, batch cwd/wait/confirmation. | One admission/confirmation boundary; ordered children inherit the parent group; wait ≤30 seconds. | Validation/capacity rejection starts none; post-admission child failures remain per child and are retained as Processes. | Normal + Room; Hub full `process.batch` mirrors. |
| `process.status` | Inspect or briefly wait for one Process; no new work and no output/result body. | `processId`; optional `waitSeconds`. | Standalone wait defaults to 5 seconds and is clamped to 30; explicit `0` polls without waiting. | Returns bounded status metadata only; use `process.output` and `process.result` for payloads. | Normal + Room; Hub full `process.status`; HTTP status defaults to 5 seconds. |
| `process.list` | Discover active/recent Processes; no mutation or admission. | No required fields; optional `group`, `kind`, `state`, and cursor. | Returns a bounded page of Process metadata. | Read-only; rows do not contain output or result bodies. | Normal + Room; Hub full `process.list`; HTTP `GET /v1/process` mirrors. |
| `process.output` | Read bounded captured output for one Process; no new work. | `processId`; optional cursor and `maxBytes`. | Starts at the requested cursor (or the beginning if omitted); `maxBytes` defaults to 8 KiB and is capped at 32 KiB. | Returns a bounded output chunk and continuation cursor when more output is available. | Normal + Room; Hub full `process.output`; HTTP `GET /v1/process/{processId}/output` mirrors. |
| `process.result` | Read the current result projection for one Process; no new work. | `processId`; optional `maxBytes`. | MCP `maxBytes` defaults to 8 KiB and is capped at 512 KiB; HTTP defaults to 8 KiB and is capped at 512 KiB. | Returns truthful current Process status and includes a result only when available; status metadata alone never carries the result body. | Normal + Room; Hub full `process.result`; HTTP `GET /v1/process/{processId}/result` mirrors. |
| `process.cancel` | Request Process cancellation; does not claim unobserved termination. | `processId`. | Explicit request; no wait argument. | Reports observed cancellation/evidence and preserves unknown or detached outcomes. | Normal + Room; Hub full `process.cancel`; HTTP `POST /v1/process/{processId}/cancel` mirrors. |
| `mcp.list` | Discover configured downstream servers or one server's tools; no downstream execution. | Optional `serverId`; omit to list servers. | Bounded server/tool metadata. | Config/transport errors are typed and read-only. | Normal + Room; Hub full uses split `mcp.listServers`/`mcp.listTools`. |
| `mcp.callTool` | Start one downstream MCP call as a managed Process; no direct transactional call. | `serverId`, `toolName`; optional `group`, JSON-object `arguments`, `waitSeconds`, `timeoutSeconds`. | Arguments ≤256 KiB; wait default 5/max 30; a wait timeout does not cancel the Process; timeout default 300/max 900 seconds. | Confirmation/policy/transport/downstream errors are retained; oversized results keep hash/size/preview; follow up with `process.*`. | Normal + Room; Hub full mirrors Process lifecycle semantics. |
| `mcp.batch` | Validate/admit 1–16 downstream calls with one aggregate confirmation; no rollback of downstream side effects. | `calls` with `serverId`/`toolName`; optional parent `group`, per-call arguments plus `mode`, `failFast`, waits/deadline. | Parallel default; sequential is explicit; aggregate args/response ≤2 MiB; global/per-server concurrency 8/2. Children inherit the parent group; public results are ordered so correlation ids/indexes stay internal. | Admission is atomic; `failFast` skips only not-started children; ordered child Processes and aggregate audit remain. | Normal + Room; Hub full mirrors admission, group, and bounds. |
| `skills.list` | Discover valid workspace skills; no install or execution. | Optional query/limit/active filter. | Bounded summaries. | Invalid/unreadable skills become warnings or omitted; read-only. | Normal + Room; Hub full uses the same Room workspace. |
| `skills.read` | Read one skill package/resource; no arbitrary workspace file access. | `id`; optional package-relative `path`. | Bounded Markdown/frontmatter/resource. | Invalid/missing skill/resource is typed; read-only. | Normal + Room; Hub full `skills.read` mirrors. |
| `skills.setActive` | Set active flag only; no execution or permission grant. | `id`, `active`. | Active state persists in Agent private durable state; workspace package contents remain separate. | Invalid IDs/skills are typed; state change is audited. | Normal + Room; Hub full exposes `skills.activate`/`skills.deactivate` aliases with split intent. |
| `skills.install` | Start asynchronous skill installation; no inline network payload or arbitrary URL fetch. | `id`, `source`; optional replacement/activation/idempotency. | Returns `installId`; source and package/file bounds apply. | Validate/commit failures are retained; existing skill archive/commit is atomic; use install get/cancel. | Normal + Room; Hub full and HTTP Room install mirror. |
| `skills.install.get` | Inspect or briefly wait for installation; no new install. | `installId`; optional `waitSeconds`. | Wait default 5, maximum 30 seconds; a wait timeout does not cancel installation; terminal `pollAfterMs` is 0. | Bounded persisted status; missing/expired IDs are typed; use `skills.install.cancel` for cancellation. | Normal + Room; Hub full/HTTP install get mirror. |
| `skills.install.cancel` | Request cooperative pre-commit cancellation; no forced rollback after commit. | `installId`. | Idempotent request; it is explicit, never implicit in a wait timeout. | Outcome distinguishes cancelled/terminal/too-late; evidence is retained. | Normal + Room; Hub full/HTTP install cancel mirror. |
| `skills.run` | Run an executable under an active skill as a managed Process; no arbitrary path. | `id`, package-relative `path`; optional `group`, args/cwd/wait. | Wait default 5, maximum 30 seconds; a wait timeout does not cancel the Process; use `process.cancel` explicitly. | Policy/confirmation/script/exit failures are Process states; use `process.status`/`process.result`. | Normal + Room; Hub full/HTTP skills run mirror group and Process lifecycle semantics. |
| `tmux.sessions` | List/create/close persistent sessions; no command submission. | `action`; create/close require action-compatible name/cwd; close may require confirmation. | Reuse default session where possible; policy-checked cwd. | Close is destructive; typed session/policy/confirmation errors. | Normal + Room; Hub full uses split tmux names. |
| `tmux.panes` | List/capture panes; no input submission. | `action`; capture requires target; list may filter session. | Capture history default 160 lines and bounded. | Action-incompatible fields are rejected, not ignored. | Normal + Room; Hub full uses split tmux names. |
| `tmux.exec` | Submit structured command to a shell pane; no claim that submission completed. | `target`, `program`; optional args/wait/capture/confirmation. | Bounded post-submit wait/history. | Shell/policy/confirmation errors; inspect pane or process result for completion. | Normal + Room; Hub full `tmux.exec` mirrors. |
| `tmux.pasteText` | Paste into non-shell pane/TUI; no shell execution. | `target`, `text`; optional `submit`, confirmation. | Text/history bounded. | Shell panes are rejected; pane state remains otherwise unchanged. | Normal + Room; Hub full mirrors. |
| `bootstrap` | Load Room bootstrap entrypoint/guide manifest; no generic file read or file creation. | No fields. | Bounded guide summaries and package revision. | Missing/invalid package is typed/warned; read-only and retry-safe. | Room only standalone; Hub full has `bootstrap` and `room.bootstrap` routes. |
| `bootstrap.read` | Read one validated bootstrap guide; no arbitrary path. | `id`. | Bounded Markdown/frontmatter. | Unknown/invalid/duplicate guide is `guide_not_found`; read-only. | Room only standalone; Hub full has `bootstrap.read` and `room.bootstrap.read`. |
| `room.diary.active` | Read the active Daily, Weekly, and Monthly Room diary documents; no mutation. | Empty JSON object. | Three bounded Markdown layer results; each reports a validated path, availability, and optional typed issue. | Missing, unreadable, or invalid-UTF-8 documents are reported per layer; read-only and retry-safe. | Standalone Room + Hub Full active-Room route; Coordinator hidden/rejects. |
| `room.diary.read` | Read one exact Room diary document by semantic layer and period; no arbitrary path. | `layer`, `period`; period is `current`, a daily date, or an ordered weekly/monthly range. | One bounded Markdown document. | Invalid periods are rejected; missing or unreadable documents are returned as typed layer issues. | Standalone Room + Hub Full active-Room route; Coordinator hidden/rejects. |
| `room.notebook.recent` | Read bounded recent current Room Notebook Markdown previews; no mutation. | Optional `limit`. | Limit defaults to 20 and is bounded to 1–100; previews are capped. | Missing or malformed documents become bounded warnings; read-only discovery. The public name now uses the current Markdown DTO, not the retired passage/JSONL shape. | Standalone Room + Hub Full active-Room route; Coordinator hidden/rejects. |
| `room.notebook.search` | Search current Room Notebook Markdown by a case-insensitive substring; no mutation. | Required `query`; optional `limit`. | Query is non-empty and capped at 256 Unicode characters; limit defaults to 20 and is bounded to 1–100. | Empty or oversized queries and invalid limits are typed validation errors; read-only. The public name now uses the current Markdown DTO, not the retired passage/JSONL shape. | Standalone Room + Hub Full active-Room route; Coordinator hidden/rejects. |
| `room.notebook.read` | Read one exact current Room Notebook Markdown document; no arbitrary repository path. | Required validated Notebook-relative `.md` `path`. | One bounded Markdown document; content is capped at 512 KiB. | Unsafe, non-Markdown, missing, or oversized paths are typed; read-only. | Standalone Room + Hub Full active-Room route; Coordinator hidden/rejects. |
| `room.state.list` | List deterministic Room state entity documents; no mutation. | Empty JSON object. | Returns sorted `.md` entities under `State/entities`. | Symlinks and non-files are skipped; malformed repository roots are typed; read-only. | Standalone Room + Hub Full active-Room route; Coordinator hidden/rejects. |
| `room.state.read` | Read one exact Room state entity Markdown document; no arbitrary path. | Required safe entity filename stem `entity`. | One bounded Markdown document under `State/entities`; content is capped at 512 KiB. | Unsafe, missing, or oversized entities are typed; read-only. | Standalone Room + Hub Full active-Room route; Coordinator hidden/rejects. |
| `room.maintenance.status` | Inspect Room repository, scaffold, executor, workflow, remote, sync, and slot readiness; no mutation. | Empty JSON object. | Bounded status, heads, missing paths, and five-slot occupancy. | Read-only; readiness dimensions remain independent and failures are typed. | Standalone Room + Hub Full active-Room route; Coordinator hidden/rejects. |
| `room.maintenance.submit` | Apply one to five validated Room maintenance requests through the repository-owned executor. | `items` with unique `slot`/`payload`; optional `mode` and `waitSeconds`. | Items are bounded to 1–5; mode is `local` or `workflow`; wait defaults to 0 and is capped at 30 seconds. | Admission, local apply, semantic commit, and remote/workflow sync are reported independently; destructive but repository-confined. A wait timeout does not cancel maintenance and there is no separate wait API. | Standalone Room + Hub Full active-Room route; Coordinator hidden/rejects. |

The nine current Room names are one semantic contract across local Agent MCP,
Hub Full MCP, and the nine `/v1/room/<namespace>/<action>` POST routes. Hub
selects only the captured active Room lease and owns routing/receipts, not Room
files or content. A generic run receipt may contain a bounded operation result,
but it is not a Room content authority.

## Hub full and coordinator surfaces

The Hub full profile contains the execution surface below. The coordinator
profile contains only `hub.info`, `agent.list`, `hub.run.list`, `hub.run.get`,
`hub.process.status`, `hub.process.list`, `user.notify.channels`, and
`user.notify.send`; it never dispatches an Agent command. Hub tools use
`agentId` where shown, while active Room tools intentionally route to the
active Room Agent and do not take it.

| Hub public name(s) | Use / no use | Required or conditional inputs | Defaults and bounds | Failure / lifecycle | Parity |
|---|---|---|---|---|---|
| `hub.info`, `agent.list` | Inspect Hub/agent availability and safe summaries; no execution. | `agent.list` has no body; `hub.info` has no body. | Safe counts/config summaries only. | Read-only; offline agents are reported unknown, not healthy. | Coordinator + Full; no standalone alias. |
| `hub.run.list`, `hub.run.get` | Inspect persisted Hub-to-Agent request runs; no new dispatch. | `run.get` requires `runId`; list filters are optional. | Bounded retained history/results. | Timeout/delivery/late-result states remain explicit. | Coordinator + Full; HTTP run endpoints mirror. |
| `hub.process.status`, `hub.process.list` | Inspect cached Process metadata without dispatching execution. | `agentId`; status also requires `processId`; list supports Process filters/cursor. | Cache-only snapshot; status/list are metadata-only and never include output or result bodies. | Freshness and observation time are explicit; snapshots are not a live wait result. | Coordinator + Full; HTTP process status/list endpoints are live Agent-backed views. |
| `process.exec`, `process.batch`, `process.status`, `process.list`, `process.output`, `process.result`, `process.cancel` | Same managed Process semantics as standalone through selected `agentId`. | `agentId` plus standalone Process fields. | Status wait default 5/max 30 for Hub Full; output chunk default 8 KiB/max 32 KiB; result max 512 KiB. | Status is metadata-only; output/result use their dedicated calls. | Full only; HTTP `/v1/process`, `/v1/process/{processId}`, `/output`, `/result`, and `/cancel` lifecycle endpoints mirror. |
| `tmux.listSessions`, `tmux.listPanes`, `tmux.capturePane` | Discover/read persistent panes; no input mutation. | `agentId`; capture/pane targets as applicable. | Capture defaults 160 lines; bounded output. | Read-only typed pane/session errors. | Full only; standalone combines aliases. |
| `tmux.pasteText`, `tmux.exec` | Paste non-shell input or submit shell command through selected Agent. | `agentId` and action-specific target/text/program. | Bounded wait/history. | Shell-vs-non-shell and policy/confirmation boundaries are explicit. | Full only; same local semantics. |
| `tmux.createSession`, `tmux.closeSession` | Create/close persistent workspace; no generic process lifecycle. | `agentId`, name/cwd; close may confirm. | Policy-checked cwd; reuse preferred. | Close destructive; no implicit data recovery. | Full only; same local semantics. |
| `mcp.listServers`, `mcp.listTools` | Discover downstream MCP routing/schema before a managed call. | `mcp.listServers` may omit `agentId` to group connected agents; listTools requires `agentId`,`serverId`. | Bounded metadata. | Read-only timeout/agent errors. | Full only; HTTP `/v1/mcp/servers|tools` mirrors. |
| `mcp.callTool` | Start one downstream MCP call as a managed Process; not a native Agent file tool. | `agentId`, `serverId`, `toolName`; optional `group`, JSON-object `arguments`, bounded wait/deadline. | Returns a Process response; use `process.status`, `process.output`, `process.result`, or `process.cancel` for lifecycle. | Hub has no native `file.read` or `file.edit`. The generic Process bridge is not a typed MCP image-content surface and must not be relied on to preserve `file.read` image Content blocks. | Full only; HTTP `/v1/mcp/callTool` mirrors. |
| `room.bootstrap`, `room.bootstrap.read` | Active Room bootstrap manifest/guide access; no arbitrary file read. | Read requires guide `id`; no `agentId`. | Same package bounds/revision as standalone Room bootstrap. | Room inactive/invalid/not-found errors are explicit and read-only. | Full only; standalone names omit `room.` prefix. |
| `room.diary.active`, `room.diary.read` | Read active or exact semantic Diary Markdown through the captured active Room lease; no mutation or arbitrary path access. | `diary.active` uses `{}`; `diary.read` requires `layer` and `period`; no `agentId`. | Bounded layer results; daily/weekly/monthly period rules are enforced by the Agent. | Active Room absent is `room_not_active` (404), lease conflict is `room_state_conflict` (409), transport wait is 504, and Agent semantic errors remain JSON errors. | Full only; Coordinator hides and rejects Room tools. HTTP `/v1/room/diary/active|read` mirrors. |
| `room.notebook.recent`, `room.notebook.search`, `room.notebook.read` | Read current Notebook Markdown previews, search current Markdown, or read one exact validated document through the active Room lease. | `recent` optional `limit`; `search` requires `query` plus optional `limit`; `read` requires `path`; no `agentId`. | Limit defaults 20 and is 1–100; search query is non-empty and ≤256 Unicode characters; Markdown content is bounded to 512 KiB. `recent`/`search` return current Markdown DTOs, not retired passage/JSONL results. | Missing/malformed files become bounded warnings where specified; unsafe paths and validation errors are typed. Active lease/transport statuses follow the current Room projection. | Full only; Coordinator hides and rejects Room tools. HTTP `/v1/room/notebook/recent|search|read` mirrors. |
| `room.state.list`, `room.state.read` | List or read bounded non-symlink Markdown entities under the Agent-owned `State/entities` root. | `list` uses `{}`; `read` requires safe entity stem `entity`; no `agentId`. | Sorted entity list; read content is bounded to 512 KiB. | Unsafe/missing entities and malformed roots are typed; active lease/transport statuses follow the current Room projection. | Full only; Coordinator hides and rejects Room tools. HTTP `/v1/room/state/list|read` mirrors. |
| `room.maintenance.status` | Inspect Agent-owned repository, schema/scaffold, executor, workflow/remote, sync, and five-slot readiness; no mutation. | `{}`; no `agentId`. | Bounded status and optional heads/missing paths. | Read-only; readiness dimensions stay independent and errors remain explicit. Active lease/transport statuses follow the current Room projection. | Full only; Coordinator hides and rejects Room tools. HTTP `/v1/room/maintenance/status` mirrors. |
| `room.maintenance.submit` | Apply explicit semantic maintenance through the existing Agent repository owner; not a passage append/update/remove alias. | `items` (1–5 unique `slot`/`payload`); optional `mode` (`local`/`workflow`) and `waitSeconds` (0–30); no `agentId`. | Wait defaults to 0; local/workflow result and sync outcomes remain distinct. A wait timeout does not cancel maintenance; no separate wait API or new confirmation gate is implied. | Existing lock, clean-tree, path, expected-change, executor, Git, and workflow controls remain authoritative. Active lease/transport statuses follow the current Room projection. | Full only; Coordinator hides and rejects Room tools. HTTP `/v1/room/maintenance/submit` mirrors. |
| `bootstrap`, `bootstrap.read` | Full-profile transport-neutral aliases for Room bootstrap. | Read requires `id`; no `agentId`. | Same package bounds/revision. | Same bootstrap errors; read-only. | Full only; aliases are intentional bootstrap names, not compatibility for removed tools. |
| `skills.list`, `skills.read`, `skills.search`, `skills.active` | Active Room skill discovery/read/search. | Read/search fields as applicable; no `agentId`. | Bounded summaries/content. | Invalid/missing/stale skills are explicit. | Full only; HTTP Room skills endpoints mirror. |
| `skills.activate`, `skills.deactivate` | Change active skill state only; no execution/permission grant. | `id`; no `agentId`. | Idempotent state operation. | Stale/missing deactivation is allowed and reported. | Full only; standalone `skills.setActive` combines intent. |
| `skills.install`, `skills.install.get`, `skills.install.cancel` | Asynchronous active-Room installation lifecycle. | Install requires `id`,`source`; get/cancel require `installId`; no `agentId`. | Wait default 5, maximum 30; wait timeout does not cancel; cancellation is explicit. | Cooperative cancellation/atomic commit evidence. | Full only; HTTP Room install endpoints mirror. |
| `skills.run` | Run active skill executable as managed Process. | `id`,`path`; optional `group`, args/cwd/wait; no `agentId`. | Wait default 5, maximum 30; wait timeout does not cancel; use `process.cancel`. | Same Process/policy/cancellation contract as standalone. | Full only; HTTP Room skills run mirrors group and Process lifecycle semantics. |

For the nine HTTP routes, malformed JSON or missing required request fields
are `422 text/plain` extractor responses. Authenticated semantic responses use
the existing JSON projection: inactive Room `room_not_active` is 404, lease
conflict `room_state_conflict` is 409, transport wait is 504, and other
Agent-side semantic validation remains 400 unless an existing selected
not-found/conflict mapping applies.

## Review rules

- A descriptor change must update the relevant row and its focused parity test;
  do not add a compatibility alias to make an invalid call appear valid.
- Required fields describe admission, not a promise that execution succeeds.
  Conditional fields must be stated in the tool description or property
  description, especially revision/absence guards, action-specific tmux fields,
  and Process follow-up.
- “Atomic” is reserved for validation/admission/confirmation boundaries. It
  never implies rollback of an already-started process, MCP call, notification,
  or other external side effect.
- Durable Agent Process history and retention belong to `process.sqlite3`; legacy `jobs.sqlite3` is intentionally untouched and inaccessible.
