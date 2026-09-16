# Hub 调查证据

## 1. 覆盖范围与实际职责

### Hub 内部模块

- `main.rs:117-298`：解析 CLI/env，选择 `hub.sqlite3` 与 `hub.json`，初始化 `HubState`，建立 Axum listener 和后台循环。实际路由覆盖 `/v1/info`、Agent WS/SSE、`/v1/process/*`、Jobs、tmux、下游 MCP、notify、Room notebook/bootstrap/skills，以及 `/mcp` 和 OAuth 元数据/授权/token。`main.rs:300-700` 负责远程确认请求、ntfy action URL、allow/deny 回调和 pending 清理。
- `state.rs`：`HubState` 集中持有 `api_key`、`Arc<StdMutex<Connection>>`、`agents`、`pending`、`pending_confirmations`、`jobs`、`boot_generations`、`active_room`、OAuth maps 和 ntfy health cache。它是实际的跨模块所有权边界，但各模块直接锁这些 map，没有额外 service/repository facade。
- `db.rs:1-119`：创建 SQLite 父目录和连接；schema 是 `agents`、`notification_endpoints`、`agent_runs`，并通过 `ensure_column` 进行增量兼容列补齐。当前没有 schema version 表、外键或除主键/alias unique 之外的明显生命周期索引。
- `registry.rs`：Agent add/alias/enable/disable/remove/list；secret 仅以 SHA-256 hash 存入 `agents.secret_hash`；读取时暴露启用状态、能力和 last-seen，路由不会把 hash 返回给客户端。
- `agents.rs:30-648`：验证 per-agent secret，接入 WS/SSE，维护当前 `AgentConnection`，处理 Hello/Heartbeat/JobUpdate/RunReport/Response/TransportAck/TransportRunStatus/ConfirmationRequest，负责 `request_agent`、可靠 envelope 重放、连接替换、Room 释放及 60 秒左右的过期清理。
- `runs.rs:15-523`：`AgentRun` 是 Hub 的持久运行收据。`prepare_run` 把完整 command JSON 和 hash 落库；后续写入 dispatched/acked/status/timeout/result/report；同 hash result 可幂等，冲突写入 `conflict_json`；24 小时后 prune，约 70 秒无结果的 acked/started/running 标记为 `unknown`。
- `routes.rs:139-797`：外部 HTTP action API。`require_action_auth` 用 constant-time Bearer API-key 比较；process、job、tmux、MCP routes 通过 `request_agent` 执行或查询，并把等待/超时映射成 operation-specific HTTP error。Jobs 在 Agent 不可用时可从 Hub 内存 cache 降级。
- `mcp_server.rs:52-600,619-1827`：独立 JSON-RPC `/mcp`，处理 initialize、tools/list、tools/call、batch/ping。`Full` 暴露完整工具；`Coordinator` 只允许 `hub.info`、`agent.list`、`hub.run.list/get`、`hub.job.list/get`、`user.notify.channels/send`。工具描述包含 read-only/open-world/destructive 等注解，但实际执行仍由 native method 或 Agent 转发完成。
- `room.rs:33-479`：Hub 只管理一个全局 active Room Agent（`agent_id + connection_id`），Normal 连接会释放该 Agent 的 Room 资格，Room 连接会争抢全局槽位；Room HTTP/MCP wrapper 最终调用 `request_active_room -> request_agent`。Hub 没有 Room 文件、Diary、Notebook 数据库。
- `notify.rs:89-502`：枚举在线 Agent 的 freedesktop 渠道、Hub ntfy 渠道和明确不可用的 Android 占位渠道；Agent/ntfy 投递分别走 Agent command 或 HTTP POST。Android 注册会持久化 endpoint/token hash，但没有后续投递协议。
- `oauth.rs:80-405`：MCP OAuth discovery、授权码页面/提交、PKCE S256、token exchange 和 Bearer 校验。OAuth code/token 仅在 `HubState` 内存 map，`is_valid_mcp_bearer` 同时接受 Hub API key 或 OAuth token。
- `agentic_result.rs`：将 native JSON 同时放入文本和 `structuredContent`；顶层含 `error` 时设置 MCP `isError`。
- `instance_lock.rs`：`serve` 按数据库路径持有 `<db>.serve.lock`，保证单进程占用。
- `utils.rs`：ID/token 随机生成、SHA-256、不同长度也安全的 constant-time equality。

### 跨层实际边界

`crates/agentic-gpt-protocol/src/lib.rs` 定义所有 `HubCommand`，包括 process、Jobs、tmux、MCP、notify、旧 Room notebook/diary、bootstrap 和 skills；`HubCommandEnvelope` 带 `event_id/run_id/request_id/command_hash`。Hub 不执行 shell、tmux 或下游 provider：`crates/agentic-gpt/src/hub.rs` 接收 envelope，交给 `local_service::dispatch`；Agent 的本地策略、executor、Job 状态和结果回报才是实际执行侧。

Room 的当前持久化事实在 Agent 侧：`room_reads.rs` 读取受仓库根目录约束的 Diary/Notebook/State；`room_repository.rs` 检查路径、拒绝 symlink 越界、保证 Git/schema/scaffold；`room_maintenance.rs:submit` 在 `room_repository_writes` 锁下写入 Daily/weekly/monthly Diary、Notebook 或 State entity。因而不能把 Hub 的内存 `active_room` 或旧 JSONL 命令误认成 Diary/Notebook 的持久化所有者。

