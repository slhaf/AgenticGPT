# Agentic GPT

[English README](README.md)

Agentic GPT 是一个 Linux 本地执行代理与 Rust Hub，用来让 ChatGPT 在明确的策略、确认、容量和审计边界内连接本地机器。

```text
ChatGPT Actions / ChatGPT Apps MCP
  -> Rust Hub HTTPS API / MCP
  -> Local Agent WebSocket，或 Secure MCP Tunnel
  -> 本地 Unix MCP / 进程 Job / Skill Job / MCP Job / tmux / 文件系统
```

当前主线使用 Rust Hub。历史 Cloudflare-only Hub 已移出 `main`，仅保留在 `legacy/cf-worker-before-removal` 分支。

## v0.9 主要能力

- 统一的 `ManagedJob` 执行模型：`process.exec`、`process.batch`、`skills.run`、`mcp.callTool` 和 `mcp.batch` 共用容量、生命周期、审计和 `job.*` 管理入口。
- `job.get`、`job.list`、`job.cancel` 提供统一的查询与取消，不再保留 managed `session.*` 或 `process.get/list/kill` 包装。
- `mcp.callTool` 支持 bounded inline wait、绝对执行 deadline、精确 MCP request-id 取消、结果保留和超大结果摘要。
- `mcp.batch` 原子接纳 1–16 个普通 MCP child Job，只确认一次，支持 parallel / sequential、安全 fail-fast、全局 8 / 单 server 2 的共享并发限制，并保持输入顺序。
- Normal surface 固定为 24 个工具，Room surface 固定为 36 个工具；Tunnel stdio 与本地 Unix MCP 的 descriptor/schema 一致。
- 本地开发可直接运行 `run-as-local`，不需要 tunnel 凭据；同一 worker 也可在 `run-as-standalone` 下同时提供 tunnel 与 owner-only Unix MCP。
- 完整的命令策略、路径策略、桌面/ntfy 确认、bubblewrap、Room bootstrap、Skill 安装与 tmux 持久工作区。
- Hub 提供 Actions OpenAPI、Apps 兼容 `/mcp`、OAuth shim、HTTP API 与 Local Agent WebSocket。

## 仓库结构

- `crates/agentic-gpt`：Linux Local Agent CLI。
- `crates/agentic-gpt-hub`：Rust Hub HTTP/WebSocket/MCP 服务。
- `crates/agentic-gpt-protocol`：共享 JSON 协议类型。
- `openapi/hub.yaml`：Custom GPT Actions schema。
- `config.example.json`：无可用密钥的严格 v0.9 配置示例。
- `docs/interfaces.md`：接口与工具地图。
- `docs/standalone-runtime.md`：Tunnel/local runtime、工具矩阵、信任与恢复。
- `docs/operations.md`：部署与 smoke test。
- `docs/migration-v0.9.zh-CN.md`：v0.8 → v0.9 迁移指南。

## 运行要求

- Local Agent 运行在 Linux 上。
- 使用 release 二进制，或安装 Rust stable 从源码构建。
- 远程访问需要部署 Rust Hub，并建议在前面使用 Caddy/Nginx 提供 HTTPS。
- 可选：`bubblewrap` 用于沙箱；`ntfy` 用于 Hub 远程确认。

## 安装

```bash
tar -xzf agentic-gpt-x86_64-unknown-linux-gnu.tar.gz
install -m 0755 agentic-gpt agentic-gpt-hub ~/.local/bin/
```

支持的 release target：

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

源码构建、CI 与发布流程见 [`docs/development.zh-CN.md`](docs/development.zh-CN.md)。

## 快速开始

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

Hub 状态默认位于 `~/.agentic_gpt/hub.sqlite3`，配置默认位于 `~/.agentic_gpt/hub.json`。

### 2. 启动 Local Agent

```bash
agentic-gpt config init
agentic-gpt config set hubUrl http://127.0.0.1:8787
agentic-gpt config set agentId laptop
agentic-gpt config set agentSecret '<agent-secret>'
agentic-gpt config set confirmationProvider freedesktop-then-ntfy
agentic-gpt config set confirmationLanguage zh-CN
agentic-gpt run
```

Local Agent 配置位于 `~/.agentic_gpt/config.json`。升级前请先阅读 [`docs/migration-v0.9.zh-CN.md`](docs/migration-v0.9.zh-CN.md)。

### 3. 连接 ChatGPT

Custom GPT Actions 使用 [`openapi/hub.yaml`](openapi/hub.yaml)，并通过 `AGENTIC_GPT_API_KEY` 配置 Bearer auth。

ChatGPT Apps / MCP endpoint：

```text
https://<your-hub-domain>/mcp
```

