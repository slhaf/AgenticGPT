# Task Plan: Room-scoped skills installer and asynchronous installation

## Goal

Extend the existing Room Agent-owned skills subsystem with a built-in `skill-installer`, skill-scoped package reads, resilient asynchronous installation from GitHub/remote file manifests/inline content, and session-backed execution of installed skill scripts, while keeping the Hub as an MCP-facing routing and transport layer.

## Current Phase

Phase 2 — protocol and package model; all product-level contracts are frozen

## Scope and constraints

- AgenticGPT is a runtime/tool layer for other agents, not a complete autonomous agent.
- Skills management remains exclusively owned by the active Room Agent and never accepts `agentId`.
- ChatGPT Apps is a primary client; no installation tool call may depend on a long synchronous MCP request.
- The Hub transport run and the Room Agent installation job are separate lifecycles.
- Network sources are resolved and downloaded by the Room Agent; callers do not need to restate remote file contents.
- Inline UTF-8 and base64 content remain supported for generated or otherwise non-downloadable files.
- Existing workspace skills may be replaced only after the old package is moved under `skills/.archive`; replacement must not hard-delete the previous package.
- V1 GitHub installation targets public repositories only.
- Arbitrary URL-backed files accept public HTTPS by default with DNS/redirect SSRF revalidation; deployments may optionally narrow access with a host allowlist.
- Terminal installation records are retained for seven days and capped at the most recent 100 records; active jobs are never pruned and archives are outside this policy.
- Status lookup uses bounded long polling: `waitSeconds` defaults to 5, `0` means immediate lookup, and the initial protocol maximum is 30 seconds.
- The built-in `skill-installer` is active by default, but an explicit user deactivation is persisted and survives restart or upgrade.
- V1 exposes idempotent cooperative cancellation as `skills.install.cancel`; cancellation cannot interrupt or roll back a job after it enters atomic commit.
- Each install job targets exactly one explicitly identified skill; one package may still contain many file entries.
- Replacing an existing workspace skill requires explicit `replaceExisting: true`; omission defaults to refusing the collision.
- A newly installed skill is active by default. Replacement preserves the target's prior active state unless activation is explicitly requested, and installation never implicitly deactivates an active skill.
- `skills.install` accepts an optional Room-scoped `idempotencyKey` for deduplication across fresh MCP/Actions retries.
- GitHub structured input is canonical, GitHub web URLs are accepted as a convenience, omitted refs use the repository default branch, and every job records the resolved commit SHA.
- `skills.read` remains backward compatible: optional `path` adds a bounded file-only resource result with UTF-8/base64 auto-encoding, while omission preserves the existing response.
- Package modes are normalized to directories/executable files `0755` and ordinary files `0644`; arbitrary modes, symlinks, and hardlinks are rejected.
- Room skill installation uses the confirmed configurable size, count, timeout, redirect, concurrency, retry, and total-deadline defaults recorded below.
- Public status returns redacted reproducibility metadata and never echoes inline content, URL query/fragment data, or other download secrets.
- Transient network failures receive at most three total attempts; restart recovery requeues pre-commit work and reconciles commit journals without changing terminal jobs.
- Local request, ID/path/source, reserved-ID, collision, and idempotency conflicts fail synchronously without creating a job; Actions maps them to semantic 400/409 responses while MCP returns the same structured error as `isError`.
- Start, status, and cancel responses use the confirmed stable fields/enums; cancellation races are successful outcome values rather than tool errors.
- Activation is part of the installation transaction. If post-commit activation cannot complete, the candidate is rolled back and any archived previous package is restored through the commit journal.
- V1 retains both `github` and `files` sources. Structured GitHub fields are canonical, supported GitHub URLs are convenience input, and files entries retain URL/UTF-8/base64 acquisition.
- Actions use the existing POST operation style at `/v1/room/skills/install`, `/install/get`, and `/install/cancel`; install/cancel are consequential and destructive, while get/read are read-only.
- MCP and GPT Actions contracts are updated together.
- `skills.run` is Room-scoped and accepts `id`, package-relative `path`, optional direct `args`, optional path-policy-validated `workingDirectory`, and optional `waitSeconds`; it never accepts `agentId`, arbitrary `program`, shell text, or environment overrides.
- `skills.run` always creates a managed session before bounded waiting. `waitSeconds` defaults to 5 and is limited to 0–30; terminal sessions return inline, while non-terminal sessions return identifiers for existing `session.inspect`/`session.wait`/`session.kill` tools.
- Runnable targets are active workspace-backed skills only. The script must be a canonical-contained, non-symlink, regular executable file under package-relative `scripts/`; embedded built-ins are not runnable in V1.
- Skill sessions inherit the Room Agent's existing execution policy, confirmation defaults, environment, path policy, and optional generic sandbox. `skills.run` is a semantic/provenance wrapper, not a separate privilege boundary.
- `skills.run` defaults cwd to the Room workspace root; optional `workingDirectory` uses the exact existing session/path-policy validation.
- Skill session terminal data remains in memory for at most 24 hours and the newest 100 terminal records; active and confirmation-pending sessions are never pruned. Room Agent restart does not restore sessions and later lookup returns `session_not_found`.
- A running/pending skill session holds a shared per-skill execution lease. Replacement commit obtains a writer-fair exclusive lease; new runs return `skill_update_pending` while commit waits, and install deadline expiry returns retryable `target_busy` without mutating the visible package.
- MCP and Actions both expose `skills.run`; Actions uses `POST /v1/room/skills/run`, and execution is non-read-only, destructive, open-world, and consequential.
- Existing unrelated working-tree changes, including `AGENTS.md`, must be preserved.

