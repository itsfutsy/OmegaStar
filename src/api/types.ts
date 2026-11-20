// The frontend/backend contract. Mirrors the Rust types in src-tauri/src/model.rs.

export type Company = {
  name: string; ceoName: string;
  title: string;                            // your title, "CEO" unless you set another
  email: string | null;
  image?: string | null;                    // your display image as a data: URL, shown in place of your initials
  reportsTo: string | null;                 // the PERSON you report to; null = you are the top of the company
};

export type Clearance = "supervised" | "autonomous";
export type Effort = "low" | "medium" | "high" | "xhigh" | "max";

export type Employee = {
  id: string; slug: string; name: string; title: string; description: string; objective: string;
  managerId: string | null;                 // any member id below you (employee or person); null = you
  skills: { builtins: boolean; keys: string[] };   // catalog ids
  clearance: Clearance; allowRules: string[];
  model: string | null; effort: Effort | null;
  browser: boolean;                         // may drive your Chrome (Claude in Chrome); on unless you turn it off
  autoTidy: boolean;                        // tidies their own skills and notes when OmegaStar finds something to tidy; on unless you turn it off
  workspaces: string[]; hue: number;        // 0-359, avatar colour
  image?: string | null;                    // display image as a data: URL, shown in place of their initials
  chatSession: string;                      // the session of the one long chat older builds kept; nothing reads it
  hiredAt: string;
};
export type EmployeeDraft = Omit<Employee, "id" | "slug" | "chatSession" | "hiredAt">;

export type Presence = "idle" | "working" | "queued" | "attention";
export type EmployeeView = Employee & {
  presence: Presence;
  live: Run[];                        // what they are working on: queued and running runs, running first, oldest first within each
  working: number;                    // runs they have going at once
  nextFire: string | null;
  lastRun: Run | null;                // their latest work-lane run; "attention" is decided from it
  lastWorkedAt: string | null;        // when they last finished anything, in either lane
  status: EmployeeStatus;
};
/** How an employee is doing at a glance: number is their latest weekly review's, as "metric value (change)";
 *  lastReport is the first line of their last finished report; next is their next schedule slot or soonest open check. */
export type EmployeeStatus = { number: string | null; lastReport: Dated | null; next: Dated | null };
/** A line of text and the time it is about. */
export type Dated = { text: string; at: string };

export type Person = {
  id: string; name: string; title: string; description: string;
  strengths: string; responsibilities: string;
  channels: Channel[]; notes: string;
  manager: PersonManager;
  hue: number; addedAt: string;
  image?: string | null;                    // display image as a data: URL, shown in place of their initials
};
/** Who a person reports to. Only people can sit above you, so only a person can be `top`. */
export type PersonManager =
  | { type: "you" }                         // you, the user
  | { type: "member"; id: string }          // an employee or another person
  | { type: "top" };                        // nobody: the top of the company, above you
export type PersonDraft = Omit<Person, "id" | "addedAt">;
export type ChannelKind = "teams" | "slack" | "email" | "jira" | "asana" | "azureDevOps" | "github" | "linear" | "phone" | "other";
export type Channel = {
  id: string; kind: ChannelKind;
  address: string;          // handle, email address, project key or URL
  whenToUse: string;        // your guidance
};
export type PersonView = Person & { openRequests: number };

export type RequestKind = "question" | "task" | "info";
export type RequestState = "open" | "answered" | "done" | "cancelled";
export type HumanRequest = {
  id: string; personId: string; employeeId: string; runId: string | null;
  kind: RequestKind; message: string; dueAt: string | null; channelId: string | null;
  state: RequestState; response: string | null;
  createdAt: string; resolvedAt: string | null;
  delivery: Delivery | null;
};

export type Locator =
  | { type: "teamsChat"; chatId: string; messageId: string; server: string | null }
  | { type: "other"; text: string };
