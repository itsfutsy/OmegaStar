// The invented office the browser preview serves: Fernbrook, a small company that makes booking software for
// fitness studios, run by Maren Holt with six AI employees and three real people. Every name, figure and address
// here is made up. Times are relative to when the page loaded, so "2h ago" stays "2h ago" from one load to the next.
import type {
  AppSettings, BackupStatus, Card, CardAsk, Channel, Check, Company, Employee, EmployeeStatus, HireProposal, HumanRequest,
  OriginalSkill, OwnSkill, Person, Run, RunBuild, RunEvent, RunKind, Schedule, Score, SetupStatus, SkillHealth, SkillInfo,
  SkillRevision, StagedImport, UsageLimits, Watch,
} from "../api/types";

// ---------------------------------------------------------------- time

const MIN = 60_000;
export const NOW = Date.now();
export const iso = (ms: number) => new Date(ms).toISOString();
/** `minutes` before the page loaded. */
const ago = (minutes: number) => iso(NOW - minutes * MIN);
/** `minutes` after the page loaded. */
export const ahead = (minutes: number) => iso(NOW + minutes * MIN);
/** A local clock time `days` from today (negative: before today). */
export function onDay(days: number, h: number, m = 0): string {
  const d = new Date(NOW);
  d.setDate(d.getDate() + days);
  d.setHours(h, m, 0, 0);
  return d.toISOString();
}
/** The latest h:m that has passed: today's, else yesterday's. */
const lastAt = (h: number, m = 0) => (Date.parse(onDay(0, h, m)) <= NOW ? onDay(0, h, m) : onDay(-1, h, m));
/** The next h:m to come: today's, else tomorrow's. */
const nextAt = (h: number, m = 0) => (Date.parse(onDay(0, h, m)) > NOW ? onDay(0, h, m) : onDay(1, h, m));
/** The latest past `dow` (0 is Sunday) at h:m. */
function lastDow(dow: number, h: number, m = 0): string {
  for (let i = 0; i < 8; i++) {
    const t = onDay(-i, h, m);
    if (new Date(t).getDay() === dow && Date.parse(t) <= NOW) return t;
  }
  return onDay(-7, h, m);
}
/** The next `dow` at h:m. */
function nextDow(dow: number, h: number, m = 0): string {
  for (let i = 0; i < 8; i++) {
    const t = onDay(i, h, m);
    if (new Date(t).getDay() === dow && Date.parse(t) > NOW) return t;
  }
  return onDay(7, h, m);
}

// ---------------------------------------------------------------- the world

/** Everything the preview's Office holds. Handlers read and change it; nothing else does. */
export type World = {
  setup: SetupStatus;
  company: Company | null;
  employees: Employee[];
  people: Person[];
  status: Record<string, EmployeeStatus>;
  schedules: Schedule[];
  runs: Run[];
  events: Record<string, RunEvent[]>;
  /** Thread roots that are open: Inbox rows and conversations going on. */
  open: string[];
  /** The open roots that lead an Inbox row. */
  inbox: string[];
  /** Reports that reached you and need nothing: the Archive, beside whatever you archived. */
  archive: string[];
  /** A row's other threads, by the root that leads it. */
  related: Record<string, string[]>;
  requests: HumanRequest[];
  proposals: HireProposal[];
  catalog: SkillInfo[];
  skills: Record<string, OwnSkill[]>;
  /** Skill files by `<employee>/<skill>/<file>`. */
  files: Record<string, string>;
  revisions: Record<string, SkillRevision[]>;
  health: Record<string, SkillHealth>;
  checks: Check[];
  staged: Record<string, StagedImport[]>;
  originals: Record<string, OriginalSkill[]>;
  /** What a card's file holds, by its path. */
  cardFiles: Record<string, string>;
  companyPage: string;
  settings: AppSettings;
  usage: UsageLimits;
  backup: BackupStatus;
  update: string | null;
};

export const HOME = "/Users/maren";
const BUILD: RunBuild = { version: "0.5.1", commit: "a41c2e9", channel: "release", claude: "2.3.8", org: "Fernbrook" };

function setupStatus(company: Company | null, claude = true, signedIn = true): SetupStatus {
  return {
    claude: claude ? { found: true, path: `${HOME}/.local/bin/claude`, version: "2.3.8" } : { found: false, path: null, version: null },
    account: claude ? (signedIn ? { loggedIn: true, email: "maren@fernbrook.app", orgName: "Fernbrook", plan: "Max" } : { loggedIn: false, email: null, orgName: null, plan: null }) : null,
    company,
    background: !!company,
    build: { version: BUILD.version, commit: BUILD.commit, channel: BUILD.channel },
    sandbox: false,
  };
}

const SETTINGS: AppSettings = {
  maxConcurrentRuns: 3, claudePath: null, dataDir: `${HOME}/Library/Application Support/OmegaStar`, paused: false,
  backup: { frequency: "daily", dir: `${HOME}/Backups/Omega Star`, keep: 14 },
  watch: { hours: { startMinute: 9 * 60, endMinute: 17 * 60 + 30, days: [1, 2, 3, 4, 5] }, frequency: "normal", nudgeAfterDays: 2, giveUpAfterDays: 5 },
  proxy: { url: "", apiKey: "", managementKey: "" },
  transcriptDays: 90,
  notifications: { mode: "batched", quiet: { startMinute: 22 * 60, endMinute: 7 * 60 } },
};

const CATALOG: SkillInfo[] = [
  { id: "user:brand-voice", name: "brand-voice", description: "Fernbrook's voice: plain, warm, short sentences.", source: "user" },
  { id: "user:meeting-notes", name: "meeting-notes", description: "Turn a call transcript into decisions, owners and dates.", source: "user" },
  { id: "user:incident-runbook", name: "incident-runbook", description: "What to check first when the booking app is down or slow.", source: "user" },
  { id: "user:release-notes", name: "release-notes", description: "Write release notes studios actually read.", source: "user" },
  { id: "user:sql-reports", name: "sql-reports", description: "Read-only queries against the reporting replica.", source: "user" },
  { id: "synced:spreadsheets", name: "spreadsheets", description: "Read and write .xlsx and .csv files.", source: "synced" },
  { id: "plugin:chart-kit", name: "chart-kit", description: "Small, clean charts for reports.", source: "plugin", skills: ["bar-chart", "line-chart"] },
];

function emptyUsage(): UsageLimits {
  return {
    fiveHour: { percent: 4, resetsAt: ahead(287), resetsText: null },
    sevenDay: { percent: 9, resetsAt: nextDow(2, 10), resetsText: null },
    observedAt: ago(3), subscription: true, heldUntil: null, accountIssue: null, proxyIssue: null,
  };
}

function backupStatus(): BackupStatus {
  return {
    running: false, lastAt: lastAt(3, 0), lastFile: `${HOME}/Backups/Omega Star/omega-star-backup.zip`, lastBytes: 48_300_000,
    skipped: 0, lastAttemptAt: lastAt(3, 0), lastError: null, nextAt: nextAt(3, 0), fingerprint: "c0ffee12", checkedAt: null, leftOut: 2,
  };
}

export const emptyCompanyPage = (name: string) => `# ${name}\n\n## What we sell\n\n## What we use\n\n## How things work here\n\n## Voice and style\n`;

/** No company yet: onboarding. `claude` and `signedIn` choose how far setup has got. */
export function firstRunOffice(claude = true, signedIn = true): World {
  const w = emptyOffice();
  w.company = null;
  w.setup = setupStatus(null, claude, signedIn);
  w.companyPage = "";
  return w;
}

/** A company with nobody hired yet. */
export function emptyOffice(): World {
  const company: Company = { name: "Fernbrook", ceoName: "Maren Holt", title: "Founder", email: "maren@fernbrook.app", reportsTo: null };
  return {
    setup: setupStatus(company), company, employees: [], people: [], status: {}, schedules: [], runs: [], events: {},
    open: [], inbox: [], archive: [], related: {}, requests: [], proposals: [], catalog: CATALOG, skills: {}, files: {},
    revisions: {}, health: {}, checks: [], staged: {}, originals: {}, cardFiles: {}, companyPage: emptyCompanyPage(company.name),
    settings: structuredClone(SETTINGS), usage: emptyUsage(), backup: backupStatus(), update: null,
  };
}

// ---------------------------------------------------------------- builders

type RunOpts = Partial<Run> & {
  /** When it ended (or, live, when it started). */
  end?: string;
  /** How long it took, in minutes. */
  mins?: number;
};

export function mk(id: string, employeeId: string, kind: RunKind, prompt: string, o: RunOpts = {}): Run {
  const { end, mins = 3, ...rest } = o;
  const state = rest.state ?? "succeeded";
  const live = state === "running" || state === "queued";
  const endMs = end ? Date.parse(end) : NOW;
  const startedAt = state === "queued" ? null : iso(live ? endMs : endMs - mins * MIN);
  const createdAt = iso((startedAt ? Date.parse(startedAt) : endMs) - 6000);
  const cards = rest.cards ?? [];
  return {
    id, employeeId, kind, prompt, omegaWrote: false, state, createdAt, startedAt, endedAt: live ? null : iso(endMs),
    turns: live ? null : 6 + (id.length % 9), durationMs: live ? null : mins * MIN, summary: null, error: null, denials: [],
    skillChanges: [], cards: [], score: null, archivedAt: null, title: null, repliedAt: null, steered: false,
    deliveredAt: null, check: null, allowOnce: [], proposed: null, approvedAt: null, attachments: [], build: BUILD,
    limitRetries: 0, limited: false, contextTokens: live ? null : 21_000 + mins * 2_400, costUsd: live ? null : Math.round(mins * 9 + (id.length % 7) * 3) / 100,
    ...rest,
    needsYou: [...cards.map((c) => c.message), ...(rest.needsYou ?? [])],
  };
}

