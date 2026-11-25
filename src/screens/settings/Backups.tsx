import { useEffect, useRef, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { AppSettings, BackupFrequency, BackupSettings, BackupStatus } from "../../api/types";
import { useQuery } from "../../lib/hooks";
import { plural, relTime, whenInline, whenShort } from "../../lib/format";
import { Icon } from "../../components/Icon";
import { Row, States, Stepper, Swap, sentence, useRowSave, type Change, type Saves } from "./rows";
import { Group } from "./frame";

const FREQUENCIES: { value: BackupFrequency; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "hourly", label: "Hourly" },
  { value: "daily", label: "Daily" },
  { value: "weekly", label: "Weekly" },
];

const TRANSCRIPT_DAYS: { value: number | null; label: string }[] = [
  { value: null, label: "Forever" },
  { value: 365, label: "1 year" },
  { value: 90, label: "90 days" },
  { value: 30, label: "30 days" },
];

function size(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

/** "Today at 3:00 am", "Fri at 8:00 am"; `inline` lowers Today, Tomorrow and Yesterday for the middle of a sentence. */
function whenAt(iso: string, inline = false): string {
  return (inline ? whenInline : whenShort)(iso).replace(/ (\d{1,2}:\d{2} [ap]m)$/, " at $1");
}

/** The last backup's lines. They stay as they are while a backup runs (the button says so), and change in place when
 *  it finishes, so the row never collapses and regrows. */
function LastBackup({ st, frequency, hideFailure }: { st: BackupStatus; frequency: BackupFrequency; hideFailure: boolean }) {
  const failed = !!st.lastError && !hideFailure; // cleared by the next success
  return (
    <>
      {st.lastAt ? <>{whenAt(st.lastAt)}{st.lastBytes != null ? `, ${size(st.lastBytes)}` : null}</> : "No backups yet."}
      {st.skipped > 0 ? <span className="set-note">{plural(st.skipped, "file")} couldn't be read and {st.skipped === 1 ? "was" : "were"} left out.</span> : null}
      {st.leftOut > 0 ? <span className="set-note">{st.leftOut} installers and build folders were left out.</span> : null}
      {st.lastAt && st.checkedAt && Date.parse(st.checkedAt) > Date.parse(st.lastAt) ? <span className="set-note">Nothing has changed since. Checked {relTime(st.checkedAt)}.</span> : null}
      {failed ? <span className="set-note set-err">Last attempt {relTime(st.lastAttemptAt)} failed: {sentence(st.lastError ?? "")}</span> : null}
      {frequency !== "off" && st.nextAt ? <span className="set-note">Next backup {Date.parse(st.nextAt) <= Date.now() ? "within a minute" : whenAt(st.nextAt, true)}</span> : null}
    </>
  );
}

export function BackupsGroup({ s, change, saves, onOpenData }: { s: AppSettings; change: Change; saves: Saves; onOpenData: () => void }) {
  const status = useQuery(() => call("backup_status"), [], ["backup:updated"]);
  // "asking": backup_now hasn't answered; "waiting": it answered while the backup still runs. The row's lines, its
  // button and its "Saved" change together once the status says it has finished.
  const [phase, setPhase] = useState<"idle" | "asking" | "waiting">("idle");
  const lastFailed = useRowSave("Last backup")?.state === "failed";
  const b = s.backup;
  const setBackup = (row: string, next: Partial<BackupSettings>) => change(row, (x) => ({ ...x, backup: { ...x.backup, ...next } }));

  const backUpNow = async () => {
    setPhase("asking");
    saves.clear("Last backup");
    try {
      status.set(await call("backup_now"));
      setPhase("waiting");
    } catch (e) {
      setPhase("idle");
      status.reload();
      saves.failed("Last backup", sentence(errorMessage(e)), () => void backUpNow());
    }
  };
  const again = useRef(() => {});
  useEffect(() => { again.current = () => void backUpNow(); });
  const st = status.data;
  useEffect(() => {
    if (phase !== "waiting" || !st || st.running) return;
    setPhase("idle");
    const failedNow = !!st.lastError && !!st.lastAttemptAt && (!st.lastAt || Date.parse(st.lastAttemptAt) > Date.parse(st.lastAt));
    if (failedNow) saves.failed("Last backup", sentence(st.lastError ?? ""), () => again.current());
    else saves.saved("Last backup");
  }, [phase, st, saves]);
  const choose = async () => {
    saves.clear("Backup folder");
    try {
      const dir = await call("pick_backup_dir");
      if (dir && dir !== b.dir) setBackup("Backup folder", { dir });
    } catch (e) { saves.failed("Backup folder", sentence(errorMessage(e))); }
  };
  const open = async () => {
    saves.clear("Backup folder");
    try { await call("open_backup_dir"); }
    catch (e) { saves.failed("Backup folder", sentence(errorMessage(e)), () => void open()); }
  };

  const running = phase !== "idle" || !!st?.running;
  const kept = TRANSCRIPT_DAYS.find((t) => t.value === s.transcriptDays);
  const lastKey = st ? `${st.lastAt}${st.checkedAt}${st.lastError}` : status.error ? "error" : "wait";
  return (
    <Group
      title="Data and backups"
      foot={<p className="set-aside"><span className="set-aside-title">Restoring</span> Quit OmegaStar from the tray, replace everything in the office folder with the contents of a backup zip, then start OmegaStar again.</p>}
    >
      <Row title="Office folder" desc={<span className="mono">{s.dataDir}</span>}>
        <button className="btn" onClick={onOpenData}><Icon name="folder" size={13} /> Open</button>
      </Row>
      <Row title="Keep transcripts" desc={<Swap on={String(s.transcriptDays)}>{s.transcriptDays == null ? "Every run's full transcript stays in the office folder." : `A run's full transcript is deleted ${kept ? kept.label : `${s.transcriptDays} days`} after it finishes. Its report and any pages it showed stay.`}</Swap>}>
        <div className="seg" role="group" aria-label="Keep transcripts">
          {TRANSCRIPT_DAYS.map((t) => (
            <button key={t.label} aria-pressed={s.transcriptDays === t.value} onClick={() => change("Keep transcripts", (x) => ({ ...x, transcriptDays: t.value }))}>{t.label}</button>
          ))}
        </div>
      </Row>
      <Row title="Automatic backups" desc={<Swap on={String(b.frequency === "off")}>{b.frequency === "off" ? "Only when you back up by hand." : "A copy of the office: employees, their history, skills and notes. Skipped when nothing has changed. App installers and build and cache folders such as node_modules are left out."}</Swap>}>
        <div className="seg" role="group" aria-label="Backup frequency">
          {FREQUENCIES.map((f) => (
            <button key={f.value} aria-pressed={b.frequency === f.value} onClick={() => setBackup("Automatic backups", { frequency: f.value })}>{f.label}</button>
          ))}
        </div>
      </Row>
      <Row title="Backup folder" desc={<Swap on={b.dir}><span className="mono">{b.dir}</span></Swap>}>
        <button className="btn" onClick={() => void choose()}>Choose…</button>
        <button className="btn" onClick={() => void open()}><Icon name="folder" size={13} /> Open</button>
      </Row>
      <Row title="Backups to keep" desc={<>Older backups in the folder are deleted automatically.{b.keep === 1 && b.frequency !== "off" ? <span className="set-note set-warn">One copy is a single restore point: a mistake you notice after the next backup can't be undone. Keep at least 3.</span> : null}</>}>
        <Stepper
          label="Backups to keep" value={b.keep} min={1} max={100} less="Fewer backups to keep" more="More backups to keep"
          onChange={(v) => setBackup("Backups to keep", { keep: v })}
        />
      </Row>
      <Row title="Last backup" desc={<Swap on={lastKey}>{st ? <LastBackup st={st} frequency={b.frequency} hideFailure={lastFailed} /> : status.error ? <span className="set-err">{sentence(status.error)}</span> : "Checking…"}</Swap>}>
        <button className="btn" onClick={() => void backUpNow()} disabled={running}>
          <States at={running ? 1 : 0} labels={[<><Icon name="archive" size={13} />Back up now</>, "Backing up…"]} />
        </button>
      </Row>
    </Group>
  );
}
