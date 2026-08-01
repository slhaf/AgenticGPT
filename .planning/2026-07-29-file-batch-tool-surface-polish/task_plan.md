# Task Plan: File Batch and Tool Surface Polish

## Goal
Repair the real-use `file.search` / `file.batch` brittleness recorded on 2026-07-29, then audit the public Agentic tool contracts so models can select tools, build valid arguments, and interpret partial success without relying on hidden implementation knowledge. This design pass may edit planning files only.

## Workflow State
- **Stage:** implementation_complete
- **Current role:** implementer
- **Implementation authorized:** yes
- **Active plan:** `2026-07-29-file-batch-tool-surface-polish`
- **Baseline:** clean `main`, aligned with `origin/main`, Agentic v0.9.0
- **Current phase:** Phase F — complete
- **Entry phase:** Phase A
- **Open blocking decisions:** none
- **Design checkpoint:** user-accepted contract at 2026-07-30T11:10+08:00
- **Next action:** none; release actions remain separately authorized and were intentionally not performed.

## Scope and Constraints
1. Make `file.search.contextLines` resilient to requests above the configured limit while preserving typed rejection for structurally invalid values.
2. Allow multiple `file.batch` edits to one normalized file, applied in input order to one in-memory candidate and committed with one physical write.
3. Make one file group the only mutation transaction boundary; remove the legacy cross-file rollback path rather than exposing a misleading pseudo-transaction mode.
4. Keep guarded create/overwrite behavior and prevent `replace` / `patch` from silently creating missing files.
5. Preserve path policy, normalized target locking, optimistic concurrency, confirmation, bounded output, audit redaction, and crash-safety claims.
6. Audit every public tool description/schema, prioritizing `file.*`, `job.*`, and `mcp.batch`, without expanding the tool count.
7. Add deterministic contract fixtures plus an optional model-side evaluation runner based on real failure cases.

## Non-goals
- No automatic parent-directory creation.
- No unguarded overwrite or generic upsert mode.
- No shell/external-search fallback for `file.search`.
- No cross-file transaction or rollback mode in `file.batch`; genuinely atomic multi-file/package updates require a domain-specific staged container or journal.
- No claim that `mcp.batch` can roll back downstream/external side effects.
- No product source, tests, configuration, generated artifacts, commit, push, deployment, tag, or release during this design pass.

## Key Questions
| ID | Question | Blocking | Status | Resolution |
|---|---|---:|---|---|
| Q-01 | Should the legacy cross-file rollback mode be removed entirely? | yes | resolved | Yes. Keep `file.batch` as a round-trip aggregator with per-file transactions only; remove the public atomicity selector and the best-effort cross-file rollback path. |
| Q-02 | How are optimistic guards interpreted when several edits target one file? | yes | resolved | Merge guards at file-group level: all supplied base revisions must match; repeated identical guards are accepted; later operations run against the staged candidate rather than intermediate caller-supplied revisions. |
| Q-03 | What configurable context limit should be public? | yes | resolved | Add live `limits.maxFileSearchContextLines`, default 5, valid 0–100; requests above it are clipped and evidenced, not rejected. |

