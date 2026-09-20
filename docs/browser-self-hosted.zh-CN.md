# 自托管 Browser 后端

Agentic 的 `browser.*` 工具通过持久 `node_repl` kernel 使用 OpenAI 官方 Browser runtime。对于没有安装 ChatGPT/Codex Desktop 的 Linux 主机，可以使用可选的 `agentic-browser-host` Native Messaging bridge，把 OpenAI 官方 ChatGPT Chrome 扩展作为 Browser backend。

这条路径面向受控的自托管 Linux 环境，也支持用 Neko 常驻一只可人工接管的 Chromium。Agentic 不提交、复制或修改 OpenAI 的专有 Browser runtime 和 Chrome 扩展。

> **证据状态：**下面的 source guarantee 来自当前 checkout。对当前 Neko 部署的只读检查已确认运行镜像、bind mount、有效身份、共享 socket inode 与无 ACL xattr；隔离的 primitive permission smoke 也已按生产目录/socket mode 验证。ARM64 历史观察记录于 `.planning/2026-09-browser-host-runtime/progress.md`；标注为 **示例** 的布局和数字不是生产事实。

## 组件关系

Neko 部署中真正需要核对的方向是：

```text
Neko / Chromium profile
  -> 官方 ChatGPT 扩展
  -> Chrome Native Messaging（stdio + manifest origin allowlist）
  -> 独立的 agentic-browser-host 进程
  -> 共享 Unix filesystem path /tmp/codex-browser-use
  -> Agent browser-client / NodeReplKernel
  -> BrowserRuntimeManager 命名 lease（acquire -> repl -> reset/release/reaper）
```

host 为每个进程创建类似 `/tmp/codex-browser-use/agentic-browser-host-<pid>.sock` 的 socket；Browser client 通过本地 Unix socket 连接。调试时也可反向阅读，但不改变所有权：

```text
Agentic browser.* -> BrowserRuntimeManager lease
  -> NodeReplKernel -> browser-client/service
  -> /tmp/codex-browser-use/*.sock
  -> agentic-browser-host
  -> Native Messaging stdio
  -> 官方 ChatGPT Chrome 扩展 -> Chrome / Chromium
```

扩展和 Native Messaging manifest 不是 Agentic 的认证系统。`allowed_origins` 约束哪个扩展能启动 host；Unix socket 的有效 filesystem identity 与 namespace 可见性决定哪些本地进程能够到达它。

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

Native Messaging host 在 Chromium 容器内运行，而 `agentic-gpt` 通常运行在 Linux 宿主，因此两侧必须看到同一个 `/tmp/codex-browser-use` 目录；这是 filesystem/mount 合同，不是远程 socket bridge。

**仅示例布局（不是当前部署事实）：**

```text
宿主持久 bridge:       /srv/data/neko-browser/bridge
容器挂载:              /srv/data/neko-browser/bridge -> /tmp/codex-browser-use
宿主兼容路径:          /tmp/codex-browser-use -> /srv/data/neko-browser/bridge
```

**仅示例 Compose mount：**

```yaml
services:
  neko:
    volumes:
      - /srv/data/neko-browser/profile:/home/neko/.config/chromium
      - /srv/data/neko-browser/bridge:/tmp/codex-browser-use
      - /srv/apps/agentic-browser-host:/opt/agentic-browser-host:ro
```

Chromium profile 中的 Native Messaging manifest 可指向 `/opt/agentic-browser-host/native-host-launcher`。launcher 应导出 `AGENTIC_BROWSER_HOST_STANDALONE_COMPAT=1`，再执行挂入容器的 release `agentic-browser-host`。

宿主侧让 `/tmp/codex-browser-use` 指向同一持久 bridge 目录。**仅示例：**systemd tmpfiles 可在重启后恢复该路径：

```text
L+ /tmp/codex-browser-use - - - - /srv/data/neko-browser/bridge
```

