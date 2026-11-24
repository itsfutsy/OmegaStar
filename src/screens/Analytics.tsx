import { useMemo, useState } from "react";
import { call } from "../api/client";
import { changeWords } from "../lib/format";
import { useOrg, useQuery } from "../lib/hooks";
import { RANGES, summarise, type TeamPoint, type TeamRange, type TeamSummary } from "../lib/teamStats";
import { useShell } from "../Shell";
import { Icon } from "../components/Icon";
import { Avatar, ErrorNote, PageColumn, Section, Skeleton } from "../components/ui";
import { EmptyPage } from "./ActivityParts";
import "./Analytics.css";

/** Keeps a number on one line with its unit. */
const NBSP = "\u00a0";
/** Team sizes up to twelve read as words. */
const WORDS = ["no one", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];
/** Hours from milliseconds: whole from 10 up, one decimal below, and never less than 0.1 for any work at all. */
const hours = (ms: number) => {
  const h = ms / 3_600_000;
  return h >= 10 || h === 0 ? Math.round(h).toLocaleString() : Math.max(0.1, h).toLocaleString(undefined, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
};
/** A count with its noun, one or many: "1 job", "551 jobs". */
const count = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()}${NBSP}${n === 1 ? one : many}`;
/** How long an answer took, to follow "within", so rounded up: "a minute", "8 min", "2 h 21 min", "3 days". */
function waitWords(ms: number): string {
  const m = Math.ceil(ms / 60_000);
  if (m <= 1) return "a minute";
  if (m < 60) return `${m}${NBSP}min`;
  if (m < 1440) return m % 60 ? `${Math.floor(m / 60)}${NBSP}h ${m % 60}${NBSP}min` : `${m / 60}${NBSP}h`;
  return count(Math.ceil(m / 1440), "day");
}
/** The plain fractions the overnight share is rounded to. */
const SHARES: [number, string][] = [[1 / 10, "a tenth"], [1 / 5, "a fifth"], [1 / 4, "a quarter"], [1 / 3, "a third"], [1 / 2, "half"], [2 / 3, "two thirds"], [0.9, "most"]];
/** A share as a plain fraction, the nearest one: "Nearly a third", "About half", "Most". */
function shareWords(f: number): string {
  const [near, word] = SHARES.reduce((a, b) => (Math.abs(b[0] - f) < Math.abs(a[0] - f) ? b : a));
  if (word === "most") return "Most";
  return `${Math.abs(f - near) < 0.015 ? "About" : f < near ? "Nearly" : "Just over"} ${word}`;
}
/** A token count, short: "840", "12k", "3.4M". The exact number is in its tooltip. */
const tokenWords = (n: number) =>
  !n ? "–" : n >= 1e6 ? `${(n / 1e6).toLocaleString(undefined, { maximumFractionDigits: 1 })}M` : n >= 1e3 ? `${Math.round(n / 1e3)}k` : String(n);
/** Midday on a `YYYY-MM-DD` day, so no time zone shift lands it on the day before. */
const dayDate = (day: string) => new Date(`${day}T12:00:00`);

/**
 * The Analytics page, in the sidebar under the Org chart: what the team got done, what it asked of you, and the number
 * each person answers for, over the last week, month or all time. It only shows, it changes nothing. Two sentences carry
 * the range; one mirrored chart shows hours above the line and asks below; hairline rows show each person.
 */
export function AnalyticsScreen() {
  const q = useQuery(() => call("team_stats"), [], ["org:changed"]);
  const { snap } = useOrg();
  const shell = useShell();
  const [range, setRange] = useState<TeamRange>("7");
  const s = useMemo(() => (q.data ? summarise(q.data, range) : null), [q.data, range]);
  const ranges = (
    <div className="seg" role="group" aria-label="How far back">
      {RANGES.map((r) => <button key={r.id} aria-pressed={range === r.id} onClick={() => setRange(r.id)}>{r.label}</button>)}
    </div>
  );

  return (
    <PageColumn title="Analytics" className="an" actions={ranges}>
      {q.error ? <ErrorNote title="Couldn't count the team" text={q.error} onRetry={q.reload} /> : null}
      {!s && !q.error ? <Skeleton h={220} r={8} /> : null}
      {s && !s.team ? (
        <EmptyPage
          title="Nobody works here yet"
          text="Once you hire someone, their week shows up here: the hours they worked, what they asked you, and the number they answer for."
          action={<button className="btn btn-primary" onClick={(ev) => shell.hire(null, ev.currentTarget)}><Icon name="plus" size={14} strokeWidth={1.8} /> Hire an employee</button>}
        />
      ) : null}
      {s?.team ? (
        <>
          <div className="an-lede">
            <Work s={s} range={range} />
            <Asks s={s} />
          </div>
          {s.workMs || s.asks ? <Mirror points={s.points} per={s.per} /> : null}
          <Section title="The number each person answers for">
            <div className="an-rows">
              <div className="an-row an-head" aria-hidden="true">
                <span /><span /><span>Their number</span><span className="an-r">Now</span><span>Against last week</span><span className="an-r">Worked</span>
              </div>
              {s.people.map((p) => {
                const e = snap?.employees.find((x) => x.id === p.employeeId);
                return (
                  <div className="an-row" key={p.employeeId}>
                    {e ? <Avatar name={e.name} hue={e.hue} image={e.image} size={20} /> : <span />}
                    <span className="an-name" title={p.name}>{p.name}</span>
                    {p.number ? <span className="an-metric" title={p.number.metric}>{p.number.metric}</span> : <span className="an-metric an-none">No weekly number yet</span>}
                    <span className="an-value num" title={p.number?.value}>{p.number?.value || "–"}</span>
                    <span className="an-change" title={p.number ? changeWords(p.number.change) : undefined}>{p.number ? changeWords(p.number.change) : ""}</span>
                    <span className="an-hours num">{hours(p.workMs)}{NBSP}h</span>
                  </div>
                );
              })}
            </div>
          </Section>
          <Tokens s={s} />
        </>
      ) : null}
    </PageColumn>
  );
}

/**
 * Tokens each person used in the range, one column per model, and the tokens used by work they handed to a colleague.
 * A handed-over run counts to whoever did it, so nothing is counted twice across the Used columns.
 */
function Tokens({ s }: { s: TeamSummary }) {
  const { snap } = useOrg();
  const since = s.tokensSince ? dayDate(s.tokensSince).toLocaleDateString(undefined, { day: "numeric", month: "short" }) : "";
  const cols = { gridTemplateColumns: `20px 120px repeat(${s.models.length}, minmax(56px, 1fr)) 64px 96px` };
  return (
    <Section title="Tokens each person used, by model">
      {s.tokens.length ? (
        <div className="an-rows">
          <div className="an-row an-trow an-head" style={cols} aria-hidden="true">
            <span /><span />{s.models.map((m) => <span key={m} className="an-r">{m}</span>)}<span className="an-r">Total</span><span className="an-r">Handed over</span>
          </div>
          {s.tokens.map((p) => {
            const e = snap?.employees.find((x) => x.id === p.employeeId);
            return (
              <div className="an-row an-trow" style={cols} key={p.employeeId}>
                {e ? <Avatar name={e.name} hue={e.hue} image={e.image} size={20} /> : <span />}
                <span className="an-name" title={p.name}>{p.name}</span>
                {s.models.map((m) => <span key={m} className="an-tok num" title={`${(p.used[m] ?? 0).toLocaleString()} ${m} tokens`}>{tokenWords(p.used[m] ?? 0)}</span>)}
                <span className="an-tok an-tok-total num" title={`${p.total.toLocaleString()} tokens`}>{tokenWords(p.total)}</span>
                <span className="an-tok num" title={`${p.handedOver.toLocaleString()} tokens used by work ${p.name} asked a colleague for`}>{tokenWords(p.handedOver)}</span>
              </div>
            );
          })}
        </div>
      ) : null}
      <p className="an-foot">
        {s.tokens.length
          ? "Work counts to whoever did it. Handed over is what the work they asked colleagues for used. "
          : "No tokens counted in this range. "}
        {since ? `Counted since ${since}; earlier runs kept no token counts.` : "Runs keep token counts from this version on."}
      </p>
    </Section>
  );
}

/** The first sentence: how much the team worked against the span before, what it finished, and how much was overnight. */
function Work({ s, range }: { s: TeamSummary; range: TeamRange }) {
  const team = `Your team of ${s.team <= 12 ? WORDS[s.team] : s.team.toLocaleString()}`;
  const span = range === "all" ? "" : `in the last ${range} days`;
  if (!s.runs) return <p>{range === "all" ? `${team} hasn't finished anything yet.` : `${team} finished nothing ${span}.`}</p>;
  const h = hours(s.workMs);
  const worked = <b>{h}{NBSP}hours</b>;
  const jobs = <b>{count(s.runs, "job")}</b>;
  const share = s.nightMs / s.workMs;
  const night = share >= 0.1 ? <> <b>{shareWords(share)}</b> of the hours were overnight.</> : null;
  if (range === "all") {
    const year = s.since.getFullYear() === new Date().getFullYear() ? {} : { year: "numeric" } as const;
    const since = s.since.toLocaleDateString(undefined, { day: "numeric", month: "short", ...year });
    return <p>{team} has worked {worked} since {since} and finished {jobs}.{night}</p>;
  }
  let against = null;
  if (s.workBefore !== null) {
    const before = range === "7" ? "the week before" : `the ${range} days before`, b = hours(s.workBefore);
    against = b === h ? <>, the same as {before}</> : <>, {s.workMs < s.workBefore ? "down" : "up"} from <b>{b}</b> {before}</>;
  }
  return <p>{team} worked {worked} {span}{against}{against ? "," : ""} and finished {jobs}.{night}</p>;
}

