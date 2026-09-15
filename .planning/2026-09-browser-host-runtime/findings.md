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

## Rust translation-pass findings

- The authoritative `native_host.py` used for this pass matched SHA256 `3f6c3c26ef556e69ddc9f89116b9f792c8c0e57f60049d2b2b333bf9fbf9ca98` and was read in full together with `probe.py` and `native-host-launcher` before implementation.
- Python `struct.pack/unpack("=I")` was translated as native-endian `u32` framing. On the supported Linux ARM64/x86_64 targets this is the required four-byte little-endian Native Messaging prefix.
- The baseline applies the 8 MiB bound while reading. It treats EOF during either a partial header or partial payload as stream closure (`None`), while zero-length, malformed JSON, and non-UTF-8 JSON payloads raise a parsing error. The Rust translation intentionally preserves those distinctions rather than introducing protocol error responses.
- Invalid client values or objects without `method`, unknown/stale extension response IDs, and extension values that are not objects are silently ignored by the baseline and remain silently ignored.
- No material observable ambiguity required a new protocol behavior. Broader frozen-plan robustness, deployment/Neko, installer, reconnection, and Browser SDK slices remain unimplemented in this pass.

## 2026-09-15 Rust-host live parity

- The Rust translation was cross-built for Linux ARM64 and substituted for the Python native host inside the existing Neko Chromium deployment without changing the official extension or the existing probe.
- The official ChatGPT extension started the Rust host through the existing `com.openai.codexextension` manifest/launcher path and maintained its periodic `ping` requests.
- The unchanged Python probe completed the same real extension control sequence as the Python-host baseline, including real BOSS-page `Runtime.evaluate` and screenshot capture. This is stronger evidence than the unit fixtures that the translation preserved the currently exercised observable bridge behavior.
- The extension still reports `agentRequestHeaderEnabled: false` after the Rust substitution; that field is therefore current extension behavior, not a Python-host artifact.
