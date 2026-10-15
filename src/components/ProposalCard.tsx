import { useState } from "react";
import { call, errorMessage } from "../api/client";
import type { EmployeeView, HireProposal, OrgSnapshot } from "../api/types";
import { firstName, relTime } from "../lib/format";
import { toast } from "../lib/toasts";
import { openChat, openRun } from "../lib/router";
import { Icon } from "./Icon";
import { Avatar, ErrorNote } from "./ui";
import "./RequestCard.css";

/** Who a proposed hire would report to, by name. */
function managerOf(p: HireProposal, snap: OrgSnapshot | null): string {
  const id = p.draft.managerId;
  if (!id) return snap?.company.ceoName ?? "You";
  if (id === p.employeeId) return firstName(snap?.employees.find((e) => e.id === id)?.name ?? "them");
  return snap?.employees.find((e) => e.id === id)?.name ?? snap?.people.find((x) => x.id === id)?.name ?? "someone";
}

/** A hire an employee proposed: Review & hire opens the hire sheet prefilled, Decline tells them why. */
export function ProposalCard({ p, proposer, snap, onReview }: {
  p: HireProposal; proposer: EmployeeView | undefined; snap: OrgSnapshot | null; onReview: () => void;
}) {
  const [declining, setDeclining] = useState(false);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const who = proposer ? firstName(proposer.name) : "An employee";
  
  const decline = async () => {
    setBusy(true);
    setErr(null);
    try {
      await call("decline_proposal", { proposalId: p.id, note: note.trim() || null });
      toast(`Declined. ${who} hears why and adjusts their plan.`, "ok");
    } catch (x) {
      setErr(errorMessage(x));
      setBusy(false);
    }
  };

  return (
    <article className="req req-open">
      <header className="req-head">
        {proposer ? <Avatar name={proposer.name} hue={proposer.hue} size={20} /> : null}
        <span className="req-who">
          <button className="req-name" onClick={() => proposer && openChat(proposer.id)}>{who}</button>
          <span className="muted"> wants to hire</span>
          <span className="faint"> · {relTime(p.createdAt)}</span>
        </span>
      </header>
      <div className="prop-hire">
        <Avatar name={p.draft.name} hue={p.draft.hue} size={32} />
        <div className="stack" style={{ minWidth: 0 }}>
          <span className="prop-name">{p.draft.name} <span className="muted">· {p.draft.title}</span></span>
          <span className="faint prop-sub">Reports to {managerOf(p, snap)}{p.draft.objective ? <> · {p.draft.objective}</> : null}</span>
        </div>
      </div>
      <p className="req-msg">{p.reason}</p>
      {p.missingSkills.length ? (
        <p className="prop-missing" title="Not in your Claude Code skills, so it can't be given to them yet">
          <Icon name="alert" size={12} /> Needs {p.missingSkills.join(", ")}, which {p.missingSkills.length === 1 ? "isn't" : "aren't"} installed
        </p>
      ) : null}
      {declining ? (
        <div className="req-reply">
          <textarea className="textarea" rows={2} autoFocus value={note} onChange={(e) => setNote(e.target.value)}
            placeholder={`Why not, or what instead? ${who} reads this. Optional.`}
            onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void decline(); } }} />
          {err ? <ErrorNote text={err} /> : null}
          <div className="row">
            <span className="grow" />
            <button className="btn btn-ghost btn-sm" onClick={() => setDeclining(false)}>Cancel</button>
            <button className="btn btn-sm" disabled={busy} onClick={() => void decline()}>Decline and tell {who}</button>
          </div>
        </div>
      ) : (
        <div className="req-actions">
          <button className="btn btn-primary btn-sm" onClick={onReview}><Icon name="userPlus" size={12} /> Review & hire</button>
          <button className="btn btn-sm btn-ghost" onClick={() => setDeclining(true)}>Decline</button>
          <span className="grow" />
          {p.runId ? <button className="req-link" onClick={() => p.runId && openRun(p.runId)}>Why {who} asked <Icon name="arrowUpRight" size={11} /></button> : null}
        </div>
      )}
    </article>
  );
}
