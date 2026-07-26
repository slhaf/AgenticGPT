# Findings & Decisions: Standalone Info, File Tools, and Confirmation Naming

## Requirements Captured from Discussion
- Keep the three requirements in one planning record.
- Add model-facing `agent.info`; design it from the connected model's operational needs rather than reusing the public Hub `/v1/info` DTO.
- Add a namespaced file family rather than a bare `file` tool.
- Current candidate tools are `file.read`, `file.search`, `file.edit`, and `file.batch`.
- `file.read` can return metadata only, so no separate `file.stat` descriptor is needed.
- File editing must support complex but controlled work, revision conflicts, atomic replacement, bounded diffs, and batch use.
- Confirmation names can be `freedesktop` and `ntfy`; ntfy may remain implemented through Hub relay for now.
- Do not treat every user question as a design rejection; retain the best-supported design until an explicit adjustment or stronger evidence changes it.

## Repository Findings

### Standalone MCP surface
- `crates/agentic-gpt/src/stdio_server.rs` centrally owns standalone tool-name arrays, dispatch, argument decoding, JSON schemas, descriptions, annotations, exact profile surfaces, and most focused tool tests.
- Current instructions mention `process.*`, tmux, skills, and bootstrap, so the new file family and `agent.info` need concise workflow guidance there.
- Current exact standalone surfaces are 18 Normal tools and 30 Room tools. Five additions imply 23 and 35 if no existing tool is removed.

### Confirmation flow
- `Config.confirmation_provider.provider` is currently a string and the default is `freedesktop-then-hub`.
- `confirmation.rs` repeats provider normalization/fallback logic in single, cancellable, batch, and MCP-tool confirmation paths.
- The current `hub` provider sends `AgentMessage::ConfirmationRequest`; Agent retains a one-shot waiter keyed by request id.
- Hub owns pending callback state, publishes the ntfy notification, exposes HTTP action callbacks, and returns a `ConfirmationResponse` to Agent.
- Renaming the public channel to `ntfy` can preserve this transport; moving callback ownership is explicitly unnecessary for this plan.

### Existing safe summaries and config
- `config.rs` already builds `SafeConfigSummary`, including workspace summary, safe path-policy roots, policy counts/rules, built-in rules, confirmation provider, and limits.
- Existing Hub-facing summaries shorten private home paths and may intentionally hide detail. A local `agent.info` response may need a separate contract because exact paths are operationally useful to the connected model.
- Live reload currently applies `policy`, `pathPolicy`, and `limits`; startup identity remains restart-required. The new info response should distinguish effective current state from on-disk state and cached observations.

### Path policy
- `exec.rs` already expands paths, canonicalizes existing paths or the nearest existing parent, normalizes policy roots, applies deny roots first, distinguishes write roots from read-only roots, and resolves relative paths against `workspaceRoot`.
- These helpers are currently shaped around command argument preflight. File tools should reuse or extract the core path decision rather than imitate shell-command heuristics.
- Absent write targets and symlinks require explicit tests because canonicalizing only an existing parent is safe only when the final replacement cannot redirect through a symlink race.

### Hashing, text, and atomic writes
- SHA-256 helpers already exist in bootstrap, skills, Hub, tunnel distribution, and skill installation code, but no obvious shared digest utility exists yet.
- `skills.read` already reports UTF-8/base64 encoding, content, size, and SHA-256 for bounded package resources. Its response conventions are useful evidence, though generic file tools should remain text-only unless a later decision expands scope.
- Notebook, skills, config, and skill-install code already use temporary-file plus `fs::rename` patterns. Some components have rollback logic for directory replacement.
- No existing general unified-diff parser or text-patch subsystem was found in the first scan; dependency and implementation choices remain open.

