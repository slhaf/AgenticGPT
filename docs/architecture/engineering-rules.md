# 工程规则（规范草案）

> **状态：草案，待维护者和用户确认。** 本文是给 coding agent、reviewer 和维护者的放置/依赖/边界规则，不是已经完成的重构清单。目标模块目录可能尚不存在；若规则与当前代码冲突，先按“现状兼容 + 显式迁移”处理，不要偷偷引入 shim 或权限变化。
>
> 事实引用来自本轮只读调查记录：`.planning/2026-09-16-architecture-audit/execution-survey.md`、`contract-ops-survey.md`、`hub-survey.md`、`hub-supplement.md`、`console-survey.md`、`verification.md`，以及已建立的 CodeGraph 索引。调查中未验证的行为在此标为“待核验”，不得写成通过或已集成。
正式现状和诊断分别见 [现状架构](current-state.md) 与 [问题诊断](diagnosis.md)；目标边界见 [目标架构](target-architecture.md)。诊断编号 A01–A09 是 review 交叉索引，不代表规则已由代码实现。


## 1. 使用方式：先判断“谁拥有事实和副作用”

提交代码前，coding agent 必须按以下顺序回答问题，并把答案写进 PR 描述或相邻架构说明：

1. **这是纯算法、wire DTO、控制面协调、Agent 本地效果、独立进程 bridge，还是 UI/交互？**
2. **哪个进程/模块拥有最终事实和生命周期？** 先找 source of truth，再决定文件位置；不要按关键词或文件大小放置。
3. **调用来自哪个 ingress？** stdio、Local Unix、Standalone tunnel、Agent HTTP、Hub HTTP、Hub Apps MCP、CLI、TUI、Console 和 Browser bridge 不是同一工具面。
4. **操作是否有副作用？** 若涉及 process、文件、网络、MCP、tmux、Browser、Room、Skill、通知、凭证或配置，必须经过相应 policy/capability/confirmation/owner gate。
5. **是否跨进程/跨语言/跨版本？** 先盘点 Protocol、Agent descriptor、Hub schemars、HTTP/OpenAPI、旧客户端和部署文档，再改所有受影响投影。
6. **是否改变权限或生命周期语义？** “重构”不得掩盖 capability 放宽、timeout→cancel、cache→authority、placeholder→integration 或 startup→live reload 变化。

若无法回答 owner、effect、ingress 或兼容性，规则是**暂停新增抽象，补充调查/标注待核验**，不是创建万能 registry、service 或 fallback。

## 2. 放置决策表

### 2.1 快速决策树

```text
纯 patch 解析/文本替换算法（无领域语义和 I/O）？
  └─ 是 → `agentic-apply-patch`（不得 I/O）
其他纯逻辑？
  └─ 是 → 放回所属 domain/module；不要因“纯”就跨 crate
跨进程 wire DTO、serde 名称、run/request/hash、必要纯验证？
  └─ 是 → agentic-gpt-protocol
Agent 本地 process/file/MCP/tmux/Skill/Room/Browser/confirmation 副作用？
  └─ 是 → agentic-gpt operation core/resource adapter
Hub auth/registry/connection/dispatch/confirmation coordination/run receipt/projection？
  └─ 是 → agentic-gpt-hub
浏览器 extension/native messaging bridge 进程？
  └─ 是 → agentic-browser-host（独立协议/权限）
UI state、平台 repository/scheduler/client adapter？
  └─ 是 → Console common/platform source
生产配置/本机 Job 观察 TUI？
  └─ 是 → agentic-gpt config_tui/tui；不要放到 demo
仅 framing、decode、schema、入口认证、错误投影？
  └─ 是 → 对应 ingress adapter；不得在 adapter 复制执行器
```

### 2.2 依赖允许矩阵

