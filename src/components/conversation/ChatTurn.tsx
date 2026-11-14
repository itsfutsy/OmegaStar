import { Fragment, useMemo, type ReactNode } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView, Run } from "../../api/types";
import { firstName, heldLabel, threadTitle, timeOrDay, dateLong } from "../../lib/format";
import { useNow, useOrg, useProposals, useQuery, useUsage } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import type { LiveRun } from "../../lib/runs";
import { navigate, openMember, openRun } from "../../lib/router";
import { didNotFinish, isLive } from "../../lib/threads";
import { kindNames } from "../../lib/workTitle";
import { Icon } from "../Icon";
import { Markdown } from "../Markdown";
import { ProposedRow, RequestedRow, toItems, type TranscriptItem } from "../Transcript";
import { Visual } from "../Visual";
import { Avatar, ErrorNote } from "../ui";
import { WorkLine } from "./WorkLine";
import { partsOf } from "./work";
import "./Chat.css";

const isImage = (name: string) => /\.(png|jpe?g|gif|webp|bmp|heic)$/i.test(name);

/** Who a reply is from and when it finished, or "up next". While it runs, its work line says so, not this. Their face and
 *  name are a button to their profile, the same page the header's More menu opens. */
export function WhoLine({ e, run, label, last = true }: { e: EmployeeView; run: Run; label?: string; last?: boolean }) {
  const at = run.endedAt;
  const meta = label ? <span className="who-meta">{label}</span>
    : !last ? null
    : run.state === "queued" ? <span className="who-meta">up next</span>
    : at ? <span className="who-meta num" title={dateLong(at)}>{timeOrDay(at)}</span> : null;
  return (
    <div className="who-line">
      <button className="who-open" onClick={() => openMember(e.id, "profile")} aria-label={`Open ${firstName(e.name)}'s profile`}>
        <Avatar name={e.name} hue={e.hue} image={e.image} size={22} />
        <b>{firstName(e.name)}</b>
      </button>
      {meta}
    </div>
  );
}

/**
 * One exchange in a conversation: what you sent (with any files), then the employee's reply as it streams in. A message
 * you sent while it worked splits the reply where it landed. Each part reads as a colleague talking: one line under
 * their name for the work behind it (WorkLine), their first words, any page they showed, then the answer.
 * A finished run's report shows when its transcript did not already say it. `footer` sits at the end of the
 * reply, on its text edge: the cards it asks, a blocked action, Retry.
 */
