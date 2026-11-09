import { createStore, useStore } from "./store";

/** `ms`: how long it stays; null until you dismiss it (it then shows a close). */
export type Toast = { id: number; text: string; tone: "neutral" | "ok" | "err"; action?: { label: string; run: () => void }; ms: number | null };
const toasts = createStore<Toast[]>([]);
let toastN = 0;

/** Toasts on screen at once: a new one pushes out the oldest, so a burst of answers never stacks over the page. */
const MAX_TOASTS = 3;
/** How long a toast that reports something done stays. A failure stays until it is read and dismissed. */
const TOAST_MS = 4200;

/** Shows a toast and returns its id, for `dismissToast`. `opts.ms` overrides how long it stays (an Undo toast ends with
 *  its undo window); by default a failure (`err`) stays until dismissed and anything else leaves after TOAST_MS. */
export function toast(text: string, tone: Toast["tone"] = "neutral", action?: Toast["action"], opts: { ms?: number | null } = {}): number {
  const ms = opts.ms !== undefined ? opts.ms : tone === "err" ? null : TOAST_MS;
  const t: Toast = { id: ++toastN, text, tone, action, ms };
  toasts.set([...toasts.get(), t].slice(-MAX_TOASTS));
  if (ms !== null) window.setTimeout(() => dismissToast(t.id), ms);
  return t.id;
}
export function dismissToast(id: number) {
  toasts.set(toasts.get().filter((x) => x.id !== id));
}
export const useToasts = () => useStore(toasts);
