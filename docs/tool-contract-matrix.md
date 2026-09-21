# Public tool contract matrix

This is the checked-in Phase D matrix for the frozen D-11 surface. It is a
review aid, not a second schema: the live descriptors and typed request
objects remain authoritative. “No use” means the nearest tempting operation
that this tool deliberately does not perform. Bounds are inclusive unless
stated otherwise.

The deterministic Agent corpus and `scripts/check_contract_parity.py` are the
runtime/schema authorities for current behavior. This matrix is descriptive:
the prediction-shape probe at `scripts/evaluate_tool_contracts.py` cannot prove
schema validity or dispatch.

When upgrading a caller to the corrected Room request projection, coordinate
the Hub/Agent artifacts and refresh the imported schema from
[`../openapi/hub.yaml`](../openapi/hub.yaml). `room.notebook.append` requires
`scope` and `content`; `significance` defaults to `NORMAL`, while `datetime`,
`abstract`, and `tags` are optional. `room.notebook.selectExact` requires
`date`. These are request-shape changes, not a claim that the current Agent
executes the legacy commands; current legacy calls return
`room_legacy_surface_removed` and remain assigned to WP-R.

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
execution and Job result projections remain owned by the Agent
operation/resource layers.

Normal does not imply Room: Normal may use Room only with an explicit
`toolsets.room` namespace enablement. Hub retains its existing Room toolset,
Skills capability/profile, and notifications capability behavior. Actual
effects remain with policy, path, confirmation, lease, and resource owners;
external MCP, Browser JavaScript, tmux, tunnel children, and browser-host
effects are not thereby claimed to have generic OS sandbox coverage.

## Standalone advertised surface and namespace presets

The V2 advertised surface contains 29 Normal names and 40 Room names. The normal
preset enables every namespace except `room`; the room preset enables every namespace. Available
namespaces are `agent`, `file`, `mcp`, `process`, `job`, `skills`, `tmux`, `browser`, and `room`. The
logical `room` namespace includes `bootstrap`, `bootstrap.read`, semantic read tools, and maintenance
status/submit. An explicit `toolsets.enabled` selection is authoritative regardless of profile,
and filters only these advertised names; it never exposes dispatch-only aliases.

Tunnel stdio and local Unix MCP use the same descriptors, schemas, confirmation, path policy,
audit, and Job registry. Standalone calls do not accept Hub-only `agentId` or `confirmMethod`
fields.

