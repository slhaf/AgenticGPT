# Task Plan: Standalone Info, File Tools, and Confirmation Naming

## Goal
Add a model-facing standalone `agent.info` tool, a bounded and concurrency-safe `file.*` tool family with batch support, and truthful `freedesktop`/`ntfy` confirmation naming without changing the working Hub-relayed ntfy callback architecture.

## Workflow State
- **Stage:** implementation_ready
- **Current role:** implementer
- **Implementation authorized:** yes
- **Active plan:** `2026-07-26-standalone-info-file-and-confirmation`
- **Current phase:** Implementation Phase A — Confirmation semantics
- **Entry phase:** Implementation Phase A — Confirmation semantics
- **Open blocking decisions:** none
- **Design checkpoint:** `3fb762b` (`docs(plan): checkpoint standalone info and file tools plan`)
- **Baseline:** `d04f8e0`; branch `main` is clean and 16 commits ahead of `origin/main`
- **Next action:** implement Phase C (`file.read` and shared file core) under `planning-with-files`

## Scope and Constraints

### In scope
- Add a no-argument local standalone tool named `agent.info` for the connected model.
- Return useful current Agent identity, capability, path-policy, capacity, confirmation, connection, configuration-application, and health information with bounded machine-readable results.
- Add `file.read`, `file.search`, `file.edit`, and `file.batch` to both Normal and Room standalone profiles.
- Support metadata-only reads without a separate public `file.stat` tool.
- Support precise text replacement, patch application, and complete text creation/overwrite through `file.edit`.
- Apply current `pathPolicy` semantics to every file read/search/write target, including symlink-safe normalization.
- Prevent silent concurrent overwrite with content revisions and expected-state preconditions.
- Use atomic per-file replacement for successful mutations and return bounded verification evidence.
- Support mixed read/search/edit batch requests with deterministic ordered results and one confirmation decision for a mutating batch.
- Present confirmation channels truthfully as `freedesktop` and `ntfy` while leaving ntfy publication, callback, pending state, and response relay in Hub.
- Preserve existing behavior for legacy `hub`, `freedesktop-then-hub`, `freedesktopThenHub`, and `default` configuration values.
- Update standalone instructions, schemas, tests, and user documentation.

### Out of scope
- Moving ntfy notification publication or callback ownership from Hub into Agent.
- Implementing KMP confirmation UI or a new KMP confirmation channel.
- Changing Hub execution routing or the Hub full/coordinator MCP tool surfaces.
- Adding file delete, move, rename, directory creation, permission, ownership, archive, or binary mutation APIs.
- Building a general AST-aware editor or language-server integration.
- Durable cross-file transactions, crash recovery journals, or rollback guarantees across multiple committed files.
- Raw log retrieval, durable session history, policy-language redesign, program alias matching, or full Tunnel RTT instrumentation.
- Push, deployment, tag, or release unless separately requested after acceptance.

## Candidate Public Surface

Both standalone profiles add exactly five tools:

```text
agent.info
file.read
file.search
file.edit
file.batch
```

Expected surface counts after this plan:
- Normal: 18 → 23
- Room: 30 → 35

`file.read`, `file.search`, and `file.edit` remain distinct because their schemas and risk boundaries differ. `file.batch` reuses the same operation/result contracts rather than inventing incompatible duplicate semantics.

## Phases

### Phase 1: Requirements and Repository Discovery
- [x] Capture the three user requirements in one scoped plan.
- [x] Read the `planning-with-files` skill, run session catch-up, and inspect its templates.
- [x] Identify the current standalone tool registry/dispatch/schema implementation.
- [x] Locate current confirmation-provider normalization and Hub-relayed ntfy flow.
- [x] Locate reusable path-policy, hashing, UTF-8 resource, and atomic-write code.
- [x] Inspect exact config serialization/migration behavior and safe-summary types.
- [x] Inspect runtime state needed for `agent.info`, including connection and reload status.
- [x] Inspect dependency options and existing conventions for bounded search/diff/patch behavior.
- **Status:** complete

### Phase 2: Freeze Confirmation and `agent.info` Contracts
- [x] Freeze canonical confirmation configuration shape and legacy migration behavior.
- [x] Freeze the distinction between configured channel, runtime availability, and observed readiness.
- [x] Freeze `agent.info` response schema, field provenance, freshness timestamps, bounds, and redaction rules.
- [x] Decide whether exact local paths and policy rules are returned directly or through bounded summaries.
- [x] Freeze stable error codes and compatibility expectations.
- **Status:** complete

### Phase 3: Freeze Single-File Contracts
- [x] Freeze `file.read` metadata/content/range semantics and revision format.
- [x] Freeze `file.search` literal/regex, path/glob, context, hidden/build-directory, symlink, and result-bound semantics.
- [x] Freeze `file.edit` replace/patch/write discriminated schema.
- [x] Freeze UTF-8/text-only policy, newline behavior, empty files, creation/overwrite preconditions, dry-run, and diff output.
- [x] Freeze path-policy decisions for existing, absent, symlinked, read-only, and denied targets.
- [x] Freeze confirmation and audit behavior for mutations.
- **Status:** complete

### Phase 4: Freeze Batch Contract and Implementation Plan
- [x] Freeze maximum operation count and total input/output bounds.
- [x] Freeze ordered mixed-operation response shape and per-operation error envelopes.
- [x] Freeze preflight-before-mutation behavior and duplicate mutation-target rules.
- [x] Decide whether reads in a mutating batch observe the initial snapshot or committed results.
- [x] Freeze one-confirmation preview and denial/timeout behavior.
- [x] Define per-file atomicity, partial commit reporting, and explicitly excluded cross-file crash atomicity.
- [x] Map implementation work into focused commits and tests.
- **Status:** complete

