// The window's own chrome. On macOS the title bar is an overlay (src-tauri/tauri.macos.conf.json): the traffic lights
// sit over the sidebar's top, so the app reserves --chrome-top there and makes it a drag region. Everywhere else
// (Windows, Linux, the browser preview) the native frame stays and nothing is reserved.
//
// The browser preview can draw the macOS case with ?chrome=mac, for screenshots.

/** Whether the macOS overlay title bar is in use. Set once, before the first render. */
export const macChrome = (() => {
  if (import.meta.env.DEV && new URLSearchParams(location.search).get("chrome") === "mac") return true;
  return "__TAURI_INTERNALS__" in window && /Mac/.test(navigator.platform || navigator.userAgent);
})();

/** Marks the root (`data-chrome="mac"`), which sets --chrome-top and widens the rail for the traffic lights. */
export function applyChrome() {
  if (macChrome) document.documentElement.dataset.chrome = "mac";
}

/** Props for the strip the window is dragged by: the traffic lights' band. Empty where there is no overlay. */
export const dragRegion = macChrome ? { "data-tauri-drag-region": "" } : {};

/** Props for a page's header band, so the window drags by the top of every page as it does on a Mac title bar, and a
 *  double-click there zooms it. "deep": the band's text and gaps drag too, while its buttons, links and fields keep
 *  their own clicks (Tauri's drag script skips them). Empty where there is no overlay. */
export const headDragRegion = macChrome ? { "data-tauri-drag-region": "deep" } : {};
