# Browser Host Runtime — Progress

Status: Rust host translation, Linux ARM64 clean-room Agentic Browser E2E, release packaging, and reproducible self-hosted/Neko deployment guidance accepted.

## Translation pass

- Added the independent `agentic-browser-host` Rust crate and workspace member with a same-named binary.
- Translated the verified Orange Pi Python host's Native Messaging framing, including native `u32` length headers, the 8 MiB inbound limit, compact JSON output, and serialized stdout writes.
- Added `/tmp/codex-browser-use` socket/log behavior with per-process `agentic-browser-host-<pid>.sock`, mode `0660`, append-only `agentic-browser-host.log`, concurrent Unix-socket clients, and normal-exit socket removal.
- Added client numbering, pending request correlation, `agentic-browser-host:<client_id>:<uuid>` outbound ID rewriting, response ID restoration, and pending-route removal when clients disconnect.
- Preserved local `bridge.getStatus`, client-to-extension notifications, extension-to-client notification broadcast, and baseline extension request responses for `ping`, app-server ensure/restart methods, `codexRuntime/hello`, and unknown methods.
- Did not change existing `browser.*` behavior or add Browser SDK semantics, HTTP/services, installation, deployment, configuration UI, or Neko work.

## Tests run

Passed focused `cargo test -p agentic-browser-host <filter>` runs for all nine unit tests covering frame round-trip encoding/decoding, oversized frames, truncated frames, zero/malformed/non-UTF-8 payloads, multi-client ID rewrite/restore with out-of-order responses, local status, extension request handlers, bidirectional notification forwarding/broadcast, and disconnected-client route cleanup.

Passed final `cargo test -p agentic-browser-host` (9 tests), `cargo fmt --all -- --check`, `cargo check --workspace`, and `git diff --check`. The workspace check retained five existing dead-code warnings in `agentic-gpt` browser distribution code and introduced no browser-host warnings.

## Orchestrator live replacement acceptance

- Independently re-ran the nine host unit tests, workspace check, fmt check, and diff check; all passed.
- Cross-built `agentic-browser-host` release for `aarch64-unknown-linux-gnu` using the Laptop toolchain. The produced ARM64 ELF SHA256 was `47c86ffc46af7cbaacf556da6254b7b57fe82a0b21c20d5396e301a99ed0a5b1` and it executed successfully on Orange Pi before integration.
- Preserved the existing Python launcher as `native-host-launcher.python-baseline`, installed the Rust binary alongside it, switched the Neko native-host launcher to Rust, and restarted only the Neko browser container. The official ChatGPT extension immediately started the Rust host and resumed its ping lifecycle.
- Re-ran the exact existing `/srv/apps/neko-browser-host/probe.py` against the Rust host without changing the probe. `bridge.getStatus`, `getInfo`, `getUserTabs`, `claimUserTab`, `attach`, `Target.setAutoAttach`, `Runtime.evaluate`, `Page.captureScreenshot`, `detach`, and `finalizeTabs` all succeeded against the real Neko Chromium extension backend. The live BOSS tab returned `readyState=complete`; screenshot capture returned a non-empty payload (`base64Length=355224`).
- No rollback was required; Neko is currently using the Rust host. The Python implementation and launcher backup remain available as live-test rollback evidence only.

## Standalone compatibility and ARM64 clean-room E2E

