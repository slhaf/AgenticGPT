# Process API 与存储切换说明

这是一次破坏性 Job→Process clean cutover，不提供旧接口别名或兼容双轨。升级时应协调部署 Agent 与 Hub，并同步升级所有调用方（包括 TUI、HTTP/MCP 客户端和自动化）；旧客户端不能假定仍可使用 Job surface。

## API 迁移

| 旧调用 | 新调用 |
|---|---|
| MCP `job.get` | `process.status`（仅元数据）；按需使用 `process.output` 和 `process.result` 获取输出与结果 |
| MCP `job.list` / `job.cancel` | `process.list` / `process.cancel` |
| Hub MCP `hub.job.get` / `hub.job.list` | `hub.process.status` / `hub.process.list` |
| HTTP `GET /v1/jobs` | `GET /v1/process` |
| HTTP `GET /v1/jobs/{jobId}` | `GET /v1/process/{processId}`（仅元数据），另用 `GET /v1/process/{processId}/output` 和 `/result` |
| HTTP `POST /v1/jobs/{jobId}/cancel` | `POST /v1/process/{processId}/cancel` |

Status and list responses contain metadata only: they do not include output or result payloads. Fetch output through the output endpoint (default 8 KiB, maximum 32 KiB cursor window); fetch a completed result separately (maximum 512 KiB). Treat reported lifecycle/freshness status as authoritative only for its stated scope; status is not evidence that an external side effect was rolled back. Hub status/list are projections, not Agent execution authority.

## Data and upgrade safety

The Agent writes current Process history to a fresh private `process.sqlite3`. It does **not** open, migrate, or delete the previous `jobs.sqlite3`; leave that file untouched. Existing rows in the old database are intentionally unavailable through the new Process API and are not imported into the new history. Back up both files before any manual maintenance, and do not rename the old database to `process.sqlite3` or treat its contents as current Process history.

Upgrade Agent and Hub together so the new Process identity, projections, and routes agree. Before upgrading, stop or account for active work and move every client to the new names; old binaries/clients and the new API are not a supported mixed-version compatibility mode. A rollback requires deploying the previous Agent and Hub together with their matching clients. The previous release will not see Process history/results written to `process.sqlite3`; preserve that file separately, and do not assume rollback can reconstruct completed external effects or recover new-format history. Prefer rollback only after safely accounting for in-flight operations.

This note describes the cutover contract; it does not claim that hosted publication, cross-builds, or every external integration has been verified.
