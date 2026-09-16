# Agentic 架构整理计划

## Goal
以现有实现为依据，为“Agent 的受控执行基础设施”形成现状、诊断、目标边界、工程规则及渐进重构计划。仅修改规划与架构文档，不改生产代码。

## Next Step
本轮决策文档已完成。后续代码实施从WP0事实基线开始，遵循D01–D08及真实依赖；不重新审批已确认原则，也不自动开始生产重构。

## Current Phase
Phase 4 — complete

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

### Phase 4: 用户决策收敛
- [x] 确认 Room 受控资源职责与远端能力必须补齐
- [x] 确认一次升级、迁移文档而非兼容双轨
- [x] 保留安全默认与具体副作用控制，不扩大威胁模型
- [x] 保留 Neko/container/共享目录与 Unix socket 实际拓扑
- [x] 分层持久化并接受 Hub 易失 session/cache 重启失效
- [x] Console 维护及新产品从核心重构完成条件移出
- [x] 同步所有正式文档，核验与单独提交
- **Status:** complete

## Decisions Made
- 调查分为执行端、Hub、Console、协议与工程部署四个独立切片。只读 scout 提交证据；Main 整合现状/诊断，两位文档执行者分别拥有目标/工程规则与路线图。
- 源码是当前行为依据；已有文档和历史规划仅辅助说明意图，不能覆盖源码。
- 保留部署边界、协议及现有能力，目标架构不引入长期记忆、Provider orchestration 或 reasoning loop。
- 不以文件大小或目录名直接判定缺陷；每条问题须给出路径/符号、机制、影响、建议及验证门槛。
- 每阶段单独提交；不混入已有用户变更。初始工作区干净。
- 用户已确认的D01–D08以docs/architecture/decisions.md为权威；阶段三的范围讨论是历史基线，不覆盖该决策。

## Errors Encountered
| Error | Resolution |
|---|---|
| 猜测的 skill 绝对路径不存在 | 使用 skill://planning-with-files/scripts 的工具路径解析成功初始化独立计划 |
| 根相对路径检查发现 tests/local_control.rs 简称 | 改完整 crates/agentic-gpt/tests/local_control.rs，最终26具体路径全部存在 |
| 阶段四DAG检查缺少graphlib导入 | 补充导入，仅重跑未完成的DAG与旧语义检查；11边无环 |
