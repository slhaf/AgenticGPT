# 开发指南

本文面向维护者和编写代码的 agent，说明如何从请求定位代码、沿现有边界实现行为并验证结果。正常安装和使用请从[项目入口](../../README.zh-CN.md)开始。对应页面：[英文开发指南](README.md)。发布专属步骤见[发布指南](releasing.md)；本页不重复 tag 或发布工作流。

## 修改前：确认行为与所有者

开始编辑前先把请求改写为可观察结果，并核实：

1. **入口**：调用方实际触达 CLI、Agent MCP/Unix、Hub HTTP/Apps MCP、Hub-Agent wire、Console 屏幕还是其他入口？入口适配器各自保留认证、协议、错误与响应封套。
2. **效果**：哪个服务、进程、数据库、文件、调度器或平台适配器真正执行副作用？不要只改转发层或展示层来模拟行为。
3. **状态**：配置、运行时状态、持久记录、缓存投影、审计证据分别由谁拥有？确认并发写入、恢复、取消、过期和清理者。
4. **合同与消费者**：沿调用链识别 DTO/schema、权限门、错误投影、CLI/TUI、Hub 与 Console 消费者；只更新受影响接口面，不为对称性新增无用入口。
5. **观察方式**：选定返回值/错误、持久状态、进程生命周期、UI 状态或 wire/HTTP 投影作为验证结果。仅测试 helper 不能证明消费者可见行为。

`docs/archive/` 仅供历史追溯，不是当前架构规范或待办来源。实现事实以当前代码和对应契约为准；下文的开发约定不会把实现差异宣称为统一强制规则。项目文档写作规则见[文档标准](../documentation-standard.md)。

## 模块边界与代码放置

Rust workspace 当前有五个 crate，成员见根目录 [`Cargo.toml`](../../Cargo.toml)：`agentic-gpt`、`agentic-gpt-hub`、`agentic-gpt-protocol`、`agentic-apply-patch`、`agentic-browser-host`。当前依赖方向为 Agent → protocol、apply-patch；Hub → protocol。Protocol 放共享 wire/domain DTO 与序列化合同，不承担数据库、网络、授权或业务副作用。[apply-patch](../../crates/agentic-apply-patch/src/lib.rs) 是纯 patch 解析/定位/文本更新算法库；[browser-host](../../crates/agentic-browser-host/src/main.rs) 是独立扩展桥接程序，不是 Agent runtime 或 Hub 的状态所有者。调整依赖前检查 crate manifest 和实际调用，避免反向依赖或把桥接进程逻辑塞入核心 crate。

Agent 与 Hub 均以二进制 crate 根显式装配许多私有模块；目录不是 Cargo crate，且部分模块通过 `#[path]` 映射。以下是当前代码的定位图，不意味着每种改动都要同时改所有层：

