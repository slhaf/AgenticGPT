# 架构诊断与根因判断

状态：历史调查结论、已交付边界与 2026-09-23 复审并存；历史基线为 2026-09-16，WP2 closure 为 2026-09-20，WP3、WP4-A、WP-R 的有界状态以[重构计划](refactoring-plan.md)和[现状架构](current-state.md)为准。本文不把完成包扩展成全局目标架构完成。

后续用户决策见[已确认决策](decisions.md)。A01/A02/A04/A07/A09 的基线段落保留修复前机制与根因，但各节新增当前闭环/残余说明；A03/A05/A06 保留 WP2 的窄 operation/config/security 边界。WP1、WP2、WP3、WP4-A、WP-R 的证据均有范围和未验证项，不能替代外部部署、Console OS 或完整跨进程场景证明。当前 Agent direct 与 Hub paths 共享 gate/resource owners，但仍有局部 routing/projection，不应读成一个 universal dispatcher。

本文件用“历史基线”“[推断]”“[未验证]”“当前残余”区分证据等级。没有运行竞态、攻击或真实多进程场景，不声称这些风险已完成复现。

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

### A01 — 多份公共合同缺少语义一致性门槛【历史基线；WP4-A 有界闭环；残余合同边界】

**证据（WP4-A 前历史基线）**：`protocol/src/lib.rs::{JobInfo,JobListItem,JobListResponse,JobCancelResponse,NotebookSelectExactRequest}`；Hub `routes.rs::{JobListQuery,JobGetQuery}`、`room.rs::room_notebook_select_exact`；Agent `stdio_server.rs::{tool_schema,properties_for,slim_job_list_response,slim_cancel_response}`；`openapi/hub.yaml`。

具体差异（WP4-A 前）已通过解析 YAML 与源码对照确认：

- OpenAPI `JobInfo.required` 含 startedAt，而 Rust 允许未开始 Job 的 started_at 为 None 并省略。
- OpenAPI JobListResponse 使用 JobInfo，实际 DTO 使用更精简的 JobListItem 并可返回 nextCursor；静态 schema additionalProperties=false。
- HTTP Job list 的 group/cursor、Job get 的 waitOnly 未完整进入 OpenAPI；list default limit 的 100 与当前 helper 的 50 不一致。
- cancel 文档引用 JobDetail，执行端返回独立 JobCancelResponse；不能假定“同为 Job JSON”即可兼容。
- NotebookSelectExactRequest 的 OpenAPI 要求 year/month/day，实际 Hub handler 直接 decode 要求 date 的 Rust DTO；没有日期适配器。
- Agent job.get descriptor 的等待默认值与实际 dispatch 默认值不同；Skill wait helper 声明最大值但未像其他 helper 一样 clamp，应核对所有入口而非只信 schema。

**历史机制/影响**：wire、Agent-local MCP、Hub MCP、HTTP/OpenAPI 各自维护类型、schema、默认值和投影；编译只证明各自类型成立，不证明客户端收到的合同可执行。

**历史根因 [推断]**：功能按消费者入口增量接入，投影有 owner 但跨投影 parity 没有共同验收责任。

**历史建议**：建立逐操作 authority/投影表，用真实 decode/dispatch/response 验证合同；先修已确认差异，再决定可生成部分。不要强迫不同 surface 具有相同工具集合或响应。

**历史验证门槛/残余**：queued 无 startedAt、分页 nextCursor、cancel 终态、Notebook date、等待边界经真实 HTTP/MCP 表面验证；外部 Actions importer 行为未在本轮验证。

**当前状态（2026-09-23）**：以上差异是 WP4-A 前的历史基线。WP4-A 已以 `scripts/check_contract_parity.py` 和当前 `openapi/hub.yaml` 收口其选定的 HTTP/MCP/OpenAPI 实时合同与行为 gate，并保留各 surface 的不同 DTO、认证和投影；这不证明外部 Actions importer、所有 Protocol lifecycle 或未纳入 gate 的消费者。

### A02 — Room 接口 cutover 未跨端完成【历史基线；WP-R 已完成 clean cutover；不再作为当前断裂】

