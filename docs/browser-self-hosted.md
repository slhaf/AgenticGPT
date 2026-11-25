# Self-hosted Agentic Browser

Install Agentic Browser in a Chrome or Chromium profile. The extension connects to Agentic and executes JavaScript for the existing `browser.*` tools. An Agentic instance can connect to multiple browsers, and each browser can pair with multiple instances. Each profile grants one execution lease at a time.

## Install and pair

1. Build the [Agentic Browser extension](https://github.com/slhaf/mcp-chrome-for-Agentic). In Chrome 116+ or Chromium, open `chrome://extensions`, enable developer mode, and load the build directory in the target profile.
2. Add `browser` to the existing Agentic `toolsets.enabled` list and configure `browser.pairingToken`. Use `openssl rand -hex 32` to generate a token for each Agentic instance.
3. Click **Agentic Browser** in the Chrome toolbar. In the popup, enter the browser name and add the address and token for each instance. The default address for Chrome and Agentic on the same host is `ws://127.0.0.1:39421`. Click **Save settings**.
4. Start Agentic. Check the connection status in the extension and use `browser.list` to find its `browserId`.

```json
{
  "browser": {
    "listenAddr": "127.0.0.1:39421",
    "pairingToken": "<replace with a random token>"
  }
}
```

Use a distinct listen port for each Agentic instance running on the same host.

Across hosts or containers, configure an address reachable from the browser. A TLS reverse proxy in front of the Agentic listener lets the extension use `wss://`, encrypting pairing credentials, scripts, and page results in transit. For Neko, load the extension in its persistent Chromium profile and keep that browser running.

To upgrade from the official Browser runtime, replace the old `browser` section with the listener and pairing configuration shown here. Install and verify the new extension, then remove the old Native Messaging registration, host mount, and shared socket from the deployment.

When changing the listen address, Agentic tries to bind the new address first. If the old and new addresses overlap on the same port, such as when switching from loopback to all interfaces, save the configuration and restart Agentic to apply the new address.

## Verify calls

```bash
agentic-gpt local call browser.manual --arguments '{"action":"read","path":"quickstart.md"}'
agentic-gpt local call browser.list --arguments '{}'
```

Substitute the advertised ID and use the same lease `name` for subsequent calls.

```bash
agentic-gpt local call browser.acquire \
  --arguments '{"browserId":"<browserId>","name":"smoke","idleTimeoutSeconds":60}'
agentic-gpt local call browser.repl \
  --arguments '{"name":"smoke","code":"globalThis.tab = await browser.tabs.new(); await tab.goto(\"https://example.com\"); nodeRepl.write(await tab.title());"}'
agentic-gpt local call browser.release --arguments '{"name":"smoke"}'
```

Agentic includes the manual. With the Browser toolset enabled, you can read it with the extension offline. Agentic checks the SDK API version when the extension connects.

An occupied browser returns `browser_busy` from `acquire`. Idle expiry, release, or a lost owning connection ends the JavaScript session. After acquiring a fresh lease, inspect the page before retrying an action dispatched before a timeout. Closing the browser disconnects it. The extension reloads saved pairings and reconnects when the profile starts again.
