import { useCallback, useEffect, useRef, type ReactNode } from "react";
import { call, errorMessage } from "../api/client";
import type { EmployeeView, NewAttachment, Run } from "../api/types";
import { firstName, runKindLabel, whenInline } from "../lib/format";
import { refreshOrg, useChat, useOrg, useThread } from "../lib/hooks";
import { chatKey, lastMoved, setMark, useMarkSeen } from "../lib/marks";
import { openChat, openFullChat, openMember, openRun, type Route } from "../lib/router";
import { threadStatus, type Status } from "../lib/status";
import { toast } from "../lib/toasts";
import { isLive, NEW_TASK_FOCUS, openCard, parseThreadFocus, rowCardOpen, threadFocus } from "../lib/threads";
import { kindNames } from "../components/RunRow";
import { Icon } from "../components/Icon";
import { Avatar, ErrorNote, Skeleton, StatePill } from "../components/ui";
import { OngoingChat } from "../components/conversation/OngoingChat";
import { NewTask } from "../components/conversation/NewTask";
import { ConversationChips } from "../components/conversation/ConversationChips";
import { DoneButton, markDone, reopen, sendAndDone, SnoozeButton, ThreadView } from "../components/conversation/ThreadView";
import { employeeItems, useStatuses, type Item } from "../components/conversation/useStatuses";
import "./Conversation.css";

const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

/** Where a conversation shows: filling the main window, or on the employee's panel over the org chart. */
export type ConversationVariant = "full" | "panel";

/** The conversation filling the main window: an employee's ongoing chat, one of their threads, or a task about to start. */
export function ConversationScreen({ route }: { route: Extract<Route, { page: "chat" }> }) {
  const { snap, error } = useOrg();
  if (!snap) {
    return <div className="convo convo-wait">{error ? <ErrorNote text={error} onRetry={refreshOrg} /> : <Skeleton h={44} r={10} />}</div>;
  }
  const e = snap.employees.find((x) => x.id === route.employeeId);
  if (!e) {
    return (
      <div className="convo empty">
        <Icon name="chat" size={22} />
        <span className="empty-title">They no longer work here</span>
        <p>Their conversations left with them. Their runs are still in Activity.</p>
      </div>
    );
  }
  return <Conversation e={e} focus={route.focus} variant="full" />;
}

/** The employee's conversation that `focus` names, the same in either variant: their ongoing chat (null), a thread
 *  (`thread~<root run id>`) or a new task. */
export function Conversation({ e, focus, variant }: { e: EmployeeView; focus: string | null; variant: ConversationVariant }) {
  const rootId = parseThreadFocus(focus);
  if (rootId) return <ThreadConversation key={rootId} e={e} rootId={rootId} variant={variant} />;
  if (focus === NEW_TASK_FOCUS) return <NewTaskConversation key={e.id} e={e} variant={variant} />;
  return <ChatConversation key={e.id} e={e} variant={variant} />;
}

/** The thread or task with `rootId`: its header actions, and E to mark it done. */
function ThreadConversation({ e, rootId, variant }: { e: EmployeeView; rootId: string; variant: ConversationVariant }) {
  const { snap } = useOrg();
  const thread = useThread(rootId);
  const { items, filed, ctx } = useStatuses();
  const leave = useLeave(e.id, rootId, items);
  const runs = thread.turns?.map((t) => t.run) ?? [];
  useMarkSeen(rootId, lastMoved(runs));
  const root = runs[0] ?? null;
  const task = root?.kind.type === "task";
  const archived = !!root?.archivedAt;
  const name = firstName(e.name);
  const row = items?.find((i) => i.thread.root.id === rootId) ?? null;
  const status = root ? threadStatus({ root, replies: runs.slice(1), related: [] }, !!filed?.has(rootId), ctx) : null;
  const title = root ? runKindLabel(root.kind, kindNames(root, snap?.employees ?? [], snap?.people ?? [])) : row?.title ?? "";

  const done = async () => {
    const go = leave();
    if (await markDone(rootId, name)) go();
  };
  const sendDone = async (text: string, attachments: NewAttachment[]) => {
    const go = leave();
    await sendAndDone(rootId, name, text, attachments);
    go();
  };
  const snooze = (until: Date) => {
    const go = leave();
    setMark(rootId, { snoozedUntil: until.toISOString() });
    go();
  };

  const doneRef = useRef(done);
  doneRef.current = done;
  const ready = !!root?.endedAt && !archived && !runs.some(isLive);
  // A card waiting on you is answered on the card, so neither Done nor E closes the thread.
  const carded = (root ? !!openCard({ root, replies: runs.slice(1), related: [] }) : false) || (row ? rowCardOpen(row.thread) : false);
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.ctrlKey || ev.metaKey || ev.altKey || typing(ev.target) || document.querySelector(".popover")) return;
      if (ev.key.toLowerCase() !== "e" || !ready || carded) return;
      ev.preventDefault();
      void doneRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ready, carded]);

  const actions = root ? (
    <>
      {archived ? null : <SnoozeButton snoozedUntil={status?.snoozedUntil ?? null} onSnooze={snooze} onWake={() => setMark(rootId, { snoozedUntil: undefined })} />}
      {task ? null : <button className="btn btn-ghost btn-sm" onClick={() => openRun(root.id)}>Open run <Icon name="arrowUpRight" size={12} /></button>}
      {archived
        ? <button className="btn btn-sm" onClick={() => void reopen(rootId)}><Icon name="undo" size={13} /> Reopen</button>
        : carded ? null : <DoneButton name={name} task={task} disabled={!ready} onDone={done} />}
    </>
  ) : null;
  return (
    <ConversationFrame e={e} variant={variant} focus={threadFocus(rootId)} title={title} root={root} status={<StatusPill status={status} />} actions={actions}>
      <ThreadView e={e} rootId={rootId} thread={thread} row={row?.thread ?? null} onSendDone={archived ? null : sendDone} />
    </ConversationFrame>
  );
}

