# Structural refactoring findings

- Existing five-crate graph is Agent→Protocol+apply-patch, Hub→Protocol, browser-host independent; Console separate Gradle. No evidence currently justifies a new crate.
- `jobs::start_process_job_inner` stores admission Config at line 1117 while `run_async_job` reloads `state.config` line 1301 before policy/preflight/spawn. This is source-level only until deterministic reproduction.
- Current uncommitted diff before plan initialization affects only the unrelated earlier WP-R scoped findings file. Preserve it verbatim outside staging.
- CodeGraph targeted exploration confirms Agent stdio direct branch and HubCommand trampoline with shared gate, plus duplicated Hub Job projection helpers; preserve intentional ingress contracts.
- New worker implementation slices have disjoint ownership. Protocol retains root public facade; no wire version or serde contract changes. User authorizes design patterns/crate splits when justified, not as a mandate; present evidence favors private domain modules and narrow functions rather than another crate.