### Phase 5: Independent Plan Refinement
- [x] Run `refine-implementation-plan` after the first complete contract draft.
- [x] Resolve every blocker or record an explicit user decision.
- [x] Ensure no implementation authorization conflict remains.
- [x] Mark the plan `implementation_ready` and freeze the entry phase.
- **Status:** complete

### Phase 6: Implementation, Verification, and Handoff
- [ ] Implement in the frozen phase/commit sequence.
- [ ] Run focused tests after each phase.
- [ ] Verify exact 23/35 standalone surfaces and unchanged Hub surfaces.
- [ ] Run formatting, workspace checks/tests, diff checks, and real standalone probes.
- [ ] Independently review security, compatibility, output bounds, and race behavior.
- [ ] Leave the worktree clean and report the final commit without pushing/deploying/releasing.
- **Status:** in_progress

### Implementation Phase A — Confirmation semantics
- [x] Replace raw confirmation-provider execution with canonical ordered channels and shared legacy parsing.
- [x] Accept canonical `channels` and legacy `{provider}` config forms; serialize canonical `channels`.
- [x] Preserve scalar CLI/protocol aliases and Hub-relayed ntfy request/callback behavior.
- [x] Update `SafeConfigSummary` to emit the truthful canonical display label.
- **Status:** complete; focused Agent unit suite passed, with standalone supervisor failures isolated to missing runtime directory in the test environment.

### Implementation Phase B — `agent.info`
- [x] Add startup metadata and local info response module.
- [x] Register no-argument `agent.info` in both standalone profiles and include workflow guidance.
- [x] Derive bounded/redacted surface, policy, capacity, confirmation, connection, and config-health fields.
- [x] Complete focused Phase B tests and commit.
- **Status:** complete; focused info tests and full Agent binary unit suite passed (168/168).

### Implementation Phase C — File core and reads
- [x] Add shared bounded file path-policy/revision/lock/read core and file-shaped audit records.
- [x] Register `file.read` in both standalone profiles with metadata-only and ranged UTF-8 modes.
- [x] Verify deny/read-only precedence, canonical symlink containment, binary/large-file handling, exact newlines, and revision stability.
- **Status:** complete; focused file/read tests passed (7 total) and full Agent binary unit suite passed (175/175).

### Phase commit ledger
- Design checkpoint: `3fb762b`
- Phase A: `9c06c48` — `refactor(agent): clarify confirmation channel semantics`
- Phase B: `aa9ced6` — `feat(agent): expose standalone runtime info`




## Frozen `file.batch` Contract

`file.batch` is one synchronous local MCP call for efficient multi-file inspection and coordinated text edits. Its descriptor is conservatively annotated non-read-only and destructive because one valid request shape can mutate files.

### Input

```json
{
  "operations": [
    {
      "id": "read-config",
      "type": "read",
      "path": "src/config.rs",
      "includeContent": true,
      "startLine": 1,
      "endLine": 200
    },
    {
      "id": "find-callers",
      "type": "search",
      "path": "src",
      "query": "confirmation_provider",
      "mode": "literal"
    },
    {
      "id": "edit-config",
      "type": "edit",
      "mode": "replace",
      "path": "src/config.rs",
      "expectedRevision": "sha256:...",
      "oldText": "old",
      "newText": "new",
      "expectedMatches": 1
    }
  ],
  "dryRun": false,
  "needConfirm": false
}
```

Rules:
- `operations` is required and contains 1–32 entries.
- At most 16 entries may be `type: "edit"`.
- Optional operation `id` must be 1–64 characters, use `[A-Za-z0-9._-]`, and be unique within the batch.
- `type` is exactly `read`, `search`, or `edit`. Each entry otherwise uses the corresponding single-tool fields.
- Embedded edit entries do not accept their own `dryRun` or `needConfirm`; those controls exist only at batch level.
- Batch `dryRun` and `needConfirm` default to false.
- Aggregate request text across queries, glob patterns, replacement text, patches, and write content is capped at 16 MiB.
- Aggregate original bytes retained for rollback are capped at 32 MiB, and aggregate prepared candidate bytes are separately capped at 32 MiB.
- Aggregate search traversal is capped at 20,000 files and 128 MiB scanned bytes across all search operations.
- Aggregate structured response is capped at 1 MiB. Each operation also retains its single-tool payload bounds.

### Ordering and observation model
1. Decode and validate every operation and all aggregate bounds.
2. Execute every read/search operation in input order before any batch-owned mutation.
3. Acquire all normalized edit-target locks in sorted path order.
4. Re-read every edit target, validate expected state, and prepare every candidate/diff/temp file.
5. If required, request one confirmation for the complete effective mutation set.
6. Revalidate target revisions, absence preconditions, and symlink/path-policy state.
7. Commit staged files in sorted normalized-path order.
8. Return results in original input order.

Observation guarantees:
- Read/search entries observe the filesystem before any writes made by this batch.
- They are not advertised as one globally atomic filesystem snapshot; external processes may change files between individual reads.
- Edit preconditions are evaluated later under Agentic's sorted target locks and remain authoritative for commit.
- A read/search and one edit may address the same path; the read/search result is explicitly the pre-edit view.
- More than one edit resolving to the same normalized target is rejected as `file_batch_duplicate_edit_target`, even when the request spellings differ.

### Failure behavior

#### Pure read/search batch
- All operations run independently even if one fails.
- Successful and failed operation envelopes are both returned.
- Batch status is `completed` when all succeed, otherwise `completed_with_errors`.

#### Mutating batch preflight
- Every read/search finishes before edit preflight.
- Any hard read/search error, edit validation error, revision conflict, path-policy failure, temp-file preparation failure, or aggregate-bound failure prevents all edits from committing.
- Batch status is `rejected` and every edit that did not itself fail is marked `skipped` with `file_batch_rejected`.
- Search truncation or skipped binary/oversized files is metadata, not a hard error.
- An unchanged edit candidate is not considered an effective mutation and does not require confirmation or commit.

