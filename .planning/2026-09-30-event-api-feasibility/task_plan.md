# 异步事件 API 可行性调研

状态：仅调研，不实现代码，不修改公开契约。
选定计划目录：.planning/2026-09-30-event-api-feasibility；不切换其他任务的 active 指针。

## 阶段一：现状核对
Status: complete
- 核对响应骨架和入口、process/skill 安装终态与存储。
- 使用只读 scout 分别核对入口和生产者。
- 隔离运行已有入口，观察当前响应。

## 阶段二：方案评估
Status: complete
- event.list/event.mark/event.get 与固定面板。
- low/medium/high 展示次数、过期、状态所有权和并发。
- 内部生产与 stdin/Unix socket 外部接入。

## 阶段三：结论
Status: in_progress
- 提供可行性、建议架构、风险与待定决策。
- 不做实现提交；按仓库阶段提交约束只提交调研记录，记录不代表已交付行为。

## 错误
无。
