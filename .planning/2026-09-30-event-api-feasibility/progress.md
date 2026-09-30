# 调研进度

- 已阅读 planning skill、开发指南、文档标准。
- 初始 git diff --stat 无输出；未改实现代码。
- 两个只读 scout 并行核对入口封套与异步终态。
- 仅生成本任务计划记录；没有改 schema 或 API。

## 阶段一完成
- 两个 scout 已完成；直接核对 call_tool 与 finalize_process 核心范围。
- 隔离运行现有 Unix MCP 服务，agent.info/process.list 响应已观察；无事件面板。
- 可复用 Agent-local 共享所有者和 MCP 结果边界；持久交付状态需新增，当前终态日志不等于 inbox。
- 阶段提交仅调研记录；未修改代码/API/schema。

## 阶段二完成
- 已给出三个消费 API、统一面板合同迁移建议、曝光/处理/过期独立状态语义。
- 已比较共享 inbox 和稳定消费者 inbox，指出 high 常驻与面板有界的冲突。
- 已直接核对 skill install 的 Completed/Failed/Cancelled 保存点和 JSON rename 边界。
- 外部写入建议独立 CLI 的 stdin → 已有 Unix MCP → 本地准入操作；不污染服务 stdin，不默认暴露模型创建工具。
- 未实现新 API，未把建议写成现有行为。
