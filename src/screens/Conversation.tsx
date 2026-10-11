import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { call, errorMessage } from "../api/client";
import type { EmployeeView, NewAttachment } from "../api/types";
import { firstName, runKindLabel, whenInline, whenShort } from "../lib/format";
import { refreshOrg, useChat, useOrg, useThread } from "../lib/hooks";
import { chatKey, markSeen, setMark, useMarks, type Mark } from "../lib/marks";
import type { LiveRun } from "../lib/runs";
import { navigate, openMember, openRun, type Route } from "../lib/router";
import { nextRow, snoozePresets, type Placed } from "../lib/shelves";
import { toast } from "../lib/toasts";
import { isLive, NEW_TASK_FOCUS, parseThreadFocus } from "../lib/threads";
import { kindNames } from "../components/RunRow";
import { Icon } from "../components/Icon";
import { Avatar, ErrorNote, Popover, Skeleton, StatePill } from "../components/ui";
import { OngoingChat } from "../components/conversation/OngoingChat";
import { NewTask } from "../components/conversation/NewTask";
import { DoneButton, ThreadView } from "../components/conversation/ThreadView";
import { openRow, unstick, useShelves } from "../components/shelves/useShelves";
import "./Conversation.css";

const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

/** The conversation open in the main window: an employee's ongoing chat, one of their threads, or a task about to start. */
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
  const rootId = parseThreadFocus(route.focus);
  if (rootId) return <ThreadConversation key={rootId} e={e} rootId={rootId} />;
  if (route.focus === NEW_TASK_FOCUS) return <NewTaskConversation key={e.id} e={e} />;
  return <ChatConversation key={e.id} e={e} />;
}

/** The thread or task with `rootId`: its header actions, and E to mark it done. */
function ThreadConversation({ e, rootId }: { e: EmployeeView; rootId: string }) {
  const { snap } = useOrg();
  const thread = useThread(rootId);
  const { current, visible } = useShelves();
  const mark = useMarks()[rootId] ?? {};
  const leaving = useLeave(rootId, visible);
  useMarkSeen(rootId, lastMoved(thread.turns));
  const root = thread.turns?.[0]?.run ?? null;
  const replies = Math.max(0, (thread.turns?.length ?? 0) - 1);
  const task = root?.kind.type === "task";
  const archived = !!root?.archivedAt;
  const name = firstName(e.name);
  const row = current?.row.kind === "thread" ? current.row : null;
  const title = root ? runKindLabel(root.kind, kindNames(root, snap?.employees ?? [], snap?.people ?? [])) : row?.title ?? "";

  const done = async (brief: boolean) => {
    const go = leaving();
    try {
      await call("set_archived", { runId: rootId, archived: true, brief });
    } catch (err) {
      toast(errorMessage(err), "err");
      return;
    }
    const undo = () => void call("set_archived", { runId: rootId, archived: false }).catch((err) => toast(errorMessage(err), "err"));
    toast(brief && (task || replies) ? `Done. ${name} hears what was decided in their ongoing chat.` : "Done", "ok", { label: "Undo", run: undo });
    go();
  };
  const reopen = async () => {
    try {
      await call("set_archived", { runId: rootId, archived: false });
      unstick();
      toast("Reopened", "ok");
    } catch (err) {
      toast(errorMessage(err), "err");
    }
  };
  const sendDone = async (text: string, attachments: NewAttachment[]) => {
    const go = leaving();
    await call("reply_to_run", { runId: rootId, text, archive: true, attachments });
    toast(`Sent to ${name}. It comes back only if the answer needs you.`, "ok");
    go();
  };
  const snooze = (until: Date) => {
    const go = leaving();
    setMark(rootId, { snoozedUntil: until.toISOString() });
    go();
  };

  const doneRef = useRef(done);
  doneRef.current = done;
  const live = thread.turns?.filter((t) => isLive(t.run)) ?? [];
  const ready = !!root?.endedAt && !archived && live.length === 0;
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.ctrlKey || ev.metaKey || ev.altKey || typing(ev.target) || document.querySelector(".popover")) return;
      if (ev.key.toLowerCase() !== "e" || !ready) return;
      ev.preventDefault();
      void doneRef.current(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ready]);

  const fallback = archived ? "done" : live.length ? (live.every((t) => t.run.state === "queued") ? "queued" : "working") : null;
  return (
    <div className="convo">
      <ConversationHeader e={e} title={title} status={<Status p={current} fallback={fallback} />}>
        {root && !archived ? <PinButton k={rootId} mark={mark} /> : null}
        {root && !archived ? <SnoozeButton k={rootId} mark={mark} onSnooze={snooze} /> : null}
        {root && !task ? <button className="btn btn-ghost btn-sm" onClick={() => openRun(root.id)}>Open run <Icon name="arrowUpRight" size={12} /></button> : null}
        {root ? (
          archived
            ? <button className="btn btn-sm" onClick={() => void reopen()}><Icon name="undo" size={13} /> Reopen</button>
            : <DoneButton name={name} task={task} replies={replies} disabled={!ready} onDone={done} />
        ) : null}
      </ConversationHeader>
      <ThreadView e={e} rootId={rootId} thread={thread} row={row?.thread ?? null} onSendDone={archived ? null : sendDone} />
    </div>
  );
}

