import { useState, type ReactNode } from "react";
import { call, errorMessage } from "../api/client";
import type { Channel, Delivery, EmployeeView, Heard, HumanRequest, PersonView, Verdict, Watch, WatchAction } from "../api/types";
import { channelLabel } from "../lib/channels";
import { firstName, relTime, whenInline } from "../lib/format";
import { refreshOrg, useNow, useQuery } from "../lib/hooks";
import { toast, type Toast } from "../lib/toasts";
import { openChat, openMember, openRun } from "../lib/router";
import { Icon } from "./Icon";
import { Avatar, ErrorNote, MenuButton, type MenuItem } from "./ui";
import "./RequestCard.css";

const VERDICT: Record<Verdict, string> = {
  answered: "answered",
  done: "done",
  askedBack: "asked something back",
  acknowledged: "acknowledged",
  noReply: "not about this",
};

const QUOTE_MAX = 220;

function dueTone(dueAt: string | null): "late" | "soon" | "later" | null {
  if (!dueAt) return null;
  const ms = Date.parse(dueAt) - Date.now();
  if (ms < 0) return "late";
  if (ms < 8 * 3_600_000) return "soon";
  return "later";
}

const readsNatively = (d: Delivery) => d.locator.type === "teamsChat" && d.locator.server !== null;

function sentOn(d: Delivery, channel: Channel | undefined): string {
  if (d.locator.type === "teamsChat") return "Sent on Teams";
  return channel ? `Sent on ${channelLabel(channel.kind)}` : "Sent";
}

/** What the watch is doing, in one phrase. The check-by-check detail is in the tooltip. */
function WatchingLine({ d, w, channel }: { d: Delivery; w: Watch; channel: Channel | undefined }) {
  useNow(30_000);
  const now = Date.now();
  const plan = w.plan;
  const reminder = w.nudgedAt ? `reminded ${relTime(w.nudgedAt, now)}` : plan?.nudgeAt ? `reminder ${whenInline(plan.nudgeAt)}` : null;
  const detail = [
    w.lastCheckedAt ? `Last checked ${relTime(w.lastCheckedAt, now)}` : "Not checked yet",
    plan ? (Date.parse(plan.nextCheckAt) <= now ? "next check soon" : `next check ${whenInline(plan.nextCheckAt)}`) : null,
    `sent with ${d.via}`,
  ].filter(Boolean).join(" · ");
  return <span title={detail}>{sentOn(d, channel)} {relTime(d.sentAt, now)} · watching for a reply{reminder ? ` · ${reminder}` : ""}</span>;
}

function GaveUpText({ empName }: { empName: string }) {
  const settings = useQuery(() => call("settings"), []);
  const days = settings.data?.watch.giveUpAfterDays;
  return <span>{days === undefined ? "No reply in time." : `No reply after ${days} business ${days === 1 ? "day" : "days"}.`} {empName} went on without it.</span>;
}

function Status({ r, channel, empName }: { r: HumanRequest; channel: Channel | undefined; empName: string }): ReactNode {
  const d = r.delivery;
  if (r.kind === "info") return <span>{d ? `${sentOn(d, channel)} · no reply needed` : "Not sent"}</span>;
  if (!d) return <span>Not sent. Pass it on yourself, or answer it here.</span>;
  const w = d.watch;
  if (!w) return <span title={`Sent with ${d.via}`}>{sentOn(d, channel)} {relTime(d.sentAt)}</span>;
  switch (w.phase.type) {
    case "watching": return <WatchingLine d={d} w={w} channel={channel} />;
    case "stopped": return <span>{sentOn(d, channel)} {relTime(d.sentAt)} · not watching for a reply</span>;
    case "gaveUp": return <GaveUpText empName={empName} />;
  }
}

function HeardLine({ heard }: { heard: Heard }) {
  const clipped = heard.text.length > QUOTE_MAX;
  return (
    <p className="req-heard" title={clipped ? heard.text : undefined}>
      <b>{firstName(heard.from)}:</b> “{clipped ? `${heard.text.slice(0, QUOTE_MAX).trimEnd()}…` : heard.text}”
      <span className="req-tag" title={heard.note}>{VERDICT[heard.verdict]}</span>
    </p>
  );
}

