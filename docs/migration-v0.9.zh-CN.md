# 从 v0.8 迁移到 v0.9

v0.9 是有意的 breaking release：原先 process-shaped managed session 被统一的 Job 生命周期替代。Hub 与全部 Local Agent 必须一起升级；v0.9 的 `Hello.bootGeneration` 也是必填字段，因此不与 v0.8 wire protocol 兼容。

## 升级前

1. 停止 Hub 与 Local Agent。
2. 备份 Hub 数据库/配置和每台 Local Agent 的配置/审计文件。
3. 对照 [`config.example.json`](../config.example.json) 检查配置。
4. 不要让 v0.9 二进制读取未迁移的 limits 对象。

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

`maxActiveJobs` 也可以设置为 `"auto"`。`sessionIdleTimeoutSecs` 历史上从未控制运行时行为，必须删除。v0.9 会明确拒绝这两个旧字段，不会静默回退到默认值。

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

不提供兼容 alias。tmux session 名称和 tmux API 不变。

## HTTP 与响应迁移

- `/v1/exec` → `/v1/process/exec`
- `/v1/batchExec` → `/v1/process/batch`
- `/v1/sessions/*` → `/v1/jobs/*`
- `mcp.callTool` 改为返回 managed `JobResponse`，不再 raw passthrough
- 新增 `/v1/mcp/batch`
- `job.get/cancel` 返回 `JobDetail`；Hub 只有缓存摘要时 `detailAvailable=false`

Custom GPT 必须替换为 v0.9 的 [`openapi/hub.yaml`](../openapi/hub.yaml)。

## 推荐部署顺序

1. 停止 v0.8 Local Agent。
2. 停止 v0.8 Hub。
3. 迁移每台 Local Agent 配置。
4. 同时替换 Hub 与 Agent 二进制。
5. 先启动 Hub，再启动 Local Agent。
6. 刷新 Actions schema，并重新连接 Apps MCP client。

## 验收

```bash
agentic-gpt --version
agentic-gpt-hub --version
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info \
  --config ~/.agentic_gpt/config.json \
  --arguments '{}'
```

预期：

- 两个二进制均为 `0.9.0`；
- Normal surface 24 个工具，Room surface 36 个工具；
- `job.get/list/cancel` 与 `mcp.batch` 存在；
- managed `session.*`、`process.get/list/kill`、`process.batchExec` 不存在；
- `agent.info.execution.jobs` 与 `agent.info.mcp.concurrency` 存在；
- local/tunnel descriptor revision 一致。
