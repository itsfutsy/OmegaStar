// The browser preview's Office: every command and event of src/api/types.ts, answered from the invented office in
// office.ts. Loaded by src/api/client.ts only under `npm run dev` outside Tauri, so production never contains it.
// The URL picks what it shows; README.md beside this file lists every option.
import type { Transport } from "../api/client";
import type {
  AnswerHow, Card, CardReply, CommandName, Commands, Employee, EmployeeView, EventName, Events, HumanRequest, OrgSnapshot,
  Run, RunEvent, RunKind, TeamStats, Thread,
} from "../api/types";
import { describeCron } from "../lib/cron";
import { setMark } from "../lib/marks";
import { COLLAPSE_KEY, NARROW_EXPANDED_KEY } from "../lib/sidebar";
import { storageSet } from "../lib/storage";
import { dayKey } from "../lib/teamStats";
import { setTheme } from "../lib/theme";
import { activityAt, archiveAt, isLive, isOpen, startedByYou } from "../lib/threads";
import { toast } from "../lib/toasts";
import {
  ahead, cannedAnswer, CRAFT, deskEmployee, emptyCompanyPage, emptyOffice, firstRunOffice, fullOffice, HOME, inboxMoreOffice, init, iso, largeOffice, liveMoreOffice, mk, NOW,
  frontDeskOffice, onDay, own, quietOffice, review, tool, type World,
} from "./office";
import { imageUrl, installTauriShim } from "./pages";

/** What `?preview=` can be. */
const STATES = [
  "full", "empty", "first-run", "first-run-no-claude", "first-run-signed-out", "loading", "loading-boot", "error", "error-boot",
  "update-failed", "usage-limit", "api-key",
  "first-run-recheck", "first-run-opening", "first-run-error", "slow-saves", "failed-saves", "quiet", "large-org", "inbox-more",
  "live-more", "thread-error", "live-arrives", "live-ends", "front-desk",
] as const;
type State = (typeof STATES)[number];

type Handlers = { [K in CommandName]: (args: Commands[K]["args"]) => Commands[K]["ret"] | Promise<Commands[K]["ret"]> };

const OFFICE_ERROR = { code: "io", message: "Couldn't read the office: the database is busy. Close any other copy of Omega Star and try again." };
const BOOT_ERROR = { code: "io", message: "Couldn't open the office in ~/Library/Application Support/OmegaStar: another copy of Omega Star has it open." };
const UPDATE_ERROR = { code: "update", message: "Couldn't download the update: the connection timed out." };
const CREATE_ERROR = { code: "io", message: "Couldn't create the office: the folder ~/Library/Application Support/OmegaStar is read-only." };
const LOGIN_ITEM_ERROR = { code: "io", message: "Couldn't turn on start at login: macOS didn't allow it. You can turn it on later in Settings." };
const SAVE_ERROR = { code: "io", message: "Couldn't save: the office folder is read-only." };
const THREAD_ERROR = { code: "io", message: "Couldn't load this conversation: its transcript file is locked by another program." };
/** What Settings saves through; the slow-saves and failed-saves states slow or fail exactly these. */
const SAVES: CommandName[] = ["save_settings", "set_background", "backup_now"];
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const notFound = (message: string) => ({ code: "notFound", message });
const never = <T>() => new Promise<T>(() => {});
const DAY = 86_400_000;
/** The prompt of a run started with no instruction. Mirrors Rust `DEFAULT_PROMPT`. */
const DEFAULT_PROMPT = "Work toward your objective. Start from your role skill, pick the most valuable next step, do it, and report.";

function worldFor(state: State): World {
  switch (state) {
    case "empty": return emptyOffice();
    case "first-run": case "first-run-opening": case "first-run-error": return firstRunOffice();
    case "first-run-no-claude": case "first-run-recheck": return firstRunOffice(false, false);
    case "first-run-signed-out": return firstRunOffice(true, false);
    case "quiet": return quietOffice();
    case "large-org": return largeOffice();
    case "inbox-more": return inboxMoreOffice();
    case "live-more": return liveMoreOffice();
    case "front-desk": return frontDeskOffice();
    default: {
      const w = fullOffice();
      if (state === "usage-limit") {
        w.usage = { ...w.usage, fiveHour: { percent: 100, resetsAt: ahead(52), resetsText: null }, sevenDay: { percent: 84, resetsAt: w.usage.sevenDay?.resetsAt ?? null, resetsText: null }, heldUntil: ahead(52) };
      }
      if (state === "api-key") w.usage = { ...w.usage, fiveHour: null, sevenDay: null, subscription: false };
      return w;
    }
  }
}

/** Whether `n` is in a cron field: `*`, a number, a range `1-5` or a list `1,3,5` (and lists of ranges). */
function cronHas(field: string, n: number): boolean {
  if (field === "*") return true;
  return field.split(",").some((part) => {
    const [a, b] = part.split("-").map(Number);
    return b === undefined ? a === n : n >= a && n <= b;
  });
}

/** The next three times a five-part schedule fires, from now: minute and hour as numbers (or every hour at a minute),
 *  day of the month and weekday as cron reads them (either matching when both are given). Anything fancier gets three
 *  hourly times, which is enough for a preview. */
