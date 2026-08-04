# Agentic GPT

[English README](README.md)

Agentic GPT 通过每台机器独立的 Secure MCP Tunnel、可选的集中式 Rust Hub，或仅本机可访问的 Unix MCP socket，将 ChatGPT 连接到 Linux 机器。

对大多数部署，**推荐优先使用 Secure MCP Tunnel / Standalone 模式**。它不需要 VPS、公开反向代理或处在命令链路中的自建 Hub。每台机器拥有独立 tunnel 与 worker，因此某个 Agent 断连不会影响其他机器。

```text
推荐——Standalone
ChatGPT Secure MCP Tunnel
  -> 官方 tunnel-client
  -> agentic-gpt worker
  -> 策略 / 文件 / Process Job / Skill / 下游 MCP / tmux

集中式——Hub
ChatGPT Actions 或 Apps MCP
  -> HTTPS Rust Hub
  -> WebSocket 或 SSE Local Agent
  -> 相同的本地执行能力

开发联调——Local
本地 MCP client 或 agentic-gpt CLI
  -> owner-only Unix MCP socket
  -> 相同的 Agent surface
```

历史 Cloudflare-only Hub 已移出 `main`，仅在 `legacy/cf-worker-before-removal` 分支保留归档。

## 为什么优先 Standalone？

- 不需要 VPS、公开域名、反向代理、Hub 数据库或共享命令路由器。
- 每台机器具有独立连接与重启边界。
- Tunnel 与 owner-only Unix MCP 对同一 profile 暴露一致的 24 个 Normal 工具或 36 个 Room 工具。
- 策略、确认、审计、热配置、容量和 Managed Job 都保留在本机。
- fresh stdio worker 即使先收到旧逻辑会话续发的请求、尚未收到新的 MCP `initialize`，也能自动恢复而不退出。

当你需要多 Agent 的统一公开入口、Custom GPT Actions、集中式运行历史、Hub 聚合/通知或 Hub relay 远程确认时，Hub 模式仍然适合。

## 选择运行模式

| 模式 | 适用场景 | 是否需要公开服务器 | 故障范围 | 启动入口 |
| --- | --- | --- | --- | --- |
| **Secure MCP Tunnel / Standalone** | 推荐的直接部署 | 不需要 | 单个 tunnel/Agent | `agentic-gpt run-as-standalone` |
| **Hub + Local Agent** | 集中路由、Actions、共享历史/报告 | 需要 | Hub 是共享依赖 | `agentic-gpt-hub serve` + `agentic-gpt run` |
| **Local Unix MCP** | 开发、smoke test、本地自动化 | 不需要 | 单个本地 worker | `agentic-gpt run-as-local` |

## v0.9 主要能力

- `process.exec`、`process.batch`、`skills.run`、`mcp.callTool`、`mcp.batch` 共用统一 Managed Job 生命周期。
- 使用 `job.get`、`job.list`、`job.cancel` 管理不同类型的 Job。
- 批量执行原子接纳，并使用有界的确认边界。
- allow / confirm / deny 命令策略，以及可写、只读、拒绝路径根。
- 本地桌面确认与可选的 Hub-backed ntfy 确认。
- 可选 bubblewrap 沙箱。
- 下游 MCP 参数/结果边界、精确 request-id 取消，以及无法证明远端终止时如实返回 `detached`。
- Room bootstrap、日记/笔记、Skill 安装与执行、tmux 持久工作区。
- 可选 Rust Hub：Actions OpenAPI、Apps 兼容 `/mcp`、OAuth shim、HTTP API、WebSocket/SSE Agent、历史、报告和通知。

## 仓库结构

- `crates/agentic-gpt`：Linux Agent、Standalone supervisor、本地 MCP runtime 与 CLI。
- `crates/agentic-gpt-hub`：可选 Rust Hub HTTP/WebSocket/SSE/MCP 服务。
- `crates/agentic-gpt-protocol`：共享 JSON 协议类型。
- `config.example.json`：无可用密钥的严格 v0.9、Standalone-first 配置示例。
- `openapi/hub.yaml`：Hub 模式的 Custom GPT Actions schema。
- `docs/configuration.zh-CN.md`：按 runtime 区分的配置、密钥与热加载边界。
- `docs/standalone-runtime.md`：Tunnel/local 拓扑、信任、恢复、报告与工具矩阵。
- `docs/interfaces.md`：Hub HTTP、Actions、Apps MCP 与 Agent 协议地图。
- `docs/operations.md`：验证、部署与 smoke test。

## 运行要求

共同要求：

- `agentic-gpt` 运行在 Linux 上。
- 使用对应架构的 release 二进制，或使用 Rust stable 从源码构建。
- 可选安装 `bubblewrap` 以启用沙箱。

