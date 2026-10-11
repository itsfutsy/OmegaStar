import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { call, errorMessage } from "../api/client";
import type { HumanRequest, OwnSkill, Run, Schedule } from "../api/types";
import { firstName, relTime, runKindLabel, whenShort } from "../lib/format";
import { useOrg } from "../lib/hooks";
import { toast } from "../lib/toasts";
import { setTheme, useResolvedDark } from "../lib/theme";
import { openHelp } from "../lib/help";
import { navigate, openChat, openMember, openRun, openYou } from "../lib/router";
import { Icon } from "./Icon";
import { kindIcon, kindNames } from "./RunRow";
import { Avatar } from "./ui";
import "./CommandPalette.css";

type Item = {
  id: string; group: string; label: string; hint?: string; icon: ReactNode; run: () => void; keywords?: string;
  /** "query": only searched, never listed (the index is long). "recent": only listed while the query is empty. */
  when?: "query" | "recent";
};
type Index = { schedules: Schedule[]; skills: { employeeId: string; skills: OwnSkill[] }[]; runs: Run[]; requests: HumanRequest[] };

function score(q: string, text: string): number {
  if (!q) return 1;
  const t = text.toLowerCase();
  const i = t.indexOf(q);
  if (i === 0) return 100;
  if (i > 0) return 80 - Math.min(i, 40) + (t[i - 1] === " " ? 10 : 0);
  let ti = 0;
  for (const ch of q) {
    ti = t.indexOf(ch, ti);
    if (ti < 0) return 0;
    ti++;
  }
  return 20;
}

const GROUP_ORDER = ["Team", "Schedules", "Skills", "Waiting on people", "Actions", "Go to", "Run now", "Runs"];
const GROUP_CAP = 6;

const quiet = <T,>(p: Promise<T[]>): Promise<T[]> => p.catch(() => []);

/** Everything the palette searches beyond the org snapshot, fetched when it opens. */
function useIndex(employeeIds: string[]): Index | null {
  const [index, setIndex] = useState<Index | null>(null);
  const key = employeeIds.join(",");
  useEffect(() => {
    let alive = true;
    const ids = key ? key.split(",") : [];
    void Promise.all([
      Promise.all(ids.map((id) => quiet(call("schedules", { employeeId: id })))).then((x) => x.flat()),
      Promise.all(ids.map((id) => quiet(call("employee_skills", { employeeId: id })).then((skills) => ({ employeeId: id, skills })))),
      quiet(call("runs", { employeeId: null, limit: 80 })),
      quiet(call("requests", { personId: null, employeeId: null, state: "open" })),
    ]).then(([schedules, skills, runs, requests]) => { if (alive) setIndex({ schedules, skills, runs, requests }); });
    return () => { alive = false; };
  }, [key]);
  return index;
}

