# Findings & Decisions: Agentic Standalone Tunnel Runtime

## Phase 7 Implementation Assumptions

- The wire-compatible extension will add a `connectionMode` field to `AgentMessage::Hello`, defaulting to `command_capable`; this lets existing clients keep execution semantics while Tunnel reporting explicitly opts into `reporting_only`.
- Reporting-only Agent messages will use a separate `RunReport` event envelope rather than reusing `Response` or `TransportRunStatus`; Hub can therefore reject execution-oriented messages and keep Agent-originated records distinct from Hub-originated command runs.
- The stdio worker will own the best-effort reporting connection and bounded queue. Local MCP dispatch remains authoritative; report enqueue is a nonblocking side effect and the worker will not await Hub delivery before returning a tool result.
- The first reporting implementation will use the existing WebSocket/SSE Hub transports and the existing `hubUrl`, `hubTransport`, `agentId`, and `agentSecret` fields. It will advertise the worker profile and detail mode in the Hello/config summary, without adding a second authentication or persistence channel.
- Session synchronization will be scoped to the current connection: the Hub clears snapshots on replacement/disconnect, and a reporting reconnect sends only the worker's current in-memory session snapshot.
- Existing `agent_runs` rows remain backward-compatible. New Agent-originated rows will use nullable source/profile/detail columns and a stable `run_id` supplied by the Agent; old Hub-originated rows continue through their current prepare/ack/result path.
- Report payloads will be sanitized at the Agent boundary: metadata excludes tool arguments/results and process fields; full detail uses bounded JSON values and existing bounded `SessionInfo` tails. Oversized JSON is represented by byte count plus SHA-256.
- A newly accepted connection is not command-ready until its Hello has been processed. This closes the replacement/Hello race without changing the default mode for legacy Hello payloads.
- Hub `session.list` and `session.inspect` are snapshot queries in Phase 7; they no longer issue a remote execution command first. The snapshot cache is cleared on current-connection replacement/disconnect, while terminal history remains available through run records.
- The new `hub.run.list` query filters non-expired rows in descending creation order, defaults to 20 results, caps at 100, and exposes the same common `AgentRun` shape as `hub.run.get`.

## Phase 8 Implementation Assumptions

- The MCP profile is selected once at Hub startup through `serve --mcp-profile` (with `AGENTIC_GPT_HUB_MCP_PROFILE` and a `full` default) and stored in `HubState`; it is not negotiated per request.
- The custom `/mcp` JSON-RPC dispatcher remains the authoritative Apps-compatible path. It will filter descriptors and reject hidden tool names before the operation match, while the underlying rmcp router remains the full implementation registry.
- The coordinator allowlist is the frozen seven-tool set: `agent.list`, `hub.run.list`, `hub.run.get`, `hub.session.list`, `hub.session.get`, `user.notify.channels`, and `user.notify.send`. The two Hub session aliases are native snapshot queries and never call `request_agent`.
- Full profile keeps its existing execution surface and adds the Hub-native coordinator tools plus additive `bootstrap`/`bootstrap.read` aliases; existing `room.bootstrap*` names remain.
- OAuth/resource metadata remains shared across profiles and gains only an additive profile label, so clients can discover the selected surface without being told that hidden tools are available.

## Requirements
- The user-facing startup remains `agentic-gpt run-as-standalone`.
- Agentic internally manages the official OpenAI tunnel-client lifecycle.
- The official binary is downloaded from a configurable source into a configurable cache, or an existing executable can be selected.
- Tunnel id and runtime API-key reference are configured through Agentic.
- tunnel-client invokes an internal Agentic stdio MCP worker.
- Standalone exposes local execution tools, downstream MCP bridge, skills, and bootstrap.
- Diary and notebook remain Room-only.
- Existing Hub-centric command routing is retained.
- Hub should be able to remain the aggregation surface for a future KMP dashboard, run records, status, and notifications.

## External Validation Already Completed
- The official tunnel-client embedded MCP stub was connected successfully through Secure MCP Tunnel.
- ChatGPT discovered and called `server_info`; the returned stub advertised `server_info`, `echo`, and `uppercase`.
- This proves the tested account, tunnel, runtime key, connector association, long-poll path, local stub binding, and response path work end-to-end.

## Repository Findings

### Current local-agent modes
- `crates/agentic-gpt/src/state.rs` defines only `RunMode::Normal` and `RunMode::Room`.
- `RunMode` currently determines both local policy behavior and the protocol `AgentRole` sent to Hub.
- `crates/agentic-gpt/src/main.rs` exposes `run` and `run-as-room`.
- Both public modes enter one `run(config_path, run_mode)` function.
- That function always ends in `hub::connect_loop(state).await`; there is no non-Hub command transport.

### Current tool implementation and routing
- Local execution implementations already live in the agent crate: exec, sessions, tmux, downstream MCP client, skills, skill installs, bootstrap, notebook, and diary.
- Hub's `mcp_server.rs` defines the public MCP schemas and converts calls into `HubCommand` values.
- The local agent handles those commands after Hub WebSocket/SSE delivery.
- Standalone should reuse local implementations rather than duplicate behavior inside a second MCP stack.

