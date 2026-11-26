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

- Agentic 接入提交 3b5860c。扩展实现与重新命名提交 e81ef14，类型检查和 ZIP 生产构建通过，产物为 app/chrome-extension/.output/agentic-browser-0.1.0-chrome.zip。
- 最终 remote-build test 498 通过、0 失败，remote-build lint 通过。
- Chromium 158 中使用两个实际 profile 与两个隔离 Agentic 进程，验证设置页配对、弹窗状态、重启自动重连、profile 独占与跨 profile 并行、REPL 状态、元素引用与输入、结构化数据和截图、死循环超时与重置、执行中断线交接、空闲释放，以及跨 35 秒空闲窗口保留租约状态。
- 依据联调结果修正 Browser 生命周期日志错误码，并让启动监听沿用配置应用锁。已清理验证用独立 HOME、Chromium profile 与临时脚本。
- 更新 MCP 初始化说明，指引使用 Agentic Browser 手册、选择 browserId 和按 name 调用 REPL。Rust 格式和补丁检查通过。使用既有远程构建产物和隔离 HOME，通过实际 WebSocket 握手验证同一 Agentic 进程内热更新配对密钥、迁移监听端口及禁用后恢复 Browser 工具集。验证进程与临时目录已清理。
- 扩展 0cab010 将浏览器名称、多实例配对和连接状态合入工具栏弹窗，移除独立设置页。类型检查和 Chrome MV3 ZIP 构建通过。使用构建机的 Chromium 158 和两个隔离 Agentic 实例验证真实工具栏弹窗宽 380 px、多条连接可滚动且保存按钮始终可见。保存后两端识别同一 browserId，重新打开可读回配置并移除连接。验证进程和临时目录已清理。
- 用户确认修复顺序和 SDK 范围。Room 在保留配置内容的情况下热重新应用 Browser 监听，原 Tailnet 地址已恢复。Laptop Browser 自动上线且没有占用。Neko 启动配置已备份并写入持久扩展路径，等待新产物后冷启动。
- Agentic 监听重试及 listenerReady、扩展地址去重补丁进入远端构建。去重在保存时提示，后台只连接每个规范化地址的第一条已存配置。
- 配对去重提交 23d1e33。构建机通过锁定依赖的 TypeScript 和 ZIP 构建，实际 Chromium 验证旧重复项仅建一条连接、弹窗提示、移除后直接保存、同 profile 重启回收租约。
- Agentic 远端 fmt、严格 Clippy 和标准 Linux 发布构建通过。隔离网络命名空间内重现地址未配置时的真实绑定失败，地址加入后未修改配置即恢复，热更目标地址失败时旧监听运行，新地址出现后自动切换。
- Agentic 监听修复 9fc9cd6 已部署 Room，实际 connector 显示 listenerReady=true，Laptop 和 Room Neko 均恢复在线。Neko Chromium 从持久启动参数自动加载去重扩展并恢复同一 browserId。Room MCP 实际 browser.repl 返回原生 PNG，确认可渲染测试页面，已关闭测试标签页并释放租约。
- SDK 增量采用扩展 0.2.0 识别，WebSocket 协议仍为 v1。Frame 通过 documentId 绑定已选文档，操作和快照沿用脚本注入。CDP 评估绑定目标文档的主页面上下文，跨站 iframe 使用扁平子会话。视觉操作维持 CSS 像素坐标与 PNG 回传。
- 远程生产构建及 Chromium 158 联调通过。验证了标签导航和等待、语义定位及原生表单、三个 iframe（含两个相同地址的跨站文档）、文档求值与旧 frame 失效、坐标点击和滚动。MCP 通过三张原生 PNG 传回全页、区域和元素截图，超限返回 browser_result_too_large 后浏览器仍在线。
- 扩展 e29976b 与 Agentic 手册 52431cf 已部署。Room Neko 首次升级后出现新 manifest 版本沿用旧后台脚本的情况。通过调试接口核实实际脚本为 0.1.0 并显式重载扩展，运行中的脚本与 0.2.0 构建产物一致。
- 再次重启 Neko Chromium 后保留原 browserId 并自动上线。实际 Room MCP 可查阅新版手册，完成导航等待、定位和文档求值，返回 `image/png` 原生 `image` 块，裁图尺寸 700×250。测试标签页已关闭，租约释放。ZIP 和解压目录已从远程构建机取回。
- 对话框阶段提交 aaa001c。远程 Chromium 通过 MCP 验证点击和坐标动作触发原生弹框，跨 REPL 取消确认、读取和填写 prompt、处理连续 alert、拒绝过期句柄、取消或接受离开页面，以及打开弹框时释放租约。
- 控制台阶段提交 9cdca65，扩展版本 0.3.0。远程 Chromium 经 MCP 验证旧版过滤选项、等级别名、顺序、来源 URL 与时间戳、500 条缓存和默认 100 条。验证了未捕获异常、跨站 iframe 日志、标签隔离、弹框打开时读日志、导航后保留记录，以及原 SDK 的页面操作和截图。
- 内嵌手册已同步 0.3.0。提交后的扩展重新通过远程 TypeScript 检查和 ZIP 构建，ZIP 和解压目录已取回。Room Neko 当前在线版本为 0.2.0。

## Browser 0.4.0 手册

- 远端构建的 Agentic 在没有扩展连接时，通过实际 MCP 读取了快速开始及目录内全部 18 个链接目标；对嵌套路径、搜索命中和按行读取进行了交叉核对。三个主题全文与参考包相同，两个 AX 和两个窗口/标签管理示例保持原样，操作指南保留了 16 条中的 13 条原文。
- 从 MCP 读取的 API 参考按对象和方法核对了已确认清单中的 125 项，涵盖保留的 111 个 JS 方法。其余 14 项按清单由外层工具承接、暂缓或排除。参考中的 TypeScript 声明通过远端语法检查。
- 构建机以独立 Agentic 和 Chromium profile 加载了生产扩展 0.4.0，按快速开始取租约并打开测试页。手册示例成功读取 AX、按当前编号填写输入框、跨调用复用标签页变量、取消原生确认框并回传 JPEG。显式释放关闭临时页，同名租约接续了已标记的交接页。
