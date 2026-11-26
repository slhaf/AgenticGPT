# Agentic GPT 0.12.0 发布说明

本说明涵盖自已发布的 `v0.10.0` 以来的累计变化，包括此前准备在 `0.11.0` 发布的改动。Agent、Hub 和共享 Protocol 使用 `0.12.0`。升级前重点核对浏览器接入、Room 资料工具、确认参数和 Standalone HTTP MCP 的变化。

## Agentic Browser

Chrome／Chromium 通过独立的 Agentic Browser 扩展连接 Agentic。用户在扩展弹窗内添加 Agentic 的 WebSocket 地址和配对令牌。一个 Agentic 可连接多个 Chrome profile，一个 profile 也可配对多个 Agentic，扩展为整个 profile 分配互斥租约。原依赖 ChatGPT 扩展的 Native Messaging host 已移除。

- 用 `browser.list` 选择 profile，用 `browser.acquire` 取得具名租约。后续通过 `browser.repl` 以同一 `name` 运行脚本，`browser.reset` 重建 JavaScript 会话，`browser.release` 释放租约。
- Agentic Browser 0.4.0 支持标签页接管、导航、无障碍树、定位器、截图、JavaScript 对话框和控制台日志。截图可以通过 `nodeRepl.emitImage` 作为图片回传。释放租约时关闭本次创建且未标记的临时页，交付页和交接页可继续使用。
- `browser.manual` 可离线查阅操作指南、示例和 SDK 参考。`browser.list` 分别报告监听是否就绪、扩展是否连接及租约占用。监听地址晚于进程启动就绪时会重试绑定。

安装、配对及从旧运行时切换的步骤见[Agentic Browser 自托管指南](browser-self-hosted.zh-CN.md)。

## Room 资料 API 移除

移除 `bootstrap`、`bootstrap.read` 及 Hub 上的 `room.bootstrap`、`room.bootstrap.read` 同义入口，以及 `room.diary.*`、`room.notebook.*`、`room.state.*` 和 `room.maintenance.*`。Agent 和 Hub 的 MCP、Hub HTTP／OpenAPI、Room 仓库初始化及维护执行器都已移除对应实现。原有 Room Git 仓库和 Markdown 资料可继续独立使用。

