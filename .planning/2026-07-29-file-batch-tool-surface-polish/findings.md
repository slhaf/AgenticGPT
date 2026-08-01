# Findings: File Batch and Tool Surface Polish

## Source requirements
- Room notebook passage `psg_f444de745cc24944801b5992ea472db9` records three repeated real-use failures: hard rejection above `contextLines=5`, rejection of multiple edits to one file, and missing-file/revision errors collapsing unrelated edits.
- Room notebook passage `psg_c7c550fe9d6541079539c0c124fbefb1` requests a complete public tool schema/description audit grounded in actual model misuse and rejection cases.
- The notebook establishes one-file transactions and cross-file partial success by default. On 2026-07-30 the user confirmed removal of the explicit whole-batch rollback mode after reviewing its actual guarantees and usage.

## Repository baseline
- Repository: `/home/slhaf/Projects/AgenticGPT`.
- Baseline: clean `main`, aligned with `origin/main`, current code/version v0.9.0.
- The completed v0.9 interface plan remains historical; this work has a separate active scoped plan.
- Product code remains unchanged; only `.planning` state has been added/selected.

## Live reproductions
- `file.search` with `contextLines:8` returns `file_context_limit_exceeded` and no search result.
- Creating three new planning files beneath a missing parent plus an unrelated existing-file update returned `file_parent_not_found` for the new files and skipped the unrelated update as `file_batch_rejected`.
- After the parent existed, one missing `expectedRevision` on the existing-file update still skipped all otherwise valid new-file creations.
- These demonstrate two distinct brittle boundaries: strict harmless-limit rejection and one global mutation preflight across independent targets.

## Observed usage review
- The available Laptop audit contains 410 `file.batch` records: 302 read-only and 108 mutating.
- 97 of the 108 reconstructed mutating batches touched multiple paths, so the batch tool itself is clearly valuable for round-trip reduction and coordinated work.
- Across all 315 logical batch-edit audit records, no `rolled_back`, `rollback_failed`, `not_committed`, or `partial_failed` outcome appears.
- This is evidence from the currently retained local audit, not proof that rollback can never be useful; however it shows no observed workload that depends on the rollback path.
- Common multi-path batches were planning files, code plus styles/types/tests, and related source files. These need coordinated review but are normally recoverable through the worktree/version control and do not require a generic filesystem transaction.

## Existing implementation

### Search
- `crates/agentic-gpt/src/file_ops.rs` defines `MAX_SEARCH_CONTEXT = 5` and rejects larger values before scanning.
- `SearchOptions.context_lines` is used directly to build before/after context arrays.
- Search output includes query/mode, matches, scan counts, skipped-file counts, truncation, and reason, but no requested/effective limit evidence.
- `stdio_server.rs` advertises the static phrase `Context lines, max 5.` for both single and nested batch search.
- `LimitsConfig` contains only `maxConcurrentTasks` and `maxActiveJobs`; no file-search limit exists.
- The entire `limits` object already hot-reloads atomically through `apply_standalone_live_subset`, so an additive defaulted limit naturally fits that lifecycle.
- `agent.info` exposes Job and MCP limits but no file-tool operational bounds.

### Batch mutation
- `file.batch` is a bespoke cross-file transaction coordinator, not a loop over `file.edit`.
- Reads/searches execute first against disk. Any hard read/search failure currently helps reject every edit.
- Edit targets are resolved/normalized and duplicate normalized paths are rejected with `file_batch_duplicate_edit_target`.
- All targets are locked in sorted order; every candidate is preflighted and staged before one optional confirmation.
- Commits are ordered. A commit failure invokes guarded best-effort rollback across already committed files.
- Any target-resolution, guard, candidate, or staging error causes otherwise valid edits to become `file_batch_rejected`.
- `batch_candidate` rereads one actual file and applies exactly one operation, which is why same-file chaining requires a new in-memory candidate layer rather than deleting one validation branch.
- Existing files require exact `expectedRevision`; absent files require `write + expectedAbsent:true`; `patch` explicitly requires an existing file.
- Parent directories must already exist and are revalidated before commit. Auto-directory creation would alter security/path semantics and is not needed for the recorded defect.

