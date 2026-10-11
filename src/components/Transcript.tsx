import { useState } from "react";
import type { EmployeeView, HireProposal, HumanRequest, PersonView, RequestKind, Run, RunEvent, SkillChangeKind } from "../api/types";
import { runStateLabel, runStateTone, firstName } from "../lib/format";
import type { TaskView } from "../lib/runs";
import { navigate, openRun } from "../lib/router";
import { Icon, type IconName } from "./Icon";
import { Avatar, StatePill } from "./ui";
import "./Transcript.css";

export type TranscriptItem =
  | { kind: "text"; key: string; text: string }
  | { kind: "user"; key: string; text: string }
  | { kind: "tools"; key: string; tools: ToolItem[] }
  | { kind: "skill"; key: string; skill: string; file: string; change: SkillChangeKind }
  | { kind: "delegated"; key: string; childRunId: string; employeeId: string; task: string; asked: boolean; handoff: boolean }
  | { kind: "requested"; key: string; requestId: string; personId: string; reqKind: RequestKind; message: string }
  | { kind: "proposed"; key: string; proposalId: string; name: string; title: string }
  | { kind: "init"; key: string; model: string; skills: string[] }
  | { kind: "result"; key: string; ok: boolean; text: string; turns: number; durationMs: number }
  | { kind: "stderr"; key: string; text: string };
type ToolItem = { id: string; tool: string; brief: string; result: { ok: boolean; brief: string } | null };

/** Fold raw events into display items: pair tool calls with results, group consecutive tools. */
export function toItems(events: RunEvent[]): TranscriptItem[] {
  const out: TranscriptItem[] = [];
  const toolIndex = new Map<string, ToolItem>();
  events.forEach((ev, i) => {
    const key = `e${i}`;
    switch (ev.type) {
      case "text": out.push({ kind: "text", key, text: ev.text }); break;
      case "user": out.push({ kind: "user", key, text: ev.text }); break;
      case "textDelta": break;
      case "toolCall": {
        const t: ToolItem = { id: ev.id, tool: ev.tool, brief: ev.brief, result: null };
        toolIndex.set(ev.id, t);
        const last = out[out.length - 1];
        if (last && last.kind === "tools") last.tools.push(t);
        else out.push({ kind: "tools", key, tools: [t] });
        break;
      }
      case "toolResult": {
        const t = toolIndex.get(ev.id);
        if (t) t.result = { ok: ev.ok, brief: ev.brief };
        break;
      }
      case "skillChanged": out.push({ kind: "skill", key, skill: ev.skill, file: ev.file, change: ev.change }); break;
      case "delegated": out.push({ kind: "delegated", key, childRunId: ev.childRunId, employeeId: ev.employeeId, task: ev.task, asked: false, handoff: false }); break;
      case "asked": out.push({ kind: "delegated", key, childRunId: ev.childRunId, employeeId: ev.employeeId, task: ev.question, asked: true, handoff: !!ev.task }); break;
      case "requested": out.push({ kind: "requested", key, requestId: ev.requestId, personId: ev.personId, reqKind: ev.kind, message: ev.message }); break;
      case "proposedHire": out.push({ kind: "proposed", key, proposalId: ev.proposalId, name: ev.name, title: ev.title }); break;
      case "init": out.push({ kind: "init", key, model: ev.model, skills: ev.skills }); break;
      case "result": out.push({ kind: "result", key, ok: ev.ok, text: ev.text, turns: ev.turns, durationMs: ev.durationMs }); break;
      case "stderr": out.push({ kind: "stderr", key, text: ev.text }); break;
      // Sub-agents are shown by SubAgents from the tasks foldEvent keeps, not as transcript items.
      case "taskStarted": case "taskProgress": case "taskEnded": break;
      default: { const never: never = ev; void never; }
    }
  });
  return out;
}