#### Confirmation
- A mutating batch requests at most one confirmation when top-level `needConfirm` is true and at least one effective mutation remains.
- The preview includes operation ids/indexes, normalized paths, modes, before/after sizes, revisions, and bounded change counts; it excludes file content, replacement text, patch text, and full diffs.
- `dryRun: true` never requests confirmation.
- Denial, timeout, or unavailable confirmation commits nothing; read/search results remain available and effective edits are marked rejected.

### Commit, rollback, and crash boundary
- Every edit is first staged as a fully written and synced unique temporary file in its target directory.
- Original bytes/absence and ordinary permissions are retained in separately bounded memory for guarded best-effort rollback as confirmed by D-14.
- Commits use per-file atomic rename and parent-directory sync where supported.
- If a normal commit error occurs after earlier targets committed, Agentic stops and restores an earlier target only when its current revision still equals the exact post-revision written by this batch; external changes are never overwritten during rollback. This is confirmed by D-14.
- Batch status is `rolled_back` when every committed target was safely restored successfully.
- Batch status is `partial_failed` when at least one committed target cannot be restored; each affected edit reports `committed`, `rolled_back`, `rollback_failed`, or `not_committed`.
- V1 does not claim cross-file atomicity under process crash, SIGKILL, kernel failure, power loss, or storage-device failure. It has no durable transaction journal or startup recovery.
- Temporary files from a normal handled error are removed best-effort. Startup cleanup of abandoned generic file temp files is not required in V1; temp names are private and never mistaken for user targets.

### Response

```json
{
  "batchId": "file_batch_<uuid>",
  "status": "completed|completed_with_errors|dry-run|rejected|rolled_back|partial_failed",
  "startedAt": "RFC3339",
  "updatedAt": "RFC3339",
  "operationCount": 3,
  "editCount": 1,
  "effectiveMutationCount": 1,
  "confirmation": {
    "requested": false,
    "result": null
  },
  "results": [
    {
      "index": 0,
      "id": "read-config",
      "type": "read",
      "status": "completed",
      "result": {}
    },
    {
      "index": 2,
      "id": "edit-config",
      "type": "edit",
      "status": "updated",
      "result": {}
    }
  ],
  "truncated": false,
  "truncationReason": null
}
```

Per-operation envelope rules:
- `result` reuses the single-tool response structure.
- Failed entries use `status: "failed"` plus the standard structured `error` object.
- Non-executed entries use `status: "skipped"` and a stable error code explaining the batch-level cause.
- Result envelopes always preserve input order and indexes.
- The 1 MiB output budget is consumed in input order. Envelope, status, revisions, counts, and errors are always preserved; large `content`, `matches`, context lines, and `diff` payloads are truncated first.
- Per-operation `resultTruncated` and batch-level `truncated/truncationReason` make this loss explicit.

Stable batch errors include:
- `file_batch_empty`
- `file_batch_too_many_operations`
- `file_batch_too_many_edits`
- `file_batch_duplicate_id`
- `file_batch_invalid_id`
- `file_batch_duplicate_edit_target`
- `file_batch_request_too_large`
- `file_batch_candidate_limit_exceeded`
- `file_batch_scan_limit_exceeded`
- `file_batch_rejected`
- `file_batch_confirmation_denied`
- `file_batch_confirmation_unavailable`
- `file_batch_commit_failed`
- `file_batch_rollback_failed`

### Batch audit
- One best-effort batch summary audit record captures batch id, operation/edit counts, confirmation result, final status, duration, and truncation state; audit failure is reported but does not change the batch commit result.
- Each non-dry-run edit also receives the same file-shaped audit evidence as standalone `file.edit`.
- Read/search payloads and mutation content/diffs are never audited.

## Frozen Implementation Sequence

Each implementation phase ends with focused tests and one commit. Planning files may be updated in those commits, but product scope may not be silently widened.

### Implementation Phase A — Confirmation semantics
- Replace raw confirmation-provider string execution logic with a typed ordered channel representation and one central legacy scalar parser.
- Deserialize both canonical `channels` and legacy `provider` object forms; serialize canonical `channels` only.
- Keep scalar CLI `confirmationProvider` and protocol `confirmMethod` compatibility.
- Route `ntfy` through the existing Hub confirmation request/callback path.
- Unify single, cancellable, process-batch, and downstream-MCP fallback behavior.
- Update `SafeConfigSummary` display value and confirmation documentation.
- Focused tests: alias matrix, duplicate/unknown channel rejection, canonical serialization, fallback order, cancellation cleanup, legacy Hub behavior.
- Commit intent: `refactor(agent): clarify confirmation channel semantics`.

### Implementation Phase B — `agent.info`
- Add minimal startup metadata (`startedAt`, `supervised`) to `AppState` construction helpers/tests.
- Add a dedicated local info response module; do not reuse Hub `/v1/info` DTO wholesale.
- Derive exact surface, current session capacity, path/policy summaries, confirmation availability, current reporting sender state, disk config validity, live-subset match, and restart-required fields.
- Bound/redact output exactly as frozen and avoid network probes.
- Register `agent.info` in both standalone profiles and update instructions.
- Focused tests: Normal/Room surface, exact local paths, secret absence, invalid/missing config, restart differences, profile-correct builtins, capacity exhaustion, freedesktop/ntfy availability.
- Commit intent: `feat(agent): expose standalone runtime info`.

