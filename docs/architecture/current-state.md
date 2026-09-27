# 现状架构：受控执行核心与多入口控制面

状态：源码调查快照与全仓架构再评估，2026-09-23；WP1/WP2/WP3/WP4-A/WP-R 的既定边界修复与本轮 Agent Process/Job 配置、Hub projection、Protocol 内部模块、Skill/path、Android local transition 和 release preflight 实现均已落在既有拓扑内，证据仍按边界区分。WP-T 标准未决；Console remote 是未来产品。Rust workspace check、确定性 Job reload regression 与 workspace test（668 passed、1 ignored）已通过；release preflight 与 bounded local parity gate 已在镜像环境通过，但 hosted GitHub publication/cross-build/ARM runtime 仍未执行；`:shared:jvmTest` 已在 17 tasks 下 BUILD SUCCESSFUL，仅覆盖 shared 纯 `AttentionTransitionPolicy` 测试与 common Kotlin compile，不覆盖 Android app/Room/OS；Android app host test/assemble 因 SDK/网络边界、device/emulator 因设备缺失仍待补证。严格 Clippy 的既有债务不纳入 release preflight；外部 publication/tunnel、Browser client/service、Actions importer 仍未验证。目标规则另见 [目标架构](target-architecture.md)，问题判断见 [诊断](diagnosis.md)。文中代码路径均相对仓库根；符号名优先于可能随编辑变化的行号。

## 1. 调查范围和证据等级

本轮覆盖根 `Cargo.toml` 声明的五个 Rust crate 一级模块、独立 `console/` Gradle root 的四个模块及平台源集、OpenAPI、合同 corpus/evaluator、CI/release、部署脚本、实验与 UX 示例。重点追踪入口到副作用、确认/取消、断线恢复、持久化与能力边界；不是逐行安全审计，也不宣称所有平台均已运行验证。Console、OpenAPI、脚本和示例的仓库存在不等于它们进入 Rust 编译图或 Linux release。

证据分为历史调查基线与后续 closure：历史调查来自源码、CodeGraph 定位、Cargo metadata 与解析后的 OpenAPI，完整分域记录位于 `.planning/2026-09-16-architecture-audit/`；当时没有启动真实 Hub/Agent/Android/浏览器/tunnel。WP1/WP2/WP3/WP4-A/WP-R 的完成状态按各自 closure 记录保留；下段列出的运行证据只覆盖所标明的 Agent/Hub 场景，不替代外部组件、生产 tunnel/GitHub、Android/Console 或 ARM release 验证。

WP2 closure 已补充真实 Agent/Hub binary evidence：Local Unix、hidden stdio worker、HTTP bearer 401/有效 session/SSE、process output parity、Skill runs、Normal Room disabled→enabled、policy/limits 与 workspace/path reload health，以及 Hub loopback WebSocket 的 process、Room skills list/search/activate/run（含 invalid-CWD reason）、pending 清零、CLI tmux create/close audit 和 live policy deny；两个临时 driver 均 exit 0。该证据不覆盖 external tunnel/cloud、OAuth provider、Browser JavaScript/service、OS sandbox；WP3 durability/retention 有独立 closure，不能由这组 WP2 场景代替。WP-R 后续 live gate 已通过九个 HTTP/Full MCP、Coordinator reject、active lease/reconnect 和 local/workflow maintenance 场景；这不构成生产 tunnel/GitHub 部署声明。

用户后续确认的需求与部署事实见[已确认决策](decisions.md)：Room 能力需要远端提供；WP-R 已将九个当前 Room operation 收口到远端公共面并完成 legacy surface clean cutover，Hub 未同步不再是当前合同遗漏。实际已有 Neko/container/共享目录与 Unix socket 部署仍是用户报告，不是 WP2 runtime 证据。

历史规模口径（2026-09-16，未按 WP1/WP2 变更重算）：对已跟踪 `.rs/.kt/.kts/.js/.ts/.py/.sh` 文件统计物理行，含注释、测试、配置和脚本，共 148 文件、91,041 行。执行端 69,497；Hub 9,965；protocol 3,223；apply-patch 1,146；browser-host 866；Console 2,868；其余为示例/实验/脚本。文件大只能说明调查优先级，不能单独证明架构错误。

### 1.1 当前覆盖与工作包状态

