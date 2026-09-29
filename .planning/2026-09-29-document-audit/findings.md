# Findings

## Superseded design artifacts (verified by ArchiveDocsAudit)
- `docs/superpowers/specs/2026-08-04-config-init-wizard-design.md:14-17,170-174` prescribes `inquire` and explicitly no fullscreen TUI; `docs/superpowers/specs/2026-08-07-config-init-fullscreen-tui-design.md:18-24` explicitly supersedes those interaction sections. Current `docs/configuration.md:25-31,99-114` and `crates/agentic-gpt/src/config/config_cli.rs:147-190` describe/implement TUI. Only partially obsolete, preserve historical rationale.
- `docs/superpowers/specs/2026-08-07-config-init-fullscreen-tui-design.md:1-4` status claims pending implementation plan, but TUI is implemented in `crates/agentic-gpt/src/ui/config_tui/mod.rs:1-31`.
- `docs/superpowers/plans/2026-08-04-interactive-config-initialization.md:1-9,34-43` old `inquire` plan and unchecked steps superseded by fullscreen design; historical plan, not actionable.
- `docs/release-notes-v0.9.1.md`/`.zh-CN.md` describe historical `file.batch`, while `docs/migration-v0.10.md:1-20` guides current file API. Do not classify release notes as erroneous current guidance.

## Hub operations (verified by HubOpsAudit and independent source reads)
- `docs/operations.md:397` promises credentials never in argv, but `README.md:285-288,298-302` gives `agent add --secret` and `config set hub.agentSecret` argv examples; `crates/agentic-gpt-hub/src/agents/registry.rs:10-20` accepts the CLI secret. Security-facing contradiction.
- `openapi/agents-minimal.yaml:1-12` explicitly historical/noncanonical, replaced by `openapi/hub.yaml`. `docs/operations.md:345-349` v0.9 checklist archival section only, not entire doc.

## Console and architecture (verified by ConsoleOtherAudit; selected claims source-read)
- `docs/architecture/engineering-rules.md:248`, `target-architecture.md:324` falsely say CLI tmux bypasses authorization; `crates/agentic-gpt/src/ui/cli.rs:233-248` invokes `operation::authorize`.
- `docs/architecture/current-state.md:101,179` and `target-architecture.md:174,318` reference old `storage/job_history.rs`; current source is `storage/process_history.rs` via `main.rs:63-66`; stale line count in current-state:179.
- `console/README.md:7-8` mentions iosMain without iOS target in `console/shared/build.gradle.kts:12-32`; `console/README.md:12-16` labels assembleDebug as running app, but it only builds APK.
- `tests/tool-contract-cases/README.md:10-11` promises `$fixtureRevision` substitution; `crates/agentic-gpt/src/ingress/stdio_server_tests.rs:749-783` passes cases' arguments through verbatim.

## Runtime interfaces (verified by RuntimeToolsAudit and independent source reads)
- `docs/standalone-runtime.md:258` and `docs/interfaces.md:69` claim `process.status` / HTTP status default `waitSeconds=0`, but `crates/agentic-gpt-protocol/src/process.rs:327-333` defaults to 5. HTTP route `crates/agentic-gpt-hub/src/ingress/http/routes.rs:359-363` forwards optional value; `docs/tool-contract-matrix.md:77` correctly specifies 5 and explicit 0.
- No assigned runtime/tool/browser document as a whole is demonstrated obsolete. Browser self-hosted live-environment assertions remain unverified without external deployment access.

## READMEs and configuration (verified by ReadmeConfigAudit and selected independent source reads)
- `README.md:388-389` and `README.zh-CN.md:360-362` hard-code tag v0.9.0; both runtime Cargo packages are 0.9.1, `scripts/release-preflight.sh:7-35` rejects version mismatch. Current dynamic instructions in `docs/development.md:108-114`.
- `docs/configuration.md:485,493` and `docs/configuration.zh-CN.md:444,452` use unsupported `maxActiveJobs`; schema `config.rs:527-536` requires `maxActiveProcesses`.
- `docs/configuration.md:41,69` and `.zh-CN.md:36,60` advertise `job` namespace; enum `config.rs:43-88` has no Job variant.
- `docs/configuration.md:104` and `.zh-CN.md:90-91` erroneously gate Room settings on profile; `config_templates.rs:344-360` gates on live Room toolset.
- `docs/configuration.md:122-124` and `.zh-CN.md:105-106` wrongly exclude MCP server collection from interactive initializer; `config_templates.rs:344-356` includes it.
- `README.zh-CN.md:37,180-182` says 29 Normal / 40 Room; `stdio_server_tests.rs:111-161` specifies 31 Normal + 11 Room = 42 Room tools.
