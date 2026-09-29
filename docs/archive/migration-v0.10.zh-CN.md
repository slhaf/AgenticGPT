# Agentic GPT v0.10 迁移说明

文件工具现在保留单次读/搜的易用平铺字段，并把批量能力合并到同一个
工具的 `requests` 字段；原来的组合文件批处理工具已移除。

- `file.read` 单次使用平铺字段，批量使用 1–32 个 `requests`。默认返回内容；
  `metadata: true` 额外附带元数据；截断只返回 `nextStartLine`，不会切断单行。
- `file.search` 单次使用平铺字段，批量使用 1–32 个 `requests`。
- 两种形式互斥；批量结果按输入顺序返回，单项失败不会压制其他项。
- `file.edit` 只接受 Codex apply-patch 文本，可跨文件 Add、Update、Delete、Move。
  旧的 mode/path/revision/content 字段不再接受。
- `needConfirm` 对整份有效补丁只请求一次确认。

读取/搜索批量调用查看有序 `results`。`file.edit` 正常成功只返回精简的已提交
路径/动作列表；部分提交失败才返回按顺序的状态与错误证据。详细 diff、resolved path、
changed-line 统计和 revision hash 仅保留在内部 confirmation/audit 路径。解析、路径/
上下文、暂存、确认以及最终再次校验若失败，都发生在第一次物理提交之前，因此不会
写入任何文件。物理提交开始后不承诺跨文件回滚；若后续某项提交失败，`file.edit`
返回 `completed_with_errors`，并按顺序明确标记哪些变更已经提交、哪一项失败，以及
哪些后续变更尚未尝试。

## Managed Job 契约

Managed Job 的常规响应现已精简，丰富且有界的明细继续保留在内部历史中。
`process.exec`、`process.batch`、`skills.run`、`mcp.callTool` 和 `mcp.batch`
均可传可选的人类可读 `group`，批量子 Job 继承父级 `group`。`job.get`
支持 `waitOnly`；`job.list` 支持精确 `group`/kind/state 过滤、默认 50
（最大 100）的 limit，以及不透明 cursor 分页。终态 Job 在持久历史保留期内
仍可通过 `jobId` 精确回看。Hub full 与 HTTP Job 路由在 Agent 在线时透传这些
字段；Hub 缓存降级不会伪造 cursor 续页，也不会把缓存快照冒充为一次新的等待结果。
