import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { call } from "../api/client";
import type { EmployeeUsage, EmployeeView, PersonView, RoutineUsage, Run, RunUsage, SkillRevision } from "../api/types";
import { dayHeading, firstName, listNames, MOD, plural, runKindLabel, shortAge, timeOrDay } from "../lib/format";
import { refreshOrg, useNow, useOrg, useQuery, useRunDetail, type Query } from "../lib/hooks";
import { curve, ms, reducedMotion } from "../lib/motion";
import { navigate, openChat, openMember, openRun, type ActivityView } from "../lib/router";
import { useScrollFade } from "../lib/scrollFade";
import { blocked, isLive, threadFocus } from "../lib/threads";
import { workLabel } from "../lib/workTitle";
import { useShell } from "../Shell";
import { Age, RunRow } from "../components/RunRow";
import { RequestCard } from "../components/RequestCard";
import { AskRow } from "../components/ColleagueAsks";
import { groupByDay } from "../components/panel/WorkTab";
import { lineStats } from "../components/Diff";
import { toItems } from "../components/Transcript";
import { Icon } from "../components/Icon";
import { Avatar, PageHeader, Popover, Roll, Section, Skeleton } from "../components/ui";
import { EmptyPage, ErrorCard } from "./ActivityParts";
import "./Activity.css";

/** How far back the week's numbers look: a week, with a day's slack for a review that ran late. */
const WEEK_MS = 8 * 24 * 3600 * 1000;
/** Wide enough for the history beside a work column. */
const WIDE = 1040;

/** Whether an element is at least `px` wide, measured before paint and kept up to date. Pass the returned callback as
 *  the element's `ref`, so it is followed however it mounts. */
function useWide(px: number): [(el: HTMLElement | null) => void, boolean] {
  const [el, setEl] = useState<HTMLElement | null>(null);
  const [wide, setWide] = useState(false);
  useLayoutEffect(() => {
    if (!el) return;
    const update = () => setWide(el.clientWidth >= px);
    update();
    const sizes = new ResizeObserver(update);
    sizes.observe(el);
    return () => sizes.disconnect();
  }, [el, px]);
  return [setEl, wide];
}

/**
 * Everything that happened, in three views: the runs your employees did, the numbers (what they did and used today or
 * this week), and the changes they (and you) made to their skills. The runs open on what needs you. In a wide window
 * the live work and what is coming up get the work column beside them; in a narrow one the live work sits under what
 * needs you.
 */
export function ActivityScreen({ view }: { view: ActivityView }) {
  const { snap, error } = useOrg();
  const [who, setWho] = useState<string | null>(null);
  const employees = useMemo(() => snap?.employees ?? [], [snap]);
  const people = useMemo(() => snap?.people ?? [], [snap]);
  const chosen = employees.find((e) => e.id === who);
  const name = chosen ? firstName(chosen.name) : null;
  const runs = useQuery(() => call("runs", { employeeId: null, limit: 300 }), [], ["run:updated"]);
  const hired = !snap || employees.length > 0;
  const [page, wide] = useWide(WIDE);

  // Every view's data loads with the page, so switching views shows what is there at once, never a skeleton between.
  const numbersData = useNumbersData();
  const changes = useChanges(employees);
  const swapKey = `${view}~${who ?? ""}`;

  const side = useScrollFade<HTMLElement>();
  const main = useScrollFade<HTMLDivElement>();

  const go = (to: ActivityView) => navigate({ page: "activity", view: to });

  // The work column is drawn only when there is something to put in it; otherwise the page takes the whole width.
  const column = wide && hired && !(!snap && error);
  let body: ReactNode;
  if (!snap && error) body = <ErrorCard title="Couldn't load activity" text={error} onRetry={refreshOrg} />;
  else if (!snap) body = <PageSkeleton view={view} wide={wide} />;
  else if (!hired) body = <FirstUse view={view} />;
  else if (view === "runs") body = <Runs q={runs} who={who} name={name} employees={employees} people={people} wide={wide} />;
  else if (view === "numbers") body = <Numbers data={numbersData} who={who} employees={employees} />;
  else body = <SkillChanges q={changes} who={who} name={name} employees={employees} people={people} runs={runs.data} />;

  return (
    <div ref={page} className="activity" data-column={column}>
      <PageHeader title="Activity">
        {hired && (snap || !error) ? <WhoPicker who={who} employees={employees} onPick={setWho} loading={!snap} /> : null}
        <div className="seg" role="group" aria-label="Show">
          <button aria-pressed={view === "runs"} onClick={() => go("runs")}>Runs</button>
          <button aria-pressed={view === "numbers"} onClick={() => go("numbers")}>Numbers</button>
          <button aria-pressed={view === "skills"} onClick={() => go("skills")}>Skill changes</button>
        </div>
      </PageHeader>
      <div ref={main} className="act-main scroll-fade">
        <div className="act-main-inner"><Crossfade k={swapKey}>{body}</Crossfade></div>
      </div>
      {column ? (
        <aside ref={side} className="act-side scroll-fade" aria-label="Live work and what is coming up">
          {snap ? <><LiveWork who={who} employees={employees} people={people} column /><ComingUp who={who} employees={employees} people={people} /></> : <RailSkeleton />}
        </aside>
      ) : null}
    </div>
  );
}

/** Whose activity: a quiet control that fits its words, opening a menu of everyone. */
function WhoPicker({ who, employees, onPick, loading }: { who: string | null; employees: EmployeeView[]; onPick: (id: string | null) => void; loading: boolean }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const chosen = employees.find((e) => e.id === who);
  const pick = (id: string | null) => { setOpen(false); onPick(id); anchor.current?.focus(); };
  return (
    <>
      <button ref={anchor} type="button" className="btn btn-ghost btn-sm act-who" aria-haspopup="menu" aria-expanded={open} disabled={loading} onClick={() => setOpen((o) => !o)}>
        <Roll value={chosen ? chosen.name : "Everyone"} /> <Icon name="chevronDown" size={13} />
      </button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} width={220}>
        <div className="menu" role="menu" aria-label="Whose activity">
          <button type="button" role="menuitem" className="menu-item" aria-checked={who === null} onClick={() => pick(null)}>
            <span className="act-who-mark">{who === null ? <Icon name="check" size={13} /> : null}</span>Everyone
          </button>
          <div className="menu-sep" />
          {employees.map((e) => (
            <button key={e.id} type="button" role="menuitem" className="menu-item" aria-checked={who === e.id} onClick={() => pick(e.id)}>
              <span className="act-who-mark">{who === e.id ? <Icon name="check" size={13} /> : null}</span>
              <Avatar name={e.name} hue={e.hue} image={e.image} size={18} />{e.name}
            </button>
          ))}
        </div>
      </Popover>
    </>
  );
}

