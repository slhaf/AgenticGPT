# Agentic GPT 0.10.0 发布说明

本文记录相对于 `v0.9.1` 的累计变化，适用于 Agent、Hub 和共享协议 `0.10.0`。发布标签为 `v0.10.0`，Linux 发行包由标签触发的 GitHub Actions 构建和发布。

本版本包含不兼容的配置、工具参数、HTTP/MCP 接口和私有状态布局调整。使用 Hub 的部署需要配套更新 Agent、Hub、调用方及导入的 OpenAPI；Standalone/Local 部署也需要更新客户端请求。旧接口和字段不提供运行时兼容别名。

## 主要变化

| 范围 | 0.10.0 的行为 |
| --- | --- |
| 配置与 CLI | Config v2 使用显式 `mode`、`profile`、`toolsets.enabled` 和嵌套 `hub`；统一通过 `agentic-gpt run` 启动。初始化提供全屏键盘 TUI、本地化和显式非交互流程。 |
| 受管执行 | Job 生命周期改为 Process；命令执行使用 Bash `command`，支持受控 Shell 初始化。`process.read` 合并状态、输出和下游 MCP 结果读取，提供有界等待、响应预算及游标。 |
| 确认与策略 | 通过 `acquireConfirmation` 申请本次主动许可；统一 `policy` 管理命令规则、具体路径根和可选 bubblewrap 挂载；初始化把预设写入配置，运行时不再维护另一份内置策略。 |
| 文件 | 采用 apply-patch 编辑，支持有界批量读取和图片结果，并强化新建父目录、文件身份及 revision 检查。 |
| 事件 | 新增 Agent 本地持久事件收件箱及 `event.list/get/mark`；有界提醒与进程结果、调用等待分别处理。 |
| Room | Room v2 使用 Agent 所有的仓库，提供 bootstrap、语义读取、状态和维护提交接口；Hub 将远程 Room 操作路由到活动 Room Agent。 |
| 接入与 Browser | Local/Standalone 新增 `local stdio` 桥接已有 Agent；Standalone 可选启用 HTTP MCP `/mcp`，支持 bearer 认证或 connector OAuth；新增 Browser runtime 管理和可选 Linux Native Messaging host。 |
| 状态与恢复 | 事件、Process 和 Skill 私有状态统一放在 `~/.agentic_gpt/state/`；修改连接 `agentId` 不切换本地持久记录。强化 Hub 连接代际、请求归属、恢复和实时 Toolset 准入；关闭 Toolset 后，启动和恢复不再创建对应资源或重启排队安装。 |
| 验证与发布 | 锁定 Rust 工具链，增加 live 契约门禁和独立 latest-stable 检查；Linux 发行包包含 Agent、Hub、Browser host。 |

详细行为以[配置参考](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/configuration.zh-CN.md)、[接口参考](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/interfaces.md)、[工具合同矩阵](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/tool-contract-matrix.md)和[Browser 部署指南](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/browser-self-hosted.zh-CN.md)为准。`experimental/chrome-control-poc` 不属于生产 Browser runtime。

## 不兼容接口变化

### Job、Process 与 Bash 输入

| 0.9.1 调用或字段 | 当前替代 |
| --- | --- |
| `job.get` | `process.read` |
| `job.list` / `job.cancel` | `process.list` / `process.cancel` |
| Job 响应的 `jobId` | Process 响应的 `processId` |
| `process.exec` 的 `program`、`args`、`workingDirectory` | `command`、`cwd` |
| `process.batch` 的旧 argv 调用集合 | `elements`，每项包含 `command`，可覆盖 `cwd` |
| `limits.maxActiveJobs` | `limits.maxActiveProcesses` |
| HTTP `/v1/jobs` 及 Job 查询、取消路径 | 当前 `/v1/process`、`/{processId}/read` 和取消路径 |

中间版本的 `process.status/output/result` 也已合并到 `process.read`。调用方需要注意：

- 旧参数数组必须按 Bash 引用规则构造脚本，不能直接拼接。
- `waitSeconds` 到期只结束本次等待，不取消执行。
- Hub 后续调用须复用返回的 `agentId/processId`。

