# 其他 Rust crate：单一 Tier 测试案例

本清单列出四个已审计 Rust crate 中除 `other-rust-mixed-cases.md` 所列 10 个 `MIXED` 案例以外的全部 163 个单一 Tier 测试。每个案例按源码文件分组，仅出现一次；保留准确的源码路径、函数名、Tier 标签和证据行。

计数按断言实际依赖的行为归类：T5 仅用于断言依赖实际文件、SQLite、socket、进程等资源状态的案例；若断言验证 in-memory SQLite 的实际 SQL 结果/副作用，也计入 T5。仅打开或初始化数据库本身不构成 T5。具有另一项独立 T1–T4 行为的案例另列在混合清单。

## `crates/agentic-gpt-hub` — 133 个单一 Tier 案例

### `crates/agentic-gpt-hub/src/agents/dispatch_tests.rs`

**T2 — 机械处理、映射或转发**
- `pending_replay_sends_reliable_envelope` — 重放待处理命令时发送文本信封，包含预期 run ID、request ID、hash 和 command，且不发送额外消息。**证据：** 108–138。

**T3 — 独立组件逻辑**
- `failed_request_send_removes_current_connection` — 发送失败返回 `agent_offline`，移除该连接，并且该 run 没有待处理 waiter。**证据：** 584–616。
- `reporting_only_connection_is_not_a_command_target` — 仅报告用途的 agent 返回 `agent_reporting_only`，且不创建 run。**证据：** 617–652。

**T4 — 内部多组件行为链**
- `response_owner_isolates_runs_sharing_request_id` — 两个共用 request ID 的调用获得不同 run ID，并分别收到匹配的响应和结果。**证据：** 243–330。
- `response_owner_timeout_preserves_other_run_and_accepts_late_result` — 一个 run 超时不会消耗其同伴的 waiter；随后两个 run 均能接受并保存各自匹配的结果。**证据：** 331–412。
- `returned_decision_write_failure_returns_error_and_next_call_settles_false` — feedback-decision 写入失败时返回错误；下一次公开请求等待结算，并在收到确认后继续。**证据：** 727–865。
- `result_store_failure_preserves_sources_and_settles_before_public_call` — 结果写入失败时保留活动连接且结果仍不存在；下一次公开调用继续前，保留的 source 元数据已完成结算。**证据：** 867–1001。

**T5 — 断言依赖实际资源或副作用**
- `stale_response_with_matching_run_is_accepted` — 旧连接上的响应仍会被对应 run 接受；该 run 保存为已完成状态并带有该结果。**证据：** 139–176。
- `response_owner_rejects_unmatched_without_consuming_waiter` — 不存在的 run、错误 request 和缺失 run 的响应均被拒绝，不会完成 waiter 或保存结果；有效响应随后完成该 waiter。**证据：** 177–242。
- `response_owner_checks_pending_hash_against_durable_run` — waiter 的 hash 与持久化 run 不一致时拒绝响应且保留已存结果；恢复 hash 后响应可被接受。**证据：** 413–464。
- `response_owner_rejects_conflict_and_accepts_duplicate` — 拒绝冲突结果并记录 conflict JSON；接受与规范结果重复的结果。**证据：** 465–523。
- `response_owner_keeps_waiter_on_store_failure` — 触发 SQLite 写入失败后结果仍不存在且 waiter 保留；移除触发器后响应成功保存。**证据：** 524–582。
- `wp1_send_failure_marks_not_sent_and_excludes_replay` — 发送失败后持久化 `not_sent` 及原因 `agent_offline`，并将该 run 排除在重放之外。**证据：** 654–694。
- `wp1_unknown_transport_status_is_mismatch_without_mutation` — 未知 transport status 返回 `transport_status_run_mismatch`，且已存 run 的 status、reason 和 timestamp 均不变。**证据：** 696–725。
- `feedback_intent_insert_failure_rolls_back_creation_run_before_send` — feedback-intent 插入失败时不发送命令，且 SQLite 中既无该 run，也无 intent 行。**证据：** 1003–1053。
- `event_sources_validation_conflict_is_http_409_for_unknown_source` — 接受已知 source；未知 source 返回 HTTP 409 和 `event_sources_validation`；触发 source 更新失败时返回 HTTP 400。**证据：** 1055–1195。

### `crates/agentic-gpt-hub/src/agents/lifecycle_tests.rs`

**T3 — 独立组件逻辑**
- `stale_heartbeat_is_rejected_without_touching_current_connection` — 旧连接的 heartbeat 返回 conflict，当前连接的 ID 和 last-seen 时间保持不变。**证据：** 748–770。
- `stale_process_update_is_rejected_without_writing_process_cache` — 过期 process update 返回 conflict，且 process cache 仍为空。**证据：** 772–791。
- `expired_connection_cleanup_removes_only_stale_current_entries` — 清理操作移除过期连接并保留仍有效的连接。**证据：** 793–805。
- `generation_expiry_rechecks_current_liveness` — 刷新过的连接通过过期检查；真正过期的连接被移除，而已替换的 generation 仍为当前 generation。**证据：** 1283–1344。
- `guarded_send_rejects_captured_target_after_replacement` — 替换连接后，准入检查拒绝已捕获的目标，旧连接和新连接都不会收到消息。**证据：** 1346–1371。
- `generation_stale_heartbeat_direct_handler` — 直接处理旧 generation 的 heartbeat 返回 `stale_connection`，且新连接的 last-seen 时间不变。**证据：** 1373–1423。

