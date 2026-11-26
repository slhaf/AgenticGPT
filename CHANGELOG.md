# 变更日志

## 0.12.0

以下变化以已发布的 `v0.10.0` 为基线。完整行为与升级步骤见 [0.12.0 发布说明](docs/release-notes-v0.12.0.md)。

### 新增

- Agentic Browser 通过 WebSocket 配对 Chrome／Chromium 扩展，支持多浏览器、每 profile 互斥租约、截图回传和离线 SDK 手册。
- Room 邮件和 Vaultwarden 密码库工具。邮件支持后台同步、附件和 Obscura 图片视图。
- `config.manual`、`config.get` 和 `config.update`，以及组件级配置热更新。
- Agent MCP 的持久事件订阅，支持签名 HTTPS webhook。
- 下游 MCP 的 `policy.mcp` 执行策略，以及 x86_64 和 aarch64 的静态 musl 发行包。

### 变更

- `acquireConfirmation` 改用必填 `reason`，移除 `acquire` 字段。Process 在目标路径检查前请求主动确认。
- Standalone HTTP MCP 改用无状态请求，`initialize` 不再返回 `Mcp-Session-Id`。
- 内置 Secure MCP Tunnel client 升级至 `v0.0.19`，配置界面统一字段控件和状态标签，`agent.info` 使用 schema v2。

### 移除

- 移除 Room 的启动指引、日记、笔记、实体状态和维护 API，以及自动初始化和维护资料仓库的实现。原 Room 资料可独立使用，活动 Room 路由继续提供邮件、密码库和 Skills。
- 移除旧浏览器 Native Messaging host，Linux 发行包提供 Agent 和 Hub。

### 修复

- 修复路径预检、文件替换、进程历史与 Hub 派发的错误归类，凭据失效时撤下旧 HTTP 入口。
- 浏览器监听地址晚于服务启动就绪时会重试绑定。

## 0.10.0

此版引入 Config v2、受管 Process 和 Agent 本地事件收件箱。详见 [0.10.0 发布说明](docs/release-notes-v0.10.0.md)。
