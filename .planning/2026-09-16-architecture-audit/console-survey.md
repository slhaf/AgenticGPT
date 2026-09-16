# Console 调查证据

## 1. 一级模块与实际能力

### Console shared

`console/settings.gradle.kts` 包含 androidApp、desktopApp、shared、webApp。`console/shared/build.gradle.kts` 配置 `jvm()`、`js { browser() }`、`wasmJs { browser() }` 和 Android library；commonMain 依赖 Compose/Lifecycle/Coroutines，未引入 Ktor、OkHttp、serialization、Hub protocol 或网络实现。

`shared/src/commonMain` 的 `App.kt:7-11` 只有 `App() -> AgenticPlaceholderApp()`；`navigation/AgenticNavigationScaffold.kt` 只有 Material3 导航壳和两行 placeholder 文案。commonMain 确实有 `AttentionItem`、`AttentionRepository`、`AttentionScheduler`、`AttentionSection`、`AttentionCard` 等结构，但 repository/scheduler 只是端口。`AttentionSourceKind` 有 LocalMock、Hub、User，`ScheduleMode` 有 MockOnly、LocalNotification、LocalAlarm、Degraded、Failed；没有远端调度模式，也没有实际 Hub source producer。

`androidMain`、`jvmMain`、`jsMain`、`wasmJsMain` 当前都只实现 Platform expect/actual：Android 返回 SDK 名称，JVM 返回 Java 版本，JS 读取 user agent，Wasm 返回 Web with Kotlin/Wasm。它们没有 Attention 或 Hub 平台 adapter。

### Android

`MainActivity.onCreate()` 调用 `AndroidAgenticApp()`。`AndroidAgenticApp.kt` 组装 `AttentionDatabase.create(context)`、`AndroidRoomAttentionRepository`、`AndroidAttentionScheduler`、`AttentionListStateHolder`、权限 reader、`ReminderNotificationService` 和 `AttentionRuntimeCoordinator`，再把 Attention/Settings destinations 交给导航壳。`AttentionScreen` 通过 `stateHolder.state.collectAsState()` 显示列表、过滤器、状态分组和 due time；`AttentionListStateHolder` 处理 done、acknowledge、cancel、固定时长 snooze、创建 mock、清理 mock。

Android 是目前唯一具备真实业务 side effect 的 Console target：Room 持久化到 `agentic_attention.db`；未来 item 使用 AlarmManager；到期后由 notification service 发系统通知；通知 action、全屏 AlarmActivity、开机恢复和 app 启动 restore 均走 runtime coordinator。这里的 mock 按钮只 mock 数据来源，当前实际注入的是 Android scheduler，所以会产生真实 Android 本地通知/闹钟。

### Desktop/Web

`desktopApp/main.kt` 创建 Compose Desktop Window 后直接调用 shared `App()`；`webApp/main.kt` 用 `ComposeViewport` 后同样调用 `App()`。没有 AndroidDestinations 等价物，没有 Attention state/repository/scheduler、Hub client、本地持久化或浏览器网络。它们是可启动的 UI 壳，不是 Android 功能的跨平台实现。

### Demo 与生产 TUI

`example/agentic-tui-ux-demo/Cargo.toml` 自带 `[workspace]`，不属于根 Cargo workspace；只依赖 ratatui/crossterm 等终端绘制库。`src/main.rs` 的页面、Review、Identity、Limits、搜索、视觉变体和 process rows 都是本地内存值；源码反复标出 Demo 不会写真实配置文件，Review 的 Hub report/process 内容也是静态预览。`tests/state.rs` 只测 `AppState` 的 focus/selection/border 转移。

生产 TUI 位于 `crates/agentic-gpt/src/config_tui` 和 `src/tui`，与 demo 没有共享状态或组件。`config_cli::handle_init()` 在真实 TTY 中调用 `run_config_tui()`；`SetupSession`、Navigation、字段验证、Review redaction、可注入 Committer 最终走 `commit_wizard_outcome()`，写配置并按权限处理 secret。生产 Process TUI 另走 `LocalJobClient` 到本机 Unix MCP socket 的 `job.list`，不是 Hub HTTP。Demo 与生产 TUI 在 Mode/Profile/Review 等概念上相似，但属于原型与实现的概念重叠，不是代码重复或可直接替换的实现。