function card(message: string, ask: CardAsk, extra: Partial<Card> = {}): Card {
  return { message, why: null, key: null, file: null, ask, answer: null, link: null, images: [], backAt: null, timesBack: 0, parked: false, proposal: null, ...extra };
}

const sched = (s: Schedule, slot: string): RunKind => ({ type: "scheduled", scheduleId: s.id, slot, label: s.label, quiet: s.quiet, ...(s.review ? { review: true } : {}) });
export const init = (n: number): RunEvent => ({ type: "init", model: "claude-sonnet-4-5", skills: Array.from({ length: n }, (_, i) => `skill-${i}`) });
export const tool = (id: string, name: string, brief: string, result: string, ok = true): RunEvent[] =>
  [{ type: "toolCall", id, tool: name, brief }, { type: "toolResult", id, ok, brief: result }];
const said = (text: string): RunEvent => ({ type: "text", text });
const done = (r: Run): RunEvent => ({ type: "result", ok: r.state === "succeeded", text: r.summary ?? r.error ?? "", turns: r.turns ?? 1, durationMs: r.durationMs ?? 0 });

function schedule(id: string, employeeId: string, label: string, cron: string, instruction: string, nextFire: string | null, o: Partial<Schedule> = {}): Schedule {
  return { id, employeeId, label, cron, instruction, enabled: true, quiet: false, model: null, effort: null, activeHours: false, nextFire, ...o };
}
export const review = (employeeId: string): Schedule =>
  schedule(`${employeeId}-review`, employeeId, "Weekly review", "0 16 * * 5", "Your weekly review: how the week went against your objective, one number, and what you'd change.", nextDow(5, 16), { review: true });

function employee(id: string, name: string, title: string, o: Partial<Employee>): Employee {
  return {
    id, slug: name.toLowerCase().replace(/[^a-z]+/g, "-"), name, title, description: "", objective: "", managerId: null,
    skills: { builtins: true, keys: [] }, clearance: "supervised", allowRules: [], model: null, effort: null, browser: false,
    autoTidy: true, workspaces: [], hue: 210, chatSession: "", hiredAt: onDay(-40, 10), ...o,
  };
}

const skillDoc = (name: string, description: string, body: string) => `---\nname: ${name}\ndescription: ${description}\n---\n\n${body.trim()}\n`;
export const own = (name: string, description: string, updatedAt: string, files = ["SKILL.md"], o: Partial<OwnSkill> = {}): OwnSkill =>
  ({ name, description, role: false, managed: false, files, updatedAt, ...o });
export const CRAFT = (at: string) => own("omega-skill-craft", "How to write and tidy your own skills. Omega Star keeps this one up to date.", at, ["SKILL.md"], { managed: true });

// ---------------------------------------------------------------- Fernbrook, busy

