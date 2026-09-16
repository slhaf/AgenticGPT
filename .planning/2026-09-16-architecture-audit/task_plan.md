# Agentic 架构整理计划

## Goal
以现有实现为依据，为“Agent 的受控执行基础设施”形成现状、诊断、目标边界、工程规则及渐进重构计划。仅修改规划与架构文档，不改生产代码。

## Next Step
本轮文档任务已完成。后续从 refactoring-plan.md 的 WP0 行为基线开始；公开兼容、权限及产品范围决策另行确认，不自动启动代码重构。

## Current Phase
Complete

## Phases
### Phase 1: 调查与诊断
- [x] 明确产品边界、工作范围及证据格式
- [x] 完整描述实际架构与关键调用链
- [x] 诊断主要问题及根因，区分事实与推断
- [x] 提交调查文档
- **Status:** complete

### Phase 2: 目标架构与规范
- [x] 制定适合当前产品的目标架构
- [x] 明确模块职责、依赖方向和工程规则
- [x] 建立长期文档入口及更新规则
- [x] 提交目标架构文档
- **Status:** complete

### Phase 3: 渐进计划与核验
- [x] 制定分批重构、兼容约束、验证门槛与回退策略
- [x] 核验文档证据、引用、覆盖面及相互一致性
- [x] 提交最终计划及核验记录
- [x] 清理临时指针，保留正式证据与勘误，不留下临时脚本
- **Status:** complete

## Decisions Made
- 调查分为执行端、Hub、Console、协议与工程部署四个独立切片。只读 scout 提交证据；Main 整合现状/诊断，两位文档执行者分别拥有目标/工程规则与路线图。
- 源码是当前行为依据；已有文档和历史规划仅辅助说明意图，不能覆盖源码。
- 保留部署边界、协议及现有能力，目标架构不引入长期记忆、Provider orchestration 或 reasoning loop。
- 不以文件大小或目录名直接判定缺陷；每条问题须给出路径/符号、机制、影响、建议及验证门槛。
- 每阶段单独提交；不混入已有用户变更。初始工作区干净。

## Errors Encountered
| Error | Resolution |
|---|---|
| 猜测的 skill 绝对路径不存在 | 使用 skill://planning-with-files/scripts 的工具路径解析成功初始化独立计划 |
| 根相对路径检查发现 tests/local_control.rs 简称 | 改完整 crates/agentic-gpt/tests/local_control.rs，最终26具体路径全部存在 |