## Initial `agent.info` Operational Questions
A useful response should let the model determine, in one call:
- Which Agent/profile/build/runtime it is connected to.
- Which exact tool surface is active and whether the connector is stale.
- The effective workspace and path-policy boundaries needed for subsequent tool calls.
- Current active-session capacity and resolved configured limit.
- Which confirmation channels are configured and which are currently usable or only observed/cached.
- Tunnel supervision and Hub-reporting connectivity.
- Which config revision is effective, whether the last live reload succeeded, and which changed fields require restart.
- Whether there are actionable structured health issues.

The response must not include secrets, tokens, command arguments, session output/history, or unbounded raw configuration.

## Initial File Contract Questions
- Whether `file.read` supports line ranges only, byte ranges only, or both.
- Whether metadata includes modification time/inode in addition to content SHA-256; content digest is the likely public revision.
- Whether a missing file returns a typed result or a tool error.
- Whether `file.search` uses an in-process regex walker or shells out to `rg`; product reliability and cross-platform behavior favor in-process, but performance and ignore semantics need comparison.
- Whether default search respects `.gitignore`, skips hidden files, and skips binary files.
- Whether `file.edit` accepts standard unified diff, a simpler single-file hunk format, or structured line edits.
- How newline style and final newline are preserved.
- Whether dry-run computes and returns the same revision/diff evidence without writing.
- Whether write confirmation is always requested or follows existing configured policy/explicit `needConfirm` semantics.

## Batch Design Constraints
- Batch is required for efficient multi-file reads and coordinated multi-file edits.
- All operations should be decoded and bounded before execution.
- A mutating batch should request one human confirmation with a bounded aggregate preview.
- Validation failures should occur before any mutation when possible.
- Per-file replacement can be atomic; multi-file crash atomicity should not be claimed without a recovery journal.
- Results should preserve input order and reuse single-operation result envelopes.
- Duplicate mutation targets and same-path read/write ordering remain open decisions rather than silently guessed behavior.

## Resources
- `~/.codex/skills/planning-with-files/SKILL.md`
- `crates/agentic-gpt/src/stdio_server.rs`
- `crates/agentic-gpt/src/config.rs`
- `crates/agentic-gpt/src/confirmation.rs`
- `crates/agentic-gpt/src/exec.rs`
- `crates/agentic-gpt-hub/src/main.rs`
- `crates/agentic-gpt/src/skills.rs`
- `crates/agentic-gpt/src/bootstrap.rs`
- `docs/standalone-runtime.md`

## Discovery Update: Config Serialization and Runtime State

### Confirmation config shape
- `ConfirmationProviderConfig` currently serializes exactly as `{ "provider": <string> }`; there is no custom deserializer or canonicalization at config load time.
- Aliases such as `default` and `freedesktopThenHub` are normalized only inside execution-time confirmation functions, and the same matching logic is duplicated across single, cancellable, batch, and MCP-tool paths.
- Therefore a truthful rename should centralize parsing into a typed/canonical channel sequence while preserving legacy input aliases. Otherwise runtime behavior, `agent.info`, config summaries, and config writes can disagree.
- Current config writing serializes the in-memory string directly. A migration decision is required: either preserve the loaded legacy literal through writes or canonicalize to the new representation. Repository precedent for top-level skills favors canonical serialization on a later config write while accepting legacy input.

### Existing `AppState`
- `AppState.runtime` already provides authoritative transport (`hub` or `tunnel-stdio`), capability profile (`normal` or `room`), Hub mode (`command-capable`, `reporting-only`, or `disabled`), and coarse capabilities.
- `AppState.config_path` and the live `Arc<RwLock<Config>>` expose the effective current configuration and exact local workspace path.
- Session, pending-confirmation, and temporary-allow collections are available for bounded current counts.
- `hub_sender` and `reporting_sender` expose only a current sender presence check. They do not retain connection timestamps, last disconnect/error, last observation time, or remote ntfy readiness.
- `AppState` currently has no general startup timestamp, build commit, config revision, last live-reload result, restart-required fields, or health issue registry.
- Initial inspection suggested a runtime-diagnostics registry; later evidence showed most config/restart health can be derived on demand by comparing disk config with the effective live config. Only minimal startup metadata may be needed.

