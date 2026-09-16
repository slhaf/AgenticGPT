# 现状架构：受控执行核心与多入口控制面

状态：源码调查快照，2026-09-16。目标规则另见 [目标架构](target-architecture.md)，问题判断见 [诊断](diagnosis.md)。文中代码路径均相对仓库根；符号名优先于可能随编辑变化的行号。

## 1. 调查范围和证据等级

本轮覆盖五个 Rust crate 的一级模块、Console 四个 Gradle 模块及平台源集、OpenAPI、合同 corpus/evaluator、CI/release、部署脚本、实验与 UX 示例。重点追踪入口到副作用、确认/取消、断线恢复、持久化与能力边界；不是逐行安全审计，也不宣称所有平台均已运行验证。

证据来自源码、CodeGraph 定位、Cargo metadata 与解析后的 OpenAPI。完整分域记录和核验记录位于 `.planning/2026-09-16-architecture-audit/`。本轮没有启动真实 Hub/Agent/Android/浏览器/tunnel，没有读取用户密钥或 `~/.agentic_gpt`。已存在测试是后续验证入口，不是本轮测试通过证明。

规模口径：对已跟踪 `.rs/.kt/.kts/.js/.ts/.py/.sh` 文件统计物理行，含注释、测试、配置和脚本，共 148 文件、91,041 行。执行端 69,497；Hub 9,965；protocol 3,223；apply-patch 1,146；browser-host 866；Console 2,868；其余为示例/实验/脚本。文件大只能说明调查优先级，不能单独证明架构错误。

## 2. 实际部署与编译边界

### 2.1 五 crate，而非三个

由根 `Cargo.toml` 和 `cargo metadata --no-deps --format-version 1 --offline` 确认：

| 单元 | 产物/职责 | 本地 crate 依赖 |
|---|---|---|
| `agentic-gpt` | Linux 本地执行端、CLI/TUI、多种 MCP/Hub 入口 | protocol、apply-patch |
| `agentic-gpt-hub` | HTTP/MCP 控制平面、Agent 接入、确认协调、运行收据 | protocol |
| `agentic-gpt-protocol` | Rust wire DTO、枚举、必要的纯参数规则 | 无 |
| `agentic-apply-patch` | patch 解析和文本变换库；不拥有文件系统权限 | 无 |
| `agentic-browser-host` | 独立 Native Messaging 扩展桥；lib + binary | 无 |

运行时关系不是 Cargo 依赖图：browser-host 虽随 release 打包，但执行端没有导入其 Rust 库，也不拥有其内部连接状态。两者通过外部浏览器生态/本地桥协作，不能把它等同于执行端 BrowserRuntimeManager。

```text
上层 Agent / ChatGPT / 工具客户端
  ├─ Hub HTTP Actions 或 Hub /mcp
  │    └─ agentic-gpt-hub ─WS/SSE envelope─> agentic-gpt (Hub mode)
  ├─ Tunnel ─> tunnel-client ─stdio─> agentic-gpt stdio-worker
  │                                   ├─ owner-only Unix MCP
  │                                   ├─ 可选 HTTP MCP
  │                                   └─ 可选 reporting-only Hub 连接
  └─ 同机 local CLI / Process TUI ─Unix MCP─> agentic-gpt (Local/worker)

agentic-gpt 执行核心
  ├─ process / managed jobs / files / tmux / skills / Room
  ├─ 下游 MCP HTTP 或 stdio server
  └─ BrowserRuntimeManager ─Node REPL─> 外部 browser-client/service

浏览器扩展 <─Native Messaging─> agentic-browser-host <─本地 Unix framed JSON─ 客户端
```

### 2.2 运行形态不是不同产品核心

`main.rs::build_app_state` 被 Hub、Local、Standalone worker 复用；`state.rs::RuntimeModel` 区分 transport/profile/capabilities。

