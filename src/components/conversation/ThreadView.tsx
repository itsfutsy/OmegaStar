import { useRef, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView, NewAttachment, Run, Thread } from "../../api/types";
import { dateLong, firstName, relTime, whenShort } from "../../lib/format";
import { useOrg, type Conversation } from "../../lib/hooks";
import { snoozePresets } from "../../lib/status";
import { toast } from "../../lib/toasts";
import { didNotFinish, isLive } from "../../lib/threads";
import { answerCard, CardView } from "./CardView";
import { RowHistory } from "./RowHistory";
import { BlockedCard } from "../BlockedCard";
import { TidyProposal } from "../TidyProposal";
import { Icon } from "../Icon";
import { Markdown } from "../Markdown";
import { Avatar, ErrorNote, Popover, Skeleton, SkillsGrew } from "../ui";
import { ChatSurface, type ChatSurfaceHandle } from "../panel/ChatSurface";
import { Turn } from "../panel/ChatTurn";
import "../panel/ChatTab.css";
import "../../screens/Inbox.css";

const TEACH_PREFIX = "I want to teach you a new task: ";

/**
 * A thread on one of the employee's reports, or a task, read as a conversation: the report (or your first
 * message) first, then each reply as a turn, with a composer that answers in it. What you send while the
 * employee works reaches them as they work, or runs as the next reply. Whoever shows it (the conversation
 * screen, an Inbox row) holds the thread's runs (`thread`), and the rest of an Inbox row (`row`) when it leads one.
 */
export function ThreadView({ e, rootId, thread, row, onSendDone, autoFocus = true }: {
  e: EmployeeView;
  rootId: string;
  thread: Conversation;
  /** The thread as the Inbox has it, with the other threads filed under its row in `related`. */
  row: Thread | null;
  /** "Send & done": the reply goes, and the thread comes back only if the answer needs you. Null when done already. */
  onSendDone: ((text: string, attachments: NewAttachment[]) => Promise<void>) | null;
  /** The composer takes focus when the thread appears. */
  autoFocus?: boolean;
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
        placeholder={`Reply to ${name} in this ${place}…`} hint={hint} onSend={send} onSendDone={onSendDone ?? undefined} onStop={stop} autoFocus={autoFocus}
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
              {t.run.card ? <CardView run={t.run} e={e} onAnswer={async (reply) => { await answerCard(t.run, reply, name); }} /> : null}
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
          {root.card ? <CardView run={root} e={e} onAnswer={async (reply) => { await answerCard(root, reply, firstName(e.name)); }} /> : null}
          {root.summary ? <Markdown text={root.summary} /> : <p className="muted">{failed ? root.error ?? "This run didn't finish." : "No report."}</p>}
          {root.skillChanges.length ? <SkillsGrew run={root} /> : null}
          {blocked && root.denials.length > 0 ? <BlockedCard run={root} employee={e} /> : null}
          {!archived ? <TidyProposal run={root} employee={e} /> : null}
        </div>
      </div>
    </div>
  );
}

/** Marks the thread with `rootId` done, which archives it, and offers Undo. Resolves false, having said why, when it failed. */
export async function markDone(rootId: string, _name: string): Promise<boolean> {
  try {
    await call("set_archived", { runId: rootId, archived: true });
  } catch (err) {
    toast(errorMessage(err), "err");
    return false;
  }
  const undo = () => void call("set_archived", { runId: rootId, archived: false }).catch((err) => toast(errorMessage(err), "err"));
  toast("Done", "ok", { label: "Undo", run: undo });
  return true;
}

/** Brings a thread you marked done back, and offers Undo. */
export async function reopen(rootId: string) {
  try {
    await call("set_archived", { runId: rootId, archived: false });
  } catch (err) {
    toast(errorMessage(err), "err");
    return;
  }
  const undo = () => void call("set_archived", { runId: rootId, archived: true }).catch((err) => toast(errorMessage(err), "err"));
  toast("Reopened", "ok", { label: "Undo", run: undo });
}

/** "Send & done": the reply goes with the thread marked done, and it comes back only if the answer needs you. */
export async function sendAndDone(rootId: string, name: string, text: string, attachments: NewAttachment[]) {
  await call("reply_to_run", { runId: rootId, text, archive: true, attachments });
  toast(`Sent to ${name}. It comes back only if the answer needs you.`, "ok");
}

/** Done is the archive: the thread leaves the Inbox and the employee's chips, and comes back if the employee raises something new. */
export function DoneButton({ name, disabled, onDone }: {
  /** The employee's first name. */
  name: string;
  task: boolean;
  /** The thread is still working, so it cannot be done yet. */
  disabled: boolean;
  onDone: () => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      className="btn btn-sm" disabled={disabled || busy} title={disabled ? `${name} is still working on it` : "Done (E)"}
      onClick={async () => { setBusy(true); try { await onDone(); } finally { setBusy(false); } }}
    >
      <Icon name="check" size={13} /> Done <kbd className="kbd">E</kbd>
    </button>
  );
}

/** Snooze puts the thread away until a time, then brings it back to the Inbox as unread. While it is snoozed
 *  (`snoozedUntil`), Wake now. */
export function SnoozeButton({ snoozedUntil, onSnooze, onWake }: { snoozedUntil: string | null; onSnooze: (until: Date) => void; onWake: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  if (snoozedUntil) {
    return (
      <button className="btn btn-ghost btn-sm" onClick={onWake}>
        <Icon name="clock" size={13} /> Wake now
      </button>
    );
  }
  const presets = open ? snoozePresets(new Date()) : [];
  return (
    <>
      <button ref={ref} className="btn btn-ghost btn-sm" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <Icon name="clock" size={13} /> Snooze
      </button>
      <Popover anchor={ref} open={open} onClose={() => setOpen(false)} width={232}>
        <div className="menu" role="menu" aria-label="Snooze until" data-esc-stop>
          {presets.map((s) => (
            <button key={s.label} type="button" role="menuitem" className="menu-item" onClick={() => { setOpen(false); onSnooze(s.until); }}>
              <span className="grow">{s.label}</span>
              <span className="faint">{whenShort(s.until.toISOString()).replace(s.label === "Tomorrow" ? /^Tomorrow / : /^Today /, "")}</span>
            </button>
          ))}
        </div>
      </Popover>
    </>
  );
}
