# 审查证据

- 用户请求不含 needConfirm、group，包含 command/cwd/waitSeconds。
- 文件读写成功，process.exec 返回 rejected/provider_unavailable；先区分确认机制与 allow 准入。
- 共享 ProcessExecRequest 的 need_confirm 没有 serde default；须核实 Apps MCP 包装层默认值。
- 工作区开始时干净，存在 .codegraph，已优先调用 CodeGraph。
- 当前 Hub Full Apps MCP 的 need_confirm 使用 unwrap_or(false)（mcp_server.rs:582）；stdio 入口仍需定点确认。
- managed.rs:2524-2562 仅在 PolicyDecision::Confirm 时请求确认；非 allow_once 结果转 rejected；provider_unavailable 发生在 spawn 前。
- confirmation.rs:229-275 遍历配置通道；全部不可用或无通道返回 provider_unavailable。
- docs/interfaces.md:60 已描述 process_confirmation_unavailable，rejectReason/审计仍为 provider_unavailable。
- init-session.sh 更新了受跟踪的 .planning/.active_plan；审查结束应还原该共享指针，保留独立计划目录。
- 已只读查询 Room agent.info（2026-10-12T04:52Z）：version=0.9.1，profile=room，tunnel-stdio，启动于 2026-10-08；allow 中有 ls/cat/date/rg，无配置 confirm/deny，liveSubsetMatchesDisk=true。
- Room 两个确认提供者均不可用：freedesktop=notification_service_unavailable；ntfy=hub_relay_disconnected；hubReporting disabled。
- 独立 scout：未引用的 glob 标记不完整，强制确认；date+rg 的字面参数与换行在当前实现应可 Allow。
- 已只读 process.list/read 核对最近三次 rejected；error.code/message 均是 provider_unavailable，旧部署没有当前代码中的友好错误映射。
- 当前 stdio ProcessExecArgs 的 #[serde(default)] bool 缺省为 false，group 缺省 None；Oct 8 版本源码也相同。
- Room 运行时间早于 Oct 11 修改；仅 version=0.9.1 不足以确定构建提交。

## 已复现的代码缺陷
- 本地 HEAD=1a3786d，cargo build --offline -p agentic-gpt 成功；复用 parity 的 Room stdio-worker/stdio_tool_call，在临时 HOME/XDG/TMPDIR、临时工作区执行。
- allow=date/rg/ls/cat，needConfirm 与 group 均省略，确认通道空列表：date 单独=completed；rg 单独=completed；date LF rg=rejected/process_confirmation_unavailable；date 分号 rg=completed；ls glob LF cat glob=rejected。
- 临时审计：LF 请求 needConfirm=false、policyDecision=Confirm、confirmationResult=provider_unavailable；单命令和分号请求为 Allow。
- 结论：当前代码可复现 LF 被错误升级确认；不仅是部署陈旧或参数误用。具体 AST gap 根因待独立核实。
- 修正先前结构推断：grammar 将 LF 声明为 terminator，并不证明运行时 tree.children 暴露 LF；真实链路结果优先。

## 审查结论与后续修复边界
1. **P2 已确认：普通 LF 分隔的白名单脚本被误判需要确认。** shell_parser.rs:149-155 使用 source gap 校验；:200-202 仅允许 space/tab；实际 LF 落在 sequence gap 后标记不完整，policy.rs:108-111 将它提升 Confirm。Room 无可用确认通道后走 managed.rs:2524-2562 拒绝。真实 stdio MCP 冒烟与审计已闭合此链路。
2. 原始 ls/cat 请求中的 unquoted glob 本来就被 decode_unquoted（shell_parser.rs:355-368）标记不完整；修复 LF 不应自动放行动态展开。
3. 当前 Room freedesktop/ntfy 不可用是运行状态事实；任何真正需要确认的请求仍会拒绝。file.read/file.edit 成功不证明执行确认可用。
4. 参数无误：MCP needConfirm 默认 false；false 不绕过策略确认。group 非确认开关，waitSeconds 仅等待。
5. 当前代码已提供 process_confirmation_unavailable 友好映射（managed.rs:1073-1078），Room 返回裸 provider_unavailable，提示部署错误呈现与仓库有差异；尚未核实部署精确提交，不能用 version=0.9.1 断言一致。