### Implementation Phase C — File core and reads
- Add `file_ops` module with shared path-policy extraction, component-level symlink rejection, metadata, bounded UTF-8 scanning, SHA-256 revisions, target locks, and structured file errors.
- Avoid routing direct file operations through executable argument heuristics.
- Implement `file.read` with metadata-only and line-range modes.
- Extend audit types with a sibling file/batch audit shape while preserving existing command JSONL compatibility.
- Register schema/dispatch/annotations/instructions for `file.read` in both profiles.
- Focused tests: relative/absolute roots, deny/read-only precedence, directories, missing files, large metadata-only files, non-UTF-8, CRLF/final newline, long line truncation, symlink components, revision stability.
- Commit intent: `feat(agent): add bounded file reads`.

### Implementation Phase D — Search
- Add direct, pinned dependencies for ignore-aware walking, regex, and glob matching; do not shell out to `rg` and do not use transitive crates implicitly.
- Implement fixed traversal, scan, result, context, and line-size bounds.
- Register `file.search` in both profiles.
- Focused tests: literal/regex and case modes, include/exclude globs, gitignore, hidden files, binary/large/symlink skips, line/column accuracy, duplicate matches, every truncation reason, read-only roots.
- Commit intent: `feat(agent): add bounded file search`.

### Implementation Phase E — Single-file edits
- Implement exact replace, exact single-file unified patch, and guarded write/create.
- Add direct, pinned diff/patch dependencies only after API/license review; keep exact hunk behavior and no external process.
- Implement dry-run, no-op, expected revision/absence, candidate limits, bounded diff evidence, in-process target lock, final revalidation, atomic same-directory commit, ordinary permission preservation, and file audit.
- Register `file.edit` as non-read-only/destructive and closed-world.
- Focused tests: all three modes, count mismatch, malformed/fuzzy/multi-file patch rejection, revision conflict, create race, unchanged result, permission preservation, temp cleanup, confirmation allow/deny/unavailable, audit redaction, commit failure.
- Commit intent: `feat(agent): add guarded file edits`.

### Implementation Phase F — Batch
- Implement discriminated embedded operations with top-level dry-run/confirmation controls and aggregate bounds.
- Execute reads/searches first, resolve duplicate normalized edit targets, acquire sorted locks, stage all candidates, confirm once, revalidate, commit, and best-effort rollback.
- Implement deterministic original-order response envelopes and global output truncation.
- Register `file.batch` as non-read-only/destructive and closed-world.
- Focused tests: pure read partial errors, mixed pre-edit observations, duplicate aliases to one target, aggregate limits, preflight rejection with zero writes, one confirmation, no-op exclusion, sorted-lock deadlock resistance, staged commit, successful rollback, injected rollback failure, partial-failure reporting, output truncation, audit redaction.
- Commit intent: `feat(agent): add coordinated file batches`.

### Implementation Phase G — Surface, docs, and acceptance
- Update exact profile arrays/tests to Normal 23 and Room 35.
- Update standalone runtime documentation, config examples, tool workflow guidance, security boundaries, and migration notes.
- Verify Hub full/coordinator tool surfaces and public Hub `/v1/info` remain unchanged except canonical confirmation summary text where protocol compatibility requires it.
- Run formatting, focused Agent tests, standalone supervisor tests, full workspace tests, `git diff --check`, tool schema snapshots/count assertions, and live Normal/Room smoke calls for all five new tools.
- Independently inspect secret redaction, symlink handling, revision races, rollback claims, schema size, legacy config loading, and ntfy callback behavior.
- Commit intent: `docs(agent): finalize info and file tool contracts`.

## Verification Matrix

Minimum acceptance evidence:

| Area | Required evidence |
|------|-------------------|
| Confirmation migration | Legacy and canonical configs load; Agentic write emits canonical channels; all aliases preserve behavior; ntfy callback still resolves through Hub. |
| `agent.info` | No arguments; exact 23/35 surface; exact local roots; bounded policy; no secrets; truthful current capacity/connection/config comparison. |
| `file.read` | Metadata-only and ranged UTF-8 reads; deterministic revision/line/truncation data; policy and symlink enforcement. |
| `file.search` | Literal/regex/glob/ignore behavior; scan/result bounds; skip accounting; no external process. |
| `file.edit` | Revision/absence guards; exact replace/patch/write; dry-run/no-op; atomic per-file commit; bounded diff; confirmation/audit behavior. |
| `file.batch` | Ordered mixed results; read-before-write; global preflight; duplicate-target rejection; one confirmation; staged commit; rollback/partial failure reporting. |
| Compatibility | Existing 18/30 tools unchanged in behavior; five additions only; Hub execution/public surfaces not expanded. |
| Quality | Agent focused suite, standalone supervisor suite, Hub suite, Protocol suite, workspace suite, formatting and diff checks all pass. |
| Delivery | Clean worktree; no push, deployment, tag, or release without separate user confirmation. |

## Frozen Single-File Contract

### Shared file rules
- Paths may be absolute or relative to effective `workspaceRoot`.
- Deny roots override every operation. Write roots allow read/search/edit. Read-only roots allow read/search and reject edit.
- Configured policy roots are expanded and canonicalized. File targets follow existing pathPolicy semantics: resolve/canonicalize the actual target, then allow access only while the resolved target remains inside an allowed canonical root. Symlinks escaping allowed roots are rejected; mutation targets are revalidated immediately before commit. This is confirmed by D-13.
- Direct file I/O is not executed inside the process bubblewrap sandbox; `pathPolicy`, strict path resolution, revisions, and file-specific limits are its authorization and containment boundary.
- Direct file tools reject the workspace-root `.agentic-gpt-audit.jsonl` path and private file-tool temporary-name patterns. Diary, notebook, bootstrap, skills, and state remain accessible when permitted by `pathPolicy`.
- Generic file content is UTF-8 text only. Metadata-only inspection may describe a non-UTF-8 regular file, but content reads, search, and edit return `file_not_utf8` or skip it as documented.
- A regular file may be at most 8 MiB for content read or edit. Metadata-only inspection remains available above that size but does not compute revision or line count.
- Public revision format is `sha256:<64 lowercase hex characters>` over the exact file bytes, including original newline style and final-newline state.
- No operation normalizes `LF`/`CRLF`, inserts a final newline, trims whitespace, or changes encoding implicitly.
- Output errors use the existing structured `{ "error": { "code", "message", ... } }` convention.

