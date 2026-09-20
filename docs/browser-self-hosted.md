# Self-hosted Browser backend

Agentic's `browser.*` tools run the official OpenAI Browser runtime through persistent `node_repl` kernels. On machines without ChatGPT/Codex Desktop, Agentic can use the official ChatGPT Chrome extension as the Browser backend through the optional `agentic-browser-host` Native Messaging bridge.

This path is intended for controlled self-hosted Linux deployments, including a persistent Chromium instance exposed through Neko for human takeover. It does **not** vendor or modify the proprietary OpenAI Browser runtime or Chrome extension.

> **Evidence status:** Source guarantees below are from this checkout. Read-only inspection of the current Neko deployment verified the running image, bind mounts, effective identities, shared socket inode, and absent ACL xattrs; an isolated primitive permission smoke also verified the exact production directory/socket modes. Historical ARM64 observations are recorded in `.planning/2026-09-browser-host-runtime/progress.md`; layouts and numeric values marked **example** are not production facts.

## Components

The direction that matters for a Neko deployment is:

```text
Neko / Chromium profile
  -> official ChatGPT extension
  -> Chrome Native Messaging (stdio + manifest origin allowlist)
  -> independent agentic-browser-host process
  -> shared Unix filesystem path /tmp/codex-browser-use
  -> Agent browser-client / NodeReplKernel
  -> BrowserRuntimeManager named lease (acquire -> repl -> reset/release/reaper)
```

The host creates a per-process socket such as `/tmp/codex-browser-use/agentic-browser-host-<pid>.sock`; the Browser client connects to that local Unix socket. The reverse view is useful when debugging, but does not change ownership:

```text
Agentic browser.* -> BrowserRuntimeManager lease
  -> NodeReplKernel -> browser-client/service
  -> /tmp/codex-browser-use/*.sock
  -> agentic-browser-host
  -> Native Messaging stdio
  -> official ChatGPT Chrome extension -> Chrome / Chromium
```

The extension and Native Messaging manifest are not an Agentic authentication system. `allowed_origins` constrains which extension may launch the host; the Unix socket's effective filesystem identity and namespace visibility constrain which local processes can reach it.

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

The Native Messaging host runs inside the Chromium container, while `agentic-gpt` normally runs on the Linux host. Both sides must see the same directory as `/tmp/codex-browser-use`; this is a filesystem/mount contract, not a remote socket bridge.

**Example layout only (not a present-deployment fact):**

```text
host persistent bridge: /srv/data/neko-browser/bridge
container mount:         /srv/data/neko-browser/bridge -> /tmp/codex-browser-use
host compatibility path: /tmp/codex-browser-use -> /srv/data/neko-browser/bridge
```

**Example Compose mounts only:**

```yaml
services:
  neko:
    volumes:
      - /srv/data/neko-browser/profile:/home/neko/.config/chromium
      - /srv/data/neko-browser/bridge:/tmp/codex-browser-use
      - /srv/apps/agentic-browser-host:/opt/agentic-browser-host:ro
```

The Chromium profile's Native Messaging manifest can point to `/opt/agentic-browser-host/native-host-launcher`. The launcher should export `AGENTIC_BROWSER_HOST_STANDALONE_COMPAT=1` and execute the release `agentic-browser-host` binary mounted into the container.

On the host, make `/tmp/codex-browser-use` resolve to the same persistent bridge directory. **Example only:** a systemd tmpfiles entry can recreate the path after reboot:

```text
L+ /tmp/codex-browser-use - - - - /srv/data/neko-browser/bridge
```

Install that line under `/etc/tmpfiles.d/agentic-browser.conf`, create the target bridge directory with permissions that allow the Chromium container user to create Unix sockets, and run `systemd-tmpfiles --create /etc/tmpfiles.d/agentic-browser.conf`. A bind mount to the same directory is also valid; do not expose the socket directory over the network.

### Identity, permission, and mount checks

Socket mode `0660` is **not authentication**. Reachability is decided by the effective UID, primary and supplementary GIDs, POSIX ACLs, execute permission on every parent directory, and whether the host/container namespaces expose the same inode. Do not infer these facts from the example paths or historical Neko record.

Run the following on the Agent host and inside the Neko/Chromium container, replacing placeholders; the values printed are the deployment facts to record:

```bash
# EXAMPLE commands; placeholders are intentional.
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

For a container runtime, an **example** identity check is `docker exec <neko-container> id` and `docker inspect <neko-container> --format '{{.Config.User}}'`; use the runtime actually deployed. Preserve a group-shared topology when that is the existing contract: an **example** bridge directory may use a stable shared group, setgid mode `2770`, and socket mode `0660`, with group execute/search permission on parents and an ACL only when required. Do not retrofit owner-only `0600`, silently chown one side, or add a remote/network bridge.

Current read-only deployment inspection (without publishing host address or ephemeral PID/inode numbers) verified image `ghcr.io/m1k1o/neko/chromium:latest`; profile `/srv/data/neko-browser/profile` is mounted at `/home/neko/.config/chromium`, bridge `/srv/data/neko-browser/bridge` is mounted read-write at `/tmp/codex-browser-use`, and `/srv/apps/neko-browser-host` is mounted read-only at `/opt/agentic-browser-host`. Chromium and Native host run as UID/GID `1000:1000`; the host compatibility path is a symlink to the bridge, whose directory is `1000:1000` mode `755`, and the per-process socket is `1000:1000` mode `660`. Host and both Agent namespaces saw the same socket device/inode; ACL xattrs were absent. Agent service contexts are root `0:0` with no supplementary groups. An isolated primitive smoke against those exact production modes allowed `1000:1000` and `1001:1000` access, rejected `1001:1001` with `EACCES`, and confirmed stdin close removes the caller's socket. This verifies Unix reachability and cleanup, not full browser tab operations or downstream side-effect rollback.

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

The ARM64 observation of OpenAI runtime `26.908.70816` is historical evidence, not a current deployment claim: that real extension backend exposed `tab.dom_cua`, `tab.cua`, and `tab.playwright`, while `tab.ax` was undefined even though bundled `accessibility.md` documented `tab.ax`. Prefer the APIs actually exposed by the selected runtime/backend and use `browser.manual` as the version-matched documentation source; if documentation and runtime disagree, report the mismatch rather than adding an Agentic compatibility API.

## Security boundary

`browser.repl` executes arbitrary JavaScript against a real browser and is intentionally marked destructive/open-world. Keep the browser host and socket local, restrict access to Agentic's MCP ingress, and do not expose the Native Messaging/socket bridge directly as a remote service. The host's `0660` socket mode and a shared mount provide filesystem reachability only; they do not authenticate a caller or prove that a downstream browser side effect can be rolled back.