## 2. 关键调用/状态生命周期链

### 链 A：Android UI -> state -> Room 与本地调度

`MainActivity.onCreate` -> `AndroidAgenticApp` 依赖组装 -> `AndroidDestinations` -> `AttentionScreen.collectAsState` -> `AttentionCard` action -> `AttentionListStateHolder`。State holder 一侧调用 `AndroidAttentionScheduler.cancel/schedule`，另一侧异步调用 `AndroidRoomAttentionRepository`；repository 通过 `AttentionDao` 将 domain 映射到 Room，`observeItems()` Flow 再回到 Compose。Room 是当前数据权威，AlarmManager 只是 OS side effect。

创建 mock 的实际链是 Settings debug button -> `createMockReminder/createMockAlarm` -> `AttentionDemoData` 生成 `source=LocalMock` item -> Android scheduler -> Room upsert。`AttentionDemoData.initialItems()` 生成的五条初始数据没有全仓调用者，因此数据库默认可为空，必须按 debug 入口创建数据。

### 链 B：启动/开机恢复 -> Room -> AlarmManager

`AndroidAgenticApp` 的 `LaunchedEffect` 或 `AttentionBootRestoreReceiver` -> `AttentionRuntimeCoordinator.restoreFutureItems()` -> `AttentionDao.queryPendingForRestore()` -> domain mapper -> `AndroidAttentionScheduler.schedule()`。查询只选 Waiting/Snoozed 且 `dueAtEpochMillis > now` 的 item；无 Hub 请求，也没有 overdue reconciliation。

### 链 C：系统触发/通知 action -> Receiver/Activity -> coordinator -> Room + Notification

AlarmManager 的 PendingIntent -> `ReminderNotificationActionReceiver.onReceive()` 的 `goAsync` -> `AttentionRuntimeCoordinator.handleNotificationIntent()`。fire 路径先找 Room item、标记 Triggered、再由 `ReminderNotificationService` 发通知；Done/Acknowledge 取消通知并更新 terminal state；Snooze 取消旧通知、更新 Room dueAt/status、再 schedule 新 item。全屏 alarm 进入 `AlarmActivity`，按钮仍回到 coordinator。此链完全本地，没有远端 acknowledgement 或 Hub control。

### 链 D：实际 Hub 的远程链，用来界定 Console 缺口

Hub main 注册 `/v1/info`、`/v1/agents`、`/v1/jobs`、runs、process、tmux、MCP、notify 等路由。action route 经过 `require_action_auth()` 校验 `Authorization: Bearer <Hub api key>`；需要 agent 的请求经过 enabled/capability 检查；`agents::request_agent()` 选择 hello-ready、CommandCapable 的 WS/SSE connection，`runs::prepare_run()` 写 SQLite run、记录 pending，随后发送 envelope；Agent response 由 `runs::store_result()` 幂等落库并唤醒等待者。Console 搜索不到 HTTP/WS client、Authorization/Bearer、fetch、Ktor/OkHttp，Manifest 也没有 INTERNET，因此这条真实 Hub 链与 Console 没有调用边。

### 链 E：生产 TUI 与 demo 的副作用边界

生产配置链是 CLI TTY 检查 -> `config_tui::run_config_tui` -> `TuiApp::config` terminal loop -> `ConfigTuiApp`/`SetupSession` -> validation/review -> Committer -> secret/config 文件。生产 Process 链是 `main.rs` poller -> `LocalJobClient` -> UnixStream/RMCP `job.list` -> `ProcessScreen`。Demo 则是 `main` -> 内存状态 -> draw/key loop -> 清理终端；没有 repository、transport、committer 或文件写入。

## 3. Hub 真集成 vs placeholder/mock/local