### `file.read`

Input:

```json
{
  "path": "crates/agentic-gpt/src/main.rs",
  "includeContent": true,
  "startLine": 1,
  "endLine": 240
}
```

Rules:
- `path` is required.
- `includeContent` defaults to `true`.
- `startLine` and `endLine` are optional, 1-based, inclusive, and valid only when content is requested.
- Omitted `startLine` starts at line 1. Omitted `endLine` reads until EOF or the 256 KiB response-content bound.
- `endLine` must be greater than or equal to `startLine`.
- Metadata-only mode supports regular files and directories. Content mode requires a regular UTF-8 file.
- Reading a missing path returns `file_not_found`; it is not represented as a successful `exists: false` response.
- Content truncation occurs on a UTF-8 boundary and, when possible, a complete-line boundary.

Response fields:

```json
{
  "path": "input path",
  "resolvedPath": "/canonical/exact/path",
  "type": "file|directory",
  "sizeBytes": 1234,
  "modifiedAt": "RFC3339",
  "encoding": "utf-8|null",
  "totalLines": 120,
  "revision": "sha256:...",
  "content": "...",
  "startLine": 1,
  "returnedThroughLine": 120,
  "returnedBytes": 1234,
  "truncated": false,
  "lastLineComplete": true,
  "nextStartLine": null
}
```

- `revision`, `encoding`, and `totalLines` may be null for directories or metadata-only files above 8 MiB.
- `content` and line-range fields are omitted when `includeContent` is false.
- `nextStartLine` is present only when another line-range request can continue after a complete returned line. A single line exceeding the content bound sets `lastLineComplete: false` and no misleading continuation line.

### `file.search`

Input:

```json
{
  "path": "crates",
  "query": "HumanTerminalTracker",
  "mode": "literal",
  "caseSensitive": true,
  "include": ["**/*.rs"],
  "exclude": ["**/target/**"],
  "contextLines": 2,
  "maxResults": 50,
  "hidden": false,
  "respectGitignore": true
}
```

Rules:
- `path` and non-empty `query` are required.
- `path` may be one regular file or one directory.
- `mode` is `literal` by default and may be `regex`.
- `caseSensitive` defaults to `true`.
- `include` and `exclude` are optional glob arrays, each capped at 16 patterns; patterns are matched against `/`-normalized paths relative to the search root.
- `contextLines` defaults to 0 and is capped at 5.
- `maxResults` defaults to 50 and is capped at 200.
- `hidden` defaults to false; `respectGitignore` defaults to true.
- Directory traversal does not follow symlinks and uses an ignore-aware in-process walker. Add direct dependencies rather than shelling out to `rg` or importing transitive crates.
- Individual files above 8 MiB and non-UTF-8/binary files are skipped and counted.
- A call scans at most 10,000 files or 64 MiB of file content, whichever comes first.
- The total returned match/context payload is capped at 256 KiB. Individual displayed lines are capped at 4 KiB on a UTF-8 boundary.
- Search is line-oriented. A match reports a 1-based line and a 1-based Unicode-scalar column; Rust-regex syntax is used for regex mode. Multiple matches on one line may be separate results until the result bound is reached.
- No pagination cursor is introduced in V1; truncation metadata tells the model to narrow its query.

Response fields:

```json
{
  "query": "HumanTerminalTracker",
  "mode": "literal",
  "matches": [
    {
      "path": "crates/agentic-gpt/src/stdio_server.rs",
      "line": 110,
      "column": 8,
      "lineText": "...",
      "before": [],
      "after": []
    }
  ],
  "matchCount": 1,
  "scannedFiles": 42,
  "scannedBytes": 123456,
  "skippedFiles": {
    "tooLarge": 0,
    "nonUtf8": 0,
    "symlink": 0,
    "unreadable": 0
  },
  "truncated": false,
  "truncationReason": null
}
```

Stable search errors include `file_invalid_regex`, `file_invalid_glob`, `file_search_path_not_found`, and standard path-policy errors.

### `file.edit`

Common input:

```json
{
  "mode": "replace|patch|write",
  "path": "src/main.rs",
  "expectedRevision": "sha256:...",
  "dryRun": false,
  "needConfirm": false
}
```

Common rules:
- `mode` and `path` are required.
- `dryRun` defaults to false. A dry run performs all validation and candidate generation but does not request confirmation or write.
- `needConfirm` defaults to false. The write-root path policy is the normal authorization boundary; callers may explicitly request confirmation for sensitive edits. This default is confirmed by D-12.
- Existing-file mutation requires `expectedRevision`. There is no force/bypass flag in V1.
- Agentic holds an in-process lock per normalized target and rechecks revision immediately before commit. This serializes Agentic file edits and detects external changes observed before the final commit check.
- The contract does not claim an impossible filesystem-wide compare-and-swap guarantee against a non-cooperating writer in the final OS race window.
- Candidate output above 8 MiB is rejected.
- A no-op candidate returns `status: "unchanged"` and performs no confirmation or write.

#### Replace mode

```json
{
  "mode": "replace",
  "path": "src/main.rs",
  "expectedRevision": "sha256:...",
  "oldText": "exact old text",
  "newText": "replacement text",
  "expectedMatches": 1
}
```