/** The second sentence: how often they asked you, how fast half got an answer, and how many waited more than a day. */
function Asks({ s }: { s: TeamSummary }) {
  if (!s.asks) return <p>They asked you nothing.</p>;
  const one = s.asks === 1;
  let after = null;
  if (s.medianWaitMs !== null) {
    const within = <>{one ? "It" : "Half"} got an answer within <b>{waitWords(s.medianWaitMs)}</b></>;
    after = one ? <> {within}.</> : <> {within}, and {s.overDay ? <><b>{s.overDay.toLocaleString()}</b>{NBSP}waited</> : "none waited"} more than a day.</>;
  } else if (s.overDay) {
    if (!one) return <p>They asked you <b>{count(s.asks, "thing")}</b>, and <b>{s.overDay.toLocaleString()}</b>{NBSP}waited more than a day.</p>;
    after = <> It has waited more than a day.</>;
  }
  return <p>They asked you <b>{count(s.asks, "thing")}</b>.{after}</p>;
}

/** The chart's width, the room above, between and below its two baselines, and where they sit. */
const W = 772, UP = 96, MID = 22, DOWN = 66, BASE = UP, LOW = BASE + MID, H = LOW + DOWN + 4;
/** The room the tallest bars leave on each side for their value and the corner labels. */
const HEAD = 30, FOOT = 32;