## Phases

### Phase 0: Repository and architecture discovery

- [x] Trace the current `skills.list/read/search/active/activate/deactivate` path.
- [x] Confirm all skills commands route through the active Room Agent.
- [x] Inspect the existing Hub `agent_runs` reliable transport lifecycle.
- [x] Review ChatGPT Apps MCP tool-design guidance relevant to asynchronous workflows.
- [x] Record findings in `findings.md`.
- **Status:** complete

### Phase 1: Freeze public contracts and acceptance criteria

- [x] Finalize `skills.read` request and response semantics for `SKILL.md` and package-relative paths.
- [x] Finalize the `SkillInstallSource` variants: retain `github` and `files`, with structured GitHub canonical and URL convenience input.
- [x] Finalize file entries with exactly one of `url`, `content`, or `contentBase64`, plus optional digest and executable flag.
- [x] Finalize `skills.install` start response and `skills.install.get` status response.
- [x] Fix public tool names as `skills.install`, `skills.install.get`, and `skills.install.cancel`.
- [x] Decide built-in `skill-installer` activation policy: active by default with persistent explicit deactivation.
- [x] Decide collision policy: archive and replace existing workspace skills without hard deletion.
- [x] Decide initial private GitHub support: public repositories only in V1.
- [x] Decide URL-host policy: public HTTPS with SSRF checks and optional deployment allowlist.
- [x] Decide terminal record retention: seven days and at most 100 terminal records.
- [x] Decide status-wait behavior: bounded wait defaults to 5 seconds and is caller-configurable in seconds.
- [x] Decide cancellation scope: expose an idempotent V1 cancel tool with a non-cancellable atomic-commit boundary.
- [x] Decide install cardinality: one explicit target skill per job.
- [x] Decide target identity: top-level `id` is required for every source.
- [x] Decide collision intent: replacement requires explicit `replaceExisting: true`.
- [x] Decide activation after install: new skills default active; replacement preserves prior state unless activation is explicitly requested.
- [x] Decide cross-call retry deduplication: optional `idempotencyKey` with conflict detection.
- [x] Decide GitHub input shape: structured canonical form plus URL convenience parsing, default-branch resolution, and commit pinning.
- [x] Define terminal states, phases, progress fields, core error taxonomy, cancellation outcomes, and retryability.
- [x] Define exact size, count, timeout, redirect, retry, concurrency, deadline, and retention defaults.
- [x] Decide skill-script isolation policy: inherit the Room Agent's existing session policy, path policy, environment, and optional generic sandbox; do not introduce a mandatory skill-only sandbox.
- [x] Decide replacement concurrency: a skill run holds a per-skill execution lease through session terminal state; install may prepare concurrently but commit waits for the exclusive lease and fails retryably on its deadline.
- [x] Expose a restricted Room-scoped `skills.run` wrapper over the existing managed-session engine.
- [x] Use a bounded inline wait: always create the session first, return terminal output inline when it finishes in time, otherwise return identifiers for `session.inspect`/`session.wait`/`session.kill`.
- [x] Register the session before policy confirmation as `waiting_confirmation`; confirmation runs asynchronously and the pending session is inspectable and cancellable.
- [x] Freeze remaining `skills.run` package containment, active-state, terminal retention/restart, response schema, audit semantics, and MCP/Actions exposure.
- **Status:** complete

