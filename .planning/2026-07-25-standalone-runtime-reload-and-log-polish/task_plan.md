# Task Plan: Standalone Runtime Reload and Log Polish

## Goal
Make standalone policy/limit configuration changes take effect without restarting, introduce an adaptive `maxActiveSessions: "auto"` default with useful capacity diagnostics, and make local journal output concise and correctly leveled without weakening audit or execution evidence.

## Workflow State
- **Stage:** implementation_ready
- **Current role:** repair implementer
- **Implementation authorized:** yes
- **Active plan:** `2026-07-25-standalone-runtime-reload-and-log-polish`
- **Current phase:** Phase 5 — Focused Verification Repair (complete)
- **Entry phase after handoff:** none
- **Open blocking decisions:** none
- **Design checkpoint:** `d1a0028` independently reviewed and superseded for delivery
- **Next action:** none; D-13 through D-16 are repaired and independently verified

## Scope and Constraints

### In scope
- Make standalone hidden workers live-reload `policy`, `pathPolicy`, and `limits` from the active config.
- Keep the last valid live configuration when a reload fails.
- Ensure policy/limit changes affect new admissions only; never retroactively cancel an already admitted session.
- Accept `limits.maxActiveSessions` as either a JSON integer or the string `"auto"`.
- Make new default configs serialize `"maxActiveSessions": "auto"`.
- Resolve `auto` from process-visible parallelism as `clamp(ceil(parallelism × 1.5), 6, 24)`.
- Preserve existing numeric values as explicit overrides; do not migrate an existing `4` to `auto`.
- Improve capacity rejection diagnostics while preserving the stable `max_active_sessions_reached` error code and atomic all-or-reject batch admission.
- Clean up human-facing standalone journal logs: correct forwarded severity, remove duplicate timestamps under journald, remove routine lifecycle duplication, and shorten human-facing run/session identifiers.
- Preserve full identifiers and complete bounded evidence in audit records, Hub reports, MCP results, and tunnel-client structured log files.

### Out of scope
- `StartLimitIntervalSec` or any systemd unit change; it will be handled manually outside this plan.
- Changing exact program matching in policy rules. `python`, `python3`, `bash`, and `/usr/bin/bash` remain distinct configured program strings.
- Adding basename/path alias matching, wildcard rules, or a new policy language.
- Changing the 18/30 standalone tool surfaces, Hub full/coordinator surfaces, schemas, result envelopes, or confirmation semantics.
- Queueing oversized batches or automatically serializing them into waves. Batch admission remains concurrent and all-or-reject.
- Logging command arguments, working directories, stdout/stderr, secrets, or full result bodies to the human journal.
- Adding a raw log retrieval API, durable session history, a new tracing framework, or changes to the 24-hour Hub run retention model.

## Frozen Runtime Configuration Contract

### Live-reload subset
The standalone hidden worker must apply these config sections to subsequent tool admissions without service restart:
- `policy`
- `pathPolicy`
- `limits`

Reload semantics:
- Detect a valid on-disk change within the existing approximately two-second polling window or an equivalent bounded mechanism.
- Atomically replace the live subset so a call sees either the old complete subset or the new complete subset.
- Invalid JSON, invalid typed values, or failed validation retain the previous live subset and emit one bounded warning per observed failed version.
- New policy and path rules apply only to calls admitted after the swap.
- Lowering a limit below the current active count does not kill existing work; new admissions remain blocked until active usage falls below the resolved limit.

### Restart-required subset
Standalone startup identity remains immutable for the current process tree. At minimum, changes to these values require restart and must not silently alter the running worker:
- CLI capability profile
- `agentId`
- `workspaceRoot`
- Tunnel id, API-key reference, client executable/download/cache/version/hash settings
- Hub-reporting enablement/connection identity
- Skill-install manager concurrency captured during worker construction

The outer supervisor remains the canonical owner of `restart_required` diagnostics. Implementation may expand its private identity snapshot to cover the listed fields. The worker must not emit duplicate restart warnings for the same config version.