| 范围 | 当前记录 | 证据边界 |
|---|---|---|
| WP1 | Hub connection/run/waiter owner tuple 与 bounded closure 已完成 | simulated peer/Hub-side 证据不等于完整 Agent restart、transport-ledger 或 external ntfy/provider E2E |
| WP2 | Agent operation/context gate、provenance、config live subset 与 Process/Skill/batch admission snapshot 已完成；确定性 reload regression 已通过 | 不覆盖 external tunnel/cloud、OAuth provider、Browser JS/service 或 OS sandbox；startup supervisor watcher 仍是独立边界 |
| WP3 | durability/retention 与 recovery 分层已完成 | 不把持久化事实当作外部副作用回滚；跨部署/设备恢复仍按边界核验 |
| WP4-A | 当前本地 cross-surface contract/parity gate 与已修复投影已完成 | strict 外部 Actions importer、外部 Apps client 和完整 Protocol lifecycle 未由 WP4-A 证明；本轮 release 的 bounded local live parity 另见 Release 行，不代表 hosted publication |
| WP-R | 九个当前 Room operation 的 Hub→Protocol→Agent clean cutover 已完成，当前 live gate 已通过 | 不构成生产 tunnel/GitHub/外部 Room repository 部署证明 |
| WP4-B | Protocol 已在同一 crate 内按 wire domain 拆为私有模块，`lib.rs` 保留 root facade/re-exports；serde/wire 边界不变 | 仍需保持 dependent-crate/serde 证据与外部 caller 边界；不扩成新 crate |
| Source seams | Process stdio adapter、Hub neutral Job/info projection、Skill lease/digest、共享 path-root normalization、Android transition owner 已实现 | 这些是窄源码边界，不等于所有入口、OS 行为或外部效果均已验证 |
| Release | tag preflight 已接入同一 SHA、Agent/Hub Cargo 版本匹配与 fmt/check/test/build/parity 命令；v0.9.1 bounded local preflight/parity gate 已通过镜像环境 | hosted GitHub publication/cross-build/ARM runtime 未执行；tag mismatch guard 已拒绝 deliberate `v0.0.0`；严格 Clippy debt 不在 preflight |
| Console | Android local attention transition owner、overdue claim 与 payload-aware snooze 已实现；ExactRequired 无 exact 权限时失败而不静默转 inexact，ExactPreferred 的 inexact fallback 持久标为可见 `Degraded`；`:shared:jvmTest` 17-task BUILD SUCCESSFUL 只证明 shared policy/common Kotlin compile；remote Console 是未来产品 | Android app host test/assemble、Room/Android runtime 与 device/emulator 仍未完成（SDK/Maven TLS/设备边界）；根 Rust CI/release 不覆盖 Console Android/桌面/Web |

此表是当前状态索引，不把目标文档的建议目录、规则或未验证边界改写为实现事实。

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

`RuntimeModel` 的 capability 与 `toolsets.enabled` 是不同机制：Hub+Normal 的 skills/bootstrap/Room 能力受既有 Hub profile/toolset/capability 规则限制；Local/Tunnel+Normal 的 namespace gate 默认不含 Room，但显式启用 `toolsets.room` 后 Normal 仍可使用 Room。Agent-local 固定 surface 测试给出 Normal 29、Room 40 工具，实时广告还受 toolsets 过滤。Hub Full/Coordinator 是另一组入口 profile，不要求与 Agent-local 工具集合完全相同；现在 operation gate 已统一真实 ingress/context 的准入，但 namespace/toolset、capability、annotations 仍是不同层次。

`agentic-gpt local` remains an ordinary Unix MCP client (`local:` provenance and
normal MCP gates). `agentic-gpt tmux` is a separate CLI-admin path limited to
`tmux.listSessions`, `tmux.attach`, `tmux.createSession`, and
`tmux.closeSession` (`localadmin:` provenance); MCP `tmux.sessions/panes/exec/pasteText`
are not aliases for those four commands.

## 3. 模块职责地图

以下路径中的 Agent 前缀为 `crates/agentic-gpt/src/`，Hub 前缀为 `crates/agentic-gpt-hub/src/`。两个入口的 `main.rs` 声明 crate-root 模块，实现在下列按 owner 组织的目录；目录迁移不改变 `crate::` 模块名或执行边界。

### 3.1 执行端

