# Process API 与存储切换说明

这是一次破坏性的 `Job`→`Process` 彻底切换，不提供旧接口别名或兼容双轨。升级时应协调部署 Agent 与 Hub，并同步升级所有调用方（包括 TUI、HTTP/MCP 客户端和自动化）；旧客户端不能假定仍可使用 `Job` 接口面。

## API 迁移

| 旧调用 | 新调用 |
|---|---|
| MCP `job.get` | `process.status`（仅元数据）；按需使用 `process.output` 和 `process.result` 获取输出与结果 |
| MCP `job.list` / `job.cancel` | `process.list` / `process.cancel` |
| Hub MCP `hub.job.get` / `hub.job.list` | `hub.process.status` / `hub.process.list` |
| HTTP `GET /v1/jobs` | `GET /v1/process` |
| HTTP `GET /v1/jobs/{jobId}` | `GET /v1/process/{processId}`（仅元数据），另用 `GET /v1/process/{processId}/output` 和 `/result` |
| HTTP `POST /v1/jobs/{jobId}/cancel` | `POST /v1/process/{processId}/cancel` |

状态和列表响应只含元数据；不含输出或结果负载。通过输出端点获取输出（默认 8 KiB，最大 32 KiB 游标窗口）；单独获取已完成结果（最大 512 KiB）。报告的生命周期/新鲜度状态仅在其声明范围内具有权威性；状态不能证明外部副作用已回滚。Hub 状态/列表是投影，不是 Agent 执行权威。

## 数据与升级安全

Agent 会将当前 Process 历史写入新的私有 `process.sqlite3`。它不会打开、迁移或删除旧 `jobs.sqlite3`；应保留该文件原样。旧数据库的现有记录会有意地无法通过新的 Process API 访问，也不会导入到新历史中。任何手动维护前都应备份两个文件；不要将旧数据库重命名为 `process.sqlite3`，也不要将其内容视为当前 Process 历史。

将 Agent 与 Hub 一起升级，使新的 Process 标识、投影和路由保持一致。升级前先停止或清点正在执行的工作，并将每个客户端迁移至新名称；旧二进制/客户端与新 API 不支持混合版本兼容模式。回滚需要将旧版 Agent 和 Hub 与相匹配的客户端一起部署。旧版看不到写入 `process.sqlite3` 的 Process 历史/结果；应单独保留该文件，且不得假定回滚能重建已完成的外部效果或恢复新格式历史。只有在安全清点所有正在进行的操作后，才宜回滚。

本说明描述切换契约；不表示托管发布、交叉构建或每项外部集成都已验证。
