import { useEffect, useRef } from "react";
import type { EmployeeView, Run } from "../../api/types";
import { firstName } from "../../lib/format";
import { openChat } from "../../lib/router";
import type { State, Status } from "../../lib/status";
import { NEW_TASK_FOCUS, parseThreadFocus, threadFocus } from "../../lib/threads";
import { kindIcon } from "../RunRow";
import { Icon, type IconName } from "../Icon";
import { employeeItems, useStatuses } from "./useStatuses";

/** One chip: what it opens, how it looks, and where its conversation stands (null when it is not open any more). */
type Chip = { focus: string | null; label: string; icon: IconName; tip: string; replies: number; status: Status | null };

/**
 * The employee's conversations as a row of chips above the one open on their panel: Ongoing, then each open
 * thread, newest first, then + for a new task. A thread that is not open any more (done, or opened by link) still
 * gets a chip while it is the open one, from its `root` run and `title`.
 */
export function ConversationChips({ e, focus, title, root }: {
  e: EmployeeView;
  /** The open conversation, as the route has it. */
  focus: string | null;
  /** The open conversation's title. */
  title: string;
  /** The open thread's root run, once loaded. */
  root: Run | null;
}) {
  const { items, chats } = useStatuses();
  const chips: Chip[] = [{ focus: null, label: "Ongoing", icon: "chat", tip: "Ongoing chat", replies: 0, status: chats.get(e.id) ?? null }];
  for (const i of employeeItems(items ?? [], e.id)) {
    const t = i.thread;
    chips.push({ focus: threadFocus(t.root.id), label: i.title, icon: kindIcon(t.root), tip: [i.title, i.status.reason].filter(Boolean).join(". "), replies: t.replies.length, status: i.status });
  }
  if (root && parseThreadFocus(focus) === root.id && !chips.some((c) => c.focus === focus)) {
    chips.push({ focus, label: title, icon: root.archivedAt ? "archive" : kindIcon(root), tip: title, replies: 0, status: null });
  }
  return (
    <div className="thread-switch" role="group" aria-label={`Conversations with ${firstName(e.name)}`}>
      {chips.map((c) => <ConversationChip key={c.focus ?? "chat"} c={c} on={c.focus === focus} onClick={() => openChat(e.id, c.focus, true)} />)}
      <button className="chip chip-new" aria-pressed={focus === NEW_TASK_FOCUS} aria-label="New task" title="New task, alongside everything else" onClick={() => openChat(e.id, NEW_TASK_FOCUS, true)}>
        <Icon name="plus" size={13} />
      </button>
    </div>
  );
}

/** The dot a chip shows for where its conversation stands; none for the other states. */
const DOTS: Partial<Record<State, { className: string; label: string }>> = {
  needsYou: { className: "thread-chip-dot thread-chip-dot-warn", label: "Needs you" },
  queued: { className: "thread-chip-dot", label: "Waiting to start" },
  working: { className: "thread-chip-dot thread-chip-dot-live", label: "Working" },
};

/** A chip with a dot while the conversation needs you, works, or waits for a free slot. The open one stays in sight
 *  when the chips scroll. */
function ConversationChip({ c, on, onClick }: { c: Chip; on: boolean; onClick: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (on) ref.current?.scrollIntoView({ block: "nearest" }); }, [on]);
  const dot = c.status ? DOTS[c.status.state] : undefined;
  return (
    <button ref={ref} className={`chip thread-chip ${c.status?.state === "queued" ? "task-chip-wait" : ""}`} aria-pressed={on} onClick={onClick} title={c.tip}>
      <Icon name={c.icon} size={13} />
      <span className="thread-chip-label">{c.label}</span>
      {c.replies ? <span className="thread-chip-n num">{c.replies}</span> : null}
      {dot ? <span className={dot.className} aria-label={dot.label} /> : null}
    </button>
  );
}
