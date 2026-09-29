# Agentic GPT

[English README](README.md)

Agentic GPT 通过每台机器独立的 Secure MCP Tunnel、可选的集中式 Rust Hub，或仅本机可访问的 Unix MCP socket，将 ChatGPT 连接到 Linux 机器。

对大多数部署，**推荐优先使用 Secure MCP Tunnel / Standalone 模式**。它不需要 VPS、公开反向代理或处在命令链路中的自建 Hub。每台机器拥有独立 tunnel 与 worker，因此某个 Agent 断连不会影响其他机器。

```text
推荐——Standalone
ChatGPT Secure MCP Tunnel
  -> 官方 tunnel-client
  -> agentic-gpt worker
  -> stdio MCP + 仅所有者可访问的 Unix MCP
  -> 可选的 worker 自有 HTTP MCP 端点：http://<host>:<port>/mcp
  -> 策略 / 文件 / 受管进程 / 技能 / 下游 MCP / tmux / Browser 运行时

集中式——Hub
ChatGPT Actions 或 Apps MCP
  -> HTTPS Rust Hub
  -> WebSocket 或 SSE 本地 Agent
  -> 相同的本地执行能力

开发联调——Local
本地 MCP 客户端或 agentic-gpt CLI
  -> 仅所有者可访问的 Unix MCP socket
  -> 相同的 Agent 工具面
```

历史上仅依赖 Cloudflare 的 Hub 已从 `main` 移除，仅在 `legacy/cf-worker-before-removal` 分支保留归档。

## 为什么优先 Standalone？

- 不需要 VPS、公开域名、反向代理、Hub 数据库或共享命令路由器。
- 每台机器具有独立连接与重启边界。
- Tunnel、HTTP 与仅所有者可访问的 Unix MCP 接入面对同一 `profile` 公布一致的工具集。按默认 profile 预设实时发现时，Normal 有 31 个工具、Room 有 42 个工具；该计数仅适用于默认预设，显式 `toolsets.enabled` 选择会改变工具数量。
- Standalone 还可启用 worker 自有的 Streamable HTTP MCP 端点，路径固定为 `/mcp`；该端点默认关闭，可使用直接 bearer token 认证，或选择 ChatGPT connector OAuth 流程。
- 策略、确认、审计、热配置、容量和受管进程状态都保留在本机。
- 新启动的 stdio worker 即使先收到旧逻辑会话续发的请求、尚未收到新的 MCP `initialize`，也能自动恢复而不退出。

当你需要多 Agent 的统一公开入口、Custom GPT Actions、集中式运行历史、Hub 原生聚合/通知或 Hub 中继远程确认时，Hub 模式仍然适用。

## 选择运行模式

| 模式 | 适用场景 | 是否需要公开服务器 | 故障范围 | 启动入口 |
| --- | --- | --- | --- | --- |
| **Secure MCP Tunnel / Standalone** | 推荐的直接部署；可选 worker 自有 HTTP MCP | 不需要 | 单个 tunnel/Agent | `agentic-gpt run`（配置 `mode=standalone`） |
| **Hub + Local Agent** | 集中路由、Actions、共享历史/报告 | 需要 | Hub 是共享依赖 | `agentic-gpt-hub serve` + `agentic-gpt run` |
| **Local Unix MCP** | 开发、冒烟测试、本地自动化 | 不需要 | 单个本地 worker | `agentic-gpt run`（配置 `mode=local`） |

## 主要能力