| 层/组件 | 允许的编译依赖 | 允许的运行时调用 | 明确禁止 |
|---|---|---|---|
| `agentic-gpt-protocol` | 外部 serde/纯类型依赖；当前无 workspace 内部 crate | 被 Hub 和 Agent 序列化/反序列化 | I/O、DB、网络、spawn、policy、confirmation、业务资源服务 |
| `agentic-apply-patch` | 纯解析/错误/文本依赖；当前无 workspace 内部 crate | 被 Agent `file_ops` 调用纯 parse/compute/apply | filesystem、Git、path policy、secret、audit、confirmation |
| `agentic-gpt` ingress | 同 crate ingress support 和 Protocol DTOs | 通过 operation core + authorization/confirmation gate 调用本地操作 | 直接依赖 apply-patch/resource executor；调用另一个 transport 的 DTO/loop；绕过 gate 直接 spawn/写文件/下游调用 |
| `agentic-gpt` operation/execution | 同 crate policy/jobs/resources/durability | 拥有 Agent 本地效果和生命周期 | Hub DB/Hub registry、Console state、Browser-host 内部状态 |
| `agentic-gpt-hub` ingress/control | Hub 内部模块、Protocol request/response 类型 | auth、registry、connection、dispatch、receipt、projection；通过 protocol 投递 Agent | spawn shell、打开 Agent 文件、直接 tmux/MCP/Browser/Room I/O |
| `agentic-browser-host` | 进程内 bridge 依赖；无 workspace Agent crate | stdin/stdout + 本地 framed socket ↔ extension | 读取 Agent state、共享 BrowserManager lease、远程暴露未认证 bridge |
| Console common | common UI/state/port 依赖 | 消费真实 platform adapter 或远程 status projection | 直接使用 Android Room/AlarmManager、直接放 API secret、假定 Hub 已连接 |
| Console platform app | 平台 SDK + common | Android local attention 或明确的 Hub client adapter | 将本地 OS side effect 当远端 authority、把 placeholder 当能力 |
| TUI/demo | TUI UI 库；生产 TUI 可调用 Local Unix client | 观察/交互/配置 commit | Demo 运行时事实、第二 Job executor、未经同一 gate 的 consequential action |

当前 Cargo 事实是：`agentic-gpt → protocol + apply-patch`，`agentic-gpt-hub → protocol`，Protocol/apply-patch/browser-host 无 workspace 内部 crate 依赖。Console 当前不编译依赖 Rust protocol 或网络实现。目录迁移不得改变这条方向，除非有独立 compile/deploy/security 论证。

## 3. 分层规则：适配器薄，操作核心唯一，资源 owner 明确

### R-01：入口适配器只处理入口问题

入口适配器可以做 framing、decode、认证、profile/toolset 可见性、connection generation、参数 schema 和错误 projection；然后生成带 `RequestContext` 的 operation request。它不得自己实现另一套 policy、Job、文件提交、MCP client、Room 写入或 Browser lifecycle。

适用入口：

- Agent `stdio_server.rs` 的 stdio/MCP；
- Agent `local_control.rs` 的 local Unix；
- Agent `http_server.rs`/`http_oauth.rs` 的 worker HTTP；
- Agent `hub.rs` 的 Hub WS/SSE client；
- Hub `routes.rs` 的 Actions/HTTP；
- Hub `mcp_server.rs` 的 Apps MCP；
- CLI、TUI、Console 平台 client、独立 Browser host。

入口认证是 perimeter gate，不等于 operation authorization；operation core 仍必须根据 mode/profile/toolset/effect/confirmation 判断。

### R-02：共享 operation core，不复制执行器

Local、Standalone 和 Hub agent 当前复用真正的 `jobs`、`exec`、`policy`、`confirmation`、`file_ops`、`mcp`、`tmux`、Room/Skill/Browser 实现；差异来自 `RuntimeModel`、Transport、profile 和 capability。新增入口应接入共享 operation layer，而不是新增 `run_*_again`。

`stdio_server::dispatch_with_lifecycle` 与 `local_service::dispatch_inner` 的现状重复是映射/metadata/gate 漂移问题，不是允许增加第三套执行器的理由。优先收敛 value/error operation；删除旧分支必须在所有 caller、descriptor、HTTP/MCP projection 和回归验证迁移后进行。

### R-03：共享窄语义，不制造万能 schema/registry

可以共享：

- 稳定 operation identity；
- owner tuple（principal、agent、connection、run、request、hash、job）；
- effect/trust 分类；
- capability/approval decision；
- lifecycle 状态与 error code 语义。

不可因此强迫以下 authority 使用同一输入/输出 schema：

1. Protocol `lib.rs`：Hub↔Agent wire authority；
2. Agent `stdio_server.rs`：Agent-local live toolset、descriptor、conditional validation authority；
3. Hub `mcp_server.rs`：Apps `/mcp` rmcp/schemars authority；
4. `routes.rs` + `openapi/hub.yaml`：HTTP/Actions DTO 与其静态 contract authority；
5. `docs/tool-contract-matrix.md`：review aid，不是 runtime authority。

每项跨表面变化必须经过 parity gate（见第 8 节），而不是依赖一个“万能 registry”声称自动一致。