export type Verdict = "answered" | "done" | "askedBack" | "acknowledged" | "noReply";
export type WatchPhase = { type: "watching" } | { type: "stopped"; at: string } | { type: "gaveUp"; at: string };
export type WatchPlan = { nextCheckAt: string; nudgeAt: string | null; giveUpAt: string };
export type Seen = { id: string; at: string };
export type Heard = { at: string; from: string; text: string; verdict: Verdict; note: string };
export type Watch = {
  phase: WatchPhase; resumedAt: string | null; lastCheckedAt: string | null;
  failures: number; lastError: string | null;
  seen: Seen | null;
  heard: Heard | null;
  nudgedAt: string | null;
  plan: WatchPlan | null;
};
export type DeliverySource = "employee" | "transcript";
export type Delivery = {
  locator: Locator; senderId: string | null; sentAt: string; via: string;
  source: DeliverySource;
  watch: Watch | null;
};
export type WatchAction = "check" | "nudge" | "stop" | "resume";

export type ProposalState = "open" | "approved" | "declined" | "withdrawn";   // withdrawn: the proposer took it back or left
/** A hire an employee proposed. Approving it (as proposed or changed) hires them; either way the proposer hears back. */
export type HireProposal = {
  id: string; employeeId: string; runId: string | null;   // employeeId: who proposed it
  reason: string;              // the gap the hire fills, in the proposer's words
  draft: EmployeeDraft;        // supervised, reporting to the proposer or one of their reports
  missingSkills: string[];     // skills they asked for that are not in your catalog
  state: ProposalState;
  note: string | null;         // your note to the proposer when you decided
  hiredId: string | null;      // the employee hired when approved
  createdAt: string; resolvedAt: string | null;
};

export type OrgSnapshot = { company: Company; employees: EmployeeView[]; people: PersonView[] };

export type SkillSource = "user" | "synced" | "plugin";
export type SkillInfo = {
  id: string; name: string; description: string;
  source: SkillSource; skills?: string[];
};
/** Where the catalog is read from, and what the last scan could not read. */
export type SkillSources = {
  skillsDir: string;
  found: number;              // catalog entries
  unreadable: number;         // skill folders that exist but could not be read
  error: string | null;       // the first I/O error, with its path
};

export type Schedule = {
  id: string; employeeId: string; label: string; cron: string; instruction: string;
  enabled: boolean;
  quiet: boolean;            // reports reach you only when the run calls notify or fails
  model: string | null; effort: Effort | null;   // null: the employee's own
  review?: boolean;          // the weekly review OmegaStar seeds for every employee
  activeHours: boolean;      // only during business hours (watch.hours): a time outside them is skipped
  nextFire: string | null;
};
export type ScheduleDraft = Omit<Schedule, "id" | "nextFire" | "review"> & { id?: string };
export type CronPreview = { ok: true; description: string; next: string[] } | { ok: false; error: string };

export type RunKind =
  | { type: "manual" }
  | { type: "chat" }   // a turn of the one long chat older builds had; nothing starts one now
  | { type: "scheduled"; scheduleId: string; slot: string; label: string; quiet: boolean; model?: string | null; effort?: Effort | null; review?: boolean }
  | { type: "delegated"; parentRunId: string }
  | { type: "asked"; parentRunId: string; askedBy: string; task: boolean }   // a colleague's question or task; askedBy = their employee id
  | { type: "answered"; askedRunId: string; from: string; task: boolean; chat: boolean; thread?: string | null }    // brings an answer or report to an asker whose run had ended; thread: the conversation it came back into, as its next turn; chat: an older build's chat
  | { type: "followUp"; requestId: string; personId: string }
  | { type: "replyCheck"; requestId: string; personId: string }
  | { type: "nudge"; requestId: string; personId: string }
  | { type: "unanswered"; requestId: string; personId: string }
  | { type: "organize"; imported: ImportedSkill[]; settled: boolean }
  | { type: "reply"; toRunId: string; label: string; task: boolean; card: boolean }    // your reply to a report or in a conversation (task); label = what it was; card: OmegaStar wrote it from your answer on a card
  | { type: "task"; label: string; from: string | null; context: string }   // a conversation you started; from and context: an older build's chat it began in
  | { type: "hireDecided"; proposalId: string; approved: boolean }   // you approved or declined their hire proposal
  | { type: "tidy"; reason: TidyReason; signature: string[]; plan: string | null };   // they tidy their own skills and notes
