# 其他四个 Rust crate 的混合档用例

本文件列出跨越 T4/T5 的 6 个混合档审查用例。其他 Rust crate 的技术分类仍有 10 个 MIXED：Hub 5、Protocol 4、Apply-patch 1、Browser-host 0；其中 Protocol 的 4 个 T2+T3 用例已按新规则移至 `review-candidates.md`，其余 6 个保留在本文件待审。

| Crate | 源文件 / 测试函数 | 档位组成 | 实际验证内容 | 证据行 |
|---|---|---|---|---|
| `agentic-gpt-hub` | `crates/agentic-gpt-hub/src/storage/runs.rs` — `pending_unacked_retires_legacy_argv_and_preserves_current_and_completed_runs` | T3 旧命令形状分类 + T5 SQLite 退役与重放持久化 | 写入 argv 形状的旧 `process.exec` / `process.batch` 行（含空 batch）、当前结构化命令和已完成的旧结果。断言旧行变为 `unknown` 并带有 `retired_process_command`，同时保留命令字段与 hash；当前结构化命令仍 pending 且可重放；畸形旧形状不会被误分类；已完成结果保持不变。 | `1115–1423` |
| `agentic-gpt-hub` | `crates/agentic-gpt-hub/src/storage/event_feedback.rs` — `dropped_owner_queues_and_persists_no_terminal_with_raw_metadata` | T3 响应 owner guard／协调器队列逻辑 + T5 SQLite 决策及 outbox 副作用 | 丢弃响应 owner guard 后，队列中出现一项带原始来源 metadata 的 `no_terminal` 修复。修复持久化后，断言存储决策为 `NO_TERMINAL`、持久 pending disposition 的 `includes_terminal = false`，且内存修复队列已清空。 | `2300–2331` |
| `agentic-gpt-hub` | `crates/agentic-gpt-hub/src/storage/event_feedback.rs` — `public_preflight_requires_idle_barrier_and_drained_queued_repairs` | T3 public preflight／barrier 与 owner-drop 队列逻辑 + T5 SQLite pending-feedback 状态 | 断言一个 public preflight 能取得 idle barrier，而并发 preflight 会被拒绝。响应 owner 被丢弃并排入修复后，preflight 仍不可用，持久化 pending disposition 为 no-terminal（`includes_terminal = false`）。 | `2515–2539` |
| `agentic-gpt-hub` | `crates/agentic-gpt-hub/src/storage/event_feedback.rs` — `delta_arriving_during_ack_is_settled_before_public_request_dispatch` | T4 feedback／ack／dispatch 行为链 + T5 SQLite outbox 副作用 | 启动 flush 并观察主 `EventSettle`；settlement 未完成时，断言不会发送 public `Exec`。主 ack 期间加入第二个 source，随后断言其 false-terminal delta 在 `Exec` dispatch 前也完成结算。两个 settlement 和模拟命令响应被确认后，flush 与请求成功，pending outbox 为空。 | `2415–2513` |
| `agentic-gpt-hub` | `crates/agentic-gpt-hub/src/runtime/main_tests.rs` — `hub_info_reports_safe_runtime_summary` | T4 runtime 投影／序列化／脱敏链 + T5 断言依赖 SQLite 查询得到的 registry 计数 | 构建并序列化 hub-info，检查服务／配置字段、注册与在线 agent 数为零、pending 请求数，并确认 topic／callback／API-key 字符串不会泄漏。断言的 `registeredCount` 来自对测试 registry 表执行的真实 SQLite 查询。 | `53–73` |
| `agentic-apply-patch` | `crates/agentic-apply-patch/src/lib.rs` — `applies_update` | T2 patch 解析／chunk 映射 + T4 parser→updater 跨组件更新链 | 从固定 patch 解析出 `UpdateFile` chunks，将 parser 产出的 chunks 传给 `apply_update`，再以 `PreserveLineEndings` 模式断言内存文本 `alpha\n` 变为 `beta\n`；核心断言覆盖解析／映射与跨组件应用的端到端结果。 | `36–55` |

`crates/agentic-browser-host` 已审查的 10 个用例中没有混合档用例。