### Output and audit
- Results already preserve input index, optional id, operation type, status, nested result/error, and bounded truncation.
- Truncation removes content/matches/diffs before collapsing to compact operation envelopes.
- Finalization writes one redacted `FileAuditRecord` for every logical edit plus one `BatchAuditRecord`.
- Aggregate audit currently records only operation/edit counts, confirmation, outcome, duration, and truncation; it cannot explain file-group partial success.

### Tool surface
- Standalone descriptions are centralized in `stdio_server.rs`; many are terse labels rather than selection/constraint guidance.
- Exact Normal/Room counts and finite descriptor/schema byte budgets are tested.
- Descriptor tests prove schema-to-serde parity and surface revision changes but do not prove model tool selection or conditional-parameter understanding.
- Hub does not expose `file.*`; file contract work is standalone/local-Agent only. `job.*` and `mcp.*` descriptions have separate Hub definitions and require shared-behavior parity.
- `mcp.batch` currently says `atomically admit`, which is technically accurate but should explicitly deny downstream side-effect rollback to prevent a transactional inference.

## Contract gaps
| Surface | Gap | Classification |
|---|---|---|
| Search input | Static maximum is a brittle hard error and cannot reflect live configuration. | confirmed requirement |
| Search output | No requested/effective/clipped evidence. | confirmed public contract |
| Configuration | No named file-search context bound, validation, diagnostics, or docs. | confirmed operational contract |
| Batch identity | Duplicate normalized targets are forbidden rather than grouped. | confirmed requirement |
| Guard lifecycle | Multiple operations may repeat the same base revision; intermediate revisions are unknowable to callers. | confirmed public contract |
| Failure behavior | One invalid file/read/search rejects all edits. | confirmed requirement to change |
| Legacy rollback compatibility | Current docs promise guarded best-effort rollback, but no visible audit shows it executing and the guarantee is not crash-atomic or snapshot-isolated. | confirmed removal for v0.10 |
| Results/audit | No file-group status/correlation or partial-success counts. | confirmed public contract |
| Tool guidance | Use/non-use, conditional fields, partial states, and transaction boundaries are incomplete. | confirmed audit scope |
| Verification | No deterministic task-contract corpus or optional model selection evaluation. | confirmed verification design |

## Options and tradeoffs

### Does a cross-file rollback mode still serve a unique purpose?
- **Recommended — remove it:** keep `file.batch` for one-round-trip reads/searches/edits and per-file grouping, but expose no atomicity selector and delete best-effort cross-file rollback.
- The legacy path prevents writes on preflight failure and attempts guarded rollback on a normal mid-commit failure, but it does not protect against crash, SIGKILL, power loss, storage failure, or another process observing the intermediate per-file renames.
- `dryRun:true` already provides whole-request validation/preview. The real call remains guarded by exact revisions/absence, so races fail safely at the affected file rather than silently overwriting it.
- Code/planning work is recoverable through the worktree and version control. A genuinely indivisible package/config-set update needs a domain-specific staged directory rename, database transaction, or durable journal—not a generic multi-file edit option.
- Keeping the mode would preserve compatibility but also retain the most complex part of `file.batch`: original-byte retention, global staging, rollback state machines, rollback-only errors, and failure-injection coverage.

### Same-file guard model
- **Recommended — merge base assertions:** accept omitted or repeated identical guards, require at least one valid base assertion, reject conflicts, then apply all operations to the staged candidate. This accepts common model output without weakening optimistic concurrency.
- **First-operation-only guard:** simpler internally but rejects harmless repeated guards and adds another model-only rule.
- **Intermediate revision per operation:** impossible for ordinary callers to know before the batch runs.
- **New nested edit-group schema:** structurally clean but larger and unnecessarily breaks the existing flat ordered-operation contract.

