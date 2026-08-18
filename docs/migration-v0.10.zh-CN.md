# Agentic GPT v0.10 迁移说明

文件工具现在保留单次读/搜的易用平铺字段，并把批量能力合并到同一个
工具的 `requests` 字段；原来的组合文件批处理工具已移除。

- `file.read` 单次使用平铺字段，批量使用 1–32 个 `requests`。
- `file.search` 单次使用平铺字段，批量使用 1–32 个 `requests`。
- 两种形式互斥；批量结果按输入顺序返回，单项失败不会压制其他项。
- `file.edit` 只接受 Codex apply-patch 文本，可跨文件 Add、Update、Delete、Move。
  旧的 mode/path/revision/content 字段不再接受。
- `dryRun` 只校验和预览，不确认、不写入；`needConfirm` 对整份有效补丁只请求一次确认。

读取/搜索批量调用查看有序 `results`，编辑查看 `changes` 与 `summary`。解析、路径/
上下文、暂存、确认以及最终再次校验若失败，都发生在第一次物理提交之前，因此不会
写入任何文件。物理提交开始后不承诺跨文件回滚；若后续某项提交失败，`file.edit`
返回 `completed_with_errors`，并按顺序明确标记哪些变更已经提交、哪一项失败，以及
哪些后续变更尚未尝试。
