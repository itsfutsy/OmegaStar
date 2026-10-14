import { useEffect, useRef, useState } from "react";
import { call } from "../api/client";
import type { NewAttachment } from "../api/types";
import { dayHeading, firstName } from "../lib/format";
import { useOrg, useProposals, useQuery } from "../lib/hooks";
import { setMark } from "../lib/marks";
import { activityAt } from "../lib/threads";
import { RequestCard } from "../components/RequestCard";
import { ProposalCard } from "../components/ProposalCard";
import { AskRow } from "../components/ColleagueAsks";
import { useShell } from "../Shell";
import { Icon } from "../components/Icon";
import { ErrorNote, PageColumn, Section, Skeleton } from "../components/ui";
import { markDone, reopen, sendAndDone } from "../components/conversation/ThreadView";
import { toItem, useStatuses, type Item } from "../components/conversation/useStatuses";
import { InboxItem, type ItemActions } from "./InboxItem";
import "./Inbox.css";

type View = "inbox" | "snoozed" | "archive";

const VIEWS: { id: View; label: string }[] = [
  { id: "inbox", label: "Inbox" },
  { id: "snoozed", label: "Snoozed" },
  { id: "archive", label: "Archive" },
];

const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

/** Rows under a day heading: the day each moved, or in the Archive the day each was done. Snoozed rows are one list,
 *  soonest to wake first. */
function groupRows(view: View, rows: Item[]): { day: string | null; rows: Item[] }[] {
  if (view === "snoozed") return rows.length ? [{ day: null, rows }] : [];
  const groups: { day: string; rows: Item[] }[] = [];
  for (const i of rows) {
    const t = i.thread;
    const day = dayHeading(view === "archive" ? t.root.archivedAt ?? activityAt(t) : activityAt(t));
    const g = groups[groups.length - 1];
    if (g && g.day === day) g.rows.push(i);
    else groups.push({ day, rows: [i] });
  }
  return groups;
}

/** The row to open once `id` leaves the list: the next one down, else the one above. */
function after(rows: Item[], id: string): string | null {
  const i = rows.findIndex((x) => x.thread.root.id === id);
  if (i < 0) return null;
  return (rows[i + 1] ?? rows[i - 1])?.thread.root.id ?? null;
}

/**
 * What is waiting on you: hire proposals, requests to people, colleagues' asks in flight, then every report and
 * answered thread until you mark it done. A row opens in place into the whole thread; E marks the open row done
 * (else the one under the pointer) and opens the next, so you work down to zero. Snoozed holds what you put away
 * until later, and the Archive what you marked done.
 */
