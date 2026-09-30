# 盘点记录

- 标准位置：`docs/documentation-standard.md:42-60`。
- 每个工具说明用途与选择时机，参数说明条件必填、默认、范围、格式及优先级；工具说明结果、错误与副作用。
- 名称与 Schema 是当前协议合同，不因文案修复而变更。
- CodeGraph 存在，但本次通用 explore 列出旧路径，需以当前磁盘源码核实。
- 开始时工作区干净。

## 官方参考（已实际读取）
- OpenAI：https://developers.openai.com/plugins/plan/tools 。按用户目标描述选择时机，说明输入、结果、权限、副作用及失败；注解不能替代授权。
- Anthropic：https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools 。解释用途、调用/不调用时机、参数含义和限制；详细程度取决于复杂度，不机械套句数。
- MCP：https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/server/tools.mdx 。区分协议错误与工具执行错误、content 与 structuredContent；本次不升级协议版本。
- Context7：已 resolve `/modelcontextprotocol/modelcontextprotocol` 并查询 ToolAnnotations；readOnly/destructive/idempotent/openWorld 均为 hints，destructive/idempotent 仅对非只读工具有意义。

## 验证设计
- 现有 `scripts/check_contract_parity.py` 可启动私有 HOME/XDG 与 loopback 的 Agent/Hub，真实发现和调用工具。
- 修复前后导出工具，递归忽略 description/title 对比合同，核实工具名、Schema 结构、默认未变；注解差异必须精确落在经实现核实的修复名单。
- `find` 查询 scripts 时所有 judge 请求失败，不据此推断文件不存在；已按已知脚本名和字面符号找到验证入口。

## 完整定义来源
- Agent 42：`ingress/stdio_schema.rs` 描述及输入 schema；`operations/operation.rs` 注解。toolsets 显式全部启用时 normal/room 均公开42，正常预设并非必然相同。
- Hub Full49/Coordinator8：`ingress/mcp/mcp_server.rs` 的49个宏与注解，`args.rs` 的 Schemars 参数（含嵌套来源/文件/批次/通知/维护）。
- 其他 crates、Console、experimental 未发现自有静态模型工具定义。动态下游 MCP 工具/官方 Browser SDK 接口不由本项目拥有，排除重写。
- 已验证注解风险：Agent `skills.setActive` 缺于非只读集合；Agent/Hub process.exec/batch、tmux.exec、mcp.callTool/batch 缺于 destructive 集合，尽管可执行破坏性操作。
- 三名工作者分别独占 Agent schema、Hub工具宏、Hub参数文件；父任务负责 Agent注解、集成、验证与用户参考文档。
- 先前两名 scout 的结果未交付且 jobs 消失，已报告工具问题；新的压缩盘点已成功返回，不影响范围完整性。

## 测试维护决策
- 保留 `file_surface_schema_is_exact` 的名称、必填字段、批次数量和结构断言。
- 去掉其两处 `requests.description.contains("mutually exclusive")`：只检测英文措辞，不证明实际拒绝混用。具名保留测试 `file_read_and_search_batches_preserve_order_and_isolate_failures` 直接调用 file.read/search 并验证混用失败；不将英文措辞断言重新固定为中文措辞。
- 新增消费者可见元数据回归测试 `mutating_tool_annotations_do_not_promise_read_only_or_additive_effects`，覆盖激活写入及可能破坏性执行的公开发现信息。
- 保留 `compact_tool_schema_budgets_hold` 原阈值作为发现载荷体积守卫，不视为官方协议/token保证。最终真实Normal总31441/input16885字节，Room总39390/input20168字节，均达标。

## 最终验证
- Agent：558项通过、1项忽略；Hub：92项通过。
- 构建Agent/Hub成功；真实导出Agent全命名空间Normal/Room各42、HubFull49/Coordinator8，忽略文案后Schema/default/required/limits均与修复前一致，注解差异精确符合已核实名单。
- 原 live parity 启动检查只等待 online=true，但命令准入要求 `hello_received=true`（Hub lifecycle.rs）。曾遇 `agent_not_ready`；一次性包装验证等待 `/v1/agents` 的Hello配置概要到达后，原全部gate逻辑通过。不修改生产逻辑/永久验证脚本，不据此添加执行重试。
- 真实私有环境验证 file.edit/model-visible变化清单及缺失父目录创建、file.read内容、browser.list和内置技能发现；未声称运行真实Browser网页操作或生产Tunnel。
- 受影响参考页9条相对链接均有效。
