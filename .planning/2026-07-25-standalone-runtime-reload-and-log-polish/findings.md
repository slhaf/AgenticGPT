# Findings & Decisions: Standalone Runtime Reload and Log Polish

## Requirements
- Policy rules added through `agentic-gpt config allow add` must affect the running standalone connector without a service restart.
- The default active-session ceiling should scale with the device rather than remain fixed at four.
- `maxActiveSessions` should visibly support `"auto"`, while old numeric configuration remains explicit.
- Capacity failures should explain whether the active count or requested batch size exceeded the limit.
- Local journal output should be readable and correctly leveled without losing machine-facing evidence.
- The systemd `StartLimitIntervalSec` warning is explicitly excluded and will be repaired manually.

## Existing Implementation

### Config write and reload
- `policy::mutate_rule` loads the current config, appends/removes a rule, and calls `write_config_with_backup`; the CLI write path is functioning.
- Current installed config contains an explicit numeric `limits.maxActiveSessions=4` and a broad `bash` allow rule.
- `main.rs::run` starts `watch_config(state.clone())` for Hub-command runtimes.
- `main.rs::run_stdio_worker` loads the config once, constructs state/managers, and enters `stdio_server::serve` without starting `watch_config`.
- This explains why rules written during standalone execution did not affect the hidden worker until the service restarted.
- Existing `watch_config` replaces the complete `Config` in `state.config`; some managers, such as skill-install concurrency, are separately constructed and therefore cannot honestly hot-reload through a plain config swap.

### Supervisor identity
- `supervisor.rs::StartupIdentity` currently includes Tunnel id, API-key reference, client source/cache/version/hash, and profile.
- `watch_startup_identity` polls the config and emits `restart_required` only when that limited identity changes.
- Workspace, agent/reporting identity, and manager-construction inputs are not currently part of the restart snapshot.

### Session capacity
- `LimitsConfig.max_active_sessions` is currently a plain `usize` with default `4`.
- Single-session and skill admission count active states; completed sessions do not consume slots.
- Managed batch admission refreshes terminal states under the session lock, then atomically checks `active + batch_size > limit` before inserting any starting records.
- The observed rejection was normal admission behavior: a five-element batch was submitted against limit four while active count was zero.
- Existing error text exposes only `max_active_sessions_reached`, making an oversized request look like an existing-session leak.

### Logging path
- `utils::log_line` always prepends an RFC3339 timestamp and level to stderr.
- systemd/journald adds its own timestamp, creating duplicate time information.
- `supervisor.rs::forward_log` redacts child output but forwards every stdout/stderr line through `log_warn`, so parseable child INFO records appear as outer WARN records.
- `stdio_server` currently logs a `stdio_tool ... started` record, a managed terminal record, and a `stdio_tool ... completed` record for a short inline process call.
- Full run/session ids and repeated source/profile fields cause narrow-terminal wrapping.
- Existing terminal records correctly avoid argv, paths, process output, and secrets; that safety behavior must remain.

### Phase 2 implementation reconnaissance
- `AppState.config` is an `Arc<RwLock<Config>>`; existing Hub `watch_config` replaces the whole config, while `run_stdio_worker` starts no watcher. A standalone worker watcher can update `policy`, `path_policy`, and `limits` under the same write lock, preserving an atomic complete live subset and leaving startup-owned fields untouched.
- `Config::load` is the single typed JSON boundary and preserves flattened extra fields; changing `LimitsConfig.max_active_sessions` to a custom integer-or-`auto` value preserves explicit numeric serialization while allowing the new default to emit `auto`.
- Session admission has three relevant checks: synchronous `start_session`, async single/skill registration, and atomic `start_prepared_managed_batch`. The latter already rejects before inserting sessions, but the stdio batch handler currently allocates per-element session IDs before invoking it; capacity preflight must move ahead of that allocation.
- Capacity rejection is currently a bare `max_active_sessions_reached` string. A bounded helper should append deterministic `active`, `requested`, and `limit` fields while keeping the stable code as the leading token.
- Supervisor `StartupIdentity` already owns tunnel/client/profile comparisons. It needs the frozen `agentId`, `workspaceRoot`, reporting connection identity, and skill-install concurrency fields; its poller should advance the observed file version on failed loads so one broken version produces one diagnostic rather than a warning every two seconds.

