// The work behind a reply, read from its run: what the talk says (its first words, its answer), and the steps behind it
// as rows, which the reply's one work line (WorkLine) stands for: what it is doing now while it runs, a summary once it
// is done, and the rows when you open it. Pure.
import type { EmployeeView, Run } from "../../api/types";
import { firstName } from "../../lib/format";
import type { LiveRun, TaskView } from "../../lib/runs";
import { isLive } from "../../lib/threads";
import type { IconName } from "../Icon";
import { saidElsewhere, skillWords, stepWords, toItems, type ToolItem, type TranscriptItem } from "../Transcript";

type StepState = "pending" | "ok" | "err" | "stopped" | "wait" | null;
type TextItem = Extract<TranscriptItem, { kind: "text" }>;

/** A step: the verb, what it acted on, and how it ended. `detail` opens under it. A colleague asked carries their run
 *  (`runId`, `child`); a skill written carries the change it opens (`skill`). */
export type WorkRow = {
  key: string; icon: IconName; verb: string; object: string; code: boolean; state: StepState;
  tag?: string; meta?: string; detail?: string; avatar?: { name: string; hue: number; image?: string | null }; runId?: string; child?: Run;
  skill?: { skill: string; file: string };
};

/** One stretch of a reply: what you said to start it (after the first), its items, and how the talk reads them. */
export type Part = { said: string | null; items: TranscriptItem[]; opener: TextItem | null; notes: TextItem[]; answer: TextItem[] };

// Not a skill change: Omega Star records those after the run ends, so one would always fall after the answer and fold it
// into the work line. Its row still shows (rowsOf).
const isWork = (it: TranscriptItem) => it.kind === "tools" || it.kind === "delegated" || it.kind === "requested" || it.kind === "proposed" || it.kind === "visual";

/** A reply split where a message you sent while it worked landed (one still waiting to go in splits nothing). In each part the talk keeps the first words (when they
 *  came before any work) and the answer (the words after the last of it); the notes between are rows of its work. */
export function partsOf(events: TranscriptItem[]): Part[] {
  const raw: { said: string | null; items: TranscriptItem[] }[] = [{ said: null, items: [] }];
  for (const it of events) {
    if (it.kind === "user") { if (!it.waiting) raw.push({ said: it.text, items: [] }); }
    else if (it.kind !== "init" && it.kind !== "stderr") raw[raw.length - 1].items.push(it);
  }
  return raw.map(({ said, items }) => {
    const texts = items.filter((it): it is TextItem => it.kind === "text");
    const work = items.flatMap((it, i) => (isWork(it) ? [i] : []));
    if (!work.length) return { said, items, opener: null, notes: [], answer: texts };
    const first = items[0];
    const opener = first?.kind === "text" && work[0] > 0 ? first : null;
    const last = work[work.length - 1];
    return {
      said, items, opener,
      notes: items.slice(opener ? 1 : 0, last).filter((it): it is TextItem => it.kind === "text"),
      answer: items.slice(last + 1).filter((it): it is TextItem => it.kind === "text"),
    };
  });
}

/** A row's key, stable as the reply streams. */
const keys = {
  tool: (runId: string, id: string) => `t:${runId}:${id}`,
  agent: (runId: string, id: string) => `a:${runId}:${id}`,
  delegate: (childRunId: string) => `d:${childRunId}`,
  note: (runId: string, key: string) => `n:${runId}:${key}`,
  visual: (runId: string, id: string) => `v:${runId}:${id}`,
  skill: (runId: string, skill: string, file: string) => `s:${runId}:${skill}:${file}`,
};

/** Elapsed time in minutes, never ticking seconds: "<1 min", "3 min", "1 h 5 min". */
export function minutes(ms: number): string {
  const m = Math.floor(Math.max(0, ms) / 60_000);
  if (m < 1) return "<1 min";
  if (m < 60) return `${m} min`;
  return `${Math.floor(m / 60)} h${m % 60 ? ` ${m % 60} min` : ""}`;
}

/** How long something took: "under a minute", "3 min", "1 h 5 min". */
const took = (ms: number): string => (ms < 60_000 ? "under a minute" : minutes(ms));

