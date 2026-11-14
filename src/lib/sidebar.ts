// The sidebar's width rule: below RAIL_BELOW px wide it folds to the rail by itself, unless you expanded it at that
// size; at or above it, your saved choice holds. Each size keeps its own choice. The Shell, the boot screen and
// onboarding all ask here, so the band is the same width everywhere.
import { useCallback, useSyncExternalStore } from "react";
import { storageGet, storageSet } from "./storage";

const RAIL_BELOW = 1180;
/** The saved choice at RAIL_BELOW px and wider: "1" collapsed to the rail. */
export const COLLAPSE_KEY = "omega.sidebarCollapsed";
/** The saved choice below RAIL_BELOW px: "1" expanded (the rail is the default there). */
export const NARROW_EXPANDED_KEY = "omega.sidebarNarrowExpanded";
const NARROW = `(max-width: ${RAIL_BELOW - 0.02}px)`;

const narrowNow = () => window.matchMedia(NARROW).matches;
/** Whether the sidebar shows as the rail right now, for code that renders once (the boot screen). */
export function railNow(narrow = narrowNow()): boolean {
  return narrow ? storageGet(NARROW_EXPANDED_KEY) !== "1" : storageGet(COLLAPSE_KEY) === "1";
}

// Changes to the saved choice, so every user of the hook hears about a toggle.
const listeners = new Set<() => void>();
function subscribe(cb: () => void) {
  const mq = window.matchMedia(NARROW);
  listeners.add(cb);
  mq.addEventListener("change", cb);
  return () => { listeners.delete(cb); mq.removeEventListener("change", cb); };
}

/** The rail or the full sidebar, and a toggle that saves the choice for the size you are at. */
export function useRail(): { rail: boolean; toggle: () => void } {
  const narrow = useSyncExternalStore(subscribe, narrowNow);
  const rail = useSyncExternalStore(subscribe, () => railNow(narrow));
  const toggle = useCallback(() => {
    if (narrowNow()) storageSet(NARROW_EXPANDED_KEY, railNow(true) ? "1" : "0");
    else storageSet(COLLAPSE_KEY, railNow(false) ? "0" : "1");
    listeners.forEach((l) => l());
  }, []);
  return { rail, toggle };
}

// The sidebar's width, which you set by dragging its right edge. Saved per device; double-clicking the edge puts it
// back. It is the --side-w token on the root, so the Shell, the boot screen and onboarding all follow it.
/** The saved width in px; absent or empty means the default. */
export const WIDTH_KEY = "omega.sidebarWidth";
export const SIDE_W_DEFAULT = 216;
/** Narrow enough to give the page room, wide enough that names and the usage card still read. */
export const SIDE_W_MIN = 180;
export const SIDE_W_MAX = 360;

/** A width kept inside the sidebar's limits, in whole pixels. Anything that is not a number is the default. */
export function clampSideWidth(px: number): number {
  if (!Number.isFinite(px)) return SIDE_W_DEFAULT;
  return Math.round(Math.min(SIDE_W_MAX, Math.max(SIDE_W_MIN, px)));
}

/** The width you last left the sidebar at, or the default. */
export function savedSideWidth(): number {
  const raw = storageGet(WIDTH_KEY);
  return raw ? clampSideWidth(Number(raw)) : SIDE_W_DEFAULT;
}

/** Shows the sidebar at `px` (clamped) without saving it: what a drag does on each move. Returns the width shown. */
export function showSideWidth(px: number): number {
  const w = clampSideWidth(px);
  if (w === SIDE_W_DEFAULT) document.documentElement.style.removeProperty("--side-w");
  else document.documentElement.style.setProperty("--side-w", `${w}px`);
  return w;
}

/** Shows and saves the width; the default clears the saved one, so a later change to the default reaches you. */
export function saveSideWidth(px: number) {
  const w = showSideWidth(px);
  storageSet(WIDTH_KEY, w === SIDE_W_DEFAULT ? "" : String(w));
}

/** Puts the saved width on the root before the first render, so nothing jumps. */
export function applySideWidth() {
  showSideWidth(savedSideWidth());
}