/**
 * Hours worked as ink bars rising from the upper baseline, and the times they asked you as coral bars hanging from the
 * lower one. Up to ten columns each carry their values and a label; past that only the tallest on each side does, and a
 * few evenly spaced columns carry a date.
 */
function Mirror({ points, per }: { points: TeamPoint[]; per: "day" | "week" }) {
  const n = points.length, step = W / n, dense = n <= 10;
  const bw = dense ? Math.min(56, step * 0.72) : Math.max(2, step * 0.6);
  const maxW = Math.max(...points.map((p) => p.workMs)), maxA = Math.max(...points.map((p) => p.asks));
  const tallW = points.findIndex((p) => p.workMs === maxW), tallA = points.findIndex((p) => p.asks === maxA);
  const ticks = new Set(dense ? points.map((_, i) => i) : Array.from({ length: 6 }, (_, k) => Math.round((k * (n - 1)) / 5)));
  const short = (p: TeamPoint) => {
    const d = dayDate(p.day);
    return dense && per === "day" ? `${d.toLocaleDateString(undefined, { weekday: "short" })} ${d.getDate()}` : d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
  };
  const when = (p: TeamPoint) => (per === "week" ? `Week of ${short(p)}` : dayDate(p.day).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" }));
  const edgeAnchor = (i: number) => (dense ? "middle" : i === 0 ? "start" : i === n - 1 ? "end" : "middle");
  const lx = (i: number) => { const x = step * i + (step - bw) / 2; return edgeAnchor(i) === "start" ? x : edgeAnchor(i) === "end" ? x + bw : x + bw / 2; };
  const label = `Hours worked and times they asked you, each ${per}, from ${when(points[0])} to ${when(points[n - 1])}`;
  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="an-svg" role="img" aria-label={label}>
      <text className="an-tick" x={0} y={10}>Hours worked</text>
      <text className="an-tick" x={0} y={H - 3}>Times they asked you</text>
      <line className="an-axis" x1={0} x2={W} y1={BASE + 0.5} y2={BASE + 0.5} />
      <line className="an-axis" x1={0} x2={W} y1={LOW - 0.5} y2={LOW - 0.5} />
      {points.map((p, i) => {
        const x = step * i + (step - bw) / 2;
        const hh = p.workMs ? Math.max(2, (p.workMs / maxW) * (UP - HEAD)) : 0, ah = p.asks ? Math.max(2, (p.asks / maxA) * (DOWN - FOOT)) : 0;
        return (
          <g key={p.day}>
            {hh ? <rect className="an-ink" x={x} y={BASE - hh} width={bw} height={hh} rx={dense ? 3 : 1}><title>{`${when(p)}: ${hours(p.workMs)} h worked`}</title></rect> : null}
            {hh && (dense || i === tallW) ? <text className="an-val" x={lx(i)} y={BASE - hh - 6} textAnchor={edgeAnchor(i)}>{`${hours(p.workMs)} h`}</text> : null}
            {ticks.has(i) ? <text className="an-tick" x={lx(i)} y={BASE + 15} textAnchor={edgeAnchor(i)}>{short(p)}</text> : null}
            {ah ? <rect className="an-you" x={x} y={LOW} width={bw} height={ah} rx={dense ? 3 : 1}><title>{`${when(p)}: asked you ${count(p.asks, "time")}`}</title></rect> : null}
            {ah && (dense || i === tallA) ? <text className="an-val" x={lx(i)} y={LOW + ah + 14} textAnchor={edgeAnchor(i)}>{p.asks.toLocaleString()}</text> : null}
          </g>
        );
      })}
    </svg>
  );
}
