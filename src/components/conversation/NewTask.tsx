import { call } from "../../api/client";
import type { EmployeeView, NewAttachment } from "../../api/types";
import { firstName } from "../../lib/format";
import { openChat } from "../../lib/router";
import { threadFocus } from "../../lib/threads";
import { Icon } from "../Icon";
import { ChatSurface } from "../panel/ChatSurface";
import "../panel/ChatTab.css";

/** A task before its first message: what you send starts it, beside everything else, and opens it. */
export function NewTask({ e }: { e: EmployeeView }) {
  const name = firstName(e.name);
  const start = async (text: string, attachments: NewAttachment[]) => {
    const run = await call("start_task", { employeeId: e.id, text, attachments });
    openChat(e.id, threadFocus(run.id), true);
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
