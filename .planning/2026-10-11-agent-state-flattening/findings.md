# 实施依据

前期证据见 ../2026-10-11-agent-state-directory-audit/findings.md；本文件记录实施新增事实与选择，旧候选建议不覆盖用户最终边界。

- 工作区开始干净；AGENTS.md 已有临时 HOME 规范。
- 独立脚本默认保留旧源，不执行真实设备迁移；用户状态清理不在范围。
- 三个并行子任务按文件隔离：测试 fixture/parity；private_state/event_store；独立迁移脚本。主 agent 处理本地进程模型、wire adapter、process history、文档整合。

- 本地 process/model.rs 定义 ProcessInfo/Detail 及详细 MCP batch，均无 agentId；短 wire response 继续使用协议类型并从 transport_agent_id 注入，最终序列化大小预算仍含该字段。Hub ProcessUpdate/RunReport 显式映射到共享协议类型。
- transport_agent_id 是启动配置的连接身份快照；live reload 仅更新安全子集，agentId 已是 restart-required，保持现有语义。
- schema：事件 v4 去 owner metadata；进程 v2 去 processes.agent_id/info_json/detail_json.process.agentId；启动无旧版本DDL升级。
- 首次全套回归暴露短 TMPDIR 下 socket symlink fixture 长度和旧测试 identity 假设，已缩短路径、明确配置身份；临时只读浏览器包使用临时根内 chmod 清理。
- 独立迁移脚本实际回归 2 passed：真实 local Agent 读历史/输出，换连接身份重启保留同根状态；源不变、skip、不合并其他身份、错误拒绝与发布回滚。
