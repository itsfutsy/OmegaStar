import { useMemo, useState } from "react";
import { call, errorMessage } from "../../api/client";
import type { EmployeeView, OwnSkill, Run, SkillHealth, SkillRevision, StagedImport } from "../../api/types";
import { firstName, recordedName, relTime, runKindLabel, skillsChanged } from "../../lib/format";
import { refreshOrg, useCatalog } from "../../lib/hooks";
import { toast } from "../../lib/toasts";
import { openChat, openMember, openRun } from "../../lib/router";
import { lineStats } from "../Diff";
import { Icon } from "../Icon";
import { Upkeep } from "../SkillHealth";
import { ConfirmButton, Section } from "../ui";
import { ImportSheet, isOrganize, OriginalsSheet, type OrganizeRun } from "./SkillImport";

/** Leaves nested by name: `sam-weekly-patching` sits under `sam-weekly` when both exist. */
type TreeNode = { s: OwnSkill; children: TreeNode[] };
function nest(leaves: OwnSkill[]): TreeNode[] {
  const byName = new Map(leaves.map((s) => [s.name, { s, children: [] as TreeNode[] }]));
  const roots: TreeNode[] = [];
  for (const node of byName.values()) {
    const parent = [...byName.keys()].filter((n) => n !== node.s.name && node.s.name.startsWith(`${n}-`)).sort((a, b) => b.length - a.length)[0];
    (parent ? byName.get(parent)!.children : roots).push(node);
  }
  return roots;
}

function Branch({ nodes, refinements, onOpenFile }: { nodes: TreeNode[]; refinements: Map<string, number>; onOpenFile: (skill: string, file: string) => void }) {
  return (
    <>
      {nodes.map((n) => (
        <div key={n.s.name} className="node-wrap">
          <SkillNode s={n.s} kind={n.children.length ? "router" : "leaf"} count={refinements.get(n.s.name) ?? 0} onOpenFile={onOpenFile} />
          {n.children.length ? <div className="tree-children"><Branch nodes={n.children} refinements={refinements} onOpenFile={onOpenFile} /></div> : null}
        </div>
      ))}
    </>
  );
}