/** The employee's ongoing chat, with New conversation in its header. */
function ChatConversation({ e, variant }: { e: EmployeeView; variant: ConversationVariant }) {
  const chat = useChat(e.id);
  const status = useStatuses().chats.get(e.id) ?? null;
  const runs = chat.turns?.map((t) => t.run) ?? [];
  useMarkSeen(chatKey(e.id), lastMoved(runs));
  const busy = runs.some(isLive);
  const newConversation = async () => {
    try {
      await call("new_chat", { employeeId: e.id });
      chat.reload();
      toast(`Fresh conversation. Everything ${firstName(e.name)} learned stays in their skills.`, "ok");
    } catch (err) {
      toast(errorMessage(err), "err");
    }
  };
  const actions = (
    <button className="btn btn-ghost btn-sm" onClick={() => void newConversation()} disabled={!runs.length || busy}>
      <Icon name="newChat" size={13} /> New conversation
    </button>
  );
  return (
    <ConversationFrame e={e} variant={variant} focus={null} title="Ongoing chat" status={<StatusPill status={status} />} actions={actions}>
      <OngoingChat e={e} chat={chat} />
    </ConversationFrame>
  );
}

/** A task before its first message. */
function NewTaskConversation({ e, variant }: { e: EmployeeView; variant: ConversationVariant }) {
  return (
    <ConversationFrame e={e} variant={variant} focus={NEW_TASK_FOCUS} title="New task" status={null}>
      <NewTask e={e} />
    </ConversationFrame>
  );
}

/** What every conversation has around it: one header saying what it is, where it stands and what you can do with
 *  it, and a button to the other variant. Full window, the header also says who it is with; on the panel, which
 *  already does, the employee's other conversations sit above it as chips. */
function ConversationFrame({ e, variant, focus, title, root = null, status, actions, children }: {
  e: EmployeeView;
  variant: ConversationVariant;
  /** The conversation, as the route names it. */
  focus: string | null;
  title: string;
  /** A thread's root run, once loaded, for its chip when it is not open any more. */
  root?: Run | null;
  status: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  const panel = variant === "panel";
  return (
    <div className={`convo ${panel ? "convo-panel" : ""}`}>
      {panel ? <ConversationChips e={e} focus={focus} title={title} root={root} /> : null}
      <header className="convo-head">
        {panel ? <h3 className="convo-title" title={title}>{title}</h3> : (
          <>
            <Avatar name={e.name} hue={e.hue} size={32} presence={e.presence} />
            <div className="convo-id">
              <h1 className="convo-title" title={title}>{title}</h1>
              <button className="convo-sub" onClick={() => openMember(e.id, "work")} title={`Open ${firstName(e.name)}'s work`}>{e.name}, {e.title}</button>
            </div>
          </>
        )}
        {status}
        <span className="grow" />
        <div className="convo-actions">
          {actions}
          {panel ? (
            <button className="btn btn-ghost btn-icon btn-sm" aria-label="Open full window" title="Open full window" onClick={() => openFullChat(e.id, focus)}>
              <Icon name="expand" size={14} />
            </button>
          ) : (
            <button className="btn btn-ghost btn-icon btn-sm" aria-label="Show on the org chart" title="Show on the org chart" onClick={() => openMember(e.id, "chat", focus)}>
              <Icon name="org" size={14} />
            </button>
          )}
        </div>
      </header>
      {children}
    </div>
  );
}

/** Where the conversation stands, as a pill; nothing while it is idle. */
function StatusPill({ status }: { status: Status | null }) {
  switch (status?.state) {
    case "needsYou": return <StatePill tone="warn">Needs you</StatePill>;
    case "working": return <StatePill tone="accent" pulse>Working</StatePill>;
    case "queued": return <StatePill tone="accent">Queued</StatePill>;
    case "snoozed": return <StatePill tone="neutral">Snoozed until {whenInline(status.snoozedUntil)}</StatePill>;
    case "done": return <StatePill tone="ok">Done</StatePill>;
    default: return null;
  }
}

/** Leaving the thread after you snooze it or mark it done: call before acting, to fix the employee's next thread
 *  while this one is still among `items`, then call what it returns to open it (their ongoing chat when none is left). */
function useLeave(employeeId: string, rootId: string, items: Item[] | null): () => () => void {
  const latest = useRef(items);
  latest.current = items;
  return useCallback(() => {
    const mine = employeeItems(latest.current ?? [], employeeId);
    const i = mine.findIndex((x) => x.thread.root.id === rootId);
    const next = i < 0 ? null : mine[i + 1] ?? mine[i - 1] ?? null;
    return () => openChat(employeeId, next ? threadFocus(next.thread.root.id) : null, true);
  }, [employeeId, rootId]);
}