/** auto: OmegaStar saw something to tidy. manual: you asked. approved: you approved the changes an earlier tidy proposed. */
export type TidyReason = "auto" | "manual" | "approved";
/** A catalog skill copied into an employee's imports/ folder. */
export type ImportedSkill = { id: string; name: string };
/** An imported skill waiting in imports/ to be organized. */
export type StagedImport = { name: string; description: string; files: string[] };
/** One of your originals behind an Organize run, as offered for removal. */
export type OriginalSkill = {
  id: string; name: string; source: SkillSource;
  present: boolean;            // still in your Claude Code skills
  removable: boolean;          // OmegaStar can move it to the Recycle Bin (user skills only)
  notes: string[];             // why removing it could hurt
};
export type RunState = "queued" | "running" | "succeeded" | "failed" | "cancelled" | "lost";
export type Denial = { tool: string; brief: string; suggestedRule: string };
export type SkillChangeKind = "added" | "modified" | "removed";
export type SkillChange = { skill: string; file: string; change: SkillChangeKind };
/** What a run asks you with the answer built in. A run can ask several; each message is also in needsYou. */
export type CardItem = { id: string; text: string; about: string | null; recommended: boolean };   // text: what goes out; about: one line on what it answers
export type CardAsk =
  | { type: "choices"; choices: string[]; recommended: number }   // recommended: the index of the one they recommend
  | { type: "items"; verb: string; items: CardItem[] }            // verb: what a tick means, e.g. "Post"
  | { type: "chore"; subject: string; url: string | null }        // subject: "chrome" or "sign-in:<site>"; url: the page to open
  | { type: "task"; steps: string; url: string | null }           // a to-do only you can do
  | { type: "question" }                                          // a fact only you know, answered in your own words
  | { type: "news" };                                             // something you should know that needs no decision
export type AnswerHow = "tapped" | "typed" | "dismissed" | "rejected" | "replaced" | "withdrawn";   // dismissed: Not now; rejected: No
// held: not sent yet, since its run still asks something else; it goes with the rest once nothing is left open
export type CardAnswer = { at: string; how: AnswerHow; text: string; held: boolean };   // text: the fixed answer sent, empty when nothing was
/** What a card proposes to change for everyone: a line for the company page under one of its headings, or a new
 *  version of what the employee owns or of the company page, which the card's file holds. */
export type Proposal = { type: "companyLine"; section: string; line: string } | { type: "owns" } | { type: "companyPage" };
/** message: the question. why: one line on why it matters. key: what it is about (`ask:<name>` or `chore:<subject>`),
 *  so the same question from anyone is one Inbox item; null on cards asked before keys. file: the markdown or text
 *  file it is about, e.g. a draft, as a full path; `card_file` reads it. link: a web address it is about, such as a
 *  pull request. images: up to 3 full local image paths. backAt: when a card you put off comes back; timesBack: how
 *  often it has; parked: you put it off again once it came back, so it waits in Parked and never comes back by itself. */
export type Card = {
  message: string; why: string | null; key: string | null; file: string | null; ask: CardAsk; answer: CardAnswer | null;
  link: string | null; images: string[];
  backAt: string | null; timesBack: number; parked: boolean;
  proposal: Proposal | null;
};
/** A weekly review's one number, for Activity's week in numbers. */
export type Score = { metric: string; value: string; change: string; note: string };
/** Your answer on a card. */
export type ItemPick = { id: string; tick: boolean; note?: string | null };
/** text: your own words, answering this card only. ask: a question that goes now and leaves the card open. later: it
 *  comes back by itself at `until` (an ISO time), or is parked if it came back already. cant: a task you can't do.
 *  seen: you read the news, which sends nothing. */
export type CardReply =
  | { type: "choice"; index: number } | { type: "items"; picks: ItemPick[] } | { type: "done" } | { type: "dismiss" } | { type: "reject" }
  | { type: "text"; text: string } | { type: "ask"; text: string }
  | { type: "later"; until: string } | { type: "cant" } | { type: "seen" };