| Public name | Use / no use | Required or conditional inputs | Defaults and bounds | Failure / lifecycle | Surface parity |
|---|---|---|---|---|---|
| `agent.info` | Inspect local runtime; no execution or mutation. | No required fields. | Bounded diagnostics and safe config summary. | Read-only snapshot; live Job/config state may change after return. | Normal + Room; Tunnel = local Unix. |
| `browser.manual` | Read or search the selected runtime's official Browser documentation; no semantic Browser operation. | `action`; `read` requires `path` with optional `startLine`/`endLine`, while `search` requires `query` with optional `maxResults`/`contextLines`; action-incompatible fields are rejected. | Docs-root-relative bounded reads; search `maxResults` is 1–100 and `contextLines` is 0–5. | Runtime absence and manual bounds/path errors are stable Browser errors; read-only and non-destructive. | Normal + Room; uses the selected runtime docs root. |
| `browser.acquire` | Acquire or reuse one named persistent Browser JavaScript lease; no semantic Browser call. | `name`, `idleTimeoutSeconds`. | Idle timeout is 1–86,400 seconds; same name is idempotent. | Missing runtime or manager failures are stable Browser errors; lease state is ready on success. | Normal + Room; same manager in both ingress paths. |
| `browser.repl` | Run arbitrary JavaScript in a persistent Browser lease; no Rust-side Browser semantic translation. | `name`, non-empty `code`; optional `timeoutMs` and observability `title`. | Code ≤256 KiB UTF-8 bytes; timeout default 20,000 ms and bounded to 1–120,000; title ≤128 Unicode scalars. | Official text/image/structured content, error state, and metadata pass through; use acquire first. Destructive and open-world. | Normal + Room; same persistent lease manager in both ingress paths. |
| `browser.reset` | Recover/admin-reset one Browser lease; no normal per-call cleanup. | `name`. | Preserves lease identity while resetting/rebootstrapping kernel state. | Missing runtime, lease, or recovery failures are stable Browser errors; destructive but not open-world. | Normal + Room; same manager in both ingress paths. |
| `browser.release` | Perform final cleanup for one Browser lease; no implicit reuse after release. | `name`. | Bounded turn-ending/shutdown cleanup; absent names return `released: false`. | Cleanup failures remain stable Browser errors while the lease is removed; destructive but not open-world. | Normal + Room; same manager in both ingress paths. |
| `browser.list` | Discover bounded Browser runtime/lease state; no mutation. | No fields. | Reports runtime availability, version/channel, sorted lease names, lower-case state, idle timeout, and bounded remaining idle seconds. | Missing runtime succeeds with `runtimeAvailable: false` and no leases; read-only and non-destructive. | Normal + Room; opaque IDs and paths remain hidden. |
| `file.read` | Read UTF-8 content and optional metadata; no shell, search process, or write. | Flat `path` form or ordered `requests` of the same shape (1–32), mutually exclusive; optional `metadata`, inclusive `startLine`/`endLine`. | Content is default; `metadata=true` adds metadata. Reads stop on complete-line boundaries at 256 KiB and return `nextStartLine` only when continuation is needed. | Typed path/UTF-8/size/oversized-line errors; retry-safe and non-destructive. | Normal + Room; same file schema in both ingress paths. |
| `file.search` | In-process literal/regex search; no shell or external search fallback. | Flat `path`/`query` form or ordered `requests` of the same shape (1–32), mutually exclusive; optional mode/globs/context/limits. | Normal success returns matches only; clipping/truncation/skipped-file evidence is conditional. Per-search limits plus 20k-file/128 MiB aggregate scan and ~1 MiB response bounds apply. | Invalid regex/glob/path or typed argument errors; read-only and bounded. | Normal + Room; same search schema in both ingress paths. |
| `file.edit` | Apply a complete Codex apply-patch patch across UTF-8 files; no model-supplied revision guards. | `patch` plus optional `needConfirm`; patch supports Add/Delete/Update/Move across multiple files. | One complete preflight, deterministic locks, one confirmation, internal source revalidation, bounded diff, and atomic/temp commits. | Context/path/UTF-8/size/race/confirmation failures write nothing before commit; audit retains internal revisions without exposing them in the response. | Normal + Room; standalone only. |
| `process.exec` | Start one policy-controlled local process; no direct unbounded shell API. | `program`; optional `group`, direct `args`, `workingDirectory`, `waitSeconds`, `needConfirm`. | Wait is bounded at 30 seconds; `group` is a validated human-readable workstream key. | Policy/confirmation/capacity/spawn/exit errors are retained in the Job; use `job.get`/`job.cancel`. | Normal + Room; Hub full `process.exec` mirrors group and lifecycle semantics. |
| `process.batch` | Admit multiple managed processes; no implicit sibling rollback after admission. | `elements` (each requires `program`); optional parent `group`, batch cwd/wait/confirmation. | One admission/confirmation boundary; ordered children inherit the parent group; wait ≤30 seconds. | Validation/capacity rejection starts none; post-admission child failures remain per child. | Normal + Room; Hub full `process.batch` mirrors semantics. |
| `job.get` | Inspect or briefly wait for one managed Job; no new work. | `jobId`; optional `waitSeconds`, `waitOnly`. | Wait default 0, maximum 30 seconds; `waitOnly=true` suppresses active intermediate detail while waiting. | `jobId` remains the stable handle for retained terminal history; fresh or retained state is bounded. | Normal + Room; Hub full `job.get` and HTTP job GET are equivalent lifecycle views. |
| `job.list` | Discover active/recent Jobs; no mutation or admission. | No required fields; optional exact `group`, `kind`, `state`, `limit`, opaque `cursor`. | Default limit 50, maximum 100; stable order is `createdAt DESC, jobId DESC`. | Read-only live+history view; Hub cache may filter the first page but cannot continue an Agent-issued cursor while the Agent is unavailable. | Normal + Room; Hub full `job.list` and HTTP jobs list mirror filters/cursor online. |
| `job.cancel` | Request kind-aware cancellation; no guarantee of remote termination. | `jobId`. | One request per Job; no wait argument. | Returns observed outcome/evidence; unconfirmed remote stop is `detached`, not `cancelled`. | Normal + Room; Hub full `job.cancel` and HTTP cancel mirror evidence. |
| `mcp.list` | Discover configured downstream servers or one server's tools; no downstream execution. | Optional `serverId`; omit to list servers. | Bounded server/tool metadata. | Config/transport errors are typed and read-only. | Normal + Room; Hub full uses split `mcp.listServers`/`mcp.listTools`. |
| `mcp.callTool` | Start one downstream MCP call as a managed Job; no direct transactional call. | `serverId`, `toolName`; optional `group`, JSON-object `arguments`, `waitSeconds`, `timeoutSeconds`. | Arguments ≤256 KiB; wait default 5/max 30; a wait timeout does not cancel the Job; timeout default 300/max 900 seconds. | Confirmation/policy/transport/downstream errors are retained; oversized results keep hash/size/preview; follow up with `job.*` or `job.cancel`. | Normal + Room; Hub full mirrors group and Job lifecycle semantics. |
| `mcp.batch` | Validate/admit 1–16 downstream calls with one aggregate confirmation; no rollback of downstream side effects. | `calls` with `serverId`/`toolName`; optional parent `group`, per-call arguments plus `mode`, `failFast`, waits/deadline. | Parallel default; sequential is explicit; aggregate args/response ≤2 MiB; global/per-server concurrency 8/2. Children inherit the parent group; public results are ordered so correlation ids/indexes stay internal. | Admission is atomic; `failFast` skips only not-started children; ordered child Jobs and aggregate audit remain. | Normal + Room; Hub full mirrors admission, group, and bounds. |
| `skills.list` | Discover valid workspace skills; no install or execution. | Optional query/limit/active filter. | Bounded summaries. | Invalid/unreadable skills become warnings or omitted; read-only. | Normal + Room; Hub full uses the same Room workspace. |
| `skills.read` | Read one skill package/resource; no arbitrary workspace file access. | `id`; optional package-relative `path`. | Bounded Markdown/frontmatter/resource. | Invalid/missing skill/resource is typed; read-only. | Normal + Room; Hub full `skills.read` mirrors. |
| `skills.setActive` | Set active flag only; no execution or permission grant. | `id`, `active`. | Active state persists in Agent private durable state; workspace package contents remain separate. | Invalid IDs/skills are typed; state change is audited. | Normal + Room; Hub full exposes `skills.activate`/`skills.deactivate` aliases with split intent. |
| `skills.install` | Start asynchronous skill installation; no inline network payload or arbitrary URL fetch. | `id`, `source`; optional replacement/activation/idempotency. | Returns `installId`; source and package/file bounds apply. | Validate/commit failures are retained; existing skill archive/commit is atomic; use install get/cancel. | Normal + Room; Hub full and HTTP Room install mirror. |
| `skills.install.get` | Inspect or briefly wait for installation; no new install. | `installId`; optional `waitSeconds`. | Wait default 5, maximum 30 seconds; a wait timeout does not cancel installation; terminal `pollAfterMs` is 0. | Bounded persisted status; missing/expired IDs are typed; use `skills.install.cancel` for cancellation. | Normal + Room; Hub full/HTTP install get mirror. |
| `skills.install.cancel` | Request cooperative pre-commit cancellation; no forced rollback after commit. | `installId`. | Idempotent request; it is explicit, never implicit in a wait timeout. | Outcome distinguishes cancelled/terminal/too-late; evidence is retained. | Normal + Room; Hub full/HTTP install cancel mirror. |
| `skills.run` | Run an executable under an active skill as a managed Job; no arbitrary path. | `id`, package-relative `path`; optional `group`, args/cwd/wait. | Wait default 5, maximum 30 seconds; a wait timeout does not cancel the Job; use `job.cancel` explicitly. | Policy/confirmation/script/exit failures are Job states; use `job.get`/`job.cancel`. | Normal + Room; Hub full/HTTP skills run mirror group and lifecycle semantics. |
| `tmux.sessions` | List/create/close persistent sessions; no command submission. | `action`; create/close require action-compatible name/cwd; close may require confirmation. | Reuse default session where possible; policy-checked cwd. | Close is destructive; typed session/policy/confirmation errors. | Normal + Room; Hub full uses split tmux names. |
| `tmux.panes` | List/capture panes; no input submission. | `action`; capture requires target; list may filter session. | Capture history default 160 lines and bounded. | Action-incompatible fields are rejected, not ignored. | Normal + Room; Hub full uses split tmux names. |
| `tmux.exec` | Submit structured command to a shell pane; no claim that submission completed. | `target`, `program`; optional args/wait/capture/confirmation. | Bounded post-submit wait/history. | Shell/policy/confirmation errors; inspect pane or process result for completion. | Normal + Room; Hub full `tmux.exec` mirrors. |
| `tmux.pasteText` | Paste into non-shell pane/TUI; no shell execution. | `target`, `text`; optional `submit`, confirmation. | Text/history bounded. | Shell panes are rejected; pane state remains otherwise unchanged. | Normal + Room; Hub full mirrors. |
| `bootstrap` | Load Room bootstrap entrypoint/guide manifest; no generic file read or file creation. | No fields. | Bounded guide summaries and package revision. | Missing/invalid package is typed/warned; read-only and retry-safe. | Room only standalone; Hub full has `bootstrap` and `room.bootstrap` routes. |
| `bootstrap.read` | Read one validated bootstrap guide; no arbitrary path. | `id`. | Bounded Markdown/frontmatter. | Unknown/invalid/duplicate guide is `guide_not_found`; read-only. | Room only standalone; Hub full has `bootstrap.read` and `room.bootstrap.read`. |
| `room.diary.active` | Read the active Daily, Weekly, and Monthly Room diary documents; no mutation. | No fields. | Three bounded Markdown layer results; each reports a validated path and availability. | Missing, unreadable, or invalid-UTF-8 documents are reported per layer; read-only and retry-safe. | Room only standalone; no legacy JSONL alias. |
| `room.diary.read` | Read one exact Room diary document by semantic layer and period; no arbitrary path. | `layer`, `period`; period is `current`, a daily date, or an ordered weekly/monthly range. | One bounded Markdown document. | Invalid periods are rejected; missing or unreadable documents are returned as typed layer issues. | Room only standalone; no legacy JSONL alias. |
| `room.notebook.recent` | Read bounded recent Room notebook Markdown previews; no mutation. | Optional `limit`. | Limit defaults to 20 and is bounded to 1–100; previews are capped. | Missing or malformed documents become bounded warnings; read-only discovery. | Room only standalone; no legacy JSONL alias. |
| `room.notebook.search` | Search Room notebook Markdown by a case-insensitive substring; no mutation. | Required `query`; optional `limit`. | Query is capped at 256 characters; limit defaults to 20 and is bounded to 1–100. | Empty or oversized queries and invalid limits are typed validation errors; read-only. | Room only standalone; no legacy JSONL alias. |
| `room.notebook.read` | Read one exact Room notebook Markdown document; no arbitrary repository path. | Required validated Notebook-relative `.md` `path`. | One bounded Markdown document. | Unsafe, non-Markdown, missing, or oversized paths are typed; read-only. | Room only standalone; no legacy JSONL alias. |
| `room.state.list` | List deterministic Room state entity documents; no mutation. | No fields. | Returns sorted `.md` entities under `State/entities`. | Symlinks and non-files are skipped; malformed repository roots are typed; read-only. | Room only standalone; no legacy JSONL alias. |
| `room.state.read` | Read one exact Room state entity Markdown document; no arbitrary path. | Required safe entity filename stem `entity`. | One bounded Markdown document under `State/entities`. | Unsafe, missing, or oversized entities are typed; read-only. | Room only standalone; no legacy JSONL alias. |
| `room.maintenance.status` | Inspect Room repository, scaffold, executor, workflow, remote, sync, and slot readiness; no mutation. | No fields. | Bounded status, heads, missing paths, and five-slot occupancy. | Read-only; readiness dimensions remain independent and failures are typed. | Room only standalone; mutations use `room.maintenance.submit`. |
| `room.maintenance.submit` | Apply one to five validated Room maintenance requests through the repository-owned executor. | `items` with unique `slot`/`payload`; optional `mode` and `waitSeconds`. | Items are bounded to 1–5; mode is `local` or `workflow`; wait is capped at 30 seconds. | Admission, local apply, semantic commit, and remote/workflow sync are reported independently; destructive but repository-confined. | Room only standalone; replaces legacy JSONL mutations. |

