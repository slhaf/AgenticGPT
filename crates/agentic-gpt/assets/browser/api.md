# Agentic Browser JavaScript SDK v1

`browser.repl` 提供 `browser` 和 `nodeRepl`，支持在代码中使用 `await`。要跨调用保留值，请赋给 `globalThis`。同一租约一次运行一段脚本。上一段还在执行时，新 `repl` 会返回 `browser_busy`。

先通过 `browser.list` 查看 `extensionVersion`。扩展 0.2.0 起提供本页的导航等待、frame、语义定位、页面求值和截图选项。选择 0.1.0 浏览器时可使用基础标签页操作、DOM 快照、CSS/引用定位、点击填写按键和视口截图。

## 标签页和导航

| API | 行为 |
|---|---|
| `await browser.tabs.list()` | 列出 profile 中的标签页，包括 ID、标题、URL 和活动状态。 |
| `await browser.tabs.selected()` | 返回最近获得焦点的窗口中活动标签页的句柄。没有活动标签页时返回 `null`。 |
| `await browser.tabs.get(id)` | 按 Chrome 标签页 ID 获取 Tab 句柄。 |
| `await browser.tabs.new(url?)` | 新建标签页并返回 Tab 句柄。提供 URL 时等待导航完成。 |
| `tab.id` | 当前句柄的标签页 ID。 |
| `await tab.goto(url)`、`await tab.reload()`、`await tab.back()`、`await tab.forward()` | 发起导航并等待加载完成或同文档跳转。没有对应历史记录时，前进或后退会报错。 |
| `await tab.title()`、`await tab.url()` | 读取当前标题和地址。 |
| `await tab.waitForURL(url, {timeoutMs}?)` | 等待标签页提交指定绝对 URL。按规范化后的完整地址匹配。 |
| `await tab.waitForLoadState({timeoutMs}?)` | 等待当前标签页完成加载。 |
| `await tab.close()` | 关闭这个标签页。 |

等待的默认时限为 10,000 毫秒，可以传入非负整数 `timeoutMs`。等待超时返回 `browser_wait_timeout`。REPL 本身的总时限适用于整段脚本。

## 文档、iframe 和定位器

`await tab.domSnapshot()` 返回当前主文档的可读文本和元素引用。使用 `tab.ref(refId)` 定位快照中的元素；导航或页面更新后重新获取快照和引用。

`await tab.frames.list()` 返回活动文档的 `frameId`、`parentFrameId`、`documentId` 和 `url`。主文档的 `parentFrameId` 是 `-1`。用 `await tab.frames.get(documentId)` 获取指定文档句柄。frame 导航后，旧句柄会失效，需要重新列出文档。

Tab 和 Frame 都提供 `domSnapshot()`、`ref(refId)`、`locator(css)`、`getByRole(role, {name}?)`、`getByText(text)` 和 `getByLabel(label)`。语义匹配压缩空白后精确比较。定位器的单元素操作要求唯一匹配，可以用 `nth(index)` 选择零起始的匹配位置。

| 定位器操作 | 行为 |
|---|---|
| `click()`、`fill(value)`、`press(key)` | 点击、填写或按键。 |
| `count()`、`textContent()`、`innerText()`、`getAttribute(name)` | 查询匹配数、文本和属性。属性缺失时返回 `null`。 |
| `isVisible()`、`isEnabled()` | 判断渲染可见及是否启用。没有匹配元素时返回 `false`。 |
| `waitFor({state, timeoutMs}?)` | 等待指定状态。`visible` 是默认状态。也可选择 `attached`、`detached`、`hidden` 或 `actionable`（可见且启用）。 |
| `setChecked(checked)`、`selectOption(value)` | 设置可见且启用的原生复选框、单选框或下拉框。`selectOption` 接受选项值或值数组，返回选中的值。 |

`await tab.evaluate(fn, arg?)` 和 `await frame.evaluate(fn, arg?)` 在指定 HTTP(S) 文档的页面上下文执行函数。通过 `arg` 传入可序列化值，函数可以返回 JSON 值或其 Promise。`undefined` 返回为 `null`。通过独立进程运行的跨站 iframe 求值使用 Chrome 125 起提供的调试子会话。

## 截图和坐标操作

- `await tab.screenshot()` 截取当前视口。传入 `{fullPage: true}` 截取全页，传入 `{clip: {x, y, width, height}}` 截取文档区域。区域参数使用以文档原点为基准的 CSS 像素。
- `await locator.screenshot()` 截取主文档中可见元素的边界。可以定位 iframe 元素本身并截取其显示区域。
- `await tab.clickAt(x, y)` 左键点击视口坐标。`await tab.scrollAt(x, y, deltaX, deltaY)` 在指定坐标发送滚轮事件。位置和滚动增量都使用 CSS 像素。

截图返回 `{mimeType:"image/png", data}`，`data` 是 Base64。用 `nodeRepl.emitImage(image)` 返回原生 MCP 图片。也可以传入已有的 PNG、JPEG、WebP 或 GIF 的 MIME 类型和 Base64。结果超过 16 MiB WebSocket 消息上限时，会返回 `browser_result_too_large`。

用 `nodeRepl.write(value)` 输出文本或 JSON。最后一次写出的 JSON 对象也会进入 `structuredContent`。

## 会话恢复

- `browser_busy` 表示这个浏览器已被占用。查看 `browser.list`，等待交接或选择另一个空闲 profile。
- `browser.reset` 为同一租约重建 JS 上下文。之后重新获取原标签页句柄。
- 连接断开或租约过期后，重新 `acquire`，从页面现状继续。等待超时时，已派发的网页动作仍可能生效，重试前先观察结果。
- `browser.release` 回收会话。使用 `tab.close()` 可明确关闭指定页面。
