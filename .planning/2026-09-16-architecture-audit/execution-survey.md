# 执行端调查证据

> 原始 scout 调查材料，风险定级与实施建议未全部采纳。Room 的 workspace/room 只是默认，实际支持配置 `repositoryRoot`（`room_repository.rs::repository_root`）；不要按固定路径备份/迁移。显式 policy allow、sandbox 默认关闭、外部 trusted capability 需先澄清威胁模型，不自动定为漏洞或更改默认；不采用先造万能 registry、永久兼容 alias 等建议。正式结论见 `docs/architecture/current-state.md`、`diagnosis.md`、`target-architecture.md`。

## 1. 覆盖范围与一级模块实际职责

### 1.1 agentic-gpt 运行时与入口

- `main.rs`：解析 `run/local/stdio-worker/config/tui/tmux`；`run_hub`、`run_stdio_worker`、`run_local` 都调用同一个 `build_app_state`，差异主要在 Transport/HubMode 与入口适配器。
- `state.rs`：`RuntimeModel::{hub,tunnel,local}`、`Capabilities`、`AppState`。同一执行核心在不同 Transport/Profile 下有意产生能力差异：Hub+Normal 禁用 skills/bootstrap/Room 读能力；Tunnel/Local+Normal 启用 skills/bootstrap 但禁用 diary/notebook/notifications；Room profile 全开（`state.rs:69-135`）。`AppState` 持有 job map、SQLite history、确认 pending、MCP semaphore、file locks、skill lease/install manager、browser context（`state.rs:148-191`）。
- `instance_lock.rs`/`utils.rs`：按 config/resource 取得 flock、计算 `$HOME/.agentic_gpt` 及默认 config/runtime 路径；supervisor 是 standalone runtime lock 的 owner，hidden worker 本身不取得同一 run lock。
- `supervisor.rs`/`tunnel_distribution.rs`：Standalone 的进程 owner、tunnel-client 解析/固定版本与 SHA256 cache、health file、child process group、启动超时、指数 backoff、优雅退出。`authorize_worker` 用 env token 验证 hidden `stdio-worker`。
- `stdio_server.rs`：MCP schema/annotations、toolset availability、stdio resume handshake、生命周期日志/reporting、参数校验与工具路由。它既是 tunnel stdio server，也被 Local Unix/HTTP server 复用 `AgentMcpServer::with_ingress`。
- `local_control.rs`：`agentic_home()/runtime/agent/<agent_id>/mcp.sock`，创建时 parent 0700/socket 0600、UID 校验、peer UID 校验、陈旧 socket probe 与 device/inode cleanup；同时提供 `LocalJobClient`。
- `local_service.rs`：注释明确是共享“value-returning operation layer”。Hub 的 `HubCommand` 在这里映射到 `jobs`、`mcp`、`tmux`、`skills`、`bootstrap`、notify 等，并负责部分 Room/profile gate（`local_service.rs:1-212`）。
- `hub.rs`：Hub agent/client 侧的 WS/SSE 连接、Hello/heartbeat、确认响应、可靠命令 ledger、command dispatch、JobUpdate/RunReport。ReportingOnly 连接只报告，不接收执行 envelope（`hub.rs:31-211`、`799-922`）。
- `http_server.rs`/`http_oauth.rs`：Standalone worker 可选的 Streamable HTTP MCP server；bearer/OAuth、PKCE、resource scope、Host/Origin allow list，默认 bind `127.0.0.1`。HTTP 实例仍构造同一个 `AgentMcpServer(RequestIngress::Http)`。
- `tui/`：终端状态/渲染/输入/恢复 terminal；当前 Process screen 只通过 Local Unix `job.list` 每 500ms 轮询并展示，不创建或取消 job（`main.rs:693-747`、`tui/process.rs`）。
- `cli_i18n.rs`、`notify.rs`、`agent_info.rs`：语言选择、桌面 notification channel/delivery、受限 runtime/config/job/connection 诊断；不是执行 owner。

### 1.2 执行核心

