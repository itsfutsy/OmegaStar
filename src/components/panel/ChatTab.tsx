import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { Brief, EmployeeView, NewAttachment, Run, Thread } from "../../api/types";
import { dateLong, firstName, relTime, runKindLabel } from "../../lib/format";
import { useChat, useOrg, useQuery, type Conversation } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import type { LiveRun } from "../../lib/runs";
import { openMember } from "../../lib/router";
import { isLive, lastFrom, NEW_TASK_FOCUS, parseThreadFocus, threadFocus } from "../../lib/threads";
import { kindIcon, kindNames } from "../RunRow";
import { Icon } from "../Icon";
import { Avatar, Skeleton } from "../ui";
import { ChatSurface, type ChatSurfaceHandle } from "./ChatSurface";
import { Turn } from "./ChatTurn";
import { ThreadView } from "./ThreadView";
import "./ChatTab.css";

const TEACH_PREFIX = "I want to teach you a new task: ";

/**
 * Conversations with an employee: the ongoing chat, and beside it each task you started and a thread for
 * each report you replied to. They all run at the same time. The route's focus (`thread~<root run id>`, or
 * `new-task`) says which one is open, so a link from the Inbox lands here.
 */
export function ChatTab({ e, focus }: { e: EmployeeView; focus: string | null }) {
  const threads = useQuery(() => call("threads", { employeeId: e.id }), [e.id], ["run:updated"]);
  const chat = useChat(e.id);
  const selected = parseThreadFocus(focus);
  const newTask = focus === NEW_TASK_FOCUS;
  const onOngoing = !selected && !newTask;
  // A thread opened before it has replies (or after it was archived) is not in the list; its chip comes from the view.
  const [extra, setExtra] = useState<Run | null>(null);
  const list = threads.data ?? [];
  const shown: Thread[] = selected && extra && extra.id === selected && !list.some((t) => t.root.id === selected) ? [...list, { root: extra, replies: [], related: [] }] : list;
  const select = useCallback((focus: string | null) => openMember(e.id, "chat", focus, true), [e.id]);

  const chatBusy = !!chat.turns?.some((t) => isLive(t.run));
  // When the ongoing chat last answered, and when you last looked at it: a reply that lands while you are
  // in another conversation marks its chip.
  const answered = chat.turns ? chat.turns.reduce((at, t) => (t.run.endedAt && t.run.endedAt > at ? t.run.endedAt : at), "") : null;
  const [seen, setSeen] = useState<string | null>(null);
  if (answered !== null && (seen === null || (onOngoing && seen !== answered))) setSeen(answered);
  const chatNew = !onOngoing && seen !== null && answered !== null && answered > seen;

  return (
    <div className="chat-host">
      <div className="thread-switch" role="group" aria-label={`Conversations with ${firstName(e.name)}`}>
        <button className="chip thread-chip" aria-pressed={onOngoing} onClick={() => select(null)}>
          <Icon name="chat" size={13} /> Ongoing
          {chatBusy ? <span className="thread-chip-dot thread-chip-dot-live" aria-label="Replying" /> : chatNew ? <span className="thread-chip-dot" aria-label="New reply" /> : null}
        </button>
        {shown.map((t) => <ThreadChip key={t.root.id} t={t} on={t.root.id === selected} onClick={() => select(threadFocus(t.root.id))} />)}
        <button className="chip chip-new" aria-pressed={newTask} aria-label="New task" title="New task, alongside everything else" onClick={() => select(NEW_TASK_FOCUS)}>
          <Icon name="plus" size={13} />
        </button>
      </div>
      {selected ? (
        <ThreadView key={selected} e={e} rootId={selected} onRoot={setExtra} onArchived={() => select(null)} />
      ) : newTask ? (
        <NewTask e={e} />
      ) : (
        <OngoingChat e={e} chat={chat} />
      )}
    </div>
  );
}

