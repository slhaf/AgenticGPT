# 发布指南

本文只说明发布前检查、版本对应关系与 Rust/Linux 发布入口；源码开发和完整本地验证见[开发说明](README.md)。本指南描述仓库当前脚本与 GitHub Actions，不记录单次发布报告。

## 发布前：本地非发布检查

在仓库根目录运行：

```bash
bash scripts/release-preflight.sh
```

该[预检脚本](../../scripts/release-preflight.sh)不会创建 tag、构建 Linux 发行包或发布 GitHub Release；它会依次检查格式（`cargo fmt --all -- --check`）、检查并测试 Rust 工作区、构建 `agentic-gpt` 与 `agentic-gpt-hub`，再运行 OpenAPI schema 和真实进程的跨端契约检查。Python 环境默认放在 `target/contract-venv`：环境不存在时创建虚拟环境；无论新建还是复用，都会安装 `PyYAML` 与 `jsonschema[format]>=4.25,<5`。

若当前已检出候选标签对应的提交，可运行以下非发布检查：

```bash
release_tag="$(git describe --tags --exact-match)" &&
  RELEASE_TAG="$release_tag" bash scripts/release-preflight.sh
```

只有当前提交确实有 tag 时，上述命令才会继续预检。脚本要求 tag 以 `v` 开头，并确认去掉 `v` 后与 [`agentic-gpt`](../../crates/agentic-gpt/Cargo.toml)、[`agentic-gpt-hub`](../../crates/agentic-gpt-hub/Cargo.toml) 两个 Cargo 包的版本都相同。此模式仍只是预检，不发布产物。

## 版本权威与发布配对

Rust、HTTP 契约和 Console 各自拥有版本来源，不能把一个版本号视为其他部分的自动来源：

- **Rust 发布标签**：[`agentic-gpt`](../../crates/agentic-gpt/Cargo.toml) 与 [`agentic-gpt-hub`](../../crates/agentic-gpt-hub/Cargo.toml) 必须采用同一版本；发布标签为 `v<该版本>`。带标签的预检会校验配对。`agentic-browser-host` 随包发布，但不决定该标签。
- **HTTP/OpenAPI 契约**：权威定义是 [`openapi/hub.yaml`](../../openapi/hub.yaml)，包括 `info.version`、路径、schema 与响应；不会由 Rust 标签自动生成或更新。[`scripts/check_contract_parity.py`](../../scripts/check_contract_parity.py) 针对当前检出的代码检查 schema 与 Agent/Hub 运行行为。
- **Console**：独立于 Rust 发布工作流。Android 的版本字段在 [`console/androidApp/build.gradle.kts`](../../console/androidApp/build.gradle.kts)（`versionCode`、`versionName`）；桌面分发的 `packageVersion` 在 [`console/desktopApp/build.gradle.kts`](../../console/desktopApp/build.gradle.kts)。Rust 工作流不构建或发布 Console。[`console/gradle/libs.versions.toml`](../../console/gradle/libs.versions.toml) 管理依赖和插件版本，不是产品发布版本来源。

## Linux 产物

本地多目标 Linux 打包由 [`scripts/dist-linux.sh`](../../scripts/dist-linux.sh) 执行，要求已安装 `cross`：

```bash
cargo install cross --git https://github.com/cross-rs/cross
./scripts/dist-linux.sh
```

脚本对 `x86_64-unknown-linux-gnu` 与 `aarch64-unknown-linux-gnu` 执行工作区的 release 构建，并将三个二进制复制到各自 `dist/<target>/` 目录：`agentic-gpt`、`agentic-gpt-hub`、`agentic-browser-host`。此命令会编译并写入或覆盖本地 `dist/` 产物，但不会创建 tag 或发布 GitHub Release。

[`release.yml`](../../.github/workflows/release.yml) 将每个目标目录打成 `agentic-gpt-<target>.tar.gz`，内容为上述三个二进制，并生成 `release/SHA256SUMS`。它通过 `softprops/action-gh-release@v2` 发布两个压缩包与校验和文件。

## 触发实际发布

实际发布由 [`.github/workflows/release.yml`](../../.github/workflows/release.yml) 中的 `v*` 标签推送触发。确认准备发布的提交和版本配对后，按两个 Rust 包的共同版本计算标签并推送：

```bash
rust_version="$(cargo metadata --no-deps --format-version=1 | python3 -c 'import json,sys; packages={p["name"]:p["version"] for p in json.load(sys.stdin)["packages"]}; print(packages["agentic-gpt"])')" &&
  git tag "v${rust_version}" &&
  git push origin "v${rust_version}"
```

**创建 tag 会改变本地 Git 引用；推送 tag 会启动 GitHub Actions，并可能创建公开 GitHub Release、上传 Linux 压缩包和 `SHA256SUMS`。** 不要把本地预检或本地 `dist-linux.sh` 当成发布动作。

发布工作流在标签对应的提交上先运行 `preflight`，Linux 构建、打包与发布任务通过 `needs: preflight` 等待它成功；预检失败会阻止后续发布任务。推送标签本身不会绕过检查。

## CI 覆盖范围与证据边界

[`ci.yml`](../../.github/workflows/ci.yml) 对 `main` 的推送和以其为目标的 pull request 运行 `fmt`、工作区 `check`、严格 Clippy（`cargo clippy --workspace --all-targets -- -D warnings`）、工作区测试、Agent/Hub 构建和跨端契约检查。严格 Clippy 属于 CI 门禁；本地发布预检与标签发布工作流都**不运行 Clippy**，因此预检成功不能证明严格 Clippy 已通过，也不能据此判断是否存在警告。

发布工作流检查标签提交的格式、编译、测试、Agent/Hub 构建、标签与 Cargo 版本配对及跨端契约，再构建两个 Linux 目标的包。其结果只证明这些步骤在该次运行中通过；不证明 ARM 上的实际运行行为、外部 Actions 导入、Console 构建或生产部署。请以对应工作流运行记录为准，不将本指南或本地预检视为这些独立范围的验证。