Other config fields retain their existing behavior unless repository evidence proves they are already safely live and adding them cannot broaden this plan's observable contract.

## Frozen `maxActiveSessions` Contract

Public JSON accepts:

```json
"maxActiveSessions": "auto"
```

or an existing numeric override:

```json
"maxActiveSessions": 12
```

Rules:
- New default config writes `"auto"`.
- Existing numeric config remains numeric through load/write round trips and is honored exactly; no migration or backup rewrite changes it to `auto`.
- `auto` uses `std::thread::available_parallelism()` or an equivalent process-visible CPU quota source.
- Resolution formula: `clamp(ceil(parallelism × 1.5), 6, 24)`.
- If parallelism detection fails, resolve to the minimum `6` and emit at most one bounded diagnostic for that resolution event.
- Representative outcomes: 1→6, 4→6, 8→12, 12→18, 16→24, 20→24.
- Re-resolve `auto` on startup and each valid limits reload.
- Startup/config-reload diagnostics expose both the configured mode and resolved value, for example `maxActiveSessions=auto; resolvedMaxActiveSessions=24`.
- The resolved value, not the enum/string form, is used by every single-session, batch, and skill-process admission path.

### Capacity rejection
- Preserve the stable leading code `max_active_sessions_reached`.
- Include bounded deterministic context: `active`, `requested`, and `limit`.
- Single-session admission uses `requested=1`; batch admission uses the full batch element count.
- A batch larger than the limit is rejected before id allocation or process spawn, even when no session is active.
- No queue, partial admission, wave scheduling, or implicit retry is introduced.

## Frozen Human Log Contract

### Journal timestamp behavior
- When running under journald/systemd, Agentic omits its own RFC3339 prefix so journal output contains one timestamp.
- Foreground/non-journal execution retains an Agentic timestamp so standalone terminal logs remain self-contained.
- Detection may use `JOURNAL_STREAM`, `INVOCATION_ID`, or an equivalent reliable runtime signal.

### Forwarded child severity
- Parse known Agentic child lines shaped as `<timestamp> <LEVEL> <message>`.
- Strip the child timestamp and re-emit the message at its original `INFO`, `WARN`, or `ERROR` severity with a bounded component marker.
- Unknown/unparseable child stdout is informational; unknown/unparseable child stderr is warning-level.
- Secret and worker-token redaction remains mandatory before parsing or forwarding.
- Do not change the tunnel-client structured file log format or contents.

### Lifecycle deduplication
Human-facing journal output follows these observable rules:
- A routine tool call that finishes inline emits one final human lifecycle record, not separate `started`, `managed_session exited`, and `completed` records.
- A managed call returned while still active emits one tool-response record identifying the active session, then one later terminal record.
- A failure before session creation emits one failed tool record.
- Confirmation waiting, cancellation, kill, spawn failure, and later terminal failure remain observable without duplicate terminal lines.
- Full machine evidence remains unchanged in audit, Hub reporting, MCP responses, and session inspection.

### Identifier presentation and safety
- Human journal records use a deterministic 12-hex-character body for run/session ids, with clear `run=` / `session=` labels.
- Full identifiers remain unchanged in every machine-facing record and API/result.
- Human logs continue to exclude argv, cwd, stdout/stderr, secrets, and result payloads.
- Bounded error codes, duration, exit code, source/profile, and terminal state remain available where applicable.

## Decisions Made

