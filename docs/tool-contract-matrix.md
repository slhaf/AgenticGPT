# 公共工具契约矩阵

本矩阵说明当前公开工具契约；实时描述符和类型化请求对象仍是权威依据。“不执行”指最容易被误用、但此工具有意不会执行的相近操作。除非另有说明，范围端点均包含在内。

确定性的 Agent 语料集和 `scripts/check_contract_parity.py` 是当前行为的运行时/schema 权威依据。本矩阵仅作说明：`scripts/evaluate_tool_contracts.py` 的形状预测探针不能证明 schema 有效，也不能证明请求会被派发。

当前远程 Room 调用方应协调配对的 Hub 和 Agent 工件，并从 [`../openapi/hub.yaml`](../openapi/hub.yaml) 刷新导入的 schema。当前九项 Room 操作使用现有语义 Agent DTO 和 camelCase JSON 请求体：`room.diary.active`/`room.diary.read`、`room.notebook.recent`/`room.notebook.search`/`room.notebook.read`、`room.state.list`/`room.state.read`，以及 `room.maintenance.status`/`room.maintenance.submit`。空请求使用 `{}`；远程 Room 操作均不接受 `agentId`。`room.notebook.recent` 和 `room.notebook.search` 现在返回当前 Markdown 预览/结果，不再返回已退役的 passage 型 JSONL 格式。

## Agent 准入与副作用边界（WP2）

本矩阵说明公开工具契约；准入由 Agent 内部的 `RequestContext`/操作准入门强制执行，而不是由本文或描述符注解强制执行。上下文记录真实接入来源以及以借用形式保存的操作名称。命名空间/工具集选择与 RuntimeModel 能力是相互独立的检查；`read_only`/`destructive`/`open_world` 仍只是发现元数据，不构成授权。

本地 Unix、Tunnel stdio、HTTP MCP 工作进程、Hub 命令和 CLI 路径保留各自不同的封装/身份验证/错误封套。Agent 审计来源前缀分别为 `local:`、`tunnel:`、`http:`、`hub:` 和 `localadmin:`。CLI 接口面仅包含现有四项本地 tmux 管理操作：`tmux.listSessions`、`tmux.attach`、`tmux.createSession` 和 `tmux.closeSession`；MCP `tmux.sessions`、`tmux.panes`、`tmux.exec` 和 `tmux.pasteText` 是单独的 MCP 操作，不是 CLI 别名。共享 Skill 执行和 Process 结果投影仍由 Agent 操作/资源层负责。

Normal 不代表 Room：只有显式启用 `toolsets.room` 命名空间后，Normal 才可使用 Room。Hub 保留现有 Room 工具集、Skills 能力/配置档和通知能力行为。实际副作用仍由策略、路径、确认、租约和资源所有者控制；不能因此声称外部 MCP、Browser JavaScript、tmux、tunnel 子进程或 Browser 宿主副作用受到通用 OS 沙箱保护。

## Standalone 公布接口面与命名空间预设

Standalone 公布的接口面有 Normal 和 Room 两种预设。Normal 预设启用除 `room` 以外的所有命名空间；Room 预设启用所有命名空间。可用命名空间为 `agent`、`file`、`mcp`、`process`、`skills`、`tmux`、`browser` 和 `room`。逻辑 `room` 命名空间包括 `bootstrap`、`bootstrap.read`、语义读取工具以及 Room 维护状态/提交操作。显式设置的 `toolsets.enabled` 无论配置档为何都具有权威性，只筛选这些已公布名称；它不会暴露仅供派发使用的别名。

Tunnel stdio 和本地 Unix MCP 使用相同的描述符、schema、确认、路径策略、审计和 Process 注册表。Standalone 调用不接受仅 Hub 支持的 `agentId` 或 `confirmMethod` 字段。