**证据（WP-R 前历史基线）**：Hub `mcp_server.rs::allows_tool/app_tool_descriptors/call_app_tool` 在 Full profile 广告并 dispatch 旧 notebook/diary 工具；`room.rs` 转发旧 HubCommand。Agent `local_service.rs:132-141,275-281` 将十个旧 Notebook/Diary command 返回 `room_legacy_surface_removed`。Agent 当前工具转向 `room.diary.active/read`、notebook read/search/recent、state 与 maintenance。

**历史机制/影响**：认证、active Room 路由与投递都可以成功，最终能力却明确拒绝；单端 fake connection 测试无法证明真实 Hub↔Agent parity。Coordinator 不广告这些工具，不受同一工具发现路径影响。

**历史根因判断**：用户确认 Room 能力需要远端提供，Hub API 未同步主要是实现遗漏；源码也显示 Agent 与 Hub 的合同切换未作为同一端到端工作包完成。这不是有意保留本地专属能力的产品选择。

**已确认方向（D01–D03；WP-R 前）**：补齐产品要求的 Room 远端公共面，使 Hub→Protocol→Agent 与当前本地能力语义一致；不通过隐藏工具或永久 unsupported 掩盖遗漏。协调一次升级，迁移全部调用方、移除被替代的旧路径，并随实现提供迁移文档与步骤；不刻意维持旧 alias/shim/双轨。不把旧 append 静默映射为不等价 maintenance，保留底层 Room repository/安全/文件数据。

**历史验证门槛/残余输入**：真实 Hub Full → 当前 Room Agent 读/维护链及迁移后的公共合同通过；旧入口退出与调用方升级步骤明确；现有文件数据不丢失。不要求继续支持混合旧版本。该门槛已由 WP-R 的有界证据承接；本节基线本身不是实际双进程复现。


**当前状态（2026-09-23）**：WP-R 已完成九项当前 Room operation 的 Hub→Protocol→Agent clean cutover，并迁移/退场被替代的旧 surface；Room 文件、Git、maintenance 和安全语义仍由 Agent repository 所有，Hub 只有 active lease、路由和 bounded receipt/projection。A02 因此不再是当前接口断裂。

**残余与证据边界**：未来 Room 合同仍须更新 descriptor、Protocol、HTTP/MCP projection、caller inventory 和迁移步骤；外部客户端采用情况及生产 tunnel/GitHub workflow 未在本轮证明。不得恢复旧 alias/shim，也不得把该历史基线当作删除 Room owner 的理由。

### A03 — tool visibility、capability、authorization 分散【历史基线；WP2 有界闭环；外部边界未验证】

**历史证据（WP2 前）**：Agent `state.rs::RuntimeModel::capabilities`、`stdio_server.rs::{current_tools,tool_is_available,dispatch_with_lifecycle}`、`local_service.rs::{dispatch_inner,require_capability,require_room_toolset}`；`config_cli.rs` toolset enable；`main.rs` CLI tmux 分支。

**机制/影响（WP2 前历史）**：MCP 分发中部分 Room/skills 直接调用具体模块，HubCommand 分发则经过另一套 capability gate；广告主要按 toolset。RuntimeModel 声明不支持不等于每条操作路径都检查了相同条件。CLI tmux 又是本机管理路径，确认/审计语义不应靠入口巧合决定。

**历史建议（WP2 前）**：为同一受控操作建立共同 gate 和 request context；入口保留身份认证/协议适配，具体能力保留路径/生命周期约束。CLI 管理命令是否具有不同信任等级须显式约定，不默认视为远端安全 bypass。无需先造全能 registry。

**历史验证门槛/残余边界**：按 ingress × mode/profile × toolset 选择高风险操作，验证不可见/不可用/需确认/允许的行为及错误。任何安全收紧或权限扩展独立审阅，不能掩藏于搬文件；WP2 已承接其窄 gate，外部边界仍按 closure limits 标注。


**WP2 closure（2026-09-20；有界）**：Agent `operation.rs` 现以不可变 `RequestContext`（真实 ingress、借用 operation）调用同步 `authorize(runtime, config, context)`；Local Unix 由 namespace/toolset gate 约束，Hub 保留既有 Room toolset、Skills capability/profile 与 notifications capability，Normal + explicit Room 仍有效。`local_service` 与 `operation_result` 共享 value/error/slim projection；CLI 仅四个本机 tmux admin 操作。`read_only`/`destructive`/`open_world` annotations 仅为发现/UX 元数据，不是授权。实际效果仍由 policy/path/confirmation/lease/resource owner 负责。

