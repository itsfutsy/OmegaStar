import type { EmployeeView, PersonView, Run } from "../api/types";
import { duration, elapsedMs, firstName, heldLabel, relTime, runKindLabel, runStateLabel, runStateTone, whenShort } from "../lib/format";
import { useNow, useUsage } from "../lib/hooks";
import { openRun } from "../lib/router";
import { Icon, type IconName } from "./Icon";
import { Avatar, StatePill } from "./ui";
import "./RunRow.css";

export function kindNames(run: Run, employees: EmployeeView[], people: PersonView[]): { manager?: string; person?: string; asker?: string } {
  if (run.kind.type === "asked" || run.kind.type === "answered") {
    const other = run.kind.type === "asked" ? run.kind.askedBy : run.kind.from;
    const a = employees.find((x) => x.id === other);
    return { asker: a ? firstName(a.name) : undefined };
  }
  if (run.kind.type === "delegated") {
    const e = employees.find((x) => x.id === run.employeeId);
    const m = e?.managerId ? employees.find((x) => x.id === e.managerId) : undefined;
    return { manager: m ? firstName(m.name) : undefined };
  }
  if (run.kind.type === "followUp" || run.kind.type === "replyCheck" || run.kind.type === "nudge" || run.kind.type === "unanswered") {
    const personId = run.kind.personId;
    const p = people.find((x) => x.id === personId);
    return { person: p ? firstName(p.name) : undefined };
  }
  return {};
}

export function kindIcon(run: Run): IconName {
  switch (run.kind.type) {
    case "scheduled": return "clock";
    case "manual": return "play";
    case "delegated": return "branch";
    case "asked": return "ask";
    case "answered": return "reply";
    case "followUp": return "ask";
    case "replyCheck": return "search";
    case "nudge": return "bell";
    case "unanswered": return "clock";
    case "chat": return "chat";
    case "organize": return "layers";
    case "tidy": return "sparkle";
    case "reply": return "reply";
    case "task": return "branch";
    case "hireDecided": return "userPlus";
  }
}

/** While the office waits for the usage limit, when a queued run starts. */
function Held() {
  const { usage } = useUsage();
  const now = useNow(60_000);
  if (!usage?.heldUntil || Date.parse(usage.heldUntil) <= now) return null;
  return <span className="runrow-detail">{heldLabel(usage.heldUntil, new Date(now))}</span>;
}

export function RunRow({ run, employee, employees, people, showEmployee = false }: {
  run: Run; employee?: EmployeeView; employees: EmployeeView[]; people: PersonView[]; showEmployee?: boolean;
}) {
  const live = run.state === "running";
  const now = useNow(1000, live);
  const label = runKindLabel(run.kind, kindNames(run, employees, people));
  const detail = run.summary ? run.summary.replace(/[*_`#>|]/g, "").split("\n").find((l) => l.trim()) ?? "" : run.prompt;
  const took = duration(elapsedMs(run, now));
  // A plain success needs no badge; anything else says what happened.
  const pill = run.state === "succeeded"
    ? (run.denials.length ? <StatePill tone="warn">Blocked</StatePill> : null)
    : <StatePill tone={runStateTone(run)} pulse={live}>{runStateLabel(run.state)}</StatePill>;
  return (
    <button className="runrow" onClick={() => openRun(run.id)} title={took ? `${whenShort(run.createdAt)} · took ${took}` : whenShort(run.createdAt)}>
      {showEmployee && employee ? <Avatar name={employee.name} hue={employee.hue} size={28} presence={live ? "working" : undefined} /> : (
        <span className="runrow-icon"><Icon name={kindIcon(run)} size={14} /></span>
      )}
      <span className="runrow-main">
        <span className="runrow-title">
          {showEmployee && employee ? <b>{firstName(employee.name)}</b> : null}
          <span className={showEmployee ? "muted" : ""}>{label}</span>
        </span>
        <span className="runrow-detail">{detail}</span>
        {run.state === "queued" ? <Held /> : null}
      </span>
      <span className="runrow-meta">
        <span className="runrow-when">
          {pill}
          <span className="faint">{relTime(run.startedAt ?? run.createdAt, now)}</span>
        </span>
      </span>
    </button>
  );
}
