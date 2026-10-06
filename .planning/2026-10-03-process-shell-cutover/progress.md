# 进度

## 阶段1：所有者与共享接口
已完成只读映射：MapShellOwners/MapShellSurfaces/MapShellConfig，证据保留agent输出。核心单owner负责managed/exec/Skill内部argv；独立policy owner仅policy/confirmation/parser模块；配置owner负责配置与CLI/TUI/reload；输入owner负责protocol+Agent/Hub ingress/OpenAPI及对应合同测试；验证owner迁移其余测试与parity请求。

冻结公开DTO：ProcessExecRequest(command:String,cwd:Option<String>,其他字段不变)；ProcessExecElement(command,cwd)；ProcessBatchExecRequest顶层working_directory→cwd，保留继承规则。deny_unknown_fields拒绝旧输入，非输入ProcessInfo历史元数据保持现有协议。内部Skill不使用公开command DTO，核心owner建立内部argv spec并更新Skill。

冻结policy接口：policy::shell_policy_decision_for_profile(config:&Config,profile:CapabilityProfile,command:&str,need_confirm:bool)->PolicyDecision。每个字面调用沿用现有matcher及configured precedence，shell入口无allow匹配的默认Allow提升为Confirm；needConfirm针对整段，不能被单条allow消除。完整解析失败/不支持至少Confirm，但已知deny仍须整体Deny。确认按原始script整体展示；BatchConfirmationElement改为command/cwd。

冻结配置接口：Config.shell:ShellConfig，ShellConfig.init_file:ShellInitFile，ShellInitFile::{Default,Disabled,Path(String)}。Default对应字段省略、Disabled对应null、Path对应显式String（即使与默认路径相同）。serde保持三态，不把省略默认序列化为显式路径。热更新遵循准入配置快照。执行者可match这些variant。
不运行中途build/lint/test/format；全部实现合并后父代理统一最多20轮验收。历史已有实际smoke证明取消child-only后代存活，不重跑旧行为。Context7当前网络不可用，官方Tree-sitter/Codex源码已读供受限提取参考。

## 阶段2：实现源码交付
CoreExecution阶段2初版已交付managed/exec/Skill内部argv，Bash control channel独立于stdout/stderr；Default实际sandbox内open ENOENT-only skip，显式/非零加载失败Failed+shell_init_file_failed。初版R后的普通非零被误置Completed，阶段3按现有状态契约修正为Failed+exitCode且无初始化error；0仍Completed。初始化后cwd重设。TERM→KILL组取消含leader终态后台child，捕获与组收敛前保留runtime。
ShellConfiguration已交付三态serde/CLI/TUI/live reload/import；`config unset shell.initFile`只用于此新键恢复Default，其他键不可unset。PublicProcessInput已交付所有输入schema/adapter/OpenAPI；ConsumerMigration已迁移其余fixture/parity。PersistedArgvCutover保持旧argv raw/hash/completed result，旧未完成退休不重放。
独立审查发现的三处 parser 词法完整性缺陷及配置/持久恢复缺陷已修正，见 findings.md。Smoke 位于 /tmp/process-shell-cutover-smoke.py，覆盖 init 三态/symlink/error/cwd、脚本/策略/确认、真实 PATH 及函数、group 升级/leader 退出/EOF，并包含 sandbox.enabled 的实际场景；尚未运行此 Smoke。
本机`bwrap --help`/`--version`实际证实0.13.0且无--preserve-fds；官方源码确认child可继承fd，Core已移除不存在参数，未新增挂载。现有bwrap未启用--new-session。
父代理验证日志根：/tmp/agentic-shell-verify-9iltucll；既有隔离Python依赖可用，等待源码停写后统一格式与完整验收。

阶段2已统一执行 `cargo fmt --all`（退出0）并提交 `0fdf5da feat(process): execute managed Bash commands with trusted init`；阶段1提交为 `e399179`。