**WP2 evidence/limits**：526 Agent tests passed、1 ignored；Agent/Hub build 与 fmt check 通过（5 个 Browser distribution dead-code warnings）。Local Unix、hidden stdio、HTTP bearer 401/有效 session/SSE、process parity、Skill runs、Normal Room toggle、policy/limits 与 workspace/path reload，以及 Hub loopback WebSocket process/Room skills/CLI tmux/audit/live deny 均有真实证据；external tunnel/cloud、OAuth provider、Browser JS/service、OS sandbox 未运行。

### A04 — Hub 的连接、run、waiter 所有权未形成同一不变量【历史基线；WP1 有界闭环；剩余边界未验证】

**证据**：Hub `agents/{transport,lifecycle,dispatch}`、`runs.rs::store_result` 与 `state.rs` 的连接/dispatch owners。WP1 已将可靠结果 owner、连接代际和 Hub connection/dispatch ownership 收口；WP2 不改变该 wire/receipt 边界。

**机制/影响（WP1 前历史）**：持久 run 匹配与同步 waiter 完成曾分开处理；旧连接的 metadata/JobUpdate 与允许迟到的可靠 Response 需要不同分类门槛。不能从“有 connectionId/hash”推导出所有消息都受相同保护。

**根因 [推断]**：request-id rendezvous 先于可靠 envelope/连接代际演化，后增身份字段未回填所有状态转换。

**建议/剩余验证**：继续明确 `(agent, connection generation)` 与 `(agent, run, request, command hash)` 两类身份；迟到可靠消息可合法完成既有 run，但不能刷新当前连接或消费不匹配 waiter。真实 Agent restart/transport-ledger、OAuth、external ntfy 和完整跨进程 executor E2E 仍不由 WP2 证明。

### A05 — 配置可变性与启动派生资源不一致【历史基线；WP2 有界闭环；配置语义仍需明确】

**历史证据（WP2 前）**：Agent `main.rs::{watch_config,config_matches_runtime,build_app_state}`、`state.rs::AppState` 及 Standalone live reload 分支。

**机制/影响（WP2 前历史）**：watcher 曾可能替换 startup/ownership 字段，而 private state、job history、skill install、Browser runtime 和连接资源未随 config 整体重建，可能形成新 config 与旧资源 owner 混用。

**根因 [推断]**：配置从单一执行配置演变为身份、部署、资源和热参数集合，热重载没有同步升级为 mutability contract。

**历史建议/当前边界**：原建议要求明确 startup-only/live-safe/restart-required，并让运行中 Job 按一致的配置语义完成。WP2 已收窄 reload subset；但当前源码中 `jobs.rs::start_process_job_inner` 将 admission 配置复制进 `ManagedAuditContext`，随后 `run_async_job` 又读取 `state.config`，两者之间的 reload 交错仍是待定的 source-level lifetime question，不是已复现 bug。配置快照规则应在后续 targeted scenario 中决定并记录，不能以文档默认值替代证据。

**WP2 closure / WP3 boundary（有界）**：reload 现在保留既有 startup-derived resources，只应用 policy、limits、mcpServers、toolsets、httpMcp；workspace 未改变时才应用 pathPolicy。identity/mode/profile/workspace/runtime/socket、Browser 配置整体（不只是 `browser.runtime`）、history/install 等 restart-required 变化不重建半套资源；启用 Room 时先准备既有 live root。WP3 已另行盘点 durability/retention；配置 lifetime 的快照选择仍按上段保留为待证语义。

### A06 — “受控”被不同机制赋予不同保证【持续边界说明；WP2 有界闭环；非 OS sandbox】

**证据**：Agent `exec.rs::{preflight,build_command}`、`config.rs::default_config`、`policy.rs`、`mcp.rs` stdio client、`browser_kernel.rs` 与 browser dispatch；browser-host `lib.rs::{handle_client,prepare_socket}`。

**事实**：process 参数路径检查仍是启发式，bwrap 可选且默认关闭；显式配置规则可覆盖 builtin policy；外部 MCP stdio/Browser Node 不经 process sandbox；browser.repl 接受任意 JS。browser-host socket 0660，目录/组/容器挂载仍影响实际可达性。

