# 实现进度

- 目标已创建，用户授权实现；最多3轮完整验收失败修复。
- 阶段一进行中：读取技能/开发指南/文档规范，初始工作区干净，CodeGraph已定位核心所有者。
- 尚未修改实现代码，尚未运行构建/测试/格式化。

## 阶段一调查与阻塞
- 三个只读scout完成Agent/Hub/配置TUI映射，主线程CodeGraph核对Hub无目标及离线cache分支。
- 共享接口候选已记录；尚无实现代码改动。
- 公开响应合同歧义触发用户批准的停止条件：无目标Hub调用及Agent离线如何呈现events。
- 待用户确认后继续；完整验收修复轮次0/3。

## 恢复实现
- 两项Hub响应例外已由用户确认；解除合同阻塞。
- 核心共享接口已钉住，准备并行实现真实切片；所有任务跳过中途build/test/formatters，由主线程统一验收。

## 并行实现启动
- EventCore/EventConfig/EventProducer/EventAgentIngress/EventHubIngress/EventParity六个独立owner已启动。
- 已发送config reload和producer原响应仲裁helper精确接口协调消息。
- 系统python依赖检查失败，现有隔离venv依赖检查通过；此为前置环境检查，不计完整验收修复轮次。

## 最终入口仲裁扩展
- 按停止条件问询并获得用户选择“最终入口返回”；Hubtimeout/迟到结果成为显式验收场景。
- 共享新接口已记录、发给Core/Producer/Agent/Hub/Parity；追加EventHubFeedback独立实现Hub决策/outbox模块，其他owner仅做各自接线。
- 目标文本更新保留全部旧合同/边界/最多3轮，完整验收修复轮次仍0/3。

## 全部实现暂停
- goal更新过程中drop后工具不可用，重建/读取的多条可用工具路由均失败，宿主问题已报告。
- 已向所有owner广播HOLD；Core/Config/Producer/AgentIngress/HubIngress/HubFeedback/Parity全部确认停止编辑和检查。
- 保存objective.md（五段目标）及pause_handoff.md（精确未完成状态），只提交planning记录，source不做完成提交。
- 当前无active goal可用；等待宿主恢复工具后继续，不重做已确认访谈。验收修复0/3。

## 用户恢复目标
- goal(create)成功返回active，五项授权/验收/范围/3轮/停止条件此前已确认，本次不重复访谈。
- 读取本任务四份记录与pause handoff、解析固定PLAN_ID目录，初始paused diff已记录。
- 共享接口复制到interfaces.md及response_feedback.md，成为可提交依据；现有local文件保持辅助，不是唯一规范。
- 准备解除全部外部工具阻塞，并通知原owner RESUME；验收修复0/3。

## 恢复执行接线
- goal(get)确认active；七个原owner收到RESUME（EventConfig由parked恢复），沿原文件所有权继续。
- 所有暂停todo已解除，core任务active；缺失origin helpers、ConfigValueKind旧variant和producer旧草案优先补齐。
- 本次无build/test/formatter执行；完整验收修复仍0/3。

## 静态风险复核与实测准备
- 将remote origin绑定提前到实体副作用前写入共享合同；原响应helper只提取metadata，防崩溃窗口误变local仲裁。
- 启动EventCoreReview只读复核已知DTO/store并直达Core反馈，禁止中途检查或编辑。
- 隔离venv具yaml/jsonschema，无pexpect/websocket/websockets；实际TUI用stdlib pty，parity复用stdlib WebSocket支持。

## 核心切片待审
- Core报告DTO/store实现和15类行为测试已就绪，含并发曝光、origin sticky、身份拒绝、tombstone/重开；尚未执行任何检查，不视为验收成功。
- 独立CoreReview继续检查其稳定源；新增ConfigReview只读核对typed配置/导入/optional TUI保存边界，修正由原owner负责。
- safe summary不变已明确在共享合同。一次edit因最新snapshot未显示锚点被拒绝，重读精确范围后成功；无source影响。

