# Room API V2 — Progress

- 2026-09-11: optional toolsets completed, independently reviewed/fixed, verified with 433 workspace tests, and committed as `23a46c5 feat: add configurable optional toolsets`.
- Reviewed current Room repository manuals/workflow and the complete Hearth hook/procedure set.
- Reviewed frozen `refactor/room-markdown-repository` implementation to identify reusable repository-read code versus obsolete old-world mutation/Diary semantics.
- Refined V2 after architecture discussion: Room is local-first. New/empty roots receive a complete versioned generic scaffold and deterministic initial commit; existing unknown content is never silently baselined. `room.maintenance.submit` remains the semantic request boundary, while execution is configurable as local (default) or workflow. Local execution applies/commits immediately and remote auto-push is optional; workflow execution uses push + remote writer. Remote failure never rolls back a successful local apply.
- Canonical maintenance executor should live at `scripts/apply_maintenance.py` and be shared by local Agentic execution and the GitHub workflow; `.github` is transport, not the semantic implementation.
- Next: hand the frozen plan to OMP for Phase 1 implementation; no product code has been modified in this V2 workstream yet.