### R-04：资源副作用只能由资源 owner 执行

- Agent host 执行 process/file/downstream MCP/tmux/Skill/Room/Browser local effects。
- Hub 只记录、协调、投递和投影，不执行 Agent host effects。
- Room 文件/Git 由 Agent Room repository 所有；Hub active Room 只是 lease。
- Android attention item 由 Android Room 所有；AlarmManager/Notification 是 side effect。
- BrowserManager lease 由 Agent 进程所有；browser-host bridge route 由独立 host 所有。
- Console UI state、Hub cache、TUI rows 都是 projection，除非明确声明为 source of truth。

## 4. 身份、生命周期与并发规则

### R-05：ID 不能互换

跨进程请求应保留可验证的 owner tuple：

```text
principal/auth
+ agent_id
+ connection_id / boot_generation
+ run_id
+ request_id
+ command_hash
+ optional job_id
+ ingress/profile/toolset/effect
```

- `agent_id` 识别执行端；`connection_id` 识别连接代际。
- `run_id` 是 Hub 控制面收据；`job_id` 是 Agent effect lifecycle。
- `request_id` 是 request/response rendezvous，不是权限凭证。
- `event_id` 是事件去重/传输记录，不是用户操作 owner。
- Room lease、Browser lease 是资源租约，不能填入 run/job 字段。

Hub pending waiter 的 value 必须能验证 agent/run/request/hash；Response 只有在 `store_result` 为 matching/idempotent 时才允许唤醒对应 waiter。无 run id 的 legacy fallback 只能在明确版本兼容模式内使用，不得用于新的受控执行。

### R-06：连接代际分流可靠和非可靠消息

- 非可靠入站（如当前连接状态、Heartbeat、JobUpdate、RunReport 等）必须绑定当前 connection generation；stale connection 不能更新当前 metadata/cache。
- 可靠 Response/ACK/status 若允许旧代际补交，必须匹配既有 `(agent, run, request, command_hash)`，且不得借此更新连接角色、last-seen 或无关 Job projection。
- Hello/role/mode/boot generation 变更必须使用一致的 current-connection 校验；不能只在 SSE 路径保护，遗漏 WS reader。
- ReportingOnly connection 不可作为执行目标；其报告仍需 owner/status transition 校验。

这些是目标规则，不是声称当前所有路径都已满足；现有调查明确指出 WS stale inbound、Response waiter 和 report identity 仍需验证/收敛。

### R-07：等待、运行、取消、未知分开

任何新增 API/字段/状态必须说明：

- dispatch 是否已送出（`not_sent` / `sent_unknown` / `acked`）；
- Agent 是否 accepted/started/running/completed；
- caller wait 是否超时（`wait_expired`）；
- 是否发出 cancel、是否确认终止、是否有 evidence；
- 重启/断线后是否 `unknown_after_restart` 或 `remote_unknown`。

`timeout` 在此仅表示 caller wait 或控制面等待预算结束，不能自动发送 cancel、改写远端 Job terminal state，或向用户承诺副作用不存在。资源自身若声明独立 execution deadline，可以按其契约触发终止；该终止必须有独立状态、结果和 evidence。`cancel_requested` 也不等于 `cancelled`。

### R-08：重复、迟到、冲突结果不能覆盖事实

- 相同 owner tuple + 相同 hash 的重复结果可以幂等接受。
- 相同 identity + 不同 hash 必须进入 conflict/error/audit，不能覆盖既有 result。
- foreign agent/run/request、缺失 owner、未知 status transition 的结果不能消费 waiter。
- Agent ledger 的 `First/Duplicate/Completed/HashMismatch` 语义和 Hub receipt 必须保持双侧一致。
- Hub cache、Console status、TUI projection 遇到旧 report 只能单调更新或标 stale，不得回退终态。

## 5. 数据权威、缓存和耐久性规则

### R-09：先声明 source of truth

新增字段/缓存/数据库表时必须在代码或 PR 中标出 owner、retention、重启行为和 projection：

