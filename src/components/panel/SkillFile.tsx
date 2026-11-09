import { useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView, OwnSkill, Run, SkillRevision } from "../../api/types";
import { dateLong, firstName, recordedName, runKindLabel } from "../../lib/format";
import { refreshOrg, useQuery } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import { openRun } from "../../lib/router";
import { Diff, lineStats } from "../Diff";
import { Icon } from "../Icon";
import { Markdown, splitFrontmatter } from "../Markdown";
import { ConfirmButton, ErrorNote, Section, Skeleton } from "../ui";
import { TimelineItem } from "./SkillsOverview";

const folder = (name: string): OwnSkill => ({ name, description: "", role: false, managed: false, files: [], updatedAt: "" });
/** Files recorded like skills that are not skills: a tidy's notes in workspace/, the company page, and what the employee owns. */
const PSEUDO_FOLDERS: Record<string, OwnSkill> = { "@workspace": folder("@workspace"), "@company": folder("@company"), "@own": folder("@own") };
export const pseudoFolder = (name: string): OwnSkill | undefined => (name.startsWith("@") ? PSEUDO_FOLDERS[name] : undefined);

export function FileView({ e, skill, file, history, onBack, onOpenRev, onChanged }: {
  e: EmployeeView; skill: OwnSkill | undefined; file: string; history: SkillRevision[];
  onBack: () => void; onOpenRev: (id: string) => void; onChanged: () => void;
}) {
  const name = skill?.name ?? "";
  const q = useQuery(() => (skill ? call("read_skill_file", { employeeId: e.id, skill: name, file }) : Promise.reject({ code: "gone", message: "This skill no longer exists." })), [e.id, name, file], ["run:updated"]);
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const fileHistory = history.filter((r) => r.skill === name && r.file === file);

  const startEdit = () => { setText(q.data ?? ""); setEditing(true); setErr(null); };
  const save = async () => {
    setSaving(true);
    setErr(null);
    try {
      const rev = await call("save_skill_file", { employeeId: e.id, skill: name, file, text });
      setEditing(false);
      q.set(text);
      onChanged();
      refreshOrg();
      toast(rev ? "Saved. You can undo it under Changes." : "No changes to save.", rev ? "ok" : "neutral");
    } catch (x) {
      setErr(errorMessage(x));
    } finally {
      setSaving(false);
    }
  };
  const promote = async () => {
    try {
      const info = await call("promote_skill", { employeeId: e.id, skill: name });
      toast(`${info.name} is now a company skill anyone can be given`, "ok");
    } catch (x) {
      toast(errorMessage(x), "err");
    }
  };

  const parsed = q.data !== undefined ? splitFrontmatter(q.data) : null;
  const isMd = file.endsWith(".md");
  const shown = recordedName(name);
  return (
    <div className="fileview">
      <div className="tab-scroll">
        <nav className="crumbs" aria-label="Breadcrumb">
          <button onClick={onBack}>Skills</button><Icon name="chevronRight" size={11} />
          <button onClick={onBack} className="mono">{shown}</button><Icon name="chevronRight" size={11} />
          <span className="crumb-here mono">{file}</span>
        </nav>
        <div className="fileview-head">
          <div className="stack grow" style={{ minWidth: 0 }}>
            <span className="row" style={{ gap: 8 }}>
              <h3 className="h-md mono fileview-title">{shown}</h3>
              {skill?.role ? <span className="pill pill-accent">Role</span> : null}
              {skill?.managed ? <span className="pill"><Icon name="lock" size={11} /> Managed</span> : null}
            </span>
            {parsed?.meta.description ? <p className="fileview-desc">{parsed.meta.description}</p> : null}
          </div>
          {!skill?.managed && !editing ? (
            <div className="row" style={{ gap: 4 }}>
              {skill && !skill.role && !pseudoFolder(name) ? <ConfirmButton label="Make a company skill" icon="promote" className="btn btn-ghost" size="btn-sm" question="Copy it into your Claude Code skills?" confirmLabel="Copy it" onConfirm={promote} /> : null}
              <button className="btn btn-sm" onClick={startEdit} disabled={q.data === undefined}><Icon name="edit" size={12} /> Edit</button>
            </div>
          ) : null}
        </div>
        {skill?.managed ? <p className="fileview-lock"><Icon name="lock" size={12} /> Omega Star rewrites this before every run. It's how {firstName(e.name)} learns to write skills, so it can't be edited here.</p> : null}
        {q.error ? <ErrorNote text={q.error} onRetry={q.reload} /> : null}
        {q.data === undefined && !q.error ? <div className="stack" style={{ gap: 8 }}><Skeleton /><Skeleton w="85%" /><Skeleton w="70%" /><Skeleton w="90%" /></div> : null}
        {editing ? (
          <textarea className="textarea mono fileview-editor" value={text} onChange={(ev) => setText(ev.target.value)} spellCheck={false} autoFocus
            onKeyDown={(ev) => { if ((ev.ctrlKey || ev.metaKey) && ev.key === "s") { ev.preventDefault(); void save(); } }} />
        ) : parsed ? (
          <div className="fileview-doc">{isMd ? <Markdown text={parsed.body} /> : <pre className="mono">{q.data}</pre>}</div>
        ) : null}
        {!editing && fileHistory.length > 0 ? (
          <Section title="History of this file" className="fileview-history">
            <ol className="timeline">
              {fileHistory.map((r) => <TimelineItem key={r.id} r={r} run={undefined} onOpen={() => onOpenRev(r.id)} />)}
            </ol>
          </Section>
        ) : null}
      </div>
      {editing ? (
        <div className="savebar enter-rise">
          {err ? <ErrorNote text={err} /> : <span className="muted" style={{ fontSize: "var(--t-sm)" }}>You can undo this later under Changes.</span>}
          <span className="grow" />
          <button className="btn btn-ghost" onClick={() => setEditing(false)}>Cancel</button>
          <button className="btn btn-primary" disabled={saving} onClick={() => void save()}>{saving ? "Saving…" : "Save"}</button>
        </div>
      ) : null}
    </div>
  );
}

