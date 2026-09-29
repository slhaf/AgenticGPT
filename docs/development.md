# 开发

本文档记录源码开发、本地验证、CI 和 release 发布流程。正常安装和使用请从主 [README 中文版](../README.zh-CN.md) 开始。对应页面：[开发说明](development.zh-CN.md)。

## 修改前先确认架构

从[架构指南](architecture/README.md)开始：现状部署与模块边界、证据化诊断、目标架构草案、工程规则和分批重构计划。Agentic 是面向上层 Agent 的受控执行基础设施，不是通用 Agent Runtime。文档明确区分当前行为与建议规则；草案不代表代码已完成迁移。

改变状态所有权、依赖方向、公共合同、权限、持久化或部署的修改，必须同步相关架构文档。历史迁移/发布说明仍保留其版本基线，不作为当前架构权威。

## 从源码开发

开发时可以把二进制命令替换成 Cargo package 命令：

```bash
cargo run -p agentic-gpt-hub -- init
cargo run -p agentic-gpt -- config init
cargo run -p agentic-gpt -- run
```

## 验证

完整的本地验证顺序与当前 CI policy 对齐：

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p agentic-gpt -p agentic-gpt-hub
python3 -m venv target/contract-venv
target/contract-venv/bin/python -m pip install "PyYAML" "jsonschema[format]>=4.25,<5"
target/contract-venv/bin/python scripts/check_contract_parity.py
```

Parity gate 默认使用已构建的 `target/debug/agentic-gpt` 与
`target/debug/agentic-gpt-hub`，也可通过 `--agent-bin PATH` 和
`--hub-bin PATH` 指定二进制。它会运行隔离的 loopback/private-home 进程；
schema 验证与 live behavior 是分开的检查。合同环境故意放在被忽略的
`target/contract-venv` 下。

严格 Clippy（`-D warnings`）仍是 CI policy，可能报告现有 finding。上面是
明确的验证命令，不表示当前仓库或所有外部 release 检查都已通过。

## 构建和发布

### 本地 release preflight（不会发布）

运行 tag 打包之前使用的同一个非发布 gate：

```bash
bash scripts/release-preflight.sh
```

脚本会运行格式检查、workspace check、workspace test、Agent 与 Hub 构建，
以及 OpenAPI schema/live contract parity gate；它会创建或复用
`target/contract-venv` 来安装 parity 依赖。若要在一个已经 checkout 到 tag
的 commit 上本地检查 tag/version 配对，可运行：

```bash
RELEASE_TAG="$(git describe --tags --exact-match)" bash scripts/release-preflight.sh
```

preflight 不会把仓库已知的严格 Clippy debt 变成永久的发布阻断；严格
Clippy 仍由 CI 强制执行。这两个命令都不会发布 artifact，也不需要 release
credential。

### 独立的版本权威与 artifact 配对

Rust、OpenAPI contract 和 Console 是彼此独立的 release surface：

- Rust release identity 是 `crates/*/Cargo.toml` 中 `agentic-gpt` 与
  `agentic-gpt-hub` package 的共同版本。它们的 CLI `--version` 输出来自
  对应 Cargo package version。Rust release tag 使用 `v<该版本>`；tag
  preflight 会拒绝不匹配的 tag。每个 archive 仍包含
  `agentic-browser-host`，但它不是第二个 Rust release-tag authority。
- HTTP contract 的权威是 `openapi/hub.yaml`，包括其中的 `info.version`、
  paths、schemas 和 responses。它不会从 Rust tag 自动推导。
  `scripts/check_contract_parity.py` 会针对同一 commit 检查这个 contract
  以及 Agent/Hub 的 live behavior。
- Console package version 由 Console 各 Gradle target 自己负责，而不是
  这个 Rust workflow：Android 使用
  `console/androidApp/build.gradle.kts` 的 `versionCode`/`versionName`，
  desktop distribution 使用 `console/desktopApp/build.gradle.kts` 的
  `packageVersion`。Console version 可以独立演进；Rust release workflow
  不构建或发布 Console。`console/gradle/libs.versions.toml` 是依赖/plugin
  的版本权威，不是 Rust release number。

本地多目标 Linux release 仍使用 `cross`：

```bash
cargo install cross --git https://github.com/cross-rs/cross
./scripts/dist-linux.sh
```

产物写入：

- `dist/x86_64-unknown-linux-gnu/agentic-gpt`
- `dist/x86_64-unknown-linux-gnu/agentic-gpt-hub`
- `dist/x86_64-unknown-linux-gnu/agentic-browser-host`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt-hub`
- `dist/aarch64-unknown-linux-gnu/agentic-browser-host`

发布时创建并推送 `v<agentic-gpt/agentic-gpt-hub Cargo version>`。不要从旧
release note 复制数字 tag；preflight 会把实际 tag 与 checkout 中的 Rust
package version 比对：

```bash
rust_version="$(cargo metadata --no-deps --format-version=1 | python3 -c 'import json,sys; packages={p["name"]:p["version"] for p in json.load(sys.stdin)["packages"]}; print(packages["agentic-gpt"])')"
git tag "v${rust_version}"
git push origin "v${rust_version}"
```

每个 target archive 保持现有的三个 binary、名称和内容：

- `agentic-gpt-x86_64-unknown-linux-gnu.tar.gz`
- `agentic-gpt-aarch64-unknown-linux-gnu.tar.gz`
- `SHA256SUMS`

## CI

GitHub Actions 会在 push 和 pull request 到 `main` 时运行 CI：

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- 严格 `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- 创建被忽略的 `target/contract-venv`，安装 `PyYAML` 与 `jsonschema[format]>=4.25,<5`。
- 使用已构建 binary 和隔离的 loopback/private-home 进程运行
  `python3 scripts/check_contract_parity.py`。

Release workflow 与 branch/PR CI 分开。每一个 `v*` tag 都会在 tag commit 上
启动 `preflight` job；Linux 打包和发布 job 通过 `needs: preflight` 依赖它。
因此仅推送 tag 不能绕过 release preflight，任何失败都会阻止打包和发布。
Tag workflow 覆盖上述 fmt/check/test/build、version pairing 和 schema/live
parity 检查；它不宣称严格 Clippy 已通过、ARM runtime 行为、外部 Actions
importer、Console build 行为或生产部署，这些仍是独立边界。

本次文档更新没有运行 GitHub Actions、cross 编译、外部 importer 或发布。
现有按版本保存的 release notes 继续作为历史记录，特意保持不变。