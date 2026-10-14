import { useEffect, useState, type ReactNode } from "react";
import { call, errorMessage } from "../api/client";
import type { AppSettings, BackupFrequency, BackupSettings, BackupStatus, BuildChannel, BusinessHours, CheckFrequency, IsoWeekday, SetupStatus, WatchSettings } from "../api/types";
import { refreshOrg, rescanSkills, useQuery, useSkillScan, type SkillScan } from "../lib/hooks";
import { toast } from "../lib/toasts";
import { clockHM, relTime, whenShort } from "../lib/format";
import { setTheme, useTheme, type ThemePref } from "../lib/theme";
import { CopyCommand } from "../components/CopyCommand";
import { Icon } from "../components/Icon";
import { ErrorNote, PageColumn, Section, Skeleton, Switch } from "../components/ui";
import { openHelp } from "../lib/help";
import "./Settings.css";

function Row({ title, desc, children }: { title: string; desc?: ReactNode; children: ReactNode }) {
  return (
    <div className="set-row">
      <div className="stack grow" style={{ minWidth: 0 }}>
        <span className="set-title">{title}</span>
        {desc ? <span className="set-desc">{desc}</span> : null}
      </div>
      <div className="set-control">{children}</div>
    </div>
  );
}

/** A settings section: its title, an optional one-line explanation, then its rows in one group. */
function SettingsGroup({ title, sub, children }: { title: string; sub?: string; children: ReactNode }) {
  return (
    <Section title={title}>
      {sub ? <p className="set-sub">{sub}</p> : null}
      <div className="set-group">{children}</div>
    </Section>
  );
}

function SkillsDesc({ scan }: { scan: SkillScan }) {
  const src = scan.sources;
  if (!src) return scan.error ? <span className="set-err">{scan.error}</span> : <>Looking for your Claude Code skills…</>;
  const note = src.unreadable > 0 ? `${src.unreadable === 1 ? "1 skill" : `${src.unreadable} skills`} couldn't be read.` : null;
  return (
    <>
      <span className="mono">{src.skillsDir}</span>
      {note ? <span className="set-note">{note}</span> : null}
      {src.error && (src.found === 0 || src.unreadable > 0) ? <span className="set-note mono">{src.error}</span> : null}
    </>
  );
}

