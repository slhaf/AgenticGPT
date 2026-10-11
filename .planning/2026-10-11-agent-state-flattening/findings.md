# 实施依据

前期证据见 ../2026-10-11-agent-state-directory-audit/findings.md；本文件记录实施新增事实与选择，旧候选建议不覆盖用户最终边界。

- 工作区开始干净；AGENTS.md 已有临时 HOME 规范。
- 独立脚本默认保留旧源，不执行真实设备迁移；用户状态清理不在范围。
- 三个并行子任务按文件隔离：测试 fixture/parity；private_state/event_store；独立迁移脚本。主 agent 处理本地进程模型、wire adapter、process history、文档整合。