| 子系统 | 当前职责与常见放置位置 |
| --- | --- |
| Agent runtime | [`runtime/`](../../crates/agentic-gpt/src/runtime/state.rs)：共享运行态、启动、supervisor、instance lock；`state.rs` 聚合各所有者的共享状态。 |
| Agent ingress | [`ingress/`](../../crates/agentic-gpt/src/ingress/stdio_server.rs)：stdio MCP、Unix local control、Hub/HTTP transport 与 OAuth adapter；每个入口保留自己的认证、协议和错误边界。工具参数 schema 在 [`stdio_schema.rs`](../../crates/agentic-gpt/src/ingress/stdio_schema.rs)。 |
| Agent operations | [`operations/`](../../crates/agentic-gpt/src/operations/operation.rs)：真实入口准入/授权、共享本地服务、命令策略、确认与结果映射。它不是无差别接管所有工具入口的通用 dispatcher。 |
| Agent process | [`process/`](../../crates/agentic-gpt/src/process/managed.rs)：受管理进程生命周期、执行预检、输出/取消；历史记录 adapter 在 [`storage/process_history.rs`](../../crates/agentic-gpt/src/storage/process_history.rs)。 |
| Agent files / MCP | [`files/`](../../crates/agentic-gpt/src/files/file_ops.rs) 管文件操作；[`mcp/`](../../crates/agentic-gpt/src/mcp/mcp.rs) 管下游 MCP 客户端/调用与并发，不把任意工具请求等同本地工具。 |
| Agent skills / Room / Browser | [`skills/`](../../crates/agentic-gpt/src/skills/skills.rs) 与 [`skill_installs.rs`](../../crates/agentic-gpt/src/skills/skill_installs.rs) 管 skill 发现、执行和安装；[`room/`](../../crates/agentic-gpt/src/room/room_repository.rs) 管 Agent 本地 Room 读取/维护；[`browser/`](../../crates/agentic-gpt/src/browser/browser_runtime.rs) 管 Browser runtime 与发现/分发。 |
| Agent storage / config / UI | [`storage/`](../../crates/agentic-gpt/src/storage/private_state.rs) 管私有状态、传输账本与审计；[`config/`](../../crates/agentic-gpt/src/config/config.rs) 管配置；[`ui/`](../../crates/agentic-gpt/src/ui/cli.rs) 含 CLI、TUI、配置 TUI。 |
| Hub runtime / agents | [`runtime/`](../../crates/agentic-gpt-hub/src/runtime/state.rs) 管 Hub 运行态与服务器；[`agents/`](../../crates/agentic-gpt-hub/src/agents/dispatch.rs) 管 Agent registry、连接生命周期、派发与运行回执协调。Hub 派发命令，Agent 执行副作用。 |
| Hub ingress / storage | [`ingress/http/`](../../crates/agentic-gpt-hub/src/ingress/http/routes.rs) 管 HTTP/OpenAPI 路由；[`ingress/mcp/`](../../crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs) 管 Apps MCP；[`storage/`](../../crates/agentic-gpt-hub/src/storage/runs.rs) 与 [`db.rs`](../../crates/agentic-gpt-hub/src/storage/db.rs) 管 Hub 持久运行回执和数据库。两种 ingress 的身份与响应封套不同。 |

Agent crate 根 [`main.rs`](../../crates/agentic-gpt/src/main.rs) 可查完整 `#[path]` 装配。新增模块沿用所在 crate 的根装配、可见性与命名；先确认调用方和 `pub` 边界，再决定位置，不因目录存在就新增 crate，也不为单项能力预建通用框架。

Console 是独立 Kotlin Multiplatform 工程，当前 Gradle 项目为 `shared`、`androidApp`、`desktopApp`、`webApp`。平台无关规则放 `shared/src/commonMain`，纯规则测试放 `shared/src/commonTest`；Android SDK、Room、通知与 receiver 留在 Android source set/适配器。Android、Desktop、Web app 各有自身 source set；不要从 common 依赖 Android SDK。目标平台与任务见[Console 开发说明](../../console/README.md)。

## 从请求到可观察行为

1. **复现并缩小范围。** 记录入口、当前响应/状态和预期差异；读接口、配置、运维文档以及对应实现。核对直接调用方、持久化 schema、共享 DTO 与固定工具表面测试，不使用归档诊断或过时索引路径代替源码。
2. **确定所有者与合同。** 判断改动影响本地行为、远程 wire/HTTP/MCP/API 合同还是 UI 投影。列出数据如何进入所有者、效果在哪执行、结果如何返回；沿用现有 DTO、适配器和错误类型。不要把所有本地能力远程化，也不要让多个传输共用一个抹平语义的封套。
3. **沿现有方向实现。** 入口适配器解析输入、验证、认证/准入并映射值；领域/服务所有者执行行为；持久层或平台适配器持有资源。迁移所有调用方，删除已替换路径，不保留无用别名或兼容层。
4. **定义生命周期。** 说明状态创建、并发访问、过期、取消、重启恢复和删除；在真实消费边界限制资源。读写运行时配置、进程/任务、连接代际或 UI 投影时检查锁、异步等待与失败路径。
5. **同步实际受影响接口面。** 按需更新 Agent-local、Hub wire、HTTP/OpenAPI、Apps MCP、CLI/TUI、Console 和用户文档。公共合同变化要核对消费者与权威 schema；本地能力不因存在远程入口而自动复制到 Hub。
6. **验证消费者行为。** 为边界和失败路径选有意义测试，再运行相应真实入口或隔离冒烟。单元测试证明局部规则，不证明传输、CLI/TUI 启动、数据库迁移或 OS 效果。