## Core审阅修正完成
- Core报告三项独立发现已修正：锁内migration版本读取与双opener回归、fallible duration/checked date与越界回归、remote_policy解除helper遮蔽。
- 持久store和紧凑panel实现todo已完成；API真实入口仍在集成中，所有新测试仍未执行。
- CLI私有注入ack不取panel，防low曝光被注入者吞掉；AgentIngress/Parity已确认真实下一公开调用回归。

## 配置切片实现完成
- Config报告typed字段/12个CLI键/strict import/稀疏load与show、配置Optional及Review/save/outcome已完成；review缺brace修正、超大TTL不panic校验已同步。
- 新增CLI拒绝不写盘、inherit恢复、Review四等级及导入备份保留的行为测试；尚未运行。
- 保存了owner给出的source-derived PTY按键顺序供实际核对，不能当作真实TUI证据。live reload/生产和入口仍待集成，验收修复0/3。

## 生产与反馈切片待集成审阅
- Producer交付managed process/MCP/batch/install准入origin、ProcessHistory小outbox及install JSON marker；独立ProducerReview检查所有者终态/恢复/原响应解析。
- 要求补齐有意义eligibility边界测试（terminal预算裁剪、mixedbatch、dedup及remote Awaiting），不以单纯outbox/marker测试替代完整规则。
- HubFeedback模块已交付，FeedbackReview检查durable decision/outbox/可靠ack/reader调度。HubIngress报告实际装配/prepare/metadata/finalize/flush已接入，仍未验收。
- Parity已覆盖各入口基础事件行为；正在补真实Agent的stdlib WebSocket延迟relay与双服务重启receipt恢复，未执行脚本。

## 独立生产/反馈审阅修正中
- ProducerReview七项完整发现已取得并分配：三个语法/字段编译问题、管道未EOF时终态证据延迟、install live marker重试、history损坏阻塞eventAPI、local panel失败前错误suppress。
- FeedbackReview四项分配：run/intent崩溃窗口、Agent pre-reply crash缺source发现、Returned持久失败仍返成功、普通公开调用不重试反馈。
- 新内部event.sources仅恢复exact origin的source身份；与原reply终态flags分开，避免活跃重连抢先写false。不伪造业务Response、不生成panel/模型推送。
- HTTP/OpenAPI未完成切片完整转给EventHubHttp；路径与Parity已钉住/v1/events GET、/v1/events/{eventId} GET、/v1/events/mark POST，不缩scope。
- 上述均为静态审阅修正，未执行验收，计数仍0/3。

## 阶段二source checkpoint
- Core/Config/Agent公开API与external Unix CLI、Producer审阅修正source已交付；阶段二source完成，跨入口/实际验收继续阶段三，不宣称功能已验收。
- 独立installed-Chrono throwaway probe实际输出+33715年期限，现有jsonschema date-time checker拒绝；Core/Config已改为RFC3339 0..9999并保留1e11大TTL，拒绝1e12。probe已清理；这是依赖边界repro，不是workspace验收轮次。
- Root特殊Browser/file结果同逻辑panel将增加紧凑text fallback，同时保留原content块/_meta/isError；待AgentIngress补齐真实image路径回归。
- Room HTTP room_value_response完整Json(value)保留error及events，无需重复修改。HTTP process list/status实际在线forward Agent（非cache-only），已纠正之前推断，绝不再取第二个panel。
- Source提交延后至完整ABI/入口集成；本阶段仅planning checkpoint提交，所有命令/TUI完整验收仍0/3。

