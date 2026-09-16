# 架构调查发现

## 已核实
- 初始工作区无未提交变更。
- 根目录有 `.codegraph/`，调查优先使用 CodeGraph；宽泛查询召回不精准，后续按具体文件/符号定位。
- `Cargo.toml:1-9` 实际 workspace 为五个 crate：agentic-gpt、agentic-gpt-hub、agentic-gpt-protocol、agentic-apply-patch、agentic-browser-host。上下文中“三个 crate”描述已过时（本阶段不改上下文文件）。
- Console 同时存在共享代码与 Android 本地 attention 持久化/调度，需要区分真实 Hub 集成与本地功能。
- rust-analyzer 可用；Kotlin LSP 未配置。
- 现有 docs 主要为配置、接口、运维、standalone/browser 运行时和历史迁移；正式架构目录尚不存在。计划创建 docs/architecture/ 并从 development 中链接。

## 规模基线（2026-09-16）
`git ls-files` 列出已跟踪文件，Python 统计 `.rs/.kt/.kts/.js/.ts/.py/.sh` 的物理行，包含测试、注释和构建配置，不等于生产代码行数：

| 范围 | 文件 | 行 |
|---|---:|---:|
| crates/agentic-gpt | 67 | 69497 |
| crates/agentic-gpt-hub | 14 | 9965 |
| crates/agentic-gpt-protocol | 1 | 3223 |
| crates/agentic-apply-patch | 6 | 1146 |
| crates/agentic-browser-host | 2 | 866 |
| console（各源集及构建） | 51 | 2868 |
| example/agentic-tui-ux-demo | 3 | 2873 |
| experimental/chrome-control-poc | 1 | 365 |
| scripts | 3 | 238 |

大文件仅为调查入口，不直接作为缺陷证据。stdio_server.rs 6200 行；config_tui/pages.rs 4923 行；protocol/lib.rs 3223 行；browser_distribution.rs 3122 行；Hub mcp_server.rs 3108 行。

## 证据规则
正式调查记录应给出相对路径及关键符号；区分事实、推断、未验证，不把名称重复当作实现重复。

## 依赖与验证基线
- 已运行 `cargo metadata --no-deps --format-version 1 --offline`，成功。agentic-gpt 仅依赖两个本地库 protocol/apply-patch；Hub 仅依赖 protocol；browser-host 无本地 crate 依赖。browser-host 是独立进程边界，不是 agent 的 Rust 库依赖。
- Cargo target 列出执行端四个集成测试入口：config_cli、local_control、standalone_http_mcp、standalone_supervisor；此阶段未执行测试。
- 已阅读双语 development 文档，当前均介绍构建、CI、三个 release 二进制；适合作为长期架构文档入口。
