# Agentic GPT 混合层级测试用例

**最终 MIXED 声明数：323。** 所有唯一测试声明均按 repository-relative 源文件分组，且各列一次。Tier 组件指出测试分别断言的行为；MIXED 是单独分类，不会额外计入任何单一 tier。若分片报告提供了源代码行范围，则在条目中保留为证据。`#[ignore]` 和平台条件门控的声明仍按声明各计一次。

## Tier 判定边界与数量核对

只有当测试断言依赖生产代码对真实资源（文件系统、Git、socket、进程或持久化数据库状态/副作用）的操作时，才归为 T5。仅有 fixture 设置/清理或扫描空 store 不足以提升分类。若断言依赖实际 SQL 状态或效果，则内存 SQLite 也计为 T5。

以下涉及空存储的最终分类已修正：`event_api_business_errors_keep_their_code_and_include_the_panel` = T4；`browser_repl_event_decoration_preserves_inner_result_channels` = MIXED(T2+T4)，不含 T5；`in_process_stdio_initialize_list_and_call` = T4；`denied_process_batch_creates_no_processes` = MIXED(T4+T5)，因为拒绝后的 SQLite 列表断言了进程未持久化的不变量；`in_process_room_stdio_initialize_list_and_call` = T4，因为只断言错误标记和构造出的 daily 对象形状，而空/缺失 root 仍会返回该对象。

| 范围 | 声明数 | T1 | T2 | T3 | T4 | T5 | MIXED |
|---|---:|---:|---:|---:|---:|---:|---:|
| Browser/config | 202 | 3 | 28 | 50 | 24 | 3 | 94 |
| Ingress/operations | 105 | 7 | 10 | 34 | 8 | 3 | 43 |
| MCP/room | 68 | 0 | 3 | 5 | 12 | 12 | 36 |
| Storage/skills | 69 | 0 | 2 | 7 | 0 | 2 | 58 |
| UI/support/tmux/integration | 62 | 1 | 14 | 8 | 4 | 24 | 11 |
| Runtime/files/process | 148 | 0 | 16 | 30 | 11 | 10 | 81 |
| **合计** | **654** | **11** | **73** | **134** | **59** | **54** | **323** |

六个分片合计 654 个声明；单一层级分类共 331 个，另有 323 个 MIXED，合计 654。下列 MIXED 清单共有 94 + 43 + 36 + 58 + 11 + 81 = 323 项。

## MIXED 用例清单

### `crates/agentic-gpt/src/browser/browser_distribution_tests.rs`
- `acquisition_temp_cleanup_handles_materializer_success_and_failure` — MIXED (T4+T5); 检查生产代码物化成功或包哈希校验失败后的获取临时文件清理；T4:临时资源守卫/物化器调用链；T5:对包/缓存文件的影响 [证据 L439]
- `active_manifest_corruption_fails_closed` — MIXED (T4+T5); 安装制品，以多种方式破坏磁盘上的活动清单，并检查生产发现逻辑拒绝该清单；T4:安装/发现调用链；T5:清单文件读取 [证据 L1089]
- `ar_parser_rejects_malformed_truncated_duplicate_and_unsupported_members` — MIXED (T3+T5); 将每个归档测试样本写入临时包路径，由生产代码 `ar_data` 读取；接受大小为奇数的成员，拒绝不支持的压缩、重复的控制成员和截断；T3:ar 格式规则；T5:生产代码读取真实临时包文件 [证据 L915]
- `duplicate_data_member_is_rejected` — MIXED (T3+T5); 生产物化器读取含重复数据成员的包文件并拒绝它；T3:归档校验；T5:基于文件的包处理 [证据 L895]
- `hash_mismatch_is_rejected_before_archive_processing` — MIXED (T3+T5); 生产物化器读取包文件，拒绝错误的摘要，且不留下暂存目录树；T3:哈希检查关卡；T5:基于文件的物化断言 [证据 L880]
- `identical_install_is_idempotent_and_corrupt_artifact_repairs` — MIXED (T4+T5); 生产安装逻辑在重复安装时保持清单字节不变，随后修复被删除的缓存文件；T4:安装/修复调用链；T5:缓存读写 [证据 L1063]
- `local_http_status_redirect_and_body_bounds_are_rejected` — MIXED (T3+T5); 通过回环 HTTP 检查状态/重定向及响应体限制；T3:响应规则；T5:真实套接字交互 [证据 L309]
- `provisioning_lock_rechecks_cache_after_waiting` — MIXED (T4+T5); 在另一调用物化缓存制品时等待目标锁，随后检查等待方复用该制品；T4:供给/锁/缓存调用链；T5:生产锁/缓存文件的影响 [证据 L403]
- `real_package_materializes_from_explicit_fixture_path` — MIXED (T4+T5); 读取提供的官方包，将其物化，并检查版本/通道和安装路径。`#[ignore]` 位于 L538；需要 `AGENTIC_BROWSER_REAL_DEB*`；T4:包验证/提取/运行时描述符调用链；T5:实际包/缓存文件 [证据 L539]
- `selected_links_special_types_and_duplicate_files_are_rejected` — MIXED (T3+T5); 生产物化器读取归档，拒绝选中的符号链接/硬链接/FIFO 条目及重复的选中文件；T3:归档条目规则；T5:对包/缓存文件的影响 [证据 L981]
- `streamed_package_checks_size_hash_and_temp_cleanup` — MIXED (T3+T5); 将回环 HTTP 响应体流式写入临时文件；检查字节、哈希/大小失败及清理；T3:大小/哈希规则；T5:套接字及生产文件写入 [证据 L339]
- `symlinked_critical_parent_in_cached_artifact_fails_closed` — MIXED (T4+T5); 将关键缓存目录替换为符号链接，并检查生产发现逻辑在失败时采取拒绝策略。`#[cfg(unix)]` 位于 L1150；T4:制品/发现调用链；T5:符号链接/文件系统影响 [证据 L1152]
- `synthetic_deb_materializes_only_selected_resources` — MIXED (T4+T5); 物化合成归档，检查选中文件/权限/路径，并重新发现缓存运行时；T4:归档到运行时/发现调用链；T5:生产包/缓存文件系统 [证据 L813]
- `target_names_are_frozen` — MIXED (T1+T3); 固定受支持的目标名称字面量，并检查组件/哈希校验；T1:固定名称；T3:校验 [证据 L801]

### `crates/agentic-gpt/src/browser/browser_kernel.rs`
- `bootstrap_sends_escaped_path_and_frozen_browser_setup_code` — MIXED (T1+T2); 检查转义后的路径、超时、固定的设置代码/顺序，以及作为 JS 发送的 browser-ID 输出；T1:固定内容；T2:转义/请求转发 [证据 L702]
- `sequential_calls_reuse_initialized_service_and_metadata` — MIXED (T2+T4); 发送两次 JS 调用；检查仅一次 initialize、协议及元数据一致性；T2:元数据转发；T4:内核/客户端/服务复用 [证据 L602]

### `crates/agentic-gpt/src/browser/browser_manager.rs`
- `list_is_sorted_and_contains_no_kernel_identity` — MIXED (T2+T4); 检查排序后的快照、存在超时且不存在会话/轮次 ID；T2:快照投影；T4:租约状态 [证据 L1229]

### `crates/agentic-gpt/src/browser/browser_manual.rs`
- `read_ranges_and_rejects_bad_paths` — MIXED (T3+T5); 从测试样本读取请求的行，拒绝空路径/点路径/路径穿越/绝对路径；输出不包含根路径；T3:边界/路径规则；T5:生产文件读取 [证据 L341]
- `read_rejects_root_component_and_final_symlinks` — MIXED (T3+T5); 生产路径检查拒绝根目录/中间目录/最终文件为符号链接的情况。`#[cfg(unix)]` 位于 L436；T3:符号链接拒绝规则；T5:真实文件系统元数据 [证据 L438]
- `read_reports_continuation_and_rejects_giant_line` — MIXED (T3+T5); 读取实际测试样本文件，检查输出续接/上限及拒绝超长行；T3:输出规则；T5:生产文件读取 [证据 L402]
- `search_is_literal_nested_and_sorted` — MIXED (T3+T5); 生产搜索逻辑读取嵌套测试样本文件，对字面匹配项排序并返回上下文；T3:搜索规则；T5:生产目录/文件读取 [证据 L374]
- `search_skips_symlink_and_non_utf8` — MIXED (T3+T5); 生产遍历/读取逻辑报告跳过的符号链接和非 UTF-8 文件。`#[cfg(unix)]` 位于 L521；T3:跳过规则；T5:真实文件系统遍历/读取 [证据 L523]

### `crates/agentic-gpt/src/browser/browser_runtime.rs`
- `absent_codex_cli_path_is_supported` — MIXED (T2+T3+T5); 生产代码读取注册表，将缺失的 CLI 映射为 `None`，并省略启动变量；T2:可选映射；T3:缺失字段处理；T5:注册表读取 [证据 L718]
- `absent_node_module_dirs_defaults_to_empty` — MIXED (T2+T5); 生产代码读取注册表，将缺失目录映射为空向量；T2:默认映射；T5:注册表文件读取 [证据 L707]
- `client_path_without_bundle_parent_is_rejected` — MIXED (T3+T5); 读取注册表测试样本，拒绝没有有效 bundle/docs 根目录的客户端路径；T3:路径规则；T5:注册表读取 [证据 L784]
- `derives_docs_root_from_browser_client_path` — MIXED (T2+T5); 生产代码读取注册表，将客户端路径映射到 docs 根目录；T2:路径映射；T5:注册表文件读取 [证据 L693]
- `derives_ordered_deduplicated_trusted_code_paths` — MIXED (T3+T5); 生产代码读取注册表测试样本，推导出顺序稳定且去重的可信路径；T3:去重/排序逻辑；T5:注册表文件读取 [证据 L673]
- `empty_entries_are_rejected` — MIXED (T3+T5); 生产代码读取注册表测试样本并拒绝空条目；T3:空注册表规则；T5:注册表文件读取 [证据 L733]
- `explicit_descriptor_allows_missing_codex_cli_path` — MIXED (T2+T3); 接受 CLI 路径缺失，并省略对应启动变量；T2:可选映射；T3:缺失分支 [证据 L828]
- `malformed_json_is_rejected_with_browser_runtime_context` — MIXED (T3+T5); 生产代码打开格式错误的临时注册表，返回带上下文的 JSON 错误；T3:解析/错误规则；T5:注册表读取 [证据 L775]
- `malformed_latest_entry_does_not_fall_back_to_older_valid_entry` — MIXED (T3+T5); 读取注册表测试样本，拒绝格式错误的最新条目，而非回退；T3:选择/错误规则；T5:注册表文件读取 [证据 L759]
- `missing_or_empty_required_fields_are_rejected` — MIXED (T3+T5); 生产代码读取注册表测试样本，拒绝缺失应用版本/空 Node 路径，并返回字段错误；T3:必填字段规则；T5:注册表文件读取 [证据 L739]
- `selects_greatest_updated_at_and_maps_selected_entry` — MIXED (T3+T2+T5); 生产代码读取注册表文件，选择最新条目并映射字段，且不受输入顺序影响；T3:选择；T2:映射；T5:注册表文件读取 [证据 L609]