| 模块组 | 当前职责与边界 |
|---|---|
| `main.rs`、`runtime/{startup,state,instance_lock,supervisor,tunnel_distribution,agent_info,notify}.rs`、`support/utils.rs` | main 保留模块声明与启动入口；startup 拥有启动组装和热重载，state/instance lock 与 supervisor 拥有运行状态、单实例和 tunnel/worker 进程链 |
| `ingress/{stdio_server,stdio_transport,stdio_schema}.rs` | MCP 调用分派、stdio framing/resume 和纯工具 descriptor/schema 分属同一入口的明确责任；Process Exec/Batch 通过 `local_service::dispatch_process`，其余资源仍可保留 direct branches，annotation 只作描述/发现 |
| `operations/{local_service,operation,operation_result}.rs` | Process shared adapter、HubCommand/value-returning operation 映射、不可变 RequestContext、同步 authorize、共享 error/slim result projection；承接 Hub ingress，但不是 Hub runner，也不反向依赖 stdio transport |
| `ingress/hub.rs`、`storage/transport_ledger.rs` | WS/SSE、Hello/heartbeat、可靠 envelope、ACK/重放、Job/report、确认响应 |
| `ingress/local_control.rs` | Unix listener/client、UID/权限、stale socket inode guard、LocalJobClient |
| `ingress/{http_server,http_oauth}.rs` | worker HTTP MCP、Bearer/PKCE、Host/Origin/resource 边界 |
| `process/jobs.rs`、`storage/job_history.rs` | Process/Skill/MCP 准入、等待、取消、输出上限、终态、SQLite 历史与重启 unknown；Process/Skill admission 捕获 Arc<Config>，batch preflight 将同一 snapshot 传给 queued workers |
| `process/exec.rs`、`operations/{policy,confirmation}.rs` | process preflight/CWD/path、program rule、可选 bwrap、人工确认及临时 MCP allow；`exec::normalize_roots` 是配置 path-root 的共享归一化入口 |
| `files/file_ops.rs` | 文件路径策略、读/搜索、patch 计划、revision/lock/revalidation、暂存提交与审计；复用 `exec::normalize_roots` 并保留 Expansion/Resolution failure stage；调用纯 apply-patch 库 |
| `mcp/{mcp,batch}.rs`、`config/mcp_servers.rs` | 下游 MCP 单次调用/client/Job 衔接与 batch 预检/调度/聚合分属不同生命周期；持久 MCP server 配置及校验归配置 owner，实际 Job 与取消状态仍归 `process/jobs.rs` |
| `tmux/tmux.rs` | 外部 tmux server/session/pane 观察与控制；session 生命周期不等同 Agent child Job |
| `skills/{skills,skill_installs}.rs` | `skills.rs` 拥有 Skill package metadata、`package_sha256` 与 activation/shared lease；`skill_installs.rs` 拥有安装 journal/staging/commit/recovery；skill run 复用 Process Job |
| `room/{bootstrap,room_repository,room_reads,room_maintenance}.rs` | 有界 bootstrap/Room 文件资源、Git/scaffold、语义槽位与维护提交；不是 reasoning loop |
| `browser/{browser_distribution,browser_distribution_verify,browser_runtime,browser_kernel,browser_manager,browser_manual,browser_discovery}.rs` | 独立的仓库签名/包索引验证、获取/解包/缓存/激活、运行时来源发现、Node 子进程和命名 lease 各按自身 owner 保留 |
| `config/{config,config_cli,config_keys,config_templates,mcp_servers}.rs`、`config/setup/`、`ui/config_tui/` | 配置模型/加载、独立的键合同/赋值注册表、CLI init/命令、MCP server 配置及向导 draft/validation/review/commit 分别归相应 owner；磁盘合同不变 |
| `ui/{cli,cli_i18n}.rs`、`ui/tui/` | CLI 命令解析与交互调度、展示/输入/终端恢复、本地化 |
| `storage/{private_state,audit}.rs` | 私有状态目录与迁移、workspace append-only 审计；耐久等级不同于 Job history/transport ledger |

### 3.1.1 Agent operation routing（当前）

当前 Agent operation routing 是**按已收口 family 部分共享，而非一个 universal dispatcher**：`stdio_server::dispatch_with_lifecycle` 的 Process Exec/Batch 进入 `local_service::dispatch_process`，由同一 config admission/gate 与 Job owner 完成；其他资源仍可保留 direct branches，Hub ingress 则进入 `local_service::dispatch`。共享 WP2 `RequestContext`/`authorize` gate 与同一批资源 owner，因此源码没有显示出两套独立 executor；剩余问题是非 Process family 的结构性 `[推断]`，不可写成当前行为故障。后续收敛仍按一个 operation family 一次迁移 caller、descriptor 和 projection，不把仅 Agent-local 的操作强行新增为 Protocol wire command。

### 3.2 Hub

| 模块 | 当前职责 |
|---|---|
| `main.rs`、`runtime/{cli,config,server,state}.rs` | main 保留模块声明与启动入口；CLI/config、HTTP serve/router/后台清理、HubState 分属明确的 runtime owner；server 还组装远程确认回调 |
| `agents/{transport,lifecycle,dispatch,registry}.rs` | Agent secret 接入、WS/SSE、连接替换、Hello、消息处理、受 owner 约束的 dispatch/重放，以及 Agent 注册/alias/secret hash |
| `ingress/http/routes.rs` | action-key HTTP process/Job/tmux/MCP/运行查询及响应适配 |
| `ingress/mcp/{mcp_server,args,transport}.rs`、`support/agentic_result.rs` | Apps 工具 handler/tool router、类型化参数/schema、JSON-RPC framing/auth/Full/Coordinator、业务 JSON → MCP result 各留在 MCP 入口边界 |
| `storage/runs.rs` | command/run 收据、hash/ACK/status/result/conflict、stale 与 retention |
| `room/{http,control}.rs`、`room/room.rs` | HTTP handlers/status projection 与中立 active Room `(agent_id, connection_id)` 租约/路由分离；`room.rs` 只声明子模块并保留跨 owner 测试；Hub 不持有 Room 内容或 run receipt owner |
| `notifications/notify.rs` | freedesktop Agent/ntfy 渠道、健康缓存、Android 注册但未实现 delivery |
| `ingress/oauth.rs` | 授权码/PKCE/token 与 MCP Bearer 校验，session 在内存 |
| `storage/db.rs` | SQLite schema/兼容增列 |
| `runtime/{instance_lock,confirmation}.rs`、`support/utils.rs` | 同 DB serve 锁、确认协调与 ID/hash/constant-time 比较 |

