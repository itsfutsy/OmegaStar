import { useMemo, useState } from "react";
import { call } from "../api/client";
import type { EmployeeView, SkillRevision } from "../api/types";
import { dayHeading, firstName, relTime } from "../lib/format";
import { useOrg, useQuery } from "../lib/hooks";
import { navigate, openMember, type ActivityView } from "../lib/router";
import { RunRow } from "../components/RunRow";
import { groupByDay } from "../components/panel/WorkTab";
import { lineStats } from "../components/Diff";
import { Icon } from "../components/Icon";
import { Avatar, ErrorNote, PageColumn, Skeleton } from "../components/ui";
import "./Activity.css";

/** Everything that happened: the runs your employees did, or the changes they (and you) made to their skills. */
export function ActivityScreen({ view }: { view: ActivityView }) {
  const { snap } = useOrg();
  const [who, setWho] = useState<string | null>(null);
  const employees = snap?.employees ?? [];
  const chosen = employees.find((e) => e.id === who);

  return (
    <PageColumn
      title="Activity"
      actions={
        <>
          <select className="select activity-who" value={who ?? ""} onChange={(ev) => setWho(ev.target.value || null)} aria-label="Whose activity">
            <option value="">Everyone</option>
            {employees.map((e) => <option key={e.id} value={e.id}>{e.name}</option>)}
          </select>
          <div className="seg" role="group" aria-label="Show">
            <button aria-pressed={view === "runs"} onClick={() => navigate({ page: "activity", view: "runs" })}>Runs</button>
            <button aria-pressed={view === "skills"} onClick={() => navigate({ page: "activity", view: "skills" })}>Skill changes</button>
          </div>
        </>
      }
    >
      {view === "runs" ? <Runs who={who} name={chosen ? firstName(chosen.name) : null} employees={employees} /> : <SkillChanges who={who} name={chosen ? firstName(chosen.name) : null} employees={employees} />}
    </PageColumn>
  );
}

function Runs({ who, name, employees }: { who: string | null; name: string | null; employees: EmployeeView[] }) {
  const { snap } = useOrg();
  const q = useQuery(() => call("runs", { employeeId: null, limit: 300 }), [], ["run:updated"]);
  // Chat turns stay in each employee's ongoing chat.
  const runs = useMemo(() => (q.data ?? []).filter((r) => (who === null || r.employeeId === who) && r.kind.type !== "chat"), [q.data, who]);
  const groups = useMemo(() => groupByDay(runs), [runs]);

  if (q.error) return <ErrorNote text={q.error} onRetry={q.reload} />;
  if (!q.data) return <div className="stack" style={{ gap: 10 }}>{[0, 1, 2, 3, 4].map((i) => <Skeleton key={i} h={44} r={10} />)}</div>;
  if (runs.length === 0) {
    return (
      <div className="empty">
        <Icon name="activity" size={22} />
        <span className="empty-title">{name ? `No runs for ${name} yet` : "No runs yet"}</span>
        <p>Runs show up here the moment they start: schedules, Run now, and work employees hand each other.</p>
      </div>
    );
  }
  return (
    <div className="activity-list">
      {groups.map((g) => (
        <div key={g.day} className="daygroup">
          <div className="daygroup-title">{g.day}</div>
          {g.runs.map((r) => <RunRow key={r.id} run={r} employee={employees.find((e) => e.id === r.employeeId)} employees={employees} people={snap?.people ?? []} showEmployee />)}
        </div>
      ))}
    </div>
  );
}

type Change = { r: SkillRevision; e: EmployeeView };

/** Every employee's skill history, newest first, reloaded when the office changes. */
function useChanges(employees: EmployeeView[]): Change[] | undefined {
  const load = () => Promise.all(employees.map(async (e) => (await call("skill_history", { employeeId: e.id }).catch(() => [])).map((r) => ({ r, e }))))
    .then((all) => all.flat().sort((a, b) => b.r.at.localeCompare(a.r.at)));
  return useQuery(load, [employees.map((e) => e.id)], ["org:changed"]).data;
}

function SkillChanges({ who, name, employees }: { who: string | null; name: string | null; employees: EmployeeView[] }) {
  const all = useChanges(employees);
  const changes = useMemo(() => (all ?? []).filter((c) => who === null || c.e.id === who), [all, who]);
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

  if (!all) return <div className="stack" style={{ gap: 10 }}>{[0, 1, 2, 3].map((i) => <Skeleton key={i} h={40} r={10} />)}</div>;
  if (changes.length === 0) {
    return (
      <div className="empty">
        <Icon name="layers" size={22} />
        <span className="empty-title">{name ? `${name} hasn't changed a skill yet` : "No skill changes yet"}</span>
        <p>When you teach an employee something, or they learn from a run, the change to their skill shows up here. Any change can be undone.</p>
      </div>
    );
  }
  return (
    <div className="activity-list">
      {groups.map((g) => (
        <div key={g.day} className="daygroup">
          <div className="daygroup-title">{g.day}</div>
          {g.items.map(({ r, e }) => <ChangeRow key={r.id} r={r} e={e} />)}
        </div>
      ))}
    </div>
  );
}

function ChangeRow({ r, e }: { r: SkillRevision; e: EmployeeView }) {
  const s = lineStats(r.before, r.after);
  const verb = r.change === "added" ? "created" : r.change === "removed" ? "removed" : "changed";
  const how = r.runId ? "while working" : r.change === "added" ? "at setup" : "edited by you";
  return (
    <button className="runrow" onClick={() => openMember(e.id, "skills", `rev~${r.id}`)}>
      <Avatar name={e.name} hue={e.hue} size={28} />
      <span className="runrow-main">
        <span className="runrow-title"><b>{firstName(e.name)}</b> <span className="muted">{verb}</span> <span className="mono change-skill">{r.skill}{r.file !== "SKILL.md" ? `/${r.file}` : ""}</span></span>
        <span className="runrow-detail">{how}</span>
      </span>
      <span className="runrow-meta">
        <span className="runrow-when">
          <span className="mono num change-lines">{s.added ? <span className="tl-add">+{s.added}</span> : null}{s.removed ? <span className="tl-del">−{s.removed}</span> : null}</span>
          <span className="faint">{relTime(r.at)}</span>
        </span>
      </span>
    </button>
  );
}