未发现 Hub/protocol 中的 reminder/task 产品模型、scheduler、提醒路由或持久化表；代码中的 `task` 主要是 Tokio task 变量，`Job` 是执行生命周期。提醒/任务调度若未来需要，应作为独立资源/调度域，不应为了本次受控执行基础设施审计而塞进 Hub。

## 2. 主要调用与状态生命周期链

### 链 1：HTTP process/Job/tmux/MCP 执行

1. 客户端调用 `routes::process_exec`、`process_batch`、`cancel_job`、tmux route 或 MCP native method；`routes.rs:727-797` 的 `require_action_auth` 验证 Hub action Bearer key，执行类 route 还检查 registry 中 Agent enabled。
2. route 生成/保留 request id，调用 `agents::request_agent(state, agent_id, command, timeout)`。该函数先确认连接存在、`hello_received` 且 `CommandCapable`，再 `runs::prepare_run` 写入 `agent_runs`，把 oneshot 放进全局 `pending[request_id]`，封装 envelope，经当前连接的 mpsc sender 发送，并标记 dispatched。
3. Agent `hub.rs::connect_loop/handle_reliable_envelope` 用 `transport_ledger::accept` 判断首次、重复 started 或已完成；首次发送 TransportAck，标记 started，调用 `local_service::dispatch`。真正的 process、Jobs、tmux、MCP 与通知操作都在 Agent。
4. Agent 回发 JobUpdate/TransportRunStatus/Response；Hub `agents::handle_agent_message` 更新内存 Job cache、`runs::mark_*`/`store_result`，再唤醒 pending oneshot，HTTP/MCP 得到结果。结果可能是业务层 `{error:{...}}`，Agent transport 仍视为成功响应。
5. Hub 的等待时间是 operation timeout（process 35 秒，Job get 最多约 32 秒，tmux 某些操作 65 秒等），并不等价于停止远端 Job。超时会把 run 标为 `timeout_waiting_result`，Agent 仍可能继续执行并稍后回传结果。

典型状态为 `agent_runs: created -> dispatched -> acked/started/running -> completed`；等待超时进入 `timeout_waiting_result`，Hub cleanup 对长期无结果的部分运行标为 `unknown`。Agent 侧 ledger 为 `accepted -> started -> completed`，重放相同 `run_id/request_id/hash` 时返回已有结果或跳过重复执行。

### 链 2：`/mcp` OAuth/JSON-RPC/工具调用

1. `/mcp` 先经过 `main.rs` 的 `require_auth_on_mcp_path`；`oauth::is_valid_mcp_bearer` 接受 Hub API key 或内存 OAuth token，否则返回 metadata challenge。
2. `mcp_server::mcp_post` 解析单个 JSON-RPC，处理 initialize、tools/list、tools/call；`call_app_tool` 先按 profile 过滤工具，再 decode 参数，进入 native method。工具 descriptor 与 dispatcher 共用 `allows_tool`，Coordinator profile 不会把执行/Job-control/tmux/Room/skills 工具暴露给调用者。
3. Hub-native 查询（info、Agent list、run list/get、cache job list/get、notify）直接读 Hub registry/SQLite/cache 或调用 ntfy；执行、Job-control、tmux、下游 MCP 调用再次使用 `request_agent`，因此沿用链 1 的 run/ack/replay/timeout 生命周期。Room/skills 则使用 active Room Agent 转发。
4. `agentic_result.rs` 把所有 native 返回转成 MCP tool result；业务失败多以结构化 JSON 返回，而不是 JSON-RPC transport error。这使调用方能看到 operation code，但也要求客户端区分 MCP 调用成功与业务 `error`。

### 链 3：Agent WS/SSE 连接与代际生命周期

1. Agent `hub.rs::connect_loop` 使用 Agent secret 连接 `/v1/agents/:id/connect`（WS）或 `/events`（SSE），发送 Hello，声明 `Normal/Room`、`CommandCapable/ReportingOnly`、boot generation、能力摘要和通知渠道。Hub `connect_agent/connect_agent_sse` 用 registry 的 enabled + secret hash 验证。
2. `replace_agent_connection` 写入新的随机 connection id，并关闭旧 sender；Hello 由 `handle_agent_message` 更新角色、模式、boot generation。Room role 会注册全局 active Room；ReportingOnly 会释放该 Agent 的 Room 资格。boot generation 变化触发 `mark_cached_jobs_unknown_after_restart`。
3. WS 入站在 socket reader 中直接交给 `handle_agent_message`；SSE 的 POST `/messages` 对 stale connection 拒绝非可靠消息（JobUpdate 等），但 Response/TransportAck/TransportRunStatus 被定义为可靠消息，可从旧连接补交。Heartbeat 更新 last-seen，Hub 每 15 秒 sweep，last-seen 超过约 60 秒移除当前连接。
4. 断开当前连接时释放 Room、丢弃该 Agent 未解决 confirmations；重新 Hello 后 `pending_unacked` 查询 DB 中未 ack/未完成的 `created/dispatched/timeout_waiting_result`，重新发送可靠 envelope。Agent ledger 在本地重启后 reconcile accepted/started/completed 记录。
5. Hub 重启只保留 SQLite runs/registry；`pending` oneshot、Job cache、active Room pointer、confirmation map、OAuth code/token 均丢失。已有 DB run 可被新连接 replay，但原 HTTP waiter 已不存在，不能恢复原请求的同步等待。

