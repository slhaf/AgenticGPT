# Progress Log: Skills installer planning

## Session: 2026-07-18

### Implementation convention

- **Status:** active
- Confirmed before implementation: each completed phase must be verified and committed as one focused git commit before the next phase begins.
- Planning-file status and progress updates for a completed phase are included in that phase's commit.

### Phase 2: Protocol and package model

- **Status:** complete
- Started after Phase 1 commit `68e4908`.
- Implemented shared request/source/status/run structs, resource-read response metadata, origin/read-only skill metadata, versioned installation-job records, Hub command variants, and serde/request-ID regression tests.
- Preserved legacy `skills.read` callers by making `path` and `resource` additive and optional.
- Added temporary Room-side structured `skills_not_implemented` responses for the new command variants; these are replaced by the install/run handlers in later phases.
- Verification passed: `cargo fmt --all`, `cargo test -p agentic-gpt-protocol` (5 tests), `cargo test -p agentic-gpt --lib` (67 tests), and `cargo check --workspace`.
- Phase 2 is ready to commit before beginning Phase 3.

### Phase 3: Built-in skill and scoped read behavior

- **Status:** complete
- Started after Phase 2 commit `914b6c0`.
- Embedded the source-controlled default `skill-installer/SKILL.md` as a read-only built-in and merged it into workspace discovery.
- Reserved the built-in ID, made built-in precedence deterministic, and ignored `.archive`/dot-directories during workspace scans.
- Added default activation reconciliation plus `disabledDefaults` tombstones while preserving legacy `active-skills.json` files.
- Extended `skills.read` with safe package-relative resources, UTF-8/base64 encoding, SHA-256/size metadata, 1 MiB ceiling, directory/path-escape/symlink rejection, and legacy-shape compatibility.
- Added built-in collision, list/read/search/activation, UTF-8/base64, and traversal/symlink regression tests.
- Verification passed: `cargo fmt --all` and `cargo test -p agentic-gpt` (70 tests).
- Phase 3 is ready to commit before beginning Phase 4.

### Phase 4: Persistent Room Agent installation job engine

- **Status:** complete
- Started after Phase 3 commit `fec2c7e`.
- Added Room-owned `InstallManager` with persisted versioned job records, atomic record writes, bounded status waits, revision/progress/timing fields, terminal retention (7 days/100 records), and restart recovery.
- Added worker semaphore, per-target lock, cooperative cancellation outcomes, explicit `waiting_for_target`/`committing` boundary, staging validation, archive-and-replace rollback, activation handling, package digest result, and durable commit journal.
- Wired `skills.install`, `skills.install.get`, and `skills.install.cancel` through the local Room command handler. Inline `files` sources execute through the shared staging/commit pipeline; network/GitHub resolution remains the next Phase 5 source adapter.
- Added regression tests for atomic inline installation, persistence, idempotency, and cancellation replay.
- Verification passed: `cargo fmt --all`, `cargo test -p agentic-gpt skill_installs::tests` (3 tests), and `cargo check --workspace`.
- Phase 4 is ready to commit before beginning Phase 5.

### Phase 5: Source resolution and secure download pipeline

- **Status:** complete
- Started after Phase 4 commit `b76dd30`.
- Added backward-compatible `room.skills` limits and network policy, including worker concurrency, public-host allowlisting, bounded redirects, retries, and total deadline.
- Added structured and convenience GitHub resolution with public-host validation, default-branch lookup, commit pinning, recursive tree selection, raw blob downloads, redacted source provenance, and special-mode/symlink rejection.
- Unified inline UTF-8/base64 and URL-backed files with GitHub blobs through one staging, digest, package-limit, path, permission, and atomic commit pipeline.
- Added SSRF protections for arbitrary HTTPS files: credentials/private/reserved targets are blocked and every redirect is revalidated through DNS. Added case-folded duplicate/prefix collision checks, required `SKILL.md` validation, directory/file mode normalization, and transient-failure retry/backoff.
- Added regression coverage for GitHub URL parsing, path collisions, public-IP policy, retry classification, and the existing atomic install flow.
- Verification passed: `cargo fmt --all`, `cargo test --workspace` (77 local, 49 Hub, 5 protocol tests), and `cargo check --workspace`.
- Phase 5 is ready to commit before beginning Phase 6.

### Phase 6: Hub, MCP, Actions, and Apps workflow integration

