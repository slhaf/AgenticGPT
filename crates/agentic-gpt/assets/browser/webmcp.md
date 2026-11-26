# WebMCP
Check `await tab.capabilities.list()` for `webmcp`. The capability is available when the browser exposes WebMCP and the page has registered tools. Prefer WebMCP when one covers the requested action.

```js
const webmcp = await tab.capabilities.get("webmcp");
let tools = await webmcp.fetchTools();
nodeRepl.write(tools.description());
```

Use the returned tools object. In the next REPL call, replace `"tool_name"` with the exact name from the description and `input` with an object matching its schema. Use `nodeRepl.write` to return the result.

```js
nodeRepl.write(await tools.call("tool_name", input));
```

Before the first WebMCP call on each tab in a lease, call `webmcp.fetchTools()` and inspect the available tools. Reuse handles during that lease. Fetch again after navigation, a change in the registered tools, or a stale or invalid handle error. Replace the old tools object with the returned one and print `tools.description()` again. Call only listed tools.
