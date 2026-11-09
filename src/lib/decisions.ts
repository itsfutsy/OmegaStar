// The Inbox as decisions: one entry per thing waiting on you, read from the rows the Inbox shows. The questions a
// run asks are one entry, since it hears your answers together. A question several employees asked is one entry,
// and answering it answers them all; where one of them offers other choices, their card shows in it too. Runs that
// didn't finish on the same error close together are one entry. Pure.
import type { Card, Run, Thread } from "../api/types";
import { activityAt, blockedNow, broken, flags, latestFinished, rowThreads, STREAK, topic, type OpenCard } from "./threads";

/** Something waiting on you in a row (`item`). `thread`: the row's thread it is in; `run`: the run it is about. */
export type Decision<T> = { id: string; item: T; thread: Thread; run: Run } & (
  | { type: "card"; cards: OpenCard[]; also: Run[]; differ: OpenCard[] }   // cards: the run's, open or held; also: other runs asking the same; differ: other runs' cards on it with other choices
  | { type: "flag"; text: string }                            // something Omega Star flagged
  | { type: "blocked" }                                        // blocked on a tool you could allow
  | { type: "broken"; times: number }                          // broke; times: a schedule's failures in a row
  | { type: "failures"; failures: BrokenDecision<T>[] }        // runs that broke on the same error close together, oldest first
  | { type: "answered" }                                       // the employee answered you
);

export type CardDecision<T> = Extract<Decision<T>, { type: "card" }>;
type BrokenDecision<T> = Extract<Decision<T>, { type: "broken" }>;

/** One employee's choice cards about the same thing (`about`, from keys `ask:<about>:...`), oldest first. */
export type Group<T> = { type: "group"; id: string; employeeId: string; about: string; decisions: CardDecision<T>[] };

/** How far apart runs may break on the same error and still be one entry. */
const FAILURES_WITHIN_MS = 2 * 3_600_000;

const scheduled = (x: Thread): boolean => x.root.kind.type === "scheduled";

/** A run's cards its employee has yet to hear about: open, or answered and held back until the rest are. */
const unsent = (run: Run): OpenCard[] =>
  run.cards.flatMap((card, index) => (!card.answer || card.answer.held ? [{ run, index, card }] : []));

/** What a row waits on you for, newest first. Mirrors Rust `threads::needs_you`: cards anywhere, and what each
 *  thread's newest finished run flagged, was blocked on or broke; a schedule's failures count once `STREAK` of them
 *  are in a row, as one decision. A row that asks nothing is the employee answering you. */
function rowDecisions<T extends { thread: Thread }>(item: T): Decision<T>[] {
  const all = rowThreads(item.thread);
  const open = all.filter((x) => !x.root.archivedAt).reverse();
  const out: Decision<T>[] = [];
  for (const x of open) {
    for (const run of [...x.replies].reverse().concat(x.root)) {
      const cards = unsent(run);
      if (cards.some((c) => !c.card.answer)) out.push({ type: "card", id: `${run.id}:cards`, item, thread: x, run, cards, also: [], differ: [] });
    }
    const last = latestFinished(x);
    flags(last).forEach((text, i) => out.push({ type: "flag", id: `${last.id}:f${i}`, item, thread: x, run: last, text }));
    if (blockedNow(x)) out.push({ type: "blocked", id: `${last.id}:blocked`, item, thread: x, run: last });
  }
  const failures = all.filter((x) => scheduled(x) && broken(x.root)).length;
  let streak = false;
  for (const x of open) {
    const last = latestFinished(x);
    if (!broken(last) || (scheduled(x) && (failures < STREAK || streak))) continue;
    streak ||= scheduled(x);
    out.push({ type: "broken", id: `${last.id}:broken`, item, thread: x, run: last, times: scheduled(x) ? failures : 1 });
  }
  if (out.length) return out;
  return [{ type: "answered", id: `${item.thread.root.id}:answered`, item, thread: item.thread, run: latestFinished(item.thread) }];
}

/** Two cards on the same thing ask it differently: other choices, or another one recommended. */
const differs = (a: Card, b: Card): boolean => JSON.stringify(a.ask) !== JSON.stringify(b.ask);

/** Every decision in `items`, in their order. A lone question also in another decision joins it, the first with it:
 *  as `also` when it offers the same choices, else in `differ`, so you see each recommendation. A run's several
 *  questions stay together, and failures on the same error close together become one (`failuresTogether`). */