### 3.2.1 混合职责模块的拆分判据

本次只按独立 owner 拆分，不以文件大小、行数或测试数量为目标：

- Agent `main.rs` 中 CLI 命令、启动/热重载、Browser 来源发现分别归 `ui/cli.rs`、`runtime/startup.rs`、`browser/browser_discovery.rs`；stdio resume transport 与纯工具 descriptor/schema 分别归 `ingress/stdio_transport.rs`、`ingress/stdio_schema.rs`。stdio 参数 DTO 仍被 dispatch、validation、conversion 直接消费，未制造第二套入口类型。
- Agent `mcp/mcp.rs` 的 managed single-call/client lifecycle 与 `mcp/batch.rs` 的 batch 预检、整体确认、调度和聚合审计有不同变化原因；共享 client factory 不是共享可变 client，Job/取消权威仍在 `process/jobs.rs`。`config/mcp_servers.rs` 统一持久 MCP server 模型/校验/修改，迁移配置及状态调用方，不在 MCP 执行模块保留转发；配置核心 `config/config.rs` 的通用加载/验证/备份仍同一 owner。
- Agent `config/config_cli.rs` 的 Clap/init 命令流与 `config/config_keys.rs` 的键名/元数据/解析/赋值注册表分离；Browser 的可信仓库元数据验签/Packages 包选择归 `browser/browser_distribution_verify.rs`，HTTP 获取、归档物化、cache 与 lock 仍归 `browser/browser_distribution.rs`。其余大模块只在 owner/lifecycle 连续时保持完整。
- Hub `main.rs` 的 CLI/配置存取与 server/router 各归 `runtime/cli.rs`、`runtime/config.rs`、`runtime/server.rs`；Apps MCP 参数类型/schema 与 JSON-RPC/auth transport 各归 `ingress/mcp/args.rs`、`ingress/mcp/transport.rs`，原 `mcp_server.rs` 保留工具 handler。
- Hub `storage/runs.rs` 的投递、ACK、状态、结果与 retention 共用 SQL/identity 不变量；`runtime/state.rs` 已在 `state::projection` 有中立投影边界。复审后 `room/room.rs` 的 HTTP 合同与 active-room 租约有不同调用方和变化原因，改为 `room/http.rs` 与 `room/control.rs`：共享 `HubState` 并不构成同一生命周期，`RoomRouteError` 保留中立控制语义，连接代际与 run/waiter owner 仍在 `agents`/`runs`。

### 3.2.2 Hub 超过 800 非测试实现行的复审

计数按 `crates/agentic-gpt-hub/src/**/*.rs` 逐文件计算物理行数（保留空行/注释），剔除完整 `#[cfg(test)]` 项及纯测试文件；内联非测试模块也单独核查。800 行仅触发复审，不是拆分指标。迁移前后满足阈值的文件均为：

| Hub 文件（相对 `src/`） | 非测试行 | 结论与 owner |
|---|---:|---|
| `ingress/mcp/args.rs` | 811 | 不拆：类型化 Apps MCP 参数/schema/默认值/转换均为入口 DTO，不能按工具数量推出独立生命周期。 |
| `ingress/mcp/mcp_server.rs` | 1587 | 不拆：已与 JSON-RPC transport、参数模块分离；余下 tool router/profile 与 handler/result projection 同属 Apps MCP 适配器，工具族并无自己的状态 owner。 |

`room/room.rs` 迁移前为 521 非测试行，仍因 HTTP 合同与 active lease 的不同 owner 而拆；迁移后 `room/http.rs` 439、`room/control.rs` 86，`room/room.rs` 仅保留模块声明及跨 owner 测试。不存在超过阈值的内联非测试模块。测试单独审查：`agents/lifecycle_tests.rs` 1285 行已是外置的连接代际测试模块，断连/替换/可靠回执场景属同一 lifecycle，保持完整；`room/room.rs` 的内嵌跨 owner 测试约 402 行，验证 HTTP 映射与租约交互，不据测试长度拆生产代码或删除测试。

### 3.2.3 Agent 超过 800 非测试实现行的逐项复审

对 `crates/agentic-gpt/src/**/*.rs` 逐文件计数：保留空行/注释，跳过完整 `#[cfg(test)]` 项和仅含测试的外置模块，内联非测试模块另查；旧值取迁移前提交，现值取本轮实现。阈值只触发 owner/lifecycle 复审。迁移前 **24** 个文件触发，迁移后 **23** 个；下表列出前后任一时点触发者（路径相对 Agent `src/`，`<800` 表示迁移后低于阈值）：

