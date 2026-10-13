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

## 用户已接受的配置格式（未实现）
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

- 保留allow/confirm/deny分类，将Rule对象替换为pattern字符串；defaultAction可取allow/confirm/deny，用户接受初始化confirm及其未命中回退含义。
- 单独 * 为全命令规则；其它pattern中的 * 匹配单个token内字符，尾部 ** 匹配零或多个剩余参数。
- 无 ** 的pattern要求参数数量完整匹配；规则匹配的是解析后的单条命令，不是整段脚本。
- 冲突优先级固定deny>confirm>allow，不根据顺序/长度变化；主动批准覆盖命令决策。
- 全局allow *覆盖无法静态分析脚本的额外确认要求，不代表可以识别动态展开后的所有执行行为。
- 初始化preset作为显式配置内容保存，运行时只从配置读取；建议以确认流程、单源初始化、字符串pattern三个步骤实施。

## 实施前假设核对
- 核心运行语义已确定，无需再次确认；用户选择全面升级，不保留旧格式/旧字段的兼容执行。
- 现有执行路径Deny在确认前退出（managed.rs:1660,2524）；新逻辑须在主动获取后才最终拒绝。
- 现有桌面结果混合拒绝和超时；普通主动获取的新回退依赖区分，paste保持严格失败终止，MCP不能因此改变。
- Runtime builtin及默认加载/稀疏保存假设需调整；初始化策略必须显式保存并只由初始化seed。
- defaultAction=confirm将改变当前argv未命中时Allow的行为，已作为新方案统一默认。
- 无法解析脚本的默认保护准确说法是至少Confirm；若defaultAction=Deny应保持Deny，不因该保护降为Confirm。allow的全局*按用户决定可解除解析不完整的额外确认，命中其他配置规则仍按优先级。
- 旧argsPrefix应迁移为token转义后的pattern加尾部**，而非裸join；旧配置allow覆盖builtin的效果不能用直接append保持。
- 策略声明/匹配不承担执行能力：Toolset、路径、资源、TTY与交互凭据限制不被批准覆盖。

## 用户最终升级约束
- 全面升级、彻底重构，不保留旧needConfirm/confirmMethod或program+argsPrefix的兼容运行路径。
- 升级交付包含独立迁移脚本，现有配置由用户额外手动运行迁移；启动/load/reload不自动迁移。
- 脚本按新语义转换规则，提供备份/预览/变更与冲突说明；不读取或修改真实配置作为本轮调查副作用。
- 核心语义已确定；初始化模板按profile、method仅主动获取、路径/能力限制不覆盖等按既定方案实施。
- 尚未开始实现，当前阶段为完成设计核对；与旧实现的差异是明确改造范围，不存在已知架构阻塞。

## 路径策略范围补充（以此取代早前相关约定）
- 用户明确file.*路径策略属于本次重构范围，不仅command policy。
- 路径策略也改为通配符规则；主动acquire批准可覆盖denyRoots、readonly、outside allowed roots等配置路径限制。
- 先前“批准不覆盖路径策略”的建议已撤销；Toolset、执行能力、容量、文件一致性等独立边界不因此变化。
- 需要联动file.read/search/edit、Process/tmux cwd/arg预检、bubblewrap实际挂载、配置初始化/CLI/摘要/迁移脚本。
- 不实现或迁移真实配置；正在补充只读路径执行链核对。

## 路径执行链补充证据
- file_ops.rs:174路径canonicalize后立即配置权限检查，read/search尚无确认入口；新增批准覆盖需要把规范化/完整性校验与配置裁决分开。
- file_ops.rs:382检查write/read/deny roots；file.search目前只校验起点，walker逐文件读取没有再次策略检查，通配符策略需校验实际递归访问。
- file.edit批准应覆盖全部Add/Delete/Update/Move源/目标及新父目录；等待后复核canonical目标和revision保留。
- process/exec.rs:433-490 bwrap仅按write/read-only roots及required_runtime_paths做bind；没有精确实现deny子目录mask。通配符规则不能直接当成目录bind。
- tmux.exec在既有pane提交命令，不调用Process build_command/bwrap；共享路径预检而非共享OS隔离。
- 新pathPolicy拟按read/write分区采用allow/confirm/deny patterns，未匹配默认及glob范围需要补齐；这是提案而非已确认字段。
- command allow=*不自动解除路径决策；主动一次批准拟覆盖本请求命令和路径策略，避免重复通知。
- 明确动态脚本的路径授权范围是新增设计问题：不能承诺从脚本文本识别其运行时全部文件访问，或静默扩大整个文件系统访问。

