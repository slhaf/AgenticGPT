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

## 恢复后的跨层不变量核对
- dispatch原实现先prepare run、再send、最后waiter timeout；Agent原实现dispatch完成后才ledger mark_completed。新origin必须在producer准入、副作用之前绑定，而非最终值解析时首次绑定。
- 已通知AgentIngress/Producer协调RequestContext传origin，HubIngress/Feedback在发送前持久prepare。反馈命令不参与原调用仲裁、不消耗面板。
- parity要求真实Agent配合loopback WebSocket延迟relay证明Hubtimeout/迟到结果，不能仅fake Agent回声替代生产证明。

## Agent归属边界
- CodeGraph核对PrivateStatePaths按Agent key分目录；raw安全ID与id-<hash>命名空间可别名，现有Config未限制到可证明无别名的子集。
- 仅新增trusted Agent身份到prepare路径并由EventStore私有owner metadata拒绝不同Agent混用；不迁移/重设计既有私有状态目录。
- AgentIngress拥有此prepare边界及通用fixture，Core拥有DB校验，Producer更新自身fixture。此为事件隔离要求，不扩展其他存储修复。

## 核心独立审阅发现
- CoreReview发现：migration锁外读取user_version的双连接竞态；Duration::seconds超大TTL panic；remote测试局部policy遮蔽helper的编译错误。已转交Core/Config修正，未跑验收，计数0/3。
- Context7三次查询取得Chrono原始API事实：seconds超出±i64::MAX/1000会panic，try_seconds返回None；checked_add_signed在日期超界返回None。来源：https://docs.rs/chrono/latest/chrono/struct.TimeDelta.html 及 https://docs.rs/chrono/latest/chrono/struct.NaiveDateTime.html；原始artifact 95/93。
- Hub lifecycle同步await可靠消息处理；feedback网络等待须脱离该reader路径并禁止持DB/agents锁，已通知两Hub owner。
- reviewer向agent://发消息被readonly拒绝已报告工具问题；主线程转发发现，未因此丢失修正。

## 最终投影核对
- HTTP process.exec当前成功路径直接Json(value)，Room skills.install/run将request_active_room结果交给result_from_value；继续核对公共封套是否丢终态。
- Hub owners协调finalize内部API从bool变Option dispositions以支持最终投影判据，非公开API变化；None=NoTerminal、Some=Returned且按source验证，决策与raw metadata均不可被晚结果改写。
- 创建投影实际保留：HTTP process.exec与mcp.callTool/batch直接Json(value)，Room result_from_value调用AgenticResult保留完整structuredContent及text（含error）。已告知HubIngress：这些入口成功waiter判定Returned有效，无须额外投影框架；detail/cache错误投影仍要保留events或按离线例外省略。

## 活进程持久失败与公共调用屏障
- 独立EventDurabilityOpinion确认：原调用owner明确error/drop登记pending NoTerminal，已持久Awaiting支撑crash恢复；不能按waiter暂时缺席推断。需要保留小source dispositions以重试metadata，不保存业务输出正文。
- Returned必须确认实际持久decision=Returned才允许返回原终态；已有NoTerminal时通用Ok不能证明成功。普通调用须等实际flush完成，原coalescer忙时立即return不是屏障。
- 采用既有Dispatch持有每Hub实例Coordinator，取代static DB-key coalescer，避免测试/实例状态混淆；module和既有文件owner明确分工。
- Receipt retention源码prune_expired只清result/arguments/process正文，保留identity/status/hash tombstone，不删除agent_runs origin；无须扩展无关retention修改。

## 最终source集成核对
- Hub post-barrier validate释放agents guard后，prepare及pending.lock await仍在捕获sender发送之前；替换连接能在该间隙发生。要求保持原捕获Agent/Room lease，并在既有agents线性化guard内验证generation/role后同步send，不重选Room或持锁等网络。
- cargo fmt与workspace test/build首次均被protocol EventMark缺闭合delimiter阻止；feedback同类缺brace已交给owner。尚未有构建二进制，live parity/TUI不能用旧binary冒充本次实现证据。

## 最终反馈独立审阅
- 四项具体发现已转原owner：run与Awaiting intent分事务允许reconnect orphan repair误判live调用；run-result SQL失败过早丢弃source metadata；部分batch source完成后恢复合法subset被拒绝且会阻塞SSE有序writer；flush→request_agent→request_target→flush形成未boxed async递归。
- 采用同SQLite事务创建run+Awaiting、保留已认证source metadata供普通调用重试、只在identity-only恢复接受已知subset且保留完整set和原终态flags；feedbackdispatch最小box打破类型递归。
- all-targets编译诊断其余问题按文件owner修正：serde imports、Hub imports/ProcessCancel arm、outbox RFC3339 parser、两个Vec origin类型、skills test第五参数、HTTP测试使用pending_count。
- OpenAPI独立静态prepare命令已exit0：本地refs与Draft202012/format合同断言通过。这不是live parity，也不是TUI验证。
- Room lease额外核对：生产release/claim调用位于lifecycle角色更新、disconnect或replace；既有agents guard保护这些路径。捕获Room target的generation、mode、role同步验证及send足以拒绝上述变更，无需重选activeRoom或扩展路由模型。

