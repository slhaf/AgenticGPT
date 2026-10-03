# 配置说明

Agentic GPT 的 Standalone、Local Unix MCP 与连接 Hub 的 Agent 共用一份本地 JSON 配置。默认路径：

```text
~/.agentic_gpt/config.json
```

对应页面：[配置说明](configuration.zh-CN.md)。

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

模式与配置档是两个独立选择：

- `--mode standalone|hub|local` 选择运行时连接方式与配置形状。
- `--profile normal|room` 选择默认 toolset preset。normal preset 启用除 `room` 外的所有
  namespace；room preset 启用所有 namespace。配置档本身不会固定最终 runtime surface 或数量，
  因为显式的 `toolsets.enabled` 选择具有权威性。

可用 namespace 为 `agent`、`file`、`mcp`、`process`、`skills`、`tmux`、`browser`、`room`。
逻辑上的 `room` namespace 包含 Room bootstrap（`bootstrap` 与 `bootstrap.read`）以及全部
`room.*` 工具。选择只会过滤既有的 Normal/Room advertised names，不会暴露 dispatch-only alias。
使用以下精确命令固定选择：

```bash
agentic-gpt config toolset ls
agentic-gpt config toolset enable <namespace>
agentic-gpt config toolset disable <namespace>
```
`ls` 会列出全部 namespace、当前启用/禁用状态及其所含工具的简短说明。`enable` 和
`disable` 成功后会明确输出被修改的 namespace 与结果状态。

同一个 `toolsets.enabled` 数组也可以直接编辑 JSON。有效的 toolset 修改会在 worker 运行时
热加载，并对后续工具发现与调用生效；无需重启。无效候选会保留上一次有效的 live selection。
Room bootstrap、日记和笔记本命令以实时 `room` namespace 为授权依据，而不是启动时的
profile。该 namespace 禁用时，直接分发 Room 命令会返回 `room_toolset_required`。

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

脚本需要确定性结果时，请使用以下实际 CLI 语法，并提供不应保留占位符的值：

```bash
agentic-gpt config init --non-interactive
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt config init \
  --mode standalone \
  --profile room \
  --tunnel-id tunnel_<assigned-id> \
  --tunnel-api-key file:"$HOME/.agentic_gpt/secrets/tunnel-api-key" \
  --non-interactive
```

不提供值时，非交互式 Standalone + Normal 模板使用安全占位符，例如 `tunnel_replace-me`
以及 Agentic home 下的 `file:` 引用。命令会报告替换 tunnel ID、配置所引用密钥等待处理
操作；不会自动创建或配置密钥材料。Hub 缺少值时同样会报告待配置的 Hub URL 与代理密钥。
`--agent-secret` 会暴露在 shell 历史和本地进程检查中，因此优先使用交互式隐藏输入。
使用 `file:` 或 `env:` 引用可以避免把 tunnel secret 放进命令行；明文 tunnel API key
会被拒绝。

全屏流程为 Basic → Connection（Local 除外）→ Optional settings → Review → Completion。
交互模式下的命令行 flag 只是可编辑的预填值，不会锁定字段或跳过页面。身份/显示名称、
工作区/路径策略、确认方式/语言、限制、沙箱、Toolsets、Events 与下游 MCP server 集合均可在
Optional settings 中配置。Toolsets 从配置档 preset 开始；一旦显式编辑，其 namespace
selection 具有权威性。Room 设置仅当当前有效的 toolset selection 启用 `room` namespace 时
才会提供。profile 通过默认 preset 影响该 selection；显式 `toolsets.enabled` 会覆盖 preset，
因此显示条件是有效 namespace selection，而不是 `profile` 字段本身。只有 Standalone 模式会出现
tunnel-client 覆盖和 Hub reporting。Hub 与 Local 模式不会显示这些 tunnel 部分。不选可选部分时会保留模板默认值。

界面使用键盘导航：Tab/Shift+Tab 与方向键移动焦点，Enter 编辑或触发当前操作，Esc 返回
（根 Basic 页面是 no-op），Ctrl+C 取消初始化。编辑态按 Esc 只结束编辑，不会取消初始化。
Review 会隐藏密钥，可跳回 Basic、Connection 或可选 section 编辑；最终确认前不会写入配置、
备份或密钥文件。本功能只承诺键盘全屏流程；鼠标、inline、dashboard 与 Windows 行为不在
本功能契约内。Events 设置可选择 `low`、`medium`、`high` 或 `off`，并在 Review 中复核保存；
这只是配置界面，不提供运行时事件收件箱界面。

