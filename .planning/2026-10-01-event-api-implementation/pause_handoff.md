# 外部工具阻塞暂停交接

所有7个实现owner已收到并确认HOLD，停止代码编辑、build/lint/tests/formatters/提交/回滚。主线程更新目标时goal(drop)移除了goal工具；xd://goal create/get/read及eval tool.goal均Unknown tool。已报告工具问题；CLI/RPC/SDK已知文档未找到可用恢复路由。需宿主恢复goal工具，用objective.md重建active目标后继续，不重做访谈。完整验收修复计数0/3。

## 已确定合同补充
- 无目标Hub及Agent离线/超时错误或cache回退不附events。
- 原响应以最终入口返回为准：Hubtimeout后迟到terminal仍通知，需要持久判定+可靠EventSettle反馈。
- 新共享接口local://event-response-feedback.md覆盖旧Agent-onlysettle。

## 当前实现未完成、未编译、未验收
- EventCore：新增protocol/events.rs、lib.rs导出、Agent/storage/event_store.rs，涵盖基础API/panel/SQLite仲裁、origin能力和行为测试；origin_from_fields/validate_origin尚未添加，不能认为可编译。
- EventConfig：已改config.rs/config_keys/templates/setup(model,validation)/config_tui pages/config.example，新增typed Events配置和向导表单。config_keys的AutoOrNonNegativeInteger variant被误删待恢复；Review/import完整路径、CLI行为测试与真实TUI键序未完成。
- EventAgentIngress：已改main/root平铺装配、AppState/startup store/recover/reload以及非Producer测试构造；实际事件schema/dispatch/admission/特殊结果面板/Unix专用inject/Hub origin metadata与feedback/replay尚未接入；未改transport_ledger。
- EventProducer：仅新operations/event_notifications.rs约17KB，仍是旧sidecar/Agent-only判据版本，需重写为initial_response_dispositions、local settle、remoteOrigin-aware recovery。managed/install/history/outbox/SkillInstallJobRecord pending marker等尚未改，真实producer调用和测试尚未完成。
- EventHubIngress：已改protocol/envelopes前三API+panel命令及request_id，Hub runs命令类型、MCP args/server/transport；尚无EventSettle/Response.event_sources、HTTP routes/Router/OpenAPI、DB/dispatch/lifecycle feedback接线和完整行为测试。
- EventHubFeedback：尚未创建文件，仅调研拟定API init/prepare/record_reply_metadata/finalize_original/pending_for_agent/ack/flush_for_agent，待与HubIngress确认。
- EventParity：仅scripts/check_contract_parity.py部分辅助/fixtures，包括bounded stdin privateevent.inject调用、raw stdio transport、event panel/schema/config helper和Standalone fixture；所有event live behavior、Hub/隔离/重启/timeout-late反馈尚未完成。当前Hub exec等待固定35s；live晚到验证需真实wire假Agent延迟可靠Response，不以mock echo当永久行为测试。

## 恢复顺序
1. 恢复goal工具并以objective.md创建active目标；用户授权、范围及3轮上限不变。
2. 向所有原owner或续接任务发RESUME，先修已知不完整符号/误删enum，继续最新共享接口。
3. 完成所有真实路径后统一fmt/test/build/live parity及配置TUI，最多3轮失败修复。
4. 有实测后同步正式文档；按阶段提交，不将此暂停状态宣布为完成。

当前代码改动只在各owner授权文件，无验收成功声明。planning记录可提交，未完成source不做完成功能提交。