- `oldText` must be non-empty.
- Matching is exact UTF-8 text matching with no newline or whitespace normalization.
- `expectedMatches` defaults to 1, must be positive, and must equal the actual count before any replacement occurs.
- All matched occurrences are replaced when the count equals `expectedMatches`.
- Count mismatch returns `file_match_count_mismatch` with expected and actual counts.

#### Patch mode

```json
{
  "mode": "patch",
  "path": "src/main.rs",
  "expectedRevision": "sha256:...",
  "patch": "--- a/src/main.rs\n+++ b/src/main.rs\n@@ ..."
}
```

- Accept one standard unified text diff for exactly one existing target file.
- File headers may be omitted. When present, normalized header paths must identify the requested target; create/delete/rename metadata is rejected.
- Hunks apply against the exact original text. Offset/fuzzy application is not allowed in V1; a non-exact hunk returns `file_patch_conflict`.
- Add a focused direct unified-diff dependency or a fully tested bounded parser; never invoke an external `patch` process.

#### Write mode

Create:

```json
{
  "mode": "write",
  "path": "src/new.rs",
  "content": "...",
  "expectedAbsent": true
}
```

Overwrite:

```json
{
  "mode": "write",
  "path": "src/existing.rs",
  "content": "...",
  "expectedRevision": "sha256:..."
}
```

- Exactly one of `expectedAbsent: true` or `expectedRevision` is required.
- Parent directory must already exist and be inside a write root; V1 does not create directories.
- `expectedAbsent` fails with `file_already_exists` if any target entry exists. Commit must use an atomic no-replace primitive on supported targets; it must never use ordinary replacement rename for creation.

### Commit and response semantics
- Successful edits write a unique temporary file in the target directory, flush/sync it, preserve existing ordinary permissions on overwrite, then atomically rename it over the target and sync the parent directory where supported.
- New files use normal create permissions subject to the process umask.
- Symlink components and targets are revalidated before commit.
- Extended attributes, ACLs, and non-portable metadata preservation are not promised in V1.
- Before/after revisions and a bounded unified diff are returned. Diff text is capped at 64 KiB and accompanied by `diffTruncated` and change counts.

```json
{
  "path": "src/main.rs",
  "resolvedPath": "/exact/src/main.rs",
  "mode": "replace",
  "status": "created|updated|unchanged|dry-run",
  "beforeRevision": "sha256:...|null",
  "afterRevision": "sha256:...",
  "beforeSizeBytes": 100,
  "afterSizeBytes": 110,
  "replacementCount": 1,
  "diff": "...",
  "diffTruncated": false,
  "changedLines": {"added": 2, "removed": 1}
}
```

Stable edit errors include:
- `file_not_found`
- `file_not_regular`
- `file_not_utf8`
- `file_too_large`
- `file_revision_required`
- `file_revision_invalid`
- `file_revision_conflict`
- `file_match_count_mismatch`
- `file_patch_invalid`
- `file_patch_conflict`
- `file_already_exists`
- `file_parent_not_found`
- `file_symlink_rejected`
- `file_confirmation_denied`
- `file_confirmation_unavailable`
- `file_write_failed`

### File audit
- Every non-dry-run `file.edit` attempt makes a best-effort file-shaped audit write, including mode, normalized path, requested confirmation, confirmation result, before/after revisions, outcome/error code, duration, and bounded count metadata.
- Audit failure does not reverse or reject an otherwise valid file result, matching existing command-audit behavior; the response reports `auditStatus: written|failed`. Audit never stores file content, search results, replacement text, patch text, or full diff.

## Frozen Confirmation Contract

### Canonical config
New and rewritten configuration serializes:

```json
"confirmationProvider": {
  "channels": ["freedesktop", "ntfy"]
}
```

Rules:
- `channels` is an ordered, duplicate-free list.
- V1 channel ids are only `freedesktop` and `ntfy`.
- The default order is `freedesktop`, then `ntfy`.
- An empty list disables confirmation delivery and preserves the existing unavailable/deny behavior when confirmation is required.
- `ntfy` continues to mean the existing Hub-relayed ntfy notification and callback path; its implementation transport is reported separately as `hub-relay`.

### Legacy input compatibility
The loader and scalar override parser accept:

| Legacy input | Canonical channels |
|--------------|--------------------|
| `none` | `[]` |
| `freedesktop` | `["freedesktop"]` |
| `hub` | `["ntfy"]` |
| `ntfy` | `["ntfy"]` |
| `freedesktop-then-hub` | `["freedesktop", "ntfy"]` |
| `freedesktopThenHub` | `["freedesktop", "ntfy"]` |
| `freedesktop-then-ntfy` | `["freedesktop", "ntfy"]` |
| `default` | configured channels when used as `confirmMethod`; default channels when used in config |

Compatibility rules:
- Legacy `{ "provider": "..." }` config remains readable.
- CLI `config set confirmationProvider <scalar>` remains supported and uses the same parser.
- Protocol `confirmMethod` remains a scalar override and uses the same parser.
- Agentic-managed config writes canonicalize to the `channels` array form.
- `SafeConfigSummary.confirmationProvider` remains a string for protocol compatibility and emits a canonical display label such as `freedesktop-then-ntfy`, `ntfy`, `freedesktop`, or `none`.
- All single, cancellable, batch, and downstream-MCP confirmation paths use one shared ordered fallback executor.

### Availability semantics
- `configured`: the channel is present in the configured ordered list.
- `available`: the Agent can currently attempt the channel.
- `supportsActions`: the channel can return allow/deny decisions.
- `deliveryHealth`: `unknown` for ntfy V1 because Hub does not advertise remote ntfy health to Agent.
- Freedesktop confirmation is available only when the local notification service exists and advertises action support.
- Ntfy is available when the Hub control/reporting sender required for confirmation is currently connected; this does not claim successful remote delivery.

## Frozen `agent.info` Contract

