# 接口说明

推荐的 Standalone 运行时通过 Secure MCP Tunnel 和仅所有者可访问的 Unix MCP 直接暴露 Normal/Room MCP 接口面。其传输和工具契约见 [`standalone-runtime.md`](standalone-runtime.md)。

本文主要梳理可选 Rust Hub 的接口面：GPT Actions、Apps MCP、Hub 原生工具以及 Hub 到 Agent 的协议。

跨接口面的用途/不用途、条件输入、范围、生命周期和一致性矩阵见 [`tool-contract-matrix.md`](tool-contract-matrix.md)。

## Agent 接入与操作边界（WP2）

Agent 通过一个范围狭窄的内部准入边界处理请求，不引入新的框架或注册表。每个适配器都会创建不可变的 `RequestContext { ingress, operation }`；`operation::authorize(runtime, config, context)` 会根据真实接入来源以及命名空间/工具集和能力规则进行检查。`read_only`、`destructive`、`open_world` 等描述符注解仍是发现/客户端元数据，绝不构成授权。

各接入路径的身份验证、封装和错误封套仍互相独立：本地 Unix 保留 UID/套接字防护和 `local:` 来源前缀；Tunnel stdio 使用 `tunnel:`；worker HTTP MCP 保留 bearer、Host/Origin/session 处理并使用 `http:`；Hub WS/SSE 保留其协议封套/重放行为并使用 `hub:`；CLI 使用 `localadmin:`。CLI 准入门有意限制为现有四项本地 tmux 管理操作。HTTP/MCP、Hub wire 和本地 stdio/Unix 的结果投影可以在传输边界上有所不同，而共享 Agent 操作使用同一套值/错误和精简 Process/Skill 结果层。

Normal 并不等于 Room：只有显式启用 `room` 命名空间时，Normal 运行时才可使用 Room。Hub 保留现有 Room 工具集、Skills 能力/配置档和通知能力规则。实际副作用仍由策略、路径、确认、租约和资源所有者决定；此边界不会使 Hub 成为执行方，也不声称提供通用 OS 沙箱。

重载会应用现有的安全实时更新子集（`policy`、`limits`、`mcpServers`、`toolsets`、`httpMcp`、`events`；仅当 `workspaceRoot` 未改变时重载 `pathPolicy`），且不会重建启动时派生的资源。身份/模式/配置档、workspace/runtime/socket、整个 Browser 配置（不只是 `browser.runtime`）、history/install 及相关资源所有者变更都需要重启。启用 Room 时，会先准备现有实时根目录，再使用新子集。

`agentic-gpt local` 是 Unix MCP 客户端，使用常规 MCP 操作准入门并记录 `local:` 来源。它不同于 `agentic-gpt tmux`：后者的 CLI 管理准入门仅允许 `tmux.listSessions`、`tmux.attach`、`tmux.createSession` 和 `tmux.closeSession`，并记录 `localadmin:` 来源。MCP `tmux.sessions`、`tmux.panes`、`tmux.exec` 和 `tmux.pasteText` 不属于这四项 CLI 管理操作。

## GPT Actions API 端点

GPT Actions API 由 `openapi/hub.yaml` 描述，并受 Hub API key 保护。

`openapi/hub.yaml` 是当前受支持的 OpenAPI 工件。仓库中签入的
`openapi/agents-minimal.yaml` 仅供历史/非规范参考，不是运行时或 CI 门禁；
当前 API 的调用方不应导入该文件。

核心端点：

