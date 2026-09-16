# 协议与部署调查证据

## 1. 覆盖路径、模块与实际职责

### Workspace

- `[事实]` `Cargo.toml:1-25` 当前不是三 crate，而是五个成员：`agentic-gpt`、`agentic-gpt-hub`、`agentic-gpt-protocol`、`agentic-apply-patch`、`agentic-browser-host`。
- `[事实]` `agentic-gpt` 和 `agentic-gpt-hub` 都依赖 `agentic-gpt-protocol`；只有 Agent 依赖 apply-patch。Protocol、apply-patch、browser-host 没有 workspace 内部依赖。OpenAPI、tests、scripts 不在 Cargo dependency graph 中。
- `[事实]` `AGENTS.md:5` 仍写 root workspace 有三个 Rust crate，和 Cargo 现状冲突。

### Agent 与运行形态

- `[事实]` `crates/agentic-gpt/src/main.rs:172-314` 负责根据 `Config.mode` 选择 Hub、Standalone supervisor 或 Local。Standalone 由 supervisor 启动 tunnel-client 和隐藏 `stdio-worker`；worker 拥有 stdio MCP、owner-only Unix MCP、可选 HTTP MCP，并可选 reporting-only Hub 连接。Local 只提供 Unix MCP。
- `[事实]` `stdio_server.rs:45-103` 的 `TOOL_NAMESPACE_BY_NAME` 是 Agent-local surface 表，按 live `toolsets.enabled` 过滤并排序。测试 `normal_and_room_tool_sets_follow_fixed_surface_contract`（约 `3893-4039`）冻结 Normal 29 个、Room 追加 11 个，合计 40；旧 alias 不广告。
- `[事实]` `stdio_server.rs:2954-3400` 的 `tool_descriptor`、`tool_schema`、`properties_for` 手工维护字段、required、默认值、边界和描述；条件关系在 runtime validation 中处理。
- `[事实]` `local_service.rs` 复用同一 value-returning operation layer；legacy Room command 在 Agent 侧明确返回 `room_legacy_surface_removed`，说明 Protocol 内的旧变体是 Hub 兼容残留，不是 Agent 当前公共面。
- `[事实]` `local_control.rs` 把本地 socket 作为受控入口，检查 UID，强制父目录 0700、socket 0600，并只清理可信 stale socket。

### Hub 与公共合同

- `[事实]` `crates/agentic-gpt-hub/src/main.rs:159-298` 初始化 HubState，注册 `/v1/info`、`/v1/agents`、process、Job、tmux、MCP、notify、Room、skills、Agent WS/SSE、`/mcp` 和 OAuth 相关路由。
- `[事实]` `routes.rs` 是 Actions/HTTP 入口，直接导入 Protocol request 类型；`mcp_server.rs:52-122,535-561` 是第二个 Apps MCP 入口，使用 rmcp ToolRouter 和 schemars typed args。`openapi/hub.yaml` 是第三个手工 HTTP/GPT Actions 投影。
- `[事实]` `docs/tool-contract-matrix.md` 仅是 review aid，并声明 live descriptors 和 typed request objects 权威；它不是实际 runtime schema。
- `[事实]` `openapi/agents-minimal.yaml` 是另一份 0.1.0 `/v1/agents` contract，当前 grep 未发现代码、CI 或主要文档引用；CI 只处理 `hub.yaml`。

## 2. 主要调用与状态生命周期链

### 链 A：Standalone/Local → Agent MCP → 共享本地服务

1. `main.rs:172-182` 按 mode 分流；`run_stdio_worker:228-314` 校验 supervisor token/config/profile，建立 AppState，启动 Local socket、可选 HTTP 和 stdio。
2. `stdio_server::serve_stdio:131-140` 建立 rmcp stdio；`ResumableStdioTransport:155-301` 正常接受 initialize，或在 tunnel worker 重启后用内部 synthetic initialize 恢复 logical session，并抑制内部响应。
3. descriptor 和 typed args 之后进入 `dispatch_with_lifecycle`、`local_service`、jobs/policy/confirmation；Local Unix 与 Tunnel stdio 共用同一状态、策略、审计、容量和 Job registry。