/** A busy Thursday at Fernbrook: two employees working, one waiting to start, two needing a look, and a full Inbox. */
export function fullOffice(): World {
  const w = emptyOffice();
  w.update = "0.6.0";
  w.company = { name: "Fernbrook", ceoName: "Maren Holt", title: "Founder", email: "maren@fernbrook.app", reportsTo: "harriet" };
  w.setup = setupStatus(w.company);
  w.usage = {
    fiveHour: { percent: 38, resetsAt: ahead(134), resetsText: null },
    sevenDay: { percent: 61, resetsAt: nextDow(2, 10), resetsText: null },
    observedAt: ago(4), subscription: true, heldUntil: null, accountIssue: null, proxyIssue: null,
  };

  // ---- people
  const ch = (id: string, kind: Channel["kind"], address: string, whenToUse: string): Channel => ({ id, kind, address, whenToUse });
  w.people = [
    {
      id: "harriet", name: "Harriet Cole", title: "Chair", hue: 4, addedAt: onDay(-60, 11),
      description: "Chairs the board and backs Fernbrook. Wants a short update before each board meeting.",
      strengths: "Pricing, fundraising, hiring", responsibilities: "Board meetings and investor updates",
      channels: [ch("h-mail", "email", "harriet@colehollow.vc", "The monthly update and board papers"), ch("h-phone", "phone", "+61 491 570 156", "Only if it can't wait a day")],
      notes: "Prefers one email a month to many small ones.", manager: { type: "top" },
    },
    {
      id: "kofi", name: "Kofi Mensah", title: "Product Designer", hue: 192, addedAt: onDay(-55, 9),
      description: "Designs the booking app and the marketing site. Contract, three days a week.",
      strengths: "Interface design, illustration, help-centre screenshots", responsibilities: "App design and the marketing site",
      channels: [ch("k-slack", "slack", "@kofi", "Quick questions, screenshots, design reviews"), ch("k-mail", "email", "kofi@mensah.studio", "Anything with a deadline more than a week out")],
      notes: "Works Tuesday to Thursday.", manager: { type: "you" },
    },
    {
      id: "lena", name: "Lena Fischbach", title: "Accountant", hue: 104, addedAt: onDay(-48, 14),
      description: "Does Fernbrook's tax and year-end accounts, at a small outside firm.",
      strengths: "Tax codes, payroll, year-end", responsibilities: "Quarterly tax returns and the year-end accounts",
      channels: [ch("l-teams", "teams", "lena.fischbach@fischbach-partners.com.au", "Quick questions in business hours"), ch("l-mail", "email", "lena.fischbach@fischbach-partners.com.au", "Documents and anything formal")],
      notes: "", manager: { type: "you" },
    },
  ];

  // ---- employees
  w.employees = [
    employee("juniper", "Juniper Hale", "Chief of Staff", {
      description: "Keeps the week on track: plans, follow-ups and the Monday report.",
      objective: "Make sure nothing important slips: every open loop has an owner, a date and a next step.",
      skills: { builtins: true, keys: ["user:meeting-notes", "user:brand-voice"] }, hue: 212,
      allowRules: ["Read(~/Fernbrook/**)", "Bash(git log:*)"], workspaces: ["~/Fernbrook/ops"], hiredAt: onDay(-58, 10),
    }),
    employee("otto", "Otto Lindqvist", "Cloud Engineer", {
      description: "Runs the servers, the database and the deploys.",
      objective: "Keep Fernbrook up, fast and cheap to run, and say so before anything expires.",
      managerId: "juniper", skills: { builtins: true, keys: ["user:incident-runbook", "user:sql-reports"] }, hue: 18, effort: "high",
      allowRules: ["Bash(kubectl get:*)", "Bash(terraform plan:*)", "Read(~/Fernbrook/infra/**)"], workspaces: ["~/Fernbrook/infra"], hiredAt: onDay(-51, 9),
    }),
    employee("rafi", "Rafi Mendel", "QA Engineer", {
      description: "Tests every release and keeps the test suite honest.",
      objective: "Catch booking bugs before a studio does.",
      managerId: "otto", skills: { builtins: true, keys: ["user:release-notes"] }, hue: 160,
      allowRules: ["Bash(npm test:*)", "Bash(npx playwright test:*)"], workspaces: ["~/Fernbrook/web"], hiredAt: onDay(-30, 9),
    }),
    employee("mae", "Mae Okonkwo", "Support Lead", {
      description: "Answers studios, spots patterns in tickets and keeps the help centre current.",
      objective: "Every studio gets a clear answer within a day, and nobody has to ask the same question twice.",
      managerId: "juniper", skills: { builtins: true, keys: ["user:brand-voice", "synced:spreadsheets"] }, hue: 335, browser: true,
      allowRules: ["Edit(~/Fernbrook/help-centre/**)"], workspaces: ["~/Fernbrook/help-centre"], hiredAt: onDay(-45, 9),
    }),
    employee("theo", "Theo Marchetti", "Growth Marketer", {
      description: "Writes the newsletter, runs the blog and watches trial signups.",
      objective: "Grow trial signups from studios without discounting.",
      skills: { builtins: true, keys: ["user:brand-voice", "plugin:chart-kit"] }, hue: 46,
      workspaces: ["~/Fernbrook/marketing"], hiredAt: onDay(-38, 9),
    }),
    employee("ines", "Ines Duarte", "Bookkeeper", {
      description: "Reconciles payouts, chases receipts and keeps the books ready for Lena.",
      objective: "Close every month within five working days, with nothing for Lena to chase.",
      managerId: "juniper", skills: { builtins: true, keys: ["synced:spreadsheets"] }, hue: 282, effort: "medium",
      workspaces: ["~/Fernbrook/finance"], hiredAt: onDay(-21, 9),
    }),
  ];

  // ---- schedules
  const S = {
    monday: schedule("j-monday", "juniper", "Monday report", "0 8 * * 1", "Write the Monday report: one status line, then what moved, what's stuck and what needs Maren.", nextDow(1, 8)),
    certs: schedule("o-certs", "otto", "Certificate and uptime check", "0 7 * * *", "Check every certificate and domain expiry date, and the last 24 hours of uptime.", nextAt(7), { quiet: true }),
    backups: schedule("o-backups", "otto", "Nightly backup check", "30 2 * * *", "Restore last night's database backup on the test server and check it.", nextAt(2, 30), { quiet: true }),
    regress: schedule("r-regress", "rafi", "Nightly regression sweep", "0 1 * * *", "Run the full test suite against staging and sort failures into real and flaky.", nextAt(1), { quiet: true }),
    triage: schedule("m-triage", "mae", "Morning ticket triage", "0 9 * * 1-5", "Read the overnight tickets, answer what you can and pass bugs to Rafi.", nextAt(9), { activeHours: true }),
    summary: schedule("m-summary", "mae", "Weekly support summary", "0 15 * * 5", "Summarise the week's tickets: volume, first-reply time, and what studios kept asking.", nextDow(5, 15)),
    news: schedule("t-news", "theo", "Newsletter draft", "0 10 * * 2", "Draft this month's newsletter and the two questions Maren needs to answer about it.", nextDow(2, 10)),
    ideas: schedule("t-ideas", "theo", "Blog ideas", "0 11 1 * *", "Five blog ideas from last month's tickets and searches.", null, { enabled: false }),
    recon: schedule("i-recon", "ines", "Reconcile payouts", "0 6 * * *", "Match yesterday's card payouts against the bank export and flag anything that doesn't match.", nextAt(6)),
    close: schedule("i-close", "ines", "Month-end close", "0 9 2 * *", "Close last month: reconcile, chase missing receipts, and send Lena the pack.", onDay(27, 9)),
  };
  w.schedules = [...Object.values(S), ...w.employees.map((e) => review(e.id))];

  const runs: Run[] = [];
  const add = (r: Run, events: RunEvent[] = []) => {
    runs.push(r);
    w.events[r.id] = events.length || r.state === "queued" ? events : [init(9), ...(r.summary ? [said(r.summary)] : []), done(r)];
    return r;
  };

  // ---- Juniper: the March price change, a conversation with a turn streaming now
  const planAnswer = [
    "Here's the plan. One rule I've written down for next time: studios always get 30 days' notice, so the email goes out by **30 January**.",
    "",
    "**Who needs to hear**",
    "",
    "| Who | What | When |",
    "|---|---|---|",
    "| Studio customers (212) | An email with the new price and the date | 30 Jan |",
    "| Harriet | One line in the January board update | 24 Jan |",
    "| Mae | Help-centre changes and a reply template | 27 Jan |",
    "",
    "**What changes in the app**",
    "",
    "- The new price in billing, with annual plans kept at the old price until renewal",
    "- The pricing page and the upgrade screen",
    "",
    "**What could go wrong**",
    "",
    "1. Annual customers read it as a rise for them. The email should say plainly that it isn't.",
    "2. Three help articles quote $29. Mae can update them the same day.",
    "",
    "Shall I draft the email and check the billing side with Otto?",
  ].join("\n");
  const plan = add(mk("plan-1", "juniper", { type: "task", label: "March price change plan", from: null, context: "" },
    "We're moving the Studio plan from $29 to $34 a month on 1 March. Can you plan it? Who needs to hear, what changes in the app, and what could go wrong. Studios always get 30 days' notice of a price change.",
    { end: ago(31), mins: 4, title: "March price change plan", summary: planAnswer, skillChanges: [{ skill: "price-changes", file: "SKILL.md", change: "added" }] }));
  w.events[plan.id] = [
    init(11), said("I'll check the last price change and who it touched, then plan this one."),
    ...tool("p1", "Skill", "planning", "Loaded"),
    ...tool("p2", "Read", "notes/pricing-history.md", "48 lines"),
    ...tool("p3", "Grep", "\"$29\" in help-centre/", "3 files"),
    ...tool("p4", "mcp__omega__people", "who owns billing and the help centre", "Otto, Mae"),
    ...tool("p5", "Write", "skills/price-changes/SKILL.md", "Created"),
    { type: "skillChanged", skill: "price-changes", file: "SKILL.md", change: "added" },
    said(planAnswer), done(plan),
  ];
  const planLive = add(mk("plan-2", "juniper", { type: "reply", toRunId: "plan-1", label: "March price change plan", task: true, card: false },
    "Yes, go ahead. Draft the email and check the billing side with Otto.", { state: "running", end: ago(3) }));
  w.events[planLive.id] = [
    init(11), said("On it. I'll ask Otto about the billing side while I draft the email."),
    ...tool("q1", "mcp__omega__delegate", "Otto: which price IDs change on 1 March, and does anything hard-code $29?", "Delegated to Otto"),
    { type: "delegated", childRunId: "billing-1", employeeId: "otto", task: "Check which price IDs change on 1 March and whether anything in the app hard-codes $29. Report back by the end of the day." },
    { type: "toolCall", id: "q2", tool: "Agent", brief: "Find every help-centre article that quotes the Studio price" },
    { type: "taskStarted", taskId: "task-help", toolUseId: "q2", name: "Find help-centre articles that quote $29" },
    { type: "toolResult", id: "q2", ok: true, brief: "Launched" },
    { type: "taskProgress", taskId: "task-help", activity: "Reading billing/annual-plans.md", lastTool: "Read", toolUses: 7, tokens: 18_400, durationMs: 64_000 },
    ...tool("q3", "Read", "templates/customer-email.md", "62 lines"),
    ...tool("q4", "mcp__omega__show", "Rollout timeline", "Shown"),
    { type: "visual", id: "timeline", title: "Rollout timeline" },
  ];
  const billing = add(mk("billing-1", "otto", { type: "delegated", parentRunId: "plan-2" },
    "Check which price IDs change on 1 March and whether anything in the app hard-codes $29. Report back by the end of the day.", { state: "running", end: ago(2) }));
  w.events[billing.id] = [
    init(8), said("Starting with the billing module and the pricing page."),
    ...tool("b1", "Bash", "grep -rn \"29\" web/src/billing", "4 matches"),
    ...tool("b2", "Read", "web/src/billing/prices.ts", "91 lines"),
    { type: "toolCall", id: "b3", tool: "Bash", brief: "terraform plan -target=module.billing" },
  ];

  // ---- Otto: queued, a card from a quiet routine, a conversation he answered, quiet nights
  add(mk("certs-now", "otto", sched(S.certs, lastAt(7)), S.certs.instruction, { state: "queued", end: ago(6) }));
  const pgSummary = "Backups ran and restored cleanly on the test server (3.1 GB, 4 minutes).\n\nOne thing for you: staging still runs Postgres 14, which stops getting security fixes next week. Production is already on 16.";
  add(mk("pg-1", "otto", sched(S.backups, lastAt(2, 30)), S.backups.instruction, {
    end: lastAt(2, 41), mins: 11, summary: pgSummary,
    cards: [card("Upgrade the staging database to Postgres 16 this weekend?", { type: "choices", choices: ["Upgrade Saturday night", "Wait until after the price change"], recommended: 0 }, {
      why: "Postgres 14 stops getting security fixes next week. Staging would be down for about 20 minutes.", key: "ask:staging-db:upgrade",
      link: "https://code.fernbrook.app/fernbrook/infra/pulls/88",
    })],
  }), [init(8), ...tool("g1", "Bash", "pg_restore --list nightly.dump", "Restored to test-db"), ...tool("g2", "Bash", "psql -c 'select version()'", "PostgreSQL 14.13"), said(pgSummary)]);
  const slowAnswer = "It was the database, not the app. Tuesday 6 to 7 pm is the busiest hour of the week (studios open the next week's classes), and one query read every past booking.\n\n- Added an index on `bookings.starts_at`: that query went from 1.9 s to 40 ms\n- The booking page's median is back to 310 ms\n\nNothing for you to do. I've set a check to look again next Tuesday evening.";
  const slow = add(mk("slow-1", "otto", { type: "task", label: "Why was the site slow on Tuesday?", from: null, context: "" },
    "The booking page felt slow on Tuesday evening. What happened?", { end: ago(118), mins: 9, title: "Slow booking page on Tuesday", summary: slowAnswer }));
  w.events[slow.id] = [init(8), said("Looking at Tuesday evening's traces."), ...tool("s1", "Bash", "kubectl logs deploy/web --since=48h | grep slow", "212 slow requests"), ...tool("s2", "Bash", "psql -f explain-bookings.sql", "Seq scan on bookings"), ...tool("s3", "Edit", "infra/migrations/0142_bookings_starts_at.sql", "Added"), said(slowAnswer), done(slow)];
  for (let d = 1; d <= 4; d++) {
    add(mk(`backups-${d}`, "otto", sched(S.backups, onDay(-d, 2, 30)), S.backups.instruction, { end: onDay(-d, 2, 39), mins: 9, summary: "Backups restored cleanly. Nothing to report." }));
  }
  add(mk("certs-1", "otto", sched(S.certs, onDay(-1, 7)), S.certs.instruction, { end: onDay(-1, 7, 3), mins: 3, summary: "All 6 certificates are valid for 30 days or more. Uptime 100% over the last 24 hours." }));
  add(mk("deploy-1", "otto", { type: "manual" }, "Check staging after this morning's deploy.", {
    state: "failed", end: ago(96), mins: 1, error: "Network error: couldn't reach staging.fernbrook.app (timed out after 30s)",
  }));
  add(mk("organize-1", "otto", { type: "organize", imported: [{ id: "user:incident-runbook", name: "incident-runbook" }, { id: "user:sql-reports", name: "sql-reports" }], settled: false },
    "Organize the skills Maren imported for you.", { end: onDay(-4, 11, 20), mins: 6, summary: "Split incident-runbook into first-checks and paging, and kept sql-reports as it was.",
    skillChanges: [{ skill: "first-checks", file: "SKILL.md", change: "added" }, { skill: "paging", file: "SKILL.md", change: "added" }] }));

  // ---- Rafi: waiting to start, a list to tick, a failure like Otto's, an answer for Mae
  add(mk("widget-2", "rafi", { type: "manual" }, "Check the booking widget on the new Safari release.\nScreenshots of anything that looks off.", { state: "queued", end: ago(9) }));
  const flakySummary = "412 tests, 405 passed. The other 7 are flaky: they pass on a second try.\n\nThe booking flow itself is fine.";
  add(mk("flaky-1", "rafi", sched(S.regress, lastAt(1)), S.regress.instruction, {
    end: lastAt(1, 38), mins: 38, summary: flakySummary,
    cards: [card("Quarantine these flaky tests until I fix them?", {
      type: "items", verb: "Quarantine", items: [
        { id: "t1", text: "booking/waitlist-promotes-next-in-line", about: "Fails when two people cancel within a second", recommended: true },
        { id: "t2", text: "calendar/drag-across-midnight", about: "A time-zone edge case on the test machines", recommended: true },
        { id: "t3", text: "payments/partial-refund", about: "Waits on a slow test card service", recommended: false },
        { id: "t4", text: "reminders/sms-24h-before", about: "Clock drift on the test machines", recommended: true },
      ],
    }, { why: "They fail about one run in five and hide real failures.", link: "https://ci.fernbrook.app/runs/2291" })],
  }));
  add(mk("widget-1", "rafi", { type: "manual" }, "Check the booking widget on staging.", {
    state: "failed", end: ago(82), mins: 1, error: "Network error: couldn't reach staging.fernbrook.app (timed out after 30s)",
  }));
  const askAnswer = "Reproduced it: two bookings 300 ms apart both pass the capacity check, so the last spot goes twice. The fix is a row lock in `book_slot()`. I've written it up for Otto.";
  add(mk("ask-1", "rafi", { type: "asked", parentRunId: "triage-1", askedBy: "mae", task: false },
    "Can you reproduce the double booking from ticket #4127? A studio says two people got the last spot in the 6 pm class.", { end: ago(64), mins: 14, summary: askAnswer }));

  // ---- Mae: a question, news, a proposal for the company page, a conversation done
  const triageSummary = "14 new tickets overnight. 11 answered, 2 passed to Rafi as bugs, 1 needs you.\n\n- Most asked: how to move a class without emailing everyone (4)\n- Two studios asked about paying yearly";
  add(mk("triage-1", "mae", sched(S.triage, lastAt(9)), S.triage.instruction, {
    end: ago(150), mins: 22, summary: triageSummary,
    cards: [card("Can Riverbend Pilates pay for a year by bank transfer?", { type: "question" }, { why: "They want one invoice for the year. We've only ever taken cards.", key: "ask:annual-bank-transfer" })],
  }), [
    init(10), ...tool("m1", "mcp__helpdesk__list_tickets", "status:new since yesterday", "14 tickets"),
    ...tool("m2", "mcp__omega__ask_colleague", "Rafi: reproduce the double booking from #4127", "Asked Rafi"),
    { type: "asked", childRunId: "ask-1", employeeId: "rafi", question: "Can you reproduce the double booking from ticket #4127?", task: false },
    ...tool("m3", "mcp__omega__request", "Kofi: new booking-widget screenshot", "Sent on Slack"),
    { type: "requested", requestId: "req-kofi", personId: "kofi", kind: "question", message: "Could you send the new booking-widget screenshot for the help centre? The article still shows the old button." },
    said(triageSummary),
  ]);
  add(mk("summary-1", "mae", sched(S.summary, lastDow(5, 15)), S.summary.instruction, {
    end: lastDow(5, 15, 14), mins: 14, summary: "61 tickets this week, 9 fewer than last week. Median first reply 2h 10m.\n\nDouble-booking tickets: 4, down from 9.",
    cards: [card("Double-booking tickets are down by half since the waitlist fix.", { type: "news" })],
  }));
  const refundRoot = add(mk("refund-1", "mae", { type: "task", label: "Refund policy wording", from: null, context: "" },
    "Can you rewrite the refund policy on the help centre? It's long and reads like a contract.", { end: onDay(-1, 15, 12), mins: 7, title: "Refund policy wording",
    summary: "Here's a shorter version. Two paragraphs instead of seven, and it says what happens before the fine print:\n\n> If a class is cancelled, you get your credit back straight away. For anything else, reply to your booking email within 14 days and we'll sort it out.\n\nIt's live as a draft on the help centre. Want me to publish it?" }));
  w.events[refundRoot.id] = [init(9), ...tool("f1", "Read", "help-centre/policies/refunds.md", "71 lines"), ...tool("f2", "Write", "help-centre/drafts/refunds.md", "Saved"), said(refundRoot.summary ?? ""), done(refundRoot)];
  add(mk("refund-2", "mae", { type: "reply", toRunId: "refund-1", label: "Refund policy wording", task: true, card: false },
    "Publish it. And you can approve refunds under $50 yourself from now on.", { end: ago(40), mins: 3,
    summary: "Published. I've written the $50 rule into my refunds skill.\n\nOne question, since it affects everyone who talks to studios:",
    skillChanges: [{ skill: "refunds", file: "SKILL.md", change: "modified" }],
    cards: [card("Add this to the company page so everyone follows it?", { type: "choices", choices: ["Add it", "Keep it to support"], recommended: 0 }, {
      key: "ask:refund-rule", proposal: { type: "companyLine", section: "How things work here", line: "- Refunds under $50 are approved without asking. Anything bigger goes to Maren." },
    })],
  }), [init(9), ...tool("f3", "Bash", "help-centre publish drafts/refunds.md", "Published"), ...tool("f4", "Edit", "skills/refunds/SKILL.md", "1 rule added"), { type: "skillChanged", skill: "refunds", file: "SKILL.md", change: "modified" }, said("Published. I've written the $50 rule into my refunds skill.\n\nOne question, since it affects everyone who talks to studios:")]);
  const welcome = add(mk("welcome-1", "mae", { type: "task", label: "Welcome email for new studios", from: null, context: "" },
    "Write a welcome email for studios that just signed up. Short, and it should get them to add their first class.", { end: onDay(-2, 11, 5), mins: 5, title: "Welcome email for new studios",
    summary: "Done. Three short paragraphs and one button, **Add your first class**. It's in the onboarding sequence from today.", archivedAt: onDay(-2, 11, 30) }));
  add(mk("welcome-2", "mae", { type: "reply", toRunId: welcome.id, label: "Welcome email for new studios", task: true, card: false }, "Perfect, thanks.", { end: onDay(-2, 11, 20), mins: 1, summary: "Thanks! I'll watch how many studios add a class in their first day." }));
  add(mk("followup-1", "mae", { type: "followUp", requestId: "req-kofi-old", personId: "kofi" }, "Kofi replied to your request.", {
    end: onDay(-2, 14, 40), mins: 4, summary: "Kofi sent the new screenshots. Updated 3 help articles.", omegaWrote: true,
  }));

  // ---- Theo: blocked, two newsletter questions, a pair of pricing-page decisions, a chore, a hire proposal
  const mailSummary = "I wrote the campaign and picked the Studio customers segment (212 studios), but I couldn't create the draft: I'm not allowed to use the newsletter tool yet.";
  add(mk("mail-1", "theo", { type: "manual" }, "Set up the March price-change email as a draft campaign so it's ready to schedule.", {
    end: ago(48), mins: 6, summary: mailSummary,
    denials: [{ tool: "mcp__newsletter__create_campaign", brief: "Create the draft campaign \"Your Studio plan from 1 March\"", suggestedRule: "mcp__newsletter__create_campaign" }],
  }), [init(9), ...tool("n1", "Read", "marketing/segments.md", "4 segments"), ...tool("n2", "mcp__newsletter__create_campaign", "Your Studio plan from 1 March", "Blocked: not allowed", false), said(mailSummary)]);
  const draftPath = `${HOME}/Fernbrook/marketing/drafts/november-newsletter.md`;
  w.cardFiles[draftPath] = "# The waitlists are here\n\nHi {first name},\n\nWhen a class fills up, people can now join a waitlist. If someone cancels, the next person gets the spot and a text, without you lifting a finger.\n\nStudios that tried it in October filled **31 more spots** on average.\n\n[Turn on waitlists](https://app.fernbrook.app/settings/waitlists)\n\n## Also this month\n\n- Move a class without emailing everyone: Fernbrook tells only the people booked\n- SMS reminders now go out 24 hours before, not 2\n\nThanks for running your studio with us,\nThe Fernbrook team";
  const newsSummary = "The November newsletter is drafted. Two questions before it goes:";
  add(mk("news-1", "theo", sched(S.news, lastDow(2, 10)), S.news.instruction, {
    end: lastDow(2, 10, 26), mins: 26, summary: newsSummary,
    cards: [
      card("Lead with the waitlists or with the price change?", { type: "choices", choices: ["Waitlists first", "Price change first"], recommended: 0 }, {
        why: "Leading with something new usually gets more clicks; the price change gets its own email in January.", key: "ask:newsletter:lead", file: draftPath,
        images: [`${HOME}/Fernbrook/marketing/hero-waitlist.png`, `${HOME}/Fernbrook/marketing/hero-reminders.png`],
      }),
      card("Send it Tuesday or Thursday morning?", { type: "choices", choices: ["Tuesday 7 am", "Thursday 7 am"], recommended: 1 }, {
        why: "Thursday sends got 9% more opens over the last six issues.", key: "ask:newsletter:day",
      }),
    ],
  }), [
    init(10), ...tool("w1", "Read", "marketing/newsletter/october.md", "40 lines"), ...tool("w2", "Write", "marketing/drafts/november-newsletter.md", "Saved"),
    ...tool("w3", "mcp__omega__propose_hire", "Noor Haddad, Content Writer", "Proposed"),
    { type: "proposedHire", proposalId: "prop-noor", name: "Noor Haddad", title: "Content Writer" },
    said(newsSummary),
  ]);
  add(mk("pp-1", "theo", { type: "manual" }, "Rework the pricing page for the new price.", {
    end: ago(185), mins: 12, summary: "The pricing page is ready on staging with the new price. One choice for you.",
    cards: [card("Headline: \"Simple pricing for busy studios\" or \"One plan. Everything included.\"?", { type: "choices", choices: ["Simple pricing for busy studios", "One plan. Everything included."], recommended: 1 }, { key: "ask:pricing-page:headline" })],
  }));
  add(mk("pp-2", "theo", { type: "manual" }, "Write the pricing FAQ.", {
    end: ago(170), mins: 8, summary: "Eight questions, most of them from Mae's tickets.",
    cards: [card("Keep \"Can I pay yearly?\" now that yearly is by request?", { type: "choices", choices: ["Keep it", "Take it out"], recommended: 0 }, { key: "ask:pricing-page:faq", why: "Two studios asked this week." })],
  }));
  add(mk("stats-1", "theo", { type: "manual" }, "Pull October's signup numbers for the board update.", {
    end: ago(410), mins: 2, summary: "I need you to sign in first; the analytics session ran out.",
    cards: [card("Sign in to the analytics dashboard so I can pull October's numbers", { type: "chore", subject: "sign-in:analytics.fernbrook.app", url: "https://analytics.fernbrook.app/login" }, { key: "chore:sign-in:analytics.fernbrook.app" })],
  }));
  add(mk("referral-1", "theo", { type: "manual" }, "Ideas to grow signups without discounting.", {
    end: onDay(-9, 11, 10), mins: 9, summary: "Five ideas. The referral one needs your call.",
    cards: [card("Try a free month for studios that bring another studio?", { type: "choices", choices: ["Try it for a month", "Wait until after March"], recommended: 0 }, { parked: true, timesBack: 1, backAt: onDay(-2, 9) })],
  }));

  // ---- Ines: three failures in a row, a task only you can do, a reply she's waiting on
  const reconError = "Couldn't find yesterday's payouts export in ~/Fernbrook/finance/exports. The bank's nightly export may not have run.";
  const today6 = Date.parse(onDay(0, 6)) <= NOW ? 0 : -1;
  for (let d = 0; d < 3; d++) {
    const r = mk(`recon-${3 - d}`, "ines", sched(S.recon, onDay(today6 - d, 6)), S.recon.instruction, { state: "failed", end: onDay(today6 - d, 6, 1), mins: 1, error: reconError });
    add(r, [init(6), ...tool(`x${d}`, "Bash", "ls ~/Fernbrook/finance/exports", "Empty folder", false), done(r)]);
  }
  add(mk("close-1", "ines", sched(S.close, onDay(-3, 9)), S.close.instruction, {
    end: onDay(-3, 9, 48), mins: 48, summary: "October is closed apart from one thing only you can do. Everything else is in the pack for Lena.",
    cards: [card("Pay Lena's October invoice", { type: "task", steps: "Pay $1,180 to Fischbach & Partners from the business account, reference INV-2291. It's due on Friday.", url: null })],
  }));
  add(mk("nudge-1", "ines", { type: "nudge", requestId: "req-lena", personId: "lena" }, "Nudge Lena about the annual-plan question.", {
    end: ago(185), mins: 1, summary: "Sent Lena a short reminder on Teams.", omegaWrote: true,
  }));
  add(mk("check-1", "ines", { type: "replyCheck", requestId: "req-lena", personId: "lena" }, "Check whether Lena replied.", {
    end: ago(26), mins: 1, summary: "No reply yet.", omegaWrote: true,
  }));

  // ---- Juniper: a tidy waiting on your OK, a board update put off till tomorrow, last Monday's report
  add(mk("tidy-1", "juniper", { type: "tidy", reason: "auto", signature: ["looseNotes", "repeatedEntries"], plan: null }, "Tidy your skills and notes.", {
    end: ago(205), mins: 5, summary: "Merged two rules about the Monday report that said the same thing, and moved 3 loose notes into skills. Two bigger changes need your OK.",
    proposed: "Split **weekly-report** in two:\n\n- **weekly-report** keeps the format and the status line\n- **board-update** gets the investor version, which has grown rules of its own\n\nAnd retire **old-launch-checklist**, which nothing has used since the spring launch.",
    needsYou: ["Approve Juniper's skill changes?"], skillChanges: [{ skill: "weekly-report", file: "SKILL.md", change: "modified" }],
  }));
  add(mk("board-1", "juniper", { type: "manual" }, "Draft the January board update.", {
    end: onDay(-1, 16, 20), mins: 14, summary: "The board update is drafted: one page, numbers first.",
    cards: [card("Put the churn numbers in the update, or keep them for the meeting?", { type: "choices", choices: ["Put them in", "Keep them for the meeting"], recommended: 1 }, { why: "Churn rose in October for a reason we've fixed; it reads better with the story." })],
  }));
  add(mk("monday-1", "juniper", sched(S.monday, lastDow(1, 8)), S.monday.instruction, {
    end: lastDow(1, 8, 9), mins: 9,
    summary: "**On track.** One open item: the staging database upgrade (Otto).\n\n- Moved: waitlists shipped to every studio; 9 fewer support tickets than the week before\n- Stuck: the bank's payouts export (Ines is on it)\n- Needs Maren: nothing this week",
  }));
  add(mk("hired-ines", "juniper", { type: "hireDecided", proposalId: "prop-ines", approved: true }, "Maren approved your proposal to hire Ines Duarte.", {
    end: onDay(-21, 9, 30), mins: 2, omegaWrote: true, summary: "Introduced myself to Ines and handed over the receipts backlog.",
  }));

  // ---- weekly reviews: everyone's last Friday, one row in the Archive
  const scores: [string, Score, string][] = [
    ["juniper", { metric: "Open loops", value: "4", change: "-3", note: "Three closed: the waitlist launch, the DNS move, the bank form." }, "A good week. The waitlist launch closed out, and every open loop has a date."],
    ["otto", { metric: "Uptime", value: "99.98%", change: "+0.02", note: "Eight minutes down during the DNS move." }, "Quiet week apart from the DNS move, which went to plan."],
    ["rafi", { metric: "Flaky tests", value: "7", change: "-2", note: "Fixed two; seven to go." }, "Fixed two flaky tests; the suite is more honest than it was."],
    ["mae", { metric: "Answered within a day", value: "92%", change: "+6", note: "The new FAQ answers the most common question." }, "Fewer tickets and faster replies."],
    ["theo", { metric: "Trial signups", value: "214", change: "+31", note: "The waitlist post did most of it." }, "Best week for signups since launch."],
    ["ines", { metric: "Payouts reconciled", value: "3 of 5", change: "-2", note: "The bank export stopped on Wednesday." }, "Two days I couldn't reconcile; the bank export is missing."],
  ];
  for (const [emp, score, summary] of scores) {
    const s = w.schedules.find((x) => x.employeeId === emp && x.review)!;
    add(mk(`review-${emp}`, emp, sched(s, lastDow(5, 16)), s.instruction, { end: lastDow(5, 16, 7 + emp.length), mins: 7, score, summary }));
  }

  w.runs = runs.sort((a, b) => b.createdAt.localeCompare(a.createdAt));

  // ---- where each thread is
  w.open = ["plan-1", "slow-1", "refund-1", "pg-1", "flaky-1", "triage-1", "summary-1", "mail-1", "news-1", "pp-1", "pp-2", "stats-1", "recon-3", "close-1", "tidy-1", "board-1", "deploy-1", "widget-1"];
  w.inbox = ["pg-1", "flaky-1", "triage-1", "summary-1", "mail-1", "news-1", "pp-1", "pp-2", "stats-1", "recon-3", "close-1", "tidy-1", "board-1", "deploy-1", "widget-1"];
  w.related = { "recon-3": ["recon-2", "recon-1"], "review-juniper": ["review-otto", "review-rafi", "review-mae", "review-theo", "review-ines"] };
  w.archive = ["monday-1", "review-juniper", "certs-1", "welcome-1", "followup-1", "hired-ines"];

  // ---- people and requests
  const watching = (o: Partial<Watch> = {}): Watch => ({
    phase: { type: "watching" }, resumedAt: null, lastCheckedAt: ago(20), failures: 0, lastError: null, seen: null, heard: null, nudgedAt: null,
    plan: { nextCheckAt: ahead(40), nudgeAt: onDay(2, 10), giveUpAt: onDay(5, 17) }, ...o,
  });
  w.requests = [
    {
      id: "req-kofi", personId: "kofi", employeeId: "mae", runId: "triage-1", kind: "question", channelId: "k-slack", state: "open", response: null,
      message: "Could you send the new booking-widget screenshot for the help centre? The article still shows the old button.",
      dueAt: onDay(1, 17), createdAt: ago(146), resolvedAt: null,
      delivery: { locator: { type: "other", text: "#design, thread with Kofi" }, senderId: null, sentAt: ago(146), via: "team-chat MCP", source: "employee", watch: watching() },
    },
    {
      id: "req-lena", personId: "lena", employeeId: "ines", runId: "close-1", kind: "question", channelId: "l-teams", state: "open", response: null,
      message: "Should payments for the new annual plan go to income received in advance, or straight to sales?",
      dueAt: onDay(1, 12), createdAt: onDay(-2, 10, 15), resolvedAt: null,
      delivery: {
        locator: { type: "teamsChat", chatId: "19:fernbrook-lena", messageId: "1730700000000", server: "ms365" }, senderId: null, sentAt: onDay(-2, 10, 15), via: "ms365 MCP", source: "employee",
        watch: watching({ nudgedAt: ago(185), lastCheckedAt: ago(26), plan: { nextCheckAt: ahead(34), nudgeAt: null, giveUpAt: onDay(3, 17) } }),
      },
    },
    {
      id: "req-kofi-old", personId: "kofi", employeeId: "theo", runId: null, kind: "task", channelId: "k-slack", state: "answered",
      message: "Could you export the new hero image in a wide and a square crop for the newsletter?",
      response: "Uploaded to the shared drive: hero-wide.png and hero-square.png.", dueAt: null, createdAt: onDay(-3, 10), resolvedAt: onDay(-2, 14, 30),
      delivery: { locator: { type: "other", text: "Direct message" }, senderId: null, sentAt: onDay(-3, 10), via: "team-chat MCP", source: "employee", watch: { ...watching(), phase: { type: "stopped", at: onDay(-2, 14, 30) }, plan: null } },
    },
    {
      id: "req-harriet", personId: "harriet", employeeId: "juniper", runId: "plan-1", kind: "info", channelId: "h-mail", state: "done", response: null,
      message: "For your notes: the Studio plan moves to $34 a month on 1 March. Studios hear on 30 January.",
      dueAt: null, createdAt: ago(29), resolvedAt: ago(29),
      delivery: { locator: { type: "other", text: "Email to harriet@colehollow.vc" }, senderId: null, sentAt: ago(29), via: "mail MCP", source: "employee", watch: null },
    },
  ];

  // ---- hire proposals
  const noorDraft = {
    name: "Noor Haddad", title: "Content Writer",
    description: "Writes the blog, help articles and the newsletter's longer pieces.",
    objective: "Publish one useful post a week that studios find through search.",
    managerId: "theo", skills: { builtins: true, keys: ["user:brand-voice"] }, clearance: "supervised" as const, allowRules: [],
    model: null, effort: null, browser: false, autoTidy: true, workspaces: ["~/Fernbrook/marketing/blog"], hue: 24,
  };
  w.proposals = [
    {
      id: "prop-noor", employeeId: "theo", runId: "news-1", draft: noorDraft, missingSkills: ["seo-briefs"], state: "open", note: null, hiredId: null,
      reason: "Most of my runs go on writing help articles and blog posts, so the newsletter and the signup work wait. A writer would take the posts and I'd keep the numbers.",
      createdAt: lastDow(2, 10, 24), resolvedAt: null,
    },
    {
      id: "prop-ines", employeeId: "juniper", runId: null, draft: { ...noorDraft, name: "Ines Duarte", title: "Bookkeeper", managerId: "juniper", description: "Reconciles payouts and chases receipts.", objective: "Close every month within five working days." },
      missingSkills: [], state: "approved", note: "Yes, and give her the receipts backlog first.", hiredId: "ines",
      reason: "Receipts and reconciliations take a day of my week.", createdAt: onDay(-22, 15), resolvedAt: onDay(-21, 9),
    },
  ];

  // ---- how each is doing
  const R = (id: string) => w.runs.find((r) => r.id === id)!;
  w.status = {
    juniper: { number: "Open loops 4 (-3)", lastReport: { text: "On track. One open item: the staging database upgrade.", at: R("monday-1").endedAt! }, next: { text: "Weekly review", at: nextDow(5, 16) } },
    otto: { number: "Uptime 99.98% (+0.02)", lastReport: { text: "Backups restored cleanly; staging needs Postgres 16.", at: R("pg-1").endedAt! }, next: { text: "Certificate and uptime check", at: nextAt(7) } },
    rafi: { number: "Flaky tests 7 (-2)", lastReport: { text: "405 of 412 passed; 7 flaky.", at: R("flaky-1").endedAt! }, next: { text: "Nightly regression sweep", at: nextAt(1) } },
    mae: { number: "Answered within a day 92% (+6)", lastReport: { text: "14 new tickets overnight, 1 needs you.", at: R("triage-1").endedAt! }, next: { text: "Morning ticket triage", at: nextAt(9) } },
    theo: { number: "Trial signups 214 (+31)", lastReport: { text: "The November newsletter is drafted.", at: R("news-1").endedAt! }, next: { text: "Newsletter draft", at: nextDow(2, 10) } },
    ines: { number: "Payouts reconciled 3 of 5 (-2)", lastReport: { text: "October is closed apart from one payment.", at: R("close-1").endedAt! }, next: { text: "Check whether Lena replied", at: ahead(34) } },
  };

  // ---- skills
  const craft = CRAFT(onDay(-1, 4));
  w.skills = {
    juniper: [
      own("juniper-hale", "Juniper's role: keep Maren's week on track.", ago(205), ["SKILL.md"], { role: true }),
      own("old-launch-checklist", "The checklist from the spring launch.", onDay(-61, 10)),
      own("planning", "How Juniper plans a change: owners, dates, who hears, what could go wrong.", onDay(-6, 14), ["SKILL.md", "checklists.md"]),
      own("price-changes", "Price changes: the notice period, who to tell, and the email.", ago(31)),
      own("weekly-report", "The Monday report: status line first, then what moved, what's stuck, what needs Maren.", ago(205), ["SKILL.md", "examples.md"]),
      craft,
    ],
    otto: [
      own("otto-lindqvist", "Otto's role: keep Fernbrook up, fast and cheap to run.", onDay(-4, 11), ["SKILL.md"], { role: true }),
      own("certificates", "Where every certificate and domain lives, and when each renews.", onDay(-12, 7)),
      own("first-checks", "The first five things to check when the booking app is slow or down.", onDay(-4, 11, 20)),
      own("paging", "When to wake Maren, and how.", onDay(-4, 11, 20)),
      own("sql-reports", "Read-only queries against the reporting replica.", onDay(-4, 11, 20), ["SKILL.md", "queries/bookings.sql"]),
      craft,
    ],
    rafi: [own("rafi-mendel", "Rafi's role: catch booking bugs before a studio does.", onDay(-9, 9), ["SKILL.md"], { role: true }), own("flaky-tests", "How to tell a flaky test from a real failure.", onDay(-2, 1, 40)), own("release-checks", "What to test before every release.", onDay(-15, 10)), craft],
    mae: [own("mae-okonkwo", "Mae's role: clear answers within a day.", onDay(-7, 9), ["SKILL.md"], { role: true }), own("refunds", "Refunds: what Mae approves alone, and how to word them.", ago(40)), own("ticket-triage", "Sorting the overnight tickets: answer, pass on, or ask Maren.", onDay(-3, 9, 20), ["SKILL.md", "macros.md"]), own("help-centre", "Writing and updating help articles.", onDay(-5, 14)), craft],
    theo: [own("theo-marchetti", "Theo's role: grow signups without discounting.", onDay(-8, 10), ["SKILL.md"], { role: true }), own("newsletter", "The monthly newsletter: one lead story, two short items, one button.", lastDow(2, 10, 26)), own("pricing-page", "The pricing page's rules: one plan, no asterisks.", ago(185)), craft],
    ines: [own("ines-duarte", "Ines's role: close every month within five working days.", onDay(-20, 9), ["SKILL.md"], { role: true }), own("payouts", "Matching card payouts against the bank export.", onDay(-6, 6)), own("month-end", "The month-end close and the pack for Lena.", onDay(-3, 9, 48), ["SKILL.md", "pack-checklist.md"]), craft],
  };
  w.staged = { theo: [{ name: "release-notes", description: "Write release notes studios actually read.", files: ["SKILL.md", "examples.md"] }] };
  w.originals = {
    "organize-1": [
      { id: "user:incident-runbook", name: "incident-runbook", source: "user", present: true, removable: true, notes: [] },
      { id: "user:sql-reports", name: "sql-reports", source: "user", present: true, removable: true, notes: ["Mae's reports read it too"] },
    ],
  };

  const priceAfter = skillDoc("price-changes", "Price changes: the notice period, who to tell, and the email.",
    "# Price changes\n\n## Rules\n\n- Studios always get 30 days' notice of a price change. Count back from the change date; the email goes on that day.\n- Annual plans keep their price until renewal. Say so in the first paragraph.\n\n## Who hears, in order\n\n1. Harriet, in the board update before the email\n2. Mae, with the help-centre changes, three days before\n3. Every studio on the plan, by email\n\n## The email\n\nSubject line names the plan and the date. Lead with what changes and when; then what it pays for; then how to ask.");
  const weeklyBefore = skillDoc("weekly-report", "The Monday report.", "# Monday report\n\nEvery Monday at 8 am.\n\n- What moved last week\n- What's stuck, and who it's waiting on\n- What needs Maren\n\nKeep it to one page.");
  const weeklyAfter = skillDoc("weekly-report", "The Monday report: status line first, then what moved, what's stuck, what needs Maren.", "# Monday report\n\nEvery Monday at 8 am.\n\nStart with one status line: **On track**, **At risk** or **Off track**, and why in a few words.\n\n- What moved last week\n- What's stuck, and who it's waiting on\n- What needs Maren\n\nKeep it to one page. Evidence after the status, never before.");
  w.revisions = {
    juniper: [
      { id: "rev-price", runId: "plan-1", at: ago(31), skill: "price-changes", file: "SKILL.md", change: "added", before: null, after: priceAfter },
      { id: "rev-weekly", runId: "tidy-1", at: ago(205), skill: "weekly-report", file: "SKILL.md", change: "modified", before: weeklyBefore, after: weeklyAfter },
      { id: "rev-plan", runId: null, at: onDay(-6, 14), skill: "planning", file: "checklists.md", change: "modified", before: "# Checklists\n\n- Owner\n- Date\n", after: "# Checklists\n\n- Owner\n- Date\n- Who hears, and when\n- What could go wrong\n" },
    ],
    mae: [{ id: "rev-refunds", runId: "refund-2", at: ago(40), skill: "refunds", file: "SKILL.md", change: "modified", before: skillDoc("refunds", "Refunds.", "# Refunds\n\nAsk Maren before any refund."), after: skillDoc("refunds", "Refunds: what Mae approves alone, and how to word them.", "# Refunds\n\n- Refunds under $50: approve them yourself and tell the studio the same day.\n- Anything bigger: ask Maren, with the booking and the reason.") }],
    otto: [{ id: "rev-first", runId: "organize-1", at: onDay(-4, 11, 20), skill: "first-checks", file: "SKILL.md", change: "added", before: null, after: skillDoc("first-checks", "The first five things to check when the booking app is slow or down.", "# First checks\n\n1. Status page and the last deploy\n2. Database connections and slow queries\n3. Error rate by endpoint\n4. Queue depth for reminders\n5. Certificates and DNS") }],
  };
  for (const [emp, skills] of Object.entries(w.skills)) {
    for (const s of skills) {
      for (const f of s.files) {
        const rev = w.revisions[emp]?.find((r) => r.skill === s.name && r.file === f && r.after);
        w.files[`${emp}/${s.name}/${f}`] = rev?.after ?? (f === "SKILL.md"
          ? skillDoc(s.name, s.description, `# ${s.name.replace(/-/g, " ").replace(/^./, (c) => c.toUpperCase())}\n\n${s.description}\n\n## How\n\n- Start from what changed since the last run\n- Write down anything Maren corrects, the same day\n- Ask before doing anything that can't be undone`)
          : `# ${f}\n\n- Kept up to date by ${w.employees.find((e) => e.id === emp)?.name.split(" ")[0]}\n`);
      }
    }
  }
  w.files["juniper/@own/what-you-own.md"] = "# What Juniper owns\n\n- The Monday report, and its format\n- Every plan that touches more than one person\n- Following up on anything Maren asks for in a conversation\n\nJuniper decides alone: when to chase, who to ask, how to word a reminder.\nJuniper asks first: anything sent to every studio, anything about money.";
  w.health = {
    juniper: { findings: [
      { kind: "looseNotes", skill: null, file: "notes/scratch.md", detail: "3 notes sit outside any skill." },
      { kind: "unused", skill: "old-launch-checklist", file: null, detail: "old-launch-checklist hasn't been used in 60 days." },
    ] },
  };
  w.checks = [
    { id: "chk-1", employeeId: "juniper", what: "Look at trial-to-paid again after the price email goes out", how: "Compare February's rate with January's", dueAt: onDay(9, 9), state: "open", createdAt: ago(31), runId: "plan-1", closedAt: null, result: null },
    { id: "chk-2", employeeId: "otto", what: "Look at Tuesday evening's booking times again", how: "p50 and p95 for 6 to 7 pm", dueAt: nextDow(2, 19), state: "open", createdAt: ago(118), runId: "slow-1", closedAt: null, result: null },
    { id: "chk-3", employeeId: "otto", what: "Re-check the staging certificate", how: "Expiry date after renewal", dueAt: onDay(-3, 7), state: "done", createdAt: onDay(-10, 7), runId: null, closedAt: onDay(-3, 7, 2), result: "Renewed, valid for 89 days." },
  ];

  w.companyPage = "# Fernbrook\n\nFernbrook makes booking software for small fitness studios: classes, waitlists, payments and reminders.\n\n## What we sell\n\n- Fernbrook Studio, $29 a month per studio (moving to $34 on 1 March)\n- Annual plans, by request\n\n## What we use\n\n- The web app is in ~/Fernbrook/web, the infrastructure in ~/Fernbrook/infra\n- Support tickets come in to help@fernbrook.app\n- Kofi owns design; ask him on Slack\n\n## How things work here\n\n- Show Maren a draft before anything goes to every studio\n- Studios get 30 days' notice of any price change\n\n## Voice and style\n\n- Plain and warm. Short sentences. No exclamation marks.\n- Sign messages to people as Maren's AI assistant\n";
  return w;
}

