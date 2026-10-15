import type { Run, RunKind, RunState, SkillChange, UsageWindow } from "../api/types";

const DAY_MS = 86_400_000;

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

export function firstName(name: string): string {
  return name.trim().split(/\s+/)[0] || name;
}

export function relTime(isoStr: string | null, now = Date.now()): string {
  if (!isoStr) return "";
  const t = Date.parse(isoStr);
  const diff = now - t;
  const abs = Math.abs(diff);
  const future = diff < 0;
  const fmt = (n: number, unit: string) => (future ? `in ${n}${unit}` : `${n}${unit} ago`);
  if (abs < 45_000) return future ? "in a moment" : "just now";
  if (abs < 3_600_000) return fmt(Math.round(abs / 60_000), "m");
  if (abs < DAY_MS) return fmt(Math.round(abs / 3_600_000), "h");
  if (abs < 7 * DAY_MS) return fmt(Math.round(abs / DAY_MS), "d");
  return new Date(t).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

const pad = (n: number) => String(n).padStart(2, "0");

/** 12-hour clock time: "9:00 am", "3:13 pm". Every time the app shows uses this. */
export function clockHM(hours: number, minutes: number): string {
  return `${hours % 12 || 12}:${pad(minutes)} ${hours < 12 ? "am" : "pm"}`;
}

/** A moment as a 12-hour clock time in this computer's time zone. */
function clock(d: Date): string {
  return clockHM(d.getHours(), d.getMinutes());
}

/** "Mon 9:00 am", "Today 2:30 pm", "Tomorrow 8:00 am" */
export function whenShort(isoStr: string | null, now = new Date()): string {
  if (!isoStr) return "";
  const d = new Date(isoStr);
  const time = clock(d);
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOf(d) - startOf(now)) / DAY_MS);
  if (days === 0) return `Today ${time}`;
  if (days === 1) return `Tomorrow ${time}`;
  if (days === -1) return `Yesterday ${time}`;
  if (days > 1 && days < 7) return `${d.toLocaleDateString(undefined, { weekday: "short" })} ${time}`;
  return `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })} ${time}`;
}

/** whenShort for the middle of a sentence: "due today 2:30 pm", "next run Fri 8:00 am". */
export function whenInline(isoStr: string | null, now = new Date()): string {
  return whenShort(isoStr, now).replace(/^(Today|Tomorrow|Yesterday)/, (m) => m.toLowerCase());
}

export function timeOrDay(isoStr: string, now = new Date()): string {
  const d = new Date(isoStr);
  return d.toDateString() === now.toDateString() ? clock(d) : whenShort(isoStr, now);
}

/** "Resets 2:59 pm" today, "Resets Tue 11:59 pm" this week; falls back to the CLI's own wording. */
export function resetsLabel(w: UsageWindow, now = new Date()): string {
  if (w.resetsAt) {
    const d = new Date(w.resetsAt);
    const time = clock(d);
    const days = (d.getTime() - now.getTime()) / DAY_MS;
    const day = d.toDateString() === now.toDateString() ? ""
      : days < 6 ? `${d.toLocaleDateString(undefined, { weekday: "short" })} `
      : `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })}, `;
    return `Resets ${day}${time}`;
  }
  return w.resetsText ? `Resets ${w.resetsText.replace(/\s*\([^)]*\)\s*$/, "")}` : "";
}

/** When held work starts: "Starts at 9:30 pm when the limit resets", or "Starts Tue at 9:30 pm ..." on another day. */
export function heldLabel(at: string, now = new Date()): string {
  const d = new Date(at);
  const day = d.toDateString() === now.toDateString() ? "" : `${d.toLocaleDateString(undefined, { weekday: "short" })} `;
  return `Starts ${day}at ${clock(d)} when the limit resets`;
}

export function dateLong(isoStr: string): string {
  const d = new Date(isoStr);
  return `${d.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" })}, ${clock(d)}`;
}

