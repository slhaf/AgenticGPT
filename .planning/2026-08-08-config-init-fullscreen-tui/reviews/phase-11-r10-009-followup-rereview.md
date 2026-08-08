# Phase 11 R10-009 Follow-up Scoped Re-review

- Reviewed HEAD: `4a23c0c`
- Repair range: `5d01977..4a23c0c`
- Review date: 2026-08-08
- Review mode: read-only, limited to the newly reproduced R10-009 symlink-parent/nonexistent-target case and the bounded repair. No product source, tests, or existing review artifact was changed by this review.

## Finding under review

`R10-009` remained partially resolved at `5d01977`: lexical aliases and existing-file identities were protected, but complete-path canonicalization returned no collision when both final targets were absent and only a parent directory was symlinked. A secret write could therefore create the real target before the config writer copied that secret into a backup through the alias path.

## Repair evidence

- `paths_refer_to_same_file()` retains the lexical absolute-path equality fast path.
- Existing targets still use normal full-path `fs::canonicalize()` identity comparison.
- For `NotFound` paths, `canonicalize_with_existing_ancestor()` removes missing components until the deepest existing ancestor is found, canonicalizes that ancestor, then appends the removed components in reverse order. This resolves `<root>/alias/config.json` and `<root>/real/config.json` to the same future target while the final file is absent.
- The helper preserves the prior non-`NotFound` fallback behavior instead of broadening validation or changing the config setup flow.

## Regression and verification evidence

- `cargo test --quiet -p agentic-gpt config_setup::outcome:: -- --test-threads=1`: 7 passed, including `symlink_parent_alias_to_nonexistent_target_is_rejected_before_secret_write`.
- The regression asserts the exact error string `config_init_secret_path_invalid`, that neither config nor secret path exists, and that no backup file contains `symlink-parent-secret-marker`.
- `cargo test --quiet -p agentic-gpt --bin agentic-gpt -- --test-threads=1`: 318 passed.
- `cargo test --quiet -p agentic-gpt --test config_cli -- --test-threads=1`: 14 passed.
- `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `git diff --check`: passed.
- The crate-level test command reached the known external `local_control` socket-readiness prerequisite after the 318 unit tests and 14 config CLI tests passed; it did not report a product-test failure.

## Scoped disposition

**R10-009: Resolved within the accepted scope at `4a23c0c`.** The repair closes the independently reproduced symlink-parent alias with a nonexistent final target, retains the prior lexical and existing-file protections, and proves that no secret write or secret-bearing backup occurs before rejection. No broader config setup redesign is required or included.
