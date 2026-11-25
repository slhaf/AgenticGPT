# 实施记录

- 两边起点 AgenticGPT 9aef753，fork f48e717。fork 已有未跟踪的 package-lock.json，保留用户文件。
- 现有 Agentic 运行时绑定官方 node_repl、Desktop/APT 发现和 Native Messaging host。fork 的后台使用单个 nativePort，已有页面树、元素引用、输入、截图和 offscreen worker 入口。
- 默认 bwrap 环境无法挂载 /proc。读写、提交和验证命令使用任务范围内的批准执行。
- 协议与计划已提交 6c0d633。两侧并行实施，Agentic 内嵌手册与发布脚本随运行时一同提交。
- 已核实配置的构建机可达且有 Chromium 158。实际多 profile 测试可以在构建机隔离 HOME/XDG 环境中执行。
- Agentic 内嵌 Markdown、配置目录及 TUI 已改为监听地址和配对密钥。删除旧 host、包分发和 PoC 的发布入口。远程 Cargo metadata 已更新锁文件，保留依赖的版本没有变化；发布脚本语法通过。
- 临时验证器位于 /tmp/agentic-browser-verify.1xcWxl/verify.mjs，构建机临时目录 /tmp/agentic-browser-check.oDoG0Q。计划真实运行两个 Agentic、两个 profile、扩展配置 UI、状态、租约互斥、执行中断线和重开浏览器。
- Agentic 远程 check、build、test（498 通过）和严格 Clippy 均通过。保留了离线手册、本地 MCP、跨 WebSocket 的多模态结果和脚本正文不进入审计的保障。
- Agentic Browser 扩展锁定依赖安装、TypeScript 检查和 Chrome MV3 生产构建通过。第一次真实联调已通过离线手册、通过实际设置页面配对、弹窗连接状态和重启 profile 保持身份。临时验证器调整为识别工具结果的 isError，继续验证互斥及 REPL。
