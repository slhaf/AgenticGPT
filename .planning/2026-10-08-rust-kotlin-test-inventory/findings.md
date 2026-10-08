# Findings & Decisions

## Requirements
- Scope: all Rust and Kotlin test cases in the repository; count by case, never by file.
- Classify using the repository's five-tier testing standard: fixed content, mechanical handling, independent logic, internal chain, actual execution.
- Report counts separately by language and tier. Record review-worthy cases among tiers 1–3, low-value cases among tiers 4–5, and mixed-scope cases with the behaviors they exercise.
- Ambiguous cases that cannot be responsibly assigned go to a review list with their test content summarized; continue the rest of the inventory.
- Read-only audit; only the selected `.planning/2026-10-08-rust-kotlin-test-inventory/` records may be updated.
- Verification: static inventory, Rust `cargo test --workspace -- --list`, Kotlin Gradle discovery per enabled test source set; do not run the full matrix.
- Attempt cap: five rounds. Escalate if a Rust/Kotlin test source set cannot be read or enumerated.

## Research Findings
- `Cargo.toml` defines five workspace crates: `agentic-gpt`, `agentic-gpt-hub`, `agentic-gpt-protocol`, `agentic-apply-patch`, and `agentic-browser-host`; all are in scope.
- CodeGraph located tests in inline Rust modules and integration test files, but its symbol response was partial; use Cargo's `--list` plus source review for a complete census.
- `agentic-gpt/tests/` contains four discovered integration test files: `standalone_supervisor.rs`, `standalone_http_mcp.rs`, `local_control.rs`, and `config_cli.rs`.
- `console/shared/build.gradle.kts` configures JVM, browser JS, browser Wasm, and Android Multiplatform host tests.
- Static search found four Kotlin test files so far: `commonTest/SharedCommonTest.kt`, `commonTest/AttentionTransitionPolicyTest.kt`, `jvmTest/SharedLogicDesktopTest.kt`, and `androidHostTest/SharedLogicAndroidHostTest.kt`. Compare these with Gradle's configured source sets and task discovery.
- Root console settings include `androidApp`, `desktopApp`, `shared`, and `webApp`; module inclusion alone does not establish test presence.
- Static case count for the four Kotlin files is 7 declarations: four in `AttentionTransitionPolicyTest`, one common `SharedCommonTest.example`, one JVM `SharedLogicDesktopTest.example`, and one Android-host `SharedLogicAndroidHostTest.example`. Common declarations are counted once rather than repeated per target.
- Preliminary classification: the four attention-policy tests exercise independent state-transition rules and boundary cases (tier 3; likely worth retaining for user review). The three `example` tests only assert `1 + 2 == 3` (tier 1; candidate for cleanup). Final classification is pending source-set/discovery cross-check.
- Kotlin Gradle reconciliation: four included modules; `shared` declares `commonTest` (5 cases), `jvmTest` (1), and `androidHostTest` (1). No `jsTest`/`wasmJsTest`-specific source files were found; common tests are shared across those targets. `androidApp`, `desktopApp`, and `webApp` have no test sources. Seven unique declarations total; do not multiply common tests by platform executions.
- Kotlin case details: 4 attention transition tests exercise due-time/status transition boundaries (tier 3, recommend retaining for review); 3 `example` tests assert only `1 + 2 == 3` (tier 1, low-value cleanup candidates). No mixed or ambiguous cases.
- Rust Cargo discovery completed with 827 cases across five crates: `agentic-apply-patch` 2, `agentic-browser-host` 10, `agentic-gpt` 654 (619 unit + 35 integration across five integration binaries), `agentic-gpt-hub` 138, and `agentic-gpt-protocol` 23. No doctests were listed. Classification and per-case reconciliation remain in progress.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Start with CodeGraph because the repository has `.codegraph/` | Repository instructions require it before code search |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Planning template edit initially retained its old footer | Removed duplicate footer from the planning file |

## Resources
-