## 状态、并发、生命周期与安全资源

状态应有明确的单一写入所有者。只在必要范围内持锁；不要持锁等待网络、接收器、任务完成或可能重入的辅助函数。先识别现有锁顺序和并发约定；跨连接或异步请求更新状态时核验代际与所有权，防止迟到消息覆盖当前状态。

对异步任务、子进程、MCP 调用和 UI 排程，明确谁启动/持有/回收资源；区分超时结束等待与实际取消；说明协作取消、终态证据和所有者重启后的状态。超时、缓存缺失或请求者离开不等于副作用已取消。为进程、批次、并发请求、输出/结果、文件/网络载荷及缓存设置与操作相称的上限；拒绝或达到上限时不得伪造成功或留下未说明的部分副作用。已有业务限制见[接口说明](../interfaces.md)和[工具合同矩阵](../tool-contract-matrix.md)，不要把单个数字套用到所有功能。

需分清具体权威，而非把所有记录都叫“缓存”或“审计”：

- Agent 受管理进程的执行状态由进程所有者维护；[`process_history.rs`](../../crates/agentic-gpt/src/storage/process_history.rs) 持久化进程历史。Hub 进程 cache 是可过期的状态投影，不会变成执行事实、输出/结果来源或取消证据。
- Hub [`storage/runs.rs`](../../crates/agentic-gpt-hub/src/storage/runs.rs) 的持久 receipt 保留派发身份、去重与迟到结果协调；它不是进程状态缓存，也不证明尚未观察到的副作用已停止。
- Skill install 记录由 [`skill_installs.rs`](../../crates/agentic-gpt/src/skills/skill_installs.rs) 持久化并定义恢复/终态；Skill 执行仍归受管理进程。执行持有每 skill shared lease，安装替换使用同 key exclusive lease，避免运行与替换重叠；两者不可互相替代。
- 审计记录说明发生过的操作证据；例如 [`storage/audit.rs`](../../crates/agentic-gpt/src/storage/audit.rs) 不应被当成可变业务状态或秘密仓库。修改记录内容、保留或敏感字段前需核对其消费者与安全边界。

安全分为**入口、操作、效果**三层：入口验证身份/认证和传输来源；运行时操作准入按真实 ingress、配置、profile/toolset/能力判定；副作用执行前应用命令策略、路径限制、确认与资源所有者检查。工具描述符、MCP annotations、可见性过滤和模型提示只提供描述/发现信息，绝不是授权。对外部内容、路径、网络目标、命令及重定向做运行时校验；不要夸大现有策略为通用 OS 沙箱。见[接口说明](../interfaces.md)与[运维指南](../operations.md)。

## 增加配置字段

从现有结构与相邻字段模式实现，不要只往任意 JSON 写一个键：

1. 在[配置结构与加载](../../crates/agentic-gpt/src/config/config.rs)确定 section、类型、serde 缺省、默认构造值、旧稀疏配置兼容行为及范围/跨字段校验。`Config.extra` 不代表业务字段已实现。用户可见键与示例见[配置说明](../configuration.zh-CN.md)。
2. 若需 CLI 编辑，在[配置键 registry](../../crates/agentic-gpt/src/config/config_keys.rs)补 key、类型/说明与验证 setter，沿用[配置 CLI](../../crates/agentic-gpt/src/config/config_cli.rs)的现有路径；适用时同步初始化模板、校验、回顾/隐藏逻辑。不是每个内部字段都需要 CLI/TUI 控件。
3. Agent 所有配置写者都必须通过 `acquire_config_mutation_lock` 获取共享变更锁，并使用 `write_config_with_backup` 写入；不要新增绕过锁和写入 helper 的配置修改器。具体锁与原子/备份写入实现见 [`config.rs`](../../crates/agentic-gpt/src/config/config.rs)。
4. 在[启动和 reload](../../crates/agentic-gpt/src/runtime/startup.rs)确定字段启动时读取、每请求读取、可热重载还是需重启；只有进入 live reload 子集且通过校验才热更新。已经准入的 Process/Skill 操作持有当时的配置快照；reload 后的新请求才采用新配置，不得让运行中的旧操作悄悄改变策略。
5. 检查 CLI/TUI 显示、safe summary、日志、诊断、审计和协议摘要。秘密沿用 secret reference/解析机制，不能把凭据明文放进参数、日志、summary 或固定测试数据。测试默认、旧文件、无效/边界值、拒绝写入，以及实际消费者在生效/reload 后的行为。完整配置细节放配置参考，不复制 schema。

