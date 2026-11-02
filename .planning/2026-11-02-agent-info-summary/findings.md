# 记录

- 策略数量直接取已生效配置的完整列表，不使用此前最多 128 条的展示结果。
- workspace、文件搜索限制和 Secret 同步状态按已讨论的字段路径摊平；连接、确认和事件保持原样。
- agent.info 是公开 MCP 返回，仓库内文档和现有调用点需要同步；没有独立的 AgentInfo OpenAPI 类型。
- 删除的 `info_preserves_empty_policy_lists_and_deduplicates_workspace_root` 原先检查路径列表、空列表及工作区尾部斜线处理；列表输出已移除，尾部斜线处理由 `local_runtime_cli_exercises_real_unix_mcp_surface` 的真实调用覆盖。`info_is_bounded_redacted_and_profile_correct` 中旧路径列表和旧搜索字段断言同样已移除，新字段由该真实调用覆盖。Secret 同步测试会继续检查真实同步任务与状态变化。
- 测试保留价值：真实 Unix MCP 测试应覆盖超过旧展示上限的完整规则计数、实际工作区、文件搜索限制与同步状态，防止配置加载、服务和对外返回衔接时出现截断或字段错误；它与现有测试保护的行为不同，属于第⑤档。
- 接口检查中原来用 agent.info 的长响应验证超出预算后延迟读取。改用足够长的实际文件内容，继续检查真实 Hub、MCP、Process 链路和完整取回；避免依赖 agent.info 返回恰好有多长，属于第⑤档。
