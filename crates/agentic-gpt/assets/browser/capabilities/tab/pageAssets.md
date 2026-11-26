# Tab Capability: pageAssets
Asset inventory for the current rendered page state. Use `list()` to inspect assets already observed in the tab's current state. If lazy-loaded content or another UI state matters, load that state first, then call `list()` again so the inventory reflects what is currently observable. Do not navigate directly to asset URLs just to fetch them.

```ts
const capability = await tab.capabilities.get("pageAssets");

interface PageAssetsTabCapability {
  list(): Promise<{ assets: Array<{ id: string; kind: "other" | "pdf" | "script" | "font" | "image" | "stylesheet" | "video"; name: string; sources: Array<{ kind: "attribute" | "computedStyle" | "resource"; nodeId?: number; property?: string }>; url: string }>; id: string; inlineSvgs: Array<{ id: string; markup: string; name: string }>; pageUrl: null | string; summary: { byKind: Partial<Record<"other" | "pdf" | "script" | "font" | "image" | "stylesheet" | "video", number>>; inlineSvgCount: number; totalCount: number } }>; // Inventory file assets and inline SVGs observed in the current page state.
}
```