## 本地操作、Hub 命令与远程接口

Agent-local operation 是本地入口调用 Agent 所有者的行为，可供本地 MCP/Unix 或 CLI 使用，不因此自动变成远程命令。Hub wire command 是 Agent 与 Hub 共用的显式协议 DTO/envelope，由 Hub 派发、Agent 执行；可靠回执必须保留 run/request/hash 等身份和迟到结果语义。Hub HTTP/OpenAPI 与 Apps MCP 各自拥有认证、输入 schema、状态码/错误投影及响应封套；不能混为同一接口，也不能混淆 Apps `AgenticResult` 与 HTTP 扁平响应。

改某个接口面时沿真实调用链更新消费者和测试：wire 改动需更新 protocol、Agent/Hub mapping 与序列化/回执测试；HTTP 改动需更新 handler、`openapi/hub.yaml` 与 parity；Apps MCP 改动需更新工具 schema/handler/响应测试；本地 MCP 改动需更新本地 schema/dispatch/准入与固定工具表测试。只改实际受影响接口面及共享所有者，不机械远程化本地操作。权威入口见[接口说明](../interfaces.md)、[工具合同矩阵](../tool-contract-matrix.md)和[OpenAPI](../../openapi/hub.yaml)。

新增 Hub 命令时，按以下顺序核对：

1. 在相应领域的 Protocol 文件中定义请求/响应（例如进程请求位于 [`process.rs`](../../crates/agentic-gpt-protocol/src/process.rs)），更新 [`HubCommand`](../../crates/agentic-gpt-protocol/src/envelopes.rs) 及 Hub 入口映射；不要把所有领域的 DTO 都放入进程文件。
2. 检查 Agent [`operation.rs`](../../crates/agentic-gpt/src/operations/operation.rs) 中的 `hub_command_name`、`operation_namespace` 与操作分类，再核对 `authorize` 是否在真实入口、运行模式、profile 和工具集下正确准入。Room 操作还须检查 `is_current_room_operation`、`is_hub_room_toolset_operation` 及各自的准入分支。名称或前缀相同不等于分类已经更新。
3. 更新 [`local_service.rs`](../../crates/agentic-gpt/src/operations/local_service.rs) 的消费与结果映射分支，接入实际资源所有者。若暴露 MCP 工具，另核对其描述符中的只读、破坏性和开放世界注解；这些注解不能替代运行时授权。
4. 覆盖允许、禁止及能力未启用的真实入口路径。新增 Room 命令尤其要验证普通 Agent、工具集关闭和活动 Room 的差异，而不是只证明 DTO 能序列化。

## 编码与测试

Rust 遵循 `rustfmt`，函数/变量/模块使用 `snake_case`，类型/trait 使用 `PascalCase`，常量使用 `SCREAMING_SNAKE_CASE`；Protocol 保持 DTO/序列化边界，不塞入业务副作用。沿用所在 crate 的错误传播、异步和模块装配约定。Kotlin 使用四个空格缩进；common 规则放 common source set，平台 SDK 和资源留在对应平台 source set，不从 common 引入平台依赖。新增抽象须解决真实重复或所有权问题，不为假设中的未来平台预建框架。

Rust 单元测试放模块 `#[cfg(test)]` 测试模块或现有测试文件，异步测试沿用 `#[tokio::test]`；集成测试放 crate 的 `tests/`。Kotlin 测试放对应 source set 的 `commonTest`/平台 test 路径，并用 `kotlin.test.Test` 等既有方式。跟随目标模块现有布局，不为同一行为制造第二套惯例。