## Room既有消费者边界
- preflight 130项Hub测试中仅room_dispatch_keeps_validated_generation_during_replacement失败：原request赢得agents→active_room lease验证后，旧generation应先收到command再retire，匹配stale reliable Response仍可完成，替换generation不得收到重路由命令。
- 独立RoomLeaseOpinion确认这不是仅poll数量的incidental断言。无反馈时不应因新agents重获取改业务顺序；真实反馈等待时尚未业务准入，释放agents等待、之后只重验捕获target，失效用既有RoomStateConflict/409而非重选Room。
- 采用既有per-Agent barrier的同步ready预检，try-lock排除busy flush并persist/check durable+live队列；ready时保留原agents guard直到同事务run/intent、waiter注册与同步enqueue，无网络await持agents。新增busy/真实等待边界测试且保留旧race测试。
- all-targets check与Agent静态/GIF批次消费者修后测试已exit0；Hub129项通过。完整第三轮尚未开始，当前失败计数2/3。

## 停止时已知未解决项
- 最终live parity实际失败于Agent事件多传输fixture启动：stderr tunnel_config_required；导致后续事件跨入口/Hubtimeout/恢复/delta场景未执行。已有TUI/Unix实际source smoke不替代上述完整验收。
- TargetlessExposureReview只读静态确认：Hub dispatch.rs:mcp_list_servers_all_agents (:539–580)发普通McpListServers，之后只取servers并丢掉events；Agent ingress/hub.rs (:1263–1267)正常取panel，event_store.rs (:380–414)已给选中条目增加shown_count。无单一目标的聚合调用因此消耗low一次/medium三次提醒而不公开展示，需显式内部panel-suppression标识且保留targeted发现正常行为。该消费者问题尚未独立runtime重现/修复；不通过Hub删字段补救。
- workspace tests 758 passed/1 ignored及Agent/Hub build通过，不证明live parity或上述未覆盖路径。失败3/3上限已到，停止，不以tests成功替代完成标准。

## 追加授权后停止：新增完整失败3/3
- 原启动及targetless吞曝光问题已修并获得实际Agent/Hub入口证据。Hub无新增Unix入口；事件fixture用真实process终态生产者。Crash反馈检查按私有settle身份匹配completed Response，公开事件正文/创建时间保留已实测。
- 最终第三轮strict Rust门禁无warning，但workspace test失败于file_edit_apply_patch_revalidates_external_change_before_commit（stdio_server_tests.rs:3418），error实际Null而非file_revision_conflict。此次未继续诊断或重跑，原因尚未确定。
- 本轮build/live parity被&&阻断；整体仍未完成，历史3/3及新增3/3均保留。此前定点恢复证据不替代整链成功。

## 用户授权的file.edit调查：修复前初步证据
- 修复前测试hook为cfg(test)进程全局Mutex<Option<(PathBuf,Vec<u8>)>>；inject_external_change直接替换唯一槽位（当时file_ops.rs:64–78）。take_external_change_for只按当前patch路径匹配（当时:110–127），不防另一个测试覆盖尚未消费的注入。
- 原失败测试在stdio_server_tests.rs:3411注入race.txt后await dispatch；另一测试file_edit_add_creates_nested_parents_after_whole_patch_preflight在:3178也注入external/nested/target.txt。原始artifact://276显示后者通过、前者失败；日志未记录具体线程交错，尚不能声称捕获原失败调度。
- 真实生产revalidation在file_ops.rs:1841–1846重新读取并比较revision；hook只在cfg(test)的:1833–1836写外部正文。调查将隔离执行实际hook代码，区分测试注入丢失与生产校验缺陷，不运行完整suite。

## file.edit调查结论与实际诊断
- 已用源码原样的inject_external_change/take_external_change_for函数编译临时Rust probe（只提供它们读取的source/target/path输入字段），确定执行A注入→B注入→A消费→B消费。实际输出：单独A可消费；交错后A不可消费、B可消费、A再次消费仍为空，exit0。这是hook覆盖机制的可执行证明，不是原完整测试线程调度的重放，也不是生产revalidation的验证。
- 另用现有已构建Agent在一次性HOME/config/workspace启动真实HTTP MCP，无注入地执行同一before→agent patch。实际响应{changed:1,changes:[{action:"updated",path:"race.txt"}],events:{current:"low: 0 | medium: 0 | high: 0",new:[]},status:"completed"}，文件agent换行，exit0。此处仅证明成功响应没有error；生产binary不含cfg(test)hook，不能用此smoke替代失败单测。
- 因果解释：A注入被B覆盖→A未发生外部改写→A revision不变→正常commit→error.code缺失，被JSON索引读为Null。原日志同进程B通过/A失败与此吻合；原调度无记录，因此将其标作最有证据支持的原因，而非已捕获原交错。
- 调查阶段确认影响范围为修复前cfg(test)共享单槽；两个调用者均受影响。未发现要求修改生产错误投影或事件面板的证据。当时建议仅按绝对路径隔离测试注入、消费匹配项，保留两个行为断言；调查阶段未实施修复，随后已按新授权实施，见下节。

