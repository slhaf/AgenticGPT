# Operations

This page records minimum checks for reproducible deployment. Standalone is the primary path; Hub checks are separate because Hub is optional.

## Repository verification

Run the Rust CI checks in the order specified by `.github/workflows/ci.yml`:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p agentic-gpt -p agentic-gpt-hub
python3 -m pip install "PyYAML" "jsonschema[format]>=4.25,<5"
python3 scripts/check_contract_parity.py
```

For an externally managed local Python installation, activate an ignored
virtual environment before running this sequence:

```bash
python3 -m venv target/contract-venv
source target/contract-venv/bin/activate
```

The parity checker is the cross-surface schema/live gate. With no options it
uses `target/debug/agentic-gpt` and `target/debug/agentic-gpt-hub`;
`--agent-bin PATH` and `--hub-bin PATH` select explicit binaries. It runs
isolated loopback/private-home processes and tears them down; schema validation
and live behavior are separate obligations. Passing the parity gate does not
substitute for the earlier strict Clippy check.

The standalone HTTP MCP integration fixture uses short, UUID-unique agent IDs:
its local Unix MCP socket is rooted under `HOME/.agentic_gpt`, and the runtime
rejects socket paths longer than 100 bytes. When changing the fixture, check
the full socket path against the CI runner's home directory.

## Local/Standalone smoke test (primary)

Set `mode=local` when tunnel credentials are unavailable; use `mode=standalone` to validate the full recommended path. Both start with `agentic-gpt run`.

```bash
agentic-gpt --version
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt run
```

From another shell:

```bash
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

Expected:

- `agent.info.connections.localMcp.status` is `ready`.
- Runtime directory is `0700`, socket is `0600`, and only the same UID is accepted.
- `process.exec`, `process.batch`, `process.status`, `process.list`, `process.output`, `process.result`, `process.cancel`, `mcp.callTool`, and `mcp.batch` are present.
- `process.status` reports metadata only; use the dedicated output and result tools for their payloads.
- Process history uses `process.sqlite3`; existing `jobs.sqlite3` is intentionally untouched and inaccessible, with no Job-history migration or backup.
- Removed v0.8 managed lifecycle names are absent.

For Standalone, also confirm:

- tunnel-client `doctor` and loopback readiness pass;
- the ChatGPT connector can call `agent.info`;
- one disconnected machine does not affect another machine's tunnel;
- after restart, a resumed first call does not produce `expect initialized request` or restart the worker pair;
- `mcp_stdio_session_resume` / `mcp_stdio_session_resumed` appear only when recovery is needed.

## Hub smoke test (optional centralized mode)

```bash
tmp=$(mktemp -d)
cargo run -q -p agentic-gpt-hub -- --db "$tmp/hub.sqlite3" --config "$tmp/hub.json" init
AGENTIC_GPT_API_KEY=test-key \
  cargo run -q -p agentic-gpt-hub -- --db "$tmp/hub.sqlite3" --config "$tmp/hub.json" serve --bind 127.0.0.1:18787
curl -fsS -H 'Authorization: Bearer test-key' http://127.0.0.1:18787/v1/info
```

Expected JSON includes `service`, `version`, `remoteConfirmation`, `agents`, `counts`, and `generatedAt`.

### WP1 closure evidence boundary

The current WP1 closure evidence is bounded: final cleanup passed `cargo fmt --all -- --check`, the integrated `cargo test -p agentic-gpt-hub` suite, and `cargo build -p agentic-gpt-hub` without warnings. An isolated Hub HTTP/SSE smoke used simulated Agent peers to exercise six confirmation, replacement, callback, timeout, and late-receipt cases. After that smoke, a real Hub restart against the same temporary SQLite database preserved two `completed` `/v1/runs` records and `sessions[]`, while `/v1/info` reported zero `pendingRequestCount`, `pendingConfirmationCount`, and `cachedJobCount`. This demonstrates Hub-side receipt/session cleanup and retention only; it did not execute a real Agent or transport-ledger restart and is not full end-to-end proof of every historical roadmap scenario. External ntfy provider behavior remains unverified; a local/mock callback path is not equivalent evidence.

