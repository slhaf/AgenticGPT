# Task Plan: Rust and Kotlin Test Inventory

## Goal
Count every Rust and Kotlin test case by the repository's five-tier testing standard; document counts, review candidates, and ambiguous cases in the selected planning record only.

## Next Step

Planning records are complete; commit this phase.

## Current Phase
Phase 4

## Phases

### Phase 1: Requirements & Discovery
- [x] Confirm scope: Rust and Kotlin tests only
- [x] Confirm unit of count: independent test case
- [x] Confirm read-only boundary and ambiguity handling
- [x] Identify initial Rust/Kotlin test sources and configured Kotlin targets
- **Status:** complete

### Phase 2: Inventory & Classification
- [x] Enumerate each test case and classify by tested behavior
- [x] Record review candidates and mixed-tier coverage
- [x] Record discoveries in findings.md throughout
- **Status:** complete

### Phase 3: Completeness Verification
- [x] Cross-check Rust declarations with `cargo test --workspace -- --list`
- [x] Cross-check enabled Kotlin test source sets with Gradle task discovery
- [x] Reconcile case-level mixed inventories against final per-crate totals (323 `agentic-gpt` + 10 other Rust; 333 Rust mixed total)
- **Status:** complete

### Phase 4: Delivery
- [x] Record final counts and review lists in planning files
- [x] Commit planning records for this phase
- **Status:** complete

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Count test cases, not files | User explicitly selected per-case counting |
| Keep the audit read-only except planning records | User needs an inventory to decide later cleanup |
| Handle mixed coverage separately; unclear cases go to review | User specified this ambiguity policy |
| Attempt cap: 5 rounds | User explicitly set the cap |

## Errors Encountered
| Error | Resolution |
|-------|------------|