// ---------------------------------------------------------------- motion helpers

/** Keeps items that left a list mounted for `ms`, flagged, at the place they had, so a row closes its own space before it
 *  goes. Compares by key, so a fresh array of the same items changes nothing. */
function useLeaving<T>(items: T[], keyOf: (t: T) => string, ms: number): { item: T; leaving: boolean }[] {
  const keys = items.map(keyOf).join("|");
  const [state, setState] = useState<{ keys: string; items: T[]; leaving: { item: T; at: number }[] }>({ keys, items, leaving: [] });
  if (state.keys !== keys) {
    const now = new Set(items.map(keyOf));
    const gone = state.items.flatMap((item, at) => (now.has(keyOf(item)) ? [] : [{ item, at }]));
    setState({ keys, items, leaving: [...state.leaving.filter((l) => !now.has(keyOf(l.item))), ...gone] });
  }
  useEffect(() => {
    if (!state.leaving.length) return;
    const t = window.setTimeout(() => setState((s) => ({ ...s, leaving: [] })), ms);
    return () => window.clearTimeout(t);
  }, [state.leaving, ms]);
  const out = items.map((item) => ({ item, leaving: false }));
  for (const l of [...state.leaving].sort((a, b) => a.at - b.at)) out.splice(Math.min(l.at, out.length), 0, { item: l.item, leaving: true });
  return out;
}

/**
 * Rows that move rather than jump when the set of rows changes: like `useFlip`, but it measures where each row was just
 * before the change, so a row that closed its own space in between (and is gone now) never makes the others jump back.
 * A new row drops in from just above as its room opens. Under reduced motion nothing travels and new rows fade in.
 */
function useMoveRows<T extends HTMLElement>(keys: string) {
  const ref = useRef<T>(null);
  const last = useRef(keys);
  const before = useRef<Map<string, number> | null>(null);
  const rows = (host: HTMLElement) => [...host.children].filter((el): el is HTMLElement => el instanceof HTMLElement && !!el.dataset.key);
  // Read during render, while the DOM still shows the old rows: that is "where they were".
  if (last.current !== keys && ref.current && !before.current) {
    before.current = new Map(rows(ref.current).map((el) => [el.dataset.key!, el.getBoundingClientRect().top]));
  }
  useLayoutEffect(() => {
    if (last.current === keys) return;
    last.current = keys;
    const was = before.current;
    before.current = null;
    const host = ref.current;
    if (!was || !host) return;
    const reduced = reducedMotion();
    for (const el of rows(host)) {
      const top = was.get(el.dataset.key!);
      if (top === undefined) {
        el.animate(
          reduced ? [{ opacity: 0 }, { opacity: 1 }] : [{ opacity: 0, transform: "translateY(-8px)" }, { opacity: 1, transform: "none" }],
          reduced ? { duration: ms("--d-fade"), easing: "linear", fill: "backwards" }
            : { duration: ms("--d-arrive"), delay: ms("--d-arrive") * 0.5, easing: curve("--ease-arrive"), fill: "backwards" },
        );
        continue;
      }
      const dy = top - el.getBoundingClientRect().top;
      if (!reduced && Math.abs(dy) > 0.5) el.animate([{ transform: `translateY(${dy}px)` }, { transform: "none" }], { duration: ms("--d-arrive"), easing: curve("--ease-move") });
    }
  }, [keys]);
  return ref;
}

/**
 * A body that gives way to the next without a blank frame: when `k` changes, the old body stays in the same cell and
 * fades out while the new one fades in over it. On first show it is simply there.
 */
function Crossfade({ k, children }: { k: string; children: ReactNode }) {
  const last = useRef<ReactNode>(children);
  const [s, setS] = useState<{ k: string; was: ReactNode; wasK: string; moved: boolean }>({ k, was: null, wasK: "", moved: false });
  if (s.k !== k) setS({ k, was: last.current, wasK: s.k, moved: true });
  useLayoutEffect(() => { last.current = children; });
  useEffect(() => {
    if (s.was === null) return;
    const t = window.setTimeout(() => setS((x) => ({ ...x, was: null })), ms("--d-ui") + 60);
    return () => window.clearTimeout(t);
  }, [s.was]);
  return (
    <div className="act-xfade">
      {s.was !== null ? <div key={`was~${s.wasK}`} className="act-xfade-cell leave-fade" aria-hidden inert>{s.was}</div> : null}
      <div key={k} className={`act-xfade-cell ${s.moved ? "enter-fade" : ""}`}>{children}</div>
    </div>
  );
}

/** A text cut at a word, never through one (so "$29" is never shown as "$2"), with an ellipsis when it was cut. */
function clampWords(text: string, max: number): string {
  if (text.length <= max) return text;
  const cut = text.slice(0, max);
  return `${cut.slice(0, Math.max(cut.lastIndexOf(" "), max * 0.6)).replace(/[\s,;:.]+$/, "")}…`;
}

// ---------------------------------------------------------------- live work, coming up, and who the team waits on

/** The runs going now, oldest first: who, what, and what each is doing now (the sidebar already says who is working).
 *  Work waiting to start is under Coming up. A run that finishes closes its row here as it arrives in the history. */
function LiveWork({ who, employees, people, column = false }: { who: string | null; employees: EmployeeView[]; people: PersonView[]; column?: boolean }) {
  const live = employees
    .filter((e) => who === null || e.id === who)
    .flatMap((e) => e.live.filter((run) => run.state === "running").map((run) => ({ run, e })))
    .sort((a, b) => a.run.createdAt.localeCompare(b.run.createdAt));
  const rows = useLeaving(live, (x) => x.run.id, ms("--d-settle") + 40);
  const flip = useMoveRows<HTMLDivElement>(live.map((x) => x.run.id).join("|"));
  if (!column && !rows.length) return null;
  const name = who ? firstName(employees.find((e) => e.id === who)?.name ?? "") : null;
  return (
    <Section title="Live work" count={live.length} className="act-live">
      {rows.length ? (
        <div ref={flip} className="act-rail">
          {rows.map(({ item: { run, e }, leaving }) => (
            <div key={run.id} data-key={run.id} className="act-rail-item collapse" data-open={!leaving}>
              <div><LiveRow run={run} e={e} employees={employees} people={people} /></div>
            </div>
          ))}
        </div>
      ) : (
        <p className="act-quiet">{name ? `${name} isn't working on anything right now.` : "Nothing is running right now."}</p>
      )}
    </Section>
  );
}