- **Status:** complete
- Started after Phase 5 commit `7bda5cf`.
- Routed install start/status/cancel and skill execution through the active Room Agent, added semantic Actions 400/404/409 error responses, and preserved `hub.run.get` as transport-only recovery.
- Exposed all four new MCP tools with Room-scoped schemas/annotations and added matching POST Actions/OpenAPI contracts. Updated MCP instructions and interface/README usage guidance for asynchronous Apps polling and hybrid session follow-up.
- Added active-workspace `scripts/` executable resolution, existing policy/cwd validation, asynchronous managed sessions with `starting`/`waiting_confirmation`, cancellable pending confirmation, bounded hybrid waits, terminal pruning, disconnect cache clearing, provenance audit fields, and writer-fair per-skill shared/exclusive leases.
- Added regression tests for script preflight, async sessions, lease fairness, skill audit provenance, MCP annotations, command routing, OpenAPI paths/schemas, and the full workspace baseline.
- Verification passed: `cargo fmt --all`, `cargo check --workspace`, and `cargo test --workspace` (79 local, 49 Hub, 5 protocol tests).
- Phase 6 is ready to commit before beginning Phase 7.

### Phase 0: Repository and architecture discovery

- **Status:** complete
- Actions taken:
  - Traced the current skills implementation across protocol, Room routing, local-agent storage, MCP tools, Actions/OpenAPI, tests, and documentation.
  - Confirmed that current skills management already contains six MCP tools and is centralized in the active Room Agent.
  - Verified the existing skills test baseline and Hub contract tests.
  - Compared package-read extension options and retained `skills.read` as the skill-scoped read surface.
  - Explored GitHub, URL-file, and inline-content installation source models.
  - Inspected Hub reliable `agent_runs` and separated transport state from install business state.
  - Checked current official ChatGPT Apps tool-design and MCP guidance.
- Files created/modified:
  - `task_plan.md` (created)
  - `findings.md` (created)
  - `progress.md` (created)

### Phase 1: Contract and acceptance criteria

