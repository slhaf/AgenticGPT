# Agentic GPT v0.10.0 release-boundary notes

These notes describe the v0.10.0 candidate boundary. They document the
breaking `file.batch` contract and verification; they do not create a tag,
push a branch, deploy an artifact, or publish a release.

## Highlights

- `file.search` accepts bounded context overshoots, clips them to the live
  `limits.maxFileSearchContextLines` value, and reports requested/effective
  context plus a bounded warning.
- Repeated edits to one normalized `file.batch` target chain over one candidate
  and produce one physical replacement.
- `file.batch` now commits independent normalized file groups. Read/search,
  staging, confirmation, and commit failures are isolated; mixed outcomes use
  `completed_with_errors` and bounded group/operation evidence.
- Cross-file rollback retention, execution, response states, and rollback-only
  error codes are removed. `dryRun:true` remains the whole-request preview
  path, and one confirmation still covers valid effective groups.
- Public tool descriptions and schemas now state conditional guards, Job
  follow-up, admission versus side-effect boundaries, and standalone/Hub
  parity. The checked-in matrix covers Normal 24 / Room 36 and Hub profiles.
- Nine deterministic contract cases run through descriptors, serde, dispatch,
  and dry-run. An optional provider-neutral evaluator scores model predictions
  without network access or credentials.

## Breaking changes and migration

Read [`migration-v0.10.md`](migration-v0.10.md) before upgrading callers.
Consumers must:

- Stop sending `file.batch.atomicity`; it is not accepted or advertised.
- Treat `completed_with_errors` and the `groups` summary as normal partial
  success evidence; never infer cross-file atomicity from a successful batch.
- Remove handling for `rolled_back`, `partial_failed`, `rollback_failed`, and
  `not_committed`, plus rollback-only error codes.
- Preserve exact `expectedRevision` / `expectedAbsent:true` guards and retry
  only the failed normalized file group after inspecting its current state.

The v0.9 migration remains applicable for Job names/config changes; this
boundary does not add compatibility aliases.

## Verification boundary

The candidate was checked with:

```text
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p agentic-gpt --test local_control
cargo test -p agentic-gpt --test standalone_supervisor
python3 scripts/evaluate_tool_contracts.py --cases tests/tool-contract-cases/cases.json
git diff --check
```

The full Agent suite passed 242/242 with local socket/HTTP/subprocess
permissions enabled; the Hub suite passed 59/59. No tag, push, deployment, or
publication is part of this implementation boundary.