- `jobs.rs`：统一 managed Process/Skill/MCP 生命周期。Process 经过 policy、CWD、preflight、确认、spawn、stdout/stderr tail、monitor、终态；MCP 以 peer/request id 管理 downstream request；Skill 复用 Process job 并持有共享 skill lease。`JobState`、boot generation、cancel evidence、history/audit/report 都在这里串起。
- `exec.rs`：工作目录解析、启发式 argument path policy、交互式 credential/TTY 拒绝、可选 `bwrap` 命令构造。`build_command` 在 sandbox disabled 时直接 `Command::new`，enabled 时才 unshare/bind roots（`exec.rs:19-118`、`223-296`）。
- `policy.rs`：`PolicyDecision` 的 Allow < Confirm < Deny、内置危险程序规则、配置 rule prefix match；不拥有实际 spawn。
- `confirmation.rs`：Freedesktop/Ntfy/Hub confirmation，cancellable wait，Hub pending oneshot；MCP 可按 server 在内存中授予 15/30 分钟 temporary allow。
- `file_ops.rs`：统一文件 path resolver/policy/reserved path；调用纯 `agentic-apply-patch` parser/transform，然后排序取得进程内 path locks、读取 revision、stage temp、确认、再次 revalidate、commit 与 audit。`agentic-apply-patch` 本身没有 filesystem/policy，不能被当作独立受控工具入口。
- `mcp.rs`：server config validation、HTTP/stdio transport、单 call/batch registration、global 8/per-server 2 semaphore、aggregate confirmation、fail-fast、downstream cancellation。调用真正通过 `jobs::ManagedJob` 暴露。
- `tmux.rs`：通过外部 `tmux` server 做观察、paste、structured exec、session 管理；`tmux.exec` 有 CWD/preflight/policy/confirmation，但 session create 没有确认，外部 tmux server 生命周期不归 Agent 所有。

### 1.3 Browser、Skills、Room

- `browser_distribution.rs`：固定 OpenAI repository、嵌入 PGP key/fingerprint、InRelease/包 hash/大小校验、Deb 解包路径类型限制、cache lock/staging/active manifest。
- `browser_runtime.rs`：从显式 descriptor、managed cache 或桌面 registry 发现绝对路径；构造 `NodeReplLaunchSpec`，注入 `CODEX_HOME`、trusted code/services、module dirs、browser service path。
- `browser_kernel.rs`：直接启动 Node REPL 子进程（不经过 shell），10 秒初始化、6 秒 shutdown；`js` 调用 downstream `js` tool，代码可为任意 JavaScript；生成 bootstrap JS import 外部 `browser-client.mjs` 并取得 Chrome browser。
- `browser_manager.rs`：进程内 `BTreeMap<name, LeaseEntry>`；`Initializing/Ready/Closing`，同名 acquire 等待、同 lease repl 持 lifecycle lock 串行、reset 可原地重建 kernel、release/reaper 做 `turn_ended`+shutdown。命名 lease 不跨进程持久化。
- `browser_manual.rs`：只在 descriptor 派生的 docs root 下进行 bounded read/search，拒绝 traversal/symlink；不是通用文件读取器。
- `skills.rs`：workspace `skills/<id>` 包扫描、active state、bounded resource read、无 symlink 的 `scripts` executable resolve；active/update 通过 private state atomic rename。
- `skill_installs.rs`：异步 install records、idempotency、取消、source/size/mode/symlink/digest 验证、staging、target lock、commit journal、activation lease、crash recovery；安装和运行都走技能专用边界后再进入 managed Process。
- `bootstrap.rs`：只读 workspace/bootstrap entrypoint/guides，bounded scan/hash/frontmatter。
- `room_repository.rs`、`room_reads.rs`、`room_maintenance.rs`：`workspace/room` 语义 repository/scaffold/Git status；diary/notebook/state bounded read；maintenance 只允许五类语义 slot，Git worktree/tar preflight，检查预期变更后 commit 或 workflow submit。它是显式 Room domain，不是新增长期记忆/上下文/推理循环。
- `assets/room-scaffold`：Room 初始目录/文档/维护脚本模板；`assets/browser/openai-linux-repo-key.asc` 是 Browser 包信任根。没有检入 `browser-client.mjs`、`browser-service.mjs` 等 Browser JS 源码。

### 1.4 配置与持久化

- `config.rs`：strict load/import 边界、mode/profile/toolsets、sandbox、path roots、policy、MCP、HTTP、Browser、Tunnel、Room/Skills 配置；默认 sandbox disabled、Normal toolset 是除 Room 外全部 namespace。默认 path policy 写 workspace/Documents/Downloads/tmp，read-only cache/system roots，deny SSH/GNUPG/keyring/browser profile/cloud credentials 等。
- `config_cli.rs`、`config_setup/`、`config_templates.rs`、`config_tui/`：CLI mutation、初始化模板、向导 review/validation/secret plan、配置 TUI draft/commit。Secret 文件写入有 parent 0700、file 0600、temp+rename+rollback；普通 config 仍走 `write_config_with_backup`。
- `private_state.rs`：`$HOME/.agentic_gpt/state/agent/<safe-or-hashed-agent-id>`，0700；从旧 workspace/state 迁移 active skills/install records。
- `job_history.rs`：private root/jobs.sqlite3；admission/start/terminal 持久化，启动时把旧 active 标为 `UnknownAfterRestart`，30 日/512 MiB/结果大小有界，异常 DB 改名 `.corrupt-*`。
- `audit.rs`：所有 process/MCP/file/browser/tmux audit 都 append 到 `workspace/.agentic-gpt-audit.jsonl`；记录会包含 program、args、CWD、policy/confirmation、skill/MCP metadata、result hash 等。
- `transport_ledger.rs`：`$HOME/.agentic_gpt/transport-runs.jsonl`，记录 Hub run accepted/started/completed、command/hash/result，用于断线重放和 duplicate/hash mismatch 防护。

