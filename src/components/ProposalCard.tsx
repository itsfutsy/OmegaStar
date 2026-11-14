import { useRef, useState } from "react";
import { call, errorMessage } from "../api/client";
import type { EmployeeView, HireProposal, OrgSnapshot, SkillInfo } from "../api/types";
import { folderNote, useFolderChecks } from "../lib/folders";
import { firstName } from "../lib/format";
import { toast } from "../lib/toasts";
import { openRun } from "../lib/router";
import { Icon } from "./Icon";
import { Avatar, ErrorNote } from "./ui";
import "./ProposalCard.css";

/** Who a proposed hire would report to, by name. */
function managerOf(p: HireProposal, snap: OrgSnapshot | null): string {
  const id = p.draft.managerId;
  if (!id) return snap?.company.ceoName ? firstName(snap.company.ceoName) : "you";
  if (id === p.employeeId) return firstName(snap?.employees.find((e) => e.id === id)?.name ?? "them");
  return snap?.employees.find((e) => e.id === id)?.name ?? snap?.people.find((x) => x.id === id)?.name ?? "someone";
}

/** The decision a hire proposal asks for, as its title: "Hire Noor Haddad as a Content Writer?" */
export function hireTitle(p: HireProposal): string {
  const role = p.draft.title.trim();
  if (!role) return `Hire ${p.draft.name}?`;
  return `Hire ${p.draft.name} as ${/^[aeiou]/i.test(role) ? "an" : "a"} ${role}?`;
}

/** A skill's name from its catalog id, "user:brand-voice" as "brand-voice". */
const skillName = (key: string, catalog: SkillInfo[] | undefined) => catalog?.find((c) => c.id === key)?.name ?? key.replace(/^[a-z]+:/, "");

/**
 * A hire an employee proposed, as the record of the person they would hire: who, the role and who they report to,
 * their objective, what they do, their skills (a skill not installed is flagged), and how they start. Under it the
 * reason in the proposer's own words, with the way to the conversation that asked.
 */
export function ProposalRecord({ p, proposer, snap, catalog }: {
  p: HireProposal; proposer: EmployeeView | undefined; snap: OrgSnapshot | null; catalog: SkillInfo[] | undefined;
}) {
  const who = proposer ? firstName(proposer.name) : "An employee";
  const d = p.draft;
  const skills = d.skills.keys.map((k) => skillName(k, catalog));
  const checks = useFolderChecks(d.workspaces);
  return (
    <>
      <section className="hire-rec" aria-label={`${d.name}, proposed`}>
        <Avatar name={d.name} hue={d.hue} size={64} proposed />
        <div className="hire-main">
          <div className="hire-name">{d.name}</div>
          <div className="hire-role">{d.title ? `${d.title}, would` : "Would"} report to {managerOf(p, snap)}</div>
          <dl className="hire-dl">
            {d.objective ? <><dt>Objective</dt><dd>{d.objective}</dd></> : null}
            {d.description ? <><dt>Does</dt><dd>{d.description}</dd></> : null}
            {skills.length || p.missingSkills.length ? (
              <>
                <dt>Skills</dt>
                <dd className="hire-skills">
                  {skills.map((s) => <span key={s} className="hire-skill mono">{s}</span>)}
                  {p.missingSkills.map((s) => (
                    <span key={s} className="hire-skill hire-skill-missing mono" title="Not in your Claude Code skills, so it can't be given to them yet">
                      <Icon name="alert" size={12} />{s}, not installed
                    </span>
                  ))}
                </dd>
              </>
            ) : null}
            <dt>Starts as</dt>
            <dd>{d.clearance === "supervised" ? "Supervised" : "Autonomous"}, in their own folder</dd>
            {d.workspaces.length ? (
              <>
                <dt>Also works in</dt>
                <dd className="hire-folders">
                  {d.workspaces.map((w) => {
                    const note = folderNote(w, checks.get(w));
                    return (
                      <span key={w} className="hire-folder">
                        <span className="mono">{w}</span>
                        {note ? (
                          <span className={`folder-note folder-note-${note.tone}`}>
                            <Icon name={note.tone === "ok" ? "check" : "alert"} size={12} />{note.tone === "ok" ? "Found" : "Not found"}
                          </span>
                        ) : null}
                      </span>
                    );
                  })}
                </dd>
              </>
            ) : null}
          </dl>
        </div>
      </section>
      {p.reason.trim() ? (
        <figure className="hire-quote">
          <blockquote>{p.reason.trim()}</blockquote>
          <figcaption>
            {proposer ? <Avatar name={proposer.name} hue={proposer.hue} image={proposer.image} size={18} /> : null}
            <span>{who}{proposer?.title ? `, ${proposer.title}` : ""}</span>
            {p.runId ? (
              <>
                <span aria-hidden>·</span>
                <button type="button" className="hire-why" onClick={() => p.runId && openRun(p.runId)}>Why {who} asked <Icon name="arrowUpRight" size={11} /></button>
              </>
            ) : null}
          </figcaption>
        </figure>
      ) : null}
    </>
  );
}

/**
 * A hire proposal's actions: Review & hire opens the hire sheet prefilled (focus comes back to the button when it
 * closes), and Decline asks for an optional note the proposer reads. `onDecline` settles the entry while the decline
 * goes; it resolves whether it went.
 */
export function ProposalActions({ p, proposer, onReview, onDecline }: {
  p: HireProposal;
  proposer: EmployeeView | undefined;
  onReview: (from: HTMLElement | null) => void;
  onDecline: (go: () => Promise<boolean>) => void;
}) {
  const [declining, setDeclining] = useState(false);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const review = useRef<HTMLButtonElement>(null);
  const who = proposer ? firstName(proposer.name) : "them";

  const decline = () => {
    setBusy(true);
    setErr(null);
    onDecline(async () => {
      try {
        await call("decline_proposal", { proposalId: p.id, note: note.trim() || null });
        toast(`Declined. ${proposer ? firstName(proposer.name) : "They"} hears why and adjusts the plan.`, "ok");
        return true;
      } catch (x) {
        setErr(errorMessage(x));
        setBusy(false);
        return false;
      }
    });
  };

  if (declining) {
    return (
      <div className="hire-decline enter-drop">
        <textarea
          className="textarea" rows={2} autoFocus value={note} onChange={(e) => setNote(e.target.value)} aria-label={`Why not, for ${who}`}
          placeholder={`Why not, or what instead? ${proposer ? firstName(proposer.name) : "They"} reads this. Optional.`}
          onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); decline(); } }}
        />
        {err ? <ErrorNote text={err} /> : null}
        <div className="hire-decline-acts">
          <button type="button" className="btn btn-sm" disabled={busy} data-main onClick={decline}>Decline and tell {who}</button>
          <button type="button" className="btn btn-ghost btn-sm" disabled={busy} onClick={() => { setDeclining(false); setErr(null); }}>Cancel</button>
        </div>
      </div>
    );
  }
  return (
    <>
      <button ref={review} type="button" className="btn btn-primary" data-main onClick={() => onReview(review.current)}>
        <Icon name="userPlus" size={14} /> Review &amp; hire <span className="kc">↵</span>
      </button>
      <button type="button" className="btn" data-no onClick={() => setDeclining(true)}>Decline</button>
    </>
  );
}
