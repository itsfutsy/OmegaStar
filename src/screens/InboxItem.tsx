import { useState } from "react";
import { call, errorMessage } from "../api/client";
import type { EmployeeView, PersonView, Run, Thread } from "../api/types";
import { dateLong, firstName, relTime, runKindLabel, timeOrDay } from "../lib/format";
import { toast } from "../lib/toasts";
import { openChat, openRun } from "../lib/router";
import { activityAt, didNotFinish, isLive, isStreak, lastFrom, lastReply, latestFinished, rowThreads, threadBusy, threadFocus } from "../lib/threads";
import { toolName } from "../lib/rules";
import { kindNames } from "../components/RunRow";
import { Icon } from "../components/Icon";
import { Markdown } from "../components/Markdown";
import { Avatar, ErrorNote, SkillsGrew } from "../components/ui";

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

const lines = (md: string | null) => (md ?? "").replace(/```[\s\S]*?```/g, " ").split(/\r?\n/).map(plain).filter((l) => l && !/^[-:\s]+$/.test(l));

/** The collapsed row, about the thread's newest finished run: what the employee needs from you, else what blocked them, else the first line; then a preview. */
function digest(t: Thread): { run: Run; title: string; preview: string } {
  const run = latestFinished(t);
  const ls = lines(run.summary);
  if (run.needsYou.length) return { run, title: run.needsYou[0], preview: ls.join(" ") };
  if (run.denials.length) {
    const title = run.denials.length === 1 ? `Blocked on ${toolName(run.denials[0].tool)}. Waiting for your OK` : `Blocked on ${run.denials.length} actions. Waiting for your OK`;
    return { run, title, preview: ls.join(" ") };
  }
  if (ls.length) return { run, title: ls[0], preview: ls.slice(1).join(" ") };
  const failed = didNotFinish(run);
  return { run, title: failed ? (run.id === t.root.id ? "This run didn't finish" : "This reply didn't finish") : "No report", preview: run.error ?? "" };
}

/** One row of the Archive: a thread you put away, collapsed to its digest or open with the report, its thread
 *  and a reply box. Replying or Reopen brings it back to the sidebar. */
export function InboxItem({ t, e, employees, people, open, onToggle, onHover, onReopen }: {
  t: Thread; e: EmployeeView | undefined; employees: EmployeeView[]; people: PersonView[]; open: boolean;
  onToggle: () => void; onHover: (on: boolean) => void; onReopen: () => void;
}) {
  const { run, title, preview } = digest(t);
  const r = t.root;
  const failed = didNotFinish(run);
  const who = e ? firstName(e.name) : "They";
  const context = runKindLabel(r.kind, kindNames(r, employees, people));
  const busy = threadBusy(t);
  const last = lastReply(t);
  const from = lastFrom(t);
  const n = t.replies.length;
  const openThread = () => e && openChat(e.id, threadFocus(r.id));
  // The rest of the row: a hand-off's other runs, or a schedule's earlier failures.
  const runsInRow = rowThreads(t);
  const streak = isStreak(t);
  const others = [...new Set(runsInRow.map((x) => x.root.employeeId))].filter((id) => id !== r.employeeId)
    .flatMap((id) => employees.find((x) => x.id === id) ?? []);
  return (
    <article className={`item ${open ? "item-open" : ""}`} onMouseEnter={() => onHover(true)} onMouseLeave={() => onHover(false)}>
      <div
        className="item-head" role="button" tabIndex={0} aria-expanded={open}
        onClick={onToggle}
        onKeyDown={(ev) => { if (ev.target === ev.currentTarget && (ev.key === "Enter" || ev.key === " ")) { ev.preventDefault(); onToggle(); } }}
        onFocus={() => onHover(true)} onBlur={() => onHover(false)}
      >
        {e && others.length ? (
          <span className="item-avs" aria-hidden>
            <Avatar name={e.name} hue={e.hue} size={24} presence={busy ? "working" : undefined} />
            <Avatar name={others[0].name} hue={others[0].hue} size={18} className="item-av2" />
          </span>
        ) : e ? <Avatar name={e.name} hue={e.hue} size={30} presence={busy ? "working" : undefined} /> : <span className="item-noav" />}
        <div className="item-main">
          <span className="item-context">
            <b>{e ? e.name : "Former employee"}</b>
            <span className="muted"> · {context}</span>
          </span>
          {open ? null : <span className="item-title">
            {run.needsYou.length ? <Icon name="bell" size={13} className="item-bell" /> : null}
            {failed ? <Icon name="alert" size={13} className="item-failed" /> : null}
            <span>{title}</span>
          </span>}
          {!open && preview ? <span className="item-preview">{preview}</span> : null}
          {n > 0 || run.needsYou.length > 1 || runsInRow.length > 1 ? (
            <span className="item-tags">
              {streak ? (
                <span className="item-tag-err"><Icon name="alert" size={11} /> Failed {runsInRow.length} times in a row since {timeOrDay(runsInRow[0].root.createdAt)}</span>
              ) : runsInRow.length > 1 ? (
                <span title="Everything this hand-off set off, in one row">{others.length ? `With ${others.map((x) => firstName(x.name)).join(", ")} · ` : ""}{runsInRow.length} runs</span>
              ) : null}
              {n > 0 ? (
                <span className={busy ? "item-tag-live" : ""}>
                  {n} {n === 1 ? "reply" : "replies"} · {busy ? `${who} is replying` : from === "them" && last ? `${who} replied ${relTime(last.endedAt)}` : last ? `you replied ${relTime(last.createdAt)}` : ""}
                </span>
              ) : null}
              {run.needsYou.length > 1 ? <span className="item-tag-warn">{run.needsYou.length - 1} more for you</span> : null}
            </span>
          ) : null}
        </div>
        <div className="item-side">
          <span className="faint item-when" title={dateLong(activityAt(t))}>{relTime(activityAt(t))}</span>
        </div>
        <div className="item-actions" onClick={(ev) => ev.stopPropagation()}>
          <button className="btn btn-ghost btn-sm btn-icon" title={n ? "Open thread" : "Reply"} aria-label={n ? "Open thread" : "Reply"} onClick={() => { if (n) openThread(); else if (!open) onToggle(); }}>
            <Icon name={n ? "chat" : "reply"} size={14} />
          </button>
          <button className="btn btn-ghost btn-sm btn-icon" title="Reopen (E)" aria-label="Reopen" onClick={onReopen}>
            <Icon name="undo" size={14} />
          </button>
        </div>
      </div>
      {open ? <Detail t={t} run={run} e={e} who={who} busy={busy} onReopen={onReopen} onOpenThread={openThread} employees={employees} people={people} /> : null}
    </article>
  );
}