## 阶段3：验收与修复
- 第1轮：fmt=0；check/clippy/test/build 因 static.crates.io DNS 失败退出101。parity=1，构建未成功时仍使用旧二进制，旧 Agent 拒绝 command；这不作为新实现的运行证明。
- 使用 rsproxy 下载锁定的 tree-sitter 0.25.10、tree-sitter-bash 0.25.1、streaming-iterator 0.1.9，逐个 SHA256 与 Cargo.lock 匹配后填充标准 Cargo 依赖缓存。未改变依赖版本或全局 Cargo 配置。
- 第2轮：fmt=0；check/clippy/test/build=101，发现并修正五个集成类型/参数/所有权错误、setup 测试缺失导入及 unused Command import；parity=1，仍未产生新 Agent 二进制。证据为 round2-1..6.log。
- 第3轮：fmt=0、check=0；clippy=101，四项新代码 lint 已修（删除无调用 wrapper、派生 Default、byte string、末尾 Option 借用）。test 实际超过3600秒并由父控制器终止，链尚未运行第5/6项，不把这轮标成完整通过。
- round3-4.log 实际显示 `oversized_process_admission_fails_before_command_effect FAILED`；五个 stdio/panel tests 超60秒未结束。父代理现有二进制 gdb 单 case 诊断得实际 `Starting != Failed`（exit101）；Core定位 raw Shell 命令被历史字段截断后未触发元数据边界，正在补实际 Linux 单参数 kernel bound 早拒绝，保持零副作用与空历史断言。
- RepairStdioShellHang 源码定位五个 fixture 的未 allowlisted true/false/sleep 会进入真实确认；Hub panel fixture 在处理完成前不读确认 oneshot。已增加仅测试的明确允许规则，不改断言/生产策略。精确每个超时栈未观测，不能称共享 mutex deadlock已证实。
- 原超时 Cargo 遗留 test PID3570908；pstree 观察仅线程、无子进程。核对 comm/starttime 后通过 pidfd 仅终止该自建进程，并实际观察 pidfd exit-ready。没有发信号到其共享父进程组或调整 ptrace 权限。
- gdb 对旧挂起进程 attach 被 Yama ptrace_scope1拒绝；未提升权限。debug-3 单 stdio diagnostic exit101但工具丢输出，已报 xd://report_issue；未伪造 assertion。私有 HOME/XDG/TMP 且清除 DBus/display 的同一旧二进制单 case 诊断实际exit0、0.01s，并报告 hub_sender_unavailable；支持确认环境依赖，不作为修复后 full-suite 或新功能运行证明。
- 生命周期7项已由Core修复并完成私有Bash基础before/after证明（尚非Agent/Hub proof）；窄closure review另确认取消future drop/abort会泄漏terminal_capture_waiters、永久免prune，已要求drop-safe释放，不丢snapshot-before-unpin保护。
- 发现并纠正阶段2错误概述：现有stdio consumer明确要求普通false退出state=failed、exitCode=1、无error；contract.md与goal保留状态。Core修正常非零/pipefail状态，不修改/删除原消费者断言；真实Smoke同步为Failed+实际exitCode+无shell_init_file_failed。初始化错误仍typed Failed。
- Core 已交付真实 Bash `-c` 参数长度早拒绝（共享 bootstrap emitter 计数，含 quoting/NUL，无第二份大字符串）；取消 pin 改为 RAII，abort 后释放保护。两项 Rust 回归待统一运行。
- startup control 改用 Bash 动态私有 FD，source 前释放原 FD3；source 自行重新绑定的 FD3 保留给原始 command，只关闭私有 control FD。真实 Bash after proof：control 为 `B\nR\n`、init 与 raw command 共用 FD3 文件精确为 `init-fd3\nDATA`、raw marker 为 `ran`；startup reader 丢失仍阻断 command。此证明不是 Agent/Hub 全链证明。
- 统一 `cargo fmt --all` 退出0；第3轮补跑 build 退出0，首次产生新 Agent/Hub 二进制。parity 第6项运行中。
- 新二进制额外真实 Smoke 首 case HTTP504 `process_exec_timeout/agent_not_ready`，实际退出1（0.74s）；ConsumerMigration 调查隔离路径/启动就绪条件，未放宽生产策略或验收断言。
- 实际 TUI 私有 PTY 启动成功并显示 Basic/Local，但 driver 的 Tab6 没有把 Next 聚焦，退出1（1.32s）。PrepareShellTuiSmoke 按实际 j/k 导航修正临时 driver；尚不能宣称三态 surface/save 通过。
- 第3轮补齐：build=0，parity=1。parity 已真实通过本地 Unix/HTTP、Skill、共享 inbox、hot reload、Hub 连接/恢复/事件/分页与执行；新增旧 exec 负例在正确的 HTTP422 unknown-field 拒绝上被 JSON-only helper 误判。保持既有 Axum extractor 文本422行为，仅修 gate 的 raw-HTTP 负例读取，不扩大生产错误契约。
- 第4轮当前 fmt=0、check=0、clippy=101（三项 lint）、test=101（612 passed、1 failed、1 ignored，3.23s）、build=0；parity 运行中。新增 runtime 大小触发 large_enum_variant，Core 用固定 evidence 字符串借用减少布局与分配，不加无谓 per-process Box/allow 豁免。
- `process_creation_read_cancel_and_batch_use_process_api` 不再断言已过时 child-only evidence；保留同一 stdio consumer 的 state/outcome，并检查成功取消必须返回观测到的组级终止 code（TERM 或 KILL 均可），防止缺失或降级为未验证/仅单进程 evidence。未钉住信号选型、调用顺序或人类文案；其余 list/read/batch/status/error/预算断言保留。
- 第4轮 parity=1（negative fixture 改动交付前已加载旧 JSON-only helper，同第3轮失败）；第5轮 fmt/check/clippy/build/parity=0；第5轮 test=101，仅新增 startup disconnect fixture失败（612 passed、1 failed、1 ignored）。
- Core 直接 Unix socket/Bash 实验证实：另一个活 helper 持 reader endpoint 的 fork copy 时，仅 close 父 fd 不意味着 socket断开，R仍成功且marker创建；相同条件先 shutdown(Both)再 close，则Bash exit1、marker缺失。回归只改为真实 shutdown，保留 non-success/no-marker断言；未引入 ACK 或改变生产行为。
- 父代理私有 HOME/XDG 聚焦运行 `cargo test -p agentic-gpt --bin agentic-gpt exec::tests::losing_startup_channel_before_ready_prevents_command_execution -- --exact`：exit0，1 passed，实际验证断开阻断原始 command。
- 实际配置 TUI 完整三态 Smoke 已 exit0：独立真实PTY可见 Default/Disabled/Path，Path字段只在Path模式显示；Default JSON省略initFile、Disabled null、Path保留私有绝对路径。每次临时根/自建PTY进程已清理。
- 两个 docs owner 已更新 configuration 双语功能事实及 interfaces/tool-contract-matrix/process-cutover/standalone-runtime/operations。README 双语 generic managed-process 说明及现有配置指南链接仍正确，故不改；归档历史资料不改。
- 第6轮 fmt/check/clippy/build/parity=0、test=101；Agent 单元613 passed、1 ignored及 config CLI24 passed，随后真实 local_control integration仍发旧args被Unix MCP正确拒绝。FixProcessIntegration已迁移 local_control/standalone_http_mcp/standalone_supervisor 三个正例并显式禁用fixture init；保留政策拒绝等断言，待父统一运行。
- 额外真实Shell Smoke已越过默认/显式/init环境与sandbox场景、late descendant cursor→EOF；active-limit case证明leader终态的live descendant占用唯一槽，随后HTTP cancel502。实际日志 `/tmp/process-shell-cutover-smoke-failure-3909239.log` 显示 event_feedback_delivery_not_confirmed / event_internal_source_not_registered，不将502视为通过。
- FixCancelledFeedback源码定位：此前容量拒绝创建synthetic terminal ProcessInfo却没有注册internal source，response metadata仅凭processId/state构造错误feedback，pending feedback阻断后续cancel。已改只为EventStore实际存在的source生成disposition，覆盖live/replay/recovery；不改变EventSettle failclosed校验。新增容量拒绝/registered completed leader/off policy边界回归，待实际运行。
- 窄ownership review定位：refresh与成功cancel虽清group ID，capture watcher/prune/capacity有observed-absent不撤销路径。Core改为既有processes-map互斥锁拥有的slot，所有probe/signal重新进入同一owner、gone不可复活；不加per-process Arc/Box。quiet descendant即便EOF仍需观察group至消失，避免长期无调用失主窗口；当前停写前不运行Cargo。
- Core map-owned group revocation与EOF后继续retirement观察已停写交付。第7轮 fmt=0，check/clippy/test/build=101，4个纯类型集成错误（mutable pattern guard、两个Detached Option mapper、新回归cancel flag参数）已按实际compiler输出修正；第6 parity仍用旧成功二进制，不作为group/feedback修正后的运行证明。
- Feedback registry lookup使用一条prepared query；数据库/验证错误经`?`传递或拒绝发送response，不降级empty dispositions；empty只有无candidate/无stored command正常情况。不改变EventSettle来源与origin校验。
- 第8轮 fmt/check/build/parity=0，clippy=101（TERM match语法lint及test-only runtime hook使enum再增8B），test=101；Agent单元615 passed、1 ignored，configCLI24及真实localControl1通过，standalone HTTP 3/4通过，stdio printf正例未完成。Core仅修lint/test hook footprint；FixProcessIntegration给字面printf补最窄fixture allow，不放宽生产策略。
- 第8轮新Agent/Hub binary已成功构建，额外Shell Smoke重新执行中；这次实际覆盖map-owned group与registered-source反馈修正，不使用第7轮旧binary作证明。
- 真实Shell Smoke全场景最终exit0：10个Hub-linked Agent及独立Local无确认通道fixture；默认/显式/null/relative/symlink/ENOENT/ELOOP/加载非零、PATH/function/FD3、namespace、脚本/策略/cwd、迟到确认零执行、late输出游标→EOF、active-limit、100-entry retention、leader先退出后取消、SIGKILL升级均达到原断言。最终临时根 `/tmp/process-shell-cutover-m479vwq_`、`/tmp/e-xtgjii07` 已由finally清理。
- 实际host PID终止证据：sandbox 4023645/4023648、active child4024473、cache-aging4024578/4024579、group4025072/4025074、leader-exited child4025097、SIGKILL4025104/4025106均absent；active/completed leader在取消前已exit；对应reads均EOF。未把namespace PID拿来host signal。
- 第9轮 fmt/check/clippy/build/parity=0，test=101；Agent615 passed、1 ignored，configCLI24、localControl1、standalone HTTP4全部通过。随后六个supervisor fixture实际worker stderr均 `local_mcp_socket_path_too_long`，原因测试自建长root名；只缩短唯一私有fixture路径，不改生产100byte Unixsocket限制或削减真实进程断言，准备第10轮。
- 第10轮 fmt/check/clippy/build/parity=0、test=101；六个supervisor仍socket过长。CodeGraph与实际stderr定位路径按HOME拼接，而非config root；缩短config root的初始判断不足，随后将三组worker fixture的agent ID缩至唯一16hex并按实际100byte预算限制，未改生产socket路径/上限。
- 第11轮 fmt/check/clippy/build/parity=0、test=101；Supervisor2 passed、4 failed。socket已ready，实际process.exec返回provider_unavailable且worker日志policyAllow=0；共享run_smoke配置缺字面`/usr/bin/printf` allow，已按现有最窄规则补入，Normal/Room共享生效，保留全部supervisor/journal/reload断言。
- 第12轮 fmt/check/clippy/test/build全部exit0，live parity=1：`Hub crash-recovery completed EventSettle response replay` 未观察到settled response replay（run_668f827adbdf478f86c25a25180b0ede、req_79a27d05871a49379e5b8b3c9c06b402）。前面所有local/HTTP/inbox/timeout/feedback-delta case已通过；不把第5至11轮parity通过当成本轮成功，也不只重跑确认。FixCancelledFeedback调查真实恢复/receipt与fixture时序，保持断言。workspace实际结果：Agent615 passed、1 ignored；Hub138、Protocol23；configCLI24、Unix control1、HTTP MCP4、Supervisor6；apply-patch2、browser-host10。ignored项为既有manual real-package materialization smoke，未宣称其运行成功。
- 已删除本任务自建的两个/tmp Shell/TUI Smoke driver；真实Shell与TUI结果及host PID/EOF证据保留于本记录，完整验收日志保留于`/tmp/agentic-shell-verify-9iltucll`。私有根及自建进程均由各driver/controller清理，不触碰真实用户配置。
- 针对第12轮新恢复失败，父代理只跑真实crash-recovery场景，临时driver在relay failure signal之后暂停parent2s；exit1于EventSettle barrier断言。此before场景实际观察同一run/request两次settled response及单个原始settle command，未看到event.list command；不是“Agent从未重放”的证据。失败signal与baseline/hold的post-wake时序存在窗口，round12具体因果仍[INFERENCE]。诊断日志`replay-before.log`；自建root与进程全部清理，最窄fixture修正进行中。
- 第12轮恢复夹具修正后的第一次targeted run暴露更早的wire→durable误判：relay已看到EventSources但Hub DB sources仍null。复用现有wait_until等待DB sources成为string，再保留原JSON parse/exact identity断言；未增加deadline或改变生产处理。
- 修正后的真实targeted after最终exit0：验收parent故意delay4s，failure signal时settled response count1，parent恢复前已count2，仍使用同一lock捕获的failed-response boundary准确领取replay。原始settle command只一次，held-response barrier、释放后的Hub result/outbox、精确sources、无重复公开事件等全部原断言通过。日志`replay-after-checkpoint.log`，私有root`/tmp/er-3zck1klo`及自建进程已清理；临时diagnostic driver已删除。
- 第13轮完整验收最终全部exit0：fmt(0.79s)、check(0.24s)、strict clippy(0.24s)、workspace test(42.60s)、Agent+Hub build(0.19s)、live contract parity(209.12s)。各步无timeout，owned-session cleanup均all_owned_exited=true。实际日志`round13-1..6.log`；最多20轮内已在第13轮达到全部门禁，不再重跑。

