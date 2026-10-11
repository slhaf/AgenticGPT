# 排查发现

- 仓库工作区起始干净，存在 .codegraph。
- CodeGraph 初始自然语言查询匹配过宽；使用 private_state 精确文件/符号继续。
- private_state::agent_state_key 使用 agent_id；合法短身份原样成为目录名，其他身份使用 SHA-256 id- 前缀。
- 测试来源调查由只读子 agent 独立负责。

- agentic_home 由 dirs::home_dir() 加 .agentic_gpt 得到，与 config_path / workspace_root 无关。
- 唯一正常 per-agent root 构造在 private_state::prepare_at；prepare 从 build_app_state 调用。
- build_app_state 先无条件 prepare，再打开 events.sqlite3 / process.sqlite3；无须执行业务工具即可留下目录。
- 本机只读统计：1036 个目录，当前配置 agentId 为 agentic-gpt-slhaf-laptop。1008 个具有六种集成测试前缀，其余包含 18 个短 hex、9 个其他名称及当前身份。多数有旧 jobs.sqlite3，不能把当前代码逻辑直接当作所有历史目录的创建证明。
- 用户补充：来源不限于测试；继续完整检查运行模式、替代 config、身份变更、手工实例与绕过 prepare 的写者。

## 阶段 1：创建入口
- 运行时统一创建链路：startup.rs:177 -> private_state.rs:27-41 -> ensure_private_dir:396-409。
- 正常入口：Hub run (startup.rs:65)、Standalone stdio-worker (108)、Local run (236)；任何 --config 指定的其他身份都会落入同一用户 state/agent 根。
- 测试已证实：local_control.rs:36/70 的 local-e2e；standalone_http_mcp.rs:1463/1498 的 http-mcp；standalone_supervisor.rs:955-969 的短 hex；runtime/main_tests.rs:45/272 的默认 laptop。这些入口未隔离 HOME，清理临时 workspace / runtime socket 不清私有状态。
- private_state 自身 prepare_at 测试和 for_test(_agent) helper 使用临时根，不直接污染真实 HOME。
- check_contract_parity.py:277-300 的 make_env 明确隔离 HOME；cargo test 调用未统一隔离 HOME。
- local_control 目录位于 runtime/agent，supervisor 位于 runtime/tunnel，transport ledger 位于 .agentic_gpt/transport-runs.jsonl，audit 位于 workspace；未找到独立生成其他 agentKey 的专用 state/agent 写者。

## 阶段 2：本机与边界核对
- 本机目录总数 1036，当前身份目录 1。
- 名称汇总：standalone-e2e-* 408；standalone-http-mcp-* 178；http-mcp-* 109；local-e2e-* 107；live-reload-e2e-* 104；stdio-resume-e2e-* 102；短 hex 18；其他名称 10（含当前身份）。
- 其他名称包括 laptop、manual-smoke-*、terminal-smoke-*、terminal-manual-*、toolset-smoke-*、room-toolset-smoke-*、http-mcp-smoke2、local-mcp-smoke、smoke-tail-*。名称不足以确认手工执行来源。
- 配置结构只有 agentId；agentKey 是 private_state::agent_state_key 的内部结果，不是独立配置字段。
- 专用 state/agent 根只由 prepare_at 构造；EventStore / ProcessHistoryStore / skill installs 使用选定 PrivateStatePaths，不选择其他身份。正常 CLI config init、local list-tools/call、TUI、tmux 没有 build_app_state 入口。
- prepare 在任何工具执行之前创建目录；迁移旧 workspace/state 也只进入当次身份根。正常退出没有删除整个私有状态根的代码。
- agentId 是需重启字段（config.rs:1580-1587）；热加载 apply_live_config_subset（startup.rs:363-381）不替换身份，也不会重新 prepare。修改后下次启动才会创建新身份目录，旧目录保留。
- 阶段 1 git 提交默认 sandbox 失败（.git 只读），以 require_escalated 提交排查记录成功：ccbafcb。后续提交使用同一方式。

- Git 历史补证：7600e5a 的父版本 standalone_supervisor.rs:93/291/428 使用 standalone-e2e / stdio-resume-e2e / live-reload-e2e 前缀；e56332d 的父版本 standalone_http_mcp.rs:1470 使用 standalone-http-mcp 前缀。因此 1008 个六类前缀目录均与当前或历史集成测试身份格式对应，但未逐目录追溯实际执行者。
- 最终结论：不限测试；实际加载其他配置、历史 agentId、临时手工实例都会通过同一 prepare 机制创建额外身份目录。没有找到生产代码自行生成随机 agentKey 或根据请求中的外部身份另建根的机制。安装记录只创建当次身份内部 skill-installs/staging 子目录。

## 第二轮：规范与共享状态分析
- 用户拟取消 state/agent/<agentKey>，直接维护 .agentic_gpt/state；本轮授权先改 AGENTS.md 并分析影响。
- AGENTS.md 新增临时 HOME 和清理要求，明确配置/workspace/agentId 不能替代 HOME 隔离；派生进程沿用隔离环境；禁止并行测试改全局 HOME；退出后先停进程再清理。
- 两个只读子任务分别核对存储身份/恢复语义和测试/CI/文档影响。