| 文件 | 旧→现非测试行 | 结论与变化 owner |
|---|---:|---|
| `browser/browser_distribution.rs` | 1921→1488 | 拆出可信仓库验签/包索引；其余获取、cache、物化、锁和激活共享验证后包的生命周期。 |
| `config/config.rs` | 2233→2236 | 保留通用配置模型、加载/import、校验、原子写入；MCP 专属配置已经另归 owner。 |
| `config/config_cli.rs` | 1601→<800 | 键注册表移至 `config_keys.rs`；这里只处理 init/Clap/命令流。 |
| `config/config_keys.rs` | —→1174 | 新 owner：键名、类型、解析、setter 和说明共同定义一个配置键合同，不再拆静态表与赋值规则。 |
| `config/setup/model.rs` | 863→863 | setup state 与转换同一 draft 状态机。 |
| `config/setup/validation.rs` | 1056→1056 | 跨字段校验、MCP draft 转换、错误映射及 InitInput 属同一次 setup 准入。 |
| `files/file_ops.rs` | 1662→1662 | 路径策略、revision/revalidation 与原子文件效果共同约束读、搜索、写和 edit。 |
| `ingress/http_oauth.rs` | 912→912 | OAuth discovery、callback、token 和持久会话同一认证生命周期。 |
| `ingress/hub.rs` | 1208→1208 | Hub transport/命令回传共享 request context 与重连/reporting 规则。 |
| `ingress/stdio_schema.rs` | 886→886 | 一套 stdio 工具 descriptor/schema 与校验合同。 |
| `ingress/stdio_server.rs` | 2380→2380 | 同一 Agent MCP 入口和 dispatch/lifecycle；transport、schema 已各有模块，测试长度单独处理。 |
| `mcp/mcp.rs` | 1569→<800 | 拆出持久配置 owner 与 batch 编排，余下单次调用/client/Job 交互同一运行生命周期。 |
| `operations/confirmation.rs` | 871→871 | provider、pending response、临时 MCP allow 与取消同一确认策略生命周期。 |
| `process/jobs.rs` | 2095→2095 | Process/Skill/MCP 共用 ManagedJob admission、终态、取消与 history owner；不造另一套 executor。 |
| `room/bootstrap.rs` | 807→807 | Room bootstrap 探测、路径安全、扫描预算、哈希/revision 和 warning 同一只读响应。 |
| `room/room_maintenance.rs` | 954→954 | 一次 Room 维护的计划、提交和清理事务。 |
| `room/room_repository.rs` | 1027→1027 | repository 路径/锁/revision 与文件持久化是同一保护边界。 |
| `runtime/supervisor.rs` | 950→950 | worker/tunnel 启停、重启与回收属于一次监督生命周期。 |
| `skills/skill_installs.rs` | 1788→1788 | 安装下载、暂存、提交、租约及恢复是一条安装事务。 |
| `skills/skills.rs` | 807→807 | 发现、包信任、active/lease 与 Skill 运行共享 Skill registry 规则。 |
| `storage/job_history.rs` | 1364→1364 | durable history 的查询、retention、退化/修复共享持久化事务。 |
| `tmux/tmux.rs` | 861→861 | Session/pane 命令共享目标检查、执行与输出合同。 |
| `ui/cli_i18n.rs` | 1001→1001 | 语言词条与查找/render 共同维护 CLI 文案合同。 |
| `ui/config_tui/app.rs` | 2683→2683 | 跨页输入、review/edit/commit 共享 ConfigTuiApp/SetupSession；仅搬方法不产生新 owner。 |
| `ui/config_tui/pages.rs` | 4923→4923 | 页渲染、focus/inspector 与 review 共用 TuiState/SetupSession 投影；单拆 MCP 页面会割裂编辑 controller。 |

新 `mcp/batch.rs` 758 行、`config/mcp_servers.rs` 162 行、`browser/browser_distribution_verify.rs` 446 行，均按职责而非阈值拆出；没有超过阈值的内联非测试模块。测试组织独立审查：`ingress/stdio_server_tests.rs`（原内嵌约 2405 行）、`mcp/mcp_tests.rs`（约 1459 行）、`browser/browser_distribution_tests.rs`（约 1201 行）移为同一父模块下的外置测试，保留私有访问与全部断言；config 与 browser manager 的内嵌测试约 905 行，分别仍围绕配置持久化与 fake-kernel lease 状态，未因大小移动或删除。

### 3.3 Protocol（当前内部组织）

`crates/agentic-gpt-protocol/src/lib.rs` 现在是 root facade，只声明并 re-export 私有 wire-domain modules：`envelopes`、`identity_config`、`mcp`、`notification_tmux`、`process_jobs`、`room`、`skill_bootstrap`。这些模块继续只拥有 wire DTO、枚举、envelope 与纯契约 helper；root public names、serde tags、camelCase、bytes 和五 crate 依赖方向不因内部组织改变。Protocol 是跨 Hub↔Agent wire authority，不是 Agent-local operation 或所有入口的万能 schema。