### `crates/agentic-gpt/src/config/config.rs`
- `checked_in_v09_config_example_is_strict_and_safe_to_copy` — MIXED (T1+T4+T5); 通过临时配置加载已签入版本库的示例，校验 MCP/Standalone 设置，检查占位符/默认值及不含秘密材料；T1:示例内容；T4:加载/校验；T5:生产配置文件读取 [证据 L2474]
- `config_load_rejects_missing_toolsets` — MIXED (T3+T5); 生产代码加载不含 toolsets 的临时配置并拒绝它；T3:严格字段规则；T5:配置文件读取 [证据 L2697]
- `durable_writer_uses_sparse_projection_and_preserves_unknown_fields` — MIXED (T2+T4+T5); 生产写入器写入稀疏配置；检查未来字段被保留且默认值被省略；T2:稀疏投影；T4:写入器/投影调用链；T5:生产文件写入/读取 [证据 L3381]
- `explicit_browser_runtime_round_trips_through_sparse_write_and_import` — MIXED (T2+T4+T5); 写入显式浏览器配置，生产代码加载/稀疏投影/导入，并检查往返一致性；T2:浏览器 JSON 映射；T4:写入/加载/导入调用链；T5:文件操作 [证据 L3399]
- `explicit_import_clears_invalid_http_public_url_and_keeps_other_fields` — MIXED (T2+T3+T5); 生产导入逻辑对无效公共 URL 发出警告并将其移除，同时保留其他 HTTP MCP 值；T2:导入映射；T3:URL 规则；T5:文件读取 [证据 L3350]
- `explicit_import_clears_plaintext_tunnel_secret_and_reports_it` — MIXED (T2+T3+T5); 生产导入逻辑对明文秘密发出警告并将其清除，保留其他字段，且序列化配置中不包含标记；T2:导入/脱敏映射；T3:秘密策略；T5:文件读取 [证据 L3323]
- `explicit_import_maps_legacy_hub_and_preserves_recognized_and_unknown_fields` — MIXED (T2+T4+T5); 生产导入逻辑读取旧版 JSON，映射 Hub/配置字段并保留未来字段；T2:旧版字段映射；T4:导入/重建调用链；T5:文件读取 [证据 L3131]
- `explicit_import_prefers_top_level_skills_over_legacy_room_skills` — MIXED (T3+T5); 生产代码导入临时配置，保留顶层 skills，并发出 legacy-room 警告；T3:优先级/警告规则；T5:读取导入文件 [证据 L2794]
- `explicit_import_preserves_shell_init_file_tristate` — MIXED (T2+T3+T5); 生产代码导入时将缺失/null/显式 shell 值映射为 Default/Disabled/Path；T2:JSON 映射；T3:三态；T5:读取导入文件 [证据 L3198]
- `explicit_import_preserves_toolsets_that_omit_a_room_profile_namespace` — MIXED (T3+T5); 生产代码导入时保留显式 toolset 列表，而不是从 Room profile 填充；T3:导入/保留规则；T5:文件读取 [证据 L3266]
- `explicit_import_rejects_invalid_toolsets` — MIXED (T3+T5); 生产代码导入时读取文件并拒绝未知 toolset；T3:验证；T5:文件读取 [证据 L3281]
- `explicit_import_reports_unimportable_recognized_fields_and_keeps_other_values` — MIXED (T2+T3+T5); 生产代码导入时对不支持的字段发出警告，同时保留有效的 display/未知字段/默认值；T2:导入映射；T3:字段警告规则；T5:文件读取 [证据 L3296]
- `explicit_import_uses_normal_toolset_preset_when_toolsets_are_omitted` — MIXED (T2+T5); 生产代码导入时读取 Normal profile 配置并选择 Normal 预设；T2:预设映射；T5:文件读取 [证据 L3252]
- `explicit_import_uses_room_toolset_preset_when_toolsets_are_omitted` — MIXED (T2+T5); 生产代码导入时读取 Room profile 配置并选择所有 toolset；T2:预设映射；T5:文件读取 [证据 L3238]
- `explicit_limit_stays_numeric_after_config_load_and_write` — MIXED (T2+T5); 生产代码加载临时配置，序列化数值限制，并保留未来字段；T2:数值/额外字段映射；T5:生产代码读取配置文件 [证据 L2737]
- `loading_a_legacy_file_without_selectors_returns_migration_error` — MIXED (T3+T5); 生产代码读取缺少 mode/profile 的配置并返回迁移指导；T3:迁移规则；T5:文件读取 [证据 L3097]
- `managed_browser_policy_defaults_are_enabled_without_auto_provisioning` — MIXED (T1+T2+T3); 检查固定默认值、空值序列化/解析及稀疏省略；T1:固定默认值；T2:serde 映射；T3:稀疏/默认规则 [证据 L3437]
- `managed_browser_policy_round_trips_non_default_values` — MIXED (T2+T4+T5); 生产代码通过写入器/加载/导入往返处理非默认浏览器策略；T2:serde 映射；T4:写入器/加载/导入链；T5:文件操作 [证据 L3452]
- `max_active_processes_supports_auto_and_explicit_round_trips` — MIXED (T2+T3); 将 `auto`/数值 JSON 与变体双向映射；拒绝负数/大写形式；T2:serde 映射；T3:无效输入规则 [证据 L2530]
- `new_default_config_serializes_auto_limit` — MIXED (T1+T2); 检查固定的默认 JSON 进程限制、搜索上下文及确认通道；T1:固定默认值；T2:序列化 [证据 L2637]
- `old_config_without_process_response_bytes_uses_default` — MIXED (T2+T5); 生产代码加载缺少字段的临时文件并检查默认值；T2:默认值映射；T5:配置文件读取 [证据 L2512]
- `room_repository_and_maintenance_use_the_v2_json_shape` — MIXED (T2+T3); 检查 v2 room JSON 并拒绝旧版 `notebookRoot`；T2:JSON 映射；T3:拒绝旧版 [证据 L2756]
- `sparse_load_preserves_inactive_mode_and_profile_sections` — MIXED (T2+T5); 生产代码加载稀疏文件并保留未激活的 tunnel 和 Room 值；T2:字段映射；T5:配置文件读取 [证据 L3018]
- `sparse_load_reconstructs_workspace_dependent_path_policy_and_unknown_fields` — MIXED (T2+T4+T5); 生产代码加载临时稀疏 JSON，推导路径策略，并保留未知字段；T2:字段/额外字段映射；T4:加载/默认策略链；T5:文件读取 [证据 L2995]
- `sparse_projection_always_keeps_selectors_and_omits_reconstructable_defaults` — MIXED (T2+T3); 保留 mode/profile/toolsets，并省略可重建的默认字段；T2:投影；T3:省略规则 [证据 L2902]
- `sparse_projection_keeps_inactive_sections_and_redacts_config_secrets` — MIXED (T2+T3); 保留未激活的 tunnel/ref，并对 Hub/MCP 机密信息进行脱敏；T2:投影；T3:机密信息/未激活数据规则 [证据 L2956]
- `sparse_projection_preserves_custom_workspace_root_but_reconstructs_its_path_defaults` — MIXED (T2+T4+T5); 投影自定义工作区根目录，省略默认路径策略，并由生产代码加载时重建该策略；T2:投影；T4:投影/加载重建；T5:配置文件读取 [证据 L2934]
- `sparse_projection_preserves_explicit_inactive_hub_tunnel_and_room_data` — MIXED (T2+T4+T5); 投影未激活的数据，然后由生产代码加载文件并检查保留的 Hub/tunnel/Room/未来字段；T2:投影；T4:投影/加载链；T5:文件读取 [证据 L3043]
- `strict_load_rejects_legacy_confirmation_provider_shape` — MIXED (T3+T5); 生产代码读取临时文件，拒绝旧版 provider 结构并提供导入指导；T3:迁移规则；T5:文件读取 [证据 L3080]
- `strict_load_rejects_legacy_room_skills` — MIXED (T3+T5); 生产代码读取临时配置，并在出现旧版 room skills 时拒绝缺少顶层 skills 的配置；T3:严格迁移规则；T5:文件读取 [证据 L2780]
- `strict_load_rejects_legacy_top_level_hub_fields_even_with_selectors` — MIXED (T3+T5); 生产代码读取临时文件并拒绝旧版顶层 Hub 字段；T3:严格 schema 规则；T5:文件读取 [证据 L3112]
- `toolset_profiles_are_closed_and_deterministic` — MIXED (T1+T3); 检查固定的 namespace/profile 列表及启用/禁用/解析器行为；T1:冻结的 namespace/顺序；T3:profile/变更规则 [证据 L2651]
- `tunnel_secret_references_are_strict_and_safe_summary_is_redacted` — MIXED (T3+T2); 检查 env/file ref 策略及摘要脱敏；T3:引用策略；T2:摘要投影 [证据 L2832]

### `crates/agentic-gpt/src/config/config_cli.rs`
- `registry_applies_new_scalar_and_list_keys` — MIXED (T2+T3); 将 registry 字符串映射到 display、backup、path-list 和 limit 字段；T2:映射；T3:类型化解析 [证据 L596]
- `registry_updates_room_repository_and_maintenance_settings` — MIXED (T2+T3); 映射 Room 设置，清除可选路径，并拒绝无效 mode/已废弃的键；T2:映射；T3:验证 [证据 L618]
- `toolset_commands_dispatch_and_reject_unknown_namespaces` — MIXED (T2+T3); 将 list/enable/disable 解析为变体，并拒绝未知 namespace；T2:参数映射；T3:拒绝 [证据 L459]
- `toolset_enable_and_disable_persist_across_config_loads` — MIXED (T4+T5); 调用处理器及生产代码的写入器/加载器，检查持久化的 toolset 状态；T4:处理器/写入/加载链；T5:配置文件操作 [证据 L506]
- `toolset_listing_describes_all_namespaces_and_feedback_is_localized` — MIXED (T1+T2); 检查固定的英文/中文列表、状态、行及消息；T1:固定文本；T2:渲染/本地化 [证据 L547]

### `crates/agentic-gpt/src/config/config_templates.rs`
- `default_template_is_standalone_normal_with_safe_placeholders` — MIXED (T1+T3+T4); 检查默认值/占位符/待执行操作及 Standalone 验证；T1:固定占位符；T3:模板规则；T4:构建器/验证器 [证据 L425]
- `hub_template_uses_supplied_connection_values` — MIXED (T2+T4); 根据提供的值/语言构建 Hub 配置并验证；T2:所提供内容的映射；T4:构建器/验证器 [证据 L452]
- `imported_base_survives_tui_managed_field_overlay` — MIXED (T2+T4); 叠加受管理的输入，同时保留导入的字段/MCP/未知额外字段；T2:字段保留；T4:导入/模板链 [证据 L507]
- `local_template_omits_tunnel_and_validates_locally` — MIXED (T3+T4); 构建不含 tunnel 的 Local 配置并验证；T3:mode 分支；T4:构建器/验证器 [证据 L440]

### `crates/agentic-gpt/src/config/setup/model.rs`
- `imported_base_seeds_reviewable_fields_without_requiring_an_editor_for_every_field` — MIXED (T2+T4); 初始化 identity/limits/MCP 草稿，并在预览中包含未知字段；T2:草稿映射；T4:导入配置/会话/预览链 [证据 L1118]
- `mcp_server_draft_defaults_empty_and_saves_as_configured` — MIXED (T2+T3); 检查空默认值、保存/状态转换、token 往返处理及 debug 脱敏；T2:字段映射；T3:会话/状态/脱敏 [证据 L1183]
- `preview_is_the_redacted_sparse_projection_without_transaction_secret_material` — MIXED (T2+T4); 比较预览与构建的稀疏配置，保留 ref，并排除事务机密信息；T2:投影/脱敏；T4:活动输入/构建/预览链 [证据 L1086]
- `tunnel_secret_reference_seeds_are_parsed_without_exposing_secret_text` — MIXED (T2+T3); 将 file/env ref 映射到草稿字段，并检查 Hub 机密信息的 debug 脱敏；T2:引用映射；T3:机密信息处理 [证据 L1038]

