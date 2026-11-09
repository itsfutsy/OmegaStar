// Where each conversation stands, worked out in one place. The Inbox, its badge, the panel's chips and the
// conversation header all read what `threadStatus` returns and decide nothing themselves. Pure.
import type { Thread } from "../api/types";
import type { Mark, Marks } from "./marks";
import { toolName } from "./rules";
import { activityAt, blockedNow, broken, flags, isLive, isTask, lastReply, latestFinished, openCards, threadBusy } from "./threads";

/** What a conversation is waiting on: you, the employee (working, or queued for a free slot), a time (snoozed),
 *  nothing ever again (done), or nothing right now (idle). */
export type State = "needsYou" | "working" | "queued" | "snoozed" | "done" | "idle";

/** Where one conversation stands. */
export type Status = {
  state: State;
  /** It is a row of the Inbox: a report that reached you, or a conversation or replied thread the employee
   *  answered, until you archive it. A snoozed row sits under Snoozed instead. */
  inbox: boolean;
  /** It moved since you last looked, or its snooze ended. */
  unread: boolean;
  /** Why it needs you, as one line; null unless `state` is needsYou. */
  reason: string | null;
  /** When its snooze ends, while it is snoozed. */
  snoozedUntil: string | null;
};

/** What reading a status takes besides the conversation: your marks, when activity started counting as new
 *  (`seenSince`), and now in ms since the epoch. */
export type StatusContext = { marks: Marks; since: string; now: number };

/** What the state reads, worked out once per conversation. */
type Facts = {
  /** You archived it. */
  done: boolean;
  snoozed: boolean;
  /** The employee is on it. */
  busy: boolean;
  /** Everything in flight waits for a free slot. */
  queued: boolean;
  /** It is yours to act on until you mark it done: an Inbox row, or an answered thread. */
  waiting: boolean;
  unread: boolean;
  /** It asks you something: a card, a flag, a blocked tool, or a run that broke. */
  flagged: boolean;
};

/** A conversation's state: the first rule that matches. */
const RULES: [State, (f: Facts) => boolean][] = [
  ["done", (f) => f.done],
  ["snoozed", (f) => f.snoozed],
  ["queued", (f) => f.busy && f.queued],
  ["working", (f) => f.busy],
  ["needsYou", (f) => f.waiting && (f.unread || f.flagged)],
  ["idle", () => true],
];

const stateOf = (f: Facts): State => RULES.find(([, when]) => when(f))![0];
const ms = (iso: string | null | undefined): number => (iso ? Date.parse(iso) : 0);

/** Who spoke last in a thread: the employee once its newest run (the report, or your last reply) has ended. */
function lastSpeaker(t: Thread): "you" | "them" {
  return (lastReply(t) ?? t.root).endedAt ? "them" : "you";
}

/** The thread asks you something: a card anywhere in it, or its newest finished run flagged something, was blocked
 *  (with no retry on its way) or broke (not on the usage limit). Mirrors Rust `threads::asks_you` and `threads::broken`. */
function flagged(t: Thread): boolean {
  const r = latestFinished(t);
  return openCards(t).length > 0 || flags(r).length > 0 || blockedNow(t) || broken(r);
}

/** A snooze that has ended and you have not looked at since: snoozing means "remind me". */
function woke(mark: Mark, now: number): boolean {
  return !!mark.snoozedUntil && ms(mark.snoozedUntil) <= now && ms(mark.seenAt) < ms(mark.snoozedUntil);
}

/** Activity after this is new to you. */
const seenFloor = (mark: Mark, since: string): number => Math.max(ms(mark.seenAt), ms(since));

/** Why a thread needs you, as one line. */
function threadReason(t: Thread, mark: Mark, now: number): string {
  const r = latestFinished(t);
  const card = openCards(t)[0];
  if (card) return card.card.message;
  const flag = flags(r)[0];
  if (flag) return flag;
  if (blockedNow(t)) return `Blocked on ${toolName(r.denials[0].tool)}`;
  if (broken(r)) return "Didn't finish";
  if (woke(mark, now)) return "Snooze ended";
  return t.replies.length || isTask(t) ? "New reply" : "New report";
}

/** Where a thread stands. `filed`: it leads an Inbox row (`Conversations.inbox`). */
export function threadStatus(t: Thread, filed: boolean, { marks, since, now }: StatusContext): Status {
  const mark = marks[t.root.id] ?? {};
  const busy = threadBusy(t);
  const answered = !busy && lastSpeaker(t) === "them";
  const done = !!t.root.archivedAt;
  const waiting = !done && (filed || answered);
  const unread = waiting && answered && (ms(activityAt(t)) > seenFloor(mark, since) || woke(mark, now));
  const snoozed = !done && ms(mark.snoozedUntil) > now;
  const live = [t.root, ...t.replies].filter(isLive);
  const state = stateOf({ done, snoozed, busy, queued: live.every((r) => r.state === "queued"), waiting, unread, flagged: flagged(t) });
  return {
    state,
    inbox: waiting && !snoozed,
    unread,
    reason: state === "needsYou" ? threadReason(t, mark, now) : null,
    snoozedUntil: snoozed ? mark.snoozedUntil! : null,
  };
}

/** Later's choices from `now`: in 3 hours, tomorrow at 9:00 am, and next week, the Monday after tomorrow at 9:00 am. */
export function snoozePresets(now: Date): { label: string; until: Date }[] {
  const at9 = (days: number) => new Date(now.getFullYear(), now.getMonth(), now.getDate() + days, 9, 0, 0, 0);
  const toMonday = ((8 - now.getDay()) % 7 || 7) + (now.getDay() === 0 ? 7 : 0);
  return [
    { label: "In 3 hours", until: new Date(now.getTime() + 3 * 3_600_000) },
    { label: "Tomorrow", until: at9(1) },
    { label: "Next week", until: at9(toMonday) },
  ];
}

/** When the next snooze ends, in ms since the epoch, or null when none is pending. */
export function nextWake(marks: Marks, now: number): number | null {
  let next: number | null = null;
  for (const m of Object.values(marks)) {
    const t = ms(m.snoozedUntil);
    if (t > now && (next === null || t < next)) next = t;
  }
  return next;
}