- `process.exec`、`process.batch`、`skills.run`、`mcp.callTool` 与 `mcp.batch` 可创建受管进程。
- 使用 `process.status`、`process.list`、`process.output`、`process.result` 和 `process.cancel`，通过 `processId` 查询或控制进程。
- 批量接纳具有原子性，确认边界有界。
- 可配置 allow / confirm / deny 命令策略，以及可写、只读和拒绝访问的路径根目录。
- 支持本地桌面确认，以及可选的 Hub 中继 ntfy 确认。
- 可选集成 bubblewrap 沙箱。
- 下游 MCP 参数/结果有界；支持按精确 request-id 取消；无法证明远端终止时如实返回 `detached` 状态。
- Room 引导、日记/笔记工具、公开来源的 Skill 安装、受管 Skill 执行和 tmux 持久工作区。
- 可选 Rust Hub 提供 Actions OpenAPI、兼容 Apps 的 `/mcp`、OAuth shim、HTTP API、WebSocket/SSE Agent、历史记录、报告和通知。

## 仓库结构

- `crates/agentic-gpt`：Linux Agent、Standalone supervisor、本地 MCP 运行时与 CLI。
- `crates/agentic-gpt-hub`：可选的 Rust Hub HTTP/WebSocket/SSE/MCP 服务。
- `crates/agentic-browser-host`：供自托管 ChatGPT Chrome 扩展 Browser backend 使用的可选 Linux Native Messaging 桥接程序。
- `crates/agentic-gpt-protocol`：共享 JSON 协议类型。
- `config.example.json`：严格的 v0.9、以 Standalone 为优先的配置示例，不含可用密钥。
- `openapi/hub.yaml`：Hub 模式 Custom GPT Actions schema。
- `docs/configuration.zh-CN.md`：按运行时区分的配置、密钥与热加载边界。
- `docs/standalone-runtime.md`：Tunnel/local 拓扑、信任、恢复、报告与工具矩阵。
- `docs/interfaces.md`：Hub HTTP、Actions、Apps MCP 与 Agent 协议索引。
- `docs/operations.md`：验证、部署与冒烟测试。

## 运行要求

共同要求：

- `agentic-gpt` 运行在 Linux 上。
- 使用对应架构的发行版二进制，或使用 Rust stable 从源码构建。
- 可选安装 `bubblewrap` 以启用沙箱。

Standalone 还需要分配好的 Secure MCP Tunnel id 与 API key 引用，但**不需要 VPS 或入站公开端口**。

Hub 模式额外需要服务器/VPS、HTTPS、公开部署时的反向代理、Hub API key 和每个 Agent 的 secret。

## 安装

发行压缩包包含 Agent、Hub 和 Browser-host 三个二进制。Standalone 与 Local 模式只需安装 `agentic-gpt`；仅 Hub 模式需要安装 `agentic-gpt-hub`，仅自托管 Chrome 扩展 Browser backend 需要安装 `agentic-browser-host`。

```bash
tar -xzf agentic-gpt-x86_64-unknown-linux-gnu.tar.gz
install -m 0755 agentic-gpt ~/.local/bin/
# 仅 Hub 模式：
install -m 0755 agentic-gpt-hub ~/.local/bin/
# 仅自托管 Browser backend：
install -m 0755 agentic-browser-host ~/.local/bin/
```

支持：

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

新增功能或模块，或修改配置、状态所有权及公共契约前，请先阅读[开发指南](docs/development/README.zh-CN.md)；打包与发布见[发布指南](docs/development/releasing.md)。
自托管 Browser 扩展/Neko 的部署方法见[自托管 Browser 部署指南](docs/browser-self-hosted.zh-CN.md)。

## 快速开始：Secure MCP Tunnel（推荐）

### 1. 初始化本地配置

在终端中运行 `agentic-gpt config init` 会打开键盘驱动的全屏配置界面，默认选择 Standalone 模式和 Normal 工具集预设。全屏模式要求 stdin、stdout、stderr 全部连接到终端；如果使用管道或重定向，会返回可操作的错误且不会写入文件。脚本和 CI 应显式使用 `--non-interactive`，通过确定性的流程生成配置。

```bash
agentic-gpt config init
agentic-gpt config set agentId laptop
agentic-gpt config set confirmationProvider.channels '["freedesktop"]'
```

脚本中请显式提供需要确定的值，并加上 `--non-interactive`：

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

