# Agentic GPT v0.9.1 release notes

本文记录 v0.9.1 release 的 breaking `file.batch` 契约与验证边界；不创建
tag、不 push 分支、不部署 artifact，也不发布 release。

## 重点变化

- `file.search` 接受有界的 context overshoot，按 live
  `limits.maxFileSearchContextLines` 裁剪，并返回 requested/effective context
  与有界 warning。
- 同一规范化 `file.batch` 目标的重复编辑在一个 candidate 上串联，只产生一次物理替换。
- `file.batch` 按规范化文件组独立提交；读取/搜索、暂存、确认和提交失败彼此隔离，混合结果使用
  `completed_with_errors` 并提供有界的 group/operation 证据。
- 删除跨文件 rollback 的保留、执行、响应状态和 rollback-only 错误码。`dryRun:true` 仍是整批预览路径，
  一次确认仍覆盖有效变更组。
- 工具描述/schema 明确条件守卫、Job 后续操作、准入与副作用边界，以及 Standalone/Hub parity；矩阵覆盖
  Normal 24 / Room 36 与 Hub profiles。
- 9 个 deterministic contract case 通过 descriptor、serde、dispatch、dry-run；可选 provider-neutral evaluator
  不访问网络或 credentials。

## Breaking changes 与迁移

升级调用方前请阅读 [`migration-v0.10.zh-CN.md`](migration-v0.10.zh-CN.md)：

- 不再发送或依赖 `file.batch.atomicity`。
- 将 `completed_with_errors` 与 `groups` 视为正常的部分成功证据；不要从整批成功推断跨文件原子性。
- 删除 `rolled_back`、`partial_failed`、`rollback_failed`、`not_committed` 及仅用于 rollback 的错误码处理。
- 保留精确 `expectedRevision` / `expectedAbsent:true` 守卫；检查当前状态后只重试失败的规范化文件组。

现有 v0.9 的 Job 名称/config 迁移仍适用；本次 v0.9.1 不增加兼容 alias。

## 验证边界

已完成格式、workspace check、workspace clippy、workspace tests、local-control、standalone supervisor、
contract corpus/evaluator 与 `git diff --check` 验证。启用本地 socket/HTTP/子进程权限后，Agent 全套 242/242、Hub
全套 59/59 通过。不包含 tag、push、部署或发布。
