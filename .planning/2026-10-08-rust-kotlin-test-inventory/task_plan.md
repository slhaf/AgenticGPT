# Task Plan: Rust and Kotlin Test Inventory

## Goal
Count every Rust and Kotlin test case by the repository's five-tier testing standard; persist a complete case/name/behavior inventory, final counts, and review candidates in planning records only.

Await the user’s review of the retention, cleanup, and uncertain-value candidates; no test or implementation edits are in scope.

## Current Phase
Phase 6

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

### Phase 5: Exhaustive Case Index
- [x] Record the technical classification of all 333 Rust MIXED cases across the mixed catalogs and lower-only routing candidates
- [x] Record all seven Kotlin declarations with tier and behavior
- [x] Persist every Rust single-tier declaration with path, function, tier, and behavior
- [x] Reconcile all 834 named cases once against Rust and Kotlin discovery totals
- [x] Commit the exhaustive case-index follow-up
- **Status:** complete

### Phase 6: Lower-Tier Mixed-Case Review
- [x] Identify every MIXED case containing only T1–T3 components
- [x] Move cases worth retaining into `review-candidates.md`; remove all lower-only cases from mixed-review catalogs
- [x] Keep MIXED cases containing T4 or T5 and preserve their asserted-behavior descriptions
- [x] Reconcile candidate and mixed counts; record unclear value judgments for the user
- [x] Commit planning-only changes; do not alter test or implementation files
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