### 1.5 外部 `agentic-gpt-hub` 与 `agentic-browser-host`

- `agentic-gpt-hub`（`main.rs`、`routes.rs`、`agents.rs`、`runs.rs`、`mcp_server.rs`）是 VPS 控制面：API key 保护 action API，agent registry 存 secret hash/capabilities，WS/SSE 用 per-agent secret，`runs` SQLite 存 command JSON/hash/ack/result（TTL 24h），Full MCP profile 转发执行工具，Coordinator profile 只暴露 Hub status/run/job/notify。它不拥有 agent 主机上的 process/file/browser/tmux。
- `agentic-browser-host` 是 native messaging extension bridge：stdin/stdout 与外部 extension 通讯，同时在 `/tmp/codex-browser-use/agentic-browser-host-<pid>.sock` 接收本地 framed JSON client，维护 pending route、ID 替换/恢复和兼容 response。源码搜索未发现 agentic-gpt 对该 crate 的生产调用；release 一起打包不等于逻辑集成。它与 `BrowserRuntimeManager` 没有共享 kernel/lease/agent state。

## 2. 运行面、调用链与“同实现/真重复”边界

### 2.1 Runtime surface 矩阵

| Surface | 进程/所有权 | 入口 | 真实执行能力 |
|---|---|---|---|
| `RuntimeMode::Local` | `agentic-gpt` 持 run lock 与 AppState | Local Unix MCP；`local call` 作为客户端 | `RuntimeModel::local`，核心 jobs/file/MCP/tmux/skills；无 Hub command |
| `RuntimeMode::Standalone` | supervisor→tunnel-client→hidden stdio-worker；worker 持 AppState | tunnel stdio、worker Local Unix、worker 可选 HTTP MCP；可选 reporting-only Hub | `RuntimeModel::tunnel`；实现与 Local 相同，但 Hub 只报告，不接收远程执行 |
| `RuntimeMode::Hub`（Hub agent/client） | `agentic-gpt` 持 AppState/run lock | Hub WS/SSE command-capable | `RuntimeModel::hub`；Hub+Normal 有意关闭 skills/bootstrap/Room 能力 |
| TUI | 独立 TUI 进程不拥有运行时 job | 连接 Local Unix 的 `job.list` | 观察器，不是第二执行器 |
| `agentic-gpt-hub` | VPS control-plane process/SQLite | HTTP API、Hub MCP、agent WS/SSE | 认证、路由、pending/replay/report；效果在 agent 主机执行 |
| `agentic-browser-host` | 独立 native host/extension bridge | stdin/stdout + `/tmp` Unix socket | 转发浏览器扩展 JSON，不是 agent Browser SDK/Node lease |

### 2.2 主要调用/状态生命周期链

1. **Hub 远程 process/MCP 链**：
   `agentic-gpt-hub::routes::{process_exec,mcp_call_tool}` → `runs::prepare_run` 写 Hub SQLite → `agents::request_agent` → agent `hub::handle_reliable_envelope` 调 `transport_ledger::{accept,mark_started}` → `local_service::dispatch` → `jobs`/`mcp`/`tmux` → `hub::handle_hub_command` 发 Response/JobUpdate → Hub `runs::{mark_acked,store_result/upsert_agent_report}`。Hub 是 envelope/replay/response owner，Agent 是效果 owner。

2. **Standalone 跨进程链**：
   `supervisor::run` 取得 config lock、解析 secret/tunnel client、构造 `Invocation` → `spawn_tunnel` 把 `--mcp.command=channel=main,command=<agentic-gpt stdio-worker ...>` 交给外部 tunnel → worker `main::run_stdio_worker` 验证 token/config mode/profile → `build_app_state` 恢复 skills/jobs → 同时启动 Local Unix listener、可选 HTTP MCP、`stdio_server::serve_stdio`、可选 `hub::connect_loop` reporting。stdio transport 的 `ResumableStdioTransport` 在 worker 重启时 synthetic initialize 并回放 pending request，不泄露内部 request id。