### 链 4：远程确认

1. Agent `confirmation.rs` 在本地策略判定需要授权时生成 `ConfirmationPayload`，发送 `AgentMessage::ConfirmationRequest`，把本地 pending waiter 与 request id 关联。
2. Hub `main::handle_confirmation_request` 在启用 remote confirmation 且 ntfy 配置有效时生成 confirmation id/token，仅将 token hash 和预览/风险/过期时间写入内存 `pending_confirmations`，然后 POST ntfy。
3. ntfy action URL 命中 `/v1/confirmations/:id/{allow|deny}`；Hub 校验 token hash、过期和 resolved 状态，记录 Decision，再向该 Agent sender 发 `HubMessage::ConfirmationResponse`。正常路径由 Agent waiter 返回 allow/deny 给本地执行器。
4. 2 秒 cleanup 会将过期项标为 Timeout；Agent/Hub 断线则可能在不同层分别超时。该链不把确认写入 SQLite，重启后 Hub 无法恢复 pending confirmation。

### 链 5：Room active 路由与实际文件写入

1. HTTP `room.rs` wrapper 或 MCP `room.*` 工具先执行 action auth，调用 `request_active_room`；它要求内存 active pointer 的 agent/connection 仍匹配当前 map、连接是 CommandCapable 且 role=Room。
2. 通过后包装 `HubCommand::RoomNotebook*`/`RoomDiary*`/bootstrap 等交给 `request_agent`。Hub 只做路由和错误映射，不打开 Room 文件。
3. 当前 Agent 的 `local_service.rs` 对所有 `RoomNotebookAppend/Recent/Select/Search/Current/Update/Remove` 和 `RoomDiaryAppend/Recent/SelectExact` 返回 `room_legacy_surface_removed`。同一 Agent 的 `stdio_server.rs` 当前表面是 `room.diary.active/read`、`room.notebook.recent/search/read`、`room.state.*` 和 `room.maintenance.status/submit`；写操作通过 `room_maintenance::submit` 进入真实 Git 仓库。
4. 因此 Hub 的旧 Room route/tool 在真实当前 Agent 上会产生结构化 legacy error，而不是写入 Diary/Notebook；现有 Hub Room 单测只足以证明 fake connection 的 routing/active-room 逻辑，不能证明跨进程 live parity。

## 3. 持久化、所有权、安全与部署约束

### 持久化分层

| 层 | 实际内容 | 重启行为/约束 |
|---|---|---|
| Hub SQLite `agent_runs` | 完整 command JSON、command hash、run/request/agent、ack/status、result/hash/conflict、reason、时间和部分 Job 字段 | 24 小时后 prune；可用于可靠重放和历史查询；原始 command/result 可能含敏感参数，文件权限/备份必须按秘密数据处理 |
| Hub SQLite `agents` | Agent identity、alias、enabled、secret hash、capabilities、last_seen | registry 持久；没有 FK/版本迁移框架，兼容列靠 `ALTER TABLE`；同一 DB 由 `InstanceLock` 限制一个 serve 进程 |
| Hub SQLite `notification_endpoints` | Android 注册的 endpoint/token hash 等 | endpoint 可持久化，但当前没有实际后续认证/投递/heartbeat 使用路径 |
| Hub 内存 | 当前连接/pending waiter/Job cache/boot generations/active Room/confirmations/OAuth/ntfy health | Hub 重启全部丢失；Job cache 没有 Hub 侧可见的 TTL/上限清理；active Room 必须重新注册 |
| Agent `transport-runs.jsonl` | 每个 Agent 的 run ledger、request/hash、accepted/started/completed/result | 提供跨 Agent 重启幂等依据；当前观察到 append-only，未见清理/压缩策略 |
| Agent Room repository | Diary、Notebook、State、bootstrap/scaffold 和 Git 元数据 | 由 Agent 的路径安全和 maintenance lock 负责；这才是 Room 文档/状态所有权 |
| Hub `hub.json` | Hub config、remote confirmation、ntfy、public base 等 | `write_if_missing` 只在缺失时写，未见热加载；默认位于 `HOME/.agentic_gpt` |

### 信任边界

- 外部 action API 由 `AGENTIC_GPT_API_KEY`/`HubState.api_key` 保护；`/mcp` 的 OAuth shim 与该 API key 实际共享 Bearer 接受路径。API key 具有 Full/Coordinator 之外的 route 权限，不能把 OAuth token 自动理解成细粒度执行授权。
- Agent 接入用每个 registry entry 的 `x-agent-secret`，只允许 enabled Agent。Hello 的 role/mode/config summary 由 Agent 声明，Hub 据此决定是否可投递；`request_agent` 确实拒绝 ReportingOnly 作为执行目标，但入站消息的模式/连接代际校验并不完全对称，见问题清单。
- secret 比较使用 SHA-256 + constant-time equality；列表响应只输出安全摘要。远程 confirmation token 以 hash 存内存，ntfy action URL 携带原 token，回调不要求 Hub action key，因此 callback URL 必须视为 bearer capability。
- Room 仓库边界由 Agent `room_repository` 负责：检查 root/workspace、拒绝 symlink 越界、Git/schema/scaffold readiness；Hub 的 active pointer 只表示当前路由租约，不是文件锁或持久所有权。