## Standalone deployment checks

1. Confirm `agentic-gpt --version` is the intended release.
2. Confirm the tunnel secret is a protected `file:` or `env:` reference.
3. Confirm `agentic-gpt run` with `mode=standalone` reaches readiness and stays stable beyond the restart-budget reset interval.
4. Call `agent.info` through both ChatGPT tunnel and Local Unix MCP.
5. Start one harmless process and inspect it with `process.status`, `process.output`, and `process.result`; use `process.cancel` only when explicitly requesting cancellation.
6. Restart one Agent and verify other machine connectors remain usable.
7. Confirm audit JSONL is beneath `workspaceRoot` and contains no raw tunnel/MCP secrets.

## Configuration reload and restart diagnostics

The live/restart boundary is shared by Standalone, Local, and Hub-connected Agent
workers; it is not a Standalone-only feature. While a worker is running, valid
changes to `policy`, `limits`, `mcpServers`, and `toolsets.enabled` apply
atomically to subsequent admissions, calls, and tool discovery. `pathPolicy`
also reloads when `workspaceRoot` is unchanged. If a candidate changes
`workspaceRoot`, the previous `workspaceRoot` and `pathPolicy` remain effective
as one atomic pair until restart; do not treat a partial candidate as active.

For a Standalone HTTP MCP listener, verify the reconciler with a controlled
change to `httpMcp.enabled`, `host`, `port`, `publicUrl`, `allowHosts`, and the
bearer-token reference or resolved content. Enable/disable and endpoint changes
reconcile without restarting the worker. A listener-identity change closes
stateful HTTP sessions and discards listener-local OAuth state, while a token
change updates direct authentication in place and preserves existing sessions
while revoking OAuth records. An unresolved credential fails closed; an invalid
candidate retains the last-good listener; a bind conflict is retried without
disturbing tunnel or Unix execution.

Changes to startup-owned fields such as `mode`, `profile`, `agentId`,
`workspaceRoot`, `browser`, Room settings, tunnel/client settings, reporting
mode, or skill-install concurrency are restart-owned. The shared watcher logs
`config changes require restart; fields=...` with the changed field names but
never secret values. The Standalone supervisor additionally emits
`restart_required`; Hub mode has no supervisor event. Editing the file does not
switch the existing child tree. Enabling the Room namespace live uses the
current live Room root; restart-required Room edits do not move that root until
restart.

The `agentic-gpt local` command is the owner-only Unix MCP client and retains
`local:` audit provenance. It is distinct from the `agentic-gpt tmux`
local-admin CLI, which exposes exactly four commands: `list`, `attach`,
`create`, and `close` (request-context operation names `tmux.listSessions`,
`tmux.attach`, `tmux.createSession`, and `tmux.closeSession`). Those CLI
calls use `localadmin:` provenance, do not add remote approval semantics, and
do not fabricate AppState. Other ingress sources remain `tunnel:`, `http:`,
and `hub:`.

## Hub deployment checks

1. Confirm the Hub and Agent binaries are the intended paired artifacts by
   recording `agentic-gpt --version` and `agentic-gpt-hub --version` output,
   artifact paths, and checksums. Do not substitute a release number or
   service name for this pairing check.
2. Confirm `/v1/info` responds through public HTTPS.
3. Confirm `/v1/agents` shows expected command-capable Agents online.
4. Run one harmless command through `/v1/process/exec`.
5. Inspect process metadata through `GET /v1/process` and
   `GET /v1/process/{processId}`; fetch output from
   `GET /v1/process/{processId}/output`, the result from
   `GET /v1/process/{processId}/result`, and request cancellation through
   `POST /v1/process/{processId}/cancel`.
   The status endpoint is metadata-only. The standalone `process.status` wait
   defaults to 5 seconds and is capped at 30; HTTP `waitSeconds` defaults to
   5 seconds. Output starts at byte zero when no cursor is supplied; `maxBytes`
   defaults to 8 KiB and is capped at 32 KiB. MCP result `maxBytes` defaults
   to 8 KiB and is capped at 512 KiB; HTTP result reads use the same bounds.