export function Turn({ turn, e, footer, earlier = false, latestMine = false }: {
  turn: LiveRun; e: EmployeeView; footer?: ReactNode;
  /** A turn before the latest: its words step back to the secondary ink, so the latest exchange leads. */
  earlier?: boolean;
  /** This turn holds your latest message, the one solid block; your earlier messages are a quiet step. */
  latestMine?: boolean;
}) {
  const { snap } = useOrg();
  const proposals = useProposals();
  const items = useMemo(() => toItems(turn.events), [turn.events]);
  const parts = useMemo(() => partsOf(items), [items]);
  const waiting = items.flatMap((it) => (it.kind === "user" && it.waiting ? [it] : []));
  const running = isLive(turn.run);
  const queued = turn.run.state === "queued";
  const failed = didNotFinish(turn.run);
  const report = turn.run.summary?.trim();
  const showReport = !running && !!report && !items.some((i) => i.kind === "text" && i.text.trim() === report);
  // A colleague's report or answer that came back, or a check that came due: Omega Star wrote the prompt, so it is not
  // shown as yours.
  const came = turn.run.kind.type === "answered" ? turn.run.kind : null;
  const fromName = came ? snap?.employees.find((x) => x.id === came.from)?.name : undefined;
  const who = fromName ? firstName(fromName) : "A colleague";
  // Your messages in this turn, in order; the last of them is your latest when `latestMine`.
  const mine = [turn.run.prompt && !came && !turn.run.check ? "prompt" : null, ...parts.map((p, i) => (p.said !== null ? `said:${i}` : null))].filter(Boolean) as string[];
  const lastMine = latestMine ? mine[mine.length - 1] : null;
  const me = (key: string) => `msg-me ${key === lastMine ? "msg-me-latest" : ""}`;
  const result = (it: TranscriptItem) => {
    switch (it.kind) {
      case "requested": return <RequestedRow key={it.key} personId={it.personId} reqKind={it.reqKind} message={it.message} people={snap?.people ?? []} />;
      case "proposed": return <ProposedRow key={it.key} name={it.name} title={it.title} proposal={proposals?.find((p) => p.id === it.proposalId)} />;
      case "visual": return <Visual key={it.key} runId={turn.run.id} id={it.id} title={it.title} from={firstName(e.name)} />;
      case "result": return !it.ok && it.text ? <ErrorNote key={it.key} text={it.text} /> : null;
      default: return null;
    }
  };
  return (
    <div className={`turn ${earlier ? "turn-earlier" : ""}`}>
      {turn.run.attachments.length ? (
        <div className="msg-files">
          {turn.run.attachments.map((a) => (
            <button key={a} className="attach attach-sent" data-tip={`Open ${a}`} onClick={() => call("open_attachment", { runId: turn.run.id, name: a }).catch((err) => toast(errorMessage(err), "err"))}>
              <Icon name={isImage(a) ? "image" : "file"} size={13} /><span className="attach-name">{a}</span>
            </button>
          ))}
        </div>
      ) : null}
      {came ? (
        <details className="msg-came">
          <summary><Icon name="reply" size={14} /> {came.task ? `${who}'s report came back` : `${who} answered`}<Icon name="chevronDown" size={13} className="msg-came-chev" /></summary>
          <div className="msg-came-body"><Markdown text={turn.run.prompt} /></div>
        </details>
      ) : turn.run.check ? (
        <details className="msg-came">
          <summary><Icon name="clock" size={14} /> {firstName(e.name)}'s check came due<Icon name="chevronDown" size={13} className="msg-came-chev" /></summary>
          <div className="msg-came-body"><Markdown text={turn.run.prompt} /></div>
        </details>
      ) : turn.run.prompt ? <div className={me("prompt")}>{turn.run.prompt}</div> : null}
      {parts.map((part, i) => {
        const last = i === parts.length - 1;
        const rest = part.items.filter((it) => it.kind === "requested" || it.kind === "proposed" || it.kind === "visual" || it.kind === "result");
        const empty = !part.items.length && !(last && (turn.live || showReport || running || failed || footer));
        return (
          <Fragment key={i}>
            {part.said !== null ? <div className={me(`said:${i}`)}>{part.said}</div> : null}
            {empty ? null : (
              <div className="msg-emp">
                <WhoLine e={e} run={turn.run} last={last} />
                <div className="msg-body">
                  <WorkLine turn={turn} e={e} part={part} last={last} />
                  {part.opener ? <Markdown text={part.opener.text} /> : null}
                  {rest.map(result)}
                  {part.answer.map((t) => <Markdown key={t.key} text={t.text} />)}
                  {last ? (
                    <>
                      {/* The caret sits against the last letter: a chunk's trailing space is not shown. */}
                      {turn.live.trim() ? <Markdown text={turn.live.replace(/\s+$/, "")} className="md-live" /> : null}
                      {showReport ? <Markdown text={report} /> : null}
                      {queued ? <Waiting run={turn.run} e={e} /> : null}
                      {failed ? <p className="msg-failed"><Icon name="alert" size={14} /><span>{turn.run.error ?? "This reply didn't finish."}</span></p> : null}
                      {footer}
                    </>
                  ) : null}
                </div>
              </div>
            )}
          </Fragment>
        );
      })}
      {/* Yours, not taken yet: after the reply so far, as Claude Code queues a message under its work. */}
      {waiting.map((it) => (
        <div key={it.key} className="msg-wait">
          <div className="msg-me">{it.text}</div>
          <span className="msg-wait-note">{running ? `Waiting for ${firstName(e.name)} to finish this step` : `${firstName(e.name)} didn't see this before the reply ended`}</span>
        </div>
      ))}
    </div>
  );
}

/** A run that has not started: what it waits for (the usage limit, the runs ahead of it, a free slot), and what is ahead
 *  of it in its employee's line, each opening its run. */
export function Waiting({ run, e }: { run: Run; e: EmployeeView }) {
  const { usage } = useUsage();
  const { snap } = useOrg();
  const now = useNow(60_000);
  const lanes = useQuery(() => call("settings"), []).data?.maxConcurrentRuns ?? 1; // also each employee's limit
  const name = firstName(e.name);
  const others = e.live.filter((r) => r.id !== run.id);
  const working = others.filter((r) => r.state === "running");
  const ahead = others.filter((r) => r.state === "queued" && r.createdAt < run.createdAt);
  const held = usage?.heldUntil && Date.parse(usage.heldUntil) > now ? usage.heldUntil : null;
  const why = held ? `${heldLabel(held, new Date(now))}.`
    : working.length >= lanes ? `${name} starts this once ${working.length === 1 ? "the run below" : "one of the runs below"} finishes.`
    : ahead.length ? `${name} starts this after ${ahead.length === 1 ? "the run" : `the ${ahead.length} runs`} ahead of it.`
    : `${name} starts this as soon as there's room.`;
  const line = [...working, ...ahead];
  return (
    <div className="task-wait">
      <p className="task-wait-why">{why}</p>
      {line.length ? (
        <div className="task-wait-list">
          <span className="task-wait-label">Ahead of it</span>
          {line.map((r) => (
            <button key={r.id} type="button" className="task-wait-row" onClick={() => openRun(r.id)}>
              <span className="task-wait-what">{threadTitle(r, kindNames(r, snap?.employees ?? [], snap?.people ?? []))}</span>
              <span className={`status ${r.state === "running" ? "status-live" : "status-wait"}`}>{r.state === "running" ? "Working" : "Up next"}</span>
            </button>
          ))}
        </div>
      ) : held ? null : (
        <button className="btn btn-ghost btn-sm task-wait-link" onClick={() => navigate({ page: "settings" })}>
          Change how many run at once <Icon name="arrowUpRight" size={12} />
        </button>
      )}
    </div>
  );
}