## 路径通配符与实际隔离的边界
- 工程复核支持pathPolicy.read/write各defaultAction/allow/confirm/deny，匹配规范化绝对路径；具体字段与默认值为后续提案。
- 路径 *不跨目录、**跨目录，单独*全路径；不能按命令参数通配符的token语义直接实现路径匹配。
- search批准应说明搜索根与递归范围，未批准时逐实际候选路径校验；edit授权全patch范围，保留文件身份和revision复核。
- bwrap为静态路径/子树bind计划，任意glob不等价于可精确编译的挂载规则；现有参数预检不完整分析动态脚本。
- 已向用户询问Process批准后的sandbox访问范围：可选read/write scope临时开放（建议）、仅准入覆盖原mount不变、或者本次进程全文件系统访问。
- 该选择待答；不得自行扩大sandbox到/、关闭sandbox或承诺批准后任意动态访问一定成功。

## 用户确定的文件路径默认与搜索批准范围
- file.*读/写路径未匹配任何规则时，defaultAction=deny。
- file.search主动获准后，覆盖本次请求指定目录及递归搜索范围，仍遵守include/exclude、hidden、respectGitignore等请求过滤条件；确认提示必须明确该范围。
- 文件部分行为选择已齐：read/write规则分别配置，acquire默认false；命中配置Confirm仍确认；主动批准覆盖配置路径限制；edit批准覆盖完整补丁范围，保留路径身份与revision复核。
- Process/bubblewrap批准后的实际访问范围选择仍待用户回复；本次文件选择不代替该选择。

## 用户接受Process范围授权，调整参数归属
- 用户接受保留bubblewrap并以本次read/write范围临时开放实际文件访问的方案。
- 用户要求read/write放入acquireConfirmation，不另设独立accessScope参数；建议直接使用acquireConfirmation.read/write数组。
- Process批准范围绑定脚本、cwd与具体读写文件/目录，目录表示子树；仅本次生效，不修改全局配置。未声明动态访问仍受sandbox限制。
- sandbox基础具体挂载与pathPolicy通配符判定分开；不能把任意glob扩大到父目录以冒充精确执行限制。
- read/write建议用于Process范围申请；file.*由具体请求路径/搜索目录/patch确定范围，无需重复填写。
- 建议非空read/write必须与acquire=true一起使用，否则报参数错误，避免静默忽略；此为接口细节提案。

## 用户确认scope字段约束与文件API应用说明
- 用户批注接受：method省略默认fallback，Process read/write可省略、具体目录覆盖子树、只对本次进程临时生效；非空read/write必须配合acquire=true，否则参数错误。
- file.*只需acquire/method；请求本身确定文件批准范围，不由调用方重复传read/write。建议文件API schema不暴露read/write，额外字段报错，避免扩大本次文件操作之外的范围。
- file.read批准请求的文件读取；file.search批准指定目录递归范围并遵守请求过滤；file.edit批准完整patch中的实际读写路径及必要父目录。
- 先确定真实目标/校验结构，再取得一次主动许可，批准覆盖本次路径策略；拒绝/取消终止，超时/不可用按既定策略回退，Confirm复用失败，默认Deny仍拒绝。
- acquire=false时按路径规则正常处理：Allow执行，Confirm一次确认，Deny拒绝；不改变全局配置、不缓存文件临时通用授权。
- 文件身份、revision/竞态、保留runtime路径、大小限制、资源清理仍生效，批准不等于跳过完整性校验。

## 迁移入口简化
- 用户明确要求删除输入、输出、HOME、workspace 参数，仅保留 --write。自动使用当前用户 HOME 下 .agentic_gpt/config.json；已有 workspaceRoot 沿用，否则使用标准 workspace。
- 保留默认预览、配置锁、字节复核、备份、原子替换及再次运行无变更语义。

## 合并迁移入口
- 用户要求将配置和私有状态迁移脚本合成一个；统一保留 --write，默认预览，自动标准 HOME/config。
- 状态迁移需要 run lock/迁移锁及旧 socket 检查，事件/Process 转换必须在临时副本进行；目标状态已存在时跳过，旧源保留。配置策略转换与状态检查需先完成，避免验证失败却写入配置。

- 实现为 scripts/migrate-agentic.py 单文件，删除两个旧入口；迁移/run/config 三锁贯穿验证与写入，配置先转换、状态副本先验证；无旧状态/已有新状态报告跳过，不阻止配置转换。
- 长期测试价值：跨 CLI/文件/SQLite 验证配置与状态的共同写入、原数据保留、拒绝活动实例、错误与中断后撤回；原脚本无具名测试覆盖这些边界。新增 8 个真实文件/SQLite 集成用例（⑤档），纳入 CI；未新增纯字段/默认值快照测试。
- 审查发现配置成功写入后中断的恢复缺口，已将恢复扩展至外层完整提交区，故障注入测试通过，独立复审无剩余 finding。