**T4 — 内部多组件行为链**
- `room_dispatch_keeps_validated_generation_during_replacement` — 替换期间，按旧 Room generation 验证的请求仍发送给该 generation 并由其应答；新 generation 不会收到重复请求。**证据：** 194–290。
- `room_lease_invalidated_during_feedback_flush_returns_conflict_without_reroute` — Room 请求将待处理 feedback 结算发送给已捕获的 generation；替换并收到确认后，请求返回状态冲突且不改道发送。**证据：** 292–394。
- `wp1_confirmation_callback_does_not_route_to_replacement_generation` — 已替换 generation 的 callback 返回 conflict，通知并关闭旧连接，且不通知新连接。**证据：** 396–451。
- `wp1_confirmation_callback_claims_response_once` — 首次 callback 成功，重复 callback 返回 conflict，且只发送一次 allow 响应。**证据：** 452–505。
- `wp1_confirmation_retirement_does_not_claim_reused_generation` — 旧 generation 的 retirement 不会认领复用 generation 的 confirmation；新 callback 成功，旧 owner 收到 provider-unavailable retirement 并关闭。**证据：** 506–616。
- `changed_boot_generation_marks_only_active_processes_unknown_after_restart` — boot generation 改变后，仅活动 process 被标为 unknown-after-restart；已完成 process 保持不变，agent 的 boot generation 更新。**证据：** 667–746。
- `generation_stale_messages_preserve_current_state` — 旧 generation 的 hello、heartbeat、process update、report 和 confirmation 均返回 `stale_connection`；当前快照不变、不创建 stale report，也不向新连接发送消息。**证据：** 807–936。
- `generation_current_messages_keep_existing_effects` — 当前 generation 的 hello、heartbeat、process update 和 report 在 boot generation 相同或变化时，保留预期的 Room、process、acknowledgement 和 run 效果。**证据：** 937–1100。
- `generation_stale_reliable_messages_do_not_touch_current` — 旧 generation 的 response、transport acknowledgement 和 transport status 不改变当前快照或 outbound channel，同时仍应用对应 run 结果。**证据：** 1102–1183。
- `generation_process_update_and_replace_are_linearized` — process update 与连接替换串行化：更新要么在替换前提交，要么随后因过期而被拒绝。**证据：** 1184–1245。
- `generation_replacement_preserves_new_room_on_old_disconnect` — 断开旧 generation 会保留新 Room 和待处理 confirmation；断开当前 generation 则清除 Room，并将该 confirmation 结算为 provider unavailable。**证据：** 1246–1281。

### `crates/agentic-gpt-hub/src/agents/transport_tests.rs`

**T4 — 内部多组件行为链**
- `generation_sse_rejects_duplicate_and_empty_ids` — 重复 connection ID 返回 conflict 且不改变 generation snapshot；空 ID 返回 bad request 且 snapshot 不变；新 ID 则连接成功。**证据：** 17–108。
- `current_sse_heartbeat_route_sends_ack_and_touches_registry` — 当前 SSE heartbeat 返回成功、发出匹配的 heartbeat acknowledgement，并更新 registry 的 last-seen 时间。**证据：** 109–171。

### `crates/agentic-gpt-hub/src/ingress/http/routes.rs`

**T2 — 机械处理、映射或转发**
- `event_mark_body_requires_an_agent_and_event_ids` — 拒绝缺少任一必需字段的 event-mark 请求体。**证据：** 1091–1101。
- `process_read_keeps_domain_errors_as_http_errors` — 保留 agent 错误代码和消息并映射为 HTTP 状态；成功的 MCP 结果数据原样传递。**证据：** 1478–1539。

**T3 — 独立组件逻辑**
- `offline_event_list_returns_an_error_without_cached_events` — agent 离线时返回 gateway-timeout 错误，且不合成 events 字段。**证据：** 1206–1221。
- `unavailable_process_read_reports_only_stale_cache_metadata` — 报告不可用/过期的进程元数据，同时排除 output/result 字段。**证据：** 1426–1476。

**T4 — 内部多组件行为链**
- `event_http_routes_require_authorization_and_an_enabled_agent` — 检查 event list/get/mark 路由的授权及 agent 可用性，并验证返回预期状态且未分派工作。**证据：** 1103–1204。
- `online_event_domain_errors_keep_the_agent_panel_and_http_status` — 验证在线 agent 响应，并在映射领域错误和状态时保留 event panel。**证据：** 1223–1325。
- `process_read_resolves_reliable_response_and_preserves_contract_shape` — 分派 process-read 命令、接收 agent 响应，并返回符合契约形状的数据。**证据：** 1327–1424。

