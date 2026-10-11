import { useEffect, useRef, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView, NewAttachment, Run } from "../../api/types";
import { dateLong, firstName, relTime, runKindLabel, whenInline } from "../../lib/format";
import { useOrg, useThread } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import { openMember, openRun } from "../../lib/router";
import { didNotFinish, isLive, threadFocus } from "../../lib/threads";
import { kindIcon, kindNames } from "../RunRow";
import { BlockedCard } from "../BlockedCard";
import { TidyProposal } from "../TidyProposal";
import { Icon } from "../Icon";
import { Markdown } from "../Markdown";
import { Avatar, ErrorNote, Popover, Skeleton, SkillsGrew, Switch } from "../ui";
import { ChatSurface, type ChatSurfaceHandle } from "./ChatSurface";
import { Turn } from "./ChatTurn";
import "./ChatTab.css";
import "../../screens/Inbox.css";

const TEACH_PREFIX = "I want to teach you a new task: ";
const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

/** Starts a task and opens it, so you see it start. `from`: the thread (any run in it) it starts from;
 *  without it, the ongoing chat. */
export async function startTask(e: EmployeeView, text: string, attachments: NewAttachment[], from: string | null) {
  const run = await call("start_task", { employeeId: e.id, text, attachments, from });
  openMember(e.id, "chat", threadFocus(run.id), true);
}

/**
 * A thread on one of the employee's reports, or a task, read as a conversation: the report (or your first
 * message) first, then each reply as a turn, with a composer that answers in it. A report's thread also sits
 * in your Inbox; archiving it there or here ends it in both places. What you send while the employee works
 * reaches them as they work, or runs as the next reply.
 */
export function ThreadView({ e, rootId, onRoot, onArchived }: { e: EmployeeView; rootId: string; onRoot: (root: Run) => void; onArchived: () => void }) {
  const { snap } = useOrg();
  const { turns, error, reload, appendRun } = useThread(rootId);
  const surface = useRef<ChatSurfaceHandle>(null);
  const root = turns?.[0]?.run;
  const replies = turns?.slice(1) ?? [];
  const live = turns?.find((t) => t.run.state === "running") ?? turns?.find((t) => isLive(t.run));
  const archived = !!root?.archivedAt;
  const task = root?.kind.type === "task";
  const name = firstName(e.name);

  useEffect(() => { if (root) onRoot(root); }, [root, onRoot]);

  const liveKey = turns ? `${turns.length}:${turns[turns.length - 1]?.events.length}:${turns[turns.length - 1]?.live.length}` : "";

  const send = async (text: string, attachments: NewAttachment[]) => {
    const run = await call("reply_to_run", { runId: rootId, text, archive: false, attachments });
    appendRun(run);
  };
  const stop = async () => {
    if (!live) return;
    try { await call("cancel_run", { runId: live.run.id }); } catch (err) { toast(errorMessage(err), "err"); }
  };
  const archive = async (brief: boolean) => {
    try {
      await call("set_archived", { runId: rootId, archived: true, brief });
      toast(brief && (task || replies.length) ? `Archived. ${name} hears what was decided in their ongoing chat.` : "Archived", "ok");
      onArchived();
    } catch (err) {
      toast(errorMessage(err), "err");
    }
  };
  const unarchive = async () => {
    try { await call("set_archived", { runId: rootId, archived: false }); toast(task ? "Reopened" : "Moved to Inbox", "ok"); } catch (err) { toast(errorMessage(err), "err"); }
  };

  // E archives, with the brief on, as in the Inbox.
  const archiveRef = useRef(archive);
  archiveRef.current = archive;
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.ctrlKey || ev.metaKey || ev.altKey || typing(ev.target) || document.querySelector(".popover")) return;
      if (ev.key.toLowerCase() !== "e" || archived || !root?.endedAt) return;
      ev.preventDefault();
      void archiveRef.current(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [archived, root]);

  if (error) return <div style={{ padding: 20 }}><ErrorNote text={error} onRetry={reload} /></div>;
  if (!turns || !root) {
    return (
      <div className="thread">
        <div className="chat-inner">
          <Skeleton h={44} r={10} />
          <div className="stack" style={{ gap: 8 }}><Skeleton w="90%" /><Skeleton w="75%" /><Skeleton w="40%" /></div>
        </div>
      </div>
    );
  }

  const label = runKindLabel(root.kind, kindNames(root, snap?.employees ?? [], snap?.people ?? []));
  const tail = replies.length ? replies[replies.length - 1].run : root;
  const place = task ? "task" : "thread";
  const hint = archived ? (task ? "Replying reopens it" : "Replying brings it back to your Inbox") : `Stays in this ${place} · Shift+Enter for a new line`;
  const from = root.kind.type === "task" ? root.kind.from : null;
  // A task's first message is a turn like the rest; a report opens its thread on its own.
  const shown = task ? turns : replies;

  return (
    <div className="thread">
      <div className="thread-anchor">
        <span className="thread-anchor-icon"><Icon name={kindIcon(root)} size={14} /></span>
        <div className="stack grow" style={{ minWidth: 0 }}>
          <span className="thread-anchor-title">{task ? "Task" : "Thread on a report"} · {label}</span>
          <span className="thread-anchor-sub muted">
            {task
              ? archived
                ? `Archived ${relTime(root.archivedAt)} · in ${name}'s Work tab`
                : from ? `Started from ${from} ${whenInline(root.createdAt)} · knows the context` : `Started ${whenInline(root.createdAt)}`
              : archived ? `Archived ${relTime(root.archivedAt)} · under Archive in your Inbox` : `${dateLong(root.endedAt ?? root.createdAt)} · also in your Inbox`}
          </span>
        </div>
        {task ? null : <button className="btn btn-ghost btn-sm" onClick={() => openRun(root.id)}>Open run <Icon name="arrowUpRight" size={12} /></button>}
        {archived ? (
          <button className="btn btn-sm" onClick={() => void unarchive()}><Icon name="undo" size={13} /> {task ? "Reopen" : "Move to Inbox"}</button>
        ) : (
          <ArchiveButton name={name} task={task} replies={replies.length} disabled={!root.endedAt} onArchive={archive} />
        )}
      </div>
      <ChatSurface
        ref={surface} e={e} busy={!!live} live={live} draftKey={`thread:${rootId}`} scrollKey={liveKey}
        placeholder={`Reply to ${name} in this ${place}…`} hint={hint} onSend={send} onStop={stop}
        actions={live ? null : <button className="btn btn-ghost btn-sm" onClick={() => surface.current?.prefill(TEACH_PREFIX)}><Icon name="teach" size={13} /> Teach a new task</button>}
      >
        <div className="chat-inner">
          {task ? null : <ReportTurn root={root} e={e} blocked={tail.id === root.id && !archived} archived={archived} />}
          {shown.map((t, i) => (
            <div key={t.run.id} className="stack" style={{ gap: 10 }}>
              <Turn turn={t} e={e} withReport />
              {t.run.needsYou.length ? (
                <ul className="report-needs thread-needs" aria-label="Needs you">
                  {t.run.needsYou.map((m, j) => <li key={j}><Icon name="bell" size={12} /><span>{m}</span></li>)}
                </ul>
              ) : null}
              {i === shown.length - 1 && t.run.denials.length > 0 && t.run.endedAt && !archived ? <BlockedCard run={t.run} employee={e} /> : null}
            </div>
          ))}
        </div>
      </ChatSurface>
    </div>
  );
}