- `GET /v1/info`：安全的 Hub 运行时概要。
- `GET /v1/agents`：启用的本地 Agent、其在线状态和安全配置概要。
- `POST /v1/process/exec`：启动一个受管理的进程并短暂等待。请求必须提供 `agentId`、`program`、`args` 和 `needConfirm`；支持可选的 `workingDirectory`、`group`（1–32 个字符）和有界 `waitSeconds`。响应为扁平的 `ProcessResponse`。只有当序列化后的创建响应不超过 8 KiB 时才会内联完整输出/结果；超出部分的输出会用 stdout/stderr 共用的预览表示，预览上限为 2 KiB。
- `POST /v1/process/batch`：原子准入一批受管理进程。请求必须提供 `agentId`、`elements` 和 `needConfirm`；支持批次级 `workingDirectory`、每个元素的覆盖项，以及由子进程继承的可选 `group`。响应为 `ProcessBatchResponse`，其中按序排列的子进程投影共用一个 8 KiB 的完整内联预算，输出预览也有界。
- `GET /v1/process?agentId=...`：列出活动或近期保留的进程元数据，可选用 `group`、kind、state、limit 和 cursor 筛选。`limit` 默认值为 50，上限为 100。Agent 不可用时，Hub 可以为第一页返回缓存元数据，但不会从缓存继续使用 Agent 发出的 cursor。
- `GET /v1/process/{processId}?agentId=...&waitSeconds=...`：仅查看进程状态元数据，或短暂等待；`waitSeconds` 默认值为 5，上限为 30。状态永不包含 stdout、stderr 或结果正文。
- `GET /v1/process/{processId}/output?agentId=...&cursor=...&maxBytes=...`：读取非消费式 stdout/stderr 分页。cursor 绑定到进程，并推进两个流的原始字节偏移；`maxBytes` 默认值为 8 KiB，上限为 32 KiB。响应以 base64 保留无效 UTF-8，并明确报告保留区间缺口和 EOF。
- `GET /v1/process/{processId}/result?agentId=...&maxBytes=...`：获取完整且保留的结构化结果，或如实返回 `complete`、`too_large` 或 `unavailable` 状态。`maxBytes` 默认值为 8 KiB，上限为 512 KiB；过大的结果绝不会以不完整 JSON 返回，Hub 元数据缓存也不会提供结果内容。
- `POST /v1/process/{processId}/cancel?agentId=...`：请求按进程类型执行取消，并返回观察到的结果/终止证据；超时或缺少响应不能作为已取消的证据。
- `POST /v1/mcp/servers`：列出一个本地 Agent 中配置的 MCP 服务器；省略 `agentId` 时，则汇总所有当前已连接 Agent 的 MCP 服务器。
- `POST /v1/mcp/tools`：列出一个 MCP 服务器暴露的工具。
- `POST /v1/mcp/callTool`：通过所选本地 Agent 启动一个受管理的下游 MCP 工具进程。HTTP 响应为扁平的 `ProcessResponse`；完整内联输出/结果遵循 8 KiB 创建响应预算，超出部分的输出受 2 KiB 共用预览限制。`waitSeconds` 默认值为 5，上限为 30；等待超时不会取消进程。`timeoutSeconds` 默认值为 300，上限为 900。
- `POST /v1/mcp/batch`：原子准入 1–16 个按序排列的下游 MCP 子进程。响应为 `McpBatchToolResponse`，所有子进程共用 8 KiB 完整内联预算，并提供有界输出预览；该操作使用一次聚合确认，支持并行或顺序模式、可选的安全快速失败调度、共用的全局/每服务器并发限制，以及 2 MiB 聚合响应预算。
- `GET /v1/runs/{runId}`：查看一项 Hub 到 Agent 命令运行的已持久化状态和可选的迟到结果。
- `POST /v1/room/skills/list`、`/read`、`/search`、`/active`、`/activate`、`/deactivate`：通过活动 Room Agent 发现 workspace Skills 并维护本地激活状态。这些端点不接收 `agentId`。
- `POST /v1/room/skills/install`：异步安装一个 Skill，来源可以是公开 GitHub、HTTPS 文件条目或内联 UTF-8/base64 文件。网络操作开始前，响应会先返回 `installId`。
- `POST /v1/room/skills/install/get`：通过有界长轮询查询安装状态。`waitSeconds` 默认值为 5，上限为 30；等待超时不会取消安装；终态响应将 `pollAfterMs` 设为 `0`。
- `POST /v1/room/skills/install/cancel`：在原子提交之前请求幂等的协作式取消。
- `POST /v1/room/skills/run`：运行活动 workspace Skill 在 `scripts/` 下的可执行脚本。`waitSeconds` 默认值为 5，上限为 30；等待超时不会取消执行。若可能则内联返回终态输出，否则使用响应中的实际 `agentId` 和 `processId` 调用 `process.status`、`process.output` 或 `process.cancel`；Skill 脚本不适用 `process.result`。此运行端点的输入不接收 `agentId`。
- `POST /v1/room/bootstrap`：读取活动 Room Agent 的重复会话入口及确定性指南清单。无请求体，也不接收 `agentId`。
- `POST /v1/room/bootstrap/read`：按 frontmatter 中的 `id` 读取一份有效引导指南。不接收 `agentId`。
- `POST /v1/room/diary/active` 和 `POST /v1/room/diary/read`：通过捕获的活动 Room 租约读取当前或一个经过验证的 Diary 层。请求分别使用 `RoomDiaryActiveRequest` 或 `RoomDiaryReadRequest`；响应分别为 `RoomDiaryActiveResponse` 或 `RoomDiaryReadResponse`。
- `POST /v1/room/notebook/recent`、`POST /v1/room/notebook/search` 和 `POST /v1/room/notebook/read`：读取有界的当前 Markdown 预览、搜索当前 Markdown，或读取一份经过验证的 Notebook 文档。`recent` 和 `search` 使用 `RoomNotebookResultsResponse`；`read` 使用 `RoomNotebookReadResponse`。
- `POST /v1/room/state/list` 和 `POST /v1/room/state/read`：列出或读取有界的 `State/entities` Markdown 文档，分别使用 `RoomStateListResponse` 或 `RoomStateReadResponse`。
- `POST /v1/room/maintenance/status` 和 `POST /v1/room/maintenance/submit`：检查由仓库所有者负责的就绪状态，或提交明确的语义维护请求。响应分别为 `RoomMaintenanceStatusResponse` 和 `RoomMaintenanceSubmitResponse`；submit 使用现有 local/workflow 模式及有界等待契约。

