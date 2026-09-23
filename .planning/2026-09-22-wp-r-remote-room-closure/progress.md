# WP-R progress

## 2026-09-22
- User authorized proceeding with WP-R and requested a separate future low-necessity test-cleanup work package; standards remain undecided.
- Read planning skill, confirmed decisions and complete WP-R scope/acceptance. git diff --stat showed no pre-existing tracked changes before initializing this package.
- Initialized PLAN_ID=2026-09-22-wp-r-remote-room-closure in the architecture-cleanup worktree.
- Main decomposed current-source mapping into independent Agent/Protocol and Hub/consumer slices; scouts are read-only and do not validate or commit.
- No WP-R implementation/runtime proof claimed yet. Production deployment untouched.
- Reserved future WP-T with explicitly undecided standards; committed f8fd5a8. No tests removed or new cleanup policy selected.
- Scouts completed current-source maps; Main verified authoritative service entrypoints and nested HubCommand payload convention using CodeGraph/current source. Exactly nine current operations selected; no invented standalone wait API.
- Frozen interface/ownership/migration contract in findings.md and started four independent writing slices. Existing Agent maintenance safeguards and Hub captured lease/run receipt ownership remain unchanged.
- Created private verification root /tmp/wpr-uSTFdSHg; isolated Python dependencies will support the existing gate, not a new harness.
- Committed contract baseline as8c21527. A phase-line edit briefly duplicated the baseline row and replaced verification; corrected immediately using returned anchors before committing. Docs worker's scoped findings ledger was included in the baseline; instructed worker not to create more auxiliary planning files.
- Installed isolated PyYAML6.0.3/jsonschema4.26.0 format dependencies under the private probe root.
- Real pre-cutover Hub binary probe passed its negative expectations: seven newly required semantic tools absent from Full tools/list; POST diary.active returned unregistered404 with empty body, not a typed Room routing error. The Hub exited/reaped. Probe imported committed WP4-A helpers, avoiding workers' in-flight script edits.
- Implemented the coordinated Protocol/Agent/Hub nine-operation cutover, public contracts, migration instructions, and existing live parity gate extensions. No implementation commit yet.
- Integration build exposed dropped Agent imports, unsupported Schemars attribute syntax, and removal of a helper still used by bootstrap/skills. Restored shared behavior and corrected owned changes; subsequent Agent/Hub builds passed.
- Workspace test compilation exposed two leftover legacy helper references; migrated them. The next workspace run passed 667 tests across 13 suites, with one ignored test (artifact://471), before final boundary fixes.
- First live gate failed on requiring metadata absent from the unchanged local descriptor. Corrected the gate to permit omitted local metadata while requiring complete Hub metadata. Second live run exposed a missing program field in the existing process.exec fixture; restored /usr/bin/printf.
- Boundary review identified permissive read payload decoding and explicit missing-resource codes mapping to 400. All nine current request DTOs now reject unknown fields; explicit notebook/entity not-found codes map to 404. Added one real HTTP foreign-agentId rejection scenario. Final rebuild and live verification remain in progress.
- Final Agent/Hub build passed (artifact://473). The earlier private venv was no longer present, so recreated isolated dependencies under /tmp/wpr-final-verify; no global Python packages changed.
- Live gate progressed through all nine HTTP operations, then exposed a wrong expected invalid-path code in the fixture. Agent repository validation correctly returned room_repository_path_invalid for ParentDir; corrected only the expected code, preserving strict HTTP 400/error assertions.
- Final supervised wpr-live-final run passed and exited 0: nine HTTP/Full MCP operations, Coordinator rejection, actual local maintenance/Git ownership, invalid path and dirty tree, unknown agentId 422, no-active/ReportingOnly no-fallback, same identity reconnect/new lease/content retention, and local bare-origin workflow wait preserving submitted requests.
- Final cargo test --workspace passed 667 tests across 13 suites, one ignored (artifact://480). cargo fmt --all -- --check passed. Strict Clippy failed on existing browser, confirmation, Job freshness and test-helper debt (artifact://479); no suppression or unrelated broad cleanup.
- Committed coordinated runtime cutover as 581c2ce. Production tunnel/SSH and hosted GitHub workflow were intentionally not exercised; workflow proof uses private local bare origin, and generation races use existing Rust coverage.
- After smoke success, removed owned Python bytecode and the recreated private venv. Supervised processes are exited/failed, none running. Earlier /tmp/wpr-uSTFdSHg was already unavailable. Current architecture closure and phase evidence are being committed separately.
- Committed public OpenAPI and the passing live gate with verification ledger as dfdb77c. Existing gate was extended; no parallel test framework or test-removal policy introduced.