The legacy JSONL Room commands are intentionally absent from this standalone table. Their
protocol variants and Hub HTTP/MCP forwarding remain below as compatibility residue for the
separate Hub parity workstream; the Agent does not execute them.

## Hub full and coordinator surfaces

The Hub full profile contains the execution surface below. The coordinator
profile contains only `hub.info`, `agent.list`, `hub.run.list`, `hub.run.get`,
`hub.job.list`, `hub.job.get`, `user.notify.channels`, and
`user.notify.send`; it never dispatches an Agent command. Hub tools use
`agentId` where shown, while active Room tools intentionally route to the
active Room Agent and do not take it.

| Hub public name(s) | Use / no use | Required or conditional inputs | Defaults and bounds | Failure / lifecycle | Parity |
|---|---|---|---|---|---|
| `hub.info`, `agent.list` | Inspect Hub/agent availability and safe summaries; no execution. | `agent.list` has no body; `hub.info` has no body. | Safe counts/config summaries only. | Read-only; offline agents are reported unknown, not healthy. | Coordinator + Full; no standalone alias. |
| `hub.run.list`, `hub.run.get` | Inspect persisted Hub-to-Agent request runs; no new dispatch. | `run.get` requires `runId`; list filters are optional. | Bounded retained history/results. | Timeout/delivery/late-result states remain explicit. | Coordinator + Full; HTTP run endpoints mirror. |
| `hub.job.list`, `hub.job.get` | Inspect cached/live Job snapshots without dispatching execution. | `agentId` and Job filters/id as applicable; `waitSeconds` is optional and defaults to 0 for get. | List default limit 50/max 100; get wait maximum 30; cache fallback is explicit. | Cache-only data is not proof of fresh liveness; wait timeout does not cancel the Job. | Coordinator + Full; no standalone `hub.` names. |
| `process.exec`, `process.batch` | Same managed-process semantics as standalone through selected `agentId`. | `agentId` plus standalone process fields. | Wait ≤30; batch admission/ordered child Jobs. | Policy/capacity/child failures remain per contract; no sibling rollback. | Full only; HTTP POST process endpoints mirror. |
| `job.list`, `job.get`, `job.cancel` | Same shared Job lifecycle through selected Agent. | `agentId` plus standalone Job fields, including list `group/cursor` and get `waitOnly`. | List default 50/max 100; get wait default 0/max 30; `waitOnly=true` suppresses active intermediate detail; wait timeout does not cancel. | Online requests preserve Agent semantics; cache fallback is explicitly degraded, never a fake cursor continuation or fresh wait result. | Full only; HTTP `/v1/jobs` mirrors. |
| `tmux.listSessions`, `tmux.listPanes`, `tmux.capturePane` | Discover/read persistent panes; no input mutation. | `agentId`; capture/pane targets as applicable. | Capture defaults 160 lines; bounded output. | Read-only typed pane/session errors. | Full only; standalone combines aliases. |
| `tmux.pasteText`, `tmux.exec` | Paste non-shell input or submit shell command through selected Agent. | `agentId` and action-specific target/text/program. | Bounded wait/history. | Shell-vs-non-shell and policy/confirmation boundaries are explicit. | Full only; same local semantics. |
| `tmux.createSession`, `tmux.closeSession` | Create/close persistent workspace; no generic process lifecycle. | `agentId`, name/cwd; close may confirm. | Policy-checked cwd; reuse preferred. | Close destructive; no implicit data recovery. | Full only; same local semantics. |
| `mcp.listServers`, `mcp.listTools` | Discover downstream MCP routing/schema before a managed call. | `mcp.listServers` may omit `agentId` to group connected agents; listTools requires `agentId`,`serverId`. | Bounded metadata. | Read-only timeout/agent errors. | Full only; HTTP `/v1/mcp/servers|tools` mirrors. |
| `room.bootstrap`, `room.bootstrap.read` | Active Room bootstrap manifest/guide access; no arbitrary file read. | Read requires guide `id`; no `agentId`. | Same package bounds/revision as standalone Room bootstrap. | Room inactive/invalid/not-found errors are explicit and read-only. | Full only; standalone names omit `room.` prefix. |
| `room.diary.append`, `room.diary.recent`, `room.diary.selectExact` | Legacy JSONL Room forwarding for Hub parity only; not a standalone Agent surface. | Append requires `entry` (optional `tags`); selectExact requires `date` (optional `limit`); no `agentId`. | Hub request projection is bounded; append/selectExact are not current Agent execution paths. | Current Agent rejects legacy commands with `room_legacy_surface_removed`; WP-R owns any future implementation/migration. | Hub compatibility residue only; never advertised by standalone Agent. |
| `room.notebook.append`, `room.notebook.current`, `room.notebook.recent`, `room.notebook.remove`, `room.notebook.search`, `room.notebook.selectExact`, `room.notebook.update` | Legacy JSONL Room forwarding for Hub parity only; not a standalone Agent surface. | Append requires `scope`/`content`; `significance` defaults `NORMAL`, `datetime`/`abstract`/`tags` optional. `selectExact` requires `date`; no `agentId`. | Hub request projection is bounded; append/update/remove/selectExact are not current Agent execution paths. | Current Agent rejects legacy commands with `room_legacy_surface_removed`; WP-R owns any future implementation/migration. | Hub compatibility residue only; never advertised by standalone Agent. |
| `bootstrap`, `bootstrap.read` | Full-profile transport-neutral aliases for Room bootstrap. | Read requires `id`; no `agentId`. | Same package bounds/revision. | Same bootstrap errors; read-only. | Full only; aliases are intentional bootstrap names, not compatibility for removed tools. |
| `skills.list`, `skills.read`, `skills.search`, `skills.active` | Active Room skill discovery/read/search. | Read/search fields as applicable; no `agentId`. | Bounded summaries/content. | Invalid/missing/stale skills are explicit. | Full only; HTTP Room skills endpoints mirror. |
| `skills.activate`, `skills.deactivate` | Change active skill state only; no execution/permission grant. | `id`; no `agentId`. | Idempotent state operation. | Stale/missing deactivation is allowed and reported. | Full only; standalone `skills.setActive` combines intent. |
| `skills.install`, `skills.install.get`, `skills.install.cancel` | Asynchronous active-Room installation lifecycle. | Install requires `id`,`source`; get/cancel require `installId`; no `agentId`. | Wait default 5, maximum 30; wait timeout does not cancel; cancellation is explicit. | Cooperative cancellation/atomic commit evidence. | Full only; HTTP Room install endpoints mirror. |
| `skills.run` | Run active skill executable as managed Job. | `id`,`path`; optional `group`, args/cwd/wait; no `agentId`. | Wait default 5, maximum 30; wait timeout does not cancel; use `job.cancel`. | Same Job/policy/cancellation contract as standalone. | Full only; HTTP Room run mirrors group and lifecycle semantics. |

## Review rules

- A descriptor change must update the relevant row and its focused parity test;
  do not add a compatibility alias to make an invalid call appear valid.
- Required fields describe admission, not a promise that execution succeeds.
  Conditional fields must be stated in the tool description or property
  description, especially revision/absence guards, action-specific tmux fields,
  and Job follow-up.
- “Atomic” is reserved for validation/admission/confirmation boundaries. It
  never implies rollback of an already-started process, MCP call, notification,
  or other external side effect.
- Standalone V2 surface counts are Normal 29 / Room 40. They are profile preset counts, not a
  guarantee after explicit toolset selection.
- Legacy JSONL names remain documented only in the Hub compatibility rows above; standalone
  aliases and Hub full/coordinator profile membership are intentionally different.
