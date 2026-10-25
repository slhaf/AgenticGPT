# 源码调查

## 已核实的落地约束

- 无 `.codegraph/`，按仓库规则直接查询源码。
- ConfigKeySpec 已有部分字段说明，但不是完整配置目录；Browser 等未覆盖，MCP/Toolset 有专用命令。CLI 与 TUI 覆盖互补，需要补齐。
- config_keys::apply_config_key 混合字符串解析/修改/即时关联校验，不能直接循环充当原生 JSON 批量引擎。
- Config::load_locked 合并默认值并规范化配置；sparse_config_value 会裁掉默认值。unset 与父对象 set 必须沿一致的默认补全语义实现。
- TUI 导入全部 optional drafts 并在 build_active_input 中重放，commit_wizard_outcome 直接写旧 build.config；需要独立记录编辑意图，不能仅将 imported_base 换成最新值。
- write_config_with_backup 已有锁外调用约定、备份、staged file、rename、目录同步；rename 后同步失败可能已修改文件。
- Agent MCP 的 stdio/Unix/HTTP 复用 AgentMcpServer；Hub HTTP/Apps MCP 是独立远程协议。
- stdio_server 默认将 arguments 原样报告；full 报告包含原始参数，敏感输入需纳入配置 API 的保护范围。
- 当前 live subset 仅复制部分配置；mode/profile 不匹配会整体拒绝 reload，其他 restart-required 字段通常只记录待重启并继续应用 live subset。

## 组件可重启调查

调查中；以接下来的逐组件源码证据补充。区分当前已支持的能力、可局部改造的能力、需要运行时生命周期重构的能力。

### 初步组件事实

- AppState 聚合了固定 runtime/transport_agent_id/boot_generation、进程注册表，以及不可替换的 Arc<EmailService>/Arc<SecretService>/OnceCell<BrowserRuntimeContext>（runtime/state.rs:175-203）。整份 AppState 重建会越过连接重建的边界，需保护已有进程和持久任务所有权。
- startup::build_app_state_with_private_state 初始化全部服务、恢复进程历史并固定技能安装并发 semaphore；启动流程不等于可反复调用的无副作用重建方法。
- InstallManager 的 records/cancellations/worker_slots 为共享状态；每项安装会取得配置快照。改全局下载规则通常可以对新任务生效，不应为了改变并发数丢弃整个 manager。
- HTTP 已有局部重绑定/认证更新机制（子调查）；Hub 虽断线重连但尚无配置触发重连入口；Unix/stdio/supervisor 的生命周期仍需显式拆分。

### 应用语义建议（尚未确认）

- 静态合法性与运行态可立即应用是两个判断维度；地址/凭据/可执行路径即便语法合法也可能连接/绑定/启动失败。
- 建议按实际效果分为：快照更新（新操作使用）、组件重连/重建（可能中断其会话）、运行时拓扑/身份切换（可能需新 worker generation）。这是具体生命周期边界，不拟新增统一重启框架。
- 对进行中进程和安装任务，已有准入/执行配置快照应保持；改变未来配置不意味着重启用户进程或重放任务。
- 下游 MCP client 当前按调用创建并退出；修改 mcpServers 不需要重启常驻客户端，进行中调用沿用其快照（mcp/mcp.rs:280-324,652-700）。
- 数据库/历史/技能安装持久根来自 ~/.agentic_gpt/state 而非 agentId；agentId 与工作目录变化不构成迁移持久状态的指令（storage/private_state.rs:6-28）。
- Room repository 不是常驻服务；切根涉及目录/Git/scaffold 准备及在途维护操作，需要准备新上下文和保护当前操作，不应通过重启伪造完成（room/room_repository.rs:306-355）。

### 配置消费偏差，不能靠重启解决

- backupLimit 只由配置保存时的备份清理读取（config/config.rs:2426），当前列 restart-required 过宽。
- room.timezone 当前除配置/UI外没有生产运行消费者；新 scaffold 的逻辑日期固定使用 Shanghai（room/room_repository.rs:549-550）。重启不会令它生效，应在后续配置完善阶段明确接通或如实标记边界。
- room.diaryDayBoundaryHour 仅用于新空仓库初始化 scaffold（room/room_repository.rs:307-310,364-365），不是现有仓库日记计算的运行参数。重启不重写已有仓库。
- 当前 live watcher 应用并非整体事务：配置交换后 resume_queued 仍可能失败，tmux 创建失败只日志。后续不能以 reload 返回值替代各组件最终应用状态。