第一条命令会写入安全的 Standalone + Normal 占位配置，并报告尚待处理的 tunnel ID 与密钥引用步骤；它不会自动配置密钥。`--mode` 选择运行时传输方式（Standalone、Hub 或 Local），`--profile` 选择默认工具集预设（normal 启用除 `room` 外的所有 namespace；room 启用所有 namespace）。显式 `toolsets.enabled` 选择优先于 profile 预设。全屏界面中的这些选项只会预填可编辑字段，不会锁定字段。配置流程为 Basic（基础信息）→ Connection（连接，Local 除外）→ Optional settings（可选设置）→ Review（复核）→ Completion（完成）；可反复进入可选设置，Review 会隐藏密钥值，也可跳回先前页面编辑。使用 Tab/Shift+Tab 和方向键移动焦点，Enter 编辑或执行操作，Esc 返回（在根 Basic 页面上不执行操作），Ctrl+C 取消。最终确认 Review 之前不会写入配置、备份或密钥文件。`--agent-secret` 是命令行参数；使用该参数会使密钥暴露于本地进程检查，若将密钥字面量写入 shell 命令还会进入 shell 历史记录。优先在全屏界面中通过隐藏输入设置。Tunnel API key 应使用 `file:`/`env:` 引用。本指南只说明键盘驱动的全屏配置流程，不承诺鼠标操作、行内模式、仪表盘模式或 Windows 行为。

默认配置路径为 `~/.agentic_gpt/config.json`。开放写入根或启用 MCP server 前，请先检查 [`config.example.json`](config.example.json) 和[配置说明](docs/configuration.zh-CN.md)。
Room 设置位于 `room`：`repositoryRoot` 为可选项，默认是 `<workspaceRoot>/room`；`maintenance.mode` 可设为 `local` 或 `workflow`，默认是 `local`；`maintenance.autoPush` 默认是 `false`。也可以使用 `agentic-gpt config set room.repositoryRoot null`、`agentic-gpt config set room.maintenance.mode local` 和 `agentic-gpt config set room.maintenance.autoPush false` 修改这些设置。
使用 `agentic-gpt config keys [--section <SECTION>] [--json]` 查看受控的 `config set` 配置项登记表。
使用以下命令管理工具 namespace：

```bash
agentic-gpt config toolset ls
agentic-gpt config toolset enable <namespace>
agentic-gpt config toolset disable <namespace>
```

`ls` 会显示每个 namespace 的启用状态与简短说明；`enable` 和 `disable` 成功修改后会显示 namespace 及修改后的状态。

可用工具 namespace 为 `agent`、`file`、`mcp`、`process`、`skills`、`tmux`、`browser` 和 `room`。逻辑上的 `room` namespace 包含 `bootstrap`、`bootstrap.read` 以及所有 `room.*` 工具。也可以直接编辑 JSON 中的 `toolsets.enabled`；有效修改会热加载，无需重启；无效候选配置会保留上一次有效选择。

### 2. 通过引用保存 tunnel 密钥

```bash
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
```

`tunnel.apiKey` 只接受 `file:PATH` 或 `env:NAME`；明文密钥会被拒绝。

### 3. 启动配置的运行时

```bash
agentic-gpt run
```

配置文件中的 `mode` 选择 Standalone/Hub/Local，`profile` 选择默认工具集预设。默认 normal 预设启用除 `room` 外的所有 namespace，room 预设启用所有 namespace；使用默认预设时，Normal 与 Room 的实时工具发现分别得到 31 个和 42 个工具。显式 `toolsets.enabled` 选择优先于 profile，因此实际工具数量可能改变。同一 worker 还会提供仅所有者可访问（owner-only）的 Unix MCP socket，便于本机检查：