### Phase 2: Protocol and package model

- [ ] Add protocol types for package-relative reads and binary/text results.
- [ ] Add the frozen `SkillInstallRequest`: required `id` and discriminated `source`; default-false `replaceExisting`; optional `activateAfterInstall`; optional `idempotencyKey`.
- [ ] Add `github` source with exactly one of canonical `repository` or convenience `url`, plus compatible optional `ref`/`path`; add `files` source with a non-empty bounded file list.
- [ ] Add file entries with required relative `path`, exactly one of `url`/`content`/`contentBase64`, optional SHA-256, and optional `executable`.
- [ ] Add protocol types for install start, status/progress/source/result/error, bounded get, and cancellation outcomes.
- [ ] Include cancellable install phase `waiting_for_target` before non-cancellable `committing`, plus retryable `target_busy` for lease deadline expiry.
- [ ] Add `SkillRunRequest { id, path, args?, workingDirectory?, waitSeconds? }` with camelCase serialization, default 5-second wait, and a 0–30 range enforced at the public boundary.
- [ ] Add `SkillRunResponse { agentId, sessionId, completedInline, pollAfterMs, session }` and stable session states `starting | waiting_confirmation | running | exited | failed | killed`.
- [ ] Model `id`, `replaceExisting`, context-sensitive post-install activation, and optional `idempotencyKey` explicitly in the install request schema.
- [ ] Add `HubCommand` variants for install start, status lookup, cancellation, and Room-scoped skill run with a Hub-generated session ID.
- [ ] Add command serde-name and request-ID regression tests.
- [ ] Add origin/read-only metadata needed to distinguish built-in and workspace skills.
- [ ] Define a versioned persisted installation-job schema independent of Hub `agent_runs`.
- **Status:** pending

### Phase 3: Built-in skill and scoped read behavior

- [ ] Add the source-controlled `skill-installer/SKILL.md` package under the local-agent crate.
- [ ] Embed built-in skills into the Room Agent binary and merge them with workspace discovery.
- [ ] Reserve built-in IDs and define deterministic same-ID collision behavior.
- [ ] Reconcile `skill-installer` into active state on Room initialization and persist a disabled-default tombstone when a user explicitly deactivates it.
- [ ] Migrate existing `active-skills.json` files compatibly so upgrade activates the new default once, while later user deactivation survives restarts and upgrades.
- [ ] Explicitly exclude `skills/.archive` and other internal dot-directories from workspace skill discovery.
- [ ] Extend `skills.read` to normalize and validate package-relative paths.
- [ ] Preserve the exact legacy response when `path` is omitted; when present, append `resource { path, encoding, content, mediaType?, sizeBytes, sha256 }` while retaining `skill`.
- [ ] Support file-only UTF-8/base64 auto-encoding up to 1 MiB without truncation, range reads, path escape, directory reads, or symlink traversal.
- [ ] Add tests for built-in list/read/search/activation behavior and scoped reads.
- [ ] Return `skill_not_runnable` for embedded built-ins in V1; do not materialize embedded scripts onto disk.
- **Status:** pending

### Phase 4: Persistent Room Agent installation job engine

