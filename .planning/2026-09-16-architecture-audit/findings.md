# 架构调查发现

## 已核实
- 初始工作区无未提交变更。
- 根目录有 `.codegraph/`，调查优先使用 CodeGraph；宽泛查询召回不精准，后续按具体文件/符号定位。
- `Cargo.toml:1-9` 实际 workspace 为五个 crate：agentic-gpt、agentic-gpt-hub、agentic-gpt-protocol、agentic-apply-patch、agentic-browser-host。上下文中“三个 crate”描述已过时（本阶段不改上下文文件）。
- Console 同时存在共享代码与 Android 本地 attention 持久化/调度，需要区分真实 Hub 集成与本地功能。
- rust-analyzer 可用；Kotlin LSP 未配置。
- 现有 docs 主要为配置、接口、运维、standalone/browser 运行时和历史迁移；正式架构目录尚不存在。计划创建 docs/architecture/ 并从 development 中链接。

## 规模基线（2026-09-16）
`git ls-files` 列出已跟踪文件，Python 统计 `.rs/.kt/.kts/.js/.ts/.py/.sh` 的物理行，包含测试、注释和构建配置，不等于生产代码行数：

| 范围 | 文件 | 行 |
|---|---:|---:|
| crates/agentic-gpt | 67 | 69497 |
| crates/agentic-gpt-hub | 14 | 9965 |
| crates/agentic-gpt-protocol | 1 | 3223 |
| crates/agentic-apply-patch | 6 | 1146 |
| crates/agentic-browser-host | 2 | 866 |
| console（各源集及构建） | 51 | 2868 |
| example/agentic-tui-ux-demo | 3 | 2873 |
| experimental/chrome-control-poc | 1 | 365 |
| scripts | 3 | 238 |

大文件仅为调查入口，不直接作为缺陷证据。stdio_server.rs 6200 行；config_tui/pages.rs 4923 行；protocol/lib.rs 3223 行；browser_distribution.rs 3122 行；Hub mcp_server.rs 3108 行。

## 证据规则
正式调查记录应给出相对路径及关键符号；区分事实、推断、未验证，不把名称重复当作实现重复。

## 依赖与验证基线
- 已运行 `cargo metadata --no-deps --format-version 1 --offline`，成功。agentic-gpt 仅依赖两个本地库 protocol/apply-patch；Hub 仅依赖 protocol；browser-host 无本地 crate 依赖。browser-host 是独立进程边界，不是 agent 的 Rust 库依赖。
- Cargo target 列出执行端四个集成测试入口：config_cli、local_control、standalone_http_mcp、standalone_supervisor；此阶段未执行测试。
- 已阅读双语 development 文档，当前均介绍构建、CI、三个 release 二进制；适合作为长期架构文档入口。

## 正式结论与调查材料勘误
- 正式结论以 `docs/architecture/current-state.md`、`diagnosis.md` 为准，survey 文件是原始证据/建议材料，不是全部采纳的规范。
- 已确认九组诊断：A01多入口合同、A02Room cutover、A03gate、A04Hub身份、A05reload、A06信任保证、A07durability、A08Console成熟度/本地语义、A09文档与验证保证。
- 独立 reviewer 核实主结论成立；纠正现状调用图：只有 Response 走 store_result/pending，JobUpdate 写cache，RunReport upsert receipt。
- Room repositoryRoot 可配置，workspace/room 只是默认，原执行调查将默认写成固定路径处不作为部署规范。
- contract-ops 调查关于 ARM 的负面表述过宽：release 确有 aarch64 cross-build/打包；未运行 ARM 场景不等于未覆盖 ARM 构建。
- 历史 release/migration 的旧数字本身不算漂移；只修当前指引与当前合同冲突。
- CI 的 cargo test --workspace 已运行 existing Rust fixed surface/deterministic corpus；缺口是跨 surface 语义一致性，不是没有任何runtime corpus gate。
- agents-minimal.yaml 应先确认是否为受支持artifact，再决定保留后的gate或退出；不为无消费者历史文件强制增加维护负担。
- 无证据要求不采用：所有纯函数塞进apply-patch、全部Agent Room工具远端化、强制Console联网、签名/provenance新专题、Room实现阻塞Hub身份修复、存储重构阻塞已有合同修复。

## 用户确认（后续会话，取代先前未决建议）
- D01–D08 正式记录于 docs/architecture/decisions.md。Room远端需求已确认，Hub未同步为实现遗漏；先前“仅选定远端consumer才立项”不再适用。
- 支持一次协调升级；公开发布须有迁移文档/步骤，不刻意保留旧alias/shim/双轨；不因迁移授权删除数据。
- 自用可控环境与具体副作用控制并存，不简单二选完全可信/强敌对。本轮保留sandbox、policy override与权限模型。
- 用户真实部署含Neko/container/共享目录/Unix socket；无需重新证明这些环境存在，具体权限/peer/token机制仍待拓扑盘点。
- 正确性事实尽量可靠，history/确认结果/error有retention，audit/report可best-effort；接受Hub OAuth/pending/cache重启失效，不全量durable。
- Android Attention继续独立维护；Console本地和remote产品均不计本轮核心重构完成条件。核心优先Agent/Hub/Protocol/Room。
- 本轮是文档决策同步，不是生产代码实现；目标内部目录形状等由实际seam细化。
