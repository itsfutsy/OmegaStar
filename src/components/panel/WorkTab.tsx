import { useMemo, useState } from "react";
import { call } from "../../api/client";
import type { EmployeeView, Run, ScheduleDraft } from "../../api/types";
import { dayHeading, firstName } from "../../lib/format";
import { refreshOrg, useOrg, useQuery } from "../../lib/hooks";
import { RunRow } from "../RunRow";
import { ErrorNote, Section, Skeleton } from "../ui";
import { ScheduleEditor, Schedules } from "./Schedules";

export function groupByDay(runs: Run[]): { day: string; runs: Run[] }[] {
  const out: { day: string; runs: Run[] }[] = [];
  for (const r of runs) {
    const d = dayHeading(r.startedAt ?? r.createdAt);
    const last = out[out.length - 1];
    if (last && last.day === d) last.runs.push(r);
    else out.push({ day: d, runs: [r] });
  }
  return out;
}

/** What an employee does: their schedules, then every run, live ones first. Chat turns stay in their ongoing chat. */
export function WorkTab({ e }: { e: EmployeeView }) {
  const { snap } = useOrg();
  const [editing, setEditing] = useState<ScheduleDraft | null>(null);
  const q = useQuery(() => call("runs", { employeeId: e.id, limit: 100 }), [e.id], ["run:updated"]);
  // Anything going right now, whichever lane it is in: asking for work in chat is how most of it starts.
  // Everything else is history, and finished chat turns stay in the ongoing chat.
  const live = useMemo(() => (q.data ?? []).filter((r) => r.state === "running" || r.state === "queued"), [q.data]);
  const work = useMemo(() => (q.data ?? []).filter((r) => r.kind.type !== "chat"), [q.data]);
  const groups = useMemo(() => groupByDay(work.filter((r) => !live.includes(r))), [work, live]);

  if (editing) return <ScheduleEditor e={e} initial={editing} onDone={() => { setEditing(null); refreshOrg(); }} />;
  const employees = snap?.employees ?? [];
  const people = snap?.people ?? [];
  return (
    <div className="tab-scroll">
      <Schedules e={e} onEdit={setEditing} />
      <Section title="Runs">
        {q.error ? <ErrorNote text={q.error} onRetry={q.reload} /> : null}
        {!q.data && !q.error ? <div className="stack" style={{ gap: 8 }}>{[0, 1, 2].map((i) => <Skeleton key={i} h={40} r={10} />)}</div> : null}
        {q.data && work.length === 0 && live.length === 0 ? (
          <p className="tab-empty">No runs yet. They show up here when {firstName(e.name)} works on a schedule, when you press Run now, or when a colleague hands them something.</p>
        ) : null}
        <div className="work-runs">
          {live.length ? (
            <div className="daygroup">
              <div className="daygroup-title">Now</div>
              {live.map((r) => <RunRow key={r.id} run={r} employees={employees} people={people} />)}
            </div>
          ) : null}
          {groups.map((g) => (
            <div key={g.day} className="daygroup">
              <div className="daygroup-title">{g.day}</div>
              {g.runs.map((r) => <RunRow key={r.id} run={r} employees={employees} people={people} />)}
            </div>
          ))}
        </div>
      </Section>
    </div>
  );
}
