# Browser Host Runtime — Findings

## Starting evidence

- Orange Pi + Neko Chromium already runs the official ChatGPT Chrome extension and has previously completed real Browser control through a small compatible Native Messaging/socket host PoC.
- Official Linux ARM64 ChatGPT packages include native ARM64 `node_repl`, `node`, native dependencies, and the current Browser SDK/service bundle; an earlier Orange Pi probe completed official ARM `node_repl -> setupBrowserRuntime() -> extension backend -> real page/AX` control.
- Browser Runtime Slices 1–16 now provide the public Agentic six-tool surface, persistent named kernels, official runtime documentation access, signed Linux APT acquisition, managed runtime materialization, and production source selection.
- The existing repository `experimental/chrome-control-poc/browser_bridge.py` is an older node_repl client PoC and is not the Orange Pi compatible Native Messaging host implementation; do not confuse the two roles.

## Historical compatibility questions to re-verify

- Reconfirmed on 2026-09-15: current Neko ChatGPT extension `1.26.901.11451` still reports `getInfo.agentRequestHeaderEnabled:false`, and current official Browser runtime `26.908.70816` uses that information in a path that attempts Codex app-server on the pure standalone host. Hiding only that optional field for the standalone host closes the app-server dependency without modifying the official extension.
- Reconfirmed on 2026-09-15: current official Browser runtime still exposes `BROWSER_USE_SECURITY_MODE`; the ARM64 standalone Agentic E2E uses the official `disabled-for-local-testing` mode scoped to that isolated Browser runtime process.

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

## 2026-09-15 ARM64 Agentic clean-room E2E findings

- From an empty isolated Browser cache, current Agentic signed-APT acquisition successfully provisioned the official Linux ARM64 ChatGPT package and materialized runtime `26.908.70816/prod`. This directly validates that the signed distribution work from Browser Runtime Slice 16 is portable to ARM64 rather than only x86_64.
- The full live chain succeeded after the two narrowly scoped standalone compatibility inputs were present: `Agentic browser.* -> official ARM64 node_repl / Browser SDK -> Rust compatible host -> official ChatGPT extension -> Neko Chromium`.
- `browser.acquire` alone succeeds even without the standalone shims because Browser discovery can reach `chrome/browserId=1`; the app-server failure appears when a real Browser operation such as `browser.tabs.new()` evaluates the current service policy path. This distinction is useful for diagnostics.
- Current Tab objects on this real extension backend expose `playwright`, `dom_cua`, `cua`, `content`, `clipboard`, `dev`, and `capabilities`, plus navigation/screenshot methods, but `tab.ax` is `undefined`. The selected runtime's bundled `accessibility.md` still instructs callers to use `tab.ax`, while `api.json` and `capabilities/tab/browserAuth.md` document the available `dom_cua` surface. `tab.dom_cua.get_visible_dom()` worked against Example Domain. Treat this as an upstream runtime-doc/backend-surface mismatch, not as an Agentic host translation defect.
- Screenshot fidelity is confirmed through the final public tool boundary: a real `Tab.screenshot()` produced 16,921 bytes, and `nodeRepl.emitImage` preserved it as `image/jpeg` through Agentic `browser.repl`.
- The E2E's host-namespace bind mount was only a test seam to expose Neko's bind-backed socket directory to the isolated host Agentic process. It was unmounted after the test. A public deployment still needs reproducible durable socket plumbing rather than relying on that manual test mount.