### Room-only behavior
- Notebook, diary, and bootstrap commands are currently rejected in Normal mode by local command dispatch.
- Hub routes Room tools to the active Room Agent without an `agentId`.
- Skills are currently also routed through the active Room Agent and described publicly as Room-scoped, even though their storage and core implementation are local workspace operations.
- Bootstrap internals and public API contain Room-specific names, including `room.bootstrap`, `room.bootstrap.read`, `Room Bootstrap`, and a canonical revision prefix containing `agentic-room-bootstrap-v1`.

### MCP dependencies
- The agent crate already uses `rmcp 1.7.0` with client transports for downstream MCP.
- It does not currently enable rmcp server or stdio server transport features.
- The Hub crate already uses rmcp server macros and Streamable HTTP server transport, providing patterns for tool schemas and result envelopes.

### Hub value beyond command routing
- Hub stores agent registry/online state, managed run records, cached session state, notification channels, and Room routing state.
- These are relevant to a future KMP dashboard even if direct tool execution bypasses Hub.
- Current Agent hello/role handling assumes connected agents can participate in command routing; reporting-only behavior is not represented separately.

### Runtime topology constraint
- In stdio binding mode, tunnel-client launches the downstream command and owns its stdin/stdout.
- Therefore the public Agentic process should supervise tunnel-client, while tunnel-client launches a hidden/internal Agentic MCP worker process.
- The supervisor and worker need distinct lock semantics.
- The worker's stdout must be exclusively MCP protocol output; logs belong on stderr.

## Initial Technical Direction
- Split internal runtime concepts into at least:
  1. command transport (`Hub` versus local MCP stdio),
  2. capability profile (`Normal`, `Standalone`, `Room`),
  3. Hub relationship (`command-capable`, `reporting-only`, `disabled`).
- Keep public CLI commands simple and map each command onto those axes.
- Introduce a shared local tool service/dispatcher reused by the Hub command adapter and standalone MCP adapter.
- Implement tunnel-client as an externally versioned official executable managed by Agentic, not linked through Go FFI.
- Prefer runtime secret injection through environment/file references rather than argv.
- Bind custom download URL to explicit version and checksum validation.

## Security and Operational Concerns to Refine
- Remote executable download must not become unconstrained arbitrary-code execution through config.
- Define allowed URL schemes, checksum requirements, redirects, archive extraction safety, symlink handling, file permissions, and atomic replacement.
- Decide whether Agentic owns an embedded trusted release manifest or requires user-supplied checksums for every source.
- Prevent API keys from appearing in command lines, logs, generated diagnostics, or persistent generated tunnel profiles.
- Define safe behavior when config reload changes tunnel identity or binary distribution while running.
- Define child-process cleanup if supervisor, tunnel-client, or MCP worker exits unexpectedly.
- Reporting failure must not change tool-call success or block the direct command path.

## Skill Workflow Findings
- `planning-with-files` v3.7.0 is active and provides scoped `.planning/<id>` initialization, persistent task/findings/progress files, session catch-up, and phase/error discipline.
- `refine-implementation-plan` is active and requires an existing active plan, complete repository inspection, decision-focused discussion, contract freezing, and readiness checks before implementation.
- Refinement must not modify product code.

## Decisions

| ID | Decision | Rationale | Status |
|---|---|---|---|
| U-01 | Preserve a separate standalone public mode. | Direct Tunnel and Hub-centric command routing serve different deployments. | confirmed |
| U-02 | Use official tunnel-client binary. | Reuses the supported implementation and avoids maintaining the control-plane protocol. | confirmed |
| U-03 | Agentic supervises tunnel-client. | Removes manual profile/env/startup steps from normal usage. | confirmed |
| U-04 | Use stdio for the first internal MCP binding. | No local network listener is required and tunnel-client owns worker lifecycle. | confirmed direction |
| U-05 | Standalone receives skills/bootstrap but not diary/notebook. | Skills/bootstrap are workspace execution context; diary/notebook carry Room identity and continuity semantics. | confirmed |
| U-06 | Cache directory and download URL are configurable. | Supports custom filesystems, mirrors, offline/preprovisioned systems, and deployment constraints. | confirmed |
| A-01 | Treat transport/profile/Hub role as separate internal axes. | Prevents `RunMode` from accumulating unrelated semantics and supports future local HTTP/Unix transports. | adopted by D-04 |
| A-02 | Use a supervisor → tunnel-client → internal Agentic worker topology. | Required by tunnel-client stdio child-command ownership while preserving one public entry. | adopted in frozen supervisor contract |

## Issues Encountered

| Issue | Resolution |
|---|---|
| Room `skills.run` cannot execute the planning initializer in the laptop project path. | Read the skill package through `agentic.skills`, then invoked the same installed script with the laptop process tool. |