### Search limit
- **Recommended — live `limits.maxFileSearchContextLines`, default 5, hard safety ceiling 100:** preserves current default, allows local tuning, and prevents pathological allocations.
- **No limit:** output remains bounded, but a single match can still construct very large before/after arrays and consume avoidable memory.
- **Static larger constant:** fixes today’s value but not the configurability/observability defect.

### Write/create/upsert
- **Recommended — retain current guarded `write`:** `expectedAbsent:true` means create; `expectedRevision` means overwrite. Improve descriptions rather than adding aliases.
- New `create`/`upsert` modes would enlarge schemas and invite unguarded overwrite semantics without enabling a missing capability.

### Contract testing
- **Recommended split:** deterministic fixtures are required CI; a provider-neutral model runner is optional/manual. This distinguishes runtime correctness from nondeterministic model usability.
- Required live-model CI would be flaky, costly, secret-bearing, and unsuitable as the only correctness gate.

## Decision rationale
- D-01/D-02: clamping is safe because scan/output bounds remain authoritative; explicit evidence prevents silent semantic degradation.
- D-03/D-04: file grouping preserves normalized-path security and one lock/write while removing accidental cross-file coupling.
- D-05: repeated identical revisions are best interpreted as one group-level base assertion, which matches how models naturally duplicate guarded edits.
- D-06: current guarded write semantics already satisfy the intended missing-target boundary; failure isolation, not an upsert feature, is missing.
- D-07: retaining pre-edit read/search snapshots keeps scope and memory semantics understandable. Interleaved reads over staged candidates would require a virtual filesystem and new search accounting.
- D-08: one confirmation remains valuable, but only valid effective groups should enter its preview.
- D-09/D-10: adding a file-group layer is necessary because logical operation success and physical file commit are no longer one-to-one.
- D-11/D-12: richer concise descriptors plus detailed fixtures avoid exceeding current surface budgets while creating a repeatable feedback loop.
- D-13: the batch tool's observed value is aggregation, not rollback. Removing pseudo-transaction semantics makes the public contract more truthful and materially simplifies the refactor.

## Risks and mitigations
- **Compatibility break:** remove rollback-only states/errors in v0.10 migration notes and include deterministic per-file partial-success cases.
- **Alias race/symlink change:** retain canonical resolution, sorted locks, and pre-commit revalidation per group.
- **Misleading successful staged operations:** mark group-not-committed/failed outcomes explicitly and retain the failing operation index/id.
- **Audit bloat/leakage:** add only ids/counts/revisions/statuses; never raw old/new/patch/content or full diff.
- **Schema bloat:** keep examples in checked fixtures/docs and enforce existing finite budgets.
- **Config incompatibility:** add a serde default and strict range validation; existing v0.9 configs continue loading.
- **Version ambiguity:** treat removal of cross-file rollback semantics as v0.10.0 with migration/release notes.

## Final contract confirmation
- Q-01: remove the public atomicity selector and the legacy best-effort cross-file rollback path.
- Q-02: merge same-file optimistic guards at the file-group base and chain later operations over the staged candidate.
- Q-03: expose `limits.maxFileSearchContextLines` with default 5, valid range 0–100, clipping, and requested/effective evidence.
- The user accepted the consolidated D-01 through D-13 contract on 2026-07-30; no user-owned product choice remains for the Implementer.

## Remaining implementation discretion
- Private group/candidate structs and collection choices.
- Compact group-id generation and helper naming.
- Exact failure-injection mechanism and optional model-provider adapters.
- Test assertion wording and internal refactoring boundaries that do not alter the frozen contract.

## 2026-08-01 implementation handoff
- The active plan marker resolves to this scoped plan; the worktree is clean on `main` before product changes.
- Phase A is the current implementation boundary. It covers only live search context limits, search result evidence, diagnostics, descriptors, configuration/docs, and focused regression coverage; batch mutation semantics remain frozen for later phases.
- The accepted contract requires `limits.maxFileSearchContextLines` (default 5, range 0–100), clamping rather than rejecting non-negative overshoots, and requested/effective/clipped evidence on single and batch search results.

