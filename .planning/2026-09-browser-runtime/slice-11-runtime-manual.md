# Slice 11 — Runtime Manual Reader/Search

This is an orchestrator-owned implementation contract. It adds only the internal bounded documentation access layer consumed later by the frozen `browser.manual` tool. No model-facing Browser tool is registered in this slice.

## Goal

Read/search documentation from the `docs_root` of the currently selected `BrowserRuntimeDescriptor` without copying official runtime docs into AgenticGPT and without routing runtime-bundle paths through the user's workspace path policy.

The docs root is runtime data. This layer must not know or assume `~/.codex`; a later explicit-path or managed-cache runtime source must work unchanged.

## Module / boundary

Add a small internal Browser-manual module (repository-conventional name/location) whose public-to-crate API accepts an explicit docs root plus validated request data. It must not read `AppState` itself; the later tool layer will choose `state.browser_runtime.descriptor.docs_root` and call this module.

Support two operations internally:

1. **read** one UTF-8 documentation file by docs-root-relative path with optional inclusive 1-based `start_line` / `end_line`;
2. **search** a literal UTF-8 query across regular documentation files below docs root with bounded optional `max_results` and `context_lines`.

Do not add a third semantic Browser tool. A later `browser.manual` schema may expose these two operations through one tool.

## Path safety

- Reject absolute paths, empty paths, `.` / `..`, platform prefix/root components, and any path that escapes docs root.
- Reject a symlink at docs root, any traversed path component, and the final file for read. Search must skip symlink entries/directories rather than follow them.
- Require the read target to be a regular file.
- Do not canonicalize an untrusted path first and then accept it merely because its target lands somewhere; the runtime docs tree is treated as a non-symlink tree.
- Keep stable Browser-prefixed errors suitable for later dispatch. At minimum distinguish invalid path, docs unavailable/not directory, not-a-file, symlink rejection, non-UTF8, invalid range/query/bounds, and bounded read/search failure.

## Bounds

Keep constants local to the Browser manual layer. Current installed docs are small, but enforce defensive caps:

- single file input: <= 512 KiB;
- read output: <= 256 KiB UTF-8 bytes;
- search scan: <= 512 regular files and <= 16 MiB total bytes;
- search results: caller range 1..=100, default 50;
- search context lines: caller range 0..=5, default 2;
- search serialized/output payload: <= 256 KiB;
- query must be non-empty after validation and have a reasonable fixed UTF-8 byte cap (e.g. 4 KiB).

If a requested read range would exceed the output cap, return the largest whole-line prefix that fits plus bounded continuation metadata (`next_start_line` or equivalent), not an oversized result. A single line larger than the output cap must return a stable bounded-output error rather than splitting invalid UTF-8/line semantics.

Search skips oversized, non-UTF8, symlink, or unreadable files and returns bounded skip accounting; it must never follow symlinks or exceed file/byte/output/result limits. Literal matching is sufficient for V1; do not add regex/glob semantics here.

## Result shape

Use typed structs or stable JSON-friendly structs rather than preformatted prose.

Read result should include at least: relative `path`, returned `start_line`, returned `end_line`, `content`, and optional continuation line.

Search result should include matches with at least relative `path`, 1-based `line`, the matching line text, and bounded before/after context; also include truncation/skip accounting where applicable.

Do not expose the absolute docs root/runtime cache path in results.

## Tests

Use temp fixtures only; never depend on the developer's installed Desktop docs.

Cover at minimum:

- normal read and inclusive line range;
- output continuation bound and giant-line rejection;
- traversal/absolute/empty paths rejected;
- root/component/final symlinks rejected for read;
- literal search across nested docs with relative paths/context;
- search skips symlinks, non-UTF8, oversized/unreadable files as testable, with accounting;
- search query/result/context bounds;
- deterministic ordering (path then line) regardless filesystem walk order;
- no returned result leaks absolute docs-root path.

## Verification

- focused tests for the new Browser manual module;
- existing `cargo test -p agentic-gpt browser_` remains green;
- `cargo fmt --all -- --check`;
- `git diff --check`.

## Non-goals

- no `browser.manual` tool descriptor/schema/dispatch yet
- no Browser namespace/config/toolset changes
- no AppState changes
- no runtime discovery/provisioning changes
- no web docs fallback
- no documentation copy/vendoring
- no mutation/write operation
- no regex/glob search

## Worker rules

- Read `AGENTS.md`, frozen `PLAN.md`, this contract, current progress/findings, `browser_runtime.rs`, and only the relevant bounded-file patterns in `file_ops.rs` / `skills.rs` before editing.
- Keep implementation independent from concrete Codex/Desktop paths.
- Update `progress.md` at completion; `findings.md` only for a genuine contract/evidence contradiction.
- Do not modify PLAN.md or this frozen contract.
- No commit/push and no destructive git cleanup. Ignore the unrelated PoC `__pycache__` artifact.
