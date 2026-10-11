import { useMemo, useState } from "react";
import { call, errorMessage } from "../api/client";
import type { EmployeeDraft, HireProposal, SkillInfo } from "../api/types";
import { refreshOrg, useCatalog, useOrg } from "../lib/hooks";
import { toast } from "../lib/toasts";
import { setDraft } from "../lib/drafts";
import { openChat } from "../lib/router";
import { firstName, hueFromName } from "../lib/format";
import { AdvancedFields, ClearancePicker, EmployeeManagerSelect } from "./EmployeeFields";
import { Markdown } from "./Markdown";
import { SkillPicker } from "./SkillPicker";
import { Icon } from "./Icon";
import { Avatar, ErrorNote, SlideOver } from "./ui";
import "./Sheet.css";

type Template = { id: string; title: string; description: string; objective: string; skills: string[]; firstLesson: string };

export const TEMPLATES: Template[] = [
  {
    id: "cloud", title: "Cloud Engineer",
    description: "Looks after the cloud estate: patching, backups, cost and certificates.",
    objective: "Keep infrastructure patched, backed up and within budget, and prove it with a weekly report.",
    skills: ["recurring-weekly", "aws-cost-review", "aws-toolkit", "incident-postmortem"],
    firstLesson: "Let's start with your weekly checks. Every Monday morning I want to know that patching ran, backups exist and cost is on track. Interview me until you can write it down as a skill.",
  },
  {
    id: "ea", title: "Executive Assistant",
    description: "Runs your inbox and calendar so nothing important waits and nothing trivial interrupts.",
    objective: "Every morning, I see only what needs me, with a draft reply ready.",
    skills: ["inbox-triage", "workspace", "docx", "meeting-notes"],
    firstLesson: "Let's start with the morning inbox pass. I'll tell you who matters and what I never want to see. Ask me questions until you can write it down as a skill.",
  },
  {
    id: "cos", title: "Chief of Staff",
    description: "Coordinates the team: hands out work, chases it, and reports up with one clear picture.",
    objective: "One honest weekly picture of the whole company, with an owner for every open action.",
    skills: ["recurring-weekly", "docx", "meeting-notes"],
    firstLesson: "You'll run our weekly rhythm. Let's define the Monday report together: what goes in, who you collect it from, and what counts as a red flag. Ask me until you can write it as a skill.",
  },
  { id: "blank", title: "Blank", description: "", objective: "", skills: [], firstLesson: "" },
];

const emptyDraft = (managerId: string | null): EmployeeDraft => ({
  name: "", title: "", description: "", objective: "", managerId,
  skills: { builtins: false, keys: [] }, clearance: "supervised", allowRules: [],
  model: null, effort: null, browser: false, autoTidy: true, workspaces: [], hue: 250,
});

function keysFor(t: Template, catalog: SkillInfo[] | undefined): string[] {
  if (!catalog) return [];
  return t.skills.map((n) => catalog.find((c) => c.name === n)?.id).filter((x): x is string => !!x);
}

const withTemplate = (d: EmployeeDraft, t: Template, catalog: SkillInfo[] | undefined): EmployeeDraft => ({
  ...d,
  title: t.id === "blank" ? "" : t.title,
  description: t.description,
  objective: t.objective,
  skills: { ...d.skills, keys: keysFor(t, catalog) },
});

function initialDraft(managerId: string | null, template: string | null, proposal: HireProposal | null, catalog: SkillInfo[] | undefined): EmployeeDraft {
  if (proposal) return { ...proposal.draft, skills: { ...proposal.draft.skills, keys: [...proposal.draft.skills.keys] } };
  const t = TEMPLATES.find((x) => x.id === template);
  return t ? withTemplate(emptyDraft(managerId), t, catalog) : emptyDraft(managerId);
}

/** Hire an employee; with `proposal`, review a hire an employee proposed and approve it, changed or not.
 *  Its form starts from the props when it mounts, so the Shell remounts it (a new `key`) each time it opens. */
