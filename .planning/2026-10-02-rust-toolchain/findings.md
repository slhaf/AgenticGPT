# 核实结果

- origin/main 与本地 HEAD 均为 2911a66c42f25bd096af146cc3821c7b78ab1406，工作区干净。
- CI run 36973154852 成功，原始日志确认 Rust 1.99.0；36970377495 日志确认 fetch_update 弃用导致 strict Clippy 失败。
- 根 manifest 无 rust-version，无工具链文件，主 CI 使用 dtolnay/rust-toolchain@stable。
- 选择已在修复后本地和 CI 验证过的 1.99.0；这是开发/CI 基线，不是 MSRV。
- rustup 官方说明：rust-toolchain.toml 自动选择及安装指定组件；cargo +stable 可以绕过文件验证升级。
- https://rust-lang.github.io/rustup/overrides.html

- 实际 rustup show 确认 active because rust-toolchain.toml，rustc commit b940084d7eb6a299eb4bfeb8e34901bc051e7ac4，与成功 CI 相同。
- 主 CI gate 名称/顺序与命令不变，latest-stable 检查单独 workflow，不增加每次 push 的全链成本。