| 形态 | 状态/进程 owner | 入口与限制 |
|---|---|---|
| Local | agent 进程持 AppState/run lock | owner-only Unix MCP；local CLI 是客户端 |
| Standalone | supervisor → 外部 tunnel-client → hidden stdio-worker；worker 持 AppState | stdio + Unix + 可选 HTTP；可选 Hub reporting-only，不接受 Hub 执行命令 |
| Hub Agent | agent 进程持 AppState/run lock | Hub WS/SSE command-capable；本地执行，不是 Hub 进程执行 |
| Process TUI | 独立观察器 | LocalJobClient 轮询本机 `job.list`；当前不创建/取消 Job |
| Hub server | 单进程、单 SQLite owner | 公共 API、连接/路由、收据与投影；不是执行器 |

`RuntimeModel` 的 capability 与 `toolsets.enabled` 是不同机制：Hub+Normal 的 skills/bootstrap/Room 能力受限；Local/Tunnel+Normal 可用 skills/bootstrap，Room profile 扩展 Room 能力。Agent-local 固定 surface 测试给出 Normal 29、Room 40 工具，但实时广告还受 toolsets 过滤。Hub Full/Coordinator 是另一组入口 profile，不要求与 Agent-local 工具集合完全相同。当前 visibility、capability 和操作 gate 尚未完全统一，见诊断 A03。

## 3. 模块职责地图

以下路径中的 Agent 前缀为 `crates/agentic-gpt/src/`，Hub 前缀为 `crates/agentic-gpt-hub/src/`。

### 3.1 执行端

| 模块组 | 当前职责与边界 |
|---|---|
| `main.rs`、`state.rs`、`instance_lock.rs`、`utils.rs` | 启动组装、运行形态、共享状态、单实例和路径基础；main 同时承载 CLI 与热重载 |
| `stdio_server.rs` | MCP descriptor/schema、toolset 过滤、参数/结果适配、生命周期和部分直接操作分发；被 stdio/Unix/HTTP 复用 |
| `local_service.rs` | HubCommand 到 value-returning 本地操作映射，部分 capability/Room gate；不是纯粹无协议依赖的应用层 |
| `hub.rs`、`transport_ledger.rs` | WS/SSE、Hello/heartbeat、可靠 envelope、ACK/重放、Job/report、确认响应 |
| `local_control.rs` | Unix listener/client、UID/权限、stale socket inode guard、LocalJobClient |
| `http_server.rs`、`http_oauth.rs` | worker HTTP MCP、Bearer/PKCE、Host/Origin/resource 边界 |
| `supervisor.rs`、`tunnel_distribution.rs` | tunnel/worker 进程链、secret reference、健康、退避重启、下载缓存 |
| `jobs.rs`、`job_history.rs` | Process/Skill/MCP 准入、等待、取消、输出上限、终态、SQLite 历史与重启 unknown |
| `exec.rs`、`policy.rs`、`confirmation.rs` | process preflight/CWD/path、program rule、可选 bwrap、人工确认及临时 MCP allow |
| `file_ops.rs` | 文件路径策略、读/搜索、patch 计划、revision/lock/revalidation、暂存提交与审计；调用纯 apply-patch 库 |
| `mcp.rs` | 下游 HTTP/stdio server、配置快照、call/batch、确认、并发与取消；效果通过 managed Job 观测 |
| `tmux.rs` | 外部 tmux server/session/pane 观察与控制；session 生命周期不等同 Agent child Job |
| `skills.rs`、`skill_installs.rs` | 包资源/激活/执行路径、安装任务、staging/digest/commit journal/恢复；skill run 复用 Process Job |
| `bootstrap.rs`、`room_repository.rs`、`room_reads.rs`、`room_maintenance.rs` | 有界 bootstrap/Room 文件资源、Git/scaffold、语义槽位与维护提交；不是 reasoning loop |
| `browser_distribution.rs`、`browser_runtime.rs` | 签名包/哈希/解包与缓存、descriptor/managed/desktop runtime 发现 |
| `browser_kernel.rs`、`browser_manager.rs`、`browser_manual.rs` | Node 子进程、命名 lease 串行调用/reset/reaper/release、受限运行时文档读取 |
| `config.rs`、`config_cli.rs`、`config_setup/`、`config_templates.rs`、`config_tui/` | 配置加载/import/default/修改、向导 draft/validation/review/commit、secret 写入；并非新执行核心 |
| `tui/`、`cli_i18n.rs`、`notify.rs`、`agent_info.rs` | 展示/输入/终端恢复、本地化、桌面通知、诊断摘要 |
| `private_state.rs`、`audit.rs` | 私有状态目录与迁移、workspace append-only 审计；耐久等级不同于 Job history/transport ledger |

