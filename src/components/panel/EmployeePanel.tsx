import { useEffect, useRef, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView } from "../../api/types";
import { headDragRegion } from "../../lib/chrome";
import { elapsedMs, firstName, relTime } from "../../lib/format";
import { useNow, useOrg } from "../../lib/hooks";
import { employeeState, numberParts, type MemberState } from "../../lib/memberState";
import { toast } from "../../lib/toasts";
import { openChat, openMember, openRun, openWork, type PanelTab } from "../../lib/router";
import { shortTask } from "../../lib/workTitle";
import { conversationFor, useStatuses } from "../conversation/useStatuses";
import { Icon } from "../Icon";
import { Avatar, Popover, Roll, Tabs } from "../ui";
import { WorkTab } from "./WorkTab";
import { SkillsTab } from "./SkillsTab";
import { ProfileTab } from "./ProfileTab";
import "./Panel.css";

const TABS: { id: PanelTab; label: string }[] = [
  { id: "work", label: "Work" }, { id: "skills", label: "Skills" }, { id: "profile", label: "Profile" },
];

/**
 * An employee's page beside the outline: who they are, one band saying what they are doing now, the tabs, and the
 * body. You talk to them in their conversation: Message opens it, on what matters now.
 */
export function EmployeePanel({ employee: e, tab, focus, short, onClose }: { employee: EmployeeView; tab: PanelTab | null; focus: string | null; short: boolean; onClose: () => void }) {
  const { snap } = useOrg();
  const current = tab ?? "work";
  const managerName = e.managerId
    ? snap?.employees.find((x) => x.id === e.managerId)?.name ?? snap?.people.find((x) => x.id === e.managerId)?.name ?? "None"
    : null;
  const s = employeeState(e, snap?.employees ?? [], snap?.people ?? []);
  // On a short window a quiet band (nothing running, nothing needing you) folds into the header's line, so the tab
  // gets that room; the outline already shows who they report to.
  const fold = short && s.tone === "plain";

  return (
    <section className={`mpage ${short ? "mpage-short" : ""}`} aria-label={`${e.name}, ${e.title}`}>
      <PageHead
        av={<Avatar name={e.name} hue={e.hue} image={e.image} size={short ? 32 : 56} />}
        name={e.name} role={fold ? `${e.title} · ${quietLine(e, s)}` : `${e.title} · Reports to ${managerName ?? "you"}`}
        actions={<><Message e={e} /><RunNow e={e} /></>} onClose={onClose}
      />
      {fold ? null : <Band e={e} s={s} tab={current} />}
      <div className="mpage-tabs">
        <Tabs tabs={TABS} value={current} onChange={(t) => openMember(e.id, t, null, true)} />
      </div>
      <div key={current} className="mpage-body enter-rise">
        {current === "work" ? <WorkTab e={e} /> : null}
        {current === "skills" ? <SkillsTab e={e} focus={focus} /> : null}
        {current === "profile" ? <ProfileTab e={e} onGone={onClose} /> : null}
      </div>
    </section>
  );
}

/** The identity row every page shares: avatar, the name in Display, one line under it, then actions and Close. Focus
 *  lands on the name when the page opens, so the page is announced and nothing invites typing. */
export function PageHead({ av, name, role, actions, onClose }: {
  av: React.ReactNode; name: string; role: React.ReactNode; actions?: React.ReactNode; onClose: () => void;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => { heading.current?.focus({ preventScroll: true }); }, [name]);
  return (
    <header className="mpage-head" {...headDragRegion}>
      {av}
      <div className="mpage-id">
        <h2 className="mpage-name" ref={heading} tabIndex={-1}>{name}</h2>
        <span className="mpage-role">{role}</span>
      </div>
      <div className="mpage-acts">
        {actions}
        <button className="btn btn-ghost btn-icon" aria-label="Close" data-tip="Close" onClick={onClose}><Icon name="close" size={16} /></button>
      </div>
    </header>
  );
}

/** What is true of someone with nothing running and nothing needing you: when they next run, else when they last
 *  worked, else that they work when asked. Never "No runs yet": the Work tab says that once. */
function quietLine(e: EmployeeView, s: MemberState, now = Date.now()): string {
  if (s.word !== "Works when asked") return s.word;
  return e.lastWorkedAt ? `Last worked ${relTime(e.lastWorkedAt, now)}` : "Works when asked";
}

/** One full-width band, one line: on the left what they are doing now (or what is true instead), on the right their
 *  number, and their last report only when it is a few words ("on track"); a longer report is on the Work tab. The task
 *  is its short name, the one the chart card uses, so it never wraps or cuts; what is up next is on the Work tab. Nothing
 *  is said twice: on the Work tab, a failure has its own card, so the band gives the state only. Working is
 *  ultramarine, needing you is coral, the rest is quiet. */