const CODE_TOOLS = new Set(["Bash", "Read", "Edit", "Write", "Glob", "Grep", "NotebookEdit", "MultiEdit"]);

/** One run going on the rail: who, how long, the task, and the step it is on now. */
function LiveRow({ run, e, employees, people }: { run: Run; e: EmployeeView; employees: EmployeeView[]; people: PersonView[] }) {
  const detail = useRunDetail(run.id);
  const now = useNow(60_000);
  const items = useMemo(() => toItems(detail?.events ?? []), [detail?.events]);
  const tools = items.flatMap((i) => (i.kind === "tools" ? i.tools : []));
  const step = tools[tools.length - 1];
  return (
    <button className="act-rail-row" onClick={() => openRun(run.id)}>
      <Avatar name={e.name} hue={e.hue} image={e.image} size={28} />
      <b className="act-rail-who">{firstName(e.name)}</b>
      <span className="act-rail-meta act-rail-live"><Roll value={shortAge(run.startedAt ?? run.createdAt, now)} /></span>
      <span className="act-rail-what">{clampWords(workLabel(run, employees, people), 84)}</span>
      <span className="act-rail-now">
        <span className="act-rail-now-label">Now</span>
        {detail?.live && (!step || step.result) ? <span className="act-rail-now-what">writing</span>
          : step ? <span className={`act-rail-now-what ${CODE_TOOLS.has(step.tool) ? "mono" : ""}`}>{step.brief || step.tool}</span>
          : <span className="act-rail-now-what">starting</span>}
      </span>
    </button>
  );
}

/**
 * What comes next, on the same rail rows: first the work waiting to start ("Up next"), then each employee's next
 * scheduled run or check, soonest first, leaving out a routine already waiting to start. In a narrow window only the
 * work waiting to start is shown, under Live work.
 */
function ComingUp({ who, employees, people, waitingOnly = false }: { who: string | null; employees: EmployeeView[]; people: PersonView[]; waitingOnly?: boolean }) {
  const mine = employees.filter((e) => who === null || e.id === who);
  const queued = mine.flatMap((e) => e.live.filter((r) => r.state === "queued").map((run) => ({ run, e })))
    .sort((a, b) => a.run.createdAt.localeCompare(b.run.createdAt));
  const waiting = new Set(queued.map((q) => `${q.e.id}~${workLabel(q.run, employees, people)}`));
  const next = waitingOnly ? [] : mine
    .filter((e) => e.status.next && !waiting.has(`${e.id}~${e.status.next.text}`))
    .sort((a, b) => a.status.next!.at.localeCompare(b.status.next!.at))
    .slice(0, Math.max(0, 6 - queued.length));
  const flip = useMoveRows<HTMLDivElement>(queued.map((q) => q.run.id).join("|"));
  if (!queued.length && !next.length) return null;
  return (
    <Section title={waitingOnly ? "Up next" : "Coming up"} className="act-coming">
      <div ref={flip} className="act-rail">
        {queued.map(({ run, e }) => (
          <div key={run.id} data-key={run.id} className="act-rail-item">
            <button className="act-rail-row" onClick={() => openRun(run.id)}>
              <Avatar name={e.name} hue={e.hue} image={e.image} size={28} />
              <b className="act-rail-who">{firstName(e.name)}</b>
              <span className="act-rail-meta"><span className="status status-wait">Up next</span></span>
              <span className="act-rail-what">{clampWords(workLabel(run, employees, people), 84)}</span>
            </button>
          </div>
        ))}
        {next.map((e) => (
          <div key={e.id} className="act-rail-item">
            <button className="act-rail-row" onClick={() => openMember(e.id, "work")}>
              <Avatar name={e.name} hue={e.hue} image={e.image} size={28} />
              <b className="act-rail-who">{firstName(e.name)}</b>
              <span className="act-rail-meta">{timeOrDay(e.status.next!.at)}</span>
              <span className="act-rail-what">{clampWords(e.status.next!.text, 84)}</span>
            </button>
          </div>
        ))}
      </div>
    </Section>
  );
}

/** What the team is waiting on someone else for, folded to one line under what needs you: real people first (each
 *  answered or chased in place), then colleagues. */
