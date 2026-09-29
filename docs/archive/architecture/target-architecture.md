# 目标架构（技术草案）

> **状态：技术草案，受已确认的 D01–D08 约束。** 本文描述目标边界和迁移方向，不表示代码已经按此组织，也不得据此直接删除现有接口或改变权限、协议、部署方式。下文的目标逻辑模块、建议目录和 `OperationRequest` 等记号只是示意性提案；只有明确附有当前源码或闭环证据的段落才陈述现状。
>
> 适用决策： [D01](decisions.md#d01--room-是受控资源不是-agent-记忆运行时) · [D02](decisions.md#d02--room-远端能力必须补齐) · [D03](decisions.md#d03--一次升级明确迁移不刻意维持旧兼容) · [D04](decisions.md#d04--自用部署背景与具体副作用控制并存) · [D05](decisions.md#d05--browser-host-先盘点实际拓扑再收紧机制) · [D06](decisions.md#d06--持久化按正确性与用途分层) · [D07](decisions.md#d07--console-与本轮核心重构解耦) · [D08](decisions.md#d08--不借架构重构扩张范围)。这些决策确定本轮产品边界和工程取舍，不表示相关代码已经改造；剩余内容是实施细节，不是再次请求用户选择。
>
> 本文的现状事实来自本轮只读调查、`CodeGraph` 索引和对应调查记录：`.planning/2026-09-16-architecture-audit/execution-survey.md`、`contract-ops-survey.md`、`hub-survey.md`、`hub-supplement.md`、`console-survey.md`、`verification.md`。调查记录中的源码路径和符号是本文件引用的证据；未调查的关系标为“待核验”。

## 1. 目标、范围与不变量

### 1.1 目标

Agentic 的长期定位仍是 **Agent 的受控执行基础设施**：在明确的入口、身份、策略和确认边界内，提供进程、文件、下游 MCP、终端、Browser、Skill、Room 资源访问，以及 Job 生命周期和运行观测。
本目标稿与历史诊断编号的交叉索引：A01（多入口合同）→§7；A02（Room 接口收口）→§6.5、§7.2；A03（可见性/能力/准入）→§2.4、§3、§6.1；A04（Hub 身份/连接）→§5、§7；A05（配置热重载）→§7.3；A06（信任保证）→§6.1、§6.3、§6.4、§9；A07（耐久性/投影）→§5；A08（Console）→§3.4、§6.6；A09（规范/验证漂移）→§7、[工程规则](engineering-rules.md)。[历史诊断](../architecture-diagnosis-2026-09-23.md)保留证据和根因，不是当前缺陷清单。


目标架构采用三类职责的组合，而不是再造一套完整 Agent Runtime：

1. **多入口适配器**：stdio、HTTP、local Unix socket、Hub WS/SSE、Hub Apps MCP、CLI 和 TUI 将外部形状转换为共享操作请求。
2. **执行核心与资源适配器**：Agent 在本机拥有策略、确认、执行和资源副作用；不同运行模式复用同一执行核心，仅由 `RuntimeModel`、profile 和 ingress capability 产生有意差异。
3. **控制面与投影**：Hub 拥有公共入口、认证、Agent registry、连接租约、受控 dispatch、confirmation coordination、run receipt 和可诊断 projection；它不拥有 Agent 主机的执行效果。

本轮目标不包括：

- provider orchestration、模型选择、context manager、reasoning loop 或长期上下文管理；
- 把 Room Notebook/Diary/State 变成通用长期记忆系统；
- 因架构整理由五个 crate 强行拆出更多 service/crate；
- 未先盘点受影响合同、调用方和迁移影响，就改变既有默认部署拓扑、权限范围、HTTP 路径、camelCase 名称或 wire 语义。破坏性调整按一次协调升级实施，并随真实实现交付迁移文档，而不是维持长期兼容双轨。

### 1.2 不可变约束

以下约束是在 D01–D08 已确认前提下的目标硬规则，不得由“重构”绕过：

- **权限不放宽**：入口、profile、toolset、policy 和 confirmation 的变化必须说明授权前后差异；元数据 annotation 不是授权。
- **等待不等于取消**：Hub/HTTP/MCP 的 caller wait timeout 只结束该等待或标记观察状态，不自动声称远端 Job 已停止；资源自身若有独立 execution deadline，可以按其契约请求终止，但必须有独立状态和 termination evidence。
- **身份分离**：user/auth principal、`agent_id`、`connection_id`、`run_id`、`request_id`、`event_id`、`job_id`、Room lease 和 Browser lease 不是同一 ID，不得用其中一个代替另一个。
- **迟到/重复结果必须验证 owner**：至少验证 agent、connection generation（按可靠/非可靠消息规则）、run、request、command hash；不匹配的结果不能唤醒无关 waiter 或覆写终态事实。
- **真实状态、缓存和占位必须明示**：`live`、`cached`、`stale`、`unknown_after_restart`、`unavailable`、`placeholder` 不得互相冒充。
- **先盘点合同与迁移影响**：变更前必须盘点既有 HTTP/OpenAPI、MCP profile、stdio toolset、协议字段、工具名、camelCase、部署路径和 release 产物；一次升级时迁移全部仓库调用方，不得只改一侧。

## 2. 目标拓扑

### 2.1 部署拓扑保持不变

以下是**目标中的逻辑关系**，不是对现状的完成声明：

```text
             Console / TUI / CLI / 外部客户端
                  │        │
        HTTP/Apps MCP     local Unix / stdio / CLI
                  │        │
          ┌───────▼────────▼───────┐
          │       入口适配器         │
          └───────┬────────┬───────┘
                  │        │
       ┌──────────▼──┐  ┌──▼──────────────────┐
       │ Hub 控制面   │  │ Agent 执行面         │
       │ auth/registry│  │ policy/confirmation │
       │ connection   │  │ jobs/resources      │
       │ dispatch/run │  │ local state/effect  │
       └──────┬───────┘  └──┬──────────────────┘
              │ WS/SSE       │
              │ protocol     ├── tmux / MCP downstream / tunnel-client
              │ envelope     ├── Room repository / Git
              │              ├── Browser runtime / optional browser-host
              ▼              └── filesystem / process / OS side effects
         Agent connection
```

现有五个 Rust crate 和 Console 的部署角色保留：

- `agentic-gpt`：Agent 执行端、Local/Standalone/Hub agent 运行形态、生产 TUI 与 CLI。
- `agentic-gpt-hub`：VPS/控制面进程及其 SQLite；外部 HTTP、Apps MCP、Agent WS/SSE 是不同入口。
- `agentic-gpt-protocol`：Hub↔Agent wire DTO、消息、命令和必要的纯契约规则。
- `agentic-apply-patch`：纯 patch 解析/变换算法，不拥有文件系统或策略。
- `agentic-browser-host`：独立 Browser extension/native-messaging bridge 进程。
- `console/`：交互适配和 Android 本地 attention；Desktop/Web 当前保持诚实的 UI 壳，remote console、approval board、exec ledger 是独立的未来产品，不是本轮核心完成条件。

`agentic-browser-host` 与 release 一起打包不等于它已经被 `agentic-gpt` 生产调用；现有调查未发现 Agent 对该 crate 的生产 import，目标上仍保持独立进程边界（证据：`execution-survey.md` §1.5、§2.1）。

### 2.2 编译依赖图（compile-time）

编译依赖与运行时调用必须分开记录。当前 Cargo metadata/CodeGraph 证据和目标允许方向如下：

```text
agentic-gpt ───────────────► agentic-gpt-protocol
     │
     └──────────────────────► agentic-apply-patch

agentic-gpt-hub ────────────► agentic-gpt-protocol

agentic-browser-host       （无 workspace 内部 crate 依赖）
agentic-apply-patch         （无 workspace 内部 crate 依赖）
agentic-gpt-protocol        （无 workspace 内部 crate 依赖）

Console Kotlin modules      （当前不编译依赖 Rust protocol/network client）
```

- 目标继续保持 `agentic-gpt-hub → protocol`、`agentic-gpt → protocol + apply-patch`；不得让 Hub 反向依赖 Agent crate、Browser host、Console 或 apply-patch。
- `agentic-apply-patch` 不得反向依赖 `agentic-gpt`、Hub 或文件 I/O 适配器；其调用方负责读取、锁定、重验证、写入和审计。
- `agentic-browser-host` 不得为了复用 Agent 内部状态而成为 Agent 库；若未来需要通信，使用明确的进程协议和安全边界，不把 crate 合并为“共享 Browser runtime”。
- Protocol 仍是 wire authority，但不是所有入口的万能 schema：Agent-local operation identity、admission 和 resource call 属于 Agent crate；仅跨 Hub↔Agent 进程边界的 operation 才在 Protocol 增加 wire DTO/command/envelope 投影。Agent-local descriptor、Hub Apps MCP schemars、HTTP/OpenAPI DTO 各自负责其消费者投影，并通过 parity gate 对照。
- Console 目标上可以拥有独立的 Kotlin wire DTO/client adapter；除非另有明确的跨语言生成方案，不把 Rust crate 直接塞进 Console 编译图。当前 Console shared 未引入 Hub protocol 或网络实现，不能写成已集成。
- `openapi/`、`docs/`、测试和脚本不是 Cargo 编译依赖；它们是合同、验证或发布输入，不能凭文件存在推断运行时调用。

任何建议目录均不代表已经存在。首选在现有 crate 内先划清模块边界；只有当编译、发布、权限或故障隔离确实需要时，才提出新的 crate，并附迁移和回退方案。

### 2.3 运行时调用图（runtime call graph）

运行时调用图不等同于上面的 Rust `use`/Cargo graph：

1. **Hub 远程执行**（现状链条保留其正确方向）：HTTP action 或 Apps `/mcp` → Hub auth/profile/route → Hub agents dispatch → `runs::prepare_run`/pending → protocol envelope 经 WS/SSE → Agent `hub::connect_loop`/可靠 ledger → `local_service` 的共享 operation/result boundary → process、Job、tmux、下游 MCP、notify，以及 D02 要求补齐的 Room 读与维护能力 → Agent Response/JobUpdate/RunReport → Hub receipt/projection/waiter。这里不表示 Hub 已拥有 Agent-local file/Browser 全能力；Room 远端入口必须逐项定义输入、结果、错误、权限和生命周期，并可使用与本地不同的 transport projection/schema。
2. **Local/Standalone/stdio**：目标路径是 local Unix、tunnel stdio、worker HTTP MCP、直接 stdio → Agent MCP framing/ingress → `RequestContext` + operation gate → 共享 value-returning operation/result layer → capability/approval gate → 执行核心和资源适配器。当前 checkout 的 Process Exec/Batch 已由 stdio 与 `local_service::dispatch_process` 共用 admission/Job path；其他 operation family 仍保留部分直接分支或选定 trampoline。WP2 已收紧共享 context/gate/resource owner，但没有声称一个 universal dispatcher 或已复现的行为故障。
3. **Hub 控制面不反向调用自己的 transport**：HTTP route 与 Apps MCP 是两个入口适配器，二者共享 Hub operation/control logic；一个入口不得通过伪造另一个入口的 HTTP/MCP DTO 来实现复用。Hub wire DTO dispatch 仍由 protocol envelope/Hub agents owner 负责，未被 stdio DTO 替代。
 
4. **TUI/CLI**：生产 Process TUI 通过 Local Unix Process API 观察状态/列表，并按需读取输出与结果；它是客户端，不是执行器。配置 TUI 的 commit 是配置/secret 写入适配器。WP2 已为 tmux CLI 的四项本机管理操作（list、attach、create、close）设置显式 CLI 准入：当前实现会调用 `operation::authorize`，其 CLI 分支只按四个精确 operation 名称作 allowlist；CLI 路径不构造 `AppState`，因此不等于通常的 `AppState` policy/confirmation 流程。这不扩展为通用 CLI executor，也不移除其他 CLI 功能。

当前 Process API 的 status/list 仅返回 metadata，output/result 通过独立接口按需读取；breaking upgrade 与旧数据库边界见[Process cutover 说明](../../process-cutover.md)。目标描述不得恢复旧 `job.list` surface。
5. **Console**：当前 Android attention 是由 Room、AlarmManager 和 Notification 构成的仅本地链路；Hub connection field/按钮只是 placeholder，不存在 Console→Hub 调用边。remote console、approval board、exec ledger 不属于本轮核心依赖；未来若作为独立产品接入，必须新增明确的 HTTP/WS/SSE client adapter、认证和状态 projection，不能因接入 token field 就宣称已经集成。
6. **Browser**：Agent 的 `BrowserRuntimeManager`/Node kernel 与 `agentic-browser-host` extension bridge 是两个不同运行时边界。若二者需要交互，必须显式经过 bridge contract；不得凭共同的“Browser”命名推断它们共享 lease 或执行状态。

### 2.3.1 扩展放置（目标规则；不是当前实现声明）

- **仅 Agent 内部的 operation**：放在 `agentic-gpt` 的 `operation`/`local_service` 与实际资源 owner 中，经过 `RequestContext`、capability/policy/confirmation gate；不要仅为复用内部函数新增 Protocol `HubCommand`、serde tag、OpenAPI path 或 Apps-MCP tool。
- **跨 Hub↔Agent 的 operation**：Protocol 只承载稳定的 wire identity、DTO、envelope/hash 和必要纯 metadata；Agent 仍拥有 admission、Job、文件/MCP/tmux/Room/Skill/Browser effect。wire mapping 与 Agent-local operation mapping 在边界处显式转换，不把 Protocol enum 当内部万能 union。
- **Hub 中立 control/projection**：可在现有 Hub crate 内放置不依赖 HTTP response 或 MCP schema 的 command construction、owner/freshness、neutral Job/Room projection；`routes.rs` 保留 HTTP auth/status/OpenAPI adapter，`mcp_server.rs` 保留 OAuth/profile/schemars/`AgenticResult` adapter。两者不得互相调用，也不得直接打开 Agent resource。
- **扩展顺序**：先确定 source of truth、ingress 和是否跨进程，再分别更新 Agent operation、Protocol wire（如需要）、Hub neutral helper、HTTP/MCP projection 与 parity fixture；不要求一个 universal DTO 或 universal dispatch。

### 2.4 运行时分层

目标上，每个 Agent 入口都经过以下逻辑层；目录名仅为建议：

```text
入口适配器
  ├─ 帧处理 / 解码 / 认证 / profile / 连接代际
  ├─ 入口专属 schema 与错误投影
  └─ 转换为 OperationRequest + RequestContext

操作核心
  ├─ capability / policy / confirmation 准入
  ├─ managed Job 准入 / 生命周期（适用时）
  ├─ 共享的值与错误语义
  └─ 写入事实源或生成明确的投影事件

资源适配器
  ├─ process / OS、文件 repository、下游 MCP、tmux
  ├─ Room / Git、Skill workspace、Browser lease / kernel
  └─ 外部 tunnel / extension / notification 服务

耐久性与投影
  ├─ Agent Process 历史 / transport ledger / audit
  ├─ Hub run receipt / registry / connection projection
  └─ Console 本地 Room / UI cache 或远端 status projection
```

`stdio_server` 与 `local_service` 的工具/HubCommand 映射及错误/兼容转换已在 WP2 纳入较窄的 context/gate/result 边界；当前源码进一步将 Process Exec/Batch 收敛到 `local_service::dispatch_process`，但其他 operation family 仍保留部分 stdio 直接资源分支和选定的 HubCommand trampoline。Hub ingress 直接进入 `local_service`。这表示边界/元数据漂移已部分收敛，不表示存在两套执行器或已复现行为故障。后续一致性核验仍应保留各入口的 framing、schema、auth 和 projection 职责，按 operation family 逐步迁移；不应先造覆盖所有入口的万能 schema、registry 或 dispatcher。破坏性接口变更应通过一次升级迁移调用方和文档，而非长期兼容转换来掩盖语义差异。

WP2 已在现有 Agent crate 中实现 `RequestContext`/`authorize` 接缝；目标图中的 `OperationRequest` 仍只是逻辑记号，并非新增的公开类型或框架。Hub protocol DTO 与 wire dispatch 仍保持分离。

## 3. 模块职责、禁止责任与建议边界

> 下表中的“目标逻辑模块”是建议命名；除现有路径外，目标目录尚未创建，均为 illustrative/conditional placement，不是已交付迁移。

### 3.1 Agent（`crates/agentic-gpt`）

| 当前路径/符号 | 目标逻辑模块 | 应负责 | 禁止负责 |
|---|---|---|---|
| `main.rs`、`supervisor.rs`、`tunnel_distribution.rs` | 运行时组装/监督器适配器 | 选择 Local/Standalone/Hub mode，构造 `AppState`，管理 worker/tunnel 生命周期、lock、health 和 restart | 编排 provider/reasoning loop；让 supervisor 代替 operation core；把 worker token 当作公开 API |
| `stdio_server.rs`、`local_control.rs`、`http_server.rs`、`http_oauth.rs` | Agent 入口适配器 | MCP framing、tool descriptor/typed args、stdio resume、local peer UID、HTTP bearer/Origin/Host 约束、入口错误映射 | 自己实现第二套 job/policy/file/MCP executor；把 annotation 当 authorization；通过另一个 transport DTO 调用内部逻辑 |
| `local_service.rs`、`operation.rs`、`operation_result.rs`、`state.rs` | 操作核心/请求上下文 | Agent-local operation identity、共享 value-returning operation、不可变 ingress/operation context、同步 admission、slim result projection、mode/profile/capability 组合；跨 Hub 的 Protocol mapping 只在 ingress/wire boundary 显式转换；保留有意的 Normal/Room/Tunnel/Hub 差异 | 持有 Hub 公共 registry 或 Room 文件；把 Protocol `HubCommand` 当所有内部操作的 union；把 `request_id` 当唯一 owner；吞掉 capability denial |
| `process/managed.rs`、`storage/process_history.rs`、`process/exec.rs`、`operations/{policy,confirmation}.rs` | 执行与审批 owner | Agent Process runtime 拥有 Process/Skill/MCP admission、运行/取消/终态；`storage/process_history.rs` 实现当前 Process history（`process.sqlite3`）；policy/confirmation 各自负责对应 gate；保留 Skill run 复用 Process lifecycle 的关系 | Hub 远程运行；把 wait timeout 写成 cancel；把 SkillInstallStatus 与 Process state 合并；让任意入口绕过 gate 或审计 |
| `file_ops.rs` + `agentic-apply-patch` 调用 | 文件资源操作 | canonical path、reserved path、共享 `exec::normalize_roots` 的 configured write/read-only/deny roots、TOCTOU/revision、lock、stage、confirmation、commit 和 audit；路径 root expansion/resolution failure stage 必须保留；调用纯 patch 算法 | 在 patch crate 内加入 filesystem/policy；用 patch 成功代替写入成功；跳过二次 revalidate |
| `mcp.rs` | 下游 MCP 适配器 | server config、HTTP/stdio child、batch admission、并发、confirmation、cancel/timeout、结果和 audit | 把下游 provider 的任意副作用伪装为 core sandbox 已隔离；在 Hub 中复制下游 client |
| `tmux.rs`、相关 CLI 路径 | 终端/tmux 适配器 | 会话/窗格观察、粘贴、结构化执行、按 operation 分类的策略/确认及外部 tmux 状态投影；CLI 仅有四项既有本机管理操作。当前 CLI 调用 `operation::authorize` 的精确操作名白名单（allowlist），但不构造 `AppState`，不能把该白名单说成通常的 AppState 策略/确认流程 | 让外部 tmux server 的生命周期成为 Hub 所有；把本机白名单扩展成未经审查的通用执行入口，或将其误述为由 AppState 支持的副作用准入门 |
| `hub.rs`、`transport_ledger.rs` | Hub 入口/传输客户端 | WS/SSE Hello/heartbeat、connection generation、可靠 envelope、ACK/replay、run report、ReportingOnly 约束 | 公共 Hub auth/registry；把 reporting 当完整 audit；混淆 tunnel key、worker token、Hub API key |
| `room/mod.rs`（建议边界：`repository`、`read/{diary,notebook,state}`、`maintenance`；当前对应 `room_repository.rs`、`room_reads.rs`、`room_maintenance.rs`） | Room 资源适配器/owner | repository-relative path、symlink/root、schema/scaffold、bounded reads、semantic slots、Git/worktree/preflight/maintenance lock；按真实 seam 划分，不预建通用抽象 | Hub-owned notebook/diary 内容；通用长期记忆、context manager 或 reasoning loop；把维护 preflight 写成纯校验、凭 patch/检查成功声称提交完成、编造 ACID transaction，或静默复活旧 JSONL/append/update 语义 |
| `skills.rs`、`skill_installs.rs`、`bootstrap.rs` | Skill/bootstrap 资源适配器 | `skills.rs` 拥有包 digest、activation lease 与 Skill 状态；`skill_installs.rs` 拥有安装 journal/staging/commit/recovery；Skill run 可借用 `jobs` 的 Process lifecycle，但安装/激活事实不归 Job history；bootstrap 只读/初始化受控资源 | provider orchestration、长期记忆；把安装状态当 Job terminal state；绕过 Process/Skill-specific policy 运行任意脚本 |
| `browser_distribution.rs`、`browser_runtime.rs`、`browser_kernel.rs`、`browser_manager.rs`、`browser_manual.rs` | Browser 运行时/资源适配器 | package provenance、descriptor/cache discovery、Node kernel、named lease、reaper/reset、bounded manual docs read | 把 arbitrary JS 当作 core sandbox 已覆盖；把 browser lease 传播成 Hub durable state；把 Browser host 当同一实现 |
| `audit.rs`、`private_state.rs`、`config*.rs` | 本地耐久性/配置适配器 | 配置分层、secret/config commit、private state、history/audit/retention/redaction | 让 best-effort audit 充当 command receipt；在 live reload 中替换 startup-only identity/root 而不重启 |

执行核心中的 `jobs/exec/policy/confirmation/file_ops/mcp/tmux/Room/Browser` 仍应被视为共享实现；Local、Standalone 和 Hub agent 入口只改变 transport/profile/capability，不复制执行器。

### 3.2 Hub（`crates/agentic-gpt-hub`）

| 当前路径/符号 | 目标逻辑模块 | 应负责 | 禁止负责 |
|---|---|---|---|
| `main.rs`、`routes.rs` | HTTP 控制入口 | 监听、action auth、HTTP DTO、错误投影、路由 composition；调用 Hub neutral control/receipt helper | 打开 Agent 文件、spawn shell、直接调用 tmux/MCP downstream/Browser；拥有 Apps MCP schema |
| `mcp_server.rs`、`agentic_result.rs` | Apps MCP 入口/投影 | initialize、OAuth/profile/tool descriptor、typed args、JSON-RPC、native Hub query、执行请求投递和 MCP result projection；可消费 neutral Hub projection | 把 Apps MCP schema 当 Agent-local schema；调用 HTTP route 复用；把 `read_only/destructive/open_world` 注释直接当安全 gate |
| `oauth.rs` | MCP 认证/session 适配器 | PKCE、allowlist、Bearer validation、resource/scope 约束、明确的 restart/session 语义 | 用 OAuth token 自动获得 Agent secret 或细粒度权限；未配置可信 public base 时假定代理可信 |
| `registry.rs`、部分 `state.rs` | Agent 注册与连接投影 | enabled、alias、secret hash、capability/last-seen 摘要；当前 connection lease 投影 | 持有执行结果的最终效果；把 registry capability 摘要当 Agent runtime authorization 的唯一依据 |
| `agents/{transport,lifecycle,dispatch}` | 连接/分发/协调 | connection generation、mode/role、pending owner tuple、可靠 envelope、replay/late result 校验、confirmation coordination | 无 owner 校验地消费 waiter；让 stale 非可靠消息更新当前 projection；无条件把 `store_result` failure 当成功 |
| `runs.rs`、`db.rs` | 控制面持久化收据 | `agent_runs` command/hash/ack/status/result/conflict/reason/history、schema/retention/migration；区分 not-sent/sent-unknown/acked/wait-expired/remote-unknown | 把 receipt 当远端副作用证明；把同步 timeout 当远端 cancel；无限增长的 job cache 当历史库 |
| `room.rs` | active Room 租约/RPC 门面 | active `(agent_id, connection_id)` lease、Room request routing、按入口投影结果/错误和必要的窄 surface adapter | 打开 Room 文件或保存长期 Room content；把 active pointer 当文件锁/持久所有权；以透明 alias/shim 隐藏必需远端能力或把不等价 maintenance 当旧 append/update |
| `notify.rs` | 投递适配器 | Agent freedesktop、ntfy 和明确 unavailable 的 Android 状态；投递结果/健康状态 | 借通知接口引入 reminder/task scheduler；把 Android registration placeholder 说成 delivery |
| `instance_lock.rs`、`utils.rs` | 进程/安全防护 | 单 DB serve lock、ID/token/hash 工具、constant-time comparison | 代替业务 owner validation 或持久化设计 |

Hub 内部的 `HubState` 是控制面运行时组合根，不是所有领域状态的总仓库。`active_room` 是租约，`jobs` 是缓存 projection，OAuth token、pending confirmation 和临时 cache 是易失 session；已产生的 confirmation result、Process history 和错误原因则按实际结果 owner 的 retention 尽量保留。它们都不能冒充 Agent 或 Room 的 source of truth；pending 会话失效也不能推断已批准、已取消或任务已停止。

### 3.2.1 Hub 扩展与状态 owner（目标规则）

Hub 新增 operation 时，neutral control/projection helper 只处理 command construction、receipt/freshness/owner-neutral values；HTTP `routes` 负责 action auth、HTTP status/JSON/OpenAPI，Apps MCP `mcp_server` 负责 OAuth/profile/schemars/JSON-RPC/`AgenticResult`。两种 ingress 保持 distinct auth/error contracts，不以一方调用另一方或共享一个 transport DTO 来“复用”。

状态边界保持窄而显式：Agent Process runtime 是执行生命周期 owner，`process.sqlite3` 保存可查询的 Process 历史；内部 managed Job 表达执行工作，不是外部 API 的独立权威。`skills`/`skill_installs` 是 Skill package/activation/install owner；Hub `runs` 是 dispatch receipt owner，Hub Process cache 只是 live/cached/stale projection；`HubState` 只是组合根。任何新字段先归属这些 owner，再决定 projection，不创建通用 state/repository 层来抹平不同 retention 或副作用语义。

### 3.3 Protocol、Apply Patch、Browser Host

| 组件 | 目标职责 | 禁止责任 |
|---|---|---|
| `agentic-gpt-protocol/src/lib.rs` + 私有 domain modules | `HubCommand`、`AgentMessage`、`HubMessage`、envelope、request/run/hash、Job/confirmation DTO、显式 serde/camelCase 名称和必要的纯验证/边界规则；`lib.rs` 是 root facade，wire domain 实现在 `envelopes`、`identity_config`、`mcp`、`notification_tmux`、`process_jobs`、`room`、`skill_bootstrap` 等私有模块 | filesystem、network、Hub DB、process spawn、policy decision、confirmation delivery、Room/Browser/MCP 业务服务；成为所有入口的万能 schema |
| `agentic-apply-patch/src/{parser.rs,streaming_parser.rs,file_update.rs,seek_sequence.rs,text_file.rs,lib.rs}` | parse/normalize/compute/apply replacement 的纯算法 | 读写文件、path authorization、secret filtering、确认、audit、Git commit |
| `agentic-browser-host/src/{main.rs,lib.rs}` | 独立 native messaging/extension framed bridge、client/pending route 生命周期 | 共享 Agent Browser kernel/lease；把共享目录或 socket 当作无需保护；未经明确 peer/auth 机制和部署边界作为网络服务暴露 |

Browser host 当前 socket `/tmp/codex-browser-use` 的 mode 为 0660，调查未见 peer UID 检查；这与 Agent local MCP 的 parent 0700/socket 0600 不是同一安全等级。Neko、container、共享目录与 Unix socket 是需要保留的真实部署拓扑；应先盘点进程/用户/容器、挂载、UID/GID、访问主体和连接链，再在实施中选择 peer credential、token、组/文件权限等收紧机制，不预选 owner-only。任何本地 bridge 仍不得无授权暴露到网络。

### 3.4 Console、TUI 和 Demo

| 当前路径/入口 | 目标逻辑模块 | 应负责 | 禁止责任 |
|---|---|---|---|
| `console/shared/src/commonMain/.../App.kt`、`navigation/`、`ui/common/`、`domain/attention/` | common UI state/ports | 稳定 UI state、repository/scheduler/client ports、loading/error/status projection、导航与交互语义 | 直接持有 Android Room/AlarmManager 类型；实现 Hub secret、WS/SSE、Agent executor 或长期记忆 |
| `console/androidApp/.../app/AndroidAgenticApp.kt`、`attention/`、`platform/attention/`、`persistence/`、`settings/` | Android local adapter | Room attention source、AlarmManager/Notification/boot restore、Android permission/capability、Hub settings 的真实状态（未来） | 把 Alarm/Notification 当远端 run authority；将 `HubConnectionCard`/未连接按钮宣称 Hub 已集成 |
| `console/desktopApp/.../main.kt`、`webApp/.../main.kt` | platform entry adapter | 各自初始化平台 UI 和未来明确的 local/remote client | 假定 Android attention/Hub parity 已存在；共享不支持的平台 side effect |
| `example/agentic-tui-ux-demo` | visual prototype | 展示视觉/交互原则，作为可选参考 | 生产配置、Hub rows、process 状态、文件写入或 runtime 事实来源 |
| `crates/agentic-gpt/src/tui`、`config_tui` | production TUI adapters | TUI 观察 Local Job、配置 draft/review/commit、secret redaction | 创建第二 Job executor；把 demo 静态内容当真实 Hub projection |

Console 当前 Android 本地 attention 的源码层 transition owner 已收敛到 `AttentionRuntimeCoordinator`/`AttentionTransitionPolicy`：Room 是数据权威，OS alarm/notification 是副作用，overdue restore 使用 atomic claim，snooze 传递完整 item payload。`:shared:jvmTest` 于 2026-09-23 有一条 17 项任务的成功运行记录，仅覆盖 shared `AttentionTransitionPolicy` 测试与 common Kotlin 编译，不覆盖 Android app/Room/OS；`console/settings.gradle.kts` 是独立 Gradle root，Android app host test/assemble、device/emulator 与 OS smoke 仍未取得证据。根 Rust CI/release 不覆盖其 Android/桌面/Web 行为。`HubConnectionCard` 以及 Android settings 的测试按钮仍是 placeholder；`console/shared` 未有 HTTP client、Bearer、Hub protocol，`AndroidManifest.xml` 未声明 `INTERNET` 权限（证据：`console-survey.md` §1–§3）。remote Console 保持独立未来产品。

## 4. 当前路径到目标逻辑模块映射

完整的“当前路径 → 目标逻辑模块”映射已经按 owner 写在 §3：Agent 见 §3.1，Hub 见 §3.2，Protocol/Apply Patch/Browser Host 见 §3.3，Console/TUI/Demo 见 §3.4。§3 的第一列保留现有路径/符号，第二列是目标逻辑模块；这些目标目录尚不存在，不能把映射表当作已完成迁移。

使用映射时，先按 source of truth 和副作用 owner 选择 §3 模块，再按入口类型接入 §2.4 的 operation core。只为更名或“看起来分层”重复建立 facade；若确需搬迁，必须保留调用方迁移记录、发布文档和回退边界，不以长期 alias/shim 或双轨执行替代 clean cutover。

### 4.1 建议目录（尚不存在）

当一个 crate 内部边界需要更清晰时，可以在现有 crate 内逐步采用类似布局；目录迁移是**建议**，不构成当前路径已存在的声明，也不要求一次性搬迁：

```text
crates/agentic-gpt/src/
  ingress/       # stdio, local, HTTP, Hub transport adapters
  operation/     # request context, capability/approval, shared value layer
  execution/     # jobs, process, policy, confirmation, cancellation
  resources/     # files, MCP, tmux, Room（room/mod.rs 建议按 repository/read/{diary,notebook,state}/maintenance 划 seam）, Skills, Browser
  durability/    # job history, transport ledger, audit, private state
  tui/ config*/  # existing interaction/config surfaces

crates/agentic-gpt-hub/src/
  ingress/       # HTTP actions, Apps MCP, OAuth callback
  control/       # registry, connection, dispatch, confirmation coordination
  receipts/      # runs, DB, projections
  resources/     # Room lease, notify adapters
```

目录和模块迁移必须在接口、测试/验证和回退策略准备好后进行，不强制新增 crate。


## 5. Source of truth、状态所有权与投影

| 事实/状态 | 权威所有者 | 允许的投影/缓存 | 不可作何种替代 |
|---|---|---|---|
| Agent 当前执行策略、profile、resource roots、confirmation policy | Agent `AppState` + validated config/policy | Hub capability/last-seen 摘要、Console capability state | Hub registry 摘要不得替代 Agent 的本地授权 |
| 正在运行的 Process/Skill/MCP execution | Agent Process runtime；进程历史由 `process.sqlite3` 持久化 | Hub Process projection/run report、TUI Process API、Console remote status | Hub cache 不得声称本机真实 running；Console 不得成为 Agent 执行权威 |
| Agent 本地 Process 历史 | Agent `process.sqlite3`（仅历史记录；输出/结果按 API 明确请求） | Hub status/result projection | audit/report 不得替代 Process history |
| Hub 侧 dispatch/run receipt | Hub SQLite `agent_runs`（command/hash/ack/status/result/conflict/reason；当前约 24h retention） | HTTP/MCP run response、运维摘要 | receipt 不得证明远端副作用已发生；wait timeout 不得改写为 cancel |
| Hub 当前 Agent connection | Hub `HubState.agents` 中 `(agent_id, connection_id, mode, role)` 的易失 lease | `/v1/info`、agent list、online 摘要 | active connection 不得成为 Agent execution state 或 Room 文件锁 |
| 可靠 envelope 幂等事实 | Agent `transport-runs.jsonl` 与 Hub receipt 的双侧 tuple/hash | replay/pending projection | 任意同 `request_id` 的数据不得越过 agent/run/hash owner 校验 |
| Room Diary/Notebook/State 与 Git metadata | Agent Room repository 文件、schema/scaffold/Git | Hub active Room lease、bounded response、Console Room view（未来） | Hub memory、Console Room DB、长期上下文 manager 不得替代文件 owner |
| Android attention item | Android Room `agentic_attention.db` | Compose UI state、AlarmManager/notification PendingIntent | OS alarm/notification 不得当数据库；Hub item 不得伪装成 LocalMock |
| Browser named lease/kernel | Agent 进程内 `BrowserRuntimeManager` | audit/result metadata | Hub durable map 或 browser-host client map 不得代替 Agent lease |
| Browser extension bridge client/pending route（若启用） | 独立 `agentic-browser-host` 进程 | framed response/status | 不得假定与 Agent BrowserManager 共享 state；socket 可访问性不得由“本地”一词证明 |
| OAuth code/token、pending confirmation、Hub Job cache | Hub 内存 session/projection | status/error response | Hub 重启可使 OAuth/pending/cache 失效；失效不得变成默认批准、远端停止或结果丢失的推断 |
| 已产生的 confirmation result、关联 Process history 与错误原因 | 实际结果 owner 的 Agent Process history/Hub receipt（按明确 retention） | status/error/history projection | 不得因 pending session 失效而抹掉已产生结果；audit/telemetry 也不能替代结果事实 |
| audit/report/telemetry | 各自明确的 evidence/projection durability（当前 audit/report 有 best-effort 成分） | 运维日志、Hub report、TUI/Console 摘要 | telemetry 丢失不得伪装为 command outcome；敏感 args/CWD 不能无界扩散 |

### WP3 durability / recovery 合同（实施基线）

以下是 WP3 的目标合同，不代表故障验收已经通过。持久化成功只证明本层事实，不证明外部副作用可回滚。

| 数据 | durability / 可接受丢失 | retention 与恢复 | 敏感度 / projection |
|---|---|---|---|
| Hub run receipt | correctness-critical：先提交 identity/hash/result，再确认或唤醒 waiter | 完成结果沿用 24h 保留窗口；未完成、unknown、冲突及 replay 去重依据不得随 TTL 删除。SQLite 迁移原子提交并记录版本，保留迁移前恢复副本 | command/result 可能含凭据；API 只返回既有授权投影，省略或压缩必须明确标记 |
| Agent transport ledger | correctness-critical：持锁的条件状态转移；执行前 started、响应前 completed 必须落盘 | 保留全局文件及旧记录；不凭 run/request/hash 或旧 command 的 agentId 推断历史执行 owner。所有未关联旧记录保持 LegacyUnowned，启动扫描和新 envelope 均不自动执行/披露其结果；旧全局扫描可能跨 Agent 执行，payload 目标不能补证实际 owner。损坏 fail-closed；压缩保留身份、hash、结果与冲突，不设置去重过期 | command/result 私密；新记录显式 owner，旧记录恢复需人工核对而非清空去重依据 |
| Agent Process history | retained result：已产生结果不能被 best-effort 通知失败跳过 | 新建 `process.sqlite3`；不打开、迁移或删除旧 `jobs.sqlite3`，旧库中的历史行不进入当前 Process API；active 项重启后为 UnknownAfterRestart | per-Agent private SQLite；状态/list 只给 metadata，输出/结果独立按需读取并受大小限制 |
| Hub Process projection | ephemeral：重启、容量或 TTL 淘汰允许 | 上限 4096 条、最后观测 15 分钟淘汰、60 秒后标 stale；定期清理。以 Hub observedAt 而非 Agent 时钟判年龄；缓存淘汰不修改 run/history/Process | 原始 Process status 不增加伪造时间；响应另标 live/cached/stale/unknown 及观测时间 |
| workspace audit / report | best-effort：允许失败、队列丢弃和轮转；不承担执行去重 | audit 持锁写完整行并有界轮转；损坏旧行保留在轮转副本而非当执行记录恢复。report 维持既有有界队列 | args/path/output 可能敏感；不是完整审计或外部副作用证明 |
| config / secret | correctness-critical：完整旧值或完整新值；不能把半写文件投入运行 | 原子替换、唯一备份、文件和目录同步；setup 多文件提交必须说明并处理恢复边界。加载验证失败不静默降级权限 | 保持既有明确权限与共享部署合同；新私密文件限制访问，禁止意外跟随目标 symlink |
| Room files / Git | Agent repository authority；Hub 无内容副本 | 延续 repository revision/path/Git 写入合同，不受 Hub restart 或 cache TTL 影响 | 文件 owner/path policy；Git 历史不是所有外部写入的事务回滚 |
| Browser lease / bridge routes | ephemeral：Agent lease 与独立 host routes 各自进程所有 | 重启失效不等于关闭所有外部 tab；bridge 仅本地共享文件系统，不远程暴露 | socket 0660 依赖真实 UID/GID、目录和 mount；不得套用 Local MCP owner-only 结论 |
| notification endpoint | Hub SQLite registry authority；投递 best-effort | 与 DB 一同迁移/恢复，不因连接/cache 过期删除注册 | endpoint/token 按现有授权投影；注册成功不证明设备收件 |
| Console local attention | Android Room authority；OS alarm/notification 是 projection | 保持独立本地恢复合同；Android boot/process-death 验证归 WP5 | 不搬入 Hub，不通过本包伪造已完成恢复验收 |

### 5.1 运行身份模型

每次跨进程操作的最小逻辑 owner tuple 应可表达：

```text
principal/auth context
  + agent_id
  + connection_id / boot_generation
  + run_id
  + request_id
  + command_hash
  + optional job_id
  + ingress/profile/toolset/effect kind
```

- `run_id` 表示一次控制面运行收据；`job_id` 表示 Agent 执行生命周期；二者可关联但不相等。
- `request_id` 是一次请求/响应 rendezvous，不得单独授权或作为跨 Agent 全局 owner。
- `connection_id` 表示连接代际；可靠迟到 Response 可按显式规则由旧代际补交，但不能更新当前连接 metadata 或任意 Job projection。
- `event_id` 用于 transport/replay 事件去重，不等于用户操作或结果 owner。

## 6. 关键能力的边界示例

### 6.1 Process、Job 与 terminal

**目标放置**：`operation/execution`（现有 `process/managed.rs`、`process/exec.rs`、`operations/policy.rs`、`operations/confirmation.rs`）拥有 admission、policy、confirmation、spawn、monitor、cancel 和 Process lifecycle/terminal state；`storage/process_history.rs` 持有当前 Process history（`process.sqlite3`）的终态持久化与恢复。Hub/stdio/local/HTTP/TUI 只做入口/投影，Skill install/activation state 仍归 `skills.rs`/`skill_installs.rs`。

**规则**：

- `exec::preflight` 的路径启发式检查不等于 OS sandbox；`bwrap` 当前受配置控制且默认关闭，不能在文档中宣称 generic process 已被 roots 隔离。
- sandbox enforcement、外部执行的信任假设和既有安全默认必须分开描述；自用可控环境不等于脚本/MCP/Browser代码完全可信。本轮不改变 sandbox 默认、policy override 或权限模型，也不把未受 core sandbox 约束的 MCP stdio/Browser JS/tmux/tunnel child 说成同一安全等级。caller wait timeout 不能覆盖资源自身可终止的 execution deadline；二者必须使用不同状态和 evidence。
每个入口都必须经过与其权限和副作用相匹配的准入检查；这不要求所有入口调用同一个 gate。当前 CLI `tmux` 的四项操作会调用 `operation::authorize`，但其 CLI 分支只按 operation 名称 allowlist 准入，且该路径不构造 `AppState`，因此不经过通常的 `AppState` policy/confirmation 流程。不要把它写成完全绕过 `authorize`，也不要把 allowlist 说成与效果策略/确认等价的 gate。

### 6.2 File 与 apply-patch

**目标放置**：文件副作用在 Agent `file_ops`/file resource adapter；patch 算法在 `agentic-apply-patch`。

**正确链**：canonicalize/path policy/reserved roots → parse/compute pure replacement → sorted path locks + revision snapshot → stage temp → confirmation（如需要）→ revalidate → commit/add/delete/move → audit。

**禁止**：

- 在 `agentic-apply-patch` 加 filesystem、path policy、secret、Git 或 confirmation；
- 只验证 patch 文本，不验证目标路径/revision；
- 把 file operation 的成功写成 generic process Job，或把 patch algorithm 的成功写成文件已经落盘。

### 6.3 MCP 与下游 provider

**目标放置**：Agent `mcp.rs`/managed Job 负责下游 HTTP/stdio client、concurrency、batch、confirmation、timeout/cancel；Hub `mcp_server.rs` 负责 Apps MCP ingress/profile/projection；Agent `stdio_server.rs` 负责 Agent-local surface。

三套消费者表面不是一套万能工具面：

- Protocol 是 Hub↔Agent wire authority；
- Agent `stdio_server` 的 live descriptor/toolset 是 Agent-local surface authority（当前调查记录 Normal 29、Room 追加 11，合计 40）；
- Hub `mcp_server` 的 rmcp/schemars 是 Apps MCP authority；
- `openapi/hub.yaml` 是 GPT Actions HTTP projection，但必须与 `routes.rs` 的真实 HTTP DTO/response adapter parity；
- `docs/tool-contract-matrix.md` 是 review aid，不是 runtime schema。

下游 MCP 服务器和 Browser JS 的副作用可能超出 generic process path policy；应显示 `effect/trust/owner`，而不是用 annotation 或工具名给予虚假的 sandbox 保证。

### 6.4 Browser

Agent Browser 路径（`browser_distribution` → `browser_runtime` → `browser_kernel` → `browser_manager` lease）负责包 provenance、runtime discovery、Node kernel 和进程内 named lease。`browser.repl` 的 arbitrary JavaScript 是显式 open-world/external effect，必须有单独授权/部署说明，不得自动等同于 `policy.rs` 的 process guard。

`agentic-browser-host` 只负责独立 extension bridge。当前固定 `/tmp/codex-browser-use`、socket 0660、未见 peer UID 检查；Neko、container、共享目录与 Unix socket 是需要保留的真实部署拓扑，具体 peer/auth、组/文件权限或 token 机制在盘点后细化，不预选 owner-only。任何本地 bridge 仍不得无授权暴露到网络。实验性 `experimental/chrome-control-poc` 不迁入核心 runtime。

### 6.5 Room

Room 是受控文件/文档资源，不是通用 memory；D02 已确定所需 Room 读与维护能力必须补齐远端公共面，WP-R 已将其收口为九个当前语义 operation：`diary.active/read`、`notebook.recent/search/read`、`state.list/read`、`maintenance.status/submit`：

- **建议模块边界（仅技术布局，不表示代码已搬）**：`room/mod.rs` 作为模块入口，按真实 seam 划分 `repository`（repository-relative path、root、symlink、schema/scaffold、Git readiness）、`read/{diary,notebook,state}`（各资源的 bounded read）和 `maintenance`（semantic slot、预期变更、worktree/tar preflight、受控写入/提交）。
- Room maintenance 是真实副作用链：本地操作可能写入文件、创建 worktree 或 archive、运行 executor、提交 Git，并可由 `auto_push` 产生或使用 Git remote；workflow 也可以采用本地 request/Git。应复用 Agent 已有的 policy、path/symlink/Git/lock/clean-tree/expected-change/executor/controlled-maintenance authority gate 并留下结果 evidence；不得另造流程 confirmation gate，不能把 preflight 写成纯校验或编造 ACID transaction。
- Hub `room.rs` 只拥有 active Room `(agent_id, connection_id)` lease、路由与结果/错误投影；Hub 不打开 Room 文件，不把内容装入长期 `HubState`。通用 run receipt 可以保留有界 operation result，但不构成 Room 内容 authority；实际内容、文件/Git 副作用和提交事实仍由 Agent Room repository 所有。
WP-R 已由 Hub→Protocol→Agent 的九个当前语义 operation 覆盖产品要求的 Room 远端读与维护语义；远端与本地应在 operation、权限、错误和生命周期上对齐，但不要求机械复制 transport envelope 或同一输入/输出 schema。各入口应有明确的 surface adapter/parity 记录；九项操作的 live gate 于 2026-09-22 记录为通过。
WP-R 已将 Hub Full/HTTP/Protocol/Agent surface 收口到上述九个 operation；`recent/search` 保留公共 operation 名称但采用当前 Markdown DTO 语义。旧 JSONL/legacy HubCommand 路径已在 caller、descriptor、OpenAPI/Protocol 与文档迁移后退出当前 contract，旧 passage/date/append/update/remove 语义不做静默映射，也不添加长期透明 alias/shim。旧分叉描述仅保留为历史调查基线；2026-09-22 的 live gate 记录为通过，不构成生产 tunnel/GitHub 部署声明。

### 6.6 Console、Android attention 与 TUI

- Android attention 当前是 local-only：Android Room 保存 item，`AttentionRuntimeCoordinator`/`AttentionTransitionPolicy` 统一 UI、receiver、alarm、boot transition；AlarmManager/Notification 是 OS side effect，boot/process restore 通过 Room 读取并以 atomic overdue claim 避免重复触发。
- `AttentionSourceKind.Hub` 是数据模型预留，不是远端 producer；不能将 Hub run/job 写入 Android Room 后称为同步完成。scheduler snooze 必须接收完整 item payload，required schedule 不得以 degraded/unavailable 静默当作成功。
- Desktop/Web 当前是 UI 壳，shared `App()` 指向 placeholder；`HubConnectionCard` 只保存短生命周期 UI field。remote console、approval board、exec ledger 另立产品，不是本轮核心完成条件；若未来接入，另行定义 client、认证、token storage 和状态 projection，且 Android OS/device 证据不由源代码实现替代。
- 生产 TUI 的 Process screen 通过 Local Unix Process API 观察状态/列表，并按需读取输出/结果；demo 的静态 process/Hub rows 只可借鉴视觉原则，不能成为生产事实。

## 7. 合同、迁移与发布边界

### 7.1 多入口 parity gate

任何工具/命令变更先建立差异表，再决定各投影如何迁移。至少比较：

- 名称与 camelCase serde；
- request required/default/bounds；
- response optionality、分页字段、slim shape、错误 code；
- profile/toolset visibility；
- effect annotation 与真实 authorization；
- lifecycle（wait、cancel、late result、unknown）；
- 迁移影响、发布/部署组合和回退边界；不要求各 transport 使用同一 schema。

WP4-A 前的 A01 Job/OpenAPI drift（例如 `startedAt` optionality、分页/等待字段和 cancel projection）是**历史基线**，不能当成当前行为失败或要求 Protocol 吞并 HTTP schema。当前本地 gate 由 `scripts/check_contract_parity.py` 负责 schema/local-ref、选定 response/descriptor 与 loopback live checks；WP-R 九个 Room operation 的 OpenAPI/HTTP projection 已更新，对应 live gate 于 2026-09-22 记录为通过。仍未由该 gate 证明 strict 外部 Actions importer、外部 Apps client、生产 tunnel/cloud、完整 Protocol-wire lifecycle 或 release/tag 组合；`openapi/agents-minimal.yaml` 也不得因存在即宣称被 CI/runtime 使用。

### 7.2 公开合同与 clean cutover

- 项目已公开发布；破坏性变更随同一代码批次提供迁移文档和可执行步骤，说明版本/产物组合、接口/配置变化、备份、升级顺序、验证及回退边界。本文只规定交付要求，不编造尚不存在的命令或版本号。
- 一次协调升级中迁移全部仓库 caller、descriptor、OpenAPI/Protocol 投影和部署说明，并移除被替代路径；不为历史形状添加长期 alias、shim、双轨执行或额外协议协商。
- 远端与本地语义需要一致，但各 surface 可拥有自己的输入/输出 projection；显式 adapter 用来表达真实差异，不能用静默转换吞掉旧错误或权限变化。
- 历史 release/migration 文档中的版本数字本身不是运行时 bug；真正需要修复的是当前运维指引、descriptor、OpenAPI 和 release artifact 的漂移。迁移文档不等于保留旧服务兼容，也不授权删除用户数据。


### 7.3 配置与部署

startup-only identity/root/resource 字段与 live-safe policy/limits 字段必须分层；不能热加载一整份 config 后留下旧 `private_state`、Job history、Browser manager、Hub connection 与新 identity/root 不一致。当前 Agent Process/Skill admission 与 managed batch 已统一采用 admission-time `Arc<Config>` snapshot，queued workers 不重新读取 live policy；后续 admission 才使用 reload 后 config。startup supervisor watcher 保持独立，字段分类变化仍须 deterministic reload 验证，无法证明 live-safe 的字段要求 restart。

release preflight 由 same-SHA checkout 驱动，并校验 `RELEASE_TAG`、Agent/Hub Cargo version 精确匹配、fmt/check/test/build 与 bounded contract parity；2026-09-23 的历史记录显示，在可达镜像环境中本地 `v0.9.1` preflight/parity 通过，且故意的 `v0.0.0` mismatch guard 按预期拒绝。strict Clippy 既有 CI debt 不属于该 preflight；hosted publication、cross-build/ARM runtime、external importer 与生产部署仍须单独取证。

## 8. 保留项与拒绝的替代方案

### 8.1 明确保留

1. **五 crate 与部署拓扑**：当前 compile graph 已有清晰方向，不因目录大小做全仓重写。
2. **单一 Agent execution core + `AppState`/`RuntimeModel`**：Local、Standalone、Hub agent 共享真正执行实现；profile/capability 的差异保留并加固。
3. **Hub durable run receipt + Agent transport ledger**：控制面可查询收据与执行侧幂等事实分层，不改成 Hub executor。
4. **Managed Job 状态机和 bounded output/history**：保留 admission、confirmation、cancel evidence、`UnknownAfterRestart` 和结果大小/retention 边界。
5. **File safety chain 与纯 apply-patch**：canonical path、symlink/revision/lock/staging/audit 和纯算法拆分方向正确。
6. **Browser provenance/lease cleanup、Room semantic repository/Git、Local socket guard、Standalone supervisor**：这些是已有安全/生命周期基础设施，不因边界整理删除。
7. **Hub Full/Coordinator、Agent Normal/Room/ReportingOnly**：它们是不同最小权限 surface，不合并为一套不透明的“万能模式”。
8. **Android attention local-only**：保留真实本地业务，同时明确它不是 Hub reminder 产品。

### 8.2 暂不采用的替代方案及理由

| 替代方案 | 拒绝理由 |
|---|---|
| 先把五 crate 拆成多个 service/crate 或全仓重写 | 编译/部署边界已有；大拆分扩大兼容和回退面，不能解决 owner/gate/parity 漂移。先在 crate 内建立模块边界。 |
| 让 Hub 执行 shell、文件、MCP、tmux 或 Browser | 破坏 Agent 本地 policy/confirmation/resource ownership，增加双 executor 和权限分叉。 |
| 让 Protocol 成为所有入口的统一 schema，或先造万能 registry | Agent-local、Hub Apps MCP、HTTP/OpenAPI、WS wire 的 authority 和消费者约束不同；应共享窄的 operation identity/effect 语义并做 parity gate，不强行一套 schema。 |
| 给所有旧 Room/工具名加永久 alias/shim | 与一次升级、迁移全部 caller、移除 obsolete path 的 clean cutover 相冲突；会掩盖旧权限和错误语义。用随实现交付的迁移文档说明真实步骤，不把必需远端能力标成永久 unsupported。 |
| 把 Hub `active_room`/Room 文本做成长期 Hub memory | 当前文件/Git ownership 在 Agent；会引入长期上下文管理，超出受控资源目标。 |
| 自动把 Android attention、Hub notify 做成统一 reminder scheduler | Android 现有功能是 local-only；Hub 目前没有 reminder/task domain，混合会模糊 source of truth 和部署边界。 |
| 把 `agentic-browser-host` 与 Agent BrowserManager 合并 | 一个是 extension bridge，一个是 Node/browser SDK lease；安全、进程和生命周期不同。 |
| 直接把 sandbox 默认打开或把 Browser/MCP annotation 当隔离保证 | 会改变行为或权限；本轮按 D04 保持 sandbox 默认、policy override 和权限模型不变。若以后另立威胁模型专题，须分别说明 trusted external effect 与 OS containment，不能用 annotation 代替隔离。 |

## 9. 剩余实施细节（随工作包细化）

以下事项不是新的用户选择；第 4 项记录已完成的 WP-R closure，其余事项也不是当前代码已完成的声明；实现者应在 D01–D08 约束下，依据真实 seam、消费者和部署证据细化：

1. Browser host 在 Neko/container/共享目录/Unix socket 拓扑中的具体 peer/auth、组/文件权限或 token 机制，以及对应的部署检查；不预选 owner-only，也不允许无授权网络暴露。
2. generic Process、MCP stdio、Browser JS、tmux 和 tunnel child 的 effect/trust 分类及其逐操作的 policy/confirmation/evidence 表达；本轮不改变 sandbox 默认、policy override 或权限模型，不扩大公网多租户威胁模型。
3. operation core 的最小 `RequestContext`/authorization helper 接口；不预设一个跨所有 projection 的万能 registry。
4. WP-R 九个当前 Room operation 的精确输入/输出、错误、权限、生命周期和 surface adapter 已实现，caller/descriptor/OpenAPI/Protocol/文档迁移已完成；live gate 于 2026-09-22 记录为通过。维护仍以 Agent 既有 safeguards/controlled-maintenance authority 为准，不新增流程 confirmation gate。生产 tunnel/GitHub 部署不在该证据范围内。
5. HTTP/OpenAPI、Hub Apps MCP、Agent-local descriptors、Protocol wire 的 parity checker 与各自 authority 的变更顺序；不要求同一 transport schema。
6. Console 本地 Attention 的独立维护细节；remote console、approval board、exec ledger 的 transport、token/TLS/CORS 和 status projection 只有在另立产品时再设计，不作为核心完成门槛。
7. config startup/live 字段清单，以及 correctness/history/observability 各层的 durability、retention、secret projection、crash/recovery 规则；OAuth/pending confirmation/cache 可在 Hub 重启失效，但已产生的 confirmation result、Process history 和错误原因按 owner/retention 尽量保留。
8. Agent operation routing 的窄收敛只能按 operation family 逐步迁移：Process Exec/Batch 已通过 `dispatch_process` 共享 adapter，其他直接 stdio operation 与 HubCommand wire operation 仍是结构残余；共享 gate/resource owner 不是行为 parity 的证明。2026-09-23 的结构复审未复现行为失败，也不因此要求 universal dispatcher。
9. Release preflight 的 same-SHA/version/fmt/check/test/build/parity 源码 gate 与有界本地运行结果已记录（2026-09-23）；GitHub tag/live publication、托管 runner 上的 artifact/version pairing、strict external importers、生产 tunnel/cloud、ARM runtime、外部 Browser/MCP effects 与 Console 设备行为仍需各自 owner 单独核验。文档目标不把这些外部边界写成已验证，也不把 Console remote product 拉入核心依赖。

以上决策已确认；WP-R 的当前 Room contract 已实现，live gate 于 2026-09-22 记录为通过。本文仍是其他领域的目标/技术草案，不是全局实现证明。编码代理必须按当前代码和安全边界实施，不能把未落地的目标段落当作 API；破坏性改动须随真实实现交付迁移文档和实际验证证据。