- **Status:** complete
- Started: 2026-07-18
- Actions taken:
  - Created the first persistent implementation plan.
  - Recorded confirmed decisions, acceptance criteria, unresolved product choices, security boundaries, and verification requirements.
  - Confirmed archive-and-replace semantics for existing workspace skills.
  - Confirmed public-only GitHub support for V1.
  - Confirmed simultaneous MCP and GPT Actions contract updates.
  - Reduced the remaining product questions to default activation, URL-host policy, job retention, and bounded status waiting.
  - Explained the three remaining policy terms and recorded recommended V1 defaults without treating them as user decisions.
  - Confirmed public-HTTPS URL policy with SSRF revalidation and an optional narrowing host allowlist.
  - Confirmed terminal job retention of seven days and at most 100 records, separate from `.archive` retention.
  - Confirmed bounded status waiting with a 5-second default and caller-specified seconds; recorded `0` as immediate and an initial 30-second maximum.
  - Confirmed `skill-installer` is active by default and designed a compatible disabled-default tombstone so explicit deactivation remains durable.
  - Confirmed the V1 `skills.install.cancel` interface and recorded idempotent cooperative cancellation with a non-cancellable atomic-commit boundary.
  - Closed the remaining product-policy questions; Phase 1 now only needs exact schema fields and resource limits to be frozen.
  - Re-read the `planning-with-files` requirements and reconciled the plan error table with the existing progress error log.
  - Migrated the current plan from repository-root files into `.planning/skill-installer/` and selected it through `.planning/.active_plan` so future requirements can use independent scoped plans.
  - Began the implementation-handoff readiness pass and recorded the remaining contract gaps that could otherwise lead a downstream coding agent to make divergent product decisions.
  - Confirmed new GPT Actions endpoints should normally follow the existing `POST /v1/room/skills/<operation>` convention.
  - Confirmed all public fields must be synchronized across camelCase protocol structs, strict OpenAPI schemas, MCP argument schemas, routing, and regression tests.
  - Confirmed `skills.read` can remain backward compatible by adding optional `path` defaulting to `SKILL.md`.
  - Confirmed the Room Agent already has a streaming rustls HTTP client dependency but lacks skills-specific limits/configuration and archive extraction support.
  - Confirmed the 30-second status wait cap fits under the Hub's 35-second Room request timeout with a five-second margin.
  - Confirmed transport replay protection does not deduplicate fresh external retries, motivating a public install idempotency key.
  - Confirmed a defaulted `room.skills` configuration block is the natural owner for Room-side download and resource policies.
  - Confirmed first handoff batch: one skill per job, required explicit ID, explicit replacement flag, new installs active by default with replacement-state preservation, optional idempotency key, and structured-plus-URL GitHub inputs pinned to a resolved commit.
  - Confirmed second handoff batch: additive skill resource reads, UTF-8/base64 auto-encoding, normalized executable permissions, configurable resource/network limits, redacted provenance, bounded retries, and restart journal recovery.
  - Resolved retry wording: three total attempts imply default waits of one and two seconds; a four-second backoff occurs only when an operator raises the attempt count.
  - Confirmed MCP and Actions already share a top-level structured error body convention, but semantic Actions 4xx mapping for Room business/preflight errors must be implemented explicitly.
  - Confirmed Hub `AgentRun` remains transport-only and is unsuitable as the public installation job schema.
  - Confirmed third handoff items 14A–17A: synchronous preflight without job creation, stable start/status/error/cancel schemas, idempotent cancellation outcomes, and activation rollback as part of the install transaction.
  - Left question 13 partially open because “first form” may refer only to canonical structured GitHub input or may remove the previously required files source; question 18 remains unanswered.
  - Clarified 13A-A: both GitHub and files sources remain; structured GitHub is canonical and URL parsing remains a convenience.
  - Confirmed 18A: three operation-style POST Actions paths with install/cancel consequential-destructive and get/read read-only annotations.
  - Completed Phase 1 with no unresolved external contract or product-policy questions; the plan is ready for a downstream implementation agent to begin Phase 2.
  - Ran the final handoff consistency check: Markdown diff checks pass, scoped-plan paths resolve, Phase 2 is the explicit entry point, and the worktree contains only the expected `.planning/` additions.
  - Reopened a focused Phase 1 addendum after the user raised installed skill script execution.
  - Confirmed public `process.exec` has a reusable policy/confirmation/sandbox/timeout/output/audit engine but requires `agentId` and does not contain the executable itself to a skill package.
  - Recommended a thin Room-scoped `skills.run` wrapper over the existing execution engine rather than direct public API delegation or a second subprocess implementation.
  - Corrected the async recommendation after confirming the existing `session.start`/`inspect`/`wait`/`kill` lifecycle: `skills.run` should only be a restricted Room-scoped session start wrapper and should return standard session identifiers for existing follow-up tools.
  - Confirmed remaining session gaps relevant to downloaded scripts: in-memory-only lifecycle, synchronous confirmation before start returns, no defined terminal pruning, generic sandbox disabled by default, inherited environment, and no package-specific read-only mount.
  - Adopted the user's hybrid execution proposal: `skills.run` always starts a managed session, returns the terminal result inline when it finishes within a bounded wait, and otherwise returns the same session identifiers for existing inspect/wait/kill tools.
  - Defined the recommended skill sandbox as a fail-closed bubblewrap execution profile with a read-only package, path-policy-controlled mounts, private temporary/work storage, network disabled by default, sanitized allowlisted environment, direct argv, and resource/time/output bounds.
  - Revised the sandbox recommendation after the user clarified the Room Agent threat model: because generic execution is intentionally available and relatively open, a stricter skill-only sandbox would be bypassable. `skills.run` now inherits existing session policy/sandbox behavior and is treated as an ergonomic, containment-of-identity, and audit surface rather than a new privilege boundary.
  - Identified concurrent replacement of a package used by a running session as the more relevant new invariant; proposed rejecting/defering replacement in V1 or using immutable execution snapshots later.
  - User confirmed the V1 execution-lease approach: running sessions hold a shared per-skill lease, replacement commit waits for the exclusive lease, new runs cannot starve a pending commit, and deadline expiry is a retryable no-mutation `target_busy` failure.
  - User selected asynchronous/cancellable confirmation: `skills.run` registers a managed session in `waiting_confirmation` before prompting, so bounded waiting always has a session ID and `session.kill` can cancel the pending phase.
  - Froze `skills.run` as a managed-session-backed hybrid call: fast terminal results return inline, while non-terminal states return the same identifiers for existing inspect/wait/kill lifecycle tools.
  - Verified workspace skills live under `workspace_root/skills`, while generic sessions default cwd to `workspace_root`; recommended preserving that default and optionally accepting the existing path-policy-validated `workingDirectory` rather than fixing cwd to the installed package.
  - User confirmed all final script-execution defaults: in-memory 24-hour/100-terminal session retention, optional existing-policy-validated cwd, active workspace executable scripts only, stable wrapped response, and simultaneous MCP/Actions exposure.
  - Closed the Phase 1 addendum and expanded Phases 2/3/4/6/7 with protocol, built-in limitation, writer-fair lease, managed-session refactor, MCP/Actions, audit, and regression-test work for `skills.run`.
  - Set the implementation handoff entry point to Phase 2 with no remaining product-level questions.