- [ ] Create per-install persisted request/status storage under Room workspace state.
- [ ] Make job creation idempotent across reliable command replay.
- [ ] Return `installId`, `queued`, and `pollAfterMs` before any network operation.
- [ ] Implement worker queue/semaphore and per-target skill locking.
- [ ] Track stable job status plus detailed phases including `waiting_for_target`, progress, timestamps, elapsed time, result, and error.
- [ ] Return stable start/status/cancel structs with monotonic revision, attempt counters, progress, redacted source, timestamps, elapsed time, result/error, cancellation time, and polling guidance.
- [ ] Implement bounded status waiting that returns early on terminal state or a newer job revision, returns the latest snapshot on timeout, and treats `waitSeconds: 0` as an immediate read.
- [ ] Persist `cancelRequestedAt`, make cancel replay idempotent, and signal queued/running workers cooperatively.
- [ ] Cancel queued jobs immediately; abort cancellable network/extraction work and clean staging for running jobs.
- [ ] Define the transition into `committing` as the cancellation boundary: check and persist cancellation before entering it, reject later cancellation as too late, and let a successful commit finish as `completed`.
- [ ] Reconcile queued/running/partially committed jobs after Room Agent restart.
- [ ] Retry only transient network/status failures for at most three total attempts using exponential backoff and capped `Retry-After`; expose `attempt` and `maxAttempts`.
- [ ] Keep downloads outside the final `skills_writes` critical section.
- [ ] Commit only through staging validation and atomic rename.
- [ ] Include default/explicit activation in the durable commit journal; if activation fails, remove the candidate and restore the archived previous package so the job has no partial-success result.
- [ ] On replacement, move the old package to `skills/.archive/<id>/<archive-entry>` before committing the new package and roll back if the second rename fails.
- [ ] Implement writer-fair per-skill execution leases: run acquisition fails with `skill_update_pending` once commit is pending; commit waits cancellably before the non-cancellable boundary and fails retryably with `target_busy` at the install deadline.
- [ ] Prune terminal job records after seven days or beyond the newest 100 records, never prune active jobs, and remove staging data at terminal completion.
- **Status:** pending

### Phase 5: Source resolution and secure download pipeline

- [ ] Normalize GitHub repository/ref/path or canonical GitHub URL into a resolved commit and package subtree.
- [ ] Reject ambiguous GitHub tree URLs, credential-bearing URLs, unsupported hosts, and conflicting structured/URL fields; direct callers to structured `ref`/`path` when URL parsing is ambiguous.
- [ ] Select and implement the GitHub download strategy, favoring a single commit-pinned archive when safe.
- [ ] Normalize `files` entries into one internal resolved file plan.
- [ ] Normalize `executable` into `0644`/`0755`, preserve only the GitHub executable bit, use `0755` directories, and reject arbitrary/special modes.
- [ ] Download URL-backed entries directly into the staging package.
- [ ] Materialize inline UTF-8/base64 entries through the same plan.
- [ ] Accept public HTTPS URL entries by default, revalidate DNS and every redirect against private/reserved ranges, support an optional narrowing host allowlist, and enforce bounded resource use.
- [ ] Add defaulted `room.skills` configuration for the accepted package/read/inline ceilings, network timeouts, redirects, concurrency, retries, and total deadline.
- [ ] Reject absolute paths, traversal, symlinks/hardlinks, duplicates, case conflicts, and package-limit violations.
- [ ] Validate required `SKILL.md`, compute content digests, and save provenance.
- [ ] Ensure installation never executes bundled scripts.
- **Status:** pending

### Phase 6: Hub, MCP, Actions, and Apps workflow integration

- [ ] Route install start/status/cancel through `request_active_room` without `agentId`.
- [ ] Implement Room-side `skills.run` validation: active workspace skill, `scripts/` containment, no symlink traversal, regular executable file, and existing-policy-validated cwd before session creation.
- [ ] Refactor the shared managed-session engine to support `starting`/`waiting_confirmation`, asynchronous cancellable confirmation, an optional child, event-driven state notification, and a skill execution-lease guard released on every terminal path.
- [ ] Implement the hybrid run path: create the session first, wait event-driven for up to `waitSeconds`, return immediately on terminal state, and otherwise return the current session identity/state without creating a second job system.
- [ ] Retain terminal sessions in memory for 24 hours and at most 100 entries, never prune active/pending sessions, and clear stale Room session cache on disconnect/restart so later inspection returns `session_not_found` rather than stale `running`.
- [ ] Keep `hub.run.get` limited to Hub-to-Agent command delivery and late-result recovery.
- [ ] Expose `skills.install`, `skills.install.get`, `skills.install.cancel`, and `skills.run` as MCP tools with predictable output schemas.
- [ ] Add POST Actions routes `/v1/room/skills/install`, `/v1/room/skills/install/get`, `/v1/room/skills/install/cancel`, and `/v1/room/skills/run` plus strict OpenAPI schemas in the same change as MCP support.
- [ ] Set frozen annotations: install non-read-only/destructive/open-world and consequential; get read-only/non-destructive/closed-world and non-consequential; cancel non-read-only/destructive/closed-world and consequential; run non-read-only/destructive/open-world and consequential; read remains read-only/non-destructive/closed-world.
- [ ] Extend MCP server instructions with install start/poll/cancel and skill-run bounded-inline/session-follow-up workflows, including `waitSeconds` and `pollAfterMs` guidance.
- [ ] Extend audit records for skill ID, package-relative script path, installed digest, session ID, args, resolved cwd, policy/confirmation outcome, exit state, and duration.
- [ ] Ensure structured content exposes reusable IDs and only necessary status data.
- [ ] Redact URL query/fragment data and omit inline content from public status/logs while returning GitHub commit provenance plus file path/size/digest summaries.
- **Status:** pending

