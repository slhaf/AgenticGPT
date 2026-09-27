# WP4-A findings

## Confirmed boundaries
- Protocol owns Hub↔Agent wire DTOs and pure rules; it is not the schema for every public entry point. Agent descriptors, Hub MCP schemas and HTTP/OpenAPI DTOs own their consumer projections, with semantic parity where operations overlap.
- WP3 changed Job freshness/result metadata and some OpenAPI responses. Old drift descriptions are investigation inputs, not assumed current failures.
- Room legacy errors remain real current outcomes. Do not restore removed commands or silently translate old append/update into different maintenance semantics; WP-R will implement/migrate required remote capabilities.
- WP4-B is internal module organization; user wants it later but it is not part of WP4-A.
- Wait timeout and execution cancellation are distinct; public request limits must be enforced consistently without fabricating terminal states.

## Authority and current difference matrix
Current source inventories completed. Decisions preserve runtime defaults and correct their projections:

| Contract | Authority / current behavior | Decision / verification |
|---|---|---|
| Job startedAt | Protocol omits absent Option; queued/not-started may lack it | Remove incorrect OpenAPI required entry; validate actual not-started projection, never invent timestamp |
| Job list | Protocol default50, clamp1..100; HTTP accepts group/cursor; nextCursor already added by WP3 | Add missing query declarations/default50; retain live pagination and cursor-unavailable errors, no cache-generated cursor |
| Job get | Actual dispatch defaults0/caps30; Agent descriptor advertises5; HTTP waitOnly missing in schema | Keep runtime default0; correct metadata, declare waitOnly=false, validate active wait-only vs terminal |
| Job cancel / freshness | WP3 already uses distinct cancel DTO and provenance metadata | Preserve corrected response; validate actual success/cache/unavailable/error branches |
| Notebook append/selectExact | scope/content required, significance default/abstract optional; selectExact uses date | Correct request projection. Explicitly classify current legacy-command rejection; do not claim successful Room execution or implement WP-R |
| Skill waits | Both pure helpers default5 but omit max clamp; actual consumers already cap30 | Clamp Protocol helpers using existing policy; preserve default5/zero/30/no implicit cancellation; align metadata |
| Runtime vs prediction | Rust corpus dispatches; Python evaluator only matches predicted tool/argument shape | Separate reports; label Python as prediction-shape probe, not runtime gate |
| CI | Existing Rust tests run; OpenAPI only YAML-parsed | Add supported-artifact schema/actual-response and live descriptor/profile parity, not source regex |
| agents-minimal.yaml | No in-repo code/CI/primary-doc consumer found; external consumers not enumerated | Keep explicitly historical/noncanonical; no speculative CI gate or silent deletion. Supported import is hub.yaml |

Inventories: Wp4JobContractMap, Wp4SkillContractMap, Wp4GateArtifactMap. Important correction to old roadmap wording: Skill runtime waits were already bounded downstream; this closes a helper/projection inconsistency, not a proven unbounded runtime wait.

## Validation boundaries
Static schema parsing and model prediction shape are not runtime-dispatch evidence. Existing deterministic Rust corpus and live multi-entry tests have separate roles. New checks must defend actual semantic contracts, not source text or incidental wording.

## Implementation ownership and libraries
- Protocol worker owns lib.rs Skill helpers/tests and any narrow redundant normalization removal in skill_installs.rs. Surface worker owns openapi/hub.yaml, Agent stdio_server.rs metadata and Hub mcp_server.rs metadata.
- Gate worker owns live/schema parity script, CI wiring and scoped OpenAPI source-string tests in Hub main.rs. Docs/probe worker owns current docs, prediction probe labeling and historical agents-minimal labeling. Shared files require Main coordination.
- Strict schema validation uses Python jsonschema Draft202012Validator, local references and format checking, plus existing PyYAML. Current v4.25.1 docs retrieved via Context7: https://github.com/python-jsonschema/jsonschema/blob/v4.25.1/docs/validate.rst . Schema validation does not replace live dispatch.

## Integration findings
- Actual HTTP process/MCP/Skill creation returns flat JobToolResponse; process/MCP batches have flat child projections. Correct the OpenAPI operation references, not runtime adapters. Keep live Job-get provenance required via a separate live projection rather than weakening it when sharing a flat base.
- Real gate execution exposed missing Hub MCP Job-list limit metadata. Align its default50/min1/max100; preserve all existing filters, including state.
- The real standalone HTTP fixture must supply AGENTIC_GPT_SUPERVISOR_TOKEN matching the private worker flag. This exercises the authorized worker/listener, not the production tunnel or operator launcher.
- Hub MCP legitimately returns stateless application/json while the Agent HTTP listener returns SSE/session headers. The client must accept both media types and optional session IDs; verified against https://modelcontextprotocol.io/specification/2025-06-18/basic/transports and Hub mcp_post. No legacy transport fallback or production topology change is introduced.
- Stronger live cases must prove actual completed output, active waitOnly compactness, prompt default0, exact pagination cardinality and operation-selected schemas. A policy-rejected jobId, empty second page, unrelated Coordinator error or standalone component validation is insufficient evidence.
- Actual downstream MCP success requires real confirmation; executable policy.allow does not authorize MCP and there is no needConfirm escape field. Use the already-running HTTP Agent as downstream, isolated loopback notification delivery and the real tokenized Hub callback; never seed authorization state or contact desktop/public notification services.
- Current AGENTS.md is an active entrypoint, not release history. Cargo.toml lists five workspace members; corrected its stale three-crate statement and linked the supported gate setup.
- Notebook append/selectExact extractor failures are actual HTTP422 text/plain, distinct from valid-decoding legacy400 JSON errors. Both media/status contracts are declared and strictly validated; missing response schemas are gate failures, not passing limitations.
- Six old nested response wrapper components had no remaining OpenAPI operation consumers and were removed. JobInfo and current flat components remain; all107 schemas and local references validate, including references outside exercised operation paths.
- Actual ordinary-Hub cache fallback exposed an obsolete internal response consumer: Agent hub.rs jobs_from_command_response expects nested/full JobInfo after local_service has projected a flat response. Typed authoritative metadata must reach the existing JobUpdate path independently of public JSON projection; a reporter-only fixture or direct cache seed would hide this defect. Preserve wire payloads, reporting privacy and volatile-cache ownership.
- Repair uses a Hub-only optional typed snapshot collector: projectors build unchanged flat values, then move JobInfo into the collector. Non-Hub callers pass None; there is no second Job lookup, legacy JSON decoder, fabricated metadata or new background publisher. The existing JobUpdate/reporting privacy path remains authoritative.
- The same real gate failed before the repair and passed afterward, including offline cached/unknown/cancel branches and actual never-started JobInfo schema validation. Final Rust suite:664 passed/1ignored; strict format/build pass. Existing strict Clippy failures remain explicitly separate from this package's passing runtime/schema acceptance.

## Existing strict lint debt
The existing CI command cargo clippy --workspace --all-targets -- -D warnings fails in OAuth test helpers, Hub confirmation/cache code, Browser code, Job registration and transport-ledger code. The runtime suite passes, but this does not imply the full CI job is green. Preserve strict lint with no suppressions; run the new independent runtime gate before that lint step so the old failure does not hide contract evidence. Broad unrelated lint cleanup is not silently folded into WP4-A.
