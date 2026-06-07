# Agentic GPT

[English README](README.md)

Agentic GPT 是一个 Linux 本地执行代理和 Rust Hub，用来让 ChatGPT 在受控条件下连接本地机器。

它适合这样的工作流：ChatGPT 需要查看本地状态、执行短命令、启动长时间会话、桥接本地配置的 MCP 服务，并且在敏感操作前请求用户确认。

```text
ChatGPT Actions / ChatGPT Apps MCP
  -> Rust Hub 的 HTTPS API
  -> 到 Local Agent 的 WebSocket 连接
  -> 本地进程 / 会话 / 确认 / MCP 桥接 / 沙箱
```

当前主线使用 Rust Hub。旧的 Cloudflare Worker 实现已经从 `main` 移出；如果确实需要历史上的 Cloudflare-only Hub，可以查看 `legacy/cf-worker-before-removal` 分支。

## 功能

- 通过常驻 local agent 执行本地命令。
- 支持短命令同步执行和长时间会话。
- 支持批量命令执行，并采用 all-or-nothing 的确认语义。
- 支持本地桌面确认，以及可选的 Hub 远程确认。
- 可配置命令策略：allow、confirm、deny。
- 可配置路径策略：可写根目录、只读根目录、拒绝访问根目录。
- 可选 bubblewrap 沙箱。
- 支持把 ChatGPT 请求桥接到 local agent 内配置的 MCP server。
- 提供 ChatGPT Actions OpenAPI schema 和 ChatGPT Apps 友好的 MCP endpoint。

## 仓库结构

- `crates/agentic-gpt`：Linux local agent CLI。
- `crates/agentic-gpt-hub`：Rust Hub HTTP/WebSocket 服务。
- `crates/agentic-gpt-protocol`：共享 JSON 协议类型。
- `openapi/hub.yaml`：Rust Hub 的 Custom GPT Actions schema。
- `docs/interfaces.md`：Actions、Apps MCP、Local Agent WebSocket 的接口地图。
- `docs/operations.md`：本地验证、smoke test、部署检查和安全不变量。
- `scripts/dist-linux.sh`：多目标 Linux release 构建脚本。

## 运行要求

- local agent 运行在 Linux 机器上。
- 对应目标平台的 release 二进制；如果从源码构建，则需要 Rust stable。
- 如果要让远端 ChatGPT 访问，需要一台服务器或 VPS 跑 Hub。
- 对外暴露 Hub 时建议使用 Caddy 或 Nginx 做 HTTPS 反代。
- 可选：`bubblewrap`，用于沙箱执行。
- 可选：`ntfy`，用于 Hub 远程确认。

## 安装

从 GitHub Releases 下载对应目标平台的 release archive，然后解压两个二进制并放到 `PATH` 中：

```bash
tar -xzf agentic-gpt-x86_64-unknown-linux-gnu.tar.gz
install -m 0755 agentic-gpt agentic-gpt-hub ~/.local/bin/
```

当前 release 目标：

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

源码构建、CI 和 release 发布流程见 [`docs/development.zh-CN.md`](docs/development.zh-CN.md)。

## 快速开始

### 1. 启动 Hub

```bash
agentic-gpt-hub init
agentic-gpt-hub agent add \
  --agent-id laptop \
  --display-name my-laptop \
  --secret '<agent-secret>'
AGENTIC_GPT_API_KEY='<high-entropy-api-key>' \
  agentic-gpt-hub serve --bind 127.0.0.1:8787
```

Hub 状态默认保存在 `~/.agentic_gpt/hub.sqlite3`，Hub 配置默认保存在 `~/.agentic_gpt/hub.json`。

如果要公开访问，把 Caddy 或 Nginx 放在 Hub 前面，并通过 HTTPS 暴露。Hub 同时提供 HTTP API 和 WebSocket endpoint。

### 2. 启动 Local Agent

```bash
agentic-gpt config init
agentic-gpt config set hubUrl http://127.0.0.1:8787
agentic-gpt config set agentId laptop
agentic-gpt config set agentSecret '<agent-secret>'
agentic-gpt config set confirmationProvider freedesktop-then-hub
agentic-gpt config set confirmationLanguage zh-CN
agentic-gpt run
```

Local agent 配置位于 `~/.agentic_gpt/config.json`；审计日志以 JSONL 写入 `~/.agentic_gpt/audit.log`。

`workerUrl` 作为旧字段别名仍然可以读取和设置，但规范字段是 `hubUrl`。

