# 建议保留或清理的测试用例

仅作盘点，供用户后续决定；本清单不表示要修改或删除任何测试或实现。档位依据实际断言的行为。技术分类中的 MIXED 总数不变；仅含 T1–T3 的混合用例按用户规则并入本清单，含 T4/T5 的混合用例仍列在专门的混合档清单中。

本清单中 31 个低档保留候选已有后续代码级价值复审，见 [cleanup-review.md](cleanup-review.md)（用户认可 20 个保留、8 个合并/精简、3 个清理）。用户也确认其余 296 个低档清理候选无需再做二次价值审查；执行目标已单独整理在 [cleanup-manifest.md](cleanup-manifest.md)。这里仍保留原始分类和候选理由，**尚未实际清理测试**。

## 低档但值得保留的行为

| 用例 | 档位 | 值得保护的行为 |
|---|---|---|
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::pinned_repository_key_has_exact_fingerprint` | T1 | 固定受信任仓库公钥指纹。 |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::authenticated_inrelease_metadata_selects_exact_target` | T2 | 将已认证的平台 metadata 映射到精确的软件包目标。 |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::cleartext_signature_verifies_and_tampering_fails` | T3 | 验证合法签名 metadata，并拒绝遭篡改的内容。 |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::fixed_repository_url_rejects_untrusted_path_forms` | T3 | 拒绝路径穿越、外部主机、query 和 fragment URL。 |
| `crates/agentic-gpt/src/config/config.rs::process_response_bytes_defaults_validates_bounds_and_rejects_invalid_values` | T3 | 锁定面向 agent 的输出预算默认值、合法边界和非法值拒绝规则。 |
| `crates/agentic-gpt/src/config/config.rs::explicit_import_clears_plaintext_tunnel_secret_and_reports_it` | MIXED (T2+T3+T5) | 检查明文 secret 清除策略、告警、脱敏和真实导入行为。 |
| `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::process_read_preserves_raw_byte_offsets_and_utf8_output` | MIXED (T2+T4+T5) | 保护字节游标／偏移和 UTF-8／base64 投影，并贯穿真实命令输出路径。 |
| `crates/agentic-gpt/src/mcp/mcp_tests.rs::server_config_validation_is_complete_and_typed` | T3 | 覆盖 MCP 配置的类型化校验分支及稳定错误。 |
| `crates/agentic-gpt/src/storage/event_store.rs::list_summaries_count_unicode_scalars_and_preserve_full_message_on_get` | MIXED (T3+T5) | 保护 Unicode 摘要截断规则，以及从持久化事件中读取完整消息。 |
| `crates/agentic-gpt/src/storage/process_history.rs::oversized_admission_metadata_rejects_the_entire_batch` | MIXED (T3+T5) | 验证超限拒绝具有原子性，SQLite 中不会留下部分 batch 行。 |
| `crates/agentic-gpt/src/files/file_ops.rs::search_rejects_invalid_patterns_and_enforces_bounds` | T3 | 覆盖无效 pattern 及结果数／上下文边界错误。 |
| `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs::mcp_batch_descriptor_freezes_bounds_and_side_effect_annotations` | T2 | 保护 public schema 的边界和副作用 metadata。 |
| `crates/agentic-gpt-protocol/src/lib.rs::managed_mcp_batch_defaults_bounds_and_wire_type_are_frozen` | MIXED (T2+T3) | 保护 public batch wire 字段、默认值、钳制和边界行为。 |
| `crates/agentic-gpt-protocol/src/process.rs::process_read_rejects_explicit_budgets_outside_shared_limits` | T3 | 确保 process-read 预算校验与共享限制一致。 |
| `crates/agentic-browser-host/src/lib.rs::zero_malformed_and_non_utf8_payloads_are_errors` | T2 | 覆盖 wire 边界对无效 framed payload 的解码处理。 |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::overdue_restore_claim_is_trigger_once_at_due_boundary` | T3 | 保护到期触发和单次 claim 行为。 |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::future_restore_schedules_until_snoozed_due_boundary` | T3 | 保护到期前 Schedule、到期边界 Trigger 的规则。 |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::degraded_item_stays_pending_until_due_and_can_be_terminal` | T3 | 保护 Degraded／Pending 与终态之间的行为。 |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt::terminal_transition_clears_actions_and_rejects_duplicate_transition` | T3 | 保护进入终态时清空 actions，以及拒绝重复终态转换。 |
| `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::target_names_are_frozen` | T1+T3 | 固定浏览器目标名，并拒绝路径穿越、错误空白和无效 hash；保护制品名称与校验边界。 |
| `crates/agentic-gpt/src/browser/browser_kernel.rs::bootstrap_sends_escaped_path_and_frozen_browser_setup_code` | T1+T2 | 检查注入到 bootstrap 脚本的路径经过转义，并锁定浏览器初始化代码，防止路径内容改变脚本语义。 |
| `crates/agentic-gpt/src/config/config.rs::managed_browser_policy_defaults_are_enabled_without_auto_provisioning` | T1+T2+T3 | 保护托管浏览器默认启用、但默认不自动安装的配置策略。 |
| `crates/agentic-gpt/src/config/config.rs::room_repository_and_maintenance_use_the_v2_json_shape` | T2+T3 | 保护 Room/maintenance v2 配置形状，并拒绝旧 `notebookRoot` 字段。 |
| `crates/agentic-gpt/src/config/config.rs::sparse_projection_always_keeps_selectors_and_omits_reconstructable_defaults` | T2+T3 | 保护稀疏配置投影必须保留选择器、可重建默认值可省略的规则。 |
| `crates/agentic-gpt/src/config/config.rs::sparse_projection_keeps_inactive_sections_and_redacts_config_secrets` | T2+T3 | 保护未激活配置段保留和 secret 脱敏，避免稀疏投影丢失用户数据或泄漏凭据。 |
| `crates/agentic-gpt/src/config/config.rs::tunnel_secret_references_are_strict_and_safe_summary_is_redacted` | T2+T3 | 保护 tunnel secret 引用的严格格式规则及安全摘要脱敏。 |
| `crates/agentic-gpt/src/config/setup/model.rs::mcp_server_draft_defaults_empty_and_saves_as_configured` | T2+T3 | 保护 MCP 草稿默认值与保存结果，并覆盖 token 往返和 Debug 脱敏。 |
| `crates/agentic-gpt/src/config/setup/review.rs::review_is_redacted_active_mode_only_and_reports_secret_write_intent` | T2+T3 | 保护仅展示活动模式、secret 脱敏及 secret 写入意图提示。 |
| `crates/agentic-gpt/src/config/setup/review.rs::review_preserves_pending_actions_and_redacted_standalone_reference` | T2+T3 | 保护 review 中待执行动作、独立 secret 引用和脱敏信息的保留。 |
| `crates/agentic-gpt/src/config/setup/validation.rs::active_input_ignores_inactive_connection_secrets_and_restores_staged_drafts` | T2+T3 | 保护活动配置输入忽略非活动连接 secret，并恢复暂存草稿的行为。 |
| `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::tunnel_local_and_http_ingress_advertise_identical_surface` | T1+T2 | 保护 local 与 HTTP ingress 公布相同工具描述和来源标识，防止入口契约分叉。 |
| `crates/agentic-gpt/src/runtime/tunnel_distribution.rs::manifest_and_platforms_are_pinned` | T1+T2 | 固定支持平台、运行时下载 URL、版本和 SHA-256，避免分发清单意外漂移。 |
| `crates/agentic-gpt-protocol/src/lib.rs::maintenance_v2_shapes_close_slots_bound_wait_and_separate_sync` | T2+T3 | 保护 maintenance v2 wire 名称、未知 slot 拒绝、数量/等待边界及同步状态语义。 |
| `crates/agentic-gpt-protocol/src/lib.rs::install_and_run_protocol_defaults_and_command_names_are_stable` | T2+T3 | 保护 `skills.install` / `skills.run` public command 名称、request 字段及等待默认值。 |
| `crates/agentic-gpt-protocol/src/process.rs::process_read_defaults_view_and_bounds_wait` | T2+T3 | 保护 process-read 的默认 view/wait、可选字段缺省，以及最大等待上限。 |