export function SettingsScreen() {
  const status = useQuery(() => call("setup_status"), []);
  const settings = useQuery(() => call("settings"), []);
  const theme = useTheme();
  const scan = useSkillScan();
  useEffect(() => { rescanSkills(); }, []);
  const [path, setPath] = useState("");
  useEffect(() => { if (settings.data) setPath(settings.data.claudePath ?? ""); }, [settings.data]);

  const save = async (next: AppSettings, msg?: string) => {
    settings.set(next);
    try {
      settings.set(await call("save_settings", { settings: next }));
      refreshOrg();
      if (msg) toast(msg, "ok");
    } catch (e) {
      toast(errorMessage(e), "err");
      settings.reload();
    }
  };
  const setBackground = async (enabled: boolean) => {
    if (status.data) status.set({ ...status.data, background: enabled });
    try { status.set(await call("set_background", { enabled })); }
    catch (e) { toast(errorMessage(e), "err"); status.reload(); }
  };
  const openData = async () => {
    try { await call("open_data_dir"); }
    catch (e) { toast(errorMessage(e), "err"); }
  };

  const s = settings.data;
  const st = status.data;
  const err = settings.error ?? status.error;
  return (
    <PageColumn title="Settings">
        {err ? <ErrorNote text={err} onRetry={() => { settings.reload(); status.reload(); }} /> : null}
        {!s || !st ? <div className="stack" style={{ gap: 12 }}><Skeleton h={120} r={12} /><Skeleton h={160} r={12} /></div> : (
          <>
            <SettingsGroup title="Claude Code">
              <Row title="Installation" desc={st.claude.found ? <span className="mono">{st.claude.path}</span> : "Not found. Install Claude Code or point to it below."}>
                {st.claude.found ? <span className="pill pill-ok"><Icon name="check" size={11} /> v{st.claude.version}</span> : <span className="pill pill-err">Not found</span>}
              </Row>
              <Row title="Path override" desc="Leave empty to find Claude Code automatically.">
                <form className="row" onSubmit={(e) => { e.preventDefault(); void save({ ...s, claudePath: path.trim() || null }, "Saved"); }}>
                  <input className="input mono set-path" value={path} onChange={(e) => setPath(e.target.value)} placeholder="Automatic" aria-label="Claude Code path" />
                  <button className="btn" type="submit" disabled={(s.claudePath ?? "") === path.trim()}>Save</button>
                </form>
              </Row>
              <Row title="Account" desc={st.account?.loggedIn ? <>{st.account.email} · {st.account.orgName}{st.account.plan ? ` · ${st.account.plan}` : null}</> : "Not signed in. Run this in a terminal, then come back."}>
                {st.account?.loggedIn ? <span className="pill pill-ok"><Icon name="check" size={11} /> Signed in</span> : <CopyCommand command="claude auth login" />}
              </Row>
              <Row title="Documentation" desc="How Claude Code works, from Anthropic.">
                <button className="btn" onClick={() => void openHelp()}><Icon name="external" size={13} /> Open</button>
              </Row>
              <Row title="Skills" desc={<SkillsDesc scan={scan} />}>
                <div className="row">
                  {scan.sources ? <span className={`pill num ${scan.sources.found === 0 ? "pill-warn" : ""}`}>{scan.sources.found === 1 ? "1 skill" : `${scan.sources.found} skills`}</span> : null}
                  <button className="btn" onClick={rescanSkills} disabled={scan.scanning}><Icon name="refresh" size={13} /> {scan.scanning ? "Scanning…" : "Rescan"}</button>
                </div>
              </Row>
            </SettingsGroup>

            <SettingsGroup title="Running">
              <Row title="Keep Omega Star running in the background" desc="Schedules only run while Omega Star is open. With this on, it starts at login and stays in the tray.">
                <Switch checked={st.background} onChange={(v) => void setBackground(v)} label="Run in the background" />
              </Row>
              <Row title="Pause all schedules" desc={s.paused ? "Nobody runs on a schedule until you resume. Manual runs and chat still work." : "Stop every schedule at once, for a holiday or a noisy week."}>
                <Switch checked={s.paused} onChange={(v) => void save({ ...s, paused: v }, v ? "Schedules paused" : "Schedules resumed")} label="Pause schedules" />
              </Row>
              <Row title="Runs at the same time" desc="Extra work and tasks wait their turn. Chat is never held up.">
                <div className="stepper">
                  <button className="btn btn-icon stepper-btn" aria-label="Fewer" disabled={s.maxConcurrentRuns <= 1} onClick={() => void save({ ...s, maxConcurrentRuns: s.maxConcurrentRuns - 1 })}>−</button>
                  <span className="mono num stepper-value">{s.maxConcurrentRuns}</span>
                  <button className="btn btn-icon stepper-btn" aria-label="More" disabled={s.maxConcurrentRuns >= 16} onClick={() => void save({ ...s, maxConcurrentRuns: s.maxConcurrentRuns + 1 })}>+</button>
                </div>
              </Row>
            </SettingsGroup>

            <RepliesCard settings={s} save={save} />

            <SettingsGroup title="Appearance">
              <Row title="Theme" desc="System follows your computer's light or dark setting.">
                <div className="seg" role="group" aria-label="Theme">
                  {(["system", "light", "dark"] as ThemePref[]).map((t) => (
                    <button key={t} aria-pressed={theme === t} onClick={() => setTheme(t)}>
                      <Icon name={t === "system" ? "monitor" : t === "light" ? "sun" : "moon"} size={12} /> {t[0].toUpperCase() + t.slice(1)}
                    </button>
                  ))}
                </div>
              </Row>
            </SettingsGroup>

            <BackupsCard settings={s} save={save} onOpenData={() => void openData()} />

            <SettingsGroup title="About">
              <Row title="Omega Star" desc={aboutDesc(st)}>
                <span className="pill">{CHANNELS[st.build.channel]}</span>
              </Row>
            </SettingsGroup>
          </>
        )}
    </PageColumn>
  );
}

const CHANNELS: Record<BuildChannel, string> = { release: "Release", dogfood: "Test build", dev: "Developer build" };

/** This build, then the Claude Code version and organisation the same status found, when known. */
function aboutDesc(st: SetupStatus): string {
  const app = `Version ${st.build.version}, built from ${st.build.commit}.`;
  const version = st.claude.version;
  const org = st.account?.orgName ?? null;
  if (version && org) return `${app} Claude Code ${version}, signed in to ${org}.`;
  if (version) return `${app} Claude Code ${version}.`;
  if (org) return `${app} Signed in to ${org}.`;
  return app;
}

const FREQUENCIES: { value: BackupFrequency; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "hourly", label: "Hourly" },
  { value: "daily", label: "Daily" },
  { value: "weekly", label: "Weekly" },
];

