# Findings & Decisions: Skills installer planning

Plan scope: `.planning/skill-installer`.

## Requirements captured from discussion

- Add a default `skill-installer` skill that teaches other agents how to install skills.
- Treat AgenticGPT as a runtime/tool layer, not a full intelligent agent.
- Keep every skills management operation concentrated in the active Room Agent.
- Preserve and build on the existing skills MCP tools rather than treating the API as empty.
- Extend the existing `skills.read` for package-relative content instead of creating duplicate read tools.
- Add GitHub-aware installation.
- Allow a package file list to contain URLs downloaded directly by the Room Agent to package-relative destinations.
- Retain inline content support.
- Make installation asynchronous because ChatGPT Apps-facing MCP calls can time out during network/archive work.
- Expose durable installation status, progress, elapsed time, and final results through a separate query.
- Allow replacement of existing workspace skills, but archive the old package under `skills/.archive` instead of deleting it.
- Limit initial GitHub support to public repositories.
- Update GPT Actions alongside the MCP tools.
- Accept arbitrary public HTTPS file URLs with SSRF checks and an optional deployment allowlist.
- Retain terminal job records for seven days and at most 100 records, independently of skill archives.
- Make status lookup a bounded long poll that defaults to 5 seconds and accepts a caller-specified duration in seconds.
- Make the built-in `skill-installer` active by default while preserving explicit user deactivation across restart and upgrade.
- Expose an idempotent `skills.install.cancel` tool in both MCP and GPT Actions.
- Limit each install job to one explicit top-level target `id`.
- Require `replaceExisting: true` before archiving and replacing an existing workspace skill.
- Make newly installed skills active by default while preserving an existing target's activation state during replacement.
- Accept an optional Room-scoped `idempotencyKey` for deduplication across fresh external retries.
- Treat structured GitHub input as canonical, accept unambiguous GitHub URLs as convenience input, default to the repository default branch, and persist the resolved commit SHA.

## Existing implementation

- `crates/agentic-gpt/src/skills.rs` implements list, read, search, active, activate, and deactivate.
- Skills are currently discovered from `workspace/skills/<id>/SKILL.md`.
- `skills.read` currently returns the full `SKILL.md`, parsed frontmatter, summary flags, warnings, and active status, but not arbitrary package-relative files.
- Active state is stored at `workspace/state/active-skills.json` and written atomically.
- `crates/agentic-gpt-hub/src/room.rs` routes all skills commands through the current active Room connection.
- `crates/agentic-gpt/src/hub.rs` rejects skills commands when the local agent is not running as Room.
- MCP and Actions schemas intentionally omit `agentId` for skills tools.
- Existing targeted tests passed: six local skills tests, MCP read-only annotations, and the no-`agentId` OpenAPI contract test.
- Existing GPT Actions skills endpoints consistently use `POST /v1/room/skills/<operation>` with JSON bodies, while MCP uses separately derived argument schemas. New install/get/cancel Actions should preserve this convention unless a concrete incompatibility appears.

## Handoff contract gaps to freeze before implementation

- Install cardinality: one target skill per `installId` versus a multi-skill batch job.
- Target identity and collision intent: whether `id` is always explicit and whether replacement requires an explicit flag.
- Post-install activation behavior for ordinary workspace skills.
- Exact GitHub structured/URL input forms and default-ref behavior.
- Exact `skills.read` path, encoding, file-only, and response-size behavior.
- External request/response schemas for install start/get/cancel, including idempotency and cancellation outcomes.
- Resource ceilings, timeouts, redirects, concurrency, and total job deadline.
- Persisted job/status schema and restart recovery rules. These can be engineering defaults if external behavior remains stable.

## Contract-shape observations from the current code