测试要捕捉消费者可见行为、边界、错误、状态转移、并发不变量和合同差异，而非仅断言调用、转发、mock echo、非空输出或偶然默认值。持久化要测重开/冲突/迁移；并发要测竞争与原子性；授权要覆盖 ingress 差异和拒绝；wire/schema 要测形状及实际 handler。纯测试不替代高风险入口冒烟。

删除测试前先列出其断言和能捕捉的具体失败，记录**哪些具名保留测试**已覆盖相同失败，或说明原断言为何仅固定实现细节、非契约措辞或偶然默认值。输入相似、覆盖率重叠不足以证明冗余。无法排除独有且仍有效的失败模式时，保留测试或先补等价行为测试；不能以清理为名丢失消费者可见回归的检测。

| 代表场景 | 实际实现链接 | 测试/验证入口与应观察结果 |
| --- | --- | --- |
| 配置 limits 字段及边界 | [配置结构](../../crates/agentic-gpt/src/config/config.rs)、[键 registry](../../crates/agentic-gpt/src/config/config_keys.rs)、[进程所有者](../../crates/agentic-gpt/src/process/managed.rs) | [`tests/config_cli.rs`](../../crates/agentic-gpt/tests/config_cli.rs)：`cargo test -p agentic-gpt --test config_cli`；模块测试见 [`runtime/main_tests.rs`](../../crates/agentic-gpt/src/runtime/main_tests.rs)。观察缺省/旧文件解析、非法值不写盘、热重载后续请求使用新值、已准入进程仍用快照，以及消费端在真实上限的拒绝/准入。按字段现有限制选边界，不虚构统一阈值。 |
| `process.exec` 跨接口面 | [Protocol request DTO](../../crates/agentic-gpt-protocol/src/process.rs)、[Agent 授权](../../crates/agentic-gpt/src/operations/operation.rs)、[共享本地服务](../../crates/agentic-gpt/src/operations/local_service.rs)、[stdio schema](../../crates/agentic-gpt/src/ingress/stdio_schema.rs)、[受管理执行](../../crates/agentic-gpt/src/process/managed.rs)、[Hub HTTP 路由](../../crates/agentic-gpt-hub/src/ingress/http/routes.rs)、[Hub Apps MCP](../../crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs) | 固定 Agent 工具表：`cargo test -p agentic-gpt normal_and_room_tool_sets_follow_fixed_surface_contract`；Hub 回执重放：`cargo test -p agentic-gpt-hub pending_replay_sends_reliable_envelope`；HTTP/OpenAPI schema 与 live handler：`target/contract-venv/bin/python scripts/check_contract_parity.py`。固定工具表和重放测试本身不证明执行了一次，也不证明所有接口封套一致；需观察受管理执行、实际返回/错误及各接口面自己的投影。Parity 覆盖 HTTP/OpenAPI，不替代本地 MCP/Apps MCP 全覆盖。 |
| Console Attention 到期与终态 | [common transition policy](../../console/shared/src/commonMain/kotlin/work/slhaf/agentic/console/domain/attention/AttentionTransitionPolicy.kt)、[Android state holder](../../console/androidApp/src/main/kotlin/work/slhaf/agentic/console/attention/AttentionListStateHolder.kt)、[Room/runtime coordinator](../../console/androidApp/src/main/kotlin/work/slhaf/agentic/console/platform/attention/AttentionRuntimeCoordinator.kt) | [`AttentionTransitionPolicyTest.kt`](../../console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt)，从 `console/` 运行 `./gradlew :shared:jvmTest`。观察 `dueAt - 1` 仍为 Schedule、到达 `dueAt` 为 Trigger、重复 transition 不再触发、终态清空 actions 且不重复终结。若改 Room 条件更新或平台调度，还要测相应适配器/DAO。 |

表中测试名与命令是源码中可定位的现有入口，不表示本页改动已运行测试；定点 Rust filter 无匹配时不能将空跑视为验证。

