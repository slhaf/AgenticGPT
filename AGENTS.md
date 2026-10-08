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

## 测试指南

Rust 单元测试应与实现代码放在一起，位于 `#[cfg(test)]` 模块中；异步行为使用 `#[tokio::test]`。Kotlin 测试应放在对应的源集（如 `commonTest` 或 `jvmTest`）中，测试类名应以 `Test` 结尾。修复缺陷时应验证相关回归；需要新增测试时，遵循下方分级规则。目前未设定数字化的覆盖率门槛；应优先测试策略、协议、持久化和传输方面的边界情况。

### 测试分级

本项目中，按照测试覆盖范围，将其划分为五档：

| 档位 | 实际验证的内容 | 典型例子 |
| --- | --- | --- |
| **① 固定内容** | 写死的文本、常量、声明 | 提示文字、默认值、字段清单 |
| **② 机械处理** | 直接映射、搬运、拼装 | 枚举转错误码、字段转发、格式转换 |
| **③ 独立逻辑** | 一个组件内的规则和运算 | 策略判断、算法、状态机、调度规则 |
| **④ 内部链路** | 多个组件接起来后的行为 | 请求进入后经过解析、策略、确认，得到结果 |
| **⑤ 实际执行** | 链路与真实运行资源发生交互 | 真正启动进程、写入数据库、发送通知，以及检查实际副作用 |

在编写、审查测试时，按照上述规则区分，其中：

- 第 1、2、3 档默认不主动编写
- 第 4、5 档允许 Agent 主动编写
- 前 1、2、3 档如果 Agent 认为有编写价值，需要主动与用户商讨并获取许可

## Commit & Pull Request Guidelines

Write short, imperative commit subjects. Existing history accepts both plain subjects (`Add skills support`) and scoped Conventional Commit forms (`feat(hub): ...`, `fix(android): ...`); use a scope when it clarifies the affected component. Keep commits focused. Pull requests should explain motivation and behavior changes, list verification commands, link related issues, and include screenshots for console UI changes. Call out OpenAPI, configuration, security-policy, or migration impacts explicitly.

## Security & Configuration

Never commit API keys, agent secrets, ntfy topics, local databases, or files from `~/.agentic_gpt`. Preserve conservative confirmation and path-policy defaults, and document any change that broadens command or filesystem access.
