import { useEffect, useRef, type ReactNode } from "react";
import { call } from "../api/client";
import type { EmployeeView, NewAttachment } from "../api/types";
import { firstName, typing } from "../lib/format";
import { refreshOrg, useOrg, useThread } from "../lib/hooks";
import { lastMoved, useMarkSeen } from "../lib/marks";
import { navigate, type Route } from "../lib/router";
import { parseThreadFocus, threadFocus } from "../lib/threads";
import { Icon } from "../components/Icon";
import { ErrorNote, Skeleton } from "../components/ui";
import { NewConversation } from "../components/conversation/NewConversation";
import { ConversationMenu, ConversationPicker, QuietState, useArchive, useLeave, useThreadState } from "../components/conversation/ConversationHeader";
import { sendAndDone, ThreadSkeleton, ThreadView } from "../components/conversation/ThreadView";
import "./Conversation.css";

/** The conversation filling the main window: one of an employee's threads, or a new one about to start. It is the one
 *  place you talk to an employee. */
export function ConversationScreen({ route }: { route: Extract<Route, { page: "chat" }> }) {
  const { snap, error } = useOrg();
  if (!snap) {
    return error ? (
      <div className="convo"><div className="thread-error"><ErrorNote title="Couldn't open this conversation" text={error} onRetry={refreshOrg} /></div></div>
    ) : (
      <div className="convo" aria-busy="true">
        <header className="convo-head"><Skeleton h={24} w={320} r={4} /></header>
        <ThreadSkeleton />
      </div>
    );
  }
  const e = snap.employees.find((x) => x.id === route.employeeId) ?? (snap.desk?.id === route.employeeId ? snap.desk : undefined);
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
  return (
    <ConversationFrame title="New conversation" picker={<ConversationPicker e={e} rootId={null} title="New conversation" />} menu={<ConversationMenu e={e} rootId={null} s={null} />}>
      <NewConversation key={e.id} e={e} />
    </ConversationFrame>
  );
}

/** The thread or conversation with `rootId`: its header, its menu, and E to archive it. */
function ThreadConversation({ e, rootId }: { e: EmployeeView; rootId: string }) {
  const thread = useThread(rootId);
  const s = useThreadState(rootId);
  const runs = thread.turns?.map((t) => t.run) ?? s.runs;
  useMarkSeen(rootId, lastMoved(runs));
  const name = firstName(e.name);
  const leave = useLeave(e.id, rootId);
  const archive = useArchive(e.id, rootId);
  const sendDone = async (text: string, attachments: NewAttachment[]) => {
    const go = leave();
    await sendAndDone(rootId, name, text, attachments);
    go();
  };

  const archiveRef = useRef(archive);
  archiveRef.current = archive;
  const { ready, carded } = s;
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.ctrlKey || ev.metaKey || ev.altKey || typing(ev.target) || document.querySelector(".popover")) return;
      if (ev.key.toLowerCase() !== "e" || !ready || carded) return;
      ev.preventDefault();
      void archiveRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ready, carded]);

  const title = s.title || "Conversation";
  return (
    <ConversationFrame title={title} picker={<ConversationPicker e={e} rootId={rootId} title={title} />} state={<QuietState s={s} />} menu={<ConversationMenu e={e} rootId={rootId} s={s} />}>
      <ThreadView e={e} rootId={rootId} thread={thread} row={s.row?.thread ?? null} onSendDone={s.archived ? null : sendDone} />
    </ConversationFrame>
  );
}

/** One quiet band over the talk: the conversation's title, which switches to another, a word when it is put away, and
 *  one menu for the rest. Who it is with is the sidebar's open row and each reply's name. */
function ConversationFrame({ title, picker, state, menu, children }: { title: string; picker: ReactNode; state?: ReactNode; menu: ReactNode; children: ReactNode }) {
  return (
    <div className="convo" aria-label={title}>
      <header className="convo-head">
        {picker}
        {state}
        <span className="grow" />
        {menu}
      </header>
      {children}
    </div>
  );
}

/** An old link to a run's own page: it opens the conversation the run is part of. */
export function RunRedirect({ runId }: { runId: string }) {
  useEffect(() => {
    let live = true;
    call("thread", { runId }).then(
      (t) => { if (live) navigate({ page: "chat", employeeId: t.root.employeeId, focus: threadFocus(t.root.id) }, { replace: true }); },
      () => { if (live) navigate({ page: "activity", view: "runs" }, { replace: true }); },
    );
    return () => { live = false; };
  }, [runId]);
  return <div className="convo" aria-busy="true"><header className="convo-head"><Skeleton h={24} w={320} r={4} /></header><ThreadSkeleton /></div>;
}
