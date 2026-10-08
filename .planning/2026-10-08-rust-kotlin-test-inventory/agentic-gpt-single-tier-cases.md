# Agentic GPT 单一层级测试用例

**单一层级声明数：331。** 本文件逐项列出每个最终归类为单一层级（T1–T5）的声明，按 repository-relative 源文件分组；已排除单独索引于 `agentic-gpt-mixed-cases.md` 的 323 个 MIXED 声明。每行包含函数名、最终 tier、实际断言行为和源代码行证据。

## 数量核对

单一层级用例共 331 项：T1=11、T2=73、T3=134、T4=59、T5=54。与混合用例清单中的 MIXED=323 合并后，完整 crate 清单为 654 项（331+323）。MIXED 用例只在混合清单中计数一次。

| 范围 | 单一层级用例数 | T1 | T2 | T3 | T4 | T5 |
|---|---:|---:|---:|---:|---:|---:|
| browser/config | 108 | 3 | 28 | 50 | 24 | 3 |
| ingress/operations | 62 | 7 | 10 | 34 | 8 | 3 |
| MCP/room | 32 | 0 | 3 | 5 | 12 | 12 |
| storage/skills | 11 | 0 | 2 | 7 | 0 | 2 |
| UI/support/tmux/integration | 51 | 1 | 14 | 8 | 4 | 24 |
| runtime/files/process | 67 | 0 | 16 | 30 | 11 | 10 |
| **合计** | **331** | **11** | **73** | **134** | **59** | **54** |

每项仅归入一个 tier；T1–T3 的保留标记为 `[KEEP]`。明确低价值的 T4/T5 个案保留 `[LOW-VALUE]` 标记及具体原因。三项需要复核的 runtime 个案都属于 MIXED，已在混合清单中标明；本文件只列最终单一层级用例。

## 逐用例清单

### `crates/agentic-gpt/src/browser/browser_distribution_tests.rs`
- `authenticated_inrelease_metadata_rejects_duplicates_and_contradictions` — T3 [KEEP]；拒绝重复的索引记录及冲突的 Suite/Codename 元数据 [证据 `L133`]
- `authenticated_inrelease_metadata_selects_exact_target` — T2 [KEEP]；选择 amd64 索引，并将其映射到软件包获取路径 [证据 `L106`]
- `cleartext_signature_hash_policy_is_sha2_only` — T3 [KEEP]；验证测试夹具，并检查接受和拒绝的哈希算法 [证据 `L93`]
- `cleartext_signature_verifies_and_tampering_fails` — T3 [KEEP]；接受已签名测试夹具，并以稳定的签名错误拒绝被篡改的已签名文本 [证据 `L78`]
- `fixed_repository_url_rejects_untrusted_path_forms` — T3 [KEEP]；检查固定的 HTTPS 主机，并拒绝路径穿越、外部 URL、查询及片段形式 [证据 `L290`]
- `injected_fetcher_rejects_status_and_metadata_bounds` — T3 [KEEP]；注入的获取器返回重定向和超大响应体；两者均被拒绝 [证据 `L261`]
- `package_selector_rejects_ambiguity_duplicate_fields_and_unsafe_names` — T3 [KEEP]；拒绝多个候选项、重复字段和含路径穿越的文件名 [证据 `L578`]
- `package_selector_validates_required_identity_fields` — T3 [KEEP]；拒绝格式错误的版本、零大小和格式错误的摘要 [证据 `L616`]
- `packages_digest_and_size_are_verified_before_selection` — T3 [KEEP]；精确验证索引摘要和大小，并在选择软件包前拒绝错误的摘要或大小 [证据 `L498`]
- `pinned_repository_key_has_exact_fingerprint` — T1 [KEEP]；解析固定密钥并精确比较指纹 [证据 `L69`]
- `release_date_parser_accepts_rfc2822_timestamp` — T3 [KEEP]；接受有效的 RFC2822 日期，并拒绝无效时区 [证据 `L127`]
- `selected_entry_and_size_bounds_are_enforced_without_large_fixtures` — T3 [KEEP]；使用计数器检查已选条目、单文件和总大小限制 [证据 `L1033`]
- `selected_paths_reject_traversal_and_empty_aliases` — T3 [KEEP]；接受正常的已选路径，拒绝含路径穿越或为空的别名，忽略无关形式及含反斜杠的形式 [证据 `L955`]
- `target_locks_serialize_without_global_lock` — T5；生产环境锁文件阻止同路径的竞争者，直至锁释放，而不同路径可独立获取锁 [证据 `L1182`]
- `unix_lock_reuses_existing_path_with_stale_owner_metadata` — T5；生产环境 Unix 锁获取并移除含过期所有者元数据的文件。`#[cfg(unix)]` 位于 L1195 [证据 `L1197`]

### `crates/agentic-gpt/src/browser/browser_kernel.rs`
- `bootstrap_converts_tool_error_to_stable_failure` — T2 [KEEP]；将引导工具错误映射为稳定的失败 [证据 `L768`]
- `bootstrap_preserves_rmcp_service_failure_prefix` — T2 [KEEP]；检查引导期间的 RMCP 失败前缀 [证据 `L790`]
- `bootstrap_sentinel_maps_to_browser_unavailable` — T2 [KEEP]；将模拟哨兵值映射为浏览器不可用错误 [证据 `L748`]
- `command_helper_uses_direct_program_cwd_and_env_overrides` — T2 [KEEP]；将启动规范映射为直接执行的可执行文件、cwd、无参数和环境覆盖配置 [证据 `L327`]
- `constructor_rejects_empty_or_whitespace_session_ids` — T3 [KEEP]；针对内存服务，拒绝空或仅含空白的会话 ID [证据 `L390`]
- `constructor_rejects_empty_or_whitespace_turn_ids` — T3 [KEEP]；针对内存服务，拒绝空或仅含空白的轮次 ID [证据 `L408`]
- `impossible_executable_is_a_spawn_failure` — T5；生产环境进程启动逻辑尝试为缺失的可执行文件创建 OS 进程，并检查进程启动错误前缀 [证据 `L371`]
- `js_attaches_exact_codex_turn_metadata` — T2 [KEEP]；检查转发的会话和轮次元数据 [证据 `L455`]
- `js_keeps_tool_error_result_as_successful_transport_return` — T2 [KEEP]；检查工具错误结果是否仍为成功的传输返回，并带有 `is_error=true` [证据 `L667`]
- `js_prefixes_rmcp_service_failures` — T2 [KEEP]；检查 JS 服务错误的稳定前缀 [证据 `L683`]
- `js_preserves_mixed_result_fields_unchanged` — T2 [KEEP]；检查文本、图像、结构化、错误和元数据结果是否等于模拟结果 [证据 `L651`]
- `js_sends_exact_tool_name_and_arguments` — T2 [KEEP]；检查是否将 `js`、代码和超时转发给模拟服务 [证据 `L426`]
- `node_repl_client_info_uses_the_frozen_initialize_contract` — T1 [KEEP]；检查固定的协议、能力、客户端名称和软件包版本 [证据 `L317`]
- `reset_js_converts_tool_error_to_stable_failure` — T2 [KEEP]；将模拟工具错误映射为稳定的重置错误 [证据 `L548`]
- `reset_js_prefixes_rmcp_service_failures` — T2 [KEEP]；检查模拟 RMCP 服务失败的稳定前缀 [证据 `L583`]
- `reset_js_sends_exact_tool_name_and_empty_arguments` — T2 [KEEP]；检查不带参数的 `js_reset` [证据 `L509`]
- `shutdown_cleanly_terminates_an_in_memory_service` — T4；关闭内核并等待模拟内存服务终止 [证据 `L637`]
- `spawn_rejects_invalid_ids_before_attempting_process_creation` — T3 [KEEP]；在尝试运行故意不存在的可执行文件前，拒绝空白会话 ID [证据 `L355`]
- `turn_ended_converts_tool_error_to_stable_failure` — T2 [KEEP]；将模拟工具错误映射为稳定的轮次已结束错误 [证据 `L532`]
- `turn_ended_prefixes_rmcp_service_failures` — T2 [KEEP]；检查模拟 RMCP 服务失败的稳定前缀 [证据 `L564`]
- `turn_ended_sends_exact_tool_name_and_arguments` — T2 [KEEP]；检查 `turn_ended` 名称以及 Stop、会话和轮次参数 [证据 `L479`]

