import type { CardReply, EmployeeView, NewAttachment, Run, Thread } from "../api/types";
import { dateLong, firstName, relTime, timeOrDay, whenInline } from "../lib/format";
import { useThread } from "../lib/hooks";
import { lastMoved, useMarkSeen } from "../lib/marks";
import { openMember } from "../lib/router";
import { toolName } from "../lib/rules";
import { activityAt, didNotFinish, isStreak, isTeamWeek, lastFrom, lastReply, latestFinished, openCard, rowCardOpen, rowThreads, threadFocus } from "../lib/threads";
import { Icon } from "../components/Icon";
import { Avatar } from "../components/ui";
import { CardView } from "../components/conversation/CardView";
import { lines } from "../components/conversation/RowHistory";
import { DoneButton, SnoozeButton, ThreadView } from "../components/conversation/ThreadView";
import type { Item } from "../components/conversation/useStatuses";

/** The collapsed row, about the thread's newest finished run: what the employee needs from you, else what blocked them, else the first line; then a preview. */
function digest(t: Thread): { run: Run; title: string; preview: string } {
  const run = latestFinished(t);
  const ls = lines(run.summary);
  if (run.needsYou.length) return { run, title: run.needsYou[0], preview: ls.join(" ") };
  if (run.denials.length) {
    const title = run.denials.length === 1 ? `Blocked on ${toolName(run.denials[0].tool)}. Waiting for your OK` : `Blocked on ${run.denials.length} actions. Waiting for your OK`;
    return { run, title, preview: ls.join(" ") };
  }
  if (ls.length) return { run, title: ls[0], preview: ls.slice(1).join(" ") };
  const failed = didNotFinish(run);
  return { run, title: failed ? (run.id === t.root.id ? "This run didn't finish" : "This reply didn't finish") : "No report", preview: run.error ?? "" };
}

/** A team week's title: the first names of everyone in it, as "Aura, Mace, Cynthia and 3 more". */
function teamNames(threads: Thread[], employees: EmployeeView[]): string {
  const names = [...new Set(threads.map((x) => x.root.employeeId))]
    .flatMap((id) => employees.find((x) => x.id === id) ?? [])
    .map((x) => firstName(x.name));
  if (names.length > 3) return `${names.slice(0, 3).join(", ")} and ${names.length - 3} more`;
  return names.length > 1 ? `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}` : names[0] ?? "";
}

/** What an Inbox row can do to its thread. */
export type ItemActions = {
  /** Mark it done. */
  done: () => Promise<void>;
  snooze: (until: Date) => void;
  wake: () => void;
  reopen: () => void;
  /** Reply and mark it done in one go. */
  sendDone: (text: string, attachments: NewAttachment[]) => Promise<void>;
  /** Answer the card on `run`, which moves the row to the Archive. */
  answer: (run: Run, reply: CardReply) => Promise<void>;
};

/** One Inbox row: a thread collapsed to its digest, or open in place as the whole conversation. Its head carries
 *  what you can do with it, by where it stands: Snooze and Done, Wake now while snoozed, Reopen once done, and
 *  always the way to the employee's panel. */