**T5 — 断言依赖实际资源或副作用**
- `process_http_routes_reject_removed_read_paths` — 通过已绑定的本地 TCP 监听器发送 HTTP 请求，并断言已移除的 process-read 路径返回授权/未找到响应。**证据：** 1541–1572。

### `crates/agentic-gpt-hub/src/ingress/mcp/args.rs`

**T2 — 机械处理、映射或转发**
- `process_exec_arguments_accept_command_cwd_and_reject_legacy_fields` — 将当前 command/cwd 字段反序列化为执行及批处理参数，并拒绝旧版 program/args 字段。**证据：** 1046–1080。

### `crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs`

**T2 — 机械处理、映射或转发**
- `tool_read_only_hints_match_side_effect_semantics` — 断言只读工具带有只读提示，而有副作用的工具没有该提示。**证据：** 2144–2197。
- `coordinator_profile_exposes_only_native_tools` — 断言 coordinator 配置公开的工具名称集合完全符合预期。**证据：** 2199–2222。
- `full_profile_keeps_bootstrap_aliases_and_execution_surface` — 检查公开的 bootstrap 别名、process/room 工具、process 数量及已移除的工具名称。**证据：** 2283–2328。
- `mcp_batch_descriptor_freezes_bounds_and_side_effect_annotations` — 检查 batch 描述符的注解、schema 边界及必需字段。**证据：** 2330–2365。
- `skill_install_and_run_tools_are_exposed_with_stable_annotations` — 检查工具公开情况，以及 install/bootstrap 描述符的注解值。**证据：** 2367–2407。
- `bootstrap_timeout_values_preserve_operation_specific_codes` — 将 bootstrap 路由错误转换为工具结果时保留不同操作对应的超时代码。**证据：** 2455–2469。
- `tmux_paste_schema_exposes_confirmation_default_field` — 检查序列化后的 paste schema 包含 confirmation 和 submit 字段。**证据：** 2471–2477。
- `tmux_exec_schema_exposes_snapshot_fields` — 检查序列化后的 exec schema 包含 wait 和 capture 字段。**证据：** 2479–2484。
- `room_mcp_input_schemas_do_not_include_agent_id` — 检查所列 room/skill 参数 schema 不包含两种 agent-ID 拼写。**证据：** 2486–2511。
- `native_tool_values_use_agentic_result_shape` — 将值映射为预期的 structured-content、成功状态和 text-content 结果形状。**证据：** 2513–2523。
- `process_read_arg_schema_matches_contract_and_defaults` — 检查 process-read schema/描述符，并验证默认值、有界等待、view、cursor 和字节上限的载荷映射。**证据：** 2686–2750。

**T3 — 独立组件逻辑**
- `offline_native_process_cache_tools_omit_events` — 确认离线 process list/status 结果不会合成 event panel。**证据：** 2118–2142。

**T4 — 内部多组件行为链**
- `event_tools_forward_only_explicit_agent_responses_and_keep_error_panels` — 验证 event list/get/mark 的转发，保留返回的 panels 和错误；缺少目标时拒绝调用且不向任一 agent 分派。**证据：** 1928–2018。
- `native_cache_status_adds_one_panel_and_preserves_error_when_panel_fails` — 验证 process-status 成功及 panel 响应格式错误时的处理，保留 process 错误且避免额外 panel 请求。**证据：** 2020–2116。
- `coordinator_hides_event_tools_before_target_dispatch` — coordinator 配置下拒绝 event 工具，且不向目标分派。**证据：** 2265–2281。
- `every_advertised_tool_is_accepted_by_apps_dispatcher` — 验证从描述符到 dispatcher 的调用链，确保每个公开工具均被识别而不被判为未知工具。**证据：** 2409–2423。
- `apps_bootstrap_tools_are_callable_through_tools_call` — 通过 JSON-RPC 路由 bootstrap 调用，并检查其结构化路由错误结果。**证据：** 2425–2453。
- `mcp_tools_call_wire_response_uses_agentic_result_shape` — 检查 JSON-RPC handler/dispatcher 响应的 ID 和 agentic 结果形状。**证据：** 2525–2549。
- `process_read_mcp_dispatches_process_read_and_returns_unified_response` — 验证 MCP 分派、可靠命令封装/参数、agent 响应处理及统一工具输出。**证据：** 2551–2623。
- `offline_process_read_is_unavailable_and_metadata_only` — 对离线进程报告过期缓存元数据，同时排除 output/result 字段。**证据：** 2625–2684。

**T5 — 断言依赖实际资源或副作用**
- `coordinator_rejects_hidden_execution_tools_before_dispatch` — 拒绝隐藏工具，并断言 SQLite 对 `agent_runs` 的查询结果为预期数量。**证据：** 2224–2263。

### `crates/agentic-gpt-hub/src/notifications/notify.rs`

