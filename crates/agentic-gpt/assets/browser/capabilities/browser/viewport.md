# Browser Capability: viewport
Browser viewport override control. Do not set the viewport during normal browser setup; most tasks should use the existing/default viewport. Use `set()` only when the user asks for specific dimensions, asks to test a responsive breakpoint or device size, or the task cannot be answered correctly without a specific viewport. Do not resize the browser just to make a screenshot larger, prettier, or fit more content. Use the default viewport, a normal screenshot, or a full-page screenshot instead. If you set a temporary viewport, call `reset()` before finishing unless the user asked to keep that viewport for the rest of the lease.

The override applies to the selected task tab. If no task tab is selected, the next tab you create or claim receives the override. Ending the lease restores the default size on all affected tabs.

```ts
const capability = await browser.capabilities.get("viewport");

interface ViewportSize {
  height: number;
  width: number;
}

interface ViewportBrowserCapability {
  reset(): Promise<void>; // Clear the explicit viewport override and return to default browser sizing.
  set(options: ViewportSize): Promise<void>; // Apply an explicit browser viewport override.
}
```
