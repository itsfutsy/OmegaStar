import { useEffect, useRef, useState } from "react";
import { call, errorMessage } from "../api/client";
import type { Thread } from "../api/types";
import { dayHeading } from "../lib/format";
import { useOrg, useProposals, useQuery } from "../lib/hooks";
import { toast } from "../lib/toasts";
import { activityAt } from "../lib/threads";
import { RequestCard } from "../components/RequestCard";
import { ProposalCard } from "../components/ProposalCard";
import { AskRow } from "../components/ColleagueAsks";
import { useShell } from "../Shell";
import { Icon } from "../components/Icon";
import { ErrorNote, PageColumn, Section, Skeleton } from "../components/ui";
import { InboxItem } from "./InboxItem";
import "./Inbox.css";

type View = "inbox" | "archive";

const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

/** What is waiting on you outside a conversation: hire proposals, requests to people, colleagues' asks in flight,
 *  and the Archive of reports you put away. Reports and threads themselves live in the sidebar's shelves. */
export function InboxScreen() {
  const { snap } = useOrg();
  const [view, setView] = useState<View>("inbox");
  const archive = useQuery(() => (view === "archive" ? call("archived") : Promise.resolve(undefined)), [view], ["run:updated"]);
  const waiting = useQuery(() => call("requests", { personId: null, employeeId: null, state: "open" }), [], ["request:updated", "org:changed"]);
  const asks = useQuery(() => call("open_asks"), [], ["run:updated"]);
  const proposals = (useProposals() ?? []).filter((p) => p.state === "open");
  const shell = useShell();
  const [openId, setOpenId] = useState<string | null>(null);
  const hovered = useRef<string | null>(null);

  const restore = async (t: Thread) => {
    try {
      await call("set_archived", { runId: t.root.id, archived: false });
      if (openId === t.root.id) setOpenId(null);
      toast("Reopened. It is back in the sidebar.", "neutral", {
        label: "Undo",
        run: () => void call("set_archived", { runId: t.root.id, archived: true }).catch((e) => toast(errorMessage(e), "err")),
      });
    } catch (e) {
      toast(errorMessage(e), "err");
    }
  };

  const restoreRef = useRef(restore);
  restoreRef.current = restore;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (view !== "archive" || e.ctrlKey || e.metaKey || e.altKey || typing(e.target) || document.querySelector("[role=dialog]")) return;
      const items = archive.data ?? [];
      if (e.key === "Escape" && openId) { setOpenId(null); return; }
      if (e.key.toLowerCase() !== "e") return;
      const t = items.find((x) => x.root.id === hovered.current) ?? items.find((x) => x.root.id === openId);
      if (!t) return;
      e.preventDefault();
      void restoreRef.current(t);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [view, archive.data, openId]);

  const employees = snap?.employees ?? [];
  const people = snap?.people ?? [];
  const groups: { day: string; threads: Thread[] }[] = [];
  for (const t of archive.data ?? []) {
    const day = dayHeading(t.root.archivedAt ?? activityAt(t));
    const g = groups[groups.length - 1];
    if (g && g.day === day) g.threads.push(t);
    else groups.push({ day, threads: [t] });
  }

  const showWaiting = view === "inbox" && !!waiting.data?.length;
  const showAsks = view === "inbox" && !!asks.data?.length;
  const showProposals = view === "inbox" && proposals.length > 0;
  const nothing = view === "inbox" ? !showWaiting && !showAsks && !showProposals && !!waiting.data && !!asks.data && !!proposals : archive.data?.length === 0;

  return (
    <PageColumn
      title="Inbox"
      actions={
        <div className="seg" role="tablist" aria-label="Inbox or archive">
          <button role="tab" aria-selected={view === "inbox"} aria-pressed={view === "inbox"} onClick={() => { setView("inbox"); setOpenId(null); }}>Inbox</button>
          <button role="tab" aria-selected={view === "archive"} aria-pressed={view === "archive"} onClick={() => { setView("archive"); setOpenId(null); }}>Archive</button>
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

      {view === "archive" && archive.error ? <ErrorNote text={archive.error} onRetry={archive.reload} /> : null}
      {view === "archive" && !archive.data && !archive.error ? <div className="stack" style={{ gap: 8 }}>{[0, 1, 2].map((i) => <Skeleton key={i} h={56} r={10} />)}</div> : null}
      {nothing ? (
        <div className="empty">
          <Icon name={view === "inbox" ? "inbox" : "archive"} size={22} />
          {view === "inbox" ? (
            <>
              <span className="empty-title">All caught up</span>
              <p>Hire proposals and requests your employees sent to people land here. Reports and threads are in the sidebar.</p>
            </>
          ) : (
            <>
              <span className="empty-title">Nothing archived</span>
              <p>Archive a report once you've dealt with it. If the employee raises something new about it, it comes back.</p>
            </>
          )}
        </div>
      ) : null}
      {view === "archive" && archive.data && archive.data.length > 0 ? (
        <Section title="Archived reports">
          <div className="inbox-list">
            {groups.map((g) => (
              <div key={g.day} className="inbox-group">
                <div className="inbox-day">{g.day}</div>
                {g.threads.map((t) => (
                  <InboxItem
                    key={t.root.id} t={t} e={employees.find((x) => x.id === t.root.employeeId)} employees={employees} people={people}
                    open={openId === t.root.id} onToggle={() => setOpenId((o) => (o === t.root.id ? null : t.root.id))}
                    onHover={(on) => { hovered.current = on ? t.root.id : hovered.current === t.root.id ? null : hovered.current; }}
                    onReopen={() => void restore(t)}
                  />
                ))}
              </div>
            ))}
          </div>
        </Section>
      ) : null}
    </PageColumn>
  );
}
