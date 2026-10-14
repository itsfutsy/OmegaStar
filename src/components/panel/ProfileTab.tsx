import { useMemo, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { Employee, EmployeeView } from "../../api/types";
import { firstName } from "../../lib/format";
import { refreshOrg, useCatalog, useDraft, useOrg, useQuery } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import { AdvancedFields, ClearancePicker, EmployeeManagerSelect } from "../EmployeeFields";
import { up } from "../../lib/org";
import { SkillPicker } from "../SkillPicker";
import { Icon } from "../Icon";
import { Markdown } from "../Markdown";
import { ConfirmButton, ErrorNote, Section, Skeleton } from "../ui";

function toEmployee(v: EmployeeView): Employee {
  const { presence: _p, activeRunId: _a, nextFire: _n, lastRun: _l, ...e } = v;
  return e;
}

export function ProfileTab({ e, onGone }: { e: EmployeeView; onGone: () => void }) {
  const { snap } = useOrg();
  const catalog = useCatalog();
  const base = useMemo(() => toEmployee(e), [e]);
  const { draft: d, setDraft: setD, set, dirty, reset } = useDraft(base);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [rule, setRule] = useState("");
  const name = firstName(e.name);

  const save = async () => {
    setBusy(true);
    setErr(null);
    try {
      await call("update_employee", { employee: { ...d, workspaces: d.workspaces.filter((w) => w.trim()) } });
      refreshOrg();
      toast("Saved", "ok");
    } catch (x) {
      setErr(errorMessage(x));
    } finally {
      setBusy(false);
    }
  };
  const letGo = async () => {
    try {
      await call("let_go", { employeeId: e.id });
      refreshOrg();
      toast(`${name} has left the company`, "ok");
      onGone();
    } catch (x) {
      toast(errorMessage(x), "err");
    }
  };

  const reports = snap ? [...snap.employees, ...snap.people].filter((m) => up(snap, m.id) === e.id) : [];
  const upTo = e.managerId
    ? snap?.employees.find((x) => x.id === e.managerId)?.name ?? snap?.people.find((x) => x.id === e.managerId)?.name ?? "their manager"
    : "you";

  return (
    <div className="fileview">
      <div className="tab-scroll">
        <Section title="About" className="tab-form">
          <div className="form-2">
            <label className="field"><span className="field-label">Name</span><input className="input" value={d.name} onChange={(ev) => set("name", ev.target.value)} /></label>
            <label className="field"><span className="field-label">Title</span><input className="input" value={d.title} onChange={(ev) => set("title", ev.target.value)} /></label>
          </div>
          <label className="field">
            <span className="field-label">Reports to</span>
            {snap ? <EmployeeManagerSelect value={d.managerId} onChange={(v) => set("managerId", v)} snap={snap} selfId={e.id} /> : null}
            <span className="field-help">A manager who is an employee can hand {name} work.</span>
          </label>
          <label className="field"><span className="field-label">Description</span><textarea className="textarea" rows={2} value={d.description} onChange={(ev) => set("description", ev.target.value)} /></label>
          <label className="field">
            <span className="field-label">Objective <span className="hint">what good looks like</span></span>
            <textarea className="textarea" rows={2} value={d.objective} onChange={(ev) => set("objective", ev.target.value)} />
          </label>
        </Section>
        <Owns e={e} name={name} />
        <Section title={`Tools ${name} can use`} action={<span className="section-note">From your Claude Code skills</span>} className="tab-form">
          <SkillPicker catalog={catalog.data} keys={d.skills.keys} builtins={d.skills.builtins} onChange={(s) => set("skills", s)} />
        </Section>
        <Section title="Permissions" className="tab-form">
          <ClearancePicker value={d.clearance} onChange={(v) => set("clearance", v)} />
          <div className="field">
            <span className="field-label">Allowed without asking <span className="hint">{d.clearance === "autonomous" ? "not needed while autonomous" : "everything else is blocked"}</span></span>
            <div className="rules">
              {d.allowRules.length === 0 ? <span className="faint" style={{ fontSize: "var(--t-sm)" }}>Nothing extra yet. When something is blocked, you can allow it from the Inbox.</span> : null}
              {d.allowRules.map((r) => (
                <span key={r} className="rule mono">
                  {r}
                  <button type="button" aria-label={`Remove ${r}`} onClick={() => set("allowRules", d.allowRules.filter((x) => x !== r))}><Icon name="close" size={11} /></button>
                </span>
              ))}
            </div>
            <form className="row" onSubmit={(ev) => { ev.preventDefault(); const r = rule.trim(); if (r && !d.allowRules.includes(r)) set("allowRules", [...d.allowRules, r]); setRule(""); }}>
              <input className="input mono" value={rule} onChange={(ev) => setRule(ev.target.value)} placeholder="e.g. Bash(git status:*)" aria-label="New allow rule" />
              <button className="btn" type="submit" disabled={!rule.trim()}>Add</button>
            </form>
          </div>
        </Section>
        <div className="stack" style={{ gap: 18, marginTop: 24 }}>
          <AdvancedFields value={{ model: d.model, effort: d.effort, browser: d.browser, autoTidy: d.autoTidy, workspaces: d.workspaces }} onChange={(a) => setD((x) => ({ ...x, ...a }))} />
          <div className="danger">
            <div className="stack grow">
              <span className="danger-title">Let {name} go</span>
              <span className="danger-desc">
                {reports.length ? `${reports.map((r) => firstName(r.name)).join(", ")} will report to ${upTo}. ` : ""}
                Their schedules stop. Their past runs stay in Activity.
              </span>
            </div>
            <ConfirmButton label="Let go" className="btn btn-danger" question={`Let ${name} go?`} confirmLabel="Let go" onConfirm={letGo} />
          </div>
        </div>
      </div>
      {dirty || err ? (
        <div className="savebar">
          {err ? <ErrorNote text={err} /> : <span className="muted" style={{ fontSize: "var(--t-sm)" }}>Unsaved changes</span>}
          <span className="grow" />
          <button className="btn btn-ghost" onClick={() => { reset(); setErr(null); }}>Discard</button>
          <button className="btn btn-primary" disabled={busy || !dirty} onClick={() => void save()}>{busy ? "Saving…" : "Save"}</button>
        </div>
      ) : null}
    </div>
  );
}

/** What the employee owns: a file in their folder that every run reads, recorded so Undo works. */
function Owns({ e, name }: { e: EmployeeView; name: string }) {
  const q = useQuery(
    () => call("read_skill_file", { employeeId: e.id, skill: "@own", file: "what-you-own.md" }).catch((x: unknown) => {
      if (typeof x === "object" && x !== null && "code" in x && x.code === "notFound") return null;
      throw x;
    }),
    [e.id],
    ["run:updated"],
  );
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const save = async () => {
    if (text === null) return;
    setBusy(true);
    setErr(null);
    try {
      const rev = await call("save_skill_file", { employeeId: e.id, skill: "@own", file: "what-you-own.md", text });
      q.set(text);
      setText(null);
      toast(rev ? "Saved. You can undo it under Changes in the Skills tab." : "No changes to save.", rev ? "ok" : "neutral");
    } catch (x) {
      setErr(errorMessage(x));
    } finally {
      setBusy(false);
    }
  };

  const edit = text === null && q.data !== undefined ? <button className="btn btn-sm" onClick={() => { setText(q.data ?? ""); setErr(null); }}><Icon name="edit" size={12} /> Edit</button> : null;
  return (
    <Section title={`What ${name} owns`} action={edit}>
      <span className="field-help">The number {name} answers for, their limits, and what they decide alone. Only a conversation with you changes it.</span>
      {q.error ? <ErrorNote text={q.error} onRetry={q.reload} /> : null}
      {q.data === undefined && !q.error ? <Skeleton h={72} r={12} /> : null}
      {text !== null ? (
        <>
          <textarea className="textarea mono" rows={8} value={text} onChange={(ev) => setText(ev.target.value)} spellCheck={false} autoFocus
            onKeyDown={(ev) => { if ((ev.ctrlKey || ev.metaKey) && ev.key === "s") { ev.preventDefault(); void save(); } }} />
          <div className="row" style={{ gap: 8 }}>
            {err ? <ErrorNote text={err} /> : null}
            <span className="grow" />
            <button className="btn btn-ghost" onClick={() => setText(null)}>Cancel</button>
            <button className="btn btn-primary" disabled={busy} onClick={() => void save()}>{busy ? "Saving…" : "Save"}</button>
          </div>
        </>
      ) : q.data?.trim() ? (
        <div className="fileview-doc"><Markdown text={q.data} /></div>
      ) : q.data !== undefined ? (
        <span className="faint" style={{ fontSize: "var(--t-sm)" }}>Not set yet. {name} proposes it in their first weekly review.</span>
      ) : null}
    </Section>
  );
}
