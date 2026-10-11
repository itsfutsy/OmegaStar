import { useEffect, useRef } from "react";
import type { EmployeeView, Run } from "../../api/types";
import { firstName } from "../../lib/format";
import { chatKey } from "../../lib/marks";
import { openChat } from "../../lib/router";
import type { Placed } from "../../lib/shelves";
import { NEW_TASK_FOCUS, parseThreadFocus, threadFocus } from "../../lib/threads";
import { kindIcon } from "../RunRow";
import { Icon, type IconName } from "../Icon";
import { useShelves } from "../shelves/useShelves";

/** One chip: what it opens and how it looks. */
type Chip = { focus: string | null; label: string; icon: IconName; tip: string; replies: number; p: Placed | null };

/**
 * The employee's conversations as a row of chips above the one open on their panel: Ongoing, then each thread
 * in sidebar order, then + for a new task. Done today stays out unless it is the open one. A thread opened by
 * link that the sidebar does not have still gets a chip, from its `root` run and `title`.
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
  const { shelves } = useShelves();
  const open = focus === null ? chatKey(e.id) : parseThreadFocus(focus);
  const rows = (shelves ?? []).flatMap((s) => s.rows).filter((p) => p.row.kind !== "proposal" && p.row.employee.id === e.id && (p.shelf !== "done" || p.row.key === open));
  const chips: Chip[] = [{ focus: null, label: "Ongoing", icon: "chat", tip: "Ongoing chat", replies: 0, p: rows.find((p) => p.row.kind === "chat") ?? null }];
  for (const p of rows) {
    if (p.row.kind !== "thread") continue;
    const t = p.row.thread;
    const tip = [p.row.title, p.reason].filter(Boolean).join(". ");
    chips.push({ focus: threadFocus(p.row.key), label: p.row.title, icon: t.root.archivedAt ? "archive" : kindIcon(t.root), tip, replies: t.replies.length, p });
  }
  if (root && open === root.id && !chips.some((c) => c.p?.row.key === open)) {
    chips.push({ focus, label: title, icon: root.archivedAt ? "archive" : kindIcon(root), tip: title, replies: 0, p: null });
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

/** A chip with a dot while the conversation needs you, works, or waits for a free slot. The open one stays in sight
 *  when the chips scroll. */
function ConversationChip({ c, on, onClick }: { c: Chip; on: boolean; onClick: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (on) ref.current?.scrollIntoView({ block: "nearest" }); }, [on]);
  const needsYou = c.p?.natural === "needsYou";
  const waiting = !needsYou && !!c.p?.queued;
  const working = !needsYou && !waiting && !!c.p?.busy;
  return (
    <button ref={ref} className={`chip thread-chip ${waiting ? "task-chip-wait" : ""}`} aria-pressed={on} onClick={onClick} title={c.tip}>
      <Icon name={c.icon} size={13} />
      <span className="thread-chip-label">{c.label}</span>
      {c.replies ? <span className="thread-chip-n num">{c.replies}</span> : null}
      {needsYou ? <span className="thread-chip-dot thread-chip-dot-warn" aria-label="Needs you" />
        : waiting ? <span className="thread-chip-dot" aria-label="Waiting to start" />
        : working ? <span className="thread-chip-dot thread-chip-dot-live" aria-label="Working" /> : null}
    </button>
  );
}
