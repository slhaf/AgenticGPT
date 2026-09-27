# Repository Guidelines

## Project Structure & Module Organization

The root Cargo workspace contains five Rust crates: `crates/agentic-gpt` (Linux local-agent CLI), `crates/agentic-gpt-hub` (HTTP/WebSocket hub), `crates/agentic-gpt-protocol` (shared wire types), `crates/agentic-apply-patch`, and `crates/agentic-browser-host`. API contracts live in `openapi/`; operational and development notes are in `docs/`; release helpers are in `scripts/`.

`console/` is a separate Kotlin Multiplatform/Compose project. Shared UI and domain code belongs in `console/shared/src/commonMain`; platform integrations belong in `androidMain`, `jvmMain`, `jsMain`, or `wasmJsMain`. Host applications live in `androidApp`, `desktopApp`, and `webApp`. Keep generated output (`target/`, `console/build/`, `dist/`) out of commits.

## Build, Test, and Development Commands

- `cargo check --workspace`: type-check all Rust crates quickly.
- `cargo test --workspace`: run the full Rust test suite.
- `cargo fmt --all -- --check`: enforce CI formatting.
- `python3 scripts/check_contract_parity.py`: run the live cross-surface contract gate after building the Agent/Hub binaries and installing its Python dependencies; see `docs/operations.md` for the isolated environment setup.
- `cargo run -p agentic-gpt-hub -- init`: initialize a development hub.
- `cargo run -p agentic-gpt -- run`: launch the local agent.
- `cd console && ./gradlew :desktopApp:run`: run the desktop console.
- `cd console && ./gradlew :shared:jvmTest`: run shared JVM tests. Android, JS, and Wasm tasks are listed in `console/README.md`.
- `./scripts/dist-linux.sh`: create multi-architecture Linux release artifacts using `cross`.

## Coding Style & Naming Conventions

Use `rustfmt` defaults and idiomatic Rust naming: `snake_case` functions/modules, `PascalCase` types, and `SCREAMING_SNAKE_CASE` constants. Preserve camelCase JSON contracts through explicit Serde attributes. Kotlin uses four-space indentation, `PascalCase` types/composables, `camelCase` members, and lowercase package names under `work.slhaf.agentic.console`. Prefer shared code over duplicated platform implementations.

## Testing Guidelines

Place Rust unit tests beside implementation code in `#[cfg(test)]` modules; use `#[tokio::test]` for async behavior. Kotlin tests belong in the matching source set, such as `commonTest` or `jvmTest`, and test classes should end in `Test`. Add regression tests for bug fixes. No numeric coverage threshold is configured; prioritize policy, protocol, persistence, and transport edge cases.

New tests must demonstrate a distinct, consumer-observable behavior, boundary, error, state transition, protocol contract, or meaningful side effect that existing tests do not already establish. Assert outcomes rather than implementation call order, field forwarding, fixed wording, or source layout; keep higher-level tests when they prove integration or contract behavior that lower-level tests cannot. Do not use line coverage or similar inputs alone to judge test value.

Before deleting a test, identify its assertions and the failure it could catch. Record why that failure is already detected by named retained tests, or why the test only detects an implementation change without a behavior change. Similar inputs or overlapping coverage alone do not prove redundancy. Preserve tests for distinct boundaries, errors, side effects, and cross-surface contracts; if distinct failure detection cannot be ruled out, retain the test. These rules apply to all future test additions and deletions, not only refactoring.

## Commit & Pull Request Guidelines

Write short, imperative commit subjects. Existing history accepts both plain subjects (`Add skills support`) and scoped Conventional Commit forms (`feat(hub): ...`, `fix(android): ...`); use a scope when it clarifies the affected component. Keep commits focused. Pull requests should explain motivation and behavior changes, list verification commands, link related issues, and include screenshots for console UI changes. Call out OpenAPI, configuration, security-policy, or migration impacts explicitly.

## Security & Configuration

Never commit API keys, agent secrets, ntfy topics, local databases, or files from `~/.agentic_gpt`. Preserve conservative confirmation and path-policy defaults, and document any change that broadens command or filesystem access.
