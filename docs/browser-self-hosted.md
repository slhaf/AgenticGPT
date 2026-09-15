# Self-hosted Browser backend

Agentic's `browser.*` tools run the official OpenAI Browser runtime through persistent `node_repl` kernels. On machines without ChatGPT/Codex Desktop, Agentic can use the official ChatGPT Chrome extension as the Browser backend through the optional `agentic-browser-host` Native Messaging bridge.

This path is intended for controlled self-hosted Linux deployments, including a persistent Chromium instance exposed through Neko for human takeover. It does **not** vendor or modify the proprietary OpenAI Browser runtime or Chrome extension.

## Components

```text
Agentic browser.*
  -> signed managed OpenAI Linux Browser runtime
  -> official node_repl + Browser SDK
  -> /tmp/codex-browser-use/*.sock
  -> agentic-browser-host
  -> official ChatGPT Chrome extension
  -> Chrome / Chromium
```

The official ChatGPT extension id used by the current backend is:

```text
hehggadaopoacecdllhhajmbjkdcmajg
```

The extension is installed through Chrome/Chromium's normal extension distribution mechanism. Do not copy an unpacked extension or CRX into the Agentic repository.

## 1. Install the release binaries

Use a release archive for the target architecture and install `agentic-gpt` plus the optional Browser host:

```bash
install -m 0755 agentic-gpt ~/.local/bin/
install -m 0755 agentic-browser-host ~/.local/bin/
```

The release workflow publishes both `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` builds.

## 2. Install a Native Messaging launcher and manifest

The self-hosted path uses an explicit launcher so standalone compatibility is scoped to this Chrome Native Messaging process rather than changing the default host behavior:

```sh
#!/bin/sh
export AGENTIC_BROWSER_HOST_STANDALONE_COMPAT=1
exec "$HOME/.local/bin/agentic-browser-host" "$@"
```

Make it executable and reference its **absolute path** from `com.openai.codexextension.json`:

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

For ordinary same-namespace Linux Chrome/Chromium, install the manifest in the browser's user Native Messaging host directory. Common locations include `~/.config/chromium/NativeMessagingHosts/` and `~/.config/google-chrome/NativeMessagingHosts/`; use the directory for the browser installation actually being controlled.

The host creates its backend socket under `/tmp/codex-browser-use`. When Chrome/Chromium and `agentic-gpt` run in the same Linux namespace, no extra socket bridge is required.

## 3. Enable managed Browser runtime acquisition

The Browser runtime itself is not copied from ChatGPT Desktop. Agentic can acquire the current official Linux runtime from OpenAI's signed APT distribution. In config, enable managed runtime acquisition and opt into provisioning when the cache is empty:

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

For the standalone extension backend, start Agentic with the official Browser runtime's local-testing security mode scoped to this worker:

```bash
BROWSER_USE_SECURITY_MODE=disabled-for-local-testing agentic-gpt run
```

Do not export that setting globally for unrelated processes. Agentic's own tool policy, confirmation, audit, and Browser lease boundaries remain the outer control layer for this self-hosted deployment.

## Neko / containerized Chromium

Neko keeps the browser visible and persistent while Agentic attaches through the same official extension backend. `browser.acquire` does not own the Neko lifecycle.

The native host runs inside the Chromium container, while `agentic-gpt` normally runs on the Linux host. Both sides must therefore see the same directory as `/tmp/codex-browser-use`.

One reproducible layout is:

```text
host persistent bridge: /srv/data/neko-browser/bridge
container mount:         /srv/data/neko-browser/bridge -> /tmp/codex-browser-use
host compatibility path: /tmp/codex-browser-use -> /srv/data/neko-browser/bridge
```

Example Compose mounts:

```yaml
services:
  neko:
    volumes:
      - /srv/data/neko-browser/profile:/home/neko/.config/chromium
      - /srv/data/neko-browser/bridge:/tmp/codex-browser-use
      - /srv/apps/agentic-browser-host:/opt/agentic-browser-host:ro
```

The Chromium profile's Native Messaging manifest can point to `/opt/agentic-browser-host/native-host-launcher`. The launcher should export `AGENTIC_BROWSER_HOST_STANDALONE_COMPAT=1` and execute the release `agentic-browser-host` binary mounted into the container.

On the host, make `/tmp/codex-browser-use` resolve to the same persistent bridge directory. For systemd hosts this can be made reboot-safe with a tmpfiles entry such as:

```text
L+ /tmp/codex-browser-use - - - - /srv/data/neko-browser/bridge
```

Install that line under `/etc/tmpfiles.d/agentic-browser.conf`, create the target bridge directory with permissions that allow the Chromium container user to create Unix sockets, and run `systemd-tmpfiles --create /etc/tmpfiles.d/agentic-browser.conf`. A bind mount to the same directory is also valid; do not expose the socket directory over the network.

Install the official extension through Chromium policy/update flow, for example with the current extension id and Google's normal update service. Keep the browser profile persistent so extension state survives container restarts.

## Smoke test

Start the browser/extension first, then Agentic. Check runtime discovery:

```bash
agentic-gpt local call browser.list --arguments '{}'
```

Acquire one persistent lease:

```bash
agentic-gpt local call browser.acquire \
  --arguments '{"name":"smoke","idleTimeoutSeconds":300}'
```

Then exercise the official Browser SDK through `browser.repl`. A minimal navigation smoke is:

```js
globalThis.tab ??= await globalThis.browser.tabs.new();
await globalThis.tab.goto("https://example.com");
nodeRepl.write(JSON.stringify({
  title: await globalThis.tab.title(),
  url: await globalThis.tab.url()
}));
```

Always release the lease when the smoke is finished:

```bash
agentic-gpt local call browser.release --arguments '{"name":"smoke"}'
```

## Current runtime note

During the ARM64 acceptance of OpenAI runtime `26.908.70816`, the real extension backend exposed `tab.dom_cua`, `tab.cua`, and `tab.playwright`, while `tab.ax` was undefined even though the bundled `accessibility.md` still documented `tab.ax`. Prefer the APIs actually exposed by the selected runtime/backend and use `browser.manual` as the version-matched documentation source; if documentation and runtime disagree, treat the live runtime surface as authoritative for execution and report the mismatch rather than adding an Agentic compatibility API.

## Security boundary

`browser.repl` executes arbitrary JavaScript against a real browser and is intentionally marked destructive/open-world. Keep the browser host and socket local, restrict access to Agentic's MCP ingress, and do not expose the Native Messaging/socket bridge directly as a remote service.
