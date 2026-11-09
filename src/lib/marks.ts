// What you did to a conversation on this device: snoozed it, last looked at it. Keyed by a thread's root run id.
// Per-device UI organisation, so it lives in localStorage.
import { useEffect, useState } from "react";
import type { Run } from "../api/types";
import { createStore, useStore } from "./store";
import { storageGet, storageSet } from "./storage";

export type Mark = { snoozedUntil?: string; seenAt?: string };
export type Marks = Record<string, Mark>;

const MARKS_KEY = "omega.threadMarks";
// Activity before this is never new, so the first launch does not flood the Inbox with unread history.
const SINCE_KEY = "omega.shelvesSince";
const INBOX_SEEN_KEY = "omega.inboxSeen";

/** The saved marks, or none when storage is empty or unreadable. */
function load(): Marks {
  try {
    const v: unknown = JSON.parse(storageGet(MARKS_KEY) ?? "{}");
    return v && typeof v === "object" ? (v as Marks) : {};
  } catch {
    return {};
  }
}

const store = createStore<Marks>(load());

/** Replaces every mark and saves them. */
function save(next: Marks) {
  store.set(next);
  storageSet(MARKS_KEY, JSON.stringify(next));
}

/** Every mark, re-rendering when one changes. */
export const useMarks = (): Marks => useStore(store);

/** Changes one conversation's mark; a field set to undefined is cleared, and an empty mark is dropped. */
export function setMark(key: string, patch: Mark) {
  const merged: Mark = { ...store.get()[key], ...patch };
  const mark = Object.fromEntries(Object.entries(merged).filter(([, v]) => v !== undefined)) as Mark;
  const next = { ...store.get() };
  if (Object.keys(mark).length) next[key] = mark;
  else delete next[key];
  save(next);
}

/** You looked at the conversation now. A snooze that has ended is settled by looking. */
function markSeen(key: string) {
  const now = new Date().toISOString();
  const until = store.get()[key]?.snoozedUntil;
  setMark(key, { seenAt: now, snoozedUntil: until && until > now ? until : undefined });
}

/** Forgets the marks of conversations that are no longer open. */
export function pruneMarks(keep: Set<string>) {
  const cur = store.get();
  const stale = Object.keys(cur).filter((k) => !keep.has(k));
  if (!stale.length) return;
  const next = { ...cur };
  for (const k of stale) delete next[k];
  save(next);
}

/** When activity started counting as new: set once, from the old Inbox's "seen" if there was one. */
export function seenSince(): string {
  const saved = storageGet(SINCE_KEY);
  if (saved) return saved;
  const since = storageGet(INBOX_SEEN_KEY) ?? new Date(Date.now() - 86_400_000).toISOString();
  storageSet(SINCE_KEY, since);
  return since;
}

/** When a conversation last moved, from its runs: what makes looking at it again count as seeing something new. */
export function lastMoved(runs: Run[]): string {
  let at = "";
  for (const r of runs) {
    const x = r.endedAt ?? r.createdAt;
    if (x > at) at = x;
  }
  return at;
}

/** Marks the conversation seen when it opens and whenever it moves, while the window is in front of you. A null
 *  `key` marks nothing. */
export function useMarkSeen(key: string | null, moved: string) {
  const [focused, setFocused] = useState(() => document.hasFocus() && document.visibilityState === "visible");
  useEffect(() => {
    const update = () => setFocused(document.hasFocus() && document.visibilityState === "visible");
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    document.addEventListener("visibilitychange", update);
    return () => { window.removeEventListener("focus", update); window.removeEventListener("blur", update); document.removeEventListener("visibilitychange", update); };
  }, []);
  useEffect(() => { if (focused && key) markSeen(key); }, [key, moved, focused]);
}