## Repository Locations
- `crates/agentic-gpt/src/main.rs`: CLI, runtime creation, unconditional Hub connection, command tests.
- `crates/agentic-gpt/src/state.rs`: current `RunMode` and `AppState`.
- `crates/agentic-gpt/src/hub.rs`: local Agent-to-Hub lifecycle and command reception.
- `crates/agentic-gpt/src/exec.rs`, `sessions.rs`, `tmux.rs`, `mcp.rs`: local execution services.
- `crates/agentic-gpt/src/skills.rs`, `skill_installs.rs`, `bootstrap.rs`: capabilities to generalize.
- `crates/agentic-gpt/src/notebook.rs`, `diary.rs`: Room-only capabilities.
- `crates/agentic-gpt-hub/src/mcp_server.rs`: current complete public MCP surface and schema patterns.
- `crates/agentic-gpt-hub/src/agents.rs`, `runs.rs`, `notify.rs`, `room.rs`: command routing, records, notification aggregation, and active Room semantics.
- `crates/agentic-gpt-protocol/src/lib.rs`: roles, capabilities, commands, messages, and public serialization contracts.


## Existing Implementation Details Added During Refinement

### Configuration and compatibility
- `Config` is one camelCase JSON object with serde defaults for additive fields; `Config::load` already performs a small compatibility repair when `pathPolicy` is absent.
- Hub identity and credential fields (`hubUrl`, `hubTransport`, `agentSecret`) are top-level and currently mandatory in the Rust struct, even though standalone command execution should not require a working Hub.
- Skill installation limits are currently nested under `room.skills`, which conflicts with making skills a shared Standalone/Room capability without either migration or a compatibility alias.
- `safe_summary()` always reports built-in Normal policy rules; a future capability/runtime refactor must avoid silently changing existing summary semantics.

### Logging and stdio safety
- All current local startup logging helpers call `eprintln!`, so the default logging path is already stderr-safe for a future stdio MCP worker.
- Some command-oriented CLI paths print JSON to stdout intentionally; the internal MCP worker must bypass those paths and ensure no incidental `println!` reaches protocol stdout.

### Locking and process topology
- The existing agent lock is advisory and derived from the canonical config path plus `.run.lock`.
- Both `run` and `run-as-room` currently acquire that same lock before building state.
- A standalone supervisor and its worker cannot both acquire the existing lock. The plan needs one authority owner and a separate supervisor lock or a worker-token mechanism.

### Hub connection coupling
- `hub::connect_loop` currently owns reconnection, command reception, confirmation responses, heartbeat, sender registration, reliable-run reconciliation, and command dispatch.
- Both WebSocket and SSE transports send `AgentMessage::Hello` with one `AgentRole`; neither protocol nor Hub registry has a reporting-only connection contract.
- The local `hub_sender` is also used by confirmation delivery, so disabling Hub command reception while retaining Hub-based confirmation/notification requires an explicit directional contract rather than merely ignoring received commands.

### Persistence and reporting foundations
- Execution audit already appends local JSONL under `<workspaceRoot>/.agentic-gpt-audit.jsonl` and includes source, policy, result, skill provenance, and timing.
- `transport_ledger` is specifically shaped around Hub command envelopes and reliable response replay, not generic standalone execution events.
- Hub already stores `agent_runs` and cached sessions, but those records are created from Hub-originated commands; direct Tunnel calls currently have no Hub run identity or upload path.

### Release packaging
- Agentic's own release workflow currently builds Linux `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` archives and publishes SHA256SUMS.
- This provides a strong repository convention for initially limiting the managed tunnel-client matrix to the same two Linux targets unless the user expands scope.


## Official Tunnel-Client v0.0.10 Evidence
- GitHub latest release resolves to `v0.0.10` and publishes six platform ZIPs: Linux/macOS/Windows on amd64 and arm64, plus combined archives, `SHA256SUMS.txt`, and `PUBLIC_URLS.txt`.
- Linux archive names are `tunnel-client-v0.0.10-linux-amd64.zip` and `tunnel-client-v0.0.10-linux-arm64.zip`; their published SHA-256 values were retrieved from the release manifest during refinement.
- The official configuration contract requires a runtime API key, tunnel id, and `main` MCP binding.
- `--mcp.command` spawns one command and communicates over its stdin/stdout; stdio bindings do not support MCP transport sessions.
- Default tunnel-client logs go to stdout unless `log.file` is configured. A supervised integration must capture/redirect them deliberately.
- Health defaults to loopback `127.0.0.1:8080`; an ephemeral port plus a private per-run `health.url_file` is officially recommended when another supervisor needs the resolved URL.
- Secret-bearing fields support `env:` and `file:` references; literal values remain accepted by tunnel-client for compatibility but are not recommended.
- Official E2E testing supports both hosted-control-plane tests and a local `dev proxy` path; the release itself includes in-repo mocks and end-to-end coverage.

