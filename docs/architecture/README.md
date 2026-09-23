# Agentic 架构指南

本文档集面向维护者和 coding agent。产品定位：**Agent 的受控执行基础设施**，不是完整 Agent Runtime。

## 阅读顺序与文档状态

先读[已确认决策](decisions.md)：Room 远端能力补齐、一次升级与迁移文档、安全默认不变、真实容器/共享 socket 部署、分层持久化，以及 Console 与核心重构解耦。

1. [现状架构](current-state.md)：基于源码的部署、模块、数据和调用链快照。
2. [问题诊断](diagnosis.md)：已观察到的结构风险、根因判断和保留项。
3. [目标架构](target-architecture.md)：建议采用的职责边界；不表示代码已经完成迁移。
4. [工程规则](engineering-rules.md)：代码放置、依赖、协议、安全、生命周期与审查规则。
5. [渐进重构计划](refactoring-plan.md)：每批前置条件、非目标、验收和回退约束。

截至 2026-09-23，本目录进入“历史基线 + 已完成有界包 + 当前复审”状态：WP1、WP2、WP3、WP4-A 与 WP-R 已分别记录其有界实现和证据；这不等于全局目标架构已经迁移。`diagnosis.md` 同时保留修复前 A01–A09 的机制与根因，并标注闭环和残余边界。`target-architecture.md` 与 `engineering-rules.md` 仍是目标/技术规则，`refactoring-plan.md` 是含完成状态的路线图；WP4-B（Protocol 内部组织）仍待处理，WP-T 标准仍未决定。Console Android local Attention 继续独立维护，remote Console 是未来产品，不是核心 Rust 完成条件。本轮只做文档同步，不新增运行验证声明。既有接口可按一次升级 clean cutover，但必须迁移调用方并随实现交付迁移文档，不刻意维持兼容双轨。

## 2026-09-23 复审导航

| 原始目标 | 当前结论 | 主要入口 |
|---|---|---|
| 实际全仓架构 | 五个 Rust crate、独立 Console、OpenAPI、脚本/CI、部署及实验/示例边界均纳入；外部 tunnel、Browser client/service、Android OS 等仍按证据边界标注 | [现状架构](current-state.md) |
| 问题与根因 | A01–A09 的历史证据保留；已完成包不再作为现行缺陷，当前结构残余单列并标明推断与验证门槛 | [问题诊断](diagnosis.md) |
| 合适目标 | 保留 Agent 执行核心、Hub 控制面、Protocol/apply-patch/browser-host 与部署拓扑；不预设新 crate、通用 dispatcher 或推理/记忆运行时 | [目标架构](target-architecture.md) |
| 职责、依赖与规则 | 以资源事实所有者、入口适配器、跨进程合同和持久化层级约束放置；HTTP/MCP 认证与投影仍各自负责 | [工程规则](engineering-rules.md) |
| 持久开发指导 | 新能力按 owner、ingress、effect、authority、迁移与证据逐项盘点；Room 内容仍归 Agent，Hub 只持 lease/receipt/projection | [工程规则](engineering-rules.md) · [已确认决策](decisions.md) |
| 渐进、可验证的计划 | 已完成包与未来包/独立 Console 工作分开；下一结构工作须有窄范围 owner、行为验收和回退边界 | [渐进重构计划](refactoring-plan.md) |

## 事实、推断与决策

- **事实**：以仓库相对路径和关键符号定位的当前实现；源码与历史规划冲突时以源码描述现状。
- **推断**：由多个事实推导的原因、成本或风险；不等于已复现故障或已确认漏洞。
- **已确认决策**：用户明确选择的产品范围、部署约束和工程取舍，见 decisions.md；不等于代码已经实现或运行验证通过。
- **建议**：目标边界和迁移方案；必须通过兼容、安全与行为验证才能实施。
- 文档不是逐行代码审计，不宣称证明没有缺陷；调查覆盖和未验证范围见现状文档。

## 新增功能前的判断顺序

1. 是否为上层 Agent 提供受控执行、资源访问、审批或运行观测？若是长期记忆、模型选择、上下文组装或 reasoning loop，先说明为何不应由上层承担。
2. 谁拥有资源及生命周期：本地执行端、Hub 控制平面、Console 平台适配，还是独立 browser host？
3. 接口是否跨进程、跨语言、跨发布版本？先检查协议和既有客户端，不能只修改一个入口。
4. 是否改变确认、路径、命令、网络或凭证边界？先列明授权前后的行为，不以“重构”掩盖权限扩展。
5. 找到目标职责对应现有模块，再按工程规则修改；不要为套用架构模板先增加 crate、service 或通用框架。

## 长期维护方式

- 改变模块责任、持久化所有权、依赖方向、公共协议、部署单元或产品边界的 PR，必须同步更新相应文档。
- `current-state.md` 随已落地代码更新；目标文档只记录仍有效的规则，不把迁移中间态描述为已完成。
- 新的重大边界决策记录“背景、选择、替代方案、迁移影响、验证”；已由用户确认的取舍统一维护在 decisions.md，目标/规则/路线图同步更新，不为同一问题重复设审批前置，也无需为每个函数建立决策文档。
- `.planning/2026-09-16-architecture-audit/` 是本次调查进度，不是长期规范入口；历史 `.planning/`、`docs/superpowers/` 是设计背景，不能凌驾于当前实现及本指南的状态说明。
- 使用说明仍归现有 [接口](../interfaces.md)、[配置](../configuration.zh-CN.md)、[运维](../operations.md) 与 [开发](../development.zh-CN.md) 文档，不在本目录复制一份 API 手册。