`config init --language auto|zh-CN|en` 选择 CLI 界面语言。使用 `auto` 时依次检查
`LC_ALL`、`LC_MESSAGES`、`LANG`，都没有匹配时使用英文界面。显式的 `zh-CN` 或 `en`
优先于环境变量。这个界面选择与持久化的 `confirmationLanguage` 不同；后者控制 runtime
发出的确认提示语言，可在可选配置 section 或通过 `config set` 设置。

全屏初始化的 Optional settings 包含下游 MCP server 集合编辑器；`Shell` section 可选择 Shell
初始化文件为 Default、Disabled 或 Path，并仅在 Path 时编辑路径；三种选择都会写入并可在 Review
复核。`config init --non-interactive` 没有用于填写 `mcpServers` 集合的 CLI flags。自动化初始化后可使用 `config mcp` 配置 server；
命令策略集合仍使用 `config allow`、`config confirm`、`config deny`，路径根使用 `config path`。

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

agentic-gpt config set tunnel.tunnelId tunnel_<assigned-id>
agentic-gpt config set tunnel.apiKey file:"$HOME/.agentic_gpt/secrets/tunnel-api-key"
agentic-gpt config set tunnel.client.autoDownload true
agentic-gpt run
```

将 `profile` 设为 `room` 会选择启用所有 namespace 的 Room preset（例如
`agentic-gpt config set profile room`）；如果存在 `toolsets.enabled`，则以它为准。

## 顶层字段

| 字段 | 用途 |
| --- | --- |
| `mode` | 权威运行时分派：`standalone`、`hub` 或 `local`。 |
| `profile` | 默认能力/toolset preset：`normal` 或 `room`。 |
| `toolsets` | 已启用的 tool namespace；显式 `enabled` 列表会覆盖配置档 preset。 |
| `agentId` | 稳定本地 identity，也用于派生私有 runtime/socket 路径，以及 `~/.agentic_gpt/state/agent/<agentId>/` 下的 per-agent 持久状态根目录。 |
| `displayName` | summary/reporting 中的人类可读机器名称。 |
| `workspaceRoot` | 主可写工作区，也是 `.agentic-gpt-audit.jsonl` 所在位置。 |
| `backupLimit` | Agentic 管理配置写入时保留的备份数量。 |
| `confirmationProvider` | 有序的本地/远程确认通道。 |
| `confirmationLanguage` | `en` 或 `zh-CN`。 |
| `sandbox` | 可选 bubblewrap 配置。 |
| `shell` | Bash 命令执行的 Shell 初始化文件设置。 |
| `mcpServers` | `mcp.*` 转发的下游 MCP server。 |
| `pathPolicy` | 可写、只读、拒绝路径根。 |
| `policy` | 显式 allow / confirm / deny 命令规则。 |
| `limits` | Process 并发限制与最大活动 Process/Skill/MCP Job 容量。 |
| `skills` | Skill package/install 限制与网络策略。 |
| `room` | Room 仓库根目录、时区、日记日界线、维护模式和自动推送策略。 |
| `tunnel` | Standalone tunnel-client 来源、secret 引用与可选 reporting。 |
| `browser` | 可选的高级 Browser runtime 覆盖；普通 runtime discovery/provisioning 默认仍自动进行。 |
| `hub` | 集中式 Hub 连接，或 Standalone 的可选 Hub reporting/ntfy relay。 |
| `httpMcp` | 可选的 Standalone hidden worker 所有入站 Streamable HTTP MCP endpoint。 |
| `events` | 持久事件收件箱中 `low` 严重级别事件的保留时长，以及内部事件严重级别覆盖。 |

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

`config set` 的值是单个 shell 参数；枚举值是 JSON 字符串，对象中的 JSON 字符串也要在 shell 中引用。可使用 `config keys` 查看当前 registry、`config show` 查看有效配置：

```bash
agentic-gpt config keys --section events
agentic-gpt config set events.lowTtlSeconds 86400
agentic-gpt config set events.internalOverrides.process.completed '"high"'
agentic-gpt config set events.internalOverrides.skill_install.failed '"off"'
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

