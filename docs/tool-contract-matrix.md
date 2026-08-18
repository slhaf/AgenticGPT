# Public tool contract matrix

This is the checked-in Phase D matrix for the frozen D-11 surface. It is a
review aid, not a second schema: the live descriptors and typed request
objects remain authoritative. “No use” means the nearest tempting operation
that this tool deliberately does not perform. Bounds are inclusive unless
stated otherwise.

## Standalone Normal and Room surface

Normal exposes the first 24 names. Room adds the final 12 names, for exactly
36. Tunnel stdio and local Unix MCP use the same descriptors, schemas,
confirmation, path policy, audit, and Job registry. Standalone calls do not
accept Hub-only `agentId` or `confirmMethod` fields.

| Public name | Use / no use | Required or conditional inputs | Defaults and bounds | Failure / lifecycle | Surface parity |
|---|---|---|---|---|---|
| `agent.info` | Inspect local runtime; no execution or mutation. | No required fields. | Bounded diagnostics and safe config summary. | Read-only snapshot; live Job/config state may change after return. | Normal + Room; Tunnel = local Unix. |
| `file.read` | Read UTF-8 content/metadata; no shell, search process, or write. | Flat `path` form or ordered `requests` of the same shape (1–32), mutually exclusive; optional `includeContent`, inclusive `startLine`/`endLine`. | Content and line output are bounded; batch envelopes preserve order and isolate failures under the aggregate output budget. | Typed path/UTF-8/size errors; retry-safe and non-destructive. | Normal + Room; same file schema in both ingress paths. |
| `file.search` | In-process literal/regex search; no shell or external search fallback. | Flat `path`/`query` form or ordered `requests` of the same shape (1–32), mutually exclusive; optional mode/globs/context/limits. | Literal, case-sensitive, Git-ignore aware; per-search limits plus 20k-file/128 MiB aggregate scan and ~1 MiB response bounds. | Invalid regex/glob/path or typed argument errors; read-only and bounded. | Normal + Room; same search schema in both ingress paths. |
| `file.edit` | Apply a complete Codex apply-patch patch across UTF-8 files; no model-supplied revision guards. | `patch` plus optional `dryRun`/`needConfirm`; patch supports Add/Delete/Update/Move across multiple files. | One complete preflight, deterministic locks, one confirmation, internal source revalidation, bounded diff, and atomic/temp commits. | Context/path/UTF-8/size/race/confirmation failures write nothing before commit; audit retains internal revisions without exposing them in the response. | Normal + Room; standalone only. |
| `process.exec` | Start one policy-controlled local process; no direct unbounded shell API. | `program`; optional direct `args`, `workingDirectory`, `waitSeconds`, `needConfirm`. | Wait is bounded at 30 seconds; response is always a Job envelope. | Policy/confirmation/capacity/spawn/exit errors are retained in the Job; use `job.get`/`job.cancel`. | Normal + Room; Hub full `process.exec` mirrors semantics. |
| `process.batch` | Admit multiple managed processes; no implicit sibling rollback after admission. | `elements` (each requires `program`); optional batch cwd/wait/confirmation. | One admission/confirmation boundary; ordered children; wait ≤30 seconds. | Validation/capacity rejection starts none; post-admission child failures remain per child. | Normal + Room; Hub full `process.batch` mirrors semantics. |
| `job.get` | Inspect or briefly wait for one managed Job; no new work. | `jobId`; optional `waitSeconds`. | Wait default 0, maximum 30 seconds. | Fresh or retained state is bounded; after restart a missing generation is not claimed completed. | Normal + Room; Hub full `job.get` and HTTP job GET are equivalent lifecycle views. |
| `job.list` | Discover active/recent Jobs; no mutation or admission. | No required fields; optional `kind`, `state`, `limit`. | Bounded list; limit maximum 100. | Read-only snapshot; terminal retention is finite. | Normal + Room; Hub full `job.list` and HTTP jobs list mirror filters. |
| `job.cancel` | Request kind-aware cancellation; no guarantee of remote termination. | `jobId`. | One request per Job; no wait argument. | Returns observed outcome/evidence; unconfirmed remote stop is `detached`, not `cancelled`. | Normal + Room; Hub full `job.cancel` and HTTP cancel mirror evidence. |
| `mcp.list` | Discover configured downstream servers or one server's tools; no downstream execution. | Optional `serverId`; omit to list servers. | Bounded server/tool metadata. | Config/transport errors are typed and read-only. | Normal + Room; Hub full uses split `mcp.listServers`/`mcp.listTools`. |
| `mcp.callTool` | Start one downstream MCP call as a managed Job; no direct transactional call. | `serverId`, `toolName`; optional JSON-object `arguments`, `waitSeconds`, `timeoutSeconds`. | Arguments ≤256 KiB; wait default 5/max 30; timeout default 300/max 900 seconds. | Confirmation/policy/transport/downstream errors are retained; oversized results keep hash/size/preview; follow up with `job.*`. | Normal + Room; Hub full mirrors the Job envelope. |
| `mcp.batch` | Validate/admit 1–16 downstream calls with one aggregate confirmation; no rollback of downstream side effects. | `calls` with `serverId`/`toolName`; optional per-call id/arguments, `mode`, `failFast`, waits/deadline. | Parallel default; sequential is explicit; aggregate args/response ≤2 MiB; global/per-server concurrency 8/2. | Admission is atomic; `failFast` skips only not-started children; ordered child Jobs and aggregate audit remain. | Normal + Room; Hub full mirrors admission and bounds. |
| `skills.list` | Discover valid workspace skills; no install or execution. | Optional query/limit/active filter. | Bounded summaries. | Invalid/unreadable skills become warnings or omitted; read-only. | Normal + Room; Hub full uses the same Room workspace. |
| `skills.read` | Read one skill package/resource; no arbitrary workspace file access. | `id`; optional package-relative `path`. | Bounded Markdown/frontmatter/resource. | Invalid/missing skill/resource is typed; read-only. | Normal + Room; Hub full `skills.read` mirrors. |
| `skills.setActive` | Set active flag only; no execution or permission grant. | `id`, `active`. | Active state persists in Room workspace. | Invalid IDs/skills are typed; state change is audited. | Normal + Room; Hub full exposes `skills.activate`/`skills.deactivate` aliases with split intent. |
| `skills.install` | Start asynchronous skill installation; no inline network payload or arbitrary URL fetch. | `id`, `source`; optional replacement/activation/idempotency. | Returns `installId`; source and package/file bounds apply. | Validate/commit failures are retained; existing skill archive/commit is atomic; use install get/cancel. | Normal + Room; Hub full and HTTP Room install mirror. |
| `skills.install.get` | Inspect or briefly wait for installation; no new install. | `installId`; optional `waitSeconds`. | Wait maximum 30 seconds; terminal `pollAfterMs` is 0. | Bounded persisted status; missing/expired IDs are typed. | Normal + Room; Hub full/HTTP install get mirror. |
| `skills.install.cancel` | Request cooperative pre-commit cancellation; no forced rollback after commit. | `installId`. | Idempotent request. | Outcome distinguishes cancelled/terminal/too-late; evidence is retained. | Normal + Room; Hub full/HTTP install cancel mirror. |
| `skills.run` | Run an executable under an active skill as a managed Job; no arbitrary path. | `id`, package-relative `path`; optional args/cwd/wait. | Wait maximum 30 seconds; Job retention follows shared limits. | Policy/confirmation/script/exit failures are Job states; use `job.*`. | Normal + Room; Hub full/HTTP Room skills run mirror. |
| `tmux.sessions` | List/create/close persistent sessions; no command submission. | `action`; create/close require action-compatible name/cwd; close may require confirmation. | Reuse default session where possible; policy-checked cwd. | Close is destructive; typed session/policy/confirmation errors. | Normal + Room; Hub full uses split tmux names. |
| `tmux.panes` | List/capture panes; no input submission. | `action`; capture requires target; list may filter session. | Capture history default 160 lines and bounded. | Action-incompatible fields are rejected, not ignored. | Normal + Room; Hub full uses split tmux names. |
| `tmux.exec` | Submit structured command to a shell pane; no claim that submission completed. | `target`, `program`; optional args/wait/capture/confirmation. | Bounded post-submit wait/history. | Shell/policy/confirmation errors; inspect pane or process result for completion. | Normal + Room; Hub full `tmux.exec` mirrors. |
| `tmux.pasteText` | Paste into non-shell pane/TUI; no shell execution. | `target`, `text`; optional `submit`, confirmation. | Text/history bounded. | Shell panes are rejected; pane state remains otherwise unchanged. | Normal + Room; Hub full mirrors. |
| `bootstrap` | Load Room bootstrap entrypoint/guide manifest; no generic file read or file creation. | No fields. | Bounded guide summaries and package revision. | Missing/invalid package is typed/warned; read-only and retry-safe. | Room only standalone; Hub full has `bootstrap` and `room.bootstrap` routes. |
| `bootstrap.read` | Read one validated bootstrap guide; no arbitrary path. | `id`. | Bounded Markdown/frontmatter. | Unknown/invalid/duplicate guide is `guide_not_found`; read-only. | Room only standalone; Hub full has `bootstrap.read` and `room.bootstrap.read`. |
| `room.diary.append` | Append dated Room diary entry; no notebook replacement. | `entry`; optional time hint/tags. | Bounded entry/tags. | Storage/validation errors are typed; durable mutation is audited. | Room only; Hub full and HTTP Room diary mirror. |
| `room.diary.recent` | Read recent diary entries; no mutation. | Optional days/limit. | Bounded days and entries. | Read-only snapshot; timezone follows Room config. | Room only; Hub full/HTTP mirror. |
| `room.diary.selectExact` | Read entries for one exact Room-local date; no range scan mutation. | `year`, `month`, `day`; optional limit. | Bounded date/result count. | Invalid date/storage errors are typed; read-only. | Room only; Hub full/HTTP mirror. |
| `room.notebook.append` | Append durable notebook passage; no transient chat-only note. | `scope`, `significance`, `abstract`, `content`; optional datetime/tags. | Scope path-safe; datetime stored UTC; bounded text/tags. | Validation/storage errors are typed; ANCHOR refreshes recoverable state. | Room only; Hub full/HTTP mirror. |
| `room.notebook.current` | Read recoverable current state for one scope; no mutation. | `scope`. | One bounded state snapshot. | Missing state returns null/typed scope errors; read-only. | Room only; Hub full/HTTP mirror. |
| `room.notebook.recent` | Read recent passages with filters; no mutation. | Optional scope/days/significance/limit. | Bounded calendar scan/result count. | Read-only snapshot; Room timezone controls date partitioning. | Room only; Hub full/HTTP mirror. |
| `room.notebook.remove` | Remove one passage; no bulk or arbitrary file deletion. | `id`. | One passage per call. | Destructive; current state falls back to latest ANCHOR/null and response says what happened. | Room only; Hub full/HTTP mirror. |
| `room.notebook.search` | Substring-search notebook fields; no vector search or mutation. | `query`; optional scope/limit. | Bounded substring scan/result count. | Read-only; invalid query/storage errors are typed. | Room only; Hub full/HTTP mirror. |
| `room.notebook.selectExact` | Read passages for one exact Room-local date; no mutation. | `year`, `month`, `day`; optional scope/limit. | Bounded date/result count. | Invalid date/storage errors are typed; read-only. | Room only; Hub full/HTTP mirror. |
| `room.notebook.update` | Update editable passage fields; no scope/datetime rewrite. | `id`; at least one editable field. | Scope and datetime immutable; optional significance/abstract/content/tags. | Validation/storage errors are typed; current state refresh is reported. | Room only; Hub full/HTTP mirror. |

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
| `hub.job.list`, `hub.job.get` | Inspect cached/live Job snapshots without dispatching execution. | `agentId` and Job filters/id as applicable; `waitSeconds` capped 30 for get. | Cache fallback is explicit; bounded lists. | Cache-only data is not proof of fresh liveness. | Coordinator + Full; no standalone `hub.` names. |
| `process.exec`, `process.batch` | Same managed-process semantics as standalone through selected `agentId`. | `agentId` plus standalone process fields. | Wait ≤30; batch admission/ordered child Jobs. | Policy/capacity/child failures remain per contract; no sibling rollback. | Full only; HTTP POST process endpoints mirror. |
| `job.list`, `job.get`, `job.cancel` | Same shared Job lifecycle through selected Agent. | `agentId` plus Job fields. | Wait ≤30; bounded retention. | Evidence-based cancellation and cache/Agent availability states. | Full only; HTTP `/v1/jobs` mirrors. |
| `tmux.listSessions`, `tmux.listPanes`, `tmux.capturePane` | Discover/read persistent panes; no input mutation. | `agentId`; capture/pane targets as applicable. | Capture defaults 160 lines; bounded output. | Read-only typed pane/session errors. | Full only; standalone combines aliases. |
| `tmux.pasteText`, `tmux.exec` | Paste non-shell input or submit shell command through selected Agent. | `agentId` and action-specific target/text/program. | Bounded wait/history. | Shell-vs-non-shell and policy/confirmation boundaries are explicit. | Full only; same local semantics. |
| `tmux.createSession`, `tmux.closeSession` | Create/close persistent workspace; no generic process lifecycle. | `agentId`, name/cwd; close may confirm. | Policy-checked cwd; reuse preferred. | Close destructive; no implicit data recovery. | Full only; same local semantics. |
| `mcp.listServers`, `mcp.listTools` | Discover downstream MCP routing/schema before a managed call. | `mcp.listServers` may omit `agentId` to group connected agents; listTools requires `agentId`,`serverId`. | Bounded metadata. | Read-only timeout/agent errors. | Full only; HTTP `/v1/mcp/servers|tools` mirrors. |
| `mcp.callTool`, `mcp.batch` | Same managed MCP Job/admission contract as standalone through `agentId`. | `agentId` plus standalone MCP fields. | 256 KiB child args, 2 MiB aggregate, wait/deadline and 8/2 concurrency bounds. | Admission atomic; downstream effects never rolled back; Job follow-up/cancellation evidence retained. | Full only; HTTP `/v1/mcp/*` and standalone wording align. |
| `user.notify.channels` | List Hub-native notification routes; no Room Agent dispatch. | No required fields. | Bounded channel metadata. | Read-only channel availability. | Coordinator + Full; no standalone tool. |
| `user.notify.send` | Send one Hub-native user notification; no local Agent command. | Channel/title/body; optional actions/priority. | Channel-specific bounded payload. | Route failure is explicit; no transactional retry claim. | Coordinator + Full; no standalone tool. |
| `room.bootstrap`, `room.bootstrap.read` | Active Room bootstrap manifest/guide access; no arbitrary file read. | Read requires guide `id`; no `agentId`. | Same package bounds/revision as standalone Room bootstrap. | Room inactive/invalid/not-found errors are explicit and read-only. | Full only; standalone names omit `room.` prefix. |
| `room.diary.append`, `room.diary.recent`, `room.diary.selectExact` | Active Room diary append/read/date selection. | Same standalone diary fields; no `agentId`. | Same bounded dates/entries. | Same durable/read-only distinction and timezone. | Full only; HTTP Room endpoints mirror. |
| `room.notebook.append`, `room.notebook.current`, `room.notebook.recent`, `room.notebook.remove`, `room.notebook.search`, `room.notebook.selectExact`, `room.notebook.update` | Active Room notebook mutation/read/search. | Same standalone notebook fields; no `agentId`. | Same scope/date/content bounds. | Same anchor/current-state and destructive evidence. | Full only; HTTP Room endpoints mirror. |
| `bootstrap`, `bootstrap.read` | Full-profile transport-neutral aliases for Room bootstrap. | Read requires `id`; no `agentId`. | Same package bounds/revision. | Same bootstrap errors; read-only. | Full only; aliases are intentional bootstrap names, not compatibility for removed tools. |
| `skills.list`, `skills.read`, `skills.search`, `skills.active` | Active Room skill discovery/read/search. | Read/search fields as applicable; no `agentId`. | Bounded summaries/content. | Invalid/missing/stale skills are explicit. | Full only; HTTP Room skills endpoints mirror. |
| `skills.activate`, `skills.deactivate` | Change active skill state only; no execution/permission grant. | `id`; no `agentId`. | Idempotent state operation. | Stale/missing deactivation is allowed and reported. | Full only; standalone `skills.setActive` combines intent. |
| `skills.install`, `skills.install.get`, `skills.install.cancel` | Asynchronous active-Room installation lifecycle. | Install requires `id`,`source`; get/cancel require `installId`; no `agentId`. | Wait ≤30; bounded persisted retention. | Cooperative cancellation/atomic commit evidence. | Full only; HTTP Room install endpoints mirror. |
| `skills.run` | Run active skill executable as managed Job. | `id`,`path`; optional args/cwd/wait; no `agentId`. | Wait ≤30; shared Job retention. | Same Job/policy/cancellation contract as standalone. | Full only; HTTP Room run mirrors. |

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
- Standalone surface counts remain Normal 24 / Room 36. Hub full/coordinator
  profile membership and standalone aliases are intentionally different and
  must stay visible in tests and release notes.
