# Agentic GPT 0.11.0 发布说明

本文记录相对于 `v0.10.0` 的累计变化，适用于 Agent、Hub 和共享 Protocol `0.11.0`。本版本新增 Room 邮件、Room 密钥管理和配置 API，并扩展组件热更新、调整主动确认参数及错误分类。

使用 Hub 的部署须配套更新 Agent、Hub、调用方及导入的 OpenAPI；Local／Standalone 调用方也须更新确认请求。新增功能的配置和限制见下文，升级步骤见文末。

## Room 邮件

新增 `room.email.list/read/search/attachment/send/operate`，提供后台 IMAP 收信、提醒、本地阅读与搜索、附件读取／导出、SMTP 发送及已读／标星／垃圾箱操作。

- **按需启用：** 邮件默认关闭，需要 `room` 工具集及完整的单账户 IMAP／SMTP 配置；密码使用 `env:` 或 `file:` 引用。当前不支持 OAuth2，仅同步 INBOX，SMTP 仅发送纯文本，不发送附件，也不提供其他文件夹管理或永久删除。
- **持久保存：** 邮件索引、同步进度和发送记录归 Agent 所有，保存在 `~/.agentic_gpt/state/email.sqlite3`，原文位于 `state/email/`，不进入 Room 文档仓库。内容按保存天数和容量上限清理后保留元数据及摘要，正文／附件读取返回 `email_content_expired`；已导出的 workspace 附件由用户管理。
- **阅读与提醒：** 正文支持文本、离线图片和显式加载远程图片的视图；读取不自动标为已读。首次导入历史邮件不逐封提醒，后续收信和正常重启后的增量同步沿用持久进度。
- **发送与删除策略：** `sendPolicy`、`trashPolicy` 默认 `confirm`，未获明确批准不执行，全局 `policy.enabled=false` 不放宽要求。垃圾箱操作要求服务商支持 IMAP MOVE 并可识别垃圾箱目录。`sent` 只表示 SMTP 服务器接受；响应丢失或 `unknown` 时先查发送记录，自动恢复不重发，新一次 `send` 可能产生重复邮件。

配置、六项工具、服务器邮件身份恢复及资源限制见[Room 邮件使用指南](email.zh-CN.md)。

## Room 子工具集与配置目录

