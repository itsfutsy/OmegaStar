import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { EmployeeView } from "../../api/types";
import { useNow, useOrg } from "../../lib/hooks";
import { rollAnimations } from "../../lib/motion";
import type { LiveRun } from "../../lib/runs";
import { openMember, openRun } from "../../lib/router";
import { Icon } from "../Icon";
import { Markdown } from "../Markdown";
import { StepMark } from "../Transcript";
import { Avatar, Roll } from "../ui";
import { SponsorWords, usePauseLine } from "./Sponsor";
import { minutes, nowOf, rowsOf, summaryOf, type Part, type WorkRow } from "./work";
import "./WorkLine.css";

/**
 * One line in a reply for all the work behind it, under the name. While the reply runs: a still live dot and what it is
 * doing now, rolling to each new step, then how long it has been at it. Once done: a disclosure and a summary ("Worked
 * for 3 min, with Otto"). Opening it lists the steps; a colleague it asked opens their conversation, a skill it wrote
 * opens the change. `part` is one stretch of a reply split by a message you sent while it worked; without it, the whole
 * reply. `last`: the reply's last stretch, the only one that can still be running.
 */
export function WorkLine({ turn, e, part, last = true }: { turn: LiveRun; e: EmployeeView; part?: Part; last?: boolean }) {
  const { snap } = useOrg();
  const employees = snap?.employees;
  const rows = useMemo(() => rowsOf(turn, employees ?? [], part), [turn, employees, part]);
  const running = last && turn.run.state === "running";
  const [open, setOpen] = useState(false);
  const now = useNow(30_000, running);
  const n = running ? nowOf(rows, turn) : null;
  const said = usePauseLine(turn, e, part?.said ?? turn.run.prompt, n?.pause ?? null);
  if (!running && !rows.length) return null;
  const started = turn.run.startedAt ? Date.parse(turn.run.startedAt) : null;
  const ended = turn.run.endedAt ? Date.parse(turn.run.endedAt) : null;
  const summary = running ? "" : summaryOf(rows, last && started !== null && ended !== null ? ended - started : null);
  const elapsed = running && started !== null && now - started >= 60_000 ? minutes(now - started) : null;
  return (
    <div className={`wl ${running ? "wl-live" : ""}`}>
      <button
        type="button" className="wl-line" aria-expanded={rows.length ? open : undefined} disabled={!rows.length}
        onClick={() => setOpen((o) => !o)} data-tip={rows.length ? (open ? "Hide the steps" : "Show the steps") : undefined}
      >
        <span className="wl-mark" aria-hidden>{running ? <span className="wl-dot" /> : <Icon name="chevronRight" size={14} className="wl-chev" />}</span>
        {n ? (
          <RollLine k={said?.key ?? n.key}>
            {said?.ad ? <SponsorWords words={said.words} /> : <>
              <span className="wl-verb">{said?.words ?? n.verb}</span>
              {!said && n.object ? <span className={`wl-what ${n.code ? "mono" : ""}`}>{n.object}</span> : null}
            </>}
          </RollLine>
        ) : <span className="wl-summary">{summary}</span>}
        {elapsed ? <span className="wl-time num"><Roll value={elapsed} /></span> : null}
      </button>
      {rows.length ? (
        <div className="collapse" data-open={open}>
          <div>
            <div className="wl-rows" inert={!open}>
              {rows.map((r) => <StepRow key={r.key} row={r} e={e} />)}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

/** A line that rolls to its new words when `k` changes: the old one leaves as the new one arrives, in one slot. */
function RollLine({ k, children }: { k: string; children: ReactNode }) {
  const [roll, setRoll] = useState<{ k: string; from: ReactNode; on: boolean }>({ k, from: null, on: false });
  const last = useRef<ReactNode>(children);
  if (roll.k !== k) setRoll({ k, from: last.current, on: true });
  useLayoutEffect(() => { last.current = children; });
  const slot = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const el = slot.current;
    if (!roll.on || !el || el.children.length < 2) return;
    const anims = rollAnimations(el.children[0] as HTMLElement, el.children[1] as HTMLElement, true);
    let live = true;
    void Promise.all(anims.map((a) => a.finished)).then(() => { if (live) setRoll((r) => ({ ...r, from: null, on: false })); }, () => {});
    return () => { live = false; anims.forEach((a) => a.cancel()); };
  }, [roll.on, roll.k]);
  return (
    <span ref={slot} className="roll wl-roll">
      {roll.on ? <><span aria-hidden>{roll.from}</span><span>{children}</span></> : <span>{children}</span>}
    </span>
  );
}

/** One step: icon (or the colleague's face), verb, what it acted on, its meta and its mark. A colleague's row opens
 *  their conversation, a skill's opens the change, and a step with output opens it under the row. */
function StepRow({ row, e }: { row: WorkRow; e: EmployeeView }) {
  const [open, setOpen] = useState(false);
  const { skill, runId } = row;
  const act = skill ? () => openMember(e.id, "skills", `diff~${runId}~${skill.skill}~${skill.file}`)
    : row.key.startsWith("d:") ? () => openRun(runId)
    : row.detail ? () => setOpen((o) => !o)
    : null;
  const tip = skill ? "See what changed" : row.key.startsWith("d:") ? "Open their conversation" : undefined;
  const body = (
    <>
      {row.avatar ? <Avatar name={row.avatar.name} hue={row.avatar.hue} image={row.avatar.image} size={16} /> : <Icon name={row.icon} size={14} className="wl-icon" />}
      <span className="wl-text">
        <span className="wl-step">{row.verb}</span>
        <span className={`wl-obj ${row.code ? "mono" : ""}`}>{row.object}</span>
      </span>
      {row.meta ? <span className="wl-meta num">{row.meta}</span> : null}
      {row.tag ? <span className="tag">{row.tag}</span> : <StepMark status={row.state} />}
    </>
  );
  return (
    <div className="wl-item">
      {act ? (
        <button type="button" className="wl-row" aria-expanded={row.detail && !skill && !row.key.startsWith("d:") ? open : undefined} data-tip={tip} onClick={act}>{body}</button>
      ) : <div className="wl-row">{body}</div>}
      {row.detail ? (
        <div className="collapse" data-open={open}>
          <div>{row.key.startsWith("n:") ? <Markdown text={row.detail} className="wl-detail wl-note" /> : <p className="wl-detail">{row.detail}</p>}</div>
        </div>
      ) : null}
    </div>
  );
}
