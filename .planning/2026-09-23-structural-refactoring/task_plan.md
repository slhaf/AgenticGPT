# Structural refactoring implementation

## Goal
Implement the source-grounded structural seams identified in the 2026-09-23 whole-repository reassessment, including WP4-B Protocol internal organization. Preserve existing product scope, five-crate topology, wire/HTTP/MCP semantics and resource ownership. Add a crate/design pattern only if a concrete seam demonstrably needs it; current evidence supports none.

## Inputs and constraints
- Source and original six-goal reassessment: `.planning/2026-09-23-architecture-reassessment/findings.md`, `docs/architecture/refactoring-plan.md` conditional packages.
- User explicitly authorizes implementation and internal Protocol organization. WP-T test cleanup standards remain undecided; no opportunistic mass test deletion.
- Preserve unrelated existing change in `.planning/2026-09-22-wp-r-remote-room-closure/room-contractdocs-findings.md`. No production deployment or destructive user-data migration.
- Main owns integration, reproducible behavior proof, cross-slice contracts, validation and a commit for each coherent delivery phase. Parallel workers own disjoint files and skip formatters/linters/build/tests/commits. Independent work is split into separately committed Rust, release, and Android subphases; architecture/status documentation is the integration subphase, rather than one mixed implementation commit.

## Phase status
1. Contract/baseline — complete: independent file ownership and cross-cutover APIs frozen (`0b46d64`); deterministic pre-fix Job reload scenario reproduced Rejected vs Completed (`07f42df`, artifact://569).
2. Rust ownership subphase — complete and committed `4381e82`: Agent Process/config/Skill/path, Hub neutral projection/Apps membership and Protocol WP4-B. `cargo check --workspace`, targeted reload regression, `cargo test --workspace` (668 passed, 1 ignored), and bounded live parity passed.
3. Release gate subphase — complete and committed `3603ffc`: same-SHA tag preflight and version authority documentation. Local mismatched-tag rejection and matching `v0.9.1` full preflight/live gate passed (artifact://643); hosted GitHub publishing/cross-build not run.
4. Android local subphase — source complete and committed `7ecb87d`: one transition owner, atomic overdue claim, explicit hard failure/typed overdue and persisted visible degraded semantics. `:shared:jvmTest` passed after final policy (artifact://694); focused independent review found no remaining patch-introduced issue. Android app host/assemble/OS/device remain unavailable because SDK empty and no attached device.
5. Integration documentation/evidence — in_progress: update existing configuration/architecture status, affected caller contract notes and bounded evidence; commit once source cutovers and review freeze.
6. Delivery cleanup — pending: after smoke, remove owned throwaway probe/worktree, verify staging excludes user change, record proof and limits in final ledger/commit.

## Cross-slice contracts
- Protocol writer may reorganize only `crates/agentic-gpt-protocol`, retaining its existing root public names, serde tags/defaults/bytes and dependency set. No API rename or new wire feature. Other writers consume current root names unchanged. Pure `HubCommand::wire_name()` is optional but, if added, tell Main before Hub/Agent callers migrate.
- Agent operation writer owns `operation.rs`, `local_service.rs`, `stdio_server.rs`, optionally a new Agent-private module and affected Agent-local tests; NOT `jobs.rs`, `config.rs`, `main.rs`, `supervisor.rs`. Select one representative shared operation family (Process or MCP) and keep per-ingress auth/framing/hooks/result shapes. Wire conversion at Hub ingress only; do not invent a universal dispatcher.
- Agent config writer owns `jobs.rs`, `config.rs`, `main.rs`, `supervisor.rs`, narrow associated tests. Must first reproduce current admission/async reload behavior. Document a single effective config snapshot boundary, preserve policy/path/confirmation and startup resource rules; do not change `operation.rs` or `local_service.rs`.
- Hub writer owns Hub crate Rust only, not Protocol. One neutral Job/info projection between HTTP/Apps MCP adapters, profile-aware advertised/callable mapping, no HTTP/MCP auth or error schema merging. Retain Hub owner/generation/receipt/Room invariants.
- Console writer owns Console Kotlin only, Android local. Room remains authority; independent Desktop/Web placeholders and Hub product scope untouched. Overdue policy must be explicit, not silently ignored; OS behavior proof requested at integration.
- Release writer owns `.github/workflows`, release scripts, release/development documentation and contract gate if necessary; not Rust crates or architecture docs. Tag publication must be gated on chosen current Rust/schema/live checks, while preserving three-binary artifact set. No real tag or publish.
- Main owns architecture/configuration docs and planning ledger. Agents communicate before any overlap; first-wave agents perform no validation.
