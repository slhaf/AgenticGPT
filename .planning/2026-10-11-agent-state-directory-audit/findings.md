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
- local_control / supervisor 自己的目录位于 runtime/agent，transport ledger 位于 transport-ledger，audit 位于 workspace；未找到独立生成其他 agentKey 的专用 state/agent 写者。
