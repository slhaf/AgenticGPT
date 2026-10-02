# Rust 工具链同步

目标：本地与主 CI 使用同一明确版本，保留低成本 latest-stable 验证；不承诺 MSRV、不改 Rust 代码。

- complete：核实 HEAD、CI 与既有调查。
- complete：最小配置改动。
- complete：fmt/check/strict Clippy/tests/build/live parity 后提交。

下一步：本地提交；推送后才会启用新的远程 workflow。