- Protocol structs use camelCase serde, while OpenAPI explicitly sets `additionalProperties: false`; new public schemas must enumerate every optional field and cannot rely on loose maps.
- `SkillReadRequest` currently contains only required `id`. Adding optional `path` with a default of `SKILL.md` preserves existing callers.
- MCP tool argument structs are distinct from protocol structs, so every new/changed public field needs synchronized protocol, MCP schema, Actions/OpenAPI, routing, and serde tests.
- The Room Agent already depends on streaming `reqwest` with rustls, which is suitable for bounded URL/GitHub downloads. No existing skill-download configuration block or obvious archive extraction dependency is present.
- Resource and network policy should therefore be represented by a dedicated backward-compatible skills configuration section with conservative defaults, rather than only scattered implementation constants.
- The Hub holds normal Room commands for at most 35 seconds. The current proposed `waitSeconds` maximum of 30 leaves a five-second dispatch/serialization margin and aligns with the Hub's existing maximum-wait convention.
- The local transport ledger deduplicates replay of the same Hub `runId`, but a caller retry that creates a fresh MCP/Actions run can create another install job. A caller-supplied idempotency key is needed if cross-call retry deduplication is part of the public contract.
- The Room configuration already has backward-compatible nested defaults. A new defaulted `room.skills` block fits existing configuration evolution better than a new Hub-owned policy, since download and persistence happen in the Room Agent.
- MCP native result wrapping already marks any top-level `{ error: ... }` value as `isError: true`, while Actions uses the shared `{ error: { code, message } }` body. Install preflight errors can preserve one structured error model across both surfaces.
- Existing generic Room Actions forwarding returns Room business values as HTTP 200 even when they contain a top-level error. Using semantic 400/409 responses for new install preflight failures requires explicit Hub mapping and matching OpenAPI responses; it should be frozen rather than left to the implementer.
- Hub transport run status/result remains a separate schema and must not be reused as the install job status shape.

## Reliable transport versus installation jobs

- Hub `agent_runs` persists Hub-to-Agent commands with delivery/ack/result state and a 24-hour TTL.
- It stores one final result and supports replay/late-result recovery.
- It does not model download progress, source resolution, validation, commit, or Room restart recovery.
- Therefore install creation should use the existing reliable command path, but the returned `installId` must address a separate Room-owned business job.
- Expected normal flow:

  `skills.install` → reliable Hub command → Room persists job → immediate `installId` response → Room worker downloads/validates/commits → `skills.install.get` reads durable status.

- If the start command itself times out, `hub.run.get` can recover its late acceptance result. Once an `installId` is known, status belongs to `skills.install.get`.

## Proposed source model

- `github`: structured repository, ref, and subdirectory, with optional canonical URL convenience parsing.
- `files`: explicit package-relative entries.
- Each file entry supplies exactly one of:
  - `url`
  - UTF-8 `content`
  - `contentBase64`
- Optional expected SHA-256 can verify URL-backed entries.
- All variants normalize to an internal plan containing the target relative path, acquisition method, optional expected digest, and size constraints.
- The final target ID should be explicit so jobs can lock and deduplicate before the package is downloaded.

## Install contract decisions — first handoff batch

- Cardinality: one skill per request/job. `files` is the list of files inside that one skill, not a batch of target skills.
- Identity: top-level `id` is required for both GitHub and files sources; frontmatter `name` remains independent.
- Collision: `replaceExisting` defaults to false. A collision without explicit replacement intent is rejected before network work; built-in reserved IDs remain non-replaceable.
- Activation: a new skill becomes active by default. A replacement preserves the old target's active/inactive state. An explicit activation option may promote the result to active, but false/omitted never deactivates an already active target.
- Idempotency: `idempotencyKey` is optional and scoped to the Room. Same key plus same canonical request returns the original job; same key plus a different canonical request returns `idempotency_conflict`; omission creates a new job.
- GitHub: structured `repository`, optional `ref`, and optional `path` are canonical. An unambiguous supported GitHub web URL may normalize to the same form. Omitted `ref` resolves the default branch, and the immutable commit SHA is stored as provenance.
- GitHub tree URLs with ambiguous slash-containing refs fail with guidance to use structured fields. V1 accepts no embedded credentials, custom auth headers, or private repositories.

