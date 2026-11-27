import type { TeamStats } from "../api/types";

/** How far back the Analytics page looks. */
export type TeamRange = "7" | "30" | "all";
export const RANGES: { id: TeamRange; label: string }[] = [{ id: "7", label: "7 days" }, { id: "30", label: "30 days" }, { id: "all", label: "All time" }];

/** One column of the chart: a day, or a week once the range runs past a month. `day` is its first day. */
export type TeamPoint = { day: string; workMs: number; asks: number };

/** Someone on the team now: the hours they worked in the range and the number they answer for, if they have one yet. */
export type TeamPerson = { employeeId: string; name: string; workMs: number; number: TeamStats["numbers"][number] | null };

/** The model families the page names, in the order it shows them; any other model goes by its own id. */
const FAMILIES = ["Fable", "Opus", "Sonnet", "Haiku"];
/** A model id's family: `claude-opus-4-5-20251101` is Opus. */
export const modelFamily = (id: string) => FAMILIES.find((f) => id.toLowerCase().includes(f.toLowerCase())) ?? id;

/** Someone's tokens in the range: those their own runs used, by model family, and those used by work they handed over. */
export type TokenPerson = { employeeId: string; name: string; used: Record<string, number>; total: number; handedOver: number };

/** The Analytics page's numbers for one range: what the screen's two sentences, chart and table say. */
export type TeamSummary = {
  /** Who works here now. */
  team: number;
  /** When the first person was hired: where All time begins. */
  since: Date;
  runs: number; workMs: number; nightMs: number;
  /** Time worked over the equal span just before. Null for All time, and when the team did not exist for all of it. */
  workBefore: number | null;
  /** Cards that asked you something in the range. */
  asks: number;
  /** Half the asks got an answer within this. Null when fewer than half have one: an open ask counts as not yet, and one
   *  a newer card replaced or the employee withdrew is left out, since nobody waited on you for it. */
  medianWaitMs: number | null;
  /** Asks answered after more than a day, and asks still open more than a day after they came. */
  overDay: number;
  points: TeamPoint[];
  per: "day" | "week";
  /** Everyone here now, most hours first. */
  people: TeamPerson[];
  /** The model families any run used in the range, known ones first. */
  models: string[];
  /** Everyone here now who used tokens or handed over work that did, most tokens first. */
  tokens: TokenPerson[];
  /** The first day any run kept its tokens, in any range: runs before it count none. */
  tokensSince: string | null;
};

const DAY = 86_400_000;
const pad = (n: number) => String(n).padStart(2, "0");
/** A local date as `YYYY-MM-DD`, as the Office groups days. */
export const dayKey = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

/** The Analytics page's numbers for a range, from the Office's all-time rows. Days with nothing done still appear. */
export function summarise(stats: TeamStats, range: TeamRange, now = new Date()): TeamSummary {
  const day = (back: number) => new Date(now.getFullYear(), now.getMonth(), now.getDate() - back);
  const since = new Date(Math.min(now.getTime(), ...stats.staff.map((s) => Date.parse(s.hiredAt))));
  const teamAgeDays = Math.round((day(0).getTime() - new Date(since.getFullYear(), since.getMonth(), since.getDate()).getTime()) / DAY) + 1;
  const span = range === "all" ? teamAgeDays : Math.min(Number(range), teamAgeDays);
  const days = (from: number, n: number) => Array.from({ length: n }, (_, i) => dayKey(day(from + n - 1 - i)));
  const before = range !== "all" && teamAgeDays >= 2 * Number(range) ? new Set(days(Number(range), Number(range))) : null;

  const byDay = new Map(days(0, span).map((d): [string, TeamPoint] => [d, { day: d, workMs: 0, asks: 0 }]));
  const worked = new Map<string, number>();
  let runs = 0, workMs = 0, nightMs = 0, workBefore = 0;
  for (const r of stats.days) {
    const p = byDay.get(r.day);
    if (p) {
      p.workMs += r.workMs; runs += r.runs; workMs += r.workMs; nightMs += r.nightMs;
      worked.set(r.employeeId, (worked.get(r.employeeId) ?? 0) + r.workMs);
    } else if (before?.has(r.day)) workBefore += r.workMs;
  }

  let asks = 0, overDay = 0;
  const waits: number[] = [];
  for (const a of stats.asks) {
    const p = byDay.get(dayKey(new Date(a.at)));
    if (!p) continue;
    p.asks++; asks++;
    const at = Date.parse(a.at);
    if (a.answeredAt) { const w = Math.max(0, Date.parse(a.answeredAt) - at); waits.push(w); if (w > DAY) overDay++; }
    else if (a.open) { waits.push(Infinity); if (now.getTime() - at > DAY) overDay++; }
  }
  const median = waits.sort((x, y) => x - y)[Math.ceil(waits.length / 2) - 1];

  const all = [...byDay.values()];
  const per = all.length > 31 ? "week" : "day";
  // Weeks count back from today, so only the oldest can be short.
  const points: TeamPoint[] = per === "day" ? all : [];
  if (per === "week") for (let i = all.length; i > 0; i -= 7) {
    const wk = all.slice(Math.max(0, i - 7), i);
    points.unshift({ day: wk[0].day, workMs: wk.reduce((n, d) => n + d.workMs, 0), asks: wk.reduce((n, d) => n + d.asks, 0) });
  }

  const used = new Map<string, Record<string, number>>(), handed = new Map<string, number>(), seen = new Set<string>();
  for (const t of stats.tokens) {
    if (!byDay.has(t.day)) continue;
    const family = modelFamily(t.model), mine = used.get(t.employeeId) ?? {};
    mine[family] = (mine[family] ?? 0) + t.tokens;
    used.set(t.employeeId, mine);
    seen.add(family);
    if (t.handedOverBy) handed.set(t.handedOverBy, (handed.get(t.handedOverBy) ?? 0) + t.tokens);
  }
  const rank = (f: string) => (FAMILIES.includes(f) ? FAMILIES.indexOf(f) : FAMILIES.length);
  const models = [...seen].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));

  const here = stats.staff.filter((s) => !s.departedAt);
  const tokens = here
    .map((s) => {
      const mine = used.get(s.employeeId) ?? {};
      return { employeeId: s.employeeId, name: s.name, used: mine, total: Object.values(mine).reduce((n, x) => n + x, 0), handedOver: handed.get(s.employeeId) ?? 0 };
    })
    .filter((p) => p.total || p.handedOver)
    .sort((a, b) => b.total - a.total || b.handedOver - a.handedOver);
  const people = here
    .map((s) => ({ employeeId: s.employeeId, name: s.name, workMs: worked.get(s.employeeId) ?? 0, number: stats.numbers.find((n) => n.employeeId === s.employeeId) ?? null }))
    .sort((a, b) => b.workMs - a.workMs);
  return {
    team: here.length, since, runs, workMs, nightMs,
    workBefore: before ? workBefore : null,
    asks, medianWaitMs: median !== undefined && Number.isFinite(median) ? median : null, overDay,
    points, per, people, models, tokens, tokensSince: stats.tokens[0]?.day ?? null,
  };
}
