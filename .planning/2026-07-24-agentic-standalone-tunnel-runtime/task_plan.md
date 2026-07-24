# Task Plan: Agentic Standalone Tunnel Runtime

## Goal
Add a Tunnel-backed local command path to Agentic without removing the existing Hub-centric path. `agentic-gpt run-as-standalone` supervises the official OpenAI `tunnel-client`, which launches an internal Agentic stdio MCP worker. Normal and Room capability profiles are both supported. The Hub remains available as an optional best-effort reporting/aggregation plane and gains a bounded coordinator MCP profile for KMP-oriented status, history, and notification use.

## Workflow State
- **Stage:** implementation_complete
- **Current role:** implementer
- **Implementation authorized:** yes
- **Active plan:** `2026-07-24-agentic-standalone-tunnel-runtime`
- **Current phase:** Phase 10 - Real tunnel-client command/diagnostic repair (complete)
- **Entry phase after handoff:** Phase 3
- **Open blocking decisions:** none

## Scope

### In scope
- Public `agentic-gpt run-as-standalone [--profile normal|room] [--config ...]`; default profile is `normal`.
- Internal hidden stdio MCP worker launched only by the supervised tunnel-client topology.
- Agentic-managed official tunnel-client discovery, trusted download, checksum verification, cache installation, health observation, restart, and cleanup.
- Tunnel configuration under Agentic config: tunnel id, secret reference, client source/version/cache/download/checksum, and optional Hub reporting.
- Linux amd64 and Linux arm64 managed tunnel-client assets, matching Agentic's current release matrix.
- Shared local tool execution layer reused by Hub-command and stdio-MCP adapters.
- Tunnel Normal surface: process, managed sessions, tmux, downstream MCP bridge, skills, and workspace bootstrap.
- Tunnel Room surface: Tunnel Normal surface plus Room diary and notebook.
- Transport-neutral `bootstrap` and `bootstrap.read`; existing Hub `room.bootstrap*` names remain compatibility aliases.
- Canonical top-level `skills` config with read compatibility for legacy `room.skills`.
- Optional reporting-only Agent-to-Hub connection with online/heartbeat, direct-run lifecycle, and session snapshots.
- Configurable reporting detail `metadata | full`, default `metadata`; Hub reporting default disabled.
- Hub-native persisted direct-run records and bounded run queries.
- Full Hub MCP profile preserved as default; new coordinator profile exposes only bounded Hub-native aggregation and notification tools.
- Focused tests, migrations, documentation, packaging checks, stub smoke test, and real Secure MCP Tunnel end-to-end validation.

### Out of scope
- Native Rust implementation of the Secure MCP Tunnel wire/control-plane protocol.
- Removal or deprecation of `run`, `run-as-room`, Hub command routing, Hub Actions routes, or the full Hub MCP profile.
- KMP UI implementation.
- Durable reporting spool, reconnect replay, exactly-once delivery, or retained session history across Hub disconnects.
- Diary or notebook in the Normal capability profile.
- Managed tunnel-client distribution for Windows or macOS in V1.
- Local HTTP/Unix-socket MCP transport in V1.
- Automatic Tunnel identity/client/profile hot switching after startup.
- Custom tunnel-client fork or silent dynamic tracking of GitHub `latest`.

## Frozen Runtime Model

Three concerns are independent:

1. **Command transport:** Hub command routing or Tunnel stdio MCP.
2. **Capability/policy profile:** Normal or Room.
3. **Hub connection mode:** command-capable, reporting-only, or disabled.

Public mappings:

| Entry | Command transport | Profile | Hub mode |
|---|---|---|---|
| `agentic-gpt run` | Hub | Normal | command-capable |
| `agentic-gpt run-as-room` | Hub | Room | command-capable |
| `agentic-gpt run-as-standalone` | Tunnel stdio | Normal | reporting-only when enabled, otherwise disabled |
| `agentic-gpt run-as-standalone --profile room` | Tunnel stdio | Room | reporting-only when enabled, otherwise disabled |

Policy depends only on the selected profile: Tunnel Normal preserves current Normal built-in policy; Tunnel Room preserves current Room policy. Transport must not silently relax policy.

Capability resolution:

| Capability | Hub Normal | Hub Room | Tunnel Normal | Tunnel Room |
|---|---:|---:|---:|---:|
| process / batch execution | yes | yes | yes | yes |
| managed sessions | yes | yes | yes | yes |
| tmux | yes | yes | yes | yes |
| downstream MCP bridge | yes | yes | yes | yes |
| skills | no, preserve compatibility | yes | yes | yes |
| `bootstrap` / `bootstrap.read` | no | yes | yes | yes |
| legacy `room.bootstrap*` Hub aliases | no | yes | N/A | N/A |
| diary | no | yes | no | yes |
| notebook | no | yes | no | yes |

## Frozen Public Tool Surfaces

### Tunnel Normal
- `process.exec`, `process.batchExec`
- `session.start`, `session.list`, `session.inspect`, `session.wait`, `session.kill`
- `tmux.listSessions`, `tmux.listPanes`, `tmux.capturePane`, `tmux.pasteText`, `tmux.exec`, `tmux.createSession`, `tmux.closeSession`
- `mcp.listServers`, `mcp.listTools`, `mcp.callTool`
- `skills.list`, `skills.read`, `skills.search`, `skills.active`, `skills.activate`, `skills.deactivate`, `skills.install`, `skills.install.get`, `skills.install.cancel`, `skills.run`
- `bootstrap`, `bootstrap.read`