### Phase 7: Verification, documentation, and release readiness

- [ ] Add unit tests for validation, GitHub parsing, file plans, progress, errors, and restart reconciliation.
- [ ] Add integration tests covering Hub → active Room Agent → asynchronous job → final skill read.
- [ ] Test reliable replay around job creation and crashes before/after atomic commit.
- [ ] Test Apps-style immediate start response and bounded status polling, including default 5-second wait, immediate/terminal early return, caller override, and timeout snapshot behavior.
- [ ] Test cancel replay, queued/running cancellation, download interruption, staging cleanup, repeated cancellation, terminal cancellation, and the commit-boundary race.
- [ ] Test malicious archives/URLs, redirects, private IP resolution, oversized content, and symlink escape.
- [ ] Test normalized executable modes, strict resource ceilings, no-truncation reads, status redaction, retry classification/backoff, deadline expiry, and restart reconciliation.
- [ ] Test `skills.run` preflight errors without session creation, active/workspace/script containment, executable/symlink checks, cwd validation, stable response fields, default/0/30-second waits, early terminal return, and event-driven wake-up.
- [ ] Test `starting`/`waiting_confirmation`/approval/denial/kill transitions, policy/spawn failures after session creation, existing session inspect/wait/kill follow-up, 24-hour/100-terminal pruning, and restart `session_not_found` behavior.
- [ ] Test writer fairness and run/install races: active sessions delay only commit, pending commit rejects new runs, cancellation remains effective while waiting, terminal release unblocks commit, and deadline failure leaves the visible package unchanged.
- [ ] Test `skills.run` MCP/Actions parity, Room routing without input `agentId`, response-provided session identifiers, audit metadata, and destructive/open-world/consequential annotations.
- [ ] Update `openapi/hub.yaml`, interface docs, README usage, configuration docs, and operational verification.
- [ ] Run formatting, workspace tests, and targeted regression suites.
- [ ] Review the diff for security-policy, OpenAPI, configuration, and migration impacts.
- **Status:** pending

## Acceptance criteria

