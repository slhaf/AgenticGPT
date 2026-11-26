# Agentic Browser JavaScript API

This reference describes Agentic Browser extension 0.4.0. Check `extensionVersion` in `browser.list` before using these methods. See the [quickstart](quickstart.md) for selecting a browser, acquiring a lease, and calling `browser.repl`.

`browser.repl` supplies `browser` and `nodeRepl` and supports top-level `await`. Top-level `const` and `let` bindings survive subsequent REPL calls in the same lease. Each lease runs one REPL script at a time.

## Browser and capability discovery

```ts
interface Browser {
  browserId: string; // Browser ID selected by `browser.acquire`.
  capabilities: BrowserCapabilityCollection; // Browser-scoped optional capabilities advertised by the connected backend; discover IDs with `await browser.capabilities.list()`, then call `await (await browser.capabilities.get(id)).documentation()` for method details.
  tabs: Tabs; // API for interacting with browser tabs.
  user: BrowserUser; // Context for user-owned browser tabs.
  nameSession(name: string): Promise<void>; // Name the current browser automation session.
}

type BrowserCapabilityCollection = {
  get(id: string): Promise<unknown>;
  list(): Promise<Array<{ id: string; description: string }>>;
};

type TabCapabilityCollection = {
  get(id: string): Promise<unknown>;
  list(): Promise<Array<{ id: string; description: string }>>;
};
```

Use `browser.nameSession(name)` to set the displayed task name. Continue to use the original lease name in MCP calls.

Call `list()` on the appropriate capability collection and use `get(id)` with an advertised ID. Returned objects have `id`, `info`, and `documentation(): Promise<string>` along with their capability methods.

| Scope | ID | Reference |
|---|---|---|
| Browser | `viewport` | [Viewport](capabilities/browser/viewport.md) |
| Browser | `management` | [Windows, tabs, and tab groups](capabilities/browser/management.md) |
| Tab | `pageAssets` | [Page assets](capabilities/tab/pageAssets.md) |
| Tab | `webmcp` | [WebMCP](capabilities/tab/webmcp.md) |

The WebMCP capability returns a document-bound tool collection from `fetchTools()`. Its description contains the callable names and input schemas. See the [WebMCP workflow](webmcp.md) for the discovery and invocation example.

```ts
interface WebMcpTools {
  description(): string;
  call(name: string, input: unknown, options?: { timeoutMs?: number }): Promise<unknown>;
}
```

## Tabs and user tabs

```ts
interface Tabs {
  content(options: TabsContentOptions): Promise<Array<TabsContentResult>>; // Load one or more URLs in temporary background tabs and extract their content without changing the selected tab.
  get(id: string): Promise<Tab>; // Get a tab owned by the current lease by its string ID.
  list(): Promise<Array<TabInfo>>; // List open tabs owned by the current lease.
  "new"(): Promise<Tab>; // Create a blank tab owned by the current lease.
  selected(): Promise<undefined | Tab>; // Return the selected tab when it belongs to the current lease.
}

interface BrowserUser {
  claimTab(tab: string | BrowserUserTabInfo): Promise<Tab>; // Claim a user tab returned by `openTabs()` and return it as a controllable agent tab.
  openTabs(): Promise<Array<BrowserUserTabInfo>>; // List open top-level tabs across the user's browser windows ordered by `lastOpened` descending.
}

interface TabInfo {
  id: string; // Identifier of a tab owned by the lease.
  providerTabId?: string; // Provider-owned identifier for matching an explicitly mentioned tab.
  title?: string;
  url?: string;
}

interface BrowserUserTabInfo {
  id: string; // Opaque identifier for this browser tab.
  lastOpened?: string; // ISO 8601 timestamp for the last time the tab was accessed.
  providerTabId?: string; // Provider-owned identity for correlating an explicit reference with this fresh listing.
  tabGroup?: string; // User-visible tab group name when the tab belongs to one.
  title?: string; // User-visible tab title.
  url?: string; // Current tab URL.
}

interface TabsContentOptions {
  contentType: TabsContentType; // Content representation to extract from each page.
  timeoutMs?: number; // Maximum time to wait for each page load, in milliseconds.
  urls: Array<string>; // URLs to load in temporary background tabs.
}

type TabsContentType = "html" | "text" | "domSnapshot";

interface TabsContentResult {
  content: null | string; // Extracted page content or null if the page failed to load or extract.
  title: null | string; // The resolved page title when available.
  url: string; // The resolved page URL when available, otherwise the requested URL.
}
```