### Tunnel Room
Tunnel Normal plus:
- `room.notebook.append`, `room.notebook.recent`, `room.notebook.selectExact`, `room.notebook.search`, `room.notebook.current`, `room.notebook.update`, `room.notebook.remove`
- `room.diary.append`, `room.diary.recent`, `room.diary.selectExact`

Tunnel surfaces do not expose Hub-native notification or aggregation tools.

### Hub full profile
- Preserve all current tools and behavior.
- Add transport-neutral `bootstrap` and `bootstrap.read` aliases targeting the active command-capable Room Agent.
- Retain `room.bootstrap` and `room.bootstrap.read` as compatibility aliases.
- Add the Hub-native aggregation tools introduced for coordinator mode.

### Hub coordinator profile
Exactly these tools are discoverable and callable:
- `hub.info`
- `agent.list`
- `hub.run.list`
- `hub.run.get`
- `hub.session.list`
- `hub.session.get`
- `user.notify.channels`
- `user.notify.send`

Process, session-command, tmux, downstream MCP, skills, bootstrap, diary, and notebook tools must be absent from `tools/list` and rejected by direct `tools/call`. Coordinator requests never dispatch an Agent command.

## Frozen Configuration Contract

Additive JSON shape; omitted `tunnel` and top-level `skills` preserve old configurations:

```json
{
  "skills": {
    "maxFiles": 256,
    "maxFileBytes": 10485760,
    "maxPackageBytes": 52428800,
    "maxSkillMdBytes": 262144,
    "maxInlineBytes": 2097152,
    "connectTimeoutSecs": 10,
    "requestTimeoutSecs": 120,
    "idleTimeoutSecs": 30,
    "maxRedirects": 5,
    "maxConcurrentInstalls": 2,
    "maxParallelDownloads": 4,
    "maxAttempts": 3,
    "totalDeadlineSecs": 600,
    "allowedHosts": []
  },
  "tunnel": {
    "tunnelId": "tunnel_...",
    "apiKey": "env:AGENTIC_TUNNEL_API_KEY",
    "client": {
      "version": null,
      "cacheDir": "~/.agentic_gpt/cache/tunnel-client",
      "autoDownload": true,
      "executable": null,
      "downloadUrl": null,
      "sha256": null
    },
    "hubReporting": {
      "enabled": false,
      "detail": "metadata"
    }
  }
}
```

Rules:
- `run-as-standalone` requires non-empty `tunnel.tunnelId` and a valid `tunnel.apiKey` reference.
- `apiKey` accepts only `env:NAME` or `file:PATH`; plaintext is rejected. Empty env/file values are rejected. File content may end in one LF or CRLF, which is removed; other content is preserved. The resolved secret is child-only environment data and never appears in argv, logs, config summaries, generated files, or backups.
- `client.version: null` selects the tunnel-client version pinned by the current Agentic release manifest. An explicit version must exist in that manifest unless a custom `downloadUrl` and `sha256` are supplied.
- `client.executable` selects a user-trusted local binary and bypasses managed download/cache lookup. Its hash is optional; when configured it is verified on every startup. Version output is diagnostic only in executable-override mode.
- Without `executable`, cache lookup uses the selected expected SHA-256. `autoDownload: false` fails when that verified artifact is absent.
- `downloadUrl` is an exact HTTPS archive URL for the current platform and requires `sha256`. Redirects remain HTTPS and are bounded. The API key is never used as a download credential.
- Managed cache identity includes version, platform, and expected SHA-256 so official and custom artifacts cannot collide.
- Built-in V1 assets are official `linux-amd64` and `linux-arm64` ZIP releases. Other platforms fail before network access with `unsupported_platform`.
- Top-level `skills` is canonical. When absent, legacy `room.skills` is loaded. When both exist, top-level `skills` wins and a warning is emitted. A later Agentic config write serializes the canonical top-level block while preserving unrelated fields.
- Tunnel startup identity fields (`tunnelId`, `apiKey`, client source/version/hash/cache, and CLI profile) are immutable for the running process. Config reload reports `restart_required` when they change; it does not switch the child tree.
- Hub reporting uses existing `hubUrl`, `hubTransport`, `agentId`, and `agentSecret`. When disabled, no reporting connection or retry log is created. When enabled, placeholder/invalid Hub identity fails reporting setup visibly but does not fail Tunnel command execution.

## Frozen Tunnel-Client Trust and Installation Contract

- Agentic owns a pinned release manifest containing tunnel-client version, platform asset name, official URL, and SHA-256. V1 begins with the verified official v0.0.10 Linux amd64/arm64 assets unless implementation-time repository policy requires a later explicitly reviewed pin.
- Agentic never queries or follows GitHub `latest` during normal startup.
- Download uses a bounded temporary file, HTTPS-only redirects, archive-size limits, and a per-artifact installation lock.
- Verify SHA-256 before extraction. Extract only one regular file whose basename is `tunnel-client`; reject duplicate candidates, symlinks, path traversal, devices, and unexpected archive layout.
- Install into a private cache directory through atomic rename; preserve an already verified artifact when a new download/install attempt fails.
- Revalidate the expected hash before every execution.
- Cache/download errors are deterministic startup failures and do not enter the runtime restart loop.