const FRIENDLY: Record<string, { label: string; icon: IconName }> = {
  mcp__omega__delegate: { label: "Delegate", icon: "branch" },
  mcp__omega__colleagues: { label: "Check colleagues", icon: "org" },
  mcp__omega__ask_colleague: { label: "Ask a colleague", icon: "ask" },
  mcp__omega__await_reports: { label: "Wait for reports", icon: "clock" },
  mcp__omega__team: { label: "Check the team", icon: "org" },
  mcp__omega__people: { label: "Look up people", icon: "person" },
  mcp__omega__request: { label: "Ask a person", icon: "ask" },
  mcp__omega__my_requests: { label: "Check requests", icon: "ask" },
  mcp__omega__update_request: { label: "Update request", icon: "ask" },
  mcp__omega__catalog: { label: "Browse skills", icon: "layers" },
  mcp__omega__propose_hire: { label: "Propose a hire", icon: "userPlus" },
  mcp__omega__my_proposals: { label: "Check proposals", icon: "userPlus" },
  mcp__omega__withdraw_proposal: { label: "Withdraw proposal", icon: "userPlus" },
  Agent: { label: "Sub-agent", icon: "branch" },
  Skill: { label: "Skill", icon: "layers" },
  Read: { label: "Read", icon: "file" },
  Edit: { label: "Edit", icon: "edit" },
  Write: { label: "Write", icon: "edit" },
  Bash: { label: "Bash", icon: "tool" },
};
const toolMeta = (tool: string) => FRIENDLY[tool] ?? { label: toolName(tool), icon: "tool" as IconName };
/** "mcp__claude_ai_Atlassian_Rovo__searchConfluenceUsingCql" -> "searchConfluenceUsingCql" */
export const toolName = (tool: string) => tool.replace(/^mcp__.+?__/, "");

/**
 * `tasks` are the run's sub-agents. An `Agent` call returns "launched" at once and its agent goes on
 * working, so that row shows the agent's own state instead of a tick.
 */
