import { useState } from "react";
import type { EmployeeView, PersonView, Thread } from "../../api/types";
import { dateLong, firstName, relTime, runKindLabel } from "../../lib/format";
import { openRun } from "../../lib/router";
import { toolName } from "../../lib/rules";
import { didNotFinish, isChoreRow, isLive, isStreak, isTeamWeek, openCard, rowThreads } from "../../lib/threads";
import { answerCard, CardView } from "./CardView";
import { kindNames } from "../RunRow";
import { Icon } from "../Icon";
import { Avatar } from "../ui";
import "../../screens/Inbox.css";

/** One line of markdown as plain text. */
function plain(line: string): string {
  return line
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+[.)])\s+/, "")
    .replace(/[*_`~]/g, "")
    .replace(/\|/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/** A report's lines as plain text, without code blocks, table rules or blank lines. */
export const lines = (md: string | null) => (md ?? "").replace(/```[\s\S]*?```/g, " ").split(/\r?\n/).map(plain).filter((l) => l && !/^[-:\s]+$/.test(l));

/** Every run filed under this row, oldest first: the rest of a hand-off between employees, a schedule's
 *  failures in a row, or everyone waiting on the same chore. The one this thread shows is marked; each opens its run. */
export function RowHistory({ t, employees, people }: { t: Thread; employees: EmployeeView[]; people: PersonView[] }) {
  const all = rowThreads(t);
  const streak = isStreak(t);
  const label = streak ? "Failures in a row" : isChoreRow(t) ? "Waiting on this" : isTeamWeek(t) ? "This week's reviews" : "In this hand-off";
  const [showAll, setShowAll] = useState(false);
  const shown = showAll || all.length <= 6 ? all : all.slice(-5);
  return (
    <section className="thread-strip" aria-label={label}>
      <div className="thread-strip-head">
        <Icon name="layers" size={13} />
        <span className="thread-strip-title">{label}</span>
        <span>· {all.length} runs</span>
        {all.length > 6 ? <button className="btn btn-ghost btn-sm thread-strip-toggle" onClick={() => setShowAll((s) => !s)}>{showAll ? "Show latest" : `Show all ${all.length}`}</button> : null}
      </div>
      {shown.map((x) => <HistoryRow key={x.root.id} x={x} lead={x.root.id === t.root.id} employees={employees} people={people} />)}
    </section>
  );
}

/** One run in the row's history: who, what it was, what it said, and a way to open it. Below it, a card it raised that
 *  waits on you, answered here; the thread shown below has its own, and a chore is answered once for everyone. */
function HistoryRow({ x, lead, employees, people }: { x: Thread; lead: boolean; employees: EmployeeView[]; people: PersonView[] }) {
  const r = x.root;
  const e = employees.find((m) => m.id === r.employeeId);
  const card = lead ? undefined : openCard(x);
  const failed = didNotFinish(r);
  const text = r.needsYou[0]
    ?? (r.denials.length ? `Blocked on ${toolName(r.denials[0].tool)}` : null)
    ?? lines(r.summary)[0]
    ?? (isLive(r) ? "Working…" : failed ? r.error ?? "This run didn't finish" : "No report");
  return (
    <div className={`row-run ${lead ? "row-run-lead" : ""}`}>
      {e ? <Avatar name={e.name} hue={e.hue} size={20} presence={isLive(r) ? "working" : undefined} /> : <span className="row-run-noav" />}
      <div className="row-run-main">
        <span className="row-run-head">
          <b>{e ? firstName(e.name) : "Former employee"}</b>
          <span className="muted"> · {runKindLabel(r.kind, kindNames(r, employees, people))}</span>
          {x.replies.length ? <span className="faint"> · {x.replies.length} in thread</span> : null}
          {lead ? <span className="faint"> · shown below</span> : null}
        </span>
        <span className="row-run-text">
          {r.needsYou.length ? <Icon name="bell" size={12} className="item-bell" /> : null}
          {r.denials.length && !r.needsYou.length ? <Icon name="shield" size={12} className="item-bell" /> : null}
          {failed ? <Icon name="alert" size={12} className="item-failed" /> : null}
          <span>{text}</span>
        </span>
        {card && e && card.card?.ask.type !== "chore" ? (
          <div className="item-card">
            <CardView run={card} e={e} onAnswer={async (reply) => { await answerCard(card, reply, firstName(e.name)); }} />
          </div>
        ) : null}
      </div>
      <span className="faint row-run-when" title={dateLong(r.endedAt ?? r.createdAt)}>{relTime(r.endedAt ?? r.createdAt)}</span>
      <button className="btn btn-ghost btn-sm btn-icon" title="Open run" aria-label="Open run" onClick={() => openRun(r.id)}><Icon name="arrowRight" size={13} /></button>
    </div>
  );
}