export type Run = {
  id: string; employeeId: string; kind: RunKind; prompt: string;
  omegaWrote: boolean;         // OmegaStar wrote the prompt (an approval, a returned report), not you
  state: RunState;
  createdAt: string; startedAt: string | null; endedAt: string | null;
  turns: number | null; durationMs: number | null;
  summary: string | null; error: string | null; denials: Denial[];
  skillChanges: SkillChange[];
  needsYou: string[];          // every card's question, and what OmegaStar flagged
  cards: Card[];               // what the run asked you, each with your answer
  score: Score | null;         // a weekly review's number, from report_number
  archivedAt: string | null;   // archived reports leave the Inbox for the Archive
  title: string | null;        // a thread's first run: the name the employee gave the thread with name_thread
  repliedAt: string | null;    // when you last replied to this report
  steered: boolean;            // you wrote into it while it worked, so its report waits for you in the Inbox
  aboutCard?: boolean;         // a question you asked from a card: it does not make the thread a conversation you are in
  deliveredAt: string | null;  // asked and delegated runs: when the answer or report reached whoever handed it over
  check: string | null;        // a due check that came back into the conversation it was set in: the check's id
  allowOnce: string[];         // rules you granted this run only ("Allow once", which retries)
  proposed: string | null;     // a tidy's bigger changes, waiting for your approval
  approvedAt: string | null;   // a tidy: when you approved what it proposed (a reply to it is no decision)
  attachments: string[];       // file names you attached to a message
  build: RunBuild | null;      // the build that ran it, and the Claude Code organisation around it
  limitRetries: number;        // times it went back to the queue after the usage limit
  limited: boolean;            // it failed on the usage limit, which OmegaStar waits out itself
  contextTokens: number | null; // the context size after its last top-level message
  costUsd: number | null;      // what it cost in US dollars as Claude Code reckons it, over all its attempts; null when unknown
};
export type NewAttachment = { name: string; data: string /* base64 */ };

/** A report and the replies you exchanged on it, oldest first. A conversation you started is one too, and an answer
 *  a colleague brought back into it is one of its replies. In the Inbox and the Archive a thread leads a row,
 *  and `related` holds the row's other threads, oldest first: the rest of a hand-off, or a schedule's earlier
 *  failures in a row. Empty elsewhere. */
export type Thread = { root: Run; replies: Run[]; related: Thread[] };
/** Every conversation across the office, for the Inbox and the sidebar. */
export type Conversations = {
  threads: Thread[];           // open threads of every employee, one per root, most recently active first
  inbox: string[];             // root ids of the threads that lead an Inbox row: what needs you
};
export type OwnSkill = {
  name: string; description: string; role: boolean; managed: boolean;
  files: string[]; updatedAt: string;
};
/** Worst first: broken structure, then housekeeping. */
export type HealthKind =
  | "overdueCheck" | "dangling" | "unreachable" | "looseNotes" | "churn" | "repeatedEntries"
  | "tooLong" | "unused" | "datedLines" | "descriptionsHeavy" | "description" | "junk" | "nameMismatch";
export type HealthFinding = { kind: HealthKind; skill: string | null; file: string | null; detail: string };
export type SkillHealth = { findings: HealthFinding[] };
/** The page every employee reads at the start of every run. */
export type CompanyPage = { text: string; findings: HealthFinding[] };
/** Something an employee said they would look at again; open ones come back when they are due. */
export type CheckState = "open" | "done" | "dropped";
export type Check = {
  id: string; employeeId: string; what: string; how: string;
  dueAt: string; state: CheckState; createdAt: string; runId: string | null;
  closedAt: string | null; result: string | null;
};
export type SkillRevision = SkillChange & {
  id: string; runId: string | null; at: string; before: string | null; after: string | null;
};

export type RunEvent =
  | { type: "init"; model: string; skills: string[] }
  | { type: "user"; text: string; id?: string }    // a message you sent into the run while it worked
  | { type: "heard"; id: string }                  // the employee took that message, at their next step
  | { type: "text"; text: string }
  | { type: "textDelta"; text: string }
  | { type: "toolCall"; id: string; tool: string; brief: string }
  | { type: "toolResult"; id: string; ok: boolean; brief: string }
  | { type: "delegated"; childRunId: string; employeeId: string; task: string }
  | { type: "asked"; childRunId: string; employeeId: string; question: string; task: boolean }
  | { type: "requested"; requestId: string; personId: string; kind: RequestKind; message: string }
  | { type: "proposedHire"; proposalId: string; name: string; title: string }
  | { type: "skillChanged"; skill: string; file: string; change: SkillChangeKind }
  | { type: "visual"; id: string; title: string }   // a page the employee showed (show), served on omega-visual
  // A sub-agent's lifecycle. These never reach a transcript list: they update a task by taskId (see foldTask).
  | { type: "taskStarted"; taskId: string; toolUseId: string; name: string }
  | { type: "taskProgress"; taskId: string; activity: string; lastTool: string | null; toolUses: number; tokens: number; durationMs: number }
  | { type: "taskEnded"; taskId: string; outcome: TaskOutcome; toolUses: number; tokens: number; durationMs: number }
  | { type: "result"; ok: boolean; text: string; turns: number; durationMs: number }
  | { type: "stderr"; text: string };
