# Agentic Browser 直连改造

目标是交付 Agentic 自有的 Chrome 扩展、可与其互通的 Agentic Browser 工具，以及配套手册和安装指南。扩展执行 JavaScript，Chrome profile 是互斥单元。每个阶段提交 Conventional Commit。

- [complete] 定义 Browser 直连约定和两侧状态所有者。
- [complete] 在 fork 交付 Agentic Browser。保留 Chrome 操作底层，重做配对 UI、连接、租约和 REPL，剔除原生桥、MCP 和其他工作流。
- [complete] 在 Agentic 接入扩展。替换官方内核和 Browser host，沿用 Browser 工具集、结果与审计，随程序嵌入手册。
- [complete] 验证两个仓库的发布入口和实际 Chrome 链路，记录提交与结果。

用户已确认多对多配对、整个 profile 独占和扩展生命周期。首版工具支持显式选择 browserId。Agentic 的 Rust 编译、测试和重型验证走既有 remote-build。

- [complete] 更新 MCP 初始化说明，核对 Browser 配置热重载。
- [complete] 将扩展配置移入工具栏弹窗，移除独立设置页并同步配对文档。

## 连接恢复与 SDK 扩展

- [complete] 让 Agentic 在绑定失败后按最新 Browser 配置重试，恢复后报告实际监听状态。
- [complete] 规范化和去重扩展配对地址，交付 Neko Chromium 的持久启动参数。
- [complete] 在构建机验证延迟绑定和配对，部署至 Room，冷启动 Neko 并从实际 Room MCP 连接器回传截图。
- [complete] 扩展 SDK 的标签导航、等待、iframe、定位器读取和表单动作，以及文档上下文中的 JavaScript。
- [complete] 扩展全页、区域及元素截图，增加坐标点击和滚动，验证大结果错误与图片回传。
- [complete] 同步内嵌手册，提交各阶段，构建最终产物并验证 Room 实际入口。

文件上传下载、剪贴板、历史和用户接管按实际用例另排。

## 浏览器对话框与控制台

- [complete] 监听和处理原生 JavaScript 对话框，验证触发动作、跨 REPL 处理和租约回收。
- [complete] 按参考实现的 `tab.dev.logs()` 输出近期控制台记录和未捕获异常。
- [complete] 同步内嵌手册，远程验证页面操作及发布产物，分阶段提交。