### 部署约束

- `serve` 默认 bind `127.0.0.1:8787`，默认 DB/config 是 `HOME/.agentic_gpt/hub.sqlite3`/`hub.json`；同一 DB 不能由多个 Hub 进程同时 serve。
- 跨机器拓扑必须把 Hub 放在可被客户端访问的 HTTPS/reverse proxy 后，并在代理后显式设置 `AGENTIC_GPT_PUBLIC_BASE_URL`/`--public-base-url`。`oauth::public_base_url` 在未配置时会从 forwarded/Host 头推断，生产环境不应依赖未经信任的 forwarded host/proto。
- WS 与 SSE 都要保持 per-agent secret；SSE 的 `/events` 与 `/messages` 通过 connectionId 维持连接代际。Agent 与 Hub 必须使用同一协议语义，尤其是 `run_id/request_id/command_hash` 和 Room toolset 版本。
- Hub 重启不是无损控制面重启：数据库历史还在，但 pending HTTP、in-memory Job projection、confirmation、OAuth session、active Room 会消失；Agent ledger 可避免重复副作用，但不能恢复原同步请求。
- SQLite schema 没有显式 WAL/foreign-key/file-mode/encryption 设置的源码证据；这不是必然错误，但部署必须处理单进程写入、备份一致性、文件权限和静态敏感数据保护。

## 4. 值得保留的结构

1. **单一命令桥**：绝大多数外部执行路径都经过 `agents::request_agent`，避免各 route 自己拼 socket、自己处理 timeout/replay。
2. **明确的 envelope 身份字段**：`event_id/run_id/request_id/command_hash` 加上 Agent ledger，已经具备 at-least-once transport 下的幂等基础。
3. **Hub durable run receipt + Agent durable ledger 的分层**：Hub 保存控制面可查询收据，Agent 保存执行侧幂等事实，职责方向正确；不应改成 Hub 持有 executor 或 reasoning loop。
4. **运行与等待分离的雏形**：Jobs 可在 Agent 继续运行，Hub route 只等待有限秒数；`timeout_waiting_result`、late result、conflict hash 和 stale unknown 为异步控制提供基础。
5. **Explicit mode/profile**：`CommandCapable/ReportingOnly` 和 `Full/Coordinator` 是受控部署与最小权限的有效结构，应继续保留并强化为硬边界。
6. **连接代际与 boot generation 设计**：SSE 已经有 stale 非可靠消息拒绝，disconnect 也有 connection-id 条件删除；boot generation 能把跨 Agent 重启未完成 Job 标成 `UnknownAfterRestart`，方向正确。
7. **Room 所有权拆分**：Hub 只持有 active connection lease，Agent 负责文件路径、Git、schema/scaffold 和写锁；这是避免控制平面吞入长期记忆/文件管理的正确边界。
8. **安全基础**：secret hash、constant-time compare、Agent enabled 检查、OAuth PKCE/redirect allowlist、工具 schema/安全注解和明确 unavailable notification placeholders 均值得保留。
9. **单进程锁与可诊断摘要**：`InstanceLock` 防止同 DB 双写；`hub.info`/`agent.list` 暴露安全状态摘要，有利于运维而不泄露 secret。

## 5. 问题清单（按严重度）

### P1 / 高：Hub 旧 Room surface 与当前 Agent 不可用

- **证据**：`crates/agentic-gpt-hub/src/room.rs:33-150,341-419` 和 `mcp_server.rs:1243-1553` 仍生成/转发 `HubCommand::RoomNotebook*`、`RoomDiary*`；`crates/agentic-gpt/src/local_service.rs:132-141` 对这些分支统一返回 `legacy_room_surface_removed_error`，错误码为 `room_legacy_surface_removed`。当前 Agent 工具在 `stdio_server.rs:1076-1145` 改为 diary.active/read、notebook read/recent/search、state 和 maintenance；真实写入在 `room_maintenance.rs::submit`。
- **机制与影响**：Hub 的 Room HTTP/MCP 请求在 active Room Agent 上确实能完成认证、连接和 envelope 投递，但 payload 到 Agent 后被有意拒绝；所以 fake connection routing 测试不等于真实跨进程 Room 功能，Diary/Notebook 读写合同已经断裂。
- **严重度**：高（对声明存在的 Hub Room API 是直接功能失效）。
- **根因推断**：`[推断]` Agent 已迁移到当前 Room read/maintenance 工具，而 Hub parity/旧 JSONL command 尚未完成同一接口切换；不是 Room 持久化仓库本身应删除的证据。
- **边界建议**：保留 `room_repository`、Diary/Notebook/State 文件、Git/scaffold、maintenance 和当前只读工具；先定义 Hub 到当前 Agent 的明确 adapter/协议，再删除旧 command aliases，避免把基础设施与兼容残留一并删除。

