import { useMemo, useState } from "react";
import { call, errorMessage } from "../api/client";
import type { OrgSnapshot, PersonDraft, PersonManager } from "../api/types";
import { refreshOrg, useOrg } from "../lib/hooks";
import { toast } from "../lib/toasts";
import { openMember } from "../lib/router";
import { channelLabel } from "../lib/channels";
import { firstName, hueFromName } from "../lib/format";
import { PersonManagerSelect } from "./EmployeeFields";
import { aboveYou } from "../lib/org";
import { ChannelEditor } from "./ChannelEditor";
import { ImageField } from "./ImageField";
import { CHANNEL_ICON, Icon } from "./Icon";
import { Avatar, ErrorNote, SlideOver } from "./ui";
import "./Sheet.css";
import "./AddPersonSheet.css";

const empty = (managerId: string | null): PersonDraft => ({
  name: "", title: "", description: "", strengths: "", responsibilities: "", channels: [], notes: "",
  manager: managerId === null ? { type: "you" } : { type: "member", id: managerId }, hue: 180,
});

/** "Maya", "you", or "nobody (top of the company)". */
export function personManagerName(m: PersonManager, snap: OrgSnapshot | null): string {
  switch (m.type) {
    case "you": return "you";
    case "top": return "nobody (top of the company)";
    case "member": return snap?.employees.find((e) => e.id === m.id)?.name ?? snap?.people.find((p) => p.id === m.id)?.name ?? "someone";
  }
}

export function PersonFields({ draft, set, selfId }: { draft: PersonDraft; set: <K extends keyof PersonDraft>(k: K, v: PersonDraft[K]) => void; selfId: string | null }) {
  const { snap } = useOrg();
  return (
    <>
      <ImageField name={draft.name} hue={draft.hue} image={draft.image} person onChange={(v) => set("image", v)} />
      <div className="form-2">
        <label className="field">
          <span className="field-label">Name</span>
          <input className="input" data-autofocus={selfId === null || undefined} value={draft.name} onChange={(e) => set("name", e.target.value)} placeholder="e.g. Grace Liu" />
        </label>
        <label className="field">
          <span className="field-label">Title</span>
          <input className="input" value={draft.title} onChange={(e) => set("title", e.target.value)} placeholder="e.g. IT Support Lead" />
        </label>
      </div>
      <label className="field">
        <span className="field-label">Reports to</span>
        {snap ? <PersonManagerSelect value={draft.manager} onChange={(v) => set("manager", v)} snap={snap} selfId={selfId} /> : null}
        {snap && selfId && aboveYou(snap).has(selfId) ? (
          <span className="field-help">{firstName(draft.name || "They")} {snap.company.reportsTo === selfId ? "is who you report to" : "sits above you"}, so agents can't report to them. Change who you report to from your own card on the org chart.</span>
        ) : null}
      </label>
      <label className="field">
        <span className="field-label">About them</span>
        <textarea className="textarea" rows={2} value={draft.description} onChange={(e) => set("description", e.target.value)} placeholder="What they do here, in a sentence or two." />
      </label>
      <div className="form-2">
        <label className="field">
          <span className="field-label">Strengths</span>
          <textarea className="textarea" rows={3} value={draft.strengths} onChange={(e) => set("strengths", e.target.value)} placeholder="What to go to them for." />
        </label>
        <label className="field">
          <span className="field-label">Core responsibilities</span>
          <textarea className="textarea" rows={3} value={draft.responsibilities} onChange={(e) => set("responsibilities", e.target.value)} placeholder="What they own and decide." />
        </label>
      </div>
      <div className="field">
        <span className="field-label">How to reach them</span>
        <span className="field-help" style={{ marginTop: -2, marginBottom: 4 }}>Your employees pick the channel whose guidance fits.</span>
        <ChannelEditor value={draft.channels} onChange={(v) => set("channels", v)} />
      </div>
      <label className="field">
        <span className="field-label">Notes <span className="hint">optional</span></span>
        <textarea className="textarea" rows={2} value={draft.notes} onChange={(e) => set("notes", e.target.value)} placeholder="Working hours, preferences, anything that helps." />
      </label>
    </>
  );
}