function ThreadChip({ t, on, onClick }: { t: Thread; on: boolean; onClick: () => void }) {
  const { snap } = useOrg();
  const label = runKindLabel(t.root.kind, kindNames(t.root, snap?.employees ?? [], snap?.people ?? []));
  const live = [t.root, ...t.replies].filter(isLive);
  // Everything still to come is queued: it waits for a free slot.
  const waiting = live.length > 0 && live.every((r) => r.state === "queued");
  const theirs = lastFrom(t) === "them";
  const title = t.root.archivedAt ? `${label} (archived)` : waiting ? `${label} (waiting for a free slot)` : label;
  // The open conversation stays in sight when the chips scroll.
  const ref = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (on) ref.current?.scrollIntoView({ block: "nearest" }); }, [on]);
  return (
    <button ref={ref} className={`chip thread-chip ${waiting ? "task-chip-wait" : ""}`} aria-pressed={on} onClick={onClick} title={title}>
      <Icon name={t.root.archivedAt ? "archive" : kindIcon(t.root)} size={13} />
      <span className="thread-chip-label">{label}</span>
      {t.replies.length ? <span className="thread-chip-n num">{t.replies.length}</span> : null}
      {waiting ? <span className="thread-chip-dot" aria-label="Waiting to start" />
        : live.length ? <span className="thread-chip-dot thread-chip-dot-live" aria-label="Replying" />
        : theirs && !on ? <span className="thread-chip-dot" aria-label="New reply" /> : null}
    </button>
  );
}

/** A task before its first message: what you send starts it, beside everything else, and opens it. */
function NewTask({ e }: { e: EmployeeView }) {
  const name = firstName(e.name);
  const start = async (text: string, attachments: NewAttachment[]) => {
    const run = await call("start_task", { employeeId: e.id, text, attachments });
    openMember(e.id, "chat", threadFocus(run.id), true);
  };
  return (
    <ChatSurface
      e={e} busy={false} draftKey={`new-task:${e.id}`} scrollKey="" startsTask
      placeholder={`What should ${name} work on?`} onSend={start}
    >
      <div className="task-empty">
        <span className="task-empty-icon"><Icon name="branch" size={22} /></span>
        <h3 className="h-md">Give {name} something else to work on</h3>
        <p className="muted">
          A task is its own conversation. It runs alongside everything else {name} is doing, and starts from what you've discussed in your ongoing chat.
        </p>
      </div>
    </ChatSurface>
  );
}

type Item = { kind: "turn"; at: string; turn: LiveRun } | { kind: "brief"; at: string; brief: Brief };

/** The long-lived conversation, with a card for each thread or task you archived since it carried on. What
 *  you send while a turn works reaches it there, or runs as the next turn. */
function OngoingChat({ e, chat }: { e: EmployeeView; chat: Conversation }) {
  const { turns, error, reload, appendRun } = chat;
  const briefs = useQuery(() => call("chat_briefs", { employeeId: e.id }), [e.id], ["run:updated", "org:changed"]);
  const surface = useRef<ChatSurfaceHandle>(null);
  const name = firstName(e.name);
  const live = turns?.find((t) => t.run.state === "running") ?? turns?.find((t) => t.run.state === "queued");
  const busy = !!live;
  const liveKey = turns ? `${turns.length}:${turns[turns.length - 1]?.events.length}:${turns[turns.length - 1]?.live.length}` : "";
  const items = useMemo<Item[]>(() => {
    const out: Item[] = [
      ...(turns ?? []).map((turn): Item => ({ kind: "turn", at: turn.run.createdAt, turn })),
      ...(briefs.data ?? []).map((brief): Item => ({ kind: "brief", at: brief.createdAt, brief })),
    ];
    return out.sort((a, b) => a.at.localeCompare(b.at));
  }, [turns, briefs.data]);

  const send = async (text: string, attachments: NewAttachment[]) => {
    const run = await call("send_chat", { employeeId: e.id, text, attachments });
    appendRun(run);
  };
  const stop = async () => {
    if (!live) return;
    try { await call("cancel_run", { runId: live.run.id }); } catch (err) { toast(errorMessage(err), "err"); }
  };
  const newConversation = async () => {
    try {
      await call("new_chat", { employeeId: e.id });
      reload();
      briefs.reload();
      toast(`Fresh conversation. Everything ${name} learned stays in their skills.`, "ok");
    } catch (err) {
      toast(errorMessage(err), "err");
    }
  };
  const teach = () => surface.current?.prefill(TEACH_PREFIX);
  const empty = turns && turns.length === 0 && (briefs.data?.length ?? 0) === 0;

  return (
    <ChatSurface
      ref={surface} e={e} busy={busy} live={live} draftKey={e.id} scrollKey={`${liveKey}:${briefs.data?.length ?? 0}`}
      placeholder={`Message ${name}. Teach them something, or ask.`} onSend={send} onStop={stop}
      actions={busy ? null : (
        <>
          <button className="btn btn-ghost btn-sm" onClick={teach}><Icon name="teach" size={13} /> Teach a new task</button>
          <button className="btn btn-ghost btn-sm" onClick={() => void newConversation()} disabled={!turns?.length}><Icon name="newChat" size={13} /> New conversation</button>
        </>
      )}
    >
      {error ? <div style={{ padding: 20 }}><ErrorBlock text={error} onRetry={reload} /></div> : null}
      {!turns && !error ? (
        <div className="chat-inner">
          <div className="stack" style={{ gap: 10, alignItems: "flex-end" }}><Skeleton h={36} w="60%" r={14} /></div>
          <div className="stack" style={{ gap: 8 }}><Skeleton w="90%" /><Skeleton w="75%" /><Skeleton w="40%" /></div>
        </div>
      ) : null}
      {empty ? <ChatEmpty e={e} onTeach={teach} onPick={(t) => surface.current?.prefill(t)} /> : null}
      {turns && !empty ? (
        <div className="chat-inner">
          {items.map((it) => (it.kind === "turn" ? <Turn key={it.turn.run.id} turn={it.turn} e={e} /> : <BriefCard key={it.brief.id} b={it.brief} e={e} />))}
        </div>
      ) : null}
    </ChatSurface>
  );
}

