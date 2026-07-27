# 从 v0.8 迁移到 v0.9

v0.9 将 process-shaped managed session 替换为统一、区分 kind 的 Job 生命周期。升级边界取决于 runtime：

- **Standalone / Local Unix MCP：** 每台机器可以独立升级 `agentic-gpt`，命令链路不依赖 Hub。
- **Hub + Local Agent：** 必须协调升级 Hub 与全部 command-capable Agent。v0.9 要求 `Hello.bootGeneration`，与 v0.8 wire protocol 不兼容。

## 升级前

每台 Agent：

1. 停止该 Agent runtime。
2. 备份配置、workspace audit 和本地管理状态。
3. 对照 [`config.example.json`](../config.example.json) 与 [`configuration.zh-CN.md`](configuration.zh-CN.md) 检查配置。
4. 启动 v0.9 前迁移 limits 对象。

Hub 模式还需要在替换任一侧之前停止 Hub，并备份 Hub 数据库与配置。

## 必须迁移的配置

把：

```json
{
  "limits": {
    "maxConcurrentTasks": 2,
    "maxActiveSessions": 6,
    "sessionIdleTimeoutSecs": 3600
  }
}
```

改为：

```json
{
  "limits": {
    "maxConcurrentTasks": 2,
    "maxActiveJobs": 6
  }
}
```

`maxActiveJobs` 也可设置为 `"auto"`。`sessionIdleTimeoutSecs` 从未控制运行时行为，必须删除。v0.9 会明确拒绝这两个旧字段。

从旧 Hub-only 配置迁移到 Standalone 时，还需要添加有效 `tunnel` 块。Tunnel secret 必须使用 `file:PATH` 或 `env:NAME` 引用，明文值会被拒绝。

## 工具名称迁移

| v0.8 | v0.9 |
| --- | --- |
| `process.batchExec` | `process.batch` |
| `process.get/list/kill` | `job.get/list/cancel` |
| `session.start` | `process.exec` 或对应 domain creator |
| `session.list` | `job.list` |
| `session.inspect/wait` | `job.get` + 可选 `waitSeconds` |
| `session.kill` | `job.cancel` |
| `hub.session.list/get` | `hub.job.list/get` |

不提供兼容 alias。tmux session 工具与名称不受影响。

## Hub HTTP 路由迁移

以下变更只影响 Hub/Actions caller：

- `/v1/exec` → `/v1/process/exec`
- `/v1/batchExec` → `/v1/process/batch`
- `/v1/sessions/*` → `/v1/jobs/*`
- `mcp.callTool` 返回 managed `JobResponse`，不再 raw passthrough
- 新增 `/v1/mcp/batch`

Custom GPT 必须替换为 v0.9 的 [`openapi/hub.yaml`](../openapi/hub.yaml)，不要保留缓存的 v0.8 schema。

## 响应迁移

- `process.exec`、`skills.run`、`mcp.callTool` 返回 `JobResponse`。
- `job.get`、`job.cancel` 返回 `JobDetail`。
- `job.list` 只返回轻量 `JobInfo`，不携带保留的大结果。
- 下游 MCP 结果保留在 `mcp.callTool.result`。
- `mcp.batch` 返回按输入顺序排列的 child Job，并包含 `batchId`、可选 `batchCallId` 与 `batchIndex`。
- Hub 只有缓存摘要时设置 `detailAvailable=false`。

## Standalone / Local 部署顺序

每台机器可以独立升级：

1. 停止该机器的 v0.8 runtime。
2. 迁移本地配置。
3. 替换 `agentic-gpt` 为 v0.9。
4. 启动 `run-as-standalone` 或 `run-as-local`。
5. 验证本地 socket 与工具 surface。
6. 必要时重新连接或重试对应的 ChatGPT tunnel。

滚动升级期间其他机器继续可用。

## Hub 模式部署顺序

Hub protocol 是 breaking 的，需要协调重启：

1. 停止 v0.8 Local Agent。
2. 停止 v0.8 Hub。
3. 迁移每台 Local Agent 配置。
4. 使用同一份 v0.9 build 替换 Hub 与 Agent。
5. 先启动 Hub，再启动 Local Agent。
6. 刷新 Actions schema，并重新连接 Apps MCP client。

不要混用 v0.8 Hub 与 v0.9 Agent，也不要反向混用。

## 验收

通用 Agent 检查：

```bash
agentic-gpt --version
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info \
  --config ~/.agentic_gpt/config.json \
  --arguments '{}'
```

预期：

- `agentic-gpt` 为 `0.9.0`；
- Normal surface 24 个工具，Room surface 36 个工具；
- `job.get/list/cancel` 与 `mcp.batch` 存在；
- managed `session.*`、`process.get/list/kill`、`process.batchExec` 不存在；
- `agent.info.execution.jobs` 与 `agent.info.mcp.concurrency` 存在；
- local socket 为 owner-only；
- Standalone 重启后，即使控制面先续发请求、尚未发送新的 `initialize`，hidden worker 也不会因 `expect initialized request` 退出。

Hub 部署额外检查：

```bash
agentic-gpt-hub --version
curl -fsS -H "Authorization: Bearer $AGENTIC_GPT_API_KEY" \
  https://<hub-domain>/v1/info
```

确认 Hub 与 Agent 版本一致、Agent 在线，并且刷新后的 Actions/Apps MCP contract 暴露 v0.9 surface。