export function AddPersonSheet({ open, managerId, onClose }: { open: boolean; managerId: string | null; onClose: () => void }) {
  const { snap } = useOrg();
  const [draft, setD] = useState<PersonDraft>(() => empty(managerId));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const set = <K extends keyof PersonDraft>(k: K, v: PersonDraft[K]) => setD((d) => ({ ...d, [k]: v }));
  const hue = hueFromName(draft.name);
  const managerName = useMemo(() => (draft.manager.type === "you" ? snap?.company.ceoName ?? "You" : personManagerName(draft.manager, snap)), [draft.manager, snap]);

  const submit = async () => {
    if (!draft.name.trim()) { setError("Add their name first."); return; }
    setBusy(true);
    setError(null);
    try {
      const p = await call("add_person", { draft: { ...draft, name: draft.name.trim(), title: draft.title.trim(), hue, channels: draft.channels.filter((c) => c.address.trim()) } });
      refreshOrg();
      onClose();
      toast(`${firstName(p.name)} is in the org chart`, "ok");
      window.setTimeout(() => openMember(p.id), 380);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <SlideOver open={open} onClose={onClose} width={760} label="Add a person" modal>
      <header className="sheet-head">
        <div className="stack">
          <h2 className="h-lg">Add a person</h2>
          <p className="muted">Someone on your real team. Your employees see who they are, what they're good at, and how to ask them for help.</p>
        </div>
        <button className="btn btn-ghost btn-icon" aria-label="Close" onClick={onClose}><Icon name="close" /></button>
      </header>
      <div className="sheet-body person-sheet">
        <form className="sheet-form" onSubmit={(e) => { e.preventDefault(); void submit(); }} noValidate>
          <PersonFields draft={draft} set={set} selfId={null} />
          <button type="submit" hidden />
        </form>
        <aside className="sheet-preview" aria-label="Preview">
          <span className="msg-label">Preview</span>
          <div className="preview-card preview-card-person">
            <div className="preview-head">
              {draft.name.trim() || draft.image ? <Avatar name={draft.name} hue={hue} image={draft.image} size={40} person /> : <span className="av av-person" style={{ "--h": hue, width: 40, height: 40, flex: "none" } as React.CSSProperties} aria-hidden />}
              <div className="stack grow" style={{ minWidth: 0 }}>
                <span className="card-name">{draft.name.trim() || "Their name"}</span>
                <span className="card-title">{draft.title.trim() || "Their title"}</span>
              </div>
              <span className="tag" style={{ alignSelf: "flex-start" }}>Person</span>
            </div>
            <div className="preview-meta"><span><Icon name="org" size={12} /> Reports to {managerName}</span></div>
            {draft.strengths.trim() ? <p className="preview-objective">{draft.strengths}</p> : null}
            {draft.channels.length ? (
              <div className="stack" style={{ gap: 6 }}>
                {draft.channels.map((c) => (
                  <div key={c.id} className="preview-channel">
                    <Icon name={CHANNEL_ICON[c.kind]} size={13} />
                    <span className="stack" style={{ minWidth: 0 }}>
                      <span className="preview-channel-addr">{c.address || channelLabel(c.kind)}</span>
                      {c.whenToUse ? <span className="preview-channel-when">{c.whenToUse}</span> : null}
                    </span>
                  </div>
                ))}
              </div>
            ) : null}
          </div>
          <ol className="preview-steps">
            <li><b>Every employee learns who they are</b>, their strengths and what they own.</li>
            <li><b>When an employee is stuck</b>, they send one clear request through the channel that fits.</li>
            <li><b>Their reply resumes the work.</b> You can answer for them from the Activity page too.</li>
          </ol>
        </aside>
      </div>
      <footer className="sheet-foot">
        {error ? <ErrorNote text={error} /> : <span className="muted">People have no skills or schedules. They're who your employees can ask.</span>}
        <span className="grow" />
        <button className="btn btn-ghost" onClick={onClose}>Cancel</button>
        <button className="btn btn-primary" disabled={busy} onClick={() => void submit()}>
          {busy ? "Adding…" : draft.name.trim() ? `Add ${firstName(draft.name.trim())}` : "Add person"}
        </button>
      </footer>
    </SlideOver>
  );
}
