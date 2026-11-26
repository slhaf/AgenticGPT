# Agentic Browser v1 协议

扩展以客户端身份直连 Agentic 的专用 WebSocket 监听器。Agentic 配置词汇为 browser.listenAddr（默认 127.0.0.1:39421）和 browser.pairingToken（配置后可配对，视为密钥）。Browser Toolset 门控监听和工具。需要跨网络连接时可在 Agentic 监听器前终结 WSS。应用消息为 JSON 对象，每帧一个消息。

## 连接

- 扩展第一帧是 {"type":"hello","token":"...","browserId":"...","browserName":"...","apiVersion":1,"extensionVersion":"..."}。browserId 是安装扩展的 profile 的稳定 ID，browserName 是显示名。
- Agentic 验证密钥与 API 版本后回 {"type":"welcome","apiVersion":1,"agentId":"..."}。握手失败可以回 {"type":"error","error":{"code":"...","message":"..."}} 并关闭连接。
- 扩展发送 {"type":"ping"}，Agentic 回 {"type":"pong"}。每条连接独立重连。新的连接代替旧连接时旧租约失效。
- 扩展握手成功后及占用变化时发送 {"type":"status","busy":false}（初始化、执行和收尾期间 busy 为 true，空闲为 false）。

## 命令与结果

- Agentic → 扩展按 type 区分命令，每个命令带 requestId 字符串。
- {"type":"acquire","requestId":"...","name":"...","idleTimeoutSeconds":60}，成功结果 {"leaseId":"..."}。
- {"type":"repl","requestId":"...","leaseId":"...","code":"...","timeoutMs":20000}，成功结果为现有 MCP CallToolResult 形状，支持 content 文本和图像项、structuredContent、isError。JS 异常也用 isError 内容报告。
- {"type":"reset","requestId":"...","leaseId":"..."}，返回 {}。保留租约 ID，替换 JS 上下文。
- {"type":"release","requestId":"...","leaseId":"..."}，返回 {"released":true}。
- 扩展响应 {"type":"result","requestId":"...","result":{...}} 或 {"type":"result","requestId":"...","error":{"code":"...","message":"..."}}。冲突用 browser_busy，已失效租约用 browser_lease_expired，执行超时用 browser_repl_timeout。
- 扩展回收租约后向其原连接发送 {"type":"leaseEnded","leaseId":"..."}，主动释放也可发此事件。Agentic 仅回收匹配的本地绑定。

## 所有权

扩展对整个 profile 只发放一份活动租约，由消息所属连接和扩展创建的 leaseId 标识。连接存在与空闲计时独立。同连接同 name 可幂等 acquire 并更新时限。在途执行和扩展已派发的浏览器动作收尾时保持占用。释放、空闲、持有连接断开需要停止旧执行和清理，随后才可交接。断线后的旧动作与结果不重放。

Agentic 的 browser.acquire 要求 browserId、name 和 idleTimeoutSeconds。browser.repl/reset/release 依 name 查原连接和租约。browser.list 投影连接和占用。手册 read/search 读取 Agentic 内嵌的 SDK v1 Markdown，Browser Toolset 启用即可查询。

## SDK v1 目标

注入全局 browser 和 nodeRepl。以 globalThis 存放跨调用状态，支持顶层 await。browser.tabs.list/get/new 和 Tab 的 id、goto/url/title/close、domSnapshot、locator(selector)/ref(refId) 的 click/fill/press、screenshot。截图返回 {mimeType,data}，用 nodeRepl.emitImage 输出。nodeRepl.write 输出文本或 JSON。优先复用 fork 的快照和操作脚本。实施后以真实代码和验证收敛手册。

## 重启故障复核

Room 于 06:00:13 记录 Browser 绑定失败，Tailscale 在 09:07:09 才就绪。此后地址可绑定，Laptop 到现有监听地址仍收到 ECONNREFUSED。当前 watcher 在配置 mtime 未变化时跳过，启动绑定错误没有重试。Neko 重启后持久扩展目录仍在，运行中的 Chromium 没有加载扩展。相同配对地址会创建多个 socket，同 browserId 在服务端互相替换。

构建机的隔离 Chrome 和 Agentic 已确认 PNG 截图及两条原生 MCP 图片结果，正常 Chrome 重启会清空旧租约。本轮需在实际 Room 连接器复验图片。

## 浏览器对话框与控制台

Chromium 对话框事件只发给已启用 Page 域的调试会话。触发前启用监听，并让已派发但被弹框阻塞的操作及时结束等待。打开状态须在同一个租约的后续 REPL 仍可查询。

旧版 `tab.dev.logs()` 消费 Runtime 的控制台和未捕获异常事件，每个标签页保留最近 500 条，默认输出最后 100 条匹配项，按捕获顺序排列。复用标签页调试会话，以保留导航前的记录和捕获时间。
