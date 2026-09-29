# Agentic 架构指南

本文档集面向维护者和编码代理。产品定位：**Agent 的受控执行基础设施**，不是完整 Agent Runtime。

## 阅读顺序与文档状态

先读[已确认决策](decisions.md)：Room 远端能力补齐、一次升级与迁移文档、安全默认不变、真实容器/共享 socket 部署、分层持久化，以及 Console 与核心重构解耦。

1. [现状架构](current-state.md)：基于源码的部署、模块、数据和调用链快照。
2. [历史架构诊断](../archive/architecture-diagnosis-2026-09-23.md)：保留 A01–A09 的调查证据与根因判断；不是当前缺陷清单。
3. [目标架构](target-architecture.md)：建议采用的职责边界；不表示代码已经完成迁移。
4. [工程规则](engineering-rules.md)：代码放置、依赖、协议、安全、生命周期与审查规则。
5. [渐进重构计划](refactoring-plan.md)：每批前置条件、非目标、验收和回退约束。

截至 2026-09-29，`current-state.md` 是当前源码架构快照，记录已完成的有界包、仍存的结构边界和未验证范围。A01–A09 的历史调查与根因见[归档诊断](../archive/architecture-diagnosis-2026-09-23.md)，已闭环事项不得当作当前缺陷。`target-architecture.md` 与 `engineering-rules.md` 是目标/技术规则，`refactoring-plan.md` 是带完成状态的路线图。

WP-T 的测试增删标准见根目录 [`AGENTS.md`](../../AGENTS.md)，清理工作仍按该标准逐项进行；WP5-O 仍是未来产品。2026-09-23 的整合记录包含 Rust workspace 测试与本地发布预检结果；这些带日期的历史记录不表示本次文档重建已验证，也不作为当前测试通过声明。

同日 `:shared:jvmTest` 的 17 项任务仅覆盖 shared 测试与 common Kotlin 编译，不覆盖 Android app、Room 或 OS。严格 Clippy 的既有债务不属于 release preflight；GitHub 托管发布、交叉编译和 ARM 运行仍未验证。

## 历史复审索引（2026-09-23）

| 原始目标 | 2026-09-23 复审结论（当前状态以现状架构为准） | 主要入口 |
|---|---|---|
| 实际全仓架构 | 五个 Rust crate、独立 Console、OpenAPI、脚本/CI、部署及实验/示例边界均纳入；当前 Process/Skill/Hub projection/Protocol/path/Android seams 已落在既有 crate 内，外部 tunnel、Browser client/service、Android OS 等仍按证据边界标注 | [现状架构](current-state.md) |
| 问题与根因 | A01–A09 的历史证据保留；已完成包不再作为现行缺陷，配置、Process routing、Hub projection、Protocol、Skill/path 与 Android 的当前闭环单列，仍有发布/外部边界 | [历史架构诊断](../archive/architecture-diagnosis-2026-09-23.md) |
| 合适目标 | 保留 Agent 执行核心、Hub 控制面、Protocol/apply-patch/browser-host 与部署拓扑；已实现的窄 seam 不扩成新 crate、通用 dispatcher 或推理/记忆运行时 | [目标架构](target-architecture.md) |
| 职责、依赖与规则 | 以资源事实所有者、入口适配器、跨进程合同和持久化层级约束放置；中立 Hub projection 已由 state projection 提供，HTTP/MCP 认证与最终投影仍各自负责 | [工程规则](engineering-rules.md) |
| 持久开发指导 | 新能力按 owner、ingress、effect、authority、迁移与证据逐项盘点；Protocol root 仅作 facade，Skill lease/digest、路径归一化、Android transition 各有当前 owner；Room 内容仍归 Agent | [工程规则](engineering-rules.md) · [已确认决策](decisions.md) |
| 渐进、可验证的计划 | 已完成包、当前源码收口、待证发布门和未来包/独立 Console 工作分开；下一结构工作须有窄范围 owner、行为验收和回退边界 | [渐进重构计划](refactoring-plan.md) |

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
- `.planning/2026-09-16-architecture-audit/` 是历史调查进度，不是长期规范入口；其他历史 `.planning/` 和 [`docs/archive/`](../archive/) 的设计资料仅保留背景，不凌驾于当前实现和本指南的状态说明。
- 使用说明仍归现有 [接口](../interfaces.md)、[配置](../configuration.zh-CN.md)、[运维](../operations.md) 与 [开发](../development.zh-CN.md) 文档，不在本目录复制一份 API 手册。
