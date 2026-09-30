# 调研发现

## 已核对事实
- 根目录存在 .codegraph，已先调用 codegraph explore。
- 开发指南要求入口封套各自保留，单一状态所有者，不机械把本地能力复制到 Hub。
- AppState 已持有 processes、process_history、skill_installs；process spec 已有 terminal_event_hook。
- 已有 target/debug/agentic-gpt 可用于无真实用户状态的隔离冒烟。

## 待核对
- 统一结果骨架/输出 schema 和面板正确接入位置。
- 终态 hook 与持久化/恢复的关系。
- 外部注入现有 Unix 协议与权限门。

## 用户目标
- event.list、event.mark、event.get。
- 每次调用固定简洁面板：未处理事件、产生时间、紧急程度。
- low：一次展示后隐藏，可配置过期；medium：三次后隐藏，不按时间过期；high：一直展示。
- 内部异步完成与外部 stdin/Unix socket 注入。

## 入口冒烟证据
- 使用已有 target/debug/agentic-gpt，在一次性 HOME/XDG/config/workspace 启动 local 模式。
- 真实 Unix MCP 调用 agent.info、process.list；两者返回 content/isError/structuredContent。
- process.list 的 structuredContent 仅 processes；当前无事件面板。验证的是现有二进制，不宣称其与所有源码完全一致。
- 服务已停止，临时目录已自动清理；未运行测试或构建。

## 设计约束（建议，尚未实现）
- 展示次数与处理状态分开，low/medium 隐藏不等于 handled。
- 服务端只能统计面板输出，不能证明模型实际读过；并发输出可能消耗次数。
- 多消费者必须选择共享 inbox 还是 recipient 级 inbox，不能按每次连接重置计数。
- high 永久候选和简洁有界面板之间存在容量冲突，应明确 overflow 而非暗中丢事件。
- 只在授权后的工具结果展示；协议错误/未认证请求不附内部事件。

## 已核对集成边界
- stdio/Unix/Agent HTTP MCP 共用 AgentMcpServer::call_tool（stdio_server.rs:1494-1510）；special_result 提前返回，固定面板必须覆盖该分支并保留 image/browser 原结果。
- 通用 output_schema 目前只是 object/additionalProperties=true（stdio_schema.rs:804-809），尚无事件字段。
- Unix socket 是 MCP 而非任意 JSON 控制帧；同 UID 验证、目录 0700、socket 0600（local_control.rs:34-74,199-232）。
- stdio stdin 已承载 MCP，不能混入裸事件行；stdin 支持应放在独立注入 CLI 中，再调用 Unix MCP。
- process finalize 先尝试写 history，之后 audit.take() 并调用可选 hook；hook 与历史成功不原子（managed.rs:2229-2308，已直接阅读核对）。
- skill install 有独立 JSON 持久状态和 recover；其完成、失败、取消需从 InstallManager 终态路径发事件，不可仅靠 process hook（scout：skill_installs.rs:590-704）。
- process history 保留 30 天；install 终态记录受 7 天/100 条约束。事件生命周期必须独立。
- 现有业务记录无可恢复的 consumer 身份；request_source 不是 owner。
- Hub AgenticResult、wire 与 REST 是独立合同，不应将 Agent 面板机械加入 Hub。