function Band({ e, s, tab }: { e: EmployeeView; s: MemberState; tab: PanelTab }) {
  const running = e.live.filter((r) => r.state === "running");
  const now = useNow(15_000, running.length > 0);
  const n = numberParts(e.status.number);
  const reportWords = e.status.lastReport ? lastReportWords(e.status.lastReport.text) : null;
  const report = reportWords ? e.status.lastReport : null;
  const minutes = s.run && s.tone === "live" ? Math.max(1, Math.round((elapsedMs(s.run, now) ?? 0) / 60_000)) : null;

  let left: React.ReactNode;
  if (s.tone === "live" || s.tone === "wait") {
    const run = s.run;
    left = (
      <>
        <span className="band-word">{s.word}</span>
        {!s.about ? null : (
          <>
            <span className="band-sep" aria-hidden>·</span>
            <button className="band-what" onClick={() => run && openWork(run)} data-tip={s.about}>{shortTask(s.about)}</button>
          </>
        )}
        {minutes !== null ? <><span className="band-sep" aria-hidden>·</span><span className="band-meta num"><Roll value={minutes} /> min</span></> : null}
      </>
    );
  } else if (s.tone === "attn" || s.tone === "fail") {
    const run = s.run;
    left = (
      <>
        <span className="band-word">{s.word}</span>
        {run && tab !== "work" ? <button className="band-more" onClick={() => openRun(run.id)}>Take a look <Icon name="arrowRight" size={11} /></button> : null}
      </>
    );
  } else {
    left = <span className="band-text">{quietLine(e, s, now)}</span>;
  }

  return (
    <div className={`band band-employee band-${s.tone}`}>
      <div className="band-left">{left}</div>
      {n || report ? (
        <div className="band-right">
          {n ? (
            <span className="band-m" data-tip={e.status.number ?? undefined}>
              {n.metric ? `${n.metric} ` : ""}<b className="num">{n.value}</b>{n.change ? <>, <span className="band-change">{n.change}</span></> : null}
            </span>
          ) : null}
          {n && report ? <span className="band-div" aria-hidden /> : null}
          {report ? <span className="band-m band-report" data-tip={`Reported ${relTime(report.at, now)}: ${report.text}`}>Last report: {reportWords}</span> : null}
        </div>
      ) : null}
    </div>
  );
}

/** The last report's status line when it is a few words: "On track. One open item." becomes "on track". Anything
 *  longer is null: the band leaves it to the Work tab rather than cut it. */
function lastReportWords(text: string): string | null {
  const first = text.split(/(?<=[.!?])\s/)[0].replace(/[.!?]+$/, "").trim();
  return first.length > 0 && first.length <= 24 ? first.charAt(0).toLowerCase() + first.slice(1) : null;
}

/** Their conversation, on what matters now: what needs you, what they are working on, else the newest. */
function Message({ e }: { e: EmployeeView }) {
  const { items } = useStatuses();
  return (
    <button className="btn btn-sm btn-primary" onClick={() => openChat(e.id, conversationFor(e, items))}>
      <Icon name="chat" size={13} /> Message
    </button>
  );
}

function RunNow({ e }: { e: EmployeeView }) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLButtonElement>(null);
  const go = async () => {
    setBusy(true);
    try {
      const r = await call("run_now", { employeeId: e.id, instruction: text.trim() || null });
      setOpen(false);
      setText("");
      toast(r.state === "queued" && e.presence === "working" ? `Up next. ${firstName(e.name)} will start when the current run ends` : `${firstName(e.name)} is on it`, "ok", { label: "View run", run: () => openRun(r.id) });
    } catch (err) {
      toast(errorMessage(err), "err");
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <button ref={ref} className="btn btn-sm" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Icon name="play" size={12} /> Run now
      </button>
      <Popover anchor={ref} open={open} onClose={() => setOpen(false)} width={340}>
        <div className="stack" style={{ gap: 10 }} data-esc-stop>
          <label className="field">
            <span className="field-label">Anything specific? <span className="hint">optional</span></span>
            <textarea
              className="textarea" rows={3} autoFocus value={text} onChange={(ev) => setText(ev.target.value)}
              placeholder={`Leave empty and ${firstName(e.name)} does their usual work.`}
              onKeyDown={(ev) => { if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) { ev.preventDefault(); void go(); } }}
            />
          </label>
          <div className="row">
            <span className="muted" style={{ fontSize: "var(--t-sm)" }}>{e.presence === "working" ? "If they're busy with other work, this waits its turn." : "Starts right away."}</span>
            <span className="grow" />
            <button className="btn btn-primary btn-sm" disabled={busy} onClick={() => void go()}>Run {firstName(e.name)}</button>
          </div>
        </div>
      </Popover>
    </>
  );
}