### P1 / 高：入站连接代际检查在 WS 与可靠消息路径不一致

- **证据**：`agents.rs::handle_socket` 对 WS 消息调用 `handle_agent_message(..., true)`；`handle_agent_message` 的 `touch_agent`、Hello 更新、JobUpdate、RunReport 只按 agent id 操作，Hello 更新 `agents.get_mut(agent_id)` 未同步核对 connection id。`replace_agent_connection` 会关闭旧 sender，但旧 reader 的入站路径没有统一 `is_current_connection` 门槛。SSE `post_agent_message` 则会对 stale 非可靠消息返回 409。
- **机制与影响**：连接替换后，旧 WS 在关闭竞态中仍可发送 heartbeat/Hello/JobUpdate/RunReport，可能刷新当前 Agent 的 last-seen、覆写当前连接元数据、写入当前 Job cache 或更新 run history；ReportingOnly 入站也没有在统一 handler 中被拒绝。现有测试 `stale_job_update_is_rejected_without_writing_job_cache` 只覆盖 SSE POST，不能覆盖 WS 旧 reader；`stale_response_with_matching_run_is_accepted` 则明确证明可靠 Response 被设计为允许旧连接补交。
- **严重度**：高（连接状态/Job projection/历史可被旧代际污染；是否可被外部利用还依赖 secret 保密和关闭竞态）。
- **根因推断**：`[推断]` connection id 是后来为 SSE/reliable replay 引入的，但 inbound WS、可靠消息、角色变更没有统一成 `(agent_id, connection_id, mode)` 的 connection handle 校验。
- **边界建议**：将所有非可靠入站绑定当前 connection；可靠消息单独允许旧代际，但只允许与 DB 中相同 `(agent,run,request,hash)` 匹配且不能改变连接 metadata/Job cache；Hello/Heartbeat/RunReport 也必须按代际和 mode 校验。

### P1 / 高：Response waiter 未以 Agent/run 所有权绑定

- **证据**：`agents.rs::request_agent` 把 waiter 存为全局 `pending[request_id]`；`handle_agent_message` Response 分支（约 `agents.rs:327-339`）调用 `runs::store_result(...)` 后忽略返回的 `bool`/错误，再无条件 `pending.remove(&request_id)` 并发送 data。`runs.rs::store_result` 虽会按 `run_id/request_id/agent_id` 尝试匹配，但匹配失败不会阻止 waiter 被释放。
- **机制与影响**：同一 Agent 的 stale/duplicate/replayed connection 只要能提交一个已知 request id，就可能先消耗当前同步 waiter；若 run id 缺失或不匹配，HTTP/MCP 仍可能收到不属于当前等待上下文的 data，而 durable run 未必被完成。不同 Agent 直接跨越的条件受 per-agent secret 和路径校验限制，因此证据更直接支持“同 Agent 连接代际/同 Agent 跨 run”问题，不应夸大为无条件跨 Agent 漏洞。
- **严重度**：高（控制面响应完整性和请求/执行所有权不可靠）。
- **根因推断**：`[推断]` 为兼容迟到 Response，Hub 以 request id 做全局 rendezvous，却没有把 pending 值建模为 `(agent_id, run_id, request_id, command_hash)`，也没有把 `store_result` 的匹配结果作为发送前置条件。
- **边界建议**：pending value 保存完整 owner tuple；Response 必须匹配 tuple，只有 `store_result == changed/idempotent` 才可唤醒对应 waiter；无 run id 的 legacy fallback 应隔离到明确兼容模式，不能用于新的受控执行。

### P2 / 中：统一 request_agent 硬编码错误原因，持久生命周期语义失真

- **证据**：`agents.rs::request_agent` 超时分支（约 `agents.rs:538-545`）固定调用 `runs::mark_timeout(run_id, "process_exec_timeout")` 并返回同一错误文本，不区分 Job get、tmux、MCP、Room、skills、notify 等 command。`routes.rs` 只在 HTTP 外层把部分错误映射为 operation-specific code。
- **机制与影响**：客户端外层可能看到 `tmux_request_timeout` 或 `job_get_unavailable`，但 `agent_runs.reason` 仍是 `process_exec_timeout`；运维无法从持久 run 记录判断哪类操作超时。超时也只是 Hub waiter 结束，不会自动取消远端 Job。
- **严重度**：中。
- **根因推断**：`[推断]` generic bridge 为复用而保留了早期 process exec 错误名，没有传入 operation identity/timeout policy。
- **建议**：command metadata 携带稳定 operation code；将 dispatch timeout、remote Job timeout、cancel outcome 分成不同状态/原因；同步等待超时响应中明确 `dispatched` 与 `remote_execution_unknown`。

### P2 / 中：send failure 产生“已落库但未投递”的可重放记录