### `crates/agentic-gpt/src/config/setup/outcome.rs`
- `aliased_config_and_secret_paths_are_rejected_before_secret_write` — MIXED (T3+T5); 拒绝 config/secret 路径别名，并检查现有配置未更改/备份中无标记；T3:别名验证；T5:文件系统路径/效果断言 [证据 L905]
- `commit_creates_secret_parent_0700_file_0600_and_config` — MIXED (T4+T5); 提交 secret/config 并检查权限/引用；T4:结果/secret/config 链路；T5:生产代码写入 [证据 L702]
- `commit_replacement_restores_existing_mode_and_bytes_on_config_failure` — MIXED (T4+T5); config 写入失败时恢复原有 secret 字节/模式；T4:提交/回滚；T5:文件操作 [证据 L734]
- `config_failure_removes_new_secret_and_invalid_target_has_no_side_effect` — MIXED (T4+T5); config 写入失败时移除新 secret；目标无效时不创建任何文件；T4:回滚链路；T5:文件操作 [证据 L761]
- `config_load_does_not_recover_setup_while_mutation_lock_is_held` — MIXED (T4+T5); 使生产代码加载等待锁，检查日志状态，随后恢复操作还原 secret/模式并移除证据；T4:锁/加载/恢复；T5:线程/锁/文件系统 [证据 L658]
- `imported_config_review_edit_commits_events_and_keeps_backup` — MIXED (T4+T5); 通过生产代码导入/审查/提交来编辑事件，并检查输出/保留值/备份；T4:导入/会话/审查/提交链路；T5:文件操作 [证据 L809]
- `no_secret_outcome_writes_config_without_secret_material` — MIXED (T4+T5); 构建不含 secret 的 Local 结果，提交 config，检查文件存在；T4:会话/结果/提交；T5:生产代码写入 [证据 L789]
- `outcome_handoff_revalidates_canonical_connection_before_any_write_plan` — MIXED (T3+T4); 在制定写入计划前拒绝无效 Hub URL；无文件系统调用/实质性文件操作；T3:验证；T4:设置/结果交接 [证据 L978]
- `recovery_restores_crash_state_and_retains_conflicting_evidence` — MIXED (T4+T5); 恢复操作还原旧 secret/模式并移除日志/备份；冲突时保留 secret/证据；T4:恢复链路；T5:实际文件操作 [证据 L681]
- `symlink_parent_alias_to_nonexistent_target_is_rejected_before_secret_write` — MIXED (T3+T5); 拒绝父目录为 symlink 的别名路径，并检查未写入目标或含 secret 的备份；T3:规范路径验证；T5:文件系统操作 [证据 L938]

### `crates/agentic-gpt/src/config/setup/review.rs`
- `event_review_preserves_values_and_offers_every_override_choice` — MIXED (T1+T2); 投影事件值并检查固定选项；T1:固定选项；T2:草稿/审查映射 [证据 L189]
- `review_is_redacted_active_mode_only_and_reports_secret_write_intent` — MIXED (T2+T3); 检查活动模式投影、非活动 Hub 排除、写入意图/脱敏、Room 适用性；T2:投影；T3:模式/状态/脱敏 [证据 L16]
- `review_preserves_pending_actions_and_redacted_standalone_reference` — MIXED (T2+T3); 检查延迟/即时写入意图、待处理操作及引用；T2:投影；T3:操作规则 [证据 L224]

### `crates/agentic-gpt/src/config/setup/validation.rs`
- `active_input_ignores_inactive_connection_secrets_and_restores_staged_drafts` — MIXED (T2+T3); 过滤非活动凭据，并在模式切换时恢复；T2:活动输入投影；T3:状态行为 [证据 L219]
- `shell_init_file_modes_preserve_default_disabled_and_explicit_path` — MIXED (T2+T4); 通过 config 构建将 shell 草稿模式映射为 enum 值；T2:映射；T4:保存/构建链路 [证据 L145]

### `crates/agentic-gpt/src/files/file_ops.rs`
- `binary_content_is_rejected_with_or_without_metadata` — MIXED (T2+T5); 在两种元数据模式下读取无效 UTF-8 字节并检查错误；T2:无效内容/错误映射；T5:生产代码文件读取。证据 L2765–L2783 [证据 L2765–L2783]
- `bounded_reader_never_returns_more_than_the_requested_limit` — MIXED (T2+T5); 检查真实文件在限制为四时超限、在限制为五时完整；T2:有界读取结果映射；T5:生产代码文件读取。证据 L2813–L2830 [证据 L2813–L2830]
- `deny_and_readonly_policy_precedence_is_enforced` — MIXED (T3+T5); 读取只读文件，拒绝向其写入，并拒绝读取禁止访问的文件；T3:策略优先级；T5:实际文件读取/路径解析。证据 L2719–L2761 [证据 L2719–L2761]
- `large_files_are_rejected_with_or_without_metadata` — MIXED (T3+T5); 检查超限的真实文件在两种模式下均被拒绝；T3:大小限制判定；T5:生产代码大小/读取检查。证据 L2787–L2809 [证据 L2787–L2809]
- `move_source_removal_failure_compensates_destination` — MIXED (T4+T5); 在移动过程中注入源移除失败，并检查源仍存在/目标不存在；T4:提交/补偿链路；T5:实际文件系统影响。证据 L3179–L3212 [证据 L3179–L3212]
- `ranges_are_bounded_and_utf8_safe` — MIXED (T2+T5); 读取选定的多字节行并检查范围/内容/计数；T2:行范围/UTF-8 映射；T5:生产代码文件读取。证据 L2704–L2715 [证据 L2704–L2715]
- `reads_metadata_without_exposing_revision_and_preserves_newlines` — MIXED (T2+T5); 读取真实 CRLF/LF 内容并检查元数据/省略的 revision；T2:内容/元数据响应映射；T5:生产代码文件读取。证据 L2688–L2700 [证据 L2688–L2700]
- `search_streams_file_byte_and_output_limits_without_overshoot` — MIXED (T3+T5); 针对真实文件检查文件数/字节扫描限制及输出上限；T3:扫描/输出预算；T5:生产代码遍历/读取。证据 L2932–L3008 [证据 L2932–L3008]
- `search_supports_literal_regex_glob_context_and_skip_accounting` — MIXED (T3+T5); 搜索测试夹具树并检查 literal/regex/glob 匹配、上下文、偏移量、跳过非 UTF8 内容及结果截断；T3:搜索/匹配/上下文逻辑；T5:生产代码遍历/文件读取。证据 L2865–L2928 [证据 L2865–L2928]
- `symlinks_are_allowed_only_when_the_canonical_target_stays_inside_policy` — MIXED (T3+T5); 接受根目录内的 symlink，并拒绝根目录外的目标；T3:路径策略判定；T5:真实 Unix symlink/规范路径查找。证据 L2834–L2861（`#[cfg(unix)]`） [证据 L2834–L2861 (`#[cfg(unix)]`)]

### `crates/agentic-gpt/src/ingress/hub.rs`
- `recovery_replays_completed_results_but_never_executes_retired_argv_commands` — MIXED (T4+T5); T4 恢复操作将已退役/当前命令与响应通道协调一致，并保留已完成的结果；T5 读写临时传输账本，并验证已退役的 argv 从不创建标记文件 [证据 L1431–L1575]