export function HireSheet({ open, managerId, template, proposal, onClose }: {
  open: boolean; managerId: string | null; template: string | null; proposal: HireProposal | null; onClose: () => void;
}) {
  const { snap } = useOrg();
  const catalog = useCatalog();
  const [draft, setD] = useState(() => initialDraft(managerId, template, proposal, catalog.data));
  const [tpl, setTpl] = useState(proposal ? null : template);
  const [hueTouched, setHueTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [touched, setTouched] = useState(false);
  const [imports, setImports] = useState<string[]>([]);
  const [note, setNote] = useState("");
  const proposer = proposal ? snap?.employees.find((e) => e.id === proposal.employeeId) : undefined;
  const proposerName = proposer ? firstName(proposer.name) : "They";

  const applyTemplate = (id: string | null) => {
    setTpl(id);
    const t = TEMPLATES.find((x) => x.id === id);
    if (t) setD((d) => withTemplate(d, t, catalog.data));
  };

  // Template skill keys need the catalog; fill them in when a scan lands.
  const [scanned, setScanned] = useState(catalog.data);
  if (scanned !== catalog.data) {
    setScanned(catalog.data);
    const t = TEMPLATES.find((x) => x.id === tpl);
    if (t && catalog.data && draft.skills.keys.length === 0 && t.skills.length > 0) setD({ ...draft, skills: { ...draft.skills, keys: keysFor(t, catalog.data) } });
  }

  const hue = hueTouched ? draft.hue : hueFromName(draft.name);
  const set = <K extends keyof EmployeeDraft>(k: K, v: EmployeeDraft[K]) => setD((d) => ({ ...d, [k]: v }));
  const nameMissing = touched && !draft.name.trim();
  const titleMissing = touched && !draft.title.trim();
  const managerName = useMemo(() => {
    if (!draft.managerId) return snap?.company.ceoName ?? "You";
    return snap?.employees.find((e) => e.id === draft.managerId)?.name ?? snap?.people.find((p) => p.id === draft.managerId)?.name ?? "";
  }, [draft.managerId, snap]);
  const grantedNames = draft.skills.keys.map((k) => catalog.data?.find((c) => c.id === k)?.name).filter((x): x is string => !!x);

  const submit = async () => {
    setTouched(true);
    if (!draft.name.trim() || !draft.title.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const toImport = imports.filter((k) => draft.skills.keys.includes(k));
      const skills = { ...draft.skills, keys: draft.skills.keys.filter((k) => !toImport.includes(k)) };
      const final = { ...draft, skills, name: draft.name.trim(), title: draft.title.trim(), hue, workspaces: draft.workspaces.filter((w) => w.trim()) };
      if (proposal) {
        const e = await call("approve_proposal", { proposalId: proposal.id, draft: final, imports: toImport, note: note.trim() || null });
        refreshOrg();
        onClose();
        const briefs = e.managerId === proposal.employeeId ? ` ${proposerName} will brief them.` : "";
        toast(`${firstName(e.name)} joined ${snap?.company.name ?? "the company"}.${briefs}`, "ok", { label: "Open", run: () => openChat(e.id) });
        return;
      }
      const e = await call("hire", { draft: final, imports: toImport });
      const t = TEMPLATES.find((x) => x.id === tpl);
      const lesson = t?.firstLesson || `Welcome aboard, ${firstName(e.name)}. Let's start with the most important thing you'll do for me. Ask me questions until you can write it down as a skill.`;
      setDraft(e.id, lesson);
      refreshOrg();
      onClose();
      toast(`${firstName(e.name)} joined ${snap?.company.name ?? "the company"}`, "ok");
      window.setTimeout(() => openChat(e.id), 380);
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <SlideOver open={open} onClose={onClose} width={760} label={proposal ? "Review a hire proposal" : "Hire an employee"} modal>
      <header className="sheet-head">
        <div className="stack">
          <h2 className="h-lg">{proposal ? `${proposerName}'s hire proposal` : "Hire an employee"}</h2>
          <p className="muted">{proposal
            ? `${proposerName} proposed this hire. Change anything you like, then hire. To say no, decline it from your Inbox.`
            : "An agent on your Claude Code who works on their own and grows skills as you teach them."}</p>
        </div>
        <button className="btn btn-ghost btn-icon" aria-label="Close" onClick={onClose}><Icon name="close" /></button>
      </header>
      <div className="sheet-body">
        <form className="sheet-form" onSubmit={(e) => { e.preventDefault(); void submit(); }} noValidate>
          {proposal ? (
            <div className="sheet-reason">
              <span className="field-label"><Icon name="userPlus" size={12} /> Why {proposerName} proposes it</span>
              <Markdown text={proposal.reason} />
              {proposal.missingSkills.length ? (
                <span className="faint">Also asked for, not in your Claude Code skills: <span className="mono">{proposal.missingSkills.join(", ")}</span>. Install them and grant them later.</span>
              ) : null}
            </div>
          ) : (
          <div className="field">
            <span className="field-label">Start from</span>
            <div className="row" style={{ flexWrap: "wrap", gap: 6 }}>
              {TEMPLATES.map((t) => (
                <button key={t.id} type="button" className="chip" aria-pressed={tpl === t.id} onClick={() => applyTemplate(t.id)}>
                  {t.id === "cos" ? <Icon name="org" size={12} /> : null}
                  {t.title}
                  {t.id === "cos" ? <span className="faint" style={{ fontWeight: 400 }}>manager</span> : null}
                </button>
              ))}
            </div>
          </div>
          )}
          <div className="form-2">
            <label className="field">
              <span className="field-label">Name</span>
              <input className="input" data-autofocus value={draft.name} onChange={(e) => set("name", e.target.value)} placeholder="e.g. Felix" aria-invalid={nameMissing} />
              {nameMissing ? <span className="field-error">Every employee needs a name.</span> : null}
            </label>
            <label className="field">
              <span className="field-label">Title</span>
              <input className="input" value={draft.title} onChange={(e) => set("title", e.target.value)} placeholder="e.g. Cloud Engineer" aria-invalid={titleMissing} />
              {titleMissing ? <span className="field-error">Add a title so the team knows what they do.</span> : null}
            </label>
          </div>
          <label className="field">
            <span className="field-label">Reports to</span>
            {snap ? <EmployeeManagerSelect value={draft.managerId} onChange={(v) => set("managerId", v)} snap={snap} selfId={null} /> : null}
          </label>
          <label className="field">
            <span className="field-label">Description</span>
            <textarea className="textarea" rows={2} value={draft.description} onChange={(e) => set("description", e.target.value)} placeholder="What they look after, in a sentence or two." />
          </label>
          <label className="field">
            <span className="field-label">Objective</span>
            <textarea className="textarea" rows={2} value={draft.objective} onChange={(e) => set("objective", e.target.value)} placeholder="What success looks like, in one sentence." />
            <span className="field-help">Their role skill opens with this, so every task starts from it.</span>
          </label>
          <div className="field">
            <span className="field-label">Tools they can use <span className="hint">from your Claude Code skills</span></span>
            <SkillPicker catalog={catalog.data} keys={draft.skills.keys} builtins={draft.skills.builtins} onChange={(s) => set("skills", s)} imports={imports} onImportsChange={setImports} />
          </div>
          <div className="field">
            <span className="field-label">Clearance</span>
            <ClearancePicker value={draft.clearance} onChange={(v) => set("clearance", v)} />
          </div>
          <AdvancedFields
            value={{ model: draft.model, effort: draft.effort, browser: draft.browser, autoTidy: draft.autoTidy, workspaces: draft.workspaces }}
            onChange={(a) => setD((d) => ({ ...d, ...a }))}
          />
          {proposal ? (
            <label className="field">
              <span className="field-label">Note to {proposerName} <span className="hint">optional</span></span>
              <textarea className="textarea" rows={2} value={note} onChange={(e) => setNote(e.target.value)} placeholder="What to focus on first, or why you changed their proposal." />
            </label>
          ) : null}
          <button type="submit" hidden />
        </form>
        <aside className="sheet-preview" aria-label="Preview">
          <span className="msg-label">Preview</span>
          <div className="preview-card">
            <div className="mcard-head">
              <button
                type="button" className="preview-av" title="Try another colour" aria-label="Try another avatar colour"
                onClick={() => { setHueTouched(true); set("hue", (hue + 47) % 360); }}
              >
                <Avatar name={draft.name || "?"} hue={hue} size={40} />
              </button>
              <div className="stack grow" style={{ minWidth: 0 }}>
                <span className="card-name">{draft.name.trim() || "Their name"}</span>
                <span className="card-title">{draft.title.trim() || "Their title"}</span>
              </div>
            </div>
            <div className="preview-meta">
              <span><Icon name="org" size={12} /> Reports to {managerName}</span>
              <span>{draft.clearance === "supervised" ? <><Icon name="shield" size={12} /> Supervised</> : <><Icon name="bolt" size={12} /> Autonomous</>}</span>
            </div>
            {draft.objective.trim() ? <p className="preview-objective">{draft.objective}</p> : null}
            <div className="preview-skills">
              <span className="preview-skill preview-skill-role mono">{(draft.name.trim().split(/\s+/)[0] || "role").toLowerCase()}</span>
              {grantedNames.slice(0, 6).map((n) => <span key={n} className="preview-skill mono">{n}</span>)}
              {grantedNames.length > 6 ? <span className="preview-skill">+{grantedNames.length - 6}</span> : null}
            </div>
          </div>
          <ol className="preview-steps">
            <li><b>Omega Star writes their role skill</b> from the objective, listing the skills they may use.</li>
            {imports.some((k) => draft.skills.keys.includes(k)) ? <li><b>They organize what you import</b> into their own skill family, first thing. Your originals stay as they are.</li> : null}
            <li><b>You teach them in chat.</b> Every procedure you explain becomes one of their own skills.</li>
            <li><b>Give them a schedule</b> and they run on their own, refining their skills after each run.</li>
          </ol>
        </aside>
      </div>
      <footer className="sheet-foot">
        {error ? <ErrorNote text={error} /> : <span className="faint">You can change all of this later.</span>}
        <span className="grow" />
        <button className="btn btn-ghost" onClick={onClose}>Cancel</button>
        <button className="btn btn-primary" disabled={busy} onClick={() => void submit()}>
          {busy ? "Hiring…" : draft.name.trim() ? `${proposal ? "Approve & hire" : "Hire"} ${firstName(draft.name.trim())}` : proposal ? "Approve & hire" : "Hire"}
        </button>
      </footer>
    </SlideOver>
  );
}
