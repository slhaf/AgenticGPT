# 已确认的架构整理决策

状态：**用户已确认，2026-09-16；2026-09-23 复审同步。** 本记录确定本轮产品边界和工程取舍，不表示全局目标架构已经完成，也不批准尚未讨论的具体协议、权限或数据变更。WP1、WP2、WP3、WP4-A 与 WP-R 的有界实现及证据，以及本轮 Process/Job admission snapshot、Process adapter、Skill/path owner、Hub neutral projection、Protocol 内部 facade、Android local Attention transition owner 和 release preflight 的窄边界，已在路线图和现状/诊断文档中分别记录；完成这些包不等于所有结构整理、外部部署或 Console 工作已完成。本地完整 release preflight/live contract gate 已通过；`:shared:jvmTest` 已在 17 tasks 下 BUILD SUCCESSFUL，仅覆盖 shared 纯 `AttentionTransitionPolicy` 测试与 common Kotlin compile，不覆盖 Android app/Room/OS；Android app host test/assemble 仍受 SDK/Maven TLS 阻断，设备/模拟器及 hosted publication/cross-build/ARM/external importer 仍未验证。

适用范围：[目标架构](target-architecture.md)、[工程规则](engineering-rules.md)、[重构计划](refactoring-plan.md)。此前草案中与本记录冲突的“待用户选择”或额外完成条件不再适用。`current-state.md` 与 `diagnosis.md` 既保留修复前的历史事实/根因，也记录已完成闭环、当前残余和未验证边界；目标架构、工程规则和路线图中未标为已完成的内容仍是技术草案或后续工作。WP4-B（Protocol 内部组织）已于 2026-09-23 以私有 wire domain modules + root facade 完成，WP-T 标准仍未决定；Console Android local Attention 的 source transition owner 已实现但 Android host/device/OS 证据仍待补，remote Console 是未来产品。任何完成状态都不替代具体运行证据，也不改变既有五 crate、独立 browser-host、Hub/Agent 资源所有权和部署拓扑。

## D01 — Room 是受控资源，不是 Agent 记忆运行时

- 保留受控文档读取、写入和维护能力，Diary/Notebook/State 是资源结构。
- 上层 Agent 决定写什么、何时整理；Agentic 负责路径边界、请求校验、受控执行、提交、生命周期和结果。
- 不增加主动记忆整理、上下文管理、Provider orchestration 或 reasoning loop。

## D02 — Room 远端能力必须补齐

- 用户确认 Room 能力也需要通过远端提供；WP-R 前 Hub API 未同步主要是实现遗漏，不是有意的产品分层。
- WP-R 已按本决策完成当前九项 Room 能力的 Hub→Protocol→Agent 端到端语义及旧路径 clean cutover；此处保留原需求和实施动机，不表示当前仍需重做。未来新增 Room 操作仍须逐项明确读/维护能力与迁移范围。
- 入口的名称、参数/结果、错误、权限和生命周期投影应逐项明确；远端与本地语义对齐不要求机械复制所有 transport envelope，也不允许扩大既有权限。
- 实际内容和文件/Git 副作用仍由 Agent Room repository 所有；Hub 负责认证、路由、协调与运行收据，不新增 Hub 内容存储。
- 旧接口可随一次升级 clean cutover；不得用仅隐藏工具或把遗漏标为永久不支持代替实现，也不得把旧 append/update 静默伪装成不等价 maintenance。

## D03 — 一次升级、明确迁移，不刻意维持旧兼容

- 部署主要由用户控制，允许 Hub、Agent 和相关调用方协调一次升级。
- 项目已公开发布；有破坏性变化时必须随该代码批次提供迁移文档和可执行步骤，说明版本/产物组合、接口/配置变化、备份、升级顺序、验证及回退边界。
- 默认迁移全部仓库调用方并移除被替代路径；不为了保留历史形状添加长期 alias、shim、双轨执行或额外协议协商机制。
- 迁移文档不等于保留旧服务运行兼容；也不授权删除用户数据。数据格式变更必须保护现有 Room、run/job 和状态数据。
- 本决策形成时只确定迁移要求；WP-R 的产物核验、升级顺序、验证与回退边界已随实现记录在 `docs/operations.md`，未编造固定版本号。未来破坏性变化仍须随其真实实现编写步骤。

## D04 — 自用部署背景与具体副作用控制并存