/** What the preview's employees say back to anything you send them. */
export function cannedAnswer(prompt: string): string {
  if (prompt.startsWith("Where are we?")) {
    return "- **Done:** the plan and the customer email draft\n- **Next:** fold in Otto's billing check, then send the email to Harriet on Friday\n- **Waiting on:** Otto, for the price IDs";
  }
  return "Got it. I've written that into my skills so it sticks, and I'll report back here when it's done.";
}

// ---------------------------------------------------------------- variations on the busy office

/** Adds `r` to the world as an open Inbox row, newest first. */
function addOpen(w: World, r: Run, events: RunEvent[] = [init(9), ...(r.summary ? [said(r.summary)] : []), done(r)]) {
  w.runs.unshift(r);
  w.events[r.id] = events;
  w.open.unshift(r.id);
  w.inbox.unshift(r.id);
}

/** The busy office after everything has finished and nothing needs a look: nobody working, waiting or failing. */
export function quietOffice(): World {
  const w = fullOffice();
  for (const r of w.runs) {
    if (r.state === "running" || r.state === "queued") {
      Object.assign(r, { state: "succeeded", startedAt: r.startedAt ?? ago(9), endedAt: ago(4), durationMs: 5 * MIN, turns: 6, summary: r.summary ?? "Done. Nothing else to report." });
      w.events[r.id] = [init(9), said(r.summary!), done(r)];
    } else if (r.state === "failed" || r.state === "lost") {
      Object.assign(r, { state: "succeeded", error: null, summary: r.summary ?? "Done. Nothing else to report." });
    }
    r.denials = [];
  }
  return w;
}