### 链 B：Hub HTTP/Apps → request_agent → Agent command → Job result

1. `main.rs:159-298` 注册公共路由；`routes::process_exec`（约 `222-263`）或 `mcp_server::call_app_tool:242-447` 验证入口并构造 `HubCommand`。
2. `agents::request_agent:494-547` 只接受 command-capable 且已 Hello 的连接，调用 `runs::prepare_run` 写入 command JSON/hash，创建 pending oneshot，序列化 `HubCommandEnvelope` 后经 WS/SSE 发送；等待超时则记录 `timeout_waiting_result` 和 runId。
3. Agent `hub::connect_loop:31-184` 发送带 `bootGeneration` 的 Hello；`handle_reliable_envelope:829-876` 先 ledger accept/ACK/hash 校验，再 dispatch 到本地 service/jobs。执行中通过 JobUpdate/RunReport 观测，终结以 Response 回传。
4. Hub `handle_agent_message:99-417` 对 Response 调 `runs::store_result`；相同 hash 幂等，不同 hash 写 conflict，最后唤醒 pending caller。

### 链 C：断线、重启、重复投递

1. Hub `agents::handle_agent_message` 收到 Hello 后比较 boot generation；变化时把缓存 Job 标为 `unknown_after_restart`，然后从 `runs::pending_unacked` replay 未 ACK/未完成 envelope。
2. Agent `reconcile_transport_runs:878-922` 对 completed 只重发结果，对 accepted 未开始 command 继续执行，对 started/running 未完成 run 报 unknown，避免无法证明的副作用重放。
3. `transport_ledger.rs:56-82` 按 runId/requestId/commandHash 返回 First、Duplicate、Completed、HashMismatch；Hub `runs.rs:174-324` 再以 DB 条件更新 ACK/status/result。

### 链 D：Active Room 转发

1. `room.rs:33-350` 的 notebook/bootstrap/skills handlers 直接接收 Protocol request，生成带 requestId 的 Room/Skill `HubCommand`。
2. `request_active_room`（约 `393-430`）检查 active room 的 agentId、connectionId、`CommandCapable` 和 `AgentRole::Room`，再调用 `request_agent`；没有 active Room 是 404，连接状态冲突是 409。
3. Agent 侧沿共享 local service 执行；Hub `room_value_response` 将业务 error code 映射为 HTTP 400/404/409/500。

## 3. 持久化、所有权、安全与部署约束

- `[事实]` Hub `state.rs:15-89` 将 DB、Agent registry、pending、Job cache、boot generations、active Room、OAuth 和 ntfy health 分开；`db.rs:12-65` 持久化 agents、notification_endpoints、agent_runs。`runs.rs:11,101-134` 给 Hub run 记录 24 小时 TTL，保留 command/hash/result/conflict/reason/timestamps。
- `[事实]` Agent `job_history.rs:18-75` 管理每 agent 私有 jobs SQLite，包含 Job state、可选 start/finish 时间、输出尾部、cancel outcome/evidence、bounded detail；retention 是 30 天，容量 512 MiB，结果 512 KiB、preview 8 KiB。
- `[事实]` Agent `transport_ledger.rs` 将可靠传输记录追加到 `~/.agentic_gpt/transport-runs.jsonl`。配置文档规定 private state 在 `~/.agentic_gpt/state/agent/<agentId>/`，runtime/socket 在 `~/.agentic_gpt/runtime/agent/<agentId>/`。
- `[事实]` Agent 拥有本地执行、path/command policy、confirmation、Job/audit 和 local history；Hub 拥有 registry、公共入口、远端 run 记录、在线缓存和 active Room；Browser host 拥有独立 extension bridge；Protocol 只应保持 wire/data contract。
- `[事实]` Hub Actions `routes::require_action_auth:727-797` 使用 Bearer 和 constant-time comparison；Agent secret 经 registry hash 校验；`mcp_server::require_auth_on_mcp_path:588-600` 保护 Apps `/mcp`。Standalone 只接受 `env:`/`file:` secret reference，worker token 通过环境授权，不把 tunnel key 放进 argv。
- `[事实]` Local socket 通过 UID、0700 父目录、0600 socket、stale inode/device 检查保护；Standalone supervisor 另有 instance lock、健康文件、启动身份监测和最多 5 次退避重启。
- `[事实]` release workflow 用 cross 构建 `x86_64-unknown-linux-gnu` 和 `aarch64-unknown-linux-gnu`，每个 archive 包含三个二进制并生成 SHA256SUMS。没有在该 workflow 内做版本一致性、OpenAPI/descriptor parity、签名或 provenance 检查。
- `[事实]` Browser host 生产文档要求 host/socket 保持本地、不要作为远程服务暴露；Neko/container 场景要求双方共享 `/tmp/codex-browser-use`。实验性 Python bridge 自己读取用户 home 的外部 registry/config，启动 node_repl 并暴露任意 JS，只是 disposable spike。