export function InboxItem({ item, employees, open, onToggle, onHover, actions }: {
  item: Item;
  /** Everyone on the team, for a hand-off's other faces. */
  employees: EmployeeView[];
  open: boolean;
  onToggle: () => void;
  /** The pointer or focus entered (true) or left the row, so E knows which row you mean. */
  onHover: (on: boolean) => void;
  actions: ItemActions;
}) {
  const { thread: t, employee: e, title: context, status } = item;
  const { run, title, preview } = digest(t);
  const r = t.root;
  const failed = didNotFinish(run);
  const who = firstName(e.name);
  const busy = status.state === "working" || status.state === "queued";
  const done = status.state === "done";
  const last = lastReply(t);
  const from = lastFrom(t);
  const n = t.replies.length;
  // The rest of the row: a hand-off's other runs, or a schedule's earlier failures.
  const runsInRow = rowThreads(t);
  const streak = isStreak(t);
  // A week of the team's reviews reads as the team's, not as one employee's.
  const teamWeek = isTeamWeek(t);
  const others = [...new Set(runsInRow.map((x) => x.root.employeeId))].filter((id) => id !== r.employeeId)
    .flatMap((id) => employees.find((x) => x.id === id) ?? []);
  // A card waiting on you holds the row until you answer it, so Done steps aside.
  const card = openCard(t);
  const carded = rowCardOpen(t);
  return (
    <article className={`item ${open ? "item-open" : ""} ${status.unread ? "item-unread" : ""}`} onMouseEnter={() => onHover(true)} onMouseLeave={() => onHover(false)}>
      <div
        className="item-head" role="button" tabIndex={0} aria-expanded={open}
        onClick={onToggle}
        onKeyDown={(ev) => { if (ev.target === ev.currentTarget && (ev.key === "Enter" || ev.key === " ")) { ev.preventDefault(); onToggle(); } }}
        onFocus={() => onHover(true)} onBlur={() => onHover(false)}
      >
        {others.length ? (
          <span className="item-avs" aria-hidden>
            <Avatar name={e.name} hue={e.hue} size={24} presence={busy ? "working" : undefined} />
            <Avatar name={others[0].name} hue={others[0].hue} size={18} className="item-av2" />
          </span>
        ) : <Avatar name={e.name} hue={e.hue} size={30} presence={busy ? "working" : undefined} />}
        <div className="item-main">
          <span className="item-context">
            {teamWeek ? <b>Your team's week</b> : <><b>{e.name}</b><span className="muted"> · {context}</span></>}
          </span>
          {open ? null : <span className="item-title">
            {run.needsYou.length ? <Icon name="bell" size={13} className="item-bell" /> : null}
            {failed ? <Icon name="alert" size={13} className="item-failed" /> : null}
            <span>{teamWeek ? teamNames(runsInRow, employees) : title}</span>
          </span>}
          {!open && preview ? <span className="item-preview">{preview}</span> : null}
          {!open && card ? <div className="item-card"><CardView run={card} e={e} onAnswer={(reply) => actions.answer(card, reply)} /></div> : null}
          {n > 0 || run.needsYou.length > 1 || (run.denials.length > 0 && run.needsYou.length > 0 && !done && !busy) || runsInRow.length > 1 ? (
            <span className="item-tags">
              {streak ? (
                <span className="item-tag-err"><Icon name="alert" size={11} /> Failed {runsInRow.length} times in a row since {timeOrDay(runsInRow[0].root.createdAt)}</span>
              ) : runsInRow.length > 1 ? (
                <span title="Everything this hand-off set off, in one row">{others.length ? `With ${others.map((x) => firstName(x.name)).join(", ")} · ` : ""}{runsInRow.length} runs</span>
              ) : null}
              {n > 0 ? (
                <span className={busy ? "item-tag-live" : ""}>
                  {n} {n === 1 ? "reply" : "replies"} · {busy ? `${who} is replying` : from === "them" && last ? `${who} replied ${relTime(last.endedAt)}` : last ? `you replied ${relTime(last.createdAt)}` : ""}
                </span>
              ) : null}
              {run.denials.length > 0 && run.needsYou.length > 0 && !done && !busy ? <span className="item-tag-warn">Also blocked</span> : null}
              {run.needsYou.length > 1 ? <span className="item-tag-warn">{run.needsYou.length - 1} more for you</span> : null}
            </span>
          ) : null}
        </div>
        <div className="item-side">
          {status.unread ? <span className="report-new" aria-label="New" /> : null}
          {status.snoozedUntil
            ? <span className="faint item-when" title={dateLong(status.snoozedUntil)}>Wakes {whenInline(status.snoozedUntil)}</span>
            : <span className="faint item-when" title={dateLong(activityAt(t))}>{relTime(activityAt(t))}</span>}
        </div>
        <div className="item-actions" onClick={(ev) => ev.stopPropagation()}>
          {done ? (
            <button className="btn btn-ghost btn-sm" title="Reopen (E)" onClick={actions.reopen}><Icon name="undo" size={13} /> Reopen</button>
          ) : (
            <>
              <SnoozeButton snoozedUntil={status.snoozedUntil} onSnooze={actions.snooze} onWake={actions.wake} />
              {carded ? null : <DoneButton name={who} task={r.kind.type === "task"} disabled={busy} onDone={actions.done} />}
            </>
          )}
          <button className="btn btn-ghost btn-sm btn-icon" title={`Open on ${who}'s panel`} aria-label={`Open on ${who}'s panel`} onClick={() => openMember(e.id, "chat", threadFocus(r.id))}>
            <Icon name="org" size={14} />
          </button>
        </div>
      </div>
      {open ? <ItemThread item={item} onSendDone={done ? null : actions.sendDone} /> : null}
    </article>
  );
}

/** The open row's thread, live: the same conversation the employee's panel shows, seen once it is in front of you. */
function ItemThread({ item, onSendDone }: { item: Item; onSendDone: ItemActions["sendDone"] | null }) {
  const id = item.thread.root.id;
  const thread = useThread(id);
  useMarkSeen(item.status.state === "done" ? null : id, lastMoved(thread.turns?.map((t) => t.run) ?? []));
  return (
    <div className="item-thread">
      <ThreadView e={item.employee} rootId={id} thread={thread} row={item.thread} onSendDone={onSendDone} autoFocus={false} />
    </div>
  );
}