- **证据**：`agents.rs::request_agent` 先 `runs::prepare_run`，再向 mpsc sender 发送；发送失败时移除 `pending`、断开连接并返回 `agent_offline`，但没有把该 run 标为 failed/rejected。`pending_unacked` 会查询 `created`、`dispatched`、`timeout_waiting_result` 且无 ack/result 的记录并重放。
- **机制与影响**：调用方已经收到 offline，但重连后 Hub 仍把该记录视为未确认命令；恢复路径语义上可能再次尝试一个请求。虽然 mpsc channel 发送失败通常表示 writer 已退出、不能证明远端已执行，但“调用方错误返回”和“持久层可重放意图”不一致，重试/网络竞态下会增加副作用不确定性。
- **严重度**：中。
- **根因推断**：`[推断]` 可靠投递设计优先于同步 API 的 dispatch outcome，缺少 `never_sent`/`send_unknown` 等终态。
- **建议**：区分“未进入 transport ledger”“已送出但 ack 未知”“已 ack”；未发送记录不可 replay，未知记录必须在 API/运维面显式标记，而不是都复用未 ack 队列。

### P2 / 中：Hub Job cache 是无界、易失的 projection

- **证据**：`state.rs` 的 `jobs` 为 `HashMap<String, HashMap<String, JobInfo>>`；`agents.rs::handle_agent_message` JobUpdate 分支按 agent/job 插入；`cached_job`、`routes::list_jobs/get_job`、`mcp_server::snapshot_job_*` 只读该 cache。Hub background `cleanup_runs` 只清理 SQLite runs；代码中未见 Job cache 的 TTL、数量上限或周期 eviction。
- **机制与影响**：长期运行产生大量不同 job id 时，Hub 内存会随历史 Job id 增长；Hub 重启后所有 cache snapshot 消失，Job list/get 只能降级到空/有限的 DB run 或错误摘要，而 DB 的 24 小时 run retention 与 cache retention 不一致。
- **严重度**：中。
- **根因推断**：`[推断]` cache 是为 Agent 暂时不可用时提供 fallback 而添加，未被明确建模为带 retention 的 projection。
- **建议**：定义 cache 的用途、TTL、最大条目和清理；响应中标明 live/cached/stale；如果需要持久 Job 查询，明确从 Agent run report 投影到 SQLite，而不是无限保留内存对象。

### P2 / 中：断线时确认已标记失败但未通知 Agent waiter

- **证据**：`agents.rs::disconnect_agent`/`discard_agent_confirmations`（约 `agents.rs:615-648`）把该 Agent 未解决 confirmation 标成 `ProviderUnavailable`，但不发送 `HubMessage::ConfirmationResponse`；`main.rs:676-700` 的 `send_confirmation_response` 只按当前 `agents[agent_id]` sender 发送。Agent `confirmation.rs:390-470` 的本地 waiter 仍需 socket 错误或本地 `CONFIRM_TIMEOUT_SECS + 5` 超时结束。
- **机制与影响**：Hub 内部状态已是 provider unavailable，但 Agent 侧请求可能继续等待；若期间连接替换，回调按 agent id 找到的新 sender，可能把旧连接产生的 confirmation decision 投给新代际。
- **严重度**：中。
- **根因推断**：`[推断]` pending confirmation 只保存 agent/request，没有保存发起连接代际，且 discard 被实现成内存状态更新而非完整 waiter notification。
- **建议**：confirmation 绑定 connection id/run/request；断线立即向旧连接对应 waiter 发送终止结果（或通过可恢复的 Agent protocol 明确取消），回调只允许原代际或经协议认可的当前代际接收。

### P2 / 中：OAuth 是内存 session，且授权码错误交换会被消耗

- **证据**：`oauth.rs:134-260` 将 authorization code/token 放入 `HubState.oauth_codes/oauth_tokens`；`oauth.rs::token` 在检查过 code hash 后立即 remove，再校验 expiry、client、redirect、resource、PKCE。`cleanup_oauth` 只是内存过期清理。
- **机制与影响**：Hub 重启会使现有 ChatGPT/App token 和未完成 code 全部失效，七天 token TTL 不是跨重启 TTL；恶意/误用的错误 redirect 或 verifier exchange 会烧掉原本可用的 code，导致一次性授权可用性下降。
- **严重度**：中（重启/错误交换是可见运维和可用性约束；token 内存本身未必不符合轻量部署目标）。
- **根因推断**：`[推断]` 这是无外部 session store 的轻量 OAuth shim，采用“一次尝试即消费”的简单实现。
- **建议**：明确“Hub restart logs out all MCP clients”作为部署契约，或持久化/轮换 token；仅在所有验证通过后消费 code，并记录失败尝试策略。

### P2 / 中：OAuth scope 和 public URL 信任边界仍偏宽

- **证据**：`oauth.rs::validate_authorize_params` 验证 PKCE S256、client/redirect allowlist 和可选 resource，但未见把请求 scope 限制为唯一 `agentic:mcp`；`is_valid_mcp_bearer` 校验 token 有效期但没有按存储 scope 再授权；`public_base_url` 在未配置时从 forwarded/Host 推断。
- **机制与影响**：目前只有一个 MCP scope，尚未形成可利用的多 scope 越权，但未来扩展 scope 时 token scope 可能不会约束工具权限；未显式配置 public base 的反向代理部署可能生成错误或被伪造的 OAuth metadata/redirect action URL。
- **严重度**：中（当前主要是未来边界漂移和部署误配风险）。
- **根因推断**：`[推断]` OAuth 实现只服务单一 MCP resource，授权参数和 token claims 没有抽象成可扩展权限模型；public URL 为本地/代理两种场景提供了宽松 fallback。
- **建议**：只接受 `agentic:mcp`（或显式支持的子集），token 校验绑定 scope/resource/tool profile；生产启动时要求 configured public base，并在可信 proxy header 列表之外拒绝 forwarded 覆盖。

