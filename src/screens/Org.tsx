import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Company, EmployeeView, OrgSnapshot, PersonView } from "../api/types";
import { useShell } from "../Shell";
import { OrgChart, YouCard, type OrgChartHandle } from "../components/OrgChart";
import { OrgOutline } from "../components/OrgChartOutline";
import { startFold, type Fold, type FoldTarget } from "../components/OrgChartFold";
import { EmployeePanel } from "../components/panel/EmployeePanel";
import { PersonPanel } from "../components/panel/PersonPanel";
import { YouPanel } from "../components/panel/YouPanel";
import { TEMPLATES } from "../components/HireSheet";
import { Icon } from "../components/Icon";
import { ErrorNote, Skeleton } from "../components/ui";
import { headDragRegion } from "../lib/chrome";
import { YOU_ID, type OrgView } from "../lib/org";
import { refreshOrg, useOrg } from "../lib/hooks";
import { navigate, openMember, type PanelTab, type Route } from "../lib/router";
import "./Org.css";

type Member = { kind: "employee"; e: EmployeeView } | { kind: "person"; p: PersonView } | { kind: "you" };
const memberOf = (snap: OrgSnapshot | null, id: string | null): Member | null => {
  if (!snap || !id) return null;
  if (id === YOU_ID) return { kind: "you" };
  const e = snap.employees.find((x) => x.id === id);
  if (e) return { kind: "employee", e };
  const p = snap.people.find((x) => x.id === id);
  return p ? { kind: "person", p } : null;
};
const VIEWS: { id: OrgView; label: string }[] = [{ id: "everyone", label: "Everyone" }, { id: "employees", label: "Employees" }, { id: "people", label: "People" }];
/** Below this width the chart's cards drop to two lines and the outline to first names. */
const COMPACT_W = 1000;
/** Below this height the person's header is one line. */
const SHORT_H = 700;

/**
 * The org chart, and a person opened from it. With nobody open the chart is the page. Opening someone folds the
 * chart into an outline on the left and their page takes the rest; closing unfolds it. Both views are rendered while
 * the fold runs (`OrgChartFold`), and only the landed one after.
 */
