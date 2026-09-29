# 自托管 Browser 后端

Agentic 的 `browser.*` 工具通过持久 `node_repl` kernel 使用 OpenAI 官方 Browser runtime。对于没有安装 ChatGPT/Codex Desktop 的 Linux 主机，可以使用可选的 `agentic-browser-host` Native Messaging bridge，把 OpenAI 官方 ChatGPT Chrome 扩展作为 Browser backend。

Standalone、Local 和已连接 Hub 的 Agent 运行时、工具表面及配置行为见[Standalone 与本地 MCP 运行时参考](standalone-runtime.md)。

这条路径面向受控的自托管 Linux 环境，也支持用 Neko 常驻一只可人工接管的 Chromium。Agentic 不提交、复制或修改 OpenAI 的专有 Browser runtime 和 Chrome 扩展。

> **证据状态：**下文标注为代码行为的内容来自仓库实现。Neko 镜像、绑定挂载、有效身份、共享 socket inode、ACL 以及隔离权限冒烟属于既有部署记录；本轮文档重建未访问该部署或重新运行验证，使用前须在实际环境核对。ARM64 的历史观察记录见 `.planning/2026-09-browser-host-runtime/progress.md`；标为**示例**的布局和数值不代表生产事实。

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

## 1. 安装发布版二进制文件

从对应架构的发布归档中安装 Agent 与可选的 Browser host：

```bash
install -m 0755 agentic-gpt ~/.local/bin/
install -m 0755 agentic-browser-host ~/.local/bin/
```

发布流程同时构建 `x86_64-unknown-linux-gnu` 与 `aarch64-unknown-linux-gnu`。

## 2. 安装 Native Messaging 启动器与 manifest 清单

自托管路径使用单独的启动器，使兼容设置只作用于这个 Native Messaging 进程：

```sh
#!/bin/sh
export AGENTIC_BROWSER_HOST_STANDALONE_COMPAT=1
exec "$HOME/.local/bin/agentic-browser-host" "$@"
```

给启动器添加执行权限，并在 `com.openai.codexextension.json` 中使用它的**绝对路径**：

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

普通同一 Linux 命名空间中的 Chrome/Chromium 可将 manifest 安装到对应浏览器的用户级 Native Messaging host 目录。常见路径包括 `~/.config/chromium/NativeMessagingHosts/` 与 `~/.config/google-chrome/NativeMessagingHosts/`；应以实际控制的浏览器安装为准。

host 会在 `/tmp/codex-browser-use` 下创建后端 socket。Chrome/Chromium 与 `agentic-gpt` 运行于同一 Linux 命名空间时，不需要额外的 socket bridge。

## 3. 启用托管 Browser 运行时

Browser 运行时无需从 ChatGPT Desktop 复制。Agentic 可以从 OpenAI 官方签名 APT 分发中获取当前 Linux 运行时。若需在缓存为空时自动配置，请设置：

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

自托管扩展后端需要将官方 Browser 运行时的 local-testing 安全模式仅作用于该 Agentic 工作进程：

```bash
BROWSER_USE_SECURITY_MODE=disabled-for-local-testing agentic-gpt run
```

不要将该变量全局导出给无关进程。Agentic 自身的 tool policy、confirmation、audit 与 Browser lease 仍然构成这条自托管链路的外层控制边界。

## Neko / 容器化 Chromium

Neko 让浏览器保持可见、持久并可由人工接管；`browser.acquire` 不负责启动或停止 Neko。

Native Messaging host 在 Chromium 容器内运行，而 `agentic-gpt` 通常运行于 Linux 宿主，因此两侧必须看到相同的 `/tmp/codex-browser-use` 目录；这是文件系统/挂载合同，不是远程 socket bridge。

**仅示例布局（不是当前部署事实）：**

```text
宿主持久 bridge:       /srv/data/neko-browser/bridge
容器挂载:              /srv/data/neko-browser/bridge -> /tmp/codex-browser-use
宿主兼容路径:          /tmp/codex-browser-use -> /srv/data/neko-browser/bridge
```

**仅示例 Compose 挂载：**

```yaml
services:
  neko:
    volumes:
      - /srv/data/neko-browser/profile:/home/neko/.config/chromium
      - /srv/data/neko-browser/bridge:/tmp/codex-browser-use
      - /srv/apps/agentic-browser-host:/opt/agentic-browser-host:ro
```

Chromium profile 中的 Native Messaging manifest 可指向 `/opt/agentic-browser-host/native-host-launcher`。启动器应导出 `AGENTIC_BROWSER_HOST_STANDALONE_COMPAT=1`，然后执行已挂载到容器中的 release `agentic-browser-host` 二进制文件。