### 3.2 Hub

| 模块 | 当前职责 |
|---|---|
| `main.rs`、`state.rs` | CLI/配置/路由/后台清理、HubState；main 还拥有远程确认回调 |
| `agents.rs` | Agent secret 接入、WS/SSE、连接替换、Hello、消息处理、request_agent/重放 |
| `routes.rs` | action-key HTTP process/Job/tmux/MCP/运行查询及响应适配 |
| `mcp_server.rs`、`agentic_result.rs` | Apps JSON-RPC、tool router/schemars、Full/Coordinator、业务 JSON → MCP result |
| `runs.rs` | command/run 收据、hash/ACK/status/result/conflict、stale 与 retention |
| `room.rs` | 单一 active Room 连接租约、Room/skills 转发；不持有 Room 内容 |
| `notify.rs` | freedesktop Agent/ntfy 渠道、健康缓存、Android 注册但未实现 delivery |
| `oauth.rs` | 授权码/PKCE/token 与 MCP Bearer 校验，session 在内存 |
| `db.rs`、`registry.rs` | SQLite schema/兼容增列、Agent 注册/启停/alias/secret hash |
| `instance_lock.rs`、`utils.rs` | 同 DB serve 锁、ID/hash/constant-time 比较 |

## 4. 关键调用链与生命周期

### 4.1 Hub 远程执行

`routes::process_exec` 或 `mcp_server::call_app_tool` → `agents::request_agent` → `runs::prepare_run` 持久化 command/hash → pending waiter + envelope → Agent `hub::handle_reliable_envelope` → `transport_ledger::accept`/ACK → `local_service::dispatch` → `jobs`/具体能力 → Response/JobUpdate/RunReport → Hub `runs::store_result` 与 pending waiter。

Hub request/run 是控制面投递与收据身份；Job 是执行端资源生命周期；connection id 是连接代际；boot generation 是执行端进程代际。它们不能互换。同步等待超时不等于取消远端任务，迟到结果可以到达。当前 Response owner 校验与 waiter 唤醒之间存在缺口，不能把已有 hash 机制理解为所有路径都已严格保证完整性。

### 4.2 本地与 Standalone

`supervisor::run` → tunnel-client 的 MCP command → `main::run_stdio_worker` 验证 worker authorization → `build_app_state` → stdio/Unix/可选 HTTP 共用 `AgentMcpServer` → `dispatch_with_lifecycle` → 具体能力。

`ResumableStdioTransport` 在 worker 恢复时处理内部 initialize/session 恢复，不能直接替换为“每次请求启动全新进程”。Local 不需要 tunnel；Standalone reporting-only Hub 不能回流远端执行。

### 4.3 Process、Skill、MCP

Process：policy/CWD/preflight → Deny 或 cancellable confirmation → 可选 sandbox command → child/输出 tail/monitor → terminal snapshot/audit/report。Skill 先校验激活包、脚本路径与 lease，再进入同一 Process Job。

MCP：server/tool/args 校验与配置快照 → managed Job/批量准入 → confirmation/temporary allow → global/per-server semaphore → HTTP 或 stdio 下游 → 响应/timeout/cancel/detached。batch fail-fast 不回滚已经发生的副作用。配置的 stdio MCP server 与 Browser Node 不是自动由 process bwrap 包裹的子执行器。

### 4.4 文件与 Room

File edit：路径/权限/保留路径 → patch parse/transform → 排序 path locks/revision → 临时文件 → 必要确认 → 路径与revision再校验 → commit/audit。apply-patch 只负责算法，不得单独绕过文件权限层调用为公共工具。