6. Validate `/mcp` and refresh Actions schema when the contract changed.
7. If Standalone reporting is enabled, confirm reporting-only connections reject Hub execution.

### WP-R current Room contract cutover

Use this clean cutover when adopting the current nine-operation Room contract.
It changes the Hub/Protocol/Agent request and response projection; it does not
migrate Room content into Hub or add a compatibility alias.

1. **Inventory the paired artifacts.** Record `agentic-gpt --version` and
   `agentic-gpt-hub --version`, the exact binary paths, and checksums. Keep the
   Hub and Agent artifacts as one verified pair. Restart them with the same
   existing process invocation used by the deployment (`agentic-gpt run` for
   the Agent and `agentic-gpt-hub ... serve` for the Hub); this procedure does
   not invent a system-service command.
2. **Record effective configuration and owners.** Set `AGENT_CONFIG` to the
   actual Agent config path (the default is `~/.agentic_gpt/config.json`) and
   inspect `agentic-gpt config show --config "$AGENT_CONFIG"` for
   `workspaceRoot`, `room.repositoryRoot`, `mode`, and `profile`. The Room
   repository is `room.repositoryRoot` or `<workspaceRoot>/room`. Record the
   actual Hub `--db` and `--config` paths (the defaults are
   `~/.agentic_gpt/hub.sqlite3` and `~/.agentic_gpt/hub.json`; deployments may
   set `AGENTIC_GPT_HUB_DB` and `AGENTIC_GPT_HUB_CONFIG`).
3. **Quiesce both sides.** Pause new Hub HTTP/MCP calls, drain in-flight calls,
   and record any undrained `runId` values and corresponding Agent transport
   ledger entries. Stop the Hub and the affected Agent using the existing
   deployment process. A waiter timeout is not cancellation; do not replay a
   command merely because its HTTP wait ended.
4. **Back up before replacement.** Preserve the Agent config with its file
   mode, the complete configured Room repository including `.git`, and the
   Agent audit/transport files required by the deployment. A Git bundle of all
   refs plus a filesystem copy of the Room root is useful for verification.
   Preserve the Hub config and SQLite database together with matching `-wal`
   and `-shm` files when present. Keep backups read-only and label them with
   the artifact pair; never overwrite a newer database or Room repository with
   an older copy.

For a concrete operator backup, set `BACKUP_DIR` to a protected destination
and use the recorded paths rather than a guessed service layout:

```bash
mkdir -p "$BACKUP_DIR"
cp -a "$AGENT_CONFIG" "$BACKUP_DIR/agent-config.json"
cp -a "$ROOM_ROOT" "$BACKUP_DIR/room"
git -C "$ROOM_ROOT" bundle create "$BACKUP_DIR/room.git.bundle" --all
cp -a "$HUB_CONFIG" "$BACKUP_DIR/hub.json"
cp -a "$HUB_DB" "$BACKUP_DIR/hub.sqlite3"
test ! -e "$HUB_DB-wal" || cp -a "$HUB_DB-wal" "$BACKUP_DIR/hub.sqlite3-wal"
test ! -e "$HUB_DB-shm" || cp -a "$HUB_DB-shm" "$BACKUP_DIR/hub.sqlite3-shm"
```