/** A large org: the busy office plus 14 more employees and 2 more people (25 members), one of them hired today with no
 *  runs yet, and Kofi with two open requests. Every name is invented. */
export function largeOffice(): World {
  const w = fullOffice();
  const more: [string, string, string, string | null, number, string][] = [
    ["wren", "Wren Adeyemi", "Data Analyst", "juniper", 130, "Builds the weekly numbers and answers questions about them."],
    ["sol", "Sol Varga", "Security Engineer", "otto", 4, "Reviews access, patches and the yearly audit."],
    ["ilse", "Ilse Brandt", "Release Manager", "otto", 250, "Plans each release and writes its notes."],
    ["tomas", "Tomas Reyes", "Test Automation Engineer", "rafi", 96, "Writes the end-to-end tests for booking and payments."],
    ["noa", "Noa Lindgren", "Mobile QA Engineer", "rafi", 300, "Tests the iOS and Android apps on real devices."],
    ["kit", "Kit Okafor", "Support Specialist", "mae", 28, "Answers billing and account tickets."],
    ["bea", "Bea Moreau", "Support Specialist", "mae", 340, "Answers class and calendar tickets."],
    ["arlo", "Arlo Petrov", "Help Centre Writer", "mae", 60, "Keeps the help centre's articles and screenshots current."],
    ["remy", "Remy Castell", "Content Writer", "theo", 20, "Writes the blog and the longer newsletter pieces."],
    ["iris", "Iris Nakamura", "Social Media Manager", "theo", 320, "Runs Fernbrook's social accounts and replies."],
    ["dev", "Dev Kapoor", "SEO Specialist", "theo", 180, "Keeps the marketing site findable."],
    ["oona", "Oona Ferris", "Accounts Payable Clerk", "ines", 270, "Pays suppliers and chases invoices."],
    ["hal", "Hal Sorensen", "Payroll Assistant", "ines", 210, "Prepares payroll for Maren to approve."],
    ["mira", "Mira Solano", "Partnerships Lead", null, 150, "Finds studio chains and gyms to partner with."],
  ];
  more.forEach(([id, name, title, managerId, hue, description], i) => {
    w.employees.push(employee(id, name, title, { managerId, hue, description, objective: description, hiredAt: id === "wren" ? ago(40) : onDay(-20 + i, 9) }));
    if (id !== "wren") w.schedules.push(review(id));
  });
  w.people.push(
    { id: "ada", name: "Ada Quist", title: "Lawyer", hue: 230, addedAt: onDay(-30, 10), description: "Reviews Fernbrook's contracts and terms.", strengths: "Contracts, privacy", responsibilities: "Terms of service and studio contracts", channels: [{ id: "a-mail", kind: "email", address: "ada@quist.law", whenToUse: "Anything contractual" }], notes: "", manager: { type: "you" } },
    { id: "hugo", name: "Hugo Brandt", title: "Investor", hue: 70, addedAt: onDay(-58, 10), description: "Fernbrook's first investor; sits beside Harriet on the board.", strengths: "Growth, hiring", responsibilities: "Board meetings", channels: [{ id: "u-mail", kind: "email", address: "hugo@brandtventures.vc", whenToUse: "Board papers" }], notes: "", manager: { type: "member", id: "harriet" } },
  );
  const first = w.requests.find((r) => r.id === "req-kofi")!;
  w.requests.push({ ...first, id: "req-kofi-2", employeeId: "theo", runId: null, message: "Could you mock up the pricing page's hero for the new $34 price?", dueAt: onDay(2, 17), createdAt: ago(55), delivery: { ...first.delivery!, sentAt: ago(55) } });
  return w;
}

