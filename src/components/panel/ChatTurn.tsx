import { Fragment, useMemo } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView } from "../../api/types";
import { useOrg, useProposals } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import type { LiveRun } from "../../lib/runs";
import { navigate, openMember } from "../../lib/router";
import { Icon } from "../Icon";
import { Markdown } from "../Markdown";
import { DelegatedCard, ProposedRow, RequestedRow, SkillChip, ToolRows, toItems, type TranscriptItem } from "../Transcript";
import { SubAgents } from "../SubAgents";
import { Avatar, ErrorNote } from "../ui";
import "./ChatTab.css";

const isImage = (name: string) => /\.(png|jpe?g|gif|webp|bmp|heic)$/i.test(name);

/**
 * One exchange in a conversation: what you sent (with any files), then the employee's reply as it streams in.
 * A message you sent while it worked splits the reply where it landed. `withReport` shows a finished run's
 * report when its transcript did not already say it (a thread reply is a work run, and its last words may
 * only be in the report).
 */
export function Turn({ turn, e, withReport = false }: { turn: LiveRun; e: EmployeeView; withReport?: boolean }) {
  const { snap } = useOrg();
  const proposals = useProposals();
  const items = useMemo(() => toItems(turn.events), [turn.events]);
  const parts = useMemo(() => {
    const out: { said: string | null; items: TranscriptItem[] }[] = [{ said: null, items: [] }];
    for (const it of items) {
      if (it.kind === "user") out.push({ said: it.text, items: [] });
      else out[out.length - 1].items.push(it);
    }
    return out;
  }, [items]);
  const running = turn.run.state === "running" || turn.run.state === "queued";
  const queued = turn.run.state === "queued";
  const hasText = parts[parts.length - 1].items.some((i) => i.kind === "text") || turn.live.length > 0;
  const failed = turn.run.state === "failed" || turn.run.state === "lost" || turn.run.state === "cancelled";
  const report = turn.run.summary?.trim();
  const showReport = withReport && !running && !!report && !items.some((i) => i.kind === "text" && i.text.trim() === report);
  return (
    <div className="turn">
      {turn.run.attachments.length ? (
        <div className="msg-files">
          {turn.run.attachments.map((a) => (
            <button key={a} className="attach attach-sent" title={`Open ${a}`} onClick={() => call("open_attachment", { runId: turn.run.id, name: a }).catch((err) => toast(errorMessage(err), "err"))}>
              <Icon name={isImage(a) ? "image" : "file"} size={13} /><span className="attach-name">{a}</span>
            </button>
          ))}
        </div>
      ) : null}
      {turn.run.prompt ? <div className="msg-ceo">{turn.run.prompt}</div> : null}
      {parts.map((part, i) => {
        const last = i === parts.length - 1;
        return (
          <Fragment key={i}>
            {part.said !== null ? <div className="msg-ceo">{part.said}</div> : null}
            {part.items.length || last ? (
              <div className="msg-emp">
                <Avatar name={e.name} hue={e.hue} size={24} presence={last && queued ? "queued" : last && running ? "working" : undefined} className="msg-av" />
                <div className="msg-body">
                  {part.items.map((it) => {
                    switch (it.kind) {
                      case "text": return <Markdown key={it.key} text={it.text} />;
                      case "tools": return <ToolRows key={it.key} tools={it.tools} running={running} compact tasks={turn.tasks} />;
                      case "skill": return <SkillChip key={it.key} skill={it.skill} file={it.file} change={it.change} onClick={() => openMember(e.id, "skills", `diff~${turn.run.id}~${it.skill}~${it.file}`)} />;
                      case "delegated": return <DelegatedCard key={it.key} childRunId={it.childRunId} employeeId={it.employeeId} task={it.task} asked={it.asked} handoff={it.handoff} employees={snap?.employees ?? []} child={turn.children.find((c) => c.id === it.childRunId)} />;
                      case "requested": return <RequestedRow key={it.key} personId={it.personId} reqKind={it.reqKind} message={it.message} people={snap?.people ?? []} />;
                      case "proposed": return <ProposedRow key={it.key} name={it.name} title={it.title} proposal={proposals?.find((p) => p.id === it.proposalId)} />;
                      case "result": return !it.ok && it.text ? <ErrorNote key={it.key} text={it.text} /> : null;
                      case "user": case "stderr": case "init": return null;
                    }
                  })}
                  {last ? (
                    <>
                      {turn.live ? <Markdown text={turn.live} className="md-live" /> : null}
                      {showReport ? <Markdown text={report} /> : null}
                      {queued ? <Waiting /> : running && !hasText ? <span className="thinking"><span /><span /><span /></span> : null}
                      {failed && !running ? <span className="msg-failed"><Icon name="alert" size={12} /> {turn.run.error ?? "This reply didn't finish."}</span> : null}
                      {/* While the turn runs its sub-agents are pinned above the composer instead. */}
                      {!running ? <SubAgents tasks={turn.tasks} runActive={false} defaultOpen={false} /> : null}
                    </>
                  ) : null}
                </div>
              </div>
            ) : null}
          </Fragment>
        );
      })}
    </div>
  );
}

/** A turn that has not started: it waits for the turn ahead of it, or for a free slot. */
function Waiting() {
  return (
    <div className="task-wait">
      <Icon name="clock" size={14} />
      <div className="stack">
        <span><b>Waiting to start.</b> It starts as soon as there's room.</span>
        <span>
          <button className="btn btn-ghost btn-sm task-wait-link" onClick={() => navigate({ page: "settings" })}>
            Change how many run at once <Icon name="arrowUpRight" size={12} />
          </button>
        </span>
      </div>
    </div>
  );
}
