import { useRef } from "react";
import { call } from "../../api/client";
import type { EmployeeView, NewAttachment } from "../../api/types";
import { firstName, relTime, whenInline } from "../../lib/format";
import { openChat } from "../../lib/router";
import { threadFocus } from "../../lib/threads";
import { Icon } from "../Icon";
import { Avatar } from "../ui";
import { ChatSurface, type ChatSurfaceHandle } from "./ChatSurface";
import "./Chat.css";

/** What "Teach a new task" puts in the composer, to finish in your own words. */
export const TEACH_PREFIX = "I want to teach you a new task: ";
const SUGGESTIONS = [
  "What's your objective, in your own words?",
  "When you're unsure, ask me before acting.",
  "Here's how I like reports written: status line first, then evidence.",
];
/** The front desk's: about the team, and messages to pass on. */
const DESK_SUGGESTIONS = [
  "Who's working on what right now?",
  "Pass a message on: ",
  "Get this planned and done: ",
];

/** The draft key of an employee's new-conversation composer, so a message can wait in it, such as a hire's first lesson. */
export const newConversationDraft = (employeeId: string): string => `new-conversation:${employeeId}`;

/** Where they are now, in a word: working, up next, their next run, or when they last worked. */
function Now({ e }: { e: EmployeeView }) {
  if (e.presence === "working") return <span className="status status-live">Working</span>;
  if (e.presence === "queued") return <span className="status status-wait">Up next</span>;
  if (e.presence === "attention") return <span className="status status-attn">Needs you</span>;
  if (e.nextFire) return <span>Next run {whenInline(e.nextFire)}</span>;
  if (e.lastWorkedAt) return <span>Last worked {relTime(e.lastWorkedAt)}</span>;
  return <span>No runs yet</span>;
}

/** A conversation before its first message, as meeting a colleague: who they are and what they are doing now, one
 *  sentence on how conversations work, then three ways to start. What you send starts it and opens it. */
export function NewConversation({ e }: { e: EmployeeView }) {
  const surface = useRef<ChatSurfaceHandle>(null);
  const name = firstName(e.name);
  const desk = e.desk === true;
  const suggestions = desk ? DESK_SUGGESTIONS : SUGGESTIONS;
  const start = async (text: string, attachments: NewAttachment[]) => {
    const run = await call("start_task", { employeeId: e.id, text, attachments });
    openChat(e.id, threadFocus(run.id), true);
  };
  return (
    <ChatSurface
      ref={surface} e={e} draftKey={newConversationDraft(e.id)} scrollKey="" pin={false}
      placeholder={desk ? "Ask about the team, or pass a message on." : `Message ${name}. Teach them something, or ask.`} onSend={start}
    >
      <div className="chat-inner">
        <div className="chat-start">
          <div className="chat-start-who">
            <Avatar name={e.name} hue={e.hue} image={e.image} size={48} />
            <div>
              <div className="chat-start-name">{e.name}</div>
              <div className="chat-start-role">{e.title}{desk ? null : <> · <Now e={e} /></>}</div>
            </div>
          </div>
          <p className="chat-start-note">
            {desk
              ? "Ask who's doing what, or say what you need. A quick message goes straight to the right person. Anything bigger goes to the coordinator. Answers come back here."
              : `Each conversation runs alongside ${name}'s other work, and what you explain goes into ${name}'s skills, so it holds for every run after.`}
          </p>
          <div>
            <div className="chat-start-sec">Start with</div>
            <div className="chat-suggest">
              {suggestions.map((s) => (
                <button key={s} className="chat-suggestion" onClick={() => surface.current?.prefill(s)}>
                  <Icon name="chat" size={14} /><span className="grow">{s}</span><Icon name="arrowRight" size={13} />
                </button>
              ))}
            </div>
          </div>
        </div>
      </div>
    </ChatSurface>
  );
}
