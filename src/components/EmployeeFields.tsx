import { useState } from "react";
import { call } from "../api/client";
import type { Clearance, Company, Effort, ModelOption, OrgSnapshot, PersonManager, PersonView } from "../api/types";
import { folderNote, useFolderChecks } from "../lib/folders";
import { useModels } from "../lib/hooks";
import { aboveYou, employeeManagerOptions, personManagerOptions } from "../lib/org";
import { Icon } from "./Icon";
import { Switch } from "./ui";

/** "Name (you, CIO)": how you appear in every "Reports to" picker. */
const youLabel = (c: Company) => `${c.ceoName} (you, ${c.title})`;

type Groups = { below: { id: string; label: string }[]; above: { id: string; label: string }[]; employees: { id: string; label: string }[] };

function groupsOf(snap: OrgSnapshot, ids: Set<string>): Groups {
  const above = aboveYou(snap);
  const person = (p: PersonView) => ({ id: p.id, label: `${p.name}${p.title ? ` · ${p.title}` : ""}` });
  return {
    employees: snap.employees.filter((e) => ids.has(e.id)).map((e) => ({ id: e.id, label: `${e.name} · ${e.title}` })),
    below: snap.people.filter((p) => ids.has(p.id) && !above.has(p.id)).map(person),
    above: snap.people.filter((p) => ids.has(p.id) && above.has(p.id)).map(person),
  };
}

function MemberOptions({ g }: { g: Groups }) {
  return (
    <>
      {g.employees.length ? <optgroup label="Employees">{g.employees.map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}</optgroup> : null}
      {g.below.length ? <optgroup label="People">{g.below.map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}</optgroup> : null}
      {g.above.length ? <optgroup label="Above you">{g.above.map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}</optgroup> : null}
    </>
  );
}

/** An employee's manager: you or anyone below you. Never someone above you (see lib/org). */
export function EmployeeManagerSelect({ value, onChange, snap, selfId }: {
  value: string | null; onChange: (v: string | null) => void; snap: OrgSnapshot; selfId: string | null;
}) {
  const allowed = employeeManagerOptions(snap, selfId);
  const ids = new Set(allowed.filter((m): m is string => m !== null));
  return (
    <select className="select" value={value ?? ""} onChange={(e) => onChange(e.target.value || null)}>
      <option value="">{youLabel(snap.company)}</option>
      <MemberOptions g={groupsOf(snap, ids)} />
    </select>
  );
}

const TOP_VALUE = "__top__";
const encodeManager = (m: PersonManager) => (m.type === "you" ? "" : m.type === "top" ? TOP_VALUE : m.id);
const decodeManager = (v: string): PersonManager => (v === "" ? { type: "you" } : v === TOP_VALUE ? { type: "top" } : { type: "member", id: v });

/** A person's manager: you, a member, or (only for the top of your line) nobody. */
export function PersonManagerSelect({ value, onChange, snap, selfId }: {
  value: PersonManager; onChange: (v: PersonManager) => void; snap: OrgSnapshot; selfId: string | null;
}) {
  const allowed = personManagerOptions(snap, selfId);
  const ids = new Set(allowed.flatMap((m) => (m.type === "member" ? [m.id] : [])));
  return (
    <select className="select" value={encodeManager(value)} onChange={(e) => onChange(decodeManager(e.target.value))}>
      {allowed.some((m) => m.type === "you") ? <option value="">{youLabel(snap.company)}</option> : null}
      {allowed.some((m) => m.type === "top") ? <option value={TOP_VALUE}>Nobody, top of the company</option> : null}
      <MemberOptions g={groupsOf(snap, ids)} />
    </select>
  );
}

