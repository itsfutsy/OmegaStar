// The frontend/backend contract. Mirrors the Rust types in src-tauri/src/model.rs.

export type Company = {
  name: string; ceoName: string;
  title: string;                            // your title, "CEO" unless you set another
  email: string | null;
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
  browser: boolean;                         // may drive your Chrome (Claude in Chrome); off for new hires unless you turn it on
  autoTidy: boolean;                        // tidies their own skills and notes when Omega Star finds something to tidy; on for new hires
  workspaces: string[]; hue: number;        // 0-359, avatar colour
  chatSession: string; hiredAt: string;
};
export type EmployeeDraft = Omit<Employee, "id" | "slug" | "chatSession" | "hiredAt">;

export type Presence = "idle" | "working" | "queued" | "attention";
export type EmployeeView = Employee & {
  presence: Presence;
  activeRunId: string | null;         // the run to look at while working or queued: a work run before a chat turn
  activeLane: "work" | "chat" | null; // which lane that run is in
  working: number;                    // runs they have going at once
  waiting: number;                    // runs of theirs waiting to start
  nextFire: string | null;
  lastRun: Run | null;                // their latest work-lane run; "attention" is decided from it
  lastWorkedAt: string | null;        // when they last finished anything, in either lane
};

export type Person = {
  id: string; name: string; title: string; description: string;
  strengths: string; responsibilities: string;
  channels: Channel[]; notes: string;
  manager: PersonManager;
  hue: number; addedAt: string;
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
  configDir: string; skillsDir: string;
  found: number;              // catalog entries
  unreadable: number;         // skill folders that exist but could not be read
  error: string | null;       // the first I/O error, with its path
};

export type Schedule = {
  id: string; employeeId: string; label: string; cron: string; instruction: string;
  enabled: boolean;
  quiet: boolean;            // reports reach the Inbox only when the run calls notify or fails
  model: string | null; effort: Effort | null;   // null: the employee's own
  nextFire: string | null;
};
export type ScheduleDraft = Omit<Schedule, "id" | "nextFire"> & { id?: string };
export type CronPreview = { ok: true; description: string; next: string[] } | { ok: false; error: string };

export type RunKind =
  | { type: "manual" } | { type: "chat" }
  | { type: "scheduled"; scheduleId: string; slot: string; label: string; quiet: boolean; model?: string | null; effort?: Effort | null }
  | { type: "delegated"; parentRunId: string }
  | { type: "asked"; parentRunId: string; askedBy: string; task: boolean }   // a colleague's question or task; askedBy = their employee id
  | { type: "answered"; askedRunId: string; from: string; task: boolean; chat: boolean }    // brings an answer or report to an asker whose run had ended; chat: as a turn in their chat
  | { type: "followUp"; requestId: string; personId: string }
  | { type: "replyCheck"; requestId: string; personId: string }
  | { type: "nudge"; requestId: string; personId: string }
  | { type: "unanswered"; requestId: string; personId: string }
  | { type: "organize"; imported: ImportedSkill[]; settled: boolean }
  | { type: "reply"; toRunId: string; label: string; task: boolean }    // your reply to a report or in a task; label = what it was
  | { type: "task"; label: string; from: string | null; context: string }   // a task you started beside the chat; from = the conversation it started in
  | { type: "hireDecided"; proposalId: string; approved: boolean }   // you approved or declined their hire proposal
  | { type: "tidy"; reason: TidyReason; signature: string[]; plan: string | null };   // they tidy their own skills and notes
/** auto: Omega Star saw something to tidy. manual: you asked. approved: you approved the changes an earlier tidy proposed. */
export type TidyReason = "auto" | "manual" | "approved";
/** A catalog skill copied into an employee's imports/ folder. */
export type ImportedSkill = { id: string; name: string };
/** An imported skill waiting in imports/ to be organized. */
export type StagedImport = { name: string; description: string; files: string[] };
/** One of your originals behind an Organize run, as offered for removal. */
export type OriginalSkill = {
  id: string; name: string; source: SkillSource;
  present: boolean;            // still in your Claude Code skills
  removable: boolean;          // Omega Star can move it to the Recycle Bin (user skills only)
  notes: string[];             // why removing it could hurt
};
export type RunState = "queued" | "running" | "succeeded" | "failed" | "cancelled" | "lost";
export type Denial = { tool: string; brief: string; suggestedRule: string };
export type SkillChangeKind = "added" | "modified" | "removed";
export type SkillChange = { skill: string; file: string; change: SkillChangeKind };
export type Run = {
  id: string; employeeId: string; kind: RunKind; prompt: string;
  omegaWrote: boolean;         // Omega Star wrote the prompt (an approval, a returned report), not you
  state: RunState;
  createdAt: string; startedAt: string | null; endedAt: string | null;
  turns: number | null; durationMs: number | null;
  summary: string | null; error: string | null; denials: Denial[];
  skillChanges: SkillChange[];
  needsYou: string[];          // what the employee flagged with notify during the run
  archivedAt: string | null;   // archived reports leave the Inbox for the Archive
  repliedAt: string | null;    // when you last replied to this report
  deliveredAt: string | null;  // asked and delegated runs: when the answer or report reached whoever handed it over
  allowOnce: string[];         // rules you granted this run only ("Allow once & retry")
  proposed: string | null;     // a tidy's bigger changes, waiting for your approval
  attachments: string[];       // file names you attached to a chat message
  build: RunBuild | null;      // the build that ran it, and the Claude Code organisation around it
  limitRetries: number;        // times it went back to the queue after the usage limit
  contextTokens: number | null; // the context size after its last top-level message
};
export type NewAttachment = { name: string; data: string /* base64 */ };

