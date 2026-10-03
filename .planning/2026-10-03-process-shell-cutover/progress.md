# 进度

## 阶段1：所有者与共享接口
已完成只读映射：MapShellOwners/MapShellSurfaces/MapShellConfig，证据保留agent输出。核心单owner负责managed/exec/Skill内部argv；独立policy owner仅policy/confirmation/parser模块；配置owner负责配置与CLI/TUI/reload；输入owner负责protocol+Agent/Hub ingress/OpenAPI及对应合同测试；验证owner迁移其余测试与parity请求。

冻结公开DTO：ProcessExecRequest(command:String,cwd:Option<String>,其他字段不变)；ProcessExecElement(command,cwd)；ProcessBatchExecRequest顶层working_directory→cwd，保留继承规则。deny_unknown_fields拒绝旧输入，非输入ProcessInfo历史元数据保持现有协议。内部Skill不使用公开command DTO，核心owner建立内部argv spec并更新Skill。

冻结policy接口：policy::shell_policy_decision_for_profile(config:&Config,profile:CapabilityProfile,command:&str,need_confirm:bool)->PolicyDecision。每个字面调用沿用现有matcher及configured precedence，shell入口无allow匹配的默认Allow提升为Confirm；needConfirm针对整段，不能被单条allow消除。完整解析失败/不支持至少Confirm，但已知deny仍须整体Deny。确认按原始script整体展示；BatchConfirmationElement改为command/cwd。

冻结配置接口：Config.shell:ShellConfig，ShellConfig.init_file:ShellInitFile，ShellInitFile::{Default,Disabled,Path(String)}。Default对应字段省略、Disabled对应null、Path对应显式String（即使与默认路径相同）。serde保持三态，不把省略默认序列化为显式路径。热更新遵循准入配置快照。执行者可match这些variant。
不运行中途build/lint/test/format；全部实现合并后父代理统一最多20轮验收。历史已有实际smoke证明取消child-only后代存活，不重跑旧行为。Context7当前网络不可用，官方Tree-sitter/Codex源码已读供受限提取参考。