## Proposed job state

- Stable status: `queued`, `running`, `completed`, `failed`, `cancelled`.
- Detailed phase: `resolving`, `downloading`, `extracting`, `validating`, `committing`, `activating`.
- Progress: completed/total files, downloaded/total bytes where total is known.
- Timing: submitted, started, updated, finished, and computed elapsed milliseconds.
- Final result: installed skill summary, resolved provenance, revision, and digest.
- Error: stable code, safe message, failing phase, and retryable flag.
- Suggested persisted location: `workspace/state/skill-installs/<installId>/` with request, status, and private staging data.

## Built-in skill observations

- A binary-embedded built-in is preferable to copying a default file at every startup because it upgrades with the Room Agent and cannot be silently replaced by a workspace directory.
- Built-in and workspace skills need explicit origin/read-only metadata and deterministic ID collision behavior.
- Preinstalled and active-by-default are distinct concepts. Default activation needs a persisted migration/tombstone scheme if user deactivation must survive restart.
- The initial `skill-installer` should teach the cross-tool workflow and source selection; it should not contain a second independent downloader implementation.

## Replacement and archive semantics

- Replacement applies to workspace skills; reserved built-in IDs remain immutable.
- A candidate package is fully downloaded and validated in staging before the current package is touched.
- Under the final target lock, an existing `skills/<id>` is renamed to `skills/.archive/<id>/<timestamp-or-install-id>`.
- The staged candidate is then atomically renamed to `skills/<id>`.
- If the second rename fails, the archived package is restored to the original location.
- Existing active state is keyed by skill ID and therefore remains active across a successful replacement.
- Workspace scanning must explicitly ignore `.archive` and other internal dot-directories.
- No automatic hard deletion of archived packages is currently planned.

## Security findings

- Current ID validation blocks textual traversal but filesystem checks can follow symlinks; package reads and installs need canonical containment or explicit symlink rejection.
- Existing scanning has no strict SKILL.md/package size limits.
- Direct URL support introduces SSRF risk, including private/loopback/link-local/metadata targets and redirect changes.
- GitHub archive extraction must reject absolute paths, `..`, symlinks, hardlinks, duplicates, and size/count explosions.
- Arbitrary download destinations must never be accepted; destinations are package-relative paths only.
- Installation should never run scripts.
- Network and extraction work must happen outside the final global write lock.
- Atomic staging/rename is required so readers never observe a partial package.

## ChatGPT Apps guidance

- Official Apps SDK guidance favors one focused action per tool, explicit schemas, predictable structured output, and reusable machine-readable identifiers.
- Server instructions can document cross-tool workflows such as install-start followed by status polling.
- Enqueueing a task changes state, so the install-start tool must not be marked read-only.
- Official documentation does not provide a single numeric tool timeout that the server can safely design around; the architecture should avoid long synchronous calls rather than tune to a presumed limit.

## Relevant resources

- `crates/agentic-gpt/src/skills.rs`
- `crates/agentic-gpt/src/hub.rs`
- `crates/agentic-gpt-hub/src/room.rs`
- `crates/agentic-gpt-hub/src/mcp_server.rs`
- `crates/agentic-gpt-hub/src/runs.rs`
- `crates/agentic-gpt-hub/src/db.rs`
- `crates/agentic-gpt-protocol/src/lib.rs`
- `openapi/hub.yaml`
- `docs/interfaces.md`
- https://developers.openai.com/apps-sdk/plan/tools
- https://developers.openai.com/apps-sdk/concepts/mcp-server
- https://developers.openai.com/apps-sdk/deploy/submission#review-and-approval-faqs

## Open decisions

None. All product-level contracts are frozen for implementation handoff.

## Installed skill script execution discovery