宿主侧应让 `/tmp/codex-browser-use` 指向同一个持久 bridge 目录。**仅示例：**systemd tmpfiles 可以在重启后恢复该路径：

```text
L+ /tmp/codex-browser-use - - - - /srv/data/neko-browser/bridge
```

将该行放入 `/etc/tmpfiles.d/agentic-browser.conf`，创建目标 bridge 目录，并赋予 Chromium 容器用户创建 Unix socket 所需的权限；然后执行 `systemd-tmpfiles --create /etc/tmpfiles.d/agentic-browser.conf`。也可将同一目录 bind mount 到宿主 `/tmp/codex-browser-use`；不要将 socket 目录暴露到网络。

### 身份、权限与挂载检查

Socket 权限模式 `0660` **不是认证**。可达性取决于有效 UID、主 GID/补充 GID、POSIX ACL、每一级父目录的执行/搜索权限，以及宿主/容器命名空间是否暴露同一个 inode。不要从示例路径或历史 Neko 记录推断这些事实。

在 Agent 宿主和 Neko/Chromium 容器内分别运行以下命令并替换占位符；应记录实际输出值：

```bash
# 示例命令；占位符有意保留。
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

对于容器运行时，**示例**身份检查包括 `docker exec <neko-container> id` 和 `docker inspect <neko-container> --format '{{.Config.User}}'`；应使用实际部署的运行时。现有合同要求共享组访问时，应保留这一拓扑：**示例**bridge 目录可使用稳定共享组、setgid 权限模式 `2770` 和 socket 权限模式 `0660`，父目录保持组执行/搜索权限，仅在确有需要时添加 ACL。不要改为仅所有者可访问的 `0600`，不要静默 chown 任一侧，也不要增加远程/网络 bridge。

当前部署的只读检查（不公开宿主地址或短暂的 PID/inode 数字）确认运行镜像为 `ghcr.io/m1k1o/neko/chromium:latest`；profile `/srv/data/neko-browser/profile` 挂载到 `/home/neko/.config/chromium`，bridge `/srv/data/neko-browser/bridge` 以读写方式挂载到 `/tmp/codex-browser-use`，`/srv/apps/neko-browser-host` 以只读方式挂载到 `/opt/agentic-browser-host`。Chromium 与 Native host 使用 UID/GID `1000:1000`；宿主兼容路径是指向 bridge 的符号链接，bridge 目录为 `1000:1000`、权限模式 `755`，每进程 socket 为 `1000:1000`、权限模式 `660`。宿主和两个 Agent 命名空间看到同一 socket device/inode；未发现 ACL xattr。Agent service 上下文为 root `0:0`，且没有补充组。针对这些精确的生产权限模式所做的隔离基础权限冒烟检查允许 `1000:1000` 与 `1001:1000` 访问，以 `EACCES` 拒绝 `1001:1001`，并确认关闭 stdin 会移除调用方 socket。这验证的是 Unix 可达性与清理，不代表已验证完整浏览器标签页操作或下游副作用回滚。

官方扩展应通过 Chromium 的策略/更新流程安装，并使用持久 profile 保存扩展状态。

## 冒烟测试

先启动浏览器/扩展，再启动 Agentic。检查运行时发现状态：

```bash
agentic-gpt local call browser.list --arguments '{}'
```

获取持久租约：

```bash
agentic-gpt local call browser.acquire \
  --arguments '{"name":"smoke","idleTimeoutSeconds":300}'
```

随后通过 `browser.repl` 使用官方 Browser SDK。例如最小导航冒烟测试：

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

## 当前运行时注意事项

ARM64 上使用 OpenAI 运行时 `26.908.70816` 的观察属于历史证据，不代表当前部署：当时实际的扩展后端公开了 `tab.dom_cua`、`tab.cua` 和 `tab.playwright`，而打包的 `accessibility.md` 仍描述 `tab.ax`，但该 API 实际未定义。执行时应以当前选用的运行时/后端实际公开的 API 为准，并用 `browser.manual` 查看与版本匹配的文档；如果文档和运行时不一致，应报告差异，不要在 Agentic 中另造兼容 Browser API。

## 安全边界

`browser.repl` 会针对真实浏览器执行任意 JavaScript，并有意标记为 destructive/open-world。浏览器 host 与 Unix socket 应保持本地访问，限制 Agentic MCP 接入入口的访问范围，不要把 Native Messaging/socket bridge 直接暴露为远程服务。host 的 `0660` socket 权限模式与共享挂载只提供文件系统可达性，不会认证调用方，也不能证明下游浏览器副作用可以回滚。
