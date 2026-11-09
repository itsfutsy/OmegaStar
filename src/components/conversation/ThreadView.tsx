import { useEffect, useMemo, useRef, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView, NewAttachment, Run, Thread } from "../../api/types";
import { firstName, relTime, whenShort } from "../../lib/format";
import { dropConversation, refreshConversations, useOrg, type Conversation } from "../../lib/hooks";
import { snoozePresets } from "../../lib/status";
import { toast } from "../../lib/toasts";
import { didNotFinish, flags, isLive, isOpen, isStreak, startedByYou } from "../../lib/threads";
import { CardAim, RunCards } from "./CardView";
import { RowHistory } from "./RowHistory";
import { BlockedCard } from "../BlockedCard";
import { TidyProposal } from "../TidyProposal";
import { Icon } from "../Icon";
import { Markdown } from "../Markdown";
import { Visual } from "../Visual";
import { ErrorNote, Popover, Skeleton } from "../ui";
import { ChatSurface, type ChatSurfaceHandle } from "../panel/ChatSurface";
import { Turn, Waiting, WhoLine } from "../panel/ChatTurn";
import { WorkLine } from "./WorkLine";
import type { LiveRun } from "../../lib/runs";
import "../panel/ChatTab.css";

/** What "Where are we?" sends as your message. */
export const WHERE_ARE_WE = "Where are we? In three short lines: what is done, what is next, and what is waiting on whom.";

/**
 * A thread on one of the employee's reports, or a conversation you started, read as a conversation: the report
 * (or your first message) first, then each reply (and each answer a colleague brought back into it) as a turn, with
 * a composer that answers in it. What you send while the
 * employee works reaches them as they work, or runs as the next reply. Whoever shows it (the conversation
 * screen, an Inbox row) holds the thread's runs (`thread`), and the rest of an Inbox row (`row`) when it leads one.
 * With `recent` it opens on the latest exchange, and the turns before it wait behind "Show N earlier".
 */
export function ThreadView({ e, rootId, thread, row, onSendDone, autoFocus = true, recent = false }: {
  e: EmployeeView;
  rootId: string;
  thread: Conversation;
  /** The thread as the Inbox has it, with the other threads filed under its row in `related`. */
  row: Thread | null;
  /** "Send & done": the reply goes, and the thread comes back only if the answer needs you. Null when done already. */
  onSendDone: ((text: string, attachments: NewAttachment[]) => Promise<void>) | null;
  /** The composer takes focus when the thread appears. */
  autoFocus?: boolean;
  /** Show only the latest exchange until you ask for the rest. */
  recent?: boolean;
}) {
  const { snap } = useOrg();
  const { turns, error, reload, appendRun } = thread;
  const surface = useRef<ChatSurfaceHandle>(null);
  const [all, setAll] = useState(false);
  const [aimed, setAimed] = useState<string | null>(null);
  const aimer = useMemo(() => ({ aim: (t: Parameters<ChatSurfaceHandle["aim"]>[0]) => surface.current?.aim(t), aimed }), [aimed]);
  const root = turns?.[0]?.run;
  const replies = turns?.slice(1) ?? [];
  const live = turns?.find((t) => t.run.state === "running") ?? turns?.find((t) => isLive(t.run));
  const archived = !!root?.archivedAt;
  const task = !!root && startedByYou(root);
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

  if (error) {
    return (
      <div className="thread">
        <div className="thread-error"><ErrorNote title="Couldn't open this conversation" text={reason(error)} onRetry={reload} /></div>
      </div>
    );
  }
  if (!turns || !root) return <ThreadSkeleton />;

  const tail = replies.length ? replies[replies.length - 1].run : root;
  // A reply answers no card, so while one is open, Send & done would leave it behind.
  const carded = turns.some((t) => waiting(t.run));
  const placeholder = archived ? `Reply to ${name}. Replying ${task ? "reopens it" : "brings it back"}.` : `Reply to ${name}`;
  // Not once archived: a hand-off tried again leaves its old run there, and Reopen brings one back to retry.
  const retry = !live && !archived && didNotFinish(tail);
  // A conversation's first message is a turn like the rest; a report opens its thread on its own.
  const shown = task ? turns : replies;
  // The latest exchange starts at the last turn, or the one before it that finished while a newer one is still
  // going, or earlier still at a turn whose card waits on you. -1 is a report thread's report.
  let start = shown.length - 1;
  if (start >= 0 && isLive(shown[start].run)) {
    start--;
    while (start >= 0 && isLive(shown[start].run)) start--;
  }
  const asking = shown.findIndex((t) => waiting(t.run));
  if (asking >= 0) start = Math.min(start, asking);
  if (!task && waiting(root)) start = -1;
  if (!recent || all) start = -1;
  const visible = shown.slice(Math.max(start, 0));
  const showReport = !task && start < 0;
  const earlier = shown.length - visible.length + (task || showReport ? 0 : 1);
  // The latest exchange leads: turns before it step back, unless a card in them still waits on you. Your latest message
  // is the one solid block.
  const lastTurn = visible.length - 1;
  const quiet = (run: Run, i: number) => i < lastTurn && !waiting(run);
  const hasMine = (t: (typeof visible)[number]) => (!!t.run.prompt && t.run.kind.type !== "answered" && !t.run.check) || t.events.some((ev) => ev.type === "user");
  // ...unless a card after it waits on you: then what needs you leads, and your message is the quiet step too.
  const mineAt = visible.map(hasMine).lastIndexOf(true);
  const latestMine = mineAt >= 0 && visible.slice(mineAt).some((t) => waiting(t.run)) ? -1 : mineAt;
  const streak = !!row && isStreak(row);
  // A failure in a streak with no report of its own says nothing a turn could: its steps join the streak's error.
  const folded = streak && !root.summary && didNotFinish(root);

  return (
    <div className="thread">
      <CardAim.Provider value={aimer}>
      <ChatSurface
        ref={surface} e={e} busy={!!live} live={live} draftKey={`thread:${rootId}`} scrollKey={liveKey} onAim={setAimed}
        placeholder={placeholder} cardNote={carded ? "This won't answer the card above." : null} onSend={send} onSendDone={carded ? undefined : onSendDone ?? undefined} onStop={stop} autoFocus={autoFocus}
      >
        <div className="chat-inner">
          {row && row.related.length ? <RowHistory t={row} employees={snap?.employees ?? []} people={snap?.people ?? []} retry={streak && retry && tail.id === root.id ? <RetryButton run={root} name={name} /> : null}
            refs={folded ? <WorkLine turn={turns[0]} e={e} /> : null} /> : null}
          {earlier ? <button className="btn btn-ghost btn-sm thread-earlier" onClick={() => setAll(true)}>Show {earlier} earlier</button> : null}
          {showReport && !folded ? <ReportTurn turn={turns[0]} e={e} blocked={tail.id === root.id && !archived} archived={archived} retry={retry && tail.id === root.id && !streak} streak={streak} earlier={visible.length > 0 && !waiting(root)} /> : null}
          {visible.map((t, i) => (
            <Turn key={t.run.id} turn={t} e={e} withReport earlier={quiet(t.run, i)} latestMine={i === latestMine} footer={(
              <>
                <Flags run={t.run} />
                <RunCards run={t.run} e={e} />
                {i === visible.length - 1 && t.run.denials.length > 0 && t.run.endedAt && !archived ? <BlockedCard run={t.run} employee={e} /> : null}
                {retry && t.run.id === tail.id ? <div className="thread-retry"><RetryButton run={t.run} name={name} /></div> : null}
              </>
            )} />
          ))}
        </div>
      </ChatSurface>
      </CardAim.Provider>
    </div>
  );
}

