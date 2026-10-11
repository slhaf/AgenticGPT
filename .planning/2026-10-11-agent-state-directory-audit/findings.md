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