- Files created/modified:
  - `.planning/.active_plan`
  - `.planning/skill-installer/task_plan.md`
  - `.planning/skill-installer/findings.md`
  - `.planning/skill-installer/progress.md`

## Test results

| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Existing skills unit tests | `cargo test -p agentic-gpt skills` | Current behavior remains green | 6 passed | pass |
| MCP tool annotation regression | targeted Hub test | Current annotations remain green | 1 passed | pass |
| Room skills OpenAPI agentId regression | targeted Hub test | Skills schemas omit `agentId` | 1 passed | pass |
| Protocol serde regression | `cargo test -p agentic-gpt-protocol` | New source, read, install, cancel, and run shapes remain stable | 5 passed | pass |
| Workspace compile check | `cargo check --workspace` | All command matches and protocol consumers compile | pass | pass |
| Built-in/scoped-read regression | `cargo test -p agentic-gpt` | Built-in activation, collision, resource encoding, and path safety remain green | 70 passed | pass |
| Install manager regression | `cargo test -p agentic-gpt skill_installs::tests` | Atomic inline install, persisted status, idempotency, and cancel replay | 3 passed | pass |
| Phase 5 source/security regression | `cargo test --workspace` | GitHub parsing, SSRF policy, collision checks, retries, atomic install, Hub/protocol contracts | 77 local, 49 Hub, 5 protocol passed | pass |
| Phase 5 compile check | `cargo check --workspace` | Configured remote-source pipeline compiles without warnings | pass | pass |

## Error log

| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-07-18 | None during planning initialization | 1 | No action required. |
| 2026-07-18 | Planning-note patch matched an outdated section heading | 1 | Read the current file tail and reapplied against the actual `Open decisions` section. |
| 2026-07-18 | Tried to inspect non-existent `crates/agentic-gpt-hub/src/lib.rs` | 1 | Confirmed the Hub is binary-only and used `src/main.rs` and module files instead. |
| 2026-07-18 | Decision-table row missed its closing Markdown delimiter | 1 | Found during immediate plan verification and corrected before handoff. |
| 2026-07-18 | Phase 1 was reopened for the script-execution addendum but its progress status still said `complete` | 1 | Caught during plan verification and synchronized the status to `in_progress`. |
| 2026-07-18 | The first status-fix patch used stale task-plan error wording | 1 | No partial change was applied; inspected the exact table and reapplied with current context. |
| 2026-07-18 | The first script-execution findings patch used stale Phase 1 checklist wording | 1 | No partial change was applied; inspected the exact sections and revised the recommendation after confirming the session APIs. |
| 2026-07-18 | The first execution-lease decision patch used a stale acceptance-criteria anchor | 1 | No partial change was applied; inspected the exact sections and reapplied against current text. |
| 2026-07-18 | The first all-A finalization patch used a truncated Phase 4 context | 1 | No partial change was applied; split the update into exact section-level patches and revalidated the plan. |
| 2026-07-18 | Initial Phase 1 commit attempt could not create `.git/index.lock` in the sandbox | 1 | Retried the same scoped add/commit with approved Git escalation; commit `68e4908` succeeded. |
| 2026-07-18 | Phase 3 first compile caught moved package path/bytes and borrowed error text | 1 | Cloned the package path, borrowed bytes for base64 encoding, and formatted the dynamic error message. |
| 2026-07-18 | `cargo test -p agentic-gpt --lib` was used for a binary-only crate | 1 | Re-ran the package test target with `cargo test -p agentic-gpt`. |
| 2026-07-18 | Initial built-in activation test expected no active skills after deactivating a workspace skill | 1 | Updated the assertion to account for the default-active built-in installer. |
| 2026-07-18 | Phase 4 package digest initially used nested paths relative to the wrong directory | 1 | Preserved the package root while recursively collecting relative file names. |
| 2026-07-18 | Phase 4 first commit-journal write could leave a moved candidate on disk if journaling failed | 1 | Added rollback and journal cleanup around both rename and journal-update failures. |

## 5-question reboot check

| Question | Answer |
|----------|--------|
| Where am I? | Phase 7: verification, documentation, and release readiness. |
| Where am I going? | Add final redaction/replay/security regression coverage, review the complete diff, and commit the release-ready result. |
| What's the goal? | Deliver Room-scoped asynchronous/network-capable skill installation, a built-in installer guide, safe package reads, and session-backed skill script execution. |
| What have I learned? | See `findings.md`. |
| What have I done? | Completed and verified Phases 1–6: contracts, protocol/package model, built-in/scoped reads, persistent jobs, secure source materialization, Room routing, MCP/Actions, managed sessions, leases, and audit. |