`mcp.callTool` 返回统一 `JobResponse`；调用仍在执行时，通过 `job.get` 获取结果或通过 `job.cancel` 请求取消。`mcp.batch` 返回有序 child Job 结果；child 后续仍使用同一套 `job.*` 生命周期。

## 本地与 Tunnel runtime

不需要 tunnel 的本地联调：

```bash
agentic-gpt run-as-local --config ~/.agentic_gpt/config.json --profile normal
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info --config ~/.agentic_gpt/config.json --arguments '{}'
```

Secure MCP Tunnel：

```bash
agentic-gpt run-as-standalone --config ~/.agentic_gpt/config.json --profile normal
```

Room surface 使用 `--profile room`。两种 ingress 共用同一个 `AppState`、Job registry、配置热加载、策略、确认和审计；完整说明见 [`docs/standalone-runtime.md`](docs/standalone-runtime.md)。

## Managed Job 与 MCP 边界

- `process.exec` / `skills.run` / `mcp.callTool` 返回 `JobResponse`。
- `job.get` / `job.cancel` 返回 `JobDetail`；Hub 仅有缓存摘要时会设置 `detailAvailable=false`。
- MCP 参数必须是 JSON object，单调用参数上限 256 KiB，单调用结果保留上限 512 KiB。
- `mcp.batch` 每批 1–16 项，aggregate 参数与响应各上限 2 MiB。
- MCP cancel 使用精确 downstream request id；无法证明远端终止时返回 `detached`，不会虚报 `cancelled`。
- 审计只记录 bounded 键名摘要、字节数、哈希、状态与终止证据，不记录原始参数值或原始结果。

## 确认机制

支持的确认通道：

- `freedesktop`：本地桌面按钮。
- `ntfy`：Hub relay 的远程确认。
- `freedesktop-then-ntfy`：本地不可用时再回退到 Hub。

规范配置形式：

```json
{
  "confirmationProvider": {
    "channels": ["freedesktop", "ntfy"]
  },
  "confirmationLanguage": "zh-CN"
}
```

`mcp.batch` 在完成全部验证与原子容量接纳后只请求一次 aggregate confirmation。单 server 批次可以授予 15/30 分钟 server allow；多 server 批次只提供 batch-scoped allow/deny。

## 命令与路径策略

```bash
agentic-gpt config allow add bash
agentic-gpt config confirm add python -c
agentic-gpt config deny add ssh

agentic-gpt config path list
agentic-gpt config path write add ~/Projects
agentic-gpt config path readonly add /var/log
agentic-gpt config path deny add ~/.secrets
```

`process.exec` 与 `process.batch` 支持 `workingDirectory`。解析后的目录必须存在、位于可写根内，并且不在 denied roots 内。

## 从 v0.8 升级

v0.9 是 breaking release，Hub 与 Local Agent 应同时升级：

- `limits.maxActiveSessions` → `limits.maxActiveJobs`
- 删除从未控制运行时行为的 `sessionIdleTimeoutSecs`
- managed `session.*` 与 `process.get/list/kill` → `job.get/list/cancel`
- `process.batchExec` → `process.batch`
- `/v1/exec`、`/v1/batchExec`、`/v1/sessions/*` → `/v1/process/*`、`/v1/jobs/*`
- `mcp.callTool` 不再 raw passthrough，而是返回 managed `JobResponse`
- Agent `Hello` 必须包含 `bootGeneration`

不提供这些旧 managed execution 名称的兼容 alias。tmux session 名称和 tmux API 不变。完整步骤见 [`docs/migration-v0.9.zh-CN.md`](docs/migration-v0.9.zh-CN.md)。

## 更多文档

- [`docs/interfaces.md`](docs/interfaces.md)：HTTP、Actions、Apps MCP、Local Agent WebSocket。
- [`docs/standalone-runtime.md`](docs/standalone-runtime.md)：local/tunnel runtime 与 24/36 工具矩阵。
- [`docs/operations.md`](docs/operations.md)：部署、smoke test 与安全不变量。
- [`docs/development.zh-CN.md`](docs/development.zh-CN.md)：开发、CI 与 release。
- [`docs/release-notes-v0.9.0.md`](docs/release-notes-v0.9.0.md)：v0.9.0 变更摘要。

## 构建与发布

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git tag v0.9.0
git push origin v0.9.0
```

创建或推送 tag 是独立发布动作；普通开发提交不会发布任何内容。

## 安全说明

- Hub 必须通过 HTTPS 暴露。
- 使用高熵 Hub API key、agent secret、tunnel secret 与 ntfy topic。
- 凭据目录保留在 denied roots。
- 对 shell、网络工具和陌生 MCP server 优先要求确认。
- 使用 bounded Job wait，不要让 HTTP/MCP 请求无限等待。
- 部署 v0.9 前先迁移配置，并通过 `agent.info` 与 `local list-tools` 检查 surface。

## License

MIT