function watchNote(action: WatchAction, before: HumanRequest, after: HumanRequest, emp: string, person: string): { text: string; tone: Toast["tone"] } {
  switch (action) {
    case "check": {
      if (after.state !== "open") return { text: `${person} replied. ${emp} picks the work back up.`, tone: "ok" };
      const d = after.delivery;
      const w = d?.watch;
      const prev = before.delivery?.watch;
      if (d && !readsNatively(d)) return { text: `${emp} is looking for ${person}'s reply`, tone: "neutral" };
      if (w && w.failures > (prev?.failures ?? 0)) return { text: "Couldn't read the chat. The request says why.", tone: "err" };
      if (w?.heard && w.heard.at !== prev?.heard?.at) return { text: `${person} replied: ${VERDICT[w.heard.verdict]}`, tone: "ok" };
      return { text: `No reply from ${person} yet`, tone: "neutral" };
    }
    case "nudge": return { text: `${emp} is nudging ${person}`, tone: "ok" };
    case "stop": return { text: `Stopped watching for ${person}'s reply`, tone: "neutral" };
    case "resume": return { text: `Watching for ${person}'s reply again`, tone: "ok" };
  }
}

/** Who asked whom, in words: "Hazel asked Robert", "Felix gave Grace a task", "Leo told Grace". */
function Who({ r, emp, person, employee, showPerson }: { r: HumanRequest; emp: string; person: PersonView | undefined; employee: EmployeeView | undefined; showPerson: boolean }) {
  const name = <button className="req-name" onClick={() => employee && openChat(employee.id)}>{emp}</button>;
  const them = showPerson && person ? <button className="req-name" onClick={() => openMember(person.id)}>{firstName(person.name)}</button> : null;
  if (r.kind === "task") return them ? <>{name} <span className="muted">gave</span> {them} <span className="muted">a task</span></> : <>{name} <span className="muted">gave them a task</span></>;
  if (r.kind === "info") return <>{name} <span className="muted">told</span> {them ?? <span className="muted">them</span>}</>;
  return <>{name} <span className="muted">asked</span> {them ?? <span className="muted">them</span>}</>;
}