## rmcp Server Feasibility
- `rmcp 1.7.0` exposes a `server` feature and `transport-io`; its documented server transport can serve `(tokio::io::stdin(), tokio::io::stdout())` directly.
- The existing agent crate can add server/macros/transport-io features alongside its current client transports, while Hub remains on its existing Streamable HTTP server features.

## Dispatch Refactor Evidence
- `hub::handle_hub_command` is a large transport-aware match that both executes local operations and sends Hub responses.
- Room gating is repeated per command using `state.run_mode != RunMode::Room` across notebook, diary, bootstrap, and skills.
- Extracting value-returning local operations and capability checks before introducing the stdio adapter will avoid a second divergent dispatch implementation.
- Policy behavior is also coupled to `RunMode`: Room intentionally has fewer built-in confirmation requirements than Normal. Standalone policy semantics therefore require an explicit decision rather than inheriting a mode name accidentally.


## Official Stdio Lifecycle Details
- `mcp.command` is parsed into argv with explicit quote and escape handling, then launched directly with `exec.Command(args[0], args[1:]...)`; it is not implicitly wrapped in a shell.
- Agentic can therefore generate one correctly quoted command string containing its own executable path, internal subcommand, and config path without a temporary shell script.
- tunnel-client attaches the worker's stderr to its own stderr, reserves the worker stdin/stdout for MCP, and forwards SIGINT/SIGTERM to the worker.
- If worker stdout closes, stdin writes fail, or the worker exits unexpectedly, tunnel-client requests its own shutdown. The outer Agentic supervisor should treat tunnel-client + worker as one restart unit.
- On normal stop, tunnel-client closes worker stdin, sends SIGTERM, and waits for exit within its lifecycle shutdown context.

## Refine Process Error Record
- A parallel source inspection attempted to read `/tmp/openai-tunnel-client-v0.0.10` before the sibling clone command completed and returned `No such file or directory`.
- Resolution: reran the source inspection serially after clone completion; no repository or planning state was affected.


## Decision Rationale: Round 1

### D-01 — Live best-effort Hub reporting
- Implement online/heartbeat, direct execution lifecycle, and session updates in this delivery.
- Reporting is explicitly non-authoritative for MCP success: Hub failure cannot fail or delay the Tunnel response path.
- No durable event spool, restart replay, or delivery guarantee is included in V1; local audit remains available for diagnostics.

### D-02 — Bootstrap naming compatibility
- New transport-neutral names are `bootstrap` and `bootstrap.read`.
- Existing Hub names `room.bootstrap` and `room.bootstrap.read` remain compatibility aliases.
- All aliases call one local workspace-bootstrap implementation; Room identity is no longer embedded in the core service name.

### D-03 — Reference-only Tunnel secret
- Agentic accepts both `env:NAME` and `file:/path` forms.
- Plaintext literals are rejected during config validation rather than merely redacted later.
- The resolved secret is injected into the tunnel-client child environment and never placed in argv or generated persistent profile content.

### D-04 — Transport/profile independence
- Normal and Room are capability and policy profiles; Hub and Tunnel are command transports.
- Normal-over-Tunnel uses the current stricter Normal built-in policy.
- Room-over-Tunnel uses current Room policy and may expose Room-only diary/notebook capabilities.
- This changes the earlier shorthand in which “Standalone” looked like a third capability identity; standalone is now treated as a Tunnel-backed runtime topology.

### D-05 — Pinned release trust
- Each Agentic release owns a tested tunnel-client version and asset/checksum manifest.
- Default startup never follows GitHub `latest` dynamically.
- A custom URL is permitted only with an explicitly configured SHA-256; a user-selected local executable is a separate trusted override path.


## Q-02 Workload Estimate: Coordinator MCP Profile

### Existing surface
- Hub MCP currently exposes 43 tools from one generated `ToolRouter` plus a separate manual `tools/call` match used by the Apps-compatible endpoint.
- The current constructor always exposes the full router and one execution-oriented instruction string.
- A profile implementation must therefore filter both descriptors and dispatch, provide profile-specific instructions, and test that hidden tools cannot be called by name.

### Missing aggregation tools
- Current useful Hub-native MCP tools are limited to `agent.list`, `hub.run.get`, `user.notify.channels`, and `user.notify.send`.
- There is no MCP `hub.info`, no run-list query, and no clean cached-only session aggregation surface.
- Existing `session.list` / `session.inspect` may forward commands to an online Agent, so they are not safe coordinator tools without semantic changes.

### Bounded coordinator proposal
A useful V1 coordinator profile can remain small and Hub-native:
- `hub.info`
- `agent.list`
- `hub.run.list`
- `hub.run.get`
- `hub.session.list`
- `hub.session.get`
- `user.notify.channels`
- `user.notify.send`

It must not expose process, tmux, downstream MCP, skills, bootstrap, diary, notebook, or command-capable session tools.