/** A report and the replies you exchanged on it, oldest first. In the Inbox and the Archive a thread leads a row,
 *  and `related` holds the row's other threads, oldest first: the rest of a hand-off, or a schedule's earlier
 *  failures in a row. Empty elsewhere. */
export type Thread = { root: Run; replies: Run[]; related: Thread[] };
/** Where an employee's ongoing chat stands: read from its current session's turns. */
export type ChatHead = {
  employeeId: string;
  lastEndedAt: string | null;  // when the newest finished turn ended
  live: boolean;               // a turn is running
  queued: boolean;             // a turn is waiting to start
};
/** Every conversation across the office, for the Inbox and each employee's chips. */
export type Conversations = {
  threads: Thread[];           // open threads of every employee, one per root, most recently active first
  inbox: string[];             // root ids of the threads that lead an Inbox row: reports that reached you
  chats: ChatHead[];           // one per current employee
};
/** What an employee's ongoing chat is told about a thread or task you archived; the chat shows it as a card. */
export type Brief = {
  id: string; employeeId: string; rootRunId: string;
  label: string;               // what the report was
  replies: number;
  excerpt: string;             // the last thing the employee said in the thread, one line
  text: string;                // what the employee reads at the start of their next turn
  createdAt: string; consumedAt: string | null;   // consumed: a chat turn was launched with it
  task: boolean;               // the thread was a task's
};

export type OwnSkill = {
  name: string; description: string; role: boolean; managed: boolean;
  files: string[]; updatedAt: string;
};
/** Worst first: broken structure, then housekeeping. */
export type HealthKind =
  | "overdueCheck" | "dangling" | "unreachable" | "looseNotes" | "churn" | "repeatedEntries"
  | "tooLong" | "datedLines" | "descriptionsHeavy" | "description" | "junk" | "nameMismatch";
export type HealthFinding = { kind: HealthKind; skill: string | null; file: string | null; detail: string };
export type SkillHealth = { skills: number; descriptionChars: number; findings: HealthFinding[] };
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
  | { type: "user"; text: string }                 // a message you sent into the run while it worked
  | { type: "text"; text: string }
  | { type: "textDelta"; text: string }
  | { type: "toolCall"; id: string; tool: string; brief: string }
  | { type: "toolResult"; id: string; ok: boolean; brief: string }
  | { type: "delegated"; childRunId: string; employeeId: string; task: string }
  | { type: "asked"; childRunId: string; employeeId: string; question: string; task: boolean }
  | { type: "requested"; requestId: string; personId: string; kind: RequestKind; message: string }
  | { type: "proposedHire"; proposalId: string; name: string; title: string }
  | { type: "skillChanged"; skill: string; file: string; change: SkillChangeKind }
  // A sub-agent's lifecycle. These never reach a transcript list: they update a task by taskId (see foldEvent).
  | { type: "taskStarted"; taskId: string; toolUseId: string; name: string }
  | { type: "taskProgress"; taskId: string; activity: string; lastTool: string | null; toolUses: number; tokens: number; durationMs: number }
  | { type: "taskEnded"; taskId: string; outcome: TaskOutcome; toolUses: number; tokens: number; durationMs: number }
  | { type: "result"; ok: boolean; text: string; turns: number; durationMs: number }
  | { type: "stderr"; text: string };
export type TaskOutcome = "done" | "failed" | "stopped";
export type RunDetail = { run: Run; events: RunEvent[]; children: Run[] };

export type Account = { loggedIn: boolean; email: string | null; orgName: string | null; plan: string | null };
/** How this copy of Omega Star was built: a published release, a test build, or a developer's own. */
export type BuildChannel = "release" | "dogfood" | "dev";
export type Build = { version: string; commit: string; channel: BuildChannel };
export type RunBuild = Build & { claude: string | null; org: string | null };
export type SetupStatus = {
  claude: { found: boolean; path: string | null; version: string | null };
  account: Account | null;
  company: Company | null;
  background: boolean;
  build: Build;
};
export type AppSettings = { maxConcurrentRuns: number; claudePath: string | null; dataDir: string; paused: boolean; backup: BackupSettings; watch: WatchSettings };

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