这九个 Room 端点在 JSON 请求体中使用当前的 camelCase Agent 请求/响应 DTO。
空请求 DTO 仍须传入 `{}`；没有端点接受 `agentId` 选择器。它们解析并捕获活动
Room 租约：Room 不存在时返回 `room_not_active`（404），租约不一致/已替换时返回
`room_state_conflict`（409），Hub 传输等待超时时返回 504 操作超时。Agent 语义错误
保留现有 Room JSON 错误投影。Full MCP 以当前描述符公布相同的九个名称；
Coordinator 既不公布也不派发 Room 操作。

JSON 格式错误或缺少必需请求字段时，Axum JSON 提取器会返回 HTTP 422 和
`text/plain`；Agent 语义验证错误仍以 JSON 错误响应返回，采用本文所述的 400/
特定 404/409 投影。

`/v1/info` 刻意只返回安全元数据：Hub 版本、公共基础 URL、超时设置、远程确认
状态、Agent 数量以及待处理请求/进程数。不得暴露密钥、确认回调 URL 或私有配置值。

`/v1/agents` 为每个已启用的本地 Agent 返回一份安全配置概要。Agent 在线时，概要
包含粗粒度沙箱模式、确认提供方、路径策略根目录、已配置的命令策略规则和内置
命令策略规则。路径根目录以 `workspace`、`~/Documents` 或 `/tmp` 等显示路径表示；
若可行，私有 home 路径应缩写为 `~`。Agent 离线时可能返回 `unknown` 概要，因为
Hub 不持久保存上次取得的本地配置概要。本地确认提示可通过 `confirmationLanguage`
（`en` 或 `zh-CN`）使用英语或简体中文。

### Hub Process 权威性、新鲜度与保留

Agent 的受管理进程历史是执行侧权威来源。Hub 进程状态条目和 Hub 进程缓存是用于路由和观察的投影；它们不能证明本地进程仍在运行，也不能证明副作用已撤销。Hub 缓存最多保留 4,096 个进程，在 `observedAt` 之后 15 分钟过期，并在 60 秒后标记为 `stale`。每 15 秒运行一次清理，移除已过期条目；容量淘汰最早观察到的条目。淘汰活动投影不会影响 Agent 进程或权威的 Hub 运行回执。

Hub HTTP `process.status`、`process.list` 以及对应的 Apps MCP 进程检查响应会暴露 `freshness` 和 `observedAt` 元数据。创建端点直接返回的实时进程封套可能省略这些投影字段；其中的 Agent 进程负载仍具权威性。`live` 表示响应来自 Agent，`cached` 表示 Hub 投影仍在新鲜度窗口内且可用，`stale` 表示投影已较旧，`unknown` 表示当前没有可用事实（包括重启协调之后）。这些字段描述响应投影，不是 Agent 进程状态字段。只有缓存的状态响应属于降级证据，不是新的等待结果；Hub 不会为 Agent 发出的 cursor 臆造续页。Hub 缓存仅包含状态元数据，不能提供输出或结果内容。

Hub 运行回执仍是已派发命令的持久控制面身份。运行保留窗口为 24 小时；窗口结束后，只压缩符合条件的已完成负载：`runId`、请求/Agent 身份、命令哈希、状态以及冲突/未知/墓碑证据仍会保留。重放和去重所需的身份/哈希证据仍受保护；未知和冲突记录不会压缩。因此 `AgentRun` 会分别报告 `resultRetained` 和 `resultOmitted`；负载被省略不能证明命令未运行。

等待超时或传输超时不等于远程取消，只会结束本地等待；匹配的迟到回执或结果仍可能到达。只有观察到终止证据时才报告取消；缓存快照或缺失响应都不能推断远程进程已经停止。

### 当前 Room 边界与协调请求投影

当前 Agent 语义 Room 接口面是上述九项远程操作的权威来源。Hub Full 通过捕获的活动 Room 租约转发这些操作；它不会读取仓库、拥有 Room 文件、解释 Git 状态，也不会创建第二份内容存储。通用 Hub 运行回执可能为状态/迟到结果检查保留有界操作结果，但该回执不是 Room 内容的权威来源。

读取范围属于当前契约：Notebook `limit` 默认值为 20，范围是 1–100；搜索 `query` 不能为空，且最多 256 个 Unicode 字符；Notebook 和 State Markdown 读取会拒绝超过现有 512 KiB 上限的内容；Diary 周期可以是 `current`、严格格式的日日期，或按顺序排列的周/月日期区间。`room.notebook.recent` 和 `room.notebook.search` 保留原公开名称，但现在返回当前 Markdown DTO（`path`、`title`、`contentPreview`、`truncated`、`effectiveAt`），不再返回已退役的 passage/JSONL 格式。