### Cost assessment
- Profile plumbing and strict allowlist: small.
- New Hub-native query tools and tests: moderate.
- Database/reporting schema required by D-01 overlaps heavily with `hub.run.list/get`; cached session data already exists in memory and needs a query wrapper rather than a new execution path.
- Estimated incremental scope is roughly one focused implementation phase and about 10–15% additional effort relative to the full standalone/tunnel refactor, not a second major subsystem.
- Because the reporting data is otherwise difficult to inspect from ChatGPT and is directly relevant to future KMP, the revised recommendation is to include Q-02B with the bounded tool set above.


## Decision Rationale: Round 2

### D-06 — Coordinator Hub MCP profile
- Include the bounded coordinator profile in this delivery because reporting schema/query work overlaps directly with D-01 and future KMP needs.
- The profile is an MCP-surface restriction, not a separate Hub data plane or binary.
- Both descriptor discovery and direct tool dispatch must enforce the allowlist.

### D-07 — Tunnel profile selection
- `run-as-standalone` means Tunnel-backed command transport and defaults to Normal capability/policy.
- `run-as-standalone --profile room` selects Room capabilities and policy without creating a second long public subcommand.
- Existing `run` and `run-as-room` remain Hub-command entries for compatibility.

### D-08 — Restart unit and failure classes
- tunnel-client and the internal MCP worker form one restart unit because official tunnel-client exits when its stdio child exits or its pipes fail.
- Runtime failures use bounded exponential backoff and reset after a stable-ready interval.
- Deterministic failures such as invalid config, unsupported platform, missing secret reference, checksum mismatch, or unavailable explicitly configured executable fail fast.

### D-09 — Restart-required Tunnel config
- Existing mutable execution policy/workspace configuration may continue using current hot reload where safe.
- Tunnel id, API-key reference, tunnel-client version/source/hash/executable, and capability profile are startup identity and process-topology inputs; changes are detected and reported as restart-required.
- Automatic Tunnel switching is excluded to avoid dropping or ambiguously routing in-flight requests.

### D-10 — Local executable trust
- An explicit local executable path is treated as an administrator/user trust decision and may omit SHA-256.
- When a hash is supplied, every startup verifies it.
- Every network-downloaded artifact must have a pinned expected SHA-256 before execution.


## Final Repository Gap Findings

## Phase 3 Implementation Assumptions

- The existing `HubCommand` enum is the stable shared request envelope. Phase 3 will introduce transport-neutral bootstrap variants additively and use a local value-returning dispatcher around the same payload shapes, so the Hub adapter and future stdio adapter cannot diverge in result conversion.
- Existing modules already apply policy and path/session limits at the operation boundary. The runtime model will select profile/capabilities and pass the selected profile into those existing checks; it will not create a second policy implementation.
- The legacy `RoomConfig.skills` field is retained only for deserialization compatibility and an in-memory mirror during this phase. Config serialization will emit canonical top-level `skills`, so a later config write performs the planned migration without deleting unrelated JSON fields represented by the typed config.
- Tunnel binary resolution, secret materialization, supervisor lifecycle, and reporting transport remain deferred to their frozen phases; Phase 3 validates only configuration shape/reference syntax and safe summaries.

## Phase 3 Workspace Validation Finding

- Adding protocol-level `bootstrap` and `bootstrap.read` variants requires matching updates in the Hub crate's request-id mutation, command-name mapping, and safe-summary construction. The variants are additive and the compatibility aliases remain intact.

### Direct-run reporting storage
- Existing `agent_runs` persists full serialized Hub commands and full results for 24 hours.
- Direct Tunnel calls have no Hub-created run row, so reporting needs an Agent-originated run upsert message and Hub insertion path.
- A schema-compatible approach can retain non-null `command_json`/`command_hash` by storing a versioned direct-run report envelope and adding a `source` column defaulting to `hub`.
- `hub.run.list` requires a bounded indexed query with filters; `hub.run.get` can continue returning the common run representation.

### Session reporting
- Local managed sessions retain terminal entries for 24 hours, capped at 100.
- Session state is refreshed lazily on inspect/list/kill; the current monitor loop does not publish transition events.
- D-01 therefore requires an explicit session observer/report hook at creation, state transition, and terminal completion rather than only reusing the current one-time Hub-command response.
- Hub currently stores session snapshots in memory and removes an Agent's entire cache on disconnect.
- For V1 coordinator semantics, `hub.session.list/get` can be defined as current/recent snapshots received during the active Hub connection; durable cross-disconnect session history is excluded because run records provide the retained execution history.

### Connection directionality
- A reporting connection still needs inbound heartbeat acknowledgements and optional remote confirmation responses, but must reject/never receive `HubCommandEnvelope` values.
- Add an explicit connection mode to Hello/registry state, defaulting old clients to command-capable for wire compatibility.
- `request_agent` must reject reporting-only connections before preparing/distributing a run.

### Platform boundary
- Agentic identifies itself as a Linux local agent and only publishes Linux x86_64/aarch64 release archives.
- V1 tunnel-client management should support the corresponding official `linux-amd64` and `linux-arm64` assets; other targets fail with `unsupported_platform` before download.