把该行放入 `/etc/tmpfiles.d/agentic-browser.conf`，创建目标 bridge 目录并赋予 Chromium 容器用户创建 Unix socket 所需的权限，然后执行 `systemd-tmpfiles --create /etc/tmpfiles.d/agentic-browser.conf`。也可以把相同目录 bind mount 到宿主 `/tmp/codex-browser-use`；不要把 socket 目录暴露到网络。

### Identity、权限与 mount 检查

Socket mode `0660` **不是认证**。可达性由 effective UID、primary/supplementary GID、POSIX ACL、每一级父目录的 execute/search 权限，以及 host/container namespace 是否暴露同一个 inode 决定。不要从示例路径或历史 Neko 记录推断这些事实。

在 Agent 宿主和 Neko/Chromium 容器内分别运行以下命令，替换 placeholder；需要记录的是实际打印值：

```bash
# EXAMPLE 命令；placeholder 是有意保留的。
id
id -u; id -g; id -G
stat -c 'path=%n mode=%a uid=%u gid=%g type=%F' \
  /tmp/codex-browser-use \
  /tmp/codex-browser-use/agentic-browser-host-*.sock
namei -l /tmp/codex-browser-use
getfacl -p /tmp/codex-browser-use
findmnt -T /tmp/codex-browser-use -o TARGET,SOURCE,FSTYPE,OPTIONS
ss -xlpn
```

对于容器 runtime，**示例**身份检查是 `docker exec <neko-container> id` 与 `docker inspect <neko-container> --format '{{.Config.User}}'`；应使用实际部署的 runtime。在现有合同要求 group-shared 时保留它：**示例** bridge 目录可使用稳定共享组、setgid mode `2770` 和 socket mode `0660`，父目录保持 group execute/search，只有确有需要时才加 ACL。不要改成 owner-only `0600`、静默 chown 一侧或增加远程/network bridge。

当前部署的只读检查（不公开宿主地址或短暂的 PID/inode 数字）确认镜像为 `ghcr.io/m1k1o/neko/chromium:latest`；profile `/srv/data/neko-browser/profile` 挂载到 `/home/neko/.config/chromium`，bridge `/srv/data/neko-browser/bridge` 以 read-write 挂载到 `/tmp/codex-browser-use`，`/srv/apps/neko-browser-host` 以 read-only 挂载到 `/opt/agentic-browser-host`。Chromium 与 Native host 使用 UID/GID `1000:1000`；宿主兼容路径是指向 bridge 的 symlink，bridge 目录为 `1000:1000`、mode `755`，per-process socket 为 `1000:1000`、mode `660`。宿主与两个 Agent namespace 看到同一 socket device/inode；未发现 ACL xattr。Agent service context 为 root `0:0` 且无 supplementary group。对这些精确生产 mode 的 isolated primitive smoke 允许 `1000:1000` 与 `1001:1000`，以 `EACCES` 拒绝 `1001:1001`，并确认 stdin close 会移除调用方 socket。这证明 Unix 可达性与清理，不等于完整 browser tab 操作或下游副作用回滚。

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

ARM64 使用 OpenAI runtime `26.908.70816` 的观察是历史证据，不是当前部署声明：当时真实 extension backend 暴露 `tab.dom_cua`、`tab.cua` 与 `tab.playwright`，而 bundled `accessibility.md` 仍描述的 `tab.ax` 实际为 undefined。执行时应以当前 selected runtime/backend 真正暴露的 API 为准，并用 `browser.manual` 查看版本匹配文档；若文档和 runtime 不一致，应报告差异，不要在 Agentic 中另造一层兼容 Browser API。

## 安全边界

`browser.repl` 会针对真实浏览器执行任意 JavaScript，本身就被标记为 destructive/open-world。Browser host 与 Unix socket 应保持本地访问，限制 Agentic MCP ingress 的访问边界，不要把 Native Messaging/socket bridge 直接作为远程服务暴露。host 的 `0660` socket mode 与共享 mount 只提供 filesystem reachability，不认证调用方，也不能证明下游浏览器副作用可以回滚。