## 4. 关键调用链与生命周期

### 4.1 Hub 远程执行

`routes::process_exec` 或 `mcp_server::call_app_tool` → Hub agents dispatch/`runs::prepare_run` 持久化 command/hash → pending waiter + envelope → Agent `hub::handle_reliable_envelope` → `transport_ledger::accept`/ACK → `local_service::dispatch`（带真实 Hub `RequestContext`）→ `jobs`/具体能力。Process Exec/Batch 的 stdio ingress 由 `local_service::dispatch_process` 复用同一 Process operation seam；各 ingress 仍保留 framing/auth/error envelope。Hub `state::projection` 提供 neutral Job/info/freshness values，HTTP `routes` 与 Apps `mcp_server` 各自消费并保留边缘投影。没有“Hub runner 调 stdio server”的反向依赖。回传分为三条路径：Response → Hub `runs::store_result` → pending waiter；JobUpdate → Hub Job cache；RunReport → `runs::upsert_agent_report`。后两条不会直接唤醒该同步 waiter。

Hub request/run 是控制面投递与收据身份；Job 是执行端资源生命周期；connection id 是连接代际；boot generation 是执行端进程代际。它们不能互换。同步等待超时不等于取消远端任务，迟到结果可以到达；WP1 已收口 owner 校验，WP3 已分别收口 durability/retention，但任一边界都不替代另一边的证据。

### 4.2 本地与 Standalone

`supervisor::run` → tunnel-client 的 MCP command → `main::run_stdio_worker` 验证 worker authorization → `build_app_state` → stdio/Unix/可选 HTTP 共用 `AgentMcpServer` → `dispatch_with_lifecycle` → 具体能力。

`ResumableStdioTransport` 在 worker 恢复时处理内部 initialize/session 恢复，不能直接替换为“每次请求启动全新进程”。Local 不需要 tunnel；Standalone reporting-only Hub 不能回流远端执行。

### 4.3 Process、Skill、MCP

Process：policy/CWD/preflight → Deny 或 cancellable confirmation → 可选 sandbox command → child/输出 tail/monitor → terminal snapshot/audit/report。Skill 先由 `skills.rs` 校验激活包、脚本路径、package digest 与 shared lease，再进入同一 Process Job。

MCP：server/tool/args 校验与配置快照 → managed Job/批量准入 → confirmation/temporary allow → global/per-server semaphore → HTTP 或 stdio 下游 → 响应/timeout/cancel/detached。batch fail-fast 不回滚已经发生的副作用。配置的 stdio MCP server 与 Browser Node 不是自动由 process bwrap 包裹的子执行器。

**配置 snapshot 当前规则**：修复前 `start_process_job_inner` 在 admission/audit 处捕获 config、而 `run_async_job` 后续从 `state.config` 读取 policy/CWD/preflight，确定性 current-thread regression 在修复前得到 `Rejected` 而预期为 `Completed`。当前 Process 与 Skill admission 捕获一份 `Arc<Config>` 并将其贯穿 worker；batch 在 preflight 捕获一份 snapshot 并传给所有 queued workers。后续新 admission 读取 reload 后的 live config；startup supervisor identity watcher 仍是独立的 startup-derived 资源边界。该 green regression 证明选定 Job 语义，不把它外推为所有 Config 使用点的统一快照。

### 4.4 文件与 Room

File edit：共享 `exec::normalize_roots` 展开并归一化 configured write/read-only/deny roots（重复项去重），`file_ops` 将 `Expansion` 与 `Resolution` 映射为原有 `path_policy_error` stage；随后执行路径/权限/保留路径检查 → patch parse/transform → 排序 path locks/revision → 临时文件 → 必要确认 → 路径与revision再校验 → commit/audit。apply-patch 只负责算法，不得单独绕过文件权限层调用为公共工具。

Room：仓库根/软链接/Git/scaffold 检查 → bounded diary/notebook/state 读取，或 `room_maintenance::submit` 在写锁下执行预定义语义槽位维护。Hub Full/HTTP 通过 captured active Room lease 暴露九个当前语义 operation；Hub 不打开 Room 文件，通用 run receipt 只可保留有界 operation result。历史调查中的旧 `room.notebook.*`/`room.diary.*` 与 `room_legacy_surface_removed` 仅是历史基线；legacy HubCommand 已在 caller/descriptor/OpenAPI/文档迁移后退出当前 contract，不做静默映射。WP-R live gate 已通过，但不构成生产 tunnel/GitHub 部署声明。

### 4.5 Browser

distribution/runtime 发现并验证外部资产 → BrowserRuntimeContext/Manager → acquire(name) 创建 NodeReplKernel → bootstrap 外部 browser-client → 同 lease repl 串行执行 → reset/release/idle reaper → turn-ended/shutdown。lease 只在进程内，不是重启可恢复 session。