3. **Local/HTTP/stdIO 统一工具链**：
   `local_control::LocalMcpListener::serve` 或 `http_server::serve_listener` 或 `stdio_server::serve_stdio` → `AgentMcpServer::with_ingress`（分别 `LocalUnix/Http/TunnelStdio`）→ `call_with_result` 做 toolset availability、参数校验、lifecycle/reporting → `dispatch_with_lifecycle` → `jobs`、`file_ops`、`mcp`、`tmux`、Browser、Skill、Room。Local Unix 的 peer UID/权限检查和 HTTP bearer/Host/Origin 检查只在入口层，操作实现仍共享。

4. **Process/Skill Job 生命周期**：
   `stdio_server::dispatch_process_exec` 或 `local_service::HubCommand::Exec` → `jobs::start_and_wait_process` → `policy_decision_for_profile` + `exec::resolve_working_directory` + `exec::preflight` → Deny 终止，Confirm 进入 cancellable confirmation，Allow/approved 调 `exec::build_command` → child pipes/TailBuffer/monitor → `finish_job/finalize_job` → SQLite terminal snapshot、workspace audit、Hub report、terminal hook。Skill run 先由 `skills::resolve_run_program` 和 `SkillLeaseManager` 限定，再进入同一 Process job。

5. **MCP 下游生命周期**：
   `stdio_server`/`local_service` → `mcp::prepare_mcp_batch` 或 `start_managed_call_with_factory`（验证 server/tool/args、快照 config revision、记录 argument keys/hash）→ `jobs::register_mcp_*` → `confirmation::authorize_mcp_*`（或内存 temporary server allow）→ global/per-server semaphore → `mcp::client` 创建 HTTP client 或 stdio child → rmcp request，记录 peer/request id → response/cancel/timeout/detached → MCP Job terminal + audit/report。批量 fail-fast 只阻止尚未启动的 child，已发生 side effect 不回滚。

6. **Browser 与 file edit 生命周期**：
   - Browser：启动时 `main::resolve_browser_runtime` 依次处理显式 descriptor、managed cache/provision、desktop registry → `BrowserRuntimeContext::new` 创建 `BrowserRuntimeManager` → acquire 建立 NodeReplKernel/外部 browser-client bootstrap → named lease 多次 `browser.repl` 串行调用 → idle reaper/reset/release 调 `turn_ended`、shutdown。lease 状态只在进程内。
   - File：`file_ops::edit` → canonical path/policy/reserved check → `agentic_apply_patch::{parse_patch,apply_update}` → lock/revision/path revalidation → temp stage → optional confirmation → 再次 revalidate → add/replace/delete/move commit + parent sync + file audit。

### 2.3 真共享与真重复

- **真正共享的执行实现**：`jobs`、`exec`、`policy`、`confirmation`、`file_ops`、`mcp`、`tmux`、`skills/skill_installs`、Room modules、`BrowserRuntimeManager` 都没有按 Hub/Standalone/Local 各复制一份。
- **边界适配器**：`hub.rs` 负责 WS/SSE/envelope/replay/reporting；`local_control` 负责 Unix socket；`stdio_server` 负责 MCP framing/resume/tool schema；`http_server/http_oauth` 负责 HTTP auth/session；supervisor/tunnel 负责跨进程；TUI 负责观察。
- **确实重复的部分**：`stdio_server::dispatch_with_lifecycle` 与 `local_service::dispatch_inner` 都维护一套工具/HubCommand 映射和错误/兼容转换；`stdio_server.rs` 与 `agentic-gpt-hub/mcp_server.rs` 各自维护 tool description、`read_only/destructive/open_world` 列表。它们不是重复执行器，但会造成 metadata/gate 漂移。`agentic-apply-patch` 与 `file_ops` 不是重复：前者是纯算法边界。`browser-host` 与 BrowserManager 也不是重复：前者是 extension bridge，后者是 Node/browser SDK runtime。

## 3. 当前安全/策略覆盖图

- `file.read/search`：`file_ops::resolve_path` canonicalize 后执行 deny/read/write root 判断；结果有大小/文件/搜索界限。
- `file.edit`：有写 root、symlink/revision/TOCTOU、进程内 path lock、temp staging 与 optional `need_confirm`；**没有 `policy.rs` 的 program rule**，因为它是专门的 file mutation。
- `process.exec/batch`：走 policy、CWD、preflight、可选确认和可选 bwrap；batch 在注册前集中确认，拒绝一个 Deny element 不会启动 batch。
- `tmux.exec`：走 CWD/preflight/policy/confirmation 后 shell-quote paste；`tmux.createSession` 只做 identifier/CWD，Agent 路径无确认；CLI `tmux create/close` 更直接地调用 `create_session_for_config/close_session_local`，无 AppState、confirmation 或 audit（`main.rs:749-773`、`tmux.rs:403-421`）。
- `mcp.callTool/batch`：每 call 默认确认，temporary allow 只在内存；server 配置和参数大小/字段受限，downstream client 有并发/timeout/cancel。
- `skills.install`：源、包、路径、mode、symlink、大小、digest、commit journal 都有约束；当前没有统一 confirmation。`skills.run` 对已激活 workspace skill 做路径/lease 校验后以 `need_confirm:false` 进入 Process job，脚本内部副作用不是逐条可见的。
- Browser：descriptor/cache provenance 与 lease 生命周期控制较强；`browser.repl` 是 arbitrary JS，只有参数界限和 audit 元数据（标题、字节数/hash、timeout），没有 `policy.rs`/confirmation。
- Room：语义路径/slot/Git preflight 很强，读操作 bounded；maintenance 是独立 domain executor，不走通用 jobs。
- Tool annotation：`tool_descriptor` 生成 `read_only/destructive/open_world`，但 `call_with_result` 没有把 annotation 当作 authorization gate；它是 MCP consumer metadata，不是安全 enforcement。