export function ClearancePicker({ value, onChange }: { value: Clearance; onChange: (v: Clearance) => void }) {
  return (
    <div className="clearance" role="radiogroup" aria-label="Clearance">
      <button type="button" role="radio" aria-checked={value === "supervised"} className="clearance-opt" onClick={() => onChange("supervised")}>
        <span className="clearance-icon"><Icon name="shield" size={15} /></span>
        <span className="stack">
          <span className="clearance-title">Supervised <span className="pill pill-outline" style={{ height: 18, fontSize: 10.5 }}>Recommended</span></span>
          <span className="clearance-desc">Edits files in their own workspace and uses the tools you've allowed. Anything else is blocked and shown to you with an "Allow" button.</span>
        </span>
      </button>
      <button type="button" role="radio" aria-checked={value === "autonomous"} className="clearance-opt" onClick={() => onChange("autonomous")}>
        <span className="clearance-icon"><Icon name="bolt" size={15} /></span>
        <span className="stack">
          <span className="clearance-title">Autonomous</span>
          <span className="clearance-desc">Runs any command without asking.</span>
          {value === "autonomous" ? (
            <span className="clearance-warn"><Icon name="alert" size={12} /> Nothing is blocked, including deleting files or spending money. Choose this only for work you'd let a trusted colleague do unwatched.</span>
          ) : null}
        </span>
      </button>
    </div>
  );
}

/** What earlier versions of Omega Star saved. Claude Code resolves these to whatever is newest, so they
 *  are only offered while an employee still has one; picking a specific version replaces it. */
const MODEL_ALIASES: { value: string; label: string }[] = [
  { value: "fable", label: "Fable (whatever is newest)" }, { value: "opus", label: "Opus (whatever is newest)" },
  { value: "sonnet", label: "Sonnet (whatever is newest)" }, { value: "haiku", label: "Haiku (whatever is newest)" },
];
const MODEL_DEFAULT = "";
const MODEL_OTHER = "__other__";
const EFFORTS: { value: Effort | null; label: string }[] = [
  { value: null, label: "Default" }, { value: "low", label: "Low" }, { value: "medium", label: "Medium" },
  { value: "high", label: "High" }, { value: "xhigh", label: "Extra" }, { value: "max", label: "Max" },
];

type Advanced = { model: string | null; effort: Effort | null; browser: boolean; autoTidy: boolean; workspaces: string[] };

/** Claude Code's models by family ("Opus 5.5" is an Opus), families and models in the order it lists them. */
function byFamily(models: ModelOption[]): [string, ModelOption[]][] {
  const families = new Map<string, ModelOption[]>();
  for (const m of models) {
    const family = m.name.split(" ")[0];
    families.set(family, [...(families.get(family) ?? []), m]);
  }
  return [...families];
}

/** The model an employee runs on: one from Claude Code's own list, saved as its exact ID, which goes to
 *  `claude --model` as it is. */