维护操作仍须明确提交，且由 Agent 所有。`room.maintenance.status` 为只读。`room.maintenance.submit` 接受一至五个唯一语义 slot、可选 `local` 或 `workflow` 模式，以及范围为 0–30 的 `waitSeconds`（默认值为 0）。workflow 等待只观察消费/快进；超时只结束等待，不会取消已提交的维护操作。现有仓库、路径、symlink、锁、clean-tree、预期变更、执行器和 Git 控制仍然有效。没有单独的维护等待 API，也不承诺新增确认机制。

从已退役 Room JSONL 名称迁移的调用方必须选择明确的当前语义操作。旧 append/update/remove 或 passage/日期选择语义不会自动转换为 `room.maintenance.submit`；要更改 Room 内容的调用方必须构造文档规定的 slot/payload 请求，或弃用旧调用。应同时升级配对的 Hub 和 Agent 工件，刷新 [`../openapi/hub.yaml`](../openapi/hub.yaml)，迁移所有调用方，并在移除旧调用前验证实时活动 Room 路径。历史发布/迁移记录仍是历史记录，不是当前错误或兼容性契约。


## 持久事件收件箱

每个 Agent 有独立、持久的事件收件箱；同一 Agent 的客户端共享事件和展示计数。事件 API 与 Process/Skill Install 的生命周期操作彼此独立。Standalone 的三个公开 MCP 工具 `event.list`、`event.get`、`event.mark` 不依赖 Process/Skills 工具集开关，也不新增可配置 namespace。Standalone 请求省略 `agentId` 时使用当前 Agent；若提供，则必须与当前 Agent 匹配，不作为跨 Agent 选择器。Hub Full 的三个工具要求显式 `agentId`；Coordinator 的八项工具保持不变，且不公开事件工具。

`EventSource.kind` 是创建时固定的事件来源类型，与入口/传输（如 Unix、stdio、HTTP、Hub）以及 Hub `RunReport` 相互独立：内部事件由服务端绑定 `process` 或 `skill_install` 和对应实体 ID；外部注入固定为 `external`，调用方只提供 `ref`。读取、筛选、标记或传输事件都不能改写来源。事件记录使用 camelCase 字段 `eventId`、`message`、`severity`（`low|medium|high`）、`createdAt`、`status`（`pending|handled|expired`）、`source: { kind, ref }`、`shownCount` 与可空 `expiresAt`。列表项以 `summary` 代替完整正文。

| 操作 | 请求与结果 |
|---|---|
| `event.list` | 可选 `agentId`（Standalone 省略时为当前 Agent；若提供须匹配当前 Agent）、`status`（缺省 `pending`）、`severity`、`limit`（缺省 20，范围 1–100）和不透明 `cursor`；返回 `{ items, nextCursor? }`，每项为 `eventId/summary/severity/createdAt/status`。隐藏的 pending 事件仍可列出；有后续页时返回 `nextCursor`。 |
| `event.get` | 必填 `eventId`，可选 `agentId`；返回完整事件记录（含 `message`），读取不会标记为已处理。 |
| `event.mark` | 必填 `eventIds`（最多 512 项），可选 `agentId`；幂等地标记为 handled，只改变收件箱处理状态，返回 `{ handledIds, notFoundIds }`。不管理进程或安装。 |

Hub HTTP 路由使用现有 Hub API Bearer 认证及 Agent 启用状态授权：`GET /v1/events?agentId=...`（另支持 `status`、`severity`、`limit`、`cursor`）、`GET /v1/events/{eventId}?agentId=...`、`POST /v1/events/mark`（JSON `{ "agentId": "...", "eventIds": ["..."] }`）。Hub 工具、OpenAPI 与请求体都按目标 Agent 隔离，不跨 Agent 查找或合并。

有明确目标且在线的 Agent 响应（包括业务错误响应）会在原业务 JSON 根对象附带 `events` 面板，不包裹或替换原结果。以下仅为结构示意，标识、文案和时间均为占位，不是运行输出：

```json
{
  "items": [
    {
      "eventId": "evt_example",
      "summary": "Process completed",
      "severity": "low",
      "createdAt": "2026-10-01T12:00:00Z",
      "status": "pending"
    }
  ],
  "events": {
    "current": "low: 1 | medium: 0 | high: 0",
    "new": [{ "evt_example | 示意摘要": "low | 2026-10-01T12:00:00Z" }]
  }
}
```

面板统计所有未过期 pending 事件（含本次未展示项）；`new` 最多五项，按 high、medium、low 排序，同级按较早创建时间排序。摘要最多 32 个 Unicode 字符（超长时 31 字符加 `…`）；等级、时间和 ID 不截断。low 首次曝光后隐藏，默认 TTL 为 24 小时且可配置；medium 曝光三次后隐藏且不自动过期；high 保持候选直到处理。等级表示通知优先级，不表示内部事件成功/失败或是否需要人工介入。handled/expired 历史保留七天后清理。新事件只在后续工具调用中以面板提醒；同一响应内，原有 Browser/file 内容块会保留，面板为紧凑文本且只展示/计次一次。事件详情正文读取本身不计入曝光，面板实际展示的候选仍按正常规则计次。面板位于原结果根级键 `events`，不另加结果封套或面板封套。

