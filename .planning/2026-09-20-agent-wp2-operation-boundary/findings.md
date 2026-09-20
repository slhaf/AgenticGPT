# WP2 findings

## Architecture input
- Read docs/architecture/decisions.md D01-D08, engineering-rules.md sections1-6, target-architecture.md sections1-4, refactoring-plan.md WP2 scope/acceptance before choosing implementation.
- D04: keep current sandbox default, explicit policy allow override and controlled self-hosted deployment threat model; closing missing checks does not authorize arbitrary new limits.
- D03/D08: coordinated clean cutover; no permanent shim/alias, universal registry, new crate or speculative pattern. Suggested directories are not required migrations.
- Target sections2.4/3.1 and engineering R01-R04: ingress owns auth/framing/schema/error projection; existing local_service is intended shared value layer, resources remain result/effect owners. Context should express real ingress/profile/identity facts, not invent them.
- Roadmap WP2 includes CLI tmux and Room/Skill/Browser direct routes, metadata vs enforcement, and config startup/live consistency. The preceding conversational local-vs-Hub split alone was incomplete; execution scope follows this documented matrix.
- Existing architecture findings are historical baseline; verify against current CodeGraph/source before declaring a bypass or deleting mappings.

## Investigation ownership
- Wp2LocalIngress: stdio/local Unix/HTTP/local_service/state, CLI and direct resource paths.
- Also read current-state ingress/resource/trust baseline, diagnosis A03/A05/A06, target §7/§8, and engineering R18-R25. Historical diagnosis explicitly distinguishes local-admin CLI trust from a remote bypass; CLI behavior needs an explicit context, not accidental inheritance from remote profiles.
- Config guidance defaults unproven reload fields to startup-only; live-safe projection must not split identity/workspace/resource ownership. Multi-ingress parity permits intentional schema/error differences, but not missing authorization.
- Wp2HubIngress: Agent Hub reliable adapter and descriptor/annotation seam.
- Wp2ConfigBoundary: reload/initialization/resource-lifetime matrix and external-effect trust.
- Main: whole-repo direction, final cross-slice design, scope and verification.

## Working tree baseline
After initializing this independent plan, tracked diff contains only .planning/.active_plan pointing to WP2. No preexisting production diff observed.

## Main source check
- CodeGraph current state.rs confirms RuntimeModel transport/profile/hub_mode, with local/tunnel Normal skills/bootstrap allowed but Room denied; Hub Normal skills/bootstrap/Room denied and notifications allowed. Preserve this existing intentional split.
- AppState owns startup-derived private_state/job_history/browser_runtime/skill_installs and runtime; config remains separately mutable. This supports investigating config/resource divergence rather than adding a new runtime.
- CodeGraph current local_service dispatch uses stdio_server::slim_* projection helpers; operation boundary design must avoid introducing a second mapping loop or preserve this adapter-to-core reverse dependency merely by renaming it.
- Runtime smoke prerequisites found: tmux, Python3, Node and cargo are available. No smoke has run yet.

## Source-backed slice findings
- Local scout found an important existing contract: Normal profile with explicitly enabled Room toolset is supported by configuration validation and `normal_runtime_follows_live_room_toolset_for_bootstrap_dispatch`. Do not blindly intersect every direct Room operation with the coarse RuntimeModel diary/notebook flags.
- Existing RequestIngress already differentiates TunnelStdio/LocalUnix/Http and creates request_source; reuse rather than invent parallel transport/auth structs. CLI tmux is an explicit local-admin path, not a remote-authenticated principal.
- tmux resource methods have actual confirmation/policy/audit, but audit source is hardcoded hub:tmux.* even for local/HTTP; CLI helpers bypass the AppState-backed audit path. Keep domain approval controls, correct provenance at admission.
- Config scout confirmed Standalone/Local already apply only policy/pathPolicy/limits/mcpServers/toolsets/httpMcp; startup fields remain unchanged and restart drift is observable. Hub alone whole-replaces Config and ensures candidate workspace, despite fixed private state/history/browser/install/connection owners.
- Existing live reload also copies candidate pathPolicy after a changed workspaceRoot (config CLI rewrites matching roots). Preserve old pathPolicy whenever the candidate workspace differs; live Room enable must bootstrap the current live repository, never candidate restart-required roots.
- Config slice can be confined to main reload functions plus existing health/projection consumers as needed. No runtime resource rebuilding, Browser-host topology, or new reload fields.
