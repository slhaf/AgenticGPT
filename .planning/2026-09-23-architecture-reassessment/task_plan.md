# Architecture reassessment

## Goal
Reassess the whole Agentic repository against the original six outcomes: actual architecture, problems/root causes, suitable target, responsibilities/dependencies/rules, durable developer guidance, and a gradual verifiable restructuring plan. Preserve completed boundary fixes as valuable implementation, not evidence that all structural work is done.

## Scope and constraints
- Five Rust crates; Console shared/platform/host applications; OpenAPI, scripts, deployment/CI and experimental/example boundaries.
- Product remains controlled execution infrastructure, not reasoning loop/provider orchestration/long-term memory.
- Documentation and planning only. No production edits, runtime configuration changes, deployment or new tests.
- No preconceived file-size threshold, forced layers/crates or test-removal standard. WP-T remains undecided.
- Existing change in prior WP-R room-contractdocs-findings.md is not ours; preserve and exclude from commits.
- Main owns integration, cross-slice decisions, shared planning and commits. Scouts provide source-grounded reports and skip all validation/build/lint/tests.

## Phases
1. Scope — complete: freeze original goals, repository coverage and evidence format; committed planning baseline `e0db2d8`.
2. Investigation — complete: five parallel read-only reports plus Main manifest and source spot-check; evidence and six-goal matrix recorded in findings.md. No runtime verification claimed.
3. Synthesis — in_progress: reconcile dependency seams, update current architecture assessment and specify bounded follow-up work with structural and behavioral acceptance.
4. Delivery — pending: verify references/scope/document consistency, commit reviewed documents and ledger. No claim of runtime verification in this read-only review.

## Evidence contract
Every finding: exact source paths/symbols, concrete responsibility/dependency or duplication, consequence, root-cause hypothesis clearly labeled, what to preserve, smallest justified next action, verification criterion. File length alone is not a defect. Missing docs are not proof of missing code. Historical closed issues are not current findings.

## Completion
All six original goals have an evidence/coverage assessment; every repository surface is classified; actionable structural findings map to proposed packages without silently reopening completed work or widening product scope.
