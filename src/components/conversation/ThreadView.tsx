import { useRef, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView, NewAttachment, Run, Thread } from "../../api/types";
import { dateLong, firstName, relTime } from "../../lib/format";
import { useOrg, type Conversation } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import { didNotFinish, isLive } from "../../lib/threads";
import { RowHistory } from "../../screens/InboxItem";
import { BlockedCard } from "../BlockedCard";
import { TidyProposal } from "../TidyProposal";
import { Icon } from "../Icon";
import { Markdown } from "../Markdown";
import { Avatar, ErrorNote, Popover, Skeleton, SkillsGrew, Switch } from "../ui";
import { ChatSurface, type ChatSurfaceHandle } from "../panel/ChatSurface";
import { Turn } from "../panel/ChatTurn";
import "../panel/ChatTab.css";
import "../../screens/Inbox.css";

const TEACH_PREFIX = "I want to teach you a new task: ";

/**
 * A thread on one of the employee's reports, or a task, read as a conversation: the report (or your first
 * message) first, then each reply as a turn, with a composer that answers in it. What you send while the
 * employee works reaches them as they work, or runs as the next reply. The conversation screen holds the
 * thread's runs (`thread`) for its header, and the rest of an Inbox row (`row`) when the thread leads one.
 */
export function ThreadView({ e, rootId, thread, row, onSendDone }: {
  e: EmployeeView;
  rootId: string;
  thread: Conversation;
  /** The thread as the sidebar has it, with the other threads filed under its Inbox row in `related`. */
  row: Thread | null;
  /** "Send & done": the reply goes, and the thread comes back only if the answer needs you. Null when done already. */
  onSendDone: ((text: string, attachments: NewAttachment[]) => Promise<void>) | null;
}) {
  const { snap } = useOrg();
  const { turns, error, reload, appendRun } = thread;
  const surface = useRef<ChatSurfaceHandle>(null);
  const root = turns?.[0]?.run;
  const replies = turns?.slice(1) ?? [];
  const live = turns?.find((t) => t.run.state === "running") ?? turns?.find((t) => isLive(t.run));
  const archived = !!root?.archivedAt;
  const task = root?.kind.type === "task";
  const name = firstName(e.name);

  const liveKey = turns ? `${turns.length}:${turns[turns.length - 1]?.events.length}:${turns[turns.length - 1]?.live.length}` : "";

  const send = async (text: string, attachments: NewAttachment[]) => {
    const run = await call("reply_to_run", { runId: rootId, text, archive: false, attachments });
    appendRun(run);
  };
  const stop = async () => {
    if (!live) return;
    try { await call("cancel_run", { runId: live.run.id }); } catch (err) { toast(errorMessage(err), "err"); }
  };

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

  const tail = replies.length ? replies[replies.length - 1].run : root;
  const place = task ? "task" : "thread";
  const hint = archived ? (task ? "Replying reopens it" : "Replying brings it back") : `Stays in this ${place} · Shift+Enter for a new line`;
  // A task's first message is a turn like the rest; a report opens its thread on its own.
  const shown = task ? turns : replies;

  return (
    <div className="thread">
      <ChatSurface
        ref={surface} e={e} busy={!!live} live={live} draftKey={`thread:${rootId}`} scrollKey={liveKey}
        placeholder={`Reply to ${name} in this ${place}…`} hint={hint} onSend={send} onSendDone={onSendDone ?? undefined} onStop={stop}
        actions={live ? null : <button className="btn btn-ghost btn-sm" onClick={() => surface.current?.prefill(TEACH_PREFIX)}><Icon name="teach" size={13} /> Teach a new task</button>}
      >
        <div className="chat-inner">
          {row && row.related.length ? <RowHistory t={row} employees={snap?.employees ?? []} people={snap?.people ?? []} /> : null}
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

/** Done, with one choice: whether the employee's ongoing chat hears what the thread or task decided. Done is the
 *  archive: the thread leaves the sidebar after today, and comes back if the employee raises something new. */
export function DoneButton({ name, task, replies, disabled, onDone }: {
  /** The employee's first name. */
  name: string;
  task: boolean;
  /** How many replies the thread has: a report nobody replied to has nothing to brief. */
  replies: number;
  /** The thread is still working, so it cannot be done yet. */
  disabled: boolean;
  onDone: (brief: boolean) => Promise<void>;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [brief, setBrief] = useState(true);
  const [busy, setBusy] = useState(false);
  return (
    <>
      <button ref={ref} className="btn btn-sm" onClick={() => setOpen((o) => !o)} aria-expanded={open} disabled={disabled} title={disabled ? `${name} is still working on it` : "Done (E)"}>
        <Icon name="check" size={13} /> Done <kbd className="kbd">E</kbd>
      </button>
      <Popover anchor={ref} open={open} onClose={() => setOpen(false)} width={360}>
        <div className="stack" style={{ gap: 12 }} data-esc-stop>
          <div className="stack" style={{ gap: 3 }}>
            <span className="h-md">Done with this {task ? "task" : "thread"}?</span>
            <span className="muted" style={{ fontSize: 12.5, lineHeight: 1.5 }}>
              {task
                ? `It moves to Done today, then out of the sidebar. You can still find it in ${name}'s Work tab.`
                : `It moves to Done today, then out of the sidebar. You can still find it under Archive in your Inbox, and in ${name}'s Work tab.`}
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
              onClick={async () => { setBusy(true); try { await onDone(brief && (task || replies > 0)); } finally { setBusy(false); setOpen(false); } }}
            >
              <Icon name="check" size={13} /> Mark done
            </button>
          </div>
        </div>
      </Popover>
    </>
  );
}