Also copy `<workspaceRoot>/.agentic-gpt-audit.jsonl` when present and the
owner-bound `~/.agentic_gpt/transport-runs.jsonl` plus its `.lock`, `.recovery`,
and `.backup` evidence when present. If `HOME` or the Agent home is relocated,
use the actual `agentic_home` location from the deployment; never infer
transport state from the Hub database.
5. **Deploy and reconnect.** Replace the Hub and Agent binaries as one pair,
   retain the existing config paths, and restart with the existing arguments
   and secret references. Reconnect the command-capable Room Agent first; a
   Normal, ReportingOnly, stale, or unready connection must not become the
   Room target. Do not add a version field, feature flag, alias, or dual
   execution path.
6. **Verify the current contract.** Confirm `GET /v1/info` and
   `/v1/agents`, then inspect Full MCP `tools/list` for the nine current Room
   names (and no retired names) and Coordinator `tools/list` for their absence.
   Through
   the authenticated HTTP API, exercise `/v1/room/diary/active`,
   `/v1/room/diary/read`, `/v1/room/notebook/recent`,
   `/v1/room/notebook/search`, `/v1/room/notebook/read`,
   `/v1/room/state/list`, `/v1/room/state/read`,
   `/v1/room/maintenance/status`, and
   `/v1/room/maintenance/submit` with current camelCase DTO bodies and no
   `agentId`.
   Check 422 `text/plain` extraction for a missing required JSON field, 400
   JSON for semantic validation, 404 for no active Room, 409 for lease
   conflict, and 504 for a transport wait timeout. Verify successful reads
   are bounded Agent-owned content; Hub receipts are only bounded result
   projections.
7. **Verify maintenance ownership.** On a disposable or explicitly approved
   Room repository, submit one documented semantic slot/payload through
   `room.maintenance.submit` in `local` mode and verify the returned state,
   revision, request-residue cleanup, and Git change are limited to the
   declared target. A disposable local smoke can use the existing Notebook
   payload shape, for example
   `{"items":[{"slot":"notebook","payload":{"path":"Notebook/wp-r-smoke.md","title":"WP-R smoke","body":"bounded smoke"}}],"mode":"local","waitSeconds":0}`;
   use a repository and target that are explicitly disposable or approved.
   For workflow mode, verify `submitted`/`pending` versus observed sync
   outcomes and confirm that `waitSeconds` 0–30 only bounds the wait; it never
   cancels the request. Do not use retired append/update/remove shapes as a
   maintenance test.
8. **Migrate callers and cut over.** Update imported OpenAPI consumers and
   internal callers to the nine current operations. `recent` and `search`
   retain their names but consume current Markdown preview/results; old
   passage/JSONL shapes and old append/update/remove/date-selection semantics
   are not silently translated. Remove old callers only after the current
   route, MCP descriptor, Agent dispatch, and repository-owner checks pass.

**Rollback boundary.** If verification fails, pause new calls, stop the new
pair, preserve all current Hub/Agent/Room evidence, and restore only the
previous verified binaries (and unchanged config if necessary). Keep the
latest Hub database, `-wal`/`-shm`, Room `.git`, maintenance journal, audit,
and transport ledgers; do not restore an older database or Room copy over
newer results. Reconnect the previous pair and re-check `/v1/info`,
`/v1/agents`, active Room lease safety, and the recorded `runId` values.
Binary rollback cannot retract a maintenance commit or an already-delivered
external effect; any content correction must use the existing Agent/Git and
controlled-maintenance authority. There is no destructive data migration to
undo.

### Hub Response ownership cutover

Use this procedure when moving to a Hub build with the Response ownership fix:

1. Identify the exact new Hub artifact and the paired, verified Agent artifact/commit. Do not proceed unless the pair has been verified together.
2. Pause new requests and wait for in-flight requests to settle. If they cannot be drained, record their `runId` values and retain the corresponding Agent ledger entries.
3. Stop the Hub. While it is stopped, back up the actual Hub configuration and SQLite database; if present, preserve the matching `-wal` and `-shm` files with the database backup.
4. Deploy the paired artifacts, restart the Hub, and reconnect the Agent.
5. Execute one matching request, then verify `GET /v1/runs/{runId}` reports the matching `agentId`, `runId`, and `requestId`, with `status=completed` and the expected `result`.