**T2 — 机械处理、映射或转发**
- `parses_notify_channel_keys` — 将受支持的 channel-key 字符串映射为对应变体，并拒绝格式错误或不支持的键。**证据：** 562–580。

**T3 — 独立组件逻辑**
- `notification_channels_include_agent_ntfy_and_android_placeholders` — 构建包含 agent 别名、ntfy channel 和不可用 Android 占位项的 channel 列表。**证据：** 582–641。
- `ntfy_default_placeholder_is_not_configured` — 因未配置而将默认 ntfy channel 报告为不可用。**证据：** 643–654。
- `ntfy_health_cache_controls_listing_reason` — 将缓存的健康状态不佳或检查失败映射为相应的不可用原因。**证据：** 656–684。
- `android_registered_endpoint_still_reports_delivery_not_implemented` — 将已注册的 Android channel 报告为不可用，并返回匹配的发送错误。**证据：** 686–726。

**T4 — 内部多组件行为链**
- `user_notify_send_routes_agent_channel_by_alias` — 解析别名 channel、分派送达命令、处理 agent 响应，并检查已完成的 run 结果。**证据：** 728–838。

### `crates/agentic-gpt-hub/src/room/room.rs`

**T2 — 机械处理、映射或转发**
- `bootstrap_error_codes_map_to_frozen_http_statuses` — 将 bootstrap/guide 错误代码映射为预期的 HTTP 状态。**证据：** 106–136。

**T3 — 独立组件逻辑**
- `first_room_agent_becomes_active_room` — 将首个 room agent 注册为活动 room。**证据：** 156–166。
- `second_different_room_agent_is_rejected` — 拒绝第二个不同的活动 room agent。**证据：** 168–180。
- `same_room_agent_reconnect_replaces_old_room_connection` — 替换同一 agent 的 room 连接，并激活新连接。**证据：** 182–196。
- `same_agent_normal_hello_releases_old_active_room` — 该 agent 使用普通角色重新连接时，清除旧的活动 room。**证据：** 198–210。
- `same_agent_replacement_without_hello_does_not_leave_stale_active_room` — 确认未发送 hello 的连接替换不会留下过期的活动 room 状态。**证据：** 212–223。
- `room_api_after_replacement_without_hello_returns_not_active` — 未发送 hello 的连接替换后，拒绝 room API 请求。**证据：** 225–245。
- `stale_room_disconnect_does_not_release_new_room_connection` — 旧连接断开时仍保留新的活动连接。**证据：** 247–261。
- `room_api_without_active_room_returns_not_active` — 没有活动 room 连接时返回 not-active。**证据：** 263–278。
- `read_and_maintenance_room_api_without_active_room_returns_not_active` — 没有活动连接时，read 和 maintenance 请求均返回 not-active。**证据：** 280–305。
- `normal_agent_is_not_room_api_fallback` — 仅连接普通 agent 时拒绝 room API 请求。**证据：** 394–410。

**T4 — 内部多组件行为链**
- `room_api_routes_to_active_room_connection` — 将已授权的 room read 路由至活动连接、接收 agent 响应，并检查返回结果及已完成的 run。**证据：** 307–392。

### `crates/agentic-gpt-hub/src/runtime/cli.rs`

**T2 — 机械处理、映射或转发**
- `cli_version_uses_crate_version` — `--version` 返回 Clap 的 `DisplayVersion` 响应；渲染文本包含 crate 名称/版本和 `CARGO_PKG_VERSION`。**证据：** 50–60。

### `crates/agentic-gpt-hub/src/runtime/config.rs`

**T1 — 固定内容**
- `safe_default_summary_has_no_paths_or_secrets` — 固定回退摘要将 workspace/sandbox 值设为 `unknown`，且路径策略和策略规则列表为空。**证据：** 199–212。

### `crates/agentic-gpt-hub/src/runtime/instance_lock.rs`

**T5 — 断言依赖实际资源或副作用**
- `rejects_a_second_lock_and_releases_on_drop` — 首次获取锁成功，第二次获取报告 “already running”；释放首个锁后可以再次获取。此测试使用真实的临时目录锁资源。**证据：** 71–86。

### `crates/agentic-gpt-hub/src/runtime/main_tests.rs`

**T2 — 机械处理、映射或转发**
- `parses_bearer_case_insensitively` — 接受带空白的、小写 bearer 前缀，并拒绝 Basic authorization 值。**证据：** 75–79。
- `skills_commands_have_run_types` — 将十种 Skills 命令变体映射到各自准确的 wire command type 字符串。**证据：** 96–183。
- `bootstrap_commands_have_run_types` — 将 room bootstrap 和 bootstrap-read 变体分别映射到 `room.bootstrap` 和 `room.bootstrap.read`。**证据：** 185–201。

**T3 — 独立组件逻辑**
- `ntfy_mcp_confirmation_stays_within_three_action_limit` — 对 MCP 工具生成的确认操作恰为预期的三项：允许一次、允许 MCP 30 分钟、拒绝。**证据：** 81–94。