## 4. 主要问题（优先 8 条）

### P1. Process path policy 是启发式 preflight，不是强 capability/sandbox（严重度：High；若 threat model 只要求提示而非隔离则降为 Medium）

- **证据**：`crates/agentic-gpt/src/exec.rs:46-118` 的 `classify_program_access` 只对固定 read-only 程序分类，`looks_like_path` 只识别 `~`、绝对路径、`./`、`../`；`check_path_policy` 只检查看起来像路径的参数。`exec.rs:258-296` 只有 `config.sandbox.enabled` 才走 bubblewrap；`config.rs::default_config` 默认 sandbox disabled。
- **机制**：任意程序/脚本可以通过环境变量、间接路径、参数格式、相对路径衍生写入，且直接模式继承宿主环境和网络；非路径参数本身不表达文件/网络 capability。
- **影响**：path policy 和 program policy 对 generic process 是“启动前启发式筛查”，不能证明受控执行。即使审计记录有 policy decision，实际副作用范围仍可能超过 roots。
- **根因推断（推断）**：`exec::preflight` 与 OS-level containment 被设计成两个可选层，兼容性导致默认不启用 bwrap；缺少统一 effect/capability model。

### P2. Profile/toolset/入口 authorization 漂移，可绕过 RuntimeModel 的能力意图（严重度：High）

- **证据**：`state.rs:104-135` 定义 Profile/Transport capability；但 `stdio_server.rs:397-404` 的 `current_tools/tool_is_available` 只看 `config.toolsets`。`dispatch_with_lifecycle` 中 `skills.setActive`、`skills.list`、`skills.run` 与 `room.*` 有直达 `skills`/`room_reads` 调用（`stdio_server.rs:1000-1127`），不统一调用 `local_service::require_capability`；后者另有 `require_room_toolset`/`require_capability`（`local_service.rs:142-263`）。`config_cli.rs:1512-1522` 可直接 enable `ToolNamespace::Room`，没有 profile 约束。`main.rs:749-773` 的 CLI tmux create/close 绕过 AppState/audit/confirmation。
- **机制**：例如 Tunnel/Local Normal 的 `Capabilities` 明确 diary/notebook=false，但只要 Normal config 手工/CLI 打开 Room toolset，stdio 直达 Room route 只受 toolset 过滤，不受 profile gate；不同 route 有的走 shared service gate、有的直达。MCP annotations 同样没有被当成 gate。
- **影响**：同一个 profile 在不同入口呈现不一致能力；审计、确认、错误语义也不一致，安全边界依赖 route 维护者是否记得加 gate。
- **根因推断（推断）**：toolset visibility、RuntimeModel capability、operation authorization 和兼容路由在不同时间演进，没有单一 `authorize(tool, request-context)`。

### P3. MCP stdio 与 Browser Node/JS 是核心 sandbox 之外的外部执行边界（严重度：High/Medium，取决于是否把配置和 Browser JS 视为 trusted）

- **证据**：`mcp.rs:1529-1551` 的 stdio client 用 `sh -lc <configured command>` 启动 downstream server，不调用 `exec::build_command`/bwrap；`browser_kernel.rs:23-36,58-121` 直接启动 Node REPL 并把 arbitrary `code` 交给 `js` tool；`stdio_server.rs:1500-1529` `browser.repl` 没有 policy/confirmation。
- **机制**：MCP server 启动命令、MCP downstream tool、Browser SDK/JS 的 filesystem/network/process effects 不被 generic process path policy 逐项约束；`NODE_REPL_TRUSTED_*` 是代码/服务信任环境，不是 Agent authorization。
- **影响**：控制面可确认/审计的是“调用了某个 MCP tool/Browser repl”，不能证明其下游副作用被 core sandbox 限定。Browser `repl` 的 open-world 语义可能是有意设计，但必须显式标为 trusted external capability。
- **根因推断（推断）**：下游 provider/browser 被当成可信 adapter，生命周期被纳入 jobs/manager，但 effect enforcement 没有纳入同一 capability contract。

