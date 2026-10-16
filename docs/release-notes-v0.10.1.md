# Agentic GPT 0.10.1 发布说明

相对于 `v0.10.0` 的确认修复，适用于 Agent、Hub 和共享 Protocol `0.10.1`。

- 修复长命令因操作预览超过 1000 字符而立即返回 `process_confirmation_unavailable` 的问题。
- ntfy／freedesktop 统一简短通知：主动确认显示 `reason`；自动确认显示有界规则，默认 Confirm 或解析不完整时显示固定原因及最多 30 个字符的预览，超长以“…”截断。
- `tmux.pasteText` 基础策略改为 Confirm，标题为“请求写入 tmux”；未获准不写入，既有 shell 窗格保护继续生效。
- freedesktop 根据通知服务的标记能力转义正文，正确展示请求文本。

**确认请求参数不兼容：** 删除 `acquireConfirmation.acquire`；对象存在即主动请求，`reason` 必填，原文最多 100 个 Unicode 字符且去除首尾空白后非空。旧 `acquire=false` 改为省略整个对象；旧 `acquire=true` 改为填写请求原因。详见[调用方升级](configuration.zh-CN.md#调用方升级)。

使用 Hub 的部署须配套更新 Agent、Hub、调用方及导入的 OpenAPI；Local／Standalone 的调用方也须更新请求。下游 MCP 专用审批语义保持不变。从更早版本升级时另见 [0.10.0 升级步骤](release-notes-v0.10.0.md#升级步骤)。
