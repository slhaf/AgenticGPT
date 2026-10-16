# Confirmation 修复

## 已确认行为

- 移除 acquire；acquireConfirmation 对象存在即主动请求；reason 必填非空，最多100 Unicode字符；method可选，原支持接口保留read/write。
- 主动通知正文只reason。自动命中规则只显示短规则；默认Confirm或分析不完整显示固定原因+原始命令最多30字符预览，含Unicode省略号、合并空白。
- tmux.pasteText基础决策Confirm，取消default_acquire/strict隐藏例外；标题请求写入tmux；自动正文为短原因+输入预览。
- ntfy/freedesktop共用标题与短正文，无详情页；普通确认失败回退原策略，MCP专用审批保持原行为。
- 不增加兼容层，Agent/Hub配套升级；临时HOME/XDG/TMP/D-Bus隔离验证。

## 阶段及提交

1. 参数、schema与确认语义统一，迁移调用方和现有测试。complete
2. 策略依据与短通知展示，长命令链路回归。complete
3. 文档、完整验证及结果记录。complete

## 测试保留价值

- 复用 acquisition_controls_process_effects_and_reuses_failed_confirmation，迁移输入只为保持已覆盖真实授权/副作用/不重复确认，避免重复建测试。
- 长命令需覆盖真实入口到Hub通知及批准执行，抓取之前>1000预览的误报、正文绕过用户文案、审批前执行；现有短请求测试未覆盖该失败模式（④/⑤）。
