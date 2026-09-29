# Documentation audit

## Goal
Review current README and project documentation against implemented features; report concrete mismatches with document and implementation evidence, without modifying product documentation.

## Phases
1. Scope document inventory and assign independent audit slices — complete
2. Cross-check findings against code, commands, and adjacent contracts — complete
3. Deliver prioritized evidence-backed findings and obsolete-document inventory — complete
## Constraints
Read-only review of product code/docs. Separate agent slices: top-level READMEs/setup; runtime and tools docs; hub/API/operations docs; console and architecture docs; legacy migration/release/spec docs. Do not assert mismatch from outdated text alone. Identify whole documents already obsolete separately from specific inaccurate claims.
## Errors
- Default planning skill script absent at ~/.claude/skills/planning-with-files/scripts/resolve-plan-dir.sh; use isolated plan files directly.
