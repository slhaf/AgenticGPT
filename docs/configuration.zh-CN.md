# 配置说明

Agentic GPT 的 Standalone、Local Unix MCP 与连接 Hub 的 Agent 共用一份本地 JSON 配置。默认路径：

```text
~/.agentic_gpt/config.json
```

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
- `--profile normal|room` 选择能力/工具面。Normal 暴露 24 个工具，Room 暴露 36 个工具；
  配置档不会把 Local runtime 变成 Hub runtime。

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
工作区/路径策略、确认方式/语言、限制和沙箱始终可选。只有 Room 配置档会出现 Room 设置；
只有 Standalone 模式会出现 tunnel-client 覆盖和 Hub reporting。Hub 与 Local 模式不会显示
这些 tunnel 部分。不选可选部分时会保留模板默认值。

界面使用键盘导航：Tab/Shift+Tab 与方向键移动焦点，Enter 编辑或触发当前操作，Esc 返回
（根 Basic 页面是 no-op），Ctrl+C 取消初始化。编辑态按 Esc 只结束编辑，不会取消初始化。
Review 会隐藏密钥，可跳回 Basic、Connection 或可选 section 编辑；最终确认前不会写入配置、
备份或密钥文件。本功能只承诺键盘全屏流程；鼠标、inline、dashboard 与 Windows 行为不在
本功能契约内。

`config init --language auto|zh-CN|en` 选择 CLI 界面语言。使用 `auto` 时依次检查
`LC_ALL`、`LC_MESSAGES`、`LANG`，都没有匹配时使用 English。显式的 `zh-CN` 或 `en`
优先于环境变量。这个界面选择与持久化的 `confirmationLanguage` 不同；后者控制 runtime
发出的确认提示语言，可在可选配置 section 或通过 `config set` 设置。

首次配置刻意不包含 MCP server 集合与命令策略集合。初始化后分别使用 `config mcp`、
`config allow`、`config confirm`、`config deny` 配置它们（路径根使用 `config path`）。

## 各 runtime 必需项

| 配置组 | Standalone | Local Unix MCP | 连接 Hub 的 Agent |
| --- | --- | --- | --- |
| 公共 identity/workspace/policy | 必需 | 必需 | 必需 |
| `tunnel` | 必需 | 忽略 | 忽略 |
| `hub`（`url`、`transport`、`agentSecret`） | 仅可选 Hub reporting/ntfy relay 使用 | 忽略 | 必需 |
| 公开 Hub/VPS | 不需要 | 不需要 | 需要 |
| 启动命令 | `agentic-gpt run` | `agentic-gpt run` | `agentic-gpt run` |

所有模式的 JSON 类型仍保留嵌套 `hub` section，便于同一配置在不同 runtime 之间切换。Standalone 与 Local 的命令链路不经过 Hub。Standalone 只有在启用 `tunnel.hubReporting.enabled` 或使用 Hub-backed `ntfy` 确认时才会使用 Hub 字段；显式配置的非活动 section 会保留。

## Standalone-first 配置

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

将 `profile` 设为 `room` 即可使用 Room surface（例如 `agentic-gpt config set profile room`）。

## 顶层字段

| 字段 | 用途 |
| --- | --- |
| `mode` | 权威运行时分派：`standalone`、`hub` 或 `local`。 |
| `profile` | 权威能力 surface：`normal` 或 `room`。 |
| `agentId` | 稳定本地 identity，也用于派生私有 runtime/socket 路径。 |
| `displayName` | summary/reporting 中的人类可读机器名称。 |
| `workspaceRoot` | 主可写工作区，也是 `.agentic-gpt-audit.jsonl` 所在位置。 |
| `backupLimit` | Agentic 管理配置写入时保留的备份数量。 |
| `confirmationProvider` | 有序的本地/远程确认通道。 |
| `confirmationLanguage` | `en` 或 `zh-CN`。 |
| `sandbox` | 可选 bubblewrap 配置。 |
| `mcpServers` | `mcp.*` 转发的下游 MCP server。 |
| `pathPolicy` | 可写、只读、拒绝路径根。 |
| `policy` | 显式 allow / confirm / deny 命令规则。 |
| `limits` | Process 并发与总 active Job 容量。 |
| `skills` | Skill package/install 限制与网络策略。 |
| `room` | Room 时区、日记日界线和可选 notebook root。 |
| `tunnel` | Standalone tunnel-client 来源、secret 引用与可选 reporting。 |
| `hub` | 集中式 Hub 连接，或 Standalone 的可选 Hub reporting/ntfy relay。 |

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
agentic-gpt config allow add bash
agentic-gpt config confirm add python -c
agentic-gpt config deny add ssh