## Frozen Supervisor and Worker Lifecycle

Topology:

```text
agentic-gpt run-as-standalone [--profile normal|room]
  └─ tunnel-client run
       └─ agentic-gpt <hidden-stdio-worker> --profile ... --config ...
```

- The public supervisor acquires the existing config-derived `.run.lock` for its entire lifetime, preserving one active Agentic runtime per config. The hidden worker does not acquire a second lock and is accepted only through the internal supervised invocation contract.
- No shell wrapper is used. Agentic generates a correctly quoted `mcp.command`; tunnel-client parses it to argv and launches the worker directly.
- Run `tunnel-client doctor --json` with the final local binding before daemon startup. Failed local config, command, or health checks fail fast.
- Launch tunnel-client with the runtime key in `CONTROL_PLANE_API_KEY`, loopback health on an ephemeral port, a private health URL file below `~/.agentic_gpt/runtime/tunnel/<agentId>/`, and structured logs captured by the supervisor.
- Supervisor stdout remains unused for protocol/log output; Agentic and captured tunnel-client/worker logs go to stderr with component prefixes and secret redaction.
- Worker stdout is exclusively rmcp stdio protocol output. Any accidental non-MCP stdout is a test failure.
- Worker exit or stdio pipe failure causes tunnel-client to exit; the entire tunnel-client/worker tree is the restart unit.
- Retryable failures include transient network/control-plane failures, unexpected tunnel-client exit, readiness timeout, and unexpected worker exit. Permanent failures include Agentic config/reference errors, unsupported platform, missing local executable, checksum/trust failures, tunnel not found, and control-plane authentication/authorization rejection. Unknown exits consume the retry budget.
- Initial launch may be followed by at most five restart attempts. Delays are 1, 2, 4, 8, and 16 seconds; 30 seconds is the configurable backoff cap for any future larger retry budget. After the sixth consecutive failed run, the supervisor exits non-zero.
- Readiness must remain true for 60 seconds to reset the consecutive-failure counter. A bounded startup readiness timeout is required; the Implementer may choose its exact value between 30 and 60 seconds without changing the contract.
- SIGINT/SIGTERM initiates graceful shutdown of tunnel-client, which forwards the signal to the worker. After a bounded grace period, the supervisor kills the remaining process tree and removes runtime health files.
- Normal user shutdown exits successfully. Deterministic startup failure and exhausted retry budget exit non-zero with actionable stderr diagnostics; exact numeric non-zero codes are implementation discretion.

## Frozen Hub Reporting Contract

- Default: `enabled: false`, `detail: metadata`.
- Reporting connection is explicitly `reporting-only`; old protocol clients default to command-capable for wire compatibility.
- A reporting-only connection may exchange Hello, heartbeat/ack, session/report events, and optional remote-confirmation request/response traffic. It must never receive `HubCommandEnvelope` execution commands.
- `request_agent` and Room active-agent routing reject/ignore reporting-only connections before run creation. A reporting Room connection does not become the active command-capable Room Agent.
- One current Hub connection per `agentId` remains the registry rule; replacement semantics stay explicit and tested.
- MCP `tools/call` operations generate Agent-side run ids and nonblocking started/completed/failed events. Initialization, ping, and `tools/list` are not run records.
- A bounded in-memory reporting queue is permitted, but event enqueue must never block or fail the MCP response. When disconnected or full, events are dropped with a redacted warning/counter. There is no disk spool or post-reconnect replay.
- On reporting reconnect, send current active session snapshots as state synchronization; do not replay terminal session history.
- `metadata` detail stores tool name, source=`tunnel`, profile, status, timestamps/duration, run/session ids, exit code when applicable, and bounded failure reason. It does not upload tool arguments, results, program/args, working directory, stdout, or stderr.
- `full` detail additionally stores bounded tool argument/result JSON and full existing `SessionInfo` snapshots, including the already bounded stdout/stderr tails. Oversized argument/result values are replaced by a truncation record containing byte count and SHA-256, not a partial JSON fragment.
- Direct-run records are persisted for the existing 24-hour Hub run TTL. Add source/profile/detail metadata and an Agent-originated upsert path while preserving old Hub-originated run behavior and idempotency.
- `hub.run.list` is bounded and filterable by agent, source, status, and recency; default limit 20, maximum 100. `hub.run.get` returns one common record shape.
- `hub.session.list/get` query only current/recent snapshots held for the active reporting/command connection. Hub disconnect clears those snapshots. Durable session history is represented only through retained run records.
- Reporting failure never changes the local MCP tool result, status, timing, cancellation, or confirmation behavior.

## Frozen Decisions