export type TaskOutcome = "done" | "failed" | "stopped";
export type RunDetail = { run: Run; events: RunEvent[]; children: Run[] };

/** What the team got done and what it asked of you, all time, for the Analytics page to slice by range. */
export type TeamStats = {
  staff: { employeeId: string; name: string; hiredAt: string; departedAt: string | null }[];
  /** One row per employee per local day (`YYYY-MM-DD`) they finished runs. nightMs: the part of workMs from runs that
   *  started between 10 pm and 7 am local. */
  days: { day: string; employeeId: string; runs: number; workMs: number; nightMs: number }[];
  /** Every card that asked you something (never news): when its run ended (or began, while it works), when you answered,
   *  and whether it still waits. */
  asks: { at: string; answeredAt: string | null; open: boolean }[];
  /** Each employee's latest weekly number, from their newest weekly review that reported one. */
  numbers: { employeeId: string; metric: string; value: string; change: string }[];
};

/** What runs did and used since a time. Usage (costUsd) is what runs cost at API list price, as Claude Code reckons it. */
export type RunUsage = { employees: EmployeeUsage[]; routines: RoutineUsage[] };
/** An employee's finished runs: decisions are the cards they raised, handoffs the work they handed to colleagues. */
export type EmployeeUsage = { employeeId: string; runs: number; decisions: number; handoffs: number; failed: number; costUsd: number };
/** A routine's finished runs (weekly reviews and due checks are not routines). reached: notified you or handed work on;
 *  useful: did something (reached someone, changed a skill or failed); the rest did nothing. */
export type RoutineUsage = {
  scheduleId: string; employeeId: string; label: string; quiet: boolean;
  runs: number; reached: number; failed: number; useful: number; costUsd: number;
};

/** A model Claude Code offers: the exact ID `claude --model` takes, and the name its own picker shows. */
export type ModelOption = { id: string; name: string };
export type Account = { loggedIn: boolean; email: string | null; orgName: string | null; plan: string | null };
/** How this copy of OmegaStar was built: a published release, a test build, or a developer's own. */
export type BuildChannel = "release" | "dogfood" | "dev";
export type Build = { version: string; commit: string; channel: BuildChannel };
export type RunBuild = Build & { claude: string | null; org: string | null };
export type SetupStatus = {
  claude: { found: boolean; path: string | null; version: string | null };
  account: Account | null;
  company: Company | null;
  background: boolean;
  build: Build;
  /** A test copy that never starts Claude Code, backs up or watches anyone. */
  sandbox: boolean;
};
export type AppSettings = {
  maxConcurrentRuns: number; claudePath: string | null; dataDir: string; paused: boolean; backup: BackupSettings; watch: WatchSettings; proxy: ProxySettings;
  transcriptDays: number | null;   // days a finished run's transcript is kept; null: forever
  notifications: NotificationSettings;
};

/** all: each as it comes. batched: an employee's decisions in the 90 seconds after their first show as one. off: none. */
export type NotifyMode = "all" | "batched" | "off";
/** Local minutes since midnight, under 1440; a start later than the end runs overnight. */
export type QuietHours = { startMinute: number; endMinute: number };
/** quiet: no notification shows then, and one summary of what came in shows when they end; null: never quiet. */
export type NotificationSettings = { mode: NotifyMode; quiet: QuietHours | null };

/** A CLI proxy (CLIProxyAPI) that runs go through; an empty url means none. */
export type ProxySettings = { url: string; apiKey: string; managementKey: string };