## Agent 组件重建可行性矩阵

以下为当前源码静态调查，不表示已实现动态切换，未启动服务验证。范围是 Agent 内部组件及其连接；Hub 服务端本身不是本次配置 API 的配置所有者。

| 组件/字段组 | 当前能力与障碍 | 可行落点及影响 | 关键证据 |
| --- | --- | --- | --- |
| policy/limits/shell/events、confirmationLanguage、maintenance mode/autoPush 等规则 | 多数请求入口取得 Config 快照；已有子集交换。limits 的新上限不取消已有进程 | 后续操作使用新快照，进行中操作维持准入时语义；无需重启组件 | runtime/startup.rs:451-467；process/managed.rs:481,2489；room/room_maintenance.rs:119-138 |
| backupLimit/displayName/confirmation channels | 当前 restart-required 过宽；backupLimit 只在保存时消费，确认配置按请求读取；Hub Hello 缓存展示摘要 | 本地消费者可更新；displayName 的 Hub 投影需刷新或重连；不能将待处理确认改成另一份请求 | config/config.rs:1664-1688,2426；operations/confirmation.rs:32,109-112,445-480；ingress/hub.rs:96-103 |
| 下游 MCP | 每次调用读取 server Config 并新建 client，无常驻 client owner 需重启 | 新调用用新配置，当前调用继续；不可因连接参数变化重放已有工具调用 | mcp/mcp.rs:660-695 |
| HTTP MCP/OAuth | 已有 watch/reconcile/cancel/rebind；同 endpoint 可原地替换 token；endpoint 改变重建 auth/session manager。stop_active 无超时 await | 可局部重建，不需 AppState 重启；旧会话/OAuth 状态会失效，入口关闭不等于实际操作已取消 | ingress/http_server.rs:56-89,105-162,190-243,269-273；ingress/http_oauth.rs:143-153 |
| Hub 命令/辅助报告连接 | 故障重连每次读 Config，但无配置触发 cancel；辅助连接只在初始 enabled 时 spawn。内部 detached writer/heartbeat 无统一停止；旧命令完成使用当前 sender | 可局部重连，需连接 owner、子任务清理和代际/目标绑定。更换 Hub endpoint 时旧结果不得错发新 Hub，pending confirmation 与报告队列需收尾 | ingress/hub.rs:35-61,170-239,979-994,1415-1424；runtime/startup.rs:107-132,283-301 |
| Email | IMAP/SMTP 按操作创建，后台已有 cancellation；EmailService 固定 Arc；owner 固定 account 并持数据库 lease；停止后台不等于排空并发 dispatch/SMTP | 可以停准入、排空操作、释放旧 owner 后切换账号/重建局部 owner；保留持久账本，不重复发送。更换凭据与更换账号身份分别处理 | room/email/mod.rs:26-75,130-198,289-309；runtime/startup.rs:173-192 |
| Room Secret | 已有 shutdown：取消CLI/同步、等待写操作、锁 vault；但停止 token 不可复用，owner Arc/lease 未在 shutdown 中释放 | 可以局部关闭并新建 owner/service；替换前排空并释放所有旧 owner 引用；不能先创建第二个争同一锁的 owner | room/secret/mod.rs:76-150,215-240；runtime/state.rs:183-184 |
| Browser | 单 lease 有 reset/release，Node kernel 有 shutdown；全局 context 存 OnceCell，descriptor/launch spec 固定，manager无全局停止准入/排空接口 | 可以替换 runtime context；先处理旧 lease/在途REPL并停止所有旧 kernel。旧变量、lease与会话可能失效；reset已有kernel不等于重新解析配置 | runtime/state.rs:161-187；browser/browser_manager.rs:158,319-440；browser/browser_kernel.rs:94-99 |
| Skills 下载与安装并发 | 单任务开始时取快照；InstallManager 持 records/cancellation/target locks，semaphore在构造时固定 | 普通限制影响后续安装；并发调整需保留 manager/在途worker并重配置调度容量，不能新建manager恢复同一组活跃任务 | skills/skill_installs.rs:45-78,188-227,426-457；runtime/startup.rs:223,260-263 |
| Room repositoryRoot | 每次操作解析路径，维护持 Room 写锁捕获 Config；ensure_repository 有建目录/Git/scaffold副作用 | 准备新仓库、在同一写入边界切换上下文；旧资源不自动迁移，不能用重启声称内容已转移 | room/room_repository.rs:180-195,306-365；room/room_maintenance.rs:119-138 |
| workspaceRoot/tmux/用户进程 | 多处派生路径，已有 OS 进程与tmux session维持旧cwd；运行进程按准入快照，进程registry/boot generation属于AppState | 不宜只换字符串或重启用户进程。可另设计工作空间上下文切换并保留在途资源；本轮若不做，明确需重启且不自动迁移数据 | files/file_ops.rs:247,433；skills/skills.rs:773；tmux/tmux.rs:697-698；process/managed.rs:2489,4405-4411 |
| Unix listener | 绑定地址由agentId派生，有自身socket删除guard；accept连接detached，无全部会话停止/排空接口 | 组件可重绑，但要管理全部连接；agentId切换还涉及其他身份，不是只改socket路径 | ingress/local_control.rs:40-91,264-298,380-400 |
| Standalone tunnel/stdio | supervisor已有进程树终止与故障重启，但Invocation只构造一次，watcher仅提示；stdio退出会令worker其他入口退出 | 可重建tunnel进程子树，但必须重新解析secret/executable/Invocation；当前会带来worker/stdio会话重建，不能承诺只断开Tunnel而保持旧worker所有内存态 | runtime/supervisor.rs:54-94,264-324,341-378,453-515,874-940；runtime/startup.rs:134-170 |
| mode/profile/agentId | 决定主启动路径、固定RuntimeModel、transport identity/socket/supervisor路径及Hub命令ownership | 当前需重建运行态generation。要保持Agent主进程可做稳定外层owner并重新装配其子树，但这是运行时生命周期重构；不属于单组件重连现成功能 | runtime/startup.rs:21-29,238-248,347-353,405-418；runtime/state.rs:68-109,175-203；ingress/hub.rs:902-985 |
| EventStore/ProcessHistory/私有存储 | 持久根是HOME下共享state，不按agentId重建；配置中没有更换该根的普通字段 | 保持所有者与数据不动；服务重建不应无故重开整个AppState或把恢复逻辑重复运行 | storage/private_state.rs:15-26；runtime/startup.rs:213-263 |