| ID | Area | Status | Outcome | Rationale |
|---|---|---|---|---|
| D-01 | Plan boundary | confirmed | Use a new lightweight plan rather than reopening the delivered standalone surface-compaction plan. | These are post-delivery operational improvements, not conformance repairs to the frozen 18/30 contract. |
| D-02 | Live config | confirmed | Standalone live-reloads `policy`, `pathPolicy`, and `limits`; admitted work is never retroactively changed. | Closes the observed `config allow add` issue without pretending all startup-owned managers are dynamically replaceable. |
| D-03 | Restart ownership | confirmed | The supervisor owns restart-required diagnostics for the frozen startup subset; the worker avoids duplicate notices. | Keeps one operational authority for the supervised process tree. |
| D-04 | Adaptive limit schema | confirmed | `maxActiveSessions` accepts integer or `"auto"`; new defaults use `auto`, existing integers remain explicit. | Provides device-aware defaults without silently changing existing deployments. |
| D-05 | Adaptive formula | confirmed | `auto = clamp(ceil(availableParallelism × 1.5), 6, 24)`, failure fallback 6. | Scales across devices while retaining a bounded safety ceiling. |
| D-06 | Capacity semantics | confirmed | Keep atomic all-or-reject admission and stable code; add `active/requested/limit` diagnostics. | Improves diagnosis without turning batch execution into a queue. |
| D-07 | Journal timestamps | confirmed | Omit Agentic timestamps under journald, retain them in foreground mode. | Removes duplicated timestamps without degrading standalone terminal logs. |
| D-08 | Forwarded severity | confirmed | Preserve parseable child severity instead of wrapping every stderr line as WARN. | Eliminates false alarms while retaining warnings for unknown stderr. |
| D-09 | Lifecycle logs | confirmed | Inline-terminal calls produce one human completion record; active calls retain one response record plus one later terminal record. | Reduces visual noise while preserving meaningful asynchronous lifecycle evidence. |
| D-10 | Human ids | confirmed | Display 12-character ids in human logs; preserve full ids in machine-facing evidence. | Keeps correlation practical in narrow terminals without changing public identifiers. |
| D-11 | Safety boundary | confirmed | Do not log commands/output and do not change exact policy program matching. | Prevents a polish task from weakening the existing security boundary. |
| D-12 | Systemd warning | confirmed | Exclude `StartLimitIntervalSec` from this plan. | User will repair the unit manually. |
| D-13 | Forwarded journal grammar | confirmed | The supervisor accepts both timestamped child records and journald-mode records beginning directly with `INFO`, `WARN`, or `ERROR`; parsing remains redaction-first and unknown stderr remains WARN. | The hidden worker inherits journal environment and legitimately omits its inner timestamp. |
| D-14 | Restart comparison baseline | confirmed | Restart-required detection always compares disk config against immutable runtime startup identity; a separate last-observed/last-warned state provides bounded diagnostics. | Updating the comparison baseline to an unapplied disk value creates false restart warnings when config returns to the running value. |
| D-15 | Human terminal ordering | confirmed | Response/terminal coordination is one linearizable state machine: no terminal record may be lost, and an active response record must become visible before its one later terminal record. | Independent review found a check/clear/enqueue race and a possible terminal-before-active ordering. |
| D-16 | Invalid-config warning ownership | confirmed | The supervisor owns the human invalid-config warning for supervised standalone operation; the hidden worker keeps last-good state without emitting a duplicate human warning for the same failed version. | One bounded operator-facing warning is sufficient while preserving last-good behavior. |

## Phases

### Phase 1: Discovery and Contract Refinement
**Objective:** Reproduce the observed behavior, inspect the relevant config/session/log paths, and freeze a bounded implementation contract.

- [x] Confirm `config allow add` writes correctly but the standalone hidden worker does not start `watch_config`.
- [x] Confirm the supervisor only compares Tunnel startup identity today.
- [x] Confirm the current numeric default is `maxActiveSessions=4` and batch admission is atomic.
- [x] Confirm the observed capacity rejection came from requesting five batch elements against limit four, not a session leak.
- [x] Confirm `forward_log` wraps every child line with `log_warn` and `log_line` always adds its own timestamp.
- [x] Freeze D-01 through D-12 and pass the handoff readiness gate.
- **Status:** complete

### Phase 2: Live Runtime Configuration and Adaptive Limits
**Objective / visible outcome:** Policy/path/limit config edits become effective for new standalone calls without restart, and new installations receive a device-aware session ceiling.

**Primary areas:**
- `crates/agentic-gpt/src/config.rs`
- `crates/agentic-gpt/src/main.rs`
- `crates/agentic-gpt/src/supervisor.rs`
- `crates/agentic-gpt/src/sessions.rs`
- focused config/runtime/session tests