### `crates/agentic-gpt/src/ingress/stdio_server_tests.rs`
- `browser_manual_dispatch_uses_selected_runtime_docs_root` — MIXED (T4+T5); T4 测试手动读取/搜索分发及路径拒绝；T5 读取实际的临时文档文件 [证据 L783–L822]
- `browser_missing_runtime_has_stable_degraded_results` — MIXED (T4+T5); T4 检查 Browser 适配器的降级响应；T5 读取生成的审计文件，并验证经哈希处理/脱敏的失败调用记录 [证据 L707–L752]
- `browser_repl_event_decoration_preserves_inner_result_channels` — MIXED (T2+T4); T2 验证结果的 content/structured/error/meta 通道在装饰后仍保留；T4 通过 MCP stdio 发送预设的伪内核 Browser 结果，并检查一个面板。空 EventStore 读取不计入 T5 [证据 L876–L955]
- `changing_live_toolsets_updates_surface_and_authorization` — MIXED (T1+T4+T5); T1 检查对外公布的接口面的变化；T4 验证调用会随 config 切换而被拒绝/允许；T5 成功引导时读取预置的临时引导文件 [证据 L1189–L1232]
- `compact_mcp_skills_and_tmux_adapters_preserve_result_envelopes` — MIXED (T4+T5); T4 分发 MCP/skills/tmux 并检查基本信封结构；T5 尝试执行真实的外部 tmux list 命令 [证据 L3669–L3686]
- `denied_process_batch_creates_no_processes` — MIXED (T4+T5); T4:模拟 Hub 确认被拒绝，并验证批次被拒绝；T5:验证拒绝后的 ProcessHistory SQLite 列表仍为空（断言无进程持久化副作用）。 [证据 L2268–2309]
- `deterministic_tool_contract_corpus_exercises_public_dispatch` — MIXED (T1+T4+T5); T1 检查描述符必填字段；T4 分发正向/负向用例；T5 读取/搜索真实的临时测试夹具文件 [证据 L1013–L1164]
- `external_injection_waits_for_the_next_public_panel_exposure` — MIXED (T4+T5); T4 测试仅限 LocalUnix 的注入、面板暴露/清除及 Http 拒绝；T5 断言持久化事件存储的可见性及显示计数变化 [证据 L260–L323]
- `file_edit_add_creates_nested_parents_after_whole_patch_preflight` — MIXED (T4+T5); T4 检查整个补丁的预检、确认、外部更改及提交失败结果；T5 断言真实目录/文件内容及不写入保证 [证据 L3310–L3393]
- `file_edit_add_keeps_path_policy_for_missing_parents_and_symlinks` — MIXED (T4+T5); T4 检查被拒绝根目录/路径策略判定；T5 创建真实 symlink，并验证未向任何被拒绝的目录写入 [证据 L3421–L3453]
- `file_edit_add_reserves_audit_path_under_symlinked_workspace` — MIXED (T4+T5); T4 检查保留路径拒绝；T5 创建真实的工作区 symlink，并验证审计路径仍为文件 [证据 L3394–L3420]
- `file_edit_apply_patch_context_mismatch_and_confirmation_do_not_write` — MIXED (T4+T5); T4 检查补丁上下文及确认失败路径；T5 验证原文件保持不变 [证据 L3563–L3596]
- `file_edit_apply_patch_revalidates_external_change_before_commit` — MIXED (T4+T5); T4 检查 revision 重新验证/冲突行为；T5 验证外部更改后的文件内容仍保留在磁盘上 [证据 L3598–L3614]
- `file_edit_apply_patch_supports_multi_file_changes_and_slim_response` — MIXED (T2+T4+T5); T2 检查精简的逐文件更改投影；T4 应用多操作补丁；T5 验证真实的添加/更新/删除/移动效果 [证据 L3455–L3500]
- `file_edit_external_changes_remain_isolated_by_path` — MIXED (T4+T5); T4 检查跨服务器状态的独立 revision 冲突处理；T5 验证两个真实工作区文件保留各自不同的内容 [证据 L3616–L3653]
- `file_edit_later_commit_failure_reports_prior_and_skipped_changes` — MIXED (T2+T4+T5); T2 检查 committed/failed/skipped 响应及审计映射；T4 测试有序部分提交行为；T5 断言注入提交故障时实际的文件及审计日志影响 [证据 L3502–L3561]
- `file_image_response_adds_event_panel_text_once` — MIXED (T2+T4+T5); T2 检查 image/text/panel 通道投影无重复；T4 测试 stdio 装饰链；T5 读取 PNG 测试样本和预置的 EventStore 事件，检查显示计数 [证据 L2737–L2822]
- `file_read_and_search_batches_preserve_order_and_isolate_failures` — MIXED (T4+T5); T4 检查批次顺序、逐项故障隔离及无效的单项/批量组合；T5 读取/搜索实际临时文件 [证据 L3245–L3308]
- `file_read_dispatch_supports_content_and_metadata_modes` — MIXED (T4+T5); T4 检查内容/元数据/错误响应路径；T5 读取实际临时文件并验证其内容/行元数据 [证据 L2667–L2691]
- `file_search_dispatch_supports_literal_and_regex_queries` — MIXED (T4+T5); T4 检查配置的上下文裁剪及字面量/regex 搜索行为；T5 搜索实际临时文件 [证据 L3177–L3243]
- `in_process_stdio_file_read_batch_descriptor_and_call_contract` — MIXED (T1+T4+T5); T1 检查 file.read 批量描述符契约；T4 通过 MCP 双工调用并验证有序部分结果；T5 读取实际临时文件 [证据 L2693–L2735]
- `in_process_stdio_file_read_enforces_image_pixel_and_response_bounds` — MIXED (T2+T4+T5); T2 检查错误/预算投影及批量恢复；T4 通过 stdio 驱动文件读取；T5 读取格式错误/超大的图像和文本测试样本，并断言实际响应边界 [证据 L3013–L3175]
- `in_process_stdio_file_read_projects_static_and_gif_images` — MIXED (T2+T4+T5); T2 检查图像字节/MIME/帧及批量结果投影；T4 测试 MCP 文件读取/批量输出；T5 读取并解码实际图像测试样本 [证据 L2824–L3011]
- `local_mcp_call_audit_uses_local_request_source` — MIXED (T4+T5); T4 检查被拒绝的 MCP 操作及终态审计证据；T5 读取已写入的审计文件 [证据 L2540–L2566]
- `local_skill_audit_uses_local_request_source` — MIXED (T4+T5); T4 通过 LocalUnix 分发激活/运行技能，并检查请求来源审计映射；T5 创建并执行实际临时 shell 脚本，读取审计文件 [证据 L2504–L2538]
- `managed_batch_uses_one_confirmation_for_all_elements` — MIXED (T4+T5); T4 模拟 Hub 确认，检查一次确认服务于整个批次；T5 执行两个命令并从磁盘读取其审计记录 [证据 L2210–L2266]
- `normal_and_room_tool_sets_follow_fixed_surface_contract` — MIXED (T1+T4); T1 断言精确的 Normal/Room 工具接口及 schema；T4 更改命名空间配置，验证公布的工具及调用授权随之更新 [证据 L100–L258]
- `process_creation_and_batch_responses_obey_response_budget` — MIXED (T2+T4+T5); T2 检查紧凑响应字段/序列化字节预算；T4 检查单项/批量响应流程；T5 运行产生大量输出的命令 [证据 L1840–L1892]
- `process_creation_read_cancel_and_batch_use_process_api` — MIXED (T4+T5); T4 检查进程创建/读取/列出/取消/批量状态转换及批量预检；T5 启动 true/sleep/false 命令，并断言实际结果/终止情况 [证据 L1490–L1623]
- `process_read_omitted_waits_but_zero_is_nonblocking` — MIXED (T4+T5); T4 比较进程读取的等待语义；T5 运行实际 sleep 进程并观察其活动/完成状态 [证据 L1625–L1668]
- `process_read_preserves_mcp_result_states_and_complete_values` — MIXED (T2+T4+T5); T2 检查 included/deferred/unavailable/not-retained 结果投影；T4 预置 ProcessHistory 并通过分发读取；T5 断言持久化的 SQLite 结果数据及重试 [证据 L2050–L2208]
- `process_read_preserves_raw_byte_offsets_and_utf8_output` — MIXED (T2+T4+T5); T2 验证字节偏移/游标及 UTF-8/base64 映射；T4 将进程创建与分页读取串联；T5 执行 printf 并读取捕获的输出 [证据 L1768–L1838]
- `process_shapes_are_compact_and_keep_group_filters` — MIXED (T4+T5); T4 检查读取/列出投影、组过滤及拒绝状态；T5 执行 true/sleep/false 命令 [证据 L1670–L1766]
- `process_tools_reject_legacy_identity_and_confirmation_fields` — MIXED (T1+T4); T1 检查现行/已退役的进程 API 名称及接受的 schema 结构；T4 调用服务器，验证旧字段/别名及无效范围被拒绝 [证据 L1371–L1488]
- `room_profile_dispatches_room_memory_tools` — MIXED (T4+T5); T4 分发 bootstrap/skills/notebook/diary/state 适配器；T5 的 bootstrap 读取预置的临时工作区文件 [证据 L2619–L2633]
- `targetless_hub_listing_keeps_events_for_targeted_calls` — MIXED (T4+T5); T4 驱动先被抑制、后定向的 Hub 响应；T5 断言持久化 EventStore 中的事件记录及显示计数 [证据 L350–L428]
- `terminal_result_is_not_suppressed_when_panel_preparation_fails` — MIXED (T4+T5); T4 测试装饰失败后的进程结果/事件面板恢复；T5 执行 `true`，并触发/检查实际 SQLite 触发器故障及保留的事件 [证据 L430–L479]
- `tunnel_local_and_http_ingress_advertise_identical_surface` — MIXED (T1+T2); T1 检查公布的工具描述符完全一致；T2 检查入口标签/来源字符串映射 [证据 L2483–L2502]
- `tunnel_skill_audit_uses_tunnel_request_source` — MIXED (T4+T5); T4 检查技能完成及隧道来源审计映射；T5 创建/执行临时 shell 脚本并读取审计输出 [证据 L2568–L2600]

### `crates/agentic-gpt/src/mcp/mcp_tests.rs`
- `config_cli_rejects_invalid_server_without_writing_and_accepts_valid_server` — MIXED (T3+T5); **T3:** 检查添加无效和有效服务器时的配置验证。**T5:** 验证拒绝时实际配置文件保持不变，且有效添加会被持久化/重新加载。 [证据 L1837–1878]
- `managed_mcp_fast_result_uses_real_rmcp_transport` — MIXED (T4+T5); **T4:** 通过进程内 RMCP 测试服务器执行受管理的调用/进程流程。**T5:** 验证持久化审计 JSONL 的元数据及秘密信息脱敏。 [证据 L348–400]
- `mcp_batch_clips_late_results_to_the_aggregate_budget` — MIXED (T2+T4); **T2:** 将省略的保留结果投影为 deferred。**T4:** 测试批量结果裁剪及进程读取恢复。 [证据 L1192–1295]
- `mcp_batch_impossible_response_budget_rejects_before_registration_or_effects` — MIXED (T4+T5); **T4:** 检查拒绝发生在批量/进程/确认产生影响之前。**T5:** 检查持久化的响应预算审计记录。 [证据 L973–1026]
- `mcp_batch_preflight_and_capacity_fail_atomically_before_confirmation` — MIXED (T4+T5); **T4:** 检查批量验证及容量拒绝发生在注册、确认或工具调用之前。**T5:** 检查持久化的拒绝/容量审计记录。 [证据 L792–969]
- `mcp_batch_public_projection_obeys_exact_json_budget_and_keeps_deferred_result_readable` — MIXED (T2+T4); **T2:** 检查在最小、恰好容纳及少一个字节的 JSON 预算下的投影。**T4:** 测试批量结果及进程读取恢复。 [证据 L1298–1452]
- `mcp_batch_sequential_fail_fast_preserves_order_and_audit_correlation` — MIXED (T4+T5); **T4:** 首个工具错误会跳过后续调用，并保留子项顺序/关联。**T5:** 检查持久化的批次/子项审计记录。 [证据 L1030–1126]

### `crates/agentic-gpt/src/operations/event_notifications.rs`
- `process_history_recovery_failure_does_not_block_event_access` — MIXED (T4+T5); T4 测试恢复时处理已禁用的进程历史存储且不阻塞事件访问；T5 从临时 EventStore 检索先前插入的事件 [证据 L634–L662]
- `recovery_does_not_locally_settle_remote_awaiting_source` — MIXED (T4+T5); T4 测试恢复时保留 remote-awaiting 来源/起源；T5 断言已注册/已绑定/已完成的来源仍保留在持久化 EventStore 中 [证据 L598–L632]
- `response_feedback_requires_registered_sources_and_keeps_off_policy_quiet` — MIXED (T4+T5); T4 测试来源注册、完成结算及策略外抑制；T5 通过临时 EventStore 数据库写入/读取已注册的完成状态 [证据 L448–L559]

### `crates/agentic-gpt/src/process/exec.rs`
- `losing_startup_channel_before_ready_prevents_command_execution` — MIXED (T4+T5); 在 init 阻塞时关闭启动通道，并检查解除阻塞后命令标记仍不存在；T4: 启动通道/命令门控链；T5: 实际子进程/FD/文件系统行为。证据 L803–L878；Unix 测试模块 [证据 L803–L878]
- `relative_shell_init_file_is_not_resolved_through_path` — MIXED (T3+T5); 在 cwd 和 PATH 中存在同名 init 时启动 shell；检查 cwd 来源/输出及启动标记；T3: 相对 init 解析规则；T5: 实际 shell 及文件读取。证据 L719–L802；Unix 测试模块 [证据 L719–L802]

