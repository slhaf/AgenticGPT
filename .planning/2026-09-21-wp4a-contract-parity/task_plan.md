# WP4-A — current contract parity, documentation and CI

## Goal
Implement WP4-A end to end from current post-WP3 source, aligning supported public contract projections and runtime behavior, bounding Skill waits, and adding useful cross-surface verification. User also wants WP-R and WP4-B implemented later; they are separate packages, not omitted or folded into this work.

## Current phase
Baseline complete; implementation contracts frozen in findings.md. Main selected projection corrections preserving Job wait default0, bounded Skill helper normalization preserving default5, a live/schema parity gate, and explicit historical classification of the unconsumed minimal OpenAPI artifact. No WP4-A runtime proof is claimed yet. Independent file owners implement while Main prepares integration validation.

## Scope and phases
1. Rebuild current authority/consumer/diff/verification matrix; distinguish fixed WP3 items from active drift. Commit baseline.
2. Fix supported HTTP/OpenAPI/MCP request and response differences using actual consumers and handlers; each coherent contract correction independently reviewable/committed.
3. Enforce the evidenced Skill wait bound without changing defaults, unrelated timeouts, or cancellation semantics. Commit verified correction.
4. Extend existing cross-surface semantic/CI gates and distinguish prediction-shape checks from actual runtime dispatch. Commit useful checks with real positive/negative behavior proof.
5. Classify current versus historical docs and OpenAPI artifacts; preserve history, deliver any real migration steps, complete runtime proof and final records. Commit final docs/evidence.

## Required acceptance
- Every listed Job/Notebook/Skill drift has a current disposition with authority, consumers and evidence; no unexamined carryover from the old roadmap.
- Actual decode/dispatch/response through Agent local MCP, standalone HTTP MCP, Hub Apps MCP and Hub HTTP where supported. Explicit unsupported legacy Room behavior is documented honestly; implementing the new remote Room chain remains WP-R.
- Strict schema/response validation or real importer coverage for affected HTTP contracts, including queued timestamps, pagination, waitOnly/default, cancellation and typed errors.
- Skill waits cover absent, zero, boundary, over-boundary and cancellation; caller timeout never proves execution cancellation.
- Gate checks required/default/bounds/response/error semantics and Full/Coordinator behavior, not merely YAML parsing or source-text matching.
- Prediction probe loose/strict reports remain distinct from deterministic runtime corpus.
- agents-minimal.yaml support/retirement decision follows actual owner/consumer evidence, with no silent deletion or speculative gate.

## Constraints
D01–D08 remain binding. No new service/crate, protocol organization, permission/default-deployment change, version field, compatibility shim/alias/dual track, or Room feature work. Preserve transport envelope differences that have real purpose. Migrate every affected caller for any contract cutover; do not rewrite historical releases. Reuse existing patterns and tests; permanent tests must catch a plausible behavior regression. Workers skip builds/tests/fmt/lint; Main runs integration validation and real isolated probes. Do not touch the remote production deployment.

## User sequencing and communication
Current package: WP4-A. Future packages: WP-R core remote Room capabilities, then WP4-B internal Protocol organization (user wants both despite roadmap optional classification for B). Do not start either silently. Continue the existing KDE Connect convention only if an actual material user decision is blocked, never for routine updates.

## Baseline
Starting implementation history includes WP3 repair35b50ce and completion records3a5dfea. Initial diff showed only this new plan selector; no pre-existing tracked source edits. Active PLAN_ID=2026-09-21-wp4a-contract-parity. Relevant decisions, target ownership and full WP4-A roadmap have been read.