### `crates/agentic-gpt/src/browser/browser_manager.rs`
- `acquire_waits_for_closing_release_then_creates_fresh_kernel` — T4；acquire 等待被阻塞的 shutdown 完成，然后创建第二个内核 [证据 `L1123`]
- `closed_ready_kernel_is_removed_before_reacquire` — T4；将模拟内核标记为已关闭，并检查重新获取时是否创建全新内核 [证据 `L1267`]
- `concurrent_same_name_acquire_shares_one_initialization` — T4；同名获取共享一次工厂初始化，并留下一个超时时间较晚的租约 [证据 `L788`]
- `different_names_initialize_concurrently` — T4；由屏障控制的不同名称获取并发初始化 [证据 `L844`]
- `each_in_place_reset_failure_falls_back_to_factory` — T4；在 reset 的每个步骤注入失败，并验证回退顺序 [证据 `L1437`]
- `failed_initialization_removes_entry_and_allows_retry` — T4；工厂初始化失败时移除条目；后续获取成功 [证据 `L958`]
- `failed_reset_recovery_removes_entry_and_allows_reacquire` — T4；恢复失败时移除 entry；后续获取会创建另一个内核 [证据 `L1472`]
- `final_cleanup_times_out_stuck_turn_ended_and_still_shuts_down` — T4；被阻塞的 turn-ended 超时，但 shutdown 仍会执行 [证据 `L1041`]
- `healthy_reset_preserves_entry_and_kernel_and_orders_steps` — T4；reset 保留 entry/kernel/timeout，并记录 turn-ended/reset/bootstrap 的顺序 [证据 `L1303`]
- `idle_reaper_removes_inactive_lease` — T4；较短的超时时间会触发有序清理和租约移除 [证据 `L1164`]
- `release_orders_turn_ended_before_shutdown` — T4；release 在 shutdown 前调用 turn-ended，并移除租约 [证据 `L992`]
- `release_removes_after_shutdown_error_and_reacquire_is_fresh` — T4；shutdown 失败后仍移除租约，重新获取全新内核，并处理租约不存在时的 release [证据 `L1090`]
- `release_reports_combined_cleanup_failure_and_removes_entry` — T4；汇总清理失败，保持顺序，并移除租约 [证据 `L1059`]
- `release_turn_ended_error_still_shuts_down_and_removes_entry` — T4；turn-ended 失败时仍执行 shutdown 并移除租约，同时返回错误 [证据 `L1014`]
- `repl_activity_wins_race_with_idle_reaper` — T4；被阻塞的活跃 REPL 阻止过早回收；租约随后过期 [证据 `L1189`]
- `repl_serializes_per_lease_and_overlaps_across_leases` — T4；同一租约的 REPL 调用串行执行；不同租约的调用重叠执行 [证据 `L891`]
- `reset_does_not_revive_closing_or_removed_lease` — T4；在关闭过程中及移除后拒绝 reset [证据 `L1517`]
- `reset_refreshes_activity_before_idle_reaper_rechecks` — T4；被阻塞的 reset 刷新活动时间，使回收器等待；租约随后过期 [证据 `L1599`]
- `reset_respawns_known_closed_kernel_in_same_entry` — T4；对已关闭的内核执行 shutdown，并在同一 entry 中安装全新内核 [证据 `L1390`]
- `reset_serializes_same_lease_and_overlaps_different_leases` — T4；reset 阻止同一租约的 REPL 执行，同时允许不同租约继续执行 [证据 `L1332`]
- `reset_waits_for_initialization_before_operating` — T4；等待初始化完成，然后按顺序执行 reset，且不额外调用工厂 [证据 `L1548`]

### `crates/agentic-gpt/src/browser/browser_manual.rs`
- `search_rejects_query_and_context_bounds` — T3 [KEEP]；在读取文件前拒绝空白查询、零结果数和过大的上下文 [证据 `L478`]

### `crates/agentic-gpt/src/browser/browser_runtime.rs`
- `explicit_descriptor_derives_shared_docs_and_trusted_paths` — T3 [KEEP]；从内存中的显式配置推导文档根目录和信任路径 [证据 `L812`]
- `explicit_descriptor_rejects_empty_and_relative_fields` — T3 [KEEP]；拒绝为空、为相对路径或缺失的必需 Node 路径，以及为相对路径的可选 CLI 路径 [证据 `L839`]
- `launch_spec_creates_browser_only_trusted_services_when_absent` — T2 [KEEP]；将描述符映射为仅含浏览器的 trusted-service 对象 [证据 `L537`]
- `launch_spec_creates_trusted_code_paths_when_base_omits_them` — T2 [KEEP]；根据描述符创建可信路径环境 [证据 `L471`]
- `launch_spec_derives_program_and_cwd_from_descriptor` — T2 [KEEP]；将运行时 REPL 路径和 bundle 的父目录映射到 program/cwd [证据 `L400`]
- `launch_spec_does_not_create_empty_node_module_dirs` — T3 [KEEP]；两个来源均未提供环境键时，省略该键 [证据 `L582`]
- `launch_spec_empty_node_module_dirs_preserves_base_value` — T3 [KEEP]；描述符列表为空时，保持调用方的 module-dir 值不变 [证据 `L566`]
- `launch_spec_merges_and_deduplicates_trusted_code_paths` — T3 [KEEP]；保留调用方顺序，添加运行时路径，并去重 [证据 `L483`]
- `launch_spec_overwrites_stale_runtime_coupled_values` — T2 [KEEP]；用描述符中的值替换过时的运行时环境值 [证据 `L409`]
- `launch_spec_preserves_security_mode_without_inventing_it` — T2 [KEEP]；省略未提供的安全模式，并保留调用方的值 [证据 `L592`]
- `launch_spec_preserves_unrelated_base_environment` — T2 [KEEP]；保留自定义变量，并合并调用方与运行时的信任路径 [证据 `L447`]
- `launch_spec_rejects_invalid_trusted_services` — T3 [KEEP]；拒绝格式错误或非对象的值 [证据 `L552`]
- `launch_spec_replaces_browser_trusted_service_and_preserves_others` — T3 [KEEP]；替换浏览器的可信服务，同时保留另一个服务 [证据 `L517`]

### `crates/agentic-gpt/src/config/config.rs`
- `auto_max_active_processes_uses_the_frozen_formula` — T3 [KEEP]；检查不同并行度以及显式值和未知值情况下的 auto 解析 [证据 `L2617`]
- `confirmation_provider_disk_shape_rejects_legacy_provider` — T3 [KEEP]；拒绝内存中提供的旧版 provider 结构的 JSON [证据 `L2709`]
- `confirmation_provider_rejects_duplicate_or_unknown_channels` — T3 [KEEP]；拒绝重复和不受支持的通道 [证据 `L2717`]
- `empty_browser_section_is_omitted_from_sparse_defaults` — T2 [KEEP]；检查稀疏投影中不包含空的 browser 节 [证据 `L3428`]
- `file_search_context_limit_defaults_and_rejects_invalid_values` — T3 [KEEP]；检查默认值、可接受的值，以及无效范围和类型 [证据 `L2542`]
- `http_bearer_file_references_require_absolute_paths_without_tightening_tunnel_refs` — T3 [KEEP]；HTTP MCP 要求绝对文件引用，而 tunnel 引用接受相对文件形式 [证据 `L2894`]
- `hub_validation_rejects_invalid_url_and_transport_with_stable_errors` — T3 [KEEP]；拒绝无效的 URL/transport，并返回稳定的错误 [证据 `L3482`]
- `legacy_confirmation_labels_map_to_canonical_fallback_order` — T2 [KEEP]；将旧版标签映射到回退显示值及顺序 [证据 `L2729`]
- `limits_reject_retired_max_active_jobs_field` — T3 [KEEP]；拒绝已废弃字段，并检查错误是否指向当前字段 [证据 `L2605`]
- `managed_browser_policy_rejects_unknown_fields` — T3 [KEEP]；拒绝内存中 JSON 的未知 managed-browser 字段 [证据 `L3471`]
- `mcp_server_semantics_are_validated_before_standalone_use` — T3 [KEEP]；接受受支持的 HTTP 语义，并拒绝 `sse` 传输 [证据 `L2811`]
- `process_response_bytes_defaults_validates_bounds_and_rejects_invalid_values` — T3 [KEEP]；检查默认值、边界，以及无效的过低值、过高值、类型和字符串值 [证据 `L2572`]
- `shell_init_file_serde_preserves_default_disabled_and_explicit_paths` — T2 [KEEP]；对 default/disabled/explicit 模式进行往返转换，检查 JSON 结构，并确保摘要省略路径 [证据 `L2854`]

### `crates/agentic-gpt/src/config/config_cli.rs`
- `interactive_init_requires_all_three_terminals_and_no_non_interactive_flag` — T3 [KEEP]；检查终端/非交互式决策表 [证据 `L576`]
- `registry_keys_are_unique_and_have_bilingual_metadata` — T1 [KEEP]；检查静态键唯一，且附有双语描述和示例 [证据 `L637`]
- `setup_seed_conversion_preserves_editable_flags_and_redacts_agent_secret` — T2 [KEEP]；将 CLI 参数映射到 seed，并检查机密信息脱敏 [证据 `L649`]