### Phase 2 validation discovery
- The full `agentic-gpt` unit suite passed, but the two real standalone supervisor integration tests initially failed before launching the worker because the sandbox denied creation of the host runtime directory under `~/.agentic_gpt/runtime/tunnel`; the reported product error was `runtime_directory_unavailable`. This is an environment permission issue and needs one controlled escalated rerun.
- Controlled rerun of the two supervisor integration tests passed. The direct hidden-worker live-reload probe also passed for policy, path policy, limits, invalid reload fallback, and preservation of an already active session.

### Phase 3 logging reconnaissance
- `utils::log_line` is the single stderr rendering primitive; adding an injectable formatter there can omit the Agentic RFC3339 prefix when `JOURNAL_STREAM` or `INVOCATION_ID` is present while retaining foreground timestamps.
- `supervisor::forward_log` currently redacts but sends every child line through `log_warn`. A redaction-first parser can recognize RFC3339 + `INFO`/`WARN`/`ERROR`, strip the child timestamp, and route the original level; stdout unknown lines should use INFO and stderr unknown lines WARN.
- `stdio_server::call` currently emits `started`, then the managed terminal hook may emit `managed_session`, then emits `completed`; the hook is invoked from `sessions::finalize_session` after audit/reporting, so a per-call tracker can suppress inline terminal hooks while allowing post-response asynchronous terminal records.
- Human lifecycle IDs are currently printed as `runId`/`sessionId` with full UUID bodies. Machine reporting/audit paths receive the full IDs separately, so compacting only the human formatter is safe. `stdio_server::dispatch` is widely used by tests; lifecycle-aware dispatch should be an internal wrapper so existing direct dispatch callers keep their API.
- The direct hidden-worker probe now captures stderr and confirms inline calls emit no `status=started` or duplicate `managed_session` line, active calls emit `status=active` plus one later terminal record, and human run/session labels use 12-hex bodies without exposing full machine IDs.
- The supervised fake tunnel emits parseable INFO/WARN/ERROR and unknown stdout/stderr lines; the real supervisor probe confirms redaction-first parsing, component prefixes, severity preservation, child timestamp stripping, and journal-mode omission of Agentic's inner timestamp.

## Contract Gaps
- No standalone live-config watcher exists.
- A whole-config swap would create misleading partial hot-reload semantics because startup-owned managers do not change.
- Config cannot express an adaptive active-session default.
- Existing numeric defaults embedded in user configs prevent a simple code-default increase from changing deployed behavior.
- Capacity errors lack active/requested/limit context.
- Journal rendering has false WARN severity, nested timestamps, and duplicate lifecycle events.

## Options and Tradeoffs

### Live-reload scope
- Reloading the entire `Config` is mechanically simple but implies that workspace, reporting, skill concurrency, and other constructed resources changed when they did not.
- Reloading only `policy`, `pathPolicy`, and `limits` closes the observed operational gap with an honest and testable contract.
- Rebuilding every manager dynamically would broaden the task and add failure/cancellation semantics not requested here.

### Adaptive session limit
- Fixed 8 is simple but ignores large and small device differences.
- Unbounded `parallelism × 1.5` would allow 30 sessions on the current 20-thread laptop and can amplify resource-heavy child processes.
- The confirmed 6–24 clamp provides useful scaling while preserving a hard safety ceiling.
- String `"auto"` makes user intent visible; a nullable field would be less clear in config diagnostics.

### Log deduplication
- Removing terminal records entirely would hide asynchronous completion.
- Keeping all three current records preserves detail but creates excessive human noise.
- Distinguishing inline-terminal from returned-active calls retains two-stage evidence only where time actually separates the stages.