### `crates/agentic-gpt/src/process/managed.rs`
- `aborting_cancellation_releases_stale_terminal_process_pin` — MIXED (T4+T5); 在获取 pin 后中止取消操作，杀死抵抗 TERM 的子进程组，并检查清理成功；T4: 取消 pin/清理链；T5: 实际组/任务影响。证据 L6378–L6469 [证据 L6378–L6469]
- `active_process_capacity_and_cancel_are_truthful` — MIXED (T4+T5); 在 sleep 仍运行时检查容量拒绝，随后检查取消及观察到的进程组证据；T4: 容量/取消响应状态；T5: 实际子进程/进程组信号。证据 L6048–L6078 [证据 L6048–L6078]
- `admitted_process_keeps_policy_snapshot_across_reload` — MIXED (T4+T5); 在准入 printf 后更改策略，并检查已准入的子进程仍完成且有输出；T4: 准入/配置快照行为；T5: 实际子进程。证据 L5751–L5775 [证据 L5751–L5775]
- `cancellation_reaches_background_descendant_after_leader_exit` — MIXED (T4+T5); 组长进程退出时后台 sleep 仍在运行；取消操作杀死进程组并完成捕获收尾；T4: 受管理的组长进程/进程组/捕获生命周期；T5: 实际后代进程组。证据 L6080–L6113 [证据 L6080–L6113]
- `cancelling_a_terminal_process_reports_already_terminal_without_rewriting_state` — MIXED (T4+T5); 在 true/history 完成后，检查取消操作的幂等性以及实时/历史字段保持不变；T4:终态不变量；T5:真实子进程和历史 DB。证据 L6471–L6525 [证据 L6471–L6525]
- `cancelling_aged_terminal_process_returns_detail_after_cache_pruning` — MIXED (T4+T5); 使带后台进程组的缓存终态进程老化，执行取消，并检查终态详情/进程组终止；T4:老化缓存的取消行为；T5:真实进程组终止。证据 L6332–L6376 [证据 L6332–L6376]
- `completed_processes_release_capacity_and_keep_output` — MIXED (T4+T5); 完成 true，准入 printf，检查过期状态刷新以及捕获的输出/EOF；T4:容量释放/状态/捕获链路；T5:真实子进程。证据 L4947–L4972 [证据 L4947–L4972]
- `exec_preserves_rejection_when_process_was_not_admitted` — MIXED (T4+T5); 用 sleep 占满容量，检查第二条命令返回已拒绝/未启动响应；T4:准入到响应的行为；T5:运行中的子进程/容量争用。证据 L4913–L4945 [证据 L4913–L4945]
- `exec_waits_for_terminal_state_after_early_output` — MIXED (T4+T5); 先运行 printf，再运行 sleep，并等待终态响应；T4:等待/输出/状态链路；T5:真实子进程。证据 L4899–L4910 [证据 L4899–L4910]
- `mcp_batch_admission_failure_is_atomic_before_registration` — MIXED (T4+T5); SQLite 触发器拒绝第二次注册，验证不存在部分注册/历史记录，然后重试；T4:MCP 注册原子性；T5:真实历史数据库。证据 L5860–L5883 [证据 L5860–L5883]
- `output_pages_base64_invalid_utf8_without_loss` — MIXED (T2+T5); 子进程输出无效字节；检查 Base64 表示可往返还原字节且遵守响应预算；T2:二进制到 Base64 的响应映射；T5:真实子进程输出。证据 L5396–L5433 [证据 L5396–L5433]
- `oversized_process_admission_fails_before_command_effect` — MIXED (T4+T5); Linux 超大命令准入失败，且无标记、进程注册表条目或历史记录；T4:大小检查/准入链路；T5:真实文件系统/历史记录的无影响检查。证据 L5660–L5711（`#[cfg(target_os = "linux")]`） [证据 L5660–L5711 (`#[cfg(target_os = "linux")]`)]
- `oversized_process_batch_is_rejected_before_any_admission` — MIXED (T4+T5); 拒绝含 100 个元素的批次，检查进程/历史状态为空且无标记文件；T4:批次级预算/准入逻辑；T5:真实历史记录/文件系统的无影响检查。证据 L5712–L5748 [证据 L5712–L5748]
- `process_batch_admission_failure_is_atomic_before_spawn` — MIXED (T4+T5); SQLite 触发器使第二次准入失败，检查不存在部分历史记录/进程/标记，然后重试真实子进程；T4:原子批次准入链路；T5:历史 DB 和子进程执行。证据 L5777–L5858 [证据 L5777–L5858]
- `process_batch_respects_max_concurrent_tasks_without_blocking_batch_return` — MIXED (T4+T5); 检查批次返回时一个 sleep 正在运行、另一个正在排队，然后取消它们；T4:调度器/队列行为；T5:真实子进程/取消。证据 L5885–L5955 [证据 L5885–L5955]
- `process_batch_response_obeys_whole_body_budget_and_keeps_identities` — MIXED (T2+T5); 运行两个大输出子进程，检查整个响应体预算、批次标识/索引及分页；T2:标识/响应预算映射；T5:真实批次子进程。证据 L5580–L5659 [证据 L5580–L5659]
- `process_batch_wait_wakes_for_later_child` — MIXED (T4+T5); 在并发度为 1 时将 true 和短时 sleep 入队；检查批次等待在两者均完成后返回；T4:排队子进程的唤醒/状态链路；T5:真实子进程。证据 L5957–L6000 [证据 L5957–L6000]
- `process_filters_and_restart_loss_are_explicit` — MIXED (T2+T5); 运行 true，检查 completed 过滤条件以及旧启动实例与未知 id 的错误；T2:过滤条件/id 到错误的映射；T5:真实子进程。证据 L6527–L6558 [证据 L6527–L6558]
- `process_list_merges_live_wins_and_uses_global_cursor_order` — MIXED (T3+T5); 插入持久化行/实时重复项，检查实时状态优先、过滤条件及全局游标顺序；T3:合并/过滤/游标逻辑；T5:SQLite 历史记录读写。证据 L6629–L6737 [证据 L6629–L6737]
- `process_read_compacts_output_to_configured_response_budget` — MIXED (T2+T5); 运行大输出任务，检查响应符合配置的预算且带有后续页；T2:响应大小适配；T5:真实子进程输出。证据 L5477–L5496 [证据 L5477–L5496]
- `process_read_cursors_are_replayable_and_page_raw_offsets` — MIXED (T2+T5); 针对子进程输出，检查有界且可重放的分页、偏移量、重建，以及格式错误/状态/超前/外来游标错误；T2:游标/分页映射；T5:真实进程输出。证据 L5267–L5394 [证据 L5267–L5394]
- `reader_eof_is_visible_while_the_child_keeps_running` — MIXED (T4+T5); 检查活动子进程可在进程完成前已完成输出捕获/到达 EOF；T4:读取器/进程状态协调；T5:真实子进程/管道。证据 L5111–L5154 [证据 L5111–L5154]
- `revoked_group_is_not_reused_by_capacity_reader_or_cancellation` — MIXED (T3+T5); 在 Unix 上撤销真实 PGID，检查不探测复用的 id，并执行读取器/取消/清理路径；T3:已撤销 ID 的观测逻辑；T5:真实进程组生命周期。证据 L6191–L6330（`#[cfg(unix)]`） [证据 L6191–L6330 (`#[cfg(unix)]`)]
- `runtime_history_tracks_group_timestamps_and_hot_cache_fallback` — MIXED (T4+T5); 运行进程组中的 printf，检查时间戳/历史记录/输出，清理缓存，然后检索持久化记录/输出；T4:实时/历史回退链路；T5:真实子进程和历史 DB。证据 L6560–L6627 [证据 L6560–L6627]
- `shell_init_failure_blocks_command_and_reports_init_error` — MIXED (T4+T5); init 返回 23；验证命令未创建标记，且受管错误报告 init 失败；T4:启动失败到详情的链路；T5:真实子进程/文件。证据 L5007–L5032 [证据 L5007–L5032]
- `shell_init_fd3_changes_preserve_startup_channel` — MIXED (T4+T5); 在 init 期间关闭/重新绑定 FD3，检查命令执行情况及输出字节；T4:启动通道处理；T5:真实 shell/描述符/文件。证据 L5034–L5078 [证据 L5034–L5078]
- `shell_init_runs_at_top_level_before_working_directory_is_restored` — MIXED (T4+T5); init 设置变量/标记并更改 cwd；命令可见该变量，但使用配置的 cwd；T4:启动/命令执行顺序；T5:真实 shell/文件系统。证据 L4974–L5005 [证据 L4974–L5005]
- `terminal_events_precede_inherited_pipe_eof_and_history_keeps_final_output` — MIXED (T4+T5); 检查终态事件/钩子先于 EOF，后续继承的输出被捕获，且历史记录持久保存；T4:事件/读取器/历史记录顺序；T5:真实子进程/事件/历史记录资源。证据 L5156–L5265 [证据 L5156–L5265]
- `terminal_leader_keeps_capacity_after_eof_until_group_retirement` — MIXED (T4+T5); 检查处于终态的组长进程在进程组退出期间仍占用容量，随后新子进程获准进入；T4:容量/进程组退出链路；T5:真实后台进程组。证据 L6115–L6188 [证据 L6115–L6188]

### `crates/agentic-gpt/src/room/bootstrap.rs`
- `flat_discovery_ignores_hidden_non_markdown_and_nested_entries` — MIXED (T3+T5); **T3:**检查指南发现/过滤规则。**T5:**将这些规则应用于真实目录条目/文件。 [证据 L908–925]
- `frontmatter_scan_limit_accepts_exact_boundary_and_rejects_overflow` — MIXED (T3+T5); **T3:**检查恰好达到边界时接受、超出时拒绝。**T5:**读取真实入口点/指南文件。 [证据 L1149–1189]
- `guides_sort_by_priority_then_id_and_manifest_caps_at_64_but_read_keeps_all` — MIXED (T3+T5); **T3:**检查优先级排序、清单上限和读取选择。**T5:**发现/读取真实指南文件。 [证据 L1020–1051]
- `invalid_and_valid_guides_have_different_revision_membership` — MIXED (T3+T5); **T3:**检查有效与无效指南内容如何影响修订版本。**T5:**修改并重新加载真实文件。 [证据 L1054–1082]
- `invalid_guides_are_excluded_and_duplicate_ids_are_order_independent` — MIXED (T3+T5); **T3:**检查重复 ID、元数据及 UTF-8 的验证/警告行为。**T5:**加载真实指南文件。 [证据 L928–968]
- `missing_package_and_invalid_entrypoint_are_fail_closed` — MIXED (T2+T5); **T2:**检查错误映射到 `bootstrap_not_found` 与 `bootstrap_invalid` 的区别。**T5:**加载器检查真实文件系统中的包/入口点。 [证据 L861–878]
- `mixed_validity_duplicate_ids_exclude_every_candidate` — MIXED (T3+T5); **T3:**检查对有效和无效候选项的重复 ID 排除。**T5:**从真实文件加载候选项。 [证据 L971–1001]
- `oversized_files_keep_full_metadata_with_bounded_prefixes` — MIXED (T2+T5); **T2:**检查大小/哈希元数据和有界前缀投影。**T5:**读取超大入口点和指南文件。 [证据 L1106–1146]
- `utf8_validation_accepts_multibyte_codepoint_split_across_scan_chunks` — MIXED (T3+T5); **T3:**检查分块边界处的 UTF-8 验证、摘要及行数。**T5:**扫描真实文件。 [证据 L1192–1212]
- `valid_entrypoint_defaults_and_crlf_are_supported` — MIXED (T3+T5); **T3:**检查解析出的 CRLF 元数据和默认字段。**T5:**加载真实入口点和指南文件。 [证据 L882–905]

### `crates/agentic-gpt/src/room/room_maintenance.rs`
- `local_submit_preflights_and_commits_one_semantic_change` — MIXED (T2+T5); **T2:**检查载荷到 Markdown 的映射。**T5:**在真实仓库中验证生成的文件、提交及 worktree 状态。 [证据 L1458–1497]
- `preflight_rejects_invalid_payload_without_mutating_room` — MIXED (T3+T5); **T3:**检查无效载荷验证。**T5:**验证真实仓库文件和提交数量保持不变。 [证据 L1500–1534]
- `status_reports_capabilities_and_empty_slots` — MIXED (T3+T5); **T3:**检查状态/能力/槽位映射。**T5:**初始化真实仓库后获取状态。 [证据 L1429–1455]
- `submit_rejects_duplicate_slots_and_dirty_repository` — MIXED (T3+T5); **T3:**检查重复槽位拒绝。**T5:**检查基于真实脏 Git 状态的拒绝。 [证据 L1537–1567]
- `workflow_wait_timeout_preserves_submitted_request` — MIXED (T2+T5); **T2:**检查请求序列化/转发。**T5:**等待超时后，在真实本地 Git 仓库和裸 origin 中验证已提交的请求。 [证据 L1321–1363]

### `crates/agentic-gpt/src/room/room_reads.rs`
- `diary_active_and_exact_reads_are_deterministic_for_current_layout` — MIXED (T2+T5); **T2:** 检查日记层级/周期/路径映射。**T5:** 读取实际的当前日记文件和精确周期日记文件。 [证据 L624–758]
- `diary_active_reports_missing_layer_without_mutating_repository` — MIXED (T3+T5); **T3:** 检查层级缺失时的响应及不修改数据的行为。**T5:** 针对实际的不完整日记目录树进行验证。 [证据 L761–807]
- `notebook_recent_orders_by_recency_before_limit` — MIXED (T3+T5); **T3:** 检查先按新近程度排序再限制数量，以及确定性的同值排序规则。**T5:** 发现实际笔记本路径并从文件构建文档。 [证据 L873–920]
- `state_list_entities_round_trip_for_dotted_stem` — MIXED (T2+T5); **T2:** 检查含点文件名与实体的映射。**T5:** 列出并读取实际状态文件。 [证据 L592–621]
- `state_list_is_sorted_and_state_read_rejects_oversized_markdown` — MIXED (T3+T5); **T3:** 检查排序和大小限制规则。**T5:** 列出/读取实际 Markdown 文件，包括超大文件。 [证据 L810–870]

