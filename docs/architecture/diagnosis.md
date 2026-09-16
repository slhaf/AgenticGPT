# 架构诊断与根因判断

状态：调查结论，不是已实施修复；基线 2026-09-16。现状见 [current-state](current-state.md)，实施顺序见 [refactoring-plan](refactoring-plan.md)。

## 1. 总判断

Agentic 不是已经失去所有边界的单体。实际执行核心、Hub 控制平面、wire 协议、纯 patch 算法和独立浏览器桥已有可用分工。应保留这些边界，优先收敛**入口到操作的共同规则、跨端契约、状态所有权及恢复语义**，而不是先按理论模板分 crate 或把所有模块改成 service/repository/interface。

以下“高优先级”表示整理时应优先验证/解决，不等于已确认可远程利用的漏洞。事实、静态推导、根因推断和待运行验证分开记录。没有运行竞态、攻击或真实多进程场景，不声称这些风险已完成复现。

## 2. 应保留与不应误判的结构

| 保留项 | 原因 | 不应做的整理 |
|---|---|---|
| jobs/exec/policy/confirmation 共享执行核心 | Hub/Local/Standalone 已复用实际副作用实现 | 误判为三套执行器后重写 |
| Job state/cancel evidence/UnknownAfterRestart | 正确表达等待、取消与不可证明的重启结果 | 将 unknown 当 failure 或自动重新执行 |
| Hub receipt + Agent ledger | 控制面收据与执行端去重各有 owner | 合并为一个跨机器数据库/Hub executor |
| file_ops 与 apply-patch | 权限/文件事务和纯文本算法分离 | 让纯 parser 获得文件权限，或删除其 crate |
| Browser distribution/runtime/manager 与 browser-host | 来源、lease、扩展桥属于不同生命周期 | 按名字近似就去重或合并进程 |
| local UID/socket guard、HTTP auth、profile | 是真实入口保护结构 | 统一入口时丢掉各 transport 的特定校验 |
| skills install journal/lease 与 Room Git/semantic slots | 是包与受控文件资源的完整边界 | 将其扩展为上下文/推理引擎，或仅凭名称全部删除 |
| Console 平台 adapter、独立 UX demo | OS 能力与生产执行器本就不同 | 把 demo/mock 状态当跨平台生产模型 |

## 3. 主要问题清单

### A01 — 多份公共合同缺少语义一致性门槛【高；已确认静态差异】

**证据**：`protocol/src/lib.rs::{JobInfo,JobListItem,JobListResponse,JobCancelResponse,NotebookSelectExactRequest}`；Hub `routes.rs::{JobListQuery,JobGetQuery}`、`room.rs::room_notebook_select_exact`；Agent `stdio_server.rs::{tool_schema,properties_for,slim_job_list_response,slim_cancel_response}`；`openapi/hub.yaml`。

具体差异已通过解析 YAML 与源码对照确认：

- OpenAPI `JobInfo.required` 含 startedAt，而 Rust 允许未开始 Job 的 started_at 为 None 并省略。
- OpenAPI JobListResponse 使用 JobInfo，实际 DTO 使用更精简的 JobListItem 并可返回 nextCursor；静态 schema additionalProperties=false。
- HTTP Job list 的 group/cursor、Job get 的 waitOnly 未完整进入 OpenAPI；list default limit 的 100 与当前 helper 的 50 不一致。
- cancel 文档引用 JobDetail，执行端返回独立 JobCancelResponse；不能假定“同为 Job JSON”即可兼容。
- NotebookSelectExactRequest 的 OpenAPI 要求 year/month/day，实际 Hub handler 直接 decode 要求 date 的 Rust DTO；没有日期适配器。
- Agent job.get descriptor 的等待默认值与实际 dispatch 默认值不同；Skill wait helper 声明最大值但未像其他 helper 一样 clamp，应核对所有入口而非只信 schema。

**机制/影响**：wire、Agent-local MCP、Hub MCP、HTTP/OpenAPI 各自维护类型、schema、默认值和投影；编译只证明各自类型成立，不证明客户端收到的合同可执行。

**根因 [推断]**：功能按消费者入口增量接入，投影有 owner 但跨投影 parity 没有共同验收责任。

**建议**：建立逐操作 authority/投影表，用真实 decode/dispatch/response 验证合同；先修已确认差异，再决定可生成部分。不要强迫不同 surface 具有相同工具集合或响应。

