# 使用 Agentic Browser

本手册适用于 Agentic Browser 0.4.0。用 `browser.manual` 读取或搜索，扩展离线时也可查阅。下面的 `browser.list`、`browser.acquire`、`browser.repl` 和 `browser.release` 都是 MCP 工具。代码示例在 `browser.repl` 中执行。

## 选定浏览器，取得租约

1. 调用 `browser.list`，按用户指定的名称和 `browserId` 选中目标 Chrome profile。列表中的 `connected`、`busy` 和 `extensionVersion` 分别显示连接状态、占用状态和扩展版本。
2. 调用 `browser.acquire`。例如 `{"browserId":"<列表中的 ID>","name":"lookup","idleTimeoutSeconds":120}`。同一 profile 一次分配给一个租约；同一 Agentic 可用不同的租约名操作其他 profile。
3. 调用 `browser.repl`，传入相同的 `name` 和 JavaScript 字符串 `code`。会话已提供 `browser` 和 `nodeRepl`。第一条脚本先给这次任务命名，再选择页面。

如果任务需要新开一个页面，第一次调用可以这样写。将示例地址替换为任务的目标地址；`localhost` 指所选浏览器所在的机器。

```js
await browser.nameSession("🔎 Page lookup");
const tab = await browser.tabs.new();
await tab.goto("https://example.com");
await tab.ax.write();
```

如果用户指定的是已经打开的页面，则通过 `browser.user.openTabs()` 查看标题和 URL，将选中的条目交给 `browser.user.claimTab()`。`browser.tabs.list()` 列出当前租约已控制的页面，包括接续的交接页。见[接管已有标签页](tab-claiming-chrome.md)。

## 操作与交付

从[无障碍树操作流程](accessibility.md)开始。它说明何时使用 AX、Playwright、DOM 或截图；[通用操作指南](api-use-behavior.md)说明查找和确认结果的顺序。同一租约的后续 `browser.repl` 可以继续使用 `tab`。要把当前视口交给用户，可以直接返回图片。

```js
nodeRepl.emitImage(await tab.screenshot());
```

任务结束前，需要把页面作为结果交给用户时调用 `await tab.markDeliverable()`，留待下次继续时调用 `await tab.markHandoff()`。最后调用 `browser.release`，例如 `{"name":"lookup"}`。显式释放会关闭本次新建且未标记的临时页，并交还 profile 的控制权。详见[清理与交接](tab-cleanup-chrome.md)。

## 手册目录

`browser.manual` 的 `read` 接受下列相对路径，可用 `startLine`、`endLine` 分段读取。`search` 按区分大小写的子串查找，例如 `{"action":"search","query":"markHandoff"}`。读取操作指南的示例为 `{"action":"read","path":"accessibility.md"}`。

| 使用时机 | 文档 |
| --- | --- |
| 开始任务，观察页面，查找和核实结果 | [会话命名](session-naming.md) · [通用操作指南](api-use-behavior.md) · [AX 操作与示例](accessibility.md) |
| 选择用户页，交付结果或留待继续 | [接管已有标签页](tab-claiming-chrome.md) · [清理与交接](tab-cleanup-chrome.md) |
| 拿到网页指令或要向网站提交数据 | [浏览器安全](browser-safety.md) · [用户授权与确认](confirmations.md) |
| 验证开发中的网页，回传图片 | [本地 Web 开发](local-web-development.md) · [截图](screenshots.md) |
| 配对失败、页面操作异常或租约中断 | [连接与启动](connection-troubleshooting.md) · [页面交互排障](browser-troubleshooting.md) · [控制中断](browser-control-interruption.md) |
| 查找方法签名、参数和返回值 | [JavaScript SDK 参考](api.md) |