## Phase A code mapping
- `file_ops::search` currently owns the hard-coded `MAX_SEARCH_CONTEXT = 5` rejection and accepts `usize`, so typed negative/non-integer input rejection occurs at the RPC DTO boundary rather than inside the scanner.
- The shared search result is a `serde_json::Value`; single and batch callers both pass `SearchOptions`, making one result-envelope change sufficient for parity.
- `LimitsConfig` is `deny_unknown_fields` and currently contains only `maxConcurrentTasks` and `maxActiveJobs`; `Config::default_config` and the existing live limits subset are the migration points for the additive hot-reloadable field.
- Descriptor wording for both single and nested batch search is still the static `Context lines, max 5.` and has no `minimum`; `agent.info` has existing operational diagnostics that can expose the effective limit without adding a new tool.
- Existing response families already use bounded `warnings` arrays, so search can report a single stable clipping warning without introducing a new error channel.
- `apply_standalone_live_subset` replaces `live.limits` atomically, and the normal watcher replaces the whole validated config; adding the field to `LimitsConfig` automatically gives Phase A live reload for standalone/local calls.
- `agent_info::collect` currently publishes `execution` and `config` diagnostics plus serialized `limits` near its lower-level helper; the effective search limit should be surfaced in the top-level operational payload while preserving the existing shape/count budgets.
- `FileSearchArgs` and nested batch search use `usize`, which already rejects JSON negatives, fractions, strings, and booleans during argument validation. The public schema still needs explicit `minimum: 0` and dynamic-limit wording.
- The existing stdio tests cover descriptor defaults/parity, live dispatch, and a mixed file.batch read/search/edit flow. Phase A can extend these in place for schema minimum/evidence and batch-search clipping without changing batch mutation tests.
- `docs/configuration.md`, its Chinese counterpart, `config.example.json`, and `docs/standalone-runtime.md` are the user-facing configuration/runtime references that currently omit the file-search context limit.

## Phase A implementation notes
- Search now needs two entry paths: a default wrapper for direct unit callers and an explicit configured-limit path for single and batch dispatch. This keeps existing internal test fixtures stable while ensuring runtime calls use the live `limits` snapshot.
- The configured value is serde-defaulted to 5 and range-checked at 0–100; the search result will retain bounded scan/output limits and add only requested/effective/clipped fields plus one stable warning code when clipping occurs.

## Verification notes
- `cargo fmt --all` completed after the first formatting check identified only mechanical indentation/wrapping changes.
- `cargo check -p agentic-gpt` passed; the default `search` test helper is explicitly annotated as intentional compatibility coverage, and `cargo clippy -p agentic-gpt --all-targets -- -D warnings` is clean.
- The first focused test command used `--lib`, but this crate has no library target; package-level binary tests are the correct harness.
- Package-level tests compiled and ran all 232 unit tests: 228 passed. Four pre-existing environment-sensitive tests failed while creating/binding local sockets or starting a fake tunnel/local download, each with `Operation not permitted` or `local_mcp_bind_failed`; none touched Phase A search/config paths.
- Focused Phase A filters pass: config range/default tests plus single-search clipping/configured-20/configured-0 cases (2), the batch read/search/edit clipping case, the `agent.info` diagnostic, standalone live-subset replacement, and both descriptor/schema parity tests.

## Phase A completion evidence
- The final implementation preserves the pre-edit read/search ordering and all existing scan/result/output bounds; only context-line selection and its evidence changed.
- Runtime callers pass the cloned live limit from `Config`; config reload tests prove a valid value of 20 is applied and an invalid MCP candidate still leaves the live subset intact.
- The schema advertises `minimum: 0` and no static maximum, while runtime output makes clipping explicit. Existing tool counts and schema budget tests remain green.
- Final gates passed: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo clippy -p agentic-gpt --all-targets -- -D warnings`, `git diff --check`, and all focused Phase A filters.
