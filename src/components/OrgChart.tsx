import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Company, EmployeeView, PersonView, Run } from "../api/types";
import { whenInline } from "../lib/format";
import { YOU_ID, up } from "../lib/org";
import { COMPACT, WIDE, layoutOrg, type ChartMetrics } from "../lib/orgLayout";
import { workLabel } from "./RunRow";
import { Avatar } from "./ui";
import "./OrgChart.css";

export type OrgView = "everyone" | "employees" | "people";
export type OrgChartHandle = { fit: () => void };
/** How far a chart too wide for its window shrinks to fit before it pans instead. */
const MIN_ZOOM = 0.65;
type Org = { company: Company; employees: EmployeeView[]; people: PersonView[] };

// ---------------------------------------------------------------- what a member's card and row say

/** The first sentence of what was asked, whole: without "Please" and its closing full stop, never cut. */
function firstSentence(text: string): string {
  const line = text.split("\n").map((l) => l.trim().replace(/^[#>\-* ]+/, "").replace(/[*_`]/g, "")).find((l) => l) ?? "";
  const s = line.replace(/^(please|can you|could you|would you)[, ]+/i, "");
  const ends = [". ", "? ", "! "].map((e) => s.indexOf(e)).filter((i) => i >= 0);
  const out = (ends.length ? s.slice(0, Math.min(...ends)) : s).replace(/[?.!,:;]+$/, "").trim();
  return out.charAt(0).toUpperCase() + out.slice(1);
}

/** What a piece of work is called where a person's state is shown: the conversation's name without "Re:", the task a
 *  manager handed over without "Delegated by", in its own first sentence. */
export function workTitle(run: Run, employees: EmployeeView[], people: PersonView[]): string {
  if (run.title) return run.title;
  switch (run.kind.type) {
    case "reply": case "task": return run.kind.label;
    case "delegated": return firstSentence(run.prompt) || workLabel(run, employees, people);
    case "manual": {
      const w = workLabel(run, employees, people);
      return w === "Manual run" ? w : firstSentence(run.prompt) || w;
    }
    default: return workLabel(run, employees, people);
  }
}

/** A task short enough for a card: whole, or cut at its last clause break before `max` ("Check which price IDs change on
 *  1 March"), else at a word with an ellipsis. */
export function shortTask(text: string, max = 52): string {
  if (text.length <= max) return text;
  const head = text.slice(0, max);
  const breaks = [", ", "; ", " and ", " then ", " so ", " but "].map((b) => head.lastIndexOf(b)).filter((i) => i >= 16);
  if (breaks.length) return text.slice(0, Math.max(...breaks));
  return `${head.slice(0, head.lastIndexOf(" "))}…`;
}

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
 *  alone and the band can say the change in words: "Open loops 4, down 3". */
export function numberParts(n: string | null): { metric: string; value: string; change: string | null } | null {
  if (!n?.trim()) return null;
  const m = /^(.*?)\s*\(\s*([+\-−]?)\s*([^)]*?)\s*\)\s*$/.exec(n.trim());
  const base = (m ? m[1] : n).trim();
  const cut = base.lastIndexOf(" ");
  const [metric, value] = cut > 0 ? [base.slice(0, cut), base.slice(cut + 1)] : ["", base];
  let change: string | null = null;
  if (m && m[3] && !/^0+([.,]0+)?%?$/.test(m[3])) change = `${m[2] === "-" || m[2] === "−" ? "down" : "up"} ${m[3]}`;
  return { metric, value, change };
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

/** Nearest node above `id` that is visible under the current view (you always are), so filtering never orphans a subtree. */
function visibleParent(org: Org, id: string, visible: Set<string>): string | null {
  const seen = new Set<string>();
  let cur = up(org, id);
  while (cur !== null && cur !== YOU_ID && !visible.has(cur) && !seen.has(cur)) {
    seen.add(cur);
    cur = up(org, cur);
  }
  return cur !== null && (cur === YOU_ID || visible.has(cur)) ? cur : null;
}

/** Everyone the view shows, you first, each with the visible member they hang from. */
export function visibleMembers(org: Org, view: OrgView): { id: string; parentId: string | null }[] {
  const shown = [...(view === "people" ? [] : org.employees), ...(view === "employees" ? [] : org.people)];
  const visible = new Set(shown.map((m) => m.id));
  return [YOU_ID, ...visible].map((id) => ({ id, parentId: visibleParent(org, id, visible) }));
}

// ---------------------------------------------------------------- card heights

const LINE_H = { 2: 58, 3: 78, 4: 94 } as const;
/** The text a card's last line carries: what they are working on, shortened at a clause, else their number. */
function glanceOf(e: EmployeeView, s: MemberState): string | null {
  if (s.about) return shortTask(s.about);
  const n = numberParts(e.status.number);
  return n ? [n.metric, n.value].filter(Boolean).join(" ") : null;
}
// The width a card's text has (card, less its padding, avatar and gap), for deciding when the last line wraps.
const TEXT_W = WIDE.w - 24 - 32 - 10;
let measure: CanvasRenderingContext2D | null = null;
/** How wide a line of UI text sets, for deciding whether it fits before it is drawn. */
export function textWidth(text: string, weight: number, size: number): number {
  measure ??= document.createElement("canvas").getContext("2d");
  if (!measure) return text.length * size * 0.52;
  const family = getComputedStyle(document.documentElement).getPropertyValue("--font-ui").trim() || "system-ui";
  measure.font = `${weight} ${size}px ${family}`;
  return measure.measureText(text).width;
}
const wraps = (text: string) => textWidth(text, 400, 12) > TEXT_W - 2;
/** How tall a card is: the lines it carries decide it, so a card never has a void under its words. A task that does not
 *  fit one line takes a second rather than being cut. */
function cardHeight(org: Org, id: string, compact: boolean): number {
  if (compact || id === YOU_ID) return LINE_H[2];
  const e = org.employees.find((x) => x.id === id);
  if (e) {
    const glance = glanceOf(e, employeeState(e, org.employees, org.people));
    return glance ? LINE_H[4] + (wraps(glance) ? 16 : 0) : LINE_H[3];
  }
  // A person: name and role, and a third line only while an employee waits on them (the ring says "person").
  const p = org.people.find((x) => x.id === id);
  return p && p.openRequests > 0 ? LINE_H[3] : LINE_H[2];
}

/** How much each member matters when choosing what a panning chart opens on: needing you, then working, then up next. */
function weightOf(org: Org, id: string): number {
  const e = org.employees.find((x) => x.id === id);
  if (!e) return 0;
  const t = employeeState(e, org.employees, org.people).tone;
  return t === "attn" || t === "fail" ? 3 : t === "live" ? 3 : t === "wait" ? 1 : 0;
}

// ---------------------------------------------------------------- the chart

export const OrgChart = forwardRef<OrgChartHandle, {
  company: Company; employees: EmployeeView[]; people: PersonView[]; view: OrgView; compact: boolean;
  /** The canvas's size, measured by the page, so the first frame is already laid out for it. */
  frame: { w: number; h: number }; focusId: string | null; onOpen: (id: string) => void;
  /** Told whether you have moved the view away from the framing it opened on (so Recentre has something to do). */
  onMoved?: (moved: boolean) => void;
}>(function OrgChart({ company, employees, people, view, compact, frame, focusId, onOpen, onMoved }, ref) {
  const org = useMemo(() => ({ company, employees, people }), [company, employees, people]);
  const members = useMemo(() => visibleMembers(org, view), [org, view]);
  // The canvas's own size (the page's frame less its head, filter and foot), for deciding what fits.
  const scroller = useRef<HTMLDivElement>(null);
  const [canvas, setCanvas] = useState<{ w: number; h: number } | null>(null);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const measure = () => setCanvas((c) => (c && c.w === el.clientWidth && c.h === el.clientHeight ? c : { w: el.clientWidth, h: el.clientHeight }));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // The chart as drawn when it fits. A team with no reports of its own stacks under its manager once it is three
  // strong, or two when that is too wide for the window. Still too wide, it shrinks to fit, down to MIN_ZOOM, then
  // pans. When it nearly fits in height, the levels close up and the margins tighten so the whole chart shows at once;
  // taller than that, it scrolls down.
  const { layout, tight, zoom } = useMemo(() => {
    const W = canvas?.w ?? frame.w, H = canvas?.h ?? frame.h - 170;
    const input = members.map((m) => ({ ...m, h: cardHeight(org, m.id, compact) }));
    const metrics = compact ? COMPACT : WIDE;
    let stackMin = 3;
    let plain = layoutOrg(input, metrics, { stackMin });
    if (plain.width + 80 > W) {
      stackMin = 2;
      plain = layoutOrg(input, metrics, { stackMin });
    }
    if (plain.width + 80 > W) return { layout: plain, tight: false, zoom: Math.max(MIN_ZOOM, Math.floor(((W - 80) / plain.width) * 100) / 100) };
    if (plain.height + 72 <= H) return { layout: plain, tight: false, zoom: 1 };
    const close: ChartMetrics = { ...metrics, gapY: Math.round(metrics.gapY * 0.6) };
    const near = layoutOrg(input, close, { stackMin });
    return near.height + 32 <= H ? { layout: near, tight: true, zoom: 1 } : { layout: plain, tight: false, zoom: 1 };
  }, [members, org, compact, frame.w, frame.h, canvas]);
  const lit = useMemo(() => liveLines(employees), [employees]);

  // A member who joins while the chart is on screen drops in; the rest move to make room.
  const known = useRef<Set<string> | null>(null);
  const fresh = new Set<string>();
  if (known.current) for (const m of members) if (!known.current.has(m.id)) fresh.add(m.id);
  useEffect(() => { known.current = new Set(members.map((m) => m.id)); }, [members]);

  // What the canvas shows when the chart is larger than it: `focusId`'s card (the person you just closed), else the
  // framing that keeps the most whole cards in view, never cutting the ones that need you or are working, with the top
  // of the company in view where it can be. Only the chart's own scroller moves, never the page around it.
  const framed = useRef<{ left: number; top: number } | null>(null);
  const focus = useRef(focusId);
  focus.current = focusId;
  const shape = useRef({ layout, org, zoom });
  shape.current = { layout, org, zoom };
  const reveal = useCallback((smooth: boolean) => {
    const el = scroller.current;
    const canvas = el?.querySelector<HTMLElement>(".orgcanvas");
    if (!el || !canvas) return;
    const behavior = smooth ? "smooth" : "auto";
    const box = el.getBoundingClientRect(), c = canvas.getBoundingClientRect();
    const ox = c.left - box.left + el.scrollLeft, oy = c.top - box.top + el.scrollTop;
    const { layout: l, org: o, zoom: z } = shape.current;
    // Where each card is on screen, the chart's zoom applied.
    const boxes = [...l.boxes.values()].map((b) => ({ ...b, x: b.x * z, y: b.y * z, w: b.w * z, h: b.h * z }));
    const W = el.clientWidth, H = el.clientHeight, maxX = el.scrollWidth - W, maxY = el.scrollHeight - H;
    const node = focus.current ? boxes.find((b) => b.id === focus.current) : undefined;
    const go = (left: number, top: number) => {
      const l = Math.round(Math.max(0, Math.min(maxX, left))), t = Math.round(Math.max(0, Math.min(maxY, top)));
      framed.current = { left: l, top: t };
      el.scrollTo({ left: l, top: t, behavior });
    };
    if (node && (maxX > 0 || maxY > 0)) {
      go(ox + node.x - (W - node.w) / 2, oy + node.y - (H - node.h) / 2);
      return;
    }
    let left = 0;
    if (maxX > 0) {
      // A whole card keeps a full gutter from the edge, clear of the edge's fade.
      const G = 40;
      const root = boxes.find((b) => b.depth === 0);
      const clamp = (x: number) => Math.round(Math.max(0, Math.min(maxX, x)));
      const cands = new Set<number>([0, maxX]);
      for (const b of boxes) { cands.add(clamp(ox + b.x - G)); cands.add(clamp(ox + b.x + b.w + G - W)); }
      let best = -Infinity;
      for (const s of cands) {
        let score = 0;
        for (const b of boxes) {
          const a = ox + b.x, z = a + b.w, w = weightOf(o, b.id);
          if (a >= s && z <= s + W) score += 10 + w * 100 + (b === root ? 150 : 0);
          else if (z > s && a < s + W) score -= 200 + w * 400;
        }
        if (root) score -= Math.abs(s + W / 2 - (ox + root.x + root.w / 2)) * 0.01;
        if (score > best) { best = score; left = s; }
      }
    }
    go(left, 0);
  }, []);
  useImperativeHandle(ref, () => ({ fit: () => { focus.current = null; reveal(true); } }), [reveal]);
  useLayoutEffect(() => { reveal(false); }, [reveal, layout, zoom, frame.w, frame.h]);

  // The edges that hide part of the chart fade, so a card the window cuts reads as "more this way", never as sliced;
  // and the page hears whether you have moved away from the opening framing.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const update = () => {
      const set = (k: string, on: boolean) => el.style.setProperty(k, on ? "32px" : "0px");
      set("--fl", el.scrollLeft > 1);
      set("--fr", el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
      set("--ft", el.scrollTop > 1);
      set("--fb", el.scrollTop + el.clientHeight < el.scrollHeight - 1);
      const f = framed.current;
      onMoved?.(!!f && (Math.abs(el.scrollLeft - f.left) > 2 || Math.abs(el.scrollTop - f.top) > 2));
    };
    update();
    el.addEventListener("scroll", update, { passive: true });
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => { el.removeEventListener("scroll", update); ro.disconnect(); };
  }, [layout, onMoved]);

  const card = (id: string) => {
    if (id === YOU_ID) return <YouCard company={company} compact={compact} onOpen={() => onOpen(YOU_ID)} />;
    const e = employees.find((x) => x.id === id);
    if (e) return <EmployeeCard e={e} employees={employees} people={people} compact={compact} onOpen={() => onOpen(e.id)} />;
    const p = people.find((x) => x.id === id);
    return p ? <PersonCard p={p} compact={compact} onOpen={() => onOpen(p.id)} /> : null;
  };

  return (
    <div className="orgscroll" ref={scroller} data-fold="canvas" data-tight={tight || undefined} data-zoomed={zoom < 1 || undefined}>
      <div className="orgcanvas-wrap">
        <div className="orgcanvas" style={{ width: layout.width, height: layout.height, zoom: zoom < 1 ? zoom : undefined }}>
          <svg className="org-edges" width={layout.width} height={layout.height} aria-hidden>
            {layout.edges.filter((e) => !lit.has(e.id)).map((e) => <path key={e.id} d={e.d} style={{ d: `path("${e.d}")` } as React.CSSProperties} />)}
            {/* A line carrying work is drawn last, over the shared trunk, in the colour of working. */}
            {layout.edges.filter((e) => lit.has(e.id)).map((e) => <path key={e.id} className="org-edge-live" d={e.d} style={{ d: `path("${e.d}")` } as React.CSSProperties} />)}
          </svg>
          {members.map(({ id }) => {
            const b = layout.boxes.get(id);
            if (!b) return null;
            return (
              <div key={id} className={`org-node ${fresh.has(id) ? "enter-drop" : ""}`} style={{ transform: `translate(${b.x}px, ${b.y}px)`, width: b.w, height: b.h }}>
                {card(id)}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
});

// ---------------------------------------------------------------- cards

/** A name with the surname in its own span, so the outline's first-name rows can let it go as a card folds. */
export function NameParts({ name }: { name: string }) {
  const cut = name.indexOf(" ");
  if (cut < 0) return <span>{name}</span>;
  return <><span>{name.slice(0, cut)}</span><span className="name-last">{name.slice(cut)}</span></>;
}

function StateLine({ s }: { s: Pick<MemberState, "tone" | "word"> }) {
  return (
    <span className={`mcard-line mcard-state mcard-state-${s.tone}`} data-part="state">
      {s.tone === "plain" ? null : <i className="mcard-dot" aria-hidden />}
      <span className="mcard-trunc">{s.word}</span>
    </span>
  );
}

export function YouCard({ company, compact = false, onOpen }: { company: Company; compact?: boolean; onOpen: () => void }) {
  return (
    <button type="button" className="mcard mcard-you" onClick={onOpen} data-member={YOU_ID} aria-label={`${company.ceoName}, ${company.title}, you. Open your page`}>
      <span className="mcard-av" data-part="av"><Avatar name={company.ceoName} hue={0} image={company.image} size={compact ? 28 : 32} you /></span>
      <span className="mcard-text">
        <span className="mcard-name" data-part="name"><NameParts name={company.ceoName} /></span>
        <span className="mcard-line mcard-role" data-part="role">{company.title}, you</span>
      </span>
    </button>
  );
}

function EmployeeCard({ e, employees, people, compact, onOpen }: { e: EmployeeView; employees: EmployeeView[]; people: PersonView[]; compact: boolean; onOpen: () => void }) {
  const s = employeeState(e, employees, people);
  const busy = s.tone !== "plain";
  // While they work, what on; otherwise their number, without its change (the band says that, in words).
  const glance = glanceOf(e, s);
  // Someone who needs you leads the chart: their card takes the coral surface, their state the name's weight.
  const needs = s.tone === "attn" || s.tone === "fail";
  return (
    <button type="button" className={`mcard ${needs ? "mcard-needs" : ""}`} onClick={onOpen} data-member={e.id} aria-label={`${e.name}, ${e.title}. ${s.word}${s.about ? `: ${s.about}` : glance ? `: ${glance}` : ""}`}>
      <span className="mcard-av" data-part="av"><Avatar name={e.name} hue={e.hue} image={e.image} size={compact ? 28 : 32} /></span>
      <span className="mcard-text">
        <span className="mcard-name" data-part="name"><NameParts name={e.name} /></span>
        {compact ? (
          busy ? <StateLine s={s} /> : <span className="mcard-line mcard-role" data-part="role">{e.title}</span>
        ) : (
          <>
            <span className="mcard-line mcard-role" data-part="role">{e.title}</span>
            <StateLine s={s} />
            {glance ? <span className="mcard-line mcard-glance" data-part="glance" title={s.about ?? undefined}>{glance}</span> : null}
          </>
        )}
      </span>
    </button>
  );
}

/** A real person: ringed, and "Person" where an employee has a state, unless an employee is waiting on them. */
export function personLine(p: PersonView): { waiting: boolean; text: string } {
  return p.openRequests > 0 ? { waiting: true, text: `${p.openRequests} waiting on them` } : { waiting: false, text: "Person" };
}

function PersonCard({ p, compact, onOpen }: { p: PersonView; compact: boolean; onOpen: () => void }) {
  const line = personLine(p);
  // Waiting on them is the person's only state: said in words, without a dot (amber beside a coral dot reads as one hue).
  const waitLine = (
    <span className="mcard-line mcard-state mcard-state-wait-on" data-part="state">
      <span className="mcard-trunc">{line.text}</span>
    </span>
  );
  return (
    <button type="button" className="mcard mcard-person" onClick={onOpen} data-member={p.id} aria-label={`${p.name}, ${p.title}. ${line.waiting ? line.text : "A person"}`}>
      <span className="mcard-av" data-part="av"><Avatar name={p.name} hue={p.hue} image={p.image} size={compact ? 28 : 32} person /></span>
      <span className="mcard-text">
        <span className="mcard-name" data-part="name"><NameParts name={p.name} /></span>
        {compact ? (
          line.waiting ? waitLine : <span className="mcard-line mcard-role" data-part="role">{p.title}</span>
        ) : (
          <>
            <span className="mcard-line mcard-role" data-part="role">{p.title}</span>
            {line.waiting ? waitLine : null}
          </>
        )}
      </span>
    </button>
  );
}