```bash
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

在 ChatGPT 中连接分配给该 Agent 的 Secure MCP Tunnel。每台机器独立配置、独立启动。

可选的 Standalone HTTP MCP 与 tunnel 独立配置：

```bash
agentic-gpt config set httpMcp.bearerToken env:AGENTIC_HTTP_MCP_TOKEN
agentic-gpt config set httpMcp.host 127.0.0.1
agentic-gpt config set httpMcp.port 8765
agentic-gpt config set httpMcp.publicUrl https://mcp.example.com
agentic-gpt config set httpMcp.allowHosts '["mcp.example.com"]'
agentic-gpt config set httpMcp.enabled true
```

端点始终为 `http://<host>:<port>/mcp`；直接 Bearer 令牌认证使用 `file:` 或 `env:` 密钥引用，即使没有配置 `publicUrl` 也可使用。`publicUrl` 是供 ChatGPT connector OAuth 使用的可选、非密钥 HTTPS 外部来源地址（origin）；它会在全屏 TUI、`config init --non-interactive --http-mcp-public-url ...` 或 `config set httpMcp.publicUrl ...` 中明文显示并可编辑（使用 `null` 清除）。该值仅用于公布外部来源地址，不会改变本地绑定主机或端口。

配置 `publicUrl` 后，Standalone 会提供 OAuth 资源发现端点 `/.well-known/oauth-protected-resource/mcp`，并提供兼容根路径的 `/.well-known/oauth-protected-resource` 别名；授权服务器/OpenID Connect（AS/OIDC）元数据位于 `/.well-known/oauth-authorization-server` 和 `/.well-known/openid-configuration`。ChatGPT OAuth 授权使用 `/oauth/authorize`、`/oauth/token` 和唯一的 OAuth scope（权限范围）`agentic:mcp`，且只接受以下回调 URL：`https://chatgpt.com/connector/oauth/<suffix>`，或精确的 `https://chatgpt.com/connector_platform_oauth_redirect`。授权码和访问令牌只保存在当前监听器内存中，并可过期和撤销；不提供刷新令牌、`offline_access`、动态客户端注册、通用注册或任意重定向。OAuth 不会取代直接 Bearer 令牌认证；Standalone 的页面、工具和 `profile` 语义也不属于 Hub 的 `Hub API key`、`profile` 或路由契约。

Host 过滤默认仅允许本机环回地址（loopback）；`null` 或严格等于 `["*"]` 时明确允许任意 Host，空数组或与其他值混用的通配符会被拒绝。必须允许实际收到的 Host authority（包括反向代理或 ESA 改写 Host 的情况）；如果请求中存在 `Origin`，必须与 `publicUrl` 精确匹配；没有 `Origin` 的服务器间请求仍然允许。ChatGPT OAuth 部署必须通过 HTTPS 转发所有发现、授权、令牌和 `/mcp` 路由。来源地址（origin）可以是环回或私有网络地址；`publicUrl` 不负责路由，代理必须将实际发送的 authority 加入 `allowHosts`。重新绑定或禁用监听器会终止有状态会话并丢弃其本地 OAuth `state`；直接令牌内容轮换会原地更新认证，并撤销 OAuth 授权码/令牌。
Local 模式仍然只提供 Unix 入站接口。Hub 的 `/mcp` 属于独立的 OAuth/Hub 契约；`mcpServers` 仍是 `mcp.*` 使用的下游登记表，并非该入站监听器。

完整 schema、init/import 编辑流程、脱敏以及 live-reload/last-good 行为见[配置说明](docs/configuration.zh-CN.md)。

Tunnel-client 信任、缓存、恢复、报告及服务管理器（service manager）说明见[Standalone 运行指南](docs/standalone-runtime.md)。

## 仅本地开发

无需 tunnel 凭据：

```bash
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt run
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

Local 模式与 Standalone 使用相同的策略、路径策略、确认、审计、热配置和进程实现，但只提供仅所有者可访问（owner-only）的 Unix socket。

## 集中式 Hub 模式

只有在统一入口与集中式能力值得额外基础设施时，再选择 Hub 模式。

### 1. 启动 Hub

```bash
agentic-gpt-hub init
read -rsp "Agent secret: " AGENT_SECRET
printf '\n'
agentic-gpt-hub agent add \
  --agent-id laptop \
  --display-name my-laptop \
  --secret "$AGENT_SECRET"