function Detail({ t, run, e, who, busy, onReopen, onOpenThread, employees, people }: {
  t: Thread; run: Run; e: EmployeeView | undefined; who: string; busy: boolean; onReopen: () => void; onOpenThread: () => void;
  employees: EmployeeView[]; people: PersonView[];
}) {
  const r = t.root;
  const n = t.replies.length;
  const failed = didNotFinish(r);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);
  const send = async () => {
    const body = text.trim();
    if (!body || sending) return;
    setSending(true);
    setErr(null);
    try {
      await call("reply_to_run", { runId: r.id, text: body, archive: false });
      setText("");
      toast(`Sent to ${who}. Their answer shows up in the sidebar.`, "ok");
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setSending(false);
    }
  };
  return (
    <div className="item-detail">
      {run.needsYou.length ? (
        <ul className="report-needs" aria-label="Needs you">
          {run.needsYou.map((m, i) => <li key={i}><Icon name="bell" size={12} /><span>{m}</span></li>)}
        </ul>
      ) : null}
      <div className="report-body">
        {r.summary ? <Markdown text={r.summary} /> : <p className="muted">{failed ? r.error ?? "This run didn't finish." : "No report."}</p>}
      </div>
      <div className="item-meta">
        {r.denials.length ? <span className="pill pill-warn"><Icon name="shield" size={11} /> {r.denials.length} blocked</span> : null}
        {r.skillChanges.length ? <SkillsGrew run={r} /> : null}
        <span className="grow" />
        {e ? <button className="btn btn-ghost btn-sm" onClick={onOpenThread}>Open thread <Icon name="arrowUpRight" size={12} /></button> : null}
        <button className="btn btn-ghost btn-sm" onClick={() => openRun(r.id)}>View run <Icon name="arrowRight" size={12} /></button>
      </div>
      {t.related.length ? <RowHistory t={t} employees={employees} people={people} /> : null}
      {n > 0 ? (
        <section className="thread-strip" aria-label="Thread">
          <div className="thread-strip-head">
            <Icon name="chat" size={13} />
            <span className="thread-strip-title">Thread</span>
            <span>· {n} {n === 1 ? "reply" : "replies"}</span>
            {n > 1 ? <button className="btn btn-ghost btn-sm thread-strip-toggle" onClick={() => setShowAll((s) => !s)}>{showAll ? "Show latest" : `Show all ${n}`}</button> : null}
          </div>
          {(showAll ? t.replies : t.replies.slice(-1)).map((x) => <ReplyRow key={x.id} r={x} e={e} who={who} />)}
        </section>
      ) : null}
      <div className="item-reply">
        <textarea
          className="textarea" rows={2} value={text} placeholder={n ? `Reply to ${who} in this thread…` : `Reply to ${who}…`} aria-label={`Reply to ${who}`}
          onChange={(ev) => setText(ev.target.value)}
          onKeyDown={(ev) => { if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) { ev.preventDefault(); void send(); } }}
        />
        {err ? <ErrorNote text={err} /> : null}
        <div className="row">
          <span className="faint item-hint">
            {busy ? `${who} is replying; yours waits its turn.` : "Replying brings it back to the sidebar."}
          </span>
          <span className="grow" />
          <button className="btn btn-ghost btn-sm" onClick={onReopen}>
            <Icon name="undo" size={13} /> Reopen
          </button>
          <button className="btn btn-primary btn-sm" disabled={sending || !text.trim()} onClick={() => void send()} title="Ctrl+Enter">
            <Icon name="send" size={12} /> Send
          </button>
        </div>
      </div>
    </div>
  );
}