export function dayHeading(isoStr: string, now = new Date()): string {
  const d = new Date(isoStr);
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOf(now) - startOf(d)) / DAY_MS);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  return d.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
}

export function duration(ms: number | null): string {
  if (ms === null || !Number.isFinite(ms)) return "–";
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${pad(s % 60)}s`;
  return `${Math.floor(m / 60)}h ${pad(m % 60)}m`;
}

export function runKindLabel(kind: RunKind, names: { manager?: string; person?: string; asker?: string } = {}): string {
  switch (kind.type) {
    case "scheduled": return kind.label;
    case "manual": return "Manual run";
    case "chat": return "Chat";
    case "delegated": return names.manager ? `Delegated by ${names.manager}` : "Delegated";
    case "asked": return `${kind.task ? "Task" : "Question"} from ${names.asker ?? "a colleague"}`;
    case "answered": return kind.task ? `${names.asker ?? "A colleague"} finished your task` : `Answer from ${names.asker ?? "a colleague"}`;
    case "followUp": return names.person ? `Follow-up: ${names.person} replied` : "Follow-up";
    case "replyCheck": return names.person ? `Reply check: ${names.person}` : "Reply check";
    case "nudge": return names.person ? `Nudge: ${names.person}` : "Nudge";
    case "unanswered": return names.person ? `No reply: ${names.person}` : "No reply";
    case "organize": return `Organized ${kind.imported.length} imported ${kind.imported.length === 1 ? "skill" : "skills"}`;
    case "reply": return `Re: ${kind.label}`;
    case "task": return kind.label;
    case "tidy": return kind.reason === "approved" ? "Tidy: approved changes" : "Tidy";
    case "hireDecided": return kind.approved ? "Hire approved" : "Hire declined";
  }
}

export function runStateLabel(state: RunState): string {
  switch (state) {
    case "queued": return "Queued";
    case "running": return "Working";
    case "succeeded": return "Done";
    case "failed": return "Failed";
    case "cancelled": return "Cancelled";
    case "lost": return "Interrupted";
  }
}

export function runStateTone(run: Pick<Run, "state" | "denials">): "accent" | "ok" | "warn" | "err" | "neutral" {
  switch (run.state) {
    case "queued":
    case "running": return "accent";
    case "succeeded": return run.denials.length > 0 ? "warn" : "ok";
    case "failed": return "err";
    case "lost": return "warn";
    case "cancelled": return "neutral";
  }
}

export function elapsedMs(run: Pick<Run, "startedAt" | "endedAt" | "durationMs">, now = Date.now()): number | null {
  if (run.durationMs !== null && run.endedAt) return run.durationMs;
  if (!run.startedAt) return null;
  return (run.endedAt ? Date.parse(run.endedAt) : now) - Date.parse(run.startedAt);
}

/** How a recorded change's name reads: notes, the company page and what an employee owns are recorded like skills but are not skills. */
export function recordedName(skill: string): string {
  switch (skill) {
    case "@workspace": return "notes";
    case "@company": return "the company page";
    case "@own": return "what they own";
    default: return skill;
  }
}

/** What a run did to the employee's skills, by skill rather than by file: "Refined hazel-inbox-watch". A tidy's notes are "notes". */
export function skillsChanged(changes: SkillChange[]): { label: string; title: string } {
  const names = [...new Set(changes.map((c) => c.skill))];
  const skills = names.filter((n) => recordedName(n) === n);
  const parts = skills.length <= 2 ? skills : [`${skills.length} skills`];
  parts.push(...names.filter((n) => recordedName(n) !== n).map(recordedName));
  const verb = changes.every((c) => c.change === "added") ? "Created" : changes.every((c) => c.change === "removed") ? "Removed" : "Changed";
  return { label: `${verb} ${parts.join(" and ")}`, title: names.map(recordedName).join(", ") };
}

export function hueFromName(name: string): number {
  let h = 0;
  for (const ch of name.trim().toLowerCase()) h = (h * 31 + ch.charCodeAt(0)) % 360;
  return name.trim() ? h : 250;
}