独立 browser-host 的 framed JSON 路由和 pending id 映射服务浏览器扩展（`crates/agentic-browser-host/src/lib.rs::run`、`prepare_socket`、`Host::handle_client_message`）；它不是上述 manager 的同名重复实现。仓库未包含 browser-client/service 的 JS 源码，本轮无法证明其内部副作用边界。

#### 4.5.1 外部资源生命周期保证（源码锚点）

下列是当前 checkout 能证明的控制流保证，不是对下游实现或副作用回滚的保证：

- **下游 MCP（`mcp.rs::start_managed_call`、`run_managed_call`、`client`、`close_client`）：** Job 先登记，再经过授权/并发门控；stdio 使用 `TokioChildProcess` 启动 `sh -lc <configured command>`，由 RMCP transport 持有 child 的 stdio；HTTP 则无本地 child。调用结果来自下游 response，超时/取消会发送取消通知并尝试关闭 client；没有观察到终态时会记录 detached/timeout。请求一旦发出，不能据此证明下游副作用已回滚。
- **Browser（`browser_kernel.rs::NodeReplKernel`；`browser_manager.rs::BrowserRuntimeManager`）：** acquire 创建 Node child、bootstrap 外部 browser-client 并登记进程内命名 lease；repl 在 lease lifecycle 下串行调用 JS，reset 执行 turn-ended、JS reset 和重新 bootstrap，release/idle reaper 关闭 kernel 后移除 lease。`browser-client/service` 源码不在仓库，故返回值只能证明调用观察，不证明浏览器副作用回滚；丢失响应表示结果未知，不等于副作用未发生。
- **tmux（`tmux.rs::create_session_at`、`tmux_output`、`close_session_inner`）：** Agent 调用外部 `tmux` server 创建/查询/控制 session；短命 CLI child 只承载命令输出，server/session 不由 Agent child Job 持有。`closeSession` 请求 `kill-session` 并记录 audit，但内存 audit/返回丢失不能证明持久 server 或 pane 中的副作用已停止或回滚。
- **Standalone tunnel（`supervisor.rs::run_loop`、`spawn_tunnel`、`terminate`）：** supervisor 持运行锁，启动 tunnel-client（stdin 为 null、stdout/stderr 供日志读取），由 tunnel-client 再连接/启动隐藏 stdio worker；健康检查就绪后运行，退出/超时按有限退避重启，关闭时对本地 process group 发 TERM/KILL。该 owner 只覆盖本地进程链，不证明远端 tunnel 或已发出的下游操作结果。

因此，启动成功、调用返回、控制面 terminal state 与实际 side effect 是四个不同层次。网络/进程断线导致的 lost response 只能把结果标为 **unknown**；不得把它改写成“调用失败且没有副作用”，也不能把内存 lease/audit 当成 downstream rollback 或 kill persistent server 的证据。

### 4.6 确认、断线与恢复

Agent confirmation → Hub pending confirmation → ntfy callback capability URL → allow/deny/timeout → Agent waiter。连接/Hub 重启会影响这条内存等待链；不能假定确认会跨重启可靠恢复。

Hub replay 未 ACK 的 durable envelope；Agent ledger 按 run/request/hash 区分首次/重复/已完成/冲突。重启后的已开始且不能证明完成的执行应报告 unknown，不盲目再执行。Job history 也将上次进程遗留 active Job 标记 UnknownAfterRestart；这不等于查到了真实 OS 进程的最终状态。

## 5. 状态所有权与耐久性

| 数据/资源 | 当前 owner / 存储 | 重启与边界 |
|---|---|---|
| 实际 child、Process/MCP Job | Agent jobs + 私有 jobs.sqlite3 | 历史有 retention/大小上限；恢复 active 为 unknown，不保证副作用回滚 |
| Agent 私有安装/激活状态 | `~/.agentic_gpt/state/agent/<id>/` | 与 workspace 内容分离，部分旧状态迁移；启动时派生 owner |
| 本地 MCP socket | `~/.agentic_gpt/runtime/agent/<id>/mcp.sock` | 0700 parent/0600 socket、同 UID、stale inode guard |
| 可靠 command ledger | 全局、显式 owner、加锁并同步的 ledger | corrupt raw evidence fail-closed；按阈值 compaction；不因 identity expiry 删除 |
| 审计 | workspace `.agentic-gpt-audit.jsonl` | 单文件 8 MiB + one backup；best-effort 写入/report，不能当不可丢失审计保证 |
| Hub registry/run receipts | hub.sqlite3：agents、notification_endpoints、agent_runs | schema 1 + transaction backups；24 小时只适用于 eligible completed payload；protected hash/unknown/conflict tombstones 保留 |
| Hub连接/pending/Job cache/active Room/confirmation/OAuth | HubState 内存 | cache 上限 4096；metadata TTL 15 分钟/60 秒；Hub 重启丢失内存等待与 token，SQLite 存在不等于同步 waiter 或 token 持久 |
| Room内容 | Agent 配置的 Room repository + Git（默认 workspace/room；`repositoryRoot` 可配置） | Hub 不持有内容；文件服务不自动承担记忆检索/上下文组装 |
| Browser lease/kernel | Agent BrowserRuntimeManager 内存/Node child | 进程内命名生命周期，重启不恢复 lease；JS 返回/审计不证明下游副作用回滚 |
| browser-host socket/pending | 独立 host `/tmp/codex-browser-use` | 当前只读部署检查确认 image/mount、host/Chromium UID/GID `1000:1000`、socket `0660`、同一 socket inode 且无 ACL xattr；Agent service context 为 root `0:0`。精确生产 mode 的 primitive smoke 允许 `1000:1000`、`1001:1000`，拒绝 `1001:1001` (`EACCES`)，并确认 stdin close 清理调用方 socket；这验证 Unix 可达性/清理，不是认证或下游副作用回滚证明 |
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

