# Chrome Control PoC

Disposable experiment for reusing the OpenAI bundled Chrome control runtime outside Codex.
It is intentionally not wired into AgenticGPT production runtime.

The bridge dynamically reads `~/.local/state/openai-codex/chrome-native-hosts-v2.json`, reuses the `node_repl` environment from `~/.codex/config.toml`, starts the standalone stdio MCP `node_repl`, bootstraps `browser-client.mjs`, and keeps the same JavaScript kernel alive across calls.

```bash
python3 experimental/chrome-control-poc/browser_bridge.py connect
python3 experimental/chrome-control-poc/browser_bridge.py exec 'return await chrome.tabs.list();'
python3 experimental/chrome-control-poc/browser_bridge.py status
python3 experimental/chrome-control-poc/browser_bridge.py stop
```

Use `globalThis` for handles that should survive multiple `exec` calls, for example:

```js
globalThis.tab ??= await chrome.tabs.new();
await tab.goto("https://example.com");
return { title: await tab.title(), url: await tab.url() };
```

The daemon socket and log live under `/tmp/agentic-chrome-control-poc-$UID.*`.