/** A request an employee sent a real person. Open: the one action you'd take, the rest in a menu. Settled: the outcome. */
export function RequestCard({ r, employee, person, showPerson = false, onChanged }: {
  r: HumanRequest; employee: EmployeeView | undefined; person: PersonView | undefined; showPerson?: boolean; onChanged?: (r: HumanRequest) => void;
}) {
  const [mode, setMode] = useState<"idle" | "reply">("idle");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [checking, setChecking] = useState(false);
  const [acting, setActing] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const channel = person?.channels.find((c) => c.id === r.channelId);
  const empName = employee ? firstName(employee.name) : "An employee";
  const personName = person ? firstName(person.name) : "them";
  const tone = dueTone(r.dueAt);
  const open = r.state === "open";
  const watch = r.delivery?.watch ?? null;
  const watching = watch?.phase.type === "watching";

  const resolve = async (state: "answered" | "done" | "cancelled", response: string | null) => {
    setBusy(true);
    setErr(null);
    try {
      const next = await call("resolve_request", { requestId: r.id, state, response });
      onChanged?.(next);
      refreshOrg();
      setMode("idle");
      setText("");
      toast(state === "cancelled" ? "Request cancelled" : `Sent to ${empName}. They pick the work back up.`, "ok");
    } catch (x) {
      setErr(errorMessage(x));
    } finally {
      setBusy(false);
    }
  };

  const act = async (action: WatchAction) => {
    const setPending = action === "check" ? setChecking : setActing;
    setPending(true);
    setErr(null);
    try {
      const next = await call("watch_request", { requestId: r.id, action });
      onChanged?.(next);
      const note = watchNote(action, r, next, empName, personName);
      toast(note.text, note.tone);
    } catch (x) {
      setErr(errorMessage(x));
    } finally {
      setPending(false);
    }
  };

  const more: MenuItem[] = [
    ...(watching ? [
      { label: checking ? "Checking…" : "Check for a reply now", icon: "refresh" as const, disabled: checking, onSelect: () => void act("check") },
      { label: `Nudge ${personName} now`, icon: "bell" as const, disabled: acting, onSelect: () => void act("nudge") },
    ] : []),
    ...(watch ? [{ label: watching ? "Stop watching for a reply" : "Watch for a reply again", icon: "clock" as const, disabled: acting, onSelect: () => void act(watching ? "stop" : "resume") }] : []),
    ...(r.runId ? [{ label: "Open the run that asked", icon: "arrowUpRight" as const, onSelect: () => r.runId && openRun(r.runId) }] : []),
    { label: "Cancel request", icon: "close", danger: true, disabled: busy, onSelect: () => void resolve("cancelled", null) },
  ];

  return (
    <article className={`req ${open ? "req-open" : "req-closed"}`}>
      <header className="req-head">
        {employee ? <Avatar name={employee.name} hue={employee.hue} size={20} /> : null}
        <span className="req-who">
          <Who r={r} emp={empName} person={person} employee={employee} showPerson={showPerson} />
          <span className="faint"> · {relTime(r.createdAt)}</span>
        </span>
        {r.dueAt && open ? <span className={`req-due req-due-${tone}`}>{tone === "late" ? "Overdue since" : "Due"} {whenInline(r.dueAt)}</span> : null}
      </header>
      <p className="req-msg">{r.message}</p>
      {open || r.kind === "info" ? <p className="req-status"><Status r={r} channel={channel} empName={empName} /></p> : null}
      {open && watch?.heard ? <HeardLine heard={watch.heard} /> : null}
      {open && watching && watch.lastError && watch.failures > 0 ? <p className="req-warn"><Icon name="alert" size={12} />Couldn't check for a reply: {watch.lastError}</p> : null}
      {r.kind === "info" ? null : !open ? (
        <div className="req-resolved">
          <span className={`pill ${r.state === "cancelled" ? "" : "pill-ok"}`}>{r.state === "answered" ? "Answered" : r.state === "done" ? "Done" : "Cancelled"}</span>
          {r.response ? <span className="req-response">“{r.response}”</span> : null}
          {r.resolvedAt ? <span className="faint">{relTime(r.resolvedAt)}</span> : null}
        </div>
      ) : mode === "reply" ? (
        <div className="req-reply">
          <textarea className="textarea" rows={3} autoFocus value={text} onChange={(e) => setText(e.target.value)}
            placeholder={r.kind === "task" ? `What did ${personName} do? ${empName} continues from this.` : `${personName}'s answer, in their words or yours.`}
            onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && text.trim()) { e.preventDefault(); void resolve(r.kind === "task" ? "done" : "answered", text.trim()); } }} />
          {err ? <ErrorNote text={err} /> : null}
          <div className="row">
            <span className="grow" />
            <button className="btn btn-ghost btn-sm" onClick={() => setMode("idle")}>Cancel</button>
            <button className="btn btn-primary btn-sm" disabled={busy || !text.trim()} onClick={() => void resolve(r.kind === "task" ? "done" : "answered", text.trim())}>
              Send to {empName}
            </button>
          </div>
        </div>
      ) : (
        <>
          {err ? <ErrorNote text={err} /> : null}
          <div className="req-actions">
            {r.kind === "task" ? (
              <>
                <button className="btn btn-sm" disabled={busy} onClick={() => void resolve("done", `${personName} marked it done.`)}><Icon name="check" size={12} /> Mark done</button>
                <button className="btn btn-sm btn-ghost" onClick={() => setMode("reply")}>Reply for {personName}</button>
              </>
            ) : (
              <button className="btn btn-sm" onClick={() => setMode("reply")}><Icon name="reply" size={12} /> Answer for {personName}</button>
            )}
            <span className="grow" />
            <MenuButton items={more} label="More for this request" />
          </div>
        </>
      )}
    </article>
  );
}