### P4. 配置 allow 会覆盖内置 deny，安全默认不是 fail-closed（严重度：Medium/High）

- **证据**：`policy.rs:21-62` 先计算 builtin decision，随后只要匹配配置规则就把 `configured_decision` 作为最终结果；配置 allow 可以覆盖 builtin deny（内置 deny 在 `policy.rs:76-122` 包括 `su/mkfs/dd/ssh`）。
- **机制**：显式 `policy.allow` 对同一 program/args prefix 具有最高最终优先级，且 allow/deny 规则顺序也由配置内容决定。
- **影响**：若配置被同用户、自动化工具或 live reload 修改，内置绝对拒绝可被解除；这可能是管理员 opt-out，但不应被误解为不可绕过的 guard。
- **根因推断（推断）**：policy 将 builtin 规则视为默认规则而非不可覆盖的 invariant；配置可调性优先于 fail-closed。

### P5. Hub 热重载整份 Config，派生资源/身份未同步（严重度：Medium）

- **证据**：Hub `main.rs:775-819` 的 `watch_config` 只用 `config_matches_runtime` 检查 mode/profile（`main.rs:821-828`），随后替换整个 `state.config`；但 `build_app_state` 只在启动时创建 `private_state/job_history/skill_installs/browser_runtime`（`main.rs:536-579`）。Standalone 则有显式安全 subset，只替换 policy/path/limits/mcp/toolsets/http（`main.rs:879-912`）。
- **机制**：Hub 模式 reload 可改变 agent_id、workspace_root、Browser descriptor、Hub identity 等 startup/ownership 字段；`state.private_state`、SQLite history、InstallManager root、Browser manager、已创建的 transport sender/外部资源仍基于旧值。config、audit/job history/browser/Hub connection 可能指向不同身份或根目录。
- **影响**：热重载后可能出现跨 workspace 持久化、旧 identity 继续在线、新 identity 用于新请求、审计与 job history 分裂；当前没有统一 restart-required 处理。
- **根因推断（推断）**：Hub 复用了旧的 whole-config watcher，而 Standalone 后来引入了 live/startup 字段分层；两种模式没有共享 config mutability contract。

### P6. Standalone worker token 暴露在 tunnel child 的 argv（严重度：Medium）

- **证据**：`supervisor.rs:332-430` 的 `Invocation::new` 把随机 `worker_token` 插入 `worker_command`，再嵌入 `mcp_command()` 和 `--mcp.command`；`command_env` 同时设置 `AGENTIC_GPT_SUPERVISOR_TOKEN`。日志转发和 doctor 输出虽会 redact token（`supervisor.rs:584-617`），不能隐藏 process argv。
- **机制**：同用户的 `/proc/<pid>/cmdline`、进程监控、tunnel diagnostics 或 crash tooling 可能取得 token；取得后若能配合 env 启动 hidden worker，可伪造 supervisor authorization。API key 本身确实只进环境变量，但 worker token 不是。
- **影响**：Standalone “仅由 supervisor/tunnel 启动 worker”的边界弱于注释描述；泄露窗口覆盖 tunnel child 生命周期。
- **根因推断（推断）**：外部 tunnel 只接受一个 command string，设计上无法把 worker secret 作为独立 fd/密钥通道传递，遂把 token 同时放 argv 和 env。

### P7. `agentic-browser-host` 的 `/tmp` bridge socket 没有 per-user 隔离（严重度：Medium；若仅 PoC/未启用则为部署风险）

- **证据**：`agentic-browser-host/src/lib.rs:13-16` 固定 `BRIDGE_DIR=/tmp/codex-browser-use` 与 log path；`prepare_socket:457-470` 只 `create_dir_all`、无条件 `remove_file` 旧路径、bind 后 chmod 0660，无 parent/socket owner/UID/安全模式校验；PID socket 路径见 `:472-473`。消息直接由 `handle_client_message:270-322` 转发到 extension。
- **机制**：同机其他用户/同组进程若能访问 parent/socket，可发送任意 bridge JSON；启动时可无条件删除既有 path；logger append 无上限且错误被忽略（`lib.rs:96-98`）。
- **影响**：扩展侧请求/响应和状态桥接可能被本地非预期 client 注入或观察；与 agentic-gpt 的 0700/0600 local MCP 设计不一致。源码没有 agentic-gpt 生产 import，因此不要把它描述成 BrowserManager 的当前调用链。
- **根因推断（推断）**：该 crate 假定 `/tmp` 本地 trusted PoC 环境，没有复用 agent-local private runtime socket 的 threat model。

