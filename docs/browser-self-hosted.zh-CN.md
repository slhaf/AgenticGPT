# 自托管 Browser 后端

Agentic 的 `browser.*` 工具通过持久 `node_repl` kernel 使用 OpenAI 官方 Browser runtime。对于没有安装 ChatGPT/Codex Desktop 的 Linux 主机，可以使用可选的 `agentic-browser-host` Native Messaging bridge，把 OpenAI 官方 ChatGPT Chrome 扩展作为 Browser backend。

这条路径面向受控的自托管 Linux 环境，也支持用 Neko 常驻一只可人工接管的 Chromium。Agentic 不提交、复制或修改 OpenAI 的专有 Browser runtime 和 Chrome 扩展。

## 组件关系

```text
Agentic browser.*
  -> OpenAI 官方签名 Linux Browser runtime
  -> 官方 node_repl + Browser SDK
  -> /tmp/codex-browser-use/*.sock
  -> agentic-browser-host
  -> 官方 ChatGPT Chrome 扩展
  -> Chrome / Chromium
```

当前官方 ChatGPT 扩展 id：

```text
hehggadaopoacecdllhhajmbjkdcmajg
```

扩展应通过 Chrome/Chromium 正常的扩展分发机制安装，不要把解包扩展或 CRX 提交进 Agentic 仓库。

## 1. 安装 release binary

从对应架构的 release archive 安装 Agent 与可选 Browser host：

```bash
install -m 0755 agentic-gpt ~/.local/bin/
install -m 0755 agentic-browser-host ~/.local/bin/
```

release 同时构建 `x86_64-unknown-linux-gnu` 与 `aarch64-unknown-linux-gnu`。

## 2. 安装 Native Messaging launcher 与 manifest

自托管路径使用单独 launcher，使兼容设置只作用于这个 Native Messaging 进程：

```sh
#!/bin/sh
export AGENTIC_BROWSER_HOST_STANDALONE_COMPAT=1
exec "$HOME/.local/bin/agentic-browser-host" "$@"
```

给 launcher 加执行权限，并在 `com.openai.codexextension.json` 中使用它的**绝对路径**：

```json
{
  "name": "com.openai.codexextension",
  "description": "Agentic Browser native messaging host",
  "path": "/absolute/path/to/agentic-browser-host-launcher",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://hehggadaopoacecdllhhajmbjkdcmajg/"
  ]
}
```

普通同 namespace Linux Chrome/Chromium 可把 manifest 放进浏览器对应的用户 Native Messaging 目录。常见路径包括 `~/.config/chromium/NativeMessagingHosts/` 与 `~/.config/google-chrome/NativeMessagingHosts/`，应以实际浏览器安装为准。

host 会在 `/tmp/codex-browser-use` 下创建 backend socket。Chrome/Chromium 与 `agentic-gpt` 运行于同一 Linux namespace 时，不需要额外 socket bridge。

## 3. 启用 managed Browser runtime

Browser runtime 不需要从 ChatGPT Desktop 复制。Agentic 可以从 OpenAI 官方签名 APT 分发中获取当前 Linux runtime。空 cache 需要自动 provision 时，配置：

```json
{
  "browser": {
    "managed": {
      "enabled": true,
      "autoProvision": true
    }
  }
}
```

自托管扩展 backend 需要把官方 Browser runtime 的 local-testing security mode 只作用于该 Agentic worker：

```bash
BROWSER_USE_SECURITY_MODE=disabled-for-local-testing agentic-gpt run
```

不要把该变量全局导出给无关进程。Agentic 自身的 tool policy、confirmation、audit 与 Browser lease 仍然构成这条自托管链路的外层控制边界。

## Neko / 容器 Chromium

Neko 让浏览器保持可见、持久并可人工接管；`browser.acquire` 不负责启停 Neko。

Native host 在 Chromium 容器内运行，而 `agentic-gpt` 通常运行在 Linux 宿主，因此两侧必须看到同一个 `/tmp/codex-browser-use` 目录。

一种可复现布局：

```text
宿主持久 bridge:       /srv/data/neko-browser/bridge
容器挂载:              /srv/data/neko-browser/bridge -> /tmp/codex-browser-use
宿主兼容路径:          /tmp/codex-browser-use -> /srv/data/neko-browser/bridge
```

Compose 示例：

```yaml
services:
  neko:
    volumes:
      - /srv/data/neko-browser/profile:/home/neko/.config/chromium
      - /srv/data/neko-browser/bridge:/tmp/codex-browser-use
      - /srv/apps/agentic-browser-host:/opt/agentic-browser-host:ro
```

Chromium profile 中的 Native Messaging manifest 可指向 `/opt/agentic-browser-host/native-host-launcher`。launcher 应导出 `AGENTIC_BROWSER_HOST_STANDALONE_COMPAT=1`，再执行挂入容器的 release `agentic-browser-host`。

宿主侧让 `/tmp/codex-browser-use` 指向同一持久 bridge 目录。systemd 主机可用 tmpfiles 让它在重启后自动恢复：

```text
L+ /tmp/codex-browser-use - - - - /srv/data/neko-browser/bridge
```

把该行放入 `/etc/tmpfiles.d/agentic-browser.conf`，创建目标 bridge 目录并赋予 Chromium 容器用户创建 Unix socket 所需的权限，然后执行 `systemd-tmpfiles --create /etc/tmpfiles.d/agentic-browser.conf`。也可以把相同目录 bind mount 到宿主 `/tmp/codex-browser-use`；不要把 socket 目录暴露到网络。

官方扩展应通过 Chromium policy/update flow 安装，并使用持久 profile 保存扩展状态。

## Smoke test

先启动浏览器/扩展，再启动 Agentic。检查 runtime：

```bash
agentic-gpt local call browser.list --arguments '{}'
```

获取持久 lease：

```bash
agentic-gpt local call browser.acquire \
  --arguments '{"name":"smoke","idleTimeoutSeconds":300}'
```

随后通过 `browser.repl` 使用官方 Browser SDK。例如最小导航 smoke：

```js
globalThis.tab ??= await globalThis.browser.tabs.new();
await globalThis.tab.goto("https://example.com");
nodeRepl.write(JSON.stringify({
  title: await globalThis.tab.title(),
  url: await globalThis.tab.url()
}));
```

结束后释放 lease：

```bash
agentic-gpt local call browser.release --arguments '{"name":"smoke"}'
```

## 当前 runtime 注意点

ARM64 验收使用 OpenAI runtime `26.908.70816` 时，真实 extension backend 暴露 `tab.dom_cua`、`tab.cua` 与 `tab.playwright`，而 bundled `accessibility.md` 仍描述的 `tab.ax` 实际为 undefined。执行时应以当前 selected runtime/backend 真正暴露的 API 为准，并用 `browser.manual` 查看版本匹配文档；若文档和 runtime 不一致，应报告差异，不要在 Agentic 中另造一层兼容 Browser API。

## 安全边界

`browser.repl` 会针对真实浏览器执行任意 JavaScript，本身就被标记为 destructive/open-world。Browser host 与 Unix socket 应保持本地访问，限制 Agentic MCP ingress 的访问边界，不要把 Native Messaging/socket bridge 直接作为远程服务暴露。