agentic-gpt config path list
agentic-gpt config path write add ~/Projects
agentic-gpt config path readonly add /var/log
agentic-gpt config path deny add ~/.secrets
```

配置的 allow 规则可以显式覆盖 builtin confirm/deny。多个配置规则匹配时，除非存在按运行时策略生效的更明确 allow override，否则 deny 优先。

`workspaceRoot` 始终视为可写。Denied roots 覆盖 writable/read-only roots。Symlink 会解析，最终目标必须留在有效策略边界内。

## Limits

```json
{
  "limits": {
    "maxConcurrentTasks": 2,
    "maxActiveJobs": "auto",
    "maxFileSearchContextLines": 5
  }
}
```

`maxConcurrentTasks` 限制单次 `process.batch` 中同时实际运行的子 Process Job 数量。所有子 Job 仍会整批 admission；超过并发槽的子 Job 保持 `queued`，因此该限制不会阻止 batch 在有界 `waitSeconds` 后返回。配置小于 1 时，有效下限为 1。

`maxActiveJobs` 接受非负整数或 `"auto"`。Auto 按 `ceil(availableParallelism * 1.5)` 计算，并限制在 6–24。Process、Skill 与 MCP Job 共用该容量，排队中的 batch 子 Job 也计入该容量。

`maxFileSearchContextLines` 是 `file.search` 对每个匹配返回的前后文行数 live 上限，默认 5，接受 0–100 的整数。请求可以超过该值；运行时会裁剪到 effective 值，并返回 `requestedContextLines`、`effectiveContextLines`、`contextLinesClipped` 与一个有界 warning。负数或非整数请求仍会被拒绝。

v0.9 会拒绝 `maxActiveSessions` 与 `sessionIdleTimeoutSecs`。

## 下游 MCP server

```json
{
  "mcpServers": {
    "docs": {
      "enabled": false,
      "transport": "streamable-http",
      "url": "https://mcp.example.com/mcp"
    },
    "local-tool": {
      "enabled": false,
      "transport": "stdio",
      "url": "node /home/me/mcp/server.mjs"
    }
  }
}
```

Server id 最长 64 字节，只使用字母、数字、`.`、`_`、`-`。`streamable-http` 需要绝对 HTTP(S) URL；`stdio` 需要非空命令。在审查信任与确认策略之前，示例应保持 disabled。

## Skills、Room 与 sandbox

`skills` 控制 package 大小、redirect、timeout、重试/总 deadline、安装/下载并发，以及可选 host allowlist。规范字段是顶层 `skills`；只有缺少顶层字段时才读取 legacy `room.skills`。

`room.timezone` 控制 Room 日期时间行为；`room.diaryDayBoundaryHour` 范围 0–23；`room.notebookRoot` 可选。

`sandbox.enabled` 启用 bubblewrap；`requiredRuntimePaths` 定义 sandbox 中可见的宿主路径。Sandbox 不能替代命令策略、路径策略或确认。

## CLI 可管理字段

`config set` 使用受控 registry，并不是通用 JSONPath 编辑器。使用当前语言列出 registry：

```text
agentic-gpt config keys [--section <SECTION>] [--json]
```

文本形式按 `runtime`、`identity`、`hub`、`confirmation`、`sandbox`、`limits`、`skills`、`room`、
`tunnel` 分组；`--section` 只显示其中一个分组。`--json` 返回机器可读的类型、是否可为
null、示例、双语说明和别名元数据。`config set`
只接受 registry 中的键；结构化 policy 与 MCP 集合应使用专用命令。

注册键后的值是一个 shell 参数。因此 JSON 列表必须加引号；`room.notebookRoot` 可为 null，
使用字面量 JSON 值 `null` 可以清除它。

```bash
agentic-gpt config set sandbox.requiredRuntimePaths '["/usr","/opt/runtime"]'
agentic-gpt config set skills.allowedHosts '["skills.example.com"]'
agentic-gpt config set room.notebookRoot null
```

registry 包含以下常用 scalar：

- `mode`、`profile`、`agentId`、`hub.url`、`hub.transport`、`hub.agentSecret`、`workspaceRoot`
- `confirmationProvider.channels`、`confirmationLanguage`、`sandbox.enabled`
- `tunnel.tunnelId`、`tunnel.apiKey`
- 全部 `tunnel.client.*` 与 `tunnel.hubReporting.*`
- `room.notebookRoot`、`room.timezone`、`room.diaryDayBoundaryHour`
- 文档列出的 `skills.*` scalar/list 字段

结构化策略与 MCP 修改使用 `config allow/confirm/deny`、`config path`、`config mcp`。复杂 JSON 也可在进程停止时直接编辑，随后执行 `agentic-gpt config show` 与 smoke test。

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

Standalone 与 Local worker 会轮询配置，并原子应用通过验证的 live subset。无效候选会保留上一份有效状态。

| 配置 | 行为 |
| --- | --- |
| `policy`、`pathPolicy`、`limits`、`mcpServers` | 对新 admission/call 热加载 |
| 已接纳 Job 与已创建下游调用 | 保留原决策/配置 |
| `mode`、`profile`、`agentId`、`workspaceRoot` | 需要重启 |
| `tunnel.*` client identity/source/secret | 需要重启 |
| `hub`、reporting mode | 对相关连接需要重启 |
| Skill install 并发等 startup-owned 设置 | 需要重启 |

Standalone supervisor 检测到 startup identity 变化时会输出 `restart_required`。不要把“文件已修改”误认为现有子进程树已经切换。

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