完整字段、响应预算、取消证据和历史边界见[Process 切换说明](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/process-cutover.md)。

### 确认、命令和路径策略

- 移除 `needConfirm`、`confirmMethod`，对应工具改用 `acquireConfirmation`。`method` 为单个 `ntfy` 或 `freedesktop`，省略时使用配置中的默认 fallback。
- `acquire` 通常省略为 false；`tmux.pasteText` 默认 true，主动确认启用时只有获准才粘贴，拒绝、超时或渠道不可用都终止。
- 主动获准可覆盖本次命令及路径策略的 Allow/Confirm/Deny。拒绝或取消终止；其他操作的超时、不可用按配置规则处理，命中 Confirm 时复用失败结果，不再等待第二次。
- Process 可在 `acquireConfirmation.read/write` 声明本次具体访问范围，非空时必须 `acquire=true`。获准后范围仅对本次进程生效；`file.*` 和 tmux 不接受这两个范围字段，文件请求本身确定许可范围。
- 下游 MCP 的原有审批行为保持不变。

配置格式与默认行为：

| 策略 | 格式 | 默认行为 |
| --- | --- | --- |
| `policy.enabled` | 统一策略开关 | 默认 true；关闭后不执行自动命令与路径判定，也不启用 bubblewrap；显式主动确认仍生效 |
| `policy.command` | `allow/confirm/deny` 命令模式数组及 `defaultAction` | 无匹配时 Confirm；优先级为 `deny > confirm > allow` |
| `policy.path` | 具体文件或目录根：`readOnly/write/deny`，以及 `defaultAction` | 无匹配时 Deny；默认动作仅接受 Allow 或 Deny，`write` 同时授予读取 |
| `policy.bubblewrap` | Linux Process 沙箱开关 | 默认 false；同时启用统一策略后，挂载由 `policy.path` 派生 |

命令模式按 argv 匹配：token 内 `*` 匹配字符，尾部独立 `**` 匹配剩余参数；单独的模式 `*` 为全局规则。路径列表不接受通配符，加载时会展开 home 并规范化路径；`deny` 遮蔽对应子树。`workspaceRoot` 仅指定默认工作目录，不隐式授予路径访问。

主动确认不取消工具准入、文件完整性校验或其他资源限制。获准的具体路径范围可用于本次 Process 的授权与临时 bubblewrap 挂载；不修改持久配置。命令、路径和 bubblewrap 的有效重载只影响之后接纳的请求，已接纳 Process 保留执行配置快照。