1. `skills.install` normally returns a persisted `installId` without waiting for DNS, GitHub, archive, or file downloads.
2. `skills.install.get` reports durable state and can return the final installed skill summary or a structured retryable/non-retryable error; it waits up to 5 seconds by default, accepts bounded `waitSeconds`, and returns immediately for terminal state or `waitSeconds: 0`.
3. A replayed Hub command cannot create a second job or commit a package twice.
4. GitHub, remote URL file entries, and inline content share one validation and atomic-commit pipeline.
5. No source can write outside `workspace/skills/<id>` or its private staging directory.
6. The final skill is invisible until fully validated and atomically committed.
7. Replacing an existing workspace skill preserves the prior package under `skills/.archive` and restores it if final commit fails.
8. `skills.read` can read `SKILL.md` and safe package-relative resources without a second resource-specific tool.
9. Built-in `skill-installer` remains available through the Room Agent and cannot be silently shadowed.
10. Hub transport status and install business status remain distinguishable in APIs and documentation.
11. All new skills tools remain Room-scoped and omit `agentId` from MCP and OpenAPI schemas.
12. `skill-installer` is active after first upgrade/start, but `skills.deactivate` records an explicit disabled-default tombstone so it stays inactive until reactivated.
13. `skills.install.cancel` is idempotent, quickly requests cancellation for queued/running work, cleans private staging, and never interrupts or rolls back an atomic commit already in progress.
14. One `installId` affects one explicit target `id`; the server never infers the destination from downloaded content.
15. Existing targets are not replaced unless `replaceExisting: true`, after which the archive-and-rollback invariant applies.
16. New skills become active by default; replacement preserves existing activation unless explicitly activated, and a false/omitted activation option never deactivates an existing skill.
17. Reusing an `idempotencyKey` with the same canonical request returns the original `installId`; reusing it with different content returns `idempotency_conflict`.
18. GitHub jobs expose and persist the resolved public repository, subpath, requested ref, and immutable commit SHA.
19. Existing `skills.read { id }` callers receive the legacy shape; an explicit file `path` adds a UTF-8/base64 resource object, rejects directories and files above 1 MiB, and never truncates.
20. Installed package entries have normalized safe permissions only, with no symlink/hardlink or arbitrary-mode preservation.
21. Default limits reject more than 256 files, a file above 10 MiB, an expanded package above 50 MiB, a GitHub archive above 25 MiB, `SKILL.md` above 256 KiB, inline raw content above 2 MiB, paths above 240 UTF-8 bytes, or depth above 16.
22. Default networking uses 10-second connect, 120-second request, 30-second idle, five redirects, a ten-minute job deadline, two concurrent installs, and four downloads per install.
23. Public job status contains redacted reproducibility metadata but no inline payloads or URL query/fragment values.
24. Retryable network/HTTP failures receive no more than three total attempts; restart preserves terminals, requeues pre-commit work from clean staging, and resolves commit-journal state deterministically.
25. Invalid/reserved requests return 400, collisions/idempotency conflicts return 409, and neither creates an install job; network/content failures occur asynchronously on an accepted job.
26. A successful start returns `installId`, target ID, current status, deduplication flag, created/updated timestamps, and `pollAfterMs`; a deduplicated request may return any current status of the original job.
27. Status uses the frozen stable status/phase enums, monotonic `revision`, attempt/progress/timing/source/result/error fields, and `pollAfterMs: 0` in terminal state.
28. Cancellation returns one of `cancel_requested`, `cancelled`, `already_cancelled`, `too_late`, or `already_terminal` as an idempotent success outcome; only an unknown/expired ID errors.
29. Activation failure cannot leave a reported-failed candidate installed: journal recovery rolls back the candidate and restores any prior archive before final failure.
30. Both source variants remain supported: canonical/URL GitHub acquisition and explicit files with URL, UTF-8, or base64 content all normalize to the same package pipeline.
31. Actions expose the four frozen POST paths for install/get/cancel/run, and MCP/Actions annotations consistently identify install/cancel/run as consequential mutations and get/read as read-only.
32. A `skills.run` session holds a shared execution lease for its skill until terminal state. Replacement preparation may proceed concurrently, but commit waits for the exclusive lease; once commit is pending, new runs fail retryably with `skill_update_pending`. Install cancellation remains effective while waiting, and deadline expiry returns retryable `target_busy` without changing the visible package.
33. A valid `skills.run` request creates and returns a managed session identity before any policy confirmation. Confirmation-required sessions enter `waiting_confirmation`; approval transitions to execution, denial transitions terminally with a safe reason, and `session.kill` can cancel the pending confirmation. The initial bounded wait includes this phase and never waits for confirmation before a session ID exists.
34. `skills.run` accepts required `id` and package-relative `path`, optional direct `args`, optional path-policy-validated `workingDirectory`, and optional `waitSeconds`; it omits input `agentId`/`program`/shell/environment, defaults cwd to Room workspace root, and limits waiting to 0–30 seconds with default 5.
35. Only active workspace-backed skills are runnable. The target is a canonical-contained non-symlink regular executable beneath `scripts/`; invalid/inactive/built-in/path/cwd/update-pending requests fail before session creation.
36. Both inline-complete and still-running responses contain `agentId`, `sessionId`, `completedInline`, `pollAfterMs`, and `session`. Terminal state returns `completedInline: true` and `pollAfterMs: 0`; otherwise callers reuse existing session inspect/wait/kill tools.
37. Skill sessions use the existing Room execution policy and retain terminal state only in memory for 24 hours/the newest 100 entries; active and pending-confirmation sessions are not pruned, and Room Agent restart yields `session_not_found` rather than stale state.
38. `skills.run` is exposed through MCP and `POST /v1/room/skills/run`, is non-read-only/destructive/open-world/consequential, and audits skill ID, path, package digest, session ID, args, cwd, policy/confirmation, terminal state, and duration.

## Key questions

None. Product-level contracts are frozen; implementation may only reopen a decision when repository evidence shows a hard incompatibility.

## Decisions made