function nextFires([min, hour, dom, , dow]: string[]): string[] {
  const m = Number(min);
  if (!Number.isFinite(m)) return [ahead(60), ahead(120), ahead(180)];
  const hours = hour === "*" ? Array.from({ length: 24 }, (_, h) => h) : [Number(hour)];
  if (hours.some((h) => !Number.isFinite(h))) return [ahead(60), ahead(120), ahead(180)];
  const out: string[] = [];
  for (let d = 0; d < 400 && out.length < 3; d++) {
    const day = new Date(onDay(d, 0));
    const domOk = cronHas(dom, day.getDate());
    const dowOk = cronHas(dow.replace(/\b7\b/g, "0"), day.getDay());
    const matches = dom !== "*" && dow !== "*" ? domOk || dowOk : domOk && dowOk;
    if (!matches) continue;
    for (const h of hours) {
      const at = onDay(d, h, m);
      if (Date.parse(at) > Date.now() && out.length < 3) out.push(at);
    }
  }
  return out;
}

export function previewTransport(): Transport {
  const params = new URLSearchParams(location.search);
  const state: State = STATES.find((s) => s === params.get("preview")) ?? "full";
  const w = worldFor(state);

  installTauriShim();
  const theme = params.get("theme");
  if (theme === "light" || theme === "dark" || theme === "system") setTheme(theme, false);
  // The sidebar at every size: collapsed is the rail everywhere, expanded the full sidebar everywhere (src/lib/sidebar.ts).
  const sidebar = params.get("sidebar");
  if (sidebar) {
    storageSet(COLLAPSE_KEY, sidebar === "collapsed" ? "1" : "0");
    storageSet(NARROW_EXPANDED_KEY, sidebar === "collapsed" ? "0" : "1");
  }
  // What counts as new is the last day, so the same threads are unread on every load.
  storageSet("omega.shelvesSince", iso(NOW - DAY));
  if (w.runs.some((r) => r.id === "board-1")) setMark("board-1", { snoozedUntil: onDay(1, 9) });

  // ---------------------------------------------------------------- events

  const subs = new Map<EventName, Set<(payload: never) => void>>();
  function emit<E extends EventName>(name: E, payload: Events[E]) {
    const copy = structuredClone(payload);
    setTimeout(() => subs.get(name)?.forEach((h) => (h as (p: Events[E]) => void)(copy)), 0);
  }
  const emitRun = (r: Run) => emit("run:updated", r);
  function append(runId: string, events: RunEvent[]) {
    (w.events[runId] ??= []).push(...events);
    emit("run:event", { runId, events });
  }

  // ---------------------------------------------------------------- reading the world

  const byId = (id: string) => w.runs.find((r) => r.id === id);
  function need(id: string): Run {
    const r = byId(id);
    if (!r) throw notFound("That run could not be found.");
    return r;
  }
  const rootOf = (r: Run): string => (r.kind.type === "reply" ? r.kind.toRunId : r.kind.type === "answered" && r.kind.thread ? r.kind.thread : r.id);
  function threadOf(rootId: string, related = true): Thread {
    const root = need(rootId);
    const replies = w.runs.filter((r) => r.id !== rootId && rootOf(r) === rootId).sort((a, b) => a.createdAt.localeCompare(b.createdAt));
    return { root, replies, related: related ? (w.related[rootId] ?? []).filter(byId).map((id) => threadOf(id, false)) : [] };
  }
  // Conversations, the replies to reports and the checks on requests run beside the work an employee is given.
  const workLane = (r: Run) => !["task", "reply", "chat", "answered", "replyCheck", "nudge", "unanswered"].includes(r.kind.type);

  function view(e: Employee): EmployeeView {
    const mine = w.runs.filter((r) => r.employeeId === e.id);
    const live = mine.filter(isLive).sort((a, b) => Number(b.state === "running") - Number(a.state === "running") || a.createdAt.localeCompare(b.createdAt));
    const ended = mine.filter((r) => r.endedAt).sort((a, b) => b.endedAt!.localeCompare(a.endedAt!));
    const lastRun = ended.find(workLane) ?? null;
    const attention = !!lastRun && (lastRun.state === "failed" || lastRun.state === "lost" || lastRun.denials.length > 0);
    const fires = w.schedules.filter((s) => s.employeeId === e.id && s.enabled && s.nextFire).map((s) => s.nextFire!).sort();
    return {
      ...e, live, lastRun, working: live.filter((r) => r.state === "running").length,
      presence: live.some((r) => r.state === "running") ? "working" : live.length ? "queued" : attention ? "attention" : "idle",
      nextFire: fires[0] ?? null, lastWorkedAt: ended[0]?.endedAt ?? null, status: w.status[e.id] ?? { number: null, lastReport: null, next: null },
    };
  }
  function snapshot(): OrgSnapshot {
    if (!w.company) throw notFound("There is no company yet.");
    return {
      company: w.company, employees: w.employees.map(view),
      people: w.people.map((p) => ({ ...p, openRequests: w.requests.filter((r) => r.personId === p.id && r.state === "open").length })),
      desk: w.desk ? view(w.desk) : null, frontDesk: w.settings.frontDesk,
    };
  }

  // ---------------------------------------------------------------- work that moves

  let n = 0;
  const nextId = (prefix: string) => `${prefix}-${++n}`;
  // performance.now, not Date: the screenshot script holds the clock still, and a re-check must still be "later".
  const loadedAt = performance.now();

  function finish(r: Run, text: string) {
    if (r.state !== "running") return;
    const at = Date.now();
    append(r.id, [{ type: "text", text }, { type: "result", ok: true, text, turns: 4, durationMs: at - Date.parse(r.startedAt ?? r.createdAt) }]);
    Object.assign(r, { state: "succeeded", endedAt: iso(at), summary: text, turns: 4, durationMs: at - Date.parse(r.startedAt ?? r.createdAt), costUsd: 0.21, contextTokens: 24_000 });
    emitRun(r);
  }

  /** Streams `text` into `r`, `step` words every `every` ms, then finishes it. `from`: how many words are already in. */
  function stream(r: Run, text: string, every: number, from = 0, step = 2) {
    const words = text.match(/\S+\s*/g) ?? [];
    let i = from;
    const timer = window.setInterval(() => {
      if (r.state !== "running") return window.clearInterval(timer);
      if (i >= words.length) {
        window.clearInterval(timer);
        finish(r, text);
        return;
      }
      append(r.id, [{ type: "textDelta", text: words.slice(i, i + step).join("") }]);
      i += step;
    }, every);
  }

  /** Your words into a working run: taken at its next step a few seconds later, as a real run takes them. */
  function steer(r: Run, text: string) {
    const id = `m${Date.now()}`;
    append(r.id, [{ type: "user", text, id }]);
    window.setTimeout(() => append(r.id, [{ type: "heard", id }]), 6000);
  }

  /** A run you started: it waits a moment, starts, reads its notes, thinks a while, checks a second file, and answers.
   *  The pause is long enough for the work line to reach its ad (Sponsor.tsx), and the second file arrives while the
   *  ad is on show. */
  function respond(r: Run, text = cannedAnswer(r.prompt)) {
    w.runs.unshift(r);
    w.events[r.id] = [];
    emitRun(r);
    window.setTimeout(() => {
      if (r.state !== "queued") return;
      Object.assign(r, { state: "running", startedAt: iso(Date.now()) });
      emitRun(r);
      append(r.id, [init(9), { type: "toolCall", id: "r1", tool: "Read", brief: "notes/fernbrook.md" }]);
      window.setTimeout(() => append(r.id, [{ type: "toolResult", id: "r1", ok: true, brief: "36 lines" }]), 1200);
      window.setTimeout(() => append(r.id, [{ type: "toolCall", id: "r2", tool: "Read", brief: "notes/team.md" }]), 7200);
      window.setTimeout(() => append(r.id, [{ type: "toolResult", id: "r2", ok: true, brief: "18 lines" }]), 8400);
      window.setTimeout(() => { if (r.state === "running") stream(r, text, 90); }, 12_000);
    }, 700);
    return r;
  }
  function newRun(employeeId: string, kind: RunKind, prompt: string, o: Partial<Run> = {}): Run {
    const r = mk(nextId("run"), employeeId, kind, prompt, { state: "queued", ...o });
    r.createdAt = iso(Date.now());
    return r;
  }

  /** The live work in the busy office: Juniper's reply streaming, her sub-agent reading, and Otto checking billing. */
  function startLive(billingMs = 40_000) {
    const plan = byId("plan-2");
    const billing = byId("billing-1");
    if (!plan || !billing) return;
    const draft = "Here's the draft. It goes out on **30 January**, 30 days before the change.\n\n> **Subject:** Your Studio plan from 1 March\n>\n> Hi {first name},\n>\n> From 1 March the Studio plan moves from $29 to $34 a month. It's our first change in two years, and it pays for the waitlists and text reminders you asked for.\n>\n> If you're on an annual plan, nothing changes until your renewal date.\n>\n> Questions? Just reply. Mae and the team read every one.\n\nTwo things before it goes:\n\n1. Otto is checking the billing side now. I'll fold his answer in when it's back.\n2. I'd like Harriet to see it before the board update. Shall I send it to her on Friday?";
    const words = draft.match(/\S+\s*/g) ?? [];
    const start = Math.floor(words.length * 0.42);
    w.events[plan.id].push({ type: "textDelta", text: words.slice(0, start).join("") });
    stream(plan, draft, 700, start, 1);
    planDraft = { words, start };

    const activities = ["Reading billing/annual-plans.md", "Searching for \"$29\" in getting-started/", "Reading getting-started/choose-a-plan.md", "Checking 3 articles against the new price"];
    let step = 0;
    const sub = window.setInterval(() => {
      step++;
      if (plan.state !== "running") return window.clearInterval(sub);
      if (step > activities.length) {
        window.clearInterval(sub);
        append(plan.id, [{ type: "taskEnded", taskId: "task-help", outcome: "done", toolUses: 14, tokens: 31_200, durationMs: 64_000 + step * 6000 }]);
        return;
      }
      append(plan.id, [{ type: "taskProgress", taskId: "task-help", activity: activities[step - 1], lastTool: step % 2 ? "Grep" : "Read", toolUses: 7 + step * 2, tokens: 18_400 + step * 3_100, durationMs: 64_000 + step * 6000 }]);
    }, 6000);

    window.setTimeout(() => append(billing.id, [{ type: "toolResult", id: "b3", ok: true, brief: "2 to change, 0 to destroy" }]), 2500);
    window.setTimeout(() => append(billing.id, tool("b4", "Read", "web/src/pages/pricing.tsx", "140 lines")), 6000);
    window.setTimeout(() => finish(billing, "Two places hard-code $29: the pricing page and the upgrade screen. Both now read the price from billing. The new price ID is ready in the infrastructure config; it goes live with one change on 1 March, and annual plans keep their old price ID until renewal."), billingMs);
  }
  // Live work streams in every busy state; in live-more, Otto's delegated check finishes a few seconds in.
  let planDraft: { words: string[]; start: number } | null = null;
  if (state !== "loading" && state !== "loading-boot" && state !== "quiet") startLive(state === "live-more" ? 6000 : undefined);

  /** live-arrives: three seconds in, Juniper asks Mae to update the help centre, and Mae starts on it. */
  function arriveLive() {
    const plan = need("plan-2");
    const task = "Update the three help-centre articles that quote $29 so they say $34 from 1 March.";
    window.setTimeout(() => {
      const help = mk("help-1", "mae", { type: "delegated", parentRunId: plan.id }, task, { state: "running", end: iso(Date.now()) });
      help.createdAt = help.startedAt = iso(Date.now());
      w.runs.unshift(help);
      w.events[help.id] = [init(9), { type: "text", text: "Three articles quote the Studio price. Starting with choose-a-plan." }];
      emitRun(help);
      append(plan.id, [...tool("q9", "mcp__omega__delegate", "Mae: update the help-centre articles that quote $29", "Delegated to Mae"), { type: "delegated", childRunId: help.id, employeeId: "mae", task }]);
      window.setTimeout(() => append(help.id, [{ type: "toolCall", id: "h1", tool: "Read", brief: "help-centre/billing/choose-a-plan.md" }]), 1500);
    }, 3000);
  }

  /** live-ends: everything live finishes in turn: the sub-agent at 2.5 s, Otto's billing check at 4 s, Juniper's
   *  reply at 6.5 s, and the runs waiting to start get going and finish by 8 s, so nothing is live after that. */
  function endLive() {
    const plan = need("plan-2");
    const billing = need("billing-1");
    window.setTimeout(() => append(plan.id, [{ type: "taskEnded", taskId: "task-help", outcome: "done", toolUses: 14, tokens: 31_200, durationMs: 88_000 }]), 2500);
    window.setTimeout(() => finish(billing, "Two places hard-code $29: the pricing page and the upgrade screen. Both now read the price from billing."), 4000);
    window.setTimeout(() => {
      if (plan.state !== "running") return;
      const { words, start } = planDraft!;
      append(plan.id, [{ type: "textDelta", text: words.slice(start).join("") }]);
      finish(plan, words.join(""));
    }, 6500);
    window.setTimeout(() => {
      for (const r of w.runs.filter((x) => x.state === "queued")) {
        Object.assign(r, { state: "running", startedAt: iso(Date.now()) });
        emitRun(r);
        finish(r, "Done. Nothing else to report.");
      }
    }, 8000);
  }
  if (state === "live-arrives") arriveLive();
  if (state === "live-ends") endLive();

  let toasted = false;
  function toastsOnce() {
    if (toasted || params.get("toasts") !== "1") return;
    toasted = true;
    window.setTimeout(() => {
      toast("Sent to Otto: Upgrade Saturday night.", "ok", { label: "Undo", run: () => {} });
      toast("Archived. Also filed 2 related runs", "neutral", { label: "Undo", run: () => {} });
      toast("Couldn't reach staging.fernbrook.app. Rafi tries again at 1:00 am.", "err");
    }, 400);
  }

  // ---------------------------------------------------------------- cards

  function answerOf(c: Card, a: CardReply): { how: AnswerHow; text: string } {
    switch (a.type) {
      case "choice": return { how: "tapped", text: c.ask.type === "choices" ? c.ask.choices[a.index] ?? "" : "" };
      case "items": {
        const ticked = c.ask.type === "items" ? c.ask.items.filter((i) => a.picks.some((p) => p.id === i.id && p.tick)).map((i) => i.text) : [];
        return { how: "tapped", text: c.ask.type === "items" ? `${c.ask.verb} ${ticked.join(", ")}` : "" };
      }
      case "done": return { how: "tapped", text: "Done" };
      case "dismiss": return { how: "dismissed", text: "" };
      case "reject": case "cant": return { how: "rejected", text: a.type === "cant" ? "Can't" : "No" };
      case "text": return { how: "typed", text: a.text };
      case "seen": return { how: "tapped", text: "" };
      case "ask": case "later": return { how: "typed", text: "" };
    }
  }

  function answerCard({ runId, card, answer }: Commands["answer_card"]["args"]): Run[] {
    const r = need(runId);
    const c = r.cards[card];
    if (!c) throw notFound("That card is gone.");
    const root = need(rootOf(r));
    if (answer.type === "ask") return [respond(newRun(r.employeeId, { type: "reply", toRunId: root.id, label: root.title ?? "Report", task: startedByYou(root), card: true }, answer.text, { aboutCard: true }))];
    if (answer.type === "later") {
      Object.assign(c, { backAt: answer.until, parked: c.timesBack > 0 });
      w.inbox = w.inbox.filter((id) => id !== root.id);
      emitRun(r);
      return [];
    }
    const held = r.cards.some((x, i) => i !== card && isOpen(x));
    c.answer = { at: iso(Date.now()), ...answerOf(c, answer), held };
    if (!held) for (const x of r.cards) if (x.answer) x.answer.held = false;
    emitRun(r);
    if (!threadOf(root.id).replies.concat(root).some((x) => x.cards.some(isOpen)) && !startedByYou(root)) {
      root.archivedAt = iso(Date.now());
      w.inbox = w.inbox.filter((id) => id !== root.id);
      emitRun(root);
    }
    return [];
  }

  // ---------------------------------------------------------------- requests and people

  function request(id: string): HumanRequest {
    const r = w.requests.find((x) => x.id === id);
    if (!r) throw notFound("That request could not be found.");
    return r;
  }
  const changed = () => emit("org:changed", null);

  function hire(draft: Commands["hire"]["args"]["draft"]): Employee {
    const slug = draft.name.toLowerCase().replace(/[^a-z]+/g, "-").replace(/^-|-$/g, "") || "new-hire";
    const e: Employee = { ...draft, id: slug, slug, chatSession: "", hiredAt: iso(Date.now()) };
    w.employees.push(e);
    w.skills[e.id] = [own(slug, `${draft.name.split(" ")[0]}'s role: ${draft.objective || draft.title}`, iso(Date.now()), ["SKILL.md"], { role: true }), CRAFT(iso(Date.now()))];
    w.schedules.push({ ...review(e.id), nextFire: onDay(7, 16) });
    changed();
    return e;
  }

  // ---------------------------------------------------------------- every command

  const h: Handlers = {
    setup_status: () => {
      // first-run-recheck: the checks while the page opens find no Claude Code; any check from half a second on (a
      // re-check you ask for) finds it, signed in. Timed, not counted: React's dev mode runs the first check twice.
      if (state === "first-run-recheck" && performance.now() - loadedAt > 500) w.setup = { ...firstRunOffice().setup, background: w.setup.background };
      return { ...w.setup, company: w.company };
    },
    complete_setup: ({ companyName, ceoName, title }) => {
      if (state === "first-run-opening") return never();
      if (state === "first-run-error") throw CREATE_ERROR;
      w.company = { name: companyName, ceoName, title: title ?? "CEO", email: null, reportsTo: null };
      w.setup.company = w.company;
      w.companyPage = w.companyPage || emptyCompanyPage(companyName);
      return w.company;
    },
    update_company: ({ name, ceoName, title, reportsTo, image }) => {
      w.company = { ...w.company!, name, ceoName, title, reportsTo, image };
      changed();
      return w.company;
    },
    set_background: ({ enabled }) => {
      if (state === "first-run-error") throw LOGIN_ITEM_ERROR;
      w.setup.background = enabled;
      return { ...w.setup, company: w.company };
    },
    org: () => {
      toastsOnce();
      return snapshot();
    },
    models: () => [
      { id: "claude-opus-5-5", name: "Opus 5.5" }, { id: "claude-fable-5-2", name: "Fable 5.2" }, { id: "claude-sonnet-5-5", name: "Sonnet 5.5" },
      { id: "claude-haiku-5-5", name: "Haiku 5.5" }, { id: "claude-fable-5-1", name: "Fable 5.1" }, { id: "claude-opus-5", name: "Opus 5" },
    ],
    skill_catalog: () => w.catalog,
    skill_sources: () => ({ skillsDir: `${HOME}/.claude/skills`, found: w.catalog.length, unreadable: 0, error: null }),
    hire: ({ draft }) => hire(draft),
    front_desk: () => {
      if (!w.settings.frontDesk) throw new Error("The front desk is off. Turn it on in Settings first.");
      if (!w.desk) { w.desk = deskEmployee(); changed(); }
      return view(w.desk);
    },
    update_employee: ({ employee }) => {
      w.employees = w.employees.map((e) => (e.id === employee.id ? employee : e));
      changed();
      return employee;
    },
    let_go: ({ employeeId }) => {
      w.employees = w.employees.filter((e) => e.id !== employeeId);
      w.schedules = w.schedules.filter((s) => s.employeeId !== employeeId);
      changed();
      return null;
    },
    schedules: ({ employeeId }) => w.schedules.filter((s) => s.employeeId === employeeId),
    save_schedule: ({ draft }) => {
      const old = w.schedules.find((s) => s.id === draft.id);
      const s = { ...old, ...draft, id: draft.id ?? nextId("schedule"), nextFire: draft.enabled ? onDay(1, 9) : null };
      w.schedules = old ? w.schedules.map((x) => (x.id === s.id ? s : x)) : [...w.schedules, s];
      changed();
      return s;
    },
    delete_schedule: ({ scheduleId }) => {
      w.schedules = w.schedules.filter((s) => s.id !== scheduleId);
      changed();
      return null;
    },
    cron_preview: ({ cron }) => {
      const p = cron.trim().split(/\s+/);
      if (p.length !== 5) return { ok: false, error: "A schedule has five parts: minute, hour, day of the month, month and weekday." };
      return { ok: true, description: describeCron(cron) ?? "A custom schedule", next: nextFires(p) };
    },
    run_now: ({ employeeId, instruction }) => respond(newRun(employeeId, { type: "manual" }, instruction ?? DEFAULT_PROMPT), "Done. Nothing urgent turned up; the details are in my notes."),
    cancel_run: ({ runId }) => {
      const r = need(runId);
      Object.assign(r, { state: "cancelled", endedAt: iso(Date.now()) });
      emitRun(r);
      return null;
    },
    start_task: ({ employeeId, text, attachments }) => {
      const label = text.split("\n")[0].slice(0, 60);
      const r = respond(newRun(employeeId, { type: "task", label, from: null, context: "" }, text, { attachments: attachments.map((a) => a.name) }));
      w.open.unshift(r.id);
      return r;
    },
    open_attachment: () => null,
    read_image: ({ path }) => imageUrl(path),
    open_image: () => null,
    open_report_path: () => null,
    runs: ({ employeeId, limit }) => w.runs.filter((r) => !employeeId || r.employeeId === employeeId).sort((a, b) => b.createdAt.localeCompare(a.createdAt)).slice(0, limit),
    search_runs: ({ query, limit }) => {
      const q = query.toLowerCase();
      return w.runs.filter((r) => [r.prompt, r.summary ?? "", ...r.cards.map((c) => c.message)].some((t) => t.toLowerCase().includes(q))).slice(0, limit);
    },
    run_usage: ({ since }) => {
      const done = w.runs.filter((r) => r.endedAt && r.createdAt >= since);
      const handedOn = (r: Run) => w.runs.filter((x) => (x.kind.type === "delegated" || x.kind.type === "asked") && x.kind.parentRunId === r.id).length;
      const cost = (rs: Run[]) => Math.round(rs.reduce((s, r) => s + (r.costUsd ?? 0), 0) * 100) / 100;
      const broke = (r: Run) => r.state === "failed" || r.state === "lost";
      return {
        employees: w.employees.flatMap((e) => {
          const rs = done.filter((r) => r.employeeId === e.id);
          return rs.length ? [{ employeeId: e.id, runs: rs.length, decisions: rs.reduce((s, r) => s + r.cards.length, 0), handoffs: rs.reduce((s, r) => s + handedOn(r), 0), failed: rs.filter(broke).length, costUsd: cost(rs) }] : [];
        }),
        routines: w.schedules.filter((s) => !s.review).flatMap((s) => {
          const rs = done.filter((r) => r.kind.type === "scheduled" && r.kind.scheduleId === s.id);
          const useful = rs.filter((r) => r.cards.length || r.denials.length || r.skillChanges.length || broke(r) || handedOn(r));
          return rs.length ? [{ scheduleId: s.id, employeeId: s.employeeId, label: s.label, quiet: s.quiet, runs: rs.length, reached: s.quiet ? useful.length : rs.length, failed: rs.filter(broke).length, useful: useful.length, costUsd: cost(rs) }] : [];
        }),
      };
    },
    // An invented history: each employee works most days from their hire, a little more as weeks pass, and raises
    // fewer cards as their skills grow. Stable from load to load (no randomness).
    team_stats: () => {
      const today = new Date();
      const days: TeamStats["days"] = [];
      const learned = new Map<string, number>();
      w.employees.forEach((e, ei) => {
        const hired = new Date(e.hiredAt);
        for (let d = new Date(hired.getFullYear(), hired.getMonth(), hired.getDate()), i = 0; d <= today; d.setDate(d.getDate() + 1), i++) {
          const wobble = ((ei * 7 + i * 13) % 5) - 2;
          const weekend = d.getDay() === 0 || d.getDay() === 6;
          const runs = Math.max(0, Math.round((weekend ? 1 : 4) + i / 12 + wobble));
          if (!runs) continue;
          const cards = Math.max(0, Math.round(runs / (2 + i / 10)));
          days.push({ day: dayKey(d), employeeId: e.id, runs, workMs: runs * (6 + ((ei + i) % 9)) * 60_000, cards });
          if ((i + ei) % 3 === 0) learned.set(dayKey(d), (learned.get(dayKey(d)) ?? 0) + 1 + ((i + ei) % 4));
        }
      });
      return {
        staff: w.employees.map((e) => ({ employeeId: e.id, name: e.name, hiredAt: e.hiredAt, departedAt: null })),
        days: days.sort((a, b) => a.day.localeCompare(b.day)),
        learned: [...learned].map(([day, n]) => ({ day, n })).sort((a, b) => a.day.localeCompare(b.day)),
      };
    },
    run_detail: ({ runId }) => {
      const run = need(runId);
      return { run, events: w.events[runId] ?? [], children: w.runs.filter((r) => (r.kind.type === "delegated" || r.kind.type === "asked") && r.kind.parentRunId === runId) };
    },
    archived: () => {
      const roots = new Set([...w.archive, ...w.open.filter((id) => byId(id)?.archivedAt)]);
      return [...roots].filter(byId).map((id) => threadOf(id)).sort((a, b) => archiveAt(b).localeCompare(archiveAt(a)));
    },
    conversations: () => {
      const threads = w.open.filter((id) => byId(id) && !byId(id)!.archivedAt).map((id) => threadOf(id)).sort((a, b) => activityAt(b).localeCompare(activityAt(a)));
      return { threads, inbox: w.inbox.filter((id) => threads.some((t) => t.root.id === id)) };
    },
    thread: ({ runId }) => threadOf(rootOf(need(runId)), false),
    set_archived: ({ runId, archived }) => {
      const root = rootOf(need(runId));
      const ids = [root, ...(w.related[root] ?? [])];
      for (const id of ids) {
        const r = need(id);
        r.archivedAt = archived ? iso(Date.now()) : null;
        emitRun(r);
      }
      if (!archived && !w.open.includes(root)) w.open.unshift(root);
      return ids;
    },
    unarchive_runs: ({ runIds }) => {
      for (const id of runIds) {
        const r = need(id);
        r.archivedAt = null;
        emitRun(r);
      }
      return null;
    },
    retry_run: ({ runId }) => {
      const r = need(runId);
      const root = need(rootOf(r));
      const kind: RunKind = startedByYou(root) ? { type: "reply", toRunId: root.id, label: root.title ?? "Conversation", task: true, card: false } : r.kind;
      return respond(newRun(r.employeeId, kind, r.prompt, { omegaWrote: true }), "Done this time. The connection held, and everything checks out.");
    },
    reply_to_run: ({ runId, text, archive, attachments }) => {
      const root = need(rootOf(need(runId)));
      // A turn still working takes your words at its next step, as the app's does.
      const replies = threadOf(root.id, false).replies;
      const latest = replies[replies.length - 1] ?? root;
      if (latest.state === "running" && !attachments.length) {
        steer(latest, text);
        return latest;
      }
      root.repliedAt = iso(Date.now());
      if (archive) root.archivedAt = iso(Date.now());
      if (!w.open.includes(root.id)) w.open.unshift(root.id);
      return respond(newRun(root.employeeId, { type: "reply", toRunId: root.id, label: root.title ?? "Report", task: startedByYou(root), card: false }, text, { attachments: attachments.map((a) => a.name) }));
    },
    approve_blocked: ({ runId, rules, always }) => {
      const r = need(runId);
      const e = w.employees.find((x) => x.id === r.employeeId);
      if (always && e) {
        e.allowRules = [...new Set([...e.allowRules, ...rules])];
        changed();
      }
      const root = need(rootOf(r));
      root.archivedAt = iso(Date.now());
      emitRun(root);
      return respond(newRun(r.employeeId, r.kind, r.prompt, { allowOnce: always ? [] : rules, omegaWrote: true }), "Done. The draft campaign is ready to schedule for 30 January.");
    },
    answer_card: answerCard,
    card_file: ({ runId, card }) => {
      const path = need(runId).cards[card]?.file;
      const text = path ? w.cardFiles[path] : undefined;
      if (text === undefined) throw notFound("That file is no longer there.");
      return text;
    },
    parked_runs: () => w.runs.filter((r) => r.cards.some((c) => c.parked)),
    open_asks: () => w.runs.filter((r) => r.kind.type === "asked" && (isLive(r) || (r.state === "succeeded" && !r.deliveredAt))).sort((a, b) => a.createdAt.localeCompare(b.createdAt)),
    week_reviews: () => w.runs.filter((r) => r.kind.type === "scheduled" && r.kind.review && Date.parse(r.createdAt) > Date.now() - 8 * DAY).sort((a, b) => a.createdAt.localeCompare(b.createdAt)),
    employee_skills: ({ employeeId }) => w.skills[employeeId] ?? [],
    skill_health: ({ employeeId }) => w.health[employeeId] ?? { findings: [] },
    checks: ({ employeeId }) => w.checks.filter((c) => c.employeeId === employeeId),
    drop_check: ({ checkId }) => {
      const c = w.checks.find((x) => x.id === checkId);
      if (!c) throw notFound("That check is gone.");
      Object.assign(c, { state: "dropped", closedAt: iso(Date.now()) });
      changed();
      return c;
    },
    tidy: ({ employeeId }) => respond(newRun(employeeId, { type: "tidy", reason: "manual", signature: [], plan: null }, "Tidy your skills and notes."), "Tidied: nothing big to change. Merged one repeated rule."),
    approve_tidy: ({ runId }) => {
      const r = need(runId);
      r.approvedAt = iso(Date.now());
      w.inbox = w.inbox.filter((id) => id !== rootOf(r));
      emitRun(r);
      return respond(newRun(r.employeeId, { type: "tidy", reason: "approved", signature: [], plan: r.proposed }, "Make the changes Maren approved."), "Done. weekly-report is split in two, and old-launch-checklist is retired.");
    },
    tidy_runs: ({ employeeId }) => w.runs.filter((r) => r.employeeId === employeeId && r.kind.type === "tidy"),
    read_skill_file: ({ employeeId, skill, file }) => {
      const text = w.files[`${employeeId}/${skill}/${file}`];
      if (text === undefined) throw notFound(`${skill}/${file} doesn't exist yet.`);
      return text;
    },
    save_skill_file: ({ employeeId, skill, file, text }) => {
      const key = `${employeeId}/${skill}/${file}`;
      const before = w.files[key] ?? null;
      w.files[key] = text;
      const rev = { id: nextId("rev"), runId: null, at: iso(Date.now()), skill, file, change: before === null ? "added" as const : "modified" as const, before, after: text };
      (w.revisions[employeeId] ??= []).unshift(rev);
      changed();
      return rev;
    },
    company_page: () => ({ text: w.companyPage, findings: [] }),
    save_company_page: ({ text }) => {
      w.companyPage = text;
      return { text, findings: [] };
    },
    skill_history: ({ employeeId }) => w.revisions[employeeId] ?? [],
    revert_revision: ({ revisionId }) => {
      const rev = Object.values(w.revisions).flat().find((r) => r.id === revisionId);
      if (!rev) throw notFound("That change could not be found.");
      return { ...rev, id: nextId("rev"), at: iso(Date.now()), before: rev.after, after: rev.before };
    },
    promote_skill: ({ skill }) => {
      const info = { id: `user:${skill}`, name: skill, description: "Copied from an employee's skills.", source: "user" as const };
      w.catalog.push(info);
      return info;
    },
    import_skills: ({ employeeId, skillIds }) => respond(newRun(employeeId, { type: "organize", imported: skillIds.map((id) => ({ id, name: id.split(":").pop() ?? id })), settled: false }, "Organize the skills Maren imported for you."), "Organized them into my own skills."),
    staged_imports: ({ employeeId }) => w.staged[employeeId] ?? [],
    organize: ({ employeeId }) => {
      const staged = w.staged[employeeId] ?? [];
      w.staged[employeeId] = [];
      return respond(newRun(employeeId, { type: "organize", imported: staged.map((s) => ({ id: `user:${s.name}`, name: s.name })), settled: false }, "Organize the skills Maren imported for you."), "Organized them into my own skills.");
    },
    revert_run: () => 2,
    import_originals: ({ runId }) => w.originals[runId] ?? [],
    remove_originals: ({ names }) => names,
    settle_originals: ({ runId }) => {
      const r = need(runId);
      if (r.kind.type === "organize") r.kind = { ...r.kind, settled: true };
      emitRun(r);
      return r;
    },
    add_person: ({ draft }) => {
      const p = { ...draft, id: nextId("person"), addedAt: iso(Date.now()) };
      w.people.push(p);
      changed();
      return p;
    },
    update_person: ({ person }) => {
      w.people = w.people.map((p) => (p.id === person.id ? person : p));
      changed();
      return person;
    },
    remove_person: ({ personId }) => {
      w.people = w.people.filter((p) => p.id !== personId);
      changed();
      return null;
    },
    requests: ({ personId, employeeId, state: s }) => w.requests.filter((r) => (!personId || r.personId === personId) && (!employeeId || r.employeeId === employeeId) && (!s || r.state === s)),
    resolve_request: ({ requestId, state: s, response }) => {
      const r = request(requestId);
      Object.assign(r, { state: s, response, resolvedAt: iso(Date.now()) });
      if (r.delivery?.watch) r.delivery.watch.phase = { type: "stopped", at: iso(Date.now()) };
      emit("request:updated", r);
      return r;
    },
    watch_request: ({ requestId, action }) => {
      const r = request(requestId);
      const watch = r.delivery?.watch;
      if (watch) {
        const at = iso(Date.now());
        if (action === "check") watch.lastCheckedAt = at;
        if (action === "nudge") watch.nudgedAt = at;
        if (action === "stop") watch.phase = { type: "stopped", at };
        if (action === "resume") watch.phase = { type: "watching" };
      }
      emit("request:updated", r);
      return r;
    },
    proposals: () => w.proposals,
    approve_proposal: ({ proposalId, draft, note }) => {
      const p = w.proposals.find((x) => x.id === proposalId);
      if (!p) throw notFound("That proposal is gone.");
      const e = hire(draft);
      Object.assign(p, { state: "approved", note, hiredId: e.id, resolvedAt: iso(Date.now()) });
      emit("proposal:updated", p);
      return e;
    },
    decline_proposal: ({ proposalId, note }) => {
      const p = w.proposals.find((x) => x.id === proposalId);
      if (!p) throw notFound("That proposal is gone.");
      Object.assign(p, { state: "declined", note, resolvedAt: iso(Date.now()) });
      emit("proposal:updated", p);
      return p;
    },
    settings: () => w.settings,
    save_settings: ({ settings }) => {
      w.settings = settings;
      changed();
      return settings;
    },
    open_data_dir: () => null,
    backup_status: () => w.backup,
    backup_now: () => {
      w.backup = { ...w.backup, running: true };
      window.setTimeout(() => {
        w.backup = { ...w.backup, running: false, lastAt: iso(Date.now()), lastAttemptAt: iso(Date.now()) };
        emit("backup:updated", w.backup);
      }, 1800);
      emit("backup:updated", w.backup);
      return w.backup;
    },
    pick_backup_dir: () => `${HOME}/Backups/Omega Star`,
    open_backup_dir: () => null,
    usage_limits: () => w.usage,
    refresh_usage: () => {
      w.usage = { ...w.usage, observedAt: iso(Date.now()) };
      emit("usage:updated", w.usage);
      return w.usage;
    },
    use_signed_in_account: () => {
      w.usage = { ...w.usage, accountIssue: null };
      emit("usage:updated", w.usage);
      return w.company!;
    },
    check_update: () => w.update,
    // A real update restarts the app, so it never answers.
    install_update: () => (state === "update-failed" ? Promise.reject(UPDATE_ERROR) : never<null>()),
  };

  return {
    async invoke(name, args) {
      if (state === "loading-boot" || (state === "loading" && name !== "setup_status")) return never();
      if (state === "error-boot" && name === "setup_status") throw BOOT_ERROR;
      if (state === "error" && name !== "setup_status") throw OFFICE_ERROR;
      if (state === "thread-error" && name === "thread") throw THREAD_ERROR;
      if (state === "slow-saves" && SAVES.includes(name)) await wait(1500);
      if (state === "failed-saves" && SAVES.includes(name)) { await wait(600); throw SAVE_ERROR; }
      await wait(0);
      const run = h[name] as (a: typeof args) => Commands[typeof name]["ret"] | Promise<Commands[typeof name]["ret"]>;
      return structuredClone(await run(args));
    },
    listen(name, handler) {
      let set = subs.get(name);
      if (!set) subs.set(name, (set = new Set()));
      set.add(handler as (payload: never) => void);
      return Promise.resolve(() => void set.delete(handler as (payload: never) => void));
    },
  };
}