Standalone 还需要分配好的 Secure MCP Tunnel id 与 API key 引用，但**不需要 VPS 或入站公开端口**。

Hub 模式额外需要服务器/VPS、HTTPS、公开部署时的反向代理、Hub API key 和每个 Agent 的 secret。

## 安装

Release 压缩包同时包含两个二进制。Standalone 与 Local 模式只需安装 `agentic-gpt`；只有 Hub 模式需要 `agentic-gpt-hub`。

```bash
tar -xzf agentic-gpt-x86_64-unknown-linux-gnu.tar.gz
install -m 0755 agentic-gpt ~/.local/bin/
# 仅 Hub 模式：
install -m 0755 agentic-gpt-hub ~/.local/bin/
```

支持：

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

源码构建、CI 与发布流程见 [`docs/development.zh-CN.md`](docs/development.zh-CN.md)。

## 快速开始：Secure MCP Tunnel（推荐）

### 1. 初始化本地配置

```bash
agentic-gpt config init
agentic-gpt config set agentId laptop
agentic-gpt config set confirmationProvider freedesktop
```

默认配置路径为 `~/.agentic_gpt/config.json`。开放写入根或启用 MCP server 前，请先检查 [`config.example.json`](config.example.json) 与 [`docs/configuration.zh-CN.md`](docs/configuration.zh-CN.md)。

### 2. 通过引用保存 tunnel 密钥

```bash
install -d -m 700 "$HOME/.agentic_gpt/secrets"
touch "$HOME/.agentic_gpt/secrets/tunnel-api-key"
chmod 600 "$HOME/.agentic_gpt/secrets/tunnel-api-key"
read -rsp "Tunnel API key: " AGENTIC_TUNNEL_API_KEY
printf '\n'
printf '%s' "$AGENTIC_TUNNEL_API_KEY" > "$HOME/.agentic_gpt/secrets/tunnel-api-key"
unset AGENTIC_TUNNEL_API_KEY

agentic-gpt config set tunnel.tunnelId tunnel_<assigned-id>
agentic-gpt config set tunnel.apiKey file:"$HOME/.agentic_gpt/secrets/tunnel-api-key"
agentic-gpt config set tunnel.client.autoDownload true
```

`tunnel.apiKey` 只接受 `file:PATH` 或 `env:NAME`；明文 secret 会被拒绝。

### 3. 启动 Standalone worker

```bash
agentic-gpt run-as-standalone --profile normal
```

Room 的 36 工具 surface 使用 `--profile room`。同一 worker 还会提供 owner-only Unix MCP socket，便于本机检查：

```bash
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

在 ChatGPT 中连接分配给该 Agent 的 Secure MCP Tunnel。每台机器独立配置、独立启动。

Tunnel-client 信任、缓存、恢复、报告和 service manager 说明见 [`docs/standalone-runtime.md`](docs/standalone-runtime.md)。

## 仅本地开发

不需要 tunnel 凭据：

```bash
agentic-gpt run-as-local --profile normal
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

Local 模式与 Standalone 共用策略、路径策略、确认、审计、热配置和 Job 实现，但只开放 owner-only Unix socket。

## 集中式 Hub 模式

只有在统一入口与集中式能力值得额外基础设施时，再选择 Hub 模式。

### 1. 启动 Hub

```bash
agentic-gpt-hub init
agentic-gpt-hub agent add \
  --agent-id laptop \
  --display-name my-laptop \
  --secret '<agent-secret>'
AGENTIC_GPT_API_KEY='<high-entropy-api-key>' \
  agentic-gpt-hub serve --bind 127.0.0.1:8787
```

公开部署时在 Hub 前放置 Caddy/Nginx，并通过 HTTPS 暴露。Hub 状态默认位于 `~/.agentic_gpt/hub.sqlite3`，配置默认位于 `~/.agentic_gpt/hub.json`。

### 2. 启动连接 Hub 的 Agent

```bash
agentic-gpt config init
agentic-gpt config set hubUrl https://agentic-gpt.example.com
agentic-gpt config set hubTransport websocket
agentic-gpt config set agentId laptop
agentic-gpt config set agentSecret '<agent-secret>'
agentic-gpt run
```

Room profile 使用 `agentic-gpt run-as-room`。`hubTransport` 可设为 `websocket` 或 `sse`。

### 3. 将 ChatGPT 连接到 Hub

- Custom GPT Actions：导入 [`openapi/hub.yaml`](openapi/hub.yaml)，Bearer auth 使用 `AGENTIC_GPT_API_KEY`。
- ChatGPT Apps MCP：连接 `https://<your-hub-domain>/mcp`。