**影响**：审批、来源校验、路径校验、风险 annotation、OS 隔离不是同一保证；任何一种都不能写成“所有执行都已 sandbox”。

**根因 [推断]**：设备能力逐步增加，而信任模型没有按本机管理员、上层 Agent、配置的外部服务、任意代码桥分层描述。

**已确认方向（D04/D05）**：保留 policy/confirmation/path/lifecycle 控制，准确说明与 OS 隔离的差别；不改 sandbox 默认、policy override 或权限模型。Neko/container/共享目录/Unix socket 的实际拓扑先盘点再选 peer/token/权限机制，不强推 owner-only。

**WP2 closure 与限制**：operation gate 将真实 ingress/context、namespace/toolset 和 capability 与 annotations 分开；它不扩大 generic sandbox，也不把 MCP、Browser JS、tmux、tunnel child 或 browser-host external effect 宣称为 core sandbox 已覆盖。WP2 未运行 external tunnel/cloud、OAuth provider、Browser JavaScript/service 或新的 OS sandbox。

### A07 — 持久化、等待和投影缺少一致的语义说明【历史基线；WP3 有界闭环；分层残余】

**证据（WP3 前历史基线）**：Agent `audit.rs`、`transport_ledger.rs`、`job_history.rs`、`hub.rs` reporting `try_send`、`config.rs::write_config_with_backup`；Hub `state.rs`、`runs.rs`、`agents/{transport,lifecycle,dispatch}` 的 dispatch/receipt 路径、`oauth.rs`。

**历史机制/影响**：

- reliable ledger、Job history、audit、report 用途不同，但部分写入是 best effort；JSONL 增长/锁/轮转与 SQLite retention 不同。
- Hub Job cache 为易失投影，未见与 runs 对称的容量/清理；OAuth、confirmation、pending 也不跨重启恢复。
- request_agent 将不同 command 的等待 timeout 记录为 process_exec_timeout；send failure 后落库但未发送的 run 与可重放意图需明确区分。
- 普通 config 的 backup + write 与 secret 的暂存替换保证不同，不能统称原子提交。

**历史根因 [推断]**：各功能自行选择最简单持久化方式，未区分必须可靠的执行身份、可丢观测和可重建缓存。

**已确认方向（D06；WP3 前）**：逐存储定义 owner、敏感性、durability、retention、恢复与失败处理。执行身份/幂等/去重/run-job结果尽量可靠；history/已产生确认结果/错误原因尽量保留且允许retention；audit/report可best-effort；Hub OAuth token/pending confirmation/临时cache重启失效可接受，不为全量durable扩张Hub。区分 dispatch、wait expired、remote running/unknown、cancel requested/confirmed；失效不能变成默认批准或任务已停止。

**历史验证门槛/残余输入**：Hub/Agent 分别重启、ledger重复/损坏/增长、history写失败、cache stale、配置写中断；不得用进程退出或 HTTP timeout 证明子任务已终止。WP3 已将这些按存储 owner 分层，后续只在对应包中补窄证据。

**WP3 closure / residual（2026-09-23）**：WP3 已按 D06 为 Agent ledger、Job history、Hub receipt、cache、audit/report、config/secret 等记录 owner、durability、retention、恢复和投影语义；这关闭了“完全没有分层说明”的宽泛诊断。剩余工作是按存储和重启/写失败场景做窄验证，并继续保持等待超时、remote unknown、cancel evidence 与真实终态分离；不声称所有外部副作用因此 durable 或可回滚。

### A08 — Console 原型/本地能力与远端控制定位不清【独立 Console 维护；源级行为风险未验证】

**证据**：`console/shared/.../App.kt`、Android `AndroidAgenticApp`、`AndroidSettingsScreen`、`AttentionListStateHolder`、`AttentionRuntimeCoordinator`、`AndroidAttentionScheduler`、`AttentionDao`、`AttentionEntityMapper`；Desktop/Web main。

**成熟度事实（不是自动判为缺陷）**：Android local Attention 有真实 Room/AlarmManager/Notification，Hub 表单无 transport；Desktop/Web 只有 placeholder；Hub Android delivery 也未实现。不能承诺跨平台控制台已可用。

**实际结构/行为风险 [推断；未验证]**：UI 与通知入口的 snooze 时长/action policy 分散；schedule result 未充分进入 state；restore 只查询未来项，没有 overdue 决策；ExactRequired 与 ExactPreferred 走相同降级路径；枚举未知值静默 fallback。现有共享测试以算术 smoke 为主，不能证明这些生命周期。