- Installation and activation must never execute package scripts. Script execution is a separate consequential action after installation.
- Public `process.exec` already provides structured `program`/`args`, optional working directory, policy evaluation, confirmation, path preflight, optional bubblewrap execution, a 30-second timeout, 64 KiB stdout/stderr tails, task status, and audit records.
- Its public contract requires `agentId`, while every skills operation intentionally routes to the active Room without `agentId`. Requiring a consuming agent to rediscover the Room ID would weaken the established skills boundary.
- The existing working-directory check can safely constrain cwd to the workspace, but the executable `program` itself is not canonicalized and checked against a skill package. Path checks apply only to selected arguments that syntactically look like paths. Therefore public `process.exec` alone does not prove that the executed file belongs to `skills/<id>/scripts`.
- In Room mode, generic execution defaults may allow many programs unless the caller requests confirmation or local configured policy says otherwise. `process.exec` also defaults `needConfirm` to false at the MCP layer.
- Confirmed design: add only a thin Room-scoped `skills.run` start wrapper, launch through the existing managed-session engine, and reuse/refactor the existing local execution engine for policy, confirmation, optional sandbox, bounded output, result, and audit behavior. Do not implement a second subprocess or asynchronous-job engine.
- Confirmed `skills.run` request: required skill `id` and package-relative script `path`, optional direct `args`, optional existing-policy-validated `workingDirectory`, and optional bounded `waitSeconds`; no `agentId`, arbitrary `program`, shell command string, or environment override.
- Repository check: workspace skills live at `<workspace_root>/skills/<id>`, while generic `session.start` without `workingDirectory` executes from `<workspace_root>`. Fixing skill runs to the package root would therefore diverge from existing Room execution behavior and encourage scripts to modify their installed package rather than operate on Room workspace data.
- Confirmed cwd behavior: permit optional `workingDirectory` with the exact existing session/path-policy validation and default it to Room workspace root; continue to resolve the executable only from the selected skill package. This does not broaden the Room agent's existing execution authority and keeps `skills.run` focused on executable identity/provenance.
- Resolve the active Room, require the skill to be active and workspace-backed, restrict package-relative paths to `scripts/`, canonicalize a regular executable file beneath the selected package, and reject symlinks. Embedded built-ins return `skill_not_runnable` in V1.
- Return `agentId`, `sessionId`, `completedInline`, `pollAfterMs`, and standard `SessionInfo`; callers then use the existing `session.inspect`, `session.wait` (0–30 seconds), and `session.kill` APIs. Do not add duplicate `skills.run.get` or `skills.run.cancel` tools.
- Refined hybrid behavior proposed by the user: `skills.run` always creates the managed session first, then performs a bounded wait. If the session reaches a terminal state within the wait, the call returns that complete `SessionInfo` inline; otherwise it returns the current non-terminal `SessionInfo` and the same `agentId`/`sessionId` for existing session follow-up tools. Never start with `process.exec` and attempt to promote an already-running process after timeout.
- Confirmed wait semantics mirror install status wait: optional `waitSeconds`, default 5, `0` for immediate accepted state, range 0–30. The implementation reserves transport margin and uses event-driven notification so a fast script returns as soon as it completes rather than sleeping for the full interval.
- One stable response shape is used in both paths: always include `agentId`, `sessionId`, `completedInline`, `pollAfterMs`, and `session`; terminal `exited`/`failed`/`killed` returns `completedInline: true` and `pollAfterMs: 0`, while `starting`/`waiting_confirmation`/`running` returns false and default `pollAfterMs: 1000`.
- Route the resolved script through the same policy decision, confirmation defaults, environment, optional bubblewrap configuration, path policy, and bounded stdout/stderr tails as `session.start`. `skills.run` must not silently widen or narrow the Room Agent's configured execution authority.
- Audit `skills.run` with skill ID, relative script path, installed package digest, request source, session ID, args, resolved cwd, policy/confirmation result, terminal state, exit status, and duration.
- The current embedded `skill-installer` contains guidance only. V1 deliberately does not materialize embedded built-in scripts; a future built-in containing scripts needs a separate materialization/cache design.
- Confirmed session retention: keep terminals in memory for 24 hours and at most the newest 100 records; never prune `starting`/`waiting_confirmation`/`running`. Do not persist or recover sessions across Room Agent restart; clear stale Hub cache so lookup returns `session_not_found` rather than an obsolete running snapshot.
- `session.start` currently waits for policy confirmation before inserting the managed session and returning; despite `waiting_confirmation` appearing in API handling, the local implementation does not expose that state as a cancellable session. Therefore it is asynchronous after spawn, but not throughout confirmation.
- Confirmed change: refactor managed sessions so a valid skill run is inserted before requesting confirmation. A confirmation-required session is immediately visible as `waiting_confirmation`; approval transitions it to `running`, denial records a terminal failure/rejection, and `session.kill` cancels the pending confirmation. The initial `skills.run` bounded wait observes these state transitions and always has a session ID to return.
- Semantic preflight failures (`skill_not_found`, `skill_inactive`, `skill_not_runnable`, invalid/missing/non-executable script, invalid cwd, or `skill_update_pending`) create no session. After a session is accepted, policy denial, confirmation denial, spawn failure, execution failure, and kill are represented by its normal terminal `SessionInfo` state/reason.
- Threat-model correction from the user: a Room Agent is already a dedicated room for the consuming agent and intentionally has a relatively open command policy. The same caller can invoke generic `session.start` or `process.exec`, so imposing a stricter mandatory sandbox only on `skills.run` would be trivially bypassable and would not reduce the caller's effective authority.
- Current decision: do not add a skill-only sandbox mode, environment allowlist, network capability model, or fail-closed bubblewrap requirement. `skills.run` reuses the Room Agent's current generic execution configuration exactly; operators who need stronger isolation should tighten the existing Room command/path/sandbox policy globally rather than through a parallel skills policy.
- `skills.run` remains valuable as a semantic and provenance boundary: it resolves the active Room automatically, proves the requested executable belongs to the selected active installed skill, avoids arbitrary program input while validating optional cwd through existing policy, returns a managed session, and records skill ID/path/package digest in audit data. It is not presented as protection against an already-authorized Room agent.
- Confirmed package-version invariant: `skills.run` acquires a shared per-skill execution lease at session creation and holds it through terminal state. Installation may resolve/download/extract/validate concurrently, but its commit phase requires the exclusive lease before archiving or renaming the visible package.
- The lease is writer-fair. Once replacement commit is waiting, new runs return retryable `skill_update_pending`; existing sessions may finish or be killed. Install cancellation remains effective while waiting. If the overall install deadline expires first, the job fails retryably with `target_busy`, leaves the visible package unchanged, and creates no partial archive/commit state.
- `skills.run` is exposed through MCP and `POST /v1/room/skills/run`; annotations are non-read-only, destructive, open-world, and consequential because an installed script may mutate Room or external state.