## 阶段三集成交接与首轮验收
- 所有source owner已交付；HTTP错误保留完整Agent envelope，特殊content去重使用借用文本而非复制图片。
- Hub反馈已改为Dispatch实例队列、实际per-Agent flush屏障、owner Drop与identity-only恢复；Parity已加入真实延迟relay、双服务恢复、持久终态后的crash及live reload。
- 首次cargo fmt准备运行发现Hub反馈和protocol新增块缺闭合delimiter，已分配原owner修正；workspace test正在运行。此为第一轮验收，未有成功结论。
- fmt/test/build首轮失败均为解析错误，live/TUI因尚无本次可用binary暂不可执行；未使用旧binary验证。
- 原owner已修syntax与generation-guarded同步send，并补捕获旧target/替换后的拒绝回归。Agent最终panel/ledger失败会通过当前连接交付identity-only EventSources；新增真实process+SQLite触发失败回归。
- 主线程进行fmt准备及all-targets compile诊断以收集剩余类型问题；完整验收失败计数1/3，不因修正重置。相关文档已定位，待真实smoke后改写。
- 最终反馈审阅四项P1修正source交付：同事务run+Awaiting、SQLresult失败保留来源、recovery已知subset与队列合并、boxed反馈调用；新增rollback/resultSQL触发和subset回归，仍未运行。
- 主线程发现partial identity先materialize后full reply扩展的新增source未见后续可靠反馈；已交工程独立意见确认可达性与最小durable delta修复，不允许改已发request/payload或仅断言旧payload不变。
- 第二次all-targets诊断仍exit101：原owner修import/arm时误删既有keeper，managed.rs产生大量级联missing type；另有SQLite尾表达式借用和test私有字段问题。已要求恢复keepers并只改目标行。
- compiler实际确认opaque async Send递归：仅Box::pin调用不能满足spawn Send；采用flush返回显式Send boxed future的窄边界（去除多余内层box）。这仍是首轮修复诊断，不是成功验收。

## Agent实际smoke与测试修复
- Agent单独build实际exit0；三项生产dead wrapper只供既有MCP/安装测试fixture使用，已加cfg(test)而非删除测试或保留生产shim。原Producer两次wake因宿主mapOptionsForApi:web-search失败，已报告工具问题，由主线程完成该三处。
- Agent单独test实际563项：511通过、51失败、1忽略。48项因test private root未创建；其余为serialized schema byte proxy、旧总content块数和bare/decorated整封套比较。test root改用既有secure ensure_private_dir；production路径保护不变。
- 删除compact_tool_schema_budgets_hold的断言：Normal total/input为32000/17000、Room为48000/24000；这些test-only序列化字节代理无外部/runtime cap。保留normal_and_room_tool_sets_follow_fixed_surface_contract、event_api_schemas_preserve_defaults_and_empty_mark_boundary、file_surface_schema_is_exact、browser_descriptors_and_annotations_are_frozen。图片测试保留静态/GIF字节和text、恰一image，移除非契约总块数；adapter比较改保留业务error。修后尚未重跑，不宣称suite通过。
- 隔离真实PTY执行config init（Local/Normal/en）：观察默认TTL86400、inherit(low)及low/medium/high/off；TTL改60，form选high，Review改medium，预览JSON、实际写盘exit0。config show/keys均exit0，保存events.lowTtlSeconds=60与process.completed=medium，其他类型inherit。
- 相同TUI保存配置的真实Agent Unix入口：sleep1/wait0原响应starting，后续status completed，event.list/get实际一条medium，source.kind=process/ref匹配真实processId、expiresAt=null。独立CLI stdin privateevent.inject返回shownCount0且无events；下一agent.info曝光low一次，event.get来源external/ref=tui-stdin-smoke、shownCount1、expiresAt-createdAt准确60秒。runtime smoke exit0，输出artifact对应bg12。
- eval复用parity脚本失败于宿主Python缺referencing，改用既有contract-venv解释器的throwaway smoke；没有重做已成功TUI操作。一次PTY双动作10秒超时后通过实际resize重新读取画面，未重复已发送按键。
- supplemental module正式转给EventDeltaDelivery唯一写owner；旧Feedback已STOPACK并交接其未完成materialize_deltas调用。方案经独立意见确认可达，[INFERENCE]并非已重现；保持主payload不变、新source独立durable delta、flush屏障直到全drain。
- 已在真实Agent smoke后委派三份互不重叠既有文档更新；Hub/delta与完整workspace/parity尚待验证，完整验收失败计数仍1/3。