export function ToolRows({ tools, running, compact = false, tasks = [] }: { tools: ToolItem[]; running: boolean; compact?: boolean; tasks?: TaskView[] }) {
  const [open, setOpen] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const limit = compact ? 3 : 6;
  const hidden = tools.length > limit + 1 && !expanded ? tools.length - limit : 0;
  const shown = hidden ? tools.slice(tools.length - limit) : tools;
  return (
    <div className={`tools ${compact ? "tools-compact" : ""}`}>
      {hidden ? (
        <button className="tool-more" onClick={() => setExpanded(true)}>
          <Icon name="more" size={14} /> {hidden} earlier {hidden === 1 ? "step" : "steps"}
        </button>
      ) : null}
      {shown.map((t) => {
        const m = toolMeta(t.tool);
        const isOpen = open === t.id;
        const task = t.tool === "Agent" ? tasks.find((k) => k.toolUseId === t.id) : undefined;
        // The row's status: a launched agent answers for itself, so "launched" is never shown as done.
        const status: "pending" | "ok" | "err" | "stopped" | null = task
          ? task.state === "running" ? (running ? "pending" : "stopped") : task.state === "done" ? "ok" : task.state === "failed" ? "err" : "stopped"
          : t.result === null && running ? "pending" : t.result ? (t.result.ok ? "ok" : "err") : null;
        return (
          <div key={t.id} className={`tool ${isOpen ? "tool-open" : ""}`}>
            <button className="tool-row" onClick={() => setOpen(isOpen ? null : t.id)} aria-expanded={isOpen}>
              <Icon name={m.icon} size={13} className="tool-icon" />
              <span className="tool-name">{m.label}</span>
              <span className="tool-brief">{t.brief}</span>
              <span className="tool-status">
                {status === "pending" ? <span className="tool-pending" /> : status === "ok" ? <Icon name="check" size={12} className="tool-ok" /> : status === "err" ? <Icon name="close" size={12} className="tool-err" /> : status === "stopped" ? <span className="tool-stopped" title="Ended with the run" /> : null}
              </span>
            </button>
            {isOpen ? (
              <div className="tool-detail mono">
                <div><span className="faint">{t.tool}</span> {t.brief}</div>
                {task ? (
                  <div className={task.state === "failed" ? "tool-err" : ""}>→ {task.state === "running" ? (running ? "running in the background" : "ended with the run") : task.state === "done" ? "finished" : task.state === "failed" ? "failed" : "stopped"}</div>
                ) : t.result ? <div className={t.result.ok ? "" : "tool-err"}>→ {t.result.brief}</div> : <div className="faint">→ waiting…</div>}
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

export function SkillChip({ skill, file, change, onClick }: { skill: string; file: string; change: SkillChangeKind; onClick?: () => void }) {
  const verb = change === "added" ? "Skill created" : change === "removed" ? "Skill trimmed" : "Skill refined";
  return (
    <button className="skill-chip" onClick={onClick} disabled={!onClick} title="See what changed">
      <Icon name="sparkle" size={13} />
      <span className="skill-chip-verb">{verb}</span>
      <span className="skill-chip-name mono">{skill}{file !== "SKILL.md" ? `/${file}` : ""}</span>
      {onClick ? <Icon name="arrowRight" size={12} className="skill-chip-go" /> : null}
    </button>
  );
}

export function DelegatedCard({ childRunId, employeeId, task, employees, child, asked = false, handoff = false }: { childRunId: string; employeeId: string; task: string; employees: EmployeeView[]; child: Run | undefined; asked?: boolean; handoff?: boolean }) {
  const e = employees.find((x) => x.id === employeeId);
  return (
    <button className="delegated" onClick={() => openRun(childRunId)}>
      <span className="delegated-top">
        <Icon name={asked ? "ask" : "branch"} size={13} className="muted" />
        <span>{handoff ? "Handed a task to" : asked ? "Asked" : "Delegated to"}</span>
        {e ? <Avatar name={e.name} hue={e.hue} size={18} /> : null}
        <b>{e ? e.name : asked ? "a colleague" : "a report"}</b>
        <span className="grow" />
        {child ? <StatePill tone={runStateTone(child)} pulse={child.state === "running"}>{runStateLabel(child.state)}</StatePill> : null}
        <Icon name="arrowRight" size={13} className="faint" />
      </span>
      <span className="delegated-task">{task}</span>
    </button>
  );
}

const REQUESTED_HEAD: Record<RequestKind, { verb: string; tail: string }> = {
  question: { verb: "Asked", tail: " a question" },
  task: { verb: "Asked", tail: " to do something" },
  info: { verb: "Told", tail: "" },
};

export function RequestedRow({ personId, reqKind, message, people, request }: { personId: string; reqKind: RequestKind; message: string; people: PersonView[]; request?: HumanRequest }) {
  const p = people.find((x) => x.id === personId);
  const name = p ? firstName(p.name) : "someone";
  const channel = p && request?.channelId ? p.channels.find((c) => c.id === request.channelId) : undefined;
  const state = request?.state;
  const head = REQUESTED_HEAD[reqKind];
  return (
    <div className="requested">
      <Icon name={reqKind === "info" ? "bell" : "ask"} size={14} className="requested-icon" />
      <div className="stack grow" style={{ minWidth: 0 }}>
        <span className="requested-head">
          {head.verb} <b>{name}</b>{head.tail}
          {channel ? <span className="faint"> · via {channel.address}</span> : null}
        </span>
        <span className="requested-msg">{message}</span>
        {request?.response ? <span className="requested-reply"><b>{name}:</b> {request.response}</span> : null}
      </div>
      {reqKind === "info" ? (
        <span className="pill pill-outline">FYI</span>
      ) : state ? (
        <span className={`pill ${state === "open" ? "pill-outline" : state === "cancelled" ? "" : "pill-ok"}`}>
          {state === "open" ? "Waiting" : state === "answered" ? "Answered" : state === "done" ? "Done" : "Cancelled"}
        </span>
      ) : null}
    </div>
  );
}

const PROPOSAL_STATE: Record<HireProposal["state"], string> = { open: "Waiting on you", approved: "Hired", declined: "Declined", withdrawn: "Withdrawn" };

export function ProposedRow({ name, title, proposal }: { name: string; title: string; proposal?: HireProposal }) {
  const state = proposal?.state;
  return (
    <div className="requested">
      <Icon name="userPlus" size={14} className="requested-icon" />
      <div className="stack grow" style={{ minWidth: 0 }}>
        <span className="requested-head">Proposed a hire: <b>{name}</b>, {title}</span>
        {proposal?.note ? <span className="requested-reply"><b>Your note:</b> {proposal.note}</span> : null}
      </div>
      {state === "open" ? (
        <button className="btn btn-sm" onClick={() => navigate({ page: "inbox" })}>Review <Icon name="arrowRight" size={12} /></button>
      ) : state ? (
        <span className={`pill ${state === "approved" ? "pill-ok" : ""}`}>{PROPOSAL_STATE[state]}</span>
      ) : null}
    </div>
  );
}
