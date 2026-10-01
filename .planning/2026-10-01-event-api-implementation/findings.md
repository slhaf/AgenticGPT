# 实现发现与合同

## 初始事实
- 初始git status --short和diff --stat均无输出。
- 存在.codegraph；已优先CodeGraph定位Config、AppState/build_app_state、finalize_process、skill install save_cache、配置TUI组件。
- 已读开发指南和文档标准，所有者沿既有Agent→protocol、Hub→protocol方向。
- process现有history与hook不原子；skill install独立JSON保存。原响应仲裁与崩溃恢复需新增持久协调，不把stdio日志tracker等同持久通知。
- TUI范围经用户二次澄清，仅配置通知等级覆写。

## 未决实现细节
- 核心store/响应判定/恢复接口与跨入口对齐方式，须复用真实调用路径。
- 配置TUI当前enum选择/分组/写入锁与config key registry，待核对。
- Hub原业务response/error/cache路径需共同处理事件骨架，不越过目标Agent授权。

## 阶段一映射结果
- LSP status：No language servers configured；使用CodeGraph调用路径与精确源码，不能运行不存在的LSP引用查询。
- 配置TUI只有init/import向导，无独立编辑器。可沿OptionalDrafts/Review Choice/既有choice组件增加覆写，最终复用配置写锁和备份。无需运行时事件UI。
- Unix现有local call --arguments-file -已有受限stdin→Unix MCP方式，可复用基础读取/传输，不引入新协议。
- Hub原响应data是业务Value，可保留形状加events；Room目标需捕获原lease，不跨Agent合并。
- Console无现有事件SDK/解码消费者，不因此新增UI。
- 已定义内部store/DTO/响应仲裁的候选共享接口 local://event-implementation-interfaces.md；未实现，等待下述合同边界确认。

## 需用户确认的公开合同歧义（按目标停止条件暂停实现）
- CodeGraph直接核对hub_info（Hub MCP :376、HTTP :187）与agent.list无单一目标Agent。
- process_list MCP :587-592，以及get_process_status HTTP :368-388，会在Agent离线/超时后返回Hub缓存业务结果；当前Hub无权威事件副本。
- “都覆盖”尚未定义无目标/多目标及离线时的events呈现；不能返回假零计数，也不能未经确认增加陈旧快照字段/新的全局收件箱。
- 询问无目标返回是否省略events，以及离线缓存结果是否省略events或改用明确标注陈旧的快照。

## 工具错误
- `sh skill://.../resolve-plan-dir.sh`未自动解析URI，exit127；已报告工具问题。随后coreutils realpath得到安装实际路径，使用该路径+PLAN_ID/PWF_PLAN_ROOT成功解析到本任务目录。
- 未运行构建、测试、格式化；验收修复轮次仍0/3。

## 合同歧义已清除
- 用户明确选择：Hub无目标Agent不附events；Agent离线/超时缓存或错误返回不附events。
- 因而不需要Hub镜像事件store或陈旧面板格式，不改现有离线业务语义。
- 共享接口与文件所有权已映射为核心DTO/store、配置TUI、Agent入口、Producer和Hub入口五个真正独立编辑切片。

## 验收依赖与文档定位
- 系统python3缺jsonschema；已有target/contract-venv/bin/python已实测可import yaml/jsonschema，无需安装新依赖。最终运行目标命令时使用该venv的PATH。
- find文档语义检索因judge服务账户拒绝全失败；改用已知文档的标题/字面grep和精确范围，不把无hits当不存在。
- 文档更新入口：interfaces.md端点列表及Apps MCP封套、tool-contract-matrix.md表面矩阵、configuration两份Optional/Toplevel段、standalone-runtime工具列表及存储权威、operations实时reload段。
- 文档正式更新安排在真实实现冒烟证据之后，不把未验证行为提前写成承诺。

## 共享装配和高风险仲裁
- CodeGraph核对main.rs：Agent是平铺#[path]module，统一crate::event_store与crate::event_notifications；已纠正Core提出的crate::storage路径，不新增第二模块约定。
- wire扩展只在既有HubCommand，Core仅提供DTO，禁止额外EventCommand enum。
- Producer与入口共享settle_initial_response(state, operation, value)及recover_event_notifications；初始资格sticky，get/status/list不能改写。
- 高风险独立意见：EventDurabilityOpinion审阅分离持久层间crash窗口、原终态state与completedInline/大小预算判据；不扩大成新总线或主动推送。

## 独立高风险审阅与最终入口选择
- 读EventDurabilityOpinion全报告，直接源码核对Hub dispatch :43-81及:189-196：timeout移除waiter，迟到结果仍持久化但不交给原调用方。
- 用户明确选择以最终入口返回为准，因此废弃仅Agent生成value/receipt就suppress的Hub方案，新增可靠disposition反馈。
- EventHubFeedback拥有新Hub小型决策/outbox模块；HubIngress拥有其dispatch/db/root/lifecycle接线及wire变更。
- Core拥有Origin/Disposition/SettleDTO与bind/remote-settle；Producer新增ProcessHistory同事务completion outbox及Install私有pending marker；Agent入口拥有transportledger必要只读/关联接口。
- terminal state与终态描述已在原响应告知便可suppress；completedInline=false仅因预算/TooLarge不等于异步。安装deduplicated重放不能抢先判定原调用。
- 7天仅清事件history正文，source/origin/已通知等小tombstone不能随之删除导致旧receipt复活。

## goal运行时阻塞证据
- goal(drop)返回status=dropped后，工具从runtime消失；xd://goal read/create皆No such tool，eval tool.goal(get)报Unknown tool from js runtime。
- 已向xd://report_issue报告；在已知CLI/RPC/SDK文档检索goal恢复路由无匹配。
- 全部7个实现owner确认HOLD：部分源已写但尚未集成/编译；详细文件与剩余项在pause_handoff.md。没有验收成功声明。
- 最新objective.md保留用户全部规则，加入最终入口timeout/late-result判定与Hub恢复验收；3轮计数仍0。

## 恢复证据
- 用户再次guided-goal载入objective.md；主线程goal(create)返回Status active。
- 本轮未缺任何访谈字段：命令、真实入口/TUI验证、3轮、范围与停止条件均已逐项确认；用户明确不重新访谈。
- 停点已读，24个tracked文件暂停diff +1457/-59；未追认这些改动完成或编译可用。
- 共享接口现保存在本任务interfaces.md/response_feedback.md，最新反馈合同覆盖旧Agent-only判据。