/** More for the Inbox: runs failing together on one error, one employee's two choice cards on the same thing, the same
 *  question asked by another employee with other choices, a three-line title, and a blocked MCP tool. */
export function inboxMoreOffice(): World {
  const w = fullOffice();
  const offline = "Network error: couldn't reach staging.fernbrook.app (timed out after 30s)";
  // Two more runs break on staging within the hour, so with Rafi's widget check they are one entry.
  addOpen(w, mk("smoke-1", "otto", { type: "manual" }, "Smoke-test staging after this morning's deploy.", { state: "failed", end: ago(70), mins: 1, error: offline }));
  addOpen(w, mk("links-1", "mae", { type: "manual" }, "Check the help-centre links against staging.", { state: "failed", end: ago(61), mins: 1, error: offline }));
  // Theo's two cards about the launch, asked from two runs: listed together.
  addOpen(w, mk("launch-1", "theo", { type: "manual" }, "Plan the waitlist launch post.", {
    end: ago(35), mins: 8, summary: "The launch post is drafted. Two choices about the launch:",
    cards: [card("Publish the launch post on Tuesday or Thursday?", { type: "choices", choices: ["Tuesday 3 March", "Thursday 5 March"], recommended: 0 }, { key: "ask:launch:day", why: "Tuesday gets the most opens; Thursday gives Kofi time for the screenshots." })],
  }));
  addOpen(w, mk("launch-2", "theo", { type: "manual" }, "Plan where the launch goes first.", {
    end: ago(33), mins: 4, summary: "One more launch choice:",
    cards: [card("Send the launch email first, or post on the blog first?", { type: "choices", choices: ["Email first", "Blog post first"], recommended: 0 }, { key: "ask:launch:channel" })],
  }));
  // The question Mae asked (annual plans by bank transfer), asked again by Ines with choices of her own.
  addOpen(w, mk("annual-1", "ines", { type: "manual" }, "Work out how annual plans would be paid.", {
    end: ago(44), mins: 6, summary: "If studios pay a year up front, we need to choose how.",
    cards: [card("Can Riverbend Pilates pay for a year by bank transfer?", { type: "choices", choices: ["Yes, by bank transfer", "Cards only, as now"], recommended: 1 }, { key: "ask:annual-bank-transfer", why: "Bank transfers take three days to clear and need matching by hand." })],
  }));
  // A decision whose title runs to three lines.
  addOpen(w, mk("dns-1", "otto", { type: "manual" }, "Plan the move to the new DNS provider.", {
    end: ago(27), mins: 12, summary: "The new provider is set up and tested. Ready when you are.",
    cards: [card("Move fernbrook.app's DNS to the new provider on Saturday night, while bookings are quietest, and keep the old records for a week in case anything still points at them?", { type: "choices", choices: ["Move it Saturday", "Wait a week"], recommended: 0 })],
  }));
  // Blocked on a tool from an MCP server.
  const filed = "I sorted the 7 flaky tests and wrote each one up, but I'm not allowed to create Linear issues yet.";
  addOpen(w, mk("issues-1", "rafi", { type: "manual" }, "File the flaky tests as Linear issues so they're tracked.", {
    end: ago(22), mins: 5, summary: filed,
    denials: [{ tool: "mcp__linear__create_issue", brief: "Create \"Flaky: booking/waitlist-promotes-next-in-line\" in Fernbrook QA", suggestedRule: "mcp__linear__create_issue" }],
  }), [init(9), ...tool("l1", "Read", "test-results/flaky.json", "7 tests"), ...tool("l2", "mcp__linear__create_issue", "Flaky: booking/waitlist-promotes-next-in-line", "Blocked: not allowed", false), said(filed)]);
  // Blocked on a command in a folder outside their own, and asking for a rule and a folder with `request_access`.
  const mobile = `${HOME}/Fernbrook/mobile`;
  const e2e = "The web tests pass. I couldn't run the mobile ones: they live outside my folder, and I need the iPhone simulator and the design screens to check against.";
  addOpen(w, mk("mobile-1", "rafi", { type: "manual" }, "Run the booking tests on the mobile app too, and check the screens match the designs.", {
    end: ago(9), mins: 7, summary: e2e,
    denials: [
      { tool: "Bash", brief: `cd ${mobile} && npm run e2e`, suggestedRule: "Bash(npm:*)", folder: mobile, asked: false },
      { tool: "Bash", brief: "To start the iPhone simulator the mobile tests run in.", suggestedRule: "Bash(xcrun simctl:*)", folder: null, asked: true },
      { tool: "folder", brief: "To compare each booking screen with its design.", suggestedRule: "", folder: `${HOME}/Fernbrook/design/screens`, asked: true },
    ],
  }), [init(9), ...tool("m1", "Bash", "npx playwright test booking", "12 passed"), ...tool("m2", "Bash", `cd ${mobile} && npm run e2e`, "Blocked: not allowed", false), ...tool("m3", "mcp__omega__request_access", "Bash(xcrun simctl:*), design/screens", "Asked"), said(e2e)]);
  return w;
}