## Job API and transactional decisions — third handoff batch items 14–17 confirmed

- Confirmed question 14A: schema/ID/path/source/reserved-ID failures are synchronous preflight errors mapped to HTTP 400 in Actions; existing-target-without-replace and idempotency conflicts map to 409. MCP returns the same `{ error: { code, message } }` with `isError: true`. No job is persisted for these failures.
- Preflight performs no DNS, GitHub, archive, or remote-content work. Accepted new jobs start queued. Same-key/same-canonical-request deduplication returns the original job and sets `deduplicated: true`, even if it is already terminal.
- Frozen start response fields: `installId`, `id`, `status`, `deduplicated`, `createdAt`, `updatedAt`, and `pollAfterMs`.
- Confirmed question 15A plus the execution-lease addendum: status is `queued | running | completed | failed | cancelled`; phase is optional and one of `resolving | downloading | extracting | validating | waiting_for_target | committing | activating`. `waiting_for_target` remains cancellable; the non-cancellable boundary begins only after the exclusive lease is acquired and `committing` is durably entered.
- Frozen status fields: `installId`, `id`, monotonic `revision`, `status`, optional `phase`, `attempt`, `maxAttempts`, `progress`, redacted `source`, `createdAt`, optional `startedAt`, `updatedAt`, optional `finishedAt`, `elapsedMs`, optional `cancelRequestedAt`, optional `result`, optional `error`, and `pollAfterMs`.
- Progress fields: `filesCompleted`, `filesTotal`, `bytesDownloaded`, and optional `bytesTotal`. Terminal records return `pollAfterMs: 0`; queued/running default to 1000 ms.
- Core async error codes: `source_not_found`, `ref_not_found`, `download_blocked`, `download_failed`, `download_timeout`, `digest_mismatch`, `archive_invalid`, `package_limit_exceeded`, `skill_md_missing`, `skill_invalid`, `target_changed`, `target_busy`, `install_deadline_exceeded`, `activation_failed`, `recovery_failed`, and `internal_error`. Errors contain `code`, safe `message`, optional `phase`, and `retryable`; `target_busy` is retryable and leaves the visible package unchanged.
- Confirmed question 16A: cancel response fields are `installId`, `outcome`, `changed`, `status`, optional `phase`, and optional `cancelRequestedAt`. Outcomes are `cancel_requested | cancelled | already_cancelled | too_late | already_terminal`; all are successful lifecycle results. Unknown/expired IDs return `install_not_found`.
- Confirmed question 17A: activation is journaled as part of the transaction. If activation cannot be durably applied, the new candidate is removed from the visible target and an archived previous package is restored before the job settles failed. Recovery continues/rolls back this sequence after restart; it never reports a failed partial replacement as installed.

