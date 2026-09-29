# 进度

- 范围包括架构目录 7 篇、superpowers 3 篇、根及 docs 现行指南、双语版本发布／迁移记录、OpenAPI、Console、测试贡献指南、实验 PoC 与随产品编入的 Room/Skill Markdown；`.planning/` 内部任务记录不纳入产品文档删除候选。
- 四个只读切片已完成并核实重点引用：旧 superpowers 实施计划仓内仅审查记录文字提及；旧 config-init 设计被后继设计明确部分覆盖；`diagnosis.md` 在架构目录有多个入口；`agents-minimal.yaml` 由现行文档引用且外部消费者未盘点。
- Room scaffold 的六个 Markdown 模板由 `room_repository.rs` 直接 `include_str!` 编入，不可作为闲置文档删除；版本材料和 Process cutover 的升级／数据安全说明也有存留价值。
- 分级结论：旧 `docs/superpowers/plans/2026-08-04-interactive-config-initialization.md` 是最明确的可移除旧执行清单；`openapi/agents-minimal.yaml` 待外部消费者调查后再考虑移除；旧向导设计和架构诊断可冻结为历史，后继全屏设计、当前架构规则／目标／路线图和全部在用指南保留。局部失准需修正，不等于整体淘汰。
- 本轮未删除或修改产品文件，未执行构建或测试。
