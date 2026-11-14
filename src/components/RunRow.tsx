import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { EmployeeView, PersonView, Run } from "../api/types";
import { duration, elapsedMs, firstName, heldLabel, runStateLabel, runStateTone, shortAge, whenShort } from "../lib/format";
import { useNow, useUsage } from "../lib/hooks";
import { rollAnimations } from "../lib/motion";
import { openRun } from "../lib/router";
import { reportOf } from "../lib/threads";
import { kindNames, workLabel } from "../lib/workTitle";
import { Icon, type IconName } from "./Icon";
import { Avatar, StatePill } from "./ui";
import "./RunRow.css";

/** Who handed a delegated run over, as its second line says it: "From Juniper". */
function handedBy(run: Run, employees: EmployeeView[], people: PersonView[]): string | null {
  if (run.kind.type !== "delegated") return null;
  const m = kindNames(run, employees, people).manager;
  return m ? `From ${m}` : "From their manager";
}

export function kindIcon(run: Run): IconName {
  switch (run.kind.type) {
    case "scheduled": return "clock";
    case "manual": return "play";
    case "delegated": return "branch";
    case "asked": return "ask";
    case "answered": return "reply";
    case "followUp": return "ask";
    case "replyCheck": return "search";
    case "nudge": return "bell";
    case "unanswered": return "clock";
    case "chat": return "chat";
    case "organize": return "layers";
    case "tidy": return "checklist";
    case "reply": return "reply";
    case "task": return "chat";
    case "hireDecided": return "userPlus";
  }
}

/** The first line of a report, without its markdown. */
const firstLine = (text: string) => text.replace(/[*_`#>|]/g, "").split("\n").map((l) => l.trim()).find(Boolean) ?? "";

/** A row's second line: what came of the run, else what went wrong, else what it was asked (less the line its title
 *  already shows). */
function detailOf(run: Run, label: string): string {
  const report = reportOf(run);
  if (report) {
    const line = firstLine(report);
    if (line) return line;
  }
  if ((run.state === "failed" || run.state === "lost") && run.error) return firstLine(run.error);
  const text = run.prompt.split("\n").map((l) => l.trim()).filter(Boolean).join(" ");
  return text.startsWith(label) ? text.slice(label.length).replace(/^[.!?]?\s*/, "") : text;
}

type Status = { key: string; tone: Parameters<typeof StatePill>[0]["tone"]; word: string };

/** What a row says about its state: nothing for a plain success, else one word in its colour. Blocked needs your OK, so
 *  it is coral like everything else that needs you. */
function statusOf(run: Run): Status | null {
  if (run.state === "succeeded") return run.denials.length ? { key: "blocked", tone: "err", word: "Blocked" } : null;
  return { key: run.state, tone: runStateTone(run), word: runStateLabel(run.state) };
}

/** The status track: the pill for the run's state. When the state changes the old pill fades out as the new one fades in,
 *  in the same cell; on first show it is simply there. */
function StatusTrack({ status }: { status: Status | null }) {
  const [shown, setShown] = useState<{ now: Status | null; was: Status | null; changed: boolean }>({ now: status, was: null, changed: false });
  if ((shown.now?.key ?? "") !== (status?.key ?? "")) setShown({ now: status, was: shown.now, changed: true });
  useEffect(() => {
    if (!shown.was) return;
    const t = window.setTimeout(() => setShown((s) => ({ ...s, was: null })), 160);
    return () => window.clearTimeout(t);
  }, [shown.was]);
  return (
    <span className="runrow-status">
      {shown.was ? <span key={`was-${shown.was.key}`} className="runrow-pill leave-fade" aria-hidden><StatePill tone={shown.was.tone}>{shown.was.word}</StatePill></span> : null}
      {shown.now ? <span key={shown.now.key} className={`runrow-pill ${shown.changed ? "enter-fade" : ""}`}><StatePill tone={shown.now.tone}>{shown.now.word}</StatePill></span> : null}
    </span>
  );
}

/** A list age ("4m", "3h") that rolls up when it moves on, as time only goes one way. Exported for the area's other
 *  rows (skill changes) so every age in Activity moves alike. */
export function Age({ at, now }: { at: string | null; now: number }) {
  const value = shortAge(at, now);
  const [roll, setRoll] = useState<{ value: string; from: string | null }>({ value, from: null });
  if (roll.value !== value) setRoll({ value, from: roll.value });
  const slot = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const el = slot.current;
    if (roll.from === null || !el) return;
    const anims = rollAnimations(el.children[0] as HTMLElement, el.children[1] as HTMLElement, true);
    let live = true;
    void Promise.all(anims.map((a) => a.finished)).then(() => { if (live) setRoll((r) => ({ ...r, from: null })); }, () => {});
    return () => { live = false; anims.forEach((a) => a.cancel()); };
  }, [roll.from]);
  return (
    <span ref={slot} className="roll runrow-age num">
      {roll.from !== null ? <><span aria-hidden>{roll.from}</span><span>{value}</span></> : <span>{value}</span>}
    </span>
  );
}

/** While the office waits for the usage limit, when a queued run starts. `on`: only a queued run asks. */
function useHeld(on: boolean): string | null {
  const { usage } = useUsage();
  const now = useNow(60_000, on && !!usage?.heldUntil);
  if (!on || !usage?.heldUntil || Date.parse(usage.heldUntil) <= now) return null;
  return heldLabel(usage.heldUntil, new Date(now));
}

const HOUR_MS = 3_600_000;

/**
 * One run as a list row, on the one grid every run list shares: who (or the kind of run), what it was about over what came
 * of it, its state as a word in a fixed track (empty for a plain success), and its age in a fixed track. Opens its
 * conversation.
 */
export function RunRow({ run, employee, employees, people }: {
  run: Run; employee?: EmployeeView; employees: EmployeeView[]; people: PersonView[];
}) {
  const at = run.startedAt ?? run.createdAt;
  // Ages move by the minute for the last hour; older ones change so rarely that the next reload keeps them right.
  const now = useNow(60_000, Date.now() - Date.parse(at) < HOUR_MS || run.state === "running");
  const held = useHeld(run.state === "queued");
  const label = workLabel(run, employees, people);
  const detail = held ?? (detailOf(run, label) || handedBy(run, employees, people) || "");
  const took = run.endedAt ? duration(elapsedMs(run)) : "";
  return (
    <button className="runrow" data-state={run.state} onClick={() => openRun(run.id)} title={took ? `${whenShort(run.createdAt)} · took ${took}` : whenShort(run.createdAt)}>
      {employee ? <Avatar name={employee.name} hue={employee.hue} image={employee.image} size={28} className="runrow-lead" /> : (
        <span className="runrow-lead runrow-icon"><Icon name={kindIcon(run)} size={14} /></span>
      )}
      <span className="runrow-title">
        {employee ? <b>{firstName(employee.name)}</b> : null}
        <span className="runrow-label">{label}</span>
      </span>
      <span className="runrow-detail">{detail}</span>
      <StatusTrack status={statusOf(run)} />
      <Age at={at} now={now} />
    </button>
  );
}