**验证门槛**：queued 无 startedAt、分页 nextCursor、cancel 终态、Notebook date、等待边界经真实 HTTP/MCP 表面验证。外部 Actions importer 行为未在本轮验证。

### A02 — Room 接口 cutover 未跨端完成【高；源码可达链已确认】

**证据**：Hub `mcp_server.rs::allows_tool/app_tool_descriptors/call_app_tool` 在 Full profile 广告并 dispatch 旧 notebook/diary 工具；`room.rs` 转发旧 HubCommand。Agent `local_service.rs:132-141,275-281` 将十个旧 Notebook/Diary command 返回 `room_legacy_surface_removed`。Agent 当前工具转向 `room.diary.active/read`、notebook read/search/recent、state 与 maintenance。

**机制/影响**：认证、active Room 路由与投递都可以成功，最终能力却明确拒绝；单端 fake connection 测试无法证明真实 Hub↔Agent parity。Coordinator 不广告这些工具，不受同一工具发现路径影响。

**根因 [推断]**：Room 从旧 JSONL surface 转向文件仓库/维护语义时，Agent 与 Hub 的合同切换未作为同一个跨端工作包交付。

**建议**：先列清旧客户端和所需远端能力，再明确版本化支持面；为保留的能力实现端到端合同，删除已退出合同的旧路径。不偷偷把旧 append 映射成新 maintenance，也不无限期添加 alias。保留底层 Room repository/安全/文件数据。

**验证门槛**：真实 Hub Full → 当前 Room Agent 读/维护链；旧客户端得到约定的版本错误或升级结果；现有文件数据不丢失。本轮为静态可达链确认，不是实际双进程复现。

### A03 — tool visibility、capability、authorization 分散【高；结构风险已确认】

**证据**：Agent `state.rs::RuntimeModel::capabilities`、`stdio_server.rs::{current_tools,tool_is_available,dispatch_with_lifecycle}`、`local_service.rs::{dispatch_inner,require_capability,require_room_toolset}`；`config_cli.rs` toolset enable；`main.rs` CLI tmux 分支。

**机制/影响**：MCP 分发中部分 Room/skills 直接调用具体模块，HubCommand 分发则经过另一套 capability gate；广告主要按 toolset。RuntimeModel 声明不支持不等于每条操作路径都检查了相同条件。CLI tmux 又是本机管理路径，确认/审计语义不应靠入口巧合决定。

**根因 [推断]**：原本不同用途的“是否展示”“当前部署支持”“是否授权执行”“风险提示”逐渐被各入口当成部分替代品。

**建议**：为同一受控操作建立共同 gate 和 request context；入口保留身份认证/协议适配，具体能力保留路径/生命周期约束。CLI 管理命令是否具有不同信任等级须显式约定，不默认视为远端安全 bypass。无需先造全能 registry。

**验证门槛**：按 ingress × mode/profile × toolset 选择高风险操作，验证不可见/不可用/需确认/允许的行为及错误。任何安全收紧或权限扩展独立审阅，不能掩藏于搬文件。

### A04 — Hub 的连接、run、waiter 所有权未形成同一不变量【高；静态完整性风险】

**证据**：Hub `agents.rs::{handle_socket,post_agent_message,handle_agent_message,request_agent,replace_agent_connection}`、`runs.rs::store_result`、`state.rs::pending`。Response 分支记录 store_result 的 Err，但不依据其 bool 匹配结果决定是否 `pending.remove(request_id)`。WS 与 SSE 对 stale 非可靠消息的检查不完全对称。

**机制/影响**：持久 run 匹配与同步 waiter 完成被分开处理；旧连接的 metadata/JobUpdate 与允许迟到的可靠 Response 没有统一分类门槛。不能从“有 connectionId/hash”推导出所有消息都受相同保护。

**根因 [推断]**：request-id rendezvous 先于可靠 envelope/连接代际演化，后增身份字段未回填所有状态转换。

**建议**：明确 `(agent, connection generation)` 与 `(agent, run, request, command hash)` 两类身份；迟到可靠消息可合法完成既有 run，但不能刷新当前连接或消费不匹配 waiter。绑定确认发起 owner；对 RunReport 定义合法状态转移。

**验证门槛**：错 owner/hash/run、重复与迟到结果、旧 WS Hello/JobUpdate、SSE parity、重连确认、断线 cleanup。需要可重复场景后再作安全影响定级；本轮不声称无条件跨 Agent 攻击成立。