### `crates/agentic-gpt/src/config/config_templates.rs`
- `explicit_confirmation_language_wins_and_normal_room_override_requires_room_toolset` — T3 [KEEP]；遵循显式指定的语言，并在缺少 Room 工具集时拒绝 Room 覆盖设置 [证据 `L608`]
- `local_mode_ignores_tunnel_inputs` — T3 [KEEP]；在 Local 模式下提供 tunnel/reporting 输入，并检查不存在 tunnel 配置 [证据 `L494`]
- `mcp_servers_flow_into_built_config` — T2 [KEEP]；将 MCP 映射复制到构建的配置中 [证据 `L405`]
- `normal_profile_accepts_room_override_with_explicit_room_toolset` — T3 [KEEP]；当 Normal 启用 Room 时，接受并构建 Room 设置 [证据 `L623`]
- `partial_hub_template_rejects_invalid_url_and_transport_with_stable_errors` — T3 [KEEP]；检查无效 Hub URL/transport 的错误保持稳定 [证据 `L639`]
- `pending_actions_are_deterministic_and_unique` — T3 [KEEP]；检查 Standalone 待处理操作可重复，且 Hub 待处理操作唯一 [证据 `L548`]
- `secret_debug_output_is_redacted` — T3 [KEEP]；检查机密信息包装器和 write-plan 调试输出的脱敏 [证据 `L591`]
- `templates_cover_all_runtime_modes_and_profiles` — T4；构建每种 mode/profile，并调用对应的验证器 [证据 `L465`]

### `crates/agentic-gpt/src/config/setup/model.rs`
- `malformed_tunnel_secret_reference_is_reported_as_a_field_error` — T3 [KEEP]；将格式错误的文件引用映射到 secret-path 字段和稳定的错误码 [证据 `L1165`]
- `optional_status_and_drafts_survive_mode_and_profile_changes` — T3 [KEEP]；检查 mode/profile 切换前后的状态和暂存草稿 [证据 `L1254`]
- `room_availability_uses_profile_preset_without_explicit_toolset_selection` — T3 [KEEP]；检查 Room 可用性遵循 profile 预设 [证据 `L1224`]
- `setup_defaults_to_standalone_normal_and_preserves_inactive_mode_seeds` — T3 [KEEP]；检查所选 seed 的 mode/profile，以及暂存草稿在模式切换后仍然保留 [证据 `L1016`]

### `crates/agentic-gpt/src/config/setup/review.rs`
- `review_reports_default_and_configured_optional_statuses` — T3 [KEEP]；检查保存默认身份和已配置身份前后的状态 [证据 `L65`]
- `review_rows_expose_stable_edit_contract_without_secret_material` — T2 [KEEP]；将 secret/MCP 行映射到 editor/target 契约，并对机密信息脱敏 [证据 `L126`]
- `shell_review_shows_path_only_for_explicit_path_mode` — T3 [KEEP]；仅在显式路径模式下显示 shell 路径行，并检查选项 [证据 `L171`]

### `crates/agentic-gpt/src/config/setup/validation.rs`
- `hub_connection_transport_and_secret_are_structured` — T3 [KEEP]；返回结构化的 Hub URL/transport/secret 错误 [证据 `L65`]
- `normal_profile_accepts_explicit_room_toolset_and_round_trips_room_settings` — T4；启用 Room，保存草稿，构建配置，并检查设置 [证据 `L263`]
- `normal_profile_with_explicit_room_disabled_toolset_hides_and_rejects_room` — T3 [KEEP]；检查已禁用的 Room 被隐藏/标为不适用，且在保存时被拒绝 [证据 `L310`]
- `optional_validation_covers_paths_numbers_runtime_paths_and_reporting` — T3 [KEEP]；检查无效的工作区、限制、运行时路径、Room/reporting 及适用性 [证据 `L82`]
- `process_response_bytes_limit_accepts_protocol_bounds_and_rejects_invalid_values` — T3 [KEEP]；接受协议边界值，拒绝无效值，并检查保存失败时保留原有草稿 [证据 `L169`]
- `required_connection_fields_report_concrete_domain_fields` — T3 [KEEP]；将缺失的 tunnel ID 映射到具体字段 [证据 `L47`]