function size(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

function LastBackup({ st, frequency }: { st: BackupStatus; frequency: BackupFrequency }) {
  const failed = !!st.lastError; // cleared by the next success
  return (
    <>
      {st.running ? "Backing up the office…" : st.lastAt ? <>{whenShort(st.lastAt)} · {st.lastBytes != null ? size(st.lastBytes) : null}</> : "No backups yet."}
      {!st.running && st.skipped > 0 ? <span className="set-note">{st.skipped === 1 ? "1 file" : `${st.skipped} files`} couldn't be read and {st.skipped === 1 ? "was" : "were"} left out.</span> : null}
      {!st.running && st.leftOut > 0 ? <span className="set-note">{st.leftOut} large files and build folders were left out.</span> : null}
      {!st.running && st.lastAt && st.checkedAt && Date.parse(st.checkedAt) > Date.parse(st.lastAt) ? <span className="set-note">Nothing has changed since. Checked {relTime(st.checkedAt)}.</span> : null}
      {failed ? <span className="set-note set-err">Last attempt {relTime(st.lastAttemptAt)} failed: {st.lastError}</span> : null}
      {frequency !== "off" && st.nextAt && !st.running ? <span className="set-note">Next backup {Date.parse(st.nextAt) <= Date.now() ? "within a minute" : whenShort(st.nextAt)}</span> : null}
    </>
  );
}

function BackupsCard({ settings: s, save, onOpenData }: { settings: AppSettings; save: (next: AppSettings, msg?: string) => Promise<void>; onOpenData: () => void }) {
  const status = useQuery(() => call("backup_status"), [], ["backup:updated"]);
  const [starting, setStarting] = useState(false);
  const b = s.backup;
  const setBackup = (next: Partial<BackupSettings>, msg?: string) => save({ ...s, backup: { ...b, ...next } }, msg);

  const backUpNow = async () => {
    setStarting(true);
    try {
      await call("backup_now");
      toast("Backup saved", "ok");
    } catch (e) {
      toast(errorMessage(e), "err");
    } finally {
      setStarting(false);
      status.reload();
    }
  };
  const choose = async () => {
    try {
      const dir = await call("pick_backup_dir");
      if (dir && dir !== b.dir) await setBackup({ dir }, "Backup folder saved");
    } catch (e) { toast(errorMessage(e), "err"); }
  };
  const open = async () => {
    try { await call("open_backup_dir"); }
    catch (e) { toast(errorMessage(e), "err"); }
  };

  const st = status.data;
  const running = starting || !!st?.running;
  return (
    <SettingsGroup title="Data and backups">
      <Row title="Office folder" desc={<span className="mono">{s.dataDir}</span>}>
        <button className="btn" onClick={onOpenData}><Icon name="folder" size={13} /> Open</button>
      </Row>
      <Row title="Automatic backups" desc={b.frequency === "off" ? "Only when you back up by hand." : "A copy of the office: employees, their history, skills and notes. Skipped when nothing has changed. Large files (over 10 MB), node_modules and app builds in employees' folders are left out."}>
        <div className="seg" role="group" aria-label="Backup frequency">
          {FREQUENCIES.map((f) => (
            <button key={f.value} aria-pressed={b.frequency === f.value} onClick={() => void setBackup({ frequency: f.value })}>{f.label}</button>
          ))}
        </div>
      </Row>
      <Row title="Backup folder" desc={<span className="mono">{b.dir}</span>}>
        <div className="row">
          <button className="btn" onClick={() => void choose()}>Choose…</button>
          <button className="btn" onClick={() => void open()}><Icon name="folder" size={13} /> Open</button>
        </div>
      </Row>
      <Row title="Backups to keep" desc={<>Older backups in the folder are deleted automatically.{b.keep === 1 && b.frequency !== "off" ? <span className="set-note set-warn">One copy is a single restore point: a mistake you notice after the next backup can't be undone. Keep at least 3.</span> : null}</>}>
        <div className="stepper">
          <button className="btn btn-icon stepper-btn" aria-label="Fewer" disabled={b.keep <= 1} onClick={() => void setBackup({ keep: b.keep - 1 })}>−</button>
          <span className="mono num stepper-value">{b.keep}</span>
          <button className="btn btn-icon stepper-btn" aria-label="More" disabled={b.keep >= 100} onClick={() => void setBackup({ keep: b.keep + 1 })}>+</button>
        </div>
      </Row>
      <Row title="Last backup" desc={st ? <LastBackup st={st} frequency={b.frequency} /> : status.error ? <span className="set-err">{status.error}</span> : "…"}>
        <button className="btn" onClick={() => void backUpNow()} disabled={running}>
          <Icon name="archive" size={13} /> {running ? "Backing up…" : "Back up now"}
        </button>
      </Row>
      <Row title="Restoring" desc="Quit Omega Star from the tray, replace everything in the office folder with the contents of a backup zip, then start Omega Star again.">
        <span />
      </Row>
    </SettingsGroup>
  );
}

const CHECK_FREQUENCIES: { value: CheckFrequency; label: string }[] = [
  { value: "eager", label: "Often" },
  { value: "normal", label: "Normal" },
  { value: "relaxed", label: "Less often" },
];

const WEEKDAYS: { day: IsoWeekday; label: string; name: string }[] = [
  { day: 1, label: "Mon", name: "Monday" },
  { day: 2, label: "Tue", name: "Tuesday" },
  { day: 3, label: "Wed", name: "Wednesday" },
  { day: 4, label: "Thu", name: "Thursday" },
  { day: 5, label: "Fri", name: "Friday" },
  { day: 6, label: "Sat", name: "Saturday" },
  { day: 7, label: "Sun", name: "Sunday" },
];

const HALF_HOURS = Array.from({ length: 49 }, (_, i) => i * 30);
const NUDGE_DAYS = [0, 1, 2, 3, 4, 5];
const GIVE_UP_DAYS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

const minuteLabel = (minute: number) => clockHM(Math.floor(minute / 60) % 24, minute % 60);
const businessDays = (n: number) => `${n} business ${n === 1 ? "day" : "days"}`;
const withCurrent = (options: number[], current: number) => (options.includes(current) ? options : [...options, current].sort((a, b) => a - b));

function RepliesCard({ settings: s, save }: { settings: AppSettings; save: (next: AppSettings, msg?: string) => Promise<void> }) {
  const w = s.watch;
  const h = w.hours;
  const nudging = w.nudgeAfterDays > 0;
  const setWatch = (next: Partial<WatchSettings>) => save({ ...s, watch: { ...w, ...next } });
  const setHours = (next: Partial<BusinessHours>) => setWatch({ hours: { ...h, ...next } });
  const toggleDay = (day: IsoWeekday) =>
    setHours({ days: h.days.includes(day) ? h.days.filter((d) => d !== day) : [...h.days, day].sort((a, b) => a - b) });

  return (
    <SettingsGroup title="Replies from people" sub="When an employee asks someone on your org chart, Omega Star watches for the reply and reminds them once.">
      <Row title="Business hours" desc="Reminders and give-ups count only these hours. Teams chats are still checked outside them, less often.">
        <div className="set-hours">
          <div className="row">
            <select className="select set-select" aria-label="Start of the working day" value={h.startMinute} onChange={(e) => void setHours({ startMinute: Number(e.target.value) })}>
              {withCurrent(HALF_HOURS.slice(0, -1), h.startMinute).map((m) => <option key={m} value={m} disabled={m >= h.endMinute}>{minuteLabel(m)}</option>)}
            </select>
            <span className="muted">to</span>
            <select className="select set-select" aria-label="End of the working day" value={h.endMinute} onChange={(e) => void setHours({ endMinute: Number(e.target.value) })}>
              {withCurrent(HALF_HOURS.slice(1), h.endMinute).map((m) => <option key={m} value={m} disabled={m <= h.startMinute}>{minuteLabel(m)}</option>)}
            </select>
          </div>
          <div className="set-days" role="group" aria-label="Working days">
            {WEEKDAYS.map((d) => {
              const on = h.days.includes(d.day);
              return (
                <button key={d.day} type="button" className="chip" aria-pressed={on} aria-label={d.name} disabled={on && h.days.length === 1} onClick={() => void toggleDay(d.day)}>
                  {d.label}
                </button>
              );
            })}
          </div>
        </div>
      </Row>
      <Row title="Check for replies" desc="How often Omega Star looks. Checks start often and slow down as an ask gets older.">
        <div className="seg" role="group" aria-label="Check frequency">
          {CHECK_FREQUENCIES.map((f) => (
            <button key={f.value} aria-pressed={w.frequency === f.value} onClick={() => void setWatch({ frequency: f.value })}>{f.label}</button>
          ))}
        </div>
      </Row>
      <Row title="Nudge after" desc={nudging ? "With no reply, the employee sends one short reminder: at the due date, or after this long if there isn't one." : "Nobody gets a reminder."}>
        <select className="select set-select" aria-label="Nudge after" value={w.nudgeAfterDays} onChange={(e) => void setWatch({ nudgeAfterDays: Number(e.target.value) })}>
          {withCurrent(NUDGE_DAYS, w.nudgeAfterDays).map((n) => <option key={n} value={n} disabled={n > 0 && n >= w.giveUpAfterDays}>{n === 0 ? "Never" : businessDays(n)}</option>)}
        </select>
      </Row>
      <Row title="Give up after" desc="Then the employee goes on without the answer, or asks someone else. Counted from the due date if that's later.">
        <select className="select set-select" aria-label="Give up after" value={w.giveUpAfterDays} onChange={(e) => void setWatch({ giveUpAfterDays: Number(e.target.value) })}>
          {withCurrent(GIVE_UP_DAYS, w.giveUpAfterDays).map((n) => <option key={n} value={n} disabled={nudging && n <= w.nudgeAfterDays}>{businessDays(n)}</option>)}
        </select>
      </Row>
    </SettingsGroup>
  );
}