## Decisions Made
| ID | Area | Status | Outcome | Concise rationale | Evidence |
|---|---|---|---|---|---|
| D-01 | Search overflow | confirmed | Non-negative integer `contextLines` requests above the live maximum are clamped; negative/non-integer values remain invalid. | A harmless overshoot must not discard the whole search/batch. | Room notebook and live `contextLines=8` reproduction. |
| D-02 | Search evidence | confirmed | Return `requestedContextLines`, `effectiveContextLines`, `contextLinesClipped`, and a bounded warning when clipping occurs. | The caller can distinguish accepted degradation from exact execution. | User accepted Q-03 recommendation; current response has no limit evidence. |
| D-03 | Mutation boundary | confirmed | Same-file edits form one transaction; different files succeed or fail independently. | Batch is primarily a round-trip reducer and failure isolator, not a filesystem transaction. | Room notebook, live unrelated-edit rejection, and usage review. |
| D-04 | Same-file execution | confirmed | Normalize aliases, group by resolved target, apply operations in original order to one evolving UTF-8 candidate, then stage/commit once. | Removes the duplicate-target failure without weakening path/lock safety. | Current duplicate-target guard and one-candidate-per-file implementation. |
| D-05 | Guard merge | confirmed | Existing-file groups require at least one `expectedRevision`; all supplied revisions must be identical and match the disk base. Absent-file groups require `expectedAbsent:true`, no revision, and a first effective `write`. | Repeated model-generated guards remain valid while conflicting intent is rejected. | User accepted Q-02; callers cannot know intermediate revisions before execution. |
| D-06 | Missing targets | confirmed | `write + expectedAbsent:true` creates only when the parent exists; `write + expectedRevision` overwrites; `replace` and `patch` never create. | Existing guarded semantics are sufficient when documented clearly. | Current `edit_inner` / `batch_candidate` behavior. |
| D-07 | Read/search ordering | confirmed | Reads/searches keep the v0.9 pre-edit disk snapshot behavior, and their failures do not block independent file groups. | True interleaved read-after-staged-write semantics would greatly expand scope and ambiguity. | Current code deliberately runs reads/searches before writes. |
| D-08 | Confirmation | confirmed | Perform one aggregate confirmation for all valid effective file groups. Invalid groups are excluded; denial writes nothing. | Retains one user decision without coupling validation failures. | Existing single batch confirmation boundary. |
| D-09 | Result states | confirmed | Use `completed`, `completed_with_errors`, `rejected`, and `dry-run` with per-operation and per-file-group evidence; remove legacy cross-file rollback states from new responses. | Makes partial success explicit without preserving states for a deleted pseudo-transaction path. | Current output has ordered envelopes but no group layer. |
| D-10 | Audit | confirmed | Retain one redacted logical audit per edit plus one aggregate batch audit; add group correlation, group counts, committed state, and failure counts. | Sequential logical edits and physical commits must both remain reconstructable. | Current per-edit + aggregate audit lacks grouping/partial-success fields. |
| D-11 | Tool contracts | confirmed | Descriptions state when to use/not use, conditional fields, defaults, failure/partial-success meaning, and non-transactional boundaries; detailed examples live in docs/fixtures to protect schema budgets. | Current labels are structurally valid but under-instructive. | `tool_description`, schema budget tests, and notebook audit request. |
| D-12 | Contract verification | confirmed | Keep deterministic schema/serde/dispatch fixtures in CI and add an optional provider-neutral model evaluation runner outside required CI. | Deterministic correctness and probabilistic model usability need separate gates. | Existing tests cover parity, not model tool choice. |
| D-13 | Cross-file rollback | confirmed | Remove the public atomicity selector and delete the legacy best-effort cross-file rollback path; `file.batch` always uses independent per-file transactions. | The mode is not truly atomic, has no observed rollback use, adds substantial orchestration/testing complexity, and serious consistency cases need a domain-specific transaction. | 410 visible batch audits: 108 mutating, 97 multi-path, zero rollback-like outcomes; v0.9 contract excludes crash atomicity and external snapshot isolation; user accepted removal on 2026-07-30. |

## Public Contract Draft

### `file.search`
- `contextLines` remains a non-negative integer with default 0.
- The descriptor does not advertise a static maximum because the effective limit is live-configured; it states that oversize values are clipped.
- `limits.maxFileSearchContextLines` defaults to 5, is hot-reloadable with `limits`, and is bounded to 0–100.
- `agent.info` exposes the effective file-search context limit without requiring config-file access.
- Single and batch search responses expose identical requested/effective/clipped evidence.

### `file.batch`
- Keep one flat ordered `operations` array and no atomicity selector.
- Reads/searches execute against the pre-edit disk snapshot and fail independently.
- Edits are grouped by normalized resolved path; each group has one base guard, one candidate chain, one staged file, and one atomic per-file commit.
- One failed operation rejects/skips the rest of that file group only; unrelated valid groups continue.
- No cross-file rollback is attempted or implied. `dryRun:true` is the supported whole-request validation/preview mechanism, while revision/absence guards remain authoritative on the later real call.
- Repeated identical `expectedRevision` or `expectedAbsent:true` fields within a group are accepted as the same base assertion; conflicting guards reject that group with a typed error.
- Each edit operation reports candidate-relative before/after revision and diff evidence. A compact file-group summary reports target, operation indexes/ids, base/final revision, final status, and committed state.
- Response truncation must preserve operation id/index/status/error and compact group status even after content/matches/diffs are removed.