function ModelField({ value, onChange }: { value: string | null; onChange: (model: string | null) => void }) {
  const { models, failed } = useModels();
  const alias = MODEL_ALIASES.find((a) => a.value === value);
  // "Other" stays chosen while its ID is being typed, even if that text happens to match a listed model.
  const [typing, setTyping] = useState(false);
  const unlisted = value !== null && !alias && !models?.some((m) => m.id === value);
  // A saved model the list lacks is an "Other" one. Until the list is in, it is shown as its ID alone.
  const other = typing || (unlisted && (models !== undefined || failed));
  return (
    <label className="field">
      <span className="field-label">Model</span>
      <select
        className="select" value={other ? MODEL_OTHER : value ?? MODEL_DEFAULT}
        onChange={(e) => {
          const v = e.target.value;
          setTyping(v === MODEL_OTHER);
          // "Other" starts from an empty ID rather than the model it replaces, which would be typed onto.
          onChange(v === MODEL_DEFAULT || v === MODEL_OTHER ? null : v);
        }}
      >
        <option value={MODEL_DEFAULT}>Default (your Claude Code setting)</option>
        {byFamily(models ?? []).map(([family, list]) => (
          <optgroup key={family} label={family}>
            {list.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
          </optgroup>
        ))}
        {unlisted && !other ? <option value={value}>{value}</option> : null}
        {alias ? <optgroup label="Saved earlier"><option value={alias.value}>{alias.label}</option></optgroup> : null}
        <option value={MODEL_OTHER}>Other model ID…</option>
      </select>
      {other ? (
        <input
          className="input mono" placeholder="claude-opus-4-1" aria-label="Model ID" value={value ?? ""} spellCheck={false} autoFocus={typing}
          onChange={(e) => {
            setTyping(true);
            onChange(e.target.value.trim() === "" ? null : e.target.value.trim());
          }}
        />
      ) : null}
      {failed ? <span className="field-help">Couldn't get the model list from Claude Code. Choose "Other model ID…" to type one.</span> : null}
    </label>
  );
}

export function AdvancedFields({ value, onChange }: { value: Advanced; onChange: (v: Advanced) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div className={`advanced ${open ? "advanced-open" : ""}`}>
      <button type="button" className="advanced-toggle" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <Icon name="chevronRight" size={13} className="advanced-chev" />
        Advanced
        <span className="advanced-summary">Model, effort, browser, tidying, folders</span>
      </button>
      {open ? (
        <div className="advanced-body">
          <div className="form-2">
            <ModelField value={value.model} onChange={(model) => onChange({ ...value, model })} />
            <div className="field">
              <span className="field-label">Browser control</span>
              <span className="row" style={{ justifyContent: "space-between", alignItems: "flex-start", gap: 12 }}>
                <span className="field-help" style={{ margin: 0 }}>Lets them drive your Chrome, signed in as you, through the Claude in Chrome extension.</span>
                <Switch checked={value.browser} onChange={(v) => onChange({ ...value, browser: v })} label="Browser control" />
              </span>
            </div>
          </div>
          <div className="field">
            <span className="field-label">Tidy on its own</span>
            <span className="row" style={{ justifyContent: "space-between", alignItems: "flex-start", gap: 12 }}>
              <span className="field-help" style={{ margin: 0 }}>Now and then, when Omega Star finds something worth it, they follow up what they promised to check, make sure what you taught them is written down, and clean up their skills. Bigger changes wait for your OK.</span>
              <Switch checked={value.autoTidy} onChange={(v) => onChange({ ...value, autoTidy: v })} label="Tidy on its own" />
            </span>
          </div>
          <div className="field">
            <span className="field-label">Effort</span>
            <div className="seg" role="group" aria-label="Effort">
              {EFFORTS.map((ef) => (
                <button key={ef.label} type="button" aria-pressed={value.effort === ef.value} onClick={() => onChange({ ...value, effort: ef.value })}>{ef.label}</button>
              ))}
            </div>
            <span className="field-help">More effort thinks longer and uses more of your limits. Default suits most roles.</span>
          </div>
          <WorkspaceFolders value={value.workspaces} onChange={(workspaces) => onChange({ ...value, workspaces })} />
        </div>
      ) : null}
    </div>
  );
}

/** The folders outside their own they can read and edit: each typed or chosen, with a line saying whether it's there. */
function WorkspaceFolders({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const checks = useFolderChecks(value);
  const choose = async () => {
    const picked = await call("pick_folder").catch(() => null);
    if (picked && !value.includes(picked)) onChange([...value.filter((w) => w.trim()), picked]);
  };
  return (
    <div className="field">
      <span className="field-label">Workspace folders <span className="hint">they can read and edit these</span></span>
      <div className="stack" style={{ gap: 6 }}>
        {value.map((w, i) => {
          const note = folderNote(w, checks.get(w));
          return (
            <div key={i} className="stack" style={{ gap: 2 }}>
              <div className="row">
                <span className="input-affix grow">
                  <span className="affix"><Icon name="folder" size={13} /></span>
                  <input className="input mono" value={w} placeholder="~/dev/project" style={{ paddingLeft: 30 }} aria-label={`Workspace folder ${i + 1}`}
                    aria-invalid={note?.tone === "err" || undefined}
                    onChange={(e) => onChange(value.map((x, j) => (j === i ? e.target.value : x)))} />
                </span>
                <button type="button" className="btn btn-ghost btn-icon" aria-label="Remove folder" onClick={() => onChange(value.filter((_, j) => j !== i))}>
                  <Icon name="close" size={14} />
                </button>
              </div>
              {note ? (
                <span className={`folder-note folder-note-${note.tone}`} role="status">
                  <Icon name={note.tone === "ok" ? "check" : "alert"} size={12} />{note.text}
                </span>
              ) : null}
            </div>
          );
        })}
        <div className="row" style={{ gap: 6 }}>
          <button type="button" className="btn btn-sm btn-ghost" onClick={() => void choose()}>
            <Icon name="folder" size={12} /> Choose folder…
          </button>
          <button type="button" className="btn btn-sm btn-ghost" onClick={() => onChange([...value, ""])}>
            <Icon name="plus" size={12} /> Type a path
          </button>
        </div>
      </div>
    </div>
  );
}
