// A thread is a report and the replies you exchanged on it (`Thread { root, replies }`). A task is a thread
// too, whose root is your first message. These helpers read one the same way everywhere: the sidebar's shelves,
// the conversation screen, and the Archive.
import type { Run, Thread } from "../api/types";

export const isLive = (r: Run): boolean => r.state === "running" || r.state === "queued";
export const didNotFinish = (r: Run): boolean => r.state === "failed" || r.state === "lost" || r.state === "cancelled";

/** When a thread last moved: its newest reply (sent or finished), else the report itself. */
export function activityAt(t: Thread): string {
  let at = t.root.endedAt ?? t.root.createdAt;
  for (const r of t.replies) {
    const x = r.endedAt ?? r.createdAt;
    if (x > at) at = x;
  }
  return at;
}

export const lastReply = (t: Thread): Run | undefined => t.replies[t.replies.length - 1];

/** The newest run that has finished: what the row says and what a reply box answers. */
export function latestFinished(t: Thread): Run {
  for (let i = t.replies.length - 1; i >= 0; i--) if (t.replies[i].endedAt) return t.replies[i];
  return t.root;
}

export const isTask = (t: Thread): boolean => t.root.kind.type === "task";

/** The employee is answering (a task or a reply is queued or running). */
export const threadBusy = (t: Thread): boolean => isLive(t.root) || t.replies.some(isLive);

/** Who spoke last: the employee once their newest reply finished, you while it is still on its way. A task's
 *  first message counts as a reply; a report does not. */
export function lastFrom(t: Thread): "you" | "them" | null {
  const l = lastReply(t) ?? (isTask(t) ? t.root : undefined);
  if (!l) return null;
  return l.endedAt ? "them" : "you";
}

/** An Inbox row's threads, oldest first: the one it leads with and the others filed with it. */
export const rowThreads = (t: Thread): Thread[] =>
  [t, ...t.related].sort((a, b) => a.root.createdAt.localeCompare(b.root.createdAt));

/** An Inbox row that is one schedule's failures in a row. */
export function isStreak(t: Thread): boolean {
  const k = t.root.kind;
  return k.type === "scheduled" && t.related.length > 0
    && t.related.every((x) => x.root.kind.type === "scheduled" && x.root.kind.scheduleId === k.scheduleId && didNotFinish(x.root));
}

/** The conversation route's `focus` segment that selects a thread: `#/chat/<employee>/thread~<root run id>`. */
export const threadFocus = (rootId: string): string => `thread~${rootId}`;
export const parseThreadFocus = (focus: string | null): string | null =>
  focus && focus.startsWith("thread~") ? focus.slice("thread~".length) : null;
/** The conversation route's `focus` segment for a task you are about to start. */
export const NEW_TASK_FOCUS = "new-task";
