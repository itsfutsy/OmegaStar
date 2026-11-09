import { useState } from "react";
import type { Clearance, Company, Effort, OrgSnapshot, PersonManager, PersonView } from "../api/types";
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

/** Exact model IDs, passed to `claude --model` as they are. Newest first within a family. */
const MODEL_GROUPS: { label: string; models: { value: string; label: string }[] }[] = [
  { label: "Fable", models: [{ value: "claude-fable-5-1", label: "Fable 5.1" }, { value: "claude-fable-5", label: "Fable 5" }] },
  {
    label: "Opus",
    models: [
      { value: "claude-opus-5-5", label: "Opus 5.5" }, { value: "claude-opus-5", label: "Opus 5" }, { value: "claude-opus-4-8", label: "Opus 4.8" },
      { value: "claude-opus-4-7", label: "Opus 4.7" }, { value: "claude-opus-4-6", label: "Opus 4.6" },
    ],
  },
  {
    label: "Sonnet",
    models: [{ value: "claude-sonnet-5-5", label: "Sonnet 5.5" }, { value: "claude-sonnet-5", label: "Sonnet 5" }, { value: "claude-sonnet-4-6", label: "Sonnet 4.6" }],
  },
  { label: "Haiku", models: [{ value: "claude-haiku-4-5", label: "Haiku 4.5" }] },
];
/** What earlier versions of Omega Star saved. Claude Code resolves these to whatever is newest, so they
 *  are only offered while an employee still has one; picking a specific version replaces it. */
const MODEL_ALIASES: { value: string; label: string }[] = [
  { value: "fable", label: "Fable (whatever is newest)" }, { value: "opus", label: "Opus (whatever is newest)" },
  { value: "sonnet", label: "Sonnet (whatever is newest)" }, { value: "haiku", label: "Haiku (whatever is newest)" },
];
const MODEL_DEFAULT = "";
const MODEL_OTHER = "__other__";
const KNOWN_MODELS = new Set(MODEL_GROUPS.flatMap((g) => g.models.map((m) => m.value)));
const EFFORTS: { value: Effort | null; label: string }[] = [
  { value: null, label: "Default" }, { value: "low", label: "Low" }, { value: "medium", label: "Medium" },
  { value: "high", label: "High" }, { value: "xhigh", label: "Extra" }, { value: "max", label: "Max" },
];

type Advanced = { model: string | null; effort: Effort | null; browser: boolean; autoTidy: boolean; workspaces: string[] };

export function AdvancedFields({ value, onChange }: { value: Advanced; onChange: (v: Advanced) => void }) {
  const [open, setOpen] = useState(false);
  const alias = MODEL_ALIASES.find((a) => a.value === value.model);
  // "Other" stays chosen while its ID is being typed, even if that text happens to match a listed model.
  const [other, setOther] = useState(() => value.model !== null && !alias && !KNOWN_MODELS.has(value.model));
  const modelChoice = other ? MODEL_OTHER : value.model ?? MODEL_DEFAULT;
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
            <label className="field">
              <span className="field-label">Model</span>
              <select
                className="select" value={modelChoice}
                onChange={(e) => {
                  const v = e.target.value;
                  setOther(v === MODEL_OTHER);
                  // "Other" starts from an empty ID rather than the model it replaces, which would be typed onto.
                  onChange({ ...value, model: v === MODEL_DEFAULT || v === MODEL_OTHER ? null : v });
                }}
              >
                <option value={MODEL_DEFAULT}>Default (your Claude Code setting)</option>
                {MODEL_GROUPS.map((g) => (
                  <optgroup key={g.label} label={g.label}>
                    {g.models.map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
                  </optgroup>
                ))}
                {alias ? <optgroup label="Saved earlier"><option value={alias.value}>{alias.label}</option></optgroup> : null}
                <option value={MODEL_OTHER}>Other model ID…</option>
              </select>
              {other ? (
                <input
                  className="input mono" placeholder="claude-opus-4-1" aria-label="Model ID" value={value.model ?? ""} spellCheck={false} autoFocus
                  onChange={(e) => onChange({ ...value, model: e.target.value.trim() === "" ? null : e.target.value.trim() })}
                />
              ) : null}
            </label>
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
          <div className="field">
            <span className="field-label">Workspace folders <span className="hint">they can read and edit these</span></span>
            <div className="stack" style={{ gap: 6 }}>
              {value.workspaces.map((w, i) => (
                <div key={i} className="row">
                  <span className="input-affix grow">
                    <span className="affix"><Icon name="folder" size={13} /></span>
                    <input className="input mono" value={w} placeholder="C:\\work\\project" style={{ paddingLeft: 30 }}
                      onChange={(e) => onChange({ ...value, workspaces: value.workspaces.map((x, j) => (j === i ? e.target.value : x)) })} />
                  </span>
                  <button type="button" className="btn btn-ghost btn-icon" aria-label="Remove folder" onClick={() => onChange({ ...value, workspaces: value.workspaces.filter((_, j) => j !== i) })}>
                    <Icon name="close" size={14} />
                  </button>
                </div>
              ))}
              <button type="button" className="btn btn-sm btn-ghost" style={{ alignSelf: "flex-start" }} onClick={() => onChange({ ...value, workspaces: [...value.workspaces, ""] })}>
                <Icon name="plus" size={12} /> Add folder
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