### Tool-description audit
- Audit the exact 24 Normal / 36 Room standalone tools and all Hub profile tools without adding aliases.
- First priority: `file.read`, `file.search`, `file.edit`, `file.batch`, `job.get`, `job.list`, `job.cancel`, `mcp.callTool`, and `mcp.batch`.
- Explicitly state that `mcp.batch` provides atomic admission/confirmation only, not transactional rollback of downstream effects.
- Keep standalone and Hub wording aligned where behavior is shared, while retaining `agentId`/cache distinctions on Hub.

## Implementation Phases

### Phase A — Search context resilience and operations visibility
**Status:** complete.

**Prerequisite:** frozen D-01/D-02 and implementation-ready handoff.
**Objective:** Accept bounded overshoot without losing results.

**Primary areas:** `config.rs`, `file_ops.rs`, `stdio_server.rs`, `main.rs`, `agent_info.rs`, `config.example.json`, configuration/runtime docs.

**Work:**
1. Add the defaulted, validated, hot-reloadable maximum.
2. Pass requested and effective context through the shared search path.
3. Add clipping evidence/warning to single and batch results.
4. Add schema `minimum:0`, dynamic-limit wording, `agent.info` diagnostics, and config migration/docs.
5. Add unit, serde, live-reload, descriptor, single-search, and batch-search cases including 0, 5, 8, configured 20, configured 0, negative, and non-integer inputs.

**Completion boundary:** No batch mutation semantics change in this phase.

**Completed 2026-08-01:** Added the live 0–100 config limit with default 5, clamped/evidenced single and batch searches, agent.info diagnostics, schema minimum/wording, config/runtime docs, and focused regression coverage. No Phase B/C mutation behavior was changed.

### Phase B — File-group planner and sequential candidate engine
**Status:** complete.

**Prerequisite:** Phase A complete; frozen D-03 through D-06.
**Objective:** Make same-file edits composable before changing cross-file commit behavior.

**Primary areas:** `file_ops.rs`, batch DTO/schema tests in `stdio_server.rs`.

**Work:**
1. Replace duplicate-target rejection with normalized file-group construction.
2. Resolve and merge base guards; reject conflicting groups with typed evidence.
3. Extract candidate application so each operation consumes the previous candidate without rereading disk.
4. Preserve operation order, candidate-relative revisions/diffs, no-op handling, byte bounds, path policy, and one lock per group.
5. Implement dry-run/group-failure evidence before physical commits.
6. Cover aliases, repeated guards, conflicting guards, create-then-replace/patch, later locator failure, no-op chains, UTF-8/size bounds, and concurrent external changes.

**Completion boundary:** The grouped candidate engine is complete, but legacy cross-file rollback removal waits for Phase C.

**Completed 2026-08-01:** Replaced duplicate normalized-target rejection with one group/candidate chain per resolved file, merged repeated/conflicting guards, preserved guarded create/replace/patch and one-lock/one-stage behavior, added candidate-relative no-op/UTF-8/size/race coverage, and passed all Phase B focused and static gates. Legacy cross-file preflight/rollback behavior remains intentionally unchanged for Phase C.

### Phase C — Per-file commit orchestration, confirmation, audit, and legacy cleanup
**Status:** complete.

**Prerequisite:** Phase B complete; frozen D-07 through D-10 and D-13.
**Objective:** Make file-local partial success the only mutation behavior and remove misleading cross-file rollback machinery.

**Primary areas:** `file_ops.rs`, `audit.rs`, `stdio_server.rs`, standalone docs and migration/release notes.

**Work:**
1. Isolate preflight/staging/commit failures per file group and produce `completed_with_errors` when some groups fail.
2. Keep one aggregate confirmation over valid effective groups; denial commits none.
3. Commit valid groups independently with per-file revalidation and atomic replacement/no-replace creation; one commit failure does not roll back or block unrelated groups.
4. Remove cross-file original-byte retention, rollback execution, rollback-only response states/errors, and associated tests/documentation.
5. Extend ordered results, compact group summaries, truncation preservation, per-edit audit correlation, and aggregate audit counts.
6. Add deterministic failure injection for staging, commit conflict, audit failure, missing parent, invalid read/search, and mixed valid/invalid groups.