| ID | Decision | Status |
|---|---|---|
| D-01 | Ship live best-effort Hub reporting for online/heartbeat, direct-run lifecycle, and session snapshots; no durable spool/replay. | confirmed |
| D-02 | Add `bootstrap` / `bootstrap.read`; retain Hub `room.bootstrap*` compatibility aliases. | confirmed |
| D-03 | Accept Tunnel API keys only through `env:` and `file:` references; reject plaintext. | confirmed |
| D-04 | Keep transport independent from Normal/Room policy and capability identity. | confirmed |
| D-05 | Use an Agentic-pinned tunnel-client release manifest with checksum-bound overrides. | confirmed |
| D-06 | Ship a bounded Hub coordinator MCP profile with Hub-native aggregation/notification tools only. | confirmed |
| D-07 | Use `run-as-standalone --profile room`; Normal is the default profile. | confirmed |
| D-08 | Restart the tunnel-client/worker tree with bounded exponential backoff; permanent startup/trust/auth errors fail fast. | confirmed |
| D-09 | Tunnel identity/client/profile changes require restart; no automatic hot switch. | confirmed |
| D-10 | Local executable override may omit hash; every downloaded artifact requires SHA-256. | confirmed |
| D-11 | Reporting detail is configurable `metadata | full`, default `metadata`. | confirmed |
| D-12 | Hub reporting is opt-in and disabled by default. | confirmed |
| D-13 | Allow five restart attempts with 1/2/4/8/16-second delays and reset after 60 seconds continuously ready. | confirmed |

## Implementation Phases

### Phase 1: Requirements and Repository Discovery
**Objective:** Establish the current runtime, routing, persistence, packaging, and external tunnel-client constraints.

- [x] Validate the official tunnel-client embedded stub through a real ChatGPT connector call.
- [x] Inspect Agent CLI/runtime modes, policy coupling, locks, config migration, Hub transports, command dispatch, tools, sessions, audit, run storage, and release targets.
- [x] Inspect official v0.0.10 release assets/checksums, stdio command parser, process lifecycle, doctor JSON output, health behavior, and rmcp stdio server support.
- **Status:** complete
- **Completion boundary:** Findings identify all public surfaces and constraints used by the frozen contract.

### Phase 2: Contract Refinement and Handoff Freeze
**Objective:** Remove product ambiguity and produce an executable implementation handoff.

- [x] Resolve Q-01 through Q-13 with the user.
- [x] Freeze CLI, capability/tool matrix, config/migration, secret handling, binary trust, lifecycle/retry, reporting data semantics, Hub profiles, retention, and platform scope.
- [x] Read and pass the handoff readiness checklist.
- [x] Rebuild implementation phases and acceptance criteria.
- **Status:** complete
- **Completion boundary:** Stage is `implementation_ready`; no blocking decision remains.

### Phase 3: Runtime Model, Configuration, and Shared Local Tool Service
**Objective:** Create the transport/profile foundation and one value-returning local tool layer without changing existing Hub behavior.

**Prerequisites:** Phases 1–2 complete.

**Primary areas:**
- `crates/agentic-gpt/src/main.rs`, `state.rs`, `config.rs`, `policy.rs`, `hub.rs`
- local execution modules: `exec.rs`, `sessions.rs`, `tmux.rs`, `mcp.rs`, `skills.rs`, `skill_installs.rs`, `bootstrap.rs`, `diary.rs`, `notebook.rs`
- `crates/agentic-gpt-protocol/src/lib.rs`

**Work:**
- [x] Replace overloaded `RunMode` decisions with internal transport/profile/Hub-mode concepts while preserving old public entries and serialized role compatibility.
- [x] Add canonical top-level `skills` and optional `tunnel` config types, defaults, validation, safe summaries, `config set` support where appropriate, and legacy `room.skills` migration behavior.
- [x] Define capability resolution from transport + profile; Normal Hub remains unchanged, Tunnel Normal gains skills/bootstrap, Room retains diary/notebook.
- [x] Extract local tool operations from `handle_hub_command` into reusable value-returning services/dispatcher; Hub adapter remains responsible only for envelope/response transport.
- [x] Generalize bootstrap internals to transport-neutral naming and preserve Hub aliases.
- [x] Preserve policy, path policy, confirmations, audit, session limits, and error envelopes.

**Tests / acceptance:**
- Existing `run` and `run-as-room` tests remain green and public behavior is unchanged.
- Config round trips/migration cover old-only, new-only, and both `skills` locations.
- Plaintext Tunnel key and invalid reference syntax are rejected without leaking content.
- Capability matrix tests prove every allowed/denied combination.
- Hub command adapter and direct local dispatcher return equivalent values/errors for shared operations.

**Completion boundary:** Shared local operations and frozen runtime/config model exist; no stdio server or tunnel child is started yet.

**Status:** complete
**Commit:** `feat(agent): add runtime profiles and shared local dispatcher`

**Implementation notes:**
- `RunMode` remains as a compatibility conversion type for the existing `run`/`run-as-room` entry points and tests; `AppState` will not use it as its runtime state.
- Legacy `room.skills` remains deserializable but is omitted from serialization; `Config::load` mirrors it into canonical top-level `skills`, with top-level values winning when both are present.
- Phase 3 adds transport-neutral bootstrap command variants to the shared protocol, while existing `room.bootstrap*` variants remain wire-compatible aliases for the Hub adapter.

**Phase 3 implementation result:** Runtime/profile/config foundation and shared local dispatcher are complete. Existing Hub behavior remains covered by regression tests; stdio worker and Tunnel child lifecycle are deferred to Phases 4–6.

### Phase 4: Capability-Aware Stdio MCP Worker
**Objective:** Serve the frozen Tunnel Normal/Room tool surfaces over protocol-clean stdio.