Room：仓库根/软链接/Git/scaffold 检查 → bounded diary/notebook/state 读取，或 `room_maintenance::submit` 在写锁下执行预定义语义槽位维护。Hub 只选择 active Room 连接。目前 Hub Full 仍暴露旧 `room.notebook.*`/`room.diary.*` 命令，其中十个 legacy HubCommand 被当前 Agent 明确返回 `room_legacy_surface_removed`。因此“Hub 有 route”不意味着当前端到端 Room surface 可用。

### 4.5 Browser

distribution/runtime 发现并验证外部资产 → BrowserRuntimeContext/Manager → acquire(name) 创建 NodeReplKernel → bootstrap 外部 browser-client → 同 lease repl 串行执行 → reset/release/idle reaper → turn-ended/shutdown。lease 只在进程内，不是重启可恢复 session。

独立 browser-host 的 framed JSON 路由和 pending id 映射服务浏览器扩展；它不是上述 manager 的同名重复实现。仓库未包含 browser-client/service 的 JS 源码，本轮无法证明其内部副作用边界。

### 4.6 确认、断线与恢复

Agent confirmation → Hub pending confirmation → ntfy callback capability URL → allow/deny/timeout → Agent waiter。连接/Hub 重启会影响这条内存等待链；不能假定确认会跨重启可靠恢复。

Hub replay 未 ACK 的 durable envelope；Agent ledger 按 run/request/hash 区分首次/重复/已完成/冲突。重启后的已开始且不能证明完成的执行应报告 unknown，不盲目再执行。Job history 也将上次进程遗留 active Job 标记 UnknownAfterRestart；这不等于查到了真实 OS 进程的最终状态。

## 5. 状态所有权与耐久性

| 数据/资源 | 当前 owner / 存储 | 重启与边界 |
|---|---|---|
| 实际 child、Process/MCP Job | Agent jobs + 私有 jobs.sqlite3 | 历史有 retention/大小上限；恢复 active 为 unknown，不保证副作用回滚 |
| Agent 私有安装/激活状态 | `~/.agentic_gpt/state/agent/<id>/` | 与 workspace 内容分离，部分旧状态迁移；启动时派生 owner |
| 本地 MCP socket | `~/.agentic_gpt/runtime/agent/<id>/mcp.sock` | 0700 parent/0600 socket、同 UID、stale inode guard |
| 可靠 command ledger | `~/.agentic_gpt/transport-runs.jsonl` | append-only command/result 去重；当前不在每-agent 私有根中，需独立明确 retention/并发/耐久边界 |
| 审计 | workspace `.agentic-gpt-audit.jsonl` | 含命令/路径等敏感信息；写入/report 存在 best-effort 路径，不能当不可丢失审计保证 |
| Hub registry/run receipts | hub.sqlite3：agents、notification_endpoints、agent_runs | 单 serve lock；run 24小时保留；不是 Agent Job DB 副本 |
| Hub连接/pending/Job cache/active Room/confirmation/OAuth | HubState 内存 | Hub 重启丢失；SQLite 存在不意味着同步 waiter 或 token 也持久 |
| Room内容 | Agent workspace/room + Git | Hub 不持有内容；文件服务不自动承担记忆检索/上下文组装 |
| Browser lease/kernel | Agent BrowserRuntimeManager 内存/Node child | 进程内命名生命周期，重启不恢复 lease |
| browser-host socket/pending | 独立 host `/tmp/codex-browser-use` | socket 0660，无等价 local MCP 的 peer UID gate；可访问性依部署权限 |
| Android Attention | 本地 Room DB `agentic_attention.db` | OS alarm/notification 是副作用，DB 才是 local item 权威；不是 Hub run 数据库 |

配置、Job history、command ledger、审计、report 各有不同失败与恢复语义。当前没有一个统一且严格的“所有数据可靠持久化”承诺。

## 6. 安全边界的真实强度