- 当前主要是用户自部署、自用、可控设备环境；不按公网多租户强对抗系统扩张设计。
- coding agent、MCP server、Browser JS、脚本不能简单视为完全可信；继续对具体副作用提供 policy、confirmation、path boundary 和 lifecycle control。
- 准确区分审批、路径检查、来源校验、OS 隔离和外部执行信任的保证范围。
- 本轮不改变 sandbox 默认、policy override 语义或权限模型。真正不可信 agent/provider 所需的威胁模型与隔离强度，以后单独设计。
- 修复既有检查遗漏不等于授权新增限制或放宽权限；若遇到实际行为冲突，在对应工作包中列证据并明确决策。

## D05 — Browser host 先盘点实际拓扑，再收紧机制

- **用户报告的真实部署**包括 Neko、container、共享目录与 Unix socket，不能把单用户 owner-only 作为唯一目标。
- Browser 工作包先记录进程/用户/容器边界、socket/目录挂载、UID/GID、访问主体与现有连接链，再选择权限、peer credential、token 等机制。
- 目标是明确并尽量收紧边界，同时保留真实部署。共享 socket 并不等于无需保护；也不能为形式上的更安全破坏现有容器协作。
- 当前不预选认证方案，不将本地桥无认证暴露到网络。没有授权扩大为公网多租户部署。

## D06 — 持久化按正确性与用途分层

| 层级 | 事实或记录 | 已确认方向 |
|---|---|---|
| 正确性关键 | 执行身份、幂等/去重、run/job 结果 | 尽量可靠，明确写入失败、崩溃与恢复语义，不让观测数据冒充结果 |
| 可保留历史 | Job history、已产生的确认结果、错误原因 | 尽量保留，可有明确 retention，不要求无限历史 |
| 观测 | audit/report/观测日志 | 可 best-effort，明确可能丢失，不作执行事实或安全证明 |
| 临时会话/投影 | Hub OAuth token、pending confirmation、临时 cache 等 | 接受 Hub 重启失效，不为全量 durable 增加 Hub 复杂度 |

“待确认请求重启失效”与“已产生的确认结果尽量保留”不同。不得把失效的 pending confirmation 变成允许执行，也不得把丢失的 cache 推断为远端任务已停止。具体 retention、落盘/压缩、锁和恢复机制在相关包内按实际需求设计；目前不强制某个新数据库或全量 fsync。

## D07 — Console 与本轮核心重构解耦

- 核心范围优先 Agent/Hub/Protocol/Room 的执行与控制边界，以及相应资源、部署和数据约束。
- Android 本地 Attention 作为现有能力继续维护，但其本地状态机、权限、恢复或数据库整理不是本轮核心完成条件。
- Hub remote console、approval board、exec ledger 等未来另立产品工作；不顺手接网、扩展 Hub 调度职责或把 Console 变成核心重构依赖。
- 核心改动仍须避免破坏既有 Console，未接入的能力仍须诚实展示；这不等于本轮必须修完 Console 的全部局部问题。

## D08 — 不借架构重构扩张范围

先准确描述并修正当前真实边界，不扩大威胁模型、兼容负担、持久化范围或产品职责。必要的新取舍在对应工作包实际碰到时再决策，不把所有未来可能性预先做成框架、门槛或审批清单。

以下由实现者在上述约束下自行决定：crate 内目录组织、函数/enum、窄 operation gate、owner validator、调用方迁移、parity checker 和针对性验证。Room 采用能力域聚合的目录方向属于技术组织建议；具体子文件依真实职责划分，不因此新增通用事务或执行框架。

## 尚需随未来实施细化，而非重新确认的内容

- 当前 Room 九项远端操作及旧路径 cutover 已由 WP-R 交付；未来新增或改变 Room 操作的精确合同与外部调用方迁移，仍按 D02–D03 核查，不重开已完成的九项工作。
- Browser 实际拓扑下可用的 peer/auth 机制；保留 Neko/container/共享 socket 的需求已确认。
- WP3 已交付所覆盖的 correctness/history/observability 分层与具体 retention；未来新增资源或外部效果的失败、恢复和保留规则仍按 D06 随实施细化，不因此要求 Hub 全量持久化。
- WP4-A 已修正其覆盖的公开合同差异；未来新增合同和仍未识别的外部 artifact 消费者需要按实际证据核验，不预设兼容层。