## 4. 值得保留的结构

1. Protocol 的显式 serde 名称、request/run/hash、Hello bootGeneration、connectionMode、unknown-after-restart 是正确的跨进程边界。
2. `local_service.rs` 分离业务 value/error 与 Hub envelope/ACK，避免三种 ingress 复制执行逻辑。
3. Agent namespace/live toolset 过滤、Normal/Room 与 Hub Full/Coordinator profile、旧 alias 不广告，是受控 surface 的好结构。
4. bounded waits、输出/结果上限、Job state、cancel termination evidence、MCP batch admission/fail-fast 语义把执行控制和观测边界说清楚。
5. Local socket guard、Standalone supervisor、双侧 hash/idempotency/replay、reporting-only mode 都是值得继续加固而不是重写的基础设施。
6. Agent 的 `deterministic_tool_contract_corpus_exercises_public_dispatch`（`stdio_server.rs:4552-4715`）实际走 descriptor、serde、dispatch/dry-run；比只检查文本更有价值。

## 5. 主要问题清单

### P1：多份手工合同没有统一 parity gate，严重度高

- **证据**：Protocol `lib.rs:1984-2298`；Agent descriptor `stdio_server.rs:45-103,2954-3400`；Hub schemars `mcp_server.rs:52-122,1868-2602`；静态 `openapi/hub.yaml`；`tool-contract-matrix.md`；`cases.json`。
- **机制**：同一工具的名称、required、默认值、bounds、description、response 分散在 Protocol Serde、Agent 手工 schema、Hub schemars、OpenAPI 和文档；这些文件没有生成或语义对比关系。
- **影响**：Local/Tunnel、Hub Apps、GPT Actions、WS wire 可能各自接受不同请求或返回不同 shape；只改一面不能证明跨进程兼容。
- **根因推断**：`[推断]` 入口是按消费者独立投影，但没有把投影版本、owner 和 parity checker 作为工程约束；当前 CI 只能检查编译、测试和一个 YAML 是否可 load。

### P2：OpenAPI Job contract 与实际 HTTP/Protocol 多处漂移，严重度高

- **证据**：`openapi/hub.yaml:103-205,3060-3168,3339-3348`；`routes.rs:28-47,259-398`；`protocol/lib.rs:1569-1668,1714-1743,1846-1880`；`stdio_server.rs:1171-1176,2367-2425,3300-3348`。
- **机制与具体差异**：
  - OpenAPI 把 `JobInfo.startedAt` 列为 required，但 Protocol `JobInfo.started_at` 是 Option，且 `job_info_can_represent_not_started_without_fabricated_timestamp` 明确允许 queued Job 没有 startedAt。
  - OpenAPI list 只声明 agentId/kind/state/limit，实际 `JobListQuery` 还支持 group/cursor；OpenAPI default limit=100，Protocol `JobListRequest::DEFAULT_LIMIT` 与 Agent schema 都是 50。
  - OpenAPI `JobListResponse` 只声明 jobs，实际 Protocol 有可选 `nextCursor`，Agent `slim_job_list_response` 会在分页时输出它，且 schema 是 additionalProperties=false。
  - 实际 `JobGetQuery` 支持 `waitOnly`，OpenAPI path 没有；Agent descriptor 对 waitSeconds 手工写 default=5，但 dispatch `unwrap_or(0)`，HTTP OpenAPI 写 default=0。
  - OpenAPI cancel 200 response 引用 `JobDetail`，实际 Agent `slim_cancel_response` 返回 Protocol `JobCancelResponse`，字段是 jobId/state/cancelOutcome/terminationEvidence/error；OpenAPI 没有对应 schema。