Use `browser.user.openTabs()` to find tabs outside the lease in normal browser windows. Results are ordered by most recently accessed. To claim one, pass a string ID or a matching object from the latest listing. If the tab has changed since that listing, list again before claiming.

`browser.tabs.content()` visits HTTP(S) URLs in request order. Each page loads in a temporary background tab with a default load timeout of 10,000 milliseconds. The extension closes each temporary tab after extraction. An entry with `content: null` indicates that the page could not be loaded or read.

### Tab operations

```ts
interface Tab {
  ax: AXAPI; // API for interacting with accessibility state and accessibility elements.
  capabilities: TabCapabilityCollection; // Tab-scoped optional capabilities advertised by the connected backend; discover IDs with `await tab.capabilities.list()`, then call `await (await tab.capabilities.get(id)).documentation()` for method details.
  clipboard: TabClipboardAPI; // API for interacting with the browser session's clipboard.
  cua: CUAAPI; // API for interacting with the tab via the cua api
  dev: TabDevAPI; // API for developer-oriented tab inspection.
  dom_cua: DomCUAAPI; // API for interacting with the tab via the dom based cua api
  id: string; // A tab's unique identifier
  playwright: PlaywrightAPI; // API for interacting with the tab via the playwright api
  back(): Promise<void>; // Navigate this tab back in history.
  close(): Promise<void>; // Close this tab.
  forward(): Promise<void>; // Navigate this tab forward in history.
  getJsDialog(): Promise<undefined | Dialog>; // Get the currently observed native JavaScript dialog, if present.
  waitForJsDialog(options?: { timeoutMs?: number }): Promise<Dialog>; // Wait for the current or next native JavaScript dialog.
  goto(url: string): Promise<void>; // Open a URL in this tab.
  markDeliverable(): Promise<void>; // Keep this tab open for the user after release.
  markHandoff(): Promise<void>; // Keep this tab available to the next lease with the same name and pairing.
  reload(): Promise<void>; // Reload this tab.
  screenshot(options?: ScreenshotOptions): Promise<Uint8Array>; // Capture JPEG image bytes.
  title(): Promise<undefined | string>; // Get the current title for this tab.
  url(): Promise<undefined | string>; // Get the current URL for this tab.
}

type ScreenshotOptions = {
  clip?: ClipRect; // Crop to a specific rectangle instead of the full viewport.
  fullPage?: boolean; // Capture the full page instead of the viewport.
};

type ClipRect = {
  height: number;
  width: number;
  x: number;
  y: number;
};
```

`goto()`, `back()`, `forward()`, and `reload()` wait for navigation. `title()` and `url()` return `undefined` when unavailable.

On explicit `browser.release`, the extension closes unmarked tabs created by the lease, keeps claimed user tabs open, and preserves marked tabs. A handoff is available on the next matching lease within the same running browser. See [Tab cleanup and handoff](tab-cleanup-chrome.md).

A plain `tab.screenshot()` captures the viewport. Set `fullPage` or provide a `clip` rectangle in document CSS pixels. Pass returned bytes to `nodeRepl.emitImage()` to include an image in the REPL result.

## Accessibility

`tab.ax` provides the browser's accessibility state and operations on numbered elements. Observe before using an index. A page navigation or a new dialog can invalidate earlier numbers.

