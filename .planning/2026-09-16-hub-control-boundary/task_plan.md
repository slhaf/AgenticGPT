# Hub connection/dispatch boundary refactor

## Goal
Split Hub agent transport, lifecycle, and dispatch ownership; encapsulate pending and connection state; bind Room dispatch to the validated generation; preserve existing HTTP/MCP/wire, permission, timeout, persistence, and replay contracts except the specified Room target-selection fix.

## Phases
### Discovery
- [x] Inspect indexed Hub structure and call boundaries
- [x] Confirm baseline worktree and planning files
- **Status:** complete

### Stage One
- [x] Split agents transport lifecycle dispatch modules
- [x] Encapsulate dispatch pending ownership
- [x] Migrate callers and HubState fixtures
- [x] Relocate tests into owner modules
- [x] Verify stage one Hub tests
- [x] Commit stage one refactor
- **Status:** complete (`b86e98c`)

### Stage Two
- [x] Add lifecycle Connections owner and read projections
- [x] Pin Room dispatch target to validated generation
- [x] Migrate connection fixtures and add regression
- [x] Verify stage two targeted and full tests
- [x] Run real Hub adapter smoke
- [x] Commit stage two refactor
- **Status:** complete (`07e8e18`)

### Delivery
- [x] Review final boundaries and report evidence
- **Status:** complete

## Constraints
- Two implementation commits, one per stage; no framework/trait registry/new tool variant.
- Existing protocol request_id and runs command_type mappings remain authoritative.
- Private map ownership only in lifecycle/dispatch; no root re-exports or compatibility shims.
- Verify each stage before the next; actual smoke uses temporary DB/config only.

## Errors Encountered
| Error | Attempt | Resolution |
|---|---:|---|
| Renamed agents module was temporarily overwritten by a skeleton | Early extraction | Restored exact `HEAD:.../agents.rs` source before extraction; no user work was lost. |
| Broad fixture edit used stale post-edit line anchors | Stage Two migration | Re-read affected files and repaired test-support, Room, and notify fixtures before continuing. |
| First smoke script expected the wrong wire command type | Initial WebSocket smoke | Corrected expected `process.exec`; reset the isolated DB to avoid the intentionally unacknowledged failed-run replay, then reran cleanly. |