### `crates/agentic-gpt-hub/src/runtime/state.rs`

**T3 — 独立组件逻辑**
- `process_cache_age_generation_and_ordering_are_truthful` — 条目达到配置时限后变为 stale，同时保留更新时间；重启会将其标记为 unknown，而相同 generation 的记录不能覆盖重启状态。**证据：** 497–550。
- `process_cache_capacity_is_bounded` — 记录比配置容量多一个 process 后，缓存条目数仍等于配置容量。**证据：** 552–572。

### `crates/agentic-gpt-hub/src/storage/db.rs`

**T5 — 断言依赖实际资源或副作用**
- `agent_alias_is_nullable_and_unique_when_present` — SQLite 接受多个 null alias，但拒绝将第二个 agent 更新为已被占用的 alias。**证据：** 359–391。
- `migration_sets_explicit_schema_version` — 数据库初始化会写入当前 SQLite `user_version`。**证据：** 393–400。
- `migration_renames_legacy_run_process_fields_without_losing_data` — 迁移将旧 run 值保留在 process 列中，并移除旧 job 列。**证据：** 402–434。
- `newer_schema_version_is_rejected_without_changes` — 初始化拒绝更高版本的 schema，且 `user_version` 保持不变。**证据：** 436–446。
- `failed_legacy_migration_rolls_back_schema_changes` — 重复 alias 导致迁移失败；SQLite 中没有留下部分创建的表，schema 版本仍为零。**证据：** 448–479。

### `crates/agentic-gpt-hub/src/storage/event_feedback.rs`

**T3 — 独立组件逻辑**
- `queued_recovery_subsets_preserve_all_known_identities` — 将较小的 recovery 子集加入队列，不会丢弃此前排队的 identities，也不会将排队的修复标记为冲突；此测试覆盖内存中的 coordinator。**证据：** 1936–1955。
- `public_flush_waits_for_active_agent_barrier` — 持有 agent barrier 时 flush 仍等待；释放 barrier 后 flush 完成。断言针对 barrier 协调，而非数据库结果。**证据：** 2541–2559。

**T5 — 断言依赖实际资源或副作用**
- `returned_terminal_result_forwards_suppression_disposition` — 持久化 `RETURNED` 决策，以及带有包含 terminal 的 source disposition 的待处理 feedback payload。**证据：** 1547–1559。
- `returned_active_result_has_pending_feedback` — 持久化一项待处理 feedback，保留原始 event origin 和 active-source disposition。**证据：** 1561–1572。
- `timeout_before_late_metadata_forces_no_terminal` — timeout 完成后，迟到的 terminal metadata 仍表示为 non-terminal disposition，存储的决策为 `NO_TERMINAL`。**证据：** 1574–1591。
- `concurrent_waiter_outcomes_make_one_sticky_decision` — 并发的 returned/no-terminal finalizer 最终确定一个存储决策；之后相反的 outcome 不会改写该决策或待处理 payload。**证据：** 1593–1626。
- `migration_adds_identity_column_to_existing_feedback_table` — SQLite 迁移为现有 feedback 表添加 `sources_json`。**证据：** 1628–1663。
- `restart_converts_awaiting_intent_to_no_terminal` — 关闭并重新打开 file-backed SQLite 数据库后，restart recovery 将 awaiting intent 改为 `NO_TERMINAL`，并产生待处理的 false-terminal disposition。**证据：** 1665–1725。
- `metadata_identity_conflicts_are_rejected_without_changing_intent` — 拒绝不匹配的 request ID、hash、agent 或冲突 metadata；持久化决策仍为 awaiting，且没有待处理条目。**证据：** 1727–1767。
- `returned_dispositions_must_match_persisted_reply_metadata` — 不匹配的 returned dispositions 会失败且不会完成决策；匹配的 dispositions 则将存储决策完成为 `RETURNED`。**证据：** 1768–1780。
- `late_returned_outcome_cannot_rewrite_final_no_terminal_payload` — 决策持久化为 `NO_TERMINAL` 后，迟到的 returned metadata 不能更改决策，也不能改写其 false-terminal 待处理 payload。**证据：** 1782–1800。
- `ack_and_reflush_keep_a_stable_idempotent_tombstone` — 多次读取时待处理 feedback 保持稳定；修改后的 payload/request IDs 会被拒绝，有效 ack 可幂等处理，之后的 finalization/metadata 不会重新创建已确认条目。**证据：** 1802–1865。
- `identity_recovery_during_wait_does_not_override_returned_flags` — recovery identities 在没有 reply metadata 时被存储；随后 returned metadata 将决策完成为 `RETURNED`，持久化的待处理 disposition 保留 terminal 标志。**证据：** 1867–1898。
- `recovery_subset_preserves_complete_identity_set_and_outbox` — 记录已知子集时保留完整的持久化 identity 集合和待处理 payload；未知 identity 会被拒绝且 outbox 不变。**证据：** 1900–1934。
- `late_complete_reply_metadata_adds_an_immutable_delta_for_batch_child` — 迟到的完整 metadata 不改变主 feedback payload，而是为新识别的 batch child 添加独立的 false-terminal delta，并使用不同 request ID。**证据：** 1957–2018。
- `recovery_delta_after_acked_primary_is_stable_and_not_reissued_after_ack` — primary ack 后，新恢复的 child 产生 delta；无效 ack 被拒绝，有效 ack 被持久化，重复 recovery 不会再次发出该 delta。**证据：** 2019–2077。
- `restart_repairs_missing_delta_coverage_without_rewriting_primary` — 重新打开 file-backed 数据库会生成缺失的 child delta，同时保留 primary payload；重复 recovery 不会改变待处理 outbox。**证据：** 2079–2155。
- `no_terminal_recovery_sources_materialize_false_dispositions` — 对 `NO_TERMINAL` 决策执行 source recovery 后，会持久化 `includes_terminal = false` 的待处理 disposition。**证据：** 2157–2173。
- `late_metadata_preserves_identity_only_outbox_order_and_request_id` — 恢复的 identities 按 canonical 顺序发出；后续完整 metadata 不会更改已持久化的待处理条目。**证据：** 2175–2209。
- `orphan_repair_only_decides_missing_replayable_creation_runs` — repair 仅为符合条件的、缺失的 creation-run intents 持久化 `NO_TERMINAL`，以 non-terminal 形式发出恢复的 sources；已有、已完成及非 creation runs 不会获得新决策。**证据：** 2211–2298。
- `failed_metadata_write_is_retained_and_retried_without_losing_flags` — SQLite trigger 使 metadata 持久化失败；移除 trigger 后重试会保留排队 metadata，最终 returned payload 也保留 source 标志。**证据：** 2333–2371。
- `returned_commit_failure_is_repaired_as_no_terminal` — SQLite trigger 拒绝 returned 决策；owner-drop repair 后，存储决策为 `NO_TERMINAL`，待处理 source 为 non-terminal。**证据：** 2373–2413。
- `non_creation_and_source_less_runs_do_not_create_feedback_commands` — SQLite-backed preparation 拒绝 `process.read` run；完成没有 source 的 creation run 后，不会留下待处理 feedback command。**证据：** 2561–2592。

