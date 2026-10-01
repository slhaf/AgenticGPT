# 最终入口原响应仲裁（用户新增确认；覆盖旧Agent-only/receipt-first suppress方案）

用户明确选择：原响应是否包含终态，以最终入口返回为准；Hub返回超时/未返终态，迟到Agent terminal仍产生event。不承诺客户端真实阅读确认。Agent本地stdio/Unix/HTTP由其自身结果出口settle；Hub创建操作不能自行按Agent生成value suppress。

## Wire DTO（EventCore拥有protocol/events.rs）
- EventOrigin { run_id:String, request_id:String, command_hash:String }，camelCase。
- EventResponseDisposition { source:EventSource, includes_terminal:bool }，camelCase。
- EventSettleRequest { origin:EventOrigin, dispositions:Vec<EventResponseDisposition> }，camelCase。
- AgentMessage::Response追加 `#[serde(default, skip_serializing_if="Vec::is_empty")] event_sources:Vec<EventResponseDisposition>`，只用于内部wire，HTTP/MCP业务JSON不新增此字段。非创建操作vec为空；创建响应逐新实体描述其最终原value是否terminal，跳过deduplicated install重试。
- HubCommand追加隐藏内部EventSettle { request_id:String, payload:EventSettleRequest }；不注册公开模型工具/HTTP端点，不走自动event panel生成，避免内部metadata反馈消耗展示次数。
- request_id() match、所有Rust Response构造/匹配以及wire合同消费者同步，Agent入站handler校验origin身份/来源关联。

## EventStore追加能力（EventCore）
- bind_origin(source:&EventSource, origin:&EventOrigin)->Result<()>：在初始Hub创建响应持久handoff前绑定origin；不改source，不改旧资格，不允许绑定冲突。
- remote_origin(source:&EventSource)->Result<Option<EventOrigin>>。
- settle_remote_response(source:&EventSource, origin:&EventOrigin, includes_terminal:bool)->Result<()>：必须验证同源origin匹配，再做一次性Awaiting→Suppressed/AsyncEligible；重复同判定幂等、冲突拒绝。
- 核心pending_internal_sources仍含远程Awaiting，Producer恢复不能把有remote_origin的Awaiting盲目settle(false)，只能补owner终态snapshot并等Hub反馈。无origin的local Awaiting恢复可按无已提交原响应处理。
- 小tombstone保留source/资格/origin/emitted防旧receipt复活，7天只清事件历史正文。

## Producer：原value观察与原始创建身份
- 提供 `initial_response_dispositions(operation:&str,value:&Value)->Result<Vec<EventResponseDisposition>>`纯解析真实creation结果（只有新创建实体、batch逐项，deduplicated install不参与）。includes_terminal判断实体terminal state/终态说明，不看completedInline/完整stdout/result预算；后续get/status/list不能参与。
- 本地settle_initial_response可调用此纯helper再settle。
- Agent Hub初始dispatch后先用helper取sources，bind_origin，再持久mark_completed原业务value，回包event_sources；不做本地settle。原创建完成receipt replay从原value重建同source metadata，但仍等待Hub最终判定；原响应标识必须来自该receipt command，不通过后续查询捏造。
- recovery先owner自身恢复/终态补录；local与remote Awaiting分开，不扫描所有旧历史注册新事件。

## Hub：最终返回决策/outbox（EventHubFeedback新模块owner，HubIngress接线owner）
- 实际Hub dispatch等待原调用值与超时二选一；成功返回值保留其实体终态字段，最终JSON投影如果对creation丢了终态必须据真实返回修正。
- 原创建run分配后，副作用发送前持久response-decision intent。Hub收到有效Agent Response时保存wire event_sources，但不凭收到结果擅自决定已向原调用返回。
- request_target await赢得有效原值：持久决定Returned（保持每实体includes_terminal）；超时/失败则持久NoTerminal（所有source includes_terminal=false）。最终判定只写一次，晚到结果不能改NoTerminal。
- 若超时先于metadata到达，保留NoTerminal；晚到Response取该判定+sources排队EventSettle。反馈用既有可靠HubCommand/receipt机制和稳定反馈requestId，拒绝跨Agent/run/request/hash。反馈metadata自身不触发panel/新response-decision。
- 通过Hub私有SQLite新增小型outbox/决策表，无用户数据破坏；初始化/restart把旧Awaiting原请求视为NoTerminal（旧客户端响应不再存在），等sources或已到sources后反馈。
- pending反馈在Agent连接、收到metadata、原响应决策、后续适当可靠请求时驱动flush；失败保留，已ack可收缩正文但保留必要去重身份。不要持DB锁await网络；不要新建用户主动推送。
- 新模块API由EventHubFeedback与EventHubIngress直接message精确对齐；新module/root/sql-init/dispatch/lifecycle接线只由HubIngress编辑。

## Owner终态补录证据（Producer新增文件所有权）
- Producer现在同时拥有storage/process_history.rs的事件小outbox扩展及protocol/skill_bootstrap.rs中私有SkillInstallJobRecord pending-marker。
- process终态历史写入事务同时写小completion outbox摘要；EventStore成功record后ack，业务history清理不得丢唯一pending证据。无需永久保存输出正文。
- install terminal JSON同一次rename保存event_completion_pending标记；EventStore成功record后清标记，prune跳过未ack终态record；旧记录默认false，不扫描它们生成历史提醒。
- 不能只依赖audit.take/hot cache/有限owner retention；写失败可重试并重启补录。用户说eventAPI独立于process，metadata关联不赋予eventAPI任务控制能力。

## 验收
- 原Agent response terminal但Hubwaiter timeout，晚到结果后下一次成功调用只出现一条事件；Hub正常返回terminal则不出现；原active返回后后续terminal一条；mixedbatch逐实体。
- Hubrestart在Awaiting/NoTerminal/Returned与收到sources/反馈ack不同窗口恢复正确；Agentrestart不把remote Awaiting当localfalse。
- 已handled/expired正文清理后origin/终态重放不复活，不重置TTL/计次。
- 内部EventSettle不会增加面板曝光；错误origin/另一个Agent/source不可抑制其他事件。
- 所有任务仍禁止中途build/test/lint/formatter；主线程统一验收，最多3轮失败修复。