## 本轮结论与待讨论范围

- 在已检查的 Agent 服务中，没有发现“外部依赖要求 Agent 主进程必须退出才能释放资源”的硬限制；但当前并未具备所有组件可重复启动/关闭的接口，不能宣称全部支持。
- 直接快照更新、局部连接/owner重建、运行时身份/拓扑重新装配，是成本与影响不同的动作。外部配置语法合法仍可能绑定失败、鉴权失败或目录不可用；组件重建不能将它们变成静态保证。
- 不建议为了动态配置直接重建整个AppState：它还持有用户进程、安装worker、确认等待、数据库和boot generation。若重装配拓扑，需将长期所有者与可替换子树分开。
- 上轮待讨论范围是“局部重配置”还是“连运行时拓扑也在线切换”；用户现已进一步提出实例角色配置不宜在线重建的问题。本轮建议收敛为启动契约保留重启边界、其他服务局部重配置，具体见下节；仍区分建议与批准。
- 组件应用失败的状态、重试/旧实例保留、会话中断及进行中副作用的处理仍为待讨论建议；现有实现无法支持“所有字段原子热应用”的承诺。

### 服务切换的具体限制

- Email SMTP 是阻塞外部I/O；停止同步后台不等于等待在途send结束。旧发送可能已被服务端接受，unknown不能因重连自动重试。保留Store并切换owner比重新打开全部状态更合适（room/email/mod.rs:631-707；room/email/network.rs:404-417）。
- Secret shutdown 的停止标记和取消token不可复用，且返回后旧Service仍可能持owner lease；需要释放全部旧owner引用才能重新取得同身份锁。现有CLI的写结果unknown仍需保留（room/secret/mod.rs:215-242；room/secret/cli.rs:131-171）。
- Browser failure None 目前会被缓存到OnceCell；新配置或依赖恢复需要允许重新发现。单lease reset/release不构成服务级shutdown_all；repl/原地reset部分路径没有Agent侧外层超时，不能宣称现有排空有界（runtime/startup.rs:232-235；ingress/stdio_server.rs:1421-1435；browser/browser_manager.rs:279-448）。
- 组件替换后的回滚不能承诺总能保留旧实例：同端口重绑、排他账户lease及旧service永久shutdown等情况下，准备新实例前可能必须先停止旧实例；旧配置重新启动也可能受外部环境影响而失败。