**Work:**
1. Add a backward-compatible integer-or-`auto` config representation and one canonical resolution helper.
2. Change only new default serialization to `auto`; preserve numeric round trips and unrelated extra fields.
3. Route every admission check through the resolved limit and add deterministic capacity detail.
4. Introduce standalone live-subset reload with atomic swap and last-good fallback.
5. Expand restart-required identity/diagnostics for the frozen startup subset without duplicate worker warnings.
6. Log configured and resolved limit safely at startup/reload.

**Required tests:**
- Integer and `auto` deserialize/serialize round trips, including old config fixtures.
- Formula boundary table and detection-failure fallback.
- New default writes `auto`; an old explicit `4` remains `4` after a config mutation.
- Policy add/remove and path/limit edits affect a running hidden worker without restart.
- Invalid reload keeps previous behavior.
- Lowering below active usage does not kill sessions and blocks only new admissions.
- Single and batch capacity errors include active/requested/limit and retain the stable code.
- Oversized batch creates no ids or sessions.
- Restart-required fields do not mutate the live worker and generate one supervisor diagnostic.

**Completion boundary:** Direct hidden-worker probes prove no-restart policy/limit behavior and all existing Hub/standalone execution tests remain green.

**Commit:** `feat(agent): reload standalone runtime limits`
- **Status:** complete

### Phase 3: Human-Facing Standalone Log Compaction
**Objective / visible outcome:** `journalctl --user -fu agentic-gpt.service` shows correctly leveled, single-timestamp, non-duplicated lifecycle records while machine evidence remains intact.

**Primary areas:**
- `crates/agentic-gpt/src/utils.rs`
- `crates/agentic-gpt/src/supervisor.rs`
- `crates/agentic-gpt/src/stdio_server.rs`
- managed terminal-hook integration and focused log fixtures/tests

**Work:**
1. Make timestamp rendering journal-aware while preserving foreground timestamps.
2. Add bounded child-line parsing and severity-preserving forwarding after redaction.
3. Compact run/session ids only in human records.
4. Deduplicate inline tool/session terminal records according to D-09.
5. Preserve asynchronous terminal, confirmation, cancellation, kill, spawn-failure, duration, exit, source/profile, and bounded error evidence.
6. Leave audit, Hub reports, MCP results, session tails, and tunnel-client file logs unchanged.

**Required tests:**
- Journal mode has no inner timestamp; foreground mode retains one.
- INFO child line remains INFO, WARN remains WARN, and unknown stderr remains WARN.
- Secrets and worker tokens are redacted before output and parsing.
- Inline-terminal process call produces exactly one human lifecycle completion line.
- Running-then-terminal call produces exactly two meaningful records.
- Failure before allocation, cancellation, kill, and spawn failure remain observable once.
- Human ids are 12 characters while audit/report/result ids remain full.
- Sentinel argv, cwd, stdout/stderr, secrets, and result text never appear in human logs.

**Completion boundary:** Unit fixtures and a real supervised journal probe match the frozen human log contract with no protocol stdout contamination.

**Commit:** `refactor(agent): compact standalone lifecycle logs`
- **Status:** complete

### Phase 4: Integrated Verification and Delivery
**Objective / visible outcome:** Prove the runtime behavior on a real supervised standalone deployment and deliver without surface regressions.

**Verification:**
- `cargo fmt --all -- --check`
- `git diff --check`
- focused Agent config/session/log tests
- isolated `cargo check --workspace`
- isolated `cargo test --workspace`
- hidden stdio Normal and Room initialize/list/call probes; exact 18/30 surfaces unchanged
- live config probe: add and remove a policy rule without restarting the service
- live limit probe: switch explicit/auto and verify resolved admission behavior without killing existing work
- journal probe: one timestamp, correct severity, compact ids, inline deduplication, no sensitive fields
- final Git status and product-diff inspection

**Completion boundary:** All acceptance criteria pass, planning files record evidence, and the worktree is clean after focused commits.

**Commit:** planning/delivery record only when needed after verification
- **Status:** complete