**Prerequisites:** Phase 3.

**Primary areas:** new local MCP server module(s), `main.rs`, `Cargo.toml`, shared schemas/result conversion, existing Hub `mcp_server.rs` as parity reference.

**Work:**
- [x] Enable rmcp server/macros/`transport-io` alongside current client features.
- [x] Add hidden internal worker invocation carrying config path and profile through the supervised contract.
- [x] Build capability-specific tool routers over the shared local service.
- [x] Preserve current tool names, input schemas, output/result envelopes, annotations, bounded waits, and error codes where surfaces overlap.
- [x] Add only `bootstrap` names on Tunnel; Room-only diary/notebook registration depends on Room profile.
- [x] Keep stdout exclusively MCP and logs on stderr; handle initialize, ping, tools/list, tools/call, cancellation, EOF, and graceful shutdown.

**Tests / acceptance:**
- In-process stdio initialize/list/call tests for Normal and Room.
- Exact tool-set tests for both profiles, including negative direct-call tests for absent Room tools.
- Protocol stdout contamination test.
- Representative parity tests for process, session, downstream MCP result passthrough, skills, bootstrap, diary, and notebook.
- EOF/signal cleanup leaves no managed child/session leak.

**Completion boundary:** A directly launched internal worker is a valid local stdio MCP server for both profiles.

**Status:** complete
**Commit:** `feat(agent): add capability-aware stdio worker`

**Implementation result:** Added the hidden `stdio-worker` invocation, rmcp stdio server, exact Normal/Room tool allowlists, shared-dispatch command mapping, MCP result envelopes, annotation/schema descriptors, stderr-only startup path, and in-process protocol tests. Worker lock exclusion and parent authorization remain in the frozen Phase 6 supervisor boundary.

**Phase 4 verification evidence:** `cargo fmt --all -- --check`; `cargo test -p agentic-gpt --bin agentic-gpt` (109 passed); `cargo test --workspace` (Agent 109 + Hub 56 + Protocol 8 passed). The in-process rmcp transport tests cover initialize/list/call for both profiles and the descriptor tests cover exact 29/39 sets plus absent Room-only calls.

### Phase 5: Trusted Tunnel-Client Distribution Manager
**Objective:** Resolve a trusted executable from local override or managed Linux cache without ever executing unverified downloaded bytes.

**Prerequisites:** Phase 3 config types.

**Primary areas:** new tunnel distribution/manifest modules, `Cargo.toml`, release/build tests.

**Work:**
- [x] Add pinned manifest entries for Linux amd64/arm64 official assets and checksums.
- [x] Implement platform selection, path expansion, executable override, optional local hash, cache identity, and auto-download semantics.
- [x] Implement HTTPS download/redirect/size bounds, temporary files, per-artifact lock, SHA-256, safe ZIP extraction, permissions, atomic installation, cleanup, and cache revalidation.
- [x] Preserve valid cached artifacts on failure and produce redacted deterministic error codes.

**Tests / acceptance:**
- [x] Manifest/platform mapping and checksum fixtures.
- [x] Local HTTP test server is used only in tests; production validation remains HTTPS-only.
- [x] Tests cover redirects, size limit, hash mismatch, traversal, symlink/duplicate candidate, interrupted download, concurrent installers, atomic replacement, offline verified-cache use, autoDownload=false, and executable override.
- [x] Unsupported platform and trust failures occur before execution.

**Completion boundary:** Given valid config, the resolver returns one verified executable path or a deterministic redacted error.

**Implementation result:** Added a pinned v0.0.10 Linux manifest, HTTPS-only bounded downloader with manual safe redirects, archive SHA-256 verification, a private identity-keyed cache, async per-artifact locking, symlink/path/device-safe ZIP extraction, executable permissions, atomic staged replacement, and cache revalidation that derives executable bytes from the verified archive. Local overrides are checked before use and may opt into a SHA-256 check.

**Phase 5 verification evidence:** `cargo fmt --all -- --check`; `cargo check -p agentic-gpt`; `cargo test -p agentic-gpt tunnel_distribution::tests` (10 passed). The tests use loopback HTTP only for redirect, size, and interrupted-download cases; production URL validation rejects non-HTTPS schemes.

**Commit:** focused Phase 5 commit.

### Phase 6: `run-as-standalone` Supervisor and Process Lifecycle
**Objective:** Make the public command own the complete tunnel-client/worker lifecycle with bounded recovery and observable readiness.

**Prerequisites:** Phases 4–5.

**Primary areas:** `main.rs`, `instance_lock.rs`, new supervisor/runtime modules, signal/process helpers.

**Work:**
- [x] Add public CLI entry/profile parsing and retain existing commands.
- [x] Acquire the existing config runtime lock in the supervisor; implement safe hidden-worker authorization without a second lock.
- [x] Resolve secret and executable, create private runtime paths, generate exact child argv/env, and run `doctor --json` preflight.
- [x] Launch tunnel-client with stdio command, ephemeral loopback health, URL file, captured structured logs, and no secret-bearing argv/profile.
- [x] Implement readiness observation, permanent/retryable failure classification, five-attempt backoff, 60-second reset, signal forwarding, grace timeout, process-tree kill, and runtime-file cleanup.
- [x] Detect startup-identity config changes and emit restart-required diagnostics while preserving safe existing hot reload.

