# Agentic GPT

[English](README.md)

Agentic GPT 将 ChatGPT 连接到 Linux 机器，提供文件操作、命令执行、技能、浏览器和本地事件等能力。

推荐使用 **Standalone**：每台机器通过自己的 Secure MCP Tunnel 连接 ChatGPT，无需自建 Hub、VPS 或开放入站端口。

| 运行模式 | 适用场景 |
| --- | --- |
| Standalone | 通过 Secure MCP Tunnel 直接连接一台机器；也可配置 HTTP MCP 接入。 |
| Local | 通过本机 Unix MCP socket 开发、调试和自动化。 |
| Hub | 为多台机器提供统一入口，使用 Custom GPT Actions 或集中管理运行记录。 |

## 主要能力

- 读取、搜索和编辑文件，支持图片与补丁。
- 执行命令和批量任务，查看输出、历史并取消运行中的进程。
- 安装和运行 Skill，调用下游 MCP 服务。
- 使用 tmux 工作区和 Browser 运行时。
- 保存本地事件，通过事件收件箱查看和处理提醒。
- 通过 Room 管理 Diary、Notebook 和维护任务。
- 配置命令及路径策略，使用桌面或 ntfy 确认；可选启用 bubblewrap 沙箱。

## 安装与要求

Agent 运行在 Linux 上，发行包支持 `x86_64` 和 `aarch64`。从[发行版页面](https://github.com/slhaf/AgenticGPT/releases)下载对应架构的压缩包并安装；以下以 `x86_64` 为例：

```bash
tar -xzf agentic-gpt-x86_64-unknown-linux-gnu.tar.gz
install -d ~/.local/bin
install -m 0755 agentic-gpt ~/.local/bin/
```

请确保 `~/.local/bin` 在 `PATH` 中。发行包还包含 `agentic-gpt-hub` 和 `agentic-browser-host`，分别用于 Hub 部署和自托管 Browser backend，按需安装即可。

**从旧版本升级到 0.10.0 前，请先阅读[发布说明与升级步骤](docs/release-notes-v0.10.0.md)。**

## 快速开始：Standalone

准备好分配给这台机器的 Secure MCP Tunnel ID 和 API key。

### 1. 初始化配置

```bash
agentic-gpt config init
```

在配置界面选择 Standalone，并设置 Agent 身份和 Tunnel 连接信息。默认配置文件为 `~/.agentic_gpt/config.json`。脚本环境可使用 `config init --non-interactive`；具体选项见[配置说明](docs/configuration.zh-CN.md)。

### 2. 保存 Tunnel 凭据

下面将 API key 保存到私有文件，再配置文件引用；将 `tunnel_<assigned-id>` 替换为分配的 Tunnel ID。

```bash
install -d -m 700 "$HOME/.agentic_gpt/secrets"
touch "$HOME/.agentic_gpt/secrets/tunnel-api-key"
chmod 600 "$HOME/.agentic_gpt/secrets/tunnel-api-key"
read -rsp "Tunnel API key: " AGENTIC_TUNNEL_API_KEY
printf '\n'
printf '%s' "$AGENTIC_TUNNEL_API_KEY" > "$HOME/.agentic_gpt/secrets/tunnel-api-key"
unset AGENTIC_TUNNEL_API_KEY

agentic-gpt config set tunnel.tunnelId 'tunnel_<assigned-id>'
agentic-gpt config set tunnel.apiKey file:"$HOME/.agentic_gpt/secrets/tunnel-api-key"
```

Tunnel API key 使用 `file:` 或 `env:` 引用，不接受明文配置。工作目录、可访问路径和确认方式可在初始化界面或配置文件中调整。

### 3. 启动并连接

```bash
agentic-gpt run
```

在 ChatGPT 中连接分配给该机器的 Secure MCP Tunnel。本机可用以下命令检查工具和运行状态：

```bash
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

HTTP MCP 接入及认证配置见[配置说明](docs/configuration.zh-CN.md#standalone-入站-http-mcp-端点)，服务管理和故障排查见[Standalone 运行指南](docs/standalone-runtime.md)。

## 其他运行模式

### Local

无需 Tunnel 凭据，在本机通过 Unix MCP socket 使用 Agent：

```bash
agentic-gpt config init --mode local --non-interactive
agentic-gpt run
```

使用上面的 `local list-tools` 和 `local call` 命令访问本机 Agent；stdio MCP 客户端可使用 `agentic-gpt local stdio`。详见[本地 MCP 控制通道](docs/standalone-runtime.md)。

### Hub

安装发行包中的 `agentic-gpt-hub`，在 Hub 注册 Agent，再通过 `agentic-gpt config init --mode hub` 配置该 Agent 的连接信息。公开部署需要 HTTPS。

- Custom GPT Actions 使用 [`openapi/hub.yaml`](openapi/hub.yaml)。
- ChatGPT Apps MCP 连接 Hub 的 `/mcp` 端点。

部署步骤、凭据和代理配置见[Hub 运行指南](docs/standalone-runtime.md)及[运维指南](docs/operations.md#hub-部署检查)。

## 文档

| 需要了解的内容 | 文档 |
| --- | --- |
| 配置、权限策略、确认、Toolsets | [配置说明](docs/configuration.zh-CN.md) · [配置示例](config.example.json) |
| Tunnel、本地 MCP、服务管理 | [Standalone 运行指南](docs/standalone-runtime.md) |
| 工具与 HTTP API | [接口参考](docs/interfaces.md) · [工具矩阵](docs/tool-contract-matrix.md) |
| Process 接口及旧客户端切换 | [Process 切换说明](docs/process-cutover.md) |
| Browser 与自托管部署 | [Browser 部署指南](docs/browser-self-hosted.zh-CN.md) |
| 升级与运维 | [0.10.0 发布说明](docs/release-notes-v0.10.0.md) · [运维指南](docs/operations.md) |
| 源码开发、构建与发布 | [开发指南](docs/development/README.zh-CN.md) · [发布指南](docs/development/releasing.md) |

## 许可证

AgenticGPT 原创代码和主代码采用 MIT License。

`crates/agentic-apply-patch` 包含源自 OpenAI Codex、依据 Apache License 2.0 发布的代码。详见该 crate 的 [LICENSE](crates/agentic-apply-patch/LICENSE) 和 [NOTICE](crates/agentic-apply-patch/NOTICE)。
