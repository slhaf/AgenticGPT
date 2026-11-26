# 连接与租约排障

调用 `browser.list` 查看当前 Agentic 的监听状态、已配对浏览器的连接和占用情况，以及本实例持有的租约。用 `browsers` 中目标 profile 的 `browserId`、`connected` 和 `busy` 判断下一步。

| 看到的状态 | 下一步 |
| --- | --- |
| 工具列表没有 `browser.list` | 在目标 Agentic 的 `toolsets.enabled` 中启用 `browser`。 |
| `listenerReady: false` | 检查 Agentic 的 `browser.pairingToken` 是否已配置，`browser.listenAddr` 是否能在 Agentic 所在机器上绑定。配置的网络地址稍后就绪时，Agentic 会自动重试监听。 |
| `listenerReady: true`，但目标 profile 缺失或 `connected: false` | 在**目标 Chrome profile** 打开 Agentic Browser 工具栏弹窗，查看这个 Agentic 连接的状态和错误。核对 WebSocket 地址和配对令牌，修改后保存。跨机器连接要使用浏览器可达的 Agentic 地址。连接恢复后重新调用 `browser.list`。 |
| 目标 profile 的 `busy: true` 或申请时返回 `browser_busy` | 查看本实例的 `leases`。如果是同一任务，沿用已有租约名；如果是其他任务或其他 Agentic 占用，请等持有方释放。用户指定了 profile 时，继续使用该 profile。 |
| `browser_disconnected`、`browser_lease_expired` 或 `browser_not_connected` | 等目标 profile 重新在线并空闲，用原名调用 `browser.acquire`，重新取得标签页句柄。根据实际页面状态接续任务。 |

`runtimeAvailable` 表示至少有一个扩展在线。一个 Agentic 配对了多个浏览器时，即使它为 `true`，目标 profile 仍可能离线。

如果只有标签页引用失效或受控标签页列表为空，按[页面交互排障](browser-troubleshooting.md)处理。浏览器仍在线时，`browser.reset` 可在同一租约内重建 JavaScript 会话；原来的变量和句柄需要重新取得。操作超时后，先观察页面上已经发生的变化，再决定是否重试。
