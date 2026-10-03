# Standalone 与本地 MCP 运行时

Agentic 提供三种运行时形态。推荐直接部署的 Standalone 模式会在一个 Agentic worker 前运行 OpenAI 官方 `tunnel-client`，命令路径不经过 Hub。本地集成模式通过私有 Unix socket 提供相同的 Agent 工具表面，无需 tunnel 凭据。Hub 模式仍是可选的集中式拓扑。

## 运行时拓扑

```text
Standalone 模式（推荐）：
Secure MCP Tunnel -> tunnel-client -> agentic-gpt worker
                                      |-> stdio MCP 入口
                                      |-> 仅所有者可访问的 Unix MCP 入口
                                      \-> 可选 HTTP MCP 入口：http://<host>:<port>/mcp
                                          （由 worker 持有，由 httpMcp 启用）
                                      \-> 仅用于报告的可选 Hub 连接

本地集成模式：
本地 rmcp CLI/client -> 仅所有者可访问的 Unix socket -> agentic-gpt worker

Hub 模式（集中式）：
ChatGPT -> HTTPS Hub -> WebSocket/SSE -> agentic-gpt -> 本地策略/服务

当配置包含 `mode=standalone` 时，使用 `agentic-gpt run` 启动 tunnel 运行时。Agentic 会解析并
验证 tunnel client，运行其 `doctor --json` 预检，构造 worker 命令，监管 tunnel/worker 进程树，并将
worker 的 stdout 保留给 MCP framing 使用。同一个隐藏 worker 拥有仅所有者可访问的 Unix MCP socket；
当 `httpMcp.enabled` 为 true 时，也拥有可选的 HTTP MCP listener。不要直接启动隐藏的
`stdio-worker` 命令。

Standalone HTTP endpoint 固定为 `/mcp`，支持已配置的 direct bearer，以及可选的 Standalone ChatGPT connector OAuth 合同。
未配置 `httpMcp.publicUrl` 时，direct bearer 仍然可用，OAuth 路由会 fail closed；配置有效的 HTTPS `publicUrl` 后，
listener 会公布以下路径对应的 protected-resource metadata：
`/.well-known/oauth-protected-resource/mcp`，兼容根路径的别名 `/.well-known/oauth-protected-resource`，以及
AS/OIDC 别名 `/.well-known/oauth-authorization-server` 和
`/.well-known/openid-configuration`。`/oauth/authorize` 与 `/oauth/token`
为 `agentic:mcp` 实现单一 authorization-code 流程。仅接受
`https://chatgpt.com/connector/oauth/<suffix>` 和精确的
`https://chatgpt.com/connector_platform_oauth_redirect` 回调。不提供 refresh token、
`offline_access`、DCR、通用注册或任意重定向。

OAuth code 和 access token 是不透明的 listener 本地内存记录，带有过期和撤销状态。轮换 direct bearer 内容会原地更新认证、
保留已有 rmcp session，并撤销 OAuth 记录；替换 listener、rebind、禁用或重启都会丢弃所有 listener 本地状态。
Standalone 授权页面及工具/profile 语义不等同于 Hub 的 `Hub API key`、profile 或路由合同。rmcp transport
使用有状态的 Streamable HTTP/SSE，因此 rebind 或禁用后仍需重新执行 `initialize`。schema、allow-host、CLI 和
reload 详情见 [`configuration.md`](configuration.md)。

`publicUrl` 仅用于公布外部 HTTPS origin；它不会改变本地 bind，也不负责路由流量，且该 origin 可以仍为 loopback/private。
ChatGPT 所用的所有同级 discovery、authorization、token 和 `/mcp` 路由都必须通过 HTTPS 暴露。listener 会拒绝格式错误或缺失的
Host，并在每个同级路由上禁止不在许可列表内的 authority。反向代理或 ESA 必须将其实际发送的 authority 放入 `allowHosts`；
若存在 `Origin`，它必须与 `publicUrl` 完全一致；未提供 `Origin` 时仍允许 server-to-server 请求。不会添加宽松的 CORS。

开发时若不使用 tunnel 配置或 Hub reporting，可设置
`mode=local` 并运行 `agentic-gpt run`。此模式会加载相同的、由 profile 选择的工具集预设、
策略、路径策略、确认、审计、实时配置和托管执行状态，但只提供 Unix MCP 入口；
Local 模式下 `httpMcp` 不会添加 TCP listener。

### 六种公开运行时映射

| 命令 | 命令传输 | 能力 profile | Hub 连接 |
| --- | --- | --- | --- |
| `agentic-gpt run` (`mode=standalone`, `profile=normal`) | Tunnel stdio + 本地 Unix MCP + 可选 HTTP MCP | Normal | 默认禁用；启用时仅用于报告 |
| `agentic-gpt run` (`mode=standalone`, `profile=room`) | Tunnel stdio + 本地 Unix MCP + 可选 HTTP MCP | Room | 默认禁用；启用时仅用于报告 |
| `agentic-gpt run` (`mode=local`, `profile=normal`) | 本地 Unix MCP | Normal | 禁用 |
| `agentic-gpt run` (`mode=local`, `profile=room`) | 本地 Unix MCP | Room | 禁用 |
| `agentic-gpt run` (`mode=hub`, `profile=normal`) | Hub | Normal | 可执行命令 |
| `agentic-gpt run` (`mode=hub`, `profile=room`) | Hub | Room | 可执行命令 |

传输方式不会改变本地策略。Tunnel、HTTP 和本地 Unix 入口对按 profile 选择的工具集使用相同的 Agent 工具表面和策略边界。
Normal 预设默认不包含逻辑 `room` namespace；Room 预设会启用它。显式设置的 `toolsets.enabled` 是最终依据。进入同一个
worker 的调用共享其实时配置、确认状态、审计、容量和托管执行注册表。Room bootstrap、diary 和 notebook 执行遵循实时
`room` namespace，而非启动时的 profile。因此，Normal-profile worker 可在不重启的情况下启用 `room`；若该 namespace
仍处于禁用状态，直接 dispatch Room 操作会返回 `room_toolset_required`。

