# 调查发现

- 仓库存在 CodeGraph，已优先使用索引定位当前源码。
- Agent operations/policy.rs 决定命令策略，operations/confirmation.rs 负责确认渠道与等待。
- Hub runtime/confirmation.rs 拥有待确认记录、token、到期与决策投递。
- needConfirm 需区分命令执行、文件操作、下游 MCP 和 tmux 的实际消费。
- 历史记录仅帮助定位共享授权边界，所有行为结论重新核对当前源码。