## 启动契约与局部控制结构（本轮建议，尚未批准实现）

### 实例角色、身份与启动参数

- mode 决定 run_hub/supervisor/run_local 三个不同主入口（runtime/startup.rs:21-29）；profile 固定在 RuntimeModel 并决定能力、Hello/报告角色（runtime/state.rs:68-116；ingress/hub.rs:98,258,360,516）。
- agentId 同时固定在传输身份、本地 socket 和 supervisor runtime paths（runtime/state.rs:177-178；ingress/local_control.rs:80-91；runtime/supervisor.rs:70）。在线改名不是普通服务重连。
- tunnelId/apiKey/client 启动参数被 supervisor 解析成 Invocation，后者还包含 worker 启动命令/profile/token（runtime/supervisor.rs:73-86,339-378）。替换该子树当前会影响worker/stdio，不应为了“全热更新”加入本次在线实例迁移。
- tunnel 对象仍应按消费者拆开：hubReporting.detail 由每次报告读取（ingress/hub.rs:589-600），enabled 控制辅助 Hub 生命周期；它们不是 Tunnel 进程身份。当前 StartupIdentity 将这些字段全列为启动变化（runtime/supervisor.rs:179-215），后续需按实际边界修正。

### 具体所有者配合串行协调

- 配置持久层继续负责最新文件上的 changes、最终校验、备份与提交；组件操作在配置文件写锁之外进行。
- 一个具体的配置应用协调入口读取最新持久目标，判定启动字段/直接更新字段/受影响局部组件，并串行触发具体 reconfigure；不把旧完整候选按请求依次重放，不让并行重建覆盖彼此。
- 每个组件保留具体类型及控制方法：HTTP 管 listener/auth/session；Hub 管连接任务及sender/旧结果归属；Email 保留Store并切换账号owner/同步后台；Secret 排空旧owner并重建CLI/session；Browser 排空lease并替换context；Skills 调整容量但保留manager和worker。
- 构造函数可以继续沿用各组件现有 new/open/start；需要共用的只应是小型任务控制/状态投影，不引入抽象 ComponentFactory、DI容器、动态注册表或依赖图引擎。
- 可统一变更分类和应用结果形状，但不强制统一所有生命周期步骤。不同资源不能保证相同的 prepare-before-stop/回滚能力；持久数据与在途副作用也不由协调层重置。
- AppState 对外维持稳定的服务入口，替换服务内部的连接/owner/runtime slot；不将全部服务统一成动态对象，也不重建包含用户进程与持久任务的整个 AppState。

### 待重启字段不得阻塞无关热更新

- 当前 reload_live_config_once 在 candidate 的 mode/profile 与当前 runtime 不符时直接返回（runtime/startup.rs:405-418）；保存未来角色后会连后续limits更新也挡住。
- 建议分别管理持久目标与实际已应用配置：启动字段在当前实例保留启动值，其余字段按实际应用结果更新。应用候选按当前启动契约构造/校验，不能拿未来模式验证当前实例；目标配置自身的有效性仍须校验。
- 局部组件失败时不能直接将整个 desired Config 宣布已生效。具体失败保留/重试/响应合同仍待讨论，未在本轮擅定。
- 启动契约同样必须约束故障恢复：当前 supervisor 的 worker 命令固定旧 profile，但 worker 重新从可变 config_path 读取并检查 mode/profile（runtime/supervisor.rs:359-370；runtime/startup.rs:85-96）。保存未来 mode/profile 后，当前 worker 一旦故障重启会被 stdio_worker_config_mismatch 拒绝。后续设计须让同一 Agent 生命周期的重拉子进程使用当前启动契约；只有用户重启整个实例才采用新的启动值。不能只修热更新过滤而遗漏恢复路径。