- **影响**：严格 Actions importer/response validator 可能拒绝 queued、分页和 cancel 响应；调用者无法发现 group/cursor/waitOnly；客户端根据 descriptor default=5 发出的 job.get 与实际 default=0 不一致。
- **根因推断**：`[推断]` OpenAPI 跟随旧 response 设计，未从 HTTP handler 的实际 DTO 和 slim adapter 生成或反向校验。

### P3：Room Notebook selectExact 的 Actions 请求无法直接到达实际 handler，严重度高

- **证据**：`openapi/hub.yaml:1999-2029` 要求 year/month/day 并禁止额外字段；Protocol `NotebookSelectExactRequest`（`lib.rs:544-550`）要求 `date: String`；Hub `room_notebook_select_exact`（`room.rs:67-82`）直接以 `Json<NotebookSelectExactRequest>` 接收，没有 date adapter。
- **机制**：符合 OpenAPI 的 year/month/day payload 不含 Protocol 必需 date，Axum JSON extraction 会失败；实际可用的 date payload 又不符合 OpenAPI schema。OpenAPI NotebookAppend 也把 significance 和 abstract 列为 required，而 Protocol 对 significance 有 default、abstract 为 Option。
- **影响**：HTTP/GPT Actions 与 Apps MCP/WS Room 行为分叉，升级后调用者会遇到 schema 合法但 runtime 反序列化失败。
- **根因推断**：`[推断]` Room 曾经历 legacy date projection 到 semantic Protocol request 的迁移，但 OpenAPI 没有同步到 handler 的实际类型。

### P4：Skill 等待上限只写在 schema，helper 实际不 clamp，严重度中

- **证据**：Protocol `SkillInstallGetRequest::effective_wait_seconds`（`lib.rs:1342-1357`）和 `SkillRunRequest::effective_wait_seconds`（`lib.rs:1431-1451`）声明 MAX_WAIT_SECONDS=30，却只 `unwrap_or(default)`；OpenAPI `hub.yaml:2725-2737,2888-2908` 声明 0–30。Agent stdio 通过 `stdio_server.rs:1290-1318` 使用，Hub Agent 通过 `hub.rs:1088-1127` 使用。
- **机制/影响**：直接构造 Protocol request 或走 Hub/Agent skills path 时，超过 30 的 wait 值可能继续进入 `wait_for_job`；这破坏文档所称 bounded wait，并与 process/MCP helper 的 clamp 方式不一致。
- **根因推断**：`[推断]` 新增 Skill request 时复制了常量/文档，却没有复用既有 clamp helper 模式。

### P5：合同 evaluator 不能验证它文档暗示的 contract，严重度中

- **证据**：`scripts/evaluate_tool_contracts.py:20-31,34-83`；`tests/tool-contract-cases/README.md`；`cases.json` 共 18 个 case。
- **机制**：evaluator 只比较预测的 `tool` 与 case tool，以及 `arguments` 的宽松 shape；完全忽略 case 的 kind、expect、descriptor、serde、dispatch、error code、结果字段和 lifecycle。list shape 只要求 actual 长度不小于 expected 且只比较 expected prefix；非 strict 模式即使全错也返回 0。真实 runtime 检查在 Agent 测试 `deterministic_tool_contract_corpus_exercises_public_dispatch`，不是 evaluator。
- **影响**：使用 evaluator 得到的通过结果不能证明 public tool contract；发布说明中把它和 deterministic runtime corpus 并列会让验证边界失真。
- **根因推断**：`[推断]` evaluator 被设计成模型选 tool/参数探索器，但命名和 release 文案没有明确收窄其保证范围。

### P6：CI 和 release 没覆盖第二份 OpenAPI、语义 parity、ARM 和发布 provenance，严重度中