unset AGENT_SECRET

read -rsp "Hub API key: " AGENTIC_GPT_API_KEY
printf '\n'
export AGENTIC_GPT_API_KEY
agentic-gpt-hub serve --bind 127.0.0.1:8787
unset AGENTIC_GPT_API_KEY
```

Hub 的 `agent add` 没有隐藏式交互输入；`--secret` 必须作为命令行参数提供。上例通过 `read -s` 避免将密钥写入 shell 历史记录，但变量展开后密钥仍会出现在本地进程检查中；请在可信机器上执行，并在操作后清除变量。Hub API key 通过环境变量传递，仍需保护进程环境。公开部署时应在 Hub 前配置 Caddy/Nginx，并通过 HTTPS 暴露。Hub 状态默认位于 `~/.agentic_gpt/hub.sqlite3`，配置默认位于 `~/.agentic_gpt/hub.json`。

### 2. 启动连接 Hub 的 Agent

```bash
agentic-gpt config init --mode hub
agentic-gpt config set hub.url https://agentic-gpt.example.com
agentic-gpt config set hub.transport websocket
agentic-gpt config set agentId laptop
agentic-gpt run
```

将 `profile` 设为 `room` 后使用 `agentic-gpt run` 启动 Room profile。显式 `toolsets.enabled` 选择优先；`hub.transport` 可设为 `websocket` 或 `sse`。`config init --mode hub` 会打开全屏 TUI，并将 `mode` 预填为 Hub；请在“Connection（连接）”页面通过隐藏输入设置与 Hub 注册值相同的 Agent 密钥。不要将密钥作为 `config set hub.agentSecret` 或 `config init --agent-secret` 的参数传入：参数值会暴露于本地进程检查中，若直接将密钥写在命令中还会进入 shell 历史记录。

对于现有 v0.9 配置或外部 JSON，请使用显式迁移流程 `agentic-gpt config import --config PATH [SOURCE]`（使用默认路径时可省略 `--config`）。省略 `SOURCE` 时会导入所选配置路径。该命令会将配置值填入标准 Config Init TUI，并通过常规备份事务写入当前嵌套 Hub schema。

### 3. 将 ChatGPT 连接到 Hub

- Custom GPT Actions：导入 [`openapi/hub.yaml`](openapi/hub.yaml)，并使用 `AGENTIC_GPT_API_KEY` 进行 Bearer 认证。
- ChatGPT Apps MCP：连接 `https://<your-hub-domain>/mcp`。

Hub 原生工具与转发执行使用相同的进程生命周期投影。使用 `process.status` 或 `process.list` 查询运行状态，使用 `process.output` 和 `process.result` 获取输出/结果，使用 `process.cancel` 取消，并传入返回的 `processId`。

## 受管进程与安全边界

- 每个受管进程都有一个 `processId` 和如实反映其生命周期的状态。`process.status` 只返回状态元数据；使用 `process.output` 获取有界输出，使用 `process.result` 获取保留的最终结果。
- Worker HTTP API 提供 `GET /v1/process`（列表）、`GET /v1/process/{processId}`（状态）、`GET /v1/process/{processId}/output`、`GET /v1/process/{processId}/result` 和 `POST /v1/process/{processId}/cancel`。Hub MCP 提供 `hub.process.status` 与 `hub.process.list`。
- `process.exec`、`skills.run` 和 `mcp.callTool` 会启动受管进程；`process.batch` 与 `mcp.batch` 返回有序的子进程投影。`mcp.batch` 接受 1–16 个调用，只进行一次聚合确认，并执行全局/单 server 并发限制。
- 创建响应最多内联包含 8 KiB 输出；更多初始输出使用共享预览，最多 2 KiB。`process.output` 默认 cursor 窗口为 8 KiB，最大为 32 KiB。
- 每次 MCP 调用的参数上限为 256 KiB；保留的进程结果上限为 512 KiB；批次聚合参数与结果各上限为 2 MiB。
- Worker 进程状态存储于 `process.sqlite3`。首次初始化进程存储不会迁移或修改旧的 `jobs.sqlite3` 数据。
- 审计记录包含有界元数据、哈希、状态与终止证据，不记录原始 MCP 参数/结果。
- 执行前使用 `agent.info` 查看当前 profile、路径策略、容量、确认、MCP 配置摘要和连接状态。

