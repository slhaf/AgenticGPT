# External Browser Tab Claiming
- If the user identifies an already-open tab in a particular browser, use that profile. When acquiring a lease, match the requested profile in `browser.list` and pass its `browserId` to `browser.acquire`.
- Inside `browser.repl`, call `browser.user.openTabs()` and match the details the user supplied. When given a Chrome tab ID, match it against `providerTabId`. If the request also includes a title or URL, match those fields on the same entry.
- The title and URL from the fresh listing are checked again when the tab is claimed. If the browser or exact tab no longer exists or has changed, report that it is unavailable.
- To take over an already-open external browser tab, call `browser.user.openTabs()`, choose the matching returned tab by its visible title, URL, recency, and tab group, then pass that exact object to `browser.user.claimTab(tab)`.
- Claiming gives the current browser session control of the chosen external browser tab without moving it into an agent tab group, and returns a normal controllable `Tab`. Reuse that returned tab for navigation, Playwright, screenshots, CUA, and content reads.
- Do not guess tab ids. Only claim ids that came from the current `openTabs()` result.
