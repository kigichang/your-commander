/** Platform detection for the webview. The UA is stable for this purpose:
 *  WKWebView says "Mac OS X", WebKitGTK says "Linux". */
export const isMac = navigator.userAgent.includes("Mac");

/** The primary shortcut modifier: ⌘ on macOS, Ctrl elsewhere (Super is
 *  taken by the desktop environment on Linux). Keymap entries written as
 *  "mod+…" expand to this. */
export const MOD: "meta" | "ctrl" = isMac ? "meta" : "ctrl";

/** The primary-modifier state of a mouse or keyboard event. */
export function isModKey(e: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return isMac ? e.metaKey : e.ctrlKey;
}

/** Lets CSS vary by platform: [data-platform="mac"] … */
export function applyPlatformDataset() {
  document.documentElement.dataset.platform = isMac ? "mac" : "linux";
}

/** Spread into elements that drag the window on macOS, where the Overlay
 *  title bar has no native handle. Linux keeps native decorations, so the
 *  toolbar stays a plain toolbar there. */
export const dragRegion = isMac ? ({ "data-tauri-drag-region": true } as const) : {};
