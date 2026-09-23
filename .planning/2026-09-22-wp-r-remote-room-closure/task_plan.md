# WP-R — Remote Room capability closure

## Goal
Implement the approved WP-R contract in docs/architecture/refactoring-plan.md194-258: expose required current Agent Room read and maintenance capabilities through Hub HTTP/MCP and Protocol, migrate repository callers, and retire replaced legacy commands with real migration instructions. Preserve D01–D08 and Agent ownership of files/Git/maintenance.

## User instructions
- Start WP-R next, after completed WP4-A.
- Add a separate future WP for low-necessity test cleanup. Its standards will later be considered against common testing practice; they are explicitly NOT decided now. Do not invent deletion thresholds, test ratios, or an immediate test purge.
- WP4-B remains a separate future internal Protocol organization task. Do not mix either future package into WP-R.

## Phases
1. Baseline and contract — complete: mapped current nine Agent operations and remote/caller seams; exact interface/ownership and migration inputs frozen in findings.md.
2. Implementation — complete: coordinated Protocol/Agent/Hub cutover, OpenAPI/current docs and live gate implementation integrated; final source build passed.
3. Verification — complete: actual live gate exit 0; workspace tests 667 passed/1 ignored; formatting check passed. Strict existing Clippy debt remains documented, not suppressed.
4. Delivery — in_progress: migration/rollback/current architecture updated; post-smoke temporary artifact cleanup complete; recording phase commits and acceptance evidence.

## Acceptance
- Current required read and maintenance operations have actual remote decode/dispatch/response evidence and bounded Agent-owned content.
- No active Room, Normal, ReportingOnly and stale connection cannot become unintended write/read targets.
- Agent path/symlink/Git/lock/expected-change and existing controlled maintenance lifecycle remain authoritative; no new generic process-confirmation channel; timeout is not cancellation.
- Current Full/Coordinator/Agent surfaces and OpenAPI/callers are consistent. Legacy paths retire only with replacement functionality and live/caller migration evidence.
- Preserve Room files/Git/journal on upgrades/rollback; no deletion migration, invented version/feature negotiation, shim or dual execution.
- No production SSH/tunnel deployment. Strict pre-existing Clippy debt stays separate; do not suppress or silently include broad lint cleanup.

## Ownership
Main owns decomposition, planning, cross-slice interface decisions, integration validation and all phase commits. Read-only scouts map Agent/Protocol and Hub/consumers in parallel; implementation owners will be assigned after interface selection. Workers skip builds, tests, formatters, linters and commits.