- Current official extension/runtime behavior reconfirmed the earlier standalone gap: the Neko ChatGPT extension still returns `agentRequestHeaderEnabled:false`; with that field forwarded unchanged, the official Browser service attempts to start Codex app-server and a pure Agentic host fails with `failed to start codex app-server: No such file or directory`. The current official `26.908.70816` Browser bundle still contains both the `agentRequestHeaderEnabled` compatibility branch and `BROWSER_USE_SECURITY_MODE` support.
- Added an explicit host-only opt-in mode, `AGENTIC_BROWSER_HOST_STANDALONE_COMPAT=1`. Default Rust-host behavior remains Python-baseline-equivalent. In standalone mode, only a routed `getInfo` response has the optional `agentRequestHeaderEnabled` field removed before it reaches Browser service; other responses are unchanged. A focused unit test covers this behavior; the host suite is now 10/10.
- Rebuilt the ARM64 Rust host and deployed it to Neko with the opt-in compatibility flag. The unchanged baseline probe again passed the real extension sequence; `getInfo` intentionally omitted `agentRequestHeaderEnabled`, BOSS evaluation remained complete, and screenshot capture remained non-empty (`base64Length=355108`).
- Cross-built the current `agentic-gpt` worktree for ARM64 and started it with an isolated HOME/config/cache on Orange Pi. Starting from an empty Browser cache with `browser.managed.autoProvision=true`, Agentic fetched the signed current official ARM64 package, verified/materialized it, and selected managed runtime `26.908.70816/prod`. `browser.list` reported `runtimeAvailable:true`, and `browser.manual` read/searchable docs from that newly materialized runtime.
- For this explicit standalone acceptance run, the isolated Agentic process was launched with the official `BROWSER_USE_SECURITY_MODE=disabled-for-local-testing` environment mode. A temporary host bind mount made Neko's `/srv/data/neko-browser/bridge` visible at host `/tmp/codex-browser-use`; both the isolated Agentic process and the bind mount were removed after acceptance.
- `browser.acquire` created real lease `arm64-e2e` against the Rust-host-backed Neko extension backend. `browser.repl` created an agent-owned tab, navigated to `https://example.com`, and returned `Example Domain`; a second call preserved `globalThis.__armSmoke=41`, proving persistent JavaScript state.
- Current runtime/backend surface exposed `dom_cua`, `cua`, `playwright`, etc., but not `tab.ax`, despite bundled `accessibility.md` still documenting `tab.ax`. The E2E therefore used the actual current official `tab.dom_cua.get_visible_dom()` surface, which returned the real Example Domain `Learn more` link. This docs/surface mismatch is recorded as an upstream/current-runtime observation rather than hidden by Agentic.
- Real screenshot passthrough succeeded: `Tab.screenshot()` returned 16,921 bytes and `nodeRepl.emitImage(...)` emerged through outer `browser.repl` as an `image/jpeg` content item.
- The temporary agent-owned tab was closed (`openTabs=[]`). `browser.reset` returned the lease to `ready`; the subsequent REPL observed both test globals as `undefined` while `browserId` remained `1`, proving JS reset plus Browser re-bootstrap. `browser.release` returned `released:true`, and final `browser.list` showed an empty lease list with the managed runtime still available.

## Deferred frozen-plan work

The Rust host, official ARM64 managed runtime acquisition, full Agentic `browser.*` end-to-end path, release packaging, Native Messaging manifest/launcher contract, and durable Neko socket layout are now live-proven or repository-documented. The two explicitly proven standalone compatibility settings remain opt-in launch environment settings rather than new Agentic config-schema fields; this is intentional for the current narrow self-hosted deployment rather than a blocker for the Browser tool surface.

## Public release and deployment integration

- Added `agentic-browser-host` to the standard Linux distribution binary lists, remote release copy path, and GitHub release archive. Future x86_64 and ARM64 release tarballs contain `agentic-gpt`, `agentic-gpt-hub`, and `agentic-browser-host` together.
- Updated English/Chinese README and development docs for the third optional binary.
- Added `docs/browser-self-hosted.md` and `docs/browser-self-hosted.zh-CN.md` with the official extension id, standalone launcher/manifest shape, managed runtime settings, explicit `BROWSER_USE_SECURITY_MODE=disabled-for-local-testing` worker launch, Neko volume layout, local-only socket boundary, and smoke commands.
- The documented Neko socket layout was independently live-smoked on Orange Pi after removing the temporary bind mount. Host `/tmp/codex-browser-use` was made a symlink to the persistent Neko bridge directory `/srv/data/neko-browser/bridge`; with no bind mount present, isolated Agentic `browser.acquire` still reached the Rust host/extension backend, created and navigated an agent-owned Example Domain tab, closed it, and released the lease successfully.
- `cargo test --workspace` passed, including host 10/10 and the full Agent/Hub/protocol suites. `cargo clippy -p agentic-browser-host --all-targets -- -D warnings`, fmt check, shell syntax checks, YAML parsing, and `git diff --check` passed.
- The repository-wide `cargo clippy --workspace --all-targets -- -D warnings` remains blocked by pre-existing lint debt in Browser Runtime 1–16 and unrelated Room code (dead-code/type-complexity/test-guard plus Room iterator/question-mark/then_some lints). The new `agentic-browser-host` crate itself is warning-free under the exact `-D warnings` policy; this phase did not broaden scope into unrelated lint refactoring.
