# 本轮核验记录

## 已执行

1. `git status --short && git diff --stat`：任务开始时工作区干净。
2. `cargo metadata --no-deps --format-version 1 --offline`：成功，确认五 crate 及本地依赖；未编译。
3. 对 `git ls-files` 中 `.rs/.kt/.kts/.js/.ts/.py/.sh` 统计：148 文件、91041 物理行，包含测试和构建/脚本；Rust crate 合计 84697，Console 合计 2868。不是生产 LOC。
4. 通过 Python `yaml.safe_load` 读取 `openapi/hub.yaml`：成功。确认 JobInfo.required 包含 startedAt；JobListResponse 仅允许 jobs、items 引用 JobInfo；NotebookSelectExactRequest 要求 year/month/day、additionalProperties=false。
5. 对照源码 `protocol/lib.rs::JobInfo` 的 started_at Option + skip_serializing_if、`JobListResponse` 的 JobListItem/next_cursor、`JobCancelResponse`，确认静态合同不一致。未连接外部 Actions importer，不声称复现其具体错误消息。
6. 直接核验 `local_service.rs:132-141,275-281` 的旧 Room 拒绝，以及 Hub 调查中的 Full tools/list 注册与 dispatch 链。结论是源码可达链不一致，未启动真实 Hub↔Agent 场景。
7. 直接核验 `agents.rs:327-340`：store_result 的成功返回值不参与 pending.remove 判断。记录为响应所有权完整性风险，未做利用或竞态复现。
8. 直接核验 browser host `handle_client`/`prepare_socket`：socket 0660，接收路径未见 peer UID 检查；实际可访问性还受父目录、组、umask 与容器挂载影响，不宣称任意本地用户一定可访问。

## 尚未执行且不作通过声明

未运行 cargo/Gradle 构建或测试、真实 Android/浏览器/tunnel/Hub/Agent 部署、WS 竞态、OAuth 重启、ARM 发布或网络服务。本轮无生产行为修改，验证目标是架构证据和文档一致性，不是证明整个仓库健康。

## 后续本轮交付检查

正式文档完成后执行本地路径/链接核验、需求覆盖审阅和阶段提交；结果追加于此。

## 文档检查（阶段三进行中）
- Python Markdown 链接检查：8 文件（6架构文档+双语development）、26本地链接，包括锚点；全部存在。
- 具体根相对路径检查初次发现路线图的 `tests/local_control.rs` 使用了 Agent 内相对简称；已交给路线图 owner 改完整路径。不是缺少测试文件，实际文件在 `crates/agentic-gpt/tests/`。
- 目标/规则评审纠正了纯函数归属泛化、Hub工具覆盖过度承诺、Room远端化范围、caller wait与execution deadline混淆、Android/Web权限术语及重复表格。

## 阶段三核验结果（用户确认前的历史基线）
- 修订后 Python 检查：8 Markdown 文件，26 本地链接（含锚点），26 个具体根相对路径，全部通过；4种通配/缩略路径不作机器存在性断言。
- 路线图 Mermaid 中 11 条依赖边通过拓扑排序，无环。Hub 身份、本地 gate/config、合同修复可并行；Room 合同收口独立，Console 本地收口无 Hub 存储前置。
- 独立 reviewer 的5项问题全部处置，记录在 review.md；raw survey 首部加勘误避免旧结论继续传播。
- 最终仅提交规划与架构文档、双语开发文档入口和计划指针；没有生产代码/测试/OpenAPI/配置行为修改。
- 临时核验在 Eval 内存执行，没有留下脚本文件；冗余 hub-survey.json 指针已删除。

## 用户需求覆盖
| 需求 | 交付 |
|---|---|
| 理解现有仓库架构 | current-state：五crate、运行形态、全部一级责任、调用链、数据/安全/部署/Console |
| 判断主要问题与根因 | diagnosis：A01–A09、证据/影响/根因推断/验收、保留项与非目标 |
| 适合当前产品的目标架构 | target-architecture：受控执行定位、五crate拓扑、逻辑边界及不采纳方案 |
| 模块职责/边界/依赖/规范 | target责任表 + engineering-rules放置决策/身份/生命周期/合同/安全/审查 |
| 长期维护文档 | architecture/README维护规则 + 双语development入口链接 |
| 渐进、可验证重构 | refactoring-plan：独立工作包、真实依赖、兼容/回退/验证/停止条件，明确未来未执行 |

不作通过声明：Rust/Gradle测试、真实Hub/Agent/浏览器/Android/tunnel、外部Actions importer、ARM运行、竞态/攻击/故障注入。

## 阶段四：用户决策同步

本次新增decisions.md并同步目标/规则/路线图。上面的阶段三记录只保留历史事实，不是当前范围：D02使Room远端补齐进入核心，D07使Console本地维护及远端产品均退出核心完成门槛。

- Python检查9个Markdown文件（7架构文档+双语development入口）、51个本地链接（含锚点）、26个具体根相对路径，全部通过；4种通配/缩略路径排除机器存在性断言。
- 新版Mermaid的11条依赖边拓扑排序通过；核心图包含WP-R，不含WP5/Console。WP4-B明确是后置可选维护；核心发布要求WP0/WP1/WP2/WP3/WP4-A/WP-R。
- 检查并移除已废弃的“限选Room远端能力”“M-R可选”“M4含WP5”等措辞；人工核对路线图范围、前置、里程碑、最终完成/停止条件及编号，与D01–D08一致。
- 集成时去除远端Room合同必须保持旧wire的多余约束、SQLite只能additive的预设，并区分正确性记录/已产确认结果与允许失效的pending session；不预设新的兼容或全量持久化负担。
- DAG检查首次缺少graphlib导入；补充导入后重跑未完成部分通过，未掩盖失败，也未重复已通过的链接/路径检查。
- 所有核验只在Eval内存运行，没有临时脚本或生产代码/测试/OpenAPI/配置修改。本次没有运行构建、运行时场景或故障注入；文档通过不代表未来重构实现已通过。