`publicUrl` 必须是非空 HTTPS origin，不能包含 userinfo、除空路径或 `/` 之外的
path、query 或 fragment；末尾 `/` 会被规范化。它只是公布的外部 origin，不是
路由或 proxy 覆盖：`host` 与 `port` 仍是本地 bind 坐标，origin 可以保持
loopback/private。`publicUrl` 不是 secret，在 `config show`、Review、诊断和 TUI
中始终显示且不隐藏；`bearerToken` 仍是 secret 引用，解析后的值不会暴露。

connector 只接受以下精确 ChatGPT callback family：
`https://chatgpt.com/connector/oauth/<suffix>` 或精确的
`https://chatgpt.com/connector_platform_oauth_redirect`。只接受 `agentic:mcp`；
不提供 refresh token、`offline_access`、dynamic client registration、generic
registration 或任意 redirect。authorization code 与 access token 是 opaque、
listener-local 的内存记录，有过期、单次 code 消费和 listener 替换/token 内容
轮换时撤销机制。Standalone authorization 页面以及 tool/profile surface 使用
Standalone 语义，不是 Hub 的 `Hub API key`、profile 或 routing contract。

OAuth 路由与 `/mcp` 共用 listener Host 防护。缺失或 malformed Host 会被拒绝，
不允许的 authority 返回 forbidden；不带 port 的 allowlist 项匹配任意 port，
带 port 的项目必须精确匹配。`null` 与严格等于 `["*"]` 仍是明确的全量放行值。
如果存在 `Origin`，必须精确匹配配置的 `publicUrl`；缺失 Origin 的
server-to-server 请求仍然有效，不添加 permissive CORS。未配置 `publicUrl` 时，
直接 bearer 失败仍使用普通 Bearer challenge；配置后，MCP challenge 指向
path-specific protected-resource metadata URL。

rmcp transport 使用有状态的 Streamable HTTP/SSE：客户端必须先初始化 session；
listener rebind 或关闭会终止 session，客户端必须重新 initialize。直接 bearer
内容轮换会原地更新认证并保留既有 session，同时原子撤销 OAuth 记录。无效候选
保留上一次有效 listener 及其 state。

`allowHosts` 是 HTTP MCP transport 使用的 DNS-rebinding 防护：

- 默认列表只允许 `localhost`、`127.0.0.1` 和 `::1`。
- 非空合法 host/authority 列表只允许列表中的请求。
- `null` 或严格等于 `["*"]` 时明确允许任意 Host 值；wildcard 不能与其他项混用。
- 空数组会被拒绝，不会被解释为全量允许。
- malformed authority、wildcard 混用以及其他非法值都会被拒绝。

非 loopback listener 应显式填写 authority allowlist，或有意使用上述两个全量放行值。
`host` 必须是可 bind 的非空值，且不能含空白或控制字符；`port` 必须在 `1..=65535`
范围内。

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

显式 source 在进程启动时选择并具有权威性：无效 descriptor 会关闭 Browser capability，
不会回退到 Desktop discovery，但 Agentic 仍会继续启动。修改它需要重启进程。
`docsRoot` 与 `trustedCodePaths` 由内部派生，不是配置字段。本设置不管理 installer、
downloader 或 runtime cache。

可直接作为路径组件的 `agentId` 会原样映射为私有状态目录名；历史上较宽松的 Hub identity 仍然兼容，但会使用稳定 hash 目录 key，而不会直接成为文件系统路径组件。

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

Standalone 未配置 Hub reporting 时，建议只使用 `freedesktop`。所有配置通道均不可用时，需要确认的操作会 fail closed。本地拒绝或超时不会继续回退到其他通道。

CLI 仍接受 `freedesktop-then-ntfy` 等 legacy label；Agentic 管理写入会序列化为规范有序数组。

## 命令与路径策略

```bash
agentic-gpt config allow add printf
agentic-gpt config confirm add python -c
agentic-gpt config deny add ssh

agentic-gpt config path list
agentic-gpt config path write add ~/Projects
agentic-gpt config path readonly add /var/log
agentic-gpt config path deny add ~/.secrets
```

配置的 allow 规则可以显式覆盖 builtin confirm/deny。多个配置规则匹配时，除非存在按运行时策略生效的更明确 allow override，否则 deny 优先。

`workspaceRoot` 始终视为可写。Denied roots 覆盖 writable/read-only roots。Symlink 会解析，最终目标必须留在有效策略边界内。