| 公开名称 | 用途 / 不执行的操作 | 必填或条件必填输入 | 默认值与范围 | 失败与生命周期 | 接口面一致性 |
|---|---|---|---|---|---|
| `agent.info` | 查看本地运行时；不执行操作或修改状态。 | 无必填字段。 | 有界诊断和安全配置概要。 | 只读快照；返回后实时 Process/配置状态可能变化。 | Normal + Room；Tunnel 与本地 Unix 一致。 |
| `browser.manual` | 阅读或搜索所选运行时的官方 Browser 文档；不执行语义化 Browser 操作。 | `action`；`read` 必须提供 `path`，可选 `startLine`/`endLine`；`search` 必须提供 `query`，可选 `maxResults`/`contextLines`；与当前 action 不兼容的字段会被拒绝。 | 在文档根目录内读取且有界；搜索的 `maxResults` 范围为 1–100，`contextLines` 范围为 0–5。 | 运行时缺失以及文档范围/路径错误会返回稳定的 Browser 错误；只读、非破坏性。 | Normal + Room；使用所选运行时的文档根目录。 |
| `browser.acquire` | 获取或复用一个具名的持久 Browser JavaScript 租约；不调用语义化 Browser 操作。 | `name`、`idleTimeoutSeconds`。 | 空闲超时为 1–86,400 秒；同名调用幂等。 | 运行时缺失或管理器错误使用稳定的 Browser 错误；成功时租约状态为 ready。 | Normal + Room；两个接入路径使用同一管理器。 |
| `browser.repl` | 在持久 Browser 租约中运行任意 JavaScript；不在 Rust 侧执行 Browser 语义转换。 | `name`、非空 `code`；可选 `timeoutMs` 和用于可观测性的 `title`。 | 代码最多 256 KiB UTF-8 字节；超时默认为 20,000 ms，范围为 1–120,000；`title` 最多 128 个 Unicode 标量值。 | 原样传递官方文本/图像/结构化内容、错误状态和元数据；先调用 acquire。具有破坏性，且属于开放世界。 | Normal + Room；两个接入路径共用同一持久租约管理器。 |
| `browser.reset` | 恢复/管理员重置一个 Browser 租约；不是每次调用后的常规清理。 | `name`。 | 重置/重新引导内核状态时保留租约身份。 | 运行时、租约缺失或恢复失败会返回稳定的 Browser 错误；具有破坏性，但不属于开放世界。 | Normal + Room；两个接入路径使用同一管理器。 |
| `browser.release` | 对一个 Browser 租约执行最终清理；释放后不会隐式复用。 | `name`。 | 在轮次结束/关闭时执行有界清理；名称不存在时返回 `released: false`。 | 清理失败仍返回稳定的 Browser 错误，但租约仍会移除；具有破坏性，但不属于开放世界。 | Normal + Room；两个接入路径使用同一管理器。 |
| `browser.list` | 有界地发现 Browser 运行时/租约状态；不修改状态。 | 无字段。 | 报告运行时可用性、版本/通道、排序后的租约名称、小写状态、空闲超时和有界的剩余空闲秒数。 | 运行时缺失时仍成功，返回 `runtimeAvailable: false` 且无租约；只读、非破坏性。 | Normal + Room；隐藏不透明 ID 和路径。 |
| `file.read` | 读取 UTF-8 文本、受支持的栅格图像及可选元数据；不运行 shell 或搜索进程，也不写入。 | 扁平 `path` 形式，或按序排列且形状相同的 `requests`（1–32 项），两种形式互斥；可选 `metadata`；文本可指定含首尾行的 `startLine`/`endLine`。 | 文本行为和范围不变。PNG/JPEG/WebP/GIF 图像以顶层 MCP image Content blocks 和 JSON `structuredContent` 元数据返回（不重复 base64）。静态 `image` 元数据包含检测到的 MIME 和尺寸；GIF 元数据包含画布尺寸，以及按序排列、带源时间戳的 PNG `frames`。批量 `results` 保持输入顺序，并带从 0 开始的 `index`；顶层 content 按项目顺序排列，随后按帧顺序排列。每张图像/帧的解码上限为 16 Mi 像素；GIF 遍历累计上限为 64 Mi 像素；每次 `tools/call` 序列化图像负载上限为 8 MiB。GIF 最多返回 8 个按时长均匀抽取的样本（包括播放端点），并以毫秒为单位保留源播放帧起始时间戳。 | 路径/UTF-8/大小/超长行及图像解码/范围错误均为类型化错误；批处理按序保留部分错误语义；可安全重试、非破坏性。 | Normal + Room；两个接入路径使用相同文件 schema。 |
| `file.search` | 在进程内执行字面量/正则搜索；不调用 shell 或外部搜索回退。 | 扁平 `path`/`query` 形式，或按序排列且形状相同的 `requests`（1–32 项），两种形式互斥；可选模式/glob/上下文/范围。 | 正常成功时只返回匹配项；是否提供裁剪/截断/跳过文件证据取决于具体情况。每次搜索范围之外还受 20k 文件/128 MiB 聚合扫描和约 1 MiB 响应上限约束。 | 无效正则/glob/路径或类型化参数错误；只读且有界。 | Normal + Room；两个接入路径使用相同搜索 schema。 |
| `file.edit` | 对 UTF-8 文件应用完整的 Codex apply-patch 补丁；不接受模型提供的 revision guard。 | `patch`，以及可选的 `needConfirm`；补丁可跨多个文件执行 Add/Delete/Update/Move。 | 一次完整预检、确定性加锁、一次确认、内部源文件复核、有界 diff，以及原子/临时文件提交。Add File 只能在现有路径策略范围内创建缺失父目录；Move 语义不变。 | 在提交前，路径/上下文/UTF-8/大小/竞态/确认失败都不会写入文件内容；预检拒绝或确认失败时会清理 Add 创建的空父目录。物理提交开始后，部分失败会按顺序报告，不跨文件回滚；审计保留内部 revision，但不会在响应中暴露。 | Normal + Room；仅 Standalone。 |
| `process.exec` | 启动一个受策略控制的本地进程；不提供直接且无界的 shell API。 | `program`；可选 `group`、直接传入的 `args`、`workingDirectory`、`waitSeconds`、`needConfirm`。 | 创建响应的完整内联输出/结果预算为 8 KiB；更大的响应使用上限为 2 KiB 的 stdout/stderr 共用预览。等待最多 30 秒；`group` 是经过验证、供人阅读的工作流标识。 | 策略/确认/容量/启动/退出结果会保留在 Process 历史中；使用 `process.status`、`process.output` 和 `process.result` 检查。 | Normal + Room；Hub Full 的 `process.exec` 对应相同契约。 |
| `process.batch` | 准入多个受管理进程；准入后不会隐式回滚其他子进程。 | `elements`（每项必须含 `program`）；可选父级 `group`、批次工作目录/等待/确认设置。 | 一次准入/确认边界；子进程按顺序排列并继承父级 group；等待最多 30 秒。 | 验证/容量拒绝时不会启动任何子进程；准入后的子进程失败分别记录并保留为 Process。 | Normal + Room；Hub Full 的 `process.batch` 对应相同契约。 |
| `process.status` | 检查一个 Process，或短暂等待其状态；不启动新工作，也不返回输出/结果正文。 | `processId`；可选 `waitSeconds`。 | Standalone 等待默认 5 秒，且限制为最多 30 秒；显式传入 `0` 时立即轮询。 | 只返回有界状态元数据；负载需使用 `process.output` 和 `process.result`。 | Normal + Room；Hub Full 的 `process.status`；HTTP 状态查询默认等待 5 秒、上限为 30 秒。 |
| `process.list` | 发现活动/近期 Process；不修改状态或准入新工作。 | 无必填字段；可选 `group`、`kind`、`state` 和 cursor。 | 返回有界的一页 Process 元数据。 | 只读；列表项不含输出或结果正文。 | Normal + Room；Hub Full 的 `process.list`；对应 HTTP `GET /v1/process`。 |
| `process.output` | 读取一个 Process 捕获的有界输出；不启动新工作。 | `processId`；可选 cursor 和 `maxBytes`。 | 从指定 cursor 开始（省略时从开头读取）；`maxBytes` 默认 8 KiB，上限为 32 KiB。 | 返回有界输出块；还有后续输出时附带续读 cursor。 | Normal + Room；Hub Full 的 `process.output`；对应 HTTP `GET /v1/process/{processId}/output`。 |
| `process.result` | 读取一个 Process 当前的结果投影；不启动新工作。 | `processId`；可选 `maxBytes`。 | MCP `maxBytes` 默认 8 KiB，上限为 512 KiB；HTTP 默认 8 KiB，上限为 512 KiB。 | 如实返回当前 Process 状态，仅在结果可用时包含结果；仅有状态元数据时绝不包含结果正文。 | Normal + Room；Hub Full 的 `process.result`；对应 HTTP `GET /v1/process/{processId}/result`。 |
| `process.cancel` | 请求取消 Process；不声称未经观察的终止已经发生。 | `processId`。 | 显式请求；无等待参数。 | 报告已观察到的取消/证据，并保留 unknown 或 detached 等结果。 | Normal + Room；Hub Full 的 `process.cancel`；对应 HTTP `POST /v1/process/{processId}/cancel`。 |
| `mcp.list` | 发现已配置的下游服务器或某一服务器的工具；不执行下游调用。 | 可选 `serverId`；省略时列出服务器。 | 有界服务器/工具元数据。 | 配置/传输错误为类型化错误；只读。 | Normal + Room；Hub Full 拆分为 `mcp.listServers`/`mcp.listTools`。 |
| `mcp.callTool` | 将一次下游 MCP 调用作为受管理 Process 启动；不是直接事务式调用。 | `serverId`、`toolName`；可选 `group`、JSON 对象 `arguments`、`waitSeconds`、`timeoutSeconds`。 | 参数上限为 256 KiB；等待默认 5 秒/最多 30 秒，等待超时不会取消 Process；调用超时默认 300 秒/最多 900 秒。 | 确认/策略/传输/下游错误都会保留；过大结果保留哈希/大小/预览；后续使用 `process.*`。 | Normal + Room；Hub Full 对应相同 Process 生命周期语义。 |
| `mcp.batch` | 通过一次聚合确认验证并准入 1–16 个下游调用；不回滚下游副作用。 | `calls` 中每项含 `serverId`/`toolName`；可选父级 `group`，以及每次调用的参数、`mode`、`failFast`、等待/截止时间。 | 默认并行；顺序执行需显式选择；聚合参数/响应上限为 2 MiB；全局/每服务器并发数为 8/2。子项继承父级 group；公开结果按顺序排列，因此关联 ID/索引留在内部。 | 准入是原子的；`failFast` 只跳过尚未启动的子项；按序保留子 Process 和聚合审计。 | Normal + Room；Hub Full 对应相同准入、group 和范围。 |
| `skills.list` | 发现有效 workspace Skill；不安装或执行。 | 可选 query/limit/active 筛选项。 | 有界摘要。 | 无效/不可读 Skill 会作为警告返回或省略；只读。 | Normal + Room；Hub Full 使用同一 Room workspace。 |
| `skills.read` | 读取一个 Skill 包/资源；不访问任意 workspace 文件。 | `id`；可选包内相对 `path`。 | 有界 Markdown/frontmatter/资源。 | Skill/资源无效或缺失时返回类型化错误；只读。 | Normal + Room；Hub Full 的 `skills.read` 对应相同契约。 |
| `skills.setActive` | 仅设置激活标志；不执行 Skill，也不授予权限。 | `id`、`active`。 | 激活状态持久化在 Agent 私有持久状态中；与 workspace 包内容分开存储。 | 无效 ID/Skill 返回类型化错误；状态变更会审计。 | Normal + Room；Hub Full 提供意图拆分的 `skills.activate`/`skills.deactivate` 别名。 |
| `skills.install` | 异步启动 Skill 安装；不接受内联网络负载或任意 URL 抓取。 | `id`、`source`；可选替换/激活/幂等设置。 | 返回 `installId`；适用来源及包/文件范围限制。 | 保留验证/提交失败；现有 Skill 归档/提交是原子的；后续使用 install get/cancel。 | Normal + Room；Hub Full 和 HTTP Room 安装端点对应相同契约。 |
| `skills.install.get` | 检查安装状态或短暂等待；不开始新安装。 | `installId`；可选 `waitSeconds`。 | 等待默认 5 秒，最多 30 秒；等待超时不会取消安装；终态 `pollAfterMs` 为 0。 | 状态有界且持久化；缺失/过期 ID 返回类型化错误；使用 `skills.install.cancel` 取消。 | Normal + Room；Hub Full/HTTP 安装查询端点对应相同契约。 |
| `skills.install.cancel` | 请求在提交前协作式取消；提交后不会强制回滚。 | `installId`。 | 幂等请求；必须显式调用，不会由等待超时隐式触发。 | 结果区分 cancelled/terminal/too-late；保留相关证据。 | Normal + Room；Hub Full/HTTP 安装取消端点对应相同契约。 |
| `skills.run` | 将活动 Skill 中的可执行项作为受管理 Process 运行；不接受任意路径。 | `id`、包内相对 `path`；可选 `group`、args/cwd/wait。 | 等待默认 5 秒，最多 30 秒；等待超时不会取消 Process；须显式使用 `process.cancel`。 | 策略/确认/脚本/退出失败会体现为 Process 状态；使用 `process.status`/`process.result` 检查。 | Normal + Room；Hub Full/HTTP Skills 运行端点沿用 group 和 Process 生命周期语义。 |
| `tmux.sessions` | 列出/创建/关闭持久会话；不提交命令。 | `action`；create/close 需要与 action 对应的 name/cwd；close 可能需要确认。 | 尽可能复用默认会话；cwd 受策略检查。 | close 具有破坏性；会返回类型化会话/策略/确认错误。 | Normal + Room；Hub Full 使用拆分后的 tmux 名称。 |
| `tmux.panes` | 列出/捕获窗格；不提交输入。 | `action`；capture 必须指定目标；list 可按会话筛选。 | 捕获历史默认 160 行，且有界。 | 与 action 不兼容的字段会被拒绝，不会忽略。 | Normal + Room；Hub Full 使用拆分后的 tmux 名称。 |
| `tmux.exec` | 向 shell 窗格提交结构化命令；不声称命令已经执行完成。 | `target`、`program`；可选 args/wait/capture/confirmation。 | 提交后的等待/历史均有界。 | shell/策略/确认错误；检查窗格或 Process 结果以确认完成状态。 | Normal + Room；Hub Full 的 `tmux.exec` 对应相同契约。 |
| `tmux.pasteText` | 将文本粘贴到非 shell 窗格/TUI；不执行 shell 命令。 | `target`、`text`；可选 `submit`、confirmation。 | 文本/历史均有界。 | 拒绝 shell 窗格；其他情况下窗格状态不变。 | Normal + Room；Hub Full 对应相同契约。 |
| `bootstrap` | 读取 Room 引导入口/指南清单；不通用读取文件或创建文件。 | 无字段。 | 指南摘要和包 revision 有界。 | 包缺失/无效时返回类型化错误/警告；只读、可安全重试。 | 仅 Standalone Room；Hub Full 提供 `bootstrap` 和 `room.bootstrap` 路由。 |
| `bootstrap.read` | 读取一份经过验证的引导指南；不读取任意路径。 | `id`。 | 有界 Markdown/frontmatter。 | 未知/无效/重复指南返回 `guide_not_found`；只读。 | 仅 Standalone Room；Hub Full 提供 `bootstrap.read` 和 `room.bootstrap.read`。 |
| `room.diary.active` | 读取活动的 Daily、Weekly 和 Monthly Room 日记文档；不修改内容。 | 空 JSON 对象。 | 返回三层有界 Markdown 结果；每层报告经过验证的路径、可用性和可选类型化问题。 | 按层报告文档缺失、不可读或 UTF-8 无效；只读、可安全重试。 | Standalone Room + Hub Full 活动 Room 路由；Coordinator 隐藏/拒绝。 |
| `room.diary.read` | 按语义层级和周期读取一份指定 Room 日记文档；不读取任意路径。 | `layer`、`period`；period 为 `current`、每日日期或按序排列的周/月日期范围。 | 一份有界 Markdown 文档。 | 拒绝无效周期；缺失或不可读文档作为类型化层问题返回。 | Standalone Room + Hub Full 活动 Room 路由；Coordinator 隐藏/拒绝。 |
| `room.notebook.recent` | 读取当前 Room Notebook 中近期、有界的 Markdown 预览；不修改内容。 | 可选 `limit`。 | `limit` 默认为 20，范围为 1–100；预览有上限。 | 缺失或格式错误的文档会转为有界警告；只读发现。此公开名称现在使用当前 Markdown DTO，而非已退役的 passage/JSONL 格式。 | Standalone Room + Hub Full 活动 Room 路由；Coordinator 隐藏/拒绝。 |
| `room.notebook.search` | 在当前 Room Notebook Markdown 中搜索不区分大小写的子串；不修改内容。 | 必填 `query`；可选 `limit`。 | `query` 不能为空，且最多 256 个 Unicode 字符；`limit` 默认为 20，范围为 1–100。 | 空查询、超长查询和无效 limit 返回类型化验证错误；只读。此公开名称现在使用当前 Markdown DTO，而非已退役的 passage/JSONL 格式。 | Standalone Room + Hub Full 活动 Room 路由；Coordinator 隐藏/拒绝。 |
| `room.notebook.read` | 读取一份指定的当前 Room Notebook Markdown 文档；不读取任意仓库路径。 | 必填且经过验证的 Notebook 相对 `.md` `path`。 | 一份有界 Markdown 文档；内容上限为 512 KiB。 | 不安全、非 Markdown、缺失或超大的路径返回类型化错误；只读。 | Standalone Room + Hub Full 活动 Room 路由；Coordinator 隐藏/拒绝。 |
| `room.state.list` | 列出确定排序的 Room state entity 文档；不修改内容。 | 空 JSON 对象。 | 返回 `State/entities` 下排序后的 `.md` entity。 | 跳过 symlink 和非文件；格式错误的仓库根目录返回类型化错误；只读。 | Standalone Room + Hub Full 活动 Room 路由；Coordinator 隐藏/拒绝。 |
| `room.state.read` | 读取一份指定的 Room state entity Markdown 文档；不读取任意路径。 | 必填且安全的 entity 文件名 stem `entity`。 | 读取 `State/entities` 下的一份有界 Markdown 文档；内容上限为 512 KiB。 | 不安全或缺失的 entity、超大内容均返回类型化错误；只读。 | Standalone Room + Hub Full 活动 Room 路由；Coordinator 隐藏/拒绝。 |
| `room.maintenance.status` | 检查 Room 仓库、脚手架、执行器、工作流、远端、同步和 slot 就绪状态；不修改内容。 | 空 JSON 对象。 | 状态、heads、缺失路径和五 slot 占用情况均有界。 | 只读；就绪状态的各个维度保持独立，失败会以类型化错误返回。 | Standalone Room + Hub Full 活动 Room 路由；Coordinator 隐藏/拒绝。 |
| `room.maintenance.submit` | 通过仓库所有者的执行器应用一至五项经过验证的 Room 维护请求。 | `items`，各项含唯一 `slot`/`payload`；可选 `mode` 和 `waitSeconds`。 | items 数量为 1–5；mode 为 `local` 或 `workflow`；wait 默认为 0 秒，上限为 30 秒。 | 分别报告准入、本地应用、语义提交和远端/工作流同步；操作具有破坏性，但限于仓库范围。等待超时不会取消维护操作，也没有单独的等待 API。 | Standalone Room + Hub Full 活动 Room 路由；Coordinator 隐藏/拒绝。 |