export function RevisionView({ e, rev, run, onBack, onOpenFile, onChanged }: {
  e: EmployeeView; rev: SkillRevision | undefined; run: Run | undefined;
  onBack: () => void; onOpenFile: (skill: string, file: string) => void; onChanged: () => void;
}) {
  if (!rev) {
    return <div className="tab-scroll"><div className="empty"><span className="empty-title">That change isn't in the history</span><p>The link points to a change this office no longer has. The skill itself is unchanged.</p><button className="btn" onClick={onBack}>Back to skills</button></div></div>;
  }
  const verb = rev.change === "added" ? "Created" : rev.change === "removed" ? "Removed" : "Changed";
  const stats = lineStats(rev.before, rev.after);
  const revert = async () => {
    try {
      await call("revert_revision", { revisionId: rev.id });
      refreshOrg();
      toast(`Undone. ${pseudoFolder(rev.skill) ? "It's" : `${firstName(e.name)}'s skill is`} back to how it was. The undo is listed under Changes too.`, "ok");
      onChanged();
    } catch (x) {
      toast(errorMessage(x), "err");
    }
  };
  return (
    <div className="tab-scroll">
      <nav className="crumbs" aria-label="Breadcrumb">
        <button onClick={onBack}>Skills</button><Icon name="chevronRight" size={11} />
        <span className="crumb-here">{verb} {recordedName(rev.skill)}</span>
      </nav>
      <div className="rev-head">
        <div className="stack grow" style={{ minWidth: 0, gap: 4 }}>
          <h3 className="h-md">{verb} <button className="mono linkish rev-file" onClick={() => onOpenFile(rev.skill, rev.file)}>{pseudoFolder(rev.skill) ? rev.file : `${rev.skill}/${rev.file}`}</button></h3>
          <span className="rev-sub">
            {rev.runId ? (
              <button className="linkish" onClick={() => rev.runId && openRun(rev.runId)}>
                {run ? (run.kind.type === "chat" ? "Learned in a conversation" : runKindLabel(run.kind)) : "During a run"} <Icon name="arrowUpRight" size={11} />
              </button>
            ) : "Edited by you"}
            <span className="muted"> · {dateLong(rev.at)}</span>
            <span className="mono num rev-lines">{stats.added ? <span className="tl-add">+{stats.added}</span> : null} {stats.removed ? <span className="tl-del">−{stats.removed}</span> : null}</span>
          </span>
        </div>
        <ConfirmButton label="Undo change" icon="undo" className="btn" size="btn-sm" question="Undo this change?" confirmLabel="Undo it" onConfirm={revert} />
      </div>
      {run?.kind.type === "chat" ? <p className="rev-prompt">You said: “{run.prompt}”</p> : null}
      <Diff before={rev.before} after={rev.after} />
    </div>
  );
}