## 完成证据
- command/cwd clean cutover、内部Skill argv分离、普通Bash/pipefail/no-set-e、init三态与typed失败、字面策略/集中确认、独立组取消/准确evidence、旧持久请求退休、统一读取/预算/游标等均已实现；所有受影响入口、消费者与文档已迁移。
- 额外真实Agent/Hub Shell场景全pass，真实PTY配置三态surface/save全pass；有实际host PID消失、late输出cursor→EOF、容量/retention与迟到确认零执行证据。修正后的真实恢复夹具额外delay4s仍全pass。
- 无push/release/tag；未修改真实用户配置/数据或系统Shell。自建临时driver已删除，私有根/进程已清理，保留实际验收日志。本阶段修复、文档与证据由最终本地聚焦提交一次收束。

## 阶段4：Shell配置观测补漏
- 用户真实临时Agent smoke已确认执行、init三态、policy/reload/read通过；报告liveSubsetMatchesDisk漏shell。只读核对根因在agent_info.rs::live_subset字段投影，reload已复制candidate.shell。本次只修比较投影，不改watcher/执行器/轮询/输入契约。
- 用户指定不新增永久回归测试；使用临时HOME/XDG真实Agent及local MCP验证Default、Disabled、Path（含显式默认路径文本）差异，磁盘改动未加载时false、加载后true；清理自建进程/目录/driver，按仓库规则单阶段提交。
- 源码已单行补入`"shell": config.shell`；`cargo fmt --all -- --check && cargo build -p agentic-gpt`成功，实际新Agent构建20.71s。未新增永久测试，未运行回归套件。
- 配置中英文对应页已补充同一观测边界：完整shell三态纳入liveSubsetMatchesDisk，reload前后false→true，只比较配置快照，不检查init文件内容、不证明旧进程切换或强一致生效屏障。
- 临时driver `/tmp/shell_health_smoke.py` 已交付并正在父级执行；真实local Unix MCP持久连接、短私有HOME/XDG/runtime/workspace/配置，按原子写盘逐项改变shell，必须见漂移issue后消失。等待实际exit/断言，不以driver交付当作通过。
- 第一次临时driver实际exit1（0.28s），尚未启动Agent：init_agent生成sparse配置省略sandbox，driver错误要求sandbox object。只修driver用setdefault显式enabled=false；finally覆盖此失败路径并清理私有root，不是生产实现故障。
- 修正后的现场smoke exit0、14.27s：真实新Agent `sh8b23edca`（PID136521）使用`/tmp/sh-ww253k1z`私有HOME/XDG/runtime/workspace及local Unix MCP持久连接。初始Default matchesDisk=true；随后Disabled→Path A→Path B→Default→显式默认路径文本→Disabled→Default七次变更，每次均观察到false与config_live_subset_not_applied，再经真实watcher观察true且issue消失。未mock比较函数、未暂停watcher、未新增永久测试。
- 清理实际证据：Agent returncode=0已wait、MCP连接关闭=true、tempRootExists=false。临时driver已删除；现场输出保留artifact://483。只运行格式检查、新Agent构建与此真实smoke，未重跑旧执行场景或回归套件。
- 本阶段源码只新增shell比较投影，配置双语文档同步对应观测边界；规划/证据与修复在单独本地阶段提交收束，不push/release/tag。