No SQL, database/schema, configuration-format, or Room-file migration is required. A missing `runId`, previously accepted leniently, is now rejected. The rejection uses `error.code=agent_message_rejected` and one of these fixed reasons: `response_run_id_required`, `response_run_mismatch`, `response_result_conflict`, `response_result_store_failed`, or `response_waiter_owner_mismatch`.

The `runId` requirement above applies to reliable command `Response` messages, not confirmations. A confirmation request carries no run id; its decision is owned by the captured connection/request and original sender. A local control-plane wait timeout is not remote cancellation: late matching ACK/status/Response may still advance the run receipt, while `not_sent` means only channel-send failure and is never replayed.

By accepted D06 behavior, a Hub restart loses synchronous waiters, OAuth sessions, and pending sessions. Do not re-execute commands to restore HTTP waits. Durable SQLite results and the Agent ledger remain.

To roll back, pause new requests, restore only the old Hub binary, and continue using the current database and Agent ledger. Never overwrite new results with an old database backup. The old binary reopens the Response ownership defect, so rollback is not risk-free.

### Hub connection-generation cutover

Use this procedure when moving to a Hub build with generation-safe WS/SSE connection admission:

1. Pause new Hub calls and drain in-flight requests. If a request cannot be drained, record its `runId` and retain the corresponding Agent transport-ledger entry.
2. Stop the Hub. Back up the actual configuration and SQLite database, preserving matching `-wal` and `-shm` files when present.
3. Deploy the verified Hub artifact and restart it. Reconnect each Agent; every SSE reconnect must use a fresh, non-empty `connectionId`.
4. Verify `/v1/info`, `/v1/agents`, one current Heartbeat/ack path, one stale lifecycle rejection, and one matching stale reliable result. A Hub restart may lose in-memory connections, waiters, and sessions; do not re-execute commands to restore HTTP waits.

No SQL, database/schema, wire, or configuration-format migration is required. The Hub retains only the current in-memory connection per registered Agent; a replacement retires the prior Room lease and stream under the same generation boundary.

To roll back, pause new calls, stop the new Hub, replace only the Hub binary with the previous verified artifact, and continue using the latest database and Agent ledger. Never restore an older database backup over newer results. The previous binary reopens the pre-generation connection race, so rollback restores that risk.

### WP3 storage recovery and rollback

Storage recovery is not an effect rollback procedure. Before restoring any
Hub database, Agent configuration, transport ledger, or audit file, stop every
process that owns it (Hub and the affected Agent) and keep a read-only copy of
the current data. Do not restore an older copy over newer deduplication,
result, conflict, or unknown evidence.

Hub SQLite uses a transactional schema migration at `user_version=1`. Before a
migration or completed-result retention compaction, Hub creates a private
consistent snapshot beside the database as `.pre-migration.bak` or
`.pre-retention.bak`. Preserve the current database and matching `-wal`/`-shm`
files before any manual operation. Prefer rolling back only the binary while
continuing with the newest database; use an older snapshot only as a deliberate
data-recovery operation after comparing run identities and preserving the
newer database for evidence. A restored snapshot cannot retract a command
already delivered to an Agent or undo an external side effect.

Agent process history is stored durably in the private `process.sqlite3` and
uses that store's retention behavior. The process store is created fresh:
legacy `jobs.sqlite3` is intentionally left untouched and inaccessible to the
new process lifecycle. There is no legacy Job-history migration or
`jobs.sqlite3` migration-backup procedure.


Agent config writes retain bounded private backups under `backups/` and use a
setup journal for secret-reference replacement. If journal hashes or file
types do not describe one of the expected before/after states, startup fails
closed with a recovery conflict. Stop the Agent, preserve the journal and
current files, and resolve the conflict from verified copies; do not delete
the journal, force a side, or expose secret contents in diagnostics.

