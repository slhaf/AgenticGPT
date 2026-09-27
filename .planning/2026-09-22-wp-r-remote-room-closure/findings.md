# WP-R findings

## Confirmed scope
D01–D08 remain binding. Agent owns Room files, Git, schema/scaffold, reads and maintenance; Hub owns authenticated routing and receipts only. Current legacy commands are not a suitable final remote capability contract; new public names must represent actual existing local semantics, not silently reinterpret append/update.

## Starting evidence
WP4-A completed in c8341ca, d67dfce, 3020f61, 119acd0, 27d3be1. Final reported Rust suite664 passed/1ignored, real cross-entry gate passed; strict existing Clippy debt is explicitly not green. This is prior-package evidence, not proof of WP-R changes.

## Deferred testing work
User requested a dedicated future test-cleanup WP, with necessity standards to be considered later against common practice. No specific testing standard, deletion list, target size or placement relative to WP4-B is approved now. Current Rust physical-line estimate was approximately32.9% test code, not a whole-repository or test-quality measure.

## Research in progress
RoomAgentMap maps current Agent/Protocol operations. RoomHubMap maps Hub routing, external projections and current consumers. Both are pinned to the architecture-cleanup worktree and skip validation/edits.

## Current-source facts
- Main CodeGraph confirmed current read service functions: diary_active/read, notebook_recent/search/read, state_list/read. Agent owns bounded Markdown reads and rejects path/symlink escapes.
- Maintenance public service has status and submit. RoomMaintenanceSubmitRequest uses items1..5, optional local/workflow mode and optional u8 waitSeconds default0/cap30. submit holds the existing repository-write lock and invokes local/workflow execution; local execution preflights and rejects unexpected changed/staged paths.
- No MaintenanceWait request, maintenance.wait tool or public wait function was found in current Protocol/maintenance/stdio source. Existing workflow wait tests concern submit's lifecycle, not a separate API; do not invent one.
- Hub agents::request_room resolves one current Room DispatchTarget and passes it to request_target, retaining captured sender/owner. Reuse this path, not generic agent-id fallback.
- Future WP-T placeholder was committed as f8fd5a8; no test cleanup standard or deletion list selected.

## Frozen implementation contract
- Exactly nine operations mirror current local semantics: room.diary.active/read; room.notebook.recent/search/read; room.state.list/read; room.maintenance.status/submit. Shared existing Room*Request/Response DTOs remain authority.
- HubCommand has nine corresponding RoomDiaryActive/Read, RoomNotebookRecent/Search/Read, RoomStateList/Read, RoomMaintenanceStatus/Submit variants, each with request_id and ordinary nested payload. Serde type names match dot tool names. No new wire negotiation or flattening convention.
- HTTP uses POST /v1/room/<namespace>/<action> for all nine, retaining the existing Room JSON-body convention. No explicit agentId target; resolve/capture active Room lease through current request_active_room.
- Full MCP exposes these nine with current local required/default/bounds/annotations; Coordinator excludes and rejects them. Existing bootstrap/skills remain untouched.
- Recent/search keep their public names but migrate to current Markdown request/result semantics. The other legacy JSONL operations retire; old append/update/remove are not translated into maintenance.
- HTTP preserves existing room_value_response error projection and routing errors: inactive404, conflict409, transport timeout504; submit's waiting allowance cannot be shorter than its current bounded wait. Waiting never cancels maintenance.
- Hub does not acquire repository/files/Git ownership. Existing generic run receipts can contain bounded operation results; this must be documented rather than falsely promising that no response bytes ever persist on Hub.
- Maintenance has existing repository/slot/payload/clean-tree/preflight/expected-change/Git controls. No newly invented confirmation channel is added; preserve the actual local behavior, not a stronger promise inferred from plan wording.

## Implementation ownership
- RoomAgentWrite: Protocol plus Agent dispatch/admission/current error adapters and affected callers.
- RoomHubWrite: Hub routing, MCP schemas/dispatch, HTTP registration, run mappings and affected tests.
- RoomContractDocs: supported OpenAPI and current operational/migration/interface docs.
- RoomGateWrite: existing live parity script and any current corpus case migration; reuse fixtures, no new test framework.
- All skip validation/commits. Main handles cross-slice integration, real execution, evidence and commits.