### Phase 5: Focused Verification Repair
**Objective / visible outcome:** Close the independent-review gaps without changing the frozen config, capacity, tool-surface, safety, or machine-evidence contracts.

**Primary areas:**
- `crates/agentic-gpt/src/supervisor.rs`
- `crates/agentic-gpt/src/stdio_server.rs`
- focused supervisor/journal/lifecycle integration tests
- planning evidence only outside the focused product changes

**Work:**
1. Extend redaction-first child-log parsing to accept journald-mode records whose first token is `INFO`, `WARN`, or `ERROR`, while retaining timestamped child parsing and fallback severity rules.
2. Add a real supervised probe in which the hidden worker inherits journald environment; assert its normal INFO records never become outer WARN records.
3. Keep the runtime startup identity immutable for the process lifetime; separate observed-file version and warning-dedup state so change-away warns once, repeated observation is quiet, and change-back-to-runtime is quiet.
4. Replace the split-atomic `HumanTerminalTracker` coordination with one synchronized/linearizable state transition covering response visibility, pending terminal storage, and flush ordering.
5. Guarantee cardinality and order for both races: inline terminal produces one completion record; returned-active produces active first and exactly one later terminal record, even when exit races the response boundary.
6. Make invalid-config human warning ownership supervisor-only in supervised mode, while the worker retains the last valid live subset and no machine-facing evidence is weakened.
7. Preserve all exclusions: no `agent.info`, no public tool/schema changes, no policy matching change, no systemd unit change, no Hub/protocol change.

**Required tests:**
- Timestamped child INFO/WARN/ERROR parsing remains correct.
- Untimestamped journald child INFO/WARN/ERROR parsing preserves severity after redaction.
- A real hidden worker inheriting `JOURNAL_STREAM` emits no `WARN tunnel.stderr: INFO ...` record.
- Runtime identity sequence `R → A → A → R` yields exactly one restart-required warning, only for `A`; the live process identity remains `R` throughout.
- Invalid config observed by the supervised tree yields one bounded human warning for that failed version and preserves last-good live behavior.
- Deterministic concurrency tests force terminal-before-response, response-before-terminal, and the former check/clear/enqueue interleaving; none loses or duplicates a record.
- Active-response ordering asserts `status=active` is emitted before its terminal record.
- Existing inline, active, failure, cancellation, kill, spawn-failure, redaction, compact-id, and full machine-evidence tests remain green.

**Verification:**
- focused supervisor/parser/restart-identity/tracker tests
- real supervised Normal and Room journal probes
- `cargo fmt --all -- --check`
- `git diff --check`
- isolated `cargo check --workspace`
- isolated `cargo test --workspace`
- final product-diff inspection proving no Hub/protocol/systemd/public-surface changes

**Completion boundary:** The four independent-review blockers are directly covered by failing-before/fixed-after regression evidence; all original acceptance criteria still pass; a fresh reviewer approves delivery.

**Commit:** `fix(agent): harden standalone runtime log ordering`
- **Status:** complete