### `crates/agentic-gpt-hub/src/storage/runs.rs`

**T5 — 断言依赖实际资源或副作用**
- `pending_unacked_preserves_unified_reads_until_acknowledged` — 已准备的 unified `ProcessRead` 出现在持久化的 unacked reads 中；收到与其 run、request 和 hash 匹配的 ack 后，该条目消失。**证据：** 891–921。
- `pending_unacked_retires_legacy_reads_without_blocking_supported_commands` — legacy `process.status`/`output`/`result` rows 从待处理 reads 退役，并带有预期的存储原因且无 ack/result 数据；受支持的 `Exec` 仍待处理，先前完成的 legacy result 也得到保留。**证据：** 922–1113。
- `stores_late_result_idempotently_by_run_id_and_request_id` — 为已 ack 的 run 存储迟到结果一次；重复提交报告 duplicate，已完成结果也被持久化。**证据：** 1425–1452。
- `completed_result_survives_dispatch_and_wait_timeout_updates` — dispatch 和 timeout 更新不覆盖已完成结果；未完成 run 仍转为预期的 timeout 状态/原因。**证据：** 1453–1513。
- `wp1_transport_status_preserves_completed_result` — 迟到的失败 transport status 不改变已完成 run 的 status、result、reason 或更新时间。**证据：** 1514–1551。
- `wp1_matching_stale_status_is_idempotent_and_foreign_is_rejected` — 匹配的 stale status 不改变持久化 status/reason/time；其他 agent 的更新被拒绝。**证据：** 1552–1594。
- `wp1_remote_progress_survives_dispatch_timeout_and_late_ack` — 持久化的 remote progress 在 dispatch/timeout/迟到 ack 后仍保留；timeout 后的迟到 started/running 更新被接受，running 不会倒退，已 ack 的 run 不被 dispatch/timeout 覆盖。**证据：** 1596–1743。
- `wp1_failed_and_unknown_runs_accept_only_late_results` — failed 和 unknown runs 的迟到结果被存储，并使持久化 runs 转换为 completed。**证据：** 1745–1777。
- `wp1_agent_report_identity_and_terminal_result_are_preserved` — 不匹配的 request identity 被拒绝；迟到的 failed report 不能覆盖 completed report 的 identity、result、reason 或 timestamp。**证据：** 1779–1833。
- `stale_acked_runs_become_unknown` — stale acked-run 清理将 SQLite 状态更新为 `unknown`，原因设为 `acked_result_timeout`。**证据：** 1835–1849。
- `agent_reports_upsert_idempotently_and_keeps_full_bounded_detail` — 重复、最新及 stale report upsert 均保留 completed run 的 status、source/detail、有界 arguments/result、process ID 和 process data。**证据：** 1850–1961。

### `crates/agentic-gpt-hub/src/support/agentic_result.rs`

