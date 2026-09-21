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
