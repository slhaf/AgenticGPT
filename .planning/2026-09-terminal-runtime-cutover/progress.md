# Terminal Runtime Cutover Progress

## 2026-09 planning session

- Created canonical planning under the repository root; implementation remains in `.worktrees/terminal-block-runtime`.
- Read the 2026-08 inherited Terminal baseline and the two 2026-09 review/refinement documents.
- Separated inherited architectural decisions from the new Attach/Browse, revision-generation, InputArbiter, bounded-history, and feature-boundary refinements.
- Drafted goal, contracts, non-goals, clean migration boundaries, and behavioral acceptance criteria.
- Status remains `planning` until every blocking decision is evidence-backed and all three planning files are checked for consistency.
- No new implementation started in this planning session.

## Current implementation state entering cutover

- Uncommitted worktree contains the persistent PTY/TerminalManager, console endpoint, Terminal route, transcript-backed screen, and focused tests.
- Correct foundations will be retained; transcript-screen protocol/UI will be replaced without compatibility shims.

- Closed dependency source/feature boundary with pinned WezTerm git revision and explicit acceptance of unavoidable image compile dependencies outside the Agentic DTO/rendering boundary.
- Closed live-screen transport with protocol-owned cell/style DTOs, generation/revision validation, and atomic row-chunked transfers.
- Closed history retention/paging and `terminal.repl` schema/policy/arbitration decisions with concrete limits.
- Started independent planning-consistency and implementation-readiness audits; status remains `planning` until both return.
- First independent consistency audit returned FAIL. It found missing public result/output follow-up, console lease/session/ack semantics, lifecycle states/events, Browse scrollback paging, geometry overflow behavior, and provenance/state mismatches.
- Corrected the plan with strict `terminal.repl` submit/status/cancel/log-page operations, bounded model-readable output, request pagination/retention/deadlines, full-source confirmation binding, V2 console handshake/lease/no-replay semantics, exact Fish lifecycle mapping, active-screen Browse paging, explicit geometry rejection, and September closure provenance.
- Recorded WezTerm public-API limits: no inactive-primary accessor, no private wrap-pending cursor state, no Sixel/iTerm disable hook, and edition-2024 Rust 1.85+ requirement.
- Re-review completed after all corrections.
- Closed later audit findings: hybrid exact inline style overflow, authenticated FIFO+PTY lifecycle boundaries, conservative builtin auto-allow list, bracketed-paste source integrity, public Hub/local ingress identity, and bounded vendored WezTerm parser accumulation.
- Independent planning-consistency and implementation-readiness audits returned PASS with no residual blocker or user-choice decision.
- Canonical status is now `implementation_ready`. No production source implementation was performed during this planning cycle.

## 2026-09-11 recovery checkpoint and pause

- OMP Main destroyed most of the in-progress Terminal cutover after context compaction: it re-adopted an obsolete spike constraint, ran `git restore` on tracked changes, and deleted the new Terminal source/vendor paths. The original `.worktrees/terminal-block-runtime` remains preserved as the damaged incident scene and must not be treated as the recovery source.
- Recovery was reconstructed from Snapper `/home` snapshot `#3803` (about 17:00 CST) plus successful OMP write/edit results up to the pre-cleanup boundary. The reconstructed code is preserved on branch `recovery/terminal-pre-restore-2026-09-11`, commit `46d700c` (`recovery: preserve terminal cutover before accidental restore`).
- The durable recovery worktree now lives at `.worktrees/terminal-recovery-2026-09-11`; the temporary `/tmp/agentic-terminal-replay` path is retired after verified copy/repair. A separate compressed backup remains under `~/Backups/agentic-terminal-recovery/` with SHA-256 sidecar.
- Forensic report: `.planning/2026-09-terminal-runtime-cutover/FORENSIC_RECOVERY_REPORT.md`.
- Recovery evidence is strong: 144 successful OMP edits + 8 successful writes were replayed; 6 rejected edits were excluded; structured scan found no pre-cleanup shell-side product mutation outside that result set. The captured 30 tracked-path `git diff --numstat` record matches the recovery commit exactly path-by-path, and `cargo check -p agentic-gpt` passed with the same warning locations seen before cleanup.
- The only concrete unresolved discrepancy is `console::tests::two_clients_observe_lease_denial_and_grace_resume`: the original run at about 17:40:05 CST genuinely passed, and the final later `console.rs` edit only removed diagnostic prints, but the recovered tree currently reproduces `Available / holder=None / revision=2` where the test expects `Grace`. No source-recovery omission has been found to explain this; race/scheduling/runtime/build-state differences remain plausible.
- Do not “fix” this discrepancy inside the preserved recovery worktree. When work resumes, create a separate debug branch/worktree from `46d700c`, instrument lease acquire/holder/disconnect/release/grace/timer/clear transitions, and reproduce the failure there before changing behavior.
- Status at pause: recovered cutover code preserved, forensic audit complete enough to continue safely, lease discrepancy intentionally left open, no further Terminal work planned for 2026-09-11.