## 低档混合：建议清理（6）

以下用例只覆盖 T1–T3；不保留在混合档审查目录。建议清理，不代表已删除测试。

| 用例 | 档位 | 判断理由 |
|---|---|---|
| `crates/agentic-gpt/src/config/config.rs::max_active_processes_supports_auto_and_explicit_round_trips` | T2+T3 | 主要是 `auto`/数字配置的序列化往返和输入格式检查；核心自动限额公式由独立行为测试覆盖。 |
| `crates/agentic-gpt/src/config/config_cli.rs::registry_applies_new_scalar_and_list_keys` | T2+T3 | 主要验证 registry key 到配置字段的映射与类型解析，属机械配置搬运。 |
| `crates/agentic-gpt/src/config/config_cli.rs::toolset_listing_describes_all_namespaces_and_feedback_is_localized` | T1+T2 | 主要锁定静态双语列表和提示文案。 |
| `crates/agentic-gpt/src/config/setup/model.rs::tunnel_secret_reference_seeds_are_parsed_without_exposing_secret_text` | T2+T3 | 主要检查引用到草稿字段的映射与 Debug 脱敏；更严格的引用格式和摘要脱敏已由 `tunnel_secret_references_are_strict_and_safe_summary_is_redacted` 覆盖。 |
| `crates/agentic-gpt/src/runtime/main_tests.rs::batch_confirmation_preview_supports_chinese` | T1+T2 | 主要检查固定中文提示、cwd 和转义拼装。 |
| `crates/agentic-gpt/src/runtime/main_tests.rs::cli_version_uses_crate_version` | T1+T2 | 只保护版本号从 crate 元数据到 CLI 输出的机械转发。 |