## 确认、命令策略与路径策略

```bash
agentic-gpt config set confirmationProvider.channels '["freedesktop"]'
agentic-gpt config set confirmationLanguage zh-CN

agentic-gpt config allow add bash
agentic-gpt config confirm add python -c
agentic-gpt config deny add ssh

agentic-gpt config path list
agentic-gpt config path write add ~/Projects
agentic-gpt config path readonly add /var/log
agentic-gpt config path deny add ~/.secrets
```

Hub 中继的 `ntfy` 是可选能力，只有在 Hub 模式或配置了 Standalone Hub 报告/确认中继时才有意义。本地拒绝或超时即为最终结果。

字段定义与热加载行为见[配置说明](docs/configuration.zh-CN.md)。

## 更多文档

- [配置说明](docs/configuration.zh-CN.md)：运行时选择、主要配置块、密钥引用与热加载/重启边界。
- [Standalone 运行指南](docs/standalone-runtime.md)：Standalone/Local 运行、tunnel-client 信任、恢复、报告与精确工具矩阵。
- [接口索引](docs/interfaces.md)：Hub HTTP、Actions、Apps MCP、协议与直接 MCP surface。
- [工具契约矩阵](docs/tool-contract-matrix.md)：Normal/Room/Hub 工具契约、边界与 surface parity 矩阵。
- [运维指南](docs/operations.md)：本地验证、Standalone-first 部署检查、Hub 检查与安全不变量。
- [开发指南](docs/development/README.zh-CN.md)：新增功能/模块，或审查所有权、权限、配置与契约变更前阅读；包含验证方式与完成标准。
- [发布指南](docs/development/releasing.md)：执行发布预检、打包或推送发布标签前阅读。

## 构建与发布

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
rust_version="$(cargo metadata --no-deps --format-version=1 | python3 -c 'import json,sys; packages={p["name"]:p["version"] for p in json.load(sys.stdin)["packages"]}; print(packages["agentic-gpt"])')"
git tag "v${rust_version}"
git push origin "v${rust_version}"
```

标签版本从 `agentic-gpt` 的 Cargo package version 派生；`agentic-gpt-hub` 必须使用相同版本，发布预检会校验二者匹配。创建或推送 tag 是独立发布动作；普通提交不会发布任何内容。

## 安全说明

- Tunnel API key、Hub API key、Agent 密钥与 ntfy topic 都应视为凭据。
- Tunnel 密钥优先使用 `file:` 或受保护的 `env:` 引用，不要将其以明文写入配置。
- 凭据、浏览器、云平台和 SSH 目录应放在 denied roots 中。
- 对 shell、网络工具和陌生 MCP server 优先要求确认。
- 使用有界的 `process.output`/`process.result` 获取结果，避免 HTTP/MCP 请求无限期阻塞。
- Hub 公开部署时必须使用 HTTPS。

## 许可证

AgenticGPT 原创代码和主代码采用 MIT License。

`crates/agentic-apply-patch` 子 crate 包含源自 OpenAI Codex、依据 Apache License 2.0 发布的代码。详见 [`crates/agentic-apply-patch/LICENSE`](crates/agentic-apply-patch/LICENSE) 和 [`crates/agentic-apply-patch/NOTICE`](crates/agentic-apply-patch/NOTICE)。