export function OrgScreen({ route }: { route: Extract<Route, { page: "org" }> }) {
  const { snap, error } = useOrg();
  const shell = useShell();
  const root = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<{ w: number; h: number } | null>(null);
  useLayoutEffect(() => {
    const el = root.current;
    if (!el) return;
    const measure = () => setSize((s) => (s && s.w === el.clientWidth && s.h === el.clientHeight ? s : { w: el.clientWidth, h: el.clientHeight }));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const compact = (size?.w ?? 1400) < COMPACT_W;
  const short = (size?.h ?? 900) < SHORT_H;
  const [view, setView] = useState<OrgView>("everyone");
  // Whether you have moved the chart away from the framing it opened on: then Recentre brings that framing back.
  const [moved, setMoved] = useState(false);
  const chart = useRef<OrgChartHandle>(null);

  // Who is open, and where the fold stands. `settled` is the view on screen when nothing moves.
  const member = memberOf(snap, route.memberId);
  const want: FoldTarget = member ? "open" : "chart";
  const [settled, setSettled] = useState<FoldTarget>(want);
  const [folding, setFolding] = useState(false);
  const fold = useRef<Fold | null>(null);
  const landedFold = useRef<Fold | null>(null);
  // While the page folds away it still shows who it showed.
  const shownMember = useRef<Member | null>(member);
  if (member) shownMember.current = member;
  const openId = useRef<string | null>(route.memberId);
  const openAt = useRef({ tab: route.tab, focus: route.focus });
  if (member && route.memberId) {
    openId.current = route.memberId;
    openAt.current = { tab: route.tab, focus: route.focus };
  }
  const both = !!snap && (folding || settled !== want);

  useLayoutEffect(() => {
    if (!snap || !root.current) return;
    if (fold.current) { fold.current.toward(want); return; }
    if (settled === want) return;
    const f = startFold(root.current, want, openId.current);
    fold.current = f;
    setFolding(true);
    f.onLand((t) => {
      fold.current = null;
      landedFold.current = f;
      setSettled(t);
      setFolding(false);
    });
  }, [snap, want, settled]);
  // Once the landed view has rendered alone, the fold's copies and holds go.
  useLayoutEffect(() => {
    if (folding || !landedFold.current) return;
    landedFold.current.cleanup();
    landedFold.current = null;
    // Back on the chart, focus returns to the card of the person who was open.
    if (settled === "chart" && openId.current) root.current?.querySelector<HTMLElement>(`[data-member="${openId.current}"]`)?.focus({ preventScroll: true });
  }, [folding, settled]);
  useEffect(() => () => { fold.current?.cleanup(); landedFold.current?.cleanup(); }, []);

  const close = useCallback(() => navigate({ page: "org", memberId: null, tab: null, focus: null }), []);

  // Esc closes the page, unless something on top of it (a sheet, a menu, the palette) takes it first.
  useEffect(() => {
    if (want !== "open") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (document.querySelector('[aria-modal="true"], .popover')) return;
      if ((e.target as HTMLElement | null)?.closest?.("[data-esc-stop], [role='dialog']")) return;
      e.preventDefault();
      close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [want, close]);

  const cls = `org ${compact ? "org-compact" : ""}`;
  if (!snap) {
    return (
      <div className={cls} ref={root}>
        <section className="org-chart" data-view="chart" aria-label="Org chart" aria-busy={!error}>
          <OrgHead tool={null} ruled />
          {error ? (
            <div className="org-state">
              <ErrorNote title="Couldn't open the org chart" text={error} onRetry={refreshOrg} />
            </div>
          ) : <ChartSkeleton />}
        </section>
      </div>
    );
  }

  const empty = snap.employees.length === 0 && snap.people.length === 0;
  const hasPeople = snap.people.length > 0;
  const showChart = settled === "chart" || both;
  const showOpen = settled === "open" || both;
  const shown = member ?? shownMember.current;
  const hire = (from: HTMLElement) => shell.hire(null, from);
  const addPerson = (from: HTMLElement) => shell.addPerson(from);

  return (
    <div className={cls} ref={root} data-folding={both || undefined}>
      {showChart ? (
        <section className="org-chart" data-view="chart" aria-label="Org chart" aria-hidden={both && want === "open" ? true : undefined}>
          <OrgHead
            ruled={empty || !hasPeople}
            tool={!empty && moved ? <ChartFit onFit={() => chart.current?.fit()} /> : null}
          />
          {!empty && hasPeople ? <Views view={view} setView={setView} /> : null}
          <div className="org-canvas">
            {empty ? (
              <EmptyOrg company={snap.company} onHire={(t, from) => shell.hire(t, from)} onAddPerson={addPerson} />
            ) : (
              <OrgChart
                ref={chart} company={snap.company} employees={snap.employees} people={snap.people} view={view} compact={compact}
                frame={size ?? { w: 1200, h: 800 }} focusId={openId.current} onOpen={openMember} onMoved={setMoved}
              />
            )}
          </div>
          {empty ? null : <OrgFoot icons={false} onHire={hire} onAddPerson={addPerson} />}
        </section>
      ) : null}
      {showOpen && shown ? (
        <>
          <nav className="org-outline" data-view="outline" aria-label="Org chart">
            <OrgHead
              folded ruled={compact || !hasPeople}
              tool={<button className="btn btn-ghost btn-icon" data-pair="tool" aria-label="Show the whole chart" data-tip="Show the whole chart" onClick={close}><Icon name="expand" size={15} /></button>}
            />
            {!compact && hasPeople ? <Views view={view} setView={setView} /> : null}
            <OrgOutline
              company={snap.company} employees={snap.employees} people={snap.people} view={view} compact={compact}
              selectedId={route.memberId ?? openId.current} onOpen={(id) => openMember(id, id === route.memberId ? route.tab : null)}
            />
            <OrgFoot icons={compact} onHire={hire} onAddPerson={addPerson} />
          </nav>
          <div className="org-page" data-view="page">
            <MemberPage m={shown} tab={openAt.current.tab} focus={openAt.current.focus} short={short} onClose={close} />
          </div>
        </>
      ) : null}
      <div className="org-fold-layer" data-fold="layer" aria-hidden />
    </div>
  );
}

/** The person's page: an employee's, a real person's, or yours. A new person's page rises in place of the last. */
function MemberPage({ m, tab, focus, short, onClose }: { m: Member; tab: PanelTab | null; focus: string | null; short: boolean; onClose: () => void }) {
  const id = m.kind === "employee" ? m.e.id : m.kind === "person" ? m.p.id : YOU_ID;
  // The page the fold brought in arrives with the fold; every page after it rises in place of the last.
  const first = useRef<string | null>(id);
  if (first.current !== null && first.current !== id) first.current = null;
  return (
    <div key={id} className={`org-page-in ${first.current === null ? "enter-rise" : ""}`}>
      {m.kind === "employee" ? <EmployeePanel employee={m.e} tab={tab} focus={focus} short={short} onClose={onClose} /> : null}
      {m.kind === "person" ? <PersonPanel person={m.p} short={short} onClose={onClose} /> : null}
      {m.kind === "you" ? <YouPanel short={short} onClose={onClose} /> : null}
    </div>
  );
}

// ---------------------------------------------------------------- head, filter, foot (the same in both views)

/** The chart's head. Folded beside a person's page it is a sidebar heading, one step down, so the person leads. */
function OrgHead({ tool, ruled = false, folded = false }: { tool: React.ReactNode; ruled?: boolean; folded?: boolean }) {
  return (
    <header className={`pagehead org-head ${ruled ? "org-head-ruled" : ""} ${folded ? "org-head-folded" : ""}`} {...headDragRegion}>
      <h1 className="pagehead-title" data-pair="title">Org chart</h1>
      {tool}
    </header>
  );
}

function Views({ view, setView }: { view: OrgView; setView: (v: OrgView) => void }) {
  return (
    <div className="org-views" data-pair="views">
      <div className="seg" role="group" aria-label="Show">
        {VIEWS.map((v) => <button key={v.id} aria-pressed={view === v.id} onClick={() => setView(v.id)}>{v.label}</button>)}
      </div>
    </div>
  );
}

function OrgFoot({ icons, onHire, onAddPerson }: { icons: boolean; onHire: (from: HTMLElement) => void; onAddPerson: (from: HTMLElement) => void }) {
  return (
    <footer className="org-foot">
      <div className="pagehead-actions org-foot-actions" data-pair="foot">
        <button className={`btn btn-primary btn-sm ${icons ? "btn-icon" : ""}`} onClick={(e) => onHire(e.currentTarget)} aria-label={icons ? "Hire" : undefined} data-tip={icons ? "Hire" : undefined}>
          <Icon name="plus" size={14} strokeWidth={1.8} />{icons ? null : "Hire"}
        </button>
        <button className={`btn btn-ghost btn-sm ${icons ? "btn-icon" : ""}`} onClick={(e) => onAddPerson(e.currentTarget)} aria-label={icons ? "Add person" : undefined} data-tip={icons ? "Add person" : undefined}>
          <Icon name="userPlus" size={14} />{icons ? null : "Add person"}
        </button>
      </div>
    </footer>
  );
}

// ---------------------------------------------------------------- the chart's canvas

/** Recentre: shows only once you have moved the view, and brings back the framing it opened on. */
function ChartFit({ onFit }: { onFit: () => void }) {
  return (
    <button className="btn btn-ghost btn-sm" data-pair="tool" data-tip="Back to the framing the chart opened on" onClick={onFit}>
      <Icon name="fit" size={14} /> Recentre
    </button>
  );
}

/** Loading: the chart's own shape, held still, under the real head. */
function ChartSkeleton() {
  const card = (k: string) => (
    <div key={k} className="org-skel-card">
      <Skeleton h={32} w={32} r={16} />
      <span className="stack grow" style={{ gap: 7, paddingTop: 3 }}><Skeleton h={11} w="62%" /><Skeleton h={9} w="44%" /></span>
    </div>
  );
  return (
    <div className="org-canvas org-skel" aria-label="Loading the org chart">
      <div className="org-skel-tree">
        {card("you")}
        <span className="org-skel-stem" />
        <div className="org-skel-row">{["a", "b", "c"].map(card)}</div>
      </div>
    </div>
  );
}

/** Nobody hired yet: your card, a dashed stem, and the slot for your first employee, led by its own Display title. */
function EmptyOrg({ company, onHire, onAddPerson }: { company: Company; onHire: (template: string | null, from: HTMLElement) => void; onAddPerson: (from: HTMLElement) => void }) {
  return (
    <div className="org-empty" data-fold="canvas">
      <div className="org-empty-you"><YouCard company={company} onOpen={() => openMember(YOU_ID)} /></div>
      <span className="org-empty-stem" aria-hidden />
      <div className="org-empty-slot">
        <h2 className="org-empty-title">Hire your first employee</h2>
        <p className="org-empty-text">
          Give them a role and the tools they may use, then teach them how you like things done. Everything you teach
          becomes a skill, so they get better every week.
        </p>
        <div className="org-empty-templates" role="group" aria-label="Start from a role">
          {TEMPLATES.filter((t) => t.id !== "blank").map((t) => <button key={t.id} className="chip" onClick={(e) => onHire(t.id, e.currentTarget)}>{t.title}</button>)}
        </div>
        <div className="org-empty-actions">
          <button className="btn btn-primary" onClick={(e) => onHire(null, e.currentTarget)}><Icon name="plus" size={14} strokeWidth={1.8} /> Hire employee</button>
          <button className="btn btn-ghost" onClick={(e) => onAddPerson(e.currentTarget)}><Icon name="userPlus" size={14} /> Add a person</button>
        </div>
      </div>
    </div>
  );
}
