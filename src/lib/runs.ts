// A run as the UI shows it live: its transcript events, the text still streaming, its sub-agents and the
// runs it handed work to. Pure folding, so a stored transcript and live events land in the same shape.
import type { Run, RunDetail, RunEvent, TaskOutcome } from "../api/types";

/** A sub-agent the employee started, as of the last event about it. */
export type TaskView = {
  id: string; toolUseId: string; name: string;
  state: "running" | TaskOutcome;
  toolUses: number; durationMs: number;
};

export type LiveRun = { run: Run; events: RunEvent[]; live: string; children: Run[]; tasks: TaskView[]; transcriptGone: boolean };

type TaskEvent = Extract<RunEvent, { type: "taskStarted" | "taskProgress" | "taskEnded" }>;
const isTaskEvent = (ev: RunEvent): ev is TaskEvent => ev.type === "taskStarted" || ev.type === "taskProgress" || ev.type === "taskEnded";

/** Tasks are upserted by id: progress arrives many times a second and each event replaces the last. */
function foldTask(tasks: TaskView[], ev: TaskEvent): TaskView[] {
  const i = tasks.findIndex((t) => t.id === ev.taskId);
  const prev = i < 0 ? null : tasks[i];
  let next: TaskView | null;
  switch (ev.type) {
    case "taskStarted":
      if (prev) return tasks;
      next = { id: ev.taskId, toolUseId: ev.toolUseId, name: ev.name, state: "running", toolUses: 0, durationMs: 0 };
      break;
    case "taskProgress":
      // A progress line that outlived its end (or whose start was missed) never brings a task back to life.
      if (prev && prev.state !== "running") return tasks;
      next = { ...(prev ?? { id: ev.taskId, toolUseId: "", name: ev.activity, state: "running" as const }), toolUses: ev.toolUses, durationMs: ev.durationMs };
      break;
    case "taskEnded":
      if (!prev) return tasks;
      next = { ...prev, state: ev.outcome, toolUses: ev.toolUses || prev.toolUses, durationMs: ev.durationMs || prev.durationMs };
      break;
  }
  return i < 0 ? [...tasks, next] : tasks.map((t, j) => (j === i ? next : t));
}

/** Events folded into a run: task events update its sub-agents, text deltas stream into `live` until the
 *  finished text arrives, and the rest join the transcript. */
export function applyEvents(lr: LiveRun, events: RunEvent[]): LiveRun {
  let { tasks, live } = lr;
  const transcript: RunEvent[] = [];
  for (const ev of events) {
    if (isTaskEvent(ev)) tasks = foldTask(tasks, ev);
    else if (ev.type === "textDelta") live += ev.text;
    else {
      transcript.push(ev);
      if (ev.type === "text") live = "";
    }
  }
  return { ...lr, events: transcript.length ? [...lr.events, ...transcript] : lr.events, live, tasks };
}

/** A newer copy of the run itself. Streaming text ends with the run. */
export const applyRun = (lr: LiveRun, run: Run): LiveRun => ({ ...lr, run, live: run.state === "running" ? lr.live : "" });

/** A run with nothing recorded yet. */
export const emptyLiveRun = (run: Run): LiveRun => ({ run, events: [], live: "", children: [], tasks: [], transcriptGone: false });

/** A run handing work to `child` (a delegated or asked run) gains or updates it. */
export function applyChild(lr: LiveRun, child: Run): LiveRun {
  const exists = lr.children.some((c) => c.id === child.id);
  return { ...lr, children: exists ? lr.children.map((c) => (c.id === child.id ? child : c)) : [...lr.children, child] };
}

export const isChildOf = (r: Run, runId: string) => (r.kind.type === "delegated" || r.kind.type === "asked") && r.kind.parentRunId === runId;

/** A run's stored events, as `run_detail` returns them. */
export const liveRunOf = (d: RunDetail): LiveRun => applyEvents({ ...emptyLiveRun(d.run), children: d.children, transcriptGone: d.transcriptGone }, d.events);