## 阶段5：Process工具描述审查
- 用户报告git status仅残留scripts/__pycache__；不重跑status确认。目录实际只有check_contract_parity.cpython-314.pyc。[INFERENCE] 文件名、时间与上一阶段driver的parity导入方式符合临时smoke残留，但未单独证明首次创建时间。已仅删除该文件并以非递归rmdir清空目录，命令exit0；未改gitignore或其他文件。
- 本轮只读审查Process tools描述，依据documentation-standard.md42-50核对用途、输入/default/约束、输出/状态/错误及副作用/信任边界，兼顾Agent-local与Hub MCP。不改描述/schema/API/测试；实际读取工具表面时使用python -B避免再次产生缓存。
- 主代理亲自读取项目指定OpenAI Define tools原文，亲读Agent/Hub描述、local required分支、batch DTO和并发worker、read EOF计算；子代理只作定位。明确区分必须纠正的batch串行暗示/schema必填遗漏，与面向选择的正文重排建议，不要求所有枚举和错误码进入描述。
- 隔离真实MCP审查完成：两侧均exec/batch/read/list/cancel；local batch required=[]而缺elements调用exit1/-32602，Hub Full required包含agentId/elements。临时driver首次缺subprocess import失败且已清理，修正后exit0；最终Agent/Hub均停止wait，私有root及pycache不存在。仅记录审查与建议，未修改工具/API/测试；公开文档无需改变，因本轮没有实施契约或描述变更。
- 阶段5完成；按项目阶段提交规则，仅提交三份已有规划记录。