Console 的四个 Gradle 模块位于独立 `console/` build root；根 Cargo workspace、Rust CI 与 Linux release 不把 Console 编译为 Rust 依赖或打入 Linux archive。shared/common 只提供 UI/domain ports，Android 才拥有真实 local attention side effects；remote Console 仍是独立未来产品。

- shared 有 Compose 导航、Attention domain/repository/scheduler 端口和 UI；平台 actual 主要提供 Platform 信息，不是完整多平台能力适配。
- Android 的 `AndroidAgenticApp` 组装 `AttentionRuntimeCoordinator`（`AttentionTransitionOwner`）、Room repository、AndroidAttentionScheduler、state holder、通知/权限/runtime coordinator。UI、receiver、alarm、boot restore 都通过 transition owner；`AttentionTransitionPolicy` 决定 overdue/future action，Room DAO 以原子 claim 处理 overdue，snooze 传递完整 item payload。`:shared:jvmTest` 的 17-task BUILD SUCCESSFUL 仅覆盖 shared policy/common Kotlin compile，不覆盖 Android app/Room/OS；OS effect 失败/降级仍不等于 Room transition 成功，Android app host/device 证据待补。
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
| Hub HTTP/Actions | routes/room 的实际 DTO 与 response adapter | openapi/hub.yaml 当前已投影九个 WP-R Room operation；live gate 已验证对应 route/dispatch projection |
| 文档与模型预测评估 | matrix/corpus/示例 | evaluator 仅比预测 tool/参数 shape，不执行 runtime |

`.github/workflows/ci.yml` 当前执行 `fmt`、workspace `check`、`clippy`、`test`，构建 Agent/Hub 后安装 `PyYAML`/`jsonschema` 并运行 `scripts/check_contract_parity.py`；该 gate 不等于所有外部 importer、生产 tunnel/cloud、Android/Console 或 Browser service 的验证。`openapi/hub.yaml` 是当前 HTTP artifact；`openapi/agents-minimal.yaml` 明确标为 historical/noncanonical，不是 CI/runtime gate。`docs/tool-contract-matrix.md` 与 `evaluate_tool_contracts.py` 仍是 review/prediction 辅助，不能替代 live gate。

tag-triggered `.github/workflows/release.yml` 现在先 checkout `${{ github.sha }}` 并依赖 `scripts/release-preflight.sh`：tag 必须与 `agentic-gpt`/`agentic-gpt-hub` Cargo versions exact match，再运行 fmt/check/test/build 与 `scripts/check_contract_parity.py`，然后才进入三 binary cross-build/publish。严格 Clippy 的既有 CI debt 明确不属于此 preflight，未借排除来宣称 lint 全绿。v0.0.0 mismatch guard 已按预期拒绝；在可达镜像环境中 v0.9.1 preflight 与 bounded local contract parity gate 已通过（含 Agent local/HTTP、Hub Full/Coordinator、Process/Job/MCP/Room/cache fallback），但这不是 hosted GitHub publication、cross-build、ARM runtime、外部 Actions importer 或 Console 发布证据；Android/Console host/device 行为仍未验证。

历史 migration/release 文档记录旧版本本身合理；WP4-A 前的 Job/OpenAPI drift 例子在诊断、目标和规则中应标为历史基线，不能被当成当前行为失败。当前 operations 的工具计数、上下文中的 crate 数等与现状冲突，才需要在后续规范维护中收敛。

## 9. 结论

已有架构主干值得保留：**一个本地执行核心，多种入口适配，一个轻量远端控制平面，独立协议与少量专用边界**。WP1/WP2/WP3/WP4-A/WP-R 的 bounded repairs 与本轮窄 source seams 不改变这一判断；Protocol 内部模块、Process snapshot、Hub neutral projection、Skill/path owner 与 Android transition 已实现但不扩成新框架。WP-T 标准未决，Console remote 是未来产品。当前剩余边界是非 Process family 的 routing 收敛、跨端/发布/外部验证、Android host/device 证据和 strict Clippy 既有债务，而不是缺少新的 Agent Runtime，也不是已复现的统一行为故障。