| Decision | Rationale |
|----------|-----------|
| Keep all skills operations on the active Room Agent | Skills and their state belong to the Room workspace; `agentId` would weaken this boundary. |
| Reuse and extend `skills.read` rather than add resource-specific read tools | It is already skill-scoped and can naturally accept a package-relative path. |
| Support GitHub, URL-backed file entries, and inline content | Remote downloads avoid AI content repetition while inline data preserves generated/local use cases. |
| Make install start asynchronous | Network and archive work cannot safely depend on ChatGPT Apps/MCP request duration. |
| Separate Hub `runId` from Room `installId` | `agent_runs` tracks reliable transport, while install jobs need business phases, progress, recovery, and longer retention. |
| Persist installation jobs in Room workspace state | Preserves Room ownership and enables restart reconciliation. |
| Normalize all source variants to one internal install plan | Avoids separate security and atomicity behavior per source type. |
| Do not execute scripts during installation | Installation is package acquisition and validation, not code execution. |
| Archive and replace existing workspace skills | Allows updates while preserving rollback/recovery material instead of hard deletion. |
| Store replaced packages under `skills/.archive/<id>/...` | Keeps archives inside the skills domain while allowing discovery to skip a reserved internal directory. |
| Limit V1 GitHub support to public repositories | Avoids credential transport, storage, sandbox, and private-repository authentication complexity in the first release. |
| Update MCP and GPT Actions together | Keeps the two documented public Hub integration surfaces behaviorally aligned. |
| Allow public HTTPS URL entries with SSRF revalidation and an optional narrowing allowlist | Supports direct package acquisition without making Room deployments implicitly trust private or reserved network destinations. |
| Retain terminal install records for seven days and at most 100 records | Keeps status/results queryable while bounding Room workspace growth; active jobs and skill archives follow separate lifecycles. |
| Default status waiting to 5 seconds with caller-controlled `waitSeconds` | Reduces repeated Apps tool polling while keeping the wait bounded; `0` preserves an immediate-read path. |
| Set the initial `waitSeconds` range to 0–30 | Prevents unbounded tool calls while leaving callers room to trade latency for fewer polls; this is an implementation limit rather than a retention policy. |
| Make built-in `skill-installer` active by default | Installation guidance should be available to consuming agents without a prior activation step. |
| Persist explicit deactivation of default-active built-ins | Default-active must not mean forced-active on every restart; a disabled-default tombstone preserves user intent across upgrades. |
| Expose `skills.install.cancel` in V1 | Network installs can be slow or erroneous, so callers need a Room-scoped way to stop queued/running work. |
| Make cancellation cooperative with a hard commit boundary | Network and extraction can stop safely, but interrupting the archive/rename sequence could corrupt the installed-skill invariant. |
| Limit one install job to one explicit target skill ID | Keeps locking, progress, cancellation, archive replacement, and final status atomic and unambiguous. |
| Require `replaceExisting: true` to update an existing skill | Prevents an incorrect target ID from silently replacing a package even though recovery archives exist. |
| Make new skills active by default while preserving replacement state | Makes an explicitly installed skill immediately usable without unexpectedly changing the active state of an existing target. |
| Support an optional Room-scoped `idempotencyKey` | Allows safe cross-call retries without burdening callers that intentionally want a new job. |
| Use structured GitHub input as canonical plus URL convenience parsing | Gives agents an ergonomic path while retaining an unambiguous representation for refs containing slashes and exact subpaths. |
| Add optional `path` to `skills.read` with an additive resource result | Preserves existing callers while extending the already skill-scoped read tool to safe package resources. |
| Auto-return UTF-8 or base64 and reject oversized reads without truncation | Avoids caller encoding guesswork and prevents silently incomplete instructions/scripts. |
| Normalize file permissions to `0644`/`0755` | Preserves useful executable scripts without importing arbitrary or privileged filesystem modes. |
| Adopt configurable conservative Room skill limits | Bounds Apps payloads, archive bombs, filesystem growth, and resource use while letting trusted operators tune deployments. |
| Use bounded networking and concurrency defaults | Keeps installation independent from Apps request duration without allowing unbounded Room work. |
| Expose redacted provenance only | Retains reproducibility and diagnostics without reflecting inline payloads or URL secrets through public tools/logs. |
| Retry transient failures up to three total attempts and recover from journals | Handles ordinary network/Room restarts while keeping retries bounded and atomic commit repair deterministic. |
| Reject local preflight problems before job creation | Gives callers immediate correction for invalid/conflicting requests without polluting retained job history. |
| Use semantic 400/409 Actions errors and shared MCP structured errors | Keeps transport success distinct from invalid install intent while preserving one stable error body across surfaces. |
| Freeze stable job status and cancellation outcomes | Makes Apps polling and cancellation races predictable without treating normal lifecycle outcomes as transport failures. |
| Roll back the package when activation cannot complete | Preserves all-or-nothing observable installation instead of reporting failure after silently replacing the active package. |
| Retain both GitHub and files sources | Preserves direct repository installation and generated/manifest-driven packages with URL or inline content in one pipeline. |
| Use three POST Room Actions paths | Matches the repository's existing operation-oriented Room skills API and keeps OpenAPI/MCP naming aligned. |
| Mark install/cancel consequential and destructive | Both mutate durable Room state; install can replace an existing package and cancel discards in-progress work. |
| Keep get/read read-only and non-consequential | They only inspect durable Room state or package content and do not access new external systems. |
| Reuse the Room Agent's existing session execution policy for `skills.run` | The Room is already the agent's dedicated execution boundary and exposes generic execution tools; a stricter skill-only sandbox would be bypassable and would add friction without changing effective authority. |
| Serialize replacement commit against running skill sessions | A session may read relative package resources after launch; a per-skill shared/exclusive lease avoids mixing old and new package contents without copying every package into an execution snapshot. |
| Implement `skills.run` as a bounded-wait wrapper over managed sessions | A fast script can return inline while every accepted execution still has one stable session identity for later inspection, waiting, or cancellation. |
| Register `waiting_confirmation` before prompting | Guarantees the bounded call can return a session ID even when confirmation is slow and makes the confirmation phase observable and cancellable. |
| Keep skill session terminals in memory for 24 hours/100 entries | Matches the existing managed-session lifecycle without creating another durable job system; restart explicitly loses session state. |
| Preserve Room workspace cwd semantics | Optional `workingDirectory` uses existing path-policy validation and defaults to workspace root, so scripts operate on Room data instead of modifying their installed package by default. |
| Run active workspace skill executables only | Activation continues to express user intent, while V1 avoids inventing an embedded-script materialization/cache mechanism. |
| Return one stable wrapped session response | Both fast and background paths expose the same reusable session identifiers and polling guidance. |
| Publish `skills.run` through MCP and Actions | Keeps public integration surfaces aligned; conservative mutation/open-world annotations reflect that package scripts may change Room or external state. |

