# 进度

## 阶段1：所有者与共享接口
已完成只读映射：MapShellOwners/MapShellSurfaces/MapShellConfig，证据保留agent输出。核心单owner负责managed/exec/Skill内部argv；独立policy owner仅policy/confirmation/parser模块；配置owner负责配置与CLI/TUI/reload；输入owner负责protocol+Agent/Hub ingress/OpenAPI及对应合同测试；验证owner迁移其余测试与parity请求。

冻结公开DTO：ProcessExecRequest(command:String,cwd:Option<String>,其他字段不变)；ProcessExecElement(command,cwd)；ProcessBatchExecRequest顶层working_directory→cwd，保留继承规则。deny_unknown_fields拒绝旧输入，非输入ProcessInfo历史元数据保持现有协议。内部Skill不使用公开command DTO，核心owner建立内部argv spec并更新Skill。

冻结policy接口：policy::shell_policy_decision_for_profile(config:&Config,profile:CapabilityProfile,command:&str,need_confirm:bool)->PolicyDecision。每个字面调用沿用现有matcher及configured precedence，shell入口无allow匹配的默认Allow提升为Confirm；needConfirm针对整段，不能被单条allow消除。完整解析失败/不支持至少Confirm，但已知deny仍须整体Deny。确认按原始script整体展示；BatchConfirmationElement改为command/cwd。

冻结配置接口：Config.shell:ShellConfig，ShellConfig.init_file:ShellInitFile，ShellInitFile::{Default,Disabled,Path(String)}。Default对应字段省略、Disabled对应null、Path对应显式String（即使与默认路径相同）。serde保持三态，不把省略默认序列化为显式路径。热更新遵循准入配置快照。执行者可match这些variant。
不运行中途build/lint/test/format；全部实现合并后父代理统一最多20轮验收。历史已有实际smoke证明取消child-only后代存活，不重跑旧行为。Context7当前网络不可用，官方Tree-sitter/Codex源码已读供受限提取参考。

## 阶段2：实现源码交付（尚未运行验收）
CoreExecution已交付managed/exec/Skill内部argv，Bash control channel独立于stdout/stderr；Default实际sandbox内open ENOENT-only skip，显式/非零加载失败Failed+shell_init_file_failed，正常脚本任意退出code仍沿用Completed；初始化后cwd重设。TERM→KILL组取消含leader终态后台child，捕获与组收敛前保留runtime。
ShellConfiguration已交付三态serde/CLI/TUI/live reload/import；`config unset shell.initFile`只用于此新键恢复Default，其他键不可unset。PublicProcessInput已交付所有输入schema/adapter/OpenAPI；ConsumerMigration已迁移其余fixture/parity。PersistedArgvCutover保持旧argv raw/hash/completed result，旧未完成退休不重放。
独立审查修正见findings.md；parser三处词法完整性缺陷正在收敛。新Smoke位于/tmp/process-shell-cutover-smoke.py，覆盖init三态/symlink/error/cwd、脚本/策略/确认、真实PATH及函数、group升级/leader退出/EOF，追加sandbox.enabled实际场景。没有运行check或声称通过。
本机`bwrap --help`/`--version`实际证实0.13.0且无--preserve-fds；官方源码确认child可继承fd，Core已移除不存在参数，未新增挂载。现有bwrap未启用--new-session。
父代理验证日志根：/tmp/agentic-shell-verify-9iltucll；既有隔离Python依赖可用，等待源码停写后统一格式与完整验收。