| 数据 | Source of truth | 允许 projection | 禁止误称 |
|---|---|---|---|
| Agent policy/profile/roots | Agent validated config + AppState | Hub capability summary、Console capability state | Hub registry 不是本地 authorization |
| Agent running Job | Agent managed Job runtime | Agent `job_history`、Hub run/report、TUI/Console status | Hub cache 不是本机执行事实 |
| Hub dispatch receipt | Hub SQLite `agent_runs` | HTTP/MCP run response、运维摘要 | receipt 不是副作用证明 |
| reliable transport | Agent transport ledger + Hub receipt | replay/pending | request id 单独不是 owner |
| Room content/Git | Agent Room repository | Hub active lease、bounded response | Hub/Console memory 不是 Room authority |
| Android attention | Android Room | Alarm/Notification/UI | OS alarm 不是数据库，Hub item 不是 LocalMock |
| Browser lease | Agent BrowserManager；bridge route 若启用则 browser-host | audit/result | 两个进程的 map 不自动一致 |
| OAuth/confirmation/Hub cache | 明确标为 Hub ephemeral session/projection | status/error | 不承诺 Hub restart 可恢复 |
| audit/report | 各自 evidence durability | logs/summary | telemetry 不等于 command outcome |

### R-10：耐久性等级必须写出来

当前调查显示 Agent job history、transport ledger、workspace audit、Hub receipt 和 reporting channel 的 retention、lock、fsync、大小上限并不相同。新增代码不得把这些统称“已持久化”：

- 需要可靠恢复/幂等的 command receipt 使用明确的 atomic/lock/hash/replay 设计；
- Job history 的终态、输出尾部、cancel evidence 遵守 retention/size budget；
- audit/report 可以是 best-effort，但必须记录丢失语义，不能作为安全批准或副作用证明；
- command/result/args/CWD 可能包含秘密，日志、Hub receipt、Console projection 需采用最小字段、redaction 和 retention；
- config、ledger、audit 变更需说明 crash/partial write/rotation/corrupt recovery，不可只 `append` 后宣称 durable。

## 6. 能力示例：正确放置与禁止跨越

### R-11：Process、Job、terminal

**正确放置**：

```text
Hub HTTP/Apps MCP/Local/stdio/CLI
  → ingress decode/auth/profile
  → operation authorization + confirmation
  → jobs::admission
  → policy + CWD/preflight + optional sandbox
  → exec/spawn/monitor
  → terminal state/history/audit/report
```

当前 `jobs.rs`、`exec.rs`、`policy.rs`、`confirmation.rs` 是实现定位；Hub `routes.rs`/`mcp_server.rs` 只投递/等待/投影，TUI 只观察。

**必须遵守**：

- `policy.rs` 判断，`exec.rs` 执行；不要在 Hub、stdio descriptor 或 TUI 复制 program allow/deny。
- path preflight 是启发式筛查，不是 OS containment；`bwrap` 当前由配置开启且默认关闭，不能把 roots 检查写成强 sandbox 保证。
- MCP stdio、Browser JS、tmux、tunnel child 具有独立 external effect/trust；不要用它们的 annotation 或调用成功伪装 core sandbox 已覆盖。
- CLI `tmux create/close` 等现有绕过 AppState/gate 的路径应作为边界债务处理；新代码禁止复制。

### R-12：File edit 与纯 apply-patch

**正确放置**：

- `agentic-apply-patch`：parse、normalize、compute/apply replacements 的纯算法。
- Agent `file_ops`：canonical path、root/reserved policy、symlink、revision、sorted lock、temporary staging、confirmation、revalidation、commit、audit。

**示例规则**：

```text
file request
  → resolve/canonicalize + write-root/reserved/symlink check
  → agentic_apply_patch::parse_patch/apply_update
  → lock + revision check
  → stage temporary result
  → optional confirmation
  → revalidate + commit + audit
```

patch 算法返回 success 不表示文件已经提交；文件 commit 成功也不表示 Hub run 已完成。不得让 patch crate 读取 workspace、绕过 revision 或执行 Git。

### R-13：MCP 与下游网络

MCP 必须分开看四个公共投影和一个下游执行域：

1. Agent-local stdio/Local/worker HTTP MCP：Agent descriptor、toolset、typed validation 和 operation gate；
2. Hub Apps `/mcp`：Hub rmcp/schemars、Full/Coordinator profile、Hub native/forwarding；
3. Hub HTTP Actions/OpenAPI：HTTP DTO/response adapter 和静态 contract；
4. Protocol WS：Hub↔Agent wire DTO/envelope；
5. Agent `mcp.rs`：下游 HTTP/stdio provider client、managed Job、confirmation/concurrency/cancel。

**禁止**：

