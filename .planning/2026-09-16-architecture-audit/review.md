# 独立审阅与处置

审阅范围：current-state、diagnosis 及四份 survey，按源码定点核验；未运行构建、测试或真实部署。

## 结论

审阅确认拓扑/mode/profile、Room legacy、Job/OpenAPI、Hub pending owner、reload、browser-host/manager 区分、sandbox 保证与 Console 成熟度的主结论有静态源码依据。初次审阅报告列出五项实质错漏，不应将该初次 verdict 视为修订后文档状态。

| 问题 | 处置 |
|---|---|
| Response/JobUpdate/RunReport 共用 store_result/pending 的错误链图 | current-state 已拆为 Response→store_result/pending；JobUpdate→cache；RunReport→upsert_agent_report |
| Room路径误写为固定 workspace/room | current-state 改配置 repositoryRoot、默认 workspace/room；execution-survey 首部加勘误 |
| ARM误称发布未覆盖 | 正式现状/目标明确 aarch64 cross-build/打包，未运行ARM场景；contract-ops-survey首部加勘误 |
| 历史 migration/release 旧数字误作当前漂移 | 正式诊断/规则明确历史基线不改写；raw survey首部加勘误 |
| agents-minimal未确认消费方即要求CI gate | 路线图改先确认支持状态，保留的合同才补gate，退出的artifact不新增维护负担 |

## Main 追加的设计审阅

- apply-patch 仅拥有patch算法，不是全仓所有纯函数的容器。
- Hub现有surface不等同Agent全部file/Browser能力；保留被明确选择的Room远端合同，不强推所有本地工具远端化。
- caller wait与resource execution deadline分开；合法late result不能被整体禁用。
- Room迁移、Console本地收口、Hub身份修复、HTTP合同修复拆分真依赖；不把存储重构或Protocol搬家作为合同修复前置。
- 既有Rust corpus已被workspace test覆盖，缺口是跨surface语义；不把新gate当作填补完全不存在的测试。
- 草案移除重复表格、强制provenance、新feature flag/version字段/re-export等无consumer依据的要求。

以上均为文档修订；源码风险未修复，也未宣称运行复现通过。
