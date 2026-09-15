# Browser Host Runtime — Progress

Status: baseline-compatible Rust translation accepted after real ARM64/Neko replacement smoke; ready for full Agentic Browser E2E.

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

## Deferred frozen-plan work

Protocol expansion beyond the verified Python behavior, production installation/manifests, and full Agentic `browser.*` ARM64 clean-room end-to-end acceptance remain deferred after this translation pass.