Room 子 API 分别由 `room.bootstrap`、`room.diary`、`room.notebook`、`room.state`、`room.maintenance`、`room.email`、`room.secret` 控制。
工具集统一控制发现、准入和资源启停，Email/Secret 配置不再包含单独的 enabled 字段。
API 名称与活动 Room 路由不变，关闭能力仍可在 TUI 独立页面预配置。详细规则见[Room 子工具集配置](configuration.zh-CN.md#room-子工具集配置)。

## Room 密钥管理

新增 `room.secret.list/get/upsert/delete`，通过 Agent 主机上的 Bitwarden Password Manager CLI（`bw`）连接独立 Vaultwarden 账号，管理以 `site/account` 标识的账号组。

- **按需启用：** Secret 默认关闭，需要 `room.secret` 子工具集、可执行的 `bw`、Vaultwarden Personal API 凭据和主密码。凭据使用 `env:`／`file:` 引用；项目不自动下载 CLI，也不按固定 CLI／服务端版本号设置启动门槛。
- **读取与修改：** 列表只返回站点和账号；`get` 返回密码或当前 TOTP 验证码；`upsert` 创建或合并更新密码／TOTP，省略字段保留原值；`delete` 将整个 Login item 移入回收站。默认读取允许，创建／更新和删除要求明确确认；拒绝、超时或通道不可用时不披露或修改密钥。
- **同步与结果不确定：** 每次操作先触发或复用同步。查询默认最多等待 3000 毫秒，明确网络故障或等待预算耗尽时可读取可解锁的旧缓存，并标记 `stale/lastSyncedAt`；认证、TLS 或解析等错误不降级读取。旧缓存未命中返回 `secret_presence_unknown`。写入必须先同步成功，结果不确定时返回 `secret_write_outcome_unknown`，不自动重试。
- **敏感内容边界：** CLI 使用独立的私有状态目录；密码、seed、验证码和 session 不进入 Agentic 的普通日志、工具上报、Hub 持久回执或 Agent 重放账本。`get` 向调用方返回明文，调用方的会话和模型上下文由调用方管理。

两组新增 Room 工具在 Standalone／Local 使用 Agent 本地入口；Hub Full 将请求路由到活动 Room Agent，HTTP 为 `/v1/room/email/<action>` 和 `/v1/room/secret/<action>`，请求不接收 `agentId`。Coordinator 不提供这些操作。

准备步骤、账号匹配、TOTP 支持范围、同步限制及已有联调环境见[Room 密钥使用指南](room-secret.zh-CN.md)。

## 配置 API 与组件热更新

新增 `config.manual`、`config.get`、`config.update`，复用 Agent 所有的配置目录、校验、持久化和运行态应用服务。支持 Agent MCP 入口、Hub Full Apps MCP 和 Hub HTTP `/v1/config/manual|get|update`；Hub 调用须指定目标 `agentId`，修改的是该 Agent 的配置。

- `config` 工具集在所有初始化预设中默认关闭，已有显式列表不自动追加；可通过本地 `agentic-gpt config toolset enable config` 启用。本地 CLI／TUI 配置管理仍可直接使用。
- 路径使用字符串段数组，值使用原生 JSON；一批 `set/unset/add/remove` 按序作用于最新持久配置，最终统一校验并保存一次。`dryRun` 提供脱敏预览；预览不锁定后续实写的配置版本。
- `config.updatePolicy` 默认 `confirm`，整批修改必须明确获准；超时、取消或确认不可用均不写入。修改权限自身也按修改前的有效策略判断，全局命令策略开关不能绕过。
- 保存与应用分别报告。`config.get` 提供持久 `value`、差异时的 `appliedValue`，以及 `applied/restart_required/applying/failed` 状态。`saved=true` 不代表组件全部就绪，组件失败也不整份回滚已保存配置。Hub 对 update 最多等待 180 秒；超时或连接中断后应先查询，不自动重试修改。

热更新范围扩展到 Hub 连接、HTTP 入口、Browser、邮件账户、Secret 后端、Room 根目录和 Skills 安装并发，由各组件协调排空和替换。繁忙或准备失败时报告对应应用错误，能够保留的旧组件继续服务；配置更新不自动重发邮件、重放密钥写入或网页操作。

`mode/profile/agentId/workspaceRoot` 和 Tunnel 身份、凭据、client 启动参数仍须显式重启整个实例。Standalone supervisor 自动恢复 worker 时保留原启动契约，并读取最新可热更新字段；待重启字段不阻止其他设置应用。Browser 切换可能使旧租约及 JS 会话失效，HTTP endpoint 变更后客户端需重新 initialize；Room 根目录切换不迁移旧内容。

完整字段、授权和重配置边界见[配置 API](configuration.zh-CN.md#配置-api)与[热加载与重启边界](configuration.zh-CN.md#热加载与重启边界)。

## 不兼容的确认请求变化

删除 `acquireConfirmation.acquire`，传入对象即主动请求确认。`reason` 必填，原文最多 100 个 Unicode 字符，去除首尾空白后必须非空；可选 `method` 仍为 `ntfy` 或 `freedesktop`。

```json
{"acquireConfirmation":{"reason":"更新项目配置并保存修改","method":"ntfy"}}
```

旧 `acquire=false` 调用改为省略整个对象；旧 `acquire=true` 调用删除 `acquire` 并填写原因。Process 可在对象内声明本次 `read/write` 范围，文件和 tmux 请求不接受这两个字段。`tmux.pasteText` 的基础决策为 Confirm，无主动确认对象也需要确认，未获准不写入窗格。

ntfy／freedesktop 改用简短通知：主动确认显示调用方的 `reason`，自动确认显示有界规则或固定原因与最多 30 字符的预览。修复长命令因预览超过 1000 字符而将通道视为不可用的问题；freedesktop 根据服务标记能力转义正文。实际执行继续使用完整请求，下游 MCP 的专用审批语义不变。详细迁移见[调用方升级](configuration.zh-CN.md#调用方升级)。

## 错误处理、文件与构建

- **保留实际失败原因：** 区分文件不存在、权限不足、非目录和复查失败；只有观察到路径或身份变化才报告冲突。Process 历史读取失败返回 `process_history_read_failed`，不再伪装成进程不存在或缺少历史部分的成功列表。Browser Native Messaging 区分正常 EOF 与截断帧，安装、下载、恢复和初始化错误保留实际阶段。
- **Hub 与 Room 错误：** 离线／未就绪使用 HTTP 503，连接或角色冲突使用 409，实际等待超时使用 `agent_request_timeout`（504），响应通道关闭使用 `agent_response_channel_closed`（502），内部请求失败使用 `agent_request_failed`（500）。Room 错误响应保留公共事件面板；HTTP JSON-RPC 保留结构化内部错误，区分无效请求与无效参数。调用方应按错误类别处理，传输或回执持久化失败不证明用户操作尚未执行。
- **命令与文件：** 命令路径预检不把尚未创建的输出目标当成不存在错误，也不把 `echo/printf/true/false` 的字符串参数推断为文件访问。启用策略时，含 `cd/pushd/popd` 的脚本需要确认，不按初始目录推断相对路径授权。文件读取、搜索与编辑内部使用类型化结果；修复替换或移动的 rename 失败后遗留暂存文件的问题。
- **HTTP MCP：** 凭据变为不可用时撤下旧入口，避免继续接受旧认证。
- **ARM 构建机：** 新增 `scripts/dist-linux-on-arm.sh`，在 aarch64 Linux 原生构建 ARM 包，并通过 cargo-zigbuild／Zig 交叉构建 x86_64 包；产物仍包含 Agent、Hub 和 Browser host。工具要求见[发布指南](development/releasing.md#linux-产物)。

错误码、HTTP 状态及执行证据边界见[接口说明](interfaces.md#错误原因与执行状态)。

## 升级步骤

1. 备份当前配置、Agent 私有状态及 Room 仓库，停止待替换的实例。邮件状态及 Secret CLI 状态属于私有资料，不随 Room Git 提交保存；备份范围见[私有状态布局](operations.md#私有状态布局)。
2. 配套更新 Agent／Hub 至 `0.11.0`，更新调用方的确认参数、错误处理及 OpenAPI 导入。从早于 `0.10.0` 的版本升级时，先核对 [0.10.0 升级步骤](release-notes-v0.10.0.md#升级步骤)中的配置、接口和状态布局变化。
3. 按需启用新增能力：配置 API 开启 `config` 工具集；邮件和 Secret 按各自指南配置后再启用。使用 Hub 的 Room 功能时，确认活动 Room Agent 和对应工具集／子开关。
4. 启动后检查 `agent.info`，需要使用配置 API 时再通过 `config.get` 区分保存值与已应用值；遇到 `failed/restart_required` 按具体字段处理。邮件发送结果不确定时先查询邮件记录；密钥写入结果不确定时先核对 Vaultwarden 或成功同步后的结果，避免重复操作。

Agent、Hub 和共享 Protocol 包采用 `0.11.0`；Browser host 与 apply-patch 仍各自使用 `0.1.0`，OpenAPI 和 Console 的版本来源独立。Rust/Linux 发布流程不构建或发布 Console。发布预检、标签与 Linux 产物的对应关系见[发布指南](development/releasing.md)；已有邮件与 Vaultwarden 联调的具体环境和覆盖范围分别见两份使用指南。