**T2 — 机械处理、映射或转发**
- `wraps_native_json_as_structured_success_when_value_has_no_error` — 将 native JSON 映射到 `structuredContent`，标记为非错误，并生成 text content。**证据：** 54–64。
- `wraps_native_json_as_structured_error_when_value_has_error` — 将 native JSON 保留为 `structuredContent`，并在存在 error 字段时设置 tool-result error 标志。**证据：** 66–75。

### Hub 单一 Tier 小计

| Tier | T1 | T2 | T3 | T4 | T5 | 单一 Tier 合计 |
|---|---:|---:|---:|---:|---:|---:|
| Hub | 1 | 23 | 30 | 30 | 49 | 133 |

## `crates/agentic-gpt-protocol` — 19 个单一 Tier 案例

### `crates/agentic-gpt-protocol/src/lib.rs`

**T2 — 机械处理、映射或转发**
- `diary_v2_shapes_keep_periods_semantic_and_missing_layers_explicit` — 对 diary 请求进行反序列化和重新序列化，并验证序列化后的响应层、显式缺失层元数据及往返后的结构。**证据：** 45–94。
- `notebook_and_state_v2_shapes_bound_previews_and_read_markdown_exactly` — 检查 notebook 预览的序列化与往返、read 请求中 Markdown 的精确保留，以及 state 实体序列化和请求结构。**证据：** 96–151。
- `paste_and_close_default_to_confirmation` — 将省略确认字段的 paste 和 close 请求反序列化，验证两者默认要求确认；paste 提交默认值为 false。**证据：** 269–281。
- `tmux_exec_defaults_to_structured_non_forced_confirmation_request` — 反序列化结构化 program/args，并检查请求的确认、等待和捕获上限默认值。**证据：** 283–296。
- `skills_command_serde_names_are_public_interface_names` — 检查 SkillsRead 命令封套名称，以及不存在的 `ActiveSkill.summary` 字段会被省略。**证据：** 298–321。
- `skill_read_path_is_additive_and_install_source_is_discriminated` — 接受不含 `path` 的旧版 skill-read 输入，检查 GitHub 安装来源判别标记的序列化，并验证可选资源兼容性。**证据：** 323–363。
- `bootstrap_commands_and_enums_use_public_spellings` — 检查 bootstrap 命令和枚举的公开拼写、请求 ID、负载及请求 ID 访问器。**证据：** 428–481。
- `current_room_commands_use_nested_payloads_and_public_names` — 序列化 current-room 命令，并检查公开类型名称、请求 ID 及嵌套负载。**证据：** 483–578。
- `bootstrap_resource_omits_only_absent_truncation_line` — 检查资源元数据序列化、省略不存在的截断行，以及存在该行时的反序列化。**证据：** 580–622。
- `bootstrap_response_and_read_request_round_trip_with_camel_case_fields` — 检查嵌套 bootstrap 响应的 camel-case 字段及往返结果，并检查 bootstrap read-request 的 ID 映射。**证据：** 624–689。

**T3 — 独立组件逻辑**
- `skill_wait_seconds_are_bounded_without_overflow` — 验证 install/run 等待时间在零值和上限处的有效行为，并确认超限值及 `u64::MAX` 均被钳制为 30 秒。**证据：** 405–426。
- `managed_mcp_call_defaults_and_bounds_are_frozen` — 检查 MCP 调用有效等待/超时的默认值、上限及下限行为。**证据：** 691–726。
- `hello_defaults_to_command_capable_when_generation_is_present` — 反序列化包含 boot generation 的 hello，并验证省略的 mode 默认设为 CommandCapable。**证据：** 777–810。
- `hello_without_boot_generation_is_rejected` — 验证缺少 `bootGeneration` 时 hello 反序列化会被拒绝，并指出缺失字段。**证据：** 812–826。

### `crates/agentic-gpt-protocol/src/envelopes.rs`

**T2 — 机械处理、映射或转发**
- `event_hub_commands_keep_request_identity_and_agent_scope` — 检查事件命令名称和请求身份、带作用域的负载字段、响应来源的序列化与默认值，以及 event-sources 类型。**证据：** 406–524。
- `mcp_list_servers_panel_suppression_is_internal_and_legacy_compatible` — 验证 false 面板抑制标记会被省略、旧版输入默认值为 false，且 true 抑制标记会被序列化。**证据：** 526–548。

### `crates/agentic-gpt-protocol/src/process.rs`

**T2 — 机械处理、映射或转发**
- `unified_process_response_has_compact_identity_and_observation_fields` — 检查精简的进程响应身份、状态、捕获和退出码字段，以及六个旧版字段不存在。**证据：** 557–594。
- `process_exec_wire_uses_command_and_cwd_and_rejects_legacy_argv_fields` — 检查 command/cwd 请求的反序列化与序列化；拒绝旧版 argv、working-directory 和 batch 元素字段结构。**证据：** 596–632。