- Hub MCP 直接 import Agent `mcp.rs` 或打开 provider；
- 把 Protocol response 直接当 HTTP/OpenAPI response；
- 认为 `read_only/destructive/open_world` annotation 自动授权；
- 让下游 MCP stdio command 跳过其 trusted external effect、secret、timeout、cancel 和 audit 说明；
- 在一面改 tool name/default/required/error，而不更新另一面或版本化。

批量 MCP 的 fail-fast 只阻止尚未启动 child；已发生的 side effect 不因聚合错误自动回滚。

### R-14：Browser 与 Browser host

Agent Browser：

- `browser_distribution.rs` 负责固定来源、PGP/hash/包路径 provenance；
- `browser_runtime.rs` 负责 descriptor/cache/desktop registry discovery；
- `browser_kernel.rs` 负责 Node REPL lifecycle；
- `browser_manager.rs` 负责进程内 named lease、串行调用、reset/reaper；
- `browser_manual.rs` 仅做 descriptor 派生 docs root 的 bounded read/search。

`browser.repl` arbitrary JavaScript 是显式 open-world/external effect。新增 Browser action 必须声明代码来源、权限、confirmation、网络/文件/child-process trust、lease owner 和结果审计，不能只添加一个 tool descriptor。

`agentic-browser-host` 是独立 extension bridge：`stdin/stdout` 和 `/tmp/codex-browser-use` framed socket 的 client/pending route 不得与 BrowserManager/kernel 共享状态。当前 host socket mode 0660、未见 peer UID 检查，不能与 Agent local socket 的 parent 0700/socket 0600 相提并论。正式 peer credential/token/owner-only 方案确认前，禁止远程暴露、共享不受控 volume，且将同组/容器进程视为额外威胁面。

### R-15：Room 与文件文档

Room 代码必须按资源 owner 放置：

- `room_repository.rs`：repository-relative path、root、symlink、schema/scaffold、Git readiness；
- `room_reads.rs`：bounded Diary/Notebook/State read；
- `room_maintenance.rs`：semantic slot、预期变更、worktree/tar preflight、受控写入/提交；
- Hub `room.rs`：active `(agent_id, connection_id)` lease、转发和 HTTP/MCP error projection。

**Room 规则**：

- Hub 不读写 Room 文件，不把 Notebook/Diary 内容放入长期 Hub memory。
- Room 文档是受控资源，不是 context manager、memory store 或 reasoning loop。
- 当前 Hub Full legacy `RoomNotebook*`/`RoomDiary*` 可被发现/dispatch，但 Agent 对这些命令返回 `room_legacy_surface_removed`；不得把 fake routing test 或旧 descriptor 当真实 live parity。
- 迁移到远端的 Room 能力前，先由维护者明确要保留的远端操作合同；仅为该合同定义版本化 adapter、字段/错误/权限和真实跨进程验证，其他当前 Agent Room 工具可以继续 Agent-local。不添加永久透明 alias。
- Room `selectExact`、append 等 OpenAPI/Protocol 字段若不一致，必须显式改 HTTP adapter 或版本 contract，不能在 Protocol 中悄悄吸收 Actions legacy shape。

### R-16：Console 与 Android attention

**当前事实边界**：

- Android `AndroidAgenticApp` 组装 Room attention repository、Android scheduler、notification/runtime coordinator；Android Room 是 local attention source of truth。
- AlarmManager、Notification、boot receiver 和 action receiver 是 OS side effects/recovery path，不是另一数据库。
- `AttentionSourceKind.Hub` 只是模型预留；没有 Hub producer/client/ack lifecycle。
- `HubConnectionCard`/Android settings 的连接字段和测试按钮是 placeholder；`console/shared` 没有网络 client、Bearer、Hub protocol，`AndroidManifest.xml` 未声明 `INTERNET` 权限。
- Desktop/Web 当前启动 UI shell，不拥有 Android attention/Hub parity。

**未来新增 Hub client 的正确放置**：

```text
commonMain: stable remote state + client/repository port
Android/Desktop/Web: platform HTTP/WS/SSE client + secure token storage
Hub: existing auth/route/receipt/agent semantics
```

必须区分 local attention 与 remote run/job：不得将 Hub item 写成 `LocalMock`、将 token field 接上就显示 connected、或把 Android scheduler 当远端 acknowledgement。Android token 需要 Keystore/平台安全存储、掩码、TLS/cleartext/WS lifecycle 和明确错误状态；Web 平台另行处理 CORS。上述能力未落地前不得宣称 Console Hub integration。

### R-17：TUI、CLI 与 Demo