### Safe summary reuse boundary
- `Config::safe_summary()` is a useful source for policy rules, path-policy roots, sandbox state, confirmation provider, and tunnel configuration.
- It is intentionally shaped for Hub/public summaries: workspace and home paths may be abbreviated and built-in policy uses Normal profile semantics.
- `agent.info` should not blindly reuse it as its entire response. It may reuse selected builders while supplying exact local paths and profile-correct built-ins for the currently connected local model.

## Discovery Update: Reload, Reporting, Capacity, and Dependencies

### Reload and restart diagnostics
- `watch_standalone_live_config` polls the config mtime every two seconds, applies only policy/pathPolicy/limits, and logs success or rejection. It retains no last-success/last-error record in state.
- In supervised mode, invalid live reload warnings are intentionally left to the supervisor; the worker therefore cannot infer the supervisor's warning history from its current state.
- Supervisor `StartupIdentity` already enumerates restart-required identity fields and tracks observed/warned file versions, but this state is private to the supervisor and not shared with the hidden worker that serves `agent.info`.
- A useful info contract must either add a small shared/derived status channel or reduce its promise. Simply claiming `restartRequired` from the worker would be inaccurate without comparing effective identity against the current on-disk candidate.

### Hub reporting diagnostics
- WebSocket/SSE reporting connection code sets `hub_sender` and `reporting_sender`, logs connected/disconnected, and clears the senders when the exact channel closes.
- Connection failures are returned to an outer reconnect loop but are not retained in `AppState`.
- `hub_sender.is_some()` is authoritative only for current local ability to enqueue confirmation control messages; it does not prove Hub accepted ntfy publication or that the user's notification client received it.
- The initial `agent.info` design should report ntfy as `available` when the Hub relay sender exists, not `ready`, unless Hub later advertises channel health explicitly.

### Capacity
- `sessions::current_sessions()` refreshes/prunes and returns active sessions in `starting`, `running`, or `waiting_confirmation`; this can supply an authoritative active count.
- Resolved max-active capacity is computed from the live config. Available capacity can be derived as `limit.saturating_sub(active)`.
- During discovery, a six-element `process.batchExec` was rejected with `max_active_sessions_reached`; a single shell process succeeded. This is direct evidence that model-facing capacity diagnostics are operationally useful.

### Dependencies
- The Agent crate directly depends on SHA-256 and standard async/network/serialization crates, but not a regex engine, ignore-aware filesystem walker, unified-diff parser, or diff renderer.
- Adding `file.search` and `file.edit.patch` will require either focused new direct dependencies or deliberately bounded in-house implementations. Transitive dependencies must not be imported implicitly.

## Discovery Update: Tool Metadata, Confirmation Policy, and Config Writes

### Tool metadata
- Standalone descriptors centrally set MCP `readOnly`, `destructive`, and `openWorld` annotations.
- `file.read`, `file.search`, and `agent.info` should be read-only and non-destructive; local filesystem access is not an open-world network interaction.
- `file.edit` and any mutating `file.batch` are non-read-only and destructive. Because annotations are descriptor-wide, `file.batch` must be marked destructive even when a particular call contains only reads.
- The current schema builder is hand-written JSON Schema and can express discriminated `oneOf` operation shapes, though this will add schema weight that should be bounded and reused in code.

### Confirmation boundary for direct file writes
- Existing command policy evaluates an executable name plus exact argument prefix. A direct file mutation has no truthful executable identity.
- Inventing a pseudo-program only to reuse command rules would conflate command policy with file capability and make user configuration obscure.
- Proposed boundary: pathPolicy authorizes read/write location; `needConfirm` explicitly requests user confirmation; the MCP destructive annotation informs model/UI behavior; audit records every mutation. Whether `file.edit` defaults `needConfirm` to true remains a user-facing contract decision.
- A mutating `file.batch` should request at most one confirmation with an aggregate bounded preview.