### P8. 持久化/审计/可靠传输多为 best-effort，且隐私、增长、崩溃语义不统一（严重度：Medium）

- **证据与机制**：
  - `config.rs:1980-1992` 先 copy backup，再 `fs::write` config，没有 atomic replace/fsync；崩溃可留下 partial/invalid config。
  - `audit.rs:146-184` 四类记录都对 workspace JSONL 独立 `OpenOptions::append`/`writeln!`，无显式 lock、fsync、rotation；`jobs.rs:1439-1495` 忽略 audit write error。记录字段可能含完整 args/CWD/result metadata。
  - `transport_ledger.rs:37-135` 每次读完整 JSONL 到 HashMap 后 append，无 cap/rotation/lock/fsync；文件增长后 reconciliation 成本随历史增长。
  - `job_history.rs:298-316,728+` DB 不可用时 terminal 放在 bounded in-memory pending queue；进程在 retry 前退出会丢终态细节，虽能把已落库 active 标成 `UnknownAfterRestart`。
  - `hub.rs:611-643` reporting channel 是 bounded 64 且用 `try_send`，lock/full/disconnected 都丢事件；reporting 是 best effort，不是完整 audit。
- **影响**：配置 crash recovery、审计完整性、Hub reliable ledger 与 reporting 的耐久性不在同一等级；本地 job history/audit 可能保留敏感命令参数，而 Hub metadata 模式会 redact。
- **根因推断（推断）**：多个模块各自实现轻量 JSONL/SQLite durability，没有统一 append/atomic/retention/secret-projection contract；可接受的 telemetry loss 与需要可靠的 command outcome 没有明确分层。

### 次要但应保留在 backlog 的观察

- `skill_installs.rs:1261-1326` 先 DNS resolve 检查 public IP，再由 HTTP client 重新解析/连接；存在 DNS rebinding 窗口的可能性（**推断**，需结合 reqwest resolver/部署 DNS 验证）。
- `browser_manager.rs:551-554` `validate_lease_name` 只拒绝空白，不限制长度/control 字符；目前只作内存 map/audit 字段，严重度 Low。
- Browser runtime discovery 失败会 fail-open 为 runtime unavailable（`main.rs:363-494`），这是安全上较保守但运维上不透明的选择；显式 runtime 无效不会自动 fallback 到 managed source。

## 5. 值得保留的结构

1. **单 AppState + RuntimeModel**：Hub/Standalone/Local 共用真正的 execution core，避免三套 process/file/MCP 实现；profile/transport capability matrix 也把有意差异显式化。
2. **Managed Job 状态机**：admission、WaitingConfirmation、Running、terminal、bounded tails、boot generation、kind-aware cancellation evidence、SQLite recovery 是受控执行的好骨架。
3. **`local_service` value layer**：Hub adapter 与 operation result 解耦；建议把它演进为唯一 operation core，而不是再复制一套 runtime loop。
4. **文件安全链**：canonical path、deny precedence、reserved paths、sorted locks、revision revalidation、temp staging、parent sync，以及纯 `agentic-apply-patch` 算法边界，整体值得保留。
5. **MCP 生命周期模型**：config snapshot/revision、argument key/hash metadata、batch aggregate confirmation、global/per-server concurrency、cancel/detached 语义明确。
6. **Local Unix 与 HTTP auth**：local peer UID + 0700/0600 + inode guard；HTTP bearer/OAuth/PKCE/Host/Origin 校验；这两种入口是可复用的 perimeter adapter。
7. **Browser runtime provenance 与 lease cleanup**：PGP/hash/path validation、explicit/managed/desktop discovery、Initializing/Ready/Closing、bounded turn-ended/shutdown/reaper；应与 authorization 分离而不是删除。
8. **Skill install recovery**：staging/target lock/commit journal/activation lease/digest 与 crash recovery 能承载可靠安装；不要把 Skill domain 误并成长期记忆。
9. **Hub reliable envelope**：command hash、ack、started/completed ledger、duplicate/replay、boot generation 对断线恢复有实际价值；只需明确 durability 等级。
10. **Room 语义边界**：repository-relative path、symlink rejection、semantic slots、Git preflight 和 bounded reads 是可控 domain executor，不应扩展成 generic memory/context subsystem。

## 6. 目标边界与增量切分建议

目标不是完整 Agent Runtime；不引入 provider orchestration、context manager、长期记忆、reasoning loop。建议保留“受控执行核心 + 多入口 adapter + 外部资源 adapter”三层：