`agent.info` is a no-argument, read-only, non-destructive, closed-world local tool. It performs bounded local inspection only and does not initiate network probes.

### Response shape

```json
{
  "schemaVersion": 1,
  "generatedAt": "RFC3339",
  "identity": {
    "agentId": "string",
    "displayName": "string",
    "version": "string",
    "transport": "hub|tunnel-stdio",
    "profile": "normal|room",
    "hubMode": "command-capable|reporting-only|disabled",
    "startedAt": "RFC3339",
    "supervised": true
  },
  "host": {
    "hostname": "string",
    "os": "string",
    "arch": "string",
    "availableParallelism": 16
  },
  "surface": {
    "toolCount": 23,
    "tools": ["agent.info", "file.read"],
    "revision": "sha256 of canonical tool names, input schemas, annotations, and surface schema version"
  },
  "workspace": {
    "root": "/exact/path",
    "sandbox": {"enabled": false, "mode": "disabled|bubblewrap"},
    "pathPolicy": {
      "writeRoots": ["/exact/path"],
      "readOnlyRoots": [],
      "denyRoots": [],
      "truncated": false
    }
  },
  "execution": {
    "programMatching": "exact",
    "sessions": {
      "configuredMax": "auto",
      "resolvedMax": 24,
      "active": 2,
      "available": 22
    },
    "policy": {
      "counts": {"allow": 0, "confirm": 0, "deny": 0},
      "allow": [],
      "confirm": [],
      "deny": [],
      "builtinConfirm": [],
      "builtinDeny": [],
      "truncated": false
    }
  },
  "confirmation": {
    "channels": ["freedesktop", "ntfy"],
    "pendingCount": 0,
    "providers": [
      {
        "id": "freedesktop",
        "configured": true,
        "available": false,
        "supportsActions": false,
        "reason": "actions_unavailable"
      },
      {
        "id": "ntfy",
        "configured": true,
        "available": true,
        "supportsActions": true,
        "transport": "hub-relay",
        "deliveryHealth": "unknown",
        "reason": null
      }
    ]
  },
  "connections": {
    "hubReporting": {
      "enabled": true,
      "status": "connected|disconnected|disabled"
    }
  },
  "config": {
    "path": "/exact/config/path",
    "diskStatus": "valid|invalid|missing|too-large|unreadable",
    "diskModifiedAt": "RFC3339 or null",
    "liveSubsetMatchesDisk": true,
    "restartRequiredFields": [],
    "errorCode": null
  },
  "capabilities": {
    "skills": true,
    "bootstrap": true,
    "diary": false,
    "notebook": false,
    "notifications": false
  },
  "health": {
    "status": "ready|degraded",
    "issues": []
  }
}
```

### Provenance and bounds
- Exact local config/workspace/root paths are returned because this tool is exposed only to the connected local standalone model and those paths are required for correct calls.
- Secrets, secret references, raw config, command arguments, session output/history, and full mutation evidence are never returned.
- Policy rules and path roots are bounded to 128 entries total each section; overflow sets `truncated` and preserves full counts.
- Tool names are returned in full because the frozen surface is at most 35 tools.
- `generatedAt` is the observation time. V1 does not retain connection history or last network error.
- Package version comes from `CARGO_PKG_VERSION`; no unsupported build-commit claim is made.
- Disk config is read and validated on demand with a 4 MiB read bound. Effective live config is compared with disk to derive `liveSubsetMatchesDisk` and restart-required field names.
- The disk/config comparison is bounded and must not hash or expose secret-bearing raw config bytes.
- `surface.revision` hashes the surface schema version plus each ordered public tool name, canonical input schema, and MCP annotations, so schema-only connector drift is detectable.

### Health issues
V1 emits bounded structured issue codes only when actionable, including:
- `config_missing`
- `config_invalid`
- `config_unreadable`
- `config_live_subset_not_applied`
- `config_restart_required`
- `confirmation_unavailable`
- `active_session_capacity_exhausted`

`ready` means no issue currently prevents normal tool use. `degraded` does not itself reject calls; the specific subsystem remains authoritative.

## Acceptance Criteria

- The standalone public surface is exactly 23 Normal tools and 35 Room tools; all pre-existing tool names and behavior remain compatible.
- Legacy confirmation config/overrides preserve behavior, canonical writes use ordered channels, and the current Hub-relayed ntfy callback path still resolves decisions.
- `agent.info` is no-argument, bounded, locally derived, schema-drift-sensitive, useful to the model, and contains no secrets, command history, or session output.
- File tools enforce exact pathPolicy semantics, the accepted symlink/reserved-path decisions, UTF-8/text and resource bounds, stable revisions, and atomic no-replace creation.
- Existing-file edits cannot proceed without the exact expected revision; no-op/dry-run/confirmation/audit results are explicit.
- Batch reads precede its own writes, all edits preflight before commit, duplicate normalized targets are rejected, and the accepted rollback boundary is accurately reported.
- Focused failure-injection tests and the complete Agent, supervisor, Hub, Protocol, and workspace suites pass; formatting and `git diff --check` are clean.
- No product code is pushed, deployed, tagged, or released without a separate user request and confirmation of the exact side effect.

## Implementation Discretion

The Implementer may choose private module boundaries, helper names, lock-map representation, bounded-buffer data structures, exact assertion wording, and equivalent direct dependency versions/APIs after license/API verification. These choices may not alter public tool names, schemas, defaults, path/confirmation semantics, revision format, error codes, resource limits, migration behavior, or rollback claims.

The Implementer may refine non-portable parent-directory sync handling and test injection mechanisms, but must expose no stronger durability guarantee than the frozen contract.

## Key Questions