```ts
interface AXAPI {
  click(target: number | AXPoint, options?: AXClickOptions): Promise<void>; // Click an accessibility element or viewport coordinate.
  drag(from: AXPoint, to: AXPoint): Promise<void>; // Drag between two viewport coordinates.
  get(mode?: "state", options?: AXStateOptions): Promise<string>; // Return accessibility state without displaying it; prefer write() for model-visible observation.
  get(mode: "screenshot"): Promise<Uint8Array>; // Return screenshot bytes without displaying an image or advancing accessibility state.
  get(mode: "both", options?: AXStateOptions): Promise<{ screenshot?: Uint8Array; state: string }>; // Return accessibility state and screenshot bytes when available.
  paste(elementIndex: null | number, text: string, options?: AXPasteOptions): Promise<void>; // Paste through the browser session's virtual clipboard; null uses current focus.
  performSecondaryAction(elementIndex: number, action: string): Promise<void>; // Invoke an additional action exposed by an accessibility element.
  pressKey(elementIndex: null | number, key: string, options?: AXPressKeyOptions): Promise<void>; // If elementIndex is specified, attempt to focus the element prior to key entry.
  scroll(target: number | AXPoint, direction: AXDirection, pages?: number): Promise<void>; // Scroll an accessibility element or a viewport coordinate.
  selectText(elementIndex: number, text: string, options?: AXSelectTextOptions): Promise<void>; // Select text or position the cursor within an editable element.
  setValue(elementIndex: number, value: string): Promise<void>; // Set the value of an accessibility element.
  typeText(elementIndex: null | number, text: string): Promise<void>; // If elementIndex is specified, attempt to focus the element prior to text entry.
  write(mode?: "state", options?: AXStateOptions): Promise<void>; // Prefer this method to display current accessibility state to the model.
  write(mode: "screenshot"): Promise<void>; // Display a screenshot without advancing accessibility state.
  write(mode: "both", options?: AXStateOptions): Promise<void>; // Display accessibility state followed by its screenshot when available.
}

type AXPoint = [number, number];

type AXClickOptions = {
  clickCount?: number;
  duration?: number; // Seconds to hold each mouse press, from 0 through 30.
  key?: string; // Hold one key combination throughout the click sequence, for example "super" or "shift+a".
  mouseButton?: AXMouseButton;
};

type AXMouseButton = "left" | "right" | "middle" | "l" | "r" | "m";

type AXStateOptions = {
  disableDiffing?: boolean;
};

type AXPasteOptions = {
  format?: "text" | "md" | "html";
};

type AXPressKeyOptions = {
  duration?: number; // Seconds to hold the key combination before releasing it, from 0 through 30.
};

type AXDirection = "up" | "down" | "left" | "right" | "u" | "d" | "l" | "r";

type AXSelectTextOptions = {
  prefix?: string;
  selectionType?: AXSelectionType;
  suffix?: string;
};

type AXSelectionType = "text" | "cursor_before" | "cursor_after";
```

Text state is incremental for subsequent observations of the same document. Pass `{disableDiffing: true}` to request the full state. An image-only capture leaves the text baseline alone. In `"both"` mode an active native JavaScript dialog can return text state alone. Its buttons and prompt field appear as numbered controls in that text.

`AXPoint` contains viewport CSS pixels. For methods accepting a nullable index, `null` uses the current focus. Use `performSecondaryAction()` with the action name shown for the element.

## Coordinate input

```ts
interface CUAAPI {
  click(options: ClickOptions): Promise<void>; // Click at a coordinate in the current viewport.
  double_click(options: DoubleClickOptions): Promise<void>; // Double click at a coordinate in the current viewport.
  drag(options: DragOptions): Promise<void>; // Drag from a point to a point by the provided path.
  keypress(options: KeypressOptions): Promise<void>; // Press the keys together in the focused element.
  move(options: MoveOptions): Promise<void>; // Move the mouse to a point by the provided x and y coordinates.
  scroll(options: ScrollOptions): Promise<void>; // Scroll by a delta from a specific viewport coordinate.
  type(options: TypeOptions): Promise<void>; // Type text at the current focus.
}

type ClickOptions = {
  button?: number; // Mouse button (1-left, 2-middle/wheel, 3-right, 4-back, 5-forward).
  keypress?: Array<string>; // Modifier keys held during the click.
  x: number;
  y: number;
};

type DoubleClickOptions = {
  keypress?: Array<string>; // Modifier keys held during the double click.
  x: number;
  y: number;
};

type DragOptions = {
  keys?: Array<string>; // Optional modifier keys held during the drag.
  path: Array<{ x: number; y: number }>; // Drag path as a list of points.
};

type KeypressOptions = {
  duration?: number; // Seconds to hold the keys before releasing them, from 0 through 30.
  keys: Array<string>; // Key combination to press.
};

type MoveOptions = {
  keys?: Array<string>; // Optional modifier keys held while moving.
  x: number;
  y: number;
};

type ScrollOptions = {
  keypress?: Array<string>; // Modifier keys held during scroll.
  scrollX: number;
  scrollY: number;
  x: number;
  y: number;
};

type TypeOptions = {
  text: string;
};
```

