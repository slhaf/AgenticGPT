# 调查发现

- 仓库存在 CodeGraph，已优先使用索引定位当前源码。
- Agent operations/policy.rs 决定命令策略，operations/confirmation.rs 负责确认渠道与等待。
- Hub runtime/confirmation.rs 拥有待确认记录、token、到期与决策投递。
- needConfirm 需区分命令执行、文件操作、下游 MCP 和 tmux 的实际消费。
- 历史记录仅帮助定位共享授权边界，所有行为结论重新核对当前源码。

- policy_decision_for_profile 先根据 needConfirm 和 builtin 给出默认决定，再被配置规则整体覆盖；配置冲突固定 deny > confirm > allow，不比较前缀长短。
- shell_policy_decision_for_profile 先逐命令匹配，任一 deny 拒绝整段；未显式 allow、语法不完整、空提取或 needConfirm=true 均提升到至少 Confirm。
- file.edit 缺省 needConfirm=false；仅有效变更且 true 时请求一次确认；批准后重新复核源文件 revision，失败清理暂存资源。
- tmux.exec 缺省 false，使用结构化命令策略；pasteText/closeSession 缺省 true，仅以参数控制确认。
- Standalone process 参数默认 false；Hub wire/HTTP process.exec/batch DTO 的 needConfirm 为必填。
- 调查中一次搜索包含不存在的 mcp_schema.rs；随后使用 CodeGraph/实际文件发现定位，未涉及代码修改。

- Hub Apps MCP process.exec/batch 的 needConfirm 可省略或 null，转换为 false；与 HTTP/wire DTO 必填不同。
- confirmMethod 只影响确认渠道，不影响策略；默认 channels=[freedesktop,ntfy]，legacy hub/ntfy 映射远程 relay。
- 发现契约描述差异：OpenAPI TmuxExecRequest needConfirm 描述为强制确认，但结构化策略配置 allow 可覆盖 true。
- 配置文档使用“更明确 allow override”措辞；源码配置冲突固定 deny 优先，未按前缀长短排序。
- 通用 authorize 先做实时 Toolset/能力/入口检查，确认批准不能越过这些准入门或路径策略。

- Process exec 的 worker 对 Confirm 写 waiting_confirmation；只接受 allow_once，其他结果拒绝并保存原拒绝原因。等待确认时可通过 process.cancel 取消。
- Process batch 在创建子进程前完成全部预检和一次聚合确认；确认失败时整批尚未启动，也尚无子进程 ID 可取消。
- skills.run 不接受 needConfirm/confirmMethod，内部 argv request 固定 false；仍应用精确脚本路径的命令策略。
- Agent 用户确认等待为45秒；Hub 回执外层等待50秒，Hub TTL另受provider timeout上限约束。
- 下游 MCP 默认请求确认，无 needConfirm 参数；存在按 agent/server 的15/30分钟临时批准，本地提供两档，远程只提供30分钟。

- MCP 临时许可纯 AppState 内存，重启失效，过期惰性清理；普通配置 reload 保留许可，按当前 agentId/serverId 匹配。

## 主要证据入口
- 命令策略：crates/agentic-gpt/src/operations/policy.rs:26,69,135,147。
- Toolset 准入：crates/agentic-gpt/src/operations/operation.rs:284。
- 参数 schema：crates/agentic-gpt/src/ingress/stdio_schema.rs:397,408,432,538。
- Hub Apps 参数：crates/agentic-gpt-hub/src/ingress/mcp/args.rs:146,182。
- 单进程/批次确认执行：crates/agentic-gpt/src/process/managed.rs:1615,2494。
- 文件有效变更确认：crates/agentic-gpt/src/files/file_ops.rs:1804。
- tmux 结构化策略：crates/agentic-gpt/src/tmux/tmux.rs:270。
- 确认渠道与 MCP 临时许可：crates/agentic-gpt/src/operations/confirmation.rs:125,632,753。
- Hub TTL：crates/agentic-gpt-hub/src/runtime/confirmation.rs:75。

## 范围与结果
本次为源码及公开契约调查。未使用真实 HOME 配置作为源码默认行为依据、未启动 Agent/Hub、未触发确认、未新增测试。仅调查记录发生变更；两处文档描述差异已报告，未改变行为或修订公开合同。

- 桌面确认通知成功显示后，关闭、等待超时或未知动作被归一为 deny；不会切换渠道。Hub 端 timeout/expired 则保留独立决策。
- Hub 回调检查 token 摘要，错误 token 保持 pending，已解决回调拒绝重复处理；绑定连接代际，断连/替换只失效对应代际的请求。
- Hub resolved 确认记录仅标记终态并取走 sender，不从 HashMap删除，保留到进程结束。
- 子代理额外只读核对了本机已有规则是否属于默认值；最终说明仅以当前源码规则为依据，不引用本机配置内容。
