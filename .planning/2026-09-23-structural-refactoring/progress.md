# Progress

## 2026-09-23
- User authorized structural implementation and specifically requested WP4-B Protocol internal organization. Continued from source-grounded whole-repository architecture reassessment.
- Read planning skill, initialized separate plan, checked existing diff (only unrelated prior WP-R findings), used CodeGraph before targeted code inspection. Froze disjoint worker file ownership and unchanged wire/public-surface contracts. No implementation or runtime checks yet.
- Spawned five independent implementation owners in one batch: Agent operation family, Hub neutral projection, Protocol WP4-B, Android local transitions and release preflight. Workers skip validation and commits; Main owns dependent Job config seam.
- Added a regression scenario that admits `printf`, changes live policy to deny before the current-thread async worker runs, and expects the admitted process to retain its decision. Reproducing pre-fix behavior in detached HEAD worktree `/tmp/agentic-config-baseline-20260923`; no worker edits/jobs conflict there.
- Initial reproduction command used `--exact` and filtered out the namespaced test (zero executed); reran without `--exact`. Pre-fix isolated test failed as intended: `Rejected` vs `Completed` (artifact://569). The failure is deterministic on a current-thread runtime; no assumption from source alone.
- Assigned a sixth independent writer solely to jobs/config/main/supervisor. Chosen behavior: admission-time effective config for each Process/Skill Job and one preflight snapshot for batch queued workers; later new admissions see live reload. Main continues integration/documentation.
