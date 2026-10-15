import { useMemo, useRef } from "react";
import { call, errorMessage } from "../../api/client";
import type { Brief, EmployeeView, NewAttachment } from "../../api/types";
import { dateLong, firstName, relTime } from "../../lib/format";
import { useQuery, type Conversation } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import type { LiveRun } from "../../lib/runs";
import { openChat } from "../../lib/router";
import { threadFocus } from "../../lib/threads";
import { Icon } from "../Icon";
import { Avatar, Skeleton } from "../ui";
import { ChatSurface, type ChatSurfaceHandle } from "../panel/ChatSurface";
import { Turn } from "../panel/ChatTurn";
import "../panel/ChatTab.css";

const TEACH_PREFIX = "I want to teach you a new task: ";

type Item = { kind: "turn"; at: string; turn: LiveRun } | { kind: "brief"; at: string; brief: Brief };

/** The long-lived conversation, with a card for each thread or task you archived since it carried on. What
 *  you send while a turn works reaches it there, or runs as the next turn. `chat` is its turns, which the
 *  conversation screen holds so its header can start a new conversation. */
export function OngoingChat({ e, chat }: { e: EmployeeView; chat: Conversation }) {
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
  const teach = () => surface.current?.prefill(TEACH_PREFIX);
  const empty = turns && turns.length === 0 && (briefs.data?.length ?? 0) === 0;

  return (
    <ChatSurface
      ref={surface} e={e} busy={busy} live={live} draftKey={e.id} scrollKey={`${liveKey}:${briefs.data?.length ?? 0}`}
      placeholder={`Message ${name}. Teach them something, or ask.`} onSend={send} onStop={stop}
      actions={busy ? null : <button className="btn btn-ghost btn-sm" onClick={teach}><Icon name="teach" size={13} /> Teach a new task</button>}
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
        <button className="btn btn-ghost btn-sm" onClick={() => openChat(e.id, threadFocus(b.rootRunId))}>View {b.task ? "task" : "thread"} <Icon name="arrowRight" size={12} /></button>
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