- 源码核对：当前 InstanceLock 按配置文件路径+suffix 锁（instance_lock.rs:21-40），不能阻止不同 --config 实例同时写同一 state 根。
- prepare_at 现仅迁移 workspace/state 中 active-skills.json 与 skill-installs；改共享根还需考虑旧 state/agent/<key> 中 SQLite 与历史 jobs 数据。
- migrate_file/migrate_directory 目标存在且源内容相同时会删除源；新布局应明确阻止源与目标是同一目录/文件的情形，尤其自定义 workspace_root 等于 .agentic_gpt 时。

- process_history 当前只打开 process.sqlite3；opens_fresh_process_database_without_touching_legacy_jobs_database 明确保留旧 jobs.sqlite3，不把历史 jobs 自动视作新 process 数据可直接迁移。
- 临时 HOME 新规范与工具链需区分：后续测试隔离应在 Agent 子进程或测试 runner 上实施，Cargo/Rustup 可保留显式工具链路径/缓存配置；禁止通过并发进程全局 set HOME 达成隔离。
- 临时 HOME 夹具仍需短路径：Unix socket 有 100 字节上限；runtime/agent/<agentId>/mcp.sock 与 runtime/tunnel/<agentId> 不属于本次私有状态路径扁平化范围。
- live parity 已隔离 HOME/XDG 并 finally 先停止进程后清理，但 check_contract_parity.py:2906/3950/4219-4225 事件库查询使用旧 per-agent 路径。
- 文档待随实现更新：configuration.md 与 zh-CN 版:159；interfaces.md:181/183；operations.md:28-30；开发指南的 smoke 隔离内容需核对新规范，archive 保留历史语境。

## 共享 state 根的候选设计（尚未实施）

1. 私有根改为 .agentic_gpt/state，移除 agent_state_key；保留 agentId 的协议身份、历史记录、runtime/socket、确认与传输账本身份意义。
2. 建议每个 HOME 下的共享 state 只有一个实际写入者；在迁移、数据库打开、process/event/skill recovery 前获取以 state 根为键的跨进程锁。supervisor 与 worker 要划分锁所有者，避免互相锁死；直接 worker 入口不能绕过。
3. EventStore 的 owner fingerprint 与 HOME 持久状态归属脱钩；历史记录中的原 agentId 保留为来源信息。是否允许不同身份接续已有事件/未终态任务，需要在重构前明确，不能只删目录层级。
4. 旧 per-agent 根迁移只选择明确来源，建议默认当前有效配置身份；不得扫描合并本机 1000 多个目录。目标已有数据时定义冲突/恢复规则，SQLite 在停止旧 writer 后完整迁移，兼顾 sidecar、备份、崩溃幂等性和权限；用户旧目录清理是另一个操作。
5. 现有 workspace/state 的 skills 迁移/失败回退需改写或保留明确边界；禁止源目标相同后误删目标；当前 jobs.sqlite3 未被 process store 读取，不默认转换为新格式。
6. 共享历史意味着新配置会见到旧身份 process/event/skill 状态，并可能参与恢复/通知/清理；workspace 更换还会改变同一 active skill ID 的解析和安装恢复目标。需把这些消费者可见行为写清。
7. 测试先修隔离：每夹具独立 HOME 与派生环境，builder 注入状态根；parity 更新 DB 路径；验证共享状态跨身份续用、多实例拒绝、重启恢复、迁移失败/冲突/崩溃、临时资源清理和真实 HOME 无新增。优先真实内部链路/实际执行验证。

此轮只做规范与分析；实现、用户状态迁移及旧目录清理均未执行。

## 存储与恢复补证

- EventStore: event_store.rs:191-234、903-932 通过 agentId hash 校验 owner，变更 ID 后现有库报 event_store_owner_mismatch；2021-2044 的具名测试需调整。event 查询240-270无行级 agentId 分区；local_service.rs:212-219/256-281请求 scope 和 cursor:1270-1303 属于外部连接身份契约，不能误作库隔离。
- ProcessHistory: process_history.rs:275-297 保存旧 agent_id 与 info JSON；get/list:569-579/623-676 不按身份过滤。recover_active:679-725 会将全库 active 判为 UnknownAfterRestart 并生成 outbox；不同 config 并发会误终结仍运行实例记录。
- Skill active: skills.rs:68-81 无 workspace 归属；同 HOME 更换 workspace 后共用激活列表，缺包标 missing。skills_writes 与 lease 都是进程内锁；读取:674-706也可能补默认项写回，原子 rename 不防丢更新。
- Skill installs: skill_bootstrap.rs:417-425 记录没有 agentId/workspaceRoot/owner；skill_installs.rs:81-173 扫全部非终态、清 staging 后重排 worker，当前 config 决定目标；journal 的绝对 target 要经当前 skills root 校验。共享根意味着 workspace 接手规则、全根 idempotency、pruning 都需决定。750-761 固定 tmp 写入只有内存锁，多进程风险不能靠 SQLite 弥补。
- Notification/source consistency: event_notifications.rs:282-337 drain process outbox 和安装终态；事件来源仅 kind+resource ID。迁移需把 events/process/install records/journals/outbox 作为关联状态集合，不得分别从多个旧身份拼凑。
- Transport receipt: transport_ledger.rs:843-848 位于 HOME/transport-runs.jsonl，现有跨进程锁530-543；agentId 校验是可靠执行归属，应保留历史身份。hub.rs:995-1007 只重放当前 ID ledger，而964-990的 pending remote event source 回传在 owner 过滤前，共享 events 后旧 remote origin 是否允许送往新身份须明确，不能默认转移。

