# Agentic Browser 直连改造

目标是交付 Agentic 自有的 Chrome 扩展、可与其互通的 Agentic Browser 工具，以及配套手册和安装指南。扩展执行 JavaScript，Chrome profile 是互斥单元。每个阶段提交 Conventional Commit。

- [complete] 定义 Browser 直连约定和两侧状态所有者。
- [in_progress] 在 fork 交付 Agentic Browser。保留 Chrome 操作底层，重做配对 UI、连接、租约和 REPL，剔除原生桥、MCP 和其他工作流。
- [complete] 在 Agentic 接入扩展。替换官方内核和 Browser host，沿用 Browser 工具集、结果与审计，随程序嵌入手册。
- [in_progress] 验证两个仓库的发布入口和实际 Chrome 链路，记录提交与结果。

用户已确认多对多配对、整个 profile 独占和扩展生命周期。首版工具支持显式选择 browserId。Agentic 的 Rust 编译、测试和重型验证走既有 remote-build。