### P3 / 低到中：Agent report upsert 的历史身份校验弱于 transport ack

- **证据**：`runs.rs::upsert_agent_report:286-422` 验证 report status 仅为 started/completed/failed、owner agent 和 detail kind，但对已存在 run 没有以 incoming request id/command hash/tool/profile 完整比对；`mark_status:191-208` 接受任意 status string。`store_result` 的 hash conflict 保护只覆盖 Response result。
- **机制与影响**：可信 Agent、旧连接或重复 report 可以让同一 run 的状态/job/result/reason 被更新为不符合原 command 的历史投影；任意 status 也会让消费者看到协议声明之外的状态。它不等于未经认证的外部攻击，因为 Agent ingress 仍受 secret 保护，但会削弱跨进程历史可信度。
- **严重度**：低到中。
- **根因推断**：`[推断]` report 被当作可信观测事件，只按 run_id 做方便的 upsert，没有复用 ack 对 command hash/request tuple 的严格校验。
- **建议**：统一 run identity validator 和允许的状态转换；report 只追加合法单调转移，冲突转入 conflict/audit，不覆盖已完成事实。

### P3 / 低：Android notification 是持久化占位而非可用端点

- **证据**：`notify.rs:333-339,428-468` 注册 Android endpoint、返回 raw token 和 `deliveryImplemented:false`；`notification_channels` 始终把 Android 作为 unavailable placeholder；未见使用 `notification_endpoints.token_hash` 的投递/认证/last-seen 路径。
- **机制与影响**：行为对调用者是诚实的（不会伪报送达），但 DB 会保留没有消费路径的 token hash/endpoint，容易被误当成完整通知基础设施。
- **严重度**：低（若为明确未实现占位则是边界债务，不是隐藏故障）。
- **根因推断**：`[推断]` 先铺设了 API schema/注册状态，实际 Android delivery 尚未接入。
- **建议**：要么补齐独立 Android endpoint 协议（认证、投递、ack、撤销、retention），要么从生产工具集删除注册入口，仅保留明确 unavailable 能力说明。

### P3 / 低：持久化查询存在明显单节点性能约束

- **证据**：`runs.rs::list_runs:458-494` 先取最多 1000 个 id，再逐条调用 `get_run`；`agents.rs::mcp_list_servers_all_agents` 按 enabled+online Agent 顺序逐个 `request_agent`，每个最多约 35 秒。
- **机制与影响**：历史 run 列表是 N+1 查询；多个 Agent 下 MCP server discovery 的总等待时间近似线性叠加，且结果条目标记 online 主要来自连接快照而非每个请求成功。
- **严重度**：低（当前单 Hub/小 Agent 数可接受，规模化前会成为 tail latency/资源占用问题）。
- **根因推断**：`[推断]` 设计目标是少量 Agent 的简易控制平面，未将批量 projection 和并发预算作为明确契约。
- **建议**：run list 改为单查询/批量映射；Agent discovery 设并发上限、总预算和每 Agent 结果状态，不要把“online”与“server query succeeded”混为一谈。

## 6. 目标边界与增量切分建议

### 应保留在受控执行基础设施内

1. **Hub ingress/control plane**：HTTP/MCP 入口、action/OAuth 认证、Agent registry、连接租约、CommandCapable/ReportingOnly 模式、受控 command dispatch、确认 callback、run receipt/history、重放/幂等协调。
2. **Agent execution plane**：本地 policy/confirmation、sandbox/executor、Job 状态、实际 process/tmux/downstream MCP/skill 执行，以及 Agent transport ledger。Hub 不应复制这些执行器或引入 provider orchestration/reasoning loop。
3. **Room resource adapter，而非 Hub memory**：保留 Room repository 的路径安全、Git/schema/scaffold、Diary/Notebook/State read 和 maintenance submit；Hub 只保留 active Room lease/路由与最小摘要，不把 Room 内容加载成长期 Hub memory。
4. **有限的 Coordinator profile**：保留八个 Hub-native 状态/历史/通知工具，作为只读/低风险控制入口；Full profile 继续服务兼容客户端，但所有 consequential tool 必须经过同一 dispatch/权限链。
5. **通知投递**：保留 Agent freedesktop 与 ntfy 这种明确的 delivery adapters；Android 必须独立补齐或移除占位，不应顺手变成 Hub 的任务/提醒调度器。

### 推荐的增量切分

**切分 A：先修 transport identity，不改变产品 API。**

- 引入内部 `ConnectionHandle { agent_id, connection_id, mode, role }`，所有 inbound message 先做 current/allowed-mode 检查。
- 将 pending key/value 绑定 `(agent_id, run_id, request_id, command_hash)`；可靠旧代际只允许完成既有 tuple，不允许改变当前连接 metadata 或任意 Job projection。
- 统一 `Response`、`RunReport`、`TransportAck`、`TransportRunStatus` 的 owner validator；保留 late-result/idempotence，但让 mismatch 明确落 conflict/error。