- 生产 `crates/agentic-gpt/src/tui` 的 Process screen 使用 Local Unix `job.list` 做观察；如果新增 cancel/create，必须走与其他 ingress 相同的 operation gate，不在 TUI 自己 spawn。
- `config_tui` 的 draft/validation/review/commit 是配置和 secret adapter；secret review 必须保持 redaction、0600/atomic/rollback 语义。
- `example/agentic-tui-ux-demo` 是独立 workspace 的本地内存视觉原型；不得将静态 Hub/process rows、demo state 或测试当作生产 API/运行事实。
- CLI mode 选择、supervisor、local socket、Hub reporting 的身份/lock 不得因 TUI 复用而混淆。

## 7. 配置、秘密和安全门槛

### R-18：权限变化必须显式比较

任何改动都要在 review 中回答：

```text
旧入口/主体/资源/操作/effect/confirmation
→ 新入口/主体/资源/操作/effect/confirmation
```

禁止以下做法：

- 用 Hub API key/OAuth token 代替 Agent per-agent secret；
- 认为 Hub registry capability 摘要可以绕过 Agent local policy；
- 让 annotations、tool name、UI enabled 状态代替 authorization；
- 将 `ReportingOnly` 改成可执行而不版本化/审计；
- 因 compatibility 添加能放宽 Room、Browser、MCP 或 process 权限的 alias。

### R-19：Secrets 不得进入错误边界

- Hub API key、Agent secret、OAuth token、confirmation callback token、worker token、tunnel key 是不同秘密和不同 owner；不得混淆。
- Standalone worker token 不得新增到命令行 argv；tunnel key 不因 worker token 的现状调查而被错误描述为同一泄露路径。
- 优先 `env:`/`file:` secret reference，secret 文件 parent 0700/file 0600、atomic/rollback；禁止在日志、error、TUI、Hub receipt、Console projection 复述原文。
- 当前调查显示 API key 走环境、worker token 曾嵌入 tunnel child command；后续修复需在 supervisor/tunnel 兼容边界单独讨论，不可声称已经解决。
- Browser JS/MCP provider credentials 需按外部 adapter owner 处理，不因 `NODE_REPL_TRUSTED_*` 或 MCP config 存在就视为 authorization。

### R-20：本地 socket 和 Browser bridge 分级

Agent Local MCP：parent 0700、socket 0600、同 UID/peer UID、stale inode/device guard；新 local socket 不得降低这些条件。

Browser host：当前 `/tmp/codex-browser-use` socket 0660 且没有同等 peer UID 检查。除非正式安全决策选择 owner-only/peer credential/token 并实现部署验证，规则是本地 trusted deployment、不可远程暴露；共享 volume、同组用户和容器 namespace 不得默认视为可信。

### R-21：路径、symlink、OS effect

- 文件资源先 canonicalize，再检查 workspace/write roots、reserved paths、symlink 和 revision；写前后均 revalidate。
- generic process 的 path-looking argument 检查只是 preflight；环境变量、间接路径、任意脚本、网络和 inherited environment 不能被描述成受 roots 完整约束。
- MCP stdio、Browser Node/JS、tmux server、tunnel child、Git/Room maintenance 都要声明 owner、effect 和 trust；不使用空泛“sandboxed”标签。
- 新增 network call 需说明 DNS/redirect/rebinding、TLS、Host/Origin、timeout/cancel、secret、返回值大小和 audit；不把一次连接成功当作安全证明。

### R-22：配置 live reload 与 startup identity 分离

配置字段分为：

- **startup-only**：agent identity、workspace root、private state/job history path、Browser descriptor/runtime root、Hub identity、listen/socket/transport ownership；变化要求 restart 或完整资源重建。
- **live-safe**：经审核的 policy/path limits、MCP/toolset/HTTP 选项；只在原子更新、旧请求语义和权限不放宽时热载入。

不得 whole-config reload 后留下旧 private state、history、Browser manager、transport sender 与新 identity/root 不一致。无法证明 live-safe 的字段默认归 startup-only。

## 8. 协议、合同和兼容规则

### R-23：先找 authority，再改 contract

改字段/工具/命令前记录其 authority：