### Bash 命令与 Shell 初始化文件

Process 命令由普通 Bash 以 `--noprofile --norc` 非登录、非交互方式执行原始脚本；启用 `pipefail`，
初始化完成后重置该选项，并保持 `set +e`，不会自动启用 `set -e`。每次调用相互独立，不提供 PTY、
stdin 或持久 Shell session。命令策略只分析提交的脚本，不分析初始化文件：只识别 tree-sitter-bash
可解析的字面量命令调用及 `&&`、`||`、`;`、`|` 组合。命中 deny 会在执行任何脚本前拒绝整段命令；
不完整、无法识别或不支持的语法需要对整段脚本确认，缺少可用确认通道时 fail closed。不要把
`bash -c` 加入广泛 allow：这会显式授权任意 shell 代码，不能作为默认放宽策略。

`shell.initFile` 有三种配置状态：

- 省略或执行 `agentic-gpt config unset shell.initFile`：Default，source `~/.agentic_gpt/.bashrc`。
- `null`：Disabled，不加载初始化文件。
- 字符串路径：显式 Path；相对路径以请求 cwd（未提供时为默认 cwd）解析，不按 `PATH` 查找。

Default 文件在 sandbox 可见 namespace 中打开；仅当打开失败为 `ENOENT`（包括 dangling symlink）时跳过。
显式路径的 `ENOENT` 会失败；两种模式的其他打开、读取或 source 错误均以 `shell_init_file_failed`
阻断命令。不会自动创建文件、加载用户 `~/.bashrc`，也不会隐式加载 `BASH_ENV`/`ENV`。允许 symlink；
不做专门的 lstat/readlink 验证。Source 使用现有 sandbox 可见性与挂载，不会扩大 namespace。
Source 后工作目录恢复为请求 cwd；初始化脚本对 PATH、函数、alias
（正常 Bash 中 alias 扩展需显式启用）与导出变量的修改按普通 Bash 规则生效。

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

`processResponseBytes` 是 `process.exec`、`process.batch`、`skills.run`、`mcp.callTool`、`mcp.batch` 以及 `process.read` 默认使用的序列化 UTF-8 JSON 响应预算。默认 8192 字节，范围为 4096..1048576；`process.read` 可在此范围内用 `maxBytes` 显式覆盖。`mcp.batch` 的整个 `McpBatchToolResponse` 共用一份预算，不按子项重复分配。预算包括响应 JSON 转义和 Base64，但不包括传输与 event 封套，并与 512 KiB 的 MCP 结果保留上限分离。TUI 使用相同的默认预算。

`maxConcurrentTasks` 限制单次 `process.batch` 中同时实际运行的子 Process Job 数量。所有子 Job 仍会整批 admission；超过并发槽的子 Job 保持 `queued`，因此该限制不会阻止 batch 在有界 `waitSeconds` 后返回。配置小于 1 时，有效下限为 1。

`maxActiveProcesses` 接受非负整数或 `"auto"`。Auto 基于 `availableParallelism` 计算 `ceil(availableParallelism * 1.5)`；无法获取并行度时使用 6，并将结果限制在 6–24。Process、Skill 与 MCP Job 共用该容量，排队中的 batch 子 Job 也计入该容量。

`maxFileSearchContextLines` 是 `file.search` 对每个匹配返回的前后文行数 live 上限，默认 5，接受 0–100 的整数。请求可以超过该值；运行时会裁剪到 effective 值，并返回 `requestedContextLines`、`effectiveContextLines`、`contextLinesClipped` 与一个有界 warning。负数或非整数请求仍会被拒绝。

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

Server id 最长 64 字节，只使用字母、数字、`.`、`_`、`-`。`streamable-http` 需要绝对 HTTP(S) URL，并可配置 `auth: {"type":"bearer","token":"..."}`；运行时会发送 `Authorization: Bearer <token>`。`stdio` 不接受 Bearer auth，且需要非空命令。TUI 会掩码 Bearer token，并在最终 JSON 预览中脱敏。在审查信任与确认策略之前，示例应保持 disabled。

## Skills、Room 与 sandbox

`skills` 控制 package 大小、redirect、timeout、重试/总 deadline、安装/下载并发，以及可选 host allowlist。规范字段是顶层 `skills`；只有缺少顶层字段时才读取 legacy `room.skills`。