## Source and surface decisions — third handoff batch completion

- Clarified question 13A-A: retain both discriminated source variants. `github` accepts canonical structured `repository` plus optional `ref`/`path`, or a supported unambiguous GitHub `url`; `files` retains the non-empty list of package-relative entries using exactly one of `url`, UTF-8 `content`, or `contentBase64`.
- Frozen install request top-level fields: required `id`, required discriminated `source`, `replaceExisting` default false, optional context-sensitive `activateAfterInstall`, and optional Room-scoped `idempotencyKey`.
- New install IDs use `^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$`; internal dot-directory names and built-in IDs are reserved. `idempotencyKey` is 1–128 characters.
- File entries have required relative `path`, exactly one acquisition value, optional expected `sha256`, and optional `executable`. All variants pass through the same digest, size, path, validation, staging, and commit pipeline.
- Confirmed question 18A Actions paths: `POST /v1/room/skills/install`, `POST /v1/room/skills/install/get`, and `POST /v1/room/skills/install/cancel`.
- Frozen annotations: install is non-read-only, destructive/consequential, and open-world; get is read-only, non-destructive/non-consequential, and closed-world; cancel is non-read-only, destructive/consequential, and closed-world; read remains read-only, non-destructive/non-consequential, and closed-world.

## Read, package, limits, and recovery decisions — second handoff batch