## 测试注入修复与真实dispatch回归
- 新增file_edit_external_changes_remain_isolated_by_path：两个独立AgentMcpServer workspace使用同名race.txt，先同时注册两个不同绝对路径，再逐个真实dispatch更新；每项必须返回file_revision_conflict且保留各自不同的external正文。修前该回归实际失败Null != file_revision_conflict；不是仅hook helper探针。
- cfg(test)外部改写注入改为Mutex<Vec<(PathBuf,Vec<u8>)>>，按路径更新/新增待注入项，消费只移除当前patch匹配项；不再覆盖其他路径。同路径仍保留最后一次注册内容，原生产校验和两个旧回归断言不变。
- 修后8项file_edit_回归以8线程运行全部通过，包括新确定性隔离回归和原失败/相邻add回归；fmt --check与Agent all-targets strict Clippy均exit0、无warning（artifact://284）。内部测试修复不改变用户文档/API合同；未执行全workspace/live完整验收，计数保持历史3/3及新增3/3。

## 再次授权完整验收的未解决项
- 修复测试注入后workspace实际761 passed/1 ignored，strict Clippy无warning；先前file.edit失败未在本轮出现。
- 全live parity现到达Hub inline process.exec event suppression，current断言预期全零却实际low1/medium1（artifact://291）。这是inbox计数差异证据，单凭错误不能断言inline资格仲裁缺陷，亦不能直接认定fixture残留。
- 此前真实Hub聚合/共享API、晚到结果/重启、delta检查点和crash恢复在同一完整运行中均已通过，但后续parity未完成。按本次一次验收授权记录失败并停止，原目标仍未完成。

## 五轮授权后的集成范围核对
- 当前待集成64个已修改路径均位于既有计划目录、三核心Rust crate、config.example.json、OpenAPI、parity脚本及已指派README/docs；不包含apply-patch/browser-host/Console/发布部署文件。
- 四个新事件模块为Agent storage/event_store.rs、operations/event_notifications.rs、protocol/events.rs、Hub storage/event_feedback.rs。待完整验收成功后分生产集成与文档证据两阶段提交，不提交临时状态或构建产物。
- 已核对main的GateError处理：失败时打印累计reports并返回1，后续runtime代码不会继续执行；不能由FAIL后出现PASS文本推断完整parity已完成。

## inline抑制问题已定位并定点验证
- run_hub_event_gate为曝光合同生产的medium失败进程事件与low完成进程事件，targeted最后一次展示后仍pending；隐藏/曝光不是handled。原函数返回时未mark，后续inline全零断言读到恰好这两条seed，属于fixture状态泄漏。
- 修复仅script：核对实际seed ID/source/process ref/severity/message，公开event.mark只处理这两个已知ID，核对handled记录和空pending/全零；原inline响应及后续pending全零断言不变。事件生产测试覆写的原config在inline检查后真实reload恢复，避免影响后续gate。
- 主线程隔离真实Hub+normal/Room Agent定点exit0：seed公开mark后为空、printf原响应terminal并全零、后续event.list为空且全零、原config实际reload恢复。未改生产事件仲裁，也未通过删除断言规避。完整新验收计数仍0/5。

## Room错误schema修复的来源边界
- 原在线Room invalid-path400实际返回Agent error+events，但共享ErrorResponse只准error。最终方案保留generic ErrorResponse no-events，新增strict RoomAgentErrorResponse（strict ErrorDetail、可选EventPanel），只用于九个Room路径的400及有明确Agent producer的notebook.read/state.read404；其他404、全部409、401/422/504不扩展。
- Agent room_notebook_not_found/room_state_entity_not_found在缺失Room根时映射404；普通missing-file是room_notebook_read_failed默认400。Hub room_not_active404/room_state_conflict409仍不返回events；兼容同状态Hub无panel响应不能推断它是Agent来源。
- 主线程static契约及实际Room HTTP烟测exit0：invalid-path400与missing-file400保留error/零panel并验证strict schema；Room下线后的room_not_active404无events且schema合法。未改runtime，不削弱schema或去掉面板；完整新增失败保持1/5。
