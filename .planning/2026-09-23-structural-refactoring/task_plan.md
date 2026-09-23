# Structural refactoring implementation

## Goal
Implement the source-grounded structural seams identified in the 2026-09-23 whole-repository reassessment, including WP4-B Protocol internal organization. Preserve existing product scope, five-crate topology, wire/HTTP/MCP semantics and resource ownership. Add a crate/design pattern only if a concrete seam demonstrably needs it; current evidence supports none.

## Inputs and constraints
- Source and original six-goal reassessment: `.planning/2026-09-23-architecture-reassessment/findings.md`, `docs/architecture/refactoring-plan.md` conditional packages.
- User explicitly authorizes implementation and internal Protocol organization. WP-T test cleanup standards remain undecided; no opportunistic mass test deletion.
- Preserve unrelated existing change in `.planning/2026-09-22-wp-r-remote-room-closure/room-contractdocs-findings.md`. No production deployment or destructive user-data migration.
- Main owns integration, reproducible behavior proof, cross-slice contracts, validation and each phase commit. Parallel workers own disjoint files and skip formatters/linters/build/tests/commits.

## Phase status
1. Contract/baseline — complete: independent file ownership and cross-cutover APIs frozen; deterministic pre-fix Job reload scenario reproduced Rejected vs Completed (artifact://569); baseline plan committed `0b46d64`.
2. Independent implementation — in_progress: Agent operation family, Hub neutral projection/Apps registry, Protocol WP4-B, Android local transition, release preflight and Agent Job config snapshot in disjoint owned files. Workers skip validation.
3. Integration — pending: apply dependent Agent ownership changes after the config seam, migrate callers and docs, run targeted smoke/behavior checks and one project-wide validation pass, commit coherent implementation boundaries.
4. Delivery — pending: post-smoke cleanup, independent code review, bounded limitations, final phase commit and evidence ledger.

## Cross-slice contracts
- Protocol writer may reorganize only `crates/agentic-gpt-protocol`, retaining its existing root public names, serde tags/defaults/bytes and dependency set. No API rename or new wire feature. Other writers consume current root names unchanged. Pure `HubCommand::wire_name()` is optional but, if added, tell Main before Hub/Agent callers migrate.
- Agent operation writer owns `operation.rs`, `local_service.rs`, `stdio_server.rs`, optionally a new Agent-private module and affected Agent-local tests; NOT `jobs.rs`, `config.rs`, `main.rs`, `supervisor.rs`. Select one representative shared operation family (Process or MCP) and keep per-ingress auth/framing/hooks/result shapes. Wire conversion at Hub ingress only; do not invent a universal dispatcher.
- Agent config writer owns `jobs.rs`, `config.rs`, `main.rs`, `supervisor.rs`, narrow associated tests. Must first reproduce current admission/async reload behavior. Document a single effective config snapshot boundary, preserve policy/path/confirmation and startup resource rules; do not change `operation.rs` or `local_service.rs`.
- Hub writer owns Hub crate Rust only, not Protocol. One neutral Job/info projection between HTTP/Apps MCP adapters, profile-aware advertised/callable mapping, no HTTP/MCP auth or error schema merging. Retain Hub owner/generation/receipt/Room invariants.
- Console writer owns Console Kotlin only, Android local. Room remains authority; independent Desktop/Web placeholders and Hub product scope untouched. Overdue policy must be explicit, not silently ignored; OS behavior proof requested at integration.
- Release writer owns `.github/workflows`, release scripts, release/development documentation and contract gate if necessary; not Rust crates or architecture docs. Tag publication must be gated on chosen current Rust/schema/live checks, while preserving three-binary artifact set. No real tag or publish.
- Main owns architecture/configuration docs and planning ledger. Agents communicate before any overlap; first-wave agents perform no validation.