## 阶段三source完成与第二轮完整验收
- EventDeltaDelivery完成per-source durable delta、primary不可变、已ack覆盖、恢复补漏及循环公共flush屏障；唯一写owner已冻结。worker违背no-check合同执行25项feedback前缀测试和独立barrier测试（artifact211/202），此只记录其实际输出，不代替主线程完整验收；已要求不重复检查。
- Agent图片完整函数的后续batch/GIF断言已迁移为保留原业务prefix与精确图像字节，不固定带提醒后的总content序列；source全部交付。
- 三份文档owner交付或修正中：实际CLI只有TTL+11leaf registry keys，未注册whole-map键；privateevent.inject不叫event.inject；按操作准入保留通知快照，severity不写成priority。
- 主线程开始第二轮完整命令链：fmt准备→fmt --check→cargo test --workspace→Agent/Hub build→既有venv PATH python3 live parity。阶段三source提交与阶段四证据/文档提交分开，尚不宣称整个目标完成。
- 第二轮完整链fmt准备与fmt --check通过，workspace test在新GIF断言编译失败（Content不能JSON索引），build/live被&&阻断。完整失败计数2/3；原owner已改借用as_text/as_image并检查其余新断言，未自行重验。
- 最终完整第三轮前preflight：all-targets check通过，受影响图片消费者测试通过；全部Hub包130项中129通过、1项旧Room捕获generation替换行为失败。已委派独立工程意见核对现有admission保证与新feedback屏障，不能简单删除有意义race测试或改业务契约。
- 新增EventDeltaRuntime独占parity脚本，使用真实batch source/终态与临时Hub durable partial-coverage恢复checkpoint，验证A/B均通过primary/delta可靠反馈进入实际Agent inbox及重放不重复；不宣称该checkpoint重现live admission竞态。原Parity owner不再写入。
- 一次planning edit引用未显示的line43被拒绝；精确重读后重发，没有source变化。

## 最终完整验收前冻结
- Hub guard实现保留agents→pending顺序，ready同步路径保持最初捕获lease直至enqueue，busy路径释放guard后等待feedback并只重验原target。主线程fmt准备及Hub全包preflight实际132项通过；旧Room race与新wait/invalidation边界均通过。
- 文档已复核并修正lowTTL同时覆盖外部low事件。parity补充恢复gate改为list取summary/ID、get取完整source/message，保留primary/delta独立ACK和曝光唯一性；脚本语法preflight通过。
- 两个Hub wrapper只供测试fixture使用：runs::prepare_run已cfg(test)，event_feedback::prepare同步cfg(test)；生产dispatch使用同事务prepare_in_transaction，不调用wrapper。
- 所有source/doc owner冻结；开始最终第三轮fmt --check、workspace tests、Agent/Hub build、既有venv PATH live parity完整链。完整失败计数仍2/3，不重置；尚无全部通过结论。

## 第三轮达到停止条件
- 最终完整链fmt --check实际通过；cargo test --workspace实际758通过（13 suites，1 ignored）；cargo build -p agentic-gpt -p agentic-gpt-hub实际通过。Hub production build仍有runs.rs的TransactionBehavior未使用import warning，未在停止后修正。
- live parity exit1：Agent event multi-transport MCP启动等待超时，实际Agent stderr为Error: tunnel_config_required。失败入口start_event_agent/run_agent_event_gate（scripts/check_contract_parity.py）；没有完成新的事件跨入口及后续Hubtimeout/restart/delta验收。
- 该次parity前置OpenAPI refs/schema/format检查、Agent Unix基础process/Skill检查与Standalone HTTP Skill检查实际PASS；不是事件系统完整验收通过。
- 额外只读review已结束：targetless mcp.listServers聚合仅丢掉公开events，但Agent已增加展示次数。静态路径见findings.md；未运行独立repro，不在停止后修复。
- 完整失败计数3/3，执行用户hard stop；不再修复/重验、不标goal complete。source/docs保持当前工作区，阶段source集成提交暂缓，仅提交停止planning证据。继续需要用户明确扩展失败修复上限。
