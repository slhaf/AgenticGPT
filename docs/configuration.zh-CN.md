# 配置说明

Agentic GPT 的 Standalone、Local Unix MCP 与连接 Hub 的 Agent 共用一份本地 JSON 配置。默认路径：

```text
~/.agentic_gpt/config.json
```

对应页面：[配置说明](configuration.md)。

路径字段支持 `~` 和 `~/…` 的当前用户主目录展开，不支持 `~user` 或 shell 环境变量替换。
初始化与加载配置时会展开 `workspaceRoot`；路径策略的 `readOnly`、`write`、`deny`，
Shell 初始化文件、Room 根目录及 Tunnel client 路径在各自使用边界展开。
`file:~/…` 密钥引用读取时同样展开；初始化生成密钥文件后，配置引用使用实际写入目标的绝对路径。

磁盘文件是稀疏的 Config v2 投影，始终包含权威的顶层 `mode`（`standalone`、`hub` 或
`local`）和 `profile`（`normal` 或 `room`）；省略的值会从有效默认值重建。`config show`
显示完整的有效配置，而 Agentic 管理的写入会保持文件稀疏。

从以下命令开始：

```bash
agentic-gpt config init
agentic-gpt config show
```

[`config.example.json`](../config.example.json) 是稀疏的 Config v2 示例：以 Standalone 为优先入口，不包含可用凭据，示例下游 MCP server 全部保持 disabled，同时只保留 Hub 模式需要的有意义字段。

## 全屏初始化行为

只有当 stdin、stdout、stderr 全部是终端时，`agentic-gpt config init` 才会打开键盘驱动的
全屏配置界面。管道或重定向的流不会隐式回退：裸跑的非 TTY 初始化会返回本地化的可操作
错误且不会写入文件。脚本、CI、重定向输出或其他自动化场景请使用
`config init --non-interactive`。默认模式是 `standalone`，默认配置档是 `normal`。

### 模式与工具集

模式与配置档是两个独立选择：

- `--mode standalone|hub|local` 选择运行时连接方式与配置形状。
- `--profile normal|room` 选择默认 toolset preset。normal preset 启用除 `room`、`config` 外的
  namespace；room preset 额外启用 `room`，仍不默认选中 `config`。配置档本身不会固定最终 runtime surface 或数量，
  因为显式的 `toolsets.enabled` 选择具有权威性。

可用 namespace 为 `agent`、`file`、`mcp`、`process`、`skills`、`tmux`、`browser`、`room`、`config`。
`config` 默认不选中，已有显式列表也不会自动追加；关闭时三个配置工具均不可调用，本地 CLI/TUI 配置管理仍可使用。
逻辑上的 `room` namespace 包含 Room bootstrap（`bootstrap` 与 `bootstrap.read`）以及全部
`room.*` 工具。选择只会过滤既有的 Normal/Room advertised names，不会暴露 dispatch-only alias。
使用以下命令固定选择；将 `<namespace>` 替换为上列 namespace 名称：

```bash
agentic-gpt config toolset ls
agentic-gpt config toolset enable '<namespace>'
agentic-gpt config toolset disable '<namespace>'
```
`ls` 会列出全部 namespace、当前启用/禁用状态及其所含工具的简短说明。`enable` 和
`disable` 成功后会明确输出被修改的 namespace 与结果状态。

同一个 `toolsets.enabled` 数组也可以直接编辑 JSON：

- 有效修改会在 worker 运行时热加载，并对后续工具发现与调用生效，无需重启。
- 无效候选会保留上一次有效的 live selection。
- 该选择同时约束 Hub 下发及本地 MCP/Unix/HTTP 的普通工具调用。
- Room 命令先检查实时 `room` namespace；禁用时，共享操作层返回
  `room_toolset_required`。