- `SkillReadRequest` gains optional `path`. Omission returns the exact existing `{ skill }` response. A provided file path retains `skill` and adds `resource { path, encoding, content, mediaType?, sizeBytes, sha256 }`.
- Resource encoding is automatic: valid UTF-8 uses `utf8`, otherwise `base64`. Directories fail `not_a_file`; resources above 1 MiB fail `resource_too_large`; V1 neither truncates nor supports ranges.
- File sources accept `executable?: bool`. Ordinary files normalize to `0644`, executable files and directories to `0755`. GitHub extraction preserves only whether the owner executable bit was set; all special bits, symlinks, and hardlinks are rejected.
- Default resource limits: 256 files; 10 MiB per file; 50 MiB expanded package; 25 MiB GitHub archive download; 256 KiB `SKILL.md`; 2 MiB raw inline content per install request; 1 MiB `skills.read`; 240 UTF-8 bytes per relative path; depth 16.
- Default network/work limits: connect 10 seconds; HTTP request 120 seconds; continuous idle 30 seconds; install deadline 600 seconds; five redirects; two running installs; four parallel file downloads per install.
- Public status exposes GitHub repository/requested ref/resolved commit/path and file path/size/digest/source type. It strips URL query/fragment values and never returns inline text/base64. Private persisted request data remains Room-local for recovery.
- Retryable conditions are network errors plus HTTP 408, 425, 429, 500, 502, 503, and 504. `maxAttempts` is three total attempts; waits before attempts two and three are one and two seconds. `Retry-After` is honored up to 30 seconds within the ten-minute deadline. Validation, 404, permission, and policy failures do not retry.
- Restart recovery preserves terminal states; queued jobs resume; pre-commit running jobs discard staging and requeue with incremented attempt; committing jobs reconcile a durable journal to finish the candidate commit or restore the archive. Ambiguous unsafe state fails without further mutation.
- These defaults belong to a backward-compatible `room.skills` configuration subtree and are not hard-coded into MCP argument schemas.

## Default activation — decided

- `skill-installer` is active by default so a consuming agent can immediately discover the installation workflow.
- The current `active-skills.json` only stores positive activation records. Simply computing every built-in default as active would make an explicit deactivation disappear on restart.
- Extend the state schema with a version/default-disable tombstone representation. On Room initialization, reconcile a missing `skill-installer` activation into legacy state unless a tombstone exists.
- `skills.deactivate` removes its positive activation and persists the tombstone; `skills.activate` removes the tombstone and writes a fresh activation record.
- Built-in package immutability and activation are separate: the package cannot be replaced, but the user can deactivate it.

## Installation cancellation — decided

- Public name: `skills.install.cancel`, Room-scoped and without `agentId`; expose the same operation in GPT Actions/OpenAPI.
- The request identifies `installId`. Cancellation is idempotent: repeating it never creates a second transition or a transport-level error.
- A queued job can transition directly to `cancelled`. A running job first persists `cancelRequestedAt`, signals its worker, aborts cancellable I/O/extraction, cleans staging, and then becomes `cancelled`.
- The worker checks cancellation while holding the job transition lock immediately before entering `committing`. Once `committing` is persisted, cancellation returns the current state with a stable `too_late` outcome; it does not interrupt archive/rename or roll back a successfully installed package.
- Cancelling an already cancelled job returns unchanged cancelled state. Cancelling `completed` or `failed` returns unchanged terminal state with an `already_terminal` outcome.
- `cancelled` records use the same seven-day/100-terminal-record retention policy.

## Remaining policy terms and recommended defaults

### URL host policy — decided

- This applies to arbitrary `files[].url` downloads, not the structured GitHub source.
- Without a policy, a Room could be induced to access an intranet, cloud metadata endpoint, or a private address reached after a redirect.
- V1 default: accept public HTTPS only; reject loopback, link-local, private, and reserved addresses after every DNS resolution and redirect; also allow deployments to configure an optional host allowlist to narrow access further.
- The GitHub adapter accepts public repositories only and uses supported GitHub hosts/APIs rather than inheriting the arbitrary URL scope.

### Job retention — decided

- This means how long the `installId` status, timings, source summary, result, and error remain queryable. It does not apply to the installed skill or to `skills/.archive`.
- V1 default: never prune running jobs; retain terminal jobs for 7 days with a cap of the most recent 100 records, pruning when either limit is exceeded; delete download staging data immediately after a terminal result.
- Archive cleanup should remain a separate future capability and must not be coupled to job-record retention.

### Bounded wait — decided

