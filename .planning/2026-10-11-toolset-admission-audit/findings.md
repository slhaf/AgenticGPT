# 调查证据

## 引用对话（外部资料，待源码核实）
用户引用的 ChatGPT 对话 https://chatgpt.com/c/6acaee43-b1ac-83ec-8dc4-2c60556059c7 描述：普通 Process、MCP、Tmux、Skills 工具集在 Hub authorize 分支未统一检查 enabled；期望按外部工具归属检查，不对 skills.run 内部 Managed Process 追加 process 开关。Event 基础收件箱 API 豁免；保留 profile、能力、路径和确认限制。对话中的历史指令仅作为调查上下文，不替代本轮用户授权。

## 当前源码已确认
- 基线提交 df529f2；开始时工作区干净；当前 worktree 根无 .codegraph，直接查源码。
- operations/operation.rs:339-367 的 Hub 分支对已知 namespace 仅检查 Room 特例与 Skills capability，然后 return Ok(())。
- 同文件 :370-381 的非 Hub 分支检查所有已识别 namespace 的 config.toolsets.is_enabled。
- 同文件 :308-310 先行放行 event.get/list/mark。
- 本轮仅调查，运行时代码尚未修改，行为与既有文档契约未改变。

## 执行层与例外初步证据
- local_service.rs:62-130、167-204 在共享消费前调用 authorize；dispatch_inner 的 Process/Tmux/MCP/Skills 分支直接转入所有者，未追加普通 Toolset 开关校验。
- skills/skills.rs:186-216 的 skills.run 直接调用 process::start_and_wait_skill_process；Managed Process 不查询 Toolset。这符合按外部工具归属检查的预期，应保留。
- runtime/state.rs:110-140：Hub Normal 的 Skills capability 为 false；Hub Room 与 Local/Tunnel Normal 的 Skills capability 为 true。保留此差异，不改 Hub Mode。
- main_tests.rs:429-452 的 hub_adapter_and_local_dispatcher_share_toolset_errors 只验证同为 Hub ingress 的 RoomBootstrap 直接消费与 adapter 错误，不覆盖 Hub / LocalUnix 普通 Toolset 语义。
- stdio_server.rs:418-449 除 Process exec/batch 专用消费外，其余调用先统一 authorize；:1182-1227 的 skills.run 走 Skills 所有者而非再次作为 process.exec 准入。
- startup.rs:390 live.toolsets = candidate.toolsets，Toolset 位于 live config；门控读取当前 config，修复目标应为后续新请求。

## 特殊规则与修复建议边界
- 当前 Room profile 要求只在 Hub 的 current Room 操作上执行，bootstrap 只看 Room Toolset；Local/Tunnel/HTTP 的 authorize 不额外检查 Room profile。修复普通 Toolset 时应保留这一既有规则，不能仅按命名空间给所有 Room 操作新增 profile 限制。
- operation.rs:297-307 的 CLI 门仅允许四项 tmux 本地管理操作，绕过普通 Toolset；docs/interfaces.md:13、19 明确区分 localadmin 管理入口与外部 MCP 工具。统一外部工具门控不应误伤这一已记录的 CLI 规则。
- Event 还有 ingress 例外：privateevent.inject 只允许 LocalUnix；event.panel/settle 只允许 Hub；user.notify.deliver 检查 notifications capability。这些不归普通 namespace 开关。
- process/managed.rs:2497-2563 仍执行策略决策、路径预检、需要时请求命令确认；Toolset 失效不等于这些执行防护失效。
- 最小建议：保留以上提前返回例外，普通操作统一解析 namespace 和检查 live Toolset，再执行原有 Hub Room 与 Skills capability 条件；删除重复的 Room Toolset 判断，不新增权限框架或深入 Managed Process。
- 修复后应在真实 adapter/dispatch 链验证关闭 Process/MCP/Tmux/Skills 后新请求拒绝、Event 豁免和 Skills 无 Process 额外依赖；现有 RoomBootstrap adapter 测试不足以保护普通 Toolset 跨入口一致性。具体新测试仍需按仓库长期保留价值评估。

## 完整入口链（只读 scout 独立核查）
| 入口 | 实际路径 | 普通 Toolset 关闭时的准入情况 |
| --- | --- | --- |
| Hub WS/SSE wire | ingress/hub.rs:921-937 claim 命令 → :1276-1295 handle_hub_command → Hub context → local_service::dispatch | 普通 Process/MCP/Tmux 跳过开关；Skills 只保留 capability 检查 |
| Hub HTTP JSON | agentic-gpt-hub/ingress/http/routes.rs:789-832 身份与 Agent enabled → request_agent → Hub wire | 与 Hub wire 相同；Agent enabled 不等于 Toolset enabled |
| Hub Apps MCP | agentic-gpt-hub/ingress/mcp/mcp_server.rs:571-602、964-994 等 ensure_agent_enabled → request_agent → Hub wire | 与 Hub wire 相同 |
| Tunnel stdio MCP | AgentMcpServer 默认 TunnelStdio → stdio_server.rs:217-228 工具可用性检查 → :432-445 authorize | 按实时 Toolset 拒绝；Process exec/batch 走专用 dispatch_process 授权 |
| Unix 本地 MCP | local_control.rs:45-73 peer UID/容量 → AgentMcpServer(LocalUnix) → 同 MCP 调用链 | 按实时 Toolset 拒绝 |
| Agent HTTP MCP | http_server.rs:233-258 Http context 与 bearer/Host/Origin → 同 AgentMcpServer | 按实时 Toolset 拒绝 |

stdio_schema.rs:26-38 的 Toolset 过滤控制发现列表；stdio_server.rs:217-228 还做调用时过滤。但 Hub 直接发 wire command，不经过这些 MCP 边界；不能用发现过滤证明 Hub 已授权。

## 结论与验证限度
- 当前问题仍存在，可静态确定关闭 Process/MCP/Tmux，或在有 Skills capability 的运行时关闭 Skills，不会阻止 Hub 入口通过普通 Toolset 准入门。
- 通过该门不保证最终执行成功；命令、路径、确认、下游配置、资源等条件仍可能拒绝。
- 本轮核查基线为 df529f2，只新增调查记录；未修改运行时代码、公开契约、用户文档，未新增或运行测试、未启动真实 Agent/Hub。
- 已检查记录 diff 的格式与范围；没有将静态链路核查称为运行复现。