**根因 [推断]**：Android 本地 spike 由 mock 演进为真实 OS 副作用时，统一 domain policy 与跨入口恢复语义没有一起完成。

**已确认范围（D07）**：保留本地 Attention，局部状态/恢复问题在独立维护工作中处理，不计入本轮 Agent/Hub/Protocol/Room 核心完成条件。shared 保持 domain/UI ports，平台拥有持久化和 OS adapter；Hub remote console、approval board、exec ledger 以后另立产品，不引入 Hub reminder scheduler 或以 local DB 代替远端 Job authority。

**独立维护验证门槛（不阻塞核心重构）**：真实 Android 权限/精确闹钟/通知动作/重启与过期项；Desktop/Web capability诚实展示。未来网络 token、TLS/CORS、平台安全存储随产品接入另行设计。

### A09 — 当前规范、历史说明与验证工具的保证混在一起【历史基线；WP4-A 部分闭环；同步残余】

**证据（WP4-A 前历史基线）**：`docs/operations.md` 当时的工具计数与 Agent fixed surface 不一致；仓库上下文仍写三 crate；`scripts/evaluate_tool_contracts.py` 只比较预测 tool/宽松参数 shape；真正 runtime corpus 在 `stdio_server.rs::deterministic_tool_contract_corpus_exercises_public_dispatch`；当时 CI 对 OpenAPI 仅 YAML parse。

**历史机制/影响**：路径/数字/合同多处手写，未来 coding agent 容易从过时指引开始；预测质量检查被误当 runtime 合同证明；编译与 YAML 解析不能兜住跨进程接口漂移。

**历史根因 [推断]**：规范缺少明确 authority 和同步更新触发条件，历史设计、运行手册、实验与当前架构未分层。

**历史建议**：建立本目录为长期入口、开发文档链接、状态说明、PR 边界审查；为合同选择行为级 gate。历史 release/migration 应保持对应版本事实，不批量改成当前数字；未消费的 agents-minimal artifact 先确认用途再移除。不把 provenance、新 CI 平台等额外工程扩展自动纳入本次必要重构。

**历史验证门槛/残余**：相对链接存在、模块/符号可定位、合同差异有行为验证入口、每次变更记录兼容影响；不能以“所有文档已更新”替代可执行合同检查。

**WP4-A closure / residual（2026-09-23）**：`scripts/check_contract_parity.py` 已成为选定 HTTP/MCP/OpenAPI 与当前 Room 合同的实时 gate；`scripts/evaluate_tool_contracts.py` 仍只是预测 shape probe，不能冒充 runtime authority。三 crate 叙述、旧计数和历史 release/migration 数字应按 A09 历史材料处理，不据此重写当前五 crate 拓扑或历史版本。残余是维护者在新增/退场 operation、release 和文档时同步 authority/inventory，并核验外部 importer；这属于文档/发布边界，不是 WP-R 当前断裂。

## 4. 根因归纳与处理顺序

1. **入口先行的增量演化**：增加 Hub Apps、Actions、local/HTTP 后，入口复制 metadata/gate/适配而核心执行已共享。先统一操作约束，不重写执行器。
2. **状态身份晚于功能形成**：run/request/connection/boot/Job 被逐步引入，旧 waiter/map 没有同步增强。先补不变量，再移动模块。
3. **合同 cutover 不以端到端能力为单位**：Room 和 Job schema 的各端独立推进。以一项能力的所有 producer/consumer 为迁移最小闭环。
4. **资源与配置耦合隐式化**：共享 AppState 可用，但 startup-derived owner 与 mutable Config 没有严格区分。明确生命周期，不急于依赖注入框架。
5. **实验成熟度没有同步工程约束**：Android/Browser/历史 docs 的独立探索不能自动等同正式支持。明确状态，再决定保留或退出。

优先级不是按文件长度排列：A01/A02/A04/A05/A07/A09 的历史基线已有对应闭环或残余说明，不再作为 blanket open defects；当前应先按第 6 节 R01–R06 逐项选最小 seam，再按[工程规则](engineering-rules.md)和[重构计划](refactoring-plan.md)提供行为证据。A08 归独立 Console 维护，不是核心完成前置；WP4-B（Protocol 内部组织）仍待处理，WP-T 标准仍未决定。