- **真实 Hub backend**：`crates/agentic-gpt-hub/src/main.rs`、`routes.rs`、`agents.rs`、`runs.rs`、`notify.rs` 和 `openapi/hub.yaml` 共同证明服务器路由、认证、Agent forwarding、run persistence 和通知 channel 存在。
- **Console Hub placeholder**：`HubConnectionCard` 仅画 URL/API token field；`AndroidSettingsScreen.HubConnectionSection` 用 Compose `remember` 保存它，明确显示未连接且不会发网络请求，测试按钮是空 lambda。值没有进入 repository/client/header，也没有安全持久化。
- **Android Hub notification placeholder**：Hub `notify_channels` 对 Android notice/alarm 返回 unavailable；`android_notify_register` 虽会存 endpoint，却返回 `deliveryImplemented=false`。`openapi/hub.yaml` 也没有对应的 android/register path（服务端代码有该额外路由）。Console 没有 register caller。
- **local**：Android Room + AlarmManager + Notification + boot receiver；这是独立本地 attention 功能。生产 TUI 的 Unix socket 是另一条同机 local-control transport。
- **mock**：`AttentionDemoData` 的 LocalMock item 和 Settings debug 入口；`MockAttentionScheduler`/`InMemoryAttentionRepository` 是未被 Android 生产组装使用的旧 seam。

结论：attention/reminder 当前应明确归类为独立本地功能，而不是 Hub remote control。`AttentionSourceKind.Hub` 只是未来数据模型预留；没有 Hub create/sync/action/ack 生命周期。

## 4. 持久化、所有权、安全、部署约束

1. Android Room version 1、`exportSchema=false`、数据库 singleton 由 app context 创建；AlarmManager/Notification 不保存完整权威状态，重建依赖 Room。
2. Receiver 与 AlarmActivity 是跨进程入口，使用 `goAsync`/IO coroutine；Boot restore 是 best effort。权限受 Android API、Doze、精确闹钟授权、通知授权和 full-screen policy 影响。
3. Room mapper 用字符串保存 enum/action；未知值会静默 fallback，未来接入 Hub source 或协议扩展会有数据语义损坏风险。
4. Manifest 声明 POST_NOTIFICATIONS、SCHEDULE_EXACT_ALARM、RECEIVE_BOOT_COMPLETED、USE_FULL_SCREEN_INTENT，但没有 INTERNET；Android Hub client 未来必须补齐网络权限、TLS/cleartext policy、token 存储与错误状态。
5. Hub action auth 是 Bearer API key，Agent WS/SSE auth 是 `x-agent-secret`；Console 的 API Token 字段尚未绑定任何协议。Web 还需处理 HTTPS、CORS、WS/SSE 生命周期，Desktop 要明确使用远端 Hub 还是同机 Unix socket。
6. Console Hub token 现在是普通文本 field 和短生命周期 Compose state；因没有网络/落盘，当前不是已发生的传输泄漏，但在真正接入前必须采用 Keystore/平台安全存储并在 UI 掩码。生产 TUI 的 `SecretValue`/redacted Review 是可参考的安全边界。
7. Android application 的 `allowBackup=true` 使含标题/消息的本地 Room 数据受到系统备份策略影响；当前未发现 Hub token 写入 Room。

## 5. 主要问题（证据、机制、影响、严重度）

### P1：Hub 连接完全没有 transport（High）

证据：`console/androidApp/.../settings/AndroidSettingsScreen.kt:63-76` 的连接 section 明确写不会发起网络请求，测试按钮 `onClick = {}`；`HubConnectionCard.kt:15-41` 只有 field；shared Gradle 无网络依赖，Manifest 无 INTERNET。机制是 URL/token 不进入 client、header、repository 或 connection state。影响是无法列 agent、读 job/run、发 notification 或执行受控 action。根因是 `[推断]` UI 形状先于 transport contract 落地，Hub 字段被保留为占位。

### P2：Desktop/Web 只有 placeholder，功能不具备平台 parity（High）

