# Agentic GPT v0.10 迁移说明

v0.10 将 `file.batch` 的变更边界从“尽力跨文件回滚”改为相互独立的规范化文件组。

## `file.batch`

- 继续使用扁平有序的 `operations` 数组；不再提供 `atomicity` 选择器。
- 同一规范化目标的重复编辑按输入顺序作用于一个内存候选，只产生一次物理替换。
- 读取和搜索仍使用编辑前快照；它们失败时不再阻止无关的有效文件组提交。
- 失败只影响所属文件组。其他组可以提交，混合结果使用 `completed_with_errors`，并提供有序操作错误和有界的 `groups` 摘要。
- 确认仍是针对所有有效变更组的一次聚合决定；拒绝时不会写入任何已暂存组。
- `dryRun:true` 是整批校验和预览机制；后续真实调用仍以精确的 `expectedRevision` 与 `expectedAbsent:true` 守卫为准。
- 删除旧的 `rolled_back`、`partial_failed`、`rollback_failed`、`not_committed` 状态及仅用于回滚的错误码。普通提交失败不会撤销已经提交的组，而是报告受影响组失败。

调用方应只根据 `completed`、`completed_with_errors`、`rejected`、`dry-run` 分支处理，结合 `groups` 判断每个文件的物理状态，不要从批量成功响应推断跨文件原子性。