export type CheckFrequency = "eager" | "normal" | "relaxed";
export type IsoWeekday = 1 | 2 | 3 | 4 | 5 | 6 | 7;
export type BusinessHours = { startMinute: number; endMinute: number; days: IsoWeekday[] };
export type WatchSettings = { hours: BusinessHours; frequency: CheckFrequency; nudgeAfterDays: number; giveUpAfterDays: number };

export type BackupFrequency = "off" | "hourly" | "daily" | "weekly";
export type BackupSettings = { frequency: BackupFrequency; dir: string; keep: number };   // keep 1-100
export type BackupStatus = {
  running: boolean;
  lastAt: string | null;          // the last successful backup
  lastFile: string | null;
  lastBytes: number | null;
  skipped: number;                // files the last backup couldn't read
  lastAttemptAt: string | null;
  lastError: string | null;       // the last attempt failed
  nextAt: string | null;          // null when automatic backups are off
  fingerprint: string | null;     // the office at the last successful backup
  checkedAt: string | null;       // the last time a backup was due but nothing had changed
  leftOut: number;                // large files and build folders the last backup left out
};

/** The phone link: `off` with no phone, `pairing` while a code shows, then for a paired phone `connecting` to the relay,
 *  `online` there with no phone, and `session` while the phone is connected. */
export type RemoteState = "off" | "pairing" | "connecting" | "online" | "session";
export type RemoteStatus = {
  state: RemoteState;
  deviceName: string | null;   // the paired phone; null when none is
  pairedAt: string | null;
  lastSeenAt: string | null;   // the last time the phone connected
  pushOn: boolean;             // phone notifications
  error: string | null;        // why the relay can't be reached; cleared once it can
  code: PairCode | null;       // the code showing while pairing
  activity: RemoteActivity[];  // what the paired phone did lately, newest first
};
/** Something the paired phone did: `connected` when it started a session, else the change it made, by its remote
 *  command (`answer_card`, `reply_to_run`, …, and `push_register` for a new push token). */
export type RemoteActivity = { at: string; what: string };
/** A code for a phone to scan: the QR as a data URL, the link it holds, and when it stops working. */
export type PairCode = { qr: string; link: string; expiresAt: string };

export type UsageWindow = { percent: number; resetsAt: string | null; resetsText: string | null };   // percent 0-100
export type UsageLimits = {
  fiveHour: UsageWindow | null; sevenDay: UsageWindow | null; observedAt: string | null; subscription: boolean;
  heldUntil: string | null;    // no run starts until then, when the usage limit resets or a rate limit eases
  heldForRateLimit: boolean;   // the hold is a rate limit's, which eases then
  accountIssue: { message: string; signedInAs: string | null } | null;   // routines wait for the right account
  proxyIssue: string | null;   // why the CLI proxy could not report its accounts' usage
  accounts: AccountUsage[];    // a CLI proxy's accounts one by one, which the windows above average; empty without one
};
/** One CLI proxy account's own windows; `modelWeeks` are weekly limits on one model, such as Fable. */
export type AccountUsage = { name: string; fiveHour: UsageWindow | null; sevenDay: UsageWindow | null; modelWeeks: { model: string; window: UsageWindow }[] };

export type ApiError = { code: string; message: string };

