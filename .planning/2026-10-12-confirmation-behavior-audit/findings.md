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

## 调整方案待核实事项
- acquireConfirmation 的许可只覆盖本次命令决策，必须明确是否仍保留路径/Toolset/参数/容量校验。
- 移除 builtin 将使旧配置失去未显式写入的限制，初始化模板与旧配置迁移需区分。
- 通配符须明确匹配原始脚本还是逐条解析命令；args.join 会丢失参数边界。

## 用户确认的调整语义
- MCP 提到修改为笔误，保持全部批准行为现状。
- acquireConfirmation 为可省略对象；acquire 省略 false；method 单渠道 ntfy/freedesktop，省略走默认fallback。
- 主动批准后覆盖命令 allow/confirm/deny；明确拒绝终止；超时/不可用回配置规则。
- 回退到配置 Confirm 时复用主动确认失败结果并终止，不二次等待。

## 本轮代码核对
- config.rs:1176-1194 普通load确实从default_config合成再merge；1702起sparse_defaults/prune会裁默认字段。因此初始化preset须独立于加载缺省值，避免隐式重建规则源。
- 初始化仅seed新配置；旧配置合并builtin时不能直接追加，因为旧allow原本能覆盖builtin confirm/deny，追加进同一分类后会改变效果。
- 移除builtin的消费者还包含agent.info builtinConfirm/builtinDeny、safe_summary builtins及protocol/OpenAPI公开字段。
- exec.rs:122起对交互凭据、TTY及路径的检查是执行能力/路径边界，不应被主动命令批准当成allow/confirm/deny绕过。
- 桌面确认目前timeout/关闭/非allow动作映射deny，必须拆分明确拒绝、超时、不可用、取消；MCP语义保持原状。
- tmux paste/close旧默认true，替换为acquire默认false会改变省略参数行为，应明确说明。
- 通配符可简化公开rule表示，但仍需逐命令判定和参数边界；原始整脚本glob会扩大规则覆盖。

## 调查纠错
- 初始化子代理早期错误认为普通load不合并默认值、配置deny非优先；根agent通过config.rs:1176-1194和policy.rs:49-66核对并收到子代理修正。最终结论采用源码证据。

## 方案建议（尚未实现）
- 统一获取许可入口，内部区分 approved/denied/timeout/unavailable/cancelled，不用布尔值覆盖原始证据。
- acquire=true：批准跳过本次命令决策；拒绝/取消终止；timeout/unavailable回policy，Allow执行、Deny拒绝、Confirm复用失败终止。
- acquire=false：正常配置决策；method建议仅控制主动获取，不隐式切换策略触发的默认确认渠道。
- 主动批准仍绑定完整操作请求：脚本、cwd、batch全部元素、pane/粘贴内容、patch有效变更；不形成命令通用许可。取消/路径/能力/文件冲突检查继续有效。
- 策略初始化使用独立preset seed，显式写入policy且禁止sparse裁掉；运行时仅认配置。profile建议只影响初始模板，后续不自动改规则。
- 旧配置提供显式迁移预览；策略默认结果（未匹配）建议统一并显式配置，当前Shell Confirm与argv Allow的差异需被决策。
- 外部规则可换为单pattern字符串，内部仍逐invocation/argv匹配；建议小范围通配符语法并明确参数边界，避免整个原始脚本glob。
- 建议实施拆分：确认流程、策略来源、匹配格式三步分别验收；本轮只完成可行性评估。

## 用户后续确定的规则
- 单独的 * 为全命令匹配规则，allow中的 * 也覆盖无法静态分析脚本的额外确认要求；已有匹配confirm/deny仍按配置优先级处理，主动批准可覆盖命令决策。
- 其他pattern中的 * 按参数内通配符解释。
- 用户称tmux pasteContext，按当前现有tmux.pasteText理解：该API单独默认acquire=true；可显式false关闭主动请求。
- 建议省略整个对象、对象为空、仅提供method时都沿用本API的acquire=true，避免对象存在与否改变默认行为。
- 用户确认paste主动确认必须获准；拒绝、超时、不可用、取消均终止，不回退到Allow。显式acquire=false仍表示不主动获取许可。

## 配置格式提案（未实现，尚未获用户确认）
```json
{
  "policy": {
    "defaultAction": "confirm",
    "allow": [
      "git status **",
      "git diff **",
      "git log **"
    ],
    "confirm": [
      "rm **",
      "mv **",
      "python -c **"
    ],
    "deny": [
      "ssh **",
      "mkfs **",
      "dd **"
    ]
  }
}
```

- 保留allow/confirm/deny分类，将Rule对象替换为pattern字符串；defaultAction可取allow/confirm/deny，建议初始化confirm，但用户尚未决定。
- 单独 * 为全命令规则；其它pattern中的 * 匹配单个token内字符，尾部 ** 匹配零或多个剩余参数。
- 无 ** 的pattern要求参数数量完整匹配；规则匹配的是解析后的单条命令，不是整段脚本。
- 冲突优先级固定deny>confirm>allow，不根据顺序/长度变化；主动批准覆盖命令决策。
- 全局allow *覆盖无法静态分析脚本的额外确认要求，不代表可以识别动态展开后的所有执行行为。
- 初始化preset作为显式配置内容保存，运行时只从配置读取；建议以确认流程、单源初始化、字符串pattern三个步骤实施。
