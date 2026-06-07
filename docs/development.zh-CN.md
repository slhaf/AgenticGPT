# 开发

本文档记录源码开发、本地验证、CI 和 release 发布流程。正常安装和使用请从主 [README 中文版](../README.zh-CN.md) 开始。

## 从源码开发

开发时可以把二进制命令替换成 Cargo package 命令：

```bash
cargo run -p agentic-gpt-hub -- init
cargo run -p agentic-gpt -- config init
cargo run -p agentic-gpt -- run
```

## 验证

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
python3 -c "import yaml; yaml.safe_load(open('openapi/hub.yaml')); print('openapi yaml ok')"
```

## 构建和发布

本地多目标 Linux release 构建使用 `cross`：

```bash
cargo install cross --git https://github.com/cross-rs/cross
./scripts/dist-linux.sh
```

产物写入：

- `dist/x86_64-unknown-linux-gnu/agentic-gpt`
- `dist/x86_64-unknown-linux-gnu/agentic-gpt-hub`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt-hub`

推送版本 tag 会构建 Linux release archives 并发布 GitHub Release：

```bash
git tag v0.1.0
git push origin v0.1.0
```

Release archive 每个 target 包含两个二进制：

- `agentic-gpt-x86_64-unknown-linux-gnu.tar.gz`
- `agentic-gpt-aarch64-unknown-linux-gnu.tar.gz`
- `SHA256SUMS`


## CI

GitHub Actions 会在 push 和 pull request 到 `main` 时运行 CI：

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo test --workspace`
- 解析 `openapi/hub.yaml`，确认 OpenAPI YAML 可读取

## 说明

Release workflow 由匹配 `v*` 的版本 tag 触发。它会调用 `scripts/dist-linux.sh`，按目标平台打包两个二进制，生成 `SHA256SUMS`，并发布 GitHub Release。