证据：`shared/.../App.kt:7-11` -> `AgenticPlaceholderApp()`；Desktop/Web main 都直接调用 `App()`；Attention/Settings destinations 只在 Android 注册。机制是没有 state holder、repository、scheduler、Hub client 或 persistence 注入。影响是 Desktop/Web 即使成功启动也只展示静态壳。根因是 `[推断]` Android spike 尚未回接为跨平台 app state/port。

### P3：restore 丢失 overdue item（High）

证据：`AttentionRuntimeCoordinator.restoreFutureItems()` 与 `AttentionDao.queryPendingForRestore()`，其中 SQL 条件为 `dueAtEpochMillis > now`。机制是重启、进程被杀或错过调度后，已经过期的 Waiting/Snoozed 行既不重新调度，也不转为 missed/failed。影响是提醒可能永久留在 pending 状态而不再通知。根因是 `[推断]` 只实现未来闹钟重建，没有定义 overdue reconciliation。

### P4：ExactRequired 被静默降级为 inexact 且报告 accepted（High）

证据：`AndroidAttentionScheduler.schedule()` 把 `ExactPreferred`、`ExactRequired` 一起交给 `scheduleExactPreferred()`；无 exact permission 或 SecurityException 时使用 `setAndAllowWhileIdle()`，返回 accepted=true、mode=Degraded。机制是 required 约束没有失败分支。影响是用户/未来控制流程会把非确定性闹钟误当成满足 deadline。根因是 `[推断]` 当前 MVP 选择尽量安排而不是实现严格策略语义。

### P5：ScheduleResult/异步 repository 错误没有进入 UI 状态（Medium-High）

证据：`AttentionListStateHolder.kt:30-74` 忽略 `scheduler` 返回的 `ScheduleResult`，repository 写入只放入 `scope.launch`；scheduler 在权限缺失或失败时已有 `Failed` 结果。机制是 Room 状态与 OS side effect 非事务，失败也可能继续持久化。影响是 UI 呈现已创建/已 snooze，但设备可能没有可交付通知，且没有重试或可见 degraded 状态。根因是 `[推断]` domain 已有结果类型，但还没有完成 operation state machine。

### P6：Snooze 和 action policy 在多个层次漂移（Medium）

证据：state holder 固定 5 分钟；Settings 文案是 5 分钟；`ReminderNotificationService` 与 `AttentionRuntimeCoordinator` 对 Reminder 使用 10 分钟、Alarm 使用 5 分钟；`AttentionDemoData` 的 Reminder metadata 默认 15 分钟；Compose `AttentionCard` 按 `item.actions` 绘制，而 notification service 固定添加 primary+snooze action。机制是 UI、domain metadata、通知 profile、Receiver 常量各自定义策略。影响是同一 item 从列表或通知触发会得到不一致的行为，且禁用 action 可能仍出现在通知。根因是 `[推断]` 本地 demo/MVP 的多入口默认值没有收敛为单一 policy source。

### P7：Room schema/decoder 不适合未来远端数据演进（Medium）

证据：`AttentionDatabase.kt` version 1 且 `exportSchema=false`，未见 migration；`AttentionEntityMapper.kt` 对未知 type/status/source/schedule 静默默认成 Reminder/Waiting/LocalMock/Flexible，未知 action 被丢弃。机制是协议/枚举升级不会显式失败。影响是未来 Hub item 落本地时可能被静默改写成 LocalMock 或错误状态。根因是 `[推断]` schema 目前只服务单设备 MVP，没有承担跨版本、跨来源数据契约。

### P8：历史 mock 与真实实现的语义边界不干净（Low-Medium）

证据：`AttentionDemoData.initialItems()` 无调用者；`MockAttentionScheduler`、`InMemoryAttentionRepository` 无生产组装调用；InMemory `clearMockData()` 清空全部 item，而 Room 实现只删 `LocalMock`；NotificationService 创建 `ALARM_CHANNEL_ID` 但 profile 使用 `CRITICAL_ALARM_CHANNEL_ID`。机制是旧 in-memory/mock/通知 channel scaffold 与 Room/真实 Android 路径并存。影响是误注入实现或复用接口时可能误删非 mock 数据，维护者也难判断正式能力。根因是 `[推断]` 从 mock spike 切到 Room/Android OS 时未做 clean cutover。

