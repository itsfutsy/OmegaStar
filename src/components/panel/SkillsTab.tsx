import { useEffect } from "react";
import { call } from "../../api/client";
import type { EmployeeView } from "../../api/types";
import { useQuery } from "../../lib/hooks";
import { openMember } from "../../lib/router";
import { ErrorNote, Skeleton } from "../ui";
import { FileView, pseudoFolder, RevisionView } from "./SkillFile";
import { SkillsOverview } from "./SkillsOverview";
import "./SkillsTab.css";

type View =
  | { kind: "overview" }
  | { kind: "file"; skill: string; file: string }
  | { kind: "rev"; id: string }
  | { kind: "resolve"; runId: string; skill: string; file: string };

function parseFocus(focus: string | null): View {
  if (!focus) return { kind: "overview" };
  const [k, ...rest] = focus.split("~");
  if (k === "file" && rest.length >= 2) return { kind: "file", skill: rest[0], file: rest.slice(1).join("~") };
  if (k === "rev" && rest[0]) return { kind: "rev", id: rest[0] };
  if (k === "diff" && rest.length >= 3) return { kind: "resolve", runId: rest[0], skill: rest[1], file: rest.slice(2).join("~") };
  return { kind: "overview" };
}

export function SkillsTab({ e, focus }: { e: EmployeeView; focus: string | null }) {
  const skills = useQuery(() => call("employee_skills", { employeeId: e.id }), [e.id], ["org:changed", "run:updated"]);
  const history = useQuery(() => call("skill_history", { employeeId: e.id }), [e.id], ["org:changed", "run:updated"]);
  const runs = useQuery(() => call("runs", { employeeId: e.id, limit: 300 }), [e.id], ["run:updated"]);
  const staged = useQuery(() => call("staged_imports", { employeeId: e.id }), [e.id], ["org:changed", "run:updated"]);
  const health = useQuery(() => call("skill_health", { employeeId: e.id }), [e.id], ["org:changed", "run:updated"]);
  const view = parseFocus(focus);
  const go = (f: string | null) => openMember(e.id, "skills", f, true);

  // A link to a run's change to a file lands on that revision once the history is in, else on the file.
  useEffect(() => {
    const v = parseFocus(focus);
    if (v.kind !== "resolve" || !history.data) return;
    const match = history.data.find((r) => r.runId === v.runId && r.skill === v.skill && r.file === v.file)
      ?? history.data.find((r) => r.skill === v.skill && r.file === v.file);
    openMember(e.id, "skills", match ? `rev~${match.id}` : `file~${v.skill}~${v.file}`, true);
  }, [focus, history.data, e.id]);

  const reload = () => { skills.reload(); history.reload(); };
  const err = skills.error ?? history.error;
  if (err) return <div className="tab-scroll"><ErrorNote text={err} onRetry={reload} /></div>;
  if (!skills.data || !history.data) {
    return <div className="tab-scroll stack" style={{ gap: 12 }}><Skeleton h={72} r={12} /><Skeleton h={40} /><Skeleton h={40} /><Skeleton h={40} /></div>;
  }

  const runById = new Map((runs.data ?? []).map((r) => [r.id, r]));
  if (view.kind === "file") {
    const s = pseudoFolder(view.skill) ?? skills.data.find((x) => x.name === view.skill);
    return <FileView e={e} skill={s} file={view.file} history={history.data} onBack={() => go(null)} onOpenRev={(id) => go(`rev~${id}`)} onChanged={reload} />;
  }
  if (view.kind === "rev") {
    const rev = history.data.find((r) => r.id === view.id);
    return <RevisionView e={e} rev={rev} run={rev?.runId ? runById.get(rev.runId) : undefined} onBack={() => go(null)} onOpenFile={(s, f) => go(`file~${s}~${f}`)} onChanged={() => { reload(); go(null); }} />;
  }
  if (view.kind === "resolve") return <div className="tab-scroll"><Skeleton h={200} r={12} /></div>;
  return <SkillsOverview e={e} skills={skills.data} history={history.data} runs={runs.data ?? []} staged={staged.data ?? []} health={health.data ?? null} runById={runById} onOpenFile={(s, f) => go(`file~${s}~${f}`)} onOpenRev={(id) => go(`rev~${id}`)} />;
}