### `crates/agentic-gpt/src/room/room_repository.rs`
- `absent_root_bootstraps_exact_scaffold_and_one_main_commit` — MIXED (T1+T5); **T1:** 检查精确的脚手架路径清单。**T5:** 初始化并检查实际 Git 仓库。尽管名称如此，测试主体并未断言提交数为一次。 [证据 L1102–1124]
- `bounded_markdown_helper_caps_bytes_and_rejects_symlinks` — MIXED (T3+T5); **T3:** 检查字节上限和扩展名行为。**T5:** 读取实际文件。尽管名称如此，测试主体并未创建/测试符号链接。 [证据 L1365–1375]
- `empty_root_bootstraps_exact_scaffold` — MIXED (T1+T5); **T1:** 检查精确的脚手架清单。**T5:** 验证实际 Git 分支/目录树及提交数为一次。 [证据 L1127–1155]
- `independent_bootstraps_have_the_same_deterministic_initial_commit` — MIXED (T3+T5); **T3:** 检查初始 bootstrap 输出的确定性。**T5:** 比较两个实际 Git 提交及其文件。 [证据 L1158–1180]
- `invalid_boundary_does_not_partially_initialize_repository` — MIXED (T3+T5); **T3:** 检查无效边界被拒绝。**T5:** 验证真实仓库根目录未被创建。 [证据 L1092–1099]
- `path_validation_rejects_escape_and_symlink_root` — MIXED (T2+T5); **T2:** 检查路径/路径遍历验证。**T5:** 在 Unix 上，验证指向真实仓库根目录的符号链接被拒绝。 [证据 L1255–1276]
- `schema_version_detection_distinguishes_outdated_and_invalid_metadata` — MIXED (T3+T5); **T3:** 检查 Outdated 与 Missing 就绪状态的分类。**T5:** 检查实际仓库中已修改的元数据。 [证据 L1233–1252]
- `status_keeps_repository_schema_executor_workflow_remote_and_sync_distinct` — MIXED (T3+T5); **T3:** 检查就绪状态分类。**T5:** 比较实际 Git/bootstrap 操作前后的状态。 [证据 L1294–1313]
- `workflow_uses_the_repository_owned_executor` — MIXED (T1+T5); **T1:** 检查生成的固定工作流命令/路径列表。**T5:** 执行 bootstrap 并读取实际仓库工作流文件。 [证据 L1316–1333]

### `crates/agentic-gpt/src/runtime/agent_info.rs`
- `config_health_ignores_path_policy_drift_with_workspace_restart` — MIXED (T3+T5); 比较运行中与磁盘上的工作区/路径根目录；要求包含重启字段且不存在 live-subset 问题；T3: config-health 比较逻辑；T5: 实际配置文件读取及真实路径解析。证据 L537–L578 [证据 L537–L578]
- `info_reports_invalid_config_and_capacity_exhaustion_without_secrets` — MIXED (T4+T5); 检查磁盘上的无效 JSON、降级的容量健康状态以及机密信息的省略；T4: 聚合健康报告；T5: 实际无效配置文件读取。证据 L695–L730 [证据 L695–L730]
- `info_reports_mcp_live_subset_revision_without_restart_requirement` — MIXED (T4+T5); 检查更改运行中 MCP 配置前后的 MCP 计数/修订版本及磁盘配置无效报告；T4: info/config-health 及运行中 MCP 状态链；T5: 实际磁盘配置读写。证据 L581–L660 [证据 L581–L660]
- `info_reports_restart_differences_and_current_ntfy_relay` — MIXED (T4+T5); 根据生效配置和磁盘配置检查重启字段/当前 ntfy 状态；T4: 运行时 info/config-health 聚合；T5: 实际临时配置文件读取。证据 L492–L534 [证据 L492–L534]
- `info_reports_toolset_live_subset_difference_without_restart_requirement` — MIXED (T3+T5); 比较生效配置和磁盘配置中的工具集，再检查应用后的 live-subset 匹配情况；T3: live-subset/重启分类；T5: 实际配置文件读取。证据 L663–L692 [证据 L663–L692]

### `crates/agentic-gpt/src/runtime/main_tests.rs`
- `batch_confirmation_preview_supports_chinese` — MIXED (T1+T2); 渲染中文预览并检查本地化措辞/cwd/转义；T1: 固定的本地化字符串；T2: 将批处理元素渲染为预览。证据 L1107–L1124 [证据 L1107–L1124]
- `cli_version_uses_crate_version` — MIXED (T1+T2); 解析 `--version` 并检查 display-version 类型及渲染的 crate 版本；T1: 固定版本文本；T2: CLI 版本渲染/转发。证据 L309–L318 [证据 L309–L318]
- `deny_roots_override_read_and_write` — MIXED (T3+T5); 检查在实际禁止访问的根目录下，读写命令均被拒绝；T3: 拒绝规则优先；T5: 真实文件系统路径解析。证据 L1040–L1075 [证据 L1040–L1075]
- `hub_panel_failure_emits_live_event_source_without_terminal_response` — MIXED (T4+T5); 在 hub `true` 命令后强制 SQLite 面板更新失败，并检查事件源交接/无响应；T4: hub/事件时序；T5: 子进程执行及 SQLite 触发器/存储操作。证据 L561–L652 [证据 L561–L652]
- `load_old_config_without_path_policy_adds_defaults` — MIXED (T2+T5); 写入不含 `pathPolicy` 的旧版配置，重新加载并检查默认值；T2: 旧版字段到默认值的映射；T5: 生产代码中的配置文件读取。证据 L1239–L1250 [证据 L1239–L1250]
- `load_partial_path_policy_uses_workspace_derived_defaults_for_missing_lists` — MIXED (T2+T5); 仅加载可写根目录，并检查其他列表使用工作区默认值；T2: 部分配置映射；T5: 生产代码中的配置文件读取。证据 L1254–L1266 [证据 L1254–L1266]
- `local_arguments_are_bounded_objects_from_inline_or_file` — MIXED (T2+T5); 检查内联/默认/无效/超大 JSON，并读取临时参数文件；T2: JSON 对象及大小处理；T5: 生产代码中的文件读取。证据 L383–L410 [证据 L383–L410]
- `managed_cache_precedes_desktop_and_preserves_descriptor_fields` — MIXED (T3+T5); 注入的 provider 使托管缓存优先胜出，并检查描述符/调用次数以及 Unix codex-home 权限模式；T3: provider 选择及描述符行为；T5: 生产代码中的目录权限断言。证据 L146–L189 [证据 L146–L189]
- `normal_runtime_follows_live_room_toolset_for_bootstrap_dispatch` — MIXED (T4+T5); 文件提供 bootstrap/指南数据；分派在运行中工具集启用前拒绝请求，启用后返回指南；T4: 运行中配置/分派链；T5: 生产代码中的工作区文件读取。证据 L488–L532 [证据 L488–L532]
- `old_rule_ids_are_ignored_when_loading_config` — MIXED (T2+T5); 加载旧版规则 id，检查规则被保留且序列化形式省略 id；T2: 旧版字段映射；T5: 配置文件往返读写。证据 L1271–L1290 [证据 L1271–L1290]
- `path_policy_allows_write_root_and_blocks_readonly_write` — MIXED (T3+T5); 针对配置的可写/只读根目录预检 touch/du；T3: 命令访问分类/策略；T5: 生产代码中对真实临时根目录的解析。证据 L996–L1036 [证据 L996–L1036]
- `path_root_remove_matches_expanded_equivalent_path` — MIXED (T3+T5); 添加根目录并使用 `target/../target` 将其移除；T3: 根目录修改/等价路径行为；T5: 已有根目录的路径规范化。证据 L1341–L1362 [证据 L1341–L1362]
- `read_only_system_file_is_allowed` — MIXED (T3+T5); 检查预检允许 `cat /proc/meminfo` 和 `df /`；T3: 只读命令/路径策略；T5: 针对实际主机路径的检查。证据 L983–L993；保留 **REVIEW**：`/proc/meminfo` 依赖特定平台/环境，且未设条件门控 [证据 L983–L993]
- `relative_path_arguments_are_resolved_from_working_directory` — MIXED (T3+T5); 检查真实的相对目标从工作区根目录访问时失败，从包含该目标的 cwd 访问时成功；T3: 相对于 cwd 的参数解析；T5: 生产代码中的路径查找。证据 L1178–L1203 [证据 L1178–L1203]
- `room_mode_dispatches_bootstrap_manifest_and_read` — MIXED (T4+T5); 分派 manifest/read 命令并检查解析后的数据及精确的指南内容；T4: hub 命令/响应链；T5: 生产代码中的测试夹具文件读取。证据 L655–L702 [证据 L655–L702]
- `room_mode_dispatches_current_diary_command` — MIXED (T4+T5); 分派 active-diary 请求并检查日记缺失结果；T4: hub 命令/响应行为；T5: 查找真实环境中不存在的仓库资源。证据 L726–L744 [证据 L726–L744]
- `room_timezone_defaults_and_can_be_overridden` — MIXED (T1+T2); 检查固定默认值及保留的自定义时区/日期边界值；T1: 默认值；T2: 配置字段更新/回读。证据 L420–L427 [证据 L420–L427]
- `standalone_live_reload_applies_valid_mcp_map_and_rejects_invalid_candidate` — MIXED (T4+T5); 检查有效的 MCP/limits/shell 更新，再检查对无效 transport 的原子性拒绝；T4: 验证及运行中状态更新链；T5: 实际候选配置文件读取。证据 L1559–L1682 [证据 L1559–L1682]
- `standalone_live_reload_bootstraps_room_repository_before_maintenance_submit` — MIXED (T4+T5); 重新加载 Room 设置，检查运行中根目录的脚手架，提交维护任务并读取创建的笔记本；T4: 重新加载/仓库维护链；T5: 真实仓库/文件系统影响。证据 L1685–L1756 [证据 L1685–L1756]
- `symlink_to_denied_path_is_rejected` — MIXED (T3+T5); 在 Unix 上创建指向禁止访问目标的符号链接，并检查预检拒绝；T3: 拒绝策略；T5: 符号链接/规范路径查找。证据 L1207–L1235；保留 **REVIEW**：在非 Unix 平台上，该断言被编译排除 [证据 L1207–L1235]
- `unknown_program_defaults_to_write_access` — MIXED (T3+T5); 检查以只读根目录为目标的未知程序被视为写入操作；T3: 未知程序访问分类；T5: 真实路径解析。证据 L1079–L1102 [证据 L1079–L1102]
- `working_directory_must_be_existing_writable_directory` — MIXED (T3+T5); 检查有效目录，并拒绝文件、不存在的路径、访问被拒绝的路径及根目录之外的路径；T3:工作目录策略结果；T5:生产环境文件系统元数据/规范化。证据 L1128–L1173 [证据 L1128–L1173]
- `wp2_hub_live_reload_preserves_restart_fields` — MIXED (T4+T5); 检查重新加载会更新实时允许规则，但保留需重启才能更改的 hub/工作区/路径字段；T4:hub 重新加载行为；T5:实际配置文件读取。证据 L1507–L1555 [证据 L1507–L1555]
- `wp2_reload_preserves_live_path_policy_when_workspace_changes` — MIXED (T4+T5); 从磁盘重新加载配置，检查实时工作区/路径策略保持不变，而允许规则更新；T4:实时重新加载/状态更新链；T5:实际配置文件读取。证据 L1437–L1504 [证据 L1437–L1504]