/** An error as its reason: "Couldn't load this conversation: it is locked" under the title "Couldn't open this
 *  conversation" reads "It is locked". */
const reason = (error: string) => {
  const r = error.replace(/^couldn'?t (?:load|open|read) (?:this|the) conversation:?\s*/i, "").trim();
  return r ? r[0].toUpperCase() + r.slice(1) : error;
};

/** The run has a card still waiting on your answer. News waits on nothing. */
const waiting = (run: Run) => run.cards.some(isOpen);

/** What Omega Star flagged on a run, apart from its cards. */
function Flags({ run }: { run: Run }) {
  // A tidy's bigger changes ask for themselves (TidyProposal), so their flag is not said again.
  const proposing = run.kind.type === "tidy" && !!run.proposed && !run.approvedAt && !run.archivedAt;
  const fs = proposing ? [] : flags(run);
  if (!fs.length) return null;
  return (
    <ul className="flags" aria-label="Needs you">
      {fs.map((m, i) => <li key={i}><Icon name="bell" size={14} /><span>{m}</span></li>)}
    </ul>
  );
}

/** The report that started the thread, as the employee's first message: who and when, its work line, the pages its run
 *  showed, the report, then what it asks you, so you read why before you answer. `retry`: it did not finish and nothing
 *  came after it, so it can be tried again. */
function ReportTurn({ turn, e, blocked, archived, retry, streak = false, earlier = false }: {
  turn: LiveRun; e: EmployeeView; blocked: boolean; archived: boolean; retry: boolean;
  /** A failure in a streak: the streak above says the error once, with Retry. */
  streak?: boolean;
  /** Replies follow it: it steps back so the latest exchange leads. */
  earlier?: boolean;
}) {
  const root = turn.run;
  const failed = didNotFinish(root);
  return (
    <div className={`turn ${earlier ? "turn-earlier" : ""}`}>
      <div className="msg-emp">
        <WhoLine e={e} run={root} label={root.state === "queued" ? "up next" : root.endedAt ? `The report · ${relTime(root.endedAt)}` : isLive(root) ? undefined : "The report"} last={isLive(root)} />
        <div className="msg-body report-body">
          <WorkLine turn={turn} e={e} />
          {turn.events.map((ev, i) => ev.type === "visual" ? <Visual key={i} runId={root.id} id={ev.id} title={ev.title} from={firstName(e.name)} /> : null)}
          {root.summary ? <Markdown text={root.summary} />
            : root.state === "queued" ? <><p className="task-ask">{root.prompt}</p><Waiting run={root} e={e} /></>
            : root.state === "running" ? <p className="task-ask">{root.prompt}</p>
            : failed ? (streak ? null : <p className="msg-failed"><Icon name="alert" size={14} /><span>{root.error ?? "This run didn't finish."}</span></p>)
            : <p className="muted">No report.</p>}
          <Flags run={root} />
          <RunCards run={root} e={e} />
          {retry ? <div className="thread-retry"><RetryButton run={root} name={firstName(e.name)} /></div> : null}
          {blocked && root.denials.length > 0 ? <BlockedCard run={root} employee={e} /> : null}
          {!archived ? <TidyProposal run={root} employee={e} /> : null}
        </div>
      </div>
    </div>
  );
}

/** A conversation loading: its own shape (your message, a reply, another message), holding still. */
export function ThreadSkeleton() {
  return (
    <div className="thread thread-wait" aria-busy="true" aria-label="Loading the conversation">
      <div className="chat-inner thread-skel">
        <span className="thread-skel-me"><Skeleton h={42} r={6} /></span>
        <div className="stack" style={{ gap: 10 }}>
          <div className="row" style={{ gap: 9 }}><Skeleton h={22} w={22} r={11} /><Skeleton h={12} w={90} /></div>
          <Skeleton h={13} w="92%" /><Skeleton h={13} w="86%" /><Skeleton h={13} w="54%" />
          <div className="row" style={{ gap: 6, marginTop: 4 }}><Skeleton h={28} w={104} r={5} /><Skeleton h={28} w={92} r={5} /></div>
        </div>
        <span className="thread-skel-me" style={{ width: "34%" }}><Skeleton h={42} r={6} /></span>
      </div>
      <div className="composer-wrap"><div className="composer thread-skel-composer" /></div>
    </div>
  );
}

/** Archives the threads with `rootIds`, each with the rest of its row, except what still needs you. They leave at
 *  once, with nothing to confirm; when related runs or several threads went, it says how many, and Undo brings back
 *  exactly what it filed. Resolves false, having said why and put back what it could not file, when any failed. */
export async function markDone(...rootIds: string[]): Promise<boolean> {
  rootIds.forEach(dropConversation);
  const filed: string[] = [];
  let ok = true;
  for (const runId of rootIds) {
    try {
      filed.push(...(await call("set_archived", { runId, archived: true })));
    } catch (err) {
      ok = false;
      toast(errorMessage(err), "err");
    }
  }
  if (!ok) refreshConversations();
  const went = rootIds.filter((id) => filed.includes(id)).length;
  const also = filed.length - went;
  if (also || went > 1) {
    const undo = () => void call("unarchive_runs", { runIds: filed }).catch((err) => toast(errorMessage(err), "err"));
    const archived = went > 1 ? `Archived ${went} runs` : "Archived";
    toast(also ? `${archived}. Also filed ${also} related ${also === 1 ? "run" : "runs"}` : archived, "ok", { label: "Undo", run: undo });
  }
  return ok;
}

/** Tries a run that did not finish again: it carries on from where it stopped, or a hand-off runs again for whoever
 *  handed it over. It stays disabled once it went, while the retry takes over. */
function RetryButton({ run, name }: { run: Run; name: string }) {
  const [busy, setBusy] = useState(false);
  const retry = async () => {
    setBusy(true);
    try {
      await call("retry_run", { runId: run.id });
      toast(`${name} is trying again`, "ok");
    } catch (err) {
      toast(errorMessage(err), "err");
      setBusy(false);
    }
  };
  return (
    <button className="btn btn-sm" disabled={busy} title="Try it again from where it stopped" onClick={() => void retry()}>
      <Icon name="refresh" size={13} /> Retry
    </button>
  );
}

/** Brings a thread you archived back, and offers Undo. */
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

/** Later puts the thread away until a time, then brings it back to the Inbox as unread. While it is away
 *  (`snoozedUntil`), Wake now. Its menu takes focus as it opens, so keys work in it. */
export function SnoozeButton({ snoozedUntil, onSnooze, onWake }: { snoozedUntil: string | null; onSnooze: (until: Date) => void; onWake: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (open) requestAnimationFrame(() => menu.current?.querySelector<HTMLButtonElement>("button")?.focus());
  }, [open]);
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
      <button ref={ref} className="btn btn-ghost btn-sm" data-later aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <Icon name="clock" size={13} /> Later
      </button>
      <Popover anchor={ref} open={open} onClose={() => setOpen(false)} width={232} align="end">
        <div ref={menu} className="menu" role="menu" aria-label="Bring it back" data-esc-stop>
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