**Completion boundary:** Observable behavior matches frozen D-03 through D-10 and D-13.

**Completed 2026-08-01:** Isolated read, planning, staging, confirmation, and commit failures by normalized file group; removed cross-file rollback retention, execution, response states, and errors; added bounded group/commit/failure evidence, audit correlation/counts, migration docs, and deterministic staging/conflict/isolation coverage. Focused `file_batch` tests (14/14), workspace check, strict Agent clippy, formatting, and diff checks passed. The full Agent package ran 240 tests with 236 passing; the four unrelated local-control/supervisor/tunnel tests remain blocked by sandbox `Operation not permitted` permissions.

### Phase D — Public tool contract audit and wording repair
**Status:** complete.

**Prerequisite:** Phases A–C complete so descriptions target final runtime semantics; frozen D-11.
**Objective:** Make the tool surface self-explanatory without schema bloat.

**Primary areas:** standalone `INSTRUCTIONS`, `tool_description`, property descriptions/required fields/annotations, Hub `#[tool]` descriptions, interface/runtime/config docs.

**Work:**
1. Build a checked-in contract matrix for every public tool: use case, non-use case, required/conditional fields, defaults, failure/partial-success states, persistence/cancellation/transaction boundaries, and surface parity.
2. Repair priority tools first, then process/tmux/skills/bootstrap/notebook/diary/notification/Hub-native groups.
3. Remove misleading internal jargon and clarify annotation semantics.
4. Keep detailed positive/negative examples in docs/fixtures; enforce finite descriptor/schema budgets and surface revision updates.

**Completion boundary:** Tool count remains 24/36 and no compatibility alias is introduced.

**Started 2026-08-01:** Phase C commit `2303762` is complete. Phase D began with a checked-in contract matrix and a descriptor/schema inventory; runtime behavior and tool counts remained frozen while wording and annotations were audited.

**Completed 2026-08-01:** Repaired priority standalone and Hub descriptions/property annotations for file, Job, process, MCP, tmux, skills, bootstrap, diary, and notebook boundaries; added `docs/tool-contract-matrix.md` for every Normal/Room/Hub profile tool; linked the matrix from interface/runtime/README docs; added descriptor phrase/annotation/parity tests; and verified Normal 24 / Room 36 counts. Formatting, workspace check, workspace clippy, full Agent tests (241/241 with authorized escalation), and full Hub tests (59/59) pass.

### Phase E — Deterministic contract corpus and optional model evaluation
**Status:** complete.

**Prerequisite:** Phase D contract matrix complete; frozen D-12.
**Objective:** Turn today’s failures into repeatable regressions.

**Primary areas:** connector/unit tests, `tests/tool-contract-cases`, optional `scripts` evaluator, CI/docs.

**Work:**
1. Encode real tasks and expected tool/argument/outcome contracts: context overshoot, same-file edits, unrelated missing target, missing revision, create/overwrite, job discovery/wait/cancel, and MCP atomic-admission wording.
2. Run positive fixtures through actual descriptors, serde, and dispatch/dry-run; run negative fixtures against typed errors.
3. Add a provider-neutral optional model runner that scores tool selection and argument validity but is not a required network CI gate and stores no credentials.
4. Document how to add a regression from future model misuse.

**Started 2026-08-01:** Phase D commit `1251285` is complete. Phase E begins from the recorded real-use failures and keeps the fixture corpus provider-neutral; required tests never need network access or credentials.

**Completed 2026-08-01:** Added nine checked-in cases and contributor guidance; the Agent test loads them through the live descriptors, argument validation, serde, dispatch, and dry-run paths; typed negative cases cover missing revision, invalid negative context, and missing MCP server; and the optional standard-library evaluator scores provider-supplied predictions without network or credentials. Final Phase E gates pass: corpus (9/9), file.batch (14/14), full Agent (242/242 with authorized escalation), formatting/diff, workspace check, and workspace clippy.

### Phase F — Full acceptance and release boundary
**Status:** complete.

**Prerequisite:** Phases A–E complete.
**Objective:** Verify all surfaces and package the behavioral migration.

