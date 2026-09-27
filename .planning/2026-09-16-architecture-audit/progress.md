# 架构整理进度

## 2026-09-16 / 阶段一：调查与诊断
- 初始化独立计划 architecture-audit；初始工作区干净；CodeGraph-first，四个只读 scout 并行调查。
- 保存 execution-survey.md、hub-survey.md、hub-supplement.md、console-survey.md、contract-ops-survey.md。它们是原始调查材料，不是全部采纳的规范；个别风险定级/建议已在正式诊断中收窄，正式文档优先。
- 创建 docs/architecture/README.md、current-state.md、diagnosis.md：覆盖五 crate、全部一级责任、多入口流程、持久化/安全/部署/Console成熟度与九组问题。
- 确认：主要问题是跨入口合同/gate、连接/run/waiter身份、配置与资源生命周期；不是缺少完整 Agent Runtime。
- 运行 Cargo metadata、源码规模统计、OpenAPI YAML解析；定点对照 Room拒绝、Response waiter和browser socket。证据和未执行项目详见 verification.md。
- 阶段一提交包含调查与规划，不含生产代码；目标/规则与路线图由两个独立文档 owner 并行写作，现状/诊断独立审阅中。

## 工具错误
- 首次猜测 skill 绝对路径不存在；改用 skill:// 路径解析成功。

## 变更边界
- 不运行真实配置初始化、远程设备或用户数据操作；不改生产代码、安全默认、协议或部署。
- 历史 migration/release 数字不自动改写为当前值；Console 未实现Hub接入属于成熟度，不作已回归漏洞。

## 阶段二：目标与工程规则
- 阶段一提交：`e9d2b93`。
- 两个文档 owner 并行形成目标/工程规则与未来路线图；Main 对目标/规则进行了边界审查并收窄建议。
- 目标保留五 crate/部署拓扑，优先 crate 内 operation gate、owner/identity、入口投影；明确 Browser host 独立进程、Room资源与 Android Jetpack Room 不同、Console local-only。
- 修正草案中“所有纯函数归 apply-patch”、Hub 已有 file/Browser 全表面、强制全部 Room 工具远端化、caller timeout 混同执行期限等不当泛化。
- 删除目标路径表与规则末尾的重复清单；长期入口链接加入双语 development 文档。
- 目标与规则是完整规范草案，不是已获用户批准的行为变更；不改生产代码。

## 阶段三：路线图、核验与交付
- 阶段二提交：`66ef019`。
- 完成路线图：WP0基线；WP1Hub身份；WP2本地gate/config与Hub接线；WP3所有权/耐久性；WP4-A合同修复；WP-R Room公开合同收口；WP5 Console本地语义。Protocol内部模块组织及Console Hub新产品分别列为可选，不作为合同修复前置。
- Main移除不必要的串行依赖、强制全部Room远端化/新增version flag/re-export/provenance要求；保留已有消费者需要的明确兼容边界。
- 独立review核对主结论并提出5项勘误；current-state调用链/Room路径已修，原始调查材料加勘误；全部处置见review.md。
- 最终机器检查：8文件/26本地链接/26具体路径全部通过；11依赖边无环。没有构建或生产运行验证声明。
- 核验脚本仅在Eval内存运行；冗余临时JSON指针已删除。正式证据、计划、进度与review保留，用户可从docs/architecture/README.md继续。
- 本轮完成，仅文档变更；后续代码工作不自动开始。

## 阶段四：用户决策收敛
- 用户确认产品/工程取舍，追加第四阶段；继续复用原plan，不重建模板。恢复时工作区干净。
- 新增 docs/architecture/decisions.md，D01–D08区分已确认原则与仍待实施细节；Main同步入口/现状补注/诊断。
- 两个文档owner并行同步目标+规则与路线图，分文件独占；禁止生产修改及中途验证。
- 重点移除旧语义：Room远端可选、兼容双轨/强制版本机制、重新选择sandbox默认、强制owner-only、全量持久化、Console核心完成门槛。
- Room目录仅为技术建议，未搬文件；maintenance local/workflow均可能有本地/远端副作用，preflight不是纯函数，不预建通用transaction框架。
- 两个owner已handoff；Main同步核心WP-R、一次升级、真实共享拓扑、分层durability及Console排除条件，移除多余wire/DB兼容约束，保留具体机制到实施时细化。
- 最终文档检查9文件、51本地链接、26具体路径通过；新版11条依赖边无环，Room在核心图、Console不在核心图。阶段三数字与范围保留为历史，阶段四详见verification.md。
- 本阶段仅形成确认后的决策及规划文档，未实施代码、迁移或部署；无临时核验脚本残留。作为第四阶段单独提交。
