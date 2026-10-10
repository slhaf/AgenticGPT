# 测试清理价值复审（用户已确认）

> 状态：**2026-10-08 用户认可本轮 31 项复审结论**，并确认原盘点的 **296 个低档清理候选无需再做一轮价值审查**。后续执行范围已汇总至 [`cleanup-manifest.md`](cleanup-manifest.md)，实际测试清理尚未启动；本文件保留复审理由。

## 范围与评估口径

- 基于仓库提交 `fa524d9` 的测试源码、实现和已有相关测试，重新评估 [`review-candidates.md`](review-candidates.md) 中「低档但值得保留的行为」的 **31 个纯 T1–T3 或仅含 T1–T3 的 MIXED**。那一节另列 4 个含 T4/T5 的 MIXED，不属于本轮 31 项。
- 按根目录 [`AGENTS.md`](../../AGENTS.md) 的五档标准判定覆盖内容；保留价值另看长期应维持的行为、未来扩展中的意外回归风险和独有失败模式。当前阶段的默认预设/工具集合/文本/实现快照不因目前正确就天然值得长期锁定。
- 经用户认可的本轮处理方向：**保留 20、合并/精简 8、清理 3**。这是后续实施方向，不代表已经修改代码或完成测试验证。

## 清理方向已确认（3；尚未执行）

| 测试（仓库相对路径::名称） | 原因与注意事项 |
| --- | --- |
| `crates/agentic-gpt/src/config/config.rs::managed_browser_policy_defaults_are_enabled_without_auto_provisioning` | 锁定当前 managed browser 默认启用、默认不自动安装的产品策略；未来变更并不天然属于回归。原候选清单的文件路径误写为 browser/browser_runtime.rs。 |
| `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::tunnel_local_and_http_ingress_advertise_identical_surface` | 把 Tunnel、Local 与 HTTP 三入口强制绑定为完全相同的工具表；未来合理分化会造成误报。执行前确认入口实际授权与契约测试仍覆盖可见性和来源标识。 |
| `crates/agentic-gpt/src/runtime/tunnel_distribution.rs::manifest_and_platforms_are_pinned` | 锁死阶段性的分发版本、URL、SHA-256；其他测试覆盖制品选择和哈希拒绝。但若团队明确要求第二处独立固定值审计，则应重新评估。 |

## 合并或精简方向已确认（8；先保住独有断言）

| 测试（仓库相对路径::名称） | 迁移/精简时必须关注的行为 |
| --- | --- |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::pinned_repository_key_has_exact_fingerprint` | 已由生产函数按同一指纹常量校验；宜改为真正独立的固定公钥/信任材料验证，或并入可信仓库验证测试。未证实完全重复前不直接删除。 |
| `crates/agentic-gpt/src/files/file_ops.rs::search_rejects_invalid_patterns_and_enforces_bounds` | 实际只检验非法 Regex、Glob，未在此验证 bounds；保留无效模式拒绝断言，并入搜索负例/边界相关测试。 |
| `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs::mcp_batch_descriptor_freezes_bounds_and_side_effect_annotations` | 保留公开 schema、实际资源上限及副作用 annotations 一致性验证；减少把当前数字硬编码成孤立快照。 |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::target_names_are_frozen` | 当前目标名称无需单独冻结；非法路径组件和非法 hash 仍是有效失败模式，合并到真实消费者的输入校验测试。 |
| `crates/agentic-gpt/src/browser/browser_kernel.rs::bootstrap_sends_escaped_path_and_frozen_browser_setup_code` | 保留 import 路径转义、初始化关键顺序和结果语义；精简对整段生成 JavaScript 文本的锁定，优先原位改写。 |
| `crates/agentic-gpt/src/config/config.rs::sparse_projection_always_keeps_selectors_and_omits_reconstructable_defaults` | 优先用保存→加载的行为验证必需 selector 和默认值可重建，合并至现有 durable writer/sparse load 测试；减少只判字段是否出现的快照断言。 |
| `crates/agentic-gpt/src/config/setup/review.rs::review_preserves_pending_actions_and_redacted_standalone_reference` | pending actions 与密钥脱敏有保留意义；可合并到 review_is_redacted_active_mode_only_and_reports_secret_write_intent 等相邻用例，但不得丢失 pending-action 负例。 |
| `crates/agentic-gpt-protocol/src/process.rs::process_read_defaults_view_and_bounds_wait` | 与同模块预算边界测试集中核对默认 view、wait 上限和无 cursor 等行为；合并前确认所有独有断言已迁移。 |

## 保留方向已确认（20；长期回归价值）

