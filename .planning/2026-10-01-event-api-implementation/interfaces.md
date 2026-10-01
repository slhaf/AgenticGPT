# 事件实现共享接口（集成所有者：主线程）

先读 .planning/2026-10-01-event-api-implementation/task_plan.md 完整用户合同。不得改为旧 eventPanel/result wrapper；不得改摘要32字或TUI范围。

## Protocol（核心任务拥有新 events.rs 和 lib.rs 导出）
- EventSeverity: Low/Medium/High，serde lowercase，as_str()。
- EventStatus: Pending/Handled/Expired，serde lowercase，as_str()。
- EventSourceKind: Process/SkillInstall/External，serde snake_case，as_str()。
- EventSource { kind: EventSourceKind, #[serde(rename="ref")] reference: String }。
- EventRecord { event_id, message, severity, created_at: DateTime<Utc>, status, source, shown_count:u32, expires_at:Option<DateTime<Utc>> }，camelCase。
- EventListItem { event_id, summary, severity, created_at, status }，camelCase；summary截断为32Unicode字符含省略号。
- EventListRequest { #[serde(default)] agent_id:String, status:Option<EventStatus>, severity:Option<EventSeverity>, limit:Option<usize>, cursor:Option<String> }。
- EventGetRequest { #[serde(default)] agent_id:String, event_id:String }。
- EventMarkRequest { #[serde(default)] agent_id:String, event_ids:Vec<String> }。
- EventListResponse { items:Vec<EventListItem>, next_cursor:Option<String> }；EventMarkResponse { handled_ids:Vec<String>, not_found_ids:Vec<String> }。
- EventPanel { current:String, new:Vec<BTreeMap<String,String>> }；Default是计数全零+空new；panel中key="eventId | 摘要", value="severity | RFC3339日期"。
- EventInjectRequest（只给受权本地写入入口，不注册模型创建工具）{ message:String, severity:Option<EventSeverity>, #[serde(rename="ref")] reference:String }，deny_unknown_fields；来源kind固定external、createdAt服务端生成。
- HubCommand沿现有模式：EventList/EventGet/EventMark { request_id:String, payload:对应请求DTO }；内部EventPanel { request_id:String }（无agent_id字段，目标由request_agent绑定）。Hub集成任务拥有envelopes及request_id() exhaustive match。

## EventStore（核心任务拥有storage/event_store.rs；Agent集成任务拥有装配/所有者/生产者）
- EventStore::open(&PrivateStatePaths) -> anyhow::Result<Arc<EventStore>>，独立私有events.sqlite3，串行事务；失败不伪造空队列。
- list(&EventListRequest)->Result<EventListResponse>, get(&str)->Result<EventRecord>, mark(&[String])->Result<EventMarkResponse>。
- panel()->Result<EventPanel>：同事务过期判定、清理、全pending计数、最多5条排序选取、曝光计次。所有查询按真实时间判定low到期。get/list不把正文计入曝光。
- inject(&EventInjectRequest, low_ttl_seconds:u64)->Result<EventRecord>：固定external，默认severity low，完整内容有资源上限，不允许提供内部source。
- InternalEventPolicy { low_ttl_seconds:u64, overrides:BTreeMap<String,Option<EventSeverity>> }，serde；map value None代表off，未配置则Low。
- register_internal(source:&EventSource, policy:&InternalEventPolicy)->Result<()>：实体产生副作用前持久创建响应仲裁记录，幂等，初始AwaitingResponse；不得覆盖旧实体的已判定状态或已通知终态。
- record_internal_completion(source:&EventSource, event_type:&str, message:&str, at:DateTime<Utc>)->Result<()>：持久终态摘要/类型，去重；若响应尚未判定暂存，若AsyncEligible则按snapshot policy插入事件，若Suppressed/off则不创建。
- settle_response(source:&EventSource, includes_terminal:bool)->Result<()>：在真实原响应handoff之前持久判定；true Suppressed（不创建），false AsyncEligible（已有completion立即创建）。不能用是否spawn或finish先后来决定。
- pending_internal_sources()->Result<Vec<EventSource>>：用于启动恢复所有AwaitingResponse（含已有completion）以及AsyncEligible尚缺completion的记录。Awaiting在恢复时可settle false（未持久响应判定，未已返回终态）。仲裁记录必须避免重放已通知/已处理事件；保留必要去重元数据，但不无限留无用正文。
- 来源创建时固定，事件API不可改变来源或操作实体。TTL创建事件时确定；处理/过期历史7天清理，不自动清理pending medium/high。
- 需要有永久测试证明隐藏仍计数、32Unicode边界、mark幂等/unknown、TTL边界、同级排序/固定cap、分页、并发低次数、重开恢复、完成早/晚于响应仲裁/抑制/off、source不可伪造。无mock echo/source-text/wiring测试。

## EventsConfig（配置任务拥有config及配置TUI；Store只使用独立policy）
- config.events: EventsConfig { low_ttl_seconds:u64(default86400), internal_overrides:BTreeMap<String,EventNotificationLevel> }，camelCase default/deny_unknown_fields。
- EventNotificationLevel Low/Medium/High/Off，serde lowercase；fn severity(self)->Option<EventSeverity>。
- EventsConfig::internal_policy()->crate::event_store::InternalEventPolicy，用来在操作准入时snapshot；既有config构造/validation/reload/safe summary/CLI registry/template同步。
- 稳定配置事件名process.completed/failed/rejected/cancelled/timed_out/detached/unknown_after_restart/skipped；skill_install.completed/failed/cancelled。按生产者实际可达终态命名。默认全low；未知事件名须给明确验证错误或沿现有map约定（先报告，不默默忽略无效配置）。
- 配置TUI只是选择这些覆写值，并可恢复未覆写默认；不是运行时事件管理。复用既有选择/保存锁/备份流程。

## 文件所有权
- 核心：protocol/events.rs, protocol/lib.rs导出, Agent/storage/event_store.rs。
- 配置：Agent/config/**, Agent/ui/config_tui/** 与必要共用配置表单/CLI i18n；runtime/startup.rs的reload字段改动请先消息给Agent集成owner。
- Agent入口：Agent/main.rs、runtime/state/startup、operations/operation/local_service、ingress/stdio_server/schema/hub/local_control、ui/cli；拥有通用AppState构造与测试构造，但排除Producer拥有的process/managed.rs与skills/skill_installs.rs内部测试构造。startup须调用Producer恢复函数并更新events live config子集。
- Producer：process/managed.rs、skills/skill_installs.rs、必要mcp/batch.rs，以及新operations/event_notifications.rs；注册/终态/恢复/响应仲裁helper由其提供，不改Agent入口所有者文件。新module根装配由Agent入口owner处理。producer自身测试AppState构造同步event_store。
- Hub集成：protocol/envelopes.rs、Hub源码、openapi、必要Kotlin合同消费者。Agent HubCommand映射由Agent集成owner消费。
- 契约live脚本：scripts/check_contract_parity.py由后续专人拥有。
- 文档/共享planning/提交/所有检查：主线程拥有；任务禁止build/lint/tests/formatters mid-flight，也不要提交。

## 最新用户确认的Hub例外（优先于任何旧描述）
- hub.info/agent.list/user.notify等无单一目标Agent的调用不附events，绝不伪造空面板或跨Agent合并。
- Agent离线或请求超时的Hub错误/cache回退不附events，保留原业务行为；不新增Hub陈旧事件缓存。
- 在线有明确目标Agent的转发响应由Agent一次产生events，Hub原样保留；在线native cache-only目标工具可单独请求EventPanel一次。若该请求失败，按上述例外省略events。
- 多Agent聚合不附顶层events；成功的每个已授权单Agent子响应可以保留其自身events，不广播独立事件查询。
- 原响应仲裁只能在初始创建调用中settle一次；后续status/get/list读取不能重写已判定资格。内部完成早到与晚到均按原响应是否携带终态结果协调。
- 禁止任务中途build/lint/test/formatter；主线程统一跑一轮完整验收，最多3轮失败修复。