### `crates/agentic-gpt/src/runtime/supervisor.rs`
- `doctor_failure_surfaces_redacted_stdout_stderr_and_exit_code` — MIXED (T2+T5); 运行失败脚本，检查退出/输出诊断信息会对秘密信息脱敏；T2:诊断信息脱敏/映射；T5:子进程及捕获的流。证据 L1216–L1246 [证据 L1216–L1246]
- `doctor_spawn_failure_preserves_os_error_kind` — MIXED (T2+T5); 尝试运行不存在的可执行文件，检查映射后的 OS 错误详情；T2:进程启动错误映射；T5:真实 OS 进程启动尝试。证据 L1249–L1269 [证据 L1249–L1269]
- `fake_tunnel_verifies_args_environment_health_and_shutdown` — MIXED (T4+T5); 运行模拟隧道/本地健康检查服务器，检查参数、含秘密信息的环境、就绪状态及终止；T4:监督器生命周期/参数顺序；T5:真实子进程及套接字。证据 L1272–L1338 [证据 L1272–L1338]
- `health_probe_disables_configured_proxy` — MIXED (T3+T5); 即使配置了不可用的代理，对本地健康检查服务器的访问仍成功；T3:客户端绕过代理的行为；T5:本地 TCP 健康检查交互。证据 L1120–L1137 [证据 L1120–L1137]
- `health_url_accepts_only_local_http_endpoints` — MIXED (T3+T5); 读取 URL 文件，接受回环 HTTP，但拒绝 HTTPS/非本地 HTTP；T3:端点验证；T5:实际健康检查文件读取。证据 L1141–L1153 [证据 L1141–L1153]
- `secret_reference_normalizes_trailing_line_endings_and_rejects_controls` — MIXED (T3+T5); 检查秘密文件首尾空白裁剪/控制字符拒绝、空文件失败及明文拒绝；T3:秘密引用/内容验证；T5:实际文件读取。证据 L1181–L1213 [证据 L1181–L1213]

### `crates/agentic-gpt/src/runtime/tunnel_distribution.rs`
- `bounded_local_download_handles_redirect_and_size_limit` — MIXED (T3+T5); 检查本地重定向下载、大小上限及响应体过短时失败；T3:重定向/大小/长度处理；T5:本地 HTTP 交互及目标文件写入。证据 L934–L959 [证据 L934–L959]
- `cache_revalidates_archive_and_repairs_binary` — MIXED (T3+T5); 安装制品后将其损坏，检查修复状态，再重新安装并读取替换文件；T3:缓存有效性/修复决策；T5:实际缓存文件操作。证据 L830–L859 [证据 L830–L859]
- `executable_override_checks_permissions_and_optional_hash` — MIXED (T3+T5); 检查文件模式/哈希，并拒绝不匹配/符号链接；T3:可执行性/哈希策略；T5:真实文件元数据/哈希/符号链接。证据 L770–L791；保留 **REVIEW**：使用 Unix 符号链接 API，但未设置 Unix 条件限制 [证据 L770–L791]
- `manifest_and_platforms_are_pinned` — MIXED (T1+T2); 检查平台映射、清单数量、固定的 URL/版本及摘要；T1:固定清单内容；T2:平台映射。证据 L711–L729 [证据 L711–L729]
- `offline_cache_and_auto_download_false_are_deterministic` — MIXED (T3+T5); 离线解析预填充的缓存，并在禁用下载时检查缓存缺失的结果；T3:缓存/下载策略；T5:实际缓存查找。证据 L864–L899 [证据 L864–L899]

### `crates/agentic-gpt/src/skills/skill_installs.rs`
- `commit_journal_recovery_rejects_paths_outside_skills_root` — MIXED (T3+T5); 拒绝磁盘上位于技能根目录之外的日志路径，并验证该外部文件系统路径仍然存在 [证据 L2167-L2192]
- `commit_journal_recovery_restores_archive_without_destroying_precommit_target` — MIXED (T3+T5); 协调真实日志/归档/目标文件，检查恢复/移除，并确认保留先前的目标 [证据 L2112-L2164]
- `inline_install_is_persisted_and_completes_atomically` — MIXED (T3+T5); 完成安装，检查实际安装的文件、持久化安装记录以及结果/状态行为 [证据 L1948-L1982]
- `live_completion_drain_retries_after_event_store_becomes_writable` — MIXED (T4+T5); T4:串联 install-manager 完成与 EventStore 通知排空。T5:断言依赖真实 SQLite 锁定/回滚/重试及持久化的待处理/事件状态 [证据 L2263-L2341]
- `terminal_install_notification_survives_recovery_until_acknowledged` — MIXED (T3+T5); 写入并重新加载持久化安装记录，检查待处理通知恢复，再检查确认操作会更新记录 [证据 L2194-L2260]

### `crates/agentic-gpt/src/skills/skills.rs`
- `activation_is_idempotent_and_state_file_only_saves_id_and_time` — MIXED (T3+T5); 根据实际活动状态文件内容检查激活语义 [证据 L995-L1024]
- `active_marks_deleted_skill_stale_without_summary_and_deactivate_cleans_it` — MIXED (T3+T5); 激活基于文件的技能，删除其文件，然后检查过期状态及持久化停用 [证据 L961-L992]
- `builtin_installer_is_default_active_and_deactivation_survives_restart` — MIXED (T3+T5); 读取/搜索内置内容，写入活动状态变更，并验证忽略工作区遮蔽 [证据 L1027-L1112]
- `invalid_id_and_missing_skill_return_clear_errors` — MIXED (T3+T5); 针对工作区文件系统检查无效 ID 验证及缺失技能查找 [证据 L1303-L1330]
- `list_reads_only_valid_first_level_skills_sorted_with_active` — MIXED (T3+T5); 读取实际技能文件和活动状态；检查第一级筛选、排序、元数据及内置条目 [证据 L860-L896]
- `read_rejects_resource_escape_directories_and_symlinks` — MIXED (T3+T5); 文件系统遍历/目录/符号链接情形返回路径错误；符号链接设置以 Unix 为条件 [证据 L1159-L1186]
- `read_returns_frontmatter_package_summary_and_warnings` — MIXED (T3+T5); 读取技能文件/包目录，检查解析后的元数据、资源及警告 [证据 L899-L923]
- `read_supports_bounded_utf8_and_base64_package_resources` — MIXED (T3+T5); 读取实际文本/二进制文件，检查编码/内容/大小转换 [证据 L1115-L1156]
- `run_resolution_requires_active_workspace_executable_under_scripts` — MIXED (T3+T5); 针对真实脚本文件及其文件系统权限检查活动状态/路径/可执行性策略 [证据 L1189-L1243]
- `run_waits_for_real_skill_process_and_returns_completed_output` — MIXED (T4+T5); 激活基于文件的技能，启动其真实 shell 脚本，等待完成至终态，并检查已完成状态及解码后的 `skill-output\n` stdout。 [证据 L1245–1300]
- `search_matches_id_frontmatter_tags_and_body_case_insensitively_with_limit` — MIXED (T3+T5); 搜索工作区中创建的文件，检查匹配/数量限制/空白查询行为 [证据 L926-L958]

### `crates/agentic-gpt/src/storage/audit.rs`
- `audit_rotation_keeps_one_bounded_backup` — MIXED (T3+T5); 执行轮转策略，验证磁盘上备份/当前审计文件的内容及大小 [证据 L262-L271]
- `concurrent_audit_appends_remain_complete_json_lines` — MIXED (T3+T5); 并发写入者向审计文件追加内容；检查 128 行完整的 JSON [证据 L274-L297]
- `oversized_audit_record_is_rejected_without_truncating_existing_data` — MIXED (T3+T5); 拒绝超限追加，并验证已有审计字节保持不变 [证据 L250-L259]

### `crates/agentic-gpt/src/storage/event_store.rs`
- `awaiting_completion_survives_recovery_and_source_cannot_be_forged` — MIXED (T3+T5); 检查持久化完成状态的恢复/结算及来源溯源，并检查拒绝伪造来源 [证据 L1745-L1781]
- `completion_order_off_suppression_and_later_reads_preserve_original_settlement` — MIXED (T3+T5); 持久化内部完成/结算决策，检查顺序、抑制行为，以及后续读取不会改变这些决策 [证据 L1679-L1742]
- `concurrent_panels_expose_a_low_event_exactly_once` — MIXED (T3+T5); 通过两个存储句柄并发调用，争用持久化展示状态；检查事件仅展示一次 [证据 L1621-L1643]
- `concurrent_v1_openers_migrate_the_schema_once` — MIXED (T3+T5); 并发打开真实的临时 v1 SQLite 数据库，断言两次打开均成功 [证据 L1854-L1900]
- `duplicate_completion_cannot_reset_or_resurrect_a_terminal_event` — MIXED (T3+T5); 检查重复完成及保留策略清理/重放后的持久化事件状态/消息/严重程度 [证据 L2096-L2160]
- `event_database_rejects_a_different_configured_agent_identity` — MIXED (T3+T5); 检查 SQLite 所有者不匹配时被拒绝，以及原所有者随后可检索 [证据 L2044-L2068]
- `expiry_boundary_and_seven_day_history_retention_are_applied_before_reads` — MIXED (T3+T5); 检查持久化事件过期，以及旧的已处理历史记录被删除/在 list/get 中不存在 [证据 L1461-L1515]
- `list_cursor_is_stable_and_bound_to_agent_scope_and_filters` — MIXED (T3+T5); 对 SQLite 支持的结果分页，并拒绝在不同 agent/severity 筛选条件下复用游标 [证据 L1534-L1571]
- `list_defaults_to_twenty_pending_and_can_filter_handled_history` — MIXED (T3+T5); 针对持久化事件行检查默认分页和状态筛选 [证据 L1574-L1618]
- `list_summaries_count_unicode_scalars_and_preserve_full_message_on_get` — MIXED (T3+T5); 插入并通过列表读取 SQLite 事件；验证 Unicode 摘要截断和完整消息检索 [证据 L1357-L1394]
- `low_one_medium_three_high_unlimited_exposure_and_hidden_counting` — MIXED (T3+T5); 面板读取会更新持久化的显示次数；检查严重程度上限和最终计数 [证据 L1433-L1458]
- `marking_is_idempotent_and_unknown_ids_remain_not_found` — MIXED (T3+T5); 对同一持久化事件标记两次，并检查 handled/not-found 结果 [证据 L1518-L1531]
- `panel_orders_severity_caps_five_and_counts_hidden_pending_events` — MIXED (T3+T5); 检查持久化事件行按严重程度/存续时间的排序和计数，包括一个隐藏行 [证据 L1397-L1430]
- `remote_settlement_requires_matching_origin_and_stays_sticky` — MIXED (T3+T5); 检查持久化的来源/结算规则、不匹配拒绝、决策保持不变，以及重新打开数据库后的状态 [证据 L1903-L2041]
- `reopen_does_not_replace_existing_policy_snapshot_or_completion` — MIXED (T3+T5); 持久化注册/完成信息，并检查后续注册不会替换原始策略快照 [证据 L2071-L2093]
- `reopen_restores_events_and_internal_response_arbitration` — MIXED (T3+T5); 关闭并重新打开 SQLite 存储，然后检查结算后的事件和内部响应状态 [证据 L1646-L1676]
- `unrepresentable_rfc3339_ttl_is_rejected_before_event_insertion` — MIXED (T3+T5); 拒绝不可表示的到期时间，并验证 DB 列表仍为空；还检查可表示的到期时间能往返保持一致 [证据 L1807-L1851]