**Verification:** focused tests per phase; full Agent, Hub, and Protocol suites; Local Unix connector E2E; standalone supervisor E2E; schema/reference/document-link checks; `cargo fmt --all --check`; `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `git diff --check`.

**Release boundary:** use v0.9.1 for this release despite the `file.batch` failure-semantics change and removal of legacy rollback-only states/guarantees; retain explicit migration and release notes. No tag/push/deploy without separate authorization.

**Started 2026-08-01:** Phase E commit `90294c3` is complete. Phase F is documentation and verification only: it does not introduce a compatibility alias, release artifact, tag, push, deployment, or publication.

**Completed 2026-08-01:** Added English/Chinese v0.9.1 release notes and README links. `cargo test --workspace` passed Agent 242/242, local-control 1/1, standalone supervisor 6/6, Hub 59/59, Protocol 12/12, and doc-tests 0; final formatting, workspace check, workspace clippy, diff check, and 40 local Markdown links with 0 missing targets passed. The release boundary is documented but not tagged, pushed, deployed, or published.

## Acceptance Criteria
- `contextLines:8` succeeds under the default max 5 and reports requested 8/effective 5/clipped=true.
- Invalid negative/non-integer context inputs fail before scanning.
- Two or more normalized-alias edits to one file apply sequentially and cause at most one physical target replacement.
- A later failed edit leaves that file unchanged and does not prevent an independent valid file group from committing in default mode.
- No `atomicity` field is advertised or accepted; no cross-file rollback state/error remains in the new contract.
- Missing parents and revision conflicts are local file-group errors.
- Create/overwrite/replace/patch behavior remains guarded and unambiguous.
- Confirmation denial writes no valid group.
- Ordered operation evidence, compact group summaries, truncation, and redacted audit remain bounded.
- Priority descriptions prevent the recorded invalid calls, and all public tools have a checked contract-matrix entry.
- Deterministic connector contract fixtures pass; optional model evaluation can run without changing runtime behavior or storing secrets.
- Exact standalone tool counts remain Normal 24 / Room 36.

## Implementation Discretion
- Private Rust type decomposition, map/index containers, and helper names.
- Exact compact file-group identifier format, provided it is stable within one response/audit and leaks no additional sensitive content.
- Failure-injection harness mechanics and optional model-provider adapter shape.
- Assertion wording and focused commit boundaries, provided each phase remains independently reviewable.

## Readiness Gate
- Goal, scope, non-goals, ownership, public inputs/outputs/defaults, failure isolation, compatibility, versioning, limits, observability, path security, and verification are frozen.
- Concurrency/idempotency: normalized-path locking, base revision/absence guards, and one physical commit per file group are frozen; cancellation and retries are N/A because `file.batch` is synchronous and does not create retained work.
- Persistence/migration/recovery: additive config loading is defaulted; no data migration or retention applies; cross-file recovery/rollback is intentionally removed, while per-file atomic replace/no-replace semantics remain.
- Network/auth/secrets are N/A to local `file.*`; existing standalone profile, path-policy, redaction, and trust boundaries remain unchanged.
- Every requirement maps to Phases A–F and an acceptance criterion; phase dependencies and implementation discretion are explicit.
- All Q-01 through Q-03 and D-01 through D-13 are resolved/confirmed; the user accepted the consolidated contract on 2026-07-30.
- Repository status contains planning changes only; no product source, tests, runtime configuration, dependencies, or generated artifacts changed during refinement.

## Implementation Handoff
- **Plan maturity:** implementation_complete
- **Design phase:** complete
- **Implementation authorized:** yes
- **Entry phase:** Phase A
- **Frozen decisions:** D-01 through D-13
- **Open blocking decisions:** none
- **Implementation discretion:** see `Implementation Discretion`
- **Verification convention:** focused verification per phase, then the full Phase F matrix
- **Commit convention:** one focused commit after each completed Phase A–E; Phase F acceptance/release documentation remains a separate final commit when authorized
- **Design checkpoint:** user-accepted contract at 2026-07-30T11:10+08:00; no checkpoint commit created
- **Next invocation:** none; resume only for separately authorized release or follow-up work.

## Errors Encountered
| Error | Attempt | Resolution |
|---|---:|---|
| Initial `file.batch` planning creation failed because the new scoped parent directory did not exist; the independent `.active_plan` update was skipped after batch preflight rejection. | 1 | Created the scoped directory explicitly, then retried the writes. |
| Second planning batch omitted the existing `.active_plan` revision, causing all otherwise valid new-file writes to be skipped. | 1 | Read the current revision and retried with optimistic concurrency evidence. |
| A planning-file unified patch used mismatched terminal context. | 1 | No write occurred; switched to exact revision-guarded replacement. |
| `skills.run` rejected `check-complete.sh` as non-executable; the same direct invocation was inadvertently repeated. | 8 | Stopped using `skills.run` for this script and invoked it explicitly through `sh`; readiness was then checked from the canonical plan state. |
| `cargo test -p agentic-gpt --lib` found no library target because `agentic-gpt` is a binary-only crate. | 1 | Switched to the package-level binary test command `cargo test -p agentic-gpt`. |
| Full `cargo test -p agentic-gpt` ran 232 tests; 228 passed and four unrelated socket/tunnel download tests failed with sandbox `Operation not permitted`/`local_mcp_bind_failed` permission errors. | 1 | Treat as environment-blocked unrelated failures; run Phase A focused tests separately and retain the full-suite result for handoff. |
| First Phase B compile check rejected `[u8]::len` in a JSON expression and reported an immutable/mutable borrow conflict while iterating group operation indexes. | 1 | Replace the function item with a closure and clone the group index list before mutating the group candidate. |
| Existing duplicate-target regression expected the removed `file_batch_duplicate_edit_target` rejection and failed after grouped planning was enabled. | 1 | Replace it with Phase B coverage for same-file sequential edits and guard conflict behavior. |
| Full `cargo test -p agentic-gpt` after Phase B ran 235 tests; 231 passed and the same four unrelated local-control/supervisor/tunnel tests failed with sandbox permission errors. | 1 | Retain as environment-limited evidence; all file.batch and Phase B tests pass. |
| Phase C full `cargo test -p agentic-gpt` ran 240 tests; 236 passed and the same four unrelated local-control/supervisor/tunnel tests failed with sandbox `Operation not permitted`/`local_mcp_bind_failed` errors. | 1 | Retain as environment-limited evidence; all Phase C file.batch tests and product checks pass, with no Phase C failure. |
| A text-search command included shell backtick syntax and unintentionally invoked a nested cargo test while constructing its pattern. | 1 | No repository files changed; use single-quoted search patterns and record the command mistake here for reproducibility. |
| The first Phase D Hub description test found the intended mcp attribute text had not landed at the exact generated attribute lines; a second assertion then required the explicit “bounded inline wait” phrase. | 2 | Patched the exact Hub `mcp.callTool`/`mcp.batch` attributes and reran the focused test successfully. |
| Phase D full Agent tests ran 241 tests; 237 passed and the same four local-control/supervisor/tunnel tests failed with sandbox permission errors. | 1 | Retain as environment-limited evidence; all descriptor, file.batch, Agent, and Hub contract tests pass. |
| The four sandbox-sensitive Agent tests initially failed only because the default sandbox denied local socket/HTTP bind and fake subprocess operations. | 1 | With the user's authorization, reran the complete Agent suite with escalation: 241/241 passed, including both local-control tests, fake tunnel, and bounded download. |
| The first deterministic corpus run expected `completed_with_errors` for a mixed dry-run, but the frozen contract correctly reports `dry-run` while retaining failed group/failure counts. | 1 | Updated the fixture expectation to assert `dry-run` plus `failedGroups`/`failureCount`; rerun passes. |
| The first MCP negative corpus shape attempted to assert duplicate-call-id before server resolution; the runtime correctly failed earlier with `mcp_server_not_found`. | 1 | Changed the fixture to the stable missing-server typed failure; duplicate-id behavior remains covered by the existing MCP unit test. |
| `uv run python scripts/evaluate_tool_contracts.py` could not acquire its read-only cache lock under the default sandbox, and a pipe masked the command's error status. | 1 | Reran the provider-neutral script directly with system `python3`; corpus listing succeeds and no network/credentials are required. |
| A combined cargo test filter using a pipe expression selected zero tests because Cargo treats the filter as a literal substring, not a regex. | 1 | Reran the deterministic corpus and `file_batch` filters separately; both pass. |
