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
