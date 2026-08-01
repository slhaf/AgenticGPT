# Agentic GPT v0.10 migration notes

The v0.10 file-batch contract changes the mutation boundary from a best-effort
cross-file rollback coordinator to independent normalized file groups.

## `file.batch`

- Keep the flat ordered `operations` array; there is no `atomicity` selector.
- Repeated edits to one normalized target run in input order against one
  in-memory candidate and produce one physical replacement.
- Reads and searches retain the pre-edit snapshot and their failures no longer
  prevent unrelated valid file groups from committing.
- A failed group is isolated. Other groups may commit, and a mixed result is
  reported as `completed_with_errors` with ordered operation errors and bounded
  `groups` summaries.
- Confirmation is one aggregate decision over valid effective groups. A denial
  writes none of the staged groups.
- `dryRun:true` is the supported whole-request validation and preview mechanism;
  exact `expectedRevision` and `expectedAbsent:true` guards remain authoritative
  for a later real call.
- The old `rolled_back`, `partial_failed`, `rollback_failed`, and
  `not_committed` states and rollback-only error codes are removed. A normal
  commit failure leaves already committed groups unchanged and reports the
  affected group as failed.

Consumers should branch on `completed`, `completed_with_errors`, `rejected`,
and `dry-run`, inspect `groups` for physical per-file state, and never infer
cross-file atomicity from a successful batch response.
