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
- 修复前后导出工具，递归忽略 description/title 对比合同，核实工具名、Schema 结构、默认与注解未被文案改写。
- `find` 查询 scripts 时所有 judge 请求失败，不据此推断文件不存在；已按已知脚本名和字面符号找到验证入口。

## 完整定义来源
- Agent 42：`ingress/stdio_schema.rs` 描述及输入 schema；`operations/operation.rs` 注解。toolsets 显式全部启用时 normal/room 均公开42，正常预设并非必然相同。
- Hub Full49/Coordinator8：`ingress/mcp/mcp_server.rs` 的49个宏与注解，`args.rs` 的 Schemars 参数（含嵌套来源/文件/批次/通知/维护）。
- 其他 crates、Console、experimental 未发现自有静态模型工具定义。动态下游 MCP 工具/官方 Browser SDK 接口不由本项目拥有，排除重写。
- 已验证注解风险：Agent `skills.setActive` 缺于非只读集合；Agent/Hub process.exec/batch、tmux.exec、mcp.callTool/batch 缺于 destructive 集合，尽管可执行破坏性操作。
- 三名工作者分别独占 Agent schema、Hub工具宏、Hub参数文件；父任务负责 Agent注解、集成、验证与用户参考文档。
- 先前两名 scout 的结果未交付且 jobs 消失，已报告工具问题；新的压缩盘点已成功返回，不影响范围完整性。
