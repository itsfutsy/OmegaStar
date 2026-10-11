import { useEffect, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { CronPreview, EmployeeView, Schedule, ScheduleDraft } from "../../api/types";
import { CRON_PRESETS, describeCron, matchPreset, presetCron, type CronPresetId } from "../../lib/cron";
import { firstName, whenInline, whenShort } from "../../lib/format";
import { refreshOrg, useDebounced, useQuery } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import { navigate } from "../../lib/router";
import { Icon } from "../Icon";
import { ConfirmButton, ErrorNote, Section, Skeleton, Switch } from "../ui";
import "./Schedules.css";

const blankSchedule = (e: EmployeeView): ScheduleDraft => ({ employeeId: e.id, label: "", cron: "0 9 * * 1-5", instruction: "", enabled: true, misfire: "catchUp", quiet: false });

/** The employee's schedules, at the top of the Work tab: when they run on their own. */
export function Schedules({ e, onEdit }: { e: EmployeeView; onEdit: (d: ScheduleDraft) => void }) {
  const q = useQuery(() => call("schedules", { employeeId: e.id }), [e.id], ["org:changed"]);
  const settings = useQuery(() => call("settings"), [], ["org:changed"]);
  const name = firstName(e.name);

  const toggle = async (s: Schedule, enabled: boolean) => {
    q.set((q.data ?? []).map((x) => (x.id === s.id ? { ...x, enabled } : x)));
    try {
      const { nextFire: _n, ...draft } = s;
      void _n;
      await call("save_schedule", { draft: { ...draft, enabled } });
      refreshOrg();
    } catch (err) {
      toast(errorMessage(err), "err");
      q.reload();
    }
  };

  return (
    <Section title="Schedules" action={q.data && q.data.length > 0 ? <button className="section-note linkish" onClick={() => onEdit(blankSchedule(e))}><Icon name="plus" size={11} /> Add</button> : null}>
      {settings.data?.paused ? (
        <div className="sched-paused">
          <Icon name="stop" size={12} /> All schedules are paused.
          <button className="linkish" onClick={() => navigate({ page: "settings" })}>Resume in Settings</button>
        </div>
      ) : null}
      {q.error ? <ErrorNote text={q.error} onRetry={q.reload} /> : null}
      {!q.data && !q.error ? <Skeleton h={44} r={10} /> : null}
      {q.data && q.data.length === 0 ? (
        <div className="sched-empty">
          <span>{name} works only when asked. Add a schedule and they run on their own.</span>
          <button className="btn btn-sm" onClick={() => onEdit(blankSchedule(e))}><Icon name="plus" size={12} /> Add a schedule</button>
        </div>
      ) : null}
      {q.data && q.data.length > 0 ? (
        <div className="sched-list">
          {q.data.map((s) => (
            <div key={s.id} className={`sched ${s.enabled ? "" : "sched-off"}`}>
              <button className="sched-main" onClick={() => onEdit({ ...s })}>
                <span className="sched-label">{s.label}</span>
                <span className="sched-when">
                  {describeCron(s.cron) ?? <span className="mono">{s.cron}</span>}
                  {s.quiet ? <> · reports only when needed</> : null}
                </span>
                <span className="sched-next num">{s.enabled && s.nextFire ? <>Next run {whenInline(s.nextFire)}</> : s.enabled ? "Paused" : "Off"}</span>
              </button>
              <Switch checked={s.enabled} onChange={(v) => void toggle(s, v)} label={`${s.label} on`} />
            </div>
          ))}
        </div>
      ) : null}
    </Section>
  );
}

export function ScheduleEditor({ e, initial, onDone }: { e: EmployeeView; initial: ScheduleDraft; onDone: () => void }) {
  const [d, setD] = useState<ScheduleDraft>(initial);
  const matched = matchPreset(initial.cron);
  const [preset, setPreset] = useState<CronPresetId>(initial.id ? matched.id : "weekdays");
  const [time, setTime] = useState(matched.time === "09:00" && !initial.id ? "08:00" : matched.time);
  const [preview, setPreview] = useState<CronPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const cron = preset === "custom" ? d.cron : presetCron(preset, time);
  const debounced = useDebounced(cron, 220);

  useEffect(() => {
    let alive = true;
    call("cron_preview", { cron: debounced }).then((p) => { if (alive) setPreview(p); }, (x) => { if (alive) setPreview({ ok: false, error: errorMessage(x) }); });
    return () => { alive = false; };
  }, [debounced]);

  const set = <K extends keyof ScheduleDraft>(k: K, v: ScheduleDraft[K]) => setD((x) => ({ ...x, [k]: v }));
  const save = async () => {
    if (!d.label.trim()) { setErr("Give the schedule a name, like “Weekly checks”."); return; }
    setBusy(true);
    setErr(null);
    try {
      await call("save_schedule", { draft: { ...d, cron, label: d.label.trim() } });
      toast(initial.id ? "Schedule saved" : `${firstName(e.name)} has a new routine`, "ok");
      onDone();
    } catch (x) {
      setErr(errorMessage(x));
      setBusy(false);
    }
  };
  const remove = async () => {
    if (!initial.id) return;
    try {
      await call("delete_schedule", { scheduleId: initial.id });
      toast("Schedule removed", "ok");
      onDone();
    } catch (x) {
      setErr(errorMessage(x));
    }
  };

  return (
    <div className="fileview">
      <div className="tab-scroll">
        <nav className="crumbs" aria-label="Breadcrumb">
          <button onClick={onDone}>Work</button><Icon name="chevronRight" size={11} />
          <span className="crumb-here">{initial.id ? initial.label : "New schedule"}</span>
        </nav>
        <div className="stack" style={{ gap: 18 }}>
          <label className="field">
            <span className="field-label">Name</span>
            <input className="input" autoFocus={!initial.id} value={d.label} onChange={(ev) => set("label", ev.target.value)} placeholder="e.g. Weekly checks" />
          </label>
          <div className="field">
            <span className="field-label">When</span>
            <div className="row" style={{ flexWrap: "wrap", gap: 6 }}>
              {CRON_PRESETS.map((p) => (
                <button key={p.id} type="button" className="chip" aria-pressed={preset === p.id} onClick={() => { if (p.id === "custom") set("cron", cron); setPreset(p.id); }}>{p.label}</button>
              ))}
            </div>
            <div className="sched-when-row">
              {preset === "custom" ? (
                <input className="input mono" value={d.cron} onChange={(ev) => set("cron", ev.target.value)} placeholder="minute hour day month weekday" aria-label="Cron expression" spellCheck={false} />
              ) : preset === "hourly" ? (
                <span className="muted" style={{ fontSize: "var(--t-sm)" }}>On the hour, every hour.</span>
              ) : (
                <label className="row" style={{ gap: 8 }}>
                  <span className="muted" style={{ fontSize: "var(--t-sm)" }}>at</span>
                  <input type="time" className="input mono sched-time" value={time} onChange={(ev) => setTime(ev.target.value || "09:00")} aria-label="Time" />
                </label>
              )}
            </div>
            <div className={`sched-preview ${preview && !preview.ok ? "sched-preview-err" : ""}`} aria-live="polite">
              {!preview ? <Skeleton h={14} w="60%" /> : preview.ok ? (
                <>
                  {/* The same words as the schedule's row for a pattern the app knows, else the Office's own description. */}
                  <span className="sched-preview-desc">{describeCron(debounced) ?? preview.description}</span>
                  <span className="sched-preview-next mono num">
                    {preview.next.map((n) => <span key={n}>{whenShort(n)}</span>)}
                  </span>
                </>
              ) : <span><Icon name="alert" size={12} /> {preview.error}</span>}
            </div>
            {preset !== "custom" ? <span className="field-help mono" style={{ fontSize: "var(--t-xs)" }}>{cron}</span> : null}
          </div>
          <label className="field">
            <span className="field-label">What should {firstName(e.name)} do each time?</span>
            <textarea className="textarea" rows={3} value={d.instruction} onChange={(ev) => set("instruction", ev.target.value)} placeholder="e.g. Run the weekly checks and report to Maya." />
            <span className="field-help">Keep it short. The how lives in their skills.</span>
          </label>
          <div className="field">
            <span className="field-label">Reports</span>
            <div className="misfire" role="radiogroup" aria-label="Reports">
              <button type="button" role="radio" aria-checked={!d.quiet} className="misfire-opt" onClick={() => set("quiet", false)}>
                <span className="misfire-radio" /><span className="stack"><b>Every run</b><span>Each report shows up in your sidebar.</span></span>
              </button>
              <button type="button" role="radio" aria-checked={d.quiet} className="misfire-opt" onClick={() => set("quiet", true)}>
                <span className="misfire-radio" /><span className="stack"><b>Only when something needs you</b><span>Routine runs stay in Activity. Anything {firstName(e.name)} flags, and any failed run, reaches you with a desktop notification.</span></span>
              </button>
            </div>
          </div>
          <div className="field">
            <span className="field-label">On</span>
            <div className="row" style={{ height: 34 }}><Switch checked={d.enabled} onChange={(v) => set("enabled", v)} label="Schedule on" /><span className="muted" style={{ fontSize: "var(--t-sm)" }}>{d.enabled ? "Runs at these times" : "Off until you turn it on"}</span></div>
          </div>
          <div className="field">
            <span className="field-label">If Omega Star wasn't running at the time</span>
            <div className="misfire" role="radiogroup" aria-label="Missed runs">
              <button type="button" role="radio" aria-checked={d.misfire === "catchUp"} className="misfire-opt" onClick={() => set("misfire", "catchUp")}>
                <span className="misfire-radio" /><span className="stack"><b>Catch up once</b><span>Run as soon as Omega Star starts again. Several missed runs become one.</span></span>
              </button>
              <button type="button" role="radio" aria-checked={d.misfire === "skip"} className="misfire-opt" onClick={() => set("misfire", "skip")}>
                <span className="misfire-radio" /><span className="stack"><b>Skip it</b><span>Only run on time. Good for things that go stale, like a morning briefing.</span></span>
              </button>
            </div>
          </div>
        </div>
      </div>
      <div className="savebar">
        {err ? <ErrorNote text={err} /> : initial.id ? <ConfirmButton label="Delete" icon="trash" className="btn btn-ghost btn-danger" size="btn-sm" question="Delete this schedule?" confirmLabel="Delete" onConfirm={remove} /> : null}
        <span className="grow" />
        <button className="btn btn-ghost" onClick={onDone}>Cancel</button>
        <button className="btn btn-primary" disabled={busy || (preview !== null && !preview.ok)} onClick={() => void save()}>{busy ? "Saving…" : initial.id ? "Save" : "Add schedule"}</button>
      </div>
    </div>
  );
}
