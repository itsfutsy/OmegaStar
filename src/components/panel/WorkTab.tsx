import { useMemo, useState } from "react";
import { call } from "../../api/client";
import type { EmployeeView, Run, ScheduleDraft } from "../../api/types";
import { dayHeading, firstName } from "../../lib/format";
import { refreshOrg, useOrg, useQuery } from "../../lib/hooks";
import { toolName } from "../../lib/rules";
import { isLive } from "../../lib/threads";
import { workTitle } from "../../lib/workTitle";
import { RunRow } from "../RunRow";
import { ErrorNote, Section, Skeleton } from "../ui";
import { blankSchedule, ScheduleEditor, Schedules } from "./Schedules";
import { Icon } from "../Icon";
import { openChat, openRun } from "../../lib/router";

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

/** What an employee does. A run that failed or was blocked leads, with what went wrong and where to act; then their
 *  schedules, then every run, live ones first. Someone with no schedules and no runs gets one next step instead. */
export function WorkTab({ e }: { e: EmployeeView }) {
  const { snap } = useOrg();
  const [editing, setEditing] = useState<ScheduleDraft | null>(null);
  const q = useQuery(() => call("runs", { employeeId: e.id, limit: 100 }), [e.id], ["run:updated"]);
  const sched = useQuery(() => call("schedules", { employeeId: e.id }), [e.id], ["org:changed"]);
  // The run that needs a look, said once: here at the top, not again in the history below.
  const failed = e.presence === "attention" && e.lastRun ? e.lastRun : null;
  // Anything going right now, conversations included: asking for work in one is how most of it starts.
  // Everything else is history.
  const live = useMemo(() => (q.data ?? []).filter(isLive), [q.data]);
  const groups = useMemo(() => groupByDay((q.data ?? []).filter((r) => !live.includes(r) && r.id !== failed?.id)), [q.data, live, failed]);

  if (editing) return <ScheduleEditor e={e} initial={editing} onDone={() => { setEditing(null); refreshOrg(); }} />;
  const employees = snap?.employees ?? [];
  const people = snap?.people ?? [];
  const name = firstName(e.name);
  if (q.data?.length === 0 && sched.data?.length === 0) {
    return (
      <div className="tab-scroll">
        <div className="first-work">
          <h3 className="first-work-title">Give {name} a first job</h3>
          <p className="first-work-text">
            {name} has no runs yet and works only when asked. Add a schedule and they run on their own, or start a
            conversation and teach them the job.
          </p>
          <div className="first-work-actions">
            <button className="btn btn-primary" onClick={() => setEditing(blankSchedule(e))}><Icon name="clock" size={14} /> Add a schedule</button>
            <button className="btn btn-ghost" onClick={() => openChat(e.id, null)}><Icon name="chat" size={14} /> Start a conversation</button>
          </div>
        </div>
      </div>
    );
  }
  return (
    <div className="tab-scroll">
      {failed ? <NeedsALook e={e} run={failed} /> : null}
      <Schedules e={e} onEdit={setEditing} />
      <Section title="Runs">
        {q.error ? <ErrorNote text={q.error} onRetry={q.reload} /> : null}
        {!q.data && !q.error ? <div className="stack" style={{ gap: 8 }}>{[0, 1, 2].map((i) => <Skeleton key={i} h={40} r={10} />)}</div> : null}
        {q.data && q.data.length === 0 ? (
          <p className="tab-empty">No runs yet. They show up here when {name} works on a schedule, when you press Run now, or when a colleague hands them something.</p>
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

/** The run that failed, was interrupted or is blocked: what it was, what went wrong, and one way to act. */
function NeedsALook({ e, run }: { e: EmployeeView; run: Run }) {
  const { snap } = useOrg();
  const what = workTitle(run, snap?.employees ?? [], snap?.people ?? []);
  const blocked = run.denials.length > 0;
  const title = blocked ? `${what} is blocked` : run.state === "lost" ? `${what} was interrupted` : `${what} failed`;
  const text = blocked
    ? `${firstName(e.name)} needs your OK to use ${run.denials.map((d) => toolName(d.tool)).filter((t, i, a) => a.indexOf(t) === i).join(", ")}. Allow it from the run, and it retries.`
    : run.error ?? "It stopped before it finished.";
  return (
    <section className="psec needs-look">
      <div className="error-note error-note-block" role="alert">
        <Icon name="alert" size={15} />
        <div className="grow">
          <div className="error-note-title">{title}</div>
          <div className="error-note-text">{text}</div>
        </div>
        <button className="btn btn-sm" onClick={() => openRun(run.id)}>Open run</button>
      </div>
    </section>
  );
}
