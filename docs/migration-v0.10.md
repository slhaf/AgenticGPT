# Agentic GPT v0.10 migration notes

The file surface now keeps single reads and searches ergonomic while adding
bounded ordered requests to those same tools. The former combined file batch
tool is removed.

- Use `file.read` with flat fields for one request or `requests` for 1–32 reads.
- Use `file.search` with flat fields for one request or `requests` for 1–32 searches.
- Keep the two forms mutually exclusive; batch envelopes preserve order and
  isolate per-item failures.
- Use `file.edit` with Codex apply-patch text for add, update, delete, and move
  operations across multiple files. Old mode/path/revision/content fields are
  no longer accepted.
- `dryRun` validates and previews without confirmation or writes. `needConfirm`
  requests one confirmation for the complete effective patch.

Callers should inspect ordered `results` for read/search requests and ordered
`changes` plus `summary` for edits. Parse, path/context, staging, confirmation,
and final revalidation failures happen before the first physical commit and
therefore write nothing. No cross-file rollback guarantee is made after commit
begins: if a later physical commit fails, `file.edit` returns
`completed_with_errors` and records which ordered changes committed, which one
failed, and which later changes were skipped without being attempted.