## 本地验证命令与范围

以下是在仓库根目录运行的本地验证流程。Rust 步骤与当前 [CI 工作流](../../.github/workflows/ci.yml) 对齐；Python 部分采用隔离虚拟环境，不是 CI 命令的逐字复制：

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p agentic-gpt -p agentic-gpt-hub
python3 -m venv target/contract-venv
target/contract-venv/bin/python -m pip install "PyYAML" "jsonschema[format]>=4.25,<5"
target/contract-venv/bin/python scripts/check_contract_parity.py
```

`fmt` 检查格式，`check` 编译 workspace，严格 `clippy` 将 warning 视为错误，workspace `test` 运行 Rust 测试，Agent/Hub 构建为 live parity 准备二进制。Clippy 是 CI gate，可能报告现有 finding；这些命令不证明实际 CLI、UI、OS 效果或生产部署行为。Parity 的隔离虚拟环境是本地等价流程：CI 当前直接用 `python3 -m pip install ...`，不是字面采用上述 venv 命令顺序。Parity 检查 OpenAPI schema 与 live behavior 两类结果，默认调用刚构建的 `target/debug/agentic-gpt`、`target/debug/agentic-gpt-hub`，也可传 `--agent-bin PATH`、`--hub-bin PATH`。它使用隔离 loopback/private-home 进程，不覆盖生产凭据、外部部署或所有平台行为。

其他已核实的定点 Rust 命令：

```bash
cargo test -p agentic-gpt normal_and_room_tool_sets_follow_fixed_surface_contract
cargo test -p agentic-gpt deterministic_tool_contract_corpus_exercises_public_dispatch
cargo test -p agentic-gpt-hub pending_replay_sends_reliable_envelope
cargo test -p agentic-gpt-hub generation_stale_reliable_messages_do_not_touch_current
cargo test -p agentic-gpt --test config_cli
```

Console Gradle 任务须从 `console/` 运行：`./gradlew :shared:jvmTest`、`:shared:testAndroidHostTest`、`:shared:jsTest`、`:shared:wasmJsTest`。`:androidApp:assembleDebug` 只构建 APK，不会安装/启动。Desktop/Web 运行任务见[Console 开发说明](../../console/README.md)；当前 Rust CI 不含 Gradle 任务。

无副作用 CLI 冒烟可运行 `cargo run -p agentic-gpt -- --help` 或 `cargo run -p agentic-gpt-hub -- --help`。不要为验证直接运行会初始化或启动服务的命令，除非 `HOME`、配置目录及其他状态根都明确指向一次性临时目录且清理范围已确认；绝不要把测试配置写进真实 `~/.agentic_gpt/`。完整 CLI/TUI 或 Console UI 验证须实际启动入口并观察输出、交互和状态；Gradle 单测或 assemble 不证明屏幕可用。不能运行目标平台时，应说明该平台行为未验证。

## 完成检查与升级决策

- [ ] 已核实入口、效果所有者、状态所有者、直接调用方及受影响契约。
- [ ] 实现沿用当前模块边界，无重复路径、伪 dispatcher 或无用兼容代码。
- [ ] 锁、生命周期、取消证据、容量、持久化/缓存/审计权威和失败路径已明确。
- [ ] 更新适用运行时授权、秘密处理、实际消费者与用户文档。
- [ ] 测试能捕捉消费者可见回归；已运行匹配的定点测试和入口冒烟。
- [ ] 双语对应页事实一致；权威 schema/参考通过链接引用。

局部小改动沿现有所有者与契约完成即可。若改动影响公共 wire/HTTP/MCP/CLI 合同、持久格式或状态所有权、信任/权限边界或跨平台行为，应先列出兼容性、迁移/恢复、安全失败语义与全部消费者，再更新相应所有者和契约。现有模式不足以确定新行为时，要明确未决点而非擅自引入新抽象。配置见[配置说明](../configuration.zh-CN.md)，接口见[接口说明](../interfaces.md)，工具矩阵见[工具合同矩阵](../tool-contract-matrix.md)，部署运行见[运维指南](../operations.md)，发布见[发布指南](releasing.md)。