### 3. 连接 ChatGPT

Custom GPT Actions 使用 `openapi/hub.yaml`，把里面的 server URL 替换成你的 HTTPS Hub 地址，并用 `AGENTIC_GPT_API_KEY` 配置 Bearer auth。

ChatGPT Apps / MCP 使用 Apps 友好的 MCP endpoint：

```text
https://<your-hub-domain>/mcp
```

OAuth discovery 和 token exchange 由 Hub 的 OAuth shim 提供。

## 确认机制

Local agent 可以在命令匹配 confirm 策略时请求用户确认。

```bash
agentic-gpt config set confirmationProvider freedesktop-then-hub
agentic-gpt config set confirmationLanguage zh-CN
```

支持的确认 provider：

- `freedesktop`：本地桌面通知按钮。
- `hub`：Hub 远程确认。
- `freedesktop-then-hub`：优先本地桌面确认；仅在本地 provider 不可用时回退到 Hub。

本地拒绝或超时是最终结果，不会继续回退到 Hub。

支持的确认语言：

- `en`
- `zh-CN`

远程确认默认关闭，需要在 Hub 侧启用，而不是每个 Local Agent 单独启用：

```json
{
  "remoteConfirmation": {
    "enabled": true,
    "provider": "ntfy",
    "timeoutSeconds": 45,
    "ntfy": {
      "serverUrl": "https://ntfy.example.com",
      "topic": "<high-entropy-topic>",
      "callbackBaseUrl": "https://agentic-gpt.example.com"
    }
  }
}
```

ntfy callback route 不会出现在 GPT Actions OpenAPI 里。它们只由 ntfy 按钮调用，并且要求 callback URL 中的一次性确认 token。

## 命令策略

命令策略可以通过命令本身添加或删除。`remove` 使用 `program` 加可选 `argsPrefix` 匹配；如果交互式终端中命中多条规则，CLI 会询问删除哪一条。

```bash
agentic-gpt config allow add bash
agentic-gpt config allow remove bash
agentic-gpt config confirm add python -c
agentic-gpt config confirm remove python -c
agentic-gpt config deny add ssh
```

策略优先级偏保守。内置 deny 规则仍会生效，除非用配置里的 allow 规则显式覆盖。

## 路径策略

路径访问由 local agent 配置中的 `pathPolicy` 控制。

`workspaceRoot` 总是可写根目录。默认还允许写入 `~/Documents`、`~/Downloads`、`~/Projects` 和 `/tmp`，允许只读访问部分系统/cache 路径，并拒绝访问常见凭据、浏览器、认证和云服务配置路径。

管理路径根目录：

```bash
agentic-gpt config path list
agentic-gpt config path write add ~/Projects
agentic-gpt config path readonly add /var/log
agentic-gpt config path deny add ~/.secrets
agentic-gpt config path write remove ~/Projects
```

`exec`、`batchExec` 和 `startSession` 也支持 `workingDirectory`。解析后的目录必须存在，必须位于可写根目录内，并且不能位于拒绝访问根目录内。
## 更多文档

- [`docs/interfaces.md`](docs/interfaces.md)：API、Actions、Apps MCP 和 Local Agent WebSocket 的接口地图。
- [`docs/operations.md`](docs/operations.md)：部署检查、smoke test 和安全不变量。
- [`docs/development.zh-CN.md`](docs/development.zh-CN.md)：源码开发、验证、CI 和 release 发布。

`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt-hub`

推送版本 tag 会构建 Linux release archives 并发布 GitHub Release：

```bash
git tag v0.1.0
git push origin v0.1.0
```

Release archive 每个 target 包含两个二进制：

- `agentic-gpt-x86_64-unknown-linux-gnu.tar.gz`
- `agentic-gpt-aarch64-unknown-linux-gnu.tar.gz`
- `SHA256SUMS`

## 安全说明

Agentic GPT 的目标是让本地执行变得明确、受控、可审计，而不是让本地执行“没有风险”。Hub API key、agent secret、ntfy topic 都应视为敏感凭据。

推荐默认做法：

- Hub 前面使用 HTTPS。
- 使用高熵 Hub API key 和 agent secret。
- 把凭据目录保留在 denied roots 内。
- 对 shell interpreter 和网络工具优先使用 confirm。
- 长时间命令使用 session，不要硬塞进短命令超时。
- 调试或收紧策略时查看 `~/.agentic_gpt/audit.log`。

## License

MIT