/** More live work for conversations: three runs going at once on Juniper's thread (her reply, Otto's billing check
 *  and Mae's help-centre update, both delegated), and a finished answer of Otto's with 22 steps. Otto's check finishes a
 *  few seconds after the page loads (the transport times it). */
export function liveMoreOffice(): World {
  const w = fullOffice();
  const help = mk("help-1", "mae", { type: "delegated", parentRunId: "plan-2" }, "Update the three help-centre articles that quote $29 so they say $34 from 1 March.", { state: "running", end: ago(1) });
  w.runs.unshift(help);
  w.events[help.id] = [init(9), said("Three articles quote the Studio price. Starting with choose-a-plan."), ...tool("h1", "Read", "help-centre/billing/choose-a-plan.md", "48 lines"), { type: "toolCall", id: "h2", tool: "Edit", brief: "help-centre/billing/choose-a-plan.md" }];
  w.events["plan-2"].splice(5, 0, ...tool("q1b", "mcp__omega__delegate", "Mae: update the help-centre articles that quote $29", "Delegated to Mae"), { type: "delegated", childRunId: "help-1", employeeId: "mae", task: help.prompt });
  const slow = w.runs.find((r) => r.id === "slow-1")!;
  const steps: RunEvent[] = [];
  const reads = ["traces/tuesday-18.json", "traces/tuesday-19.json", "web/src/pages/book.tsx", "web/src/api/bookings.ts", "infra/db/indexes.sql", "infra/db/slow-queries.log"];
  for (let i = 0; i < 22; i++) {
    const [name, brief, result] = i % 3 === 0 ? ["Bash", `kubectl logs deploy/web --since=48h | grep -c slow # pass ${i / 3 + 1}`, `${200 + i} lines`] : i % 3 === 1 ? ["Read", reads[i % reads.length], `${40 + i * 3} lines`] : ["Grep", `"bookings" in web/src (${i})`, `${i} matches`];
    steps.push(...tool(`s${i}`, name, brief, result));
  }
  w.events[slow.id] = [init(8), said("Looking at Tuesday evening's traces."), ...steps, said(slow.summary ?? ""), done(slow)];
  return w;
}