### `crates/agentic-gpt/src/files/file_ops.rs`
- `absent_commit_uses_no_replace_and_overwrite_preserves_permissions` — T5；暂存真实文件，并检查 no-replace 冲突及 overwrite 模式的行为 [LOW-VALUE: 直接调用 `fs::hard_link` 和 `fs::rename`，而非调用名称所暗示的生产 commit 路径。] [证据 `L3127–L3156 (`#[cfg(unix)]`)`]
- `add_preflight_reserves_audit_path_under_symlinked_workspace` — T5；Unix 工作区别名不能在保留的审计路径下添加内容；确认未创建审计文件 [证据 `L2667–L2684 (`#[cfg(unix)]`)`]
- `apply_patch_parser_handles_add_delete_update_and_move_without_fs_access` — T2；解析混合 Add/Delete/Update/Move patch，并检查解析出的 hunk 数量 [证据 `L3159–L3162`]
- `bounded_diff_counts_create_delete_crlf_and_final_newline` — T2；检查创建/删除计数、CRLF 保留、末尾换行标记及无变更情况 [证据 `L3079–L3108`]
- `bounded_diff_preserves_blank_lines_and_emits_disjoint_hunks` — T3；检查变更行数，以及被未变更上下文隔开的两个独立 hunk [证据 `L3063–L3075`]
- `bounded_diff_truncates_utf8_after_computing_complete_counts` — T3；检查输出上限/UTF-8 边界，同时保留完整的变更计数 [证据 `L3111–L3123`]
- `rejects_duplicate_and_ancestor_patch_paths` — T3；重复路径和父/子路径返回 ambiguous-paths 错误 [证据 `L3165–L3176`]
- `search_rejects_invalid_patterns_and_enforces_bounds` — T3；格式错误的 regex 和 include glob 分别返回各自的特定错误 [证据 `L3012–L3059`]

### `crates/agentic-gpt/src/ingress/http_oauth.rs`
- `chatgpt_redirect_families_are_exact` — T3 [KEEP]；仅接受两种指定的 ChatGPT HTTPS 重定向 URI，拒绝空路径、查询/片段、显式端口、其他主机、HTTP 和用户信息 [证据 `L945–L964`]
- `expired_authorization_codes_are_consumed_and_rejected` — T4；兑换过期授权码返回 400 `invalid_grant`、不包含访问令牌，并从授权码存储中删除该码 [证据 `L1095–L1134`]
- `expired_oauth_tokens_are_pruned_and_rejected` — T3 [KEEP]；过期 OAuth bearer token 不被接受，且会从 token 存储中清除 [证据 `L1076–L1093`]
- `host_and_origin_matching_follow_rmcp_policy` — T3 [KEEP]；允许策略匹配的 Host 与 Origin、拒绝外部 Origin 并返回 403，缺少 Origin 时仍通过 [证据 `L990–L1015`]
- `oauth_error_and_html_responses_are_uncacheable` — T1 [KEEP]；OAuth 错误响应状态为 400 且带 `no-store`/`no-cache`，授权 HTML 页带 `no-store` [证据 `L1136–L1158`]
- `oauth_tokens_reject_wrong_resource_binding` — T3 [KEEP]；OAuth token 绑定到错误资源 URL 时，即使 token 未过期也不接受 [证据 `L1039–L1055`]
- `pkce_rfc7636_s256_vector_matches` — T3 [KEEP]；RFC 7636 S256 示例 verifier/challenge 校验成功，错误 verifier 校验失败 [证据 `L933–L943`]
- `public_url_normalization_uses_config_contract` — T2 [KEEP]；规范化去除合法 HTTPS URL 两侧空白及尾随斜杠，并拒绝非 HTTPS、缺主机、路径、用户信息、点段、重复斜杠、查询和片段 [证据 `L966–L988`]
- `standalone_page_escapes_hidden_fields_and_avoids_hub_copy` — T2 [KEEP]；独立授权页包含 HTTP MCP bearer token 文案、不含 Hub API key 字段，并对隐藏字段中的尖括号进行转义 [证据 `L1057–L1074`]
- `token_rotation_revokes_oauth_state_but_direct_bearer_changes` — T3 [KEEP]；替换直接 bearer token 后旧直接 token 与旧 OAuth token 均失效，新直接 token 有效，且 OAuth token 不能用于错误资源 [证据 `L1017–L1037`]

### `crates/agentic-gpt/src/ingress/hub.rs`
- `oversized_report_json_becomes_a_hash_record` — T2 [KEEP]；超出上限的 JSON 被标记为截断，记录原字节数及与原值对应的 SHA-256 [证据 `L1577–L1589`]

### `crates/agentic-gpt/src/ingress/local_control.rs`
- `bind_is_private_rejects_second_listener_and_cleans_on_drop` — T5；绑定实际 Unix socket 后运行目录权限为 0700、socket 权限为 0600，同一路径拒绝第二个监听器，丢弃监听器会删除 socket 和空目录 [证据 `L349–L375`]
- `bind_rejects_symlink_runtime_directory` — T5；运行目录为符号链接时拒绝绑定，并返回 `local_mcp_runtime_path_unsafe` [证据 `L396–L413`]
- `bind_replaces_owned_stale_socket_but_rejects_regular_file` — T5；替换自有的遗留 socket 可成功绑定，而同路径普通文件会以 `local_mcp_socket_path_unsafe` 拒绝 [证据 `L377–L394`]
- `socket_path_rejects_invalid_identity_and_oversized_path` — T3 [KEEP]；身份含斜杠或 socket 路径超过上限时均判为无效 [证据 `L415–L420`]

### `crates/agentic-gpt/src/ingress/stdio_server_tests.rs`
- `absent_room_tools_are_rejected_when_room_toolset_disabled` — T4；Normal worker 调用列出的 bootstrap、room 日记、维护、笔记本和状态工具时均返回 `METHOD_NOT_FOUND` [证据 `L1166–L1187`]
- `active_response_precedes_terminal_for_both_serial_orderings` — T3 [KEEP]；无论终态记录先到还是活动响应先到，均先发出活动响应，再发出托管进程终态事件 [证据 `L2372–L2393`]
- `batch_lifecycle_detection_reads_process_envelopes` — T2 [KEEP]；从 processes 信封识别 running 进程为活动且非终态失败，并从 failed 进程提取终态失败及其错误消息 [证据 `L2457–L2481`]
- `browser_conditional_arguments_and_bounds_are_validated` — T3 [KEEP]；验证 browser.manual 的 read/search 条件参数、边界及未知动作，browser.acquire 的超时范围和 browser.repl 的非空代码、超时与标题长度限制 [证据 `L643–L705`]
- `browser_descriptors_and_annotations_are_frozen` — T1 [KEEP]；固定各 browser 工具的必填字段、禁止额外属性、三类注解，并验证 manual 字段集合及 repl 超时/标题约束 [证据 `L575–L641`]
- `browser_list_maps_runtime_snapshots_and_release_is_idempotent` — T4；创建两个租约后列表按名称排序并映射运行时信息和空闲秒数、不泄漏内部字段；释放不存在租约返回 false，释放现有租约成功 [证据 `L824–L874`]
- `browser_reporting_and_error_helpers_do_not_retain_source` — T3 [KEEP]；报告只保留代码字节数与 SHA-256、不含源代码，错误输出仅包含错误码而不泄漏路径或私有诊断 [证据 `L754–L781`]
- `concurrent_check_clear_enqueue_interleaving_is_linearizable` — T3 [KEEP]；并发记录终态事件与活动响应时，输出活动响应及恰好一个托管进程事件 [证据 `L2395–L2420`]
- `event_api_business_errors_keep_their_code_and_include_the_panel` — T4；缺失事件和无效游标分别保留 `event_not_found`、`event_cursor_invalid` 错误码，且错误结果包含当前事件面板 [证据 `L325–L348`]
- `event_api_schemas_preserve_defaults_and_empty_mark_boundary` — T1 [KEEP]；固定 event.list 的 pending/20 默认值，event.mark 的 eventIds 不声明最小项数且最大为 512 [证据 `L481–L498`]
- `every_room_adapter_rejects_unknown_identity_fields` — T3 [KEEP]；所有列出的 room 日记、笔记本和状态适配器均拒绝传入未知的 agentId 字段 [证据 `L2635–L2665`]
- `file_lock_registry_prunes_released_paths` — T3 [KEEP]；依次取得并释放两个不同文件路径的锁后，注册表仅保留一个条目 [证据 `L3655–L3667`]
- `file_surface_schema_is_exact` — T1 [KEEP]；固定 file 工具集合无 file.batch，read/search 的 requests 为必填以外字段且数组范围为 1–32，edit 仅含 needConfirm/patch 并要求 patch [证据 `L500–L550`]
- `in_process_room_stdio_initialize_list_and_call` — T4；通过进程内 stdio 完成初始化、列出 room.diary.active 并成功调用，返回 daily 对象 [证据 `L1343–L1369`]
- `in_process_stdio_initialize_list_and_call` — T4；通过进程内 stdio 完成初始化、列出并成功调用 process.list 和 skills.list，processes 为空且无 jobs，普通配置下 bootstrap 调用失败 [证据 `L1298–L1341`]
- `inline_terminal_tracker_discards_pending_terminal_event` — T3 [KEEP]；活动响应成功完成后清空待发终态事件、状态标记为 Inline，且不发送事件 [证据 `L2353–L2370`]
- `managed_terminal_event_includes_duration` — T3 [KEEP]；终态消息包含 42ms 时长及 12 字符的人类可读进程标识，不暴露原始进程 ID、参数或路径 [证据 `L2330–L2351`]
- `mcp_batch_projection_preserves_compact_observation_and_child_identity` — T2 [KEEP]；批次投影保留批次及子项身份、completed 状态和可用结果/字节数，并以 unavailable 标示缺失结果且省略冗余字段 [证据 `L1949–L2048`]
- `mutating_tool_annotations_do_not_promise_read_only_or_additive_effects` — T1 [KEEP]；固定列出的变更类工具均标记 readOnlyHint=false、destructiveHint=true [证据 `L552–L573`]
- `process_read_input_schema_advertises_wait_view_and_response_budget` — T1 [KEEP]；固定相关工具的 group 长度上限、process.read 的只读注解、必填 processId、wait/view/maxBytes/cursor 约束及 process.list 筛选字段 [证据 `L1894–L1947`]
- `room_maintenance_descriptors_are_frozen` — T1 [KEEP]；固定 room 维护 status/submit 工具的注解、必填项、items 数量与字段、slot/mode 枚举及等待秒数范围 [证据 `L957–L997`]
- `room_maintenance_submit_rejects_unknown_nested_fields` — T3 [KEEP]；维护提交 item 中出现未知嵌套字段时反序列化失败并报告 unknown field [证据 `L999–L1011`]
- `stdio_resumes_stale_logical_session_before_first_tool_call` — T4；通过原始 JSON-RPC stdio 在初始化通知后调用 agent.info 成功且不泄漏内部初始化响应，后续 tools/list 返回工具数组 [证据 `L1234–L1296`]
- `tmux_actions_reject_incompatible_fields` — T3 [KEEP]；tmux.sessions 的 list/create/close 与 tmux.panes 的 list/capture 请求携带不兼容字段时均被拒绝 [证据 `L2311–L2328`]
- `wp2_hub_normal_direct_skills_require_capability` — T4；Normal Hub 上 skills.list/read 均返回 `room_agent_required`，且不返回 skills 或 skill 内容 [证据 `L2602–L2617`]

### `crates/agentic-gpt/src/mcp/mcp_tests.rs`
- `managed_mcp_cancel_while_waiting_for_hub_confirmation_cleans_pending_sender` — T4；检查确认预览的脱敏，在批准前取消，并验证待处理确认已清理、fake 调用次数为零，且后续读取时结果不可用 [证据 `:452–522`]
- `managed_mcp_cancel_without_terminal_evidence_becomes_detached` — T4；使用忽略取消的 fake，验证取消通知及 detached 状态，并具有 transport/remote-error 证据 [证据 `:709–736`]
- `managed_mcp_deferred_result_is_retained_for_process_get` — T4；延迟的 fake 结果起初使进程保持活动状态；进程详情等待完成，进程读取返回保留的结构化结果 [证据 `:403–449`]
- `managed_mcp_shares_capacity_and_rejects_oversized_arguments` — T4；使一个请求保持活动，检查第二个请求超出进程容量，拒绝过大的参数，并确认仅注册了活动进程 [证据 `:739–789`]
- `managed_mcp_timeout_sends_exact_cancel_notification` — T4；等待中的 fake 请求超时；验证 TimedOut 状态、取消证据、context 取消，以及通知 ID 等于请求 ID [证据 `:642–665`]
- `managed_mcp_tool_error_and_large_result_are_truthful` — T4；检查工具错误映射为失败状态；中等大小的结果先延后提供，随后可读取；超限结果不予保留且持续不可用，同时报告字节数/hash [证据 `:525–639`]
- `managed_mcp_user_cancel_observes_remote_cancellation` — T4；取消等待中的请求，并检查 detached 状态、通知结果、终止证据，以及请求/取消 ID 匹配 [证据 `:668–706`]
- `mcp_batch_child_cancel_during_aggregate_confirmation_cancels_all_before_start` — T4；取消一个等待中的子请求会拒绝/取消聚合请求；检查各自不同的终止证据、确认清理，以及无下游调用 [证据 `:1679–1772`]
- `mcp_batch_multi_server_uses_one_non_scoped_confirmation_and_rejects_all` — T4；双服务器批次请求一次无作用域限制的确认；拒绝后所有子请求均被拒绝，两个 fake 均未被调用，且不授予临时访问权限 [证据 `:1593–1676`]
- `mcp_batch_parallel_enforces_per_server_and_global_concurrency` — T4；通过延迟的 fake 调用验证每个服务器的并发上限及跨服务器的全局上限，然后检查并发计数器归零 [证据 `:1129–1189`]
- `mcp_batch_response_budget_is_captured_before_waiting` — T4；在某一响应上限下启动延迟的 fake 调用，执行开始后降低配置的上限，并验证该批次使用其捕获的预算进行 projection [证据 `:1455–1514`]
- `mcp_batch_single_server_uses_one_confirmation_and_can_grant_temporary_allow` — T4；同一服务器的两次调用生成一个脱敏的批量确认；允许临时授权后运行两次调用，且不留下待处理确认 [证据 `:1518–1590`]
- `server_config_revision_is_deterministic_and_content_sensitive` — T3 [KEEP]；对等价的服务器 map 重新排序不改变 revision；更改某个服务器的 enabled 位会改变 revision [证据 `:1881–1903`]
- `server_config_validation_is_complete_and_typed` — T3 [KEEP]；接受有效的 HTTP/stdio 配置，并检查无效 ID、缺失/无效 URL 或命令，以及不支持的 transport 所对应的类型化拒绝码 [证据 `:1775–1834`]
- `streamable_http_bearer_auth_is_validated_and_injected` — T3 [KEEP]；接受 HTTP bearer auth，并将其 token 映射到 transport auth header；检查 Debug 输出对其脱敏，且 stdio auth 被拒绝 [证据 `:1906–1931`]

### `crates/agentic-gpt/src/operations/confirmation.rs`
- `batch_confirmation_shows_the_original_script_and_working_directory` — T2 [KEEP]；批量确认预览原样包含命令脚本及其工作目录 [证据 `L956–L975`]

### `crates/agentic-gpt/src/operations/event_notifications.rs`
- `active_process_response_remains_eligible_for_async_completion` — T2 [KEEP]；running 进程的初始响应被识别为一个非终态 disposition，保留进程引用以供异步完成 [证据 `L392–L408`]
- `deduplicated_install_response_does_not_settle_original_source` — T2 [KEEP]；去重的已完成安装响应不产生 disposition，因而不结算原始来源 [证据 `L433–L446`]
- `mixed_batch_classifies_each_process_from_its_own_terminal_state` — T2 [KEEP]；批次中 completed/failed 项各自标记为含终态，running 项标记为非终态，并各自保留进程引用 [证据 `L410–L431`]
- `terminal_process_observation_suppresses_completion_with_incomplete_capture` — T2 [KEEP]；即使终态进程的捕获不完整且仍有更多输出，初始响应仍包含该进程的终态 disposition [证据 `L373–L390`]

### `crates/agentic-gpt/src/operations/operation.rs`
- `cli_admission_is_limited_to_local_admin_tmux_operations` — T3 [KEEP]；CLI ingress 允许本地 admin tmux 操作，并拒绝 process.exec、user.notify.deliver 和未知操作 [证据 `L439–L463`]
- `normal_hub_cannot_acquire_room_capability_by_operation_name` — T3 [KEEP]；即使启用 Room 工具集，Normal Hub 也不能调用 room 日记、笔记本、状态和维护操作，错误码为 `room_agent_required` [证据 `L481–L498`]
- `process_read_keeps_process_authorization_across_agent_profiles_and_hub` — T3 [KEEP]；Normal/Room 本地及 Hub runtime 均获准 process.read；禁用 Process 工具集后本地拒绝而 Hub 仍获准 [证据 `L500–L532`]
- `same_prefix_unknown_operations_fail_closed` — T3 [KEEP]；LocalUnix 上未知 skills 操作及 Room Hub 上未知 room 操作均授权失败 [证据 `L465–L479`]

### `crates/agentic-gpt/src/operations/policy.rs`
- `dynamic_words_redirects_and_complex_scripts_never_use_partial_allows` — T3 [KEEP]；即使简单命令有 allow 规则，动态展开、重定向及复杂控制流脚本仍要求确认 [证据 `L438–L462`]
- `escaped_crlf_cannot_hide_a_default_denied_command` — T3 [KEEP]；反斜杠加 CRLF 后跟随的默认拒绝 ssh 命令不会被隐藏，整体策略判为拒绝 [证据 `L502–L513`]
- `escaped_word_separator_requires_confirmation_instead_of_partial_matching` — T3 [KEEP]；转义空格产生的参数不匹配完整 allow/deny 规则时，策略要求确认而非部分匹配 [证据 `L514–L532`]
- `known_deny_wins_inside_unsupported_or_incomplete_scripts` — T3 [KEEP]；不完整或不支持的脚本中出现匹配 deny 规则的命令时，整体仍判为拒绝 [证据 `L464–L483`]
- `shell_confirmation_cannot_be_removed_by_an_allow_rule` — T3 [KEEP]；即使命令符合 allow 规则，要求确认的调用仍返回 Confirm [证据 `L423–L436`]
- `shell_requires_allow_for_every_literal_command` — T3 [KEEP]；脚本中每个字面命令均有 allow 规则时放行，混入未允许命令则要求确认 [证据 `L379–L401`]
- `shell_supports_literal_quotes_concatenation_and_supported_operators` — T3 [KEEP]；支持引号、字面拼接及 &&、||、管道和分号组合，所有命令获准时放行，匹配 cat 确认规则后要求确认 [证据 `L403–L421`]
- `terminal_dollar_in_a_quoted_argument_is_not_truncated` — T3 [KEEP]；带引号参数末尾的 `$` 保留在 deny 匹配中，命中 `blocked$` 规则并拒绝 [证据 `L485–L500`]

### `crates/agentic-gpt/src/operations/shell_parser.rs`
- `dynamic_and_expanding_words_are_not_literal` — T3 [KEEP]；含变量展开、通配符、命令替换或重定向的脚本不会被判为完整字面命令 [证据 `L430–L435`]
- `escaped_crlf_does_not_hide_the_following_command` — T3 [KEEP]；反斜杠 CRLF 后的 ssh 命令仍被提取，脚本同时标记为不完整 [证据 `L445–L453`]
- `escaped_word_separator_does_not_preserve_truncated_arguments` — T3 [KEEP]；转义分隔符使脚本不完整，且 touch 命令不会保留被截断的参数 [证据 `L455–L468`]
- `extracts_quoted_words_and_literal_concatenation` — T3 [KEEP]；解析单/双引号、转义引号、转义美元符及相邻字面片段，得到完整命令及预期参数 [证据 `L413–L428`]
- `quoted_terminal_dollars_and_newlines_are_kept` — T3 [KEEP]；引号内参数末尾美元符和换行均原样保留在提取结果中 [证据 `L437–L443`]

### `crates/agentic-gpt/src/process/exec.rs`
- `shell_argument_limit_counts_exact_bootstrap_and_keeps_boundary` — T3；比较精确的 bootstrap 长度，拒绝引号密集的超限命令，并接受恰好达到边界的命令 [证据 `L880–L932 (`#[cfg(target_os = "linux")]`)`]

### `crates/agentic-gpt/src/process/managed.rs`
- `degraded_history_fails_closed_before_process_effect` — T4；禁用的历史存储导致 printf 准入失败及健康状态降级，但不影响进程 [证据 `L6739–L6760`]
- `failed_process_reasons_are_projected_without_fabricating_exit_errors` — T2；将 spawn 失败映射为响应错误，但不因非零退出而捏造错误 [证据 `L4796–L4844`]
- `output_overflow_reports_exact_retained_gap` — T3；写入超过内存 ring 容量的数据，并检查缺失/保留的偏移量及返回的字节 [证据 `L5435–L5475`]
- `pipefail_nonzero_exit_fails_with_shell_status` — T5；运行 `printf before | false`；检查失败状态/退出码 1，且无拒绝/详情错误 [证据 `L5094–L5109`]
- `process_error_messages_are_utf8_safe_and_bounded` — T2；验证多字节截断不超过字节上限，且符合 UTF-8 边界 [证据 `L4787–L4793`]
- `process_read_mcp_result_exposes_retained_and_not_retained_values` — T4；注册/完成内部 MCP 结果，并检查 included/too-large/not-retained 响应状态及 cursor 限制 [证据 `L5498–L5578`]
- `process_read_rejects_output_pages_that_cannot_advance` — T3；合成响应适配器拒绝无法在最小预算内容纳或推进的页面 [证据 `L4848–L4896`]
- `process_read_wait_survives_process_lock_contention` — T4；持有进程 mutex，使 auto/status MCP 读取等待；完成 MCP 结果，并检查两者均被唤醒且获得正确视图 [证据 `L6002–L6046`]
- `shell_eval_preserves_dash_leading_command_text` — T5；运行 `--`，并检查失败状态/退出码 127 [LOW-VALUE: 仅检查 `--` 的 Failed/127；未断言捕获的输出，因而不能证明开头的命令文本得到保留。] [证据 `L5080–L5092`]
- `skill_leases_still_block_updates` — T3；shared lease 在释放前阻止 exclusive 获取 [证据 `L6762–L6774`]

### `crates/agentic-gpt/src/room/bootstrap.rs`
- `entrypoint_and_guide_symlinks_do_not_get_read_as_content` — T5；entrypoint symlink 以 fail-closed 方式失败；guide symlink 被忽略并产生警告，而不计为 guide [证据 `:1215–1241`]
- `guide_directory_entry_errors_are_reported_without_dropping_readable_entries` — T2 [KEEP]；给定一个可读条目和一个注入的目录条目错误，保留可读路径并发出一条不可读条目警告 [证据 `:1004–1017`]
- `resource_truncation_is_line_aware_and_utf8_safe` — T3 [KEEP]；通过直接字节输入验证整行截断、UTF-8 安全的部分行截断、行元数据、返回字节数及警告 [证据 `:1085–1103`]

### `crates/agentic-gpt/src/room/room_maintenance.rs`
- `local_auto_push_controls_origin_sync` — T5；使用真实的本地 bare origin，验证未启用 auto-push 时，本地 submit 不改变远端；随后启用 auto-push，远端 HEAD 和内容得到更新 [证据 `:1221–1258`]
- `local_auto_push_reports_failed_sync_after_origin_disappears` — T5；在 submit 前移除本地裸 origin；本地 apply/commit 成功，而 sync 报告失败，本地内容和 HEAD 仍保持更新后的状态 [证据 `:1288–1318`]
- `local_auto_push_without_origin_reports_unavailable_after_apply` — T5；没有 origin 时，auto-push 仍在本地应用并 commit，报告同步不可用，并写入预期文件 [证据 `:1261–1285`]
- `local_submit_rejects_occupied_clean_slot` — T5；在干净的 Git repo 中预置并 commit 一个已占用的请求槽位；submit 被拒绝，且不改变 HEAD、请求内容、目标路径或干净状态 [证据 `:1188–1218`]
- `workflow_submit_requires_origin_and_workflow_transport` — T5；配置 workflow 模式且没有 origin 时，submit 返回 `room_maintenance_origin_required`。尽管函数名有所暗示，测试主体并未触发或断言缺少 workflow-transport 的错误 [证据 `:1570–1593`]
- `workflow_wait_applies_through_repository_worker` — T5；worker 克隆获取请求，运行仓库 executor，并 commit/push 结果；submit 观察到 Applied/Succeeded 状态，以及请求在本地和远程均被移除 [证据 `:1366–1426`]

### `crates/agentic-gpt/src/room/room_reads.rs`
- `diary_periods_use_strict_direct_layer_paths` — T2 [KEEP]；将有效的每日/每周周期映射到直接层路径，并拒绝无效日期、起止颠倒的月度范围以及类似路径的周期输入 [证据 `:562–580`]
- `semantic_paths_reject_arbitrary_repository_files` — T3 [KEEP]；接受 notebook 路径和含点号的 state 文件名主体；拒绝任意 state 路径和路径遍历 [证据 `:583–589`]

### `crates/agentic-gpt/src/room/room_repository.rs`
- `bootstrap_is_idempotent_and_does_not_rewrite_initial_commit` — T5；运行仓库初始化两次，并检查 HEAD 未变且提交总数为一次 [证据 `:1183–1197`]
- `existing_git_root_is_inspected_without_overwrite_or_scaffold` — T5；创建一个带有 keep 文件的 unborn Git 根目录；初始化保留该文件，且不写入 `room.json` [证据 `:1200–1212`]
- `logical_day_respects_the_configured_shanghai_boundary` — T2 [KEEP]；紧邻所配置上海时间分界点之前及恰好位于该分界点的时间戳，映射到预期的相邻逻辑日期 [证据 `:1072–1089`]
- `non_empty_non_git_root_is_unborn_and_unstaged` — T5；在现有非 Git 目录中初始化 Git，同时让现有文件保持未跟踪状态，且不创建 `room.json` [证据 `:1215–1230`]
- `status_rejects_symlinked_executor_and_workflow_ancestors` **[Unix only]** — T5；将 executor 和 workflow 的祖先目录替换为指向外部文件的符号链接；检查报告两项能力均为 Unavailable [证据 `:1335–1362`]
- `status_rejects_symlinked_git_metadata` **[Unix only]** — T5；将 `.git` 设为符号链接，并验证仓库检查以 Git-symlink 错误拒绝它 [证据 `:1278–1291`]

### `crates/agentic-gpt/src/runtime/agent_info.rs`
- `info_is_bounded_redacted_and_profile_correct` — T4；收集 Room 运行时信息，并检查 profile、workspace/config/MCP/search 值及 secret 隐去情况 [证据 `L443–L457`]
- `info_preserves_empty_policy_lists_and_deduplicates_workspace_root` — T4；检查规范化后的已配置根目录、写入根目录去重，以及空的 read-only/deny 数组 [证据 `L460–L488`]

### `crates/agentic-gpt/src/runtime/instance_lock.rs`
- `rejects_a_second_lock_and_releases_on_drop` — T5；确认持有第一个锁时第二次加锁失败，且释放后可成功获取锁 [证据 `L91–L106`]

### `crates/agentic-gpt/src/runtime/main_tests.rs`
- `auto_provision_is_gated_and_failures_fall_back_to_desktop` — T4；通过注入的源回调验证预配门控、桌面回退及预配失败后的回退 [证据 `L192–L251`]
- `builder_accepts_pre_resolved_browser_context_without_discovery` — T4；将预先解析的浏览器上下文传给应用状态构建器，并检查保留的描述符 [证据 `L280–L306`]
- `configured_allow_overrides_builtin_confirm` — T3；检查配置的 allow 覆盖针对 `curl --version` 的内置 confirm [证据 `L907–L922`]
- `configured_allow_overrides_builtin_deny` — T3；检查配置的 allow 覆盖针对 `ssh -V` 的内置 deny [证据 `L926–L941`]
- `configured_allow_overrides_need_confirm` — T3；检查配置的 allow 覆盖确认要求 [证据 `L888–L903`]
- `configured_deny_wins_when_multiple_config_rules_match` — T3；检查匹配的配置 deny 优先于范围更广的配置 allow [证据 `L945–L963`]
- `configured_room_repository_root_overrides_default` — T2；检查显式指定的仓库根目录覆盖默认值 [证据 `L412–L417`]
- `explicit_runtime_precedes_desktop_discovery_and_invalid_does_not_fallback` — T3；解析有效的显式浏览器配置，并检查相对 node 路径会使解析返回 `None` [证据 `L43–L52`]
- `hub_adapter_and_local_dispatcher_share_toolset_errors` — T4；比较同一 Room 命令的直接本地分派错误与 hub-adapter 错误 [证据 `L535–L558`]
- `invalid_explicit_runtime_keeps_app_startup_fail_open` — T4；使用无效的显式浏览器配置构建应用状态，并检查未保留浏览器上下文 [证据 `L55–L71`]
- `local_cli_accepts_config_before_or_after_subcommand` — T2；解析 `--config` 的两种放置位置，并检查本地 list-tools 命令及路径相同 [证据 `L354–L379`]
- `normal_hub_rejects_current_room_commands_without_room_profile` — T4；在 Normal profile 下分派 Room diary 命令，并检查 `room_agent_required` [证据 `L706–L722`]
- `notification_delivery_rejects_unsupported_channel` — T3；检查 `hub::ntfy` 投递因不受支持而被拒绝 [证据 `L1759–L1772`]
- `public_run_has_only_a_config_path_and_no_profile_override` — T2；解析 `run`，并拒绝 profile 覆盖 [证据 `L347–L351`]
- `remove_rule_matches_command_and_args_prefix` — T3；仅删除 Python `-c` 规则，并保留另一条规则 [证据 `L1305–L1319`]
- `remove_rule_matches_command_without_uuid` — T3；按程序删除规则，并检查列表为空 [证据 `L1294–L1301`]
- `remove_rule_refuses_ambiguous_non_interactive_match` — T3；拒绝重复匹配，且不修改规则 [证据 `L1323–L1337`]
- `room_policy_keeps_high_risk_commands_restricted` — T3；检查高风险程序需要确认，且 Room 下的 `ssh` 被拒绝 [证据 `L761–L779`]
- `room_policy_overlay_differs_from_normal_policy` — T3；检查 `rm` 在 Normal 下需要确认，在 Room 下被允许 [证据 `L748–L758`]
- `rule_matches_program_and_args_prefix_structurally` — T2；确认程序及前缀精确匹配，并拒绝其他程序及前缀 [证据 `L782–L790`]
- `run_as_room_uses_workspace_default_repository_root` — T2；检查默认 agent id 和 `<workspace>/room` 仓库路径 [证据 `L337–L344`]
- `safe_summary_includes_path_roots_and_policy_rules` — T3；检查 safe-summary 的根目录来源及数量，以及配置规则和内置规则的内容 [证据 `L793–L885`]
- `sse_post_status_classification_stops_on_stale_connection` — T2；将 OK/conflict/bad-gateway 状态映射为 delivered/stale/retry [证据 `L321–L333`]
- `standalone_reload_replaces_the_frozen_live_subset` — T3；直接应用候选配置，并检查实时子集更新，而冻结字段和已克隆的处理中 MCP 端点保持不变 [证据 `L1366–L1432`]
- `sudo_requires_credentials` — T3；检查 `sudo true` 预检返回 interactive-credential-required [证据 `L968–L978`]
- `unavailable_sources_fail_open_without_host_io` — T4；注入的源故障导致无浏览器上下文，且 provider 调用次数符合预期 [证据 `L255–L277`]

### `crates/agentic-gpt/src/runtime/notify.rs`
- `freedesktop_probe_missing_service_returns_unavailable` — T5；无通知服务的隔离 D-Bus 守护进程返回 `(false, false)` [证据 `L198–L201`]
- `freedesktop_probe_reports_notification_actions` — T5；运行中的 D-Bus 服务在有/无 `actions` 时产生相应的支持状态元组 [证据 `L204–L211`]
- `freedesktop_probe_unresponsive_service_has_short_deadline` — T5；停滞的服务在探测/外层截止时间内被判定为不可用 [证据 `L214–L224`]

### `crates/agentic-gpt/src/runtime/state.rs`
- `runtime_capabilities_follow_transport_and_profile` — T3；检查 Hub、Tunnel、Local、Normal 和 Room 模型间的能力/角色/模式映射 [证据 `L216–L239`]

### `crates/agentic-gpt/src/runtime/supervisor.rs`
- `doctor_diagnostic_output_is_bounded_and_redacted` — T2；检查机密信息替换和诊断截断标记 [证据 `L1009–L1017`]
- `forwarded_child_lines_preserve_known_severity_and_strip_timestamp` — T2；检查严重级别及时间戳解析、未知行回退处理和敏感信息遮蔽 [证据 `L1021–L1044`]
- `forwarded_journal_lines_preserve_untimestamped_severity_after_redaction` — T2；检查 INFO/WARN/ERROR 映射和机密信息替换 [证据 `L1048–L1058`]
- `mcp_binding_preserves_worker_tokenization` — T2；检查 MCP 命令原样嵌入带引号的 worker 调用 [证据 `L986–L1005`]
- `restart_decision_covers_retry_permanent_and_exhausted` — T3；检查重试延迟、尝试次数耗尽及永久退出的结果 [证据 `L1104–L1116`]
- `restart_identity_warning_compares_to_immutable_runtime_and_warns_once` — T3；观察变更、重复及恢复的版本，并检查仅警告一次的状态 [证据 `L1062–L1092`]
- `retry_schedule_is_bounded_and_exponential` — T3；检查第 1、2 次尝试时的退避，以及第 5/6 次尝试时的退避上限 [证据 `L1096–L1100`]
- `runtime_paths_reject_path_injection` — T3；检查 `../escape` 运行时身份被拒绝 [证据 `L1157–L1161`]
- `stale_health_and_pid_files_are_removed_before_start` — T5；写入过期文件，调用清理，并检查两个文件均被删除 [证据 `L1165–L1177`]
- `worker_command_quotes_paths_and_never_contains_api_key` — T2；检查 worker 命令中的路径带引号，且不含 API-key 文本 [证据 `L973–L982`]

### `crates/agentic-gpt/src/runtime/tunnel_distribution.rs`
- `archive_hash_mismatch_is_checked_before_install` — T2；检查错误的预期归档摘要产生 hash-mismatch 错误 [证据 `L903–L909`]
- `archive_rejects_traversal_symlinks_duplicates_and_extra_files` — T3；验证内存中的归档测试夹具，并拒绝路径遍历、符号链接、重复候选项及非预期布局 [证据 `L796–L825`]
- `artifact_lock_serializes_concurrent_installers` — T5；第一把锁被释放后，第二次获取最终成功 [LOW-VALUE:从未断言第二个等待者在第一把锁被持有期间持续阻塞；不保证串行化的实现也可能通过。] [证据 `L914–L921`]
- `artifact_selection_requires_pinned_or_explicit_trust` — T3；检查默认固定来源、不受支持的版本被拒绝、HTTP 被拒绝，以及受信任的 HTTPS 自定义来源 [证据 `L733–L765`]
- `download_url_requires_https_outside_tests` — T3；在不允许 HTTP 时拒绝 HTTP，接受测试允许的 HTTP，并拒绝 URL 凭据 [证据 `L926–L930`]

### `crates/agentic-gpt/src/skills/skill_installs.rs`
- `github_sources_resolve_structured_and_convenience_forms` — T2 [KEEP]；将结构化输入或仓库 URL 输入映射为 owner/repo、branch 和 subpath；拒绝 query/userinfo 形式。不发起网络请求 [证据 `L2057-L2076`]
- `idempotency_conflict_is_rejected_before_creating_a_second_job` — T3 [KEEP]；文件内容更改后复用同一键，预期得到 `idempotency_conflict` [证据 `L1997-L2016`]
- `idempotency_retries_return_the_original_install` — T3 [KEEP]；使用相同幂等键重复安装会返回原始 ID，并将重试标记为已去重 [证据 `L1985-L1994`]
- `package_paths_reject_case_conflicts_and_file_directory_collisions` — T3 [KEEP]；注册一个包路径，然后拒绝大小写折叠后的重复项及文件/目录前缀冲突 [证据 `L2079-L2089`]
- `queued_cancel_is_idempotent_and_status_is_terminal` — T3 [KEEP]；取消排队中的安装，检查重复取消的结果为已取消、已处于终态或为时已晚 [证据 `L2019-L2054`]
- `remote_policy_rejects_private_and_reserved_addresses` — T3 [KEEP]；拒绝环回、私有和链路本地 IPv4/IPv6 地址，并允许公网 IP 字面量 [证据 `L2092-L2099`]
- `transient_download_errors_are_the_only_retryable_materialization_failures` — T2 [KEEP]；将暂时性下载错误映射为可重试，将永久性错误映射为不可重试，并将 503 错误映射为下载错误代码 [证据 `L2102-L2109`]

### `crates/agentic-gpt/src/storage/event_store.rs`
- `low_ttl_out_of_range_returns_an_error_without_panicking` — T3 [KEEP]；检查时长转换、到期时间计算和内部策略验证均拒绝过大的 TTL [证据 `L1784-L1804`]

### `crates/agentic-gpt/src/storage/private_state.rs`
- `legacy_wide_agent_id_uses_stable_safe_state_key` — T3 [KEEP]；针对较长的 agent ID 执行两次准备，检查每个 agent 的路径保持稳定且键格式安全 [证据 `L641-L663`]
- `private_agent_root_is_mode_0700` — T5；在 Unix 上，检查所创建文件系统目录的实际权限模式为 `0700` [证据 `L666-L682`]

### `crates/agentic-gpt/src/storage/process_history.rs`
- `opens_fresh_process_database_without_touching_legacy_jobs_database` — T5；打开时创建 SQLite 进程 DB；检查其路径和存在性，并确认旧版 DB 文件保持不变 [证据 `L1210-L1223`]

### `crates/agentic-gpt/src/support/utils.rs`
- `compact_id_has_a_stable_twelve_hex_digit_body` — T2；检查已知前缀和可重复生成的 12 位十六进制字符紧凑 ID [证据 `L228–234`]
- `journal_rendering_omits_inner_timestamp_only_in_journal_mode` — T2；Journal 格式省略内部时间戳，而前台输出包含该时间戳 [证据 `L180–187`]
- `mcp_argument_key_summary_is_counted_and_bounded` — T3；检查键数量、键长度和输出大小的限制、截断及值省略 [证据 `L207–226`]
- `mcp_confirmation_preview_is_sorted_bounded_metadata_without_values` — T2；预览包含排序后的键、字节数和哈希元数据，且不包含参数值 [证据 `L190–205`]

### `crates/agentic-gpt/src/tmux/tmux.rs`
- `cd_requires_one_explicit_path` — T2；检查路径缺失、`-` 和单个显式路径的结果 [证据 `L886–897`]
- `parses_session_and_pane_metadata` — T2；解析 fixture 中的 session/pane 行，并检查选定的映射字段 [证据 `L866–875`]
- `session_scoped_pane_listing_does_not_request_all_panes` — T2；检查限定范围的 pane 列表参数与所有 pane 的参数 [证据 `L899–907`]
- `shell_detection_and_argument_quoting_are_structural` — T2；对 shell 名称进行分类，并对空格和单引号进行引用处理 [证据 `L877–884`]

### `crates/agentic-gpt/src/ui/cli_i18n.rs`
- `every_catalog_entry_is_non_empty_for_each_language` — T1；选定的 UI 字符串及所有命令/参数目录条目在两种语言中均非空 [证据 `L1065–1102`]
- `explicit_language_overrides_locale_environment` — T2；显式选择中文会覆盖所提供的英文 locale [证据 `L1022–1029`]
- `locale_precedence_is_lc_all_then_lc_messages_then_lang` — T2；locale 变量冲突时，按 `LC_ALL` 优先级解析 [证据 `L1031–1039`]
- `localized_command_tree_has_complete_visible_metadata` — T2；递归检查可见的本地化命令和参数目录/帮助元数据 [证据 `L1148–1154` (recursive assertions at `1104–1146`)`]
- `localized_help_errors_keep_stream_and_exit_semantics` — T4；针对无效模式和缺少子命令的情况执行 Clap 解析及本地化错误渲染；检查流、代码和文本 [证据 `L1156–1177`]
- `prescan_accepts_equals_and_split_forms_anywhere` — T2；早期 argv 扫描接受子命令之后以等号形式和分开形式指定的语言标志 [证据 `L1041–1063`]

### `crates/agentic-gpt/src/ui/config_tui/app.rs`
- `limits_form_edits_validates_saves_and_reviews_process_response_budget` — T4；执行限制值编辑，检查无效值/最小值/最大值的处理、保存和审阅的值以及构建的配置输出 [证据 `L2814–2922`]
- `review_connection_applies_http_mcp_fields_and_rolls_back_invalid_edits` — T4；审阅编辑器应用有效的 HTTP MCP 字段，并在 host/port/allow-host 编辑未通过验证时保留原值 [证据 `L2758–2813`]
- `review_final_confirmation_commits_once` — T4；审阅导航和确认恰好调用一次注入的 committer [证据 `L2725–2756`]

### `crates/agentic-gpt/src/ui/config_tui/pages.rs`
- `shell_path_field_is_focusable_only_for_explicit_path_mode` — T3；默认不出现 Shell 路径焦点，当模式为 `path` 时出现 [证据 `L5268–5292`]

### `crates/agentic-gpt/src/ui/tui/forms/state.rs`
- `empty_list_can_add_first_item_and_delete_back_to_empty` — T3；添加、编辑、删除第一项，并检查是否恢复为空状态 [证据 `L181–193`]
- `list_state_adds_edits_deletes_and_keeps_focus_valid` — T3；检查焦点规范化、插入/编辑、删除及最终焦点 [证据 `L160–178`]

### `crates/agentic-gpt/src/ui/tui/layout.rs`
- `master_detail_collapses_to_active_pane` — T3；在所测试的窄尺寸下，master/detail 模式依次各自占据整个区域 [证据 `L117–126`]
- `surface_cursor_honors_reserved_bottom_rows` — T3；Cursor 返回边界内的行矩形，并在剩余行耗尽时停止 [证据 `L129–136`]

### `crates/agentic-gpt/src/ui/tui/process.rs`
- `clipping_and_short_id_are_bounded` — T2；检查截断和短 ID 格式化 [证据 `L321–327`]

### `crates/agentic-gpt/src/ui/tui/runtime.rs`
- `restoration_seam_records_cleanup_in_reverse_setup_order` — T2；检查清理接入点依次调用光标恢复、退出备用屏幕和禁用原始模式 [证据 `L146–153`]

### `crates/agentic-gpt/src/ui/tui/shell.rs`
- `overlay_stays_inside_small_terminal` — T3；检查小型终端内的覆盖层几何范围保持内缩 [证据 `L51–58`]
- `shell_uses_config_tui_margin_and_fixed_chrome_rows` — T3；检查 shell 矩形边距以及固定的 chrome/body 行高 [证据 `L40–48`]

### `crates/agentic-gpt/src/ui/tui/workspace.rs`
- `command_filter_is_case_insensitive_and_keeps_real_routes_only` — T2；大写 `PRO` 仅找到 `process` 命令，并返回其 Process 路由 [证据 `L280–288`]

### `crates/agentic-gpt/tests/config_cli.rs`
- `config_events_set_show_and_restore_default_override` — T5；检查稀疏默认值、事件值、大 TTL，以及跨文件写入和读取时通过 null 移除默认覆盖值 [证据 `L430–527`]
- `config_import_compatible_round_trip_preserves_http_mcp_fields` — T5；创建并复制 HTTP MCP 配置，观察非 TTY 导入被拒绝且未留下不完整的目标文件，并通过 `show` 加载复制后的源配置以检查字段保留情况 [证据 `L1144–1219`]
- `config_init_set_and_show_round_trip` — T5；初始化配置，设置时区和进程响应限制，并检查持久化值及显示值 [证据 `L258–311`]
- `config_set_enforces_process_response_bounds_without_invalid_writes` — T5；持久化可接受的最小值和最大值，并拒绝越界值及非数值，且不更改文件 [证据 `L645–698`]
- `config_set_http_mcp_updates_values_and_rejects_invalid_values_without_writing` — T5；更新并清除 HTTP MCP 字段，然后拒绝无效值，同时保持配置字节不变 [证据 `L1006–1103`]
- `config_set_rejects_file_search_context_bound_without_writing` — T5；拒绝值 101 并给出诊断信息，验证配置字节保持不变 [证据 `L615–643`]
- `config_set_rejects_invalid_event_policy_without_writing` — T5；拒绝无效的 set/import 事件策略，并验证配置字节或输出文件不存在 [证据 `L529–613`]
- `config_shell_init_file_keeps_default_disabled_and_explicit_path_distinct` — T5；通过配置文件测试 unset/default、显式路径、null 禁用、恢复、不写入的不受支持 unset 操作及键元数据 [证据 `L313–369`]
- `config_show_redacts_http_mcp_bearer_reference_but_disk_keeps_it` — T5；检查 bearer 引用持久化到磁盘，但在 `show` 输出中被隐去 [证据 `L1104–1143`]
- `explicit_hub_init_writes_supplied_connection_fields` — T5；写入提供的 hub 设置和 secret，并验证进程输出中不含 secret [证据 `L831–882`]
- `explicit_local_init_does_not_emit_tunnel_config` — T5；写入本地模式配置，并验证不存在 tunnel 节 [证据 `L773–795`]
- `non_interactive_default_init_does_not_create_tunnel_secret_material` — T5；在临时主目录下写入默认配置和待处理提示，并验证未创建 tunnel-secret 目录或文件 [证据 `L736–771`]
- `non_interactive_init_writes_http_mcp_flags_as_references_and_json` — T5；写入 HTTP MCP 选项，并检查规范化 URL、端口、token 引用、allow-host JSON 及 token 未泄露 [证据 `L957–1005`]
- `run_without_config_reports_init_first_and_does_not_create_a_file` — T5；尝试在配置缺失时运行，检查要求先初始化的错误，并验证未出现文件 [证据 `L884–903`]

### `crates/agentic-gpt/tests/local_control.rs`
- `local_runtime_cli_exercises_real_unix_mcp_surface` — T5；测试实际运行中的本地 Unix socket 权限、工具列表和调用、范围读取、策略与审计、toolset/MCP 重载、运行锁、关闭及不可用状态 [证据 `LWrapper `11–18`; scenario `20–357`]

### `crates/agentic-gpt/tests/standalone_http_mcp.rs`
- `standalone_http_mcp_allow_hosts_supports_default_and_explicit_full_allow` — T5；向实际运行的 HTTP listener 发送请求，检查拒绝 host、null/通配符允许及沿用最后有效配置的行为 [证据 `L45–125`]
- `standalone_http_mcp_chatgpt_oauth_discovery_authorize_token_and_mcp_use` — T5；测试实际运行中的 HTTP OAuth 发现、授权和 token 流程、MCP 一致性、grant/host 反例、轮换及重载行为 [证据 `LWrapper `21–27`; scenario `499–1099`]
- `standalone_http_mcp_env_auth_handshake_parity_rotation_toolset_and_last_good` — T5；测试实际运行中的身份验证、工具一致性、进程调用与审计、策略/token/toolset 重载，以及保留最后一个正常工作的 listener [证据 `LWrapper `29–35`; scenario `127–356`]
- `standalone_http_mcp_file_rotation_reconfigure_disable_and_enable` — T5；使用真实 token 文件和 listener 检查轮换、端点及会话重新配置、禁用与启用，以及新会话访问 [证据 `LWrapper `37–43`; scenario `359–497`]

### `crates/agentic-gpt/tests/standalone_supervisor.rs`
- `hidden_worker_recovers_stale_tunnel_session_before_first_call` — T5；启动隐藏 worker，在 initialize 之前发送恢复会话的调用，并检查响应、后续列表及恢复日志 [证据 `LWrapper `16–23`; scenario `261–394`]
- `hidden_worker_reloads_policy_path_limit_and_mcp_without_restart` — T5；在 worker 和本地 peer 运行时更改策略、路径、MCP 和进程限制；检查调用、审计、文件效果、取消及日志 [证据 `LWrapper `25–32`; scenario `397–619`]
- `supervised_invalid_config_warning_is_supervisor_owned` — T5；在 supervisor 执行期间破坏正在使用的配置，并检查由 supervisor 发出的一条重载警告 [证据 `LWrapper `61–68`; scenario `70–259`]
- `supervisor_launches_real_room_worker_and_advertises_room_surface` — T5；在 room profile 下运行 supervisor 场景，并检查 Room 工具的宣告 [证据 `LWrapper `43–50`; scenario `70–259`]
- `supervisor_launches_real_worker_and_completes_local_mcp_call` — T5；使用假的 tunnel 可执行文件和本地健康检查端点启动 supervisor/worker；检查真实 worker 的 MCP 进程调用及转发日志 [证据 `LWrapper `34–41`; scenario `70–259`]