`room.timezone` 保留为 Room metadata；当前读取使用仓库路径，而不是 legacy JSONL
日期分区。`room.diaryDayBoundaryHour` 范围为 0–23，用于新 bootstrap 的 Daily scaffold
逻辑日期。`room.repositoryRoot` 可选，默认是 `<workspaceRoot>/room`。嵌套的
`room.maintenance.mode` 可为 `local` 或 `workflow`，默认 `local`；
`room.maintenance.autoPush` 默认是 `false`。

Room namespace 的九个语义操作是 Diary active/read、Notebook recent/search/read、
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

`sandbox.enabled` 启用 bubblewrap；`requiredRuntimePaths` 定义 sandbox 中可见的宿主路径。Sandbox 不能替代命令策略、路径策略或确认。

## CLI 可管理字段

`config set` 使用受控 registry，并不是通用 JSONPath 编辑器。使用当前语言列出 registry：

```text
agentic-gpt config keys [--section <SECTION>] [--json]
```

文本形式按 `runtime`、`identity`、`hub`、`confirmation`、`sandbox`、`shell`、`limits`、`skills`、`room`、
`tunnel`、`http-mcp` 和 `events` 分组；`--section` 只显示其中一个分组。`--json` 返回机器可读的类型、
是否可为 null、示例、双语说明和别名元数据。`config set` 只接受 registry 中的键；结构化
policy 与 MCP 集合应使用专用命令。

注册键后的值是一个 shell 参数。因此 JSON 列表必须加引号。`room.repositoryRoot` 可为 null，
使用字面量 JSON 值 `null` 可以清除它并恢复 workspace 默认目录。

只有新键 `shell.initFile` 支持 `config unset`；unset 恢复省略状态（Default），不同于 `config set shell.initFile null`
所设置的 Disabled。该键可用 `config set shell.initFile null` 或设置字符串路径；其他 registry 键不支持 unset。

```bash
agentic-gpt config set shell.initFile null
agentic-gpt config set shell.initFile /path/to/protected/bashrc
agentic-gpt config unset shell.initFile
agentic-gpt config set sandbox.requiredRuntimePaths '["/usr","/opt/runtime"]'
agentic-gpt config set skills.allowedHosts '["skills.example.com"]'
agentic-gpt config set room.repositoryRoot null
agentic-gpt config set room.maintenance.mode local
agentic-gpt config set room.maintenance.autoPush false
```

registry 包含以下常用 scalar：

- `mode`、`profile`、`agentId`、`hub.url`、`hub.transport`、`hub.agentSecret`、`workspaceRoot`
- `confirmationProvider.channels`、`confirmationLanguage`、`sandbox.enabled`
- `tunnel.tunnelId`、`tunnel.apiKey`
- 全部 `tunnel.client.*` 与 `tunnel.hubReporting.*`
- `room.repositoryRoot`、`room.timezone`、`room.diaryDayBoundaryHour`
- `shell.initFile`
- `room.maintenance.mode`、`room.maintenance.autoPush`
- 文档列出的 `skills.*` scalar/list 字段
- `httpMcp.enabled`、`httpMcp.host`、`httpMcp.port`、`httpMcp.publicUrl`、`httpMcp.bearerToken`、`httpMcp.allowHosts`
- `events.lowTtlSeconds`、以及每个 `events.internalOverrides.<event-type>` 注册键（例如 `events.internalOverrides.process.completed`、`events.internalOverrides.skill_install.failed`）

结构化策略与 MCP 修改使用 `config allow/confirm/deny`、`config path`、`config mcp`。
上面的 `config toolset` 命令用于管理 namespace 选择。复杂 JSON（包括 `toolsets.enabled`）
也可直接编辑；有效编辑会在不重启 worker 的情况下热加载，无效候选会保留上一次有效状态。
随后可执行 `agentic-gpt config show` 与 smoke test。

## 密钥文件与事务写入

Tunnel secret 必须写成 `file:PATH` 或 `env:NAME` 引用；`file:` 路径可以是绝对路径或使用
常规 home 展开，环境变量名必须是合法 shell 变量名。全屏配置在最终确认时选择写入文件，会以 `0700`
创建父目录、以 `0600` 创建密钥文件，先写临时文件再原子重命名。如果之后的配置写入失败，
会删除新建的密钥，或恢复原密钥的字节内容与权限。Escape、Ctrl-C、提示错误或最终拒绝都
发生在事务提交之前，因此不会创建或修改配置文件或密钥文件。summary、诊断与错误不会输出
密钥值。

