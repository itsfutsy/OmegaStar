// What you did to a conversation on this device: pinned it, snoozed it, last looked at it. Keyed by a thread's
// root run id, or `chat:<employee id>` for an ongoing chat. Per-device UI organisation, so it lives in localStorage.
import { createStore, useStore } from "./store";
import { storageGet, storageSet } from "./storage";

export type Mark = { pinnedAt?: string; snoozedUntil?: string; seenAt?: string };
export type Marks = Record<string, Mark>;

const MARKS_KEY = "omega.threadMarks";
// Activity before this is never new, so the first launch does not flood Needs you with history.
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

/** The conversation key of an employee's ongoing chat. */
export const chatKey = (employeeId: string): string => `chat:${employeeId}`;

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
export function markSeen(key: string) {
  const now = new Date().toISOString();
  const until = store.get()[key]?.snoozedUntil;
  setMark(key, { seenAt: now, snoozedUntil: until && until > now ? until : undefined });
}

/** Forgets the marks of conversations that no longer exist anywhere in the sidebar. */
export function pruneMarks(keep: Set<string>) {
  const cur = store.get();
  const stale = Object.keys(cur).filter((k) => !keep.has(k));
  if (!stale.length) return;
  const next = { ...cur };
  for (const k of stale) delete next[k];
  save(next);
}

/** When the shelves started counting activity as new: set once, from the old Inbox's "seen" if there was one. */
export function shelvesSince(): string {
  const saved = storageGet(SINCE_KEY);
  if (saved) return saved;
  const since = storageGet(INBOX_SEEN_KEY) ?? new Date(Date.now() - 86_400_000).toISOString();
  storageSet(SINCE_KEY, since);
  return since;
}