没有主动推送。无单一 Agent 目标的 Hub 调用不附面板；Agent 离线、Hub 请求超时或 Hub 缓存回退也不附事件，不能伪报零值或旧快照；成功在线的 native cache-only Hub 工具可单独做一次 best-effort 面板查询。原始创建响应是否已包含终态结果由最终入口决定：包含终态即抑制对应异步事件，不包含则晚到的终态可产生事件；这一判定不依赖 `completedInline`、结果/输出截断或大小限制。创建去重、事件来源/origin 绑定及可靠的私有响应反馈用于防止重放改变资格；这些不是公开事件 API。模型不可写入内部来源/仲裁元数据，也没有客户端阅读 ACK 或模型主动推送。

无目标的 `mcp.listServers` 聚合发现会在 Agent 端抑制面板生成，因此不会消耗事件曝光次数；单目标发现仍按正常规则附带面板。

Agent 内部可靠传输命令清单新增 `event.list`、`event.get`、`event.mark`；内部私有 `event.settle` 用于反馈原响应判定，不是公开工具或 HTTP 路由。Hub 创建在副作用前绑定来源/origin；Agent 对远程来源等待 Hub 的最终判定，不以重启、完成先后或再次读取推断资格。可靠 `Response.eventSources` 携带该原始创建响应的来源及 `includesTerminal` 判定；独立私有 `event.sources` 只用于恢复待仲裁来源身份。二者分工不同，配合持久决策/可靠反馈支持 Hub 重启恢复，均不替代业务 `RunReport`，也不进入公开 HTTP/MCP 业务 DTO。外部 Unix CLI 注入仅为本地集成入口，stdin JSON 及其使用方式见[Standalone 运行指南](standalone-runtime.md)；不是模型可调用工具。

公开定义见 [`openapi/hub.yaml`](../openapi/hub.yaml) 与实时 Agent MCP 工具 schema；此节概述不替代它们。

## ChatGPT Apps MCP 端点

`/mcp` 是适用于 Apps 的 MCP 端点。它受 Hub OAuth shim 保护，并将 MCP 请求转发到已配置的本地 Agent 和本地 MCP 服务器。

所有 `/mcp` `tools/call` 响应均使用 Hub 的 `AgenticResult` 封套，与 ChatGPT Apps/MCP 工具结果格式直接兼容。Hub 原生 JSON 以 `structuredContent` 和 JSON 文本 content block 暴露；顶层 `error` 会使 MCP 工具结果的 `isError=true`。

`mcp.callTool` 不会在 Hub 顶层透传下游结果封套。当前实时 HTTP 端点返回扁平的 `ProcessResponse`；下游终态结果保留在 `result` 中，下游 `isError=true` 会使进程失败，同时保留该结果。序列化参数上限为 256 KiB。最多 512 KiB 的序列化结果会予以保留；更大的结果会省略，并以 `resultBytes`、`resultSha256` 和 UTF-8 安全的 `resultPreview` 替代。活动调用使用 `process.status`、`process.output`、`process.result` 和 `process.cancel` 跟进。
Hub 没有原生 `file.read` 或 `file.edit` 工具。其通用异步 MCP 进程桥接不是类型化的图像内容接口；不要依赖它保留 `file.read` 图像 Content blocks。

`mcp.batch` 返回扁平的 `McpBatchToolResponse`，其中 `results` 按顺序包含子进程投影。确认和启动任何子进程之前，会先完成验证与容量准入。并行模式使用共享调度器（全局最多 8 个、每个服务器最多 2 个）；顺序模式会等待每个子进程进入终态。`failFast=true` 时，只有尚未启动的子进程会标记为 `skipped`；已经启动的调用不会取消。单服务器批次可以获得临时服务器 allow 操作，多服务器确认则仍以整个批次为作用范围。每个子项都是普通的受管理进程，带有 `batchId`、可选 `batchCallId` 和 `batchIndex`，后续检查和取消使用相同的 `process.*` 生命周期接口。

取消以证据为准。Agentic 会使用准确的下游请求 ID 发送 MCP `notifications/cancelled`。如果未观察到下游终态响应，进程会变为 `detached`，而不是声称取消成功。仅 Hub 缓存的进程状态属于降级元数据证据，不会提供缓存结果，也不会报告取消成功。

此契约适用于 Apps MCP `/mcp` 接口面。`/v1/*` 下的 GPT Actions 端点使用各自的 JSON 投影；上述当前实时 HTTP `/v1/mcp/callTool` 响应/schema 差异不构成通过 schema 一致性验证的声明。

OAuth 发现路由：

- `/.well-known/oauth-protected-resource`
- `/.well-known/oauth-authorization-server`
- `/.well-known/openid-configuration`
- `/oauth/authorize`
- `/oauth/token`