const firstLine = (text: string) => text.trim().split(/\r?\n/).find((l) => l.trim())?.replace(/[*_`#>]/g, "").trim() ?? "";

const delegateVerb = (asked: boolean, handoff: boolean, name: string) => `${handoff ? "Handed to" : asked ? "Asked" : "Delegated to"} ${name}`;
const childState = (c?: Run): StepState => (!c ? null : c.state === "queued" ? "wait" : c.state === "running" ? "pending" : c.state === "succeeded" ? "ok" : c.state === "failed" || c.state === "lost" ? "err" : "stopped");
const taskState = (t: TaskView, runActive: boolean): StepState => (t.state === "running" ? (runActive ? "pending" : "stopped") : t.state === "done" ? "ok" : t.state === "failed" ? "err" : "stopped");
const toolState = (t: ToolItem, running: boolean): StepState => (t.result === null ? (running ? "pending" : "stopped") : t.result.ok ? "ok" : "err");
const agentTask = (turn: LiveRun, t: ToolItem) => turn.tasks.find((k) => k.toolUseId === t.id);

/** A write into a skill's own folder, which its skill change already says ("Wrote a skill price-changes"). */
const skillWrite = (t: ToolItem, skills: Set<string>) => /^(Write|Edit|MultiEdit)$/.test(t.tool) && [...skills].some((k) => t.brief.includes(`skills/${k}/`));
const skillsOf = (items: TranscriptItem[]) => new Set(items.flatMap((it) => (it.kind === "skill" ? [it.skill] : [])));

/** The rows of one stretch of a turn's reply (`part`, or the whole reply), oldest first: its steps, the notes between
 *  them, the colleagues it asked, what it showed and the skills it wrote. Work still going is a row too, marked as
 *  working. Sub-agents whose call was not recorded (an old transcript) are rows of the whole reply. */
export function rowsOf(turn: LiveRun, employees: EmployeeView[], part?: Part): WorkRow[] {
  const id = turn.run.id;
  const running = isLive(turn.run);
  const all = toItems(turn.events);
  const notes = new Set(partsOf(all).flatMap((p) => p.notes.map((n) => n.key)));
  const skills = skillsOf(all);
  const seenTasks = new Set<string>();
  const out: WorkRow[] = [];
  const agentRow = (t: TaskView) => {
    seenTasks.add(t.id);
    out.push({ key: keys.agent(id, t.id), icon: "layers", verb: "Sub-agent", object: t.name, code: false, state: taskState(t, turn.run.state === "running"), meta: `${t.toolUses} ${t.toolUses === 1 ? "call" : "calls"} · ${minutes(t.durationMs)}`, runId: id });
  };
  for (const it of part?.items ?? all) {
    switch (it.kind) {
      case "tools":
        for (const t of it.tools) {
          if (t.tool === "Agent") {
            const task = agentTask(turn, t);
            if (task) agentRow(task);
            continue;
          }
          if ((saidElsewhere(t.tool) || skillWrite(t, skills)) && t.result?.ok !== false) continue;
          const state = toolState(t, running);
          const w = stepWords(t.tool, t.brief, state === "pending");
          out.push({ key: keys.tool(id, t.id), icon: w.icon, verb: w.verb, object: w.object, code: w.code, state, detail: t.result?.brief, runId: id });
        }
        break;
      case "text":
        if (notes.has(it.key)) out.push({ key: keys.note(id, it.key), icon: "chat", verb: "Noted", object: firstLine(it.text), code: false, state: null, detail: it.text.trim(), runId: id });
        break;
      case "delegated": {
        const child = turn.children.find((c) => c.id === it.childRunId);
        const e = employees.find((x) => x.id === it.employeeId);
        out.push({
          key: keys.delegate(it.childRunId), icon: it.asked ? "ask" : "branch", verb: delegateVerb(it.asked, it.handoff, e ? firstName(e.name) : "a colleague"),
          object: it.task, code: false, state: childState(child), avatar: e ? { name: e.name, hue: e.hue, image: e.image } : undefined, runId: it.childRunId, child,
        });
        break;
      }
      case "visual":
        out.push({ key: keys.visual(id, it.id), icon: "image", verb: "Showed", object: it.title, code: false, state: "ok", runId: id });
        break;
      case "skill": {
        const w = skillWords(it.skill, it.file, it.change);
        out.push({ key: keys.skill(id, it.skill, it.file), icon: "teach", verb: it.change === "added" ? "Wrote a skill" : w.verb, object: w.object, code: false, state: it.change === "added" ? null : "ok", tag: it.change === "added" ? "New" : undefined, runId: id, skill: { skill: it.skill, file: it.file } });
        break;
      }
      default: break;
    }
  }
  if (!part) for (const t of turn.tasks) if (!seenTasks.has(t.id)) agentRow(t);
  return out;
}

const working = (r: WorkRow) => r.state === "pending" || r.state === "wait";

/** A number from a run's id, the same every time, so its words don't change as it renders. */
export const seedOf = (id: string) => [...id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 0);

/** What the line says before the first step and between steps: Claude Code's own spinner words (2.1.318). */
const THINKING = [
  "Accomplishing", "Actioning", "Actualizing", "Architecting", "Baking", "Beaming", "Beboppin'", "Befuddling",
  "Billowing", "Blanching", "Bloviating", "Boogieing", "Boondoggling", "Booping", "Bootstrapping", "Brewing",
  "Bunning", "Burrowing", "Calculating", "Caramelizing", "Cascading", "Catapulting", "Cerebrating", "Channeling",
  "Choreographing", "Churning", "Clauding", "Coalescing", "Cogitating", "Combobulating", "Composing", "Computing",
  "Concocting", "Considering", "Contemplating", "Cooking", "Crafting", "Creating", "Crunching", "Crystallizing",
  "Cultivating", "Deciphering", "Deliberating", "Determining", "Dilly-dallying", "Discombobulating", "Doing",
  "Doodling", "Drizzling", "Ebbing", "Effecting", "Elucidating", "Embellishing", "Enchanting", "Envisioning",
  "Fermenting", "Fiddle-faddling", "Finagling", "Flambéing", "Flibbertigibbeting", "Flowing", "Flummoxing",
  "Fluttering", "Forging", "Forming", "Frolicking", "Frosting", "Gallivanting", "Galloping", "Garnishing",
  "Generating", "Gesticulating", "Germinating", "Gitifying", "Grooving", "Gusting", "Harmonizing", "Hashing",
  "Hatching", "Herding", "Honking", "Hullaballooing", "Hyperspacing", "Ideating", "Imagining", "Improvising",
  "Incubating", "Inferring", "Infusing", "Ionizing", "Jitterbugging", "Julienning", "Kerfuffling", "Kneading",
  "Leavening", "Levitating", "Lollygagging", "Manifesting", "Marinating", "Meandering", "Metamorphosing", "Misting",
  "Moonwalking", "Moseying", "Mulling", "Mustering", "Nebulizing", "Nesting", "Newspapering", "Noodling",
  "Nucleating", "Onioning", "Orbiting", "Orchestrating", "Osmosing", "Perambulating", "Percolating", "Perusing",
  "Philosophizing", "Photosynthesizing", "Polishing", "Pollinating", "Pondering", "Pontificating", "Pouncing",
  "Precipitating", "Prestidigitating", "Processing", "Proofing", "Propagating", "Puttering", "Puzzling",
  "Quantumizing", "Razzle-dazzling", "Razzmatazzing", "Recombobulating", "Reticulating", "Roosting", "Ruminating",
  "Sautéing", "Scampering", "Schlepping", "Scurrying", "Seasoning", "Shenaniganing", "Shimmying", "Simmering",
  "Skedaddling", "Sketching", "Slithering", "Smooshing", "Sock-hopping", "Spelunking", "Spinning", "Sprouting",
  "Stewing", "Sublimating", "Swirling", "Swooping", "Symbioting", "Synthesizing", "Tempering", "Thinking",
  "Thundering", "Tinkering", "Tomfoolering", "Topsy-turvying", "Transfiguring", "Transmogrifying", "Transmuting",
  "Twisting", "Undulating", "Unfurling", "Unraveling", "Vibing", "Waddling", "Wandering", "Warping",
  "Whatchamacalliting", "Whirlpooling", "Whirring", "Whisking", "Wibbling", "Working", "Wrangling", "Zesting",
  "Zigzagging",
];
/** A run's `i`th word for a pause: the line moves through them as its pauses come and go on. */
export const pauseWord = (runId: string, i: number) => THINKING[(seedOf(runId) + i) % THINKING.length];

/** What a reply is doing now, for its line while it runs: a colleague it waits on, else the newest work still going (a
 *  sub-agent, the step in hand), else writing its answer, else thinking. `key` changes with each, so the line rolls.
 *  `pause`: between steps, with nothing real to say; the pause's number in the reply. */
export function nowOf(rows: WorkRow[], turn: LiveRun): { key: string; verb: string; object: string; code: boolean; pause?: number } {
  const newest = [...rows].reverse();
  const r = newest.find((x) => working(x) && x.key.startsWith("d:")) ?? newest.find(working);
  if (r?.key.startsWith("d:")) return { key: r.key, verb: `Waiting on ${r.avatar ? firstName(r.avatar.name) : "a colleague"}`, object: firstLine(r.object), code: false };
  if (r) return { key: r.key, verb: r.verb, object: r.object, code: r.code };
  if (turn.live.trim()) return { key: "writing", verb: "Writing", object: "", code: false };
  if (!rows.length) return { key: "starting", verb: pauseWord(turn.run.id, 0), object: "", code: false };
  // Rows only grow when a step starts, so their count names the pause
  return { key: `pause:${rows.length}`, verb: pauseWord(turn.run.id, rows.length), object: "", code: false, pause: rows.length };
}

const and = (names: string[]) => (names.length < 2 ? names.join("") : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`);

/** A finished stretch in a few words: "Worked for 3 min, with Otto · wrote a skill". `ms` is how long the reply took,
 *  for its last stretch; an earlier stretch counts its steps instead. */
export function summaryOf(rows: WorkRow[], ms: number | null): string {
  const people = [...new Set(rows.flatMap((r) => (r.key.startsWith("d:") && r.avatar ? [firstName(r.avatar.name)] : [])))];
  const skills = rows.filter((r) => r.skill);
  const steps = rows.filter((r) => !r.key.startsWith("n:")).length;
  let s = ms !== null ? `Worked for ${took(ms)}` : `${steps} ${steps === 1 ? "step" : "steps"}`;
  if (people.length) s += `, with ${and(people)}`;
  if (skills.length) s += ` · ${skills.some((r) => r.tag === "New") ? "wrote" : "updated"} ${skills.length === 1 ? "a skill" : `${skills.length} skills`}`;
  return s;
}