function Waiting({ who, employees: team, people }: { who: string | null; employees: EmployeeView[]; people: PersonView[] }) {
  // The front desk is off the team but sends asks of its own: name it on the rows it started.
  const desk = useOrg().snap?.desk;
  const employees = useMemo(() => (desk ? [...team, desk] : team), [team, desk]);
  const requests = useQuery(() => call("requests", { personId: null, employeeId: who, state: "open" }), [who], ["request:updated", "org:changed"]);
  const asks = useQuery(() => call("open_asks"), [], ["run:updated"]);
  const [open, setOpen] = useState(false);
  const mine = (asks.data ?? []).filter((r) => who === null || r.employeeId === who || (r.kind.type === "asked" && r.kind.askedBy === who));
  const reqs = requests.data ?? [];
  if (!reqs.length && !mine.length) return null;
  const waitedOn = [
    ...reqs.flatMap((r) => { const p = people.find((x) => x.id === r.personId); return p ? [{ id: p.id, name: firstName(p.name), hue: p.hue, image: p.image, person: true }] : []; }),
    ...mine.flatMap((r) => { const e = employees.find((x) => x.id === r.employeeId); return e ? [{ id: e.id, name: firstName(e.name), hue: e.hue, image: e.image, person: false }] : []; }),
  ].filter((x, i, all) => all.findIndex((y) => y.id === x.id) === i);
  return (
    <section className="act-waiting" aria-label="Waiting on others">
      <button type="button" className="act-fold-head" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span className="act-fold-title">Waiting on others</span>
        <span className="act-fold-n num"><Roll value={reqs.length + mine.length} /></span>
        <span className="act-fold-faces" aria-hidden>{waitedOn.slice(0, 4).map((x) => <Avatar key={x.id} name={x.name} hue={x.hue} image={x.image} size={20} person={x.person} />)}</span>
        <span className="act-fold-sum">{listNames(waitedOn.map((x) => x.name))}</span>
        <Icon name="chevronDown" size={14} className="act-fold-chev" />
      </button>
      <div className="collapse" data-open={open} inert={!open}>
        <div>
          <div className="act-waiting-body">
            {reqs.length ? (
              <div className="plist">
                {reqs.map((r) => (
                  <RequestCard key={r.id} r={r} employee={employees.find((e) => e.id === r.employeeId)} person={people.find((p) => p.id === r.personId)} showPerson onChanged={() => requests.reload()} />
                ))}
              </div>
            ) : null}
            {mine.length ? <div className="plist">{mine.map((r) => <AskRow key={r.id} run={r} employees={employees} />)}</div> : null}
          </div>
        </div>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- runs

type Filter = "all" | "needsYou" | "failed" | "conversations" | "handoffs" | "routines";

const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  // Runs that asked you something (a decision, a sign-in, an OK). Not "Needs you": that is the section above, the
  // blocked and failed runs still waiting on you, and one word means one set on this page.
  { id: "needsYou", label: "Asked you" },
  { id: "failed", label: "Failed" },
  { id: "conversations", label: "Conversations" },
  { id: "handoffs", label: "Hand-offs" },
  { id: "routines", label: "Routines" },
];

/** A turn of a conversation with you. Mirrors Rust `RunKind::is_conversation`. */
const conversation = (r: Run): boolean =>
  r.kind.type === "task" || r.kind.type === "reply" || r.kind.type === "chat" || (r.kind.type === "answered" && (r.kind.chat || !!r.kind.thread));

function shows(r: Run, f: Filter): boolean {
  switch (f) {
    case "all": return true;
    case "needsYou": return r.needsYou.length > 0 || blocked(r);
    case "failed": return r.state === "failed" || r.state === "lost";
    case "conversations": return conversation(r);
    case "handoffs": return r.kind.type === "delegated" || r.kind.type === "asked" || (r.kind.type === "answered" && !conversation(r));
    case "routines": return r.kind.type === "scheduled";
  }
}

/**
 * The finished runs that still need the owner: blocked on something only you can allow, or failed and not yet put
 * right. A routine that failed several times running is one item (its newest run), and a routine whose next run
 * succeeded no longer needs you. Newest first.
 */
function needingYou(runs: Run[]): Run[] {
  const out: Run[] = [];
  const seen = new Set<string>();
  const routines = new Map<string, Run[]>();
  for (const r of runs) if (r.kind.type === "scheduled") routines.set(r.kind.scheduleId, [...(routines.get(r.kind.scheduleId) ?? []), r]);
  for (const r of runs) {
    if (r.archivedAt) continue;
    const failed = (r.state === "failed" || r.state === "lost") && !r.limited;
    if (!(blocked(r) && r.state === "succeeded") && !failed) continue;
    if (r.kind.type === "scheduled") {
      const id = r.kind.scheduleId;
      if (seen.has(id)) continue;
      const newer = (routines.get(id) ?? []).filter((x) => x.createdAt > r.createdAt && !isLive(x));
      if (newer.some((x) => x.state === "succeeded" && !x.denials.length)) continue;
      seen.add(id);
    }
    out.push(r);
  }
  return out;
}

function Runs({ q, who, name, employees, people, wide }: { q: Query<Run[]>; who: string | null; name: string | null; employees: EmployeeView[]; people: PersonView[]; wide: boolean }) {
  const [filter, setFilter] = useState<Filter>("all");
  const mine = useMemo(() => (q.data ?? []).filter((r) => who === null || r.employeeId === who), [q.data, who]);
  const needs = useMemo(() => needingYou(mine), [mine]);
  // Each run is listed once: what is going on now is under Live work, what needs you leads, the days hold the rest.
  const lead = useMemo(() => new Set(needs.map((r) => r.id)), [needs]);
  const past = useMemo(() => mine.filter((r) => !isLive(r) && !lead.has(r.id)), [mine, lead]);
  const runs = useMemo(() => past.filter((r) => shows(r, filter)), [past, filter]);
  const groups = useMemo(() => groupByDay(runs), [runs]);
  const handedOn = useMemo(() => new Set((q.data ?? []).flatMap((r) => (r.kind.type === "delegated" || r.kind.type === "asked" ? [r.kind.parentRunId] : []))), [q.data]);
  // With one person chosen, their name on every row says nothing: rows show the kind of run instead, as their Work tab does.
  const employee = (r: Run) => (who ? undefined : employees.find((e) => e.id === r.employeeId));
  if (q.error && !q.data) return <ErrorCard title="Couldn't load runs" text={q.error} onRetry={q.reload} />;
  if (!q.data) return <RunsSkeleton />;
  const nothing = !past.length && !needs.length;
  return (
    <>
      {needs.length ? (
        <Section title="Needs you" count={needs.length} className="act-needs">
          <NeedsList runs={needs} employee={employee} employees={employees} people={people} />
        </Section>
      ) : null}
      {wide ? null : <><LiveWork who={who} employees={employees} people={people} /><ComingUp who={who} employees={employees} people={people} waitingOnly /></>}
      <Waiting who={who} employees={employees} people={people} />
      {nothing ? (
        <EmptyPage title={name ? `Nothing from ${name} yet` : "Nothing has run yet"} text="Runs show up here the moment they start: on a schedule, when you press Run now, or when a colleague hands one over." />
      ) : (
        <section className="act-runs" aria-label="Runs">
          <div className="act-filters" role="group" aria-label="Show">
            {FILTERS.map((f) => (
              <button key={f.id} type="button" className="chip" aria-pressed={filter === f.id} onClick={() => setFilter(f.id)}>{f.label}</button>
            ))}
          </div>
          <Crossfade k={filter}>
            {runs.length === 0 ? (
              <div className="act-end">
                <p className="act-end-title">Nothing here in the latest runs</p>
                <p className="act-end-text">{filter === "failed" ? "Anything that still needs you is at the top. " : ""}To find older work, search with {MOD} K.</p>
              </div>
            ) : (
              <RunList groups={groups} handedOn={handedOn} employee={employee} employees={employees} people={people} filter={filter} />
            )}
          </Crossfade>
          {q.error ? <ErrorCard title="Couldn't refresh the runs" text={q.error} onRetry={q.reload} /> : null}
        </section>
      )}
    </>
  );
}

/** What needs you, in the run list's own rows; a row leaves (closing its space) once you have put it right. */
function NeedsList({ runs, employee, employees, people }: { runs: Run[]; employee: (r: Run) => EmployeeView | undefined; employees: EmployeeView[]; people: PersonView[] }) {
  const rows = useLeaving(runs, (r) => r.id, ms("--d-settle") + 40);
  const flip = useMoveRows<HTMLDivElement>(runs.map((r) => r.id).join("|"));
  return (
    <div ref={flip} className="act-list runlist">
      {rows.map(({ item: r, leaving }) => (
        <div key={r.id} data-key={r.id} className="act-row collapse" data-open={!leaving}>
          <div><RunRow run={r} employee={employee(r)} employees={employees} people={people} /></div>
        </div>
      ))}
    </div>
  );
}

/** A line of the run list: one run, or a routine's quiet runs of one day that did nothing. */
type Line = { run: Run } | { quiet: Run[] };

/** A day's runs as lines, newest first. A routine's quiet runs that succeeded and raised, handed on and changed nothing
 *  fold into one line where the newest of them is, once there are two. `handedOn`: runs that handed work to a colleague. */
function fold(runs: Run[], handedOn: Set<string>): Line[] {
  const idle = (r: Run) => r.kind.type === "scheduled" && r.kind.quiet && r.state === "succeeded"
    && !r.needsYou.length && !r.denials.length && !r.skillChanges.length && !handedOn.has(r.id);
  const routine = (r: Run) => (r.kind.type === "scheduled" && idle(r) ? r.kind.scheduleId : null);
  const idles = new Map<string, Run[]>();
  for (const r of runs) {
    const id = routine(r);
    if (id) idles.set(id, [...(idles.get(id) ?? []), r]);
  }
  const out: Line[] = [];
  for (const r of runs) {
    const id = routine(r);
    const same = id ? idles.get(id) ?? [] : [];
    if (same.length < 2) out.push({ run: r });
    else if (same[0] === r) out.push({ quiet: same });
  }
  return out;
}

/** The days, as one keyed list so a run that arrives drops in and the rows below make room. Inside a filter that names
 *  the state (Failed), the rows do not say it again. */
function RunList({ groups, handedOn, employee, employees, people, filter }: {
  groups: { day: string; runs: Run[] }[]; handedOn: Set<string>; employee: (r: Run) => EmployeeView | undefined; employees: EmployeeView[]; people: PersonView[]; filter: Filter;
}) {
  const lines = groups.flatMap((g) => [{ day: g.day } as const, ...fold(g.runs, handedOn)]);
  const keyOf = (l: (typeof lines)[number]) => ("day" in l ? `day~${l.day}` : "quiet" in l ? `quiet~${l.quiet[0].id}` : l.run.id);
  const flip = useMoveRows<HTMLDivElement>(lines.map(keyOf).join("|"));
  return (
    <div ref={flip} className="act-list runlist" data-filter={filter}>
      {lines.map((l) => {
        const key = keyOf(l);
        if ("day" in l) return <div key={key} data-key={key} className="act-day">{l.day}</div>;
        if ("quiet" in l) return <div key={key} data-key={key} className="act-row"><QuietRuns runs={l.quiet} employee={employee(l.quiet[0])} employees={employees} people={people} /></div>;
        return <div key={key} data-key={key} className="act-row"><RunRow run={l.run} employee={employee(l.run)} employees={employees} people={people} /></div>;
      })}
    </div>
  );
}

/** A routine's quiet runs of one day that did nothing, as one line that opens to list them. */
function QuietRuns({ runs, employee, employees, people }: { runs: Run[]; employee?: EmployeeView; employees: EmployeeView[]; people: PersonView[] }) {
  const [open, setOpen] = useState(false);
  const newest = runs[0];
  const at = newest.startedAt ?? newest.createdAt;
  const now = useNow(60_000);
  return (
    <>
      <button className="runrow act-quietruns" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        {employee ? <Avatar name={employee.name} hue={employee.hue} image={employee.image} size={28} className="runrow-lead" /> : <span className="runrow-lead runrow-icon"><Icon name="clock" size={14} /></span>}
        <span className="runrow-title">
          {employee ? <b>{firstName(employee.name)}</b> : null}
          <span className="runrow-label">{workLabel(newest, employees, people)}</span>
        </span>
        <span className="runrow-detail">{runs.length} quiet runs, nothing to report</span>
        <span className="runrow-status"><Icon name="chevronDown" size={14} className="act-fold-icon" /></span>
        <Age at={at} now={now} />
      </button>
      <div className="collapse act-quiet-runs" data-open={open} inert={!open}>
        <div>{runs.map((r) => <RunRow key={r.id} run={r} employee={employee} employees={employees} people={people} />)}</div>
      </div>
    </>
  );
}

// ---------------------------------------------------------------- numbers

/** Local midnight `daysBack` days ago, as the backend takes a time; it changes only when the day does. */
function dayStart(now: number, daysBack = 0): string {
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - daysBack);
  return d.toISOString();
}