Hub 原生工具和转发执行使用相同的 Managed Job envelope；运行中的任务通过 `job.get` 查询、通过 `job.cancel` 取消。

## Managed Job 与安全边界

- Normal surface 24 个工具，Room surface 36 个工具。
- `process.exec`、`skills.run`、`mcp.callTool` 返回 `JobResponse`。
- `mcp.batch` 接受 1–16 个有序调用，只确认一次，并执行全局/单 server 并发限制。
- MCP 单调用参数上限 256 KiB，保留结果上限 512 KiB；批次 aggregate 参数与结果各上限 2 MiB。
- 审计记录 bounded metadata、hash、状态与终止证据，不记录原始 MCP 参数/结果。
- 执行前使用 `agent.info` 查看 profile、路径策略、容量、确认、MCP 配置摘要和连接状态。

## 确认、命令策略与路径策略

```bash
agentic-gpt config set confirmationProvider freedesktop
agentic-gpt config set confirmationLanguage zh-CN

agentic-gpt config allow add bash
agentic-gpt config confirm add python -c
agentic-gpt config deny add ssh

agentic-gpt config path list
agentic-gpt config path write add ~/Projects
agentic-gpt config path readonly add /var/log
agentic-gpt config path deny add ~/.secrets
```

Hub-backed `ntfy` 是可选能力，只有在 Hub 模式或配置了 Standalone Hub reporting/confirmation relay 时才有意义。本地拒绝或超时是最终结果。

字段定义与热加载行为见 [`docs/configuration.zh-CN.md`](docs/configuration.zh-CN.md)。

## 从 v0.8 升级

v0.9 是 breaking release：

- `limits.maxActiveSessions` → `limits.maxActiveJobs`
- 删除 `sessionIdleTimeoutSecs`
- managed `session.*`、`process.get/list/kill` → `job.get/list/cancel`
- `process.batchExec` → `process.batch`
- `mcp.callTool` 返回 Managed `JobResponse`

Standalone/Local 只需独立升级 `agentic-gpt`。Hub 模式由于 v0.9 与 v0.8 wire protocol 不兼容，必须协调升级 Hub 与连接的 Agent。完整步骤见 [`docs/migration-v0.9.zh-CN.md`](docs/migration-v0.9.zh-CN.md)。

## 更多文档

- [`docs/configuration.zh-CN.md`](docs/configuration.zh-CN.md)：runtime 选择、主要配置块、secret 引用与 reload/restart 边界。
- [`docs/standalone-runtime.md`](docs/standalone-runtime.md)：Standalone/local 运行、tunnel-client 信任、恢复、报告与精确工具矩阵。
- [`docs/interfaces.md`](docs/interfaces.md)：Hub HTTP、Actions、Apps MCP、协议与直接 MCP surface。
- [`docs/tool-contract-matrix.md`](docs/tool-contract-matrix.md)：Normal/Room/Hub 工具契约、边界与 surface parity 矩阵。
- [`docs/operations.md`](docs/operations.md)：本地验证、Standalone-first 部署检查、Hub 检查与安全不变量。
- [`docs/migration-v0.9.zh-CN.md`](docs/migration-v0.9.zh-CN.md)：按 runtime 划分的 v0.8 → v0.9 迁移。
- [`docs/migration-v0.10.zh-CN.md`](docs/migration-v0.10.zh-CN.md)：`file.batch` 文件组与部分成功语义迁移。
- [`docs/release-notes-v0.9.1.zh-CN.md`](docs/release-notes-v0.9.1.zh-CN.md)：v0.9.1 release 边界与验证摘要。
- [`docs/release-notes-v0.9.0.md`](docs/release-notes-v0.9.0.md)：v0.9.0 变更与验证摘要。
- [`docs/development.zh-CN.md`](docs/development.zh-CN.md)：开发、CI 与 release。

## 构建与发布

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git tag v0.9.0
git push origin v0.9.0
```

创建或推送 tag 是独立发布动作；普通提交不会发布任何内容。

## 安全说明

- tunnel API key、Hub API key、agent secret 与 ntfy topic 都应视为凭据。
- Tunnel secret 优先使用 `file:` 或受保护的 `env:` 引用，不要写成配置明文。
- 凭据、浏览器、云平台和 SSH 目录应保留在 denied roots。
- Shell、网络工具和陌生 MCP server 优先要求确认。
- 使用 bounded Job wait，不要让 HTTP/MCP 请求无限阻塞。
- Hub 公开部署时必须使用 HTTPS。
- 不要让 v0.9 读取未迁移的 v0.8 limits 对象。

## License

MIT