**Tests / acceptance:**
- [x] Fake tunnel-client fixture verifies argv/env separation, no secret output, health URL handling, worker invocation, and lock exclusion.
- [x] Retry schedule, reset, exhausted budget, permanent-error fail-fast, startup timeout, child exit, signal shutdown, and stale runtime-file cleanup behavior are covered by the supervisor policy and lifecycle tests.
- [x] The official client command contract was validated against v0.0.10 source; the loopback fake-client smoke exercises the same `doctor`/`run`/health/stdio-child lifecycle through Agentic. Live control-plane E2E remains part of Phase 9 delivery verification.

**Completion boundary:** `agentic-gpt run-as-standalone` reliably starts/stops/restarts the official client and internal MCP worker on supported Linux targets.

**Implementation result:** Added `run-as-standalone --profile normal|room`, supervisor-owned `.run.lock`, strict child-only API-key resolution, per-run worker authorization, official tunnel-client `doctor --json` preflight and direct argv construction, private readiness/log/pid paths, loopback health polling, bounded restart/backoff/reset handling, config restart-required diagnostics, Unix process-group signal forwarding, graceful shutdown, and stale health-file cleanup.

**Phase 6 verification evidence:** `cargo fmt --all -- --check`; `git diff --check`; `cargo test --workspace` (Agent 126, Hub 56, Protocol 8 passed). The fake-client lifecycle test covers doctor, secret argv/env separation, worker command, health readiness, and process-tree shutdown; official live control-plane E2E is explicitly retained for Phase 9.

**Commit:** focused Phase 6 commit.

### Phase 7: Reporting-Only Hub Protocol and Persistence
**Objective:** Allow Tunnel runtimes to report status/run/session data without becoming command targets or coupling reporting success to tool execution.

**Prerequisites:** Phases 3–4; supervisor integration from Phase 6 for final wiring.

**Primary areas:** protocol messages, local Hub connection split, stdio tool-call wrapper, session transitions, Hub `agents.rs`, `runs.rs`, `db.rs`, `state.rs`, routes/tests.

**Work:**
- [x] Add backward-compatible connection mode in Hello/registry; old clients default command-capable.
- [x] Split Hub transport into command-capable and reporting-only behavior; permit heartbeat and confirmation responses but prohibit command envelopes.
- [x] Add nonblocking direct-run events and bounded reporting-detail transformation/redaction/truncation.
- [x] Add explicit session transition reporting and active-session sync on reconnect.
- [x] Add Agent-originated run upsert, schema migration/source/profile/detail fields, 24-hour cleanup, idempotency, and bounded list/get queries.
- [x] Define current-connection session snapshot queries and clear them on disconnect.
- [x] Ensure active Room routing and `request_agent` cannot select reporting-only connections.

**Tests / acceptance:**
- Old Hello payloads remain command-capable.
- Reporting connections never receive execution commands; attempts return a stable reporting-only error before Hub run dispatch.
- Hub offline/disconnect/full queue never delays or changes MCP results.
- Metadata mode contains no args/results/program/cwd/tails; full mode stores bounded content and truncation records.
- Direct-run started/completed/failed idempotency, TTL cleanup, filters/limits, session sync, disconnect cleanup, and remote confirmation behavior are covered.

**Completion boundary:** Optional reporting produces useful Hub state/history with the frozen best-effort and privacy semantics.

**Status:** complete

**Implementation result:** Added the wire-compatible `connectionMode` Hello extension, Hello-ready gating, reporting-only WebSocket/SSE connections with separate control and bounded event queues, Agent run lifecycle reports, bounded full-detail payloads, redacted metadata session snapshots, reconnect synchronization, Agent-originated 24-hour run upserts, conflict-safe idempotency, bounded run listing, and snapshot-only session list/inspect queries. Reporting remains opt-in and never gates local stdio tool results.

**Phase 7 verification evidence:** `cargo fmt --all -- --check`; `git diff --check`; focused Hub/Protocol tests; and escalated `cargo test --workspace` (Agent 129, Hub 58, Protocol 9, doc tests 0) passed. A non-escalated workspace run was also attempted; its two loopback failures were sandbox `Operation not permitted` errors and were resolved by the authorized rerun.

**Commit:** focused Phase 7 commit.

### Phase 8: Hub Full/Coordinator MCP Profiles
**Objective:** Preserve centralized execution while adding a strict aggregation-only connector surface.

**Prerequisites:** Phase 7 query/storage APIs.

**Primary areas:** Hub CLI/config, `mcp_server.rs`, `routes.rs`, OAuth metadata/instructions/tests.

**Work:**
- [x] Add `--mcp-profile full|coordinator`, default `full`, with config/env integration as appropriate.
- [x] Make server instructions, descriptor construction, annotations, and Apps-compatible manual dispatch profile-aware.
- [x] Implement the exact coordinator allowlist and Hub-native queries.
- [x] Add `bootstrap` aliases to full profile while retaining `room.bootstrap*`.
- [x] Ensure OAuth/resource metadata remains valid for either profile and does not imply unavailable tools.

**Tests / acceptance:**
- Full profile regression tool set plus new aliases/aggregation tools.
- Coordinator exact tool set, annotations, schemas, and instructions.
- Every hidden execution tool is absent from list and rejected when called directly.
- Coordinator calls generate no Agent command/run dispatch.
- `hub.run.list/get`, `hub.session.list/get`, agent status, and notifications work from stored/native state.

