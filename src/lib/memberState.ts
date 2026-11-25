import type { EmployeeView, PersonView, Run } from "../api/types";
import { whenInline } from "./format";
import { workTitle } from "./workTitle";

export type StateTone = "live" | "wait" | "attn" | "fail" | "plain";
/** An employee's state in a word (or a short phrase when they are idle), and what it is about. */
export type MemberState = { tone: StateTone; word: string; about: string | null; run: Run | null };

export function employeeState(e: EmployeeView, employees: EmployeeView[], people: PersonView[]): MemberState {
  const running = e.live.find((r) => r.state === "running") ?? null;
  const queued = e.live.find((r) => r.state === "queued") ?? null;
  if (running) return { tone: "live", word: "Working", about: workTitle(running, employees, people), run: running };
  if (queued) return { tone: "wait", word: "Up next", about: workTitle(queued, employees, people), run: queued };
  if (e.presence === "attention" && e.lastRun) {
    const word = e.lastRun.denials.length ? "Needs your OK" : e.lastRun.state === "lost" ? "Interrupted" : "Last run failed";
    return { tone: e.lastRun.denials.length ? "attn" : "fail", word, about: null, run: e.lastRun };
  }
  if (e.nextFire) return { tone: "plain", word: `Next run ${whenInline(e.nextFire)}`, about: null, run: null };
  return { tone: "plain", word: "Works when asked", about: null, run: null };
}

/** An employee's number ("metric value (change)") split into what it is and how it moved, so a card can show the value
 *  alone and the band can say the change in words: "Open loops 4, down 3". The change is the bracket that closes the
 *  line, matched by depth, so a change with brackets of its own ("+3 (41 last week)") stays whole. */
export function numberParts(n: string | null): { metric: string; value: string; change: string | null } | null {
  const t = n?.trim();
  if (!t) return null;
  const open = closingGroupStart(t);
  const base = (open < 0 ? t : t.slice(0, open)).trim();
  const cut = base.lastIndexOf(" ");
  const [metric, value] = cut > 0 ? [base.slice(0, cut), base.slice(cut + 1)] : ["", base];
  const m = open < 0 ? null : /^([+\-−]?)\s*(.*)$/.exec(t.slice(open + 1, -1).trim());
  let change: string | null = null;
  if (m && m[2] && !/^0+([.,]0+)?%?$/.test(m[2])) {
    change = m[1] === "-" || m[1] === "−" ? `down ${m[2]}` : m[1] === "+" || /^\d/.test(m[2]) ? `up ${m[2]}` : m[2];
  }
  return { metric, value, change };
}

/** Where the bracket group that ends `t` opens, or -1 when `t` does not end in a balanced one. */
function closingGroupStart(t: string): number {
  if (!t.endsWith(")")) return -1;
  let depth = 0;
  for (let i = t.length - 1; i >= 0; i--) {
    if (t[i] === ")") depth++;
    else if (t[i] === "(" && --depth === 0) return i;
  }
  return -1;
}

/** The reporting lines that are carrying work right now: a manager's hand-off or question to their own report, while it
 *  runs. Keyed as the layout keys its edges: "<manager>><report>". */
export function liveLines(employees: EmployeeView[]): Set<string> {
  const lit = new Set<string>();
  for (const e of employees) {
    if (!e.managerId) continue;
    for (const r of e.live) {
      if (r.state !== "running") continue;
      if (r.kind.type === "delegated" || (r.kind.type === "asked" && r.kind.askedBy === e.managerId)) lit.add(`${e.managerId}>${e.id}`);
    }
  }
  return lit;
}

/** A real person: ringed, and "Person" where an employee has a state, unless an employee is waiting on them. */
export function personLine(p: PersonView): { waiting: boolean; text: string } {
  return p.openRequests > 0 ? { waiting: true, text: `${p.openRequests} waiting on them` } : { waiting: false, text: "Person" };
}