## 5. 未采纳的扩大范围

- 不新增长期记忆、上下文管理、Provider orchestration、模型选择或通用 reasoning loop。
- 不因 Room 使用 diary/notebook/state 命名而全部删除；保留受控资源，拒绝推理责任扩张。
- 不自动引入微服务、事件总线、统一数据库、通用插件系统或新的跨语言 domain framework。
- 不把所有 placeholder 补成功能作为本轮完成条件；不把 Android 本地提醒重写成 Hub 调度器。
- 不因测试文件存在就宣称验证充分；不为形式完整新增大量永久 plumbing 测试。

## 6. 2026-09-23 当前结构残余（源级分析）

本节是当前复审提出的结构风险和条件性后续包，不是已复现运行故障，也不是已开始/已交付的代码重构。每项保留现有 owner、入口认证、投影差异和部署拓扑；实施前仍须按[目标架构](target-architecture.md)、[工程规则](engineering-rules.md)和[重构计划](refactoring-plan.md)核对源码与行为证据。

### R01 — Agent 的 direct/Hub 路由仍是两套映射，`HubCommand` 同时承担 wire DTO 与内部 union

**源证据**：Agent `stdio_server.rs::{dispatch_with_lifecycle,dispatch}` 对 process、MCP、Room、Skill、file、Browser、tmux 等保留 direct branches，选定子集再构造 `HubCommand` 进入 `local_service.rs::{dispatch,dispatch_inner}`；Hub `hub.rs::handle_hub_command` 也进入 `local_service`。共同 gate 在 `operation.rs::{authorize,RequestContext}`，命名映射在 `operation.rs::hub_command_name`，部分 Job/MCP 结果在 `operation_result.rs`。

**机制与影响**：direct Agent MCP 与 HubCommand 路径共享 WP2 authorize 和实际 resource owner，但 normalization、terminal/report hook、typed error/result projection 并非一个 universal dispatcher。`HubCommand` 是 Protocol 的跨进程 wire authority，却也被当作 Agent 内部路由载体；本地-only operation 因此可能被迫触碰 Protocol，或停留在另一条 direct branch。风险是变更 locality 和 projection drift，不是已有三套 executor，也不是已确认的行为 mismatch。

**根因 [推断]**：Hub dispatch 先以 wire enum 作为可复用 typed carrier，Agent-local MCP 随后需要更丰富的 schema、生命周期和报告钩子；复用发生在资源函数层，未按 operation family 明确内部 seam。

**保留**：`operation::authorize`、jobs/Room/MCP 等共享 resource owner、local/Hub ingress 的 framing/auth/error/projection 差异、Protocol 纯度及现有 serde/wire name。

**owner 与证据边界**：Agent operation seam 维护者按一个 family（优先 process 或 MCP）建立窄 internal call，迁移全部 caller 后再删 trampoline；以 stdio、Local Unix、worker HTTP 和 Hub 的代表性 decode/dispatch/result 场景证明 owner/gate/result 不变。不得先造 universal dispatcher/registry；当前仅为源码结构风险。

### R02 — 配置分类有多个观察点，Job admission 与 async execution 的 snapshot 规则待定

**源证据**：`jobs.rs::start_process_job_inner` 将 admission config 复制到 `ManagedAuditContext`，随后 `run_async_job` 再读取 `state.config`；配置侧有 `config::restart_required_fields`、`main::{watch_live_config,reload_live_config_once,apply_live_config_subset}`，Standalone 有 `supervisor::watch_startup_identity`，worker HTTP 还有 `http_server::reconcile` 的 listener-local 更新。

**机制与影响 [推断]**：一次 reload 可能位于 admission 与 async policy/CWD/preflight/spawn 之间，使审计快照和实际执行读取不同；startup/live 字段分类也由多个 watcher 各自消费。没有复现竞态，不能写成运行 bug或安全利用。

**根因 [推断]**：`Config` 同时承载持久 schema、live policy 和 startup-derived identity/resource 输入，watcher 与 Job 后续执行在不同阶段演化。

**保留**：WP2 的 live-safe subset、restart-required resource ownership、workspace/identity/Browser/history/install 不整体热替换、HTTP listener reconciliation，以及既有原子配置写入和安全默认。