The transport ledger is owner-bound and file-locked. A corrupt record or torn
final line fails closed and preserves raw bytes in its private `.recovery`
sidecar; compaction may leave the prior ledger in `.backup`. Keep those raw
artifacts for diagnosis and do not drop or truncate the ledger to make startup
pass. Legacy unowned records remain `LegacyUnowned`: they are not
auto-reconciled, executed, or used to disclose a result. Recovery is an
operator-led review of preserved raw records and newer owner-bound evidence;
never delete or truncate deduplication evidence to bypass an ownership error.
Agent audit JSONL is best-effort and rotates at 8 MiB, retaining the current
file and one `.1` backup. A rotation or write failure is audit loss, not proof
that a command, result, or side effect is absent. After any restore, reconnect
the owning processes and inspect process state; `unknown_after_restart`,
`unknown`, `detached`, and a local waiter timeout must not be converted into
`cancelled` without independent termination evidence.


## v0.9 acceptance checklist
This section is historical v0.9 release evidence, not the current contract or
current verification entrypoint. Preserve its recorded version/counts when
consulting that release; use the repository verification and current smoke
sections above for present artifacts.

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

agentic-gpt --version
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info \
  --config ~/.agentic_gpt/config.json \
  --arguments '{}'
```

Expected contract:

- `agentic-gpt` reports `0.9.0`; Hub mode also requires `agentic-gpt-hub 0.9.0`.
- Normal local/tunnel surfaces expose 23 tools and Room exposes 34.
- `mcp.batch`, `mcp.callTool`, `job.get`, `job.list`, and `job.cancel` are present.
- `process.batchExec`, managed `session.*`, and `process.get/list/kill` are absent.
- `agent.info.execution.jobs` and `agent.info.mcp.concurrency` are present.
- MCP concurrency reports global limit 8 and per-server limit 2.
- Local Unix MCP uses a `0700` runtime directory and `0600` socket.
- Local Unix and tunnel descriptor/schema revisions match.
- A fresh hidden worker recovers a resumed call before `initialize`, preserves the original id, and remains alive.
- `config.example.json` parses strictly, validates for Standalone, and contains no usable credentials.

A no-side-effect `mcp.batch` smoke can use duplicate call ids. It must fail before confirmation/downstream connection and write one aggregate `validation_rejected` audit with no child calls:

```bash
agentic-gpt local call mcp.batch \
  --config ~/.agentic_gpt/config.json \
  --arguments '{
    "calls": [
      {"id":"dup","serverId":"configured-server","toolName":"probe","arguments":{}},
      {"id":"dup","serverId":"configured-server","toolName":"probe","arguments":{}}
    ],
    "waitSeconds": 0
  }'
```

Expected error: `mcp_batch_failed` with message prefix `mcp_batch_call_id_duplicate`.

Before a Hub release tag, validate [`openapi/hub.yaml`](../openapi/hub.yaml) with the intended Actions importer. Tagging, deployment, migration, and connector restart remain separate actions.

## Safety invariants

- Tunnel, Hub, Agent, and ntfy credentials never appear in argv, safe summaries, reports, or audit payloads.
- OpenAPI exposes only GPT Actions endpoints; OAuth and confirmation callbacks stay outside it.
- Safe summaries contain counts/coarse modes, not secrets or complete private path lists.
- Agent-local confirmation denial or confirmation-decision timeout is final; this is distinct from a Hub/HTTP/MCP waiter timeout.
- Long work uses managed Processes and bounded waits.
- A bounded Hub/HTTP/MCP waiter timeout ends only the local wait, not the remote Process. Late remote knowledge may update the run receipt; `not_sent` is only a proven channel-send failure and is excluded from replay.
- Standalone reporting is optional and reporting-only, never a hidden shared command dependency.
- Invalid live config keeps the last valid subset; startup identity changes require restart.