`tab.cua` uses viewport CSS pixels. A drag requires at least two points. Key and mouse hold durations are measured in seconds.

## Visible DOM input

```ts
interface DomCUAAPI {
  click(options: DomClickOptions): Promise<void>; // Click a DOM node by its id from the visible DOM snapshot.
  double_click(options: DomClickOptions): Promise<void>; // Double-click a DOM node by its id.
  get_visible_dom(): Promise<string>; // Return visible DOM markup with node IDs for interactable elements.
  keypress(options: DomKeypressOptions): Promise<void>; // Press control characters at the currently focused element (focus it first via click/dblclick).
  scroll(options: DomScrollOptions): Promise<void>; // Scroll either the page or a specific node (if node_id provided) by deltas.
  type(options: DomTypeOptions): Promise<void>; // Type text into the currently focused element (focus via click first).
}

type DomClickOptions = {
  node_id: string; // Node id from `get_visible_dom()`.
};

type DomKeypressOptions = {
  keys: Array<string>; // Key combination to press.
};

type DomScrollOptions = {
  node_id?: string; // Optional node id to scroll within.
  x: number; // Horizontal scroll delta.
  y: number; // Vertical scroll delta.
};

type DomTypeOptions = {
  text: string; // Text to type into the currently focused element.
};
```

Use `tab.dom_cua.get_visible_dom()` to obtain current `node_id` values. For keypress and text input, focus the intended control first.

## Playwright page and frame operations

```ts
interface PlaywrightAPI {
  domSnapshot(): Promise<string>; // Return a snapshot of the current DOM as a string, including expanded iframe body content when available.
  evaluate<TResult, TArg>(pageFunction: PlaywrightEvaluateFunction<TArg, TResult>, arg?: TArg, options?: PlaywrightEvaluateOptions): Promise<TResult>; // Evaluate JavaScript in a read-only page scope.
  expectNavigation<T>(action: () => Promise<T>, options?: { timeout?: number; url?: string; waitUntil?: LoadState }): Promise<T>; // Expect a navigation triggered by an action.
  frameLocator(frameSelector: string): PlaywrightFrameLocator; // Create a frame-scoped locator builder.
  getByLabel(text: TextMatcher, options?: { exact?: boolean }): PlaywrightLocator; // Find elements by label text within the page.
  getByPlaceholder(text: TextMatcher, options?: { exact?: boolean }): PlaywrightLocator; // Find elements by placeholder text within the page.
  getByRole(role: string, options?: { exact?: boolean; name?: TextMatcher }): PlaywrightLocator; // Find elements by ARIA role within the page.
  getByTestId(testId: string): PlaywrightLocator; // Find elements by test id within the page.
  getByText(text: TextMatcher, options?: { exact?: boolean }): PlaywrightLocator; // Find elements by text within the page.
  locator(selector: string, options?: LocatorLocatorOptions): PlaywrightLocator; // Create a locator scoped to this tab.
  waitForLoadState(options?: PageWaitForLoadStateOptions): Promise<void>; // Wait for the page to reach a specific load state.
  waitForTimeout(timeoutMs: number): Promise<void>; // Wait for a fixed duration.
  waitForURL(url: string, options?: PageWaitForURLOptions): Promise<void>; // Wait for the page URL to match the provided value.
}

interface PlaywrightFrameLocator {
  frameLocator(frameSelector: string): PlaywrightFrameLocator; // Create a locator scoped to a nested frame.
  getByLabel(text: TextMatcher, options?: { exact?: boolean }): PlaywrightLocator; // Find elements by label within this frame.
  getByPlaceholder(text: TextMatcher, options?: { exact?: boolean }): PlaywrightLocator; // Find elements by placeholder within this frame.
  getByRole(role: string, options?: { exact?: boolean; name?: TextMatcher }): PlaywrightLocator; // Find elements by ARIA role within this frame.
  getByTestId(testId: string): PlaywrightLocator; // Find elements by test id within this frame.
  getByText(text: TextMatcher, options?: { exact?: boolean }): PlaywrightLocator; // Find elements by text within this frame.
  locator(selector: string, options?: LocatorLocatorOptions): PlaywrightLocator; // Create a locator scoped to this frame.
}

type PlaywrightEvaluateFunction<TArg, TResult> = string | ((arg: TArg) => TResult | Promise<TResult>);

type PlaywrightEvaluateOptions = {
  timeout?: number; // Maximum time to spend setting up the read-only DOM scope and running the script.
};

type TextMatcher = string | RegExp;

type LoadState = "load" | "domcontentloaded" | "networkidle";

type WaitUntil = LoadState | "commit";

type PageWaitForLoadStateOptions = {
  state?: LoadState;
  timeout?: number;
};

type PageWaitForURLOptions = {
  timeout?: number;
  waitUntil?: WaitUntil;
};
```

