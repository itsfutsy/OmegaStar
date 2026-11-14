import { useState, type ReactNode } from "react";
import type { EmployeeView, PersonView, Thread } from "../../api/types";
import { dateLong, firstName, relTime, runKindLabel, whenInline } from "../../lib/format";
import { openRun } from "../../lib/router";
import { toolName } from "../../lib/rules";
import { blocked, didNotFinish, isChoreRow, isLive, isStreak, isTeamWeek, openCards, reportOf, rowThreads } from "../../lib/threads";
import { kindNames } from "../../lib/workTitle";
import { RunCards } from "./CardView";
import { Icon } from "../Icon";
import { Avatar } from "../ui";
import "./Chat.css";

/** One line of markdown as plain text. */
function plain(line: string): string {
  return line
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+[.)])\s+/, "")
    .replace(/[*_`~]/g, "")
    // An image path at the end of the line (shown as the image in the message), with the colon that led into it.
    .replace(/(?:^|:?\s+)(?:\/|~\/|[A-Za-z]:[\\/])\S[^\n]*?\.(?:png|jpe?g|gif|webp)[.,;)]?\s*$/i, "")
    .replace(/\|/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/** A report's lines as plain text, without code blocks, table rules or blank lines. */
export const lines = (md: string | null) => (md ?? "").replace(/```[\s\S]*?```/g, " ").split(/\r?\n/).map(plain).filter((l) => l && !/^[-:\s]+$/.test(l));

/** Every run filed under this row, oldest first: the rest of a hand-off between employees, everyone waiting on the same
 *  chore, or a week of reviews; each opens its conversation. A routine failing in a row is said once instead: how many
 *  times, when, and when it runs next, with the error itself in the report below. */
export function RowHistory({ t, employees, people, retry = null, refs = null }: { t: Thread; employees: EmployeeView[]; people: PersonView[]; retry?: ReactNode; refs?: ReactNode }) {
  const all = rowThreads(t);
  const [showAll, setShowAll] = useState(false);
  if (isStreak(t)) return <Streak t={t} all={all} employees={employees} people={people} retry={retry} refs={refs} />;
  const label = isChoreRow(t) ? "Waiting on this" : isTeamWeek(t) ? "This week's reviews" : "In this hand-off";
  const shown = showAll || all.length <= 6 ? all : all.slice(-5);
  return (
    <section className="hist" aria-label={label}>
      <div className="hist-head">
        <Icon name="layers" size={14} />
        <span className="hist-title">{label}</span>
        <span>· {all.length} runs</span>
        <span className="grow" />
        {all.length > 6 ? <button className="btn btn-ghost btn-sm" onClick={() => setShowAll((x) => !x)}>{showAll ? "Show latest" : `Show all ${all.length}`}</button> : null}
      </div>
      {shown.map((x) => <HistoryRow key={x.root.id} x={x} lead={x.root.id === t.root.id} employees={employees} people={people} />)}
    </section>
  );
}

/** "Reconcile payouts failed 3 times in a row", what went wrong (once), when it runs next, when each failure was, and
 *  Retry: the one error, in the parts sheet's form. */
function Streak({ t, all, employees, people, retry, refs }: { t: Thread; all: Thread[]; employees: EmployeeView[]; people: PersonView[]; retry: ReactNode; refs: ReactNode }) {
  const r = t.root;
  const e = employees.find((m) => m.id === r.employeeId);
  const what = runKindLabel(r.kind, kindNames(r, employees, people));
  return (
    <section className="streak" aria-label="Failures in a row">
      <Icon name="alert" size={16} />
      <div className="streak-main">
        <span className="streak-title">{what} failed {all.length} times in a row</span>
        <span className="streak-sub">
          {r.error ?? "It didn't finish."}{e?.nextFire ? ` ${firstName(e.name)} tries again ${whenInline(e.nextFire)}.` : ""}
        </span>
        <span className="streak-runs">
          {all.map((x) => (
            <button key={x.root.id} type="button" className="streak-run" data-tip={`Open the run from ${dateLong(x.root.endedAt ?? x.root.createdAt)}`} onClick={() => openRun(x.root.id)}>
              {relTime(x.root.endedAt ?? x.root.createdAt)}
            </button>
          ))}
        </span>
        {refs ? <div className="streak-refs">{refs}</div> : null}
      </div>
      {retry}
    </section>
  );
}

/** One run in the row's history: who, what it was, what it said, and a way to open it. Below it, the cards it raised
 *  that wait on you, answered here; the thread shown below has its own, and a chore is answered once for everyone. */
function HistoryRow({ x, lead, employees, people }: { x: Thread; lead: boolean; employees: EmployeeView[]; people: PersonView[] }) {
  const r = x.root;
  const e = employees.find((m) => m.id === r.employeeId);
  const asking = lead ? [] : [...new Set(openCards(x).filter((c) => c.card.ask.type !== "chore").map((c) => c.run))];
  const failed = didNotFinish(r);
  const text = r.needsYou[0]
    ?? (blocked(r) ? `Blocked on ${toolName(r.denials[0].tool)}` : null)
    ?? lines(reportOf(r))[0]
    ?? (isLive(r) ? "Working" : failed ? r.error ?? "This run didn't finish" : "No report");
  return (
    <div className={`hist-row ${lead ? "hist-row-lead" : ""}`}>
      {e ? <Avatar name={e.name} hue={e.hue} image={e.image} size={20} presence={isLive(r) ? "working" : undefined} /> : <span />}
      <div className="hist-main">
        <span className="hist-who">
          <b>{e ? firstName(e.name) : "Former employee"}</b>
          <span> · {runKindLabel(r.kind, kindNames(r, employees, people))}</span>
          {x.replies.length ? <span> · {x.replies.length} in thread</span> : null}
          {lead ? <span> · shown below</span> : null}
        </span>
        <span className="hist-text">
          {r.needsYou.length ? <Icon name="bell" size={13} className="hist-attn" /> : null}
          {blocked(r) && !r.needsYou.length ? <Icon name="shield" size={13} className="hist-attn" /> : null}
          {failed ? <Icon name="alert" size={13} className="hist-attn" /> : null}
          <span>{text}</span>
        </span>
        {e ? asking.map((run) => <div key={run.id} className="hist-card"><RunCards run={run} e={e} /></div>) : null}
      </div>
      <span className="hist-when" title={dateLong(r.endedAt ?? r.createdAt)}>{relTime(r.endedAt ?? r.createdAt)}</span>
      <button className="btn btn-ghost btn-sm btn-icon" data-tip="Open run" aria-label="Open run" onClick={() => openRun(r.id)}><Icon name="arrowRight" size={13} /></button>
    </div>
  );
}
