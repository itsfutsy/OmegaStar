// The sidebar's shelves: every conversation across the office, placed by what it needs from you. Pure: the
// sidebar and the conversation screen render what `buildShelves` returns and decide nothing themselves.
import type { ChatHead, Conversations, EmployeeView, HireProposal, Thread } from "../api/types";
import { chatKey, type Mark, type Marks } from "./marks";
import { toolName } from "./rules";
import { activityAt, isLive, lastReply, latestFinished, threadBusy } from "./threads";

export type ShelfId = "needsYou" | "working" | "pinned" | "open" | "team" | "snoozed" | "done";

/** One conversation in the sidebar. `key` is what its mark is filed under; `at` is when it last moved. */
export type Row =
  | { kind: "thread"; key: string; at: string; title: string; employee: EmployeeView; thread: Thread; done: boolean }
  | { kind: "chat"; key: string; at: string; title: string; employee: EmployeeView; head: ChatHead }
  | { kind: "proposal"; key: string; at: string; title: string; proposal: HireProposal; proposer: EmployeeView | undefined };

/** What placing a row reads, worked out once per row. */
type Facts = { done: boolean; snoozed: boolean; needsYou: boolean; busy: boolean; pinned: boolean };

/** A row on its shelf, with what the sidebar shows about it. `reason` says why it needs you. */
export type Placed = { row: Row; shelf: ShelfId; mark: Mark; busy: boolean; queued: boolean; reason: string | null };
export type Shelf = { id: ShelfId; label: string; rows: Placed[] };

export type ShelvesInput = {
  conversations: Conversations;
  employees: EmployeeView[];
  /** Hire proposals still open. */
  proposals: HireProposal[];
  marks: Marks;
  /** Activity before this is never new (`shelvesSince`). */
  since: string;
  /** A thread's name, as the rest of the app labels it. */
  label: (t: Thread) => string;
};

/** The selected row keeps the shelf it had when you selected it, so reading it does not move it. */
export type Sticky = { key: string; shelf: ShelfId } | null;

/** Where a row goes: the first rule that matches. */
const RULES: { shelf: ShelfId; when: (row: Row, f: Facts) => boolean }[] = [
  { shelf: "needsYou", when: (row) => row.kind === "proposal" },
  { shelf: "done", when: (_, f) => f.done },
  { shelf: "snoozed", when: (_, f) => f.snoozed },
  { shelf: "needsYou", when: (_, f) => f.needsYou },
  { shelf: "working", when: (_, f) => f.busy },
  { shelf: "pinned", when: (_, f) => f.pinned },
  { shelf: "team", when: (row) => row.kind === "chat" },
  { shelf: "open", when: () => true },
];

const ms = (iso: string | null | undefined): number => (iso ? Date.parse(iso) : 0);
const newestFirst = (a: Placed, b: Placed) => ms(b.row.at) - ms(a.row.at);

/** The shelves top to bottom, each with how its rows are ordered. Team and Done today keep the order they came in. */
const SHELVES: { id: ShelfId; label: string; order: (a: Placed, b: Placed) => number }[] = [
  { id: "needsYou", label: "Needs you", order: newestFirst },
  { id: "working", label: "Working", order: newestFirst },
  { id: "pinned", label: "Pinned", order: (a, b) => ms(a.mark.pinnedAt) - ms(b.mark.pinnedAt) },
  { id: "open", label: "Open", order: newestFirst },
  { id: "team", label: "Team", order: () => 0 },
  { id: "snoozed", label: "Snoozed", order: (a, b) => ms(a.mark.snoozedUntil) - ms(b.mark.snoozedUntil) },
  { id: "done", label: "Done today", order: () => 0 },
];

/** Open shows this many rows until you ask for the rest. */
export const OPEN_LIMIT = 6;

/** Who spoke last in a thread: the employee once its newest run (the report, or your last reply) has ended. */
export function lastSpeaker(t: Thread): "you" | "them" {
  return (lastReply(t) ?? t.root).endedAt ? "them" : "you";
}

/** The thread's newest finished run raised something: it called notify, was blocked, or broke. Mirrors Rust
 *  `threads::flagged` and `threads::broke`. */
export function flagged(t: Thread): boolean {
  const r = latestFinished(t);
  return r.needsYou.length > 0 || (r.denials.length > 0 && r.kind.type !== "tidy") || r.state === "failed" || r.state === "lost";
}

/** A snooze that has ended and you have not looked at since: snoozing means "remind me". */
export function woke(mark: Mark, now: number): boolean {
  return !!mark.snoozedUntil && ms(mark.snoozedUntil) <= now && ms(mark.seenAt) < ms(mark.snoozedUntil);
}

/** Activity after this is new to you. */
const seenFloor = (mark: Mark, since: string): number => Math.max(ms(mark.seenAt), ms(since));

/** Why a thread needs you, as one line. */
function threadReason(t: Thread, mark: Mark, now: number): string {
  const r = latestFinished(t);
  if (r.needsYou.length) return r.needsYou[0];
  if (r.denials.length && r.kind.type !== "tidy") return `Blocked on ${toolName(r.denials[0].tool)}`;
  if (r.state === "failed" || r.state === "lost") return "Didn't finish";
  if (woke(mark, now)) return "Snooze ended";
  return t.replies.length || t.root.kind.type === "task" ? "New reply" : "New report";
}

/** Every conversation as a row: open proposals, open threads, today's done threads, then each ongoing chat. */
function rowsOf(input: ShelvesInput): Row[] {
  const byId = new Map(input.employees.map((e) => [e.id, e]));
  const threadRow = (thread: Thread, done: boolean): Row[] => {
    const employee = byId.get(thread.root.employeeId);
    return employee ? [{ kind: "thread", key: thread.root.id, at: activityAt(thread), title: input.label(thread), employee, thread, done }] : [];
  };
  return [
    ...input.proposals.map((proposal): Row => ({
      kind: "proposal", key: `proposal:${proposal.id}`, at: proposal.createdAt,
      title: `Hire: ${proposal.draft.name}, ${proposal.draft.title}`, proposal, proposer: byId.get(proposal.employeeId),
    })),
    ...input.conversations.threads.flatMap((t) => threadRow(t, false)),
    ...input.conversations.done.flatMap((t) => threadRow(t, true)),
    ...input.conversations.chats.flatMap((head): Row[] => {
      const employee = byId.get(head.employeeId);
      return employee ? [{ kind: "chat", key: chatKey(head.employeeId), at: head.lastEndedAt ?? "", title: employee.name, employee, head }] : [];
    }),
  ];
}

/** A row placed on its shelf, with what the sidebar shows about it. */
function place(row: Row, input: ShelvesInput, now: number): Placed {
  const mark = input.marks[row.key] ?? {};
  const floor = seenFloor(mark, input.since);
  let busy = false;
  let queued = false;
  let needsYou = false;
  let reason: string | null = null;
  if (row.kind === "thread") {
    const t = row.thread;
    busy = threadBusy(t);
    const live = [t.root, ...t.replies].filter(isLive);
    queued = live.length > 0 && live.every((r) => r.state === "queued");
    needsYou = !busy && lastSpeaker(t) === "them" && (flagged(t) || ms(row.at) > floor || woke(mark, now));
    if (needsYou) reason = threadReason(t, mark, now);
  } else if (row.kind === "chat") {
    busy = row.head.live || row.head.queued;
    queued = row.head.queued && !row.head.live;
    needsYou = (!busy && ms(row.head.lastEndedAt) > floor) || woke(mark, now);
    if (needsYou) reason = woke(mark, now) ? "Snooze ended" : "New reply";
  } else {
    reason = row.proposer ? `${row.proposer.name} proposed a hire` : "Hire proposal";
  }
  const facts: Facts = {
    done: row.kind === "thread" && row.done,
    snoozed: ms(mark.snoozedUntil) > now,
    needsYou,
    busy,
    pinned: !!mark.pinnedAt,
  };
  const shelf = RULES.find((r) => r.when(row, facts))!.shelf;
  return { row, shelf, mark, busy, queued, reason };
}

/** The shelves that have rows, top to bottom. The `sticky` row stays where it was selected, except into or out
 *  of Done today, which only an explicit action causes. */
export function buildShelves(input: ShelvesInput, now: number, sticky: Sticky = null): Shelf[] {
  const placed = rowsOf(input).map((row) => {
    const p = place(row, input, now);
    const keep = sticky && sticky.key === row.key && p.shelf !== "done" && sticky.shelf !== "done";
    return keep ? { ...p, shelf: sticky.shelf } : p;
  });
  return SHELVES.flatMap(({ id, label, order }) => {
    const rows = placed.filter((p) => p.shelf === id).sort(order);
    return rows.length ? [{ id, label, rows }] : [];
  });
}

/** How the sidebar shows the shelves: which you collapsed, and whether Open shows all its rows. */
export type ShelvesView = { collapsed: ReadonlySet<ShelfId>; openAll: boolean };

/** The rows a shelf shows. */
export function shownRows(shelf: Shelf, view: ShelvesView): Placed[] {
  if (view.collapsed.has(shelf.id)) return [];
  return shelf.id === "open" && !view.openAll ? shelf.rows.slice(0, OPEN_LIMIT) : shelf.rows;
}

/** Every row on screen, top to bottom: what ⌘1 to ⌘9 count and what "the next row" means. */
export const visibleRows = (shelves: Shelf[], view: ShelvesView): Placed[] => shelves.flatMap((s) => shownRows(s, view));

/** The row to open after `key` leaves its place: the next one on screen, else the one before, or the first one
 *  when `key` is not on screen. Proposals open a sheet, so they are skipped. */
export function nextRow(rows: Placed[], key: string): Placed | null {
  const i = rows.findIndex((p) => p.row.key === key);
  if (i < 0) return rows.find((p) => p.row.kind !== "proposal") ?? null;
  const open = (p: Placed) => p.row.kind !== "proposal" && p.row.key !== key;
  return rows.slice(i + 1).find(open) ?? rows.slice(0, i).reverse().find(open) ?? null;
}

/** Snooze choices from `now`: in 3 hours, tomorrow at 9:00 am, next Monday at 9:00 am. */
export function snoozePresets(now: Date): { label: string; until: Date }[] {
  const at9 = (days: number) => new Date(now.getFullYear(), now.getMonth(), now.getDate() + days, 9, 0, 0, 0);
  const toMonday = ((8 - now.getDay()) % 7) || 7;
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
