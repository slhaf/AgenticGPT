# Agentic Browser JavaScript SDK v1

`browser.repl` 提供 `browser` 和 `nodeRepl`，支持在代码中使用 `await`。要跨调用保留值，请赋给 `globalThis`。同一租约一次运行一段脚本。上一段还在执行时，新 `repl` 会返回 `browser_busy`。

## 标签页

| API | 行为 |
|---|---|
| `await browser.tabs.list()` | 列出 profile 中的标签页，包括 ID、标题、URL 和活动状态。 |
| `await browser.tabs.get(id)` | 按 Chrome 标签页 ID 获取 Tab 句柄。 |
| `await browser.tabs.new(url?)` | 新建标签页并返回 Tab 句柄。提供 URL 时等待导航完成。 |
| `tab.id` | 当前句柄的标签页 ID。 |
| `await tab.goto(url)` | 导航到指定地址并等待加载完成。 |
| `await tab.title()`、`await tab.url()` | 读取当前标题和地址。 |
| `await tab.close()` | 关闭这个标签页。 |

## 页面和元素

`await tab.domSnapshot()` 返回当前主文档的可读文本及元素引用。先获取快照，再从当前结果选择 `refId`。导航或页面更新后重新观察目标。标签关闭后，原句柄的操作会报错。

`tab.ref(refId)` 按快照引用定位。已知 CSS 选择器时使用 `tab.locator(selector)`。对返回的定位器调用 `await locator.click()`、`await locator.fill(value)` 或 `await locator.press(key)`。

`await tab.screenshot()` 返回带 `mimeType` 和 base64 `data` 的 PNG。用 `nodeRepl.emitImage(image)` 将它附在工具结果中。用 `nodeRepl.write(value)` 输出文本或 JSON。最后一次写出的 JSON 对象也会进入 `structuredContent`。

## 会话恢复

- `browser_busy` 表示这个浏览器已被占用。查看 `browser.list`，等待交接或选择另一个空闲 profile。
- `browser.reset` 为同一租约重建 JS 上下文。之后重新获取原标签页句柄。
- 连接断开或租约过期后，重新 `acquire`，从页面现状继续。等待超时时，已派发的网页动作仍可能生效，重试前先观察结果。
- `browser.release` 回收会话。使用 `tab.close()` 可明确关闭指定页面。