**T3 — 独立组件逻辑**
- `process_read_rejects_explicit_budgets_outside_shared_limits` — 检查默认预算选择、最小值/默认值/最大值处的接受行为，以及低于或高于共享限制时的拒绝行为。**证据：** 529–555。

### Protocol 单一 Tier 小计

| Tier | T1 | T2 | T3 | T4 | T5 | 单一 Tier 合计 |
|---|---:|---:|---:|---:|---:|---:|
| Protocol | 0 | 14 | 5 | 0 | 0 | 19 |

## `crates/agentic-apply-patch` — 1 个单一 Tier 案例

### `crates/agentic-apply-patch/src/lib.rs`

**T2 — 机械处理、映射或转发**
- `parses_patch` — 解析新增、删除和更新/移动指令，并断言解析出的 patch 包含三个 hunk；不检查各 hunk 的具体内容。**证据：** 31–35。

### Apply-patch 单一 Tier 小计

| Tier | T1 | T2 | T3 | T4 | T5 | 单一 Tier 合计 |
|---|---:|---:|---:|---:|---:|---:|
| Apply-patch | 0 | 1 | 0 | 0 | 0 | 1 |

## `crates/agentic-browser-host` — 10 个单一 Tier 案例

### `crates/agentic-browser-host/src/lib.rs`

**T2 — 机械处理、映射或转发**
- `native_frame_round_trip_uses_uint32_prefix` — 将 JSON 消息写入内存字节向量，断言原生字节序 `u32` 长度前缀和序列化载荷正确，随后读回并断言相等。**证据：** 603–617。
- `oversized_frame_is_rejected_before_reading_payload` — 提供超大长度前缀，并断言 `read_frame` 返回包含该长度的 `FrameError::TooLarge`。**证据：** 619–626。
- `truncated_frames_match_baseline_eof_behavior` — 断言空输入、部分长度前缀，以及短于声明长度的载荷都会产生 `Ok(None)`。**证据：** 628–642。
- `zero_malformed_and_non_utf8_payloads_are_errors` — 断言零长度、格式错误的 JSON 和非 UTF-8 分帧载荷都会产生 `FrameError::Json`。**证据：** 644–657。

**T3 — 独立组件逻辑**
- `bridge_status_is_local_and_reports_current_clients` — 注册两个客户端，通过其中一个客户端的 handler 请求 `bridge.getStatus`，断言响应包含 socket 路径和当前客户端数量，并验证扩展端未收到消息。**证据：** 762–788。

**T4 — 内部多组件行为链**
- `dropping_client_removes_its_pending_routes` — 注册客户端，将请求转发到内存扩展 writer 后丢弃客户端；断言客户端和待处理路由注册表均为空，再投递过期响应并验证它未写入客户端。**证据：** 665–689。
- `identifier_free_notifications_forward_and_broadcast` — 注册两个内存客户端后，将客户端通知转发给扩展，并将扩展通知广播给两个客户端。**证据：** 691–712。
- `extension_requests_receive_baseline_responses` — 向扩展消息 handler 提供六种方法，并断言内存扩展 writer 收到预期的基线成功、否定、hello 元数据或 method-not-found 响应。**证据：** 714–760。
- `rewritten_request_ids_restore_to_the_originating_clients` — 从两个客户端发送请求，断言客户端专属的重写 bridge ID 和长度；按相反顺序投递响应，并验证各自原始 ID/结果到达正确的内存客户端。**证据：** 790–830。
- `standalone_compat_hides_agent_request_header_capability_for_get_info` — 通过启用兼容配置的 host 转发 `getInfo`，提供 `agentRequestHeaderEnabled: false` 的内存扩展响应，并断言客户端收到的结果省略该字段。**证据：** 832–862。

### Browser-host 单一 Tier 小计

| Tier | T1 | T2 | T3 | T4 | T5 | 单一 Tier 合计 |
|---|---:|---:|---:|---:|---:|---:|
| Browser-host | 0 | 4 | 1 | 5 | 0 | 10 |

## 四个 crate 的数量核对

以下单一 Tier 数量与 `other-rust-mixed-cases.md` 的混合清单互斥；并集按唯一函数计数。

| 范围 | T1 | T2 | T3 | T4 | T5 | 单一 Tier 数 | MIXED | 合计 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Hub | 1 | 23 | 30 | 30 | 49 | 133 | 5 | 138 |
| Protocol | 0 | 14 | 5 | 0 | 0 | 19 | 4 | 23 |
| Apply-patch | 0 | 1 | 0 | 0 | 0 | 1 | 1 | 2 |
| Browser-host | 0 | 4 | 1 | 5 | 0 | 10 | 0 | 10 |
| **总计** | **1** | **42** | **36** | **35** | **49** | **163** | **10** | **173** |

**核对结果：** 本文件 163 个单一 Tier 案例 + 混合清单 10 个案例 = 四个 crate 共 173 个唯一案例。Hub 按逐用例实际 SQL/资源断言重算为 1/23/30/30/49（T1/T2/T3/T4/T5）；此前 planning 记录的 Hub 拆分已被此逐用例结果取代。