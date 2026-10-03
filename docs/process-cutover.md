# Process API 与存储切换说明

本页记录当前 `process.exec`/`process.batch` 的 Bash 输入切换、统一 `process.read` 接口切换与较早的 `Job`→`Process` 历史迁移。各次切换都不提供已移除输入或读取接口的别名；进程历史继续使用既有存储，不新建、迁移或清空数据库。当前部署须协调更新 Agent、Hub 与调用方。

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

### 当前 Bash 命令输入切换

当前 `process.exec` 必须提供原始 shell 脚本 `command`，可选工作目录字段为 `cwd`；`process.batch` 的每个元素也必须提供 `command`，并可使用批次级 `cwd`，由元素覆盖。旧 `program`、`args`、`workingDirectory` 字段不是当前输入，也没有兼容别名。若客户端原先传入可执行文件和参数数组，必须自行按 shell 规则显式引用/转义每个参数，再构造 `command`；不得把数组直接拼接成脚本。保留在非输入 `ProcessInfo` 历史元数据中的旧字段，不代表这些字段仍可作为执行请求输入。

Shell 策略现在针对提交的整个脚本做静态预检，而不是为每个 `program`/`args` 调用组装单独命令：任一可判定的拒绝会在执行脚本前拒绝整个请求；无法匹配、语法不完整或不支持的结构要求对整段脚本确认。缺少确认通道时 fail closed。不要据此扩大默认 Bash 白名单。普通字面量 `printf` 可用于无副作用的冒烟检查；需要通用 shell 结构的脚本可能要求显式确认。

历史持久化记录不会改写成新格式或重新执行：旧输入的原始命令、哈希、身份和已有结果保持不变；旧的未完成记录退休为 `UnknownAfterRestart`，绝不重放；已完成记录仍可重放其保存结果。此次切换不做数据库 rewrite/migration，也不改变 Process 历史数据库。

升级前应停止或盘点正在运行的旧调用，同步部署配对的 Agent/Hub，并迁移所有客户端；旧输入将被拒绝，不支持新旧输入混用或回退式兼容。已完成结果可按历史保留规则读取；未完成旧调用不会由新版本续跑。

`process.cancel` 对受管理进程组发 TERM，等待后再发 KILL；如组成员仍存活，保留进程组并继续占用执行容量。`process_group_sigterm_observed` 与 `process_group_sigkill_observed` 是观察到相应停止信号的正面语义证据，必须与未验证/脱离等结果区分。取消覆盖普通同组管道及后台后代，不保证脱离该进程组的后代停止。执行终态与 stdout/stderr 捕获 EOF 分开报告；捕获尚未 EOF 不会让已经结束的执行仍被称为运行中。

当前公开 Process 工具仅为 `process.exec`、`process.batch`、`process.read`、`process.list` 和 `process.cancel`；旧 `process.status`、`process.output`、`process.result` 工具及旧 HTTP 路径直接移除，不提供 wire/http 兼容别名。HTTP read 的 wait 默认 5 秒、最大 30 秒、0 立即；view 为 `auto` 或 `status`。响应包含 `captureStatus`，输出页包含 `gap`、`eof`、`hasMore`；hasMore 不要求读完整日志。cursor 仅 command/skill，非消费且不同读取者不共享。

统一响应预算由 `limits.processResponseBytes` 控制，默认 8192 字节，范围 4096..1048576；read 可用 `maxBytes` 显式覆盖。预算是序列化响应 JSON（含转义/Base64），不含传输/event 封套；与 MCP 结果 512 KiB 保留上限分离，`mcp.batch` 整个聚合响应共用预算。MCP 完整 CallToolResult 状态为 `pending`、`included`、`deferred`、`unavailable` 或 `not_retained`；完整对象不切碎，`not_retained` 不可恢复。

`mcp.batch` 因聚合预算省略某个子项的正文时，只要该进程仍保留完整结果，子项就返回 `mcpResult.status: "deferred"`，不是 `unavailable`。需要该结果时，使用子项的 `agentId`、`processId` 调用 `process.read` 并按需提高 `maxBytes`；批次响应未包含正文不代表结果已丢失。

## 数据与升级安全

本次统一读取沿用既有 `process.sqlite3`。Agent 的 `transport-runs.jsonl` 及 Hub 已持久化的旧读取请求保留原始命令、身份、哈希和已有结果，用于历史、去重及恢复边界；未完成的旧读取请求显式退休，不改写成新命令重新执行。

仅当从更早的 Job 版本升级时，才涉及 `jobs.sqlite3` 与 `process.sqlite3` 的历史边界：Process 运行时不会打开、迁移或删除旧 `jobs.sqlite3`，旧 Job 记录不导入 Process 历史。保留旧文件，维护前备份；不要重命名旧数据库来伪装成 Process 历史。

升级前先停止或清点正在执行的工作，同步迁移 Agent、Hub 和客户端。旧二进制与新 API 不支持混合版本兼容模式；回滚同样需要匹配的 Agent、Hub 和客户端。不要假定回滚可以重建已完成的外部效果或恢复正在执行的工作。回滚至旧 Job 版本时，该版本无法看到 Process 历史，应另行保留 `process.sqlite3`。

本说明描述切换契约；不表示托管发布、交叉构建或每项外部集成都已验证。
