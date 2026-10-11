import { useCallback, useRef, useState } from "react";
import type { Company, OrgSnapshot } from "../api/types";
import { useShell } from "../Shell";
import { OrgChart, YouCard, type OrgChartHandle, type OrgView } from "../components/OrgChart";
import { EmployeePanel } from "../components/panel/EmployeePanel";
import { PersonPanel } from "../components/panel/PersonPanel";
import { YouPanel } from "../components/panel/YouPanel";
import { TEMPLATES } from "../components/HireSheet";
import { Icon } from "../components/Icon";
import { ErrorNote, PageHeader, Skeleton } from "../components/ui";
import { YOU_ID } from "../lib/org";
import { refreshOrg, useOrg } from "../lib/hooks";
import { navigate, openMember, openYou, type PanelTab, type Route } from "../lib/router";
import "./Org.css";

export function OrgScreen({ route }: { route: Extract<Route, { page: "org" }> }) {
  const { snap, error } = useOrg();
  const shell = useShell();
  const close = useCallback(() => navigate({ page: "org", memberId: null, tab: null, focus: null }), []);

  if (!snap) {
    return (
      <div className="org">
        <PageHeader title="Org chart" />
        <div className="org-loading">
          {error ? <ErrorNote text={error} onRetry={refreshOrg} /> : <Skeleton h={320} r={12} />}
        </div>
      </div>
    );
  }

  const employee = route.memberId ? snap.employees.find((e) => e.id === route.memberId) ?? null : null;
  const person = route.memberId && !employee ? snap.people.find((p) => p.id === route.memberId) ?? null : null;
  const you = route.memberId === YOU_ID;
  const panelOpen = !!(employee || person) || you;
  const empty = snap.employees.length === 0 && snap.people.length === 0;
  return (
    <div className={`org ${panelOpen ? "org-panel" : ""}`}>
      <PageHeader title="Org chart">
        {empty ? null : (
          <>
            <button className="btn" onClick={() => shell.addPerson()} title="Add a real person to the org chart"><Icon name="person" size={14} /><span className="org-btn-label">Add person</span></button>
            <button className="btn btn-primary" onClick={() => shell.hire()}><Icon name="plus" size={14} strokeWidth={1.8} /><span className="org-btn-label">Hire employee</span></button>
          </>
        )}
      </PageHeader>
      <Chart snap={snap} selectedId={route.memberId} panelOpen={panelOpen} tab={route.tab} empty={empty} onClear={close} />
      {employee ? <EmployeePanel key={employee.id} employee={employee} tab={route.tab} focus={route.focus} onClose={close} /> : null}
      {person ? <PersonPanel key={person.id} person={person} onClose={close} /> : null}
      {you ? <YouPanel onClose={close} /> : null}
    </div>
  );
}

const VIEWS: { id: OrgView; label: string }[] = [{ id: "everyone", label: "Everyone" }, { id: "employees", label: "Employees" }, { id: "people", label: "People" }];

function Chart({ snap, selectedId, panelOpen, tab, empty, onClear }: { snap: OrgSnapshot; selectedId: string | null; panelOpen: boolean; tab: PanelTab | null; empty: boolean; onClear: () => void }) {
  const shell = useShell();
  const chart = useRef<OrgChartHandle>(null);
  const [view, setView] = useState<OrgView>("everyone");

  return (
    <div className="org-canvas">
      {empty ? (
        <EmptyOrg company={snap.company} selected={selectedId === YOU_ID} onHire={(t) => shell.hire(null, t)} onAddPerson={() => shell.addPerson()} />
      ) : (
        <>
          <OrgChart
            ref={chart} company={snap.company} employees={snap.employees} people={snap.people} view={view}
            selectedId={selectedId} onSelect={(id) => openMember(id, selectedId === id ? tab : null)} onClear={onClear} panelOpen={panelOpen}
          />
          <div className="org-controls">
            {snap.people.length > 0 ? (
              <div className="seg" role="group" aria-label="Show">
                {VIEWS.map((v) => <button key={v.id} aria-pressed={view === v.id} onClick={() => setView(v.id)}>{v.label}</button>)}
              </div>
            ) : null}
            <button className="btn btn-icon org-fit" onClick={() => chart.current?.fit()} aria-label="Fit to view" title="Fit to view"><Icon name="fit" size={14} /></button>
          </div>
        </>
      )}
    </div>
  );
}

function EmptyOrg({ company, selected, onHire, onAddPerson }: { company: Company; selected: boolean; onHire: (template: string | null) => void; onAddPerson: () => void }) {
  return (
    <div className="org-empty">
      <div className="org-empty-you"><YouCard company={company} selected={selected} onOpen={openYou} /></div>
      <svg className="org-empty-line" width="2" height="40" aria-hidden><path d="M1 0 V40" /></svg>
      <div className="org-empty-card">
        <h2 className="h-lg">Hire your first employee</h2>
        <p className="muted">
          Give them a role and the tools they may use, then teach them how you like things done.
          Everything you teach becomes a skill, so they get better every week.
        </p>
        <div className="org-empty-templates" role="group" aria-label="Start from a role">
          {TEMPLATES.filter((t) => t.id !== "blank").map((t) => <button key={t.id} className="chip" onClick={() => onHire(t.id)}>{t.title}</button>)}
        </div>
        <div className="row" style={{ gap: 8, marginTop: 4 }}>
          <button className="btn btn-primary" onClick={() => onHire(null)}><Icon name="plus" size={14} strokeWidth={1.8} /> Hire employee</button>
          <button className="btn btn-ghost" onClick={onAddPerson}><Icon name="person" size={14} /> Add a person</button>
        </div>
      </div>
    </div>
  );
}