**owner 与证据边界**：Config/lifecycle 与 Job owner 先选择并记录 admission snapshot 或 execution-start sampling 的单一规则，统一 field classification，再以 deterministic reload-between-admission-and-start 场景验证 policy/preflight/spawn/audit；未完成该证据前不借此拆 AppState 或声称 snapshot 已统一。

### R03 — `AppState`/`HubState` 是宽组合根，Skill install 与 Job lifecycle 的 owner seam 仍隐式耦合

**源证据**：Agent `state::AppState` 聚合 config/private state、Job/history、confirmation、MCP concurrency、Room/Skill locks/leases、Browser 和 transport/report senders；Hub `state::HubState` 聚合 DB、registry/connection、dispatch、run receipt、Job cache、Room lease、OAuth/confirmation/notification session。`jobs.rs` 同时含 `ManagedJobRuntime::{Process,Mcp}`、common terminalization/query、`McpConcurrency` 与 `SkillLeaseManager`；`skills.rs::run` 调用 `jobs::start_skill_job_with_hook_and_source`，`jobs` 读取 `skill_installs::package_sha256`，而 `skill_installs.rs` 又调用 `skills::{is_active,activate,read}` 和 Skill lease。

**机制与影响**：domain function 接受宽 state 后可触达无关 lock/manager；安装的 `SkillInstallStatus`/journal/recovery、运行的 managed Job/JobState 和共享 Skill lease 在模块间双向可见。结构扩展和 fixture 成本上升，但不据此判定现有 owner 行为错误。

**根因 [推断]**：组合根随 transport、durability、Skill 和 Room 功能累积；Job kind-aware cutover 统一了外部 lifecycle，却没有同时显式化 effect-specific adapters 与 Skill metadata owner。

**保留**：`AppState`/`HubState` 作为 process composition roots、Hub lock order/generation choreography、Job common identity/history/finalization、Process 与 MCP cancellation evidence 的差异、Skill install 与 Skill run 的两个状态机、Room/Browser/transport durability 的分层。

**owner 与证据边界**：Agent state/Jobs/Skill 维护者可先在现有 crate 引入窄 Room/Skill contexts，再视证据拆 jobs 的 private process/MCP sections；Hub 只在具体 feature 触及时抽 transition handles。验收应证明 Room lock、Skill install recovery/lease、Process/MCP terminalization、Hub generation/receipt 行为不变；不新增 crate、DI framework 或 generic repository。

### R04 — Hub ingress、projection 与 Apps-MCP registry 的依赖方向仍交叉

**源证据**：HTTP `routes.rs::{list_jobs,get_job,job_list_item,live_job_value,add_cache_metadata}` 与 Apps MCP `mcp_server.rs::{snapshot_job_list_filtered,cached_job_summary,snapshot_job_get,job_list_item,live_job_value,add_cache_metadata}` 分别维护 Job live/cache/freshness projection；`mcp_server::hub_info` 调用 `routes::build_hub_info_response`，`room.rs`、`notify.rs`、`oauth.rs`、`confirmation.rs` 复用 route-owned auth/error helpers。Apps MCP 还同时有生成的 tool router、`allows_tool`/`COORDINATOR_TOOLS` profile allowlist、`app_tool_descriptors` annotations 和 `call_app_tool` manual dispatch；Hub `runs::command_type` 与 Agent `operation::hub_command_name` 另维护命令名。

**机制与影响 [推断]**：HTTP 与 MCP 需要不同 auth、status/error、schema/projection，但重复 Job fallback、Room forwarding、tool advertisement/callability/inventory；route helper 被非 HTTP 模块反向依赖。当前 source 未证明现行 mismatch，风险是新增/退场 operation 时遗漏一面或把 Apps 名称误当 wire 名称。

**根因 [推断]**：HTTP Actions、Apps MCP、Room 和 registry 按各自合同增量演化，neutral control/projection owner 尚未形成。

**保留**：`agents::dispatch::request_agent`、`runs` receipt/owner checks、`room` active `(agent_id,connection_id)` lease；HTTP API-key 与 Apps-MCP OAuth/profile/`AgenticResult` 的 distinct auth/projection；Apps names 与 `HubCommand` wire names 的有意差异；Hub 不执行 Agent effects、不持有 Room content。

