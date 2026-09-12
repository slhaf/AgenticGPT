# Terminal Cutover Recovery Forensic Report

> Final forensic checkpoint: the structured mutation/side-effect audit is complete. Evidence supports a near-exact recovery; the only unresolved issue is the recovered lease test behaving differently from the original late passing run. The recovery source tree was not modified during the audit.

## Executive conclusion

**Current verdict: near-exact recovery, with one unresolved discrepancy requiring explanation; not materially incomplete on the evidence available.**

The recovery tree is preserved at branch `recovery/terminal-pre-restore-2026-09-11`, commit `46d700c` (`recovery: preserve terminal cutover before accidental restore`). Strong evidence establishes that the recovered tracked diff, recovered Terminal untracked files, compiler warning locations, and `cargo check -p agentic-gpt` result correspond to the destroyed pre-restore state.

The original session chronology now proves that the lease test passed at `2026-09-11T09:40:05.888Z` (the task's local 17:40:05 CST event), after the `09:39:46.736Z` edit. The final `09:40:15.325Z` edit only removed test-only diagnostic `eprintln!` calls; it did not change lease logic. Therefore the recovered tree's current failure (`Available`, `holder=None`, `revision=2` instead of `Grace`) is not explained by a final edit intentionally reintroducing the bug. The leading unresolved possibilities are an omitted non-write/edit filesystem side effect, a replay/environment difference, or a mismatch elsewhere that structural evidence has not yet isolated.

It is safe to use this tree as a **forensic continuation point**. Exact byte identity is not yet proven, and the discrepancy must be preserved as a release-blocking forensic finding until the mutation audit is complete.

## Incident timeline

- Recovery tree during forensic replay: `/tmp/agentic-terminal-replay`; preserved continuation worktree after cleanup: `/home/slhaf/Projects/AgenticGPT/.worktrees/terminal-recovery-2026-09-11`.
- Destroyed worktree: `/home/slhaf/Projects/AgenticGPT/.worktrees/terminal-block-runtime`.
- Original main session: `/home/slhaf/.omp/agent/sessions/-Projects-AgenticGPT-.worktrees-terminal-block-runtime/2026-09-05T09-16-01-064Z_01a070da-78a8-7028-a867-e2d44410c911.jsonl`.
- Frozen plan: `/home/slhaf/Projects/AgenticGPT/.planning/2026-09-terminal-runtime-cutover/{task_plan.md,findings.md,progress.md}`.
- Snapper baseline: snapshot `#3803`, approximately `2026-09-11 17:00:01 CST`.
- Replay interval: successful OMP write/edit results after the baseline and before the destructive restore, replayed in result-time order; failed writes/edits and `xd://` pseudo-writes were excluded.
- Destructive restore: approximately `2026-09-11 17:40:59 CST`.
- Concurrent tool calls existed, but the recovery audit already established no overlapping successful mutations to the same product file in the replay window.

The original session JSONL records timestamps in UTC. The task chronology labels the corresponding incident window as CST; the relevant sequence below is shown in JSONL UTC and maps to the task's local 17:xx sequence.

## Mutation completeness audit

Confirmed so far:

- The recovery procedure replayed successful OMP write/edit results mechanically and excluded failed write/edit calls.
- The reconstructed tree includes tracked changes and new Terminal files listed below.
- The task explicitly requires auditing bash and other filesystem-capable operations; that audit is not yet complete in this checkpoint.
- Known classes requiring explicit classification: read-only inspection; Cargo build/test artifacts; source-mutating `cargo fmt`/`rustfmt`; Cargo.lock changes caused by Cargo; shell/file operations such as `rm`, `chmod`, `cp`, `mv`, redirection, patching, or scripts. No conclusion about omitted product-file effects is recorded here until structured command extraction is complete.

No source/product file has been modified during this audit. Only this report has been created/updated.

## Console lease-test chronology

All entries below are extracted from structured JSONL records. Test completion records, rather than launch-call success, determine pass/fail.

1. `2026-09-11T09:36:19.138Z` — test launched: `cargo test -p agentic-gpt console::tests::two_clients_observe_lease_denial_and_grace_resume -- --nocapture`.
   - Launch tool call ID: `call_zJwj3WK3NRga9LugWDFGVf37|fc_03261ddb7b30c940016aa3cb918f6c87d2adb96fe922cf15b8`.
   - Completion/wait record: `2026-09-11T09:36:28.181Z`, wait call `call_jRDSkGPvNOs21Pn6b6QjLtvM|fc_03261ddb7b30c940016aa3cb9b9e4087d281239800dbdbe191`.
   - **Failed**: panic at `console.rs:1770`, `lease did not enter grace: Elapsed(())`; result `0 passed; 1 failed`, finished in `2.37s`.

2. `2026-09-11T09:37:02.639Z` — `console.rs` edit, call ID `call_Bgd0TdtHAYTdVbkDD3brqjFS|ctc_03261ddb7b30c940016aa3cbbc73c487d286dd50f5d875cf9c`.
   - Affected test region: around lines 1759–1775.
   - Semantic purpose: explicitly `shutdown`/drop the first client, await the server task observing disconnect, then poll for grace instead of checking before disconnect propagation.
   - This edit followed the failed run above.

3. `2026-09-11T09:37:08.278Z` — same test launched, call ID `call_C5ZjaF4bLi6PegviaBRmPVC1|fc_03261ddb7b30c940016aa3cbc2c51c87d29e4cda12b1c8d5b1`.
   - Completion: `2026-09-11T09:37:16.914Z`, wait call `call_tKZziOV7yX8CC5Xawr91j0GK|fc_03261ddb7b30c940016aa3cc47c087d29dcf008561367e78`.
   - **Failed**: panic at `console.rs:1776`, `lease did not enter grace: Elapsed(())`; result `0 passed; 1 failed`, finished in `1.47s`.

4. `2026-09-11T09:37:28.924Z` — `console.rs` edit, call ID `call_6pVcCOEMAvlywWZZ0gElufd6|ctc_03261ddb7b30c940016aa3cbd676c887d29c85478626ff3513`.
   - Affected test region: around lines 1766–1776.
   - Semantic purpose: replace the one-second grace polling timeout with an immediate snapshot/assertion and include `holder={:?} revision={}` diagnostics.
   - Diagnostic-only change; it followed the failed run above.

5. `2026-09-11T09:37:33.547Z` — same test launched, call ID `call_Xzo9HDT7gcH5pSuaolnun8yK|fc_03261ddb7b30c940016aa3cbdbefd087d2bfbff9f7c7f75a02`.
   - Completion: `2026-09-11T09:37:41.318Z`, wait call `call_6gTdMkpJ71E7187lLLixT2f4|fc_03261ddb7b30c940016aa3cbbe13e9487d2be56b04c6c487503`.
   - **Failed**: assertion at `console.rs:1767`; actual `Available`, `holder=None`, `revision=2`, expected `Grace`; result `0 passed; 1 failed`, finished in `0.49s`.

6. `2026-09-11T09:37:46.070Z` — original session ran `rm -rf .planning/terminal-rendering-fix`. This affected planning/ephemeral state, not a recovered product file, but remains part of the non-write/edit mutation classification.

7. `2026-09-11T09:38:18.936Z` — `console.rs` edit, call ID `call_7kq3Yr2107TAWmj9gDFdNXac|ctc_03261ddb7b30c940016aa3cc084e5887d285c056ced27995e5`.
   - Affected connection loop around lines 366–373 and frame helper area around 1442.
   - Semantic purpose: classify transport disconnects with `is_transport_disconnect(&error)` and break normally, while preserving fatal frame errors as explicit disconnects. Intended to preserve grace on normal socket loss.
   - Followed the failed run above.

8. `2026-09-11T09:38:26.155Z` — same test launched, call ID `call_9gp4tlelt4eQZ9xyovrr07Vj|fc_03261ddb7b30c940016aa3cc1082ac87d2a6e0145a6083a4ec`.
   - Completion: `2026-09-11T09:38:38.171Z`, wait call `call_2Y8wmH9Gn4QokD0x3TghVXPb|fc_03261ddb7b30c940016aa3cc15298487d282fd76e8f7ff3181`.
   - **Failed**: assertion at `console.rs:1782`; actual `Available`, `holder=None`, `revision=2`, expected `Grace`; result `0 passed; 1 failed`, finished in `0.48s`.

9. `2026-09-11T09:38:59.078Z` — `console.rs` edit, call ID `call_Owo6UsjyaT6QBbHaf6CWZxXQ|ctc_030df238488d6c88016aa3cc31450487d29399be9fd46cb445`.
   - Affected connection-loop branches around lines 370, 379, and 386.
   - Semantic purpose: add `#[cfg(test)] eprintln!` traces for explicit disconnect paths.
   - Followed the failed run above.

10. `2026-09-11T09:39:04.430Z` — same test launched, call ID `call_hpKz2uhbSPKRhPTMUIqiYMYJ|fc_030df238488d6c88016aa3cc37250087d2ac55d96f35fcf8d8`.
    - Completion: `2026-09-11T09:39:19.260Z`, wait call `call_ge2dJv09IjtmHgEHFT6HqEgb|fc_030df238488d6c88016aa3cc46522c87d28fbf9f06babd665b`.
    - **Failed**: assertion at `console.rs:1788`; actual `Available`, `holder=None`, `revision=2`, expected `Grace`; result `0 passed; 1 failed`, finished in `0.51s`.

11. `2026-09-11T09:39:46.633Z` — `console.rs` edit, call ID `call_rYd9ZVSKmlO7zOSLnDb3tvVg|ctc_030df238488d6c88016aa3cc60d92c87d293dcbbffee310544`.
    - Affected test region around lines 1719 and 1778 onward.
    - Semantic purpose: clone `first_connection_id` for the first input request and add an explicit held-state snapshot asserting the holder is the first connection before disconnect.
    - Followed the failed run above.

12. `2026-09-11T09:39:56.858Z` — same test launched, call ID `call_Z9uMXonZfxQOvBdsjB1nA1SN|fc_030df238488d6c88016aa3cc6c761c87d2ace9bea56b9e0e43`.
    - Completion: `2026-09-11T09:40:05.888Z`, wait call `call_QSkOK5fSfA4tvHMYmFhiBvqx|fc_030df238488d6c88016aa3cc70284487d29fb2cf6073c29433`.
    - **Passed**: result text was `cargo test: 1 passed (4 suites, 369 filtered, 20 warnings, 0.00s)`; completion record marked the job `completed`, not `failed`.
    - This is the task's 17:39:56–17:40:05 local event and resolves the key A/B question: the apparent late pass is real in the original result content.

13. `2026-09-11T09:40:12.067Z` — read of `console.rs` connection loop, call ID `call_KimD11SccydEUIKFZZCGexsJ|fc_030df238488d6c88016aa3cc7b6f9487d288f06feaa49a892a`.

14. `2026-09-11T09:40:15.268Z` / result `09:40:15.325Z` — final `console.rs` edit, call ID `call_pfNcLdTzj0fq6yD2XtLZUmjy|ctc_030df238488d6c88016aa3cc7e252887d2ba8401d8427b5d20`.
    - Affected branches around lines 371, 382, and 391.
    - Semantic purpose: remove the three test-only `eprintln!` diagnostics added at `09:38:59`; the resulting diff retains the normal/fatal disconnect control flow. The tool-result diff contains only deletion of those diagnostic statements.
    - This edit occurred **after the passing test** and does not semantically reintroduce the observed `Available` failure.

15. `2026-09-11T09:40:27.752Z` — `cargo check -p agentic-gpt` launched; completion `09:40:33.215Z`, call `call_7SpoiRsmgtDk0vBNF6ySVccN|fc_030df238488d6c88016aa3cc90988087d2a3a447d68d9d0697`; **completed successfully** with the warning locations listed below.

16. `2026-09-11T09:40:45.879Z` — `git status --short`, completion `09:40:45.921Z`, call `call_fSo4kJ0pvYp6SFPkBM0uaNXy|fc_030df238488d6c88016aa3cc9d301887d2b8834680200b35bb`; observation: staged `0`, unstaged `30`, untracked `7`.

17. `2026-09-11T09:40:59.844Z` — `git diff --numstat`, completion `09:40:59.916Z`, call `call_BIESRIVKL9p9dhPYR1moyjz0|fc_030df238488d6c88016aa3ccab266c87d2bb75f8267f8a7e04`; observation: `30 files`, with the key counts below. This is the last visible pre-cleanup scope capture.

**Why the recovered tree currently fails:** the original session proves a pass before the final diagnostic-removal edit, while the recovered tree's current replayed state fails with the earlier `Available` observation. The final edit is semantically non-causal. A source-side discrepancy or omitted side effect remains to be located; an unqualified “the final edit reintroduced it” conclusion is disproven.

## Recovery equivalence evidence

Confirmed evidence supplied by the recovery process and frozen in the task:

- Recovered tracked `git diff --numstat` matches the pre-restore capture, including:
  - `Cargo.lock`: `+1206 / -24`
  - `crates/agentic-gpt-protocol/src/lib.rs`: `+1210 / -1`
  - `crates/agentic-gpt-hub/src/mcp_server.rs`: `+473 / -3`
  - `crates/agentic-gpt/src/stdio_server.rs`: `+451 / -7`
  - `crates/agentic-gpt/src/terminal_runtime.rs`: `+1486 / -277`
  - plus the remainder of the 30 tracked modified files.
- Recovered new Terminal files include `console.rs`, `terminal_manager.rs`, `terminal_repl.rs`, `terminal_screen.rs`, `terminal_source.rs`, `tui/terminal.rs`, and `vendor/`.
- Known recovered line counts: `terminal_manager.rs` 2582; `terminal_repl.rs` 225; `terminal_source.rs` 547.
- `cargo check -p agentic-gpt` succeeds on the recovered tree.
- The original pre-cleanup check emitted warning symbols/locations matching the recovered tree: `InputRecord.input` (~82), `send_input` (~520), `resize` (~539), `reconcile_request` (~2346), `active_lifecycle_block` (~2433), and `LifecycleCwd` (~2529) in `terminal_manager.rs`; `cursor_changed` (~158) in `terminal_screen.rs`; and `detail_active` (~354) in `tui/terminal.rs`.
- The original final status captured `30` unstaged tracked files and `7` untracked files; the reconstructed scope was reported as matching this state.

These establish strong structural and behavioral correspondence. They do not prove byte identity or explain why the recovered replay does not reproduce the original late pass.

## Mutation completeness audit

This section is intentionally still open. The original session contains filesystem-capable calls beyond OMP `write`/`edit`, including Cargo commands, formatter commands, shell commands, and process/debug commands. The remaining audit must classify each target-window call by actual command and result, then compare any source/product effect with the recovered tree. In particular, `cargo fmt`/`rustfmt`, Cargo.lock updates, `rm -rf` of planning state, `chmod`, shell redirection/scripts, and vendoring must not be silently treated as read-only.

## Remaining uncertainty

- Exact cause of the recovered-tree failure despite the original `09:39:56.858Z` run passing and the `09:40:15.325Z` edit being diagnostic-only.
- Whether a formatter or another shell-side mutation changed source bytes outside successful OMP write/edit results.
- Whether the original passing command used a different effective binary/build artifact or runtime state; its output is genuine, but the terse wrapper output does not itself expose all test stdout/stderr.
- Whether source hashes or tagged reads captured before restore can prove byte identity.
- Whether all late tool events are inside the stated replay cutoff after timezone normalization.

## Frozen-plan implementation snapshot

The frozen plan/progress records the intended cutover as a persistent PTY/terminal architecture: a `TerminalManager` owning a portable-pty, `wezterm-term` as authoritative live screen state, semantic lifecycle observation, full-width Attach, a temporary Browse inspector, separate screen/history/log transports, generation/revision state, bounded history/frames, `terminal.repl`, and lease/arbitration routing.

The frozen findings record these visible pieces as reusable at cutover: lazy `AppState`/manager ownership; portable-pty child handling; Fish OSC 7/133/1337 hooks and block metadata; console socket authentication/framing/reconnect; top-level route/chrome; and master/detail Browse UI.

The same frozen findings explicitly identify unfinished integration areas at the instant before cleanup: transcript-backed `tui/terminal.rs` instead of emulator cells; TUI-owned `input: String`; workspace shortcuts intercepting ordinary keys; snapshots combining display and all block transcripts with possible >1 MiB cloning; missing prompt-gated scheduler and human/agent arbitration; resize using outer geometry and updating only PTY; reader EIO/EOF/child lifecycle reconciliation needing explicit handling; and no complete `terminal.repl` model surface. This is a historical conformance snapshot, not a request to fix those gaps during the audit.

## Recommended next action

Complete the read-only structured mutation audit and source-equivalence comparison, preserving the current recovered tree unchanged. Isolate the late-pass discrepancy before any product edit; then continue implementation from this recovery tree only. Do not perform that action as part of this forensic audit.


## Audit checkpoint: structured mutation scan

The original main session plus all subagent JSONL artifacts were scanned structurally for `2026-09-11T09:00:00Z` through `2026-09-11T09:40:59Z`, the JSONL representation of the task's 17:00:01–17:40:59 CST window.

- Tool execution starts in this interval: 28 `bash` calls, 150 `edit` calls, and 8 `write` calls across the main session and subagents.
- Edit results: 144 successful, 6 rejected/error results. Write results: 8 successful, 0 error results. The six failed edits were stale-hash, changed-file, or malformed-hunk rejections; they do not report a successful source mutation and were correctly excluded from replay.
- The 28 bash commands were limited to Cargo test/check, `which fish`, `which strace`, `pgrep -a fish`, and the timed `script`/Fish probe. No `cp`, `mv`, `rm` of a product file, `sed -i`, `perl -pi`, Python rewrite, shell redirection, `git apply`, `git restore`, `git checkout`, `git clean`, `cargo fmt`, `rustfmt`, `cargo add`, or `cargo update` command occurred in this target window.
- `rm -rf .planning/terminal-rendering-fix` occurred at `2026-09-11T09:37:46.070Z`; it removed planning/ephemeral material, not a product file.
- Cargo commands could write `target/` and Cargo dependency-resolution metadata. The recovered `Cargo.lock` was explicitly brought to the cargo-check-produced state; no other Cargo-generated source mutation was found.
- Subagent bash calls were observation/build/test only. Their source mutations were OMP `edit`/`write` results and are covered by the successful-result replay.

This scan finds no non-write/edit product-file mutation omitted from the replay. The conclusion is high confidence for the enumerated target window; it does not yet prove byte identity of every successful edit replay.

## Audit checkpoint: source inventory and discrepancy

Read-only inspection of the recovered tree confirms the planned source surfaces are present: `portable-pty`, `wezterm-term`/`wezterm-surface`, the `TerminalManager` observation fields for screen generation/revision, lifecycle/history/lease state, `TerminalScreenAdapter` and screen transfer types, Attach/Browse console modes, and `terminal.repl` dispatch. The recovered inventory measured `terminal_manager.rs` 2582 lines, `terminal_repl.rs` 225, `terminal_source.rs` 547, `terminal_screen.rs` 1632, and `tui/terminal.rs` 1747.

Current recovered hashes (inventory only; no matching pre-restore cryptographic hashes were found in the session scan):

```text
eb274979986da76a758e64311fd8d67391dd67c430da516a48ff64c6c55bfe46  crates/agentic-gpt/src/console.rs
9bd2e0393480b6e03d8f99a6dbf447e207f5d1a03e24ef9b0322f868bb5471a6  crates/agentic-gpt/src/terminal_manager.rs
1e61d296a6bd104680ab8e0d7b3f933eb43fcb59f538c637a9955d373b36d409  crates/agentic-gpt/src/terminal_repl.rs
dfcd31b940964260e16cbed3daf1c3be0d715e319b7e2379a5f12bad2c556f72  crates/agentic-gpt/src/terminal_screen.rs
266839c8c3077c4bc94e415090448cd3e9dd7b07791ee3226cf87e9bf7f23ac5  crates/agentic-gpt/src/terminal_source.rs
5fb4edef41917a721cdeec90ad2738a3f748ec1df29197f0d1999a3ee8f413f9  crates/agentic-gpt/src/tui/terminal.rs
3d2f31c2ac5b931975397d545f23fd7fa1e8e56b0f552404ea917ab8577f03e2  Cargo.lock
```

The recovered `console.rs` contains the final normal/fatal disconnect branches with no test-only diagnostic `eprintln!` calls, while the lease test contains the held-state assertion and the immediate Grace assertion. This matches the semantics of the `09:40:15.325Z` final edit result. The original raw `217.bash-original.log` independently shows the prior test compiling, running exactly one targeted test, and reporting `test ... ... ok` / `1 passed; 0 failed`.

The unresolved discrepancy is therefore narrowed: replay omission of a successful OMP edit is not indicated by the command scan, but cryptographic pre-restore byte identity is unavailable and an environmental/build-state difference cannot yet be excluded.


## Audit checkpoint: captured diff comparison

The raw final scope capture `223.bash-original.log` contains 30 tracked-path `git diff --numstat` lines. A read-only comparison against `git diff --numstat HEAD^ HEAD` for those exact 30 paths in the recovered commit produced:

```text
captured_lines 30 recovered_lines 30 exact True
mismatches []
```

This is stronger than matching only aggregate counts: every captured path's insertion/deletion pair and ordering match the recovered commit. The recovered commit additionally contains the seven new/untracked-at-capture Terminal/vendor path groups, so its commit diff has more paths than the original tracked-only `git diff --numstat` output.

The structured command scan covered the main JSONL and all subagent JSONLs in the normalized target interval. It found no shell command capable of changing a product file outside the replayed OMP edit/write results. The only direct deletion was the planning-directory `rm -rf`; Cargo's permissible generated effects were target/dependency metadata and the already-recovered Cargo.lock state.


## Audit checkpoint: destructive cleanup boundary

Structured extraction of the main session after the last scope capture records the destructive boundary precisely:

- `2026-09-11T09:41:25.498Z`, call `call_uC3HU4ffZrvDSzMobt7L05O7|fc_030df238488d6c88016aa3ccc1f80c87d2aacc88e804336eb6`: `git restore --` followed by the tracked product paths. Result at `09:41:25.543Z`: no output, `isError=false`.
- `2026-09-11T09:41:41.524Z`, call `call_uv2QCjReD3d1Qu6GDno1idfb|fc_030df238488d6c88016aa3ccd35d0887d292adf0c84a1fa641`: `rm -rf` over the six new Terminal source files and `vendor/`. Result at `09:41:41.548Z`: no output, `isError=false`.
- A second restore at `09:42:11.075Z`, call `call_WekBMOFCyXLn2UfKTgE8VdkO|fc_030df238488d6c88016aa3ccf1f98c87d2a5735c7343beebd9`: restored `Cargo.toml`, `Cargo.lock`, `crates/agentic-gpt/Cargo.toml`, and `crates/agentic-gpt/src/main.rs`; result `09:42:11.122Z`, no output, `isError=false`.

These destructive commands occur after the `09:40:59.916Z` final diff result and therefore after the stated replay cutoff. They explain the destroyed worktree and are not omitted pre-cutover mutations.


## Audit checkpoint: conclusions by question

1. **Mutation completeness:** for the normalized replay window, no product-file mutation outside successful OMP `edit`/`write` results was found. The 28 bash calls were build/test/observation only; the one `rm -rf` removed planning state. The successful OMP result set was 144 edits and 8 writes; six edit failures were rejected and no write failed. The recovery procedure's stated replay method covers the successful set. Cargo-generated target/dependency state is not product source; Cargo.lock's recovered state was explicitly synchronized to the cargo-check result.

2. **Console chronology:** the original sequence was four failures (`09:36:19`, `09:37:08`, `09:37:33`, `09:38:26`), one failure after diagnostic tracing (`09:39:04`), then a real pass (`09:39:56` launch, `09:40:05` completion). The `09:40:15` final edit removed only three test-only diagnostic prints. The original raw stdout proves the pass; it was not inferred from a successful launch call.

3. **Recovery exactness:** the captured 30-line tracked numstat is byte-for-byte equal as a numstat record to the recovered commit for the same paths (`exact True`). Warning locations, line counts, source inventory, and successful `cargo check -p agentic-gpt` also match. No pre-restore cryptographic file hashes were found, so exact bytes remain unproven. Verdict remains near-exact, not exact.

4. **Missing non-write/edit effects:** no formatter, patcher, file-copy/move, product deletion, shell rewrite, Cargo add/update, or Git restore/checkout/clean occurred before the cleanup boundary in the normalized window. The destructive `git restore` and `rm -rf` commands are post-window and were independently observed.

5. **Frozen-plan conformance:** the recovered implementation visibly contains the planned PTY, WezTerm screen/parser, semantic lifecycle, Attach/Browse, bounded transport/history, and `terminal.repl` surfaces. Frozen findings simultaneously record unfinished integration in the live TUI input/rendering path, prompt/arbitration scheduling, resize/lifecycle reconciliation, snapshot layering, and complete repl model surface. This was a cutover foundation with known unfinished integration, not a complete clean-cutover implementation.

**Disposition:** A (the late original pass was real) is established; B (late pass was misread) is rejected. C (omitted non-write/edit source mutation) is not supported by the structured command scan. D (runtime/build/environmental difference) remains the only non-source explanation currently evidenced as possible, alongside the narrower possibility of a same-numstat but byte-different replayed edit. Do not silently “fix” the recovered failure during forensic work.


## Final confidence checkpoint

- **Recovery confidence:** high for mutation-set and tracked-diff equivalence; medium for exact byte identity because no pre-restore cryptographic hashes were captured.
- **Behavioral discrepancy confidence:** high that the original late pass was real and that the final edit was diagnostic-only; low on the root cause of the recovered-tree failure until an independent environment/build comparison is performed.
- **Safety:** no recovered source/product file was changed during this audit. The working tree contains only the authorized untracked report.