`tab.playwright.evaluate()` reads live page state through a read-only DOM scope. Pass serializable arguments and return serializable data. Use locators to change the page. `domSnapshot()` includes iframe contents when accessible. Create a frame locator with the selector for the iframe element and chain `frameLocator()` for nested frames.

Pass `timeout` as a positive integer in milliseconds. The default for Playwright operations that wait is 10,000 milliseconds. `waitForLoadState()` and `waitForURL()` default to `"load"`. The URL patterns for `waitForURL()` and `expectNavigation()` use `*` within a path segment and `**` across slashes. `expectNavigation()` starts listening before it invokes the action and resolves with the action's result when the navigation finishes.

### Locators

```ts
interface PlaywrightLocator {
  all(): Promise<Array<PlaywrightLocator>>; // Resolve to a list of locators for each matched element.
  allTextContents(options?: { timeout?: number }): Promise<Array<string>>; // Return `textContent` for *all* elements matched by this locator.
  and(locator: PlaywrightLocator): PlaywrightLocator; // Return a locator matching elements that satisfy both this locator and `locator`.
  check(options?: LocatorCheckOptions): Promise<void>; // Check a checkbox or switch-like control.
  click(options?: LocatorClickOptions): Promise<void>; // Click the element matched by this locator.
  count(): Promise<number>; // Number of elements matching this locator.
  dblclick(options?: LocatorClickOptions): Promise<void>; // Double-click the element matched by this locator.
  evaluate<TResult, TArg>(pageFunction: LocatorEvaluateFunction<TArg, TResult>, arg?: TArg, options?: PlaywrightEvaluateOptions): Promise<TResult>; // Evaluate JavaScript in a read-only scope; the locator must resolve unambiguously to one element.
  evaluateAll<TResult, TArg>(pageFunction: LocatorEvaluateAllFunction<TArg, TResult>, arg?: TArg, options?: PlaywrightEvaluateOptions): Promise<TResult>; // Evaluate read-only JavaScript against all elements matched by this locator.
  fill(value: string, options?: { timeout?: number }): Promise<void>; // Replace the element's value with the provided text.
  filter(options: LocatorFilterOptions): PlaywrightLocator; // Narrow this locator by additional constraints.
  first(): PlaywrightLocator; // Return a locator pointing at the first matched element.
  getAttribute(name: string, options?: { timeout?: number }): Promise<null | string>; // Return an attribute value from the first matched element.
  getByLabel(text: TextMatcher, options?: { exact?: boolean }): PlaywrightLocator; // Find elements by label text, scoped to this locator.
  getByPlaceholder(text: TextMatcher, options?: { exact?: boolean }): PlaywrightLocator; // Find elements by placeholder text, scoped to this locator.
  getByRole(role: string, options?: { exact?: boolean; name?: TextMatcher }): PlaywrightLocator; // Find elements by ARIA role, scoped to this locator.
  getByTestId(testId: string): PlaywrightLocator; // Find elements by test id, scoped to this locator.
  getByText(text: TextMatcher, options?: { exact?: boolean }): PlaywrightLocator; // Find elements by text content, scoped to this locator.
  innerText(options?: { timeout?: number }): Promise<string>; // Return the rendered (visible) text of the first matched element.
  isEnabled(): Promise<boolean>; // Whether the first matched element is currently enabled.
  isVisible(): Promise<boolean>; // Whether the first matched element is currently visible.
  last(): PlaywrightLocator; // Return a locator pointing at the last matched element.
  locator(selector: string, options?: LocatorLocatorOptions): PlaywrightLocator; // Create a descendant locator scoped to this locator.
  nth(index: number): PlaywrightLocator; // Return a locator pointing at the Nth matched element.
  or(locator: PlaywrightLocator): PlaywrightLocator; // Return a locator matching elements that satisfy either this locator or `locator`.
  press(value: string, options?: { timeout?: number }): Promise<void>; // Press a keyboard key while this locator is focused.
  pressSequentially(value: string, options?: LocatorPressSequentiallyOptions): Promise<void>; // Focus the element and press each character in the text sequentially without clearing its existing value.
  selectOption(value: SelectOptionInput | Array<SelectOptionInput>, options?: { timeout?: number }): Promise<void>; // Select one or more options on a native `<select>` element.
  screenshot(options?: { timeout?: number }): Promise<Uint8Array>; // Capture JPEG image bytes for the matched element.
  setChecked(checked: boolean, options?: LocatorCheckOptions): Promise<void>; // Set a checkbox or switch-like control to a checked/unchecked state.
  textContent(options?: { timeout?: number }): Promise<null | string>; // Return the raw textContent of the first matched element.
  type(value: string, options?: { timeout?: number }): Promise<void>; // Type text into the element without clearing existing content.
  uncheck(options?: LocatorCheckOptions): Promise<void>; // Uncheck a checkbox or switch-like control.
  waitFor(options?: LocatorWaitForOptions): Promise<void>; // Wait for the element to become visible or reach the specified state.
}

type LocatorCheckOptions = {
  force?: boolean;
  timeout?: number;
};

type LocatorClickOptions = {
  button?: MouseButton;
  force?: boolean;
  modifiers?: Array<KeyboardModifier>;
  timeout?: number;
};

type MouseButton = "left" | "right" | "middle";

type KeyboardModifier = "Alt" | "Control" | "ControlOrMeta" | "Meta" | "Shift";

type LocatorEvaluateFunction<TArg, TResult> = string | ((element: Element, arg: TArg) => TResult | Promise<TResult>);

type LocatorEvaluateAllFunction<TArg, TResult> = string | ((elements: Array<Element>, arg: TArg) => TResult | Promise<TResult>);

type LocatorFilterOptions = {
  has?: PlaywrightLocator;
  hasNot?: PlaywrightLocator;
  hasNotText?: TextMatcher;
  hasText?: TextMatcher;
  visible?: boolean;
};

type LocatorLocatorOptions = {
  has?: PlaywrightLocator;
  hasNot?: PlaywrightLocator;
  hasNotText?: TextMatcher;
  hasText?: TextMatcher;
  visible?: boolean;
};

type LocatorPressSequentiallyOptions = {
  timeout?: number;
};

type SelectOptionInput = string | SelectOptionDescriptor;

type SelectOptionDescriptor = {
  index?: number;
  label?: string;
  value?: string;
};

type LocatorWaitForOptions = {
  state?: WaitForState;
  timeout?: number;
};

type WaitForState = "attached" | "detached" | "visible" | "hidden";
```