### Confirmation normalization
- Provider alias handling is currently duplicated across four code paths. This plan should introduce one typed/canonical parser and one ordered fallback executor used by single, cancellable, batch, and MCP-tool confirmation.
- The current remote route named `hub` is operationally an ntfy channel implemented by Hub relay. New configuration and info output should say `ntfy`; transport details may separately say `hub-relay`.
- Legacy request/config literals must remain accepted even after canonical output changes.

### Config persistence
- Config mutations use `write_config_with_backup`, which currently serializes the in-memory struct with `serde_json::to_string_pretty` after backup handling.
- Therefore any typed canonical confirmation representation will naturally be emitted on the next Agentic-managed config write, matching the repository's existing canonical-migration pattern.

### Search/patch dependencies
- `walkdir` exists only transitively in `Cargo.lock`; it is not a direct dependency and must not be treated as available API.
- No direct or locked `regex`, `ignore`, `diffy`, `similar`, or `globset` package was found by the focused scan except transitive `walkdir`.

## Discovery Update: Protocol Compatibility and Derived Info State

### Existing compatibility surfaces
- CLI `config set confirmationProvider <value>` is a legacy scalar entry point.
- Hub protocol `ExecRequest` and `BatchExecRequest` retain optional scalar `confirmMethod` overrides.
- `SafeConfigSummary` currently has one `confirmationProvider: String` field and is sent in Agent Hello messages to Hub.
- Canonical config may become an ordered channel list, but legacy scalar CLI/protocol inputs must parse through the same central channel parser. Hub protocol fields do not need to become arrays in this delivery.
- A conservative Hub compatibility strategy is to keep `SafeConfigSummary.confirmationProvider` as a canonical display label while `agent.info` exposes the full ordered channel array.

### Hub ntfy knowledge
- Hub `/v1/info` already computes whether ntfy is configured and maintains an internal ntfy health cache.
- Reporting-only `HubMessage` currently contains only heartbeat acknowledgements and confirmation responses; it does not advertise ntfy configuration/health to Agent.
- This plan need not widen the protocol merely for `agent.info`. Agent can truthfully expose channel configuration plus relay availability (`hub_sender` present) and mark remote delivery health as unknown.

### Derived config health instead of a large diagnostics registry
- The live worker config retains immutable startup fields from initial load and only replaces policy/pathPolicy/limits during valid live reload.
- Therefore `agent.info` can read and validate the current disk config on demand, compare it against the effective live config, and derive:
  - disk config validity and SHA-256/mtime;
  - whether the live-reload subset matches disk;
  - which restart-required fields differ;
  - whether the current configured confirmation route is presently enqueueable.
- This avoids maintaining duplicate last-reload/restart state across supervisor and worker. Minimal startup metadata such as `startedAt` and `supervised` may still be added to `AppState`.
- Connection history/last error can be omitted from V1; current sender presence is enough for immediate model decisions and does not overclaim remote health.

### Version/build information
- The stdio MCP implementation already exposes `CARGO_PKG_VERSION`; no Git commit is compiled into the binary.
- `agent.info` can reliably expose package version. Build commit should be omitted unless a separate build-metadata mechanism is intentionally added.

## Discovery Update: Final Phase 1 Findings

### Output and text conventions
- Existing resource APIs use SHA-256, explicit byte/line metadata, UTF-8 boundary-safe truncation, `truncated`, and line-completeness fields. The file tools should reuse these conventions rather than returning an opaque clipped string.
- Existing generic skill resources may return base64 for binary content, but the new generic file editor is intentionally text-only. `file.read` should reject non-UTF-8 rather than silently base64-encode content that `file.edit` cannot safely modify.

### Symlink and path behavior
- Existing command path policy canonicalizes existing paths and the nearest existing parent, but generic file access needs stricter component-level symlink handling.
- Existing skill-resource reads already reject symlinks at every traversed component. File tools should use equivalent component checks for mutation targets and should not follow symlinks by default.
- Deny roots override read/write roots; read-only roots permit read/search but reject edit; workspaceRoot is implicitly a write root.