已有运行时能力和 Hub Room Profile 限制继续生效；基础 Event API、CLI 管理入口等例外见
[操作准入边界](interfaces.md#agent-接入与操作边界wp2)。

例如，显式固定 normal 选择时写成：

```json
{
  "profile": "normal",
  "toolsets": {
    "enabled": ["agent", "file", "mcp", "process", "skills", "tmux", "browser"]
  }
}
```

如果之后切换 profile，这个显式选择仍然具有权威性。

### 非交互式初始化

脚本需要确定性结果时，请使用以下实际 CLI 语法，并提供不应保留占位符的值：

```bash
agentic-gpt config init --non-interactive
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt config init \
  --mode standalone \
  --profile room \
  --tunnel-id 'tunnel_<assigned-id>' \
  --tunnel-api-key file:"$HOME/.agentic_gpt/secrets/tunnel-api-key" \
  --non-interactive
```

不提供值时，非交互式 Standalone + Normal 模板使用安全占位符，例如 `tunnel_replace-me`
以及 Agentic home 下的 `file:` 引用。命令会报告替换 tunnel ID、配置所引用密钥等待处理
操作；不会自动创建或配置密钥材料。Hub 缺少值时同样会报告待配置的 Hub URL 与代理密钥。
`--agent-secret` 会暴露在 shell 历史和本地进程检查中，因此优先使用交互式隐藏输入。
使用 `file:` 或 `env:` 引用可以避免把 tunnel secret 放进命令行；明文 tunnel API key
会被拒绝。

### 交互式流程

全屏流程为 Basic → Connection → Optional settings → Review → Completion。Local 的 Connection
页可选启用 Hub 辅助连接；Standalone 与 Hub 也在该页配置各自连接字段。

- 交互模式下的命令行 flag 只是可编辑的预填值，不会锁定字段或跳过页面。
- Optional settings 可配置身份/显示名称、工作区/路径策略、确认方式/语言、限制、沙箱、
  Toolsets、Events 与下游 MCP server 集合。
- Toolsets 从配置档 preset 开始；一旦显式编辑，其 namespace selection 具有权威性。
  Room 设置仅当当前有效的 selection 启用 `room` namespace 时才会提供。profile 通过默认
  preset 影响该 selection；显式 `toolsets.enabled` 会覆盖 preset，因此显示条件是有效
  namespace selection，而不是 `profile` 字段本身。
- 只有 Standalone 模式会出现 tunnel-client 覆盖和 Hub reporting。Hub 与 Local 模式不会
  显示这些 tunnel 部分。
- Standalone 和 Local 的 Connection 页默认关闭 Hub 辅助连接。打开后可配置 Hub URL、传输方式、
  Agent ID 和密钥；这条连接可为 `ntfy` 确认提供响应，但不会接受 Hub 执行命令。连接仍会发送
  Hello 和心跳。Standalone 的上报由独立的 `tunnel.hubReporting.enabled` 控制；旧配置启用上报时
  仍会保持辅助连接。Local 只使用确认连接，不新增上报。
- 不选可选部分时会保留模板默认值。

键盘操作如下：

- Tab/Shift+Tab 与方向键移动焦点。
- Enter 编辑或触发当前操作。
- Esc 返回；根 Basic 页面中是 no-op。编辑态按 Esc 只结束编辑，不会取消初始化。
- Ctrl+C 取消初始化。

输入方式按配置含义区分，前置表单与 Review 使用相同的允许值和校验：

- Hub 传输方式、邮件 TLS 和操作策略使用选项选择；不需要手输枚举名称。
- 端口、数量、时长和容量只接受数字；数值范围与单位见字段说明，提交时仍执行配置校验。
  SHA-256 输入只接受十六进制字符，提交时检查长度。
- HTTP MCP Bearer、邮件密码和 Room Secret 凭据使用引用编辑器：选择未设置、`file` 或 `env`，
  再填写文件路径或环境变量名；也可按 C 清空。启用中的必填引用仍须通过配置校验。
  HTTP MCP 的文件引用要求绝对路径；其余引用按各自规则校验。
  编辑器显示路径或变量名以便修正，普通表单、Review 和最终 JSON 预览保持脱敏；这里不输入密码明文。
- `httpMcp.allowHosts` 使用“自动”“允许全部”“指定列表”三种模式，分别保存为 `null`、
  `["*"]` 和主机数组；指定列表逐项添加、删除和编辑，无须手写 JSON。
  “允许全部”会关闭此项 Host 限制；自动模式的含义见下文 HTTP MCP 说明。
- 引用和主机编辑器用 Tab/方向键移动、Enter 选择或编辑、A 应用暂存值；主机列表用 N 添加、
  D 删除。Esc 放弃本次弹层修改，Ctrl+C 取消整个配置流程。应用暂存值不会立即写盘。

Review 会隐藏密钥，可跳回 Basic、Connection 或可选 section 编辑；最终确认前不会写入配置、
备份或密钥文件。本功能只承诺键盘全屏流程；鼠标、inline、dashboard 与 Windows 行为不在
本功能契约内。

Events 设置可选择 `low`、`medium`、`high` 或 `off`，并在 Review 中复核保存；这只是配置
界面，不提供运行时事件收件箱界面。

`config init --language auto|zh-CN|en` 选择 CLI 界面语言。使用 `auto` 时依次检查
`LC_ALL`、`LC_MESSAGES`、`LANG`，都没有匹配时使用英文界面。显式的 `zh-CN` 或 `en`
优先于环境变量。这个界面选择与持久化的 `confirmationLanguage` 不同；后者控制 runtime
发出的确认提示语言，可在可选配置 section 或通过 `config set` 设置。

Optional settings 还提供以下编辑入口：

- 下游 MCP server 集合编辑器。
- `Shell` section：选择初始化文件为 Default、Disabled 或 Path，并仅在 Path 时编辑路径；三种
  选择都会写入并可在 Review 复核。

`config init --non-interactive` 没有用于填写 `mcpServers` 集合的 CLI flags。自动化初始化后可
使用 `config mcp` 配置 server；命令模式使用 `config allow`、`config confirm`、`config deny`；
路径基线使用 `config path` 的 `default-action`、`readonly`、`write`、`deny` 子命令。

## 各 runtime 必需项

| 配置组 | Standalone | Local Unix MCP | 连接 Hub 的 Agent |
| --- | --- | --- | --- |
| 公共 identity/workspace/policy | 必需 | 必需 | 必需 |
| `tunnel` | 必需 | 忽略 | 忽略 |
| `httpMcp` | 可选，仅在 Standalone 中生效 | 忽略 | 忽略 |
| `hub`（`url`、`transport`、`agentSecret`） | 仅可选 Hub reporting/ntfy relay 使用 | 忽略 | 必需 |
| 公开 Hub/VPS | 不需要 | 不需要 | 需要 |
| 启动命令 | `agentic-gpt run` | `agentic-gpt run` | `agentic-gpt run` |

所有模式的 JSON 类型仍保留嵌套 `hub` section，便于同一配置在不同 runtime 之间切换。Standalone 与 Local 的命令链路不经过 Hub。Standalone 只有在启用 `tunnel.hubReporting.enabled` 或使用 Hub-backed `ntfy` 确认时才会使用 Hub 字段；显式配置的非活动 section 会保留。

## 优先采用 Standalone 的配置

将 `tunnel_<assigned-id>` 替换为分配给这台机器的 Tunnel ID。

```bash
agentic-gpt config init
agentic-gpt config set agentId laptop
agentic-gpt config set confirmationProvider.channels '["freedesktop"]'

install -d -m 700 "$HOME/.agentic_gpt/secrets"
touch "$HOME/.agentic_gpt/secrets/tunnel-api-key"
chmod 600 "$HOME/.agentic_gpt/secrets/tunnel-api-key"
read -rsp "Tunnel API key: " AGENTIC_TUNNEL_API_KEY
printf '\n'
printf '%s' "$AGENTIC_TUNNEL_API_KEY" > "$HOME/.agentic_gpt/secrets/tunnel-api-key"
unset AGENTIC_TUNNEL_API_KEY

agentic-gpt config set tunnel.tunnelId 'tunnel_<assigned-id>'
agentic-gpt config set tunnel.apiKey file:"$HOME/.agentic_gpt/secrets/tunnel-api-key"
agentic-gpt config set tunnel.client.autoDownload true
agentic-gpt run
```

将 `profile` 设为 `room` 会选择启用所有 namespace 的 Room preset（例如
`agentic-gpt config set profile room`）；如果存在 `toolsets.enabled`，则以它为准。

私有持久状态不绑定 `agentId`。更改连接身份不会切换事件、进程历史或 Skill 激活/安装状态；需要独立持久状态的实例应使用独立 `HOME`。启动不会迁移旧目录，当前仓库也不提供通用迁移脚本；旧部署数据的备份与升级边界见[私有状态布局](operations.md#私有状态布局)。

## 顶层字段

| 字段 | 用途 |
| --- | --- |
| `mode` | 权威运行时分派：`standalone`、`hub` 或 `local`。 |
| `profile` | 默认能力/toolset preset：`normal` 或 `room`。 |
| `toolsets` | 已启用的 tool namespace；显式 `enabled` 列表会覆盖配置档 preset，`config` 默认不选中。 |
| `config` | 配置 API 的操作级权限；`updatePolicy` 为 `allow` 或 `confirm`，默认 `confirm`。 |
| `agentId` | 本地连接身份，用于 Hub 路由、协议身份及 `~/.agentic_gpt/runtime/agent/<agentId>/` 下的 runtime/socket 路径；私有持久状态统一位于 `~/.agentic_gpt/state/`，不按此字段分目录。 |
| `displayName` | summary/reporting 中的人类可读机器名称。 |
| `workspaceRoot` | 默认工作目录及 `.agentic-gpt-audit.jsonl` 所在位置；不会隐式扩大路径基线。 |
| `backupLimit` | Agentic 管理配置写入时保留的备份数量。 |
| `confirmationProvider` | 有序的本地/远程确认通道。 |
| `confirmationLanguage` | `en` 或 `zh-CN`。 |
| `shell` | Bash 命令执行的 Shell 初始化文件设置。 |
| `mcpServers` | `mcp.*` 转发的下游 MCP server。 |
| `policy` | 统一的自动命令／路径策略：`enabled`、`command`、`path` 与 `bubblewrap`。 |
| `limits` | Process 并发限制与最大活动 Process/Skill/MCP Job 容量。 |
| `skills` | Skill package/install 限制与网络策略。 |
| `room` | Room 仓库根目录、时区、日记日界线、维护策略及可选 Vaultwarden 密钥服务。 |
| `email` | 单账户 IMAP/SMTP、同步间隔、本地内容保存限制和发送/垃圾箱策略；见[邮件使用指南](email.zh-CN.md)。 |
| `tunnel` | Standalone tunnel-client 来源、secret 引用与可选 reporting。 |
| `browser` | 可选的高级 Browser runtime 覆盖；Browser runtime 自动 discovery 受 Browser 工具集开关控制；managed runtime 自动 provisioning 需显式启用 `browser.managed.autoProvision`，默认 `false`。 |
| `hub` | `enabled` 在 Standalone/Local 启用确认用的 Hub 辅助连接，默认 `false`；Hub 模式仍始终使用命令连接。Standalone 的上报由 `tunnel.hubReporting.enabled` 独立控制。 |
| `httpMcp` | 可选的 Standalone hidden worker 所有入站 Streamable HTTP MCP endpoint。 |
| `events` | 持久事件收件箱中 `low` 严重级别事件的保留时长，以及内部事件严重级别覆盖。 |

## 配置 API

`config.manual`、`config.get`、`config.update` 由目标 Agent 的同一配置服务执行，支持 Agent stdio、Unix 与 HTTP MCP，以及 Hub HTTP 和 Apps MCP。Hub 请求额外提供 `agentId`，修改的是该 Agent 的配置，不是 Hub 服务端配置。各入口保留自身认证与响应封套，并沿用公共 `events` 面板；HTTP 路由和完整输入见[配置接口](interfaces.md#配置-api)。

先通过本地管理入口启用工具集：

```bash
agentic-gpt config toolset enable config
agentic-gpt config manual --section config
```

`config.updatePolicy` 默认 `confirm`，整批修改须取得明确批准；拒绝、取消、超时或确认通道不可用均不写入，也不因 `policy.enabled=false` 放行。修改 `updatePolicy` 自身按修改前的有效策略判断；候选中的 `allow` 不能批准它自己。`manual/get` 不请求写入确认。此策略只约束配置 API 的 `update`，本地 CLI/TUI 管理不受工具集或该策略限制。

三个工具的请求如下；示例为工具参数，不含传输封套：

```json
{"section":"limits","detail":"full"}
```

`config.manual` 无筛选时返回目录，`section` 浏览章节，`query` 检索；二者可组合，`detail` 为 `summary` 或默认 `full`。结果包含文档 `content` 和匹配数 `matches`，说明真实路径、类型、缺省、关联约束及支持的操作，不读取当前凭据。

```json
{"paths":[["limits"],["shell","initFile"],["mcpServers","example.http","enabled"]]}
```

`config.get` 接受 1–64 个路径，每个路径是字符串段数组；空路径 `[]` 表示整个配置。事件名 `process.completed` 和服务器 ID `example.http` 各占一段，点不是 API 的分隔符。不支持数组下标。返回的 `items` 中：

| 字段 | 含义 |
| --- | --- |
| `value` | 最新持久配置合并默认值后的值；敏感项脱敏。 |
| `explicit` | 当前稀疏文件是否包含该路径，不表示用户历史上是否改过。 |
| `exists=false` | 目录支持此路径，但当前没有该条目；不同于存在且值为 `null`。未知路径返回错误。 |
| `redacted=true` | 此项含脱敏内容；不可将脱敏占位符写回作为凭据。 |
| `appliedValue` | 已应用值与持久值不同时返回，可为 `null`。 |
| `applyStatus` | `applied`、`restart_required`、`applying` 或 `failed`。 |
| `error` | 可选的配置或组件应用错误，不含凭据原文。 |

`applied` 描述配置应用状态，不证明外部邮箱、Vaultwarden、浏览器或网络依赖均已可用；连接和实际调用仍可失败。

```json
{
  "changes": [
    {"op":"set","path":["limits","processResponseBytes"],"value":16384},
    {"op":"add","path":["skills","allowedHosts"],"value":"skills.example.com"},
    {"op":"unset","path":["events","internalOverrides","process.completed"]}
  ],
  "dryRun": true
}
```

`config.update` 接受 1–64 个按序执行的 `changes`，请求大小最多 1 MiB。`value` 使用原生 JSON，不把数字、布尔值或数组装进字符串。操作作用于提交时最新的持久配置，不使用 `revision` 或 `ifRevision`；同一目标以后提交的操作为准。

- `set` 替换值或整个子树；设置对象不会与旧对象自动合并。
- `unset` 移除显式字段或 map 条目，恢复缺省；必需的 `mode/profile/toolsets/toolsets.enabled` 不可删除。
- `add/remove` 仅用于目录标为集合的列表，添加已存在值或移除不存在值均无变化。集合包括工具集、命令/路径规则、Skills 允许主机及 HTTP 允许主机；有序的确认通道和 Browser 模块目录用 `set` 整体替换。
- `null` 是值，和 `unset` 不同。例如 `shell.initFile=null` 禁用初始化文件，`unset` 恢复默认路径。CLI 的文本解析便利约定不改变 API 语义。

整批操作完成后统一校验，成功才保存一次；操作或校验失败不保存本批变更。写入在 rename 后发生目录同步错误时，响应会区分文件已替换，不能把该错误解释为未保存。

`dryRun=true` 不保存本次修改，返回 `saved=false`、`dryRun=true` 及脱敏的 `before/after/expectedEffect`；后续实写会重新基于届时最新配置，预览不是并发前置条件。API 的实写等待保存和本次局部应用尝试结束，返回 `saved`、`changes` 及各项实际 `applyStatus`。`saved=true` 不代表全部已生效；组件失败不回滚整份已保存配置，启动字段仍标记待重启。读取 `config.get` 核对结果，不因传输超时自动重试。查询、预览、公共事件及公开调用回执均脱敏；Hub 私有可靠派发记录保留待重放的原请求。

## 持久事件收件箱配置

`events` 配置所有 `low` 事件（含外部注入）的保留时长，以及内部事件的严重级别覆盖；缺少整个 section 时使用下列默认值：

```json
{
  "events": {
    "lowTtlSeconds": 86400,
    "internalOverrides": {}
  }
}
```

`lowTtlSeconds` 以秒为单位，默认 `86400`（24 小时），最小值为 `0`；取值还须确保事件到期时间不超过 RFC 3339 可表示范围（年份 `0..=9999`），边界由运行时校验。内部事件的默认严重级别为 `low`。以下稳定事件类型可配置覆盖；这些已知类型未配置覆盖时继承 `low`，未知类型键会被拒绝：

| 事件类型 | 默认严重级别 |
| --- | --- |
| `process.completed`、`process.failed`、`process.rejected`、`process.cancelled`、`process.timed_out`、`process.detached`、`process.unknown_after_restart`、`process.skipped` | `low` |
| `skill_install.completed`、`skill_install.failed`、`skill_install.cancelled` | `low` |

每个覆盖值只能是 `low`、`medium`、`high` 或 `off`。`off` 不创建该类型的事件；把某个已知事件类型的覆盖值设为 CLI 的 JSON `null` 会移除该覆盖并恢复继承的 `low`。事件收件箱的查询、计数、过期和处理语义见[持久事件收件箱](interfaces.md#持久事件收件箱)。

`config set` 的值是单个 shell 参数；CLI 的枚举值使用普通文本；原生 JSON 批量操作中的枚举值仍为 JSON 字符串。可使用 `config keys` 查看配置目录、`config show` 查看有效配置：

```bash
agentic-gpt config keys --section events
agentic-gpt config set events.lowTtlSeconds 86400
agentic-gpt config set events.internalOverrides.process.completed high
agentic-gpt config set events.internalOverrides.skill_install.failed off
agentic-gpt config show
agentic-gpt config set events.internalOverrides.process.completed null
```

`agent.info` 的安全摘要不包含事件级别覆盖；事件配置不包含密钥。

## Standalone 入站 HTTP MCP 端点

Standalone 可以由 hidden worker 提供可选的入站 MCP endpoint：

```text
http://<host>:<port>/mcp
```

它默认关闭，并且独立于 tunnel transport 与 Hub。配置形状与默认值如下：

```json
{
  "httpMcp": {
    "enabled": false,
    "host": "127.0.0.1",
    "port": 8765,
    "publicUrl": null,
    "bearerToken": "",
    "allowHosts": ["localhost", "127.0.0.1", "::1"]
  }
}
```

路径固定为 `/mcp`，不能通过配置修改。没有 `publicUrl` 时，endpoint 接受配置的
`Authorization: Bearer ...` 凭据，保持直接本地使用；配置 `publicUrl` 后，另外
提供 Standalone ChatGPT connector OAuth contract：

- `GET /.well-known/oauth-protected-resource/mcp` 是 canonical path-specific
  protected-resource metadata；`GET /.well-known/oauth-protected-resource` 是根路径
 兼容 alias。
- `GET /.well-known/oauth-authorization-server` 与
  `GET /.well-known/openid-configuration` 是 authorization-server/OpenID alias。
- `GET|POST /oauth/authorize` 与 `POST /oauth/token` 实现唯一 `agentic:mcp`
  scope 的 authorization-code flow。

### OAuth 公开地址与回调

`publicUrl` 必须是非空 HTTPS origin，不能包含 userinfo、除空路径或 `/` 之外的
path、query 或 fragment；末尾 `/` 会被规范化。它只是公布的外部 origin，不是
路由或 proxy 覆盖：`host` 与 `port` 仍是本地 bind 坐标，origin 可以保持
loopback/private。`publicUrl` 不是 secret，在 `config show`、Review、诊断和 TUI
中始终显示且不隐藏；`bearerToken` 仍是 secret 引用，解析后的值不会暴露。

connector 只接受以下精确 ChatGPT callback family：

- `https://chatgpt.com/connector/oauth/<suffix>`；
- `https://chatgpt.com/connector_platform_oauth_redirect`。

OAuth contract 还有以下边界：

- 只接受 `agentic:mcp`。
- 不提供 refresh token、`offline_access`、dynamic client registration、generic
  registration 或任意 redirect。
- authorization code 与 access token 是 opaque、listener-local 的内存记录，有过期、
  单次 code 消费和 listener 替换/token 内容轮换时撤销机制。
- Standalone authorization 页面以及 tool/profile surface 使用 Standalone 语义，不是 Hub 的
  `Hub API key`、profile 或 routing contract。

### 请求防护与 session 生命周期

OAuth 路由与 `/mcp` 共用 listener Host 防护。缺失或 malformed Host 会被拒绝，
不允许的 authority 返回 forbidden；不带 port 的 allowlist 项匹配任意 port，
带 port 的项目必须精确匹配。`null` 与严格等于 `["*"]` 仍是明确的全量放行值。
如果存在 `Origin`，必须精确匹配配置的 `publicUrl`；缺失 Origin 的
server-to-server 请求仍然有效，不添加 permissive CORS。未配置 `publicUrl` 时，
直接 bearer 失败仍使用普通 Bearer challenge；配置后，MCP challenge 指向
path-specific protected-resource metadata URL。

rmcp transport 使用有状态的 Streamable HTTP/SSE：

- 客户端必须先初始化 session。
- listener rebind 或关闭会终止 session，客户端必须重新 initialize。
- 直接 bearer 内容轮换会原地更新认证并保留既有 session，同时原子撤销 OAuth 记录。
- 无效候选保留上一次有效 listener 及其 state。

`allowHosts` 是 HTTP MCP transport 使用的 DNS-rebinding 防护：

- 默认列表只允许 `localhost`、`127.0.0.1` 和 `::1`。
- 非空合法 host/authority 列表只允许列表中的请求。
- `null` 或严格等于 `["*"]` 时明确允许任意 Host 值；wildcard 不能与其他项混用。
- 空数组会被拒绝，不会被解释为全量允许。
- malformed authority、wildcard 混用以及其他非法值都会被拒绝。

非 loopback listener 应显式填写 authority allowlist，或有意使用上述两个全量放行值。
`host` 必须是可 bind 的非空值，且不能含空白或控制字符；`port` 必须在 `1..=65535`
范围内。

### Bearer secret

`bearerToken` 永远是 secret 引用，不能填写 literal credential。只有 endpoint disabled
时才允许为空；启用后必须使用以下一种形式：

- `file:/absolute/path`（会去除末尾一个 LF 或 CRLF）；
- `env:VARIABLE_NAME`。

引用值必须可用、非空且不含控制字符。配置校验会拒绝明文、格式错误的引用，以及缺少
引用的 enabled endpoint。解析后的 token 只保留在内存中。`config show`、Review、诊断、
日志和 `agent.info` 都会同时隐藏引用和解析值；文件或环境中的 secret 由外部 secret
管理流程负责配置与轮换。

### 使用 CLI 配置 HTTP MCP

受控 registry 在 `http-mcp` section 中提供这些键：

```bash
agentic-gpt config set httpMcp.bearerToken env:AGENTIC_HTTP_MCP_TOKEN
agentic-gpt config set httpMcp.host 127.0.0.1
agentic-gpt config set httpMcp.port 8765
agentic-gpt config set httpMcp.publicUrl https://mcp.example.com
agentic-gpt config set httpMcp.allowHosts '["mcp.example.com"]'
agentic-gpt config set httpMcp.enabled true
```

要明确关闭 Host 过滤，可使用 JSON `null` 或 `["*"]`；要让 OAuth 路由 fail closed
并回到本地直接 bearer 模式，请清除可选 origin：

```bash
agentic-gpt config set httpMcp.allowHosts null
agentic-gpt config set httpMcp.allowHosts '["*"]'
agentic-gpt config set httpMcp.publicUrl null
```

`allowHosts` 是 JSON array 或 `null`，不是逗号分隔字符串。`publicUrl` 必须是 HTTPS
origin；`config set` 会在写入前校验完整候选值。被拒绝的值不会修改配置或 backup。
`config mcp` 仍专门管理下游 `mcpServers` registry，不配置这个入站 listener。

确定性部署可使用 `config init --non-interactive` 的全部 endpoint flags：

```bash
agentic-gpt config init --non-interactive \
  --mode standalone \
  --http-mcp-enabled true \
  --http-mcp-host 127.0.0.1 \
  --http-mcp-port 8765 \
  --http-mcp-public-url https://mcp.example.com \
  --http-mcp-bearer-token env:AGENTIC_HTTP_MCP_TOKEN \
  --http-mcp-allow-hosts '["mcp.example.com"]'
```

这些 flags 仍须通过 HTTPS origin、secret 引用和 allow-host 规则；启用 endpoint 却没有
token 引用时，会在写入 config 或 backup 前失败。交互式 `config init` 会把同样的 flags
作为 Connection 字段的可编辑初始值。HTTP MCP enabled toggle、host/port、非 secret 的
public-origin editor、secret-reference editor 与 JSON array/`null` allow-host editor
都可在 Review 前修改；public-origin 输入为空时会清除它。
`config import --config PATH [SOURCE]` 会识别已有的 `httpMcp` object，把字段带入同一套
交互式 editor；用户可在最终一次提交前修正或关闭 endpoint。Review 中 bearer 引用显示为
`[REDACTED]`，但会显示 `publicUrl`；取消或校验失败不会写入任何内容。

### Browser runtime 覆盖

Browser 是否启用不由 runtime 配置决定，`toolsets.enabled` 仍具有权威性。没有
`browser` section（或使用 `browser: {}`）时，普通 runtime discovery 不变。显式 descriptor
用于开发、特殊部署或恢复等高级场景，不是普通 managed-runtime 安装路径。
`codexCliPath` 可选，因为官方 Browser launcher 只有在可用时才会导出
`CODEX_CLI_PATH`。配置 `runtime` 时其他 scalar 均必填，所有配置路径必须是绝对路径；
`nodeModuleDirs` 默认为空列表：

```json
{
  "browser": {
    "runtime": {
      "appVersion": "<official-runtime-version>",
      "channel": "<runtime-channel>",
      "nodeReplPath": "/absolute/path/to/node_repl",
      "nodePath": "/absolute/path/to/node",
      "browserClientPath": "/absolute/path/to/browser-client.mjs",
      "browserServicePath": "/absolute/path/to/browser-service.mjs",
      "codexHome": "/absolute/path/to/runtime-home",
      "codexCliPath": "/optional/absolute/path/to/codex-or-compatible-cli",
      "nodeModuleDirs": ["/absolute/path/to/node_modules"]
    }
  }
}
```

显式 source 在启动或局部重配置时选择并具有权威性：

- 无效 descriptor 会关闭 Browser capability，不会回退到 Desktop discovery，但 Agentic 仍会
  继续启动。
- 修改它会协调重建 Browser runtime；旧租约与 JavaScript 会话可能失效。准备失败会报告组件错误，不能把配置保存成功视为 Browser 已可用。
- `docsRoot` 与 `trustedCodePaths` 由内部派生，不是配置字段。
- 本设置不管理 installer、downloader 或 runtime cache。

Local/Standalone 的 Unix MCP 路径位于 `~/.agentic_gpt/runtime/agent/<agentId>/`，Standalone supervisor 的运行时目录位于 `~/.agentic_gpt/runtime/tunnel/<agentId>/`。这些是按连接身份分隔的 runtime 路径；事件、Process 和 Skill 私有持久状态仍共享 `~/.agentic_gpt/state/`，不会随 `agentId` 切换。

未知顶层字段会在 load/write round trip 中保留。`limits` 等严格嵌套对象会拒绝已经删除的 v0.8 字段。

## Tunnel 配置

```json
{
  "tunnel": {
    "tunnelId": "tunnel_<assigned-id>",
    "apiKey": "file:/home/me/.agentic_gpt/secrets/tunnel-api-key",
    "client": {
      "version": null,
      "cacheDir": "~/.agentic_gpt/cache/tunnel-client",
      "autoDownload": true,
      "executable": null,
      "downloadUrl": null,
      "sha256": null
    },
    "hubReporting": {
      "enabled": false,
      "detail": "metadata"
    }
  }
}
```

`tunnelId` 不能为空。`apiKey` 只接受：

- `file:/absolute/or/expanded/path`
- `env:VARIABLE_NAME`

明文值会被拒绝。引用文件末尾允许一个 LF 或 CRLF，并会被去除；空值和控制字符会导致启动失败。

Tunnel client 来源优先级：

1. `client.executable`：受信任的本地可执行文件，可选 `sha256` 每次启动校验。
2. `client.downloadUrl` + 必需的 `sha256`：精确自定义 HTTPS archive。
3. Managed manifest/cache：当前平台内置固定版本的官方 tunnel-client。

`version: null` 使用内置 pin。`autoDownload: false` 要求 verified cache 已存在。

`hubReporting.enabled` 默认 false。启用后 Hub 连接是 reporting-only，绝不会接收执行命令。`detail` 可为 `metadata` 或 `full`；隐私边界见 [`standalone-runtime.md`](standalone-runtime.md)。

## Hub 配置

Hub 模式需要：

```json
{
  "hub": {
    "url": "https://agentic-gpt.example.com",
    "transport": "websocket",
    "agentSecret": "<agent-secret>"
  },
  "agentId": "laptop"
}
```

`hub.transport` 可为 `websocket` 或 `sse`。旧的顶层 `hubUrl`、`hubTransport`、`workerUrl`、`agentSecret` 仅由显式 `config import` 识别；普通 v2 load 会拒绝它们。

Standalone 和 Local 可在 Connection 页启用辅助连接，或用 CLI 设置 `hub.enabled`：

```bash
agentic-gpt config init --non-interactive --mode local --hub-enabled true \
  --hub-url https://agentic-gpt.example.com --hub-transport websocket \
  --agent-id laptop --agent-secret '<与 Hub 匹配的 agent secret>'
```

`hub.enabled` 默认 `false`，不会自动建立外部连接。辅助连接只支持确认响应；Hub 仍会收到 Hello
和心跳。Standalone 的 `tunnel.hubReporting.enabled` 是独立开关：开启上报也会保持辅助连接，
但关闭上报不会关闭由 `hub.enabled` 启用的确认连接。Local 不提供 Hub reporting。Hub 模式
保留命令接收语义，不受该辅助连接开关影响。配置 `ntfy` 确认通道时，Standalone/Local 必须启用
辅助连接并提供有效凭据；默认未选择 `ntfy`，旧配置中已有的 `ntfy` 设置仍兼容。

Hub credential 与 Standalone tunnel API key 是两套独立凭据，不要复用。

## 确认机制

规范形式：

```json
{
  "confirmationProvider": {
    "channels": ["freedesktop"]
  },
  "confirmationLanguage": "zh-CN"
}
```

通道：

- `freedesktop`：本地桌面通知按钮。
- `ntfy`：Hub-backed 远程 relay。

`freedesktop` 能力检测按以下方式进行：

- 先向 session D-Bus 查询当前通知服务的 owner，不自动启动桌面通知服务。
- 有 owner 时才向该连接查询 `GetCapabilities`。
- 每次 D-Bus 方法调用最多等待 2 秒。
- 服务缺失或能力查询失败时，该通道视为不可用，可继续尝试后续配置通道。

此检测超时与等待用户点击确认的超时不同。

未启用 Hub 模式、Hub 辅助连接或旧的 Hub reporting 连接时，使用 `freedesktop`。新建且未启用 Hub 连接的配置默认选择 `freedesktop`；启用 Hub 连接后才可选择 `ntfy`，不要求同时启用上报。所有配置通道均不可用时，需要确认的操作会 fail closed。本地拒绝或超时不会继续回退到其他通道。

CLI 仍接受 `freedesktop-then-ntfy` 等 legacy label；Agentic 管理写入会序列化为规范有序数组。

## 命令与路径策略

`policy` 是自动命令判定、路径准入与 bubblewrap 挂载的唯一配置来源：

```json
{
  "policy": {
    "enabled": true,
    "bubblewrap": false,
    "command": {
      "defaultAction": "confirm",
      "allow": ["printf **"],
      "confirm": ["python -c **"],
      "deny": ["ssh **"]
    },
    "path": {
      "defaultAction": "deny",
      "readOnly": ["/var/log"],
      "write": ["/path/to/workspace"],
      "deny": ["/path/to/workspace/private"]
    }
  }
}
```

`policy.enabled=false` 会关闭自动命令与路径策略，并使 bubblewrap 保持关闭；请求显式提供的
`acquireConfirmation` 仍按请求执行。命令策略始终按
`deny > confirm > allow > defaultAction` 判定；命令缺省动作是 `confirm`。

路径只接受具体目录或文件根，不接受 `*`、`**` 等通配模式；加载时会展开 home 并规范化路径。
`defaultAction` 只接受 `allow` 或 `deny`，缺省为 `deny`。`write` 根同时授予读取，`readOnly`
只授予读取，`deny` 在已挂载父目录下遮蔽对应子树。`workspaceRoot` 仅指定默认工作目录，是否可访问
仍由 `policy.path` 决定。`config init` 会写入可编辑的命令预设和具体路径根；加载和重载不会补回
已删除的预设或追加另一套隐藏规则。

命令模式按 argv 逐项匹配：`*` 匹配一个参数内的任意字符，最后一个独立的 `**` 匹配零个或多个后续参数。
参数中的字面 `*` 应使用模式引号或反斜杠转义。单独的模式 `*` 是全局规则，也作用于无法静态解析完整的脚本；这是显式授权范围，添加前须审查。
```bash
agentic-gpt config set policy.enabled true
agentic-gpt config set policy.command.defaultAction confirm
agentic-gpt config allow add 'printf **'
agentic-gpt config confirm add 'python -c **'
agentic-gpt config deny add 'ssh **'
agentic-gpt config path list
agentic-gpt config path default-action deny
agentic-gpt config path readonly add /var/log
agentic-gpt config path write add /path/to/workspace
agentic-gpt config path deny add /path/to/workspace/private
agentic-gpt config path readonly remove /var/log
```

`config set` 也支持 `policy.command.defaultAction/allow/confirm/deny` 与
`policy.path.defaultAction/readOnly/write/deny`。数组值使用 JSON 字符串数组；命令模式作为一个
CLI 参数传入，路径列表只接受具体根。旧的 `pathPolicy`、`sandbox` 和扁平命令 `policy` 形状
不属于当前配置；运行时不会自动转换旧格式。

### 单次主动确认

Process、文件及 tmux 请求使用可选 `acquireConfirmation` 对象；传入对象即主动请求确认，不传时按策略判定。对象的 `reason` 必填，原文不超过 100 个 Unicode 字符，去除首尾空白后必须非空；用自由的请求语气说明本次操作的目的和影响。可选 `method` 仅接受 `ntfy` 或 `freedesktop`，省略沿用配置。

主动确认的结果如下：

- `allow_once` 可覆盖本次操作的命令／路径拒绝，不修改持久策略。
- 明确拒绝或取消会中止操作。
- 超时、过期或提供方不可用时，回退配置策略：Allow 可继续，Confirm 或 Deny 不执行；Confirm 复用本次失败结果，不重复提示，未获准的临时路径范围不会生效。

仅 `process.exec` 和 `process.batch` 接受可选 `read`／`write` 路径数组。这些路径是本次命令／批次的临时权限与沙箱挂载范围，获准后生效，结束后失效；文件和 tmux 不接受 read/write 字段（包括空数组）。文件读取／搜索批次只在顶层接受一个 `acquireConfirmation`；主动获准的搜索覆盖请求根目录子树，仍受扫描和输出限制。
普通搜索会跳过未获授权的拒绝路径；include/exclude、hidden、gitignore 条件同样用于确认预扫描和实际搜索。主动批准不跳过工具集准入、资源容量和文件完整性等独立检查。

`tmux.pasteText` 的基础决策为 Confirm：即使不传确认对象，也会自动请求确认，未获准不会写入窗格。它不再使用默认主动确认或专用严格开关；标题为 `<设备显示名> 请求写入 tmux`，自动正文给出简短原因和输入文本预览。

普通运行、文件和 tmux 确认在 ntfy 与 freedesktop 共用简短标题和正文。主动确认正文只显示调用方提供的 `reason`；自动确认命中 confirm 规则时只显示有界的规则内容；默认策略或无法静态分析时，显示固定原因和原始内容预览。预览合并空白后最多 30 个 Unicode 字符，超长时显示前 29 个字符加“…”；执行链继续持有完整请求和实际授权范围，通知不承担完整请求展示。原有操作预览超过 1000 字符即将渠道视为不可用的限制已移除。下游 MCP 的 server/tool 专用审批、临时 server 授权按钮及 wire 决策负载保持原有语义。

#### 调用方升级

这是不兼容的请求契约变更，Agent 和 Hub 必须同步升级。删除 `acquireConfirmation.acquire`，不提供旧字段兼容层；需要主动确认时传入含 `reason` 的对象，不需要主动确认时删除整个对象。旧 `acquire=false` 应改为省略对象，旧 `acquire=true` 应改为填写原因；空对象不再有效。Process 原有 `read`／`write` 临时范围保留，无需额外开关。

例如，Process 主动请求可使用以下对象；这只是请求中的确认字段：

```json
{
  "acquireConfirmation": {
    "reason": "请允许运行项目构建并写入构建产物。",
    "method": "ntfy",
    "write": ["/path/to/workspace/target"]
  }
}
```

### Bash 命令与 Shell 初始化文件

Process 命令执行环境如下：

- 普通 Bash 以 `--noprofile --norc` 非登录、非交互方式执行原始脚本。
- 启用 `pipefail`，初始化完成后重置该选项，并保持 `set +e`，不会自动启用 `set -e`。
- 每次调用相互独立，不提供 PTY、stdin 或持久 Shell session。

命令策略只分析提交的脚本，不分析初始化文件：

- 识别 tree-sitter-bash 可解析的字面量命令调用及 LF 换行、`&&`、`||`、`;`、`|` 组合，
  包括命令末尾和注释后的换行。
- 字面词片段之间的反斜杠加 LF 按 Bash 续行语义拼接后匹配策略。
- 双引号里的 `$:`、`$/` 等不会引入展开的内容仍是字面值；引号内的反斜杠加 CRLF 也不会
  被误当作命令续行。

未主动取得覆盖授权时，命中 deny 会在执行任何前缀前拒绝整段脚本。
可识别的字面命令按 argv 模式判定；语法不完整、动态展开、重定向或不支持的结构通常要求整段确认，
仅显式全局 `*` 规则可对这些内容直接应用所配置动作。缺少可用确认通道时 fail closed。
反斜杠加 CRLF 不视为 Bash 续行。不要把 `bash -c **` 加入广泛 allow：这会授权任意 shell 代码。

`shell.initFile` 有三种配置状态：

- 省略或执行 `agentic-gpt config unset shell.initFile`：Default，source `~/.agentic_gpt/.bashrc`。
- `null`：Disabled，不加载初始化文件。
- 字符串路径：显式 Path；相对路径以请求 cwd（未提供时为默认 cwd）解析，不按 `PATH` 查找。

初始化文件加载遵循以下规则：

- Default 文件在 sandbox 可见 namespace 中打开；仅当打开失败为 `ENOENT`（包括 dangling
  symlink）时跳过。
- 显式路径的 `ENOENT` 会失败；两种模式的其他打开、读取或 source 错误均以
  `shell_init_file_failed` 阻断命令。
- 不会自动创建文件、加载用户 `~/.bashrc`，也不会隐式加载 `BASH_ENV`/`ENV`。
- 允许 symlink，不做专门的 lstat/readlink 验证。
- Source 使用现有 sandbox 可见性与挂载，不会扩大 namespace。
- Source 后工作目录恢复为请求 cwd；初始化脚本对 PATH、函数、alias（正常 Bash 中 alias
  扩展需显式启用）与导出变量的修改按普通 Bash 规则生效。

初始化文件是本地可信代码，不受命令白名单审计；其 PATH、函数、alias、hook 等影响，以及后续进程
或逃离同一进程组的子进程不由命令策略绑定到原始字面量检查对象。相对路径若由模型选择的 cwd
或模型可写位置解析，存在信任风险；建议使用由用户拥有且受保护的绝对路径。准入时冻结的是配置
设置，不是文件字节或可执行文件身份；运行中配置热加载不会更改已准入 Process 的设置快照。

## 资源限制

```json
{
  "limits": {
    "maxConcurrentTasks": 2,
    "maxActiveProcesses": "auto",
    "maxFileSearchContextLines": 5,
    "processResponseBytes": 8192
  }
}
```

`maxConcurrentTasks` 限制单次 `process.batch` 中同时实际运行的子 Process Job 数量。所有子 Job
仍会整批 admission；超过并发槽的子 Job 保持 `queued`，因此该限制不会阻止 batch 在有界
`waitSeconds` 后返回。配置小于 1 时，有效下限为 1。

`processResponseBytes` 是 `process.exec`、`process.batch`、`skills.run`、`mcp.callTool`、
`mcp.batch` 以及 `process.read` 默认使用的序列化 UTF-8 JSON 响应预算：

- 默认 8192 字节，范围为 4096..1048576；`process.read` 可在此范围内用 `maxBytes` 显式覆盖。
- `mcp.batch` 的整个 `McpBatchToolResponse` 共用一份预算，不按子项重复分配。
- 预算包括响应 JSON 转义和 Base64，不包括传输与 event 封套。
- 该预算与 512 KiB 的 MCP 结果保留上限分离。TUI 使用相同的默认预算。

`maxFileSearchContextLines` 是 `file.search` 对每个匹配返回的前后文行数 live 上限，默认 5，
接受 0–100 的整数。请求可以超过该值；运行时会裁剪到 effective 值，并返回
`requestedContextLines`、`effectiveContextLines`、`contextLinesClipped` 与一个有界 warning。
负数或非整数请求仍会被拒绝。

`maxActiveProcesses` 接受非负整数或 `"auto"`。Auto 基于 `availableParallelism` 计算
`ceil(availableParallelism * 1.5)`；无法获取并行度时使用 6，并将结果限制在 6–24。
Process、Skill 与 MCP Job 共用该容量，排队中的 batch 子 Job 也计入该容量。

v0.9 会拒绝 `maxActiveSessions` 与 `sessionIdleTimeoutSecs`。

## 下游 MCP server

```json
{
  "mcpServers": {
    "docs": {
      "enabled": false,
      "transport": "streamable-http",
      "url": "https://mcp.example.com/mcp",
      "auth": {
        "type": "bearer",
        "token": "replace-me"
      }
    },
    "local-tool": {
      "enabled": false,
      "transport": "stdio",
      "url": "node /home/me/mcp/server.mjs"
    }
  }
}
```

下游 server 配置遵循以下规则：

- Server id 最长 64 字节，只使用字母、数字、`.`、`_`、`-`。
- `streamable-http` 需要绝对 HTTP(S) URL，并可配置
  `auth: {"type":"bearer","token":"..."}`；运行时会发送 `Authorization: Bearer <token>`。
- `stdio` 不接受 Bearer auth，且需要非空命令。
- TUI 会掩码 Bearer token，并在最终 JSON 预览中脱敏。
- 在审查信任与确认策略之前，示例应保持 disabled。

## Skills、Room 与 bubblewrap

`skills` 控制 package 大小、redirect、timeout、重试/总 deadline、安装/下载并发，以及可选 host allowlist。规范字段是顶层 `skills`；旧 `room.skills` 不属于当前可写配置路径。

`room.timezone` 保留为 Room metadata；当前读取使用仓库路径，而不是 legacy JSONL
日期分区。`room.diaryDayBoundaryHour` 范围为 0–23，用于新 bootstrap 的 Daily scaffold
逻辑日期。`room.repositoryRoot` 可选，默认是 `<workspaceRoot>/room`。嵌套的
`room.maintenance.mode` 可为 `local` 或 `workflow`，默认 `local`；
`room.maintenance.autoPush` 默认是 `false`。

Room namespace 的九个文档／维护语义操作是 Diary active/read、Notebook recent/search/read、
State list/read，以及 maintenance status/submit。Notebook recent/search 的 `limit`
默认 20、范围为 1–100；search query 必须非空且不超过 256 个 Unicode 字符；
Markdown read 限制为 512 KiB。Maintenance submit 接受 1–5 个不重复的 semantic slot，可选
mode 覆盖；`waitSeconds` 默认 0、上限 30。workflow 等待超时只结束等待，不会取消
submission；没有隐式 confirmation 或单独的 maintenance wait 操作。

Hub Full 通过 active Room lease 与 `POST /v1/room/<namespace>/<action>` 暴露同一
九项操作，请求不接受 `agentId`。Hub 不拥有 Room repository，也不创建 content
replica。旧 JSONL append/update/remove 与 passage/date-selection 调用不会静默映射
为 maintenance；请迁移到显式 slot/payload request，或删除旧调用。历史 release/migration
记录只保留历史，不是当前 compatibility contract。

`policy.bubblewrap` 默认 `false`；设为 `true` 时，受管理 Process 的挂载直接由 `policy.path` 派生，不存在另一套可配置
sandbox 列表。执行层提供的只读运行依赖挂载为 `/usr`、`/bin`、`/lib`、`/lib64`、`/etc/ssl`，并使用
bubblewrap 内的 `PATH`。`defaultAction=deny` 时只挂载 `readOnly` 与 `write` 的具体根；
`defaultAction=allow` 时以宿主 `/` 的读写挂载为基线，再将 `readOnly` 根收紧为只读并遮蔽 `deny` 根。
`write` 根包含读取权限，并优先于重叠的 `readOnly` 根；`deny` 优先于两者。位于已挂载父目录下的现存 `deny` 根会被遮蔽。
若拒绝路径尚不存在且会被父目录挂载暴露，操作会因无法安全建立遮蔽而失败；不会在宿主机创建占位路径。

Process 主动确认获准的 `read`／`write` 路径可追加到本次挂载，执行结束后失效。bubblewrap
`bwrap` 可执行文件从 Agent 的 `PATH` 查找；启动或挂载失败时操作失败，不回退到未隔离执行。`policy.bubblewrap=false`、或
`policy.enabled=false` 时不启用 OS 挂载隔离；显式主动确认仍生效。tmux、下游 MCP 和 Browser
JavaScript 不属于受管理 Process 的 bubblewrap 范围。

## Room 密钥配置

`room.secret` 通过 Agent 本机的 Bitwarden CLI 访问独立 Vaultwarden 账号，默认关闭。它要求启用 `room` 工具集并设置 `room.secret.enabled=true`；当前生效配置关闭任一开关都会拒绝后续密钥调用，不能用已知工具名绕过。设置、调用示例、同步与敏感内容边界见[Room 密钥使用指南](room-secret.zh-CN.md)。

| 配置键 | 默认值 | 约束 |
| --- | --- | --- |
| `room.secret.enabled` | `false` | 启用时须填写完整连接配置。 |
| `room.secret.serverUrl` | 空 | HTTP(S) Vaultwarden 地址，不含 userinfo、query 或 fragment；最多 4096 字节。 |
| `room.secret.account` | 空 | 专用 Vaultwarden 账号；不含空白或控制字符，最多 1024 字节。 |
| `room.secret.clientId`、`clientSecret`、`masterPassword` | 空 | 仅接受 `env:NAME` 或 `file:PATH` 引用；非空引用最多 4096 字节，不接受明文或控制字符。 |
| `room.secret.getPolicy` | `allow` | 仅允许 `allow` 或 `confirm`。 |
| `room.secret.upsertPolicy`、`deletePolicy` | `confirm` | 仅允许 `allow` 或 `confirm`。 |
| `room.secret.syncWaitTimeoutMs` | `3000` 毫秒 | 0–30000；仅限制 list/get 等待同步的预算，0 表示不等待，不取消同步。 |

CLI 支持上述全部键，TUI 的 Room 页和 Review 提供对应编辑入口。`config show`、JSON 预览、Review 和 Debug 隐藏服务地址、账号及三个凭据引用；`agent.info` 的配置摘要不包含这些内容。引用在使用时按现有 secret reference 机制解析，文件引用支持 `~` 展开。

`getPolicy=confirm`、`upsertPolicy=confirm` 或 `deletePolicy=confirm` 均要求明确批准；拒绝、超时或确认不可用时不披露或修改密钥，不受 `policy.enabled=false` 绕过。`list` 不请求确认。`room.secret.getPolicy`、`upsertPolicy`、`deletePolicy` 与 `syncWaitTimeoutMs` 支持热更新，配置重载后用于新请求；已接纳请求保留原策略和等待预算，正在运行的同步不受影响。后端、账户、凭据及启停变更通过局部 owner 协调：先排空操作，再释放旧 session/账户锁，之后由新配置激活。准备繁忙或失败会报告错误；已保存不等于后端已连接，重配置不重放 Secret 写入。

## Room 邮件配置

邮件配置使用顶层 `email`，默认关闭；启用时需配置完整 `account`，并启用 `room` 工具集。全屏初始化提供邮件页与 Review；CLI 提供 `email.enabled`、`email.account`、`email.syncIntervalSeconds`、`email.retentionDays`、`email.maxStorageBytes`、`email.sendPolicy` 和 `email.trashPolicy`。账户通过完整 JSON 对象设置；密码仅接受 `env:` 或 `file:` 引用，配置显示与 Review 均脱敏。

同步间隔默认 60 秒，范围 10–86400 秒；本地内容默认保存 90 天，范围 1–36500 天；容量上限默认 2 GiB，最小 1 MiB。`sendPolicy` 和 `trashPolicy` 都默认 `confirm`，可设为 `allow` 或 `deny`；`confirm` 必须明确批准，确认不可用时不会执行，也不受全局 `policy.enabled=false` 放宽。

`email.account` 修改通过邮件服务排空在途操作、释放旧账户 owner 后局部切换，保留本地数据库与发送账本；繁忙时保留旧账户并报告应用失败，不自动重发邮件。其余邮件配置应用于后续同步和请求。启停邮件及关闭 Room 工具集会影响后台同步。邮件读取有 `text`（默认）、`image`（离线）和 `imageWithRemote`（仅额外下载远程图片）三种模式；图片渲染使用随包构建的 `litehtml-rs`，无需 Chrome、Python 或项目 Browser，运行时需要合适字体。完整账户示例、六个 API、私有存储与清理边界见[Room 邮件使用指南](email.zh-CN.md)。

## CLI 与 TUI 配置管理

本地管理不要求启用 `config` 工具集，也不经过 `config.updatePolicy` 的 API 确认。修改进入共享的路径目录、原生值更新和完整候选校验；成功写入后由运行实例监听并协调应用。CLI 输出的持久值或 `saved=true` 不构成运行态应用完成的证明，实际状态用运行实例的 `config.get` 检查。

```bash
agentic-gpt config edit
agentic-gpt config manual
agentic-gpt config manual --section browser --detail full
agentic-gpt config manual --query updatePolicy --json
agentic-gpt config get limits.processResponseBytes
agentic-gpt config get '/mcpServers/example.http/enabled'
agentic-gpt config set shell.initFile null
agentic-gpt config unset shell.initFile
agentic-gpt config add skills.allowedHosts '"skills.example.com"'
agentic-gpt config remove skills.allowedHosts '"skills.example.com"'
agentic-gpt config update '[{"op":"set","path":["limits","processResponseBytes"],"value":16384}]' --dry-run
```

`config get` 不指定路径时读取整份有效持久配置，结果脱敏；与 API 的 `items/appliedValue/applyStatus` 响应不同。`config show` 保留完整有效配置展示。`manual` 使用三端共用的完整目录；`config keys [--section <SECTION>] [--json]` 提供同一目录的字段浏览。章节包含 `runtime/identity/workspace/toolsets/config/hub/http-mcp/confirmation/policy/shell/limits/skills/room/email/tunnel/browser/mcp/events`；`manual` 另外保留完整章节上下文和对象树。

`get/set/unset/add/remove` 的路径可使用已注册点式键、JSON Pointer 或 JSON 字符串段数组。含点的动态 map key 推荐使用后两种形式，例如 `/mcpServers/example.http/enabled` 或 `'["mcpServers","example.http","enabled"]'`；不能将它拆成两个服务器 ID。`config update` 的操作数组与 API 的 `changes` 相同，支持原生对象、布尔值、数字、列表与 `null`。`add/remove` 的值必须是合法 JSON，包括字符串的双引号。

`set` 保留本地文本便利：字符串和枚举可直接传文本，布尔值、数字、对象和列表按类型解析；`config set events.internalOverrides.process.completed null` 映射为 `unset`。API 中请明确使用 `{"op":"unset",...}`，不可依赖该映射。`unset` 可移除目录支持的可选字段和 map 条目，必需字段仍受保护。

`config edit` 打开现有配置的全屏表单，不重新初始化。原有 Connection、Workspace、Room、MCP 等页面保留；附加配置页提供 Browser、Skills、命令策略、`backupLimit` 和 `config.updatePolicy` 的字段编辑。提交仅将实际修改的字段操作作用于最新持久配置，不回写整份旧快照；集合增删保留 `add/remove`，有序列表或重排使用 `set`，MCP map 按稳定服务器 ID 定位。最终校验失败保留草稿，凭据文件创建仍沿用原事务流程。

专用的 `config allow/confirm/deny`、`config path`、`config mcp` 和 `config toolset` 管理命令继续保留；不必为同一设置先删除旧配置再写新配置。

## 密钥文件与事务写入

Tunnel secret 必须写成 `file:PATH` 或 `env:NAME` 引用；`file:` 路径可以是绝对路径或使用
常规 home 展开，环境变量名必须是合法 shell 变量名。全屏配置在最终确认时选择写入文件，会以 `0700`
创建父目录、以 `0600` 创建密钥文件，先写临时文件再原子重命名。如果之后的配置写入失败，
会删除新建的密钥，或恢复原密钥的字节内容与权限。Escape、Ctrl-C、提示错误或最终拒绝都
发生在事务提交之前，因此不会创建或修改配置文件或密钥文件。summary、诊断与错误不会输出
密钥值。

## 显式 import 迁移

普通 `Config::load()` 严格要求 v2，不会推断缺失的 selector，也不会静默接受旧的 Hub 形状。

请使用 `agentic-gpt config import --config PATH [SOURCE]` 迁移旧版或外部 JSON：

- `--config` 可省略，此时使用默认配置路径。
- 省略 SOURCE 时会导入所选 `--config` 路径。
- 该流程进入普通交互式 Config Init TUI。
- 没有编辑器的已识别当前字段会保留，包括 MCP server、统一 policy、limits、非活动
  hub/tunnel/room 数据以及安全的未知扁平字段；旧 policy/path/sandbox 形状不会在这里自动转换。
- 无法导入的字段会明确报告，最终通过标准备份/密钥事务写入。

## 热加载与重启边界

Standalone、Local 和连接 Hub 的 Agent 共用串行配置应用协调。目标配置先做静态校验；保存之后，直接更新设置或协调各组件。静态校验不连接邮箱、Vaultwarden、Hub 或浏览器，也不能保证端口、凭据和外部依赖可用。组件应用失败不回滚整份持久配置：能保留旧 owner 则继续使用，否则报告该组件不可用；不保证跨组件原子切换。

| 配置 | 当前实例如何应用 |
| --- | --- |
| `mode`、`profile`、`agentId`、`workspaceRoot` | 保留当前启动值，保存的目标值需显式重启实例；不自动迁移工作空间或重启用户进程。 |
| `tunnel.tunnelId`、`tunnel.apiKey`、`tunnel.client.*` | 保留 supervisor 捕获的身份、凭据和客户端启动参数；下次显式启动采用新值。 |
| `tunnel.hubReporting.detail` | 更新后续报告策略，不重建 Tunnel。 |
| `tunnel.hubReporting.enabled`、`hub.*` | 由 Hub 连接所有者局部协调，可能处于连接中或失败；旧目标的结果不会作为新目标的调用结果重放。 |
| `httpMcp.*` | Standalone listener/auth 所有者协调监听、认证与 session；endpoint identity 改变后客户端需重新 initialize。Local/Hub 模式不因此新增 TCP listener。 |
| `browser.*`、Browser 工具集启停 | 协调发现、排空旧租约并替换 runtime；旧租约、JS 变量和会话可能失效，不重放网页动作。 |
| `email.account` | 排空在途邮件请求与同步，释放旧账户 owner 后切换；保留 Store、同步记录与发送结果，不自动重发。 |
| `room.secret` 后端／启停／凭据 | 排空请求、停止旧 CLI/sync、释放 session 和账户锁，再允许新 owner 激活；保留加密缓存及使用记录，不重放写入。 |
| `room.secret.getPolicy/upsertPolicy/deletePolicy/syncWaitTimeoutMs`、其余邮件设置 | 后续请求／同步使用新值；已准入操作保留各自策略和等待预算。 |
| `room.repositoryRoot` | 在仓库写入边界准备并切换根目录，不自动迁移已有内容。 |
| `skills.maxConcurrentInstalls` | 保留安装 manager 与 worker，更新后续准入上限；降低时等待现有任务自然释放，不取消安装。 |
| `policy`、`limits`、`shell`、`mcpServers`、`toolsets`、`events`、确认设置、其余 Skills/Room 设置、`config.updatePolicy` | 更新后续消费者；Process、Skill 和已创建下游调用保留各自准入快照，不因配置变化重新执行。 |

Room `timezone` 当前仅保留配置，现有运行逻辑未使用；`diaryDayBoundaryHour` 只影响新空仓库的初始化，不改写已有仓库。关闭工具集不等于撤销已准入副作用；Tmux 已有会话不因工具集关闭而删除。

HTTP token 无法解析时拒绝认证，监听绑定失败由组件重试，不影响其他入口。更换 endpoint 会丢弃其本地 OAuth/session 状态；token 原地更新保留 session 并撤销 OAuth 记录。邮件、Secret 或 Browser 准备繁忙时可报告应用失败并保留原实例；需要重新应用的配置通过配置操作再次提交，连接恢复沿各组件现有机制处理，不重放用户副作用。

待重启的模式、身份或工作空间值不会阻止无关设置应用。同一 Standalone supervisor 的故障恢复从私有启动快照保留原启动契约，同时读取最新磁盘的可热配置字段；只有显式重启整个实例才采用新的启动值。快照位于本次 runtime 目录，权限为 0600，supervisor 退出时清理，不修改用户目标配置。

通过 `config.get` 的 `value/appliedValue/applyStatus/error` 检查每项保存与生效状态；`agent.info.config.liveSubsetMatchesDisk` 和 `restartRequiredFields` 是概要诊断，不证明外部资源已就绪，也不证明运行中的旧进程已切换设置。

## 验证与检查

```bash
agentic-gpt config show
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

`agent.info` 只暴露安全摘要，不暴露 tunnel secret、Hub secret、完整私有路径或 MCP endpoint。工作区审计文件为：

```text
<workspaceRoot>/.agentic-gpt-audit.jsonl
```

Standalone 生命周期与恢复语义见 [`standalone-runtime.md`](standalone-runtime.md)，部署检查见 [`operations.md`](operations.md)。
