# Agentic Browser 自托管

Agentic Browser 是安装在 Chrome 或 Chromium profile 内的扩展。它主动连接 Agentic，通过现有 `browser.*` 工具执行脚本并操作该 profile 的网页。一个 Agentic 可以连接多个浏览器，浏览器也可以配对多个 Agentic。扩展为整个 profile 一次分配一个执行租约。

## 安装和配对

1. 从 [Agentic Browser 源码](https://github.com/slhaf/mcp-chrome-for-Agentic) 构建扩展。在目标 Chrome 116+ 或 Chromium profile 的 `chrome://extensions` 开发者模式中加载构建目录。
2. 在现有 Agentic 配置的 `toolsets.enabled` 中加入 `browser`，设置 `browser.pairingToken`。用 `openssl rand -hex 32` 生成每个 Agentic 实例的配对密钥。
3. 点击 Chrome 工具栏中的 **Agentic Browser**，在弹窗内填写浏览器名称，并为每个实例添加地址及其配对令牌。Agentic 与 Chromium 同机时，默认地址为 `ws://127.0.0.1:39421`。点击**保存设置**。
4. 启动 Agentic，确认扩展显示已连接，并通过 `browser.list` 获取对应的 `browserId`。

```json
{
  "browser": {
    "listenAddr": "127.0.0.1:39421",
    "pairingToken": "<替换为随机密钥>"
  }
}
```

同一台机器运行多个 Agentic 时，为每个实例配置不同的监听端口。

跨宿主或容器连接时，扩展应使用实际能访问的 Agentic 地址。可在 Agentic 监听器前部署 TLS 反向代理，在扩展中填写 `wss://` 地址，以加密传输配对凭据、脚本和网页结果。运行 Neko 时，在其持久 Chromium profile 中载入扩展，保持容器内的浏览器运行即可。

从官方 Browser runtime 升级时，将原 `browser` 配置替换为上述监听和配对字段，安装新扩展并确认可用。随后可移除部署中的旧 Native Messaging 注册、host 挂载和共享 socket。

变更监听地址时，Agentic 会先尝试绑定新地址。若新旧地址占用同一端口，例如从本机地址改为所有地址，配置保存后重启 Agentic 即可应用新监听。

## 验证调用

```bash
agentic-gpt local call browser.manual --arguments '{"action":"read","path":"quickstart.md"}'
agentic-gpt local call browser.list --arguments '{}'
```

把列表中的 ID 填入以下调用。使用相同的 `name` 继续执行脚本。

```bash
agentic-gpt local call browser.acquire \
  --arguments '{"browserId":"<browserId>","name":"smoke","idleTimeoutSeconds":60}'
agentic-gpt local call browser.repl \
  --arguments '{"name":"smoke","code":"globalThis.tab = await browser.tabs.new(); await tab.goto(\"https://example.com\"); nodeRepl.write(await tab.title());"}'
agentic-gpt local call browser.release --arguments '{"name":"smoke"}'
```

手册随 Agentic 提供，Browser 工具集启用后即可离线查阅。扩展连接后，Agentic 使用 SDK API 版本判断是否可用。

浏览器已被占用时，`acquire` 返回 `browser_busy`。空闲到期、主动释放或持有连接断开都会结束 JS 会话。重新取得租约后先观察页面现状，超时前派发的动作可能已经生效。浏览器关闭时连接断开，重新启动 profile 后扩展使用保存的配对信息重连。