## 低档混合：需用户判断（7）

以下用例只覆盖 T1–T3，但是否值得留下取决于对配置契约、命令边界或界面语义的维护价值；不确定性已记录，不升级为顾问咨询。

| 用例 | 档位 | 实际验证内容与待判断点 |
|---|---|---|
| `crates/agentic-gpt/src/browser/browser_runtime.rs::explicit_descriptor_allows_missing_codex_cli_path` | T2+T3 | 允许 descriptor 缺少可选 Codex CLI path，并省略启动变量；需判断与 `absent_codex_cli_path_is_supported` 的覆盖是否重复。 |
| `crates/agentic-gpt/src/config/config.rs::new_default_config_serializes_auto_limit` | T1+T2 | 固定新配置中的自动进程限制、搜索和确认通道默认值；需判断公开默认契约是否值得独立锁定，且与自动限制公式测试有部分重叠。 |
| `crates/agentic-gpt/src/config/config.rs::toolset_profiles_are_closed_and_deterministic` | T1+T3 | 固定工具集 profile/namespace 集合并测试启停、解析规则；需判断这属于值得保护的安全能力边界，还是静态枚举/独立规则测试。 |
| `crates/agentic-gpt/src/config/config_cli.rs::registry_updates_room_repository_and_maintenance_settings` | T2+T3 | 验证 Room 配置 key 更新、可选路径清空，以及非法模式/弃用 key 拒绝；需判断配置 CLI 契约是否需要单独保留。 |
| `crates/agentic-gpt/src/config/config_cli.rs::toolset_commands_dispatch_and_reject_unknown_namespaces` | T2+T3 | 验证 toolset list/enable/disable 命令解析及未知 namespace 拒绝；需判断命令输入边界的价值。 |
| `crates/agentic-gpt/src/config/setup/review.rs::event_review_preserves_values_and_offers_every_override_choice` | T1+T2 | 保留事件配置值并提供全部 override 选项；需判断 UI 选项完整性是否值得作为独立保护点。 |
| `crates/agentic-gpt/src/runtime/main_tests.rs::room_timezone_defaults_and_can_be_overridden` | T1+T2 | 固定默认时区及覆盖后的日期往返；测试没有覆盖时区跨日边界，需判断该默认/覆盖契约是否仍值得保留。 |

## 低价值或建议清理／补强的候选

| 用例 | 档位 | 具体问题 |
|---|---|---|
| `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::room_profile_dispatches_room_memory_tools` | MIXED (T4+T5) | 只检查 JSON 字段存在和类型，没有验证返回的 Room 数据或操作语义。 |
| `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::compact_mcp_skills_and_tmux_adapters_preserve_result_envelopes` | MIXED (T4+T5) | 两次 tmux 调用都允许返回错误仍通过，适配器即使全部失败也可能通过。 |
| `crates/agentic-gpt/src/runtime/tunnel_distribution.rs::artifact_lock_serializes_concurrent_installers` | T5 | sleep 并释放第一把锁后只检查第二方最终能获取；没有断言持锁期间 waiter 确实被阻塞。 |
| `crates/agentic-gpt/src/files/file_ops.rs::absent_commit_uses_no_replace_and_overwrite_preserves_permissions` | T5 | 直接调用 `fs::hard_link`／`fs::rename`，没有经过名称所暗示的生产 commit 路径。 |
| `crates/agentic-gpt/src/process/managed.rs::shell_eval_preserves_dash_leading_command_text` | T5 | 只检查 exit 127／Failed，没有检查捕获的命令或输出来证明前导文本被保留。 |
| `crates/agentic-gpt/tests/config_cli.rs::config_import_compatible_round_trip_preserves_http_mcp_fields` | T5 | 非 TTY 下 import 预期失败；后续 `show` 读到的是 `init` 创建的源文件，而非成功导入后的配置。 |
| `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs::every_advertised_tool_is_accepted_by_apps_dispatcher` | T4 | 只排除 “unknown tool”；任意参数／运行时失败都可通过。 |
| `crates/agentic-browser-host/src/lib.rs::identifier_free_notifications_forward_and_broadcast` | T4 | 只验证经内存 buffer 转发／广播通知。 |
| `crates/agentic-apply-patch/src/lib.rs::parses_patch` | T2 | 解析 add/delete/update 形式，但只断言有三个 hunks，没有验证各 hunk 的内容或类型。 |
| Kotlin `SharedCommonTest.example`、`SharedLogicDesktopTest.example`、`SharedLogicAndroidHostTest.example` | T1 | 三个用例都只断言固定算式 `1 + 2 == 3`。 |

唯一的 ignored 浏览器软件包物化用例 `crates/agentic-gpt/src/browser/browser_distribution_tests.rs::real_package_materializes_from_explicit_fixture_path` 未列为低价值：它需要经过校验的 release package 输入，并检查真实软件包物化及 runtime 布局。另有 3 个 Rust 用例需要人工检查跨平台适用性；见 `findings.md`。