### Timestamp handling
- Removing all internal timestamps harms foreground CLI diagnostics.
- Journal-aware omission preserves one timestamp in services and a self-contained timestamp outside journald.

## Decision Rationale

### D-02 / D-03 — Honest runtime ownership
Only values consumed from `state.config` on each admission are in the guaranteed live subset. Manager and process-tree identity changes remain restart-owned. This prevents a config log from claiming a value changed when the active object still uses its startup value.

### D-04 / D-05 — Explicit adaptive default
The enum-like public form separates an adaptive default from explicit operator intent. Existing numeric files remain stable, while new installations adapt to process-visible CPU quota. The upper clamp recognizes that a child process can itself use many cores.

### D-06 — Stable code plus detail
Keeping `max_active_sessions_reached` avoids breaking tests/callers and bounded error-code extraction. Adding deterministic detail resolves the practical ambiguity seen during this session.

### D-07 through D-10 — Human versus machine evidence
The journal is a human operational surface; audit, Hub reporting, MCP result bodies, and session inspection are machine evidence. Compacting only the human representation reduces noise without weakening traceability or changing public identifiers.

### D-11 — Preserve policy security semantics
Program matching is currently exact and structural. Converting rules to basename matching could cause an allow rule to cover paths the user did not intend. Documentation/diagnostics can explain the actual program string; matching changes require a separate security design.

### D-12 — Systemd exclusion
The unit-file warning is independent of Agentic runtime/config/log contracts and the user explicitly chose a manual repair. It must not appear as hidden implementation work.

## Risks and Unknowns
- Live reload and a simultaneous call must not expose a partially updated policy/path/limits combination; an atomic snapshot or one config write lock is required.
- A changed config may combine live and restart-required fields. Live fields should apply, while the supervisor emits one restart diagnostic for the immutable subset.
- Deduplicating terminal logs has race potential when a process exits near the inline wait boundary. Tests must force both orderings and assert cardinality, not only message text.
- Journald environment detection must be testable and must not rely on a Linux-only global state without injection/seaming.
- Human short-id collisions are possible in theory; they are presentation-only and full ids remain available in every authoritative record.

## Relevant Locations
- `crates/agentic-gpt/src/config.rs`: config schema/default/load/write compatibility.
- `crates/agentic-gpt/src/main.rs`: Hub runtime watcher and hidden stdio worker construction.
- `crates/agentic-gpt/src/supervisor.rs`: restart identity, child process launch, and log forwarding.
- `crates/agentic-gpt/src/sessions.rs`: active-session counting, batch atomic admission, terminal refresh/finalization.
- `crates/agentic-gpt/src/stdio_server.rs`: tool lifecycle and managed terminal human logs.
- `crates/agentic-gpt/src/utils.rs`: timestamped stderr log primitive.
- `docs/standalone-runtime.md`: operational config/log documentation to update during implementation.

### Phase 4 verification setup
- Phase 2 and Phase 3 product commits are present on `main`; the final phase will validate the workspace and the controlled real-supervisor probes, then record only evidence/planning changes.
- The verification must explicitly inspect the product diff for systemd unit changes and `StartLimitIntervalSec`, because that surface is frozen out of scope.

### Phase 4 verification results
- Formatting, whitespace, workspace check, and final scope inspection passed. The product diff contains no systemd unit or `StartLimitIntervalSec` change.
- Controlled `cargo test --workspace` passed all workspace tests: Agent 158/158, Hub 61/61, Protocol 9/9, and standalone supervisor 4/4.
- The real standalone probes preserve Normal/Room MCP surfaces, live policy/path/limit behavior, invalid reload fallback, existing-session survival, journal severity/timestamp rules, compact human ids, lifecycle cardinality, confirmation behavior, and machine-side full evidence.
- The first unprivileged full-test attempt failed only because the sandbox could not create the host runtime tunnel directory; the controlled rerun passed without a product workaround.