export function SkillsOverview({ e, skills, history, runs, staged, health, runById, onOpenFile, onOpenRev }: {
  e: EmployeeView; skills: OwnSkill[]; history: SkillRevision[]; runs: Run[]; staged: StagedImport[]; health: SkillHealth | null; runById: Map<string, Run>;
  onOpenFile: (skill: string, file: string) => void; onOpenRev: (id: string) => void;
}) {
  const catalog = useCatalog();
  const [showAll, setShowAll] = useState(false);
  const [importing, setImporting] = useState<string[] | null>(null);
  // Each opening remounts the import sheet, so it starts from what was preselected.
  const [importKey, setImportKey] = useState(0);
  const openImport = (keys: string[]) => { setImporting(keys); setImportKey((k) => k + 1); };
  const [originalsOf, setOriginalsOf] = useState<OrganizeRun | null>(null);
  const role = skills.find((s) => s.role);
  const leaves = skills.filter((s) => !s.role && !s.managed);
  const tree = useMemo(() => nest(leaves), [leaves]);
  const organizeRuns = runs.filter(isOrganize);
  const organizing = organizeRuns.find((r) => r.state === "queued" || r.state === "running");
  const unsettled = organizeRuns.find((r) => r.state === "succeeded" && !r.kind.settled);
  const managed = skills.filter((s) => s.managed);
  const refinements = useMemo(() => {
    const m = new Map<string, number>();
    for (const r of history) m.set(r.skill, (m.get(r.skill) ?? 0) + 1);
    return m;
  }, [history]);
  const granted = e.skills.keys.map((k) => catalog.data?.find((c) => c.id === k) ?? { id: k, name: k.split(":").pop() ?? k, description: "", source: "user" as const });
  const importable = granted.filter((g) => g.source !== "plugin");
  const groups = useMemo(() => groupHistory(history), [history]);
  const shown = showAll ? groups : groups.slice(0, 8);
  const name = firstName(e.name);
  const organizeNow = async () => {
    try {
      await call("organize", { employeeId: e.id });
      toast(`${name} is organizing them now`, "ok");
    } catch (x) {
      toast(errorMessage(x), "err");
    }
  };

  // At most one notice, and only for imports: something in flight, or waiting on you.
  const notice = organizing ? (
    <div className="skills-notice skills-notice-live">
      <span className="grow">
        <b>{name} is organizing {organizing.kind.imported.length} imported {organizing.kind.imported.length === 1 ? "skill" : "skills"}</b>
        <span>They load once {name} has sorted and merged them.</span>
      </span>
      <button className="btn btn-sm" onClick={() => openRun(organizing.id)}>{organizing.state === "queued" ? "Queued" : "Watch"} <Icon name="arrowUpRight" size={11} /></button>
    </div>
  ) : staged.length > 0 ? (
    <div className="skills-notice">
      <span className="grow">
        <b>{staged.length} imported {staged.length === 1 ? "skill is" : "skills are"} waiting to be organized</b>
        <span className="mono">{staged.map((x) => x.name).join("  ")}</span>
      </span>
      <button className="btn btn-sm btn-primary" onClick={() => void organizeNow()}>Organize now</button>
    </div>
  ) : unsettled ? (
    <div className="skills-notice">
      <span className="grow">
        <b>{name} organized the {unsettled.kind.imported.length} skills you imported</b>
        <span>Your originals are still in Claude Code, unchanged. Keep them or remove them.</span>
      </span>
      <button className="btn btn-sm" onClick={() => setOriginalsOf(unsettled)}>Review originals</button>
    </div>
  ) : null;

  return (
    <div className="tab-scroll">
      {notice}

      <Section title={`What ${name} knows`} action={<button className="section-note linkish" onClick={() => openImport([])}><Icon name="plus" size={11} /> Import skills</button>}>
        <div className="tree">
          {role ? <SkillNode s={role} kind="role" count={refinements.get(role.name) ?? 0} onOpenFile={onOpenFile} /> : null}
          {leaves.length > 0 ? (
            <div className="tree-children">
              <Branch nodes={tree} refinements={refinements} onOpenFile={onOpenFile} />
            </div>
          ) : (
            <div className="tree-children">
              <div className="tree-empty">
                Nothing yet. Teach {name} a task in Chat and it becomes a skill here.
                <button className="btn btn-sm" onClick={() => openChat(e.id)}><Icon name="teach" size={12} /> Teach a task</button>
              </div>
            </div>
          )}
          {managed.map((s) => (
            <button key={s.name} className="tree-managed" onClick={() => onOpenFile(s.name, "SKILL.md")} title="Omega Star rewrites this before every run">
              <Icon name="lock" size={12} />
              <span>How {name} writes skills</span>
              <span className="mono">{s.name}</span>
            </button>
          ))}
        </div>
      </Section>

      <Section title={`Tools ${name} can use`} action={<button className="section-note linkish" onClick={() => openMember(e.id, "profile", null, true)}>Change</button>}>
        {granted.length === 0 && !e.skills.builtins ? <p className="tab-empty">None. {name} works only from their own skills.</p> : (
          <div className="tools-grid">
            {granted.map((g) => (
              <span key={g.id} className="tool-chip" title={g.description}>
                <span className="mono">{g.name}</span>
                <span className="tool-chip-src">{g.source === "user" ? "yours" : g.source === "synced" ? "claude.ai" : "plugin"}</span>
              </span>
            ))}
            {e.skills.builtins ? <span className="tool-chip tool-chip-quiet">Claude Code built-ins</span> : null}
          </div>
        )}
        {organizeRuns.length === 0 && importable.length > 0 ? (
          <p className="tools-import">
            {importable.length === 1 ? "One of these is a skill of yours." : `${importable.length} of these are skills of yours.`}{" "}
            If {importable.length === 1 ? "it is" : "they are"} really {name}'s job, import {importable.length === 1 ? "it" : "them"} and {name} keeps their own {importable.length === 1 ? "copy" : "copies"}, improving as they learn.{" "}
            <button className="linkish" onClick={() => openImport(importable.map((g) => g.id))}>Import</button>
          </p>
        ) : null}
      </Section>

      <Section title="Changes" action={history.length ? <span className="section-note">Open one to see it or undo it</span> : null}>
        {history.length === 0 ? <p className="tab-empty">None yet. Every change {name} or you make to a skill is kept here.</p> : null}
        <ol className="timeline">
          {shown.map((g) => g.length === 1
            ? <TimelineItem key={g[0].id} r={g[0]} run={g[0].runId ? runById.get(g[0].runId) : undefined} onOpen={() => onOpenRev(g[0].id)} />
            : <TimelineGroup key={g[0].id} revs={g} run={g[0].runId ? runById.get(g[0].runId) : undefined} name={name} />)}
        </ol>
        {groups.length > 8 && !showAll ? <button className="btn btn-ghost btn-sm" style={{ alignSelf: "flex-start" }} onClick={() => setShowAll(true)}>Show all {groups.length}</button> : null}
      </Section>

      {health ? <Upkeep e={e} health={health} onOpenFile={onOpenFile} /> : null}

      <ImportSheet key={importKey} e={e} preselect={importing} onClose={() => setImporting(null)} />
      <OriginalsSheet e={e} run={originalsOf} onClose={() => setOriginalsOf(null)} />
    </div>
  );
}