/** Usage in US dollars at API list price, without its sign: "12.40", "<0.01", or "0" when no run recorded a cost. */
const dollars = (n: number) => (n <= 0 ? "0" : n < 0.01 ? "<0.01" : n.toFixed(2));

/** A weekly change in words: "+31" is "up 31", "-3" is "down 3". */
function changeWords(change: string): string {
  const c = change.trim();
  if (!c || /^[+±-]?0+(\.0+)?$/.test(c)) return "no change";
  if (c.startsWith("+")) return `up ${c.slice(1)}`;
  if (c.startsWith("-") || c.startsWith("−")) return `down ${c.slice(1)}`;
  return c;
}

/** What a routine's runs did, as a sentence: "1 of 5 runs did something". One run's count is beside it already. */
function routineWords(r: RoutineUsage): string {
  if (r.runs === 1) return r.useful ? "It did something" : "It did nothing";
  if (r.useful === r.runs) return `All ${r.runs} runs did something`;
  if (r.useful === 0) return `None of ${r.runs} runs did anything`;
  return `${r.useful} of ${r.runs} runs did something`;
}

type Span = "today" | "week";

type NumbersData = { starts: string[]; usage: Query<RunUsage[]>; reviews: Query<Run[]>; now: number };

/** What Numbers shows, loaded with the page: what was used since each of the last seven midnights (today's numbers are
 *  the first, the week's the last), and the weekly reviews. */