### `crates/agentic-gpt/src/storage/private_state.rs`
- `identical_target_and_legacy_source_cleanup_is_self_healing` — MIXED (T3+T5); 比较磁盘上的源/目标内容，并检查相同旧版数据的清理 [证据 L534-L551]
- `migration_rejects_symlink_and_falls_back_to_legacy` — MIXED (T3+T5); **`cfg(unix)`**；拒绝实际源目录树中的符号链接，并验证回退路径/警告 [证据 L611-L638]
- `migration_rejects_target_symlink_and_preserves_legacy_source` — MIXED (T3+T5); **`cfg(unix)`**；拒绝磁盘上的目标符号链接，并验证回退数据/外部文件内容 [证据 L581-L608]
- `prepare_is_idempotent_after_successful_migration` — MIXED (T3+T5); 重复执行文件系统迁移/准备，并检查路径稳定且文件字节保持不变 [证据 L515-L531]
- `prepare_migrates_known_legacy_state_and_cleans_empty_root` — MIXED (T3+T5); 将实际旧版状态文件/目录移入私有根目录，并删除旧目录树 [证据 L474-L512]
- `target_conflict_keeps_target_and_retains_legacy_source` — MIXED (T3+T5); 验证实际冲突文件仍然存在，且返回警告 [证据 L554-L578]

### `crates/agentic-gpt/src/storage/process_history.rs`
- `admitted_metadata_reserves_terminal_error_space` — MIXED (T3+T5); 持久化接纳/运行中/终态快照，并验证大型终态错误字段经 DB 检索后保持完整 [证据 L1388-L1428]
- `early_terminal_evidence_survives_full_snapshot_and_restart_recovery` — MIXED (T3+T5); 持久化提前完成的证据、后续快照和重启恢复信息；检查恢复的事件详情 [证据 L1300-L1367]
- `oversized_admission_metadata_rejects_the_entire_batch` — MIXED (T3+T5); 拒绝过大的接纳内容，并通过 SQLite 查询确认两条批次行均未存储 [证据 L1370-L1385]
- `process_list_filters_and_cursor_rejects_malformed_values` — MIXED (T3+T5); 检查带筛选的 SQLite 分页/游标创建，以及对格式错误游标的拒绝 [证据 L1431-L1455]
- `terminal_event_outbox_survives_reopen_until_acknowledged` — MIXED (T3+T5); 检查完成行在重新打开 DB 后仍然存在，并在确认后从待处理状态中移除 [证据 L1267-L1297]
- `terminal_output_and_offsets_survive_reopen_atomically` — MIXED (T3+T5); 在 SQLite 中持久化终态/输出，尝试替换，并检查重新打开后的原始状态/字节/偏移量/详情 [证据 L1226-L1264]

### `crates/agentic-gpt/src/storage/transport_ledger.rs`
- `claim_concurrency_allows_one_started_owner` — MIXED (T3+T5); 并发认领会追加/读取实际账本，并且恰好产生一个所有者 [证据 L923-L968]
- `compaction_collapses_completed_transition_history_and_retains_evidence` — MIXED (T3+T5); 压缩实际 JSONL，验证保留的结果/证据，并读取恢复备份 [证据 L1152-L1278]
- `conflicting_completion_preserves_canonical_result_and_evidence` — MIXED (T3+T5); 检查实际账本内容中的规范结果和冲突证据 [证据 L993-L1040]
- `conflicting_identity_is_rejected_with_evidence` — MIXED (T3+T5); 拒绝冲突身份，并检查账本文件中的不匹配记录 [证据 L1043-L1061]
- `malformed_transport_identity_and_owner_rows_fail_closed` — MIXED (T3+T5); 格式错误的账本行解析失败；检查账本字节未变以及磁盘上的恢复证据 [证据 L1824-L1861]
- `missing_identity_is_rejected_without_fabricating_hash` — MIXED (T3+T5); 拒绝空身份/哈希，并检查未创建账本文件 [证据 L1130-L1149]
- `retired_process_commands_survive_mixed_ledger_recovery_and_compaction` — MIXED (T3+T4+T5); T3:恢复/压缩旧版/当前命令记录，并保留结果。T4:使用 `EventStore` 构建响应。T5:读写账本、恢复备份以及 EventStore 数据库 [证据 L1281-L1489]
- `stored_legacy_argv_commands_keep_identity_results_and_current_recovery` — MIXED (T3+T4+T5); T3:检查旧版/当前命令的恢复、验证和压缩。T4:通过 `EventStore` 形成已完成响应。T5:断言账本/备份状态，并使用 EventStore 数据库 [证据 L1492-L1821]
- `torn_corruption_fails_closed_and_deduplicates_recovery_evidence` — MIXED (T3+T5); 磁盘上损坏的 JSONL 会阻止读取/接受，并创建一个恢复证据文件条目 [证据 L971-L990]
- `unowned_legacy_is_blocked_including_explicit_agent_target` — MIXED (T3+T5); 写入旧版账本记录，拒绝接管，并检查无所有者的 agent 状态保持不变 [证据 L1064-L1127]

### `crates/agentic-gpt/tests/config_cli.rs`
- `bare_non_tty_init_requires_explicit_non_interactive_mode` — MIXED (T4+T5); T4 non-TTY 初始化防护/错误指引；T5 实际 CLI 进程的状态/输出，并断言配置文件不存在。 [证据 L700–734]
- `config_help_is_fully_localized_without_changing_tokens` — MIXED (T2+T5); T2 本地化帮助/token 映射；T5 实际 CLI 子进程的 stdout/状态。 [证据 L14–42]
- `config_keys_http_mcp_section_exposes_editable_contract` — MIXED (T2+T5); T2 HTTP MCP 注册表元数据到 JSON 的映射；T5 实际 CLI 子进程的输出/状态。 [证据 L905–955]
- `config_keys_json_lists_registry` — MIXED (T2+T5); T2 注册表元数据到 JSON 的映射；T5 实际 CLI 子进程的输出/状态。 [证据 L371–428]
- `every_visible_command_has_help` — MIXED (T2+T5); T2 可见命令元数据/帮助描述；T5 所列命令的实际 CLI 子进程输出/状态。 [证据 L44–122]
- `invalid_mode_is_localized_without_changing_valid_tokens` — MIXED (T4+T5); T4 解析器到本地化错误的行为；T5 实际 CLI 退出状态和 stderr。 [证据 L174–200]
- `invalid_owned_parse_errors_keep_stream_and_tokens` — MIXED (T4+T5); T4 三种失败的解析器/错误渲染行为；T5 实际 CLI 退出状态和 stderr。 [证据 L202–249]
- `language_auto_detection_obeys_locale_precedence` — MIXED (T2+T5); T2 locale/环境优先级及显式覆盖；T5 实际 CLI 子进程的输出/状态。 [证据 L140–172]
- `language_flag_is_equivalent_before_and_after_subcommand` — MIXED (T2+T5); T2 argv 标志位置/选择行为；T5 真实 CLI 子进程的输出/状态。 [证据 L124–138]
- `plaintext_tunnel_api_key_is_rejected_without_writing_config` — MIXED (T3+T5); T3 明文密钥拒绝/无泄漏行为；T5 真实 CLI 进程的输出/状态，以及配置文件不存在的断言。 [证据 L797–829]

### `crates/agentic-gpt/tests/standalone_supervisor.rs`
- `supervised_journal_mode_omits_agentic_inner_timestamp` — MIXED (T2+T5); T2:验证转发的子进程日志中的时间戳移除和日志行渲染；T5:使用临时配置/文件、模拟隧道和本地健康检查 socket 运行真实的 supervisor/worker，然后检查 worker 输出和转发的 stderr。 [证据 L53–58; helper L161–246]

## 值得保留的低层级用例

- `crates/agentic-gpt/src/browser/browser_distribution_tests.rs` — `pinned_repository_key_has_exact_fingerprint` (T1)：固定安全信任根指纹；`authenticated_inrelease_metadata_selects_exact_target` (T2)：将经过认证的元数据映射到精确目标；`cleartext_signature_verifies_and_tampering_fails` (T3)：覆盖签名验证和篡改拒绝。
- `crates/agentic-gpt/src/ingress/stdio_server_tests.rs` — `normal_and_room_tool_sets_follow_fixed_surface_contract` (MIXED T1+T4)：保护精确 API surface/schema；`process_read_preserves_raw_byte_offsets_and_utf8_output` (MIXED T2+T4+T5)：保护 byte-offset 和 UTF-8/base64 投影行为。
- `crates/agentic-gpt/src/mcp/mcp_tests.rs::server_config_validation_is_complete_and_typed` (T3)：直接验证有效和无效的 typed MCP config 变体。
- `crates/agentic-gpt/src/storage/event_store.rs::list_summaries_count_unicode_scalars_and_preserve_full_message_on_get` (MIXED T3+T5)：检查 Unicode scalar 摘要逻辑，以及从 SQLite 完整取回 message。
- `crates/agentic-gpt/src/files/file_ops.rs::search_rejects_invalid_patterns_and_enforces_bounds` (T3)：覆盖独立 pattern/bounds 逻辑。
- `crates/agentic-gpt/src/runtime/main_tests.rs::cli_version_uses_crate_version` (T1+T2)：保护固定版本文本和 CLI 渲染行为。

## 明确低价值的 T4/T5 用例

- `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::room_profile_dispatches_room_memory_tools` (MIXED T4+T5)：只断言字段存在且类型正确，未检查 Room 数据或语义行为。
- `crates/agentic-gpt/src/ingress/stdio_server_tests.rs::compact_mcp_skills_and_tmux_adapters_preserve_result_envelopes` (MIXED T4+T5)：成功结果和错误结果都能通过，因此两个 tmux adapter 即使都失败，测试仍可能通过。
- `crates/agentic-gpt/src/runtime/tunnel_distribution.rs::artifact_lock_serializes_concurrent_installers` (T5)：只在 sleep 后释放第一把锁并检查第二次获取最终成功；没有断言第 2 个等待者在第 1 把锁仍被持有时处于阻塞状态，因此不具备串行化的实现也可能通过。
- `crates/agentic-gpt/src/files/file_ops.rs::absent_commit_uses_no_replace_and_overwrite_preserves_permissions` (T5)：直接调用 `hard_link`/`rename`，绕过名称所暗示的生产 commit 路径。
- `crates/agentic-gpt/src/process/managed.rs::shell_eval_preserves_dash_leading_command_text` (T5)：对 `--` 只检查 Failed/127；没有检查输出以证明前导命令文本得到保留。
- `crates/agentic-gpt/tests/config_cli.rs::config_import_compatible_round_trip_preserves_http_mcp_fields` (T5)：预期的 import 在无 TTY 时失败；后续 `show` 读取的是 `init` 生成的源配置，而非导入后的 config。

## 需要复核的用例

- `crates/agentic-gpt/src/runtime/main_tests.rs::read_only_system_file_is_allowed`：读取未设平台门控的 `/proc/meminfo`，依赖平台/环境。
- `crates/agentic-gpt/src/runtime/main_tests.rs::symlink_to_denied_path_is_rejected`：在非 Unix 上，该断言会被编译掉。
- `crates/agentic-gpt/src/runtime/tunnel_distribution.rs::executable_override_checks_permissions_and_optional_hash`：使用 Unix symlink API，但没有 Unix cfg gate。