| ID | Question | Blocking | Status | Resolution |
|---|---|---:|---|---|
| Q-01 | Should `file.edit` and mutating `file.batch` request confirmation by default? | yes | resolved | A confirmed: default false; write roots + revision guards authorize normal edits, with explicit `needConfirm: true` for sensitive changes. |
| Q-02 | How should file tools treat symlinks below an allowed root? | yes | resolved | B confirmed with existing pathPolicy semantics: canonicalize the actual target and allow it only while the resolved target remains inside an allowed canonical root; revalidate before mutation commit. |
| Q-03 | What should happen when a staged multi-file commit fails after earlier files were committed? | yes | resolved | A confirmed: guarded best-effort rollback only while each target still has this batch's post-revision; otherwise report `partial_failed` without overwriting external changes. |
| Q-04 | Should direct file tools reserve Agentic-owned runtime paths? | yes | resolved | A confirmed: reject access to the root audit file and private file-tool temp names only; leave ordinary workspace directories usable under pathPolicy. |

## Decisions Made

| ID | Area | Status | Outcome | Concise rationale | Evidence |
|---|---|---|---|---|---|
| D-01 | Planning scope | confirmed | Keep `agent.info`, confirmation naming, and all file tools in one scoped plan. | The first two are small and share one standalone surface acceptance pass with the file work. | User explicitly requested one planning. |
| D-02 | Public tool names | confirmed | Add `agent.info`, `file.read`, `file.search`, `file.edit`, and `file.batch`. | Names fit existing namespaces while keeping distinct read/search/edit schemas and required batch support. | User discussion and approval to begin planning. |
| D-03 | Metadata inspection | confirmed | Do not add `file.stat`; use `file.read` with `includeContent: false`. | Avoids another descriptor without losing metadata/revision checks. | Existing resource response conventions. |
| D-04 | Ntfy ownership | confirmed | Keep ntfy publication/callback/pending state in Hub. | Agent ownership would require polling/subscription and replay/recovery machinery for little current benefit. | User explicitly preferred not moving it. |
| D-05 | Confirmation names | confirmed | User-visible channels are `freedesktop` and `ntfy`; Hub is reported only as ntfy transport. | Describes the actual human channel instead of the relay implementation. | User explicitly accepted these names. |
| D-06 | Info audience | confirmed | `agent.info` is optimized for the connected model and may expose exact local operational paths while excluding secrets/history. | Exact paths and effective limits are required for correct tool use. | User asked to design info from the assistant's perspective. |
| D-07 | File content scope | confirmed | V1 reads/searches/edits UTF-8 text only; metadata-only inspection remains available for other regular files. | Keeps revisions, patching, diffs, and encoding behavior reliable. | Repository resource conventions and threat analysis. |
| D-08 | Surface parity | confirmed | Add all five tools to both Normal and Room standalone profiles only, yielding 23/35 tools. | Both profiles need local info/files; Hub surfaces remain separate. | Current exact 18/30 profile arrays. |
| D-09 | Surface revision | confirmed | Hash canonical names, input schemas, annotations, and surface schema version. | Detects stale schema even when tool names/counts are unchanged. | Refine gap analysis. |
| D-10 | File create safety | confirmed | Use atomic no-replace creation for `expectedAbsent`; ordinary rename replacement is forbidden. | Prevents a create race from silently overwriting a newly appeared target. | Filesystem race analysis. |
| D-11 | Audit behavior | confirmed | File and batch audits are best-effort, redacted, and report `auditStatus`; audit failure does not alter commit outcome. | Matches existing command-audit behavior and avoids impossible post-commit rollback solely for audit. | `audit.rs` callers ignore write errors. |
| D-12 | File confirmation default | confirmed | `needConfirm` defaults to false for file edits/batches; callers opt in for sensitive changes. | Write roots and revision guards are the normal authorization boundary; this preserves practical edit usability. | User selected Q-01A. |
| D-13 | Symlink policy | confirmed | Follow existing pathPolicy semantics: canonicalize the actual target and allow it only when it remains inside an allowed canonical root; revalidate before mutation commit. | Keeps file tools consistent with existing pathPolicy while rejecting escapes. | User requested following existing pathPolicy. |
| D-14 | Batch rollback | confirmed | Guarded best-effort rollback; restore only if the current revision still equals this batch's post-revision, otherwise report `partial_failed`. | Preserves coordinated behavior without overwriting external changes. | User selected Q-03A. |
| D-15 | Reserved runtime paths | confirmed | Protect the root audit file and private file-tool temp names only; other Agentic-managed directories remain governed by pathPolicy. | Protects operational integrity without broadly hiding useful workspace content. | User selected Q-04A. |

## Implementation Handoff

- **Plan maturity:** implementation_ready
- **Design phase:** complete
- **Implementation authorized:** yes
- **Entry phase:** Implementation Phase A — Confirmation semantics
- **Frozen decisions:** D-01 through D-15
- **Open blocking decisions:** none
- **Implementation discretion:** see `Implementation Discretion`
- **Verification convention:** focused tests per phase, then full Agent/supervisor/Hub/Protocol/workspace suites, live Normal/Room smoke, formatting and diff checks
- **Commit convention:** one focused commit per Implementation Phase A through G
- **Design checkpoint:** not set
- **Next invocation:** `$planning-with-files` without `$refine-implementation-plan`

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| Direct `find` of paths under `$HOME/.codex` was rejected by process path preflight | 1 | Ran `bash -lc` from the allowed repository working directory; the shell performed the discovery as the user indicated. |
| Six-element discovery `process.batchExec` was rejected by active capacity | 1 | Switched to one `bash` process that read the same sections serially; recorded capacity visibility as an `agent.info` requirement. |

## Notes
- Re-read this plan before each contract decision.
- Follow the planning skill's two-action rule by persisting discoveries in `findings.md`.
- Do not implement product code during planning.