/** One exchange in the thread, compact: what you said, then what the employee answered. */
function ReplyRow({ r, e, who }: { r: Run; e: EmployeeView | undefined; who: string }) {
  const live = isLive(r);
  const failed = didNotFinish(r);
  return (
    <div className="thread-reply">
      <div className="thread-reply-you">
        <span className="thread-reply-who">You</span>
        <span className="faint thread-reply-when" title={dateLong(r.createdAt)}>{relTime(r.createdAt)}</span>
        <span className="thread-reply-text">{r.prompt}{r.attachments.length ? ` (${r.attachments.length} ${r.attachments.length === 1 ? "file" : "files"})` : ""}</span>
      </div>
      <div className="thread-reply-them">
        {e ? <Avatar name={e.name} hue={e.hue} size={20} presence={live ? "working" : undefined} /> : <span style={{ width: 20 }} />}
        <div className="thread-reply-body">
          <span className="row" style={{ gap: 8 }}>
            <span className="thread-reply-who">{who}</span>
            {r.endedAt ? <span className="faint thread-reply-when" title={dateLong(r.endedAt)}>{relTime(r.endedAt)}</span> : null}
          </span>
          {live ? (
            <span className="pill pill-live state-pill thread-reply-live"><span className="state-dot state-dot-pulse" />{r.state === "queued" ? "Waiting for a turn" : "Replying"}</span>
          ) : r.summary ? (
            <Markdown text={r.summary} className="thread-reply-md" />
          ) : (
            <span className="muted">{failed ? r.error ?? "This reply didn't finish." : "No reply."}</span>
          )}
          {r.needsYou.length ? (
            <ul className="report-needs" style={{ margin: "4px 0 0" }} aria-label="Needs you">
              {r.needsYou.map((m, i) => <li key={i}><Icon name="bell" size={12} /><span>{m}</span></li>)}
            </ul>
          ) : null}
        </div>
      </div>
    </div>
  );
}

/** Every run filed under this row, oldest first: the rest of a hand-off between employees, or a schedule's
 *  failures in a row. The one shown above is marked; each opens its run. */
export function RowHistory({ t, employees, people }: { t: Thread; employees: EmployeeView[]; people: PersonView[] }) {
  const all = rowThreads(t);
  const streak = isStreak(t);
  const [showAll, setShowAll] = useState(false);
  const shown = showAll || all.length <= 6 ? all : all.slice(-5);
  return (
    <section className="thread-strip" aria-label={streak ? "Failures in a row" : "In this hand-off"}>
      <div className="thread-strip-head">
        <Icon name="layers" size={13} />
        <span className="thread-strip-title">{streak ? "Failures in a row" : "In this hand-off"}</span>
        <span>· {all.length} runs</span>
        {all.length > 6 ? <button className="btn btn-ghost btn-sm thread-strip-toggle" onClick={() => setShowAll((s) => !s)}>{showAll ? "Show latest" : `Show all ${all.length}`}</button> : null}
      </div>
      {shown.map((x) => <HistoryRow key={x.root.id} x={x} lead={x.root.id === t.root.id} employees={employees} people={people} />)}
    </section>
  );
}

function HistoryRow({ x, lead, employees, people }: { x: Thread; lead: boolean; employees: EmployeeView[]; people: PersonView[] }) {
  const r = x.root;
  const e = employees.find((m) => m.id === r.employeeId);
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
          {lead ? <span className="faint"> · shown above</span> : null}
        </span>
        <span className="row-run-text">
          {r.needsYou.length ? <Icon name="bell" size={12} className="item-bell" /> : null}
          {r.denials.length && !r.needsYou.length ? <Icon name="shield" size={12} className="item-bell" /> : null}
          {failed ? <Icon name="alert" size={12} className="item-failed" /> : null}
          <span>{text}</span>
        </span>
      </div>
      <span className="faint row-run-when" title={dateLong(r.endedAt ?? r.createdAt)}>{relTime(r.endedAt ?? r.createdAt)}</span>
      <button className="btn btn-ghost btn-sm btn-icon" title="Open run" aria-label="Open run" onClick={() => openRun(r.id)}><Icon name="arrowRight" size={13} /></button>
    </div>
  );
}