### Audit
- Current audit records are command-shaped (`program`, `args`, `workingDirectory`). Direct file operations need an extended or sibling audit shape rather than pretending to be shell commands.
- File audit evidence should include tool/action, normalized target paths, requested confirmation, confirmation result, pre/post revisions, outcome/error code, duration, and bounded mutation counts, but never full file content or full diff.

### Confirmation availability probe
- Existing freedesktop detection can report provider availability and action support at call time.
- Confirmation requires action support, so `agent.info` must distinguish desktop notifications existing from desktop confirmation actually being available.

## Frozen Phase 3 Decisions
- `file.read` provides both bounded content reads and metadata-only inspection; no separate `file.stat` tool.
- `file.search` is an in-process, ignore-aware, literal/regex line search with fixed traversal and response bounds.
- `file.edit` has exact `replace`, exact unified `patch`, and guarded `write` modes.
- Existing files always require an exact content revision; new files require `expectedAbsent: true`.
- Direct edits default to no confirmation because write roots are the capability boundary; `needConfirm` remains available and mutations are marked destructive/audited.
- V1 refuses symlink traversal and binary editing, preserves exact text/newlines, and commits atomically per file.
- In-process path locks plus a final revision check prevent Agentic-to-Agentic lost updates and detect observed external changes; no false filesystem CAS promise is made.

## Frozen Phase 4 Decisions
- `file.batch` supports 1–32 operations and at most 16 edits, with aggregate request/candidate/search/output bounds.
- Pure read batches tolerate per-operation failures; mutating batches treat every hard operation error as a global no-write rejection.
- Reads/searches run before batch-owned writes and return a pre-edit view, without claiming a global filesystem snapshot.
- Only one edit per normalized target is allowed; all target locks are acquired in sorted order.
- Mutations are fully staged before one optional confirmation and final revalidation.
- Normal commit failures trigger bounded best-effort rollback; status distinguishes full rollback from partial failure.
- Crashes and power loss remain outside cross-file atomicity guarantees; no journal/recovery subsystem is added.
- Implementation is split into seven focused phases/commits followed by full acceptance.


## Refinement Contract-Surface Map

| Surface | Status | Refinement finding |
|---|---|---|
| Scope and identity | covered | Five standalone tools only; no Hub execution surface expansion. |
| Inputs and outputs | covered | Core schemas, bounds, defaults, and compatibility are frozen. |
| Lifecycle and concurrency | covered | Per-target locks, sorted batch lock acquisition, preflight/stage/confirm/revalidate/commit ordering are explicit. |
| Failure behavior | covered | Atomic no-replace create and guarded rollback behavior are frozen. |
| Data and persistence | covered | Text-only, exact revision, per-file atomic rename, no crash journal/recovery, best-effort audit. |
| Security and trust | covered | Direct file I/O relies on pathPolicy; confirmation default, canonical-target symlink containment, and narrow reserved paths are confirmed. |
| Operations | covered | Single/batch input, scan, candidate, original-state, response, and config-read bounds are explicit. |
| Surface parity | covered | Normal 23 / Room 35; Hub surfaces unchanged; legacy config/protocol scalar overrides remain accepted. |
| Verification | covered | Unit, integration, migration, failure injection, live smoke, surface count/schema, full workspace, and diff checks mapped. |

## Refinement Evidence and Corrections

### D-09 — Surface revision
Hashing only tool names would fail to detect a stale connector whose tool names are unchanged but whose input schema or annotations differ. The revision must cover canonical names, input schemas, annotations, and an explicit surface schema version.

### D-10 — Atomic create
`std::fs::rename` replaces an existing destination on common platforms. A precheck followed by ordinary rename would violate `expectedAbsent` if another process creates the target in between. Creation needs an atomic no-replace primitive, with a typed unsupported/failure path rather than silent overwrite.

