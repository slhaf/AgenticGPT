# 历史设计、迁移与发布记录

本目录保存有日期的历史设计、架构调查及版本化迁移/发布记录。它们供追溯对应时期的决策与行为，**不是当前功能说明或待执行任务清单**。使用、部署和接口行为请以现行文档、实现与契约为准。

| 归档资料 | 保留用途 | 相关资料 |
|---|---|---|
| [2026-09-23 架构诊断](architecture-diagnosis-2026-09-23.md) | 2026-09-16 的 A01–A09 调查基线、2026-09-23 的 R01–R06 复审及根因证据 | [历史现状快照](architecture/current-state.md)、[历史重构路线图](architecture/refactoring-plan.md) |
| [2026-08-04 配置向导设计](config-init-wizard-design-2026-08-04.md) | 初始模式、模板和配置发现问题；交互部分已由后继设计替代 | [配置说明](../configuration.zh-CN.md) |
| [2026-08-07 全屏 TUI 设计](config-init-fullscreen-tui-design-2026-08-07.md) | 全屏交互、Esc/取消与分阶段编辑的设计取舍 | [配置说明](../configuration.zh-CN.md) |

版本化迁移说明和发布说明也已归档于本目录；以下链接仅供查阅对应版本的历史范围与事实，不代表当前行为：

- [v0.10 迁移说明（英文）](migration-v0.10.md) · [简体中文](migration-v0.10.zh-CN.md)
- [v0.9 迁移说明（英文）](migration-v0.9.md) · [简体中文](migration-v0.9.zh-CN.md)
- [v0.9.1 发布说明（英文）](release-notes-v0.9.1.md) · [简体中文](release-notes-v0.9.1.zh-CN.md)
- [v0.9.0 发布说明](release-notes-v0.9.0.md)

不要将历史版本事实改写为当前行为。

原 2026-08-04 `inquire` 实施任务清单已退场，不应再据其未勾选步骤安排工作。