export function InboxScreen() {
  const { snap } = useOrg();
  const [view, setView] = useState<View>("inbox");
  const { items, ctx, error } = useStatuses();
  const archive = useQuery(() => (view === "archive" ? call("archived") : Promise.resolve(undefined)), [view], ["run:updated"]);
  const waiting = useQuery(() => call("requests", { personId: null, employeeId: null, state: "open" }), [], ["request:updated", "org:changed"]);
  const asks = useQuery(() => call("open_asks"), [], ["run:updated"]);
  const proposals = (useProposals() ?? []).filter((p) => p.state === "open");
  const shell = useShell();
  const [openId, setOpenId] = useState<string | null>(null);
  const hovered = useRef<string | null>(null);

  const rows: Item[] | null =
    view === "inbox" ? items?.filter((i) => i.status.inbox) ?? null
    : view === "snoozed" ? items?.filter((i) => i.status.state === "snoozed").sort((a, b) => a.status.snoozedUntil!.localeCompare(b.status.snoozedUntil!)) ?? null
    : snap && archive.data ? archive.data.flatMap((t) => toItem(t, snap, false, ctx) ?? []) : null;
  const loadError = view === "archive" ? archive.error : error;

  /** What a row's buttons do. Done, Snooze and Send & done open the next row, so you work down the list. */
  const actionsFor = (i: Item): ItemActions => {
    const id = i.thread.root.id;
    const name = firstName(i.employee.name);
    const leaving = () => {
      const next = after(rows ?? [], id);
      return () => setOpenId(next);
    };
    return {
      done: async () => {
        const go = leaving();
        if (await markDone(id, name)) go();
      },
      snooze: (until) => {
        const go = leaving();
        setMark(id, { snoozedUntil: until.toISOString() });
        go();
      },
      wake: () => setMark(id, { snoozedUntil: undefined }),
      reopen: () => {
        if (openId === id) setOpenId(null);
        void reopen(id);
      },
      sendDone: async (text: string, attachments: NewAttachment[]) => {
        const go = leaving();
        await sendAndDone(id, name, text, attachments);
        go();
      },
    };
  };

  // E acts on the open row, else the one under the pointer: Done, or in the Archive, Reopen. Escape closes the row.
  const latest = useRef({ rows, actionsFor, openId });
  latest.current = { rows, actionsFor, openId };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey || typing(e.target) || document.querySelector("[role=dialog]")) return;
      const { rows, actionsFor, openId } = latest.current;
      if (e.key === "Escape" && openId) { setOpenId(null); return; }
      if (e.key.toLowerCase() !== "e") return;
      const list = rows ?? [];
      const i = list.find((x) => x.thread.root.id === openId) ?? list.find((x) => x.thread.root.id === hovered.current);
      if (!i) return;
      e.preventDefault();
      const act = actionsFor(i);
      if (i.status.state === "done") act.reopen();
      else if (i.status.state !== "working" && i.status.state !== "queued") void act.done();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const employees = snap?.employees ?? [];
  const people = snap?.people ?? [];
  const groups = groupRows(view, rows ?? []);
  const showWaiting = view === "inbox" && !!waiting.data?.length;
  const showAsks = view === "inbox" && !!asks.data?.length;
  const showProposals = view === "inbox" && proposals.length > 0;
  const nothing = rows?.length === 0 && !showWaiting && !showAsks && !showProposals;
  const pick = (v: View) => { setView(v); setOpenId(null); };

  return (
    <PageColumn
      title="Inbox"
      actions={
        <div className="seg" role="tablist" aria-label="Inbox, snoozed or archive">
          {VIEWS.map((v) => (
            <button key={v.id} role="tab" aria-selected={view === v.id} aria-pressed={view === v.id} onClick={() => pick(v.id)}>{v.label}</button>
          ))}
        </div>
      }
    >
      {showProposals ? (
        <Section title="Hire proposals" count={proposals.length}>
          <div className="plist">
            {proposals.map((p) => (
              <ProposalCard key={p.id} p={p} proposer={employees.find((e) => e.id === p.employeeId)} snap={snap} onReview={() => shell.reviewProposal(p)} />
            ))}
          </div>
        </Section>
      ) : null}
      {showWaiting ? (
        <Section title="Waiting on people" count={waiting.data!.length} id="waiting">
          <div className="plist">
            {waiting.data!.map((r) => (
              <RequestCard key={r.id} r={r} employee={employees.find((e) => e.id === r.employeeId)} person={people.find((p) => p.id === r.personId)} showPerson onChanged={() => waiting.reload()} />
            ))}
          </div>
        </Section>
      ) : null}
      {showAsks ? (
        <Section title="Between colleagues" count={asks.data!.length}>
          <div className="plist">
            {asks.data!.map((r) => <AskRow key={r.id} run={r} employees={employees} />)}
          </div>
        </Section>
      ) : null}

      {loadError ? <ErrorNote text={loadError} onRetry={view === "archive" ? archive.reload : undefined} /> : null}
      {!rows && !loadError ? <div className="stack" style={{ gap: 8 }}>{[0, 1, 2].map((i) => <Skeleton key={i} h={56} r={10} />)}</div> : null}
      {nothing ? <Empty view={view} /> : null}
      {rows && rows.length > 0 ? (
        <Section title={view === "inbox" ? "Reports and replies" : view === "snoozed" ? "Snoozed" : "Archived reports"} count={view === "archive" ? null : rows.length}>
          <div className="inbox-list">
            {groups.map((g) => (
              <div key={g.day ?? view} className="inbox-group">
                {g.day ? <div className="inbox-day">{g.day}</div> : null}
                {g.rows.map((i) => {
                  const id = i.thread.root.id;
                  return (
                    <InboxItem
                      key={id} item={i} employees={employees} actions={actionsFor(i)}
                      open={openId === id} onToggle={() => setOpenId((o) => (o === id ? null : id))}
                      onHover={(on) => { hovered.current = on ? id : hovered.current === id ? null : hovered.current; }}
                    />
                  );
                })}
              </div>
            ))}
          </div>
        </Section>
      ) : null}
    </PageColumn>
  );
}

/** What an empty view says. */
function Empty({ view }: { view: View }) {
  const [icon, title, text] =
    view === "inbox" ? ["inbox", "All caught up", "When an employee finishes a run or answers you, it lands here. Give them a schedule and it becomes a rhythm."] as const
    : view === "snoozed" ? ["clock", "Nothing snoozed", "Snooze a row to put it away until later. It comes back to your Inbox when it wakes."] as const
    : ["archive", "Nothing archived", "Mark a report done once you've dealt with it. If the employee raises something new about it, it comes back."] as const;
  return (
    <div className="empty">
      <Icon name={icon} size={22} />
      <span className="empty-title">{title}</span>
      <p>{text}</p>
    </div>
  );
}