### D-11 — Audit result
All existing command/MCP/tmux/session audit callers intentionally discard `write_audit` errors. File audit should preserve this operational convention, remain redacted, and report failure separately instead of changing a completed filesystem outcome.

### Q-02 — Configured-root versus descendant symlinks
Configured roots are already expanded/canonicalized. Rejecting every component including the configured root would accidentally disable a deliberately symlinked workspace root. The strict recommendation therefore permits root canonicalization but rejects symlinks beneath that canonical boundary.

### Q-03 — Rollback safety and memory
The first draft incorrectly implied the candidate-byte cap also bounded original rollback bytes. Original and candidate bytes require separate aggregate bounds. A rollback must additionally compare the current target with this batch's post-revision before restoring; otherwise it could overwrite an external writer that changed the file after Agentic committed it.

### Q-04 — Runtime-owned paths
The workspace currently contains `.agentic-gpt-audit.jsonl` plus optional diary, notebook, bootstrap, skills, and skill-install state. A broad reserved-directory rule could block legitimate user directories because `workspaceRoot` may itself be a project collection. The narrow recommendation protects only the root audit file and file-tool private temp names, leaving broader stores to explicit pathPolicy choices.

## Remaining Risks and Unknowns
- Exact direct dependency choices for ignore walking, regex/glob matching, diff rendering, and unified patch application remain implementation discretion after API/license review.
- External non-cooperating writers cannot be fully excluded from the final overwrite race; the contract intentionally avoids claiming filesystem compare-and-swap.
- Ntfy remote delivery health remains unknown to Agent until Hub explicitly advertises it; V1 reports only local relay availability.

## Implementation Discovery: Search Dependencies and Read Core
- Direct search dependencies are pinned to the lock-resolved releases: `ignore = 0.4.31`, `globset = 0.4.19`, and `regex = 1.13.1`; all are used in-process and no external `rg`/shell command is invoked.
- The shared file core canonicalizes existing targets (including symlinks) and checks resolved targets against normalized deny/write/read-only roots, matching existing pathPolicy precedence where a write root wins after deny checks.
- Metadata-only reads of small binary files return bounded metadata without a revision; content/search/edit remain UTF-8-only. Files above 8 MiB retain metadata but are rejected for content.
- The existing MCP schema budget needed a finite increase for the three newly frozen read/info/search descriptors; it now asserts 32 KiB Normal / 48 KiB Room serialized caps and 16/24 KiB aggregate input-schema caps, rather than dropping any fields or leaving schema growth unbounded.


## Decision Rationale: Refinement Round 2

### D-12 — Confirmation default
The user selected Q-01A. File mutation remains a destructive MCP surface, but normal calls do not require human confirmation unless `needConfirm: true` is supplied. This keeps routine code editing practical while preserving explicit escalation for sensitive changes.

### D-13 — Existing pathPolicy symlink semantics
The user asked to follow existing pathPolicy. Existing path checks canonicalize existing targets and compare the resolved path with canonical allowed roots. The file tools therefore follow symlinks only when the resolved target remains inside an allowed root. Mutation code must repeat containment and target-state validation immediately before commit; a symlink resolving outside the allowed roots is rejected.

### D-14 — Guarded rollback
The user selected Q-03A. Rollback is attempted only when the target still has the exact post-revision written by this batch. A later external change converts the outcome to `partial_failed` rather than being overwritten.

### D-15 — Narrow reserved paths
The user selected Q-04A. The workspace-root audit file and private file-tool temp names are reserved. Diary, notebook, bootstrap, skills, and state paths are not implicitly hidden because they may be legitimate model work targets and remain governed by explicit pathPolicy.


## Final Contract Acceptance
- The user accepted the consolidated D-01 through D-15 contract and authorized implementation.
- Exact entry phase is Implementation Phase A — Confirmation semantics.
- Refinement is complete; implementation proceeds with `planning-with-files` only.
