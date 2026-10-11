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

## Profile 用途追加调查
- 新问题：除工具集预设与 Hub 准入外，Profile 是否还有实际用途。
- 初次当前源码搜索已发现 operations/policy.rs 中 Normal/Room 参与策略决策、stdio_server.rs 中传入终态事件 hook、ingress/hub.rs 中映射 role 并上报，以及 runtime startup/supervisor 中用于启动身份。
- 尚需沿这些消费者核实可观察行为，不能仅由引用次数判断用途。

### Agent 内已核实用途
- operations/policy.rs:25-64 的 policy_decision_for_profile 调用按 Profile 选择的 builtin_rules；:147-194 中两种 Profile 的 deny 都为 su/mkfs/dd/ssh，Normal confirm 集更大（含 rm/mv/chmod/chown/docker/curl/wget 及 python -c、node -e 等），Room confirm 仅 sudo/mount/systemctl/service/scp。显式 config.policy 可覆盖，needConfirm 参与决策。
- tmux/tmux.rs:270-288 的 tmux.exec 真正使用该决策拒绝或请求确认；不局限 Hub。Process Managed 的 Argv 执行（包括 Skills）也通过 managed.rs:2430-2443 消费 Profile 规则。
- 注意 Process shell 特例：policy.rs:69-114 的 shell 决策会将未匹配显式 allow 的 Allow 提升到 Confirm；不能据内置列表宣称 Room 的 process.exec Shell 可无确认运行所有 rm/curl。
- runtime/state.rs:108-140 按 transport + profile 派生 capabilities；除了已讨论 Skills，notifications 在 Local/Tunnel Normal 为 false，Room 为 true，Hub Normal 为 true。operation.rs:330-336 实际消费 notifications；agent_info.rs:181-185 上报全组能力，bootstrap/diary/notebook 能力字段不能直接等同实际授权。
- runtime/startup.rs:86 worker 启动校验 profile；:296、:354 等对运行时选择变化拒绝热重载。supervisor.rs:284-290 检测 profile 变化提示重启；config.rs:1583 把 profile 列为 restart-required。
- stdio_server.rs:2769-2806 中 Profile 仅作为终态事件消息的标签；不是另一组通知调度逻辑。
- Agent info 的 profile/capabilities/builtin policy summary 是诊断用途，不等同 Toolset；工作区 ensure_workspace 并不按 Profile 创建不同目录（config.rs:1452-1454）。

### Profile 映射的 Hub 角色用途（scout 独立追踪）
- runtime/state.rs:29-47 把 Profile 映射为协议 AgentRole；ingress/hub.rs:96-104、257-265、356-364、735-744 的 Hello 上报该角色，包括 command-capable 与 reporting-only 连接。
- Hub agents/lifecycle.rs:185-212、355-367 注册连接并保存 role；仅 command-capable 注册活动 Room。room/control.rs:28-59：Room role 可占据唯一 active_room（另一个 Agent 会冲突）；Normal role 可释放该 Agent 的原 active slot。
- agents/dispatch.rs:241-248 的 request_room → lifecycle.rs:530-564 的 resolve_room_target 校验在线连接、Room role、command-capable、Hello-ready，发送前 :600-613 再检查 expected role。
- Hub Apps MCP 的 Room、bootstrap、Skills 工具使用 request_active_room，例如 mcp_server.rs:1374-1388、1580-1606；room/http.rs 也定向到活动 Room。这是目标选择/调度用途，不只 Agent 侧准入。
- 未发现 Hub AgentRole 驱动 Console 选择/展示；Hub agent.list（mcp_server.rs:385-411）不返回 role。role 保存在 live connection，registry.rs:139-193 的持久注册表不保存 role。Hub 自身 McpProfile Full/Coordinator 是另一套配置。

### 本阶段结论
Profile 不只是工具集初始化预设和 Hub 准入：它还选择内置命令确认策略、映射 Hub 活动 Room 角色、作为固定运行身份参与启动/热重载校验，并影响能力/日志诊断标志。前两项有实际行为意义，不能把全部 Profile 引用当作展示标签。全部结论为当前源码静态核查，未运行测试或服务、未改变实现。

## 实施与测试保留价值评估
- 长期行为：Toolset 关闭必须在任何外部 ingress 的所有者执行前拒绝；避免远端入口再次绕过、别名映射遗漏或热变更后使用过期准入。
- 已有 hub_adapter_and_local_dispatcher_share_toolset_errors 仅检查同为 Hub ingress 的 Room bootstrap，不覆盖普通 Process/MCP/Tmux/Skills；changing_live_toolsets_updates_surface_and_authorization 只验证本地 Room 发现/调用变化。
- 计划新增/扩展第④⑤档回归：真实 Hub adapter/shared dispatch 与本地入口的禁用行为、Event 存储链豁免、Skills 在 Process 关闭时仍能运行。以拒绝前无所有者副作用或真实完成为断言，避免仅测试 authorize/helper 或固定 namespace 表。
- 保留值：发现入口遗漏、热配置读取遗漏、错误进入下游所有者、内部 Process 误加 Toolset 依赖；相同失败模式未被已有具名测试覆盖，新增数量保持最小。

## 修复后的兼容性
- 不改变DTO/schema、namespace集合、工具发现或传输封套；现有未知操作、CLI、Event、通知和private ingress分支保持。
- Hub原先跳过的普通Toolset现在会返回toolset_required；Room仍用room_toolset_required。
- 当Hub Room操作同时缺少Room开关与Profile时，改为先返回开关错误，符合统一准入顺序；仅有能力不满足且开关启用时仍返回既有room_agent_required。
- 已准入Process/Skill的执行和配置快照不变；Skills内部ManagedProcess不新增Toolset门控。
- 既有normal_hub_rejects_current_room_commands_without_room_profile是第④档Hub消费链Profile约束回归；启用Room工具集作为fixture前置条件后，仍保护工具集开启不能替代Profile的长期行为，未弱化或删除断言。