### Configuration migration
- Shared skill configuration should move from `room.skills` to canonical top-level `skills`.
- Legacy `room.skills` remains accepted when top-level `skills` is absent.
- If both appear, top-level values win with a warning; config writes migrate to the canonical top-level block.
- Default Tunnel cache stays under the project's existing `~/.agentic_gpt` home rather than introducing a second XDG root.

### Supervisor observability
- Agentic can launch tunnel-client directly with argv and a child-only secret environment variable; no persistent tunnel profile or shell wrapper is required.
- tunnel-client stdout/stderr can be captured by the supervisor and re-emitted on Agentic stderr with component prefixes.
- Use a private per-run health URL file beneath `~/.agentic_gpt/runtime/tunnel/<agentId>/`; readiness controls stable-run reset and startup diagnostics.
- Config changes affecting Tunnel identity/distribution are logged as restart-required while the current child remains unchanged.


## Decision Rationale: Final Round

### D-11 — Configurable reporting detail
- `metadata` is the privacy-preserving default and excludes tool/session content that could contain local code, file contents, credentials, or personal data.
- `full` is explicit for debugging/dashboard use and remains bounded; oversized JSON is represented by size/hash metadata rather than partial content.
- The setting affects only Tunnel-originated reporting. Existing Hub-originated commands retain their current full run storage behavior.

### D-12 — Reporting opt-in
- Default-disabled avoids meaningless Hub reconnects for Tunnel-only users and preserves Tunnel as an independently functional command path.
- Enabling reporting reuses the existing Hub identity/config, but failure remains isolated from MCP execution.

### D-13 — Bounded restart defaults
- The accepted default is initial launch plus at most five restart attempts, with delays of 1/2/4/8/16 seconds.
- The 30-second value is a cap for configurable/future larger retry budgets; it is not reached by the default five attempts.
- Sixty seconds continuously ready resets the counter, preventing old transient failures from exhausting a later healthy runtime.

## Handoff Readiness Findings
- All Q-01 through Q-13 are resolved and represented by D-01 through D-13.
- Current implementation evidence covers CLI/runtime, policy, config/migration, stdio server feasibility, command dispatch, sessions, Hub protocol/registry, run DB, MCP profiles, release packaging, official client trust/lifecycle, and end-to-end stub validation.
- During the design-only period, the only repository modifications were `.planning/.active_plan` and the new active planning directory; subsequent Phase 3 implementation changes are recorded below.
- The implementation entry was Phase 3; Phases 1–2 are complete and Phase 3 is now complete.
- A focused planning checkpoint is appropriate but requires separate user authorization.

## Phase 3 Implementation Findings

- An additive `#[serde(flatten)]` map on `Config` preserves unmodeled top-level JSON fields during the existing load/write cycle, satisfying migration preservation without changing the public typed contract.
- The active Hub handler now gets its request id from a protocol-level `HubCommand::request_id()` helper. This keeps request-id extraction consistent with Hub command replay/mutation helpers and avoids another transport-specific mapping in the local service.
- Session-start `SessionUpdate` remains a Hub adapter side effect; the shared local dispatcher returns only the `SessionInfo` value, so future stdio calls do not require a Hub sender.

## Phase 4 Implementation Assumptions

- The stdio Tunnel worker reuses the existing Hub MCP argument shapes for overlapping process, session, tmux, and downstream-MCP tools, including `agentId`; the worker validates that value against its loaded local `Config.agent_id` before dispatch. This preserves the frozen shared schemas while keeping the worker local.
- Phase 4 adds a hidden `stdio-worker` CLI entry that loads the supplied config path without acquiring the runtime lock. The public supervisor's lock exclusion and child authorization remain Phase 6 responsibilities.
- The worker uses rmcp's native stdio framing and returns the existing local dispatcher JSON as both structured content and bounded text content; operational logs continue to use stderr-only helpers.

## Phase 5 Implementation Assumptions

- Archive handling will use a Rust ZIP reader in the agent binary rather than invoking a host `unzip` command. This keeps traversal, symlink, duplicate-candidate, size, and file-type checks under Agentic's policy and makes behavior portable across the supported Linux targets.
- Per-artifact installation coordination will use a lock file created atomically inside the selected cache and a temporary sibling directory/file followed by atomic rename. A failed candidate never replaces an already verified cache artifact; stale lock recovery is bounded and deterministic rather than deleting an active install.

### Phase 5 official manifest evidence

- GitHub release `openai/tunnel-client` tag `v0.0.10` asset metadata reports `tunnel-client-v0.0.10-linux-amd64.zip` SHA-256 `b9e0388a343f2d7adeff3992f411a0bd3d916a64bc56534aac5fd15ac1b20cd5` and `tunnel-client-v0.0.10-linux-arm64.zip` SHA-256 `b842a9b2352eebd80514cf01a1fbb1c0d400a7d24a4015e85a7ea5f1aeaa5b30`.
- The downloaded official amd64 archive was independently hashed and listed; it contains exactly one entry named `tunnel-client`, so the strict single-candidate extraction rule matches the real artifact layout.