- This means allowing a status query to wait briefly for a change before returning, for example `skills.install.get({ installId, waitSeconds: 3 })`, similar to lightweight long polling.
- It can reduce frequent client polling and may let a short install finish within one query, but it consumes Apps SDK call time and adds timeout/disconnect semantics.
- V1 behavior: `skills.install.get` waits up to 5 seconds by default and accepts an integer `waitSeconds` override. `0` requests an immediate snapshot; the initial schema range is 0–30 seconds.
- Terminal jobs return immediately. For a non-terminal job, the Room captures the current durable job revision and returns when a newer status/phase/progress revision is published or the bound expires; expiry returns the latest snapshot rather than a timeout error.
- The response continues to include `pollAfterMs`, so callers have a server recommendation even when their requested wait expires without a change.

## Phase 5 implementation findings

- The Room Agent now resolves GitHub sources through the public GitHub API, pins the selected ref to a commit SHA, walks the commit tree, and downloads raw blobs through a redirect-disabled client. This avoids invoking archive extractors or executing package content during installation.
- Convenience GitHub URLs are limited to `github.com/{owner}/{repo}` and unambiguous `tree`/`blob` forms; query/fragment and credential-bearing URLs are rejected. Structured `repository`/`ref`/`path` remains the canonical form for branch names or paths that need no URL guessing.
- Arbitrary file URLs require HTTPS and no credentials. Every redirect is resolved and revalidated through DNS against public/non-reserved IP ranges; an optional exact host allowlist can narrow the deployment policy. GitHub API/raw hosts are fixed adapter endpoints rather than arbitrary caller hosts.
- All sources materialize into the same staging tree, apply the same size/digest/path checks, normalize files to `0644`/`0755`, normalize directories to `0755`, and require `SKILL.md` before the existing atomic archive/rename transaction. GitHub modes other than regular or owner-executable files (including symlinks) are rejected.
- Case-folded duplicate paths and file/directory prefix collisions are rejected before a package can be committed. Remote transient network/HTTP failures retry up to the configured total-attempt limit with bounded backoff and total-deadline enforcement; permission, source-not-found, policy, and validation errors do not retry.
- `room.skills` owns the accepted limits and network policy. The Room startup creates the install worker semaphore from `maxConcurrentInstalls`; other limits are read at job execution so config remains backward-compatible and does not enter public MCP arguments.
- Chosen GitHub tree/blob resolution is intentionally more inspectable and safer than a single archive download for V1: each file receives an individual digest/source summary, and archive symlink/special-mode handling is never delegated to an external extractor.

## Phase 6 implementation findings

- The Hub now keeps the Room boundary intact: all install/run Actions and MCP calls omit input `agentId`, generate request/session IDs at the Hub, and route with `request_active_room`. Semantic Room errors are converted to Actions 400/404/409 responses while MCP retains the structured error envelope.
- `skills.run` resolves only active workspace-backed regular executable files beneath `scripts/`; built-ins and inactive skills fail before session creation. The executable is canonicalized and every package component is checked for symlink traversal; cwd still uses existing Room path-policy validation.
- The shared session map now supports `starting`, `waiting_confirmation`, and an optional child. Accepted sessions are visible before confirmation; `session.kill` sets a cancellation flag even before a child exists. A lightweight monitor refreshes child state so leases and audit records are released even if callers never poll.
- Per-skill Tokio reader/writer locks provide writer fairness: running skill sessions hold an owned read guard, replacement commits wait for an exclusive guard, and new runs use `try_read_owned` so they fail immediately with `skill_update_pending` once a writer is queued. Install waits are cancellable and deadline failures remain retryable `target_busy`.
- Terminal session records are memory-only, pruned at 24 hours or beyond the newest 100 while active/pending sessions remain. Hub cached sessions are cleared when an agent connection is replaced or disconnected, preventing stale `running` results after restart.
- Skill execution audits use the existing JSONL audit stream and add `skillId`, package-relative `skillPath`, installed package digest, session ID, inherited policy/confirmation outcome, terminal result, cwd/args, and duration without changing generic audit callers.