当前九个 Room 名称在本地 Agent MCP、Hub Full MCP 以及九个 `/v1/room/<namespace>/<action>` POST 路由上构成同一套语义契约。Hub 只选择已捕获的活动 Room 租约，并负责路由/回执，不拥有 Room 文件或内容。通用运行回执可能含有有界操作结果，但不是 Room 内容的权威来源。

## Hub Full 与 Coordinator 接口面

下表列出 Hub Full 配置档的执行接口面。Coordinator 配置档仅包含 `hub.info`、`agent.list`、`hub.run.list`、`hub.run.get`、`hub.process.status`、`hub.process.list`、`user.notify.channels` 和 `user.notify.send`；它不会派发 Agent 命令。表中标注的 Hub 工具使用 `agentId`；活动 Room 工具则有意路由到活动 Room Agent，不接收 `agentId`。

| Hub 公开名称 | 用途 / 不执行的操作 | 必填或条件必填输入 | 默认值与范围 | 失败与生命周期 | 一致性 |
|---|---|---|---|---|---|
| `hub.info`、`agent.list` | 检查 Hub/Agent 可用性和安全概要；不执行操作。 | `agent.list` 和 `hub.info` 均无请求体。 | 仅安全计数/配置概要。 | 只读；离线 Agent 报告为 unknown，而非健康。 | Coordinator + Full；无 Standalone 别名。 |
| `hub.run.list`、`hub.run.get` | 检查已持久化的 Hub 到 Agent 请求运行记录；不派发新命令。 | `run.get` 必须提供 `runId`；list 筛选项可选。 | 保留历史/结果有界。 | 明确保留超时/投递/迟到结果状态。 | Coordinator + Full；对应 HTTP 运行端点。 |
| `hub.process.status`、`hub.process.list` | 检查缓存的 Process 元数据；不派发执行。 | `agentId`；status 还必须提供 `processId`；list 支持 Process 筛选/cursor。 | 仅缓存快照；status/list 只有元数据，绝不包含输出或结果正文。 | 明确提供新鲜度和观察时间；快照不是实时等待结果。 | Coordinator + Full；HTTP 进程状态/列表端点是由 Agent 实时提供的视图。 |
| `process.exec`、`process.batch`、`process.status`、`process.list`、`process.output`、`process.result`、`process.cancel` | 通过选中的 `agentId` 提供与 Standalone 相同的受管理 Process 语义。 | `agentId` 加 Standalone Process 字段。 | Hub Full status 等待默认 5 秒/最多 30 秒；输出块默认 8 KiB/最多 32 KiB；结果最多 512 KiB。 | status 仅含元数据；输出/结果须调用各自端点。 | 仅 Full；HTTP `/v1/process`、`/v1/process/{processId}`、`/output`、`/result` 和 `/cancel` 生命周期端点对应相同契约。 |
| `tmux.listSessions`、`tmux.listPanes`、`tmux.capturePane` | 发现/读取持久窗格；不修改输入。 | `agentId`；按操作需要提供 capture/pane 目标。 | capture 默认 160 行；输出有界。 | 返回只读的类型化窗格/会话错误。 | 仅 Full；Standalone 将其合并为别名。 |
| `tmux.pasteText`、`tmux.exec` | 将非 shell 输入粘贴到窗格，或通过选定 Agent 提交 shell 命令。 | `agentId` 和按操作需要提供的 target/text/program。 | 等待/历史有界。 | 明确区分 shell 与非 shell，并说明策略/确认边界。 | 仅 Full；与本地语义相同。 |
| `tmux.createSession`、`tmux.closeSession` | 创建/关闭持久 workspace；不属于通用进程生命周期。 | `agentId`、name/cwd；close 可能需要确认。 | cwd 受策略检查；优先复用。 | close 具有破坏性；不隐式恢复数据。 | 仅 Full；与本地语义相同。 |
| `mcp.listServers`、`mcp.listTools` | 在受管理调用前发现下游 MCP 路由/schema。 | 汇总已连接 Agent 时，`mcp.listServers` 可省略 `agentId`；`listTools` 必须提供 `agentId`、`serverId`。 | 元数据有界。 | 只读；传输/Agent 错误明确。 | 仅 Full；对应 HTTP `/v1/mcp/servers|tools`。 |
| `mcp.callTool` | 将一次下游 MCP 调用作为受管理 Process 启动；不是 Agent 原生文件工具。 | `agentId`、`serverId`、`toolName`；可选 `group`、JSON 对象 `arguments`、有界等待/截止时间。 | 返回 Process 响应；使用 `process.status`、`process.output`、`process.result` 或 `process.cancel` 查看生命周期。 | Hub 没有原生 `file.read` 或 `file.edit`。通用 Process 桥接不是类型化的 MCP 图像内容接口；不得依赖它保留 `file.read` 图像 Content blocks。 | 仅 Full；对应 HTTP `/v1/mcp/callTool`。 |
| `room.bootstrap`、`room.bootstrap.read` | 访问活动 Room 引导清单/指南；不读取任意文件。 | read 必须提供指南 `id`；无 `agentId`。 | 与 Standalone Room 引导包采用相同范围/revision。 | Room 未激活/无效/未找到等错误明确，且只读。 | 仅 Full；Standalone 名称不带 `room.` 前缀。 |
| `room.diary.active`、`room.diary.read` | 通过捕获的活动 Room 租约读取当前或指定语义 Diary Markdown；不修改内容或访问任意路径。 | `diary.active` 使用 `{}`；`diary.read` 必须提供 `layer` 和 `period`；无 `agentId`。 | 返回有界层级结果；daily/weekly/monthly 周期规则由 Agent 强制执行。 | 活动 Room 缺失返回 `room_not_active`（404），租约冲突返回 `room_state_conflict`（409），传输等待超时为 504，Agent 语义错误仍返回 JSON 错误。 | 仅 Full；Coordinator 隐藏并拒绝 Room 工具。对应 HTTP `/v1/room/diary/active|read`。 |
| `room.notebook.recent`、`room.notebook.search`、`room.notebook.read` | 通过活动 Room 租约读取当前 Notebook Markdown 预览、搜索当前 Markdown，或读取一份指定且经过验证的文档。 | `recent` 可选 `limit`；`search` 必须提供 `query`，可选 `limit`；`read` 必须提供 `path`；无 `agentId`。 | `limit` 默认 20，范围 1–100；搜索 `query` 不能为空且最多 256 个 Unicode 字符；Markdown 内容上限为 512 KiB。`recent`/`search` 返回当前 Markdown DTO，不是已退役的 passage/JSONL 结果。 | 按契约将缺失/格式错误文件转为有界警告；不安全路径和验证错误为类型化错误。活动租约/传输状态遵循当前 Room 投影。 | 仅 Full；Coordinator 隐藏并拒绝 Room 工具。对应 HTTP `/v1/room/notebook/recent|search|read`。 |
| `room.state.list`、`room.state.read` | 列出或读取 Agent 所有的 `State/entities` 根目录下有界、非 symlink 的 Markdown entity。 | `list` 使用 `{}`；`read` 必须提供安全的 entity stem `entity`；无 `agentId`。 | 按序列出 entity；读取内容上限为 512 KiB。 | 不安全/缺失 entity 和格式错误根目录为类型化错误；活动租约/传输状态遵循当前 Room 投影。 | 仅 Full；Coordinator 隐藏并拒绝 Room 工具。对应 HTTP `/v1/room/state/list|read`。 |
| `room.maintenance.status` | 检查 Agent 所有的仓库、schema/scaffold、执行器、workflow/remote、sync 和五 slot 就绪状态；不修改内容。 | `{}`；无 `agentId`。 | 有界状态以及可选 heads/缺失路径。 | 只读；各就绪维度保持独立，错误明确。活动租约/传输状态遵循当前 Room 投影。 | 仅 Full；Coordinator 隐藏并拒绝 Room 工具。对应 HTTP `/v1/room/maintenance/status`。 |
| `room.maintenance.submit` | 通过现有 Agent 仓库所有者提交明确的语义维护请求；不是 passage append/update/remove 别名。 | `items`（1–5 个唯一 `slot`/`payload`）；可选 `mode`（`local`/`workflow`）和 `waitSeconds`（0–30）；无 `agentId`。 | wait 默认 0；local/workflow 结果和同步结果保持区分。等待超时不会取消维护；没有单独等待 API，也不隐含新增确认门。 | 现有锁、clean-tree、路径、预期变更、执行器、Git 和 workflow 控制仍具权威性。活动租约/传输状态遵循当前 Room 投影。 | 仅 Full；Coordinator 隐藏并拒绝 Room 工具。对应 HTTP `/v1/room/maintenance/submit`。 |
| `bootstrap`、`bootstrap.read` | Room 引导的 Full 配置档跨传输别名。 | read 必须提供 `id`；无 `agentId`。 | 相同的包范围/revision。 | 相同引导错误；只读。 | 仅 Full；这是有意提供的别名，不是对已删除工具的兼容别名。 |
| `skills.list`、`skills.read`、`skills.search`、`skills.active` | 发现/读取/搜索活动 Room Skill。 | 按 read/search 具体操作提供相应字段；无 `agentId`。 | 摘要/内容有界。 | 无效/缺失/过期 Skill 状态明确。 | 仅 Full；对应 HTTP Room Skills 端点。 |
| `skills.activate`、`skills.deactivate` | 仅更改 Skill 激活状态；不执行 Skill，也不授予权限。 | `id`；无 `agentId`。 | 幂等状态操作。 | 允许停用过期/缺失 Skill，并报告结果。 | 仅 Full；Standalone 的 `skills.setActive` 合并两种意图。 |
| `skills.install`、`skills.install.get`、`skills.install.cancel` | 活动 Room Skill 的异步安装生命周期。 | install 必须提供 `id`、`source`；get/cancel 必须提供 `installId`；无 `agentId`。 | 等待默认 5 秒、最多 30 秒；等待超时不会取消；取消必须显式请求。 | 提供协作式取消/原子提交证据。 | 仅 Full；对应 HTTP Room 安装端点。 |
| `skills.run` | 将活动 Skill 的可执行项作为受管理 Process 运行。 | `id`、`path`；可选 `group`、args/cwd/wait；无 `agentId`。 | 等待默认 5 秒、最多 30 秒；等待超时不会取消；使用 `process.cancel` 显式取消。 | 与 Standalone 使用相同的 Process/策略/取消契约。 | 仅 Full；HTTP Room Skill 运行端点沿用 group 和 Process 生命周期语义。 |

这九个 HTTP 路由中，JSON 格式错误或缺少必填请求字段时，提取器返回 `422 text/plain`。通过身份验证后的语义响应使用现有 JSON 投影：Room 未激活返回 `room_not_active`（404），租约冲突返回 `room_state_conflict`（409），传输等待超时返回 504；其他 Agent 侧语义验证错误仍为 400，除非适用现有的特定未找到/冲突映射。

## 审查规则

- 描述符变更必须同步更新对应行和针对该契约的 parity 测试；不要添加兼容别名，让无效调用看似有效。
- 必填字段说明准入条件，不保证执行成功。工具描述或属性描述中必须说明条件字段，尤其是 revision/absence guard、按 action 区分的 tmux 字段和 Process 后续操作。
- “原子”仅用于验证/准入/确认边界，绝不意味着回滚已经启动的进程、MCP 调用、通知或其他外部副作用。
- Agent Process 历史及其保留属于 `process.sqlite3`；旧 `jobs.sqlite3` 会有意保持原样且不可访问。