### Phase 5 distribution safety bounds

- The resolver will cap a downloaded archive at 64 MiB, cap total extracted regular-file bytes at 128 MiB, and inspect at most 32 ZIP entries. These are local resource-safety limits, not trust substitutes for the pinned SHA-256 checks.

### Phase 5 implementation findings

- The cache retains the verified archive beside the executable. Every cache lookup re-hashes the archive, extracts the sole trusted candidate in memory, and repairs/replaces a changed executable before returning its path; the archive digest therefore remains the cache trust identity rather than an unrelated executable digest.
- Local executable overrides reject symlinks and non-executable files before optional SHA-256 verification. The resolved override is canonicalized only after the configured path itself passes those checks.
- The downloader follows redirects manually, keeps the HTTPS requirement across every hop, writes to a unique temporary file, and removes that file on download or verification failure. Installation stages both archive and executable and swaps the staged directory only after all checks pass.
- Artifact locks are async-waiting, atomically created files keyed by version/platform/archive digest. A stale lock is recoverable after five minutes; active locks are never deleted.

## Phase 6 Implementation Assumptions

- The supervisor uses the frozen 45-second startup readiness bound (within the allowed 30–60 second range), polls the private URL-file's loopback `/healthz` and `/readyz`, and treats HTTP readiness as the process-tree startup boundary.
- The API key is resolved once by Agentic and injected only as `CONTROL_PLANE_API_KEY` into the tunnel-client child environment. The worker inherits the child environment as required by tunnel-client's stdio topology, but neither public supervisor argv nor the generated MCP command contains the key.
- The hidden worker receives a per-run random authorization token in its command and matching child-only environment variable. This prevents an ordinary direct `stdio-worker` invocation from being accepted while avoiding any persistent authorization file.
- The tunnel-client child is placed in its own Unix process group with `setpgid`; graceful stop sends SIGTERM to that group and the bounded fallback sends SIGKILL, allowing worker descendants to be cleaned with the tunnel-client.
- Runtime state uses `~/.agentic_gpt/runtime/tunnel/<agentId>/`, a mode-0700 directory, fixed health/pid paths for stale-file cleanup, and a retained structured tunnel log for post-failure diagnostics.

### Phase 6 implementation findings

- `doctor --json` and `run` receive the same tunnel id, secret reference, loopback ephemeral health settings, and quoted `channel=main` stdio worker command. The run-only flags add JSON log and pid-file paths without creating a persistent tunnel-client profile.
- Child exit code 2 is treated as a permanent runtime failure; other unexpected exits and readiness timeouts consume the five-attempt restart budget. A healthy 60-second interval resets that budget before a later failure.
- Startup identity watching compares only restart-sensitive references/settings and the selected CLI profile. It emits one redacted `restart_required` diagnostic per observed identity transition while leaving the current child tree untouched.

## Phase 9 Implementation Assumptions and Constraints

- The delivery documentation will be additive: the root README files will link to a focused standalone-runtime guide, while the existing interface and operations documents remain authoritative for their current surfaces.
- Setup examples must use `file:` or `env:` secret references without assigning the Runtime API key on a command line. Examples will use placeholders and explain secure shell/file provisioning separately; no secret-bearing fixture will be committed.
- The current repository can prove the official stub/fake-client supervisor lifecycle and local Rust behavior, but a real Secure MCP Tunnel connector call requires an external control-plane/ChatGPT connector not exposed by the local test commands. If that external path is unavailable, the evidence will explicitly distinguish health/stub success from the unmet real-call acceptance criterion.
- Release validation will use the existing `scripts/dist-linux.sh` contract where `cross` and target toolchains are available. If the environment lacks those prerequisites, local release builds and manifest/asset tests will be recorded as the available substitute rather than claiming both packaged architectures were built.
- Phase 9 is the final implementation phase; its commit may include the focused operational guide, README/interface links, planning evidence, and any narrowly scoped verification fixture changes, but it must not add unrelated product features.

### Phase 9 contract correction

- The frozen coordinator surface lists `hub.info` alongside the seven already-implemented Hub-native tools, while the Phase 8 implementation exposed `hub.info` only as the Actions route `GET /v1/info`. This is a real surface gap, not a documentation choice. Phase 9 will add an MCP `hub.info` wrapper around the existing safe Hub summary and extend the exact coordinator-list regression from seven to eight tools; no Agent dispatch is involved.

### Phase 9 verification evidence and external boundary

- The first in-sandbox `scripts/dist-linux.sh` attempt failed while `cross` tried to install its target toolchain because the sandbox denied a write pipe. The authorized rerun succeeded and produced all four release binaries under the existing ignored `dist/` directories.
- The first in-sandbox `cargo test --workspace` run reached the tests and passed 127 tests, but the two loopback fixtures were denied `Operation not permitted`. The authorized rerun passed Agent 129, Hub 61, Protocol 9, and zero doc tests.
- Release inspection confirmed x86_64 ELF output for amd64 and AArch64 ELF output for arm64. Both Agent binaries embed the pinned v0.0.10 amd64/arm64 URLs and archive digests; the target-specific platform mapping is covered by `tunnel_distribution` tests.
- The available API/tool surface for this turn has no Secure MCP Tunnel connector capable of establishing the external control-plane session, and no external tunnel credentials were provided. The local fake-client supervisor test is passing, but it is not evidence of a real connector tool call. This acceptance item remains explicitly pending rather than being inferred from health or `doctor` success.

