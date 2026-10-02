# Repository Guidelines

## Project Structure & Module Organization

The root Cargo workspace contains five Rust crates: `crates/agentic-gpt` (Linux local-agent CLI), `crates/agentic-gpt-hub` (HTTP/WebSocket hub), `crates/agentic-gpt-protocol` (shared wire types), `crates/agentic-apply-patch`, and `crates/agentic-browser-host`. API contracts live in `openapi/`; operational and development notes are in `docs/`; release helpers are in `scripts/`.

`console/` is a separate Kotlin Multiplatform/Compose project. Shared UI and domain code belongs in `console/shared/src/commonMain`; platform integrations belong in `androidMain`, `jvmMain`, `jsMain`, or `wasmJsMain`. Host applications live in `androidApp`, `desktopApp`, and `webApp`. Keep generated output (`target/`, `console/build/`, `dist/`) out of commits.

## 开发指南与使用时机

新增功能或模块、修改现有实现，或审查涉及状态所有权、并发、配置、权限及公开契约的变更前，先阅读 [`docs/development/README.zh-CN.md`](docs/development/README.zh-CN.md)（[English](docs/development/README.md)），按相关章节核对代码放置、消费者、验证入口和完成标准。归档架构资料只供历史追溯，不作为当前任务清单。

准备发布预检、Linux 打包或推送发布标签时，阅读 [`docs/development/releasing.md`](docs/development/releasing.md)，区分本地验证和实际发布的副作用。

## Build, Test, and Development Commands

本地开发和主 CI 使用根目录 `rust-toolchain.toml` 指定的 Rust 版本与组件；在仓库中运行 `rustup show` 可安装并确认。该版本是可复现的开发/CI 基线，不是 MSRV 承诺，`Cargo.toml` 不因此新增 `rust-version`。升级时修改工具链文件并运行下方完整验证链。独立 `Latest stable Rust` workflow 每周一及手动运行严格 Clippy；其显式 `cargo +stable` 绕过锁定，失败应作为升级待办处理，不放宽主 CI 门禁。

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

## 文档

新建、修改或审查文档，以及修改面向模型的工具定义前，先阅读 [`docs/documentation-standard.md`](docs/documentation-standard.md)；该文件也是工具定义写法的规范。新增文档使用中文；修改现有中英文版本时，保持两者的功能事实一致。

用户可见功能、CLI／配置默认值、API／MCP 契约或工具描述、安全／确认／策略、界面流程、发布／迁移或文档示例变化时，检查并按需更新受影响的文档。若行为和已有文档契约均未变化，无须为内部实现改动强行修改文档。历史记录应标明适用版本；文档约定变化时同步修改本标准。

## Testing Guidelines

Place Rust unit tests beside implementation code in `#[cfg(test)]` modules; use `#[tokio::test]` for async behavior. Kotlin tests belong in the matching source set, such as `commonTest` or `jvmTest`, and test classes should end in `Test`. Add regression tests for bug fixes. No numeric coverage threshold is configured; prioritize policy, protocol, persistence, and transport edge cases.

New tests must demonstrate a distinct, consumer-observable behavior, boundary, error, state transition, protocol contract, or meaningful side effect that existing tests do not already establish. Assert outcomes rather than implementation call order, field forwarding, incidental/unpromised wording, or source layout; preserve exact wording assertions when the wording itself is an external contract. Keep higher-level tests when they prove integration or contract behavior that lower-level tests cannot. Do not use line coverage or similar inputs alone to judge test value.

Before deleting a test, identify its assertions and the failure it could catch. Record why that failure is already detected by named retained tests, or why the test only detects an implementation change without a behavior change. Similar inputs or overlapping coverage alone do not prove redundancy. Preserve tests for distinct boundaries, errors, side effects, and cross-surface contracts; if distinct failure detection cannot be ruled out, retain the test. These rules apply to all future test additions and deletions, not only refactoring.

## Commit & Pull Request Guidelines

Write short, imperative commit subjects. Existing history accepts both plain subjects (`Add skills support`) and scoped Conventional Commit forms (`feat(hub): ...`, `fix(android): ...`); use a scope when it clarifies the affected component. Keep commits focused. Pull requests should explain motivation and behavior changes, list verification commands, link related issues, and include screenshots for console UI changes. Call out OpenAPI, configuration, security-policy, or migration impacts explicitly.

## Security & Configuration

Never commit API keys, agent secrets, ntfy topics, local databases, or files from `~/.agentic_gpt`. Preserve conservative confirmation and path-policy defaults, and document any change that broadens command or filesystem access.