**切分 B：把 run、wait、Job projection 三者分离。**

- command metadata 传入 operation code 和 timeout reason；区分 `not_sent`、`sent_no_ack`、`acked_running`、`wait_expired`、`remote_unknown`、`cancel_requested`。
- 为 Hub Job cache 增加 TTL/上限/周期清理，响应明确 live/cached/stale；必要时将有限摘要投影到 SQLite，不保存无限内存历史。
- 保持 Agent ledger 的幂等设计，并为 append-only JSONL 定义 retention/compact/损坏恢复策略。

**切分 C：Room 接口迁移，再清理 legacy。**

- 先实现 Hub tool/command 到当前 Agent `room.diary.active/read`、notebook read/recent/search 和 `room.maintenance.submit` 的明确 adapter，补真实 Hub↔Agent E2E。
- 确定写入语义/权限后，逐步废弃 `RoomNotebook*`/`RoomDiary*` legacy JSONL variants；不要删除 `room_repository`、Diary、Notebook、State 或 maintenance 文件基础设施。
- 如果 Hub 仍需提供 Room HTTP 路径，应将其视为 active Room RPC façade，而非 Hub-owned content store。

**切分 D：明确 auth/session/deployment 契约。**

- 生产配置强制 `public_base_url`，限制可信 proxy headers；把 OAuth scope/resource/profile 绑定到授权结果。
- 决定 OAuth token 是“重启即失效的内存 session”还是持久/可撤销 session；若保持内存实现，必须在运维文档和 health/status 中明确。
- 对 Hub DB/config、run command/result、Agent secret hash 设置 restrictive file mode、备份和 retention 规则；必要时补 SQLite migration/version/index/FK 策略。

**切分 E：确认与通知的生命周期闭环。**

- confirmation 绑定 connection/run/request，断线立即通知 Agent waiter；旧回调不能投递到无关新代际。
- Android 作为单独 delivery integration 处理 token rotation/revoke/ack；不要把它扩展成 Hub reminder/task scheduler。

## 7. 已有验证入口与未调查盲点

### 已有验证入口（本次未执行）

- Hub `agents.rs` 测试覆盖 boot generation、pending replay、stale SSE heartbeat/JobUpdate、matching-run late Response、send failure、ReportingOnly target rejection、过期连接清理。
- Hub `room.rs` 测试覆盖 active Room 选择、不同 Agent 冲突、重连/旧连接断开、无 active room、路由转发。
- Hub `runs.rs` 测试覆盖 late idempotent result、stale acked -> unknown、Agent report upsert。
- Hub `mcp_server.rs` 测试覆盖 annotations、Coordinator rejection、Full aliases、batch descriptors、skills annotation、dispatcher parity、bootstrap、timeout code mapping、schemas、AgenticResult。
- Hub `notify.rs` 测试覆盖 channel parsing/list placeholders、ntfy config/health、Android not implemented、alias routing；`main.rs` 测试覆盖 bearer parsing、hub info safe summary、OpenAPI path/schema 与 confirmation action 数量；`db.rs`/`instance_lock.rs` 有 alias uniqueness/lock 测试。
- `crates/agentic-gpt/tests/local_control.rs` 明确验证当前 Agent 本地 surface 不再暴露旧 Room write tools；`standalone_http_mcp.rs`、`standalone_supervisor.rs` 更偏 Worker/standalone HTTP MCP 和 Room worker 启动，不是 Hub live E2E。
- `openapi/hub.yaml` 与 `docs/operations.md` 可作为 API/deployment smoke 的意图入口（init/serve、`/v1/info`、Agent command-capable、harmless process、Jobs/cancel、`/mcp`、ReportingOnly rejection），但它们不是行为事实，必须以源码和真实跨进程 smoke 为准。

### 未调查/当前缺失的验证

- 没有覆盖 Hub 重启后 SQLite run replay 与 Agent JSONL ledger 的真实双进程测试；尤其没有证明原 HTTP waiter、late Response 和 duplicate side effect 的完整语义。
- 没有覆盖旧 WS 在 replace/close 竞态期间发送 Hello/Heartbeat/JobUpdate/RunReport 的代际隔离；已有 stale 测试主要是 SSE non-reliable 路径。
- 没有覆盖 foreign/mismatched Response 消耗 pending waiter、`store_result` false 后仍唤醒 waiter 的回归测试。
- Hub 没有 OAuth authorize/token/restart/scope/public-base 的模块测试清单证据；也没有反向代理真实 header 配置测试。
- 没有 Hub↔当前 Agent Room diary/notebook/maintenance 的真实 HTTP/MCP 兼容测试；现有 Hub Room tests 可能只测试 fake connection routing，Agent local_control 则只验证 legacy surface 被移除。
- 没有 Job cache eviction/内存上限、report monotonic transition、confirmation disconnect waiter、Android delivery/endpoint lifecycle 的验证。
- 本次按只读审计约束没有运行测试、build、lint、formatter 或部署 smoke；上述测试文件是可复用验证入口，不是本次通过证明。