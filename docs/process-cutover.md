# Process API 与存储切换说明

本页区分本次统一 `process.read` 的接口切换与较早的 `Job`→`Process` 迁移。本次移除旧 Process 读取接口，不提供别名或兼容双轨，但继续使用既有 Process 历史存储，不新建、迁移或清空数据库。Agent、Hub 与调用方需要协调升级。

## API 迁移

### 本次统一读取接口

| 旧调用 | 新调用 |
|---|---|
| MCP `process.status` | `process.read`，需要只读状态时指定 `view: "status"` |
| MCP `process.output` | `process.read`，使用返回的 `output.nextCursor` 续页 |
| MCP `process.result` | `process.read`，读取完整 `mcpResult.value` 或结果可用性状态 |
| HTTP `GET /v1/process/{processId}` | `GET /v1/process/{processId}/read?view=status` |
| HTTP `GET /v1/process/{processId}/output` | `GET /v1/process/{processId}/read`，按需携带 `cursor` |
| HTTP `GET /v1/process/{processId}/result` | `GET /v1/process/{processId}/read` |

### 从较早的 Job 客户端升级

以下名称属于更早的接口代际，不是本次才移除的 Process 读取工具。

| 旧调用 | 新调用 |
|---|---|
| MCP `job.get` | `process.read`（默认 auto；需要时用 `view: "status"`，输出使用非消费 cursor） |
| MCP `job.list` / `job.cancel` | `process.list` / `process.cancel` |
| Hub MCP `hub.job.get` / `hub.job.list` | `hub.process.status` / `hub.process.list`（独立 cache-only 元数据投影；不可等待或获取正文） |
| HTTP `GET /v1/jobs` | `GET /v1/process` |
| HTTP `GET /v1/jobs/{jobId}` | `GET /v1/process/{processId}/read`（统一状态/输出读取） |
| HTTP `POST /v1/jobs/{jobId}/cancel` | `POST /v1/process/{processId}/cancel` |

当前公开 Process 工具仅为 `process.exec`、`process.batch`、`process.read`、`process.list` 和 `process.cancel`；旧 `process.status`、`process.output`、`process.result` 工具及旧 HTTP 路径直接移除，不提供 wire/http 兼容别名。HTTP read 的 wait 默认 5 秒、最大 30 秒、0 立即；view 为 `auto` 或 `status`。响应包含 `captureStatus`，输出页包含 `gap`、`eof`、`hasMore`；hasMore 不要求读完整日志。cursor 仅 command/skill，非消费且不同读取者不共享。

统一响应预算由 `limits.processResponseBytes` 控制，默认 8192 字节，范围 4096..1048576；read 可用 `maxBytes` 显式覆盖。预算是序列化响应 JSON（含转义/Base64），不含传输/event 封套；与 MCP 结果 512 KiB 保留上限分离，`mcp.batch` 整个聚合响应共用预算。MCP 完整 CallToolResult 状态为 `pending`、`included`、`deferred`、`unavailable` 或 `not_retained`；完整对象不切碎，`not_retained` 不可恢复。

`mcp.batch` 因聚合预算省略某个子项的正文时，只要该进程仍保留完整结果，子项就返回 `mcpResult.status: "deferred"`，不是 `unavailable`。需要该结果时，使用子项的 `agentId`、`processId` 调用 `process.read` 并按需提高 `maxBytes`；批次响应未包含正文不代表结果已丢失。

## 数据与升级安全

本次统一读取沿用既有 `process.sqlite3`。Agent 的 `transport-runs.jsonl` 及 Hub 已持久化的旧读取请求保留原始命令、身份、哈希和已有结果，用于历史、去重及恢复边界；未完成的旧读取请求显式退休，不改写成新命令重新执行。

仅当从更早的 Job 版本升级时，才涉及 `jobs.sqlite3` 与 `process.sqlite3` 的历史边界：Process 运行时不会打开、迁移或删除旧 `jobs.sqlite3`，旧 Job 记录不导入 Process 历史。保留旧文件，维护前备份；不要重命名旧数据库来伪装成 Process 历史。

升级前先停止或清点正在执行的工作，同步迁移 Agent、Hub 和客户端。旧二进制与新 API 不支持混合版本兼容模式；回滚同样需要匹配的 Agent、Hub 和客户端。不要假定回滚可以重建已完成的外部效果或恢复正在执行的工作。回滚至旧 Job 版本时，该版本无法看到 Process 历史，应另行保留 `process.sqlite3`。

本说明描述切换契约；不表示托管发布、交叉构建或每项外部集成都已验证。