- **证据**：`.github/workflows/ci.yml:1-50` 只对 `openapi/hub.yaml` 调 `yaml.safe_load`；没有 `agents-minimal.yaml`、reference resolution、OpenAPI semantic validation、Agent descriptor parity、corpus/evaluator gate。`.github/workflows/release.yml:1-56` 和 `scripts/dist-linux.sh:1-42` 只 cross build/package/hash。
- **影响**：YAML 可解析不等于合同可导入；schema 漂移、未引用的第二 artifact、版本 tag 与 crate version 不一致、ARM 构建失败或错误归档内容都可能到发布之后才发现。SHA256SUMS 是完整 archive hash，不是签名或 provenance。
- **根因推断**：`[推断]` CI 偏向 Rust 编译/单元测试，发布脚本偏向产物搬运，合同和供应链验证尚未成为发布 gate。

### P7：当前文档数字和工作区说明明显落后源码，严重度中

- **证据**：源码测试 `stdio_server.rs:3893-4039` 是 29/40；`cases.json` 有 18 case。`docs/operations.md:20-22,39,102-103` 仍写 23/34、0.9.0；`docs/migration-v0.9.md` 写 24/36、0.9.0；`docs/release-notes-v0.9.1.md:15-24` 写 24/36、九案例；`docs/development.md:43-47` tag 示例为 v0.9.0；`AGENTS.md:5` 写三 crate。根 README 和 standalone-runtime 已写 29/40，但其 build/release snippets 仍出现 v0.9.0。
- **影响**：运维冒烟按文档会错误报警或漏测工具；迁移验收、release notes、自动化 agent 指引会把历史设计当当前事实。
- **根因推断**：`[推断]` 文档没有单一版本/计数来源，历史迁移与当前运维文档也未标出冻结基线。

### P8：生产 Browser host socket 权限比 Local MCP 宽，安全边界需明确，严重度中到高

- **证据**：`crates/agentic-browser-host/src/lib.rs:13-16,457-470` 固定 `/tmp/codex-browser-use`，`prepare_socket` 设置 socket mode 0660；`handle_client_message:270-322` 将带 id 的 client request 转发至 extension，`handle_extension_message:324-383` 把 response 按 pending route 返回。没有 agent secret 或 peer UID 检查。相对地 Local MCP `local_control.rs` 强制 0700/0600/同 UID；文档 `docs/browser-self-hosted.md:91-164` 只要求本地且不要远程暴露。
- **机制/影响**：同组用户、共享 volume 或容器 namespace 中能访问 0660 socket 的进程，可能调用 extension bridge；该 bridge 可触达 Browser backend，而 `browser.repl` 是任意 JavaScript、destructive/open-world。若部署者按共享 `/tmp` volume 操作，socket 就不再等价于 owner-only Local MCP。
- **根因推断**：`[推断]` 0660 可能是 Native Messaging/容器协作便利性选择，但没有在代码中绑定可信 peer 或独立认证；当前安全性依赖部署文档和文件系统组权限。
- **建议**：在目标架构中把 Browser host 明确归为独立高权限 bridge，决定 owner-only、受限组、peer credential 或一次性 token 中的正式边界；在未决定前不要把它与 Local socket 的安全等级混称。

## 6. 目标边界与增量切分建议

### 目标边界

- 目标是受控执行基础设施：受策略约束的命令/文件/MCP/Browser/Room/Skill 资源访问、确认、Job admission/lifecycle、运行观测、跨进程可靠传输和部署安全。
- 明确非目标：长期记忆、上下文管理、provider orchestration、模型选择、reasoning loop、完整 Agent Runtime。Room notebook/diary 是已有受控资源能力，不能借架构审计扩展成通用 memory subsystem。

### 建议的 authority matrix

1. Protocol `lib.rs`：只作为 Hub↔Agent 跨进程命令、消息、枚举和 wire serialization authority。
2. Agent `stdio_server.rs`：作为 Agent-local MCP advertised surface、live toolset filter 和本地 conditional validation authority。
3. Hub `mcp_server.rs`：作为 Apps `/mcp` 的 rmcp/schemars contract authority；Coordinator/Full 是 Hub profile 选择，不应混入 Agent-local surface。
4. `openapi/hub.yaml`：作为 GPT Actions HTTP contract，但必须以 `routes.rs` 实际 HTTP DTO/response adapter 为准；不能直接假定 Protocol response 就是 Actions response。
5. 文档和 matrix：从上述 authority 生成或明确标注版本/基线，不再手填工具计数。