| Surface | Authority | Review 必查 |
|---|---|---|
| Hub↔Agent WS/SSE | `agentic-gpt-protocol/src/lib.rs` | serde/camelCase、hash/id、状态、replay、owner |
| Agent-local MCP | `agentic-gpt/src/stdio_server.rs` | live toolset、descriptor、required/default/bounds、conditional validation |
| Hub Apps MCP | `agentic-gpt-hub/src/mcp_server.rs` | rmcp/schemars、Full/Coordinator、native/forwarded result |
| HTTP Actions | `routes.rs` 的实际 DTO/response adapter + `openapi/hub.yaml` | optionality、slim response、分页、error/status、import compatibility |
| Docs/matrix/evaluator | 明确为 review/probe | 不把文档或宽松 prediction probe 当 runtime proof |

例如，Protocol `JobInfo.started_at` 可选、Agent slim list 可能有 `nextCursor`、实际 `job.get`/cancel response 与旧 OpenAPI 字段不同；应做显式 HTTP adapter 或更新已版本化 contract，不能因“Protocol 有类型”直接宣称 parity。Room `selectExact` 的 year/month/day Actions shape 与 Protocol `date` 也必须在 HTTP 边界适配或版本化。

### R-24：多入口 parity gate 是必需的

工具/命令变更至少比较：

1. name/camelCase；
2. required/default/bounds/unknown fields；
3. request/response optionality、slim/pagination；
4. Full/Coordinator、Normal/Room、ReportingOnly、toolset visibility；
5. effect annotation 与真实 authorization；
6. wait/cancel/replay/late/error lifecycle；
7. HTTP status/JSON-RPC error/wire message；
8. version、deployment、旧客户端迁移和回退。

Parity gate 可以输出每个 surface 的差异，不要求所有 surface 共享同一 schema；但差异必须是有意、可版本化、可测试的。只跑 YAML parse、只读 docs matrix、只比较 prediction tool/arguments 都不能证明 runtime parity。

### R-25：公开 legacy 只能显式迁移

- 旧 tool/command 若继续支持，记录版本、消费者、权限、错误和 sunset；新能力不能隐式注入旧版本。
- 删除 alias 前迁移所有 caller、descriptor、OpenAPI、Protocol variant、docs 和 release note；没有 owner 的 alias 不得永久保留。
- 不新增“兼容 shim”来吞掉 `room_legacy_surface_removed` 或将 legacy HTTP shape 静默重写为当前 shape；使用显式 versioned adapter。
- 历史版本号/工具计数不是自动的 runtime bug；要修的是当前运维指引、live descriptor、OpenAPI、release artifact 的一致性。不能只因旧文档数字而改变协议。

## 9. 变更审查清单

Reviewer 和 coding agent 在合并前逐项标记 `是/否/不适用 + 证据或待核验`：

### A. 放置与依赖

- [ ] 是否选对 owner（Agent/Hub/Protocol/apply-patch/browser-host/Console/TUI）？
- [ ] 是否保持 `Hub → protocol`、`Agent → protocol + apply-patch` 的编译方向？
- [ ] 是否避免 Hub→Agent crate、Browser host→Agent state、Console→Rust runtime 的隐式编译依赖？
- [ ] 是否在现有 crate 内先收敛模块，而不是先造 service/crate/万能 registry？
- [ ] 入口代码是否只做 framing/auth/schema/error projection，没有复制 executor？

### B. 所有权与生命周期

- [ ] 是否写明 source of truth、projection/cache、retention、重启和 corruption 行为？
- [ ] 是否区分 principal、agent/connection、run/request/event/job、Room/Browser lease？
- [ ] 是否绑定 owner tuple 并验证 stale/duplicate/foreign result？
- [ ] wait timeout 是否仍与 remote cancel、unknown、terminal evidence 分开？
- [ ] 是否保持单一 Agent execution core，而不是 Local/Standalone/Hub 三套实现？

### C. 安全与权限

- [ ] 是否列出授权前后差异，确保权限不放宽？
- [ ] 是否经过正确 policy/capability/confirmation gate，而不是依赖 annotation/UI/tool list？
- [ ] process/path 是否区分 preflight、sandbox、external trusted effect？
- [ ] file 是否有 canonical/symlink/root/revision/TOCTOU/lock/stage/revalidate？
- [ ] MCP/Browser/tmux/tunnel child 的 provider、JS、network、credential trust 是否显式？
- [ ] socket、HTTP/OAuth、Agent secret、confirmation token、worker token、tunnel key 是否分级并按 owner 保护？
- [ ] 日志、audit、Hub receipt、Console projection 是否最小化/脱敏/有界？

### D. Contract 与兼容