export type UsageWindow = { percent: number; resetsAt: string | null; resetsText: string | null };   // percent 0-100
export type UsageLimits = {
  fiveHour: UsageWindow | null; sevenDay: UsageWindow | null; observedAt: string | null; subscription: boolean;
  heldUntil: string | null;    // no run starts until the usage limit resets then
  accountIssue: { message: string; signedInAs: string | null } | null;   // routines wait for the right account
};

export type ApiError = { code: string; message: string };

/** Every command: its argument object and its result. */
export type Commands = {
  setup_status: { args: Record<string, never>; ret: SetupStatus };
  complete_setup: { args: { companyName: string; ceoName: string; title: string | null }; ret: Company };
  update_company: { args: { name: string; ceoName: string; title: string; reportsTo: string | null }; ret: Company };
  set_background: { args: { enabled: boolean }; ret: SetupStatus };
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
  send_chat: { args: { employeeId: string; text: string; attachments: NewAttachment[] }; ret: Run };
  /** A task beside the ongoing chat, run at the same time as everything else. It starts from the ongoing chat. */
  start_task: { args: { employeeId: string; text: string; attachments: NewAttachment[] }; ret: Run };
  open_attachment: { args: { runId: string; name: string }; ret: null };
  read_image: { args: { path: string }; ret: string };
  open_image: { args: { path: string }; ret: null };
  /** A path an agent mentioned: documents and folders open, anything else is shown in Finder. */
  open_report_path: { args: { path: string }; ret: null };
  new_chat: { args: { employeeId: string }; ret: null };
  chat: { args: { employeeId: string }; ret: Run[] };
  runs: { args: { employeeId: string | null; limit: number }; ret: Run[] };
  run_detail: { args: { runId: string }; ret: RunDetail };
  archived: { args: Record<string, never>; ret: Thread[] };
  /** Every open thread, which of them lead an Inbox row, and each employee's ongoing chat. */
  conversations: { args: Record<string, never>; ret: Conversations };
  /** The thread any of its runs belongs to. */
  thread: { args: { runId: string }; ret: Thread };
  /** Briefs left for the employee's current chat conversation, oldest first. */
  chat_briefs: { args: { employeeId: string }; ret: Brief[] };
  /** Archives a thread (any run in it) or moves it back. Archiving one you replied in briefs the ongoing chat unless `brief: false`. */
  set_archived: { args: { runId: string; archived: boolean; brief?: boolean }; ret: Run };
  /** Your reply in a report's thread (`runId`: any run in it). `archive` means "don't bring the answer back unless it needs me". */
  reply_to_run: { args: { runId: string; text: string; archive: boolean; attachments?: NewAttachment[] }; ret: Run };
  /** Allow what a finished run was blocked on and send it back to retry. `rules` must come from its denials
   *  (a denial's suggestedRule, or `mcp__<server>__*` for an MCP tool). `always` saves them to the employee;
   *  otherwise they apply to the retry only. Archives the blocked report. Returns the retry run. */
  approve_blocked: { args: { runId: string; rules: string[]; always: boolean }; ret: Run };
  /** Colleague asks still in flight (queued, running, or answered and not yet collected), oldest first. */
  open_asks: { args: Record<string, never>; ret: Run[] };
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
  allow_rule: { args: { employeeId: string; rule: string }; ret: Employee };
  add_person: { args: { draft: PersonDraft }; ret: Person };
  update_person: { args: { person: Person }; ret: Person };
  remove_person: { args: { personId: string }; ret: null };
  requests: { args: { personId: string | null; employeeId: string | null; state: string | null }; ret: HumanRequest[] };
  resolve_request: { args: { requestId: string; state: Exclude<RequestState, "open">; response: string | null }; ret: HumanRequest };
  watch_request: { args: { requestId: string; action: WatchAction }; ret: HumanRequest };
  proposals: { args: { state: ProposalState | null }; ret: HireProposal[] };
  approve_proposal: { args: { proposalId: string; draft: EmployeeDraft; imports: string[]; note: string | null }; ret: Employee };
  decline_proposal: { args: { proposalId: string; note: string | null }; ret: HireProposal };
  settings: { args: Record<string, never>; ret: AppSettings };
  save_settings: { args: { settings: AppSettings }; ret: AppSettings };
  open_data_dir: { args: Record<string, never>; ret: null };
  backup_status: { args: Record<string, never>; ret: BackupStatus };
  backup_now: { args: Record<string, never>; ret: BackupStatus };
  pick_backup_dir: { args: Record<string, never>; ret: string | null };
  open_backup_dir: { args: Record<string, never>; ret: null };
  usage_limits: { args: Record<string, never>; ret: UsageLimits };
  refresh_usage: { args: Record<string, never>; ret: UsageLimits };
  use_signed_in_account: { args: Record<string, never>; ret: Company };
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
};
export type EventName = keyof Events;