### 增量批次

- **批次 1：事实基线**：补齐 `docs/architecture/current-state.md` 与 diagnosis；记录五 crate、四层拓扑、四条 lifecycle、所有权和非目标。保留当前 `docs/architecture/README.md` 的事实/推断/建议区分。不要先改运行时。
- **批次 2：合同差异清单与最小修复**：对 P2/P3 的 HTTP schema 先做可执行差异表；优先修 startedAt optional、nextCursor、group/cursor/waitOnly、cancel response、Notebook date。每项决定是改 OpenAPI 还是加显式 HTTP adapter，并写兼容影响；不要自动改 Protocol wire 名称。
- **批次 3：边界 helper 与 descriptor**：处理 Skill wait clamp、Agent `job.get` descriptor default 与 runtime default 的选择；若是 breaking 行为，使用已有版本/迁移边界，不添加隐式 alias。
- **批次 4：验证 gate**：CI 同时 parse/resolve 两份 YAML，做 schema structural checks、HTTP route/response smoke、Agent fixed surface/corpus 和 Hub profile checks；把 evaluator 明确命名为 prediction-shape probe，strict 失败语义单独记录。不要把 provider 网络调用塞进 CI。
- **批次 5：文档与 release**：从源码/metadata 生成版本和 29/40 计数；清理或明确废弃 `agents-minimal.yaml`；release 检查 tag 与 package versions、三个 binary 存在、目标架构、OpenAPI gate；再决定签名/attestation。
- **批次 6：Browser 安全专题**：独立评审 P8 和 Neko/shared-volume deployment；在正式 peer/auth boundary 前维持只本地、不可远程暴露的部署约束。实验性 `chrome-control-poc` 保持非生产，不迁入核心 Agent runtime。

## 7. 已有验证入口与未调查盲点

### 已有入口，均未在本次调查执行

- Rust workspace：`cargo fmt --all --check`、`cargo check --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。
- Agent contract：测试 `normal_and_room_tool_sets_follow_fixed_surface_contract`、`compact_tool_schema_budgets_hold`、`deterministic_tool_contract_corpus_exercises_public_dispatch`；Protocol 测试 `job_info_can_represent_not_started_without_fabricated_timestamp`；Browser host framing/route tests；Hub OpenAPI string tests。
- Local/Standalone smoke：`agentic-gpt config init --mode local --profile normal --non-interactive`、`agentic-gpt run`、`agentic-gpt local list-tools`、`agentic-gpt local call agent.info`；Standalone 还需 tunnel-client doctor/readiness/restart recovery。
- Hub smoke：`cargo run -p agentic-gpt-hub -- init`、`serve`、带 Bearer 请求 `/v1/info`；应额外检查 WS/SSE Agent、Job、Room、Apps `/mcp`。
- Contracts：`python3 scripts/evaluate_tool_contracts.py --cases tests/tool-contract-cases/cases.json` 只打印 corpus；带 `--predictions`/`--strict` 才比较预测，不能替代真实 Agent test。
- Release：`scripts/dist-linux.sh` 和 `scripts/remote-build.sh` 的 check/build/test/release/dist 模式；本次按要求没有执行任何 build、test、lint、formatter、release 或状态变更命令。

### 未调查盲点

- 未调用真实 Secure MCP Tunnel、外部 GPT Actions importer、ChatGPT Apps client 或 reverse proxy/TLS，因此没有证明生产 connector 对上述 OpenAPI drift 的实际错误形态。
- 未执行 x86_64/aarch64 cross build；未验证目标镜像、动态依赖、archive 内容和 release provenance。
- 未在真实重启/断线/迟到 response 场景运行 Hub DB、Agent ledger、SSE replay 或 confirmation callback；这里只读了代码和已有测试。
- 未深入执行 engine、MCP downstream provider、Room repository、skill installer、Browser official runtime/extension/Neko；这些不是本次合同/拓扑调查的深挖范围。
- 未读取任何密钥、用户 home 文件或 `~/.agentic_gpt`；实验 Python bridge 的外部 registry/config 仅检查了仓库源码。