## 本地 Unix MCP 控制通道

socket 路径根据已配置的身份生成：

```text
~/.agentic_gpt/runtime/agent/<agentId>/mcp.sock
```

runtime 目录权限为 `0700`，socket 权限为 `0600`，且接受的连接必须报告相同的本地 UID。Agentic 从不打开 TCP 调试端口。
启动时若 socket 正在使用则拒绝启动；只有确认是本进程所有的陈旧 socket 才会被安全移除。该模式使用现有的每配置
`.run.lock`，因此 tunnel-backed 与仅本地的运行时不能同时占用同一配置。`agent.info.connections.localMcp` 会报告
`ready`/`unavailable` 及准确的 socket 路径。

使用内置的真实 rmcp client 检查或调用运行中的工具表面：

```bash
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info --config ~/.agentic_gpt/config.json --arguments '{}'
printf '%s' '{"path":"README.md"}' | \
  agentic-gpt local call file.read --config ~/.agentic_gpt/config.json \
  --arguments-file -
```

`agentic-gpt local` 命令是仅所有者可访问的 Unix MCP client；其调用保留 `local:` 审计 provenance。它不同于
`agentic-gpt tmux` 本地管理 CLI；后者只提供四个命令：`list`、`attach`、`create` 和 `close`（request-context
operation 名称为 `tmux.listSessions`、`tmux.attach`、`tmux.createSession` 和
`tmux.closeSession`）。这些 CLI 调用使用 `localadmin:` provenance，不增加远程批准语义，也不会伪造 AppState。
其他入口的前缀彼此独立：Tunnel 使用 `tunnel:`，HTTP 使用 `http:`，Hub 使用 `hub:`。

通过已有的受保护本地 Unix MCP 通道，可调用隐藏的 `privateevent.inject` 工具，将外部事件注入该 Agent 收件箱。
此工具仅限本地 Unix MCP，不通过 loopback HTTP MCP 或 daemon stdio 提供；Agent 必须以同一 UID 运行，运行时目录为 `0700`，socket 为 `0600`。
例如，以下命令从标准输入提交一条低严重度事件：

```bash
printf '%s' '{"message":"需要查看部署状态","ref":"deploy-42","severity":"low"}' | \
  agentic-gpt local call privateevent.inject --config ~/.agentic_gpt/config.json \
  --arguments-file -
```

