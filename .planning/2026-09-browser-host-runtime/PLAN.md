# Browser Host Runtime — Frozen Contract

## Goal

Productize the already-proven ChatGPT Chrome extension compatibility path as a public, reproducible Agentic Browser backend, then prove the complete Linux ARM64 path from official signed runtime acquisition to a real human-takeover-capable browser.

Agentic is a public GitHub project. The implementation must therefore avoid machine-specific hidden state and undocumented manual setup as required product dependencies. A development PoC may be used as evidence, but the accepted path must be buildable, configurable, testable, and deployable by another user from the repository.

The existing Browser Runtime 1–16 phase is complete at `6800a68 feat(browser): add signed managed runtime acquisition`. This is a new phase, not Slice 17 of the old plan.

## Product surface — frozen

The model-facing Browser surface remains exactly:

- `browser.manual`
- `browser.acquire`
- `browser.repl`
- `browser.reset`
- `browser.release`
- `browser.list`

Do not add semantic Rust tools for navigation, tabs, AX, clicks, screenshots, Playwright, or CDP. Browser semantics remain owned by the official OpenAI Browser SDK and are exercised through `browser.repl` JavaScript.

The target architecture is:

```text
Agentic browser.*
    -> official managed node_repl + Browser SDK
    -> Agentic compatible Browser host
    -> official ChatGPT Chrome extension
    -> Chromium / Chrome

Neko deployment:
    Chromium / Chrome
        <-> human via Neko UI
        <-> Agentic via the same official extension backend
```

Agentic owns runtime acquisition, persistent Browser kernel/session lifecycle, policy/audit, the compatible host, and deployment integration. It does not fork or vendor the proprietary OpenAI Browser runtime or extension.

## Public-project quality bar

Accepted implementation must not depend on:

- `/srv/...`, one user's HOME layout, or one existing Orange Pi installation;
- manually copied proprietary OpenAI runtime files;
- manually edited generated manifests whose required values are not represented in config/install tooling;
- an undocumented process that must already be running;
- a private local patch to the official ChatGPT extension or Browser SDK.

Repository defaults may remain conservative and opt-in. Neko is a supported deployment profile, not a mandatory dependency for all Agentic Browser users.

## Slice 01 — freeze the compatible-host protocol contract

Before production implementation, re-verify the current official extension/runtime behavior and turn the Orange Pi PoC observations into deterministic protocol fixtures.

Required evidence:

- Chrome Native Messaging framing used by the official ChatGPT extension, including 4-byte little-endian length prefix and message-size bounds.
- Native host manifest contract: host name, allowed origin / official extension ID, launcher semantics, and Chrome/Chromium discovery locations relevant to supported Linux deployment.
- Extension request/response correlation and connection lifecycle.
- Current `getInfo`, user-tab, claim/attach, backend registration, and extension capability fields actually used by the Browser service path.
- Current backend Unix-socket contract expected by the official Browser service / native-pipe path.
- Reconnect behavior when the extension, browser, or host restarts.
- Error behavior for malformed JSON, duplicate/stale response IDs, disconnect during a request, and oversized messages.

The current Python Orange Pi host and community implementations are evidence only. The frozen contract must be represented by repository-owned fixtures/tests rather than by comments that refer to a live machine.

Two historical compatibility observations must be re-tested against the current official ARM64 runtime and current extension rather than copied forward blindly:

1. `getInfo.agentRequestHeaderEnabled` caused the Browser service to query Desktop/Codex app-server policy in a standalone unauthenticated environment.
2. `BROWSER_USE_SECURITY_MODE=disabled-for-local-testing` was required outside Desktop/Codex in an earlier ARM64 standalone probe.

If either remains required, record the exact evidence and freeze the narrowest compatibility shim. If current versions no longer require it, remove the historical workaround from the production design.

### Slice 01 acceptance

- Contract document and deterministic protocol fixtures exist in-repo.
- Tests can exercise framing/correlation/reconnect semantics without a real browser.
- Current official ARM64 runtime + current extension evidence is recorded in `findings.md`.
- No production host behavior is invented from memory when current evidence is unavailable.

## Slice 02 — implement the production compatible host

Replace the disposable Orange Pi Python native-host PoC with a first-class repository component. Preferred implementation is an independent Rust binary/crate, e.g. `agentic-browser-host`, unless Slice 01 produces concrete evidence that another repository-native form is materially better.

The host is deliberately thin. Its responsibilities are limited to:

- Chrome Native Messaging stdin/stdout framing;
- bounded JSON parsing and serialization;
- request/response correlation;
- bridging to the Browser backend Unix socket contract frozen in Slice 01;
- connection/reconnection state;
- bounded buffering/backpressure;
- deterministic startup/shutdown and stale-socket handling where applicable;
- structured operational errors;
- redacted/bounded logging;
- a small local health/status surface useful to Agentic/deployment diagnostics.

It must **not** implement Browser SDK semantics, AX, Playwright, CDP command design, page parsing, or model-facing browser APIs.

Security/robustness requirements:

- Reject impossible/oversized Native Messaging frame lengths before allocation.
- Bound per-message body size and queued in-flight requests.
- Treat malformed JSON and invalid protocol shapes as explicit errors; do not panic.
- New extension connections cannot inherit stale request/session correlation from a prior connection.
- Disconnects fail affected requests rather than hanging them indefinitely.
- Host and socket paths are explicit/config-derived, not user-machine constants.
- Files/sockets/manifests use least-privilege permissions appropriate to the deployment user.
- Logs do not emit full arbitrary browser payload bodies by default.

### Slice 02 tests

Use local fake stdin/stdout and Unix-socket peers to cover at least:

- valid framing in both directions;
- zero/invalid/oversized/truncated frames;
- malformed JSON;
- multiple in-flight request IDs;
- out-of-order responses;
- stale/unknown/duplicate response IDs;
- extension disconnect mid-request;
- backend disconnect mid-request;
- extension reconnect and clean correlation reset;
- bounded shutdown.

### Slice 02 acceptance

- Production host builds through normal repository tooling on supported Linux targets including ARM64.
- Fake-peer protocol suites pass without a real Chrome installation.
- The official extension can connect to the new host on a real test machine and complete a minimal current-contract probe.

## Slice 03 — reproducible installation and Neko/Chromium integration

Make the host and official extension connection reproducibly deployable instead of relying on hand-maintained Orange Pi state.

The product should separate generic Linux host installation from the Neko profile.

Generic Linux integration must define:

- how the native-host binary is located;
- how `com.openai.codexextension` manifest content is generated/installed for supported Chromium/Chrome-family browsers;
- how the official extension ID is represented and validated;
- how the Browser backend socket location is configured;
- install/update/uninstall behavior that does not clobber unrelated browser configuration;
- diagnostics that can report manifest presence, executable availability, extension connectivity, and backend connectivity without exposing secrets.

Neko profile must make the already-proven deployment reproducible:

- persistent browser profile;
- official ChatGPT Chrome extension installed through supported browser policy/update flow rather than committed extension files;
- native-host manifest/launcher visible inside the Chromium environment;
- compatible host executable/service available across the container/host boundary;
- Browser backend socket plumbing into the location expected by the official Browser service;
- Tailnet/private-access assumptions documented separately from Browser protocol correctness;
- browser UI remains available for human takeover and is not lifecycle-owned by `browser.acquire`.

Do not make Agentic start/stop Neko for each Browser lease. Neko/Chromium is a durable visible browser service; Browser leases attach to its official extension backend.

### Slice 03 acceptance

- A fresh supported Linux/Neko deployment can be produced from repository instructions/configuration without copying files from the developer's existing installation.
- Browser/host restarts restore the native-host connection without manually editing manifests.
- Diagnostics distinguish at least: runtime unavailable, host unavailable, extension disconnected, backend unavailable, ready.
- Existing non-Neko Desktop/Laptop Browser behavior remains unchanged.

## Slice 04 — Linux ARM64 clean-room end-to-end acceptance

Prove the complete public-product path on the Orange Pi 4 Pro (or equivalent Linux ARM64 host) without a preinstalled ChatGPT/Codex Desktop runtime and without a manually copied Browser bundle.

Start from an isolated or empty Agentic Browser runtime cache. The acceptance chain is:

```text
official OpenAI signed ARM64 APT metadata
    -> verified ARM64 ChatGPT .deb
    -> Agentic managed runtime cache
    -> official ARM64 node_repl
    -> official Browser SDK / browser-service
    -> Agentic production compatible host
    -> official ChatGPT Chrome extension
    -> Neko Chromium
    -> real Agentic browser.* tools
```

Required functional acceptance:

1. `autoProvision=true` selects the signed official ARM64 package and materializes it from an empty cache.
2. `browser.list` reports the selected runtime/version and backend availability.
3. `browser.manual` reads/searches documentation from that selected managed runtime.
4. `browser.acquire` creates a real named lease against the Neko extension backend.
5. Sequential `browser.repl` calls prove persistent JS state.
6. Through the official Browser SDK, an agent-owned tab is created and navigated to a safe test page.
7. The test reads real page/AX content and obtains a real screenshot/image result through the existing Browser tool result channel.
8. The temporary tab is closed.
9. `browser.reset` restores a usable lease after JS state reset/rebootstrap.
10. `browser.release` removes the lease with bounded cleanup.
11. The Neko UI visibly presents the same browser and remains human-takeover-capable throughout.

Recovery acceptance must cover at least:

- compatible-host restart while Chromium remains alive;
- Chromium/extension restart while Agentic remains alive;
- backend socket disappearance then reappearance;
- Agentic restart while the managed ARM64 runtime cache remains valid;
- stale host/socket artifacts from an unclean prior exit.

The acceptance should verify the narrow compatibility shims from Slice 01 if any remain. A shim may be accepted only when it is scoped to the self-hosted/Neko backend path and does not silently weaken or alter normal Desktop Browser behavior.

## Distribution and licensing boundary

- Do not commit OpenAI proprietary runtime `.deb` contents, Browser SDK bundles, or extension CRX/unpacked extension files.
- Official runtime acquisition continues to use signed OpenAI distribution metadata implemented in the completed Browser Runtime phase.
- The official Chrome extension should be installed through the browser's supported extension distribution/policy mechanism.
- Repository tests use fixtures/synthetic peers unless an explicit opt-in live acceptance is being run.

## Implementation discipline

1. This document is authoritative for this phase.
2. Maintain `progress.md` as a concise execution ledger and `findings.md` for protocol/runtime evidence and unresolved gaps.
3. Do not redesign the six Browser tools.
4. Do not fork official Browser semantics into Agentic.
5. Do not turn Neko-specific deployment assumptions into generic Browser runtime requirements.
6. A coding agent may implement slices, but orchestrator review, real-environment acceptance, and final commit decisions remain separate.
7. Do not use destructive cleanup commands (`git reset`, `git restore`, `git clean`, bulk deletion) to simplify a dirty worktree.
8. Run focused tests per slice and repository-appropriate full verification before acceptance.

## Phase completion

This phase is complete only when another user can reasonably reproduce the supported Linux host path from the public repository, and the ARM64 Neko acceptance proves that Agentic can acquire the official runtime and control a visible, human-takeover-capable browser through the official ChatGPT extension without ChatGPT/Codex Desktop being preinstalled.