/** Every command: its argument object and its result. */
export type Commands = {
  setup_status: { args: Record<string, never>; ret: SetupStatus };
  complete_setup: { args: { companyName: string; ceoName: string; title: string | null }; ret: Company };
  update_company: { args: { name: string; ceoName: string; title: string; reportsTo: string | null; image: string | null }; ret: Company };
  set_background: { args: { enabled: boolean }; ret: SetupStatus };
  /** Claude Code's own model list for the signed-in account. */
  models: { args: Record<string, never>; ret: ModelOption[] };
  org: { args: Record<string, never>; ret: OrgSnapshot };
  skill_catalog: { args: Record<string, never>; ret: SkillInfo[] };
  skill_sources: { args: Record<string, never>; ret: SkillSources };
  hire: { args: { draft: EmployeeDraft; imports: string[] }; ret: Employee };
  update_employee: { args: { employee: Employee }; ret: Employee };
  let_go: { args: { employeeId: string }; ret: null };
  schedules: { args: { employeeId: string }; ret: Schedule[] };
  save_schedule: { args: { draft: ScheduleDraft }; ret: Schedule };
  delete_schedule: { args: { scheduleId: string }; ret: null };
  cron_preview: { args: { cron: string }; ret: CronPreview };
  run_now: { args: { employeeId: string; instruction: string | null }; ret: Run };
  cancel_run: { args: { runId: string }; ret: null };
  /** A new conversation (a task under the hood), run at the same time as everything else. Every next message is `reply_to_run`. */
  start_task: { args: { employeeId: string; text: string; attachments: NewAttachment[] }; ret: Run };
  open_attachment: { args: { runId: string; name: string }; ret: null };
  read_image: { args: { path: string }; ret: string };
  open_image: { args: { path: string }; ret: null };
  /** A path an agent mentioned: documents and folders open, anything else is shown in Finder. */
  open_report_path: { args: { path: string }; ret: null };
  /** The text of a notes file (.md or .txt) a path in a report names, to read beside the conversation. */
  report_file: { args: { path: string }; ret: string };
  runs: { args: { employeeId: string | null; limit: number }; ret: Run[] };
  /** The newest runs whose instruction, report or a card's question contains `query`, ignoring case. */
  search_runs: { args: { query: string; limit: number }; ret: Run[] };
  /** What each employee's runs and each routine did and used since `since`. */
  run_usage: { args: { since: string }; ret: RunUsage };
  team_stats: { args: Record<string, never>; ret: TeamStats };
  run_detail: { args: { runId: string }; ret: RunDetail };
  /** Every row that reached you and needs nothing from you now, what you archived, and the conversations you marked
   *  done, newest first. */
  archived: { args: Record<string, never>; ret: Thread[] };
  /** Every open thread, and which of them lead an Inbox row. */
  conversations: { args: Record<string, never>; ret: Conversations };
  /** The thread any of its runs belongs to. */
  thread: { args: { runId: string }; ret: Thread };
  /** Archives a thread (any run in it) with the rest of its row, leaving what still needs you, or moves it back.
   *  Returns the ids of the runs it moved. */
  set_archived: { args: { runId: string; archived: boolean }; ret: string[] };
  /** Moves exactly these runs back out of the Archive: Done's Undo. */
  unarchive_runs: { args: { runIds: string[] }; ret: null };
  /** Tries a run that failed, was lost or was stopped again: a hand-off runs again for whoever handed it over, anything
   *  else carries on in its thread. Returns the new run. */
  retry_run: { args: { runId: string }; ret: Run };
  /** Your reply in a report's thread (`runId`: any run in it). `archive` means "don't bring the answer back unless it needs me". */
  reply_to_run: { args: { runId: string; text: string; archive: boolean; attachments: NewAttachment[] }; ret: Run };
  /** Allow what a finished run was blocked on and send it back to retry. `rules` must come from its denials
   *  (a denial's suggestedRule, or `mcp__<server>__*` for an MCP tool). `always` saves them to the employee;
   *  otherwise they apply to the retry only. Archives the blocked report. Returns the retry run. */
  approve_blocked: { args: { runId: string; rules: string[]; always: boolean }; ret: Run };
  /** Your answer on a run's card (`card`: its index in `cards`). It goes to every run asking the same thing. Returns the replies it started.
   *  A question (`ask`) answers nothing: it returns the one reply that carries it. */
  answer_card: { args: { runId: string; card: number; answer: CardReply }; ret: Run[] };
  /** The text of the file a run's card shows (`card`: its index in `cards`), as it is now. */
  card_file: { args: { runId: string; card: number }; ret: string };
  /** Runs with a card you parked: put off again after it came back, so it never comes back by itself. */
  parked_runs: { args: Record<string, never>; ret: Run[] };
  /** Colleague asks still in flight (queued, running, or answered and not yet collected), oldest first. */
  open_asks: { args: Record<string, never>; ret: Run[] };
  /** Weekly reviews from the last eight days, oldest first. */
  week_reviews: { args: Record<string, never>; ret: Run[] };
  employee_skills: { args: { employeeId: string }; ret: OwnSkill[] };
  skill_health: { args: { employeeId: string }; ret: SkillHealth };
  checks: { args: { employeeId: string }; ret: Check[] };
  drop_check: { args: { checkId: string }; ret: Check };
  tidy: { args: { employeeId: string }; ret: Run };
  approve_tidy: { args: { runId: string }; ret: Run };
  tidy_runs: { args: { employeeId: string }; ret: Run[] };
  read_skill_file: { args: { employeeId: string; skill: string; file: string }; ret: string };
  save_skill_file: { args: { employeeId: string; skill: string; file: string; text: string }; ret: SkillRevision | null };
  company_page: { args: Record<string, never>; ret: CompanyPage };
  /** Your own edit: saved as is, with no revision. */
  save_company_page: { args: { text: string }; ret: CompanyPage };
  skill_history: { args: { employeeId: string }; ret: SkillRevision[] };
  revert_revision: { args: { revisionId: string }; ret: SkillRevision };
  promote_skill: { args: { employeeId: string; skill: string }; ret: SkillInfo };
  import_skills: { args: { employeeId: string; skillIds: string[] }; ret: Run };
  staged_imports: { args: { employeeId: string }; ret: StagedImport[] };
  organize: { args: { employeeId: string }; ret: Run };
  revert_run: { args: { runId: string }; ret: number };
  import_originals: { args: { runId: string }; ret: OriginalSkill[] };
  remove_originals: { args: { runId: string; names: string[] }; ret: string[] };
  settle_originals: { args: { runId: string }; ret: Run };
  add_person: { args: { draft: PersonDraft }; ret: Person };
  update_person: { args: { person: Person }; ret: Person };
  remove_person: { args: { personId: string }; ret: null };
  requests: { args: { personId: string | null; employeeId: string | null; state: RequestState | null }; ret: HumanRequest[] };
  resolve_request: { args: { requestId: string; state: Exclude<RequestState, "open">; response: string | null }; ret: HumanRequest };
  watch_request: { args: { requestId: string; action: WatchAction }; ret: HumanRequest };
  proposals: { args: Record<string, never>; ret: HireProposal[] };
  approve_proposal: { args: { proposalId: string; draft: EmployeeDraft; imports: string[]; note: string | null }; ret: Employee };
  decline_proposal: { args: { proposalId: string; note: string | null }; ret: HireProposal };
  settings: { args: Record<string, never>; ret: AppSettings };
  save_settings: { args: { settings: AppSettings }; ret: AppSettings };
  open_data_dir: { args: Record<string, never>; ret: null };
  backup_status: { args: Record<string, never>; ret: BackupStatus };
  backup_now: { args: Record<string, never>; ret: BackupStatus };
  pick_backup_dir: { args: Record<string, never>; ret: string | null };
  open_backup_dir: { args: Record<string, never>; ret: null };
  remote_status: { args: Record<string, never>; ret: RemoteStatus };
  /** Starts pairing a phone; pairing again replaces the paired one once the new phone pairs. */
  remote_pair_start: { args: Record<string, never>; ret: PairCode };
  remote_pair_cancel: { args: Record<string, never>; ret: null };
  /** Forgets the paired phone, telling it first if it is connected. */
  remote_unpair: { args: Record<string, never>; ret: null };
  remote_set_push: { args: { on: boolean }; ret: RemoteStatus };
  usage_limits: { args: Record<string, never>; ret: UsageLimits };
  refresh_usage: { args: Record<string, never>; ret: UsageLimits };
  use_signed_in_account: { args: Record<string, never>; ret: Company };
  /** When Vibe Typer last put dictated words into OmegaStar (milliseconds since the epoch); null when it never has or cannot be read. */
  last_dictation: { args: Record<string, never>; ret: number | null };
  check_update: { args: Record<string, never>; ret: string | null };
  install_update: { args: Record<string, never>; ret: null };
};
export type CommandName = keyof Commands;

/** Every event: its payload. */
export type Events = {
  "org:changed": null;
  "run:updated": Run;
  "run:event": { runId: string; events: RunEvent[] };
  "request:updated": HumanRequest;
  "proposal:updated": HireProposal;
  "usage:updated": UsageLimits;
  "backup:updated": BackupStatus;
  "remote:updated": RemoteStatus;
  /** An employee's `open` of a notes file: show it beside the conversation whose first run is `rootId`. */
  "reader:open": { rootId: string; path: string };
};
export type EventName = keyof Events;