## Errors encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| None during planning initialization | 1 | No action required. |
| Planning-note patch matched an outdated section heading | 1 | Read the current file tail and reapplied against the actual section. |
| Tried to inspect a non-existent Hub `src/lib.rs` | 1 | Confirmed the Hub is binary-only and continued with `src/main.rs` plus module files. |
| Decision-table row missed its closing Markdown delimiter | 1 | Detected during the immediate structure check and added the missing delimiter. |
| Phase 1 addendum and progress status diverged | 1 | Verification caught the stale `complete` marker and synchronized it to `in_progress`. |
| First status-fix patch used stale task-plan error wording | 1 | The patch was rejected atomically; inspected the actual table and reapplied against current text. |
| First script-execution findings patch used stale Phase 1 checklist wording | 1 | No partial change was applied; inspected the exact sections and revised the proposal after confirming the existing session lifecycle. |
| First execution-lease decision patch used a stale acceptance-criteria anchor | 1 | No partial change was applied; inspected the exact plan sections and reapplied against current text. |
| First all-A finalization patch used a truncated Phase 4 context | 1 | No partial change was applied; split the update into exact section-level patches and revalidated the plan. |

## Notes

- This plan is stored as the scoped plan `.planning/skill-installer`; `.planning/.active_plan` selects it as the current plan.
- Handoff entry point is Phase 2. Do not reopen frozen decisions unless repository evidence proves a hard conflict; record any such conflict before changing the contract.
- Re-read this plan before each major contract or implementation decision.
- Update `findings.md` after every two repository/doc inspection actions.
- Update phase status and `progress.md` after each completed phase.
- Implementation convention: complete and verify one phase at a time, then create one focused git commit for that phase before starting the next phase. Planning-file updates required to record the phase result belong in the same phase commit.
- The implementation handoff is accepted; implementation begins at Phase 2.