Hub MCP 配置档在 Hub 启动时通过 `--mcp-profile full|coordinator` 或
`AGENTIC_GPT_HUB_MCP_PROFILE` 选择。`full` 是默认值，保留执行接口面和跨传输的
`bootstrap` 别名。`coordinator` 只公布 Hub 原生工具，包括 `hub.process.status` 和
`hub.process.list`；这些工具只读取缓存的进程元数据，不派发执行。完整配置档和
Standalone Tunnel 文档见 [`standalone-runtime.md`](standalone-runtime.md)。

ntfy 确认回调路由有意不纳入 `openapi/hub.yaml`，仅供确认操作按钮使用。

Room Skill 包在 workspace 下以 `<workspaceRoot>/skills/` 的形式可见；由工具管理的激活状态则作为私有持久状态存储在 `~/.agentic_gpt/state/agent/<agentId>/active-skills.json`。启动时，Agentic 会迁移无歧义的旧 `<workspaceRoot>/state/active-skills.json`；如果新旧副本内容不同，则以私有副本为准，并保留旧副本及一条警告。激活 Skill 不会运行它或授予权限；过期的活动条目会以 `missing` 状态继续显示，直到被显式停用。内置 `skill-installer` 指南默认处于激活状态，也可显式停用。

安装任务持久化在 `~/.agentic_gpt/state/agent/<agentId>/skill-installs/` 下；在恢复安装前，旧的 `<workspaceRoot>/state/skill-installs/` 目录树会按相同的冲突保留规则迁移。安装记录保留终态七天（最多 100 条），且公开状态绝不暴露内联负载或 URL 查询/片段值。显式替换前，现有 Skill 会归档到 `skills/.archive/<id>/`。远程文件 URL 必须使用公开 HTTPS，并在 DNS 解析和重定向后重新验证；部署方可通过 `room.skills.allowedHosts` 收窄允许的主机。

## Room 会话引导包

Room Agent 每次调用时都会直接读取配置的 `workspaceRoot` 中重复使用的会话引导包。读取不会创建文件、安装默认值、缓存索引或要求重新加载。固定布局如下：

```text
<workspaceRoot>/bootstrap/
├── bootstrap.md
└── guides/
    ├── diary.md
    ├── notebook.md
    └── ...
```

`bootstrap.md` 必需；`guides/` 可选。只有该目录下直接包含的、常规、非隐藏且扩展名为小写 `.md` 的文件会被视为指南；嵌套目录、隐藏条目和其他扩展名都会忽略。引导根目录和入口文件不得为 symlink。缺少引导包是正常的 404（`bootstrap_not_found`）；服务不会自动创建或个性化引导包。

入口文件以一个封闭的 YAML 对象开头。必需字段如下：

```markdown
---
id: room
kind: entrypoint
name: Room 会话引导
description: 会话初始化与指南路由。
schemaVersion: 1
---

Room 会话开始时，请阅读下列相关指南。
```

`id` 使用保守的 ASCII 语法 `[A-Za-z0-9_.-]+`；`.` 和 `..` 不是有效 ID。`kind` 必须是 `entrypoint`，`name` 和 `description` 必须是非空字符串，`schemaVersion` 必须是整数 `1`。入口响应会保留原始 frontmatter。入口元数据无效时，整个包以 `bootstrap_invalid` 失败。

每份指南都使用相同的封闭 frontmatter 约定：

```markdown
---
id: diary
kind: guide
title: Diary 约定
summary: 保持上下文连续，同时不取代 Diary 工具 schema。
loadPolicy: contextual
priority: 80
loadWhen:
  - 会话需要延续此前的个人或项目上下文。
toolBindings:
  - room.diary.active
  - room.diary.read
  - room.maintenance.submit
tags:
  - 连续性
---

读取当前或指定文档时使用语义化 Diary 工具；所有变更都通过维护提交契约执行。
```

指南必需字段为 `id`、`kind: guide`、`title` 和 `summary`。`loadPolicy` 默认为 `on_demand`，可选 `startup`、`contextual` 或 `on_demand`。`priority` 默认为 `0`，且为有符号 32 位整数。`loadWhen`、`toolBindings` 和 `tags` 默认为空数组，其中的非空字符串按编写顺序保留。对类型化 V1 行为而言，未知字段会被忽略，但仍保留在 `room.bootstrap.read` 返回的原始 `frontmatter` 中。

指南元数据是通用的。例如，workspace 可编写如下指南，而不改变运行时：

```markdown
<!-- guides/notebook.md -->
---
id: notebook
kind: guide
title: Notebook 连续性
summary: 先搜索并阅读持久化的项目内容，再作出假设。
loadPolicy: contextual
toolBindings: [room.notebook.search, room.notebook.read, room.maintenance.submit]
tags: [项目上下文]
---
MCP 参数 schema 应保留在工具定义中；修改 Notebook 时使用维护提交。
```