## Acceptance Criteria
1. A running standalone hidden worker applies policy additions/removals, path-policy changes, and limits changes without service restart within a bounded reload interval.
2. Invalid reload content preserves the previous live configuration and never partially applies a subset.
3. New sessions use the new live config; existing admitted sessions are not killed or re-evaluated.
4. New default config serializes `maxActiveSessions: "auto"`; existing integer values remain explicit and round-trip unchanged.
5. `auto` resolves exactly by D-05 from process-visible parallelism and is used by all process/batch/skill admissions.
6. Capacity rejection preserves `max_active_sessions_reached`, reports active/requested/limit, and creates no partial batch state.
7. Standalone startup-owned fields remain unchanged until restart and produce one bounded supervisor-owned restart-required diagnostic.
8. Journal output contains only one timestamp per line and does not misclassify parseable INFO child records as WARN.
9. Inline-terminal tool execution produces one human lifecycle completion record; asynchronous execution retains one response record and one later terminal record.
10. Human lifecycle ids are compact while machine-facing ids remain complete and unchanged.
11. Human logs contain no command arguments, working directories, stdout/stderr, secrets, or result payloads.
12. Exact 18/30 Tunnel surfaces, Hub behavior, policy exact-match semantics, confirmation behavior, audit, Hub reporting, and 64 KiB session-tail bounds remain compatible.
13. `StartLimitIntervalSec` and other systemd unit changes are absent from the product/planning diff for this plan.
14. Formatting, focused tests, isolated workspace tests, real hidden-worker probes, supervised journal probes, and clean Git status pass before delivery.
15. A journald-mode hidden worker may emit untimestamped leveled records, and the supervisor preserves their INFO/WARN/ERROR severity without false outer WARN classification.
16. Restart-required diagnostics compare against immutable runtime identity: a change away warns once, repetition is quiet, and returning disk config to the running value emits no warning.
17. Human response/terminal coordination is linearizable: terminal records are never lost or duplicated, and an active response is observable before its later terminal record.
18. A failed config version produces one bounded supervisor-owned human warning in supervised mode while the worker retains the complete last-good live subset.
19. Focused race/production-path probes and the complete isolated workspace suite pass before the plan returns to delivered.

## Implementation Discretion
The Implementer may choose:
- the private enum/type used for integer-or-auto values;
- the internal formula helper and CPU-detection injection seam for tests;
- polling versus another bounded file-change detector, provided reload semantics remain atomic and approximately as responsive as today;
- how to snapshot and compare restart-required fields privately;
- the internal deduplication mechanism for inline terminal events;
- exact human key ordering and component labels;
- whether ERROR receives a dedicated helper or maps through the generic log-line primitive.

Implementation discretion may not change the public JSON forms, auto formula/clamps, live/restart field ownership, stable error code, batch admission semantics, log cardinality, safety exclusions, or machine-facing evidence.

## Readiness Gate
- [x] Goal, scope, non-goals, and ownership boundary are explicit.
- [x] Repository evidence identifies config loading, worker construction, supervisor identity watching, session admission, and log forwarding.
- [x] No blocking question remains open.
- [x] D-01 through D-12 reflect choices confirmed in the conversation.
- [x] Public config forms, defaults, migration compatibility, reload timing, concurrency, and failure behavior are frozen.
- [x] Security and observability boundaries are explicit.
- [x] Every requirement maps to a phase and acceptance criterion.
- [x] Phase dependencies and Phase 2 entry are clear.
- [x] Implementation discretion cannot change observable behavior.
- [x] All three planning files agree on maturity and blockers.
- [x] No product code, tests, configuration, generated artifacts, or systemd unit was changed during refinement.
- **N/A:** database migration, network API versioning, durable recovery, or rollback; this plan changes only local config interpretation, live in-memory state, and human log rendering.

## Implementation Handoff
- **Plan maturity:** delivered after Phase 5 repair
- **Design phase:** complete; independent verification repair appended
- **Implementation authorized:** yes
- **Entry phase:** none
- **Frozen decisions:** D-01 through D-16
- **Open blocking decisions:** none
- **Implementation discretion:** see `Implementation Discretion`; Phase 5 may choose private synchronization/parser structures but may not change observable contracts
- **Verification convention:** direct regression tests for all four blockers, real supervised journal probes, isolated workspace suite, and fresh independent review
- **Commit convention:** one focused Phase 5 product commit, followed by planning-only acceptance evidence if verification passes
- **Design checkpoint:** `d1a0028` is the reviewed implementation baseline; delivery claim superseded pending Phase 5
- **Next invocation:** none; do not push, deploy, tag, or create a release

## Errors Encountered
| Error | Attempt | Resolution |
|---|---:|---|
| Tool discovery exposed a stale wrapper schema requiring `agentId`, while the refreshed standalone adapter rejects it. | 1 | Retried against the actual compact adapter without `agentId`; discovery and planning continued. |
| `init-session.sh` was not executable when invoked directly. | 1 | Re-ran it explicitly through `bash`; plan initialization succeeded. |
