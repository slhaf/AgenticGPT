# 现行文档中文重建

## 目标
按已确认的自主目标清理并重建现行文档，保留历史设计证据与旧版迁移记录；不改代码或 OpenAPI 契约。

## 阶段
1. 核定文档切片、归档路径及跨文件引用：已完成。
2. 并行重建现行使用说明、架构文档，迁移旧设计：已完成。
3. 集成链接与事实审查，运行目标所列验证并完成阶段提交：进行中。

## 归档与所有权契约
- 将 `docs/architecture/diagnosis.md` 移至 `docs/archive/architecture-diagnosis-2026-09-23.md`，把仍有效的残余和根因概览留在架构现状/路线图。归档后新路径由所有架构作者使用。
- 将两份已完成 config-init 设计移至 `docs/archive/config-init-wizard-design-2026-08-04.md` 和 `docs/archive/config-init-fullscreen-tui-design-2026-08-07.md`，标注历史身份、后继与当前配置指南；仅删除旧 `docs/superpowers/plans/2026-08-04-interactive-config-initialization.md`。
- 保留版本化迁移／发行说明的版本事实与原路径；现行 `docs/process-cutover.md` 要改为中文。
- `README.md` 与 `README.zh-CN.md` 等原有对应页仍保留并更新为中文；不新增临时转发页。所有受影响相对链接由集成者统一审查。
- 根 README/Console/测试说明、配置/开发、Standalone/Browser、接口/工具/运维/迁移、架构现状/规则/目标、架构决策/路线图/归档分别由独立作者拥有。交叉链接指向本节目标路径，作者不得修改其他切片或提前运行构建/测试/格式化。

## 失败限制
最多三轮针对失败验证的修正；遇外部消费者、独有决策丢失、需碰禁止范围则停并说明。
