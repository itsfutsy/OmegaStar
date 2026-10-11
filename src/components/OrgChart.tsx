import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Company, EmployeeView, PersonView } from "../api/types";
import { initials, whenInline } from "../lib/format";
import { YOU_ID, up } from "../lib/org";
import { CARD_H, CARD_W, layoutOrg } from "../lib/orgLayout";
import { Avatar } from "./ui";
import "./OrgChart.css";

export type OrgView = "everyone" | "employees" | "people";
const MIN_ZOOM = 0.55;
const MIN_ZOOM_PANEL = 0.75;
const MAX_ZOOM = 1.1;
export type OrgChartHandle = { fit: () => void };

/** Nearest node above `id` that is visible under the current view (you always are), so filtering never orphans a subtree. */
function visibleParent(org: Parameters<typeof up>[0], id: string, visible: Set<string>): string | null {
  const seen = new Set<string>();
  let cur = up(org, id);
  while (cur !== null && cur !== YOU_ID && !visible.has(cur) && !seen.has(cur)) {
    seen.add(cur);
    cur = up(org, cur);
  }
  return cur !== null && (cur === YOU_ID || visible.has(cur)) ? cur : null;
}

export const OrgChart = forwardRef<OrgChartHandle, {
  company: Company; employees: EmployeeView[]; people: PersonView[]; selectedId: string | null; view: OrgView;
  onSelect: (id: string) => void; onClear: () => void; panelOpen: boolean;
}>(function OrgChart({ company, employees, people, selectedId, view, onSelect, onClear, panelOpen }, ref) {
  const shownEmployees = view === "people" ? [] : employees;
  const shownPeople = view === "employees" ? [] : people;
  const members = useMemo(() => {
    const org = { company, employees, people };
    const visible = new Set([...(view === "people" ? [] : employees), ...(view === "employees" ? [] : people)].map((m) => m.id));
    return [YOU_ID, ...visible].map((id) => ({ id, parentId: visibleParent(org, id, visible) }));
  }, [company, employees, people, view]);
  const wide = useMemo(() => layoutOrg(members, 3), [members]);
  const compact = useMemo(() => layoutOrg(members, 2), [members]);

  const known = useRef<Set<string> | null>(null);
  const fresh = new Set<string>();
  if (known.current) for (const m of members) if (!known.current.has(m.id)) fresh.add(m.id);
  useEffect(() => { known.current = new Set(members.map((m) => m.id)); }, [members]);

  const scroller = useRef<HTMLDivElement>(null);
  const [frame, setFrame] = useState<{ w: number; h: number } | null>(null);
  useLayoutEffect(() => {
    const el = scroller.current?.parentElement;
    if (!el) return;
    const measure = () => setFrame((f) => (f && f.w === el.clientWidth && f.h === el.clientHeight ? f : { w: el.clientWidth, h: el.clientHeight }));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // The canvas frame is whatever the page leaves (it shrinks while a panel is open). Padding
  // mirrors .orgcanvas-wrap: 32px each side, 24px on top, 64px below for the view controls.
  const availW = (frame?.w ?? 1000) - 64;
  const availH = (frame?.h ?? 600) - 88;
  const fitOf = (l: typeof wide) => Math.min(availW / l.width, availH / l.height);
  const layout = fitOf(wide) >= 0.9 || fitOf(wide) >= fitOf(compact) ? wide : compact;
  // Below the minimum the chart pans instead of shrinking further; with a panel open it stays readable.
  const zoom = Math.max(panelOpen ? MIN_ZOOM_PANEL : MIN_ZOOM, Math.min(MAX_ZOOM, Math.floor(fitOf(layout) * 100) / 100));
  // Scrolls the selected card (else the top of the chart) into view. It reads the selection through a ref so
  // a new selection scrolls smoothly once, without also counting as a layout change.
  const selected = useRef(selectedId);
  selected.current = selectedId;
  const reveal = useCallback((smooth: boolean) => requestAnimationFrame(() => {
    const el = scroller.current;
    if (!el) return;
    const behavior = smooth ? "smooth" : "auto";
    const node = selected.current ? el.querySelector<HTMLElement>(`[data-member="${selected.current}"]`) : null;
    if (node) node.scrollIntoView({ block: "center", inline: "center", behavior });
    else el.scrollTo({ left: Math.max(0, (el.scrollWidth - el.clientWidth) / 2), top: 0, behavior });
  }), []);
  useImperativeHandle(ref, () => ({ fit: () => reveal(true) }), [reveal]);
  useLayoutEffect(() => { reveal(false); }, [reveal, zoom, layout, frame?.w, frame?.h]);
  useEffect(() => { if (selectedId) reveal(true); }, [reveal, selectedId]);

  const you = layout.boxes.get(YOU_ID);
  return (
    <div className="orgscroll" ref={scroller} onClick={(e) => { if (!(e.target as Element).closest("[data-member]")) onClear(); }}>
      <div className="orgcanvas-wrap">
        <div className="orgcanvas" style={{ width: layout.width, height: layout.height, zoom } as React.CSSProperties}>
          <svg className="org-edges" width={layout.width} height={layout.height} aria-hidden>
            {layout.edges.map((e) => <path key={e.id} d={e.d} style={{ d: `path("${e.d}")` } as React.CSSProperties} />)}
          </svg>
          {you ? (
            <div className="org-node" style={{ transform: `translate(${you.x}px, ${you.y}px)`, width: CARD_W, height: CARD_H }}>
              <YouCard company={company} selected={selectedId === YOU_ID} onOpen={() => onSelect(YOU_ID)} />
            </div>
          ) : null}
          {shownEmployees.map((e) => {
            const b = layout.boxes.get(e.id);
            if (!b) return null;
            return (
              <div key={e.id} className={`org-node ${fresh.has(e.id) ? "org-node-enter" : ""}`} style={{ transform: `translate(${b.x}px, ${b.y}px)`, width: CARD_W, height: CARD_H }}>
                <EmployeeCard e={e} selected={selectedId === e.id} onOpen={() => onSelect(e.id)} />
              </div>
            );
          })}
          {shownPeople.map((p) => {
            const b = layout.boxes.get(p.id);
            if (!b) return null;
            return (
              <div key={p.id} className={`org-node ${fresh.has(p.id) ? "org-node-enter" : ""}`} style={{ transform: `translate(${b.x}px, ${b.y}px)`, width: CARD_W, height: CARD_H }}>
                <PersonCard p={p} selected={selectedId === p.id} onOpen={() => onSelect(p.id)} />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
});

type Status = { tone: "live" | "err" | "warn" | "plain" | "faint"; text: string; pulse?: boolean };

/** The one line under an employee's title: working, blocked, or when they next run. */
function employeeStatus(e: EmployeeView): Status {
  if (e.presence === "working") return { tone: "live", text: e.working > 1 ? `Working on ${e.working} things` : "Working now", pulse: true };
  if (e.presence === "queued") return { tone: "live", text: "Queued" };
  if (e.presence === "attention") return { tone: "err", text: e.lastRun?.denials.length ? "Blocked, needs your OK" : "Last run failed" };
  if (e.nextFire) return { tone: "plain", text: `Next run ${whenInline(e.nextFire)}` };
  return { tone: "faint", text: "Works when asked" };
}

function StatusLine({ s }: { s: Status }) {
  return (
    <span className={`mcard-status mcard-status-${s.tone}`}>
      {s.tone === "faint" ? null : <span className={`mcard-dot ${s.pulse ? "mcard-dot-pulse" : ""}`} />}
      <span className="mcard-status-text">{s.text}</span>
    </span>
  );
}

export function YouCard({ company, selected, onOpen }: { company: Company; selected: boolean; onOpen: () => void }) {
  return (
    <button
      type="button" className={`mcard mcard-you ${selected ? "mcard-selected" : ""}`} onClick={onOpen} data-member={YOU_ID}
      aria-label={`${company.ceoName}, you, ${company.title}. Edit your profile`} aria-current={selected || undefined}
    >
      <span className="ceo-av">{initials(company.ceoName)}</span>
      <span className="mcard-text">
        <span className="card-name">{company.ceoName}</span>
        <span className="card-title">{company.title} · {company.name}</span>
        <span className="mcard-status mcard-status-faint"><span className="mcard-status-text">You</span></span>
      </span>
    </button>
  );
}

function EmployeeCard({ e, selected, onOpen }: { e: EmployeeView; selected: boolean; onOpen: () => void }) {
  const status = employeeStatus(e);
  return (
    <button
      type="button" className={`mcard ${selected ? "mcard-selected" : ""}`} onClick={onOpen} data-member={e.id}
      aria-label={`${e.name}, ${e.title}. ${status.text}`} aria-current={selected || undefined}
    >
      <Avatar name={e.name} hue={e.hue} size={32} />
      <span className="mcard-text">
        <span className="card-name">{e.name}</span>
        <span className="card-title">{e.title}</span>
        <StatusLine s={status} />
      </span>
    </button>
  );
}

function PersonCard({ p, selected, onOpen }: { p: PersonView; selected: boolean; onOpen: () => void }) {
  const waiting = p.openRequests > 0;
  const text = waiting ? `Person · ${p.openRequests} waiting on them` : "Person";
  return (
    <button
      type="button" className={`mcard mcard-person ${selected ? "mcard-selected" : ""}`} onClick={onOpen} data-member={p.id}
      aria-label={`${p.name}, ${p.title}. ${text}`} aria-current={selected || undefined}
    >
      <Avatar name={p.name} hue={p.hue} size={32} person />
      <span className="mcard-text">
        <span className="card-name">{p.name}</span>
        <span className="card-title">{p.title}</span>
        <StatusLine s={{ tone: waiting ? "warn" : "faint", text }} />
      </span>
    </button>
  );
}
