import { useMemo, useState } from "react";
import { call, errorMessage } from "../api/client";
import type { EmployeeView, Run } from "../api/types";
import { mcpServerName, mcpServerRule, toolName } from "../lib/rules";
import { dateLong, duration, elapsedMs, firstName, runKindLabel, runStateLabel, runStateTone } from "../lib/format";
import { refreshOrg, useNow, useOrg, useProposals, useQuery, useRunDetail } from "../lib/hooks";
import { toast } from "../lib/toasts";
import { navigate, openChat, openMember, openRun } from "../lib/router";
import { threadFocus } from "../lib/threads";
import { kindNames } from "../components/RunRow";
import { Icon } from "../components/Icon";
import { Markdown } from "../components/Markdown";
import { SubAgents } from "../components/SubAgents";
import { TidyProposal } from "../components/TidyProposal";
import { DelegatedCard, ProposedRow, RequestedRow, SkillChip, ToolRows, toItems } from "../components/Transcript";
import { Avatar, ConfirmButton, ErrorNote, Skeleton, StatePill } from "../components/ui";
import "./RunView.css";

export function RunView({ runId }: { runId: string }) {
  const { detail, error, reload } = useRunDetail(runId);
  const { snap } = useOrg();
  const run = detail?.run;
  const live = run?.state === "running" || run?.state === "queued";
  const now = useNow(1000, !!live);
  const e = snap?.employees.find((x) => x.id === run?.employeeId);
  const requests = useQuery(() => (run ? call("requests", { personId: null, employeeId: run.employeeId, state: null }) : Promise.resolve([])), [run?.employeeId], ["request:updated"]);
  const proposals = useProposals();
  const items = useMemo(() => toItems(detail?.events ?? []), [detail?.events]);
  const back = () => (history.length > 1 ? history.back() : navigate({ page: "org", memberId: null, tab: null, focus: null }));

  if (error) {
    return (
      <div className="runview"><div className="runview-inner">
        <div className="empty runview-missing">
          <Icon name="activity" size={22} />
          <span className="empty-title">That run could not be found</span>
          <p>{/not.?found|could not be found/i.test(error) ? "The link may be out of date, or the run belonged to someone who has left." : error}</p>
          <div className="row">
            <button className="btn btn-ghost" onClick={back}><Icon name="arrowLeft" size={13} /> Back</button>
            <button className="btn" onClick={reload}>Try again</button>
          </div>
        </div>
      </div></div>
    );
  }
  if (!detail || !run) {
    return (
      <div className="runview"><div className="runview-inner stack" style={{ gap: 14 }}>
        <Skeleton h={28} w={260} /><Skeleton h={18} w={380} /><Skeleton h={120} r={12} /><Skeleton h={60} r={12} />
      </div></div>
    );
  }

  const names = kindNames(run, snap?.employees ?? [], snap?.people ?? []);
  const label = runKindLabel(run.kind, names);
  const tone = runStateTone(run);
  const result = [...items].reverse().find((i) => i.kind === "result");
  const cancel = async () => {
    try { await call("cancel_run", { runId: run.id }); toast("Run cancelled", "ok"); }
    catch (x) { toast(errorMessage(x), "err"); }
  };

  return (
    <div className="runview">
      <div className="runview-inner">
      <header className="runview-head">
          <button className="btn btn-ghost btn-sm runview-back" onClick={back}><Icon name="arrowLeft" size={13} /> Back</button>
          <div className="runview-title">
            {e ? <Avatar name={e.name} hue={e.hue} size={40} presence={live ? "working" : undefined} /> : null}
            <div className="stack grow" style={{ minWidth: 0 }}>
              <span className="runview-who">
                {e ? <button className="linkish" onClick={() => openMember(e.id, "work")}>{e.name}</button> : "Former employee"}
                <span className="faint"> · {e?.title}</span>
              </span>
              <h1 className="h-lg runview-label">{label}</h1>
            </div>
            <StatePill tone={tone} pulse={live}>{run.state === "succeeded" && run.denials.length ? "Done, something was blocked" : runStateLabel(run.state)}</StatePill>
            {live ? <ConfirmButton label="Cancel" icon="stop" className="btn" size="btn-sm" question="Stop this run?" confirmLabel="Stop it" onConfirm={cancel} /> : null}
          </div>
          <div className="runview-stats">
            <span>{run.startedAt ? dateLong(run.startedAt) : "Waiting to start"}</span>
            {run.startedAt ? <span className="num">{live ? "running for" : "took"} {duration(elapsedMs(run, now))}</span> : null}
            {run.kind.type === "delegated" || run.kind.type === "asked" ? <button className="runview-parent" onClick={() => (run.kind.type === "delegated" || run.kind.type === "asked") && openRun(run.kind.parentRunId)}>The run that asked <Icon name="arrowUpRight" size={12} /></button> : null}
            {run.kind.type === "answered" ? <button className="runview-parent" onClick={() => run.kind.type === "answered" && openRun(run.kind.askedRunId)}>Their run <Icon name="arrowUpRight" size={12} /></button> : null}
            {run.kind.type === "reply" && e ? <button className="runview-parent" onClick={() => run.kind.type === "reply" && openChat(e.id, threadFocus(run.kind.toRunId))}>Open {run.kind.task ? "task" : "thread"} <Icon name="arrowUpRight" size={12} /></button> : null}
            {run.kind.type === "task" && e ? <button className="runview-parent" onClick={() => openChat(e.id, threadFocus(run.id))}>Open task <Icon name="arrowUpRight" size={12} /></button> : null}
          </div>
      </header>
      <div className="runview-body">
          <section className="rv-prompt">
            <span className="rv-prompt-label">{run.kind.type === "followUp" ? "Picked up with" : run.kind.type === "delegated" ? `From ${names.manager ?? "their manager"}` : run.kind.type === "asked" ? `${names.asker ?? "A colleague"} ${run.kind.task ? "handed over" : "asked"}` : run.kind.type === "answered" ? `${names.asker ?? "A colleague"} got back to them` : "Instruction"}</span>
            <p>{run.prompt}</p>
          </section>
          <SubAgents tasks={detail.tasks} runActive={!!live} />
          <div className="rv-events">
            {items.map((it) => {
              switch (it.kind) {
                case "user": return <section key={it.key} className="rv-prompt"><span className="rv-prompt-label">You added</span><p>{it.text}</p></section>;
                case "init": return <div key={it.key} className="rv-init mono"><Icon name="play" size={11} /> Started on {it.model} · {it.skills.length} skills available</div>;
                // The closing message is the report, shown once below rather than twice.
                case "text": return result?.kind === "result" && result.text?.trim() === it.text.trim() ? null : <Markdown key={it.key} text={it.text} />;
                case "tools": return <ToolRows key={it.key} tools={it.tools} running={!!live} tasks={detail.tasks} />;
                case "skill": return <SkillChip key={it.key} skill={it.skill} file={it.file} change={it.change} onClick={e ? () => openMember(e.id, "skills", `diff~${run.id}~${it.skill}~${it.file}`) : undefined} />;
                case "delegated": return <DelegatedCard key={it.key} childRunId={it.childRunId} employeeId={it.employeeId} task={it.task} asked={it.asked} handoff={it.handoff} employees={snap?.employees ?? []} child={detail.children.find((c) => c.id === it.childRunId)} />;
                case "requested": return <RequestedRow key={it.key} personId={it.personId} reqKind={it.reqKind} message={it.message} people={snap?.people ?? []} request={requests.data?.find((r) => r.id === it.requestId)} />;
                case "proposed": return <ProposedRow key={it.key} name={it.name} title={it.title} proposal={proposals?.find((p) => p.id === it.proposalId)} />;
                case "stderr": return <pre key={it.key} className="rv-stderr mono">{it.text}</pre>;
                case "result": return null;
              }
            })}
            {detail.live ? <Markdown text={detail.live} className="md-live" /> : null}
            {live && !detail.live ? <span className="thinking"><span /><span /><span /></span> : null}
          </div>
          {run.denials.length && e ? <Denials run={run} e={e} /> : null}
          <TidyProposal run={run} employee={e} />
          {result && result.kind === "result" ? (
            <section className={`rv-result ${result.ok ? "" : "rv-result-err"}`}>
              <div className="rv-result-head">
                <Icon name={result.ok ? "checkCircle" : "alert"} size={15} />
                <span>{result.ok ? "Report" : "Didn't finish"}</span>
                <span className="grow" />
                <span className="num faint">{duration(elapsedMs(run))}</span>
              </div>
              {result.text ? <Markdown text={result.text} /> : <p className="muted">No report.</p>}
              {run.skillChanges.length ? (
                <div className="rv-grew">
                  <Icon name="sparkle" size={13} /> {e ? firstName(e.name) : "They"} changed {run.skillChanges.length} {run.skillChanges.length === 1 ? "skill" : "skills"} during this run.
                </div>
              ) : null}
            </section>
          ) : !live && run.error ? <ErrorNote text={run.error} /> : null}
          {detail.children.length ? (
            <section className="rv-children">
              <h2 className="rv-section-title">{detail.children.every((c) => c.kind.type === "asked") ? "Asked of colleagues" : detail.children.some((c) => c.kind.type === "asked") ? "Handed to colleagues and asked" : "Handed to colleagues"}</h2>
              {detail.children.map((c) => <ChildRow key={c.id} c={c} employees={snap?.employees ?? []} />)}
            </section>
          ) : null}
      </div>
      </div>
    </div>
  );
}