### Phase 9 user delivery decision

- The user explicitly confirmed that the repository-local implementation is sufficient for this delivery and that a live production connector call is not required. A production call would require account-scoped, mutually consistent `tunnelId` and runtime API key credentials that are not appropriate to invent or commit.
- The official source's `dev proxy`/mock control-plane remains useful for future local client development, but it is not treated as production connector evidence and no further external E2E work is required in this plan.
- This is a user-approved delivery-boundary change, not a silent test downgrade: local supervisor/worker MCP evidence, full automated tests, release packaging, and documentation remain required and have passed.

### Phase 9 continuation constraint and local evidence improvement

- A fresh inventory found no configured MCP resources/resource templates or connector-specific tool in the current execution environment; this confirms that the external E2E boundary is still unavailable rather than merely undiscovered in the repository.
- The existing supervisor test verifies tunnel arguments, secret separation, health readiness, and shutdown, but its fake client does not launch the worker or issue an MCP request. A new integration smoke test may emulate only the tunnel client's local stdio handoff and drive the actual built `agentic-gpt stdio-worker` through `process.exec`. This strengthens local topology evidence while remaining explicitly non-equivalent to a real Secure MCP Tunnel control-plane call.
- The new integration smoke test passed after emulating the tunnel client's double-quoted `mcp.command` binding correctly. It exercised the actual supervisor CLI, actual hidden worker binary, MCP initialize/notification/tools-call traffic, and local `/usr/bin/printf` execution; the fake tunnel supplied only loopback readiness and did not claim external control-plane connectivity.
## Post-delivery Real Startup Finding

- A real `run-as-standalone` startup reached the pinned official `tunnel-client v0.0.10` and failed only the `mcp_command_executable` doctor check. Manual doctor execution showed the entire `command=<executable> <args>` value had been wrapped as one quoted token, so the client treated the full command line as the executable path.
- `Invocation::new` already quotes the executable path, config path, and supervisor token individually. `Invocation::mcp_command` must therefore concatenate the completed worker command directly and must not call `quote_arg` around the whole string.
- The Phase 9 fake tunnel integration removed whole-command quotes before `sh -c`; that was recorded as a harness parser correction but actually adapted the test to invalid production output. The repaired harness must reject this shape and consume only the valid unwrapped command binding.
- `run_doctor` currently discards piped stdout/stderr and returns only `tunnel_doctor_failed`. Failure diagnostics should include bounded UTF-8-lossy exit code/stdout/stderr after replacing the resolved secret with `[REDACTED]`.

### Phase 10 repair result

- `mcp.command` now preserves the worker command's internal token quoting and no longer wraps the full command line. The official pinned client accepted the generated binding and reached readiness under an isolated temporary config using the real account-scoped tunnel settings.
- Doctor failures now include exit code plus bounded stdout/stderr. Both the Runtime API key and the random per-run worker authorization token are redacted before error construction; forwarded tunnel stdout/stderr uses the same two-value redaction boundary.
- The fake-tunnel integration now fails with exit 23 when the worker command begins with a whole-command quote. It no longer strips delimiters or adapts invalid output before launching the real hidden worker.

## Post-delivery Authorization Header Finding

- The configured key reference points to a mode-0600 file with 166 bytes, two LF bytes, no CR/NUL/space/tab, and two logical lines (`[164, 0]`). The secret payload itself was not inspected or printed.
- `resolve_secret` removes exactly one CRLF or one LF. With two trailing LF bytes, one remains in `CONTROL_PLANE_API_KEY`; the official Go client then rejects its own Authorization header before sending metadata or poll requests.
- Local `/readyz` can become healthy while control-plane metadata/poll continuously fails, so real verification must inspect control-plane success/failure rather than treating local readiness alone as end-to-end evidence.

### Phase 11 repair result

- File-backed secrets use `trim_end_matches(['\r', '\n'])`, preserving all non-line-ending bytes while tolerating common editor-created trailing blank lines. Any control character remaining after normalization is rejected before child-process environment injection.
- With the original unmodified mode-0600 key file containing two trailing LF bytes, the rebuilt Agentic supervisor and pinned official client successfully fetched tunnel metadata and started polling; no invalid Authorization-header event remained.
- ChatGPT then discovered the exact 29-tool Normal surface. A real `process.exec` returned `agentic-tunnel-e2e-ok` with exit code 0; `bootstrap` returned the expected structured `bootstrap_not_found` business error; `skills.list` returned local skill data. This proves discovery, success responses, and structured error responses across the full production topology.