## 建议实施顺序

1. 先调整现有 fixture 的 HOME/环境传递、builder 注入、进程退出和临时资源清理，建立安全验证基础。
2. 落实 HOME/state 单写入者和本地持久状态归属；保留外部 agentId scope、历史 identity 和原远程来源边界。
3. 定义当前旧身份关联数据集到共享根的迁移和冲突恢复，结合 skills workspace 接手限制；再修改路径与文档/parity查询。
4. 运行迁移、改身份重启、同 HOME 两配置竞争、异 HOME 隔离、process/event/skill recovery 与 remote origin 的真实链路回归，随后 workspace checks 和 live parity。

尚待用户进一步确定的行为选择：是否明确限制同 HOME 单实例；旧身份关联状态迁移选择；更换 workspace 的 Skill 接手规则；更换 agentId 后旧 Hub origin 的处理。以上为影响分析与候选建议，本轮未实施。

## 已确认边界（取代前轮待决建议）

- 用户已确认：迁移是独立脚本运行时的操作；Agentic 启动只用 .agentic_gpt/state，没有旧路径探测、迁移或失败回退兼容行为。
- 本地状态不再以 agentId 定义隔离/归属；不增加生产跨身份、多实例/多 workspace 隔离机制，测试实例按独立临时 HOME 调整。
- agentId 配置、Hub 通道、传输回执、远程事件来源语义原样保留；不按字段名机械删除所有持久化 agentId。
- 独立脚本仅在新版状态不存在时找当前有效 agentId 的旧目录，旧根 key 的合法原样/哈希规则由脚本承担；不自动合并其他旧身份。
- 当前 ProcessInfo 是公共 wire DTO（protocol/process.rs:202-204），agent_id 为必填，process_history.rs 同时将其保存进 JSON 与索引列；需区分本地归属机制与远程/历史记录字段，不能直接删 JSON 字段使解析失败或改变远程契约。

## 源码可行性结论

- 目标明确，源码没有要求按 agentId 分开持久根的不可拆约束。路径工厂、EventStore owner metadata 与外部身份可分开重构；测试 HOME 隔离替代历史随机身份目录。
- EventStore 真正归属机制：SCHEMA:96-100 event_store_metadata(owner_fingerprint)，open:192/223 与 ensure_store_owner:903-932。events/internal_sources 业务行无 agentId 分区；脚本可以移除 owner metadata 并转换成新 schema，保留远程 origin_run_id/request_id/command_hash。
- EventStore open:217-223 当前包含 MIGRATE_V1_TO_V2 与 owner 检查；private_state prepare_at:31-104 包含 workspace/state skills 迁移及失败回退。按用户边界，本次旧格式转换放入独立脚本，Agentic 新布局初始化不保留上述旧布局兼容链路。
- EventList/Get/Mark 的 local_service.rs:212-229/256-281 绑定当前连接身份；这是 Hub 目标校验耦合，需要区分接入层契约与存储归属，不机械删除。EventStore list 游标仍有请求 scope 绑定，应按保持远程行为的边界处理。
- ProcessInfo.agent_id 为 wire 必填；process_history 的 info_json 被 recover_active 和读接口反序列化成该 DTO。直接删持久 JSON 中该字段会使解析失败并改变远程返回契约。实现可以保留来源字段而不当成 owner，或拆持久模型与 wire 模型；不能把去除归属等同去除全部 agentId 字段。
- Skill CommitJournal (skill_installs.rs:37-43、1596-1614) 记录 target/archive 是 workspace 包路径，无 agentId；staging_path:1852-1853 由 records_root 相对派生，整树迁移可保留关联记录并由新根派生 staging，无须新增身份/多 workspace 隔离机制。
- 独立脚本需在 Agent 停止时执行一致转换；识别新版状态文件而非 state 目录非空，既有新版状态不覆盖，只选择当前旧 key。旧 jobs.sqlite3 当前不被 process store 读取，脚本不得把它直接重命名冒充 process.sqlite3；新版缺少哪个当前格式 store，由正常新布局初始化处理，历史文件处理留脚本明确说明。
- 下一步若用户授权实施，可据以上边界拆为测试隔离、状态去归属与启动清理、独立迁移脚本与文档/真实链路验证阶段。本轮仅核对与记录，没有实施或运行测试。
