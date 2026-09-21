# WP4-A progress

## 2026-09-21
- User authorized starting WP4-A and intends to implement WP-R and WP4-B afterward.
- Read confirmed decisions, target ownership and complete WP4-A scope/acceptance. Initialized and resolved PLAN_ID=2026-09-21-wp4a-contract-parity.
- Initial tracked diff contained only the plan selector change. No current source edit, test, build or runtime result is claimed for WP4-A.
- Main decomposed discovery into three independent read-only slices: Job/Room contract projections, Skill wait enforcement, and CI/artifact consumers. Workers skip all validation and do not edit plans/source.
- Future WP-R/WP4-B tasks are retained but blocked by the user's chosen package sequencing; they are not dropped or added to this package's completion claim.
- Completed current-source inventories and recorded the authority/difference matrix. Job pagination/cancel/freshness work partly landed in WP3; remaining fields/defaults are projection corrections. Skill runtime waits already cap30 downstream; only pure helpers and descriptor metadata need alignment.
- Selected clamp-to30 with defaults unchanged (Skill5, Job get0), preserving zero/no implicit cancellation. LSP references cover both exported Skill helpers and their actual callers/tests.
- No in-repo consumer of agents-minimal.yaml was found. It will remain explicitly historical/noncanonical rather than be silently deleted or receive a speculative CI gate; external users are not claimed absent.
- Retrieved current jsonschema v4.25.1 API/reference documentation using Context7 for strict OpenAPI3.1 instance validation. One findings edit used an invalid line anchor and was rejected; re-read and applied corrected anchors without partial mutation.
- Committed authority baseline as c8341ca. Four independent writers handled Protocol bounds, public projections, parity gate and current docs/prediction labeling.
- `cargo test -p agentic-gpt-protocol skill_wait_seconds_are_bounded_without_overflow`: 1 passed; both helpers cover zero,30,31,u64::MAX.
- Ran the existing Agent binary with an isolated private HOME and real local MCP ingress. Actual tools/list advertised Job get wait default5 and omitted Skill wait min/max/default. Captured reproduction, then stopped/reaped the process before rebuilding.
- Actual prediction CLI smoke found missing report output after labeling edits (exit0, stdout0bytes). Restored report printing and exercised canned shape fixtures: good strict18/18 exit0; one mismatch loose17/18 exit0; same mismatch strict17/18 exit1. Every report states prediction-shape and runtimeValidation:not-performed; no model-quality or runtime-dispatch claim.
- Verification dependencies are isolated under /tmp/wp4-6c1yftz0/venv (PyYAML6.0.3, jsonschema4.26.0); no global Python environment changes.
- Rebuilt Agent/Hub binaries successfully. Real post-change local descriptors now report Job wait default0 and Skill wait default5, all integer0..30.
- Real Skill run waits: omitted5.018s, zero0.014s,30/31/u64::MAX each30.021s. Five Jobs remained running after bounded waits.
- Held actual Skill execution read leases while an inline replacement install waited. Install get waits: omitted5.022s, zero0.023s,30/31/u64::MAX30.025–30.027s; all still running afterward. Explicit install cancellation and five explicit Job cancellations reached cancelled; Job cancellation evidence was local_process_kill_completed. Probe and Agent exited0 and were reaped.
- Draft202012Validator accepted all113 current OpenAPI component schemas; workflow structure contains only the rust job with a steps list. This structural check is not yet the live parity gate result.
- Additional projection drift found during integration: HTTP process/MCP/Skill starts return flat JobToolResponse rather than nested JobResponse. Surface owner corrected affected success schemas and group inputs while preserving mandatory provenance for the live Job-get branch. Strict operation-schema validation is retained for unavailable errors.
- Initial rustfmt check identified formatting-only changes in authored Rust edits; applied cargo fmt --all and started the complete workspace test suite.
- `cargo test --workspace`: 665 passed across13 suites,1 ignored; five existing Browser-distribution dead-code warnings. Three source-string OpenAPI tests were removed in favor of the live/schema gate, and one genuine Skill bound regression was added.
