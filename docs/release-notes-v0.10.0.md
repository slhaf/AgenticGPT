# Agentic GPT 0.10.0 发布说明

本文记录相对于 `v0.9.1` 的累计变化，适用于 Agent、Hub 和共享协议 `0.10.0`。当前仅完成仓库内的版本准备；尚未创建 `v0.10.0` 标签、推送或发布发行包。

本版本包含不兼容的配置、工具参数、HTTP/MCP 接口和私有状态布局调整。使用 Hub 的部署需要配套更新 Agent、Hub、调用方及导入的 OpenAPI；Standalone/Local 部署也需要更新客户端请求。旧接口和字段不提供运行时兼容别名。

## 主要变化

| 范围 | 0.10.0 的行为 |
| --- | --- |
| 配置与 CLI | Config v2 使用显式 `mode`、`profile`、`toolsets.enabled` 和嵌套 `hub`；统一通过 `agentic-gpt run` 启动。初始化提供全屏键盘 TUI、本地化和显式非交互流程。 |
| 受管执行 | Job 生命周期改为 Process；命令执行使用 Bash `command`，支持受控 Shell 初始化。`process.read` 合并状态、输出和下游 MCP 结果读取，提供有界等待、响应预算及游标。 |
| 确认与策略 | 通过 `acquireConfirmation` 申请本次主动许可；命令和路径策略统一使用显式通配符规则与 `defaultAction`，初始化把预设写入配置，运行时不再维护另一份内置策略。 |
| 文件 | 采用 apply-patch 编辑，支持有界批量读取和图片结果，并强化新建父目录、文件身份及 revision 检查。 |
| 事件 | 新增 Agent 本地持久事件收件箱及 `event.list/get/mark`；有界提醒与进程结果、调用等待分别处理。 |
| Room | Room v2 使用 Agent 所有的仓库，提供 bootstrap、语义读取、状态和维护提交接口；Hub 将远程 Room 操作路由到活动 Room Agent。 |
| 接入与 Browser | Standalone 可选启用 worker 自有 HTTP MCP `/mcp`，支持 bearer 认证或 connector OAuth；新增 Browser runtime 管理和可选 Linux Native Messaging host。 |
| 状态与恢复 | 事件、Process 和 Skill 私有状态统一放在 `~/.agentic_gpt/state/`；修改连接 `agentId` 不切换本地持久记录。强化 Hub 连接代际、请求归属、恢复和实时 Toolset 准入。 |
| 验证与发布 | 锁定 Rust 工具链，增加 live 契约门禁、独立 latest-stable 检查和统一迁移脚本集成测试；Linux 发行包包含 Agent、Hub、Browser host。 |

详细行为以[配置参考](configuration.zh-CN.md)、[接口参考](interfaces.md)、[工具合同矩阵](tool-contract-matrix.md)和[Browser 部署指南](browser-self-hosted.zh-CN.md)为准。`experimental/chrome-control-poc` 不属于生产 Browser runtime。

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

中间版本的 `process.status/output/result` 也已合并到 `process.read`。旧的参数数组必须按 Bash 引用规则构造脚本，不能直接拼接；`waitSeconds` 到期只结束本次等待，不取消执行。Hub 后续调用须复用返回的 `agentId/processId`。完整字段、响应预算、取消证据和历史边界见[Process 切换说明](process-cutover.md)。

### 确认、命令和路径策略

- 移除 `needConfirm`、`confirmMethod`，对应工具改用 `acquireConfirmation`。`method` 为单个 `ntfy` 或 `freedesktop`，省略时使用配置中的默认 fallback。
- `acquire` 通常省略为 false；`tmux.pasteText` 默认 true，主动确认启用时只有获准才粘贴，拒绝、超时或渠道不可用都终止。
- 主动获准可覆盖本次命令及路径策略的 Allow/Confirm/Deny。拒绝或取消终止；其他操作的超时、不可用按配置规则处理，命中 Confirm 时复用失败结果，不再等待第二次。
- Process 可在 `acquireConfirmation.read/write` 声明本次具体访问范围，非空时必须 `acquire=true`。获准后范围仅对本次进程生效；`file.*` 和 tmux 不接受这两个范围字段，文件请求本身确定许可范围。
- 下游 MCP 的原有审批行为保持不变。

命令 `policy` 的 `allow/confirm/deny` 改为字符串模式数组；路径 `pathPolicy` 改为独立的 `read` 和 `write` 策略。三类策略都显式包含 `defaultAction`，优先级固定为 `deny > confirm > allow`，无匹配时使用默认动作。初始化命令默认 Confirm，读写路径默认 Deny。

命令模式按 argv 匹配：token 内 `*` 匹配字符，尾部独立 `**` 匹配剩余参数。路径 `*` 不跨目录，`**` 表示递归范围，`/dir/**` 包含目录本身。两种策略中的单独 `*` 都匹配全部目标；将其放在 allow 中会广泛放行，仍受同组更高优先级的规则约束。主动确认不取消工具准入、文件完整性校验或其他资源限制，路径策略也不等同于 OS 沙箱挂载。