/** The report that started the thread, as the employee's first message. */
function ReportTurn({ root, e, blocked, archived }: { root: Run; e: EmployeeView; blocked: boolean; archived: boolean }) {
  const failed = didNotFinish(root);
  return (
    <div className="turn">
      <div className="msg-emp">
        <Avatar name={e.name} hue={e.hue} size={24} presence={isLive(root) ? "working" : undefined} className="msg-av" />
        <div className="msg-body">
          <div className="row" style={{ gap: 8 }}>
            <span className="msg-label">The report</span>
            <span className="grow" />
            {root.endedAt ? <span className="faint thread-time" title={dateLong(root.endedAt)}>{relTime(root.endedAt)}</span> : null}
          </div>
          {root.needsYou.length ? (
            <ul className="report-needs thread-needs" aria-label="Needs you">
              {root.needsYou.map((m, i) => <li key={i}><Icon name="bell" size={12} /><span>{m}</span></li>)}
            </ul>
          ) : null}
          {root.summary ? <Markdown text={root.summary} /> : <p className="muted">{failed ? root.error ?? "This run didn't finish." : "No report."}</p>}
          {root.skillChanges.length ? <SkillsGrew run={root} /> : null}
          {blocked && root.denials.length > 0 ? <BlockedCard run={root} employee={e} /> : null}
          {!archived ? <TidyProposal run={root} employee={e} /> : null}
        </div>
      </div>
    </div>
  );
}

/** Archive, with one choice: whether the employee's ongoing chat hears what the thread or task decided. */
function ArchiveButton({ name, task, replies, disabled, onArchive }: { name: string; task: boolean; replies: number; disabled: boolean; onArchive: (brief: boolean) => Promise<void> }) {
  const ref = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [brief, setBrief] = useState(true);
  const [busy, setBusy] = useState(false);
  return (
    <>
      <button ref={ref} className="btn btn-sm" onClick={() => setOpen((o) => !o)} aria-expanded={open} disabled={disabled} title={disabled ? `${name} is still working on it` : undefined}>
        <Icon name="archive" size={13} /> Archive <kbd className="kbd">E</kbd>
      </button>
      <Popover anchor={ref} open={open} onClose={() => setOpen(false)} width={360}>
        <div className="stack" style={{ gap: 12 }} data-esc-stop>
          <div className="stack" style={{ gap: 3 }}>
            <span className="h-md">Done with this {task ? "task" : "thread"}?</span>
            <span className="muted" style={{ fontSize: 12.5, lineHeight: 1.5 }}>
              {task ? `It leaves this tab. You can still find it in ${name}'s Work tab.` : `It leaves the Inbox and this tab. You can still find it under Archive, and in ${name}'s Work tab.`}
            </span>
          </div>
          {task || replies > 0 ? (
            <div className="thread-brief-opt">
              <Switch checked={brief} onChange={setBrief} label={`Brief ${name}'s ongoing chat`} />
              <span className="stack" style={{ gap: 1 }}>
                <span style={{ fontWeight: 500 }}>Brief {name}'s ongoing chat</span>
                <span className="muted">What was said here, read at the start of their next turn there.</span>
              </span>
            </div>
          ) : null}
          <div className="row">
            <span className="grow" />
            <button className="btn btn-ghost btn-sm" onClick={() => setOpen(false)}>Keep</button>
            <button
              className="btn btn-primary btn-sm" disabled={busy}
              onClick={async () => { setBusy(true); try { await onArchive(brief && (task || replies > 0)); } finally { setBusy(false); setOpen(false); } }}
            >
              <Icon name="archive" size={13} /> Archive {task ? "task" : "thread"}
            </button>
          </div>
        </div>
      </Popover>
    </>
  );
}

