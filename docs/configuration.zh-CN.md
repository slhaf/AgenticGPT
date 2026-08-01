# 配置说明

Agentic GPT 的 Standalone、Local Unix MCP 与连接 Hub 的 Agent 共用一份本地 JSON 配置。默认路径：

```text
~/.agentic_gpt/config.json
```

从以下命令开始：

```bash
agentic-gpt config init
agentic-gpt config show
```

[`config.example.json`](../config.example.json) 是严格的 v0.9 superset 示例：以 Standalone 为优先入口，不包含可用凭据，示例下游 MCP server 全部保持 disabled，同时保留 Hub 模式需要的可选字段。

## 各 runtime 必需项

| 配置组 | Standalone | Local Unix MCP | 连接 Hub 的 Agent |
| --- | --- | --- | --- |
| 公共 identity/workspace/policy | 必需 | 必需 | 必需 |
| `tunnel` | 必需 | 忽略 | 忽略 |
| `hubUrl`、`hubTransport`、`agentSecret` | 仅可选 Hub reporting/ntfy relay 使用 | 忽略 | 必需 |
| 公开 Hub/VPS | 不需要 | 不需要 | 需要 |
| 启动命令 | `run-as-standalone` | `run-as-local` | `run` / `run-as-room` |

所有模式的 JSON 类型仍保留 Hub 字段，便于同一配置在不同 runtime 之间切换。Standalone 与 Local 的命令链路不经过 Hub。Standalone 只有在启用 `tunnel.hubReporting.enabled` 或使用 Hub-backed `ntfy` 确认时才会使用 Hub 字段。

## Standalone-first 配置

```bash
agentic-gpt config init
agentic-gpt config set agentId laptop
agentic-gpt config set confirmationProvider freedesktop

install -d -m 700 "$HOME/.config/agentic-gpt"
# 使用 secret manager 或受保护的编辑器写入 API key。
chmod 600 "$HOME/.config/agentic-gpt/tunnel-api-key"

agentic-gpt config set tunnel.tunnelId tunnel_<assigned-id>
agentic-gpt config set tunnel.apiKey file:"$HOME/.config/agentic-gpt/tunnel-api-key"
agentic-gpt config set tunnel.client.autoDownload true
agentic-gpt run-as-standalone --profile normal
```

Room surface 使用 `--profile room`。

## 顶层字段

| 字段 | 用途 |
| --- | --- |
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
| `hubUrl`、`hubTransport`、`agentSecret` | 集中式 Hub 连接，或 Standalone 的可选 Hub reporting/ntfy relay。 |

未知顶层字段会在 load/write round trip 中保留。`limits` 等严格嵌套对象会拒绝已经删除的 v0.8 字段。

## Tunnel 配置

```json
{
  "tunnel": {
    "tunnelId": "tunnel_<assigned-id>",
    "apiKey": "file:/home/me/.config/agentic-gpt/tunnel-api-key",
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
  "hubUrl": "https://agentic-gpt.example.com",
  "hubTransport": "websocket",
  "agentId": "laptop",
  "agentSecret": "<agent-secret>"
}
```

`hubTransport` 可为 `websocket` 或 `sse`。`workerUrl` 仍可作为 `hubUrl` 的读取/设置 alias，但写回时使用规范字段。

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

`maxActiveJobs` 接受非负整数或 `"auto"`。Auto 按 `ceil(availableParallelism * 1.5)` 计算，并限制在 6–24。Process、Skill 与 MCP Job 共用该容量。

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
      "headers": {
        "Authorization": "env:TODOS_MCP_AUTHORIZATION",
        "X-Tenant": "file:/run/secrets/todos-mcp-tenant"
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

Server id 最长 64 字节，只使用字母、数字、`.`、`_`、`-`。`streamable-http` 需要绝对 HTTP(S) URL；`stdio` 需要非空命令。在审查信任与确认策略之前，示例应保持 disabled。

`streamable-http` 支持通过 `headers` 配置静态自定义 HTTP Header。Header 名称不区分大小写；`Accept`、`Content-Type`、`Mcp-Session-Id`、`Last-Event-Id`、`MCP-Protocol-Version` 等由 transport 管理的名称会被拒绝。每个 Header 值必须完整引用 `env:VARIABLE_NAME` 或 `file:/path`；明文 credential 会被拒绝。Authorization Header 的引用值应包含完整内容，例如 `Bearer <token>`。

引用会在新的 MCP admission/call 快照创建时解析。配置 revision 基于原始引用计算；解析后的值不参与 revision，只存在于私有内存 client 快照中。secret source 缺失、为空或无效时，仅当前调用失败，不影响其他 server。带 Header 的非 loopback endpoint 必须使用 HTTPS；携带这些 Header 的 client 禁止重定向，因此重定向请求不会把 Header 转发到其他 origin。该静态 Header 功能不包含 OAuth 流程。

## Skills、Room 与 sandbox

`skills` 控制 package 大小、redirect、timeout、重试/总 deadline、安装/下载并发，以及可选 host allowlist。规范字段是顶层 `skills`；只有缺少顶层字段时才读取 legacy `room.skills`。

`room.timezone` 控制 Room 日期时间行为；`room.diaryDayBoundaryHour` 范围 0–23；`room.notebookRoot` 可选。

`sandbox.enabled` 启用 bubblewrap；`requiredRuntimePaths` 定义 sandbox 中可见的宿主路径。Sandbox 不能替代命令策略、路径策略或确认。

## CLI 可管理字段

`agentic-gpt config set` 支持常用 scalar：

- `agentId`、`agentSecret`、`hubUrl`、`hubTransport`、`workspaceRoot`
- `confirmationProvider`、`confirmationLanguage`、`sandbox.enabled`
- `tunnel.tunnelId`、`tunnel.apiKey`
- 全部 `tunnel.client.*` 与 `tunnel.hubReporting.*`
- `room.notebookRoot`、`room.timezone`、`room.diaryDayBoundaryHour`
- 文档列出的 `skills.*` scalar/list 字段

结构化策略与 MCP 修改使用 `config allow/confirm/deny`、`config path`、`config mcp`。复杂 JSON 也可在进程停止时直接编辑，随后执行 `agentic-gpt config show` 与 smoke test。

## 热加载与重启边界

Standalone 与 Local worker 会轮询配置，并原子应用通过验证的 live subset。无效候选会保留上一份有效状态。

| 配置 | 行为 |
| --- | --- |
| `policy`、`pathPolicy`、`limits`、`mcpServers` | 对新 admission/call 热加载 |
| 已接纳 Job 与已创建下游调用 | 保留原决策/配置 |
| `agentId`、`workspaceRoot`、profile | 需要重启 |
| `tunnel.*` client identity/source/secret | 需要重启 |
| `hubUrl`、`hubTransport`、`agentSecret`、reporting mode | 对相关连接需要重启 |
| Skill install 并发等 startup-owned 设置 | 需要重启 |

有效的 `mcpServers` 热加载后，新 MCP 快照会重新解析 Header 引用；已经创建的下游调用继续使用原有解析后的 Header 快照。Standalone supervisor 检测到 startup identity 变化时会输出 `restart_required`。不要把“文件已修改”误认为现有子进程树已经切换。

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
