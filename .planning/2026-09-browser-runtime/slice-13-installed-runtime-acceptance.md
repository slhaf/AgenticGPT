# Slice 13 — Installed Runtime Live Acceptance

This is an orchestrator-owned acceptance slice. No new Browser architecture is introduced here. The goal is to prove the accepted V1 surface against the real installed Desktop Browser runtime and active Chrome extension backend on Laptop.

## Acceptance path

Use a temporary Agentic config/runtime so the user's live Agentic configuration is not rewritten. The temporary runtime must discover the selected Desktop descriptor through the normal production path.

Verify through the actual local MCP surface, not by calling Browser internals directly:

1. `local list-tools` advertises exactly the six frozen `browser.*` names and no semantic Browser tools.
2. `browser.list` reports `runtimeAvailable: true` with the selected `appVersion`/channel and no opaque session/turn/process ids or runtime paths.
3. `browser.manual` reads/searches the selected runtime docs through the public tool and returns only docs-relative paths.
4. `browser.acquire` creates one named lease against the real extension-backed Chrome runtime.
5. Two `browser.repl` calls on that lease prove persistent JavaScript bindings survive calls.
6. A real Browser SDK read-only probe (`agent.browsers.list()` and/or `browser.tabs.list()`) proves the official Browser binding is live. Do not navigate, type, click, submit, or mutate a webpage for acceptance.
7. A real screenshot-producing Browser SDK call proves image content survives the full outer Agentic MCP `browser.repl` path. Prefer a harmless existing-tab screenshot/AX screenshot; do not expose or persist page contents beyond the immediate acceptance output.
8. `browser.reset` succeeds; after reset the prior arbitrary JS binding is absent while Browser bootstrap remains usable.
9. `browser.release` succeeds, then `browser.list` shows the lease removed.

## Safety / privacy

- Reuse the already running Chrome/extension backend; do not start a second browser/profile unless the backend is unavailable.
- No navigation, login, form mutation, message sending, uploads, downloads, or external side effects.
- Do not print/store screenshot payloads or page contents in planning files, git history, or audit.
- Use a temporary config/workspace/socket and remove only acceptance-owned temporary artifacts afterward.
- Never modify the user's existing Agentic config or the unrelated Browser PoC `__pycache__`.

## Completion

If all acceptance checks pass, record concise evidence in `progress.md`. If the live chain exposes a product defect, fix only the smallest Browser V1 defect, rerun focused/full verification as warranted, and then repeat the live acceptance.
