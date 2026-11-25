// Shared motion: the moves that need measuring. The ones that don't are classes in src/design/base.css (enter-rise,
// enter-drop, leave-fade, collapse). Durations and curves are read from the tokens, so reduced motion and any
// retuning reach every screen at once. Nothing here loops: every move has a cause, and a value that holds holds still.
import { useEffect, useLayoutEffect, useRef, useState } from "react";

const rootStyle = () => getComputedStyle(document.documentElement);
/** A duration token in milliseconds: ms("--d-arrive"). */
export const ms = (token: string) => parseFloat(rootStyle().getPropertyValue(token)) || 0;
/** A curve token: curve("--ease-move"). */
export const curve = (token: string) => rootStyle().getPropertyValue(token).trim() || "ease";
export const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** A value rolling in its clipped slot (`.roll`): the old one leaves as the new one arrives, upward when the value went up
 *  and downward when it went down. Under reduced motion the two crossfade in place. `Roll` in ui.tsx uses it. */
export function rollAnimations(out: HTMLElement, inn: HTMLElement, up: boolean, delay = 0): Animation[] {
  if (reducedMotion()) {
    const o: KeyframeAnimationOptions = { duration: ms("--d-fade"), easing: "linear", fill: "both" };
    return [out.animate([{ opacity: 1 }, { opacity: 0 }], o), inn.animate([{ opacity: 0 }, { opacity: 1 }], o)];
  }
  const o: KeyframeAnimationOptions = { duration: ms("--d-roll"), delay, easing: curve("--ease"), fill: "both" };
  const d = up ? 1 : -1;
  return [
    out.animate([{ transform: "none", opacity: 1 }, { transform: `translateY(${-70 * d}%)`, opacity: 0 }], o),
    inn.animate([{ transform: `translateY(${70 * d}%)`, opacity: 0 }, { transform: "none", opacity: 1 }], o),
  ];
}

/** A list's children that carry a `data-key`, in order. */
function keyed(host: HTMLElement): HTMLElement[] {
  return [...host.children].filter((el): el is HTMLElement => el instanceof HTMLElement && !!el.dataset.key);
}

/** Where each keyed child is drawn right now, inside its list's scrolled content. */
function places(host: HTMLElement): Map<string, { x: number; y: number }> {
  const box = host.getBoundingClientRect();
  return new Map(keyed(host).map((el) => {
    const r = el.getBoundingClientRect();
    return [el.dataset.key as string, { x: r.left - box.left + host.scrollLeft, y: r.top - box.top + host.scrollTop }];
  }));
}

/**
 * Rows that move rather than swap. Put the returned ref on a list whose children carry a stable `data-key`, and pass
 * anything that changes when the list does (its keys joined, a version). Each child that changed place travels from
 * where it was drawn just before the change over --d-arrive on --ease-move; a new child drops in from just above once
 * its room is half open. A child that leaves should close its own space first (`.collapse` with data-open="false"),
 * then unmount. Under reduced motion nothing travels and new rows fade in.
 */
export function useFlip<T extends HTMLElement>(change: unknown) {
  const ref = useRef<T>(null);
  const last = useRef(change);
  const drawnBeforeChange = useRef<Map<string, { x: number; y: number }> | null>(null);
  if (!Object.is(last.current, change) && ref.current) drawnBeforeChange.current = places(ref.current);
  useLayoutEffect(() => {
    if (Object.is(last.current, change)) return;
    last.current = change;
    const was = drawnBeforeChange.current;
    drawnBeforeChange.current = null;
    const host = ref.current;
    if (!host || !was?.size) return;
    const now = places(host);
    const reduced = reducedMotion();
    for (const el of keyed(host)) {
      const a = now.get(el.dataset.key as string)!;
      const b = was.get(el.dataset.key as string);
      if (!b) {
        el.animate(
          reduced ? [{ opacity: 0 }, { opacity: 1 }] : [{ opacity: 0, transform: "translateY(-8px)" }, { opacity: 1, transform: "none" }],
          reduced ? { duration: ms("--d-fade"), easing: "linear", fill: "backwards" }
            : { duration: ms("--d-arrive"), delay: ms("--d-arrive") * 0.5, easing: curve("--ease-arrive"), fill: "backwards" },
        );
        continue;
      }
      const dx = b.x - a.x, dy = b.y - a.y;
      if (reduced || (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5)) continue;
      el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "none" }], { duration: ms("--d-arrive"), easing: curve("--ease-move") });
    }
  }, [change]);
  return ref;
}

/**
 * Keeps something mounted while it leaves. Returns whether to render it; while `open` is false and it is still
 * rendered, give it its leaving class (`leave-fade`, `leave-rise`). `leaveMs` is the leaving animation's length.
 */
export function usePresence(open: boolean, leaveMs: number) {
  const [mounted, setMounted] = useState(open);
  if (open && !mounted) setMounted(true);
  useEffect(() => {
    if (open || !mounted) return;
    const timer = window.setTimeout(() => setMounted(false), leaveMs);
    return () => window.clearTimeout(timer);
  }, [open, mounted, leaveMs]);
  return mounted;
}