### A05 — 配置可变性与启动派生资源不一致【高于纯目录整理；静态风险】

**证据**：Agent `main.rs::{watch_config,config_matches_runtime,build_app_state}` 与 Standalone live reload 分支；`state.rs::AppState` 持有 private_state/history/install/browser 等启动派生资源。

**机制/影响**：Hub 模式 watcher 主要检查 mode/profile 后替换整份 Config，而其他形态已有 live-safe subset。agent id、workspace、browser descriptor 等变化不意味着已创建数据库/manager/连接也重建，可能形成新配置与旧资源 owner 混用。

**根因 [推断]**：配置从单一执行配置演变为身份、部署、资源和热参数集合，热重载没有同步升级为 mutability contract。

**建议**：列字段为 live-safe/startup-only/需专用迁移；统一 reload policy，拒绝需要重启的变更并报告具体字段，而不是隐式部分生效。

**验证门槛**：各模式 reload policy/toolset 与修改 agent id/workspace/browser 的对照；正在运行 Job 按原配置快照完成；history/audit/连接身份一致。

### A06 — “受控”被不同机制赋予不同保证【高优先级边界澄清；不是自动改安全默认】

**证据**：Agent `exec.rs::{preflight,build_command}`、`config.rs::default_config`、`policy.rs`、`mcp.rs` stdio client、`browser_kernel.rs` 与 browser dispatch；browser-host `lib.rs::{handle_client,prepare_socket}`。

**事实**：process 参数路径检查是启发式，bwrap 可选且默认关闭；显式配置规则可覆盖 builtin policy；外部 MCP stdio/Browser Node 不经 process sandbox；browser.repl 接受任意 JS。browser-host socket 0660，未采用 local MCP 的 UID/0700/0600 guard。目录/组/容器挂载仍会影响实际可达性。

**影响**：审批、来源校验、路径校验、风险 annotation、OS 隔离不是同一保证；把任何一种写成“所有执行都已 sandbox”会误导部署者。

**根因 [推断]**：设备能力逐步增加，而信任模型没有按“本机管理员、上层 Agent、配置的外部服务、任意代码桥”分层描述。

**建议**：明确 trusted mode 与 sandbox enforcement、每种能力的实际权限和确认粒度；browser-host 单独评审共享组/容器场景。保留现有默认，改变授权/沙箱/路径默认须显式兼容和产品决策。worker token 与 tunnel key 分别审查，不能用日志脱敏代替 argv 暴露分析。

**验证门槛**：受限 root/脚本间接访问、downstream spawn、browser JS/bridge owner、trusted 与 sandbox 模式；本轮没有运行这些攻击或隔离实验。

### A07 — 持久化、等待和投影缺少一致的语义说明【中高；恢复风险】

**证据**：Agent `audit.rs`、`transport_ledger.rs`、`job_history.rs`、`hub.rs` reporting `try_send`、`config.rs::write_config_with_backup`；Hub `state.rs`、`runs.rs`、`agents.rs::request_agent`、`oauth.rs`。

**机制/影响**：

- reliable ledger、Job history、audit、report 用途不同，但部分写入是 best effort；JSONL 增长/锁/轮转与 SQLite retention 不同。
- Hub Job cache 为易失投影，未见与 runs 对称的容量/清理；OAuth、confirmation、pending 也不跨重启恢复。
- request_agent 将不同 command 的等待 timeout 记录为 process_exec_timeout；send failure 后落库但未发送的 run 与可重放意图需明确区分。
- 普通 config 的 backup + write 与 secret 的暂存替换保证不同，不能统称原子提交。

**根因 [推断]**：各功能自行选择最简单持久化方式，未区分必须可靠的执行身份、可丢观测和可重建缓存。

**建议**：逐存储定义 owner、敏感性、durability、retention、恢复与失败处理；不为了统一而全部搬进同一个数据库。区分 dispatch、wait expired、remote running/unknown、cancel requested/confirmed。

**验证门槛**：Hub/Agent 分别重启、ledger重复/损坏/增长、history写失败、cache stale、配置写中断；不得用进程退出或 HTTP timeout 证明子任务已终止。

### A08 — Console 原型/本地能力与远端控制定位不清【中；成熟度与局部行为问题分开】

**证据**：`console/shared/.../App.kt`、Android `AndroidAgenticApp`、`AndroidSettingsScreen`、`AttentionListStateHolder`、`AttentionRuntimeCoordinator`、`AndroidAttentionScheduler`、`AttentionDao`、`AttentionEntityMapper`；Desktop/Web main。