Room 角色继续用于 Hub 的活动 Room 路由。邮件、密码库和 Skills 仍可通过活动 Room 调用。Normal 与 Room 角色采用相同的默认工具集，邮件和密码库默认关闭，分别通过 `room.email`、`room.secret` 启用。 [Room 配置](configuration.zh-CN.md#room-子工具集配置)

## 邮件与密码库

- 新增 `room.email.list/read/search/attachment/send/operate`。Agent 在后台同步单个 IMAP 收件箱，持久保存索引、同步进度和发送记录。读取支持文本和图片两种视图，图片由 Obscura 渲染原始 HTML，并按邮件加载远程与 CID 内嵌资源。MCP 可返回原生图片，HTTP 返回 JSON 图像数据。读取不会标为已读。
- 支持 SMTP 纯文本发送，以及已读、未读、加星、取消星标和移入垃圾箱。发送和移入垃圾箱默认要求确认。SMTP 结果为 `unknown` 时可先查发送记录，直接再次发送可能产生重复邮件。账户要求、附件权限和数据保存期限见[Room 邮件使用指南](email.zh-CN.md)。
- 新增 `room.secret.list/get/upsert/delete`，通过 Agent 上安装的 Bitwarden CLI 访问 Vaultwarden。列表提供站点和账号，`get` 返回密码或当前 TOTP 验证码。查询遇到可降级的网络故障或等待到期时，可读取可解锁的旧缓存并标记为 `stale`。写入要求先成功同步，结果不确定时返回 `secret_write_outcome_unknown`。密钥明文不进入 Hub 回执或 Agent 重放账本。 [Room 密钥使用指南](room-secret.zh-CN.md)

邮件和密码库工具集分别管理发现、准入和服务启停。可先启用工具集，再补齐账户字段，实际调用会检查所需配置。Hub Full 将两组请求路由到活动 Room，调用方无需传 `agentId`。

## 配置与运行时

- 新增 `config.manual`、`config.get`、`config.update`。默认关闭 `config` 工具集。配置更新支持批量 `set/unset/add/remove` 和脱敏预览，写入默认需要确认。Hub 调用须传目标 `agentId`。
- `config.get` 用 `value`、`appliedValue` 和 `applyStatus` 区分保存配置与生效配置。Hub 连接、HTTP 入口、浏览器、邮件账户、密码库和 Skills 安装并发可按组件热更新。运行模式、身份、工作区和 Tunnel 启动参数仍需显式重启。终端配置界面改用对应字段的输入控件，并统一检查页的状态标签。 [配置 API](configuration.zh-CN.md#配置-api) · [热加载与重启边界](configuration.zh-CN.md#热加载与重启边界)
- Standalone 和 Local 通过 `hub.enabled` 控制辅助 Hub 连接。Standalone 的报告还受 `tunnel.hubReporting.enabled` 控制。
- `agent.info` 使用 `schemaVersion: 2` 汇报身份、容量、工作区、连接及确认通道。工作区位于 `workspaceRoot`，文件搜索上限位于 `execution.fileSearchMaxContextLines`，密钥同步状态位于 `secret.syncStatus` 和 `secret.syncLastTriggeredAt`。 [Standalone 运行指南](standalone-runtime.md#tunnelhttp-与本地工具表面)

## 确认与下游 MCP 策略

传入 `acquireConfirmation` 对象即主动请求确认。删除原来的 `acquire` 字段，`reason` 必填，原文长度为 1–100 个 Unicode 字符。可选 `method` 为 `ntfy` 或 `freedesktop`。旧 `acquire=false` 的调用应省略整个对象，旧 `acquire=true` 的调用应直接填写原因。

```json
{"acquireConfirmation":{"reason":"更新项目配置并保存修改","method":"ntfy"}}
```

- Process 在目标路径检查前请求主动确认。经确认获准后跳过目标路径授权预检和身份复查，按原命令执行。独立准入、容量及 Bubblewrap 挂载要求仍然适用。Process 可通过确认对象中的 `read/write` 添加单次挂载范围。`sudo` 按配置策略准入，实际权限由操作系统决定。
- `tmux.pasteText` 的基础策略为确认。ntfy 和桌面通知使用有界预览展示确认事项，避免长命令使通知失败。
- 新增 `policy.mcp`，为下游 `mcp.callTool` 和 `mcp.batch` 配置 `server`、`tool` 匹配规则。规则按拒绝、确认、允许、默认动作的顺序决策，未设置默认动作时继承命令的默认动作。批次任一子项命中拒绝即拒绝整批。

具体授权边界和客户端更新方式见[调用方升级](configuration.zh-CN.md#调用方升级)。

## MCP 连接与事件订阅

- Standalone HTTP MCP 改用无状态 Streamable HTTP。客户端发送 `initialize` 后可直接发送请求，响应不提供 `Mcp-Session-Id`。每个请求仍可通过 SSE 接收该次响应。Hub 的 MCP 入口仍按其会话协议工作。 [HTTP MCP 端点](configuration.zh-CN.md#standalone-入站-http-mcp-端点)
- Agent MCP 新增 `events/list`、`events/subscribe`、`events/unsubscribe` 协议方法。订阅新入库事件后可通过签名 HTTPS webhook 接收通知，订阅与投递进度可从重启中恢复。订阅有效期最多 24 小时，请在 `refreshBefore` 前刷新。历史事件可用 `event.list/get` 查询。webhook ACK 与收件箱的 `event.mark` 各自独立。 [MCP Events 订阅](interfaces.md#mcp-events-订阅)
- Secure MCP Tunnel 的内置客户端升级至 `v0.0.19`。支持预下载并校验官方工件，supervisor 在 worker 退出时管理关联进程组和恢复。旧的版本、下载 URL 和摘要覆盖字段已移除。 [Tunnel client 升级指南](tunnel-client-upgrade.zh-CN.md)

## 修复与发行包

- 文件、进程历史、配置准备与 Hub 派发回执保留更具体的失败原因。修复文件重命名失败后遗留暂存文件、执行预检误判尚未创建的输出路径，以及 HTTP 凭据失效时旧入口未撤下的问题。 [错误原因与执行状态](interfaces.md#错误原因与执行状态)
- Linux 提供 `x86_64` 和 `aarch64` 的 GNU 及静态链接 musl 共四种发行包。每个包包含 Agent、Hub 和第三方许可，浏览器扩展单独构建。新增 ARM 构建机打包入口。 [构建和发布指南](development/releasing.md#linux-产物)

## 升级步骤

1. 从早于 `0.10.0` 的版本升级时，先核对 [0.10.0 升级步骤](release-notes-v0.10.0.md#升级步骤)中的配置、接口和私有状态布局。替换前备份实际配置和私有状态。
2. 停止调用已移除的 Room 资料工具。如果配置保留旧项，从 `toolsets.enabled` 删除 `room.bootstrap`、`room.diary`、`room.notebook`、`room.state`、`room.maintenance`，从 `room` 删除 `repositoryRoot`、`timezone`、`diaryDayBoundaryHour` 和 `maintenance`。保留密码库的 `room.secret` 配置及原 Room 资料。
3. 如果曾通过 Hub 执行这些旧工具，在切换二进制前检查 Hub 是否仍有相关请求待重放，以及 Agent 的 `~/.agentic_gpt/transport-runs.jsonl` 是否仍保存这些命令。确认旧的待重放请求停止后，单独归档实际存在的旧命令记录，保留其他任务及历史结果。遗留的旧命令类型可能妨碍其他 Hub 命令的接收或重放。
4. 配套更新 Agent、Hub 和调用方至本版契约，重新导入 [OpenAPI](../openapi/hub.yaml)。调整主动确认参数和 Standalone HTTP 客户端。使用旧浏览器运行时的部署按[浏览器指南](browser-self-hosted.zh-CN.md)改用扩展配对。保留旧 Tunnel 覆盖字段的部署按[Tunnel client 升级指南](tunnel-client-upgrade.zh-CN.md)更新配置。
5. 启动后用 `agent.info`、`tools/list` 核对连接与工具。使用配置 API 时，通过 `config.get` 确认修改已生效。使用浏览器时核对 `browser.list`；使用 Hub 邮件、密码库或 Skills 时核对活动 Room 和对应工具集。