### 后续建议（本次未实施）
- 在 sequence 层识别 Bash 的 LF 终止边界；不要直接全局放宽 literal argument 的 gap 判定，以免改变 CR、转义换行与不支持结构的安全行为。
- 适合增加第④/⑤档入口回归，长期保护 allow 下普通 LF 链与等价分号链不被误要求确认，并保留 deny-before-execution 边界；修复阶段再按项目规则选择保留价值。
- 临时替代：这次无 glob 的两条独立命令可用 `date -Iseconds; rg --files Hearth/hooks`，本地真实链路已验证；本次未在生产执行。
- 所有确认通道恢复、部署升级以及实现修复留给明确修复阶段，本次不改变配置/源码，不提交。

## 扩大审查：已验证候选
- 已复用真实 Room stdio MCP fixture 跑 48 次请求（35 + 13），使用临时 HOME、XDG/TMPDIR 与临时工作区，无生产执行，无持久新增测试。
- 普通 LF 家族：尾随 LF、空行、注释后的 LF、&& 后 LF、pipe 后 LF 均误 Confirm；leading LF 单独可成功，带引号参数内 LF 可成功。
- **deny 完整性缺陷**：deny printf argsPrefix=[blocked] 时，普通 `printf blocked`=policy_denied；程序名续行、参数前续行、参数词内续行均降为 Confirm；它们在普通 Bash 中仍然是同一个 printf/blocked 字面调用。
- **特殊 builtin 提取缺口**：显式 deny export/unset/declare（argsPrefix 空）时，`export REVIEW_VALUE=1`、`unset REVIEW_VALUE`、`declare REVIEW_VALUE=1` 均返回 confirmation_unavailable，而非 policy_denied；其 declaration_command/unset_command AST 不被 collect_nested_commands 直接提取。嵌套 `export REVIEW_VALUE=$(printf blocked)` 仍能抓到内层 deny，返回 policy_denied。
- 以上 Confirm 降级仍需人工确认，未发现无确认放行；managed.rs 确认 allow_once 后会继续执行，不重算 deny，故配置 deny 的不可确认覆盖性在这些输入下失效。
- **保守误判**：双引号里的 `$:`、`$/` 是 Bash 字面内容，但 decode_double_quoted:399 按 `$` 后有字符就判未知，导致 Confirm；末尾单独 `$` 可成功。
- **保守误判**：单/双引号内的反斜杠 CRLF 字节被全局 crlf_line_break_offsets 检出，无视 quote context，导致完整字面参数也被强制 Confirm。该路径不构成放行证据。
- 验证嵌套 deny：命令替换、进程替换、if、subshell、brace、function definition、引号和拼接、CRLF 的已测输入均 policy_denied，输出为空；未发现这些结构误 Allow。

## 修复实现与回归（阶段7）
- 未新增依赖，保留原始script执行；在AST token gap识别严格的反斜杠LF，按是否剩余空白将字面片段合并进program或arg。
- sequence gap识别LF和续行；续行紧邻#导致Tree-sitter与Bash存在语法分歧时仍Confirm，已新增负向入口回归避免误Allow。
- 特殊builtin（declare/typeset/export/readonly/local/unset/unsetenv）只提取确定的program和可靠prefix用于Deny，complete仍false。
- CRLF异常扫描排除raw/ansi/DQ literal内容、comments、heredoc body，保留DQ中嵌套命令分析。
- DQ字面$识别区分真实展开，跳过$后真实续行再判定，避免 `$\\LFHOME` 被误当字面$HOME。
- reviewer发现的builtin suffix假deny已修：`unset blocked\\LF"$HOME"` 丢弃partial参数，`unset\\LFfoo` 识别为unsetfoo而非unset。
- 新增2个长期链路回归：Room真实MCP dispatch+managed执行核对输出与deny-before-side-effect；fixture显式清空确认通道，外层HOME/XDG/TMPDIR隔离，helper取消和等待非终态短进程后清理。