`privateevent.inject` 不会出现在公开 `tools/list` 中。运行时将事件来源固定为 `external`，外部事件调用方提供 `ref` 作为事件引用。
注入成功的 ACK 不会向调用方暴露事件，也不表示事件已被消费。当前没有事件主动 push；提醒只会随该 Agent 的下一次公开工具调用显示。
不要把裸事件 JSON 写入 daemon 的 MCP stdin；该 stdin 保留给 MCP framing。
公开的 `event.list`、`event.get` 和 `event.mark` 是独立于工具集开关的 Agent 收件箱工具，可供 Standalone、Local、Tunnel 和 HTTP 工具表面使用。
这些工具可选接受 `agentId`，但只允许匹配当前 Agent 的配置身份；此字段不会路由到其他 Agent。Hub Full 调用则必须提供 `agentId`。
同一 Agent 的客户端共享收件箱事件及其显示计数。事件接口的规范行为见
[`接口说明：持久事件收件箱`](interfaces.md#持久事件收件箱)。

`--arguments` 和 `--arguments-file PATH|-` 接受一个 JSON object，上限为 2 MiB。结构化 MCP 结果写入 stdout；日志和
类型化连接错误写入 stderr。运行时停止或重启期间会返回 `local_mcp_unavailable`；客户端可以重新连接，但不得重放有副作用的调用。

## Tunnel、HTTP 与本地工具表面

Normal 与 Room profile 选择 namespace 预设，而不是固定最终运行时表面：Normal 启用 `agent`、`file`、`mcp`、`process`、`skills`、
`tmux` 和 `browser`；Room 在此基础上还启用 `room`。显式设置的
`toolsets.enabled` 是最终依据。逻辑 `room` namespace 包含 `bootstrap`、`bootstrap.read`、语义读取工具以及维护状态/提交工具。
这些过滤器只会从已公布的工具表面移除名称，不会暴露仅供 dispatch 使用的别名。

先调用 `agent.info`，检查当前 profile、已启用的 namespace、受限的路径策略、容量、确认功能是否可用以及 reporting 状态：
```text
process.exec, process.batch, process.read, process.list, process.cancel
skills.list, skills.read, skills.setActive, skills.install,
skills.install.get, skills.install.cancel, skills.run
tmux.sessions, tmux.panes, tmux.exec, tmux.pasteText
browser.manual, browser.acquire, browser.repl, browser.reset, browser.release, browser.list
agent.info, file.read, file.search, file.edit
event.list, event.get, event.mark
```

`event.list`、`event.get` 和 `event.mark` 是独立于 `toolsets.enabled` 的公开事件收件箱工具；
隐藏事件不会因此变为已处理。事件工具只管理收件箱状态，不是进程控制接口。规范说明见
[`接口说明：持久事件收件箱`](interfaces.md#持久事件收件箱)。

启用逻辑 `room` namespace 后，还会公布以下工具名称：

```text
bootstrap, bootstrap.read
room.diary.active, room.diary.read
room.notebook.recent, room.notebook.search, room.notebook.read
room.state.list, room.state.read
room.maintenance.status, room.maintenance.submit
```

相同的九个语义 Room 名称是本地 Unix MCP、Tunnel stdio 和 Hub Full MCP/HTTP 当前合同的一部分。Hub Full 会通过捕获的活动 Room lease 转发
`room.diary.active/read`、`room.notebook.recent/search/read`、
`room.state.list/read` 和 `room.maintenance.status/submit`；对应的 HTTP 路由是
`POST /v1/room/<namespace>/<action>`，且不接受 `agentId`。Coordinator 不会公布或 dispatch 这些 Room 操作。

Room 读取受限且由 Agent 持有：Notebook `limit` 默认值为 20，范围为 1–100；搜索查询不得为空，且最多 256 个 Unicode 字符；
Diary period 是语义 layer/date 值；Notebook/State Markdown 读取会拒绝超过 512 KiB 的内容。`room.notebook.recent` 和
`room.notebook.search` 虽保留原有公开名称，使用的却是当前 Markdown 预览/搜索结果，并非 passage/JSONL 操作。

`room.maintenance.status` 是只读操作。`room.maintenance.submit` 接受 1–5 个互不重复的 slot，可选 `local`/`workflow` mode，
并接受 0–30 的 `waitSeconds`（默认 0）。workflow 等待超时只会结束等待，不会取消维护操作。现有 Agent 路径、锁、clean-tree、
expected-change、executor 和 Git 控制仍具有最终效力；不会添加独立等待 API 或新的确认门。

Hub 只负责认证、活动 lease 路由和有界 run receipt。通用 receipt 可以保留有界操作结果，但不是 Room 内容的权威来源或副本。
已退役的 JSONL 追加/更新/删除及 passage/date-selection 调用方不会被悄然映射为维护操作；应迁移为明确的语义 slot/payload 请求，
或将其移除。历史 release/migration 记录仅供查阅，不是当前错误合同。

托管下游 `mcp.callTool` 与命令和 skill 执行共用同一 process registry 及容量上限。其 `waitSeconds` 默认值为 5，最大为 30；
`timeoutSeconds` 是获准并取得执行槽后的连接/请求截止时间，不含确认及排队等待，默认值为 300，最大为 900。arguments 必须是 JSON object，序列化大小上限为
256 KiB。最多保留 512 KiB 的结果；更大的结果不会截断成部分 JSON，而会以字节数、SHA-256 和最多 8 KiB 的 UTF-8 安全预览表示。
下游返回 `isError=true` 的结果会被保留，process 状态为 `failed`。Hub 没有原生 `file.read` 或 `file.edit` 工具；其通用异步
`mcp.callTool` bridge 使用 process 生命周期，而非带类型的图像内容表面，因此不得依赖它保留 `file.read` 的 image Content blocks。

Skill 安装查询与 skill 执行使用相同的有界等待合同：`waitSeconds` 默认值为 5，最大为 30。等待超时只会结束本地等待，不会隐式取消
安装或 process。确需取消时，必须显式调用 `skills.install.cancel` 或 `process.cancel`。

`mcp.batch` 接受 1–16 个有序调用。每个调用都要在容量准入或确认之前完成完整验证；输入无效或共享 process 容量不足时，不会创建子进程，
也不会启动下游副作用。准入是原子的。之后 batch 会在排除已由临时 allow 状态覆盖的 server 后，请求一次汇总确认。单 server batch
可获 15 或 30 分钟的 server grant；多 server batch 只支持一个作用于整个 batch 的 allow 或 deny。

默认采用并行模式；顺序模式会等每个子调用进入终态后才启动下一个。共享 scheduler 全局最多允许 8 个活动 MCP 子调用、每个 server
最多 2 个；`agent.info` 会报告这些上限及活动/排队数量。`failFast=true` 只会阻止硬失败发生后尚未启动的子调用开始；已经启动的调用
绝不会因此被取消。子结果按输入顺序返回。每次调用的 arguments/results 上限仍为 256 KiB/512 KiB；汇总 arguments 和序列化后的
batch 响应分别限制为 2 MiB。若超出响应预算，会先移除靠后的子结果正文，同时保留 hash、大小、预览、状态和 process id。

MCP 取消使用确切的 rmcp request id。`process.cancel` 和执行超时会发送 `notifications/cancelled`；若 transport 未提供终态取消响应，
Agentic 会报告 `detached` 并附带有界的终止证据，而不会声称已 `cancelled`。子级审计记录包含 `batchId`、可选的
`batchCallId` 和 `batchIndex`；一条汇总审计记录 batch mode、fail-fast、确认结果、子 process id、最终结果和截断信息。确认/审计记录
包含 server/tool 名称、有界的 argument key 子集及其总数、字节数和 hash、配置 revision、结果大小/hash 和终态证据，但绝不包含原始
arguments 或原始结果。

Standalone worker 不接受 Tunnel 命令封套中的 `agentId` 或 `confirmMethod` 字段，意外的旧版字段仍会被拒绝；事件工具另有明确的可选 `agentId` 输入，且只接受匹配当前 worker 身份的值，不会据此选择其他 Agent。worker 使用已配置的本地 Agent 身份处理调用。`bootstrap` 仅限 Room。托管 process 准入工具（`process.exec`、`process.batch`、`skills.run`、`mcp.callTool` 和
`mcp.batch`）接受可选且经过校验的可读 `group`；batch 子项继承父项的 group。`process.exec` 使用必填原始 Bash 脚本 `command` 和可选 `cwd`；`process.batch` 每个元素使用 `command`，可在批次级提供 `cwd` 并由元素覆盖。旧 `program`/`args`/`workingDirectory` 不是可接受的执行输入。响应保持精简，并将 `processId` 作为后续查询状态、输出、
结果或取消的稳定句柄。丰富但有界的 provenance 保留在内部/持久记录中，不会在每个响应里重复。

Shell 启动配置 `shell.initFile` 的配置方式和 `Default`/`Disabled`/显式路径语义见[配置说明](configuration.md)。未设置时使用 `~/.agentic_gpt/.bashrc`；`null` 禁用 init，字符串指定文件路径（相对路径以请求的 `cwd` 或默认工作目录为基准，不从 `PATH` 查找）。默认文件不存在（包括 dangling symlink）时跳过；显式路径不存在或任一文件打开、读取、source 错误会以 `shell_init_file_failed` 阻止命令。除这一默认文件外，不自动读取用户 `~/.bashrc`，也不隐式加载 `BASH_ENV`/`ENV`。init 在与命令相同的 sandbox 可见性和挂载中执行，init 后会重置工作目录；不创建文件或回滚 init 已产生的效果。模型调用不能设置 `initFile`。

init 文件是本地可信配置，不经命令白名单审计，也不绑定之后实际执行的可执行对象。它可修改 PATH、定义函数/别名或设置 exports/变量声明，改变后续命令解析/执行；非交互 Bash 默认不展开别名，若 init 需要别名展开，须自行启用 `expand_aliases`。相对路径若置于模型可选择或可写的位置，尤其需要审慎信任。优先使用用户所有、受保护的绝对路径；这是一项运维建议，不是额外运行时强制策略。已准入进程冻结配置快照，但不冻结 init 文件内容或可执行文件身份。

普通命令以非登录、非交互 Bash（`--noprofile --norc`）执行原始脚本；初始化后启用 `pipefail` 并执行 `set +e`，不会自动启用 `set -e`。策略仅分析用户提交的脚本，不分析 init 文件或 init 改写后的执行对象。退出码 0 为 `completed`，非零为 `failed` 并保留实际 `exitCode`，不会因非零状态合成 init 错误。`waitSeconds` 只限制本次响应等待，不会取消进程；执行终态与 stdout/stderr 捕获 EOF 独立，读取仍受原有游标、UTF-8/Base64、gap、hasMore/eof 和响应预算约束。

`process.cancel` 请求管理整个普通进程组：向仍存活的组发送 TERM，等待后必要时 KILL；已退出的组长不会让仍存活的组成员丢失取消/容量跟踪。只在观察到停止信号时报告正面证据 `process_group_sigterm_observed` 或 `process_group_sigkill_observed`；未验证/脱离或无响应不等于停止。范围包括普通同组管道/后台子进程，不保证 `setsid` 等方式脱离进程组的后代，也不提供 cgroup 级保证。仍有存活组成员的任务继续占用执行容量。

事件收件箱持久历史保存在独立的 `events.sqlite3` store 中，不与 `process.sqlite3` 进程历史混用；`event.list`、`event.get` 和
`event.mark` 只管理事件收件箱，不控制或取消进程。
Process 历史保存在每个 Agent 的私有 `process.sqlite3` store 中，保留 30 天并受逻辑软上限约束。`process.read` 的 wait 默认值为 5，最大为 30；显式传入 0 时立即观察。默认 `view` 为 `auto`：有 backlog 时立即返回，否则有界等待；`view: "status"` 只等待执行终态/期限。
`process.read` 返回紧凑状态与捕获输出：报告 `captureStatus`，stdout/stderr 带偏移、`gap`、`eof` 和 `hasMore`；hasMore 不要求读完整日志。cursor 非消费且不共享，只适用于 command/skill 输出；MCP CallToolResult 使用独立 `mcpResult` 状态。
读取预算由 `limits.processResponseBytes` 控制，默认 8192 字节，范围 4096..1048576；read 可显式覆盖。预算针对序列化响应 JSON，不含传输/event 封套，并与 MCP 结果 512 KiB 保留上限分离。MCP 结果状态为 `pending`、`included`、`deferred`、`unavailable`、`not_retained`；完整 CallToolResult 不切碎，`not_retained` 不可恢复。
若 preflight、策略、确认或容量检查失败，batch 准入仍会在启动任何子项之前拒绝整个 batch。

### 存储权威与恢复边界

私有 Agent process 数据库（`process.sqlite3`）会保留 30 天的 process 历史，并受逻辑软上限约束。数据库以 `0600` 权限创建在私有的
`0700` 父目录下。Process registry 和热缓存只是投影；Agent 重启后，活动 process 会表示为 `unknown_after_restart`，不会为了重放副作用而再次执行。
历史到期或 Hub cache 淘汰都不会回滚 process、MCP 调用或其他外部效果。旧 `jobs.sqlite3` 文件保持原样，不会被打开、迁移或用作 process 历史。

Process 和 MCP batch 准入记录会先在一个 SQLite transaction 中提交，然后子项才进入实时 registry。持久化失败不会留下部分 batch 准入，也不会启动子项；
已有 process 历史会保留。这保证的是准入原子性，而不是事务式执行，也不保证 batch 开始后可以回滚外部效果。


可靠 Hub 命令使用 Agent transport ledger 作为本地去重与结果权威。每次 claim 都会进行文件锁定并带有明确 owner；由其他 Agent 所有的记录会被拒绝。
未指定 owner 的旧记录仍属于 `LegacyUnowned`：不会自动 reconcile、执行，也不会用于泄露结果。恢复由操作人员检查保留的原始记录和更新的 owner-bound 证据；
不得删除或替换去重证据来绕过损坏或 ownership 错误。格式错误的 JSON 或被截断的末行会 fail closed，并将原始字节保留在私有 `.recovery` sidecar 中。
压缩时可能在私有 `.backup` 中保留先前的原始 ledger；不得删除或替换这些文件来绕过损坏错误。

替换配置时，会先暂存并同步私有临时文件，再将其 rename 到目标位置。覆盖现有配置前，会先在其 `backups/` 目录中保存私有备份，数量受
`backupLimit` 限制。配置 secret reference 时也使用私有 setup journal。如果 hash、文件类型或 journal 状态不符合预期的变更前/后状态，恢复会因冲突而
fail closed，不会自行选择一侧或覆盖用户数据。手动恢复配置或 secret 前，先停止拥有它的 Agent；绝不要将 secret 值粘贴进命令、日志或支持输出。

Workspace audit JSONL 上限为 8 MiB。轮换会保留当前文件和一个 `.1` 备份，且尽力而为；审计丢失只表明日志有缺失，不能证明操作或结果没有发生。
等待超时、缺少 receipt 或缓存未包含某记录，同样不能解释为远程取消或效果回滚。


Tunnel、HTTP 和本地 Unix 入口不会暴露 Hub 汇总或通知工具。它们在不让 Hub 进入命令路径的情况下，仍使用与 Hub 执行相同的本地策略、路径策略、
确认、审计和托管 process 生命周期。顶层 `mcpServers` block 则不同：它是 `mcp.*` 调用使用的下游注册表，不是入站 listener 定义。

仓库内的[公开工具合同矩阵](tool-contract-matrix.md)为每个 Normal、Room 和 Hub profile 工具记录适用/不适用场景、条件字段、边界、生命周期/失败语义
以及 Standalone/Hub 对等情况。

### Standalone 文件工具

`file.search` 和文本模式的 `file.read` 都是有界 UTF-8 操作。路径可相对于 `workspaceRoot` 指定，也可使用经
`pathPolicy` 授权的绝对路径；工具会在策略检查前解析符号链接，且不会调用 shell 或外部搜索进程。文本读取默认返回内容，可通过
`metadata: true` 附带 metadata，支持包含首尾行的行范围，并会在 256 KiB 响应上限之前停在最后一个完整行。若读取被截断，只返回
`nextStartLine`；超过上限的单行会被拒绝。在 Git 仓库中，搜索默认遵循 Git ignore 规则；返回的匹配/上下文内容最多为 256 KiB，
扫描的文件数和字节数也有限制。

二进制 `file.read` 支持 PNG、JPEG、WebP 和 GIF。读取图像时，顶层 MCP Content 会返回 image Content blocks，JSON 中则带有
`structuredContent` metadata；图像字节不会在 JSON 中重复。静态图像保留 PNG、JPEG 或 WebP 编码；metadata 位于 `image` 中，
包含检测到的 `mimeType`（`image/png`、`image/jpeg` 或 `image/webp`）以及 `width`/`height`。GIF metadata 包含
`sourceMimeType: "image/gif"`、画布 `width`/`height`，以及有序的 `frames` 项；每项包含 `timestampMs`、输出 `mimeType: "image/png"`
和帧的 `width`/`height`。GIF 最多生成 8 帧，按播放时长均匀采样并包括首尾播放端点；时间戳是源播放帧开始的毫秒数。图像/帧解码上限为
16 Mi 像素，GIF 遍历累计上限为 64 Mi 像素。每次 `file.read` 调用序列化后的图像 payload 上限为 8 MiB。其他格式若为有效 UTF-8，仍按文本处理；
否则返回现有的类型化读取错误，不会当作图像处理。

`contextLines` 必须是非负整数，默认值为 0。运行时最大值由
`limits.maxFileSearchContextLines` 指定（默认 5，可配置为 0–100，并显示在
`agent.info.execution.fileSearch.maxContextLines` 中）。超过最大值的请求会被裁剪，而非拒绝。普通搜索响应只返回 `matches`；
发生裁剪时会附上实际生效的 `contextLines` 和有界警告，只有实际发生截断/跳过文件时才会返回相应证据。负数或非整数值会在参数验证时失败。

`file.read` 和 `file.search` 还接受有序的 `requests` 数组，每项最多支持 32 种请求形状。扁平形式和 batch 形式互斥。结构化 batch 的
`results` 保留输入顺序，并为每个请求附上从 0 开始的 `index`；单项失败不会影响其他项。混合图像 batch 的顶层 MCP Content 也遵循相同顺序：
每项的 JSON envelope（若有图像则包含其 metadata）后面紧跟该项 image block；GIF 帧按时间顺序排列。纯文本响应及其现有响应边界不变。Search
batch 除每次搜索自身的限制外，还保留 20,000 个文件和 128 MiB 的汇总扫描上限。

`file.edit` 只接受 `patch` 和可选的 `needConfirm`。
patch 使用 Codex apply-patch 语法，可一次新增、更新、删除或移动多个文件。所有源路径和目标路径都会依据现有路径策略解析，以确定性顺序加锁，
并检查是否为 UTF-8 及是否不超过 8 MiB；所有变更会先暂存并验证，然后才可进行一次可选确认。对于 `Add File`，仅当请求路径通过既有路径策略时，
才可创建缺失的父目录。新建的空父目录会被跟踪；若 preflight 拒绝请求或确认未继续，会清理这些目录。这不会改变 `Move` 行为，也不能消除路径检查中的
竞态。提交前会重新验证源文件快照。第一次实际写入提交开始前发生失败时，不会写入任何文件内容；提交开始后，不保证跨文件回滚。正常成功响应只包含已提交的
请求路径和操作；部分失败会保留有序的状态/错误证据。Diff、解析后的路径、变更行数和 revision 仅供确认与审计使用，不对外返回。

## Hub MCP 配置档

`agentic-gpt-hub serve` 默认使用向后兼容的 `full` profile。profile 在启动时选择，不支持热切换：

```text
agentic-gpt-hub serve --mcp-profile full
agentic-gpt-hub serve --mcp-profile coordinator
```

`AGENTIC_GPT_HUB_MCP_PROFILE` 是等效的环境变量设置。Coordinator profile 恰好暴露以下八个 Hub 原生工具：

- `hub.info`
- `agent.list`
- `hub.run.list`、`hub.run.get`
- `hub.process.list`、`hub.process.status`
- `user.notify.channels`、`user.notify.send`

Coordinator 调用绝不 dispatch Agent 命令。Session 查询只读取当前连接持有的当前/近期快照；保留的 run 记录才是持久历史。隐藏的执行、
session-control、tmux、下游 MCP、skills、bootstrap、diary 和 notebook 工具不会出现在 `tools/list` 中，且会被 `tools/call` 拒绝。

Full profile 保留现有 Hub 执行表面，增加与 transport 无关的 `bootstrap` 和 `bootstrap.read` 名称，保留兼容别名
`room.bootstrap` 和 `room.bootstrap.read`，并包含 Hub 原生的汇总/通知工具。OAuth discovery metadata 会标识所选 profile，但不会公布隐藏工具。

## 配置

本地配置通常位于 `~/.agentic_gpt/config.json`。Standalone 必须配置 `tunnel` block；Hub 和 Local 模式不需要。跨运行时字段的完整说明见
[`configuration.md`](configuration.md)。规范的 skill block 位于顶层 `skills`。仅当顶层 `skills` 缺失时才读取旧版 `room.skills` block；
若两者同时存在，则顶层值优先，后续配置写入会序列化为规范的顶层 block。

最小 Standalone 配置使用 secret reference，而不是 secret 值：

```json
{
  "tunnel": {
    "tunnelId": "tunnel_<assigned-id>",
    "apiKey": "file:/home/me/.agentic_gpt/secrets/tunnel-api-key",
    "client": {
      "version": null,
      "cacheDir": "~/.agentic_gpt/cache/tunnel-client",
      "autoDownload": true,
      "executable": null,
      "downloadUrl": null,
      "sha256": null
    },
    "hubReporting": {
      "enabled": false,
      "detail": "metadata"
    }
  }
}
```

确认渠道以有序数组的规范形式序列化：

```json
"confirmationProvider": {
  "channels": ["freedesktop", "ntfy"]
}
```

旧版标量/对象形式（`hub`、`freedesktop-then-hub`、`freedesktopThenHub`、`default` 和
`{ "provider": "..." }`）仍可读取且行为不变；由 Agentic 管理的写入会使用规范的 `channels` 形式。`ntfy` 是准确的渠道名称；
通知发布、callback token、pending 状态和决策转发仍由 Hub 管理。

活动 process 上限既可设置为自适应值，也可设置为明确的整数：

```json
"limits": {
  "maxActiveProcesses": "auto"
}
```

`auto` 会在 worker 启动时，以及每次有效的实时 limits reload 后，解析为
`clamp(ceil(availableParallelism * 1.5), 6, 24)`。现有数值保持显式设置，不会迁移。容量拒绝时返回
`max_active_processes_reached`，并附带有界的 `active`、`requested` 和 `limit` 详情；batch 准入仍为原子操作，要么全部接受，要么全部拒绝。

有破坏性的 v0.9 迁移指南属于历史记录，不是当前执行接口。当前生命周期工具表面为 `process.exec`、`process.batch`、`process.read`、`process.list` 和 `process.cancel`，HTTP 路由为 `/v1/process`、`/v1/process/{processId}/read` 与 `/cancel`。旧版
`job.*` 工具名称和 `/v1/jobs/*` 路由不是当前 API。此前的 `process.status`、`process.output`、`process.result` 及其旧 HTTP 路径已移除；旧 wire/http 工具不执行。存储中的旧 receipt 证据仍保留，用于历史/去重边界，但不会退休后重新执行。

当前的多文件变更边界见 file 合同矩阵：一份完整的 apply-patch 请求会先暂存和验证，然后才进行可选确认及提交。

Standalone、Local 或已连接 Hub 的 Agent worker 运行期间，会轮询 `policy`、`limits`、`mcpServers`、`toolsets.enabled`、`events`，以及在
`workspaceRoot` 未变时的 `pathPolicy` 变更；这些配置会经过完整验证，再原子应用于新的准入、调用和工具发现。MCP server id 只能使用
`A-Z`、`a-z`、`0-9`、`.`、`_` 或 `-`，最长 64 字节；`streamable-http` 要求绝对 HTTP(S) URL，可选结构化 Bearer auth；
`stdio` 要求非空 command，且拒绝 HTTP auth。无效的配置版本会保留最后一个有效的实时配置子集。已准入的 process 和 skill installation
继续使用准入时的配置快照；`events` 配置变更仅用于后续准入，不会改写已准入工作的决定。

由启动时身份决定的配置包括 workspace root、Room 设置、Browser 配置、tunnel/client、reporting connection 及 skill-install 并发设置；
这些字段变更后必须重启。共享 watcher 会记录 `config changes require restart; fields=...`；Standalone supervisor 还会发出
`restart_required`，Hub 模式则没有 supervisor 事件。若 `workspaceRoot` 发生变化，原 workspace root 和 `pathPolicy` 会作为一个原子组合保持生效，
直到重启；只有 root 未改变时，才可单独 reload path policy。实时启用 `room` namespace 时，会依据当前实时 Room 配置执行 bootstrap；磁盘上需重启才生效的
`room.*` 变更不会在重启前改变运行时 root。`agent.info.mcp` 仅报告生效的配置 revision、已配置/已启用计数和 client 生命周期，不会暴露 endpoint。

Standalone worker 也会监视并协调 `httpMcp` 子集。启用或禁用 endpoint、变更 bearer-token reference 或其解析后的内容，以及变更 `host`、`port`、
`publicUrl` 或 `allowHosts`，都无需重启 worker。更换 public origin 或监听器会建立新的监听器身份：会关闭有状态 HTTP session，并丢弃监听器本地
OAuth code 和 token。替换绑定地址不同时，会先绑定新监听器再退役旧监听器。同一地址上的 `allowHosts` 或 `publicUrl` 变更则必须先退役旧监听器，
再绑定新监听器；若新绑定失败，旧监听器已不可用，后续重试会重新启动。若不同地址发生 bind 冲突，只要旧地址仍可用，就会保留正常工作的旧监听器，
并在不中断 tunnel 或 Unix 执行的情况下重试。变更 token reference 或其解析出的内容，会在不重新绑定的情况下更新内存中的 direct bearer 认证器、
保留现有 session，并原子撤销 OAuth 记录。若无法解析 reference，HTTP 认证会 fail closed：监听器停止监听和接受请求，直到凭据重新可用。
无效候选配置会保留上一个有效的实时配置和状态；日志或摘要中不会记录或包含 reference、token、code 或 access-token 值。

`apiKey` 仅接受 `env:NAME` 和 `file:PATH`。解析出的值会作为 `CONTROL_PLANE_API_KEY` 注入 tunnel-client 子进程环境；
不会将其放入 argv、日志、配置摘要、报告、生成的 runtime 文件或备份。文件 reference 末尾可有一个 LF 或 CRLF，读取时会将其移除。
空值和明文 reference 会导致启动失败。

使用 secret manager 或单独受保护的文件提供 secret。以下示例只配置文件路径，不会把密钥写入 shell history 或 argv：

```bash
touch "$HOME/.agentic_gpt/secrets/tunnel-api-key"
chmod 600 "$HOME/.agentic_gpt/secrets/tunnel-api-key"
read -rsp "Tunnel API key: " AGENTIC_TUNNEL_API_KEY
printf '\n'
printf '%s' "$AGENTIC_TUNNEL_API_KEY" > "$HOME/.agentic_gpt/secrets/tunnel-api-key"
unset AGENTIC_TUNNEL_API_KEY

agentic-gpt config set tunnel.tunnelId tunnel_<assigned-id>
agentic-gpt config set tunnel.apiKey file:"$HOME/.agentic_gpt/secrets/tunnel-api-key"
agentic-gpt config set tunnel.client.autoDownload true
```

`env:NAME` 形式适用于 service manager 的受保护环境或注入的 secret，例如 `env:AGENTIC_TUNNEL_API_KEY`；不要在 shell transcript 中使用
包含字面值 `AGENTIC_TUNNEL_API_KEY=<secret> ...` 的命令。

受支持的 `config set` 键包括：

- `tunnel.tunnelId`, `tunnel.apiKey`.
- `tunnel.client.version`, `tunnel.client.cacheDir`,
  `tunnel.client.autoDownload`, `tunnel.client.executable`,
  `tunnel.client.downloadUrl`, `tunnel.client.sha256`.
- `tunnel.hubReporting.enabled`, `tunnel.hubReporting.detail`.
- `httpMcp.enabled`, `httpMcp.host`, `httpMcp.port`, `httpMcp.publicUrl`,
  `httpMcp.bearerToken`, `httpMcp.allowHosts`.

Tunnel 身份、secret reference、client 来源/版本/hash/cache、Browser 配置和 CLI profile 都属于启动身份。Standalone supervisor 运行期间，
修改这些值只会记录包含变更字段名称的 `restart_required`；Browser 变更使用 `browser` 字段名，不会切换现有子进程树，也绝不会打印 secret 值。
`toolsets.enabled` 属于实时配置，不需要重启。

## 隧道客户端的信任与来源选择

未设置 executable override 时，release manifest 会为受支持的 Linux 目标固定 OpenAI tunnel-client `v0.0.10`：

| 平台 | 资源文件 | Archive SHA-256 |
| --- | --- | --- |
| `linux-amd64` | `tunnel-client-v0.0.10-linux-amd64.zip` | `b9e0388a343f2d7adeff3992f411a0bd3d916a64bc56534aac5fd15ac1b20cd5` |
| `linux-arm64` | `tunnel-client-v0.0.10-linux-arm64.zip` | `b842a9b2352eebd80514cf01a1fbb1c0d400a7d24a4015e85a7ea5f1aeaa5b30` |

`version: null` 表示使用内嵌 pin。显式指定的版本必须存在于 manifest 中。不支持的平台会在访问网络前失败。Agentic 会在解压前验证 archive，
只接受一个名为 `tunnel-client` 的普通文件；会拒绝路径穿越、符号链接、设备文件和重复布局，并通过私有 cache 与原子替换完成安装。

来源优先级如下：

1. `client.executable`：使用本地可信 executable；若提供可选的 `sha256`，每次启动都会校验。
2. `client.downloadUrl` 加 `client.sha256`：使用精确的 HTTPS archive URL，必须提供 archive digest，HTTPS 重定向次数受限。
3. 托管 manifest/cache：使用固定的 URL 和 digest。`autoDownload: false` 时，必须已有经过验证的 cache artifact。

托管资源身份包含版本、平台和 archive digest，因此自定义与官方 artifact 不会冲突。默认 cache 位于
`~/.agentic_gpt/cache/tunnel-client`。

## 可选 Hub 报告

Reporting 默认禁用，且与 Tunnel 命令执行相互独立。只有当本地配置已有 Agent 连接所需的 Hub 身份
（`hub.url`、`hub.transport`、`agentId` 和 `hub.agentSecret`）时，才启用 reporting：

```bash
agentic-gpt config set tunnel.hubReporting.enabled true
agentic-gpt config set tunnel.hubReporting.detail metadata
```

Reporting connection 会标明自身为 `reporting-only`。它可以发送 hello/heartbeat、direct-run 生命周期事件、process 快照和现有确认流量，
但绝不接受 Hub 执行 envelope。Hub 会在创建 run 之前拒绝 reporting-only Agent。Reporting 断连、队列丢弃或 Hub 不可用都不会延迟或改变本地 MCP 结果。

`metadata` 级别会记录工具/来源/profile/状态/时间戳/持续时间、标识符、退出码和有界的失败原因；不包含 arguments、results、程序 argv、工作目录或
stdout/stderr。`full` 还会保存有界的 JSON arguments/results 和有界的现有 process 快照；超大值会记录字节数与 SHA-256 的截断信息，而非部分 JSON。
Direct-run 记录在 Hub 中保留 24 小时。

每次工具调用和托管 process 终态事件也会在 stderr 写入有界生命周期记录。这些记录包含 run/tool/profile 和状态；如果有持续时间及安全的 12 位十六进制
run/process 标识符，也会一并记录。内联结束的调用只写一条最终记录；返回活动状态的调用先写一条响应记录，之后再写一条终态记录。记录中绝不包含
arguments、results、路径、secret 或 process 输出。Reporting connection 的连接状态转换会另行记录为 connected/disconnected，并标明所选 transport。

## 健康状态、日志、重启与恢复

对于 Agent id `laptop`，supervisor 使用以下私有 runtime 目录：

```text
~/.agentic_gpt/runtime/tunnel/laptop/
├── health.url          # 临时的 loopback 就绪 URL
├── tunnel-client.log   # 供诊断使用的结构化子进程日志
└── tunnel-client.pid   # 临时子进程 PID 标记
```

supervisor 会在启动第一个子进程前运行 `doctor --json`，等待 loopback 最多 45 秒以确认就绪，并将子进程输出转发到 Agentic stderr。
转发内容会带组件前缀并脱敏 secret；已知的 INFO/WARN/ERROR 级别会保留。未知级别的子进程 stdout 按信息级别处理，stderr 按警告级别处理。
在 journald 下，Agentic 会省略自身内部时间戳；前台日志则保留一个完整时间戳。正常或失败清理都会删除 health URL 和 PID 标记，但保留结构化日志。

`tunnel_doctor_spawn_failed` 或 `tunnel_client_spawn_failed` 表示无法启动子进程；错误附带 OS 错误种类、原始错误码和错误消息。排查时保留这些细节，以区分文件不存在、权限或其他启动错误，不应仅凭错误前缀判断原因。

子进程意外退出或就绪失败时，最多重试 5 次，间隔依次为 1/2/4/8/16 秒。连续就绪 60 秒会重置失败计数。配置/reference 错误、不支持的平台、
缺少本地 executable、checksum 失败，以及 tunnel authentication/authorization 失败，都会作为永久启动失败处理。收到 SIGINT/SIGTERM 时会停止
tunnel 进程组和 worker，随后使用有界的 kill fallback。

tunnel control-plane 的逻辑连接可能比重启后的 stdio 子进程存活更久。新的 worker 在新的 MCP `initialize` 之前收到非 ping 请求时，只有
tunnel stdio transport 会通过私有 handshake 恢复 rmcp 的本地初始化状态；它会抑制该私有响应，然后以原 request id 重放原请求。来自旧逻辑连接的
initialize 前通知会被忽略。普通的客户端发起初始化会原样转发；仅所有者可访问的 Local Unix 入口不使用此恢复 shim。恢复成功时会生成有界诊断
`mcp_stdio_session_resume` 和 `mcp_stdio_session_resumed`；两者都不包含请求 arguments 或 results。

恢复检查清单：

1. 阅读 Agentic stderr 诊断和保留的 `tunnel-client.log`；检查日志时绝不要打印 API key。
2. 查看 `agentic-gpt config show` 输出中的 reference 和非 secret 来源摘要，不要读取解析后的值。
3. 使用托管 client 时，检查 cache 路径；除非有意进行离线配置，否则保留 `autoDownload: true`。
4. 使用 override 时，确认文件是普通 executable，并核对配置的可选 digest 是否对应预期二进制文件。
5. 向服务提供方检查 tunnel/control-plane 状态。身份、profile 或 client 分发设置变更后应重启。
6. 若只有 reporting 失败，可暂时禁用 `tunnel.hubReporting.enabled`；本地 Tunnel 执行不受影响。

安全诊断使用 `agent.list`；查看保留的运行历史使用 `hub.run.list`/`hub.run.get`，查看缓存的 metadata 使用 `hub.process.list`/`hub.process.status`。

## 可选的集中式 Hub 模式

若集中路由、Actions、历史记录或 reporting 的价值足以覆盖额外共享基础设施的成本，仍可选择 Hub 模式：

```bash
agentic-gpt-hub init
read -rsp "Agent secret: " AGENT_SECRET
printf '\n'
agentic-gpt-hub agent add \
  --agent-id laptop \
  --display-name my-laptop \
  --secret "$AGENT_SECRET"
unset AGENT_SECRET
agentic-gpt run
```

Full Hub MCP profile 仍为默认值。仅当 Hub 实例只用于状态/历史/通知访问时，才使用 coordinator：

```text
agentic-gpt-hub serve --mcp-profile coordinator
```

若 ChatGPT 可以访问 Hub，请让 Hub 位于 HTTPS 后。现有 Actions 路由和 WebSocket/SSE Agent transport 仍可使用；Standalone reporting connection
是额外的、仅用于报告的模式，不能替代支持命令的 Hub Agent。

Hub 的 `agent add` 不提供隐藏式交互输入；`--secret` 必须作为 argv 参数。以上 `read -s` 做法可避免将 secret 写入 shell history，但变量展开后，
secret 仍可能出现在本地进程检查中。请只在可信机器上执行，并在操作后清除变量。

## 验证范围

自动化检查覆盖配置/reference 验证、worker 工具列表精确性、secret 的 argv/environment 隔离、可信资源/cache 行为、supervisor 重启与进程树清理、
reporting 隐私/幂等性，以及 Full/Coordinator MCP 工具表面。仓库的多目标 release 脚本为
[`scripts/dist-linux.sh`](../scripts/dist-linux.sh)；安装了 `cross` 和对应工具链时，该脚本会构建两个 Linux 目标。

`crates/agentic-gpt/tests/standalone_supervisor.rs` 会启动实际的 Agentic supervisor 和隐藏 stdio worker，对 Normal profile 执行
initialize/list/call smoke。它还会启动隐藏 worker，并将旧的 initialized notification 和一个工具调用作为首个请求，以证明重启恢复可以让 worker 保持运行、
隐藏私有 handshake，并接受后续调用。对应的 Room profile 由进程内测试覆盖。两者共同验证 stdout 仅包含 MCP framing，且精简后的 Normal/Room 工具表面可调用。
这些检查仍不等同于调用真实 Secure MCP Tunnel control plane：真实调用必须启动外部 connector 并返回本地 Agentic 工具结果，而不只是通过 `/healthz`、
`doctor` 或本地伪造的交接。外部凭据/环境前置条件应与仓库测试分别记录。