**Completion boundary:** Users can choose unchanged full Hub MCP or bounded coordinator MCP from the same Hub binary.

**Status:** complete

**Implementation result:** Added startup-fixed `full`/`coordinator` MCP profile selection via CLI/environment, profile labels in initialize and OAuth metadata, descriptor and direct-call filtering, the exact eight-tool coordinator allowlist (including `hub.info`), native `hub.session.list/get` snapshot tools, full-profile bootstrap aliases, and regression coverage proving hidden execution tools produce no Hub run.

**Phase 8 verification evidence:** `cargo fmt --all -- --check`; `git diff --check`; `cargo test -p agentic-gpt-hub` (61 passed), including exact coordinator tool-set, hidden direct-call/no-dispatch, full alias, and Apps-dispatcher compatibility tests.

**Commit:** focused Phase 8 commit.

### Phase 9: Documentation, End-to-End Verification, and Delivery
**Objective:** Prove both deployment models, document operational recovery, and deliver without regressions.

**Prerequisites:** Phases 3–8.

**Primary areas:** README files, `docs/interfaces.md`, config examples, release scripts/workflows, integration tests.

**Work:**
- [x] Document architecture, four runtime mappings, exact tool surfaces, config/migration, secret references, cache/source overrides, reporting privacy modes, Hub profiles, health/log diagnostics, restart behavior, and recovery.
- [x] Add setup examples that never place the Runtime API key in shell history or argv.
- [x] Validate packaged Linux amd64/arm64 binaries can resolve the pinned tunnel-client assets.
- [x] Run the actual Agentic supervisor/stdio-worker/local-tool topology with a local fake tunnel handoff.
- [x] Resolve the production connector E2E boundary: waived for this delivery by explicit user decision because consistent account-scoped `tunnelId` and runtime API key credentials are unavailable; local topology evidence remains required and passing.
- [x] Validate Tunnel Normal and Room, reporting disabled/enabled, Hub unavailable, metadata/full reports, coordinator profile, and existing centralized Hub mode.

**Verification:**
- `cargo fmt --all -- --check`
- focused crate tests during phases
- `cargo test --workspace`
- release/package build checks for both Linux targets where the environment supports them
- real end-to-end connector call when account-scoped credentials and connector access are part of the delivery scope; explicitly waived for this delivery by the user

**Completion boundary:** All repository-local automated/release suites pass, local supervisor-to-worker tool execution succeeds, docs match behavior, existing Hub mode remains compatible, and any external connector requirement is explicitly satisfied or waived by the user. The external production connector call is waived for this delivery.

**Status:** complete; repository-local work and verification passed, and the user explicitly waived production connector E2E for this delivery.

**Implementation result:** Added the standalone runtime operational guide and README/interface links, corrected the coordinator MCP surface to include the frozen Hub-native `hub.info` tool, and recorded release/test evidence plus the external E2E boundary in planning.

**Phase 9 verification evidence:** `cargo fmt --all -- --check`; `git diff --check`; authorized `./scripts/dist-linux.sh` for x86_64/aarch64 release artifacts; release ELF/CLI/embedded-manifest inspection; local supervisor-to-worker MCP smoke test; and authorized `cargo test --workspace` (Agent 129 unit tests plus 1 integration test, Hub 61, Protocol 9, doc tests 0) passed. Production connector E2E is explicitly waived by the user for this delivery.

**Commit:** focused Phase 9 delivery/docs commit.

### Phase 10: Real tunnel-client Command and Diagnostic Repair
**Objective:** Correct the production `--mcp.command` binding exposed by a real `tunnel-client doctor`, surface redacted doctor diagnostics, and make the regression harness enforce real parsing semantics.

**Trigger evidence:** A real startup reached `doctor --json` and failed `mcp_command_executable` because Agentic wrapped the entire worker command in quotes. The fake tunnel integration stripped those quotes and therefore masked the production defect.

**Work:**
- [x] Pass `channel=main,command=<worker command>` without re-quoting the complete command; retain quoting only for individual executable/argument tokens inside the worker command.
- [x] On doctor failure, return bounded redacted exit-code/stdout/stderr diagnostics without exposing the Runtime API key or per-run worker authorization token.
- [x] Update focused/unit/integration tests so the fake tunnel rejects a whole-command quoted binding instead of adapting it.
- [x] Run focused tests, full workspace tests, formatting, diff checks, and one real local startup against the configured official tunnel-client.

**Status:** complete

**Implementation result:** Removed whole-command quoting from the tunnel binding while preserving token-level quoting, added 16 KiB bounded/redacted doctor diagnostics, redacted both Runtime API key and worker authorization token from forwarded child logs, and made the local fake-tunnel integration reject the invalid production shape.

**Verification evidence:** supervisor focused tests 11/11; standalone supervisor integration 1/1; full workspace Agent 132 + 1 integration, Hub 61, Protocol 9; formatting/diff/check pass; a temporary isolated config using the real pinned official tunnel-client reached `standalone tunnel ready`.

## Cross-Phase Acceptance Criteria