/** Consecutive revisions from one run collapse into one entry once a run touched 3 or more files. */
function groupHistory(history: SkillRevision[]): SkillRevision[][] {
  const out: SkillRevision[][] = [];
  for (const r of history) {
    const last = out[out.length - 1];
    if (last && r.runId && last[0].runId === r.runId) last.push(r);
    else out.push([r]);
  }
  return out.flatMap((g) => (g.length >= 3 ? [g] : g.map((r) => [r])));
}

function TimelineGroup({ revs, run, name }: { revs: SkillRevision[]; run: Run | undefined; name: string }) {
  // Notes, the company page and what they own are not skills: they are named, not counted.
  const skillCount = new Set(revs.map((r) => r.skill).filter((n) => recordedName(n) === n)).size;
  const added = revs.filter((r) => r.change === "added").length;
  const title = run?.kind.type === "organize" ? runKindLabel(run.kind) : skillsChanged(revs).label;
  const who = run ? (run.kind.type === "chat" ? "Learned in chat" : runKindLabel(run.kind)) : "During a run";
  const revert = async () => {
    if (!revs[0].runId) return;
    try {
      const n = await call("revert_run", { runId: revs[0].runId });
      refreshOrg();
      toast(`Undone. ${name}'s skills are back to how they were before that run${run?.kind.type === "organize" ? ", and the imports are waiting again" : ""} (${n} ${n === 1 ? "file" : "files"}).`, "ok");
    } catch (x) {
      toast(errorMessage(x), "err");
    }
  };
  return (
    <li className="tl tl-run tl-group">
      <span className="tl-dot" />
      <div className="tl-body tl-group-body">
        <button className="tl-group-main" onClick={() => revs[0].runId && openRun(revs[0].runId)}>
          <span className="tl-top">
            <span>{title}</span>
            <span className="tl-lines mono num">{revs.length} files{skillCount ? ` · ${skillCount} ${skillCount === 1 ? "skill" : "skills"}` : ""}</span>
          </span>
          <span className="tl-sub">{who === title ? "" : `${who} · `}{added} added · {relTime(revs[0].at)}</span>
        </button>
        <ConfirmButton label="Undo all" icon="undo" className="btn btn-ghost" size="btn-sm" question="Undo every change from this run?" confirmLabel="Undo all" onConfirm={revert} />
      </div>
    </li>
  );
}

function SkillNode({ s, kind, count, onOpenFile }: { s: OwnSkill; kind: "role" | "router" | "leaf"; count: number; onOpenFile: (skill: string, file: string) => void }) {
  const extra = s.files.filter((f) => f !== "SKILL.md");
  return (
    <div className={`node node-${kind}`}>
      <button className="node-main" onClick={() => onOpenFile(s.name, "SKILL.md")}>
        <span className="node-glyph">{kind === "role" ? <Icon name="layers" size={13} /> : <span className={kind === "router" ? "node-dot node-dot-router" : "node-dot"} />}</span>
        <span className="stack grow" style={{ minWidth: 0 }}>
          <span className="node-top">
            <span className="node-name mono">{s.name}</span>
            {kind === "role" ? <span className="pill pill-accent node-role-pill">Role</span> : null}
            <span className="grow" />
            <span className="node-meta num" title={count > 1 ? `Changed ${count - 1} ${count - 1 === 1 ? "time" : "times"}` : "Not changed since it was written"}>{relTime(s.updatedAt)}</span>
          </span>
          <span className="node-desc">{s.description}</span>
        </span>
      </button>
      {extra.length ? (
        <div className="node-files">
          {extra.map((f) => (
            <button key={f} className="node-file mono" onClick={() => onOpenFile(s.name, f)}><Icon name="file" size={11} />{f}</button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

export function TimelineItem({ r, run, onOpen }: { r: SkillRevision; run: Run | undefined; onOpen: () => void }) {
  const stats = useMemo(() => lineStats(r.before, r.after), [r.before, r.after]);
  const verb = r.change === "added" ? "Created" : r.change === "removed" ? "Removed" : "Changed";
  const who = r.runId ? (run ? (run.kind.type === "chat" ? "Learned in chat" : runKindLabel(run.kind)) : "During a run") : r.change === "added" ? "Set up" : "Edited by you";
  return (
    <li className={`tl ${r.runId ? "tl-run" : "tl-ceo"}`}>
      <span className="tl-dot" />
      <button className="tl-body" onClick={onOpen}>
        <span className="tl-top">
          <span>{verb} {recordedName(r.skill) === r.skill
            ? <span className="mono tl-skill">{r.skill}{r.file !== "SKILL.md" ? `/${r.file}` : ""}</span>
            : <>{recordedName(r.skill)} <span className="mono tl-skill">{r.file}</span></>}</span>
          <span className="tl-lines mono num">
            {stats.added ? <span className="tl-add">+{stats.added}</span> : null}
            {stats.removed ? <span className="tl-del">−{stats.removed}</span> : null}
          </span>
        </span>
        <span className="tl-sub">{who} · {relTime(r.at)}</span>
      </button>
    </li>
  );
}
