# 评估证据

- [docs/architecture/README.md](../../docs/architecture/README.md) 将现状、诊断、目标、规则、路线图和决策分别定位；部分是当前维护入口，不能因历史快照混入而整体删掉。
- 架构重构计划明确保留后续未决/未来包，诊断明确保留修复前根因和当前残余；两者需判断长期成本与独有证据。
- 其余文档按当前指南、版本归档、非规范契约副本及设计草案分别评估。

## 版本与附属材料

- `openapi/hub.yaml` 为现行 Actions 契约并受 parity 脚本消费；`openapi/agents-minimal.yaml` 为明确非规范历史副本，删除前必须盘点未枚举的外部使用者及文档引用。
- `docs/process-cutover.md` 保留真实的升级、旧数据库和回滚边界，三个现行架构页面引用；不宜仅因版本演进删除。
- 版本化迁移指南和发行说明保留旧部署的兼容性历史，README 有入口；应冻结为版本记录而不是刷新成当前描述。
- `tests/tool-contract-cases/README.md` 为测试贡献说明，局部 `$fixtureRevision` 指引失准应修订而不是删除；`experimental/chrome-control-poc/README.md` 与存留 PoC 一起去留。内置 Skill 的 `SKILL.md` 被二进制编入，不能当普通可选说明删除。

## 当前指南与 superpowers

- `README` 双语入口、配置、Standalone、Hub 接口、工具矩阵、运维、开发、自托管浏览器、Console 和新文档标准各承担不同当前用途；局部失准应修订，不是整份移除理由。
- `docs/superpowers/` 共三份：8 月 4 日 `interactive-config-initialization.md` 是基于旧 `inquire` 的未执行勾选清单，当前 `.planning/2026-08-08-config-init-fullscreen-tui/` 与配置文档接管实现流程，故为明确删除候选。仓内仅审查记录文字提及旧计划文件名；删除可保留审查说明但不再指向真实文件。
- 8 月 4 日 `config-init-wizard-design.md` 的交互部分被 8 月 7 日全屏设计明确替代，其余模式／模板／密钥安全约定曾有独特设计背景；适合冻结为历史或摘录背景后归档，不作为当前指引。8 月 7 日全屏设计保留独特批准理由和交互设计，状态“待实现计划”已失准，应改为已实现的设计记录。

## 架构目录

- 保留现行入口 `README.md`、事实地图 `current-state.md`、边界规则 `engineering-rules.md`、已确认决策 `decisions.md`；这四类信息互补且均有在用链接。`target-architecture.md` 是明确标为目标草案的设计参考，`refactoring-plan.md` 仍列出未来工作／接受门槛，也不宜整份删除。
- `diagnosis.md` 的 A01–A09 历史根因与残余分析有审计价值，但作为每日维护状态源价值低，适合冻结为有日期的归档；如需移除，须先迁出独有证据与仍有效的 R01–R06，并更新架构入口等引用。
- 多页仍称 WP-T 标准未决，与 `refactoring-plan.md:98` 的已确立状态冲突；现状/目标中的 `storage/job_history.rs` 路径、工程规则中的 CLI tmux 绕过 gate 声明同样是局部陈旧，不构成整份删除理由。

## 产品内置 Markdown

- `crates/agentic-gpt/assets/room-scaffold/Diary/{Daily,Weekly,Monthly}/current.md` 及 `manual/{diary,notebook,entity}.md` 均由 `room_repository.rs:29-64` 的 `include_str!` 编入并作为 Room 初始化模板；不是可随意删除的外围说明。
- `crates/agentic-gpt/skills/skill-installer/SKILL.md` 也直接嵌入内置技能。其余未分配 Markdown 未发现新的独立删除候选；`.planning/` 是内部阶段证据，不作为产品使用文档误删。