## 显式 import 迁移

普通 `Config::load()` 严格要求 v2，不会推断缺失的 selector，也不会静默接受旧的 Hub 形状。
请使用 `agentic-gpt config import --config PATH [SOURCE]` 迁移旧版或外部 JSON（`--config`
可省略，此时使用默认配置路径）；省略 SOURCE 时会导入所选 `--config` 路径。该流程进入普通
交互式 Config Init TUI，保留没有编辑器的已识别字段（包括
MCP server、policy、path policy、limits、非活动 hub/tunnel/room 数据以及安全的未知扁平字段），
明确报告无法导入的字段，并通过标准备份/密钥事务写入。

## 热加载与重启边界

Standalone、Local 以及连接 Hub 的 Agent worker 轮询同一份配置，并原子应用支持的
live subset。无效候选会保留上一份有效状态；候选修改需要重启的资源时，该资源在进程
重启前仍保持原来的 live 值。

| 配置 | 行为 |
| --- | --- |
| `policy`、`limits`、`mcpServers`、`toolsets.enabled` | 所有 Agent worker 共享热加载，对新 admission/call 与工具发现生效 |
| `shell.initFile` | 热加载；之后接纳的 Process 使用新设置，已接纳 Process 保留准入时的配置快照 |
| `pathPolicy`（`workspaceRoot` 未改变时） | 所有 Agent worker 共享热加载，对后续路径检查生效 |
| `httpMcp.enabled`、`host`、`port`、`publicUrl`、`allowHosts` | Standalone HTTP watcher 协调启用状态与 endpoint identity；identity 变化会关闭有状态 session 并丢弃 listener-local OAuth state，客户端必须重新 initialize |
| `httpMcp.bearerToken` 引用或其解析内容 | Standalone HTTP watcher 原地更新认证而不重新绑定；解析凭据可用时保留已有 session |
| 已接纳 Process/Skill Job 与已创建下游调用 | Process 与 Skill Job 从准入开始保留同一份有效配置，贯穿容量、审计、包摘要、policy、工作目录、preflight、确认和异步执行。Process batch 的 preflight/确认、prepared admission 与排队 worker 使用同一份配置；后续新准入使用热加载后的配置。下游 MCP 调用保留各自资源专属快照，这不是全部操作的统一快照规则。 |
| `events.lowTtlSeconds`、各 `events.internalOverrides.<event-type>` 键 | 热加载；新接纳操作与外部注入使用新配置，已接纳的 Process/Skill install 保留其接纳时快照，之后产生的事件也继续使用该快照。 |
| `workspaceRoot` 及其配套 `pathPolicy` | 修改 workspace 需要重启；重启前原 workspace/path-policy 成对原子保留并继续生效 |
| `mode`、`profile`、`agentId` | 需要重启 |
| `browser` | 需要重启；配置的 Browser runtime 在进程启动时选择 |
| `room.*` 仓库、时区、日界线和 maintenance 设置 | 需要重启；`toolsets.enabled` 可热启用 Room，但使用当前 live Room 设置 |
| `tunnel.*` client identity/source/secret | 需要重启 |
| `hub`、reporting mode | 相关连接需要重启 |
| Skill install 并发等 startup-owned 设置 | 需要重启 |

共享 live subset 适用于每个 Agent worker；只有具有 Standalone HTTP MCP listener 的运行时
才会处理 `httpMcp` listener 字段。Local 没有 TCP listener，Hub 也不会因此把这个配置
section 变成 Hub ingress。

HTTP MCP 凭据无法解析时会 fail closed：endpoint 停止接受请求并停止监听，直到引用再次
可用。语法或语义无效的候选会被 watcher 拒绝并保留 last-good live 配置。监听地址冲突
也不会影响 tunnel 或 Unix execution；修复 endpoint 配置后 watcher 会重试。上述过程
不会输出引用或 token。

共享 watcher 在需要重启的字段变化时记录 `config changes require restart; fields=...`，其中包括
`browser`。诊断只列出发生变化的字段名，不会输出 secret 值。Standalone supervisor 另外输出
`restart_required`；Hub 没有 supervisor 事件。不要把“文件已修改”误认为现有子进程树已经切换。

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