# WP3 progress

## 2026-09-20
- User authorized starting WP3 and explicitly permits planning records in commits.
- Read planning workflow, confirmed decisions and complete WP3 roadmap requirements before implementation.
- Initialized PLAN_ID=2026-09-20-wp3-resource-durability; Main owns records and phase commits. Previous work packages remain complete.
- Initial tracked diff empty; no WP3 production edits, tests, builds or runtime probes yet.
- Completed four read-only inventories and committed authority/durability matrix + concrete ownership contracts as `0f5422b`.
- Dispatched five independent implementation slices (Hub cache, Hub DB/runs, Agent ledger, Agent config/setup, Agent history/audit/private-state) and an existing-docs external lifecycle slice. Workers skip validation; Main owns integration and runtime proof.
- Preserved pre-fix binaries outside the repo and ran a real Agent against isolated SSE: accepted ledger record followed by torn started JSON caused `/usr/bin/touch` to run again. Baseline reproduction passed, proving the replay bug rather than assuming it from source.
- Real pre-fix config mutation with a 16 MiB preserved future field was killed when the live file was truncated; target remained zero bytes and invalid JSON. Baseline reproduction passed. Post-fix confirmation pending.
- Initial Hub retention probe inspected at 2 seconds, before the existing 30-second sweep, so it did not reproduce deletion; corrected the probe to wait 32 seconds. No source change was made to shorten the production sweep.
- Actual Neko UID/GID/mount/access subjects remain unavailable locally; asked user for deployment facts. Historical Orange Pi smoke proves the shared path worked previously, not current identities. Sent one authorized KDE Connect reminder after waiting while continuing independent work; command exited successfully, device receipt not observed.
- Corrected 32-second real Hub baseline reproduced deletion of an expired `unknown` receipt, including its command hash.
- Baseline SSE-peer smoke confirmed timeout followed by late/duplicate/conflicting responses preserves canonical result, but flooding 4100 distinct Job ids left all 4100 cached. ACKed receipt remains `acked` after waiter timeout in this existing state machine; corrected the probe rather than changing code to fit an assumed status.
- User supplied the real deployment SSH destination. Read-only inspection confirmed Neko Docker native host and Chromium UID/GID1000:1000; shared bridge directory0755 and socket0660 with no ACL xattrs. Host root Agent and systemd-nspawn root Agent see exactly the same socket inode as Neko. Actual root `bridge.getStatus` returned `ok:true`; no browser navigation, restart, configuration or permission changes.
- Browser decision: preserve shared-filesystem/local-only topology and existing0660 access. Do not add token/peer-UID gate without evidence of a required access-policy change. Isolated same-user/different-user/shared-group tests remain pending.
- Critical review caught setup recovery racing a live writer and recovery evidence deletion after failed secret rollback; assigned both fixes and behavioral regressions to config owner before validation.
- Hub integration build exposed a missed `Option<&str>` conversion, a connection branch missing `None`, and cache sweep wiring accidentally replacing run cleanup. Corrected all before runtime proof; retained both periodic owners. Hub regression suite now passes 89 tests.
- Fixed Hub actual SSE smoke: 4100 distinct Job updates produce exactly4096 cached entries; fresh fallback reports cached+observedAt, then stale after60 seconds. Late/duplicate/conflict response scenario preserves canonical result. Actual15-minute TTL observation remains running.
- Fixed Hub32-second cleanup preserves expired unknown status+command hash. Real CLI migration from baseline user_version0 retains registry data and creates pre-migration backup; version999 is refused without reset.
- Isolated actual browser-host in shared Docker bind mount: host UID1000 succeeds; container1001:1001 gets EACCES;1001:1000 and1000:1000 succeed. Closing Native Messaging stdin removes the socket. Together with actual Neko read-only status/namespace inspection, this verifies the required filesystem access boundary without claiming browser-side effect rollback.
- Agent integrated build is in correction: compiler found deleted imports/constant, one missing map key and type inference failures in worker edits. No post-fix Agent runtime or test success claimed yet.