export function CommandPalette({ onClose, api }: { onClose: () => void; api: { hire: () => void; addPerson: () => void } }) {
  const { snap } = useOrg();
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const index = useIndex(useMemo(() => (snap?.employees ?? []).map((e) => e.id), [snap]));
  const dark = useResolvedDark();

  const items = useMemo<Item[]>(() => {
    const employees = snap?.employees ?? [];
    const people = snap?.people ?? [];
    const nameOf = (id: string) => firstName(employees.find((e) => e.id === id)?.name ?? people.find((p) => p.id === id)?.name ?? "Someone");
    const workRuns = (index?.runs ?? []).filter((r) => r.kind.type !== "chat");
    const runLabel = (r: Run) => runKindLabel(r.kind, kindNames(r, employees, people));
    const runWhen = (r: Run) => relTime(r.endedAt ?? r.startedAt ?? r.createdAt);

    const out: Item[] = workRuns.slice(0, 4).map((r) => ({
      id: `recent-${r.id}`, group: "Recent", label: `${nameOf(r.employeeId)} · ${runLabel(r)}`, hint: runWhen(r),
      icon: <Icon name={kindIcon(r)} size={16} />, run: () => openRun(r.id), when: "recent" as const,
    }));
    out.push(
      { id: "hire", group: "Actions", label: "Hire an employee", icon: <Icon name="userPlus" size={16} />, run: () => api.hire(), keywords: "new agent add" },
      { id: "person", group: "Actions", label: "Add a person", icon: <Icon name="person" size={16} />, run: () => api.addPerson(), keywords: "human teammate add" },
      { id: "you", group: "Actions", label: "Edit your profile", hint: snap ? `${snap.company.title} · ${snap.company.ceoName}` : undefined, icon: <Icon name="user" size={16} />, run: openYou, keywords: "me my name title company reports to boss manager ceo" },
      { id: "theme", group: "Actions", label: dark ? "Switch to light theme" : "Switch to dark theme", icon: <Icon name={dark ? "sun" : "moon"} size={16} />, run: () => setTheme(dark ? "light" : "dark"), keywords: "appearance mode colour color" },
      { id: "help", group: "Actions", label: "Open the Claude Code docs", icon: <Icon name="help" size={16} />, run: () => void openHelp(), keywords: "help documentation manual" },
      { id: "go-org", group: "Go to", label: "Org chart", icon: <Icon name="org" size={16} />, run: () => navigate({ page: "org", memberId: null, tab: null, focus: null }) },
      { id: "go-inbox", group: "Go to", label: "Inbox", icon: <Icon name="inbox" size={16} />, run: () => navigate({ page: "inbox" }), keywords: "proposals requests waiting archive" },
      { id: "go-activity", group: "Go to", label: "Activity", icon: <Icon name="activity" size={16} />, run: () => navigate({ page: "activity", view: "runs" }), keywords: "runs history" },
      { id: "go-skills", group: "Go to", label: "Skill changes", hint: "Activity", icon: <Icon name="layers" size={16} />, run: () => navigate({ page: "activity", view: "skills" }), keywords: "skills refinements growth learned" },
      { id: "go-settings", group: "Go to", label: "Settings", icon: <Icon name="settings" size={16} />, run: () => navigate({ page: "settings" }), keywords: "preferences theme" },
    );
    for (const e of employees) {
      out.push(
        { id: `open-${e.id}`, group: "Team", label: e.name, hint: e.title, icon: <Avatar name={e.name} hue={e.hue} size={18} />, run: () => openChat(e.id), keywords: e.title },
        {
          id: `run-now-${e.id}`, group: "Run now", label: `Run ${e.name} now`, hint: e.presence === "working" ? "Working, will queue" : undefined, icon: <Icon name="play" size={16} />,
          run: () => call("run_now", { employeeId: e.id, instruction: null }).then(
            (r) => toast(`${e.name} is on it`, "ok", { label: "View", run: () => openRun(r.id) }),
            (err) => toast(errorMessage(err), "err"),
          ),
        },
      );
    }
    for (const p of people) {
      out.push({ id: `open-${p.id}`, group: "Team", label: p.name, hint: p.title, icon: <Avatar name={p.name} hue={p.hue} size={18} person />, run: () => openMember(p.id), keywords: p.title });
    }
    for (const s of index?.schedules ?? []) {
      out.push({
        id: `sched-${s.id}`, group: "Schedules", label: s.label, icon: <Icon name="calendar" size={16} />,
        hint: `${nameOf(s.employeeId)}${s.nextFire ? ` · ${whenShort(s.nextFire)}` : s.enabled ? "" : " · off"}`,
        run: () => openMember(s.employeeId, "work"), keywords: `${s.instruction} schedule routine`, when: "query",
      });
    }
    for (const { employeeId, skills } of index?.skills ?? []) {
      for (const sk of skills.filter((x) => !x.managed)) {
        out.push({
          id: `skill-${employeeId}-${sk.name}`, group: "Skills", label: sk.name, hint: nameOf(employeeId), icon: <Icon name="layers" size={16} />,
          run: () => openMember(employeeId, "skills", `file~${sk.name}~SKILL.md`), keywords: `${sk.description} skill`, when: "query",
        });
      }
    }
    for (const r of workRuns) {
      out.push({
        id: `run-${r.id}`, group: "Runs", label: runLabel(r), hint: `${nameOf(r.employeeId)} · ${runWhen(r)}`, icon: <Icon name={kindIcon(r)} size={16} />,
        run: () => openRun(r.id), keywords: `${r.prompt} ${r.summary ?? ""}`.slice(0, 400), when: "query",
      });
    }
    for (const r of index?.requests ?? []) {
      out.push({
        id: `req-${r.id}`, group: "Waiting on people", label: r.message.length > 70 ? `${r.message.slice(0, 70)}…` : r.message,
        hint: `${nameOf(r.employeeId)} asks ${nameOf(r.personId)}`, icon: <Icon name="ask" size={16} />, run: () => navigate({ page: "inbox" }),
        keywords: `${r.message} request question`, when: "query",
      });
    }
    return out;
  }, [snap, api, index, dark]);

  const results = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return items.filter((it) => it.group !== "Run now" && it.when !== "query");
    // Fuzzy letter matching over long prompts is noise: secondary text must contain the query outright.
    const scored = items
      .filter((it) => it.when !== "recent")
      .map((it) => {
        const extra = `${it.hint ?? ""} ${it.keywords ?? ""}`.toLowerCase();
        return { it, s: Math.max(score(needle, it.label), extra.includes(needle) ? 40 : 0) };
      })
      .filter((x) => x.s > 0);
    // Best group first (ties by GROUP_ORDER), each group kept together so section headers stay meaningful.
    const best = new Map<string, number>();
    for (const x of scored) best.set(x.it.group, Math.max(best.get(x.it.group) ?? 0, x.s));
    const rank = (g: string) => GROUP_ORDER.indexOf(g);
    scored.sort((a, b) => (best.get(b.it.group) ?? 0) - (best.get(a.it.group) ?? 0) || rank(a.it.group) - rank(b.it.group) || b.s - a.s);
    const perGroup = new Map<string, number>();
    return scored
      .filter((x) => { const n = (perGroup.get(x.it.group) ?? 0) + 1; perGroup.set(x.it.group, n); return n <= (x.it.group === "Runs" ? 4 : GROUP_CAP); })
      .map((x) => x.it);
  }, [items, q]);

  useEffect(() => setSel(0), [q]);
  useEffect(() => {
    listRef.current?.querySelector(`[data-idx="${sel}"]`)?.scrollIntoView({ block: "nearest" });
  }, [sel]);

  const choose = (it: Item | undefined) => {
    if (!it) return;
    onClose();
    it.run();
  };

  let lastGroup = "";
  return (
    <>
      <div className="palette-scrim" onClick={onClose} />
      <div className="palette" role="dialog" aria-label="Command menu">
        <div className="palette-input">
          <Icon name="search" size={16} />
          <input
            autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search people, skills, schedules, runs…"
            role="combobox" aria-expanded aria-controls="palette-list" aria-activedescendant={`pi-${sel}`}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") { e.preventDefault(); setSel((s) => Math.min(results.length - 1, s + 1)); }
              else if (e.key === "ArrowUp") { e.preventDefault(); setSel((s) => Math.max(0, s - 1)); }
              else if (e.key === "Enter") { e.preventDefault(); choose(results[sel]); }
              else if (e.key === "Escape") { e.preventDefault(); onClose(); }
            }}
          />
        </div>
        <div className="palette-list" id="palette-list" role="listbox" ref={listRef}>
          {results.length === 0 ? <div className="palette-empty">Nothing matches “{q}”.</div> : null}
          {results.map((it, i) => {
            const header = it.group !== lastGroup ? <div className="palette-group">{it.group}</div> : null;
            lastGroup = it.group;
            return (
              <div key={it.id}>
                {header}
                <button
                  id={`pi-${i}`} data-idx={i} role="option" aria-selected={i === sel} className="palette-item"
                  onMouseMove={() => setSel(i)} onClick={() => choose(it)}
                >
                  <span className="pi-icon">{it.icon}</span>
                  <span className={`pi-label ${it.group === "Skills" ? "mono" : ""}`}>{it.label}</span>
                  {it.hint ? <span className="pi-hint">{it.hint}</span> : null}
                </button>
              </div>
            );
          })}
        </div>
        <div className="palette-foot">
          <span><span className="kbd">↑</span><span className="kbd">↓</span> move</span>
          <span><span className="kbd">↵</span> open</span>
          <span><span className="kbd">esc</span> close</span>
        </div>
      </div>
    </>
  );
}