**成熟度事实（不是自动判为缺陷）**：Android local Attention 有真实 Room/AlarmManager/Notification，Hub 表单无 transport；Desktop/Web 只有 placeholder；Hub Android delivery 也未实现。不能承诺跨平台控制台已可用。

**实际结构/行为风险**：UI 与通知入口的 snooze 时长/action policy 分散；schedule result 未充分进入 state；restore 只查询未来项，没有 overdue 决策；ExactRequired 与 ExactPreferred 走相同降级路径；枚举未知值静默 fallback。现有共享测试以算术 smoke 为主，不能证明这些生命周期。

**根因 [推断]**：Android 本地 spike 由 mock 演进为真实 OS 副作用时，统一 domain policy 与跨入口恢复语义没有一起完成。

**建议**：先明确 local-only 并收敛本地行为；shared 保持 domain/UI ports，平台拥有持久化和 OS adapter。Hub client 是另一个产品功能切片，非架构整理必做扩展；不引入 Hub reminder scheduler 或以 local DB 代替远端 Job authority。

**验证门槛**：真实 Android 权限/精确闹钟/通知动作/重启与过期项；Desktop/Web capability诚实展示。未来网络 token、TLS/CORS、平台安全存储须随接入另行设计。

### A09 — 当前规范、历史说明与验证工具的保证混在一起【中；持续漂移根因】

**证据**：`docs/operations.md` 当前工具计数与 Agent fixed surface 不一致；仓库上下文仍写三 crate；`scripts/evaluate_tool_contracts.py` 只比较预测 tool/宽松参数 shape；真正 runtime corpus 在 `stdio_server.rs::deterministic_tool_contract_corpus_exercises_public_dispatch`；CI 对 OpenAPI 仅 YAML parse。

**机制/影响**：路径/数字/合同多处手写，未来 coding agent 容易从过时指引开始；预测质量检查被误当 runtime 合同证明；编译与 YAML 解析不能兜住跨进程接口漂移。

**根因 [推断]**：规范缺少明确 authority 和同步更新触发条件，历史设计、运行手册、实验与当前架构未分层。

**建议**：建立本目录为长期入口、开发文档链接、状态说明、PR 边界审查；为合同选择行为级 gate。历史 release/migration 应保持对应版本事实，不批量改成当前数字；未消费的 agents-minimal artifact 先确认用途再移除。不把 provenance、新 CI 平台等额外工程扩展自动纳入本次必要重构。

**验证门槛**：相对链接存在、模块/符号可定位、合同差异有行为验证入口、每次变更记录兼容影响；不能以“所有文档已更新”替代可执行合同检查。

## 4. 根因归纳与处理顺序

1. **入口先行的增量演化**：增加 Hub Apps、Actions、local/HTTP 后，入口复制 metadata/gate/适配而核心执行已共享。先统一操作约束，不重写执行器。
2. **状态身份晚于功能形成**：run/request/connection/boot/Job 被逐步引入，旧 waiter/map 没有同步增强。先补不变量，再移动模块。
3. **合同 cutover 不以端到端能力为单位**：Room 和 Job schema 的各端独立推进。以一项能力的所有 producer/consumer 为迁移最小闭环。
4. **资源与配置耦合隐式化**：共享 AppState 可用，但 startup-derived owner 与 mutable Config 没有严格区分。明确生命周期，不急于依赖注入框架。
5. **实验成熟度没有同步工程约束**：Android/Browser/历史 docs 的独立探索不能自动等同正式支持。明确状态，再决定保留或退出。

优先级不是按文件长度排列：A01/A02 先建立真实合同；A04/A03/A05 是安全与状态边界；A06 先澄清保证再决策；A07 定义恢复等级；A08 限定本地职责；A09 为持续维护提供约束。可并行的验证与文档工作见路线图。

## 5. 未采纳的扩大范围

- 不新增长期记忆、上下文管理、Provider orchestration、模型选择或通用 reasoning loop。
- 不因 Room 使用 diary/notebook/state 命名而全部删除；保留受控资源，拒绝推理责任扩张。
- 不自动引入微服务、事件总线、统一数据库、通用插件系统或新的跨语言 domain framework。
- 不把所有 placeholder 补成功能作为本轮完成条件；不把 Android 本地提醒重写成 Hub 调度器。
- 不因测试文件存在就宣称验证充分；不为形式完整新增大量永久 plumbing 测试。
