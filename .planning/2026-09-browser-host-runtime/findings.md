# Browser Host Runtime — Findings

## Starting evidence

- Orange Pi + Neko Chromium already runs the official ChatGPT Chrome extension and has previously completed real Browser control through a small compatible Native Messaging/socket host PoC.
- Official Linux ARM64 ChatGPT packages include native ARM64 `node_repl`, `node`, native dependencies, and the current Browser SDK/service bundle; an earlier Orange Pi probe completed official ARM `node_repl -> setupBrowserRuntime() -> extension backend -> real page/AX` control.
- Browser Runtime Slices 1–16 now provide the public Agentic six-tool surface, persistent named kernels, official runtime documentation access, signed Linux APT acquisition, managed runtime materialization, and production source selection.
- The existing repository `experimental/chrome-control-poc/browser_bridge.py` is an older node_repl client PoC and is not the Orange Pi compatible Native Messaging host implementation; do not confuse the two roles.

## Historical compatibility questions to re-verify

- Earlier ARM64 standalone probing observed extension `getInfo.agentRequestHeaderEnabled:false` interacting with Browser service app-server policy lookup in a way that required a narrow compatibility workaround.
- Earlier ARM64 standalone probing used the official `BROWSER_USE_SECURITY_MODE=disabled-for-local-testing` path outside Desktop/Codex.
- These are historical observations only until reconfirmed against the current official ARM64 runtime and current ChatGPT extension during Slice 01.

## Public-project constraint

Agentic is publicly released on GitHub. Machine-local `/srv/...` scripts, hand-copied manifests, or undocumented existing services can be used as test evidence but cannot be the accepted installation contract.
