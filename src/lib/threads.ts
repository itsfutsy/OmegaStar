// A thread is a report and the replies you exchanged on it (`Thread { root, replies }`). A conversation you
// started is a thread too (a task under the hood), whose root is your first message, and an answer a colleague
// brought back into it is one of its replies. These helpers read one the same way everywhere: the Inbox and its
// Archive, and the conversation screen.
import type { Card, Run, Thread } from "../api/types";

export const isLive = (r: Run): boolean => r.state === "running" || r.state === "queued";
export const didNotFinish = (r: Run): boolean => r.state === "failed" || r.state === "lost" || r.state === "cancelled";

/** What a run reported, as previews show it: its answer, when its last words were only a note about a skill it kept
 *  up to date, else its summary. */
export const reportOf = (r: Run): string | null => r.answer ?? r.summary;

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

/** A conversation you started begins with `r`: a task, or an older build's chat message an answer came back into,
 *  which is one now. Mirrors Rust `threads::talking`. */
export const startedByYou = (r: Run): boolean => r.kind.type === "task" || r.kind.type === "chat";

export const isTask = (t: Thread): boolean => startedByYou(t.root);

/** The employee is answering (a conversation's first message or a later turn is queued or running). */
export const threadBusy = (t: Thread): boolean => isLive(t.root) || t.replies.some(isLive);

/** An Inbox row's threads, oldest first: the one it leads with and the others filed with it. */
export const rowThreads = (t: Thread): Thread[] =>
  [t, ...t.related].sort((a, b) => a.root.createdAt.localeCompare(b.root.createdAt));

/** Where a row sits in the Archive, newest first: the later of when it was archived and when it last moved. Mirrors
 *  Rust `threads::archive_at`. */
export function archiveAt(t: Thread): string {
  let at = activityAt(t);
  for (const x of rowThreads(t)) if (x.root.archivedAt && x.root.archivedAt > at) at = x.root.archivedAt;
  return at;
}

/** You archived something in the row, so Reopen has something to bring back. */
export const wasArchived = (t: Thread): boolean => rowThreads(t).some((x) => !!x.root.archivedAt);

/** An Inbox row that is one schedule's failures in a row. */
export function isStreak(t: Thread): boolean {
  const k = t.root.kind;
  return k.type === "scheduled" && t.related.length > 0
    && t.related.every((x) => x.root.kind.type === "scheduled" && x.root.kind.scheduleId === k.scheduleId && didNotFinish(x.root));
}

/** An Inbox row that is a week of weekly reviews, everyone's in one row. A review waiting on a chore is filed
 *  in the chore's row instead. */
export function isTeamWeek(t: Thread): boolean {
  const k = t.root.kind;
  return k.type === "scheduled" && !!k.review && !choreOpen(t);
}

/** A card waiting on your answer: the run that asked it and its index in the run's `cards`. */
export type OpenCard = { run: Run; index: number; card: Card };

/** The card waits on your answer. News waits on nothing: unseen, it holds back no answer and no Archive. Mirrors Rust
 *  `Card::is_open`. */
export const isOpen = (c: Card): boolean => !c.answer && c.ask.type !== "news";

/** The cards in a thread you have not answered, news you have not seen included, newest run first. */
export const openCards = (t: Thread): OpenCard[] =>
  [...t.replies].reverse().concat(t.root).flatMap((run) => run.cards.flatMap((card, index) => (card.answer ? [] : [{ run, index, card }])));

/** A conversation you are in: one going on (not archived, with replies) that you started or typed into. Turns Omega
 *  Star wrote from a tap alone do not make one. Mirrors Rust `threads::talking`. */
const talking = (t: Thread): boolean =>
  !t.root.archivedAt && t.replies.length > 0 && (isTask(t) || t.replies.some((r) => !r.omegaWrote && !r.aboutCard));

/** A card waiting on you holds up Archive on `thread`, a thread of the Inbox row `row`: a card in the thread itself, or
 *  in another member Archive would file with it. Another conversation you are in goes on by itself, so a card
 *  there does not, and news never does. Mirrors Rust `Office::set_archived`. */
export const doneHeld = (thread: Thread, row: Thread): boolean =>
  openCards(thread).some((c) => isOpen(c.card))
  || rowThreads(row).some((x) => x.root.id !== thread.root.id && !talking(x) && openCards(x).some((c) => isOpen(c.card)));

/** Archive can file `thread`, a thread of the Inbox row `row`, now: it is not working and no card holds it up. */
export const archivable = (thread: Thread, row: Thread): boolean => !threadBusy(thread) && !doneHeld(thread, row);

/** What a card is about, so the same question from anyone is one decision: its key, or for a chore asked
 *  before keys, `chore:<subject>`. Mirrors Rust `Card::topic`. */
export const topic = (c: Card): string | null => c.key ?? (c.ask.type === "chore" ? `chore:${c.ask.subject}` : null);

/** What needs you in a run that is not a card: flags Omega Star raised. Mirrors Rust `Run::flags`. */
export const flags = (r: Run): string[] => r.needsYou.filter((m) => !r.cards.some((c) => c.message === m));

/** Blocked on a tool you could allow (a tidy trying a command it may not run is not). */
export const blocked = (r: Run): boolean => r.denials.length > 0 && r.kind.type !== "tidy";

/** The thread's newest finished run was blocked on a tool you could allow, and no later turn (the retry you allowed)
 *  is on its way yet. Mirrors Rust `threads::asks_you`. */
export function blockedNow(t: Thread): boolean {
  const r = lastReply(t);
  return blocked(latestFinished(t)) && !(r && !r.endedAt);
}

/** Broke for a reason you can act on: not the usage limit, which Omega Star waits out and retries itself. */
export const broken = (r: Run): boolean => (r.state === "failed" || r.state === "lost") && !r.limited;

/** A schedule's failures in a row before they need you. Mirrors Rust `threads::STREAK`. */
export const STREAK = 3;

/** Some run in an Inbox row has a chore waiting on you. */
const choreOpen = (t: Thread): boolean =>
  rowThreads(t).some((x) => openCards(x).some((c) => c.card.ask.type === "chore"));

/** An Inbox row that is everyone waiting on the same chore. */
export const isChoreRow = (t: Thread): boolean => t.related.length > 0 && choreOpen(t);

/** The conversation route's `focus` segment that selects a thread: `#/chat/<employee>/thread~<root run id>`. */
export const threadFocus = (rootId: string): string => `thread~${rootId}`;
export const parseThreadFocus = (focus: string | null): string | null =>
  focus && focus.startsWith("thread~") ? focus.slice("thread~".length) : null;

/** The first run of the conversation `r` is a turn of: a conversation you started, the thread a reply is in, or the
 *  conversation an answer came back into. Null for anything else, an older build's chat included. */
export function conversationOf(r: Run): string | null {
  switch (r.kind.type) {
    case "task": return r.id;
    case "reply": return r.kind.toRunId;
    case "answered": return r.kind.thread ?? null;
    default: return null;
  }
}