- HTTP action key、MCP OAuth、Agent secret、本地 UID、确认 callback token 是不同入口授权，不能用 tool annotation 代替它们。
- `exec::preflight` 是程序/参数路径启发式筛查；sandbox 默认关闭时，不构成 OS 强隔离。显式配置 allow 可覆盖部分 builtin 决策，是可配置语义，不是未经验证就应修改的默认。
- 文件直接操作有自己的路径/revision/锁约束；这不意味着 arbitrary process、已信任 skill、外部 MCP、Browser JavaScript 自动受到相同限制。
- Browser `repl` 的 arbitrary JS 与下游 MCP 是外部信任能力；签名/包hash证明来源，不证明被调用代码的行为安全。
- 同 UID local MCP 不等于跨主机认证；browser-host 0660/socket parent 与共享容器挂载也不等于 owner-only。不得把任一本地桥直接暴露网络。
- Tunnel key 的 secret reference 与 worker authorization token 是不同秘密；前者不放 argv 的设计不能证明后者也不出现在命令行。

## 7. Console、示例与实验的成熟度

Console Gradle 模块为 shared/androidApp/desktopApp/webApp。

- shared 有 Compose 导航、Attention domain/repository/scheduler 端口和 UI；平台 actual 主要提供 Platform 信息，不是完整多平台能力适配。
- Android 的 `AndroidAgenticApp` 组装 Room repository、AndroidAttentionScheduler、state holder、通知/权限/runtime coordinator。UI action → state holder → Room/AlarmManager；Receiver/AlarmActivity → coordinator → DB/notification；boot/app startup → restoreFutureItems。
- Android 的 mock 指数据来源；注入的 scheduler 会产生真实本地通知/闹钟。Hub URL/token 表单只保存在 Compose state，测试按钮无网络实现；Manifest 没有 INTERNET，不存在已接入的 Hub 控制链。
- Desktop/Web main → shared App → AgenticPlaceholderApp。是 UI 壳，不具备 Android Attention 或远端控制 parity。缺失功能是成熟度事实，不自动等于回归缺陷。
- Hub Android notification 注册明确返回 deliveryImplemented=false；这与 Android 本地闹钟不是同一能力。
- `example/agentic-tui-ux-demo` 是独立 workspace 的内存 UX 原型，不写真实配置、不联网。生产 config TUI 有真实验证/提交/secret 写入，Process TUI 走 Unix MCP，不能合并原型状态为生产模型。
- `experimental/chrome-control-poc` 为独立实验；不得当作正式运行时依赖或安全基线。

## 8. 契约、工程与部署

没有一份 schema 同时权威覆盖所有入口：

| 表面 | 当前行为依据 | 静态投影/核验 |
|---|---|---|
| Hub↔Agent wire | protocol 的 Serde 类型 + 双端 dispatch | envelope/hash/重放测试 |
| Agent-local MCP | stdio_server 的 live descriptor、typed decode、runtime validation | fixed surface 与 deterministic corpus |
| Hub Apps MCP | mcp_server 的 rmcp/schemars/router/profile | descriptor/dispatcher/profile 测试 |
| Hub HTTP/Actions | routes/room 的实际 DTO 与 response adapter | openapi/hub.yaml，当前存在具体漂移 |
| 文档与模型预测评估 | matrix/corpus/示例 | evaluator 仅比预测 tool/参数 shape，不执行 runtime |

CI 包含 Rust fmt/check/clippy/test，以及 hub.yaml 的 YAML 解析；解析成功不证明 OpenAPI 语义、响应和 live dispatch 一致。第二份 agents-minimal.yaml 未见主要构建消费路径。

release 使用 cross 构建 x86_64/aarch64 Linux，每个包三个二进制（agent、hub、browser-host），生成 SHA256SUMS。Console、示例、实验不是这个发布链的一部分。Hub 是单节点 SQLite 控制平面，没有多实例 HA 的现状证据。TLS/反向代理、public base URL、WS/SSE、tunnel-client、tmux、bwrap 与外部浏览器资产均是部署约束，重构不能默认删除。

历史 migration/release 文档记录旧版本本身合理；当前 operations 的工具计数、上下文中的 crate 数等与现状冲突，才需要在后续规范维护中收敛。

## 9. 结论

已有架构主干值得保留：**一个本地执行核心，多种入口适配，一个轻量远端控制平面，独立协议与少量专用边界**。主要债务发生在入口到操作的 gate/dispatch、跨端合同投影、资源生命周期/耐久级别和当前/历史/实验状态的界线，而不是缺少新的 Agent Runtime。