Use locators from the same tab and frame when composing them with `and()`, `or()`, `has`, or `hasNot`. Most single-element actions require a unique match. Narrow the locator or use `first()`, `last()`, or `nth()` when position identifies the intended target. `nth()` is zero-based. Negative indices count from the end.

Text matchers accept strings and regular expressions. By default a string matches a case-insensitive substring. Use `exact: true` for a case-sensitive whole-string match where the method offers that option.

`waitFor()` defaults to `"visible"`. `selectOption()` works with native `<select>` elements and accepts strings or descriptors with `value`, `label`, or `index`. `screenshot()` returns JPEG bytes, including for elements inside accessible frames.

## Clipboard

```ts
interface TabClipboardAPI {
  read(): Promise<Array<TabClipboardItem>>; // Read clipboard items, including text and binary payloads.
  readText(): Promise<string>; // Read plain text from the lease clipboard.
  write(items: Array<TabClipboardItem>): Promise<void>; // Write clipboard items.
  writeText(text: string): Promise<void>; // Write plain text to the lease clipboard.
}

type TabClipboardItem = {
  entries: Array<TabClipboardEntry>;
  presentationStyle?: "unspecified" | "inline" | "attachment";
};

type TabClipboardEntry = {
  base64?: string;
  mimeType: string;
  text?: string;
};
```

The virtual clipboard is shared across the current lease. `write()` takes at least one item with at least one MIME entry per item. Each entry takes exactly one of `text` or `base64`. `readText()` returns an empty string when no plain-text entry is available.

## Native JavaScript dialogs