## 6. 建议的目标边界与安全增量切分

### 目标边界

- Console 作为观察/控制/本地提醒呈现层；不扩展为 Agent Runtime，不新增 provider orchestration、reasoning loop、长期记忆或上下文管理。
- 明确两类 authority：Android local attention 由 Room + Android scheduler 管理；Hub remote control 由 Hub API/Agent run 状态管理。不要把 Hub item 直接伪装成 LocalMock，也不要让 Room 成为远端 run 的唯一权威。
- commonMain 只定义稳定的 UI state 和 client/repository ports；Android/桌面/Web 分别实现能力 adapter。平台 capability 必须是真实状态，不靠 placeholder 文案冒充。

### 增量切分

1. **先收口现状契约**：把 Android attention 标为 local-only；统一 snooze/action policy，定义 overdue、ExactRequired、schedule failure 和 idempotency 语义；清理或隔离无调用者的 seed/mock/channel scaffold。
2. **先做只读 Hub vertical slice**：实现 `/v1/info`、`/v1/agents`、`/v1/jobs`、`/v1/runs/:run_id` 的平台 client/DTO、Bearer auth、timeout/error/capability state；Android 先接，Desktop 可选远端或同机 Unix，Web 只走 HTTPS/WS/SSE。不要先把 Hub actions 混入 local attention。
3. **再做受控动作**：逐个接 job/run 查询、cancel/confirmation 等已有 Hub action，UI 显示 pending/ack/timeout/error；复用 Hub 现有 run/confirmation 语义，不在 Console 新建执行循环。
4. **通知单独切片**：Hub `notify/channels`/`notify/send` 作为 user notification adapter；在 Android register/delivery 未实现期间保持 unavailable，不把本地 Reminder action 当作远程控制成功。
5. **最后做跨平台 UI 复用**：让 Desktop/Web 消费稳定 common state/port，再决定本地能力（Desktop Unix control）和浏览器能力（Hub HTTPS/WS/SSE）；不要把 Android Room/AlarmManager 类型上移到 common。
6. **UX demo 只移植验证过的视觉原则**：可择取窄屏 layout、Review 搜索、视觉 variants；不要合并其硬编码 process/Hub rows 或把 demo 状态当生产模型。生产配置 TUI 继续负责 config file/secret 写入，除非另有明确 Console 配置编辑需求。

## 7. 已有验证入口与未调查盲点

按本任务只读约束没有运行 Gradle、cargo、formatter、lint、build 或 test；以下是源码中已有入口而非本次验证结果：

- Console README 列出 Android assembleDebug、Desktop hotRun/run、Web Wasm/JS browser run 以及 shared common/JVM/Android-host test task；现有 shared tests（`SharedCommonTest.kt`、`SharedLogicDesktopTest.kt`、`SharedLogicAndroidHostTest.kt`）主要是 `1+2=3` 算术 smoke，未覆盖 Attention 或 Hub。
- Demo 有 `tests/state.rs` 的四类 AppState 转移测试；实际 UI 只能手工启动终端程序观察。
- 生产 TUI 有 config setup model/review/validation、commit-once、HTTP MCP 等测试；Hub 有 notify、routes、runs 相关测试；这些验证的是 Rust backend/生产 TUI，不会证明 Console integration。

未调查/需主线后续核验：真实 Android API 24/33/34/36 设备上的通知权限、Doze、exact alarm、full-screen、boot、force-stop/process death、Room backup/migration；真实 Hub 部署的 TLS/CORS、Bearer secret 生命周期、WS/SSE reconnect 与 Agent handshake；Desktop 应选 Unix local control 还是 Hub；Web Kotlin 网络库方案；Console 与生产 Config schema/HubReporting 的边界；Room schema export/release migration。