旧扁平命令 `policy`、`pathPolicy`、`sandbox`、`program/argsPrefix` 规则对象及旧路径规则不自动转换。升级前保留原配置，按当前结构显式重建命令规则与具体路径根。完整示例和兼容性见[命令与路径策略](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/configuration.zh-CN.md#命令与路径策略)与[策略配置兼容性](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/operations.md#策略配置兼容性)。

### Config v2、Toolsets 与 Room

Config v2 的要求：

- 显式提供 `mode/profile/toolsets` 和新策略格式。
- 旧顶层 `hubUrl/hubTransport/agentSecret` 改为嵌套 `hub`。Standalone/Local 可显式启用默认关闭的 `hub.enabled`，建立仅用于确认的辅助连接；它不接纳 Hub 下发的工具执行。
- `profile` 提供默认 Toolset preset；显式 `toolsets.enabled` 决定实时发现和普通工具调用准入，包含 Hub 下发的调用。关闭 Tmux/Browser 后，下次启动不准备对应资源；Skills 关闭时不重启排队安装。开关不撤销已接纳任务，也不自动关闭已有会话或租约。
- 配置中的 `~`、`~/…` 和 `file:~/…` 在相应加载或消费边界展开，初始化生成密钥引用时使用实际目标的绝对路径。

Room v2 的接口、Notebook/Diary 投影和维护合同与 0.9.1 不同，调用方需更新请求及响应处理。需要保持 Room 时还应确认 `room.repositoryRoot`、对应仓库及 Room Toolset。参见[工具合同矩阵](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/tool-contract-matrix.md)及[接口参考](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/interfaces.md)。

## 升级步骤

1. 停止使用同一用户目录的 Agent、supervisor、worker 和本地服务，备份配置、数据库及其 sidecar、旧状态目录和 Room 仓库。准备配套的 Agent/Hub 二进制与客户端接口切换。
2. 按[配置参考](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/configuration.zh-CN.md)重建或复核 Config v2：确认运行模式、profile、Toolsets、嵌套 Hub 和统一 `policy`。旧策略不会自动转换；`config import` 也不是旧策略转换器。将 `limits.maxActiveJobs` 改为 `limits.maxActiveProcesses`，同时检查 `limits.maxConcurrentTasks`。
3. 当前私有状态统一位于 `~/.agentic_gpt/state/`。仓库已移除一次性迁移脚本，Agent 启动不会扫描、合并或转换旧状态。需要保留旧布局数据时，使用与旧部署匹配的版本化源码和迁移资料，在副本上完成转换与校验；保留原目录，避免让新实例覆盖未经核验的数据。旧 `jobs.sqlite3` 不自动导入 Process 历史。
4. 用 `agentic-gpt config show` 检查有效配置，并在隔离 HOME 中验证策略与接口。更新客户端和 Hub 的 OpenAPI 导入后，再启动真实部署，核对 Process、事件、Skill 记录及 Room 仓库。

旧版本状态资料仅供历史追溯，见[私有状态迁移历史说明](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/development/agent-state-migration.zh-CN.md)；备份范围和当前状态所有者见[运维指南](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/operations.md)。当前版本不提供完整的 0.9.1 自动升级器。

## 本地 stdio MCP 接入

`agentic-gpt local stdio --config PATH` 为每个 stdio MCP 客户端启动独立桥接进程，将 MCP 字节双向转发到已运行 Agent 的 Unix socket。适用于 Local 和 Standalone；必须先启动 Agent，Hub 模式不提供该 socket。

桥接不自动启动 Agent、重连或重放请求。各客户端拥有独立 MCP session，仍共享 Agent 状态与 16 个 Unix 连接总额度；同 UID 限制及 `local:` 来源保持生效。EOF 结束对应桥接，不关闭 Agent 或其他连接。示例及错误行为见[本地 stdio 桥接说明](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/standalone-runtime.md#通过-stdio-桥接到已有-agent)。

## 版本及发布范围

Agent、Hub 和共享 Protocol 包使用 `0.10.0`；发布标签为 `v0.10.0`。Browser host 和 apply-patch 当前各为 `0.1.0`，OpenAPI 的 `info.version` 独立维护，Console 使用自己的 Gradle 产品版本。

Linux 发行包构建目标为 `x86_64-unknown-linux-gnu` 和 `aarch64-unknown-linux-gnu`，包含 `agentic-gpt`、`agentic-gpt-hub`、`agentic-browser-host`。Android Console 的提醒状态转换及降级调度有所改善，但不由 Rust 发布流程构建或发布。Browser 的环境部署及外部 connector 行为仍需独立验证，不能由本地契约门禁推断。

## 验证与发布链路

发布准备所用代码提交 `2f59553` 的[远端 CI](https://github.com/slhaf/AgenticGPT/actions/runs/38523507500)已通过格式、工作区编译、严格 all-targets Clippy、工作区测试、Agent/Hub 构建和 live 契约门禁。后续发布准备仅调整说明及工作流的说明文件输入，不修改运行时实现。

推送 `v0.10.0` 后，Release 工作流会在标签提交上重新检查版本配对、格式、编译、测试、Agent/Hub 构建与契约，再构建两个 Linux 目标并上传发行包及 `SHA256SUMS`。发布正文采用本文件，执行状态以 [Actions 记录](https://github.com/slhaf/AgenticGPT/actions/workflows/release.yml)为准。

上述 Rust 检查不证明 ARM 设备实际运行、Console 构建、生产部署、外部 connector 或生产 tunnel 行为。验证与发布范围见[发布指南](https://github.com/slhaf/AgenticGPT/blob/v0.10.0/docs/development/releasing.md)。