function ChildRow({ c, employees }: { c: Run; employees: EmployeeView[] }) {
  const e = employees.find((x) => x.id === c.employeeId);
  return (
    <button className="rv-child" onClick={() => openRun(c.id)}>
      {e ? <Avatar name={e.name} hue={e.hue} size={24} presence={c.state === "running" ? "working" : undefined} /> : null}
      <span className="grow" style={{ minWidth: 0 }}><b>{e?.name ?? "Former employee"}</b> <span className="muted">{c.prompt}</span></span>
      <StatePill tone={runStateTone(c)} pulse={c.state === "running"}>{runStateLabel(c.state)}</StatePill>
    </button>
  );
}

function Denials({ run, e }: { run: Run; e: EmployeeView }) {
  const [allowed, setAllowed] = useState<Set<string>>(() => new Set(run.denials.flatMap((d) => [d.suggestedRule, mcpServerRule(d.tool) ?? ""]).filter((r) => r && e.allowRules.includes(r))));
  const allow = async (rule: string) => {
    try {
      await call("allow_rule", { employeeId: e.id, rule });
      setAllowed(new Set(allowed).add(rule));
      refreshOrg();
      toast(`${firstName(e.name)} can now use ${rule} without asking`, "ok");
    } catch (x) {
      toast(errorMessage(x), "err");
    }
  };
  return (
    <section className="rv-denials">
      <div className="rv-denials-head"><Icon name="shield" size={14} /> {run.denials.length === 1 ? "One action was blocked" : `${run.denials.length} actions were blocked`}</div>
      <p className="muted" style={{ fontSize: "var(--t-sm)" }}>{firstName(e.name)} is supervised, so anything you haven't allowed is blocked. Allow it here and it isn't blocked next time.</p>
      {run.denials.map((d, i) => {
        const server = mcpServerRule(d.tool);
        const covered = allowed.has(d.suggestedRule) || (server !== null && allowed.has(server));
        return (
          <div key={i} className="rv-denial">
            <div className="stack grow" style={{ minWidth: 0 }}>
              <span className="rv-denial-what"><b className="mono">{toolName(d.tool)}</b>{mcpServerName(d.tool) ? <span className="pill">{mcpServerName(d.tool)}</span> : null}</span>
              <span className="mono rv-denial-brief" title={`Rule: ${d.suggestedRule}`}>{d.brief}</span>
            </div>
            {covered ? (
              <span className="pill pill-ok"><Icon name="check" size={11} /> Allowed</span>
            ) : (
              <span className="row" style={{ gap: 6, flex: "none" }}>
                <button className="btn btn-sm" onClick={() => void allow(d.suggestedRule)}>Allow from now on</button>
                {server ? <button className="btn btn-sm" onClick={() => void allow(server)} title={`Every tool of ${mcpServerName(d.tool)}: ${server}`}>Allow all {mcpServerName(d.tool)} tools</button> : null}
              </span>
            )}
          </div>
        );
      })}
    </section>
  );
}
