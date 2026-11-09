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
export function useRail(): { rail: boolean; narrow: boolean; toggle: () => void } {
  const narrow = useSyncExternalStore(subscribe, narrowNow);
  const rail = useSyncExternalStore(subscribe, () => railNow(narrow));
  const toggle = useCallback(() => {
    if (narrowNow()) storageSet(NARROW_EXPANDED_KEY, railNow(true) ? "1" : "0");
    else storageSet(COLLAPSE_KEY, railNow(false) ? "0" : "1");
    listeners.forEach((l) => l());
  }, []);
  return { rail, narrow, toggle };
}