1. Users start Tunnel mode only with `agentic-gpt run-as-standalone`; direct tunnel-client CLI knowledge is not required after config is set.
2. Both Normal and Room Tunnel connectors expose exactly their frozen tool sets and enforce existing local safety policies.
3. No runtime API key appears in process argv, config output beyond its reference, logs, Hub reports, generated runtime files, or test snapshots.
4. No downloaded tunnel-client binary executes before expected SHA-256 verification; cache and override semantics match the contract.
5. Tunnel command execution remains functional while Hub reporting is disabled, offline, reconnecting, or dropping events.
6. Reporting-only Agents cannot receive Hub execution commands or become the active command-capable Room Agent.
7. Metadata reporting excludes sensitive execution content; full reporting is explicit and bounded.
8. Coordinator MCP exposes only the exact Hub-native allowlist and cannot dispatch local commands.
9. Existing `run`, `run-as-room`, full Hub MCP, Actions routes, policies, and stored Hub-run behavior remain backward compatible except for additive aliases/fields.
10. **Waived for this delivery by explicit user decision:** a real ChatGPT/OpenAI Secure MCP Tunnel call reaches an Agentic local tool and returns its result through the supervised topology. Future production E2E may be added when account-scoped connector credentials and access are available.

## Implementation Discretion

The Implementer may choose:
- private module/type/trait names and whether local tool reuse uses a trait, enum dispatcher, or service object;
- exact internal hidden-subcommand spelling and authorization token shape;
- exact bounded startup-readiness timeout from 30–60 seconds and graceful-shutdown timeout;
- internal queue implementation and capacity, provided it is bounded, nonblocking, redacted, and drops rather than blocks when unavailable;
- SQL/index organization and migration helper structure while preserving public fields, TTL, idempotency, and query bounds;
- exact log wording and test helper names;
- equivalent secure ZIP/download libraries and private cache metadata layout consistent with the frozen identity/trust rules.

Implementation discretion must not alter public CLI names/defaults, tool sets, policy mapping, secret-reference rules, trust requirements, reporting privacy/defaults, retry count/delays/reset, Hub profile allowlist, retention, or compatibility behavior.

## Verification Convention
- Add focused tests with each phase; do not defer all testing to delivery.
- Preserve user changes and inspect `git status` before each phase.
- Full `cargo test --workspace` and `cargo fmt --all -- --check` are mandatory before delivery.
- Tunnel acceptance requires one real connector tool call, not only `/healthz` or `/readyz`.
- Failures and deviations are written to `progress.md`; contract changes require reopening refinement.

## Commit Convention
- Implementation uses one focused commit per completed phase, with tests green for that phase.
- A design checkpoint commit is recommended before Phase 3 but is **not authorized yet**.
- Do not combine planning checkpoint and product implementation in one commit.

## Readiness Gate

- [x] Goal, scope, non-goals, and ownership boundary are explicit.
- [x] Repository evidence identifies current implementation and conventions.
- [x] No blocking question remains open.
- [x] Every user-owned choice is confirmed.
- [x] Inputs, outputs, defaults, compatibility, and versioning are frozen.
- [x] State, timing, concurrency, idempotency, cancellation, retries, and failure boundaries are frozen.
- [x] Persistence, migration, retention, cleanup, recovery, and rollback are frozen.
- [x] Security, secrets, paths, network, and trust boundaries are frozen.
- [x] Resource limits, configuration, observability, and operational behavior are frozen.
- [x] Every requirement maps to a phase and acceptance criterion.
- [x] Phase dependencies and exact entry phase are clear.
- [x] Implementation discretion cannot change the external contract.
- [x] All three planning files agree on maturity and blockers.
- [x] User accepted the consolidated contract through Q-01 through Q-13 answers.
- [x] No product code, tests, configuration, or generated artifacts were changed during refinement.
- **N/A:** rollback of persisted reporting data beyond normal additive SQLite migration; no destructive migration or removal is planned.

## Implementation Handoff

- **Plan maturity:** implementation_complete
- **Design phase:** complete
- **Implementation authorized:** yes
- **Entry phase:** Phase 3 - Runtime model, configuration, and shared local tool service
- **Frozen decisions:** D-01 through D-13
- **Open blocking decisions:** none
- **Implementation discretion:** see `Implementation Discretion`
- **Verification convention:** focused per-phase tests, full workspace format/tests, packaged Linux checks, and one real Tunnel tool call when that external scope is available; production connector E2E is explicitly waived for this delivery
- **Commit convention:** focused per-phase commits
- **Design checkpoint:** not set
- **Next invocation:** none for this delivery; external connector E2E may be revisited only if a future scope supplies account-scoped credentials and connector access.

## Errors Encountered During Planning

| Error | Resolution |
|---|---|
| Room `skills.run` rejected the laptop repository as `invalid_working_directory`. | Read the skills through Agentic and invoked the same installed planning script through the laptop process tool. |
| A parallel official-source inspection read before the sibling clone completed. | Re-ran source inspection serially; no repository or planning state was affected. |

## Errors Encountered During Phase 3

| Error | Attempt | Resolution |
|---|---:|---|
| Workspace Hub crate failed to compile after additive protocol bootstrap variants: one `SafeConfigSummary` initializer and three Hub command matches were non-exhaustive. | 1 | Updated the Hub-side additive compatibility matches and reran workspace tests; no protocol variant was removed. |