/** The employee's ongoing chat, with New conversation in its header. */
function ChatConversation({ e }: { e: EmployeeView }) {
  const chat = useChat(e.id);
  const key = chatKey(e.id);
  const { current, visible } = useShelves();
  const mark = useMarks()[key] ?? {};
  const leaving = useLeave(key, visible);
  useMarkSeen(key, lastMoved(chat.turns));
  const live = chat.turns?.filter((t) => isLive(t.run)) ?? [];
  const newConversation = async () => {
    try {
      await call("new_chat", { employeeId: e.id });
      chat.reload();
      toast(`Fresh conversation. Everything ${firstName(e.name)} learned stays in their skills.`, "ok");
    } catch (err) {
      toast(errorMessage(err), "err");
    }
  };
  const snooze = (until: Date) => {
    const go = leaving();
    setMark(key, { snoozedUntil: until.toISOString() });
    go();
  };
  const fallback = live.length ? (live.every((t) => t.run.state === "queued") ? "queued" : "working") : null;
  return (
    <div className="convo">
      <ConversationHeader e={e} title="Ongoing chat" status={<Status p={current} fallback={fallback} />}>
        <PinButton k={key} mark={mark} />
        <SnoozeButton k={key} mark={mark} onSnooze={snooze} />
        <button className="btn btn-ghost btn-sm" onClick={() => void newConversation()} disabled={!chat.turns?.length || live.length > 0}>
          <Icon name="newChat" size={13} /> New conversation
        </button>
      </ConversationHeader>
      <OngoingChat e={e} chat={chat} />
    </div>
  );
}

/** A task before its first message. */
function NewTaskConversation({ e }: { e: EmployeeView }) {
  return (
    <div className="convo">
      <ConversationHeader e={e} title="New task" status={null} />
      <NewTask e={e} />
    </div>
  );
}

/** The one header every conversation has: who it is with, what it is, where it stands, and what you can do with it. */
function ConversationHeader({ e, title, status, children }: { e: EmployeeView; title: string; status: ReactNode; children?: ReactNode }) {
  return (
    <header className="convo-head">
      <Avatar name={e.name} hue={e.hue} size={32} presence={e.presence} />
      <div className="convo-id">
        <h1 className="convo-title" title={title}>{title}</h1>
        <button className="convo-sub" onClick={() => openMember(e.id, "work")} title={`Open ${firstName(e.name)}'s work`}>{e.name}, {e.title}</button>
      </div>
      {status}
      <span className="grow" />
      {children ? <div className="convo-actions">{children}</div> : null}
    </header>
  );
}

/** Where the conversation stands, from its place in the sidebar; `fallback` for one that is not there. */
function Status({ p, fallback }: { p: Placed | null; fallback: "done" | "working" | "queued" | null }) {
  const state = p
    ? p.natural === "working" ? (p.queued ? "queued" : "working") : p.natural
    : fallback;
  switch (state) {
    case "needsYou": return <StatePill tone="warn">Needs you</StatePill>;
    case "working": return <StatePill tone="accent" pulse>Working</StatePill>;
    case "queued": return <StatePill tone="accent">Queued</StatePill>;
    case "snoozed": return <StatePill tone="neutral">Snoozed until {whenInline(p?.mark.snoozedUntil ?? null)}</StatePill>;
    case "done": return <StatePill tone="ok">Done</StatePill>;
    default: return null;
  }
}

/** Pin keeps the conversation on the Pinned shelf while nothing else needs placing it higher. */
function PinButton({ k, mark }: { k: string; mark: Mark }) {
  const pinned = !!mark.pinnedAt;
  return (
    <button className="btn btn-ghost btn-sm" aria-pressed={pinned} onClick={() => { setMark(k, { pinnedAt: pinned ? undefined : new Date().toISOString() }); unstick(); }}>
      <Icon name="pin" size={13} /> {pinned ? "Unpin" : "Pin"}
    </button>
  );
}

/** Snooze puts the conversation away until a time, then brings it back as Needs you. While snoozed: Wake now. */
function SnoozeButton({ k, mark, onSnooze }: { k: string; mark: Mark; onSnooze: (until: Date) => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  if (mark.snoozedUntil && Date.parse(mark.snoozedUntil) > Date.now()) {
    return (
      <button className="btn btn-ghost btn-sm" onClick={() => { setMark(k, { snoozedUntil: undefined }); unstick(); }}>
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

/** When a conversation last moved, from its runs: what makes looking at it again count as seeing something new. */
function lastMoved(turns: LiveRun[] | null): string {
  let at = "";
  for (const t of turns ?? []) {
    const x = t.run.endedAt ?? t.run.createdAt;
    if (x > at) at = x;
  }
  return at;
}

/** Marks the conversation seen when it opens and whenever it moves, while the window is in front of you. */
function useMarkSeen(key: string, moved: string) {
  const [focused, setFocused] = useState(() => document.hasFocus() && document.visibilityState === "visible");
  useEffect(() => {
    const update = () => setFocused(document.hasFocus() && document.visibilityState === "visible");
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    document.addEventListener("visibilitychange", update);
    return () => { window.removeEventListener("focus", update); window.removeEventListener("blur", update); document.removeEventListener("visibilitychange", update); };
  }, []);
  useEffect(() => { if (focused) markSeen(key); }, [key, moved, focused]);
}

/** Leaving the conversation after you snooze it or mark it done: call before acting, to fix the row to open while
 *  the sidebar (`visible`) still has this one in place, then call what it returns to go there (the org chart when
 *  none is left). */
function useLeave(key: string, visible: Placed[]): () => () => void {
  const rows = useRef(visible);
  rows.current = visible;
  return useCallback(() => {
    const next = nextRow(rows.current, key);
    return () => {
      unstick();
      if (next && next.row.kind !== "proposal") openRow(next.row);
      else navigate({ page: "org", memberId: null, tab: null, focus: null });
    };
  }, [key]);
}
