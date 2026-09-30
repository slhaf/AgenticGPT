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

## 建议 API 与 schema（未实现）
- event.list：按状态/级别/来源过滤，稳定 cursor 分页；默认 pending，包括自动面板已隐藏的 pending。
- event.get(eventId)：读取完整事件、业务实体引用、展示和过期状态，不自动处理事件。
- event.mark(eventIds, status=handled)：明确确认，幂等；先标记，再生成响应面板。
- 通用 MCP structuredContent 骨架推荐明确迁移为 { result, eventPanel }；普通工具 result 是原结构结果，原本无 structuredContent 的特殊工具不捏造业务 result。
- 多模态原 content blocks 保留，追加独立简短面板 text；不可仅藏在 _meta，也不可遗漏 special_result。
- eventPanel 固定存在，含 pendingCount、hiddenCount、items、hasMore；无事件时计数0、items=[]。
- 应更新统一 output_schema 和所有本地消费者；这是合同迁移，不宣称完全兼容旧 flat 结果。

## 建议生命周期（未实现）
- 处理状态 pending/handled/expired 与面板曝光计数分开。
- low（用户原文 lowd）：曝光1次后隐藏，可配置 TTL；medium：3次后隐藏，无自动 TTL；high：直到 handled 一直有展示资格。
- 曝光只统计自动面板实际选中的事件，不统计 list/get 正文；同一响应 text+structured 两份算1次。
- 原子选择、计数、mark；采用面板逻辑响应生成计数，不保证真实阅读或 exactly-once 交付。
- 初始按 Agent 共享 inbox 建议可行，但任何入口客户端都会消耗配额；如要隔离消费者，需稳定 consumerId，不可按临时连接计数。
- high 全部每次展示与固定有界面板不可同时保证：建议明确 hasMore/总数及 event.list；若要求逐条每次必见，则响应可能无界。
- low TTL 与展示次数独立；配置策略在创建时形成 expiresAt，避免 reload 改写已创建事件的期限。
- medium/high pending 不能按时间悄悄删；hidden 也非删除。handled/expired 另有明确历史保留策略。

## 建议生产与接入（未实现）
- Agent-local EventStore 持久事件与曝光状态，由 AppState 共享，业务终态仍由 process/install owner 拥有。
- 内部按实体稳定 ID + 终态事件类型去重；同步 inline 已返回的完成不重复作为异步提醒。
- 重启可靠性需 durable outbox/同事务或基于持久终态的恢复补偿；仅 callback 有崩溃漏报窗口，skill JSON 与 process SQLite 不是同一事务。
- 外部使用受准入控制的本地事件写入操作；模型侧仍保持三个消费 API，不必默认开放创建工具。
- stdin 仅由独立注入 CLI 接收一个 JSON 请求，再经现有 Unix MCP 通道提交；不另建裸 socket 协议。
- 外部记录真实接收时间 receivedAt，允许附来源发生时间 occurredAt；source 与本机调用者身份分开，外部去重键按来源命名空间限定。
- 外部正文不可信，不作为可执行指令；设载荷/队列上限，满时明确拒绝，不暗中删除 high。
- Agent-local 面板不意味着 Hub MCP 自动覆盖；如“每次调用”包括 Hub，需同步评估 wire、Hub 封套、OpenAPI 与消费者，不静默遗漏。

## 结论与实现前必须明确的边界
- 可行；三个 Agent MCP 入口已有共同结果出口，无需先搭完整消息总线。
- 推荐 Agent-local 持久 inbox、显式输出骨架迁移、生产者终态接入、独立 CLI stdin 经 Unix MCP 注入。
- 三个消费工具本身简单；可靠终态生产/重启恢复、所有结果分支覆盖、消费者迁移是主要工程量。
- 工具返回携带提醒是拉取/捎带，不是空闲主动唤醒；无下一次调用便不可见，low 可能在无人调用期间过期。
- 待产品确定：Agent 全局还是稳定 consumer 收件人；high 是否允许有界列表+overflow；范围是否包含 Hub；是否需要空闲主动推送。
- 本轮仅运行既有二进制与读取源码；新 API、新 schema、持久事件交付均未实现或验证。

## 用户补充：按紧急程度统计
- 面板增加固定 pendingCounts: { low, medium, high }，三个键始终存在，无事件时为0。
- 计数统计同一授权/收件箱范围内所有 pending 且未过期事件，不限当前 items，也包含曝光额度耗尽后隐藏的 low/medium。
- pendingCount = low + medium + high；展示不扣减 pendingCounts，mark handled 和 low 自动过期才减少。
- hiddenCount 指因展示次数政策隐藏的 pending；hasMore 指仍具展示资格但本次面板未放下的事件，二者不可混用。
- 面板计数/条目应从同一事务快照生成，避免并发 mark/过期导致同一响应自相矛盾。
- event.list 返回按同一默认未处理口径查询的事件；过滤后的列表计数不能冒充全收件箱统计。