```ts
type Dialog = AlertDialog | BeforeUnloadDialog | ConfirmDialog | PromptDialog;

interface AlertDialog {
  type: "alert";
  message: string;
  accept(): Promise<void>;
  dismiss(): Promise<void>;
}

interface BeforeUnloadDialog {
  type: "beforeunload";
  message: string; // Chrome supplies the visible wording. The API message is empty.
  accept(): Promise<void>;
  dismiss(): Promise<void>;
}

interface ConfirmDialog {
  type: "confirm";
  message: string;
  accept(): Promise<void>;
  dismiss(): Promise<void>;
}

interface PromptDialog {
  type: "prompt";
  message: string;
  defaultValue: string;
  accept(text?: string): Promise<void>; // Submit the prompt. Use its default value when text is omitted.
  dismiss(): Promise<void>;
}
```

`tab.getJsDialog()` and `tab.waitForJsDialog()` start observing native dialogs if needed. Chrome reports dialogs opened while observation is active. `waitForJsDialog()` defaults to 10,000 milliseconds and reports `browser_wait_timeout` if no dialog appears.

An action interrupted by a dialog can report `browser_js_dialog_open`. Get the dialog in the current or next REPL call on the same lease. `accept()` and `dismiss()` both close an alert. For `beforeunload`, `accept()` leaves the page and `dismiss()` stays. A resolved dialog handle expires.

On a test page, a button whose handler calls `window.confirm()` can be handled as follows.

```js
try {
  await tab.playwright.getByRole('button', { name: 'Open confirm dialog' }).click();
} catch (error) {
  if (error.code !== 'browser_js_dialog_open') throw error;
}
const dialog = await tab.getJsDialog();
if (dialog?.type === 'confirm') await dialog.dismiss();
```

## Developer logs

```ts
interface TabDevAPI {
  logs(options?: TabDevLogsOptions): Promise<Array<TabDevLogEntry>>; // Read console log messages and uncaught exceptions captured for this tab.
}

interface TabDevLogsOptions {
  filter?: string; // Optional substring filter applied to the rendered log message.
  levels?: Array<"debug" | "info" | "log" | "warn" | "error" | "warning">; // Optional levels to include.
  limit?: number; // Maximum number of logs to return.
}

interface TabDevLogEntry {
  level: "debug" | "info" | "log" | "warn" | "error"; // Console log level.
  message: string; // Rendered log message text.
  timestamp: string; // ISO 8601 timestamp for when the runtime captured the log.
  url?: string; // Source URL reported by the browser runtime, when available.
}
```

`tab.dev.logs()` keeps the latest 500 captured entries per attached tab across navigations. `filter` matches a case-sensitive substring in the rendered message. `warning` is an alias for `warn` in the level filter. Results are returned oldest to newest after choosing the newest matching entries up to `limit`, which defaults to 100 and must be a positive integer.

Closing the tab, disconnecting its debugger, or releasing the lease clears that captured buffer. Chrome can replay available logs for the current document when the debugger first attaches.

## REPL output

```ts
interface NodeRepl {
  write(value: unknown): void;
  emitImage(image: Uint8Array | ImagePayload): void;
}

type ImagePayload = {
  mimeType: 'image/png' | 'image/jpeg' | 'image/webp' | 'image/gif';
  data: string; // Base64 image data.
};
```

`nodeRepl.write()` appends text or JSON to the result. The last JSON object written is also returned as `structuredContent`. `nodeRepl.emitImage()` appends a native MCP image block. Use `nodeRepl.emitImage(await tab.screenshot())` for a screenshot.

## Errors and session recovery

| Error | Meaning |
|---|---|
| `browser_busy` | The browser lease or REPL is occupied. |
| `browser_tab_unavailable` | The tab is outside the current lease or changed after listing. |
| `browser_js_dialog_open` | Get and resolve the active native dialog before continuing with the page. |
| `browser_wait_timeout` | The requested wait did not finish before its deadline. |
| `browser_repl_timeout` | The whole REPL script exceeded its deadline. Inspect the page before repeating actions already dispatched. |
| `browser_result_too_large` | The result exceeds the 16 MiB WebSocket message limit. Use a smaller clip or return fewer results. |
| `browser_lease_expired` | Acquire a lease again and inspect the current page state. |

Use `browser.reset` to replace the JS context while keeping the lease. Get the tab handle again before continuing. Use `browser.release` when finished.
