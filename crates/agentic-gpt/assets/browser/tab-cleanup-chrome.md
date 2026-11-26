# Tab Cleanup
- Agent-created tabs are ephemeral and close automatically when you explicitly release the lease with `browser.release`, unless you mark them. Returning from `browser.repl` leaves them open.
- Call `tab.markDeliverable()` when the live tab itself is a user-facing output or requested open page, such as a created or edited document, spreadsheet, slide deck, dashboard, checkout, submitted form result, or a page the user explicitly asked to keep open.
- Call `tab.markHandoff()` only when work must continue from the live page in a later turn, such as a page waiting for user input, login, approval, payment, CAPTCHA, or an unfinished workflow.
- Marks are lease-scoped, and the latest mark for a tab wins. To resume a handoff, acquire the same profile from the same paired Agentic instance with the same lease name. The tab appears in `browser.tabs.list()` with its previous handoff mark cleared. Re-mark any tab that must survive the next release.
- Do not mark research, search, source, intermediate, duplicate, blank, error, or routine navigation tabs. Once you have extracted what you need, let `browser.release` close them.
- Claimed user tabs that are not marked are released from browser-session control and left open.
- Disconnecting or letting the lease expire leaves the tabs open. After Chrome restarts, reacquire the profile and use a fresh `browser.user.openTabs()` listing to find any pages Chrome restored.