```markdown
<!-- guides/execution.md -->
---
id: execution
kind: guide
title: 执行方式选择
summary: 有意识地选择受管理进程或持久窗格。
loadPolicy: startup
priority: 90
toolBindings: [process.exec, process.batch, process.status, process.cancel, tmux.exec]
tags: [运维, 安全]
---
参数以工具 schema 为准；工作流、确认和恢复说明见本指南。
```

```markdown
<!-- guides/skills.md -->
---
id: skills
kind: guide
title: Skill 选择
summary: 使用已安装的工作流前，先发现并阅读相关 Skill。
loadPolicy: on_demand
toolBindings: [skills.list, skills.read, skills.run]
tags: [工作流]
---
`toolBindings` 只是路由提示，不代表权限已授予，也不保证相应工具可用。
```

MCP schema 仍是工具可用性和参数的事实来源。指南说明选择、顺序、约定、安全、示例和恢复行为；不会复制完整 MCP schema、授予授权，也不会断言其中列出的每个绑定当前都已暴露。

`room.bootstrap` 会内联返回入口文件，并返回扁平清单、包 `revision`、计数和警告。有效指南按 `priority` 降序、再按 `id` 升序排列；最多返回 64 条摘要。`totalGuides` 统计所有有效且无重复的指南，因此超过 64 条上限的有效指南仍可通过 `room.bootstrap.read` 读取，也仍会影响 `revision`。重复 ID 会排除所有发生冲突的指南。无效的可选指南会以警告排除，不会导致整个包失败。

`room.bootstrap.read` 接受 `{ "id": "diary" }`，会再次验证入口文件和指南包，并返回选中的摘要、原始指南 frontmatter、有界 Markdown 资源以及相关警告。它不是通用路径读取器。未知、无效、因 ID 重复而排除或以其他方式不可用的 ID 会返回 `guide_not_found`。

完整的 YAML frontmatter 起始块（包括两个 `---` 分隔符）必须在前 1,048,576 字节（1 MiB）内结束；结束分隔符恰好位于该边界时也接受。此元数据上限独立于返回内容截断。超限的入口文件返回 `bootstrap_invalid`；超限的可选指南以 `guide_frontmatter_invalid` 排除。整个文件仍会被流式读取，以完成 UTF-8 验证、行计数、SHA-256 计算和包 revision 计算。

每个文本资源都是 UTF-8 Markdown，`mediaType` 为 `text/markdown`。`sizeBytes` 和 `sha256` 描述完整原文件；`returnedSizeBytes` 描述实际返回的内容。入口文件上限为 65,536 字节，指南上限为 262,144 字节。有效但过大的文档会返回前缀，设置 `truncated: true`，并附带 `entrypoint_truncated` 或 `guide_truncated` 警告。截断时优先使用上限内最后一个完整换行；否则截到有效 UTF-8 边界。`totalLines` 和 `returnedThroughLine` 是从 1 开始的逻辑行数，`omittedFromLine` 标识首个省略行，`lastLineComplete` 用来区分完整行前缀与部分行前缀。

稳定的警告前缀包括 `entrypoint_truncated`、`guide_truncated`、`guides_truncated`、`guides_dir_symlink_ignored`、`guide_dir_entry_unreadable`、`guide_symlink_ignored`、`guide_unreadable`、`guide_non_utf8`、`guide_frontmatter_invalid`、`guide_metadata_invalid` 和 `guide_duplicate_id`。包级失败使用 `bootstrap_not_found`、`bootstrap_invalid` 或 `bootstrap_read_failed`；Room 路由还会返回 `room_not_active`、`room_state_conflict`、`room_bootstrap_timeout` 和 `room_bootstrap_read_timeout`。这些操作为只读、可安全重试、非破坏性，且不会触发具有后果的副作用。

MCP 工具为 `room.bootstrap` 和 `room.bootstrap.read`。对应的 GPT Actions 路由是 `POST /v1/room/bootstrap` 和 `POST /v1/room/bootstrap/read`，operation ID 为 `roomBootstrap` 和 `roomBootstrapRead`。两个接口都限定于 Room，不接收 `agentId`。

## 本地 Agent 传输

本地 Agent 连接到：

```text
GET /v1/agents/{agentId}/connect
```

WebSocket 是本地 Agent 的默认传输方式。本地 Agent 可通过 `hub.transport: "sse"` 选择 HTTP/SSE 传输，适用于出站 HTTP/SSE 比 WebSocket 更稳定的环境。

SSE 端点仅供 Agent 使用，并采用与 WebSocket 相同的 `x-agent-secret` 身份验证：

```text
GET  /v1/agents/{agentId}/events?connectionId=...
POST /v1/agents/{agentId}/messages?connectionId=...
```