function useNumbersData(): NumbersData {
  const now = useNow(60_000);
  const today = dayStart(now);
  const starts = useMemo(() => Array.from({ length: 7 }, (_, i) => dayStart(Date.parse(today), i)), [today]);
  const usage = useQuery(() => Promise.all(starts.map((since) => call("run_usage", { since }))), [starts], ["run:updated"]);
  const reviews = useQuery(() => call("week_reviews"), [], ["run:updated"]);
  return { starts, usage, reviews, now };
}

const NO_USAGE = (employeeId: string): EmployeeUsage => ({ employeeId, runs: 0, decisions: 0, handoffs: 0, failed: 0, costUsd: 0 });

/**
 * Each employee's numbers on one grid that keeps its shape between Today and This week: the same people in the same
 * rows (anyone who ran this week), their values rolling between the two, and each person's routines opening under them
 * for the week and closing for today. What it all used is the one big number, with the last seven days under it. The
 * table ends on a line that says what it covered.
 */
function Numbers({ data, who, employees }: { data: NumbersData; who: string | null; employees: EmployeeView[] }) {
  const { starts, usage, reviews, now } = data;
  const [span, setSpanNow] = useState<Span>("today");
  // The notes change with the span; they fade into place once it has changed, never on the first paint.
  const [moved, setMoved] = useState(false);
  const setSpan = (s: Span) => { if (s !== span) { setSpanNow(s); setMoved(true); } };
  const [closed, setClosed] = useState<Set<string>>(() => new Set());
  const week = usage.data?.[6];
  const shown = usage.data ? (span === "today" ? usage.data[0] : usage.data[6]) : undefined;

  const latest = new Map<string, Run>();
  for (const r of reviews.data ?? []) {
    if (r.kind.type !== "scheduled" || !r.kind.review || r.state !== "succeeded" || now - Date.parse(r.createdAt) > WEEK_MS) continue;
    const had = latest.get(r.employeeId);
    if (!had || had.createdAt < r.createdAt) latest.set(r.employeeId, r);
  }
  const rows = employees.filter((e) => (who === null || e.id === who) && week?.employees.some((y) => y.employeeId === e.id)).map((e) => {
    const x = shown?.employees.find((y) => y.employeeId === e.id) ?? NO_USAGE(e.id);
    const routines = (week?.routines ?? []).filter((r) => r.employeeId === e.id).sort((a, b) => b.costUsd - a.costUsd);
    return { e, x, routines, review: latest.get(e.id) };
  });
  const total = rows.reduce((s, r) => s + r.x.costUsd, 0);
  const runsTotal = rows.reduce((s, r) => s + r.x.runs, 0);
  const people = rows.filter((r) => r.x.runs > 0).length;
  // What each day used, from the cumulative totals: a day is what its midnight saw less what the next one did.
  const spent = (d: number) => (usage.data?.[d]?.employees ?? []).filter((x) => who === null || x.employeeId === who).reduce((s, x) => s + x.costUsd, 0);
  const days = usage.data ? starts.map((at, i) => ({ at, cost: Math.max(0, spent(i) - (i ? spent(i - 1) : 0)) })).reverse() : null;
  const toggle = (id: string) => setClosed((s) => { const n = new Set(s); if (n.has(id)) n.delete(id); else n.add(id); return n; });

  let list: ReactNode;
  if (usage.error && !usage.data) list = <ErrorCard title="Couldn't load the numbers" text={usage.error} onRetry={usage.reload} />;
  else if (!usage.data) list = <div className="num-list" aria-hidden>{[64, 48, 72, 56].map((w, i) => <div key={i} className="num-skel"><Skeleton h={20} w={20} r={10} /><Skeleton h={12} w={`${w}%`} /><Skeleton h={12} w={44} /></div>)}</div>;
  else if (!rows.length) list = <EmptyPage title="Nothing has run this week" text="Each employee's runs and what they used show up here once they work." />;
  else {
    list = (
      <>
        <div className="num-list">
          {rows.map(({ e, x, routines, review }) => {
            const open = span === "week" && routines.length > 0 && !closed.has(e.id);
            return (
              <div key={e.id} className="num-group" data-open={open}>
                <div className="num-row">
                  <button className="num-main" onClick={() => (span === "week" && review ? openChat(e.id, threadFocus(review.id)) : openMember(e.id, "work"))}>
                    <Avatar name={e.name} hue={e.hue} image={e.image} size={20} />
                    <b className="num-who">{firstName(e.name)}</b>
                    <span className="num-what"><span key={span} className={`num-what-in ${moved ? "enter-fade" : ""}`}>{span === "today" ? <TodayWhat u={x} /> : <WeekWhat review={review} />}</span></span>
                    <span className="num-runs num"><Roll value={x.runs} /> {x.runs === 1 ? "run" : "runs"}</span>
                    <span className="num-cost num">$<Roll value={dollars(x.costUsd)} /></span>
                  </button>
                  {/* The chevron's slot is held in both spans, so a row is one height whichever is shown. */}
                  <span className="num-fold-slot">
                    {span === "week" && routines.length ? (
                      <button className="num-fold" aria-expanded={open} aria-label={`${firstName(e.name)}'s routines`} data-tip={open ? "Hide routines" : `Show ${plural(routines.length, "routine")}`} onClick={() => toggle(e.id)}>
                        <Icon name="chevronDown" size={14} />
                      </button>
                    ) : null}
                  </span>
                </div>
                {routines.length ? (
                  <div className="collapse" data-open={open} inert={!open}>
                    <div>
                      <div className="num-subs">
                        {routines.map((r) => (
                          <button key={r.scheduleId} className="num-sub" onClick={() => openMember(e.id, "work")}>
                            <span className="num-sub-name">{r.label}</span>
                            <span className="num-sub-what" data-tip="A run did something when it raised a card, handed work to a colleague, changed a skill or failed.">{routineWords(r)}</span>
                            <span className="num-runs num">{plural(r.runs, "run")}</span>
                            <span className="num-cost num">${dollars(r.costUsd)}</span>
                          </button>
                        ))}
                      </div>
                    </div>
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
        <p className="act-foot">
          <Roll value={people === 0 ? `Nobody has run anything ${span === "today" ? "today" : "this week"} yet.` : `${plural(people, "person", "people")} ran ${plural(runsTotal, "run")} ${span === "today" ? "today" : "this week"}.`} />
        </p>
      </>
    );
  }

  return (
    <section className="numbers" aria-label="Numbers">
      <div className="num-head">
        <span className="num-total-value num">{usage.data ? <>$<Roll value={dollars(total)} /></> : <Skeleton h={30} w={120} />}</span>
        <SpanPicker span={span} onPick={setSpan} />
        <span className="num-total-label"><Roll value={span === "today" ? "Used today, at API list price" : "Used this week, at API list price"} /></span>
        <DaySpend days={days} span={span} />
      </div>
      {list}
    </section>
  );
}

/** Today or This week: a quiet control that fits its words, as the Inbox's own view control is, so it never reads as a
 *  second set of tabs under the page's. */
function SpanPicker({ span, onPick }: { span: Span; onPick: (s: Span) => void }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const pick = (s: Span) => { setOpen(false); onPick(s); anchor.current?.focus(); };
  const label = (s: Span) => (s === "today" ? "Today" : "This week");
  return (
    <span className="num-span">
      <button ref={anchor} type="button" className="btn btn-ghost btn-sm act-who" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <Roll value={label(span)} /> <Icon name="chevronDown" size={13} />
      </button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} width={180}>
        <div className="menu" role="menu" aria-label="Which numbers">
          {(["today", "week"] as Span[]).map((s) => (
            <button key={s} type="button" role="menuitem" className="menu-item" aria-checked={span === s} onClick={() => pick(s)}>
              <span className="act-who-mark">{span === s ? <Icon name="check" size={13} /> : null}</span>{label(s)}
            </button>
          ))}
        </div>
      </Popover>
    </span>
  );
}

/** The last seven days' spend across the column: seven thin bars on one baseline, today last (labelled with its figure
 *  when the big number above is the week's). The days the figure above covers are in ink; the rest are quiet. Each bar says its day and amount on hover. */
function DaySpend({ days, span }: { days: { at: string; cost: number }[] | null; span: Span }) {
  const max = Math.max(0.01, ...(days ?? []).map((d) => d.cost));
  return (
    <div className="num-days" aria-label="What each of the last seven days used">
      {(days ?? Array.from({ length: 7 }, (_, i) => ({ at: String(i), cost: 0 }))).map((d, i) => {
        const day = days ? new Date(d.at).toLocaleDateString(undefined, { weekday: "short" }) : "";
        const on = span === "week" || i === 6;
        return (
          <div key={d.at} className="num-day" data-on={on} data-tip={days ? `${day}: $${dollars(d.cost)}` : undefined} aria-label={days ? `${day}, $${dollars(d.cost)}` : undefined}>
            {/* On Today the big figure already says it; for the week, today's bar names its own. */}
            <span className="num-day-figure num">{i === 6 && days && span === "week" ? <>$<Roll value={dollars(d.cost)} /></> : null}</span>
            <span className="num-bar-track"><span className="num-bar" style={{ "--v": `${days ? Math.max(4, (d.cost / max) * 100) : 0}%` } as React.CSSProperties} /></span>
            <span className="num-day-label">{i === 6 && days ? "Today" : day}</span>
          </div>
        );
      })}
    </div>
  );
}

function TodayWhat({ u }: { u: EmployeeUsage }) {
  const parts = [u.decisions ? plural(u.decisions, "decision") : null, u.handoffs ? plural(u.handoffs, "hand-off") : null].filter(Boolean);
  if (!parts.length && !u.failed) return <span className="muted">{u.runs ? "Nothing to note" : "Nothing today"}</span>;
  return (
    <>
      {parts.join(" · ")}
      {u.failed ? <span className="num-failed">{parts.length ? " · " : ""}{u.failed} failed</span> : null}
    </>
  );
}

function WeekWhat({ review }: { review: Run | undefined }) {
  const s = review?.score;
  if (!s) return <span className="muted">{review ? "Reviewed the week" : "No weekly number yet"}</span>;
  return (
    <>
      {s.metric} <b className="num-value">{s.value}</b><span className="muted">, {changeWords(s.change)}</span>
    </>
  );
}

// ---------------------------------------------------------------- skill changes

type Change = { r: SkillRevision; e: EmployeeView };
type Changes = Query<{ changes: Change[]; failed: string | null }>;

/** Every employee's skill history, newest first, reloaded when the office changes. It fails only when nobody's could be
 *  read; a partial history shows what loaded, with the failure under it. */
function useChanges(employees: EmployeeView[]): Changes {
  const load = async () => {
    const tries = await Promise.allSettled(employees.map(async (e) => (await call("skill_history", { employeeId: e.id })).map((r) => ({ r, e }))));
    const ok = tries.flatMap((t) => (t.status === "fulfilled" ? [t.value] : []));
    const bad = tries.find((t): t is PromiseRejectedResult => t.status === "rejected");
    if (bad && !ok.length) throw bad.reason;
    return { changes: ok.flat().sort((a, b) => b.r.at.localeCompare(a.r.at)), failed: bad ? String(bad.reason?.message ?? bad.reason) : null };
  };
  return useQuery(load, [employees.map((e) => e.id)], ["org:changed"]);
}

function SkillChanges({ q, who, name, employees, people, runs }: { q: Changes; who: string | null; name: string | null; employees: EmployeeView[]; people: PersonView[]; runs: Run[] | undefined }) {
  const changes = useMemo(() => (q.data?.changes ?? []).filter((c) => who === null || c.e.id === who), [q.data, who]);
  const groups = useMemo(() => {
    const out: { day: string; items: Change[] }[] = [];
    for (const c of changes) {
      const day = dayHeading(c.r.at);
      const last = out[out.length - 1];
      if (last && last.day === day) last.items.push(c);
      else out.push({ day, items: [c] });
    }
    return out;
  }, [changes]);
  const byId = useMemo(() => new Map((runs ?? []).map((r) => [r.id, r])), [runs]);

  if (q.error && !q.data) return <ErrorCard title="Couldn't load skill changes" text={q.error} onRetry={q.reload} />;
  if (!q.data) return <ListSkeleton />;
  if (changes.length === 0) {
    return <EmptyPage title={name ? `${name} hasn't changed a skill yet` : "No skill changes yet"} text="When you teach an employee something, or they learn from a run, the change to their skill shows up here. Any change can be undone." />;
  }
  return (
    <div className="act-runs">
      <div className="act-list runlist">
        {groups.map((g) => [
          <div key={`day~${g.day}`} className="act-day">{g.day}</div>,
          ...g.items.map(({ r, e }) => <div key={r.id} className="act-row"><ChangeRow r={r} e={e} run={r.runId ? byId.get(r.runId) : undefined} employees={employees} people={people} /></div>),
        ])}
      </div>
      {q.data.failed ? <ErrorCard title="Some changes couldn't be loaded" text={q.data.failed} onRetry={q.reload} /> : null}
      <p className="act-foot">{changes.length === 1 ? "That's the only skill change so far." : `That's all ${changes.length} skill changes so far.`} Open one to see it, or to undo it.</p>
    </div>
  );
}

/** Where a change came from: the run's own name, else how it was made. */
function changeSource(r: SkillRevision, run: Run | undefined, employees: EmployeeView[], people: PersonView[]): string {
  if (r.runId) return run ? (run.kind.type === "chat" ? "Learned in a conversation" : run.kind.type === "manual" || run.kind.type === "delegated" ? workLabel(run, employees, people) : runKindLabel(run.kind)) : "During a run";
  return r.change === "added" ? "Set up" : "Edited by you";
}

function ChangeRow({ r, e, run, employees, people }: { r: SkillRevision; e: EmployeeView; run: Run | undefined; employees: EmployeeView[]; people: PersonView[] }) {
  const s = lineStats(r.before, r.after);
  const verb = r.change === "added" ? "created" : r.change === "removed" ? "removed" : "changed";
  const now = useNow(60_000);
  return (
    <button className="runrow act-change" onClick={() => openMember(e.id, "skills", `rev~${r.id}`)}>
      <Avatar name={e.name} hue={e.hue} image={e.image} size={28} className="runrow-lead" />
      <span className="runrow-title">
        <b>{firstName(e.name)}</b>
        <span className="runrow-label act-change-what">{verb} <span className="mono act-change-skill">{r.skill}{r.file !== "SKILL.md" ? `/${r.file}` : ""}</span></span>
      </span>
      <span className="runrow-detail">{changeSource(r, run, employees, people)}</span>
      <span className="runrow-status act-change-lines mono num" aria-label={`${s.added} lines added, ${s.removed} removed`}>
        {s.added ? <span>+{s.added}</span> : null}{s.removed ? <span>−{s.removed}</span> : null}
      </span>
      <Age at={r.at} now={now} />
    </button>
  );
}

// ---------------------------------------------------------------- loading and first use

/** Rows at the list's own geometry: a day heading, then runs of varied length. It holds still. */
function ListSkeleton() {
  const widths = [[46, 72], [38, 60], [52, 80], [34, 54], [44, 66]];
  return (
    <div className="act-list" aria-hidden>
      <div className="act-day"><Skeleton h={10} w={44} /></div>
      {widths.map(([a, b], i) => <SkelRow key={i} a={a} b={b} />)}
    </div>
  );
}

function SkelRow({ a, b }: { a: number; b: number }) {
  return (
    <div className="act-row act-skel">
      <Skeleton h={28} w={28} r={14} />
      <span className="act-skel-lines"><Skeleton h={11} w={`${a}%`} /><Skeleton h={9} w={`${b}%`} /></span>
      <Skeleton h={9} w={24} />
    </div>
  );
}

/** The runs as they load, in the order the page fills: what needs you, the folded line of who the team waits on, the
 *  filters, then the days. */
function RunsSkeleton() {
  return (
    <div className="act-skeleton" aria-busy aria-label="Loading">
      <div className="psec">
        <div className="psec-head"><Skeleton h={12} w={84} /></div>
        <div className="act-list"><SkelRow a={52} b={74} /><SkelRow a={40} b={66} /></div>
      </div>
      <div className="act-fold-head act-skel-fold"><Skeleton h={12} w={140} /><Skeleton h={20} w={60} r={10} /></div>
      <section className="act-runs">
        <div className="act-filters">{[34, 76, 54, 104, 80, 74].map((w, i) => <Skeleton key={i} h={26} w={w} r={13} />)}</div>
        <ListSkeleton />
      </section>
    </div>
  );
}

/** The whole page while the office loads, shaped like the view that will replace it. */
function PageSkeleton({ view, wide }: { view: ActivityView; wide: boolean }) {
  if (view === "runs") return <>{wide ? null : <div className="psec"><div className="psec-head"><Skeleton h={12} w={72} /></div><div className="act-rail">{[0, 1].map((i) => <RailSkelRow key={i} />)}</div></div>}<RunsSkeleton /></>;
  return <ListSkeleton />;
}

function RailSkelRow() {
  return (
    <div className="act-rail-item act-rail-skel">
      <Skeleton h={28} w={28} r={14} />
      <span className="act-skel-lines"><Skeleton h={11} w="40%" /><Skeleton h={9} w="72%" /></span>
    </div>
  );
}

/** The work column while the office loads: its two headings and rows at their own geometry. */
function RailSkeleton() {
  return (
    <>
      <div className="psec act-live"><div className="psec-head"><Skeleton h={12} w={72} /></div><div className="act-rail">{[0, 1, 2].map((i) => <RailSkelRow key={i} />)}</div></div>
      <div className="psec act-coming"><div className="psec-head"><Skeleton h={12} w={80} /></div><div className="act-rail">{[0, 1, 2].map((i) => <RailSkelRow key={i} />)}</div></div>
    </>
  );
}

/** Nobody hired yet: the message as page type in the upper third, with its one action. */
function FirstUse({ view }: { view: ActivityView }) {
  const shell = useShell();
  return (
    <EmptyPage
      title={view === "skills" ? "No skill changes yet" : view === "numbers" ? "No numbers yet" : "Nothing has run yet"}
      text={view === "skills"
        ? "Once you hire someone, every change to their skills shows up here, and any of them can be undone."
        : view === "numbers" ? "Once you hire someone, what each of them does and uses shows up here, today and over the week."
        : "Once you hire someone, their runs show up here the moment they start: on a schedule, when you press Run now, or when a colleague hands one over."}
      action={<button className="btn btn-primary" onClick={(ev) => shell.hire(null, ev.currentTarget)}><Icon name="plus" size={14} strokeWidth={1.8} /> Hire an employee</button>}
    />
  );
}