function ErrorBlock({ text, onRetry }: { text: string; onRetry: () => void }) {
  return (
    <div className="error-note" role="alert">
      <Icon name="alert" size={14} />
      <span className="grow">{text}</span>
      <button className="btn btn-sm" onClick={onRetry}>Try again</button>
    </div>
  );
}

/** A thread or task you archived while this conversation carried on: the employee reads what it decided on their next turn. */
function BriefCard({ b, e }: { b: Brief; e: EmployeeView }) {
  const name = firstName(e.name);
  return (
    <div className="brief">
      <div className="brief-top">
        <Icon name="archive" size={13} />
        <span>{b.task ? "Task archived" : "Thread archived"}</span>
        <b>{b.label}</b>
        {b.task ? null : <span className="faint">· {b.replies} {b.replies === 1 ? "reply" : "replies"}</span>}
        <span className="grow" />
        <span className="faint brief-time" title={dateLong(b.createdAt)}>{relTime(b.createdAt)}</span>
      </div>
      {b.excerpt ? <p className="brief-excerpt">{name}, last: “{b.excerpt}”</p> : null}
      <div className="row">
        <span className="faint brief-note">{b.consumedAt ? `${name} read this at the start of their next turn.` : `${name} reads this at the start of their next turn here.`}</span>
        <span className="grow" />
        <button className="btn btn-ghost btn-sm" onClick={() => openMember(e.id, "chat", threadFocus(b.rootRunId), true)}>View {b.task ? "task" : "thread"} <Icon name="arrowRight" size={12} /></button>
      </div>
    </div>
  );
}

function ChatEmpty({ e, onTeach, onPick }: { e: EmployeeView; onTeach: () => void; onPick: (t: string) => void }) {
  const name = firstName(e.name);
  const suggestions = [
    `What's your objective, in your own words?`,
    `When you're unsure, ask me before acting.`,
    `Here's how I like reports written: status line first, then evidence.`,
  ];
  return (
    <div className="chat-empty">
      <Avatar name={e.name} hue={e.hue} size={56} />
      <h3 className="h-md">Teach {name} how you like things done</h3>
      <p className="muted">
        Everything you explain lands in one of {name}'s skills before they reply, so they keep it for every future run.
      </p>
      <button className="btn btn-primary" onClick={onTeach}><Icon name="teach" size={14} /> Teach a new task</button>
      <div className="chat-suggest">
        {suggestions.map((s) => <button key={s} className="chat-suggestion" onClick={() => onPick(s)}>{s}</button>)}
      </div>
    </div>
  );
}
