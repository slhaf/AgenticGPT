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