WebSocket 与 HTTP/SSE 对请求/响应式 `HubCommand` 消息使用相同的可靠确认/重放语义。Hub 发送的命令封套包含 `eventId`、`runId`、`requestId`、`commandHash` 和原始 `HubCommand`。Agent 会先将已接受命令写入本地传输账本，再发送 `TransportAck`；命令结果带有 `runId`，即使原连接已过期，只要 `agentId`、`runId` 和 `requestId` 匹配，仍会作为迟到结果接受。
`ConfirmationRequest` 不是带有运行 ID 的命令，也不携带 `runId`。Hub 确认操作的所有权由已捕获的 `(agentId, connectionId, requestId, sender)` 元组确定：回调会针对该连接/请求及捕获的投递目标解决一项 claim；它不会只依据 Agent ID 重新选择发送方，也不会臆造运行身份。
当前连接被替换或移除时，若仍有未解决的确认请求，系统会产生一次终态 `ProviderUnavailable` claim（原因是 `provider_unavailable`）；当该传输可写时，会在关闭旧流之前将其发送到捕获的 sender。回调、确认超时、发布失败、连接替换和断开会竞争解决同一 claim，因此都不能再次发出终态决策或重新选择 sender。如果捕获的 sender 已断开，Agent 的断开排空行为仍是后备方案；Hub 不承诺通过已失效的传输投递。

有界的 Hub/HTTP/MCP 等待超时只会结束本地等待者，不会取消远程执行。即使调用方已收到超时且不再有等待者，只要迟到的 `TransportAck`、`TransportRunStatus` 或 `Response` 匹配，仍可能推进已持久化的运行回执。只有已证实的 channel-send 失败才使用 `not_sent`，且该状态不会重放；超时或缺少 ACK 不属于 `not_sent`。

本契约涵盖的 Hub 传输回执状态包括 `created`、`dispatched`、`acked`、`started`、`running`、`failed`、`unknown`、`completed`、`timeout_waiting_result` 和 `not_sent`；这并非 AgentReport 或 Process 状态的完整清单。`sent_no_ack`、`acked_running`、`wait_expired`、`remote_unknown` 和 `cancel_requested` 等词汇用于描述观察结果或调用方状态，并不是额外的 wire 状态或持久化状态。这里不会引入 `cancel_requested` 传输状态。

两种传输都采用相同的当前连接代际准入规则。`Hello`、`Heartbeat`、`ProcessUpdate`、`RunReport` 和 `ConfirmationRequest` 都是仅允许当前连接处理的生命周期消息：只有 Agent 的最新连接可以更新元数据、进程缓存、报告、最近活动状态、确认准入或 Room 租约。过期 WebSocket 消息会被处理器拒绝，已退役的流会收到 `Close`；过期 HTTP/SSE 消息会以 `409 stale_connection` 拒绝。本地 Agent 应停止该代际对应的 writer。

如果过期的可靠消息（`TransportAck`、`TransportRunStatus` 和 `Response`）中的运行元数据匹配现有 Hub 运行，它们仍可能被接受，以便重连后继续投递迟到结果。它们不会刷新或以其他方式修改当前连接的生命周期状态。`RunReport` 不是可靠重放消息，仍仅接受当前连接发来的消息。

每个 SSE 连接都必须使用全新且非空的 `connectionId`；同一个当前 ID 不能标识第二条流。省略 `connectionId` 时，Hub 会生成新的 ID。显式传入空 ID 会返回 `400 invalid_connection_id`；重复使用当前 ID 会返回 `409 connection_id_in_use`。替换成功后会关闭旧流。这些 ID 在 agent-secret 身份验证之后用于标识连接代际，不是独立的对端身份验证机制。

V1 中，`Hello`、`Heartbeat`、`HeartbeatAck`、确认消息和 `ProcessUpdate` 仍是尽力而为的生命周期消息。`process.exec`、`process.batch`、`process.status`、`process.list`、`process.output`、`process.result`、`process.cancel` 以及 `event.list`、`event.get`、`event.mark` 是可靠的请求/响应命令；私有 `event.settle` 用于响应事件仲裁反馈。`Hello.bootGeneration` 变更会使缓存中的活动进程变为 `unknown_after_restart`；终态进程仍予保留，且不会重放副作用。

Agent 重启时，传输账本是命令/结果的持久权威来源：绑定所有者的已完成记录可以重新发送匹配结果；绑定所有者的已接受记录可以从已存储命令恢复；没有完成结果的 started/running 记录会变为 `unknown`，而不会重放副作用。claim 会被锁定并绑定到显式的 `agentId`；其他所有者不能接管该记录。旧的无所有者记录继续保持 `LegacyUnowned`：不会自动协调、执行，也不会用于披露结果。恢复操作须由运维人员检查保留的原始记录和更新的所有者绑定证据；绝不能为了让启动通过而删除去重证据。

账本解析失败或最后一行被截断时会 fail closed。出错的原始字节保存在私有 `.recovery` sidecar 中，压缩过程会保留私有 `.backup`；这些都不是删除或重置账本的理由。Hub 会在超时后将缺少状态/结果的 acked 运行标记为 `unknown`，但该状态或缓存缺失都不能证明 Agent 已停止。
