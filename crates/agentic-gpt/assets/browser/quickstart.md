# 使用 Agentic Browser

1. 调用 `browser.list`，选择已连接且空闲的 `browserId`。
2. 调用 `browser.acquire`，传入该 ID、任务使用的 `name` 和 `idleTimeoutSeconds`。
3. 多次调用同名 `browser.repl`。先观察网页，再根据返回的内容定位并操作。
4. 完成后调用同名 `browser.release`，把浏览器交还给其他调用方。

第一次 `browser.repl` 可以打开页面并读出快照。

```js
globalThis.tab = await browser.tabs.new();
await tab.goto("https://example.com");
nodeRepl.write(await tab.domSnapshot());
```

第二次使用同一个租约名，可以直接操作上次的 `tab` 并返回图像。

```js
nodeRepl.write({ title: await tab.title(), url: await tab.url() });
nodeRepl.emitImage(await tab.screenshot());
```

要操作已有标签页，先用 `nodeRepl.write(await browser.tabs.list())` 查看 ID，再用 `browser.tabs.get(id)` 获取句柄。通过 `browser.manual` 读取 `api.md` 了解定位、输入和会话错误。