```text
Ingress adapters
  Hub WS/SSE + reliable ledger
  Tunnel supervisor/stdio resume/reporting
  Local Unix / local CLI
  HTTP MCP/OAuth
  TUI observer

Execution core
  RequestContext + Capability/Approval gate
  PathResolver + process policy + sandbox runner
  Managed Process/MCP Job + cancellation/history
  File operation + pure patch transform
  Skill install/run + Room semantic executor
  Browser lease/session manager (explicit external effect)

External resource adapters
  tunnel-client, tmux server, MCP downstream servers
  Node/browser-client/service assets
  optional browser-host extension bridge

Durability/observability
  atomic config, job history, command ledger, audit/report projections
```

增量建议：

1. **契约冻结与字段分层**：把 `ToolsetConfig`、`RuntimeModel::Capabilities`、tool annotations、confirmation mode、effect kind、ingress/source 形成一个 capability registry；区分 startup-only 与 live-safe config。先不改执行语义，保留兼容别名。
2. **唯一 authorization gate**：让 `stdio_server`、`local_service`、Hub command、HTTP/Local、CLI tmux、Room/Skill/Browser direct routes 都在进入操作 core 前调用同一个 gate；tool list/schema 从同一 registry 生成。确认/拒绝结果仍复用现有 `confirmation`，不新增长期状态。
3. **进程和外部子进程隔离**：把 `exec::preflight` 与 sandbox enforcement 明确分离；对 generic process 规定 sandbox/explicit trusted mode；对 MCP stdio、Browser JS、Room Git、tmux、tunnel-child 记录 owner/effect/trust，不把 arbitrary external effect 伪装成普通 path-controlled process。
4. **持久化分层加固**：先让 config atomic replace + fsync；再为 job history/transport ledger/audit 定义 lock/rotation/retention/secret projection。保留 `UnknownAfterRestart` 和 command-hash replay，不把 audit/report 当作同一可靠等级。
5. **Browser 边界单独演进**：`browser_distribution/runtime` 继续负责 provenance/descriptor；`browser_manager/kernel` 负责进程内 lease；新增的只是 browser JS authorization/trust contract，不是 reasoning loop。`agentic-browser-host` 保持独立 crate/protocol；若要部署则迁移到 agent-private runtime socket 或明确声明本地 trusted PoC。
6. **适配器收敛**：迁移完成后由 `local_service`/core 统一 operation result，`stdio_server` 只做 MCP framing/schema/compat，Hub 只做 envelope/transport，TUI 继续 observer；重复的 descriptor/risk lists 迁出 `agentic-gpt-hub/mcp_server.rs` 与 `stdio_server.rs` 后再考虑删除旧 aliases。

## 7. 已有验证入口与未调查盲点

### 已有验证入口（按要求未运行）

- `crates/agentic-gpt/src/jobs.rs`：process cancel、terminal state、job history merge/restart generation。
- `crates/agentic-gpt/src/mcp.rs`：confirmation、batch、concurrency、cancel、downstream client。
- `crates/agentic-gpt/src/file_ops.rs`：path policy、TOCTOU/revision、partial commit、temp staging。
- `crates/agentic-gpt/src/stdio_server.rs`：tool schema/annotations、lifecycle、batch、Browser result marker、skill/Room dispatch。
- `crates/agentic-gpt/src/config.rs`：default/import/strict load/path policy/redaction；`config_setup/`、`config_tui/`：secret/config commit。
- `crates/agentic-gpt/src/browser_distribution.rs`、`browser_runtime.rs`、`browser_kernel.rs`、`browser_manager.rs`、`browser_manual.rs`：包信任、descriptor/env、kernel lifecycle、lease/reaper、docs path。
- `crates/agentic-gpt/src/tunnel_distribution.rs`、`supervisor.rs`、`private_state.rs`、`job_history.rs`：archive/hash/cache、worker env/health/restart、权限/迁移、DB recovery/caps。
- `crates/agentic-browser-host/src/lib.rs`：frame、pending route、notification、status、ID restoration、compat。
- `crates/agentic-gpt/tests/standalone_supervisor.rs`：真实 supervisor/worker/tunnel smoke、stdio resume、live reload、invalid config/log behavior；未运行。

### 未调查盲点

- 仓库没有 Browser JS 源码；只审计了 Rust 如何解析/启动外部 `browser-client.mjs`、`browser-service.mjs` 和 `cua_node`。外部资产实际 filesystem/network/child-process 行为、Node REPL policy 未能从本仓库证明。
- 未执行任何 build/test/lint/format/smoke，也未读取密钥、用户数据或 `~/.agentic_gpt`；因此没有 crash fault-injection、并发 append、真实 bwrap/tmux/Node/tunnel/extension 行为证据。
- Hub server 的完整 live deployment、OAuth callback 外部网络、systemd/container/SELinux/用户组权限未做运行时验证；源码层已有 route/auth/DB 证据，但不能替代部署验证。
- 未把现有文档作为行为事实；上述结论均来自源码、符号/行号和已存在的验证入口。