export function decisions<T extends { thread: Thread }>(items: T[]): Decision<T>[] {
  const all = items.flatMap(rowDecisions);
  const first = new Map<string, CardDecision<T>>();
  const claim = (d: CardDecision<T>) => {
    for (const c of d.cards) {
      const key = c.card.answer ? null : topic(c.card);
      if (key && !first.has(key)) first.set(key, d);
    }
  };
  for (const d of all) if (d.type === "card" && d.cards.length > 1) claim(d);
  const out: Decision<T>[] = [];
  for (const d of all) {
    if (d.type === "card" && d.cards.length === 1) {
      const key = topic(d.cards[0].card);
      const seen = key ? first.get(key) : undefined;
      if (seen) {
        const theirs = seen.cards.find((c) => !c.card.answer && topic(c.card) === key);
        if (theirs && differs(theirs.card, d.cards[0].card)) seen.differ.push(d.cards[0]);
        else seen.also.push(d.run);
        continue;
      }
      claim(d);
    }
    out.push(d);
  }
  return failuresTogether(out);
}

/** When a decision started waiting on you: when its run ended. */
const since = <T>(d: Decision<T>): string => d.run.endedAt ?? d.run.createdAt;

/** An error's first line without what changes from run to run (ids, numbers, times), so the same failure reads the
 *  same; null when a run broke without saying why. */
function errorKey(r: Run): string | null {
  const line = (r.error ?? "").split("\n").map((l) => l.trim()).find(Boolean);
  if (!line) return null;
  return line.toLowerCase()
    .replace(/\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b/g, "#")
    .replace(/\b\d{1,2}(?::\d{2}){0,2}\s*(?:am|pm)\b/g, "#")
    .replace(/\b[0-9a-f]*\d[0-9a-f]*\b/g, "#");
}

/** Runs that broke on the same error (`errorKey`) within `FAILURES_WITHIN_MS` of the first of them, as one decision in
 *  place of the first. A schedule failing time after time is one decision already, so it stays apart. */
function failuresTogether<T>(ds: Decision<T>[]): Decision<T>[] {
  const lone = ds
    .filter((d): d is BrokenDecision<T> => d.type === "broken" && d.times === 1 && errorKey(d.run) !== null)
    .sort((a, b) => since(a).localeCompare(since(b)));
  const sets: BrokenDecision<T>[][] = [];
  for (const d of lone) {
    const set = sets.find((s) => errorKey(s[0].run) === errorKey(d.run) && Date.parse(since(d)) - Date.parse(since(s[0])) <= FAILURES_WITHIN_MS);
    if (set) set.push(d);
    else sets.push([d]);
  }
  const lead = new Map<string, BrokenDecision<T>[]>();
  const joined = new Set<string>();
  for (const s of sets.filter((s) => s.length > 1)) {
    lead.set(s[0].id, s);
    for (const d of s) joined.add(d.id);
  }
  return ds.flatMap((d): Decision<T>[] => {
    const s = lead.get(d.id);
    if (s) return [{ type: "failures", id: `${s[0].id}:failures`, item: s[0].item, thread: s[0].thread, run: s[0].run, failures: s }];
    return joined.has(d.id) ? [] : [d];
  });
}

/** What a lone choice card keyed `ask:<about>:...` is about, so one employee's cards about the same thing are listed
 *  together; null for anything else, a list card or a card someone asks with other choices included. */
function about<T>(d: Decision<T>): string | null {
  if (d.type !== "card" || d.cards.length !== 1 || d.differ.length) return null;
  const c = d.cards[0].card;
  if (c.ask.type !== "choices" || !c.key?.startsWith("ask:")) return null;
  return c.key.slice("ask:".length).split(":")[0] || null;
}

/** The Inbox in two lists. Decisions (cards, blocks, failures, proposals, flags), oldest waiting first, with one
 *  employee's choice cards about the same thing together in a `Group` where the first of them waits; and Replies
 *  (the employee answered you), newest first. */
export function inboxLists<T>(ds: Decision<T>[]): { decisions: (Decision<T> | Group<T>)[]; replies: Decision<T>[] } {
  const waiting = ds.filter((d) => d.type !== "answered").sort((a, b) => since(a).localeCompare(since(b)));
  const replies = ds.filter((d) => d.type === "answered").sort((a, b) => activityAt(b.thread).localeCompare(activityAt(a.thread)));
  const key = (d: Decision<T>) => {
    const a = about(d);
    return a ? `${d.run.employeeId}:${a}` : null;
  };
  const count = new Map<string, number>();
  for (const d of waiting) {
    const k = key(d);
    if (k) count.set(k, (count.get(k) ?? 0) + 1);
  }
  const groups = new Map<string, Group<T>>();
  const out: (Decision<T> | Group<T>)[] = [];
  for (const d of waiting) {
    const k = key(d);
    if (!k || d.type !== "card" || (count.get(k) ?? 0) < 2) {
      out.push(d);
      continue;
    }
    const g = groups.get(k);
    if (g) g.decisions.push(d);
    else {
      const ng: Group<T> = { type: "group", id: `group:${k}`, employeeId: d.run.employeeId, about: about(d)!, decisions: [d] };
      groups.set(k, ng);
      out.push(ng);
    }
  }
  return { decisions: out, replies };
}