**owner 与证据边界**：Hub control/projection 维护者可先抽 neutral Job projection/info 与窄 Room forwarding primitive，再整理 Apps inventory；以 HTTP/MCP 代表性 Job/Room/MCP 调用、Full/Coordinator visibility 和真实 parity gate 证明行为不变。不要让一个 ingress 调另一个 ingress，也不要把 review table 变成 universal runtime registry。

### R05 — Console Android local Attention 有双 orchestrator 与不完整 scheduler contract

**源证据**：`AndroidAgenticApp` 同时组装 `AttentionListStateHolder`、`AttentionRuntimeCoordinator`、`AndroidAttentionScheduler`、Room repository/DAO 与 notification service；UI `AttentionListStateHolder::{markDone,acknowledge,snooze,cancel}` 与 receiver/alarm/boot 侧 coordinator 各自驱动 repository/DAO/scheduler；`AndroidAttentionScheduler::snooze(itemId,duration)` 缺少完整 item payload 且返回 `ScheduleMode.Degraded`，UI 通过重建 copy 绕过；`AttentionDao::queryPendingForRestore` 只查未来 due rows。

**机制与影响 [推断；未验证]**：UI 与 OS callback 可能形成不同的持久化/schedule transition；scheduler failure 未成为统一 state transition；boot/process restore 对 overdue item 没有明确 policy。该风险属于 Console local lifecycle，不是 Hub remote 缺失，也未做 Android OS 复现。

**根因 [推断]**：Android local spike 从 mock 演进到 Room/AlarmManager/Notification 时，跨入口 transition policy 与恢复语义没有由一个 use-case owner 统一。

**保留**：Android Room 作为 attention authority，AlarmManager/Notification 作为副作用，common domain/UI ports，Console 与 Rust 核心独立；remote Console/approval board/exec ledger 仍是未来产品。

**owner 与证据边界**：Console Android maintainer 在平台内统一 UI、receiver、boot restore 的 transition boundary，先决定 overdue policy 和 scheduler input，再用 targeted Android smoke 覆盖 create/snooze/fire/done/ack/cancel、permission degradation、process death/boot future+overdue rows。不得把此包加入 Agent/Hub/Protocol 核心完成门槛。

### R06 — release gate 与跨 surface inventory 仍需显式 owner

**源证据**：`.github/workflows/ci.yml` 运行 Rust/`scripts/check_contract_parity.py` gate，`.github/workflows/release.yml` 的 tag packaging 直接调用 `scripts/dist-linux.sh`；`openapi/hub.yaml` 与 `scripts/check_contract_parity.py::{ROOM_OPERATIONS,ROOM_OPERATION_BY_NAME,ROOM_RETIRED_NAMES,ROOM_RETIRED_PATHS,run_runtime_gate}` 维护当前/退场清单，`tests/tool-contract-cases/cases.json` 另有 Room cases；`scripts/evaluate_tool_contracts.py` 仍是 prediction shape probe。

**机制与影响 [推断]**：tag release workflow 未显示依赖当前 live parity/preflight；OpenAPI、gate fixtures、descriptor/route inventories 手工重复。新增/退场 operation 可能只改一份，或发布成功而未经过该合同 gate。不是当前 parity gate 或 WP-R 行为失效的证据。

**根因 [推断]**：验证与发布由不同 workflow/维护者拥有，消费者所需的投影清单合法分离但没有统一 release/inventory preflight owner。

**保留**：`openapi/hub.yaml` 的 current HTTP authority、已有 real-binary parity gate、`agents-minimal.yaml` 的 historical/noncanonical 状态、当前三 binary Linux artifact topology，以及 Console 不进入 Rust release 的独立边界。

**owner 与证据边界**：release/contract maintainer 负责决定 release preflight dependency 和窄 inventory parity check；验收是失败 gate 阻止 publish、通过时 archive/checksum 内容不变，并覆盖 OpenAPI/descriptor/HTTP/MCP 当前与 retired operation。未运行 GitHub release 或 external importer，不在本诊断中宣称其行为。

上述 R01–R06 只是条件性结构工作包；提议的内部 contexts、projection helpers、Protocol `lib.rs` domain split（WP4-B）和 release preflight 都未在本轮开始或交付。WP-T 仍待标准决策；Console local maintenance 与 remote Console future product 继续独立。