旧 `program/argsPrefix` 规则对象、`writeRoots/readOnlyRoots/denyRoots` 需要显式迁移。原配置可覆盖内置规则的行为改为统一优先级，迁移报告会列出可识别的冲突。完整配置示例、批准范围及 sandbox 边界见[确认与策略格式迁移](operations.md#确认与策略格式迁移)。

### Config v2、Toolsets 与 Room

运行时要求显式的 `mode/profile/toolsets` 和新策略格式，旧顶层 `hubUrl/hubTransport/agentSecret` 改为嵌套 `hub`。`profile` 提供默认 Toolset preset；显式 `toolsets.enabled` 决定实时发现和普通工具调用准入，包含 Hub 下发的调用。

Room v2 的接口、Notebook/Diary 投影和维护合同与 0.9.1 不同，调用方需更新请求及响应处理。需要保持 Room 时还应确认 `room.repositoryRoot`、对应仓库及 Room Toolset。参见[工具合同矩阵](tool-contract-matrix.md)及[接口参考](interfaces.md)。

## 升级步骤

1. 停止使用本次用户目录的 Agent、supervisor、worker 和本地服务，并备份现有配置及状态。准备配套的 Agent/Hub 二进制与客户端接口切换。
2. 从新代码仓库执行统一脚本。它自动读取当前用户的 `~/.agentic_gpt/config.json`，仅接受 `--write`：

   ```bash
   # 预览配置策略变更，并复制、转换及校验旧状态副本
   python3 scripts/migrate-agentic.py
   # 核对假设、冲突和状态验证结果后，备份并执行两项迁移
   python3 scripts/migrate-agentic.py --write
   ```

3. 如果配置仍含 `limits.maxActiveJobs`，将该键改为 `maxActiveProcesses`，保留所需值；同时检查 `limits.maxConcurrentTasks`。脚本不迁移此键，直接 `config import` 可能报告无法导入旧 limits 并使用默认值。
4. 对于缺少 Config v2 选择字段或仍含旧顶层 Hub 字段的配置，在终端运行 `agentic-gpt config import`。它打开标准初始化 TUI；复核实际运行模式、profile、Toolsets、嵌套 Hub 和其他配置后保存。旧文件未指定运行模式时，导入默认使用 Standalone；Hub/Local 用户须在界面选回所需模式。策略迁移应先完成，`config import` 不转换旧策略。
5. 用 `agentic-gpt config show` 检查有效配置，更新客户端及 Hub 的 OpenAPI 导入，再启动新部署，核对 Process、事件和 Skill 记录。部署冒烟应使用隔离 HOME，不能把真实用户配置当作测试夹具。

**统一脚本覆盖的是策略配置和受支持的私有状态布局，不是全部 0.9.1 配置或客户端的自动升级器。** 新状态已存在或没有当前身份的旧目录时，状态项会跳过，配置策略仍可迁移。预览不替换配置或发布新状态，但会创建私有锁文件。

旧 `jobs.sqlite3` 不导入 Process 历史，也不删除或重命名。受支持的旧事件、Process、Skill 状态先在副本校验，再发布到新布局，旧源目录保留。配置变更有私有备份；普通失败及可处理的中断会尝试恢复配置和撤回本次发布状态。断电、强制终止或回滚失败仍需检查完整性。详情见[私有状态迁移指南](development/agent-state-migration.zh-CN.md)。

## 版本及发布范围

Agent、Hub 和共享 Protocol 包使用 `0.10.0`；未来发布标签为 `v0.10.0`。Browser host 和 apply-patch 当前各为 `0.1.0`，OpenAPI 的 `info.version` 独立维护，Console 使用自己的 Gradle 产品版本。

Linux 发行包构建目标为 `x86_64-unknown-linux-gnu` 和 `aarch64-unknown-linux-gnu`，包含 `agentic-gpt`、`agentic-gpt-hub`、`agentic-browser-host`。Android Console 的提醒状态转换及降级调度有所改善，但不由 Rust 发布流程构建或发布。Browser 的环境部署及外部 connector 行为仍需独立验证，不能由本地契约门禁推断。

## 本地验证

本次在隔离 HOME、XDG、TMP 和 D-Bus 环境完成以下本地检查：

- `cargo fmt --all -- --check`、`cargo check --workspace --offline` 和严格 all-targets Clippy 通过。
- `cargo test --workspace --offline`：518 个通过、0 个失败、1 个既有手动用例忽略；统一迁移脚本的 8 个集成测试通过。
- Agent/Hub 构建通过，两者的 `--version` 均输出 `0.10.0`；Cargo metadata 与锁文件中的三个配套包版本一致。
- `scripts/check_contract_parity.py` 的 OpenAPI schema 和本地真实 Agent/Hub 跨端检查通过。
- 文档文件链接和 `git diff --check` 通过。

契约门禁未验证生产 tunnel executable transport；部分恢复交错使用临时数据库检查点证据，不能等同于复现全部生产时序。检查使用临时数据，未读写真实用户配置或状态。

未运行远端 Actions、交叉目标打包、生产部署或 Console 构建；未创建标签、推送或发布。验证与实际发布的边界见[发布指南](development/releasing.md)。