| 测试（仓库相对路径::名称） | 核心保留理由 |
| --- | --- |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::authenticated_inrelease_metadata_selects_exact_target` | 架构选择和校验元数据必须对应正确平台包。 |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::cleartext_signature_verifies_and_tampering_fails` | 篡改过的已签名元数据必须拒绝。 |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::fixed_repository_url_rejects_untrusted_path_forms` | 外部主机、路径穿越和 URL 注入需拒绝。 |
| `crates/agentic-gpt/src/config/config.rs::process_response_bytes_defaults_validates_bounds_and_rejects_invalid_values` | 输出预算必须拒绝越界/非法类型；具体默认值可随产品版本调整。 |
| `crates/agentic-gpt/src/mcp/mcp_tests.rs::server_config_validation_is_complete_and_typed` | 错误 MCP transport、命令或 URL 不能意外被接受。 |
| `crates/agentic-gpt-protocol/src/lib.rs::managed_mcp_batch_defaults_bounds_and_wire_type_are_frozen` | 批量 wire 字段和资源边界防止跨版本回归；不要求永远固定所有默认数字。 |
| `crates/agentic-gpt-protocol/src/process.rs::process_read_rejects_explicit_budgets_outside_shared_limits` | 显式请求不能绕过共享输出预算。 |
| `crates/agentic-browser-host/src/lib.rs::zero_malformed_and_non_utf8_payloads_are_errors` | 无效原生帧不能被接受为合法数据。 |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::overdue_restore_claim_is_trigger_once_at_due_boundary` | 到期触发边界和一次性状态迁移。 |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::future_restore_schedules_until_snoozed_due_boundary` | 延后到期前的 Schedule/Trigger 边界。 |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::degraded_item_stays_pending_until_due_and_can_be_terminal` | 降级任务可等待并进入终态。 |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::terminal_transition_clears_actions_and_rejects_duplicate_transition` | 终态清理后续动作、拒绝重复迁移。 |
| `crates/agentic-gpt/src/config/config.rs::room_repository_and_maintenance_use_the_v2_json_shape` | 持久配置的 v2 格式和对不受支持旧字段的处理；版本迁移时可调整契约。 |
| `crates/agentic-gpt/src/config/config.rs::sparse_projection_keeps_inactive_sections_and_redacts_config_secrets` | 模式切换不丢用户配置且安全摘要不泄密。 |
| `crates/agentic-gpt/src/config/config.rs::tunnel_secret_references_are_strict_and_safe_summary_is_redacted` | secret 引用合法性及安全摘要脱敏。 |
| `crates/agentic-gpt/src/config/setup/model.rs::mcp_server_draft_defaults_empty_and_saves_as_configured` | MCP 草稿保存与凭据的 Debug 脱敏；当前空默认值不是核心保留理由。 |
| `crates/agentic-gpt/src/config/setup/review.rs::review_is_redacted_active_mode_only_and_reports_secret_write_intent` | 预览不泄密、不展示非活动连接秘密，并明示写入意图。 |
| `crates/agentic-gpt/src/config/setup/validation.rs::active_input_ignores_inactive_connection_secrets_and_restores_staged_drafts` | 切换连接模式不串用凭据或丢失暂存草稿。 |
| `crates/agentic-gpt-protocol/src/lib.rs::maintenance_v2_shapes_close_slots_bound_wait_and_separate_sync` | 区分本地生效和同步结果，保护输入边界；slot 列表仍可按产品演进。 |
| `crates/agentic-gpt-protocol/src/lib.rs::install_and_run_protocol_defaults_and_command_names_are_stable` | Skills wire 名称和请求结构与消费者兼容；默认等待时间并非不可变。 |

## 与全量盘点及未来执行的关系

- 这份代码级复审只覆盖原先的 **31 个低档保留候选**。用户明确确认：原盘点的另外 **296 个低档清理候选沿用 Luna 的清理建议，不再逐项开展第二轮价值审查**；与本轮确认的 3 项合计为 **299 个拟清理用例**。这只是未来执行范围的记录，不等于已经删除。原盘点的 **7 个待判断用例、303 个涉及 T4/T5 的 MIXED** 仍未纳入本次清理决定。
- 原始分类和理由仍以 `review-candidates.md` 及两份 mixed/single-tier 索引作为查找入口；本文件是后续追加的**价值审查层**，不是对技术档位的重新分类。
- `managed_browser_policy_defaults_are_enabled_without_auto_provisioning` 在旧候选表中路径误标为 `browser/browser_runtime.rs`，实际源码位于 `config/config.rs`；本文件使用真实路径。
- 已生成独立的 [`cleanup-manifest.md`](cleanup-manifest.md)，载明 299 个删除目标的准确标识、8 个合并/精简目标、20 个保留项及验收要求。8 个合并/精简用例不能在丢失独有断言的情况下直接删除；296 个既定清理候选**无需重新开展价值评估**。
- 实际执行时仍需按仓库规则作最基本的源码定位、冲突检查与变更验证；如发现清单失效、独有长期有效断言明显未覆盖或必须改实现才能继续，停止相关项并报告，不自行扩展清理范围。

## 复审验证状态

- 已读取和比对相关测试与实现源码、查找邻近替代测试；**未运行 Rust/Kotlin 测试**。
- 仅整理审查文档；测试或实现代码保持原样。31 项处理方向及原有 296 个清理候选免二次审查的决定已由用户确认，执行尚未开始。