- [ ] 是否先定位每个 surface 的 authority，而没有把 Protocol 当万能 schema？
- [ ] 是否比较 name、camelCase、defaults、optional、bounds、response/status/error/lifecycle？
- [ ] 是否更新或版本化 HTTP/OpenAPI、Hub MCP、Agent descriptor、Protocol、docs、cases/验证入口？
- [ ] 是否明确 legacy caller、deprecation、迁移、回退；没有永久 alias/shim？
- [ ] 是否将 Console placeholder、`unavailable`、`cached/stale` 与真实 integration 分开？

### E. 部署与文档

- [ ] startup-only/live-safe config 是否分开；是否需要 restart？
- [ ] 是否检查 local socket、Browser host、TLS/proxy/public base、CORS/Origin、文件权限和容器/共享 volume 信任？
- [ ] 是否更新 `docs/architecture/` 中受影响的职责/依赖/所有权，而没有把目标写成现状？
- [ ] 是否记录未调查盲点和所需真实跨进程/设备/发布验证？
- [ ] 是否只声称实际运行过的验证；未运行 build/test/smoke 不得写成通过？

## 10. 常见错误与替代写法

| 错误想法 | 应改为 |
|---|---|
| “Hub 有 MCP，所以 Hub 可以直接执行文件/Browser。” | Hub 只 auth/dispatch/receipt；Agent 才执行，Browser host 另有 bridge。 |
| “Protocol 有 DTO，所以 OpenAPI/Agent MCP 必须复制它。” | 每个 surface 有 authority；共享窄 identity/effect 语义，做 parity gate 和版本 adapter。 |
| “HTTP 等待超时就是 Job 已取消。” | caller wait timeout 只结束等待；资源 execution deadline/取消是独立状态与 evidence。 |
| “Android 有 Hub URL/token field，所以 Console 已连接。” | 当前是 placeholder；连接需真实 client、auth、平台权限、secure storage、state 和 smoke。 |
| “Room 名字像 memory，所以应搬到 Hub context store。” | Room 是 Agent-owned 受控文件/文档；禁止借名扩展长期 memory。 |
| “browser-host 与 BrowserManager 都叫 Browser，可以合并。” | 一个是 extension bridge，一个是 Agent Node/lease runtime；保持进程和安全边界。 |
| “工具 annotation 标记 destructive，所以它已经被保护。” | annotation 只供消费者；authorization 必须经过 operation gate。 |
| “目录太大，应先拆十个 crate。” | 先在现有 crate 内按 §3 切逻辑模块；只有隔离/发布/编译需要才拆。 |
| “旧版本号/计数错了，所以立即改协议或加 alias。” | 先盘点当前 authority 和运维文档；显式版本迁移，勿隐式 shim。 |

## 11. 实施前置门槛与待核验项

以下门槛属于草案，未通过前不能将目标段落当作落地证明：

1. Hub↔Agent 真实 WS/SSE 断线、重连、旧连接、late/duplicate/mismatched Response、run receipt 与 Agent ledger 的跨进程验证。
2. Agent-local 29/40 surface、Hub Full/Coordinator、HTTP/OpenAPI、Protocol wire 的 descriptor/schema/response/lifecycle parity 检查；不是单一万能 schema。
3. Room 选定远端合同与 versioned adapter 的真实 HTTP/Apps/WS E2E；未选定的 Agent-local 工具不以“必须远端化”为验收条件。
4. process sandbox/preflight、MCP stdio、Browser JS、tmux、tunnel child 的 effect/trust/confirmation 审查；不因开启新默认而放宽权限。
5. Browser host socket peer/auth、Neko/shared-volume/container 部署和不可远程暴露的实际检查。
6. Console Android local-only attention 与未来 Hub remote projection 的分离；真实 TLS/token storage/WS-SSE/client capability 之后才能改变 placeholder 状态。
7. config startup/live reload、atomic persistence、history/ledger/audit/report retention、secret projection 和 release contract/artifact checks 的明确验收；provenance/signing 是另行决定的发布专题。

这些是后续实现和主线核验入口，不是本次文档已通过的测试。本轮运行了 Cargo metadata 和静态合同检查，但未运行 Cargo/Gradle 构建或测试、lint/formatter、真实 Hub/Agent/Android/Browser/tunnel、ARM release 或外部 Actions importer，因此不作相应通过声明。

## 12. 保留项与不采用的做法

保留项和拒绝替代方案的统一理由见 [目标架构 §8](target-architecture.md#8-保留项与拒绝的替代方案)。本规则不再复制第二份清单；若工程规则与目标边界发生冲突，必须先更新目标文档并标注草案状态，再修改代码。

