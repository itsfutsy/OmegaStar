//! Domain types shared with the frontend (camelCase JSON).
//!
//! Stored types are read back by older and newer builds: every field added after a release needs
//! #[serde(default)], and new data goes into the JSON, never a new column. An older build that saves a
//! row drops the fields it does not know, so a build of an unreleased branch runs on its own copy of
//! the office (OMEGASTAR_DATA_DIR).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};

pub type Ts = DateTime<Utc>;
// Re-exported so `model::*` also brings the error every command returns.
pub use crate::error::{AppError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub name: String,
    pub ceo_name: String,
    /// Your title, "CEO" unless you set another.
    pub title: String,
    pub email: Option<String>,
    /// The person you report to; `None` = you are the top of the company.
    pub reports_to: Option<String>,
}

impl Company {
    const DEFAULT_TITLE: &'static str = "CEO";

    /// Before onboarding: no name, the default title, nobody above you.
    pub fn unset() -> Company {
        Company { name: String::new(), ceo_name: String::new(), title: Self::DEFAULT_TITLE.into(), email: None, reports_to: None }
    }

    pub fn title_or_default(title: Option<&str>) -> String {
        match title.map(str::trim) {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => Self::DEFAULT_TITLE.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Clearance {
    Supervised,
    Autonomous,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGrant {
    pub builtins: bool,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmployeeDraft {
    pub name: String,
    pub title: String,
    pub description: String,
    pub objective: String,
    pub manager_id: Option<String>,
    pub skills: SkillGrant,
    pub clearance: Clearance,
    pub allow_rules: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    /// Lets them drive your Chrome (Claude in Chrome). Off unless you turn it on for a new hire.
    #[serde(default)]
    pub browser: bool,
    /// Tidies their skills and notes on their own when the health check finds something.
    #[serde(default)]
    pub auto_tidy: bool,
    pub workspaces: Vec<String>,
    pub hue: u16,
}

/// Employees hired before the browser switch existed could already drive Chrome, so they keep it.
fn browser_on() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Employee {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub objective: String,
    pub manager_id: Option<String>,
    pub skills: SkillGrant,
    pub clearance: Clearance,
    pub allow_rules: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    #[serde(default = "browser_on")]
    pub browser: bool,
    #[serde(default)]
    pub auto_tidy: bool,
    pub workspaces: Vec<String>,
    pub hue: u16,
    pub chat_session: String,
    pub hired_at: Ts,
}

impl EmployeeDraft {
    /// Boundary validation shared by hire and update.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(AppError::invalid("name is required"));
        }
        if self.hue > 359 {
            return Err(AppError::invalid("hue must be 0-359"));
        }
        Ok(())
    }

    pub fn into_employee(self, id: String, slug: String, chat_session: String, hired_at: Ts) -> Employee {
        Employee {
            id,
            slug,
            name: self.name.trim().to_string(),
            title: self.title,
            description: self.description,
            objective: self.objective,
            manager_id: self.manager_id,
            skills: self.skills,
            clearance: self.clearance,
            allow_rules: self.allow_rules,
            model: self.model.filter(|m| !m.trim().is_empty()),
            effort: self.effort,
            browser: self.browser,
            auto_tidy: self.auto_tidy,
            workspaces: self.workspaces,
            hue: self.hue,
            chat_session,
            hired_at,
        }
    }
}

impl Employee {
    pub fn draft(&self) -> EmployeeDraft {
        EmployeeDraft {
            name: self.name.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            objective: self.objective.clone(),
            manager_id: self.manager_id.clone(),
            skills: self.skills.clone(),
            clearance: self.clearance,
            allow_rules: self.allow_rules.clone(),
            model: self.model.clone(),
            effort: self.effort,
            browser: self.browser,
            auto_tidy: self.auto_tidy,
            workspaces: self.workspaces.clone(),
            hue: self.hue,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Idle,
    Working,
    Queued,
    Attention,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmployeeView {
    #[serde(flatten)]
    pub employee: Employee,
    pub presence: Presence,
    /// The run to look at while they are working or queued: a work run before a chat turn.
    pub active_run_id: Option<String>,
    /// Which lane `active_run_id` is in, so the UI can say "in chat" rather than "run".
    pub active_lane: Option<Lane>,
    /// How many runs they have going at once, and how many more wait to start.
    pub working: u32,
    pub waiting: u32,
    pub next_fire: Option<Ts>,
    /// Their latest work-lane run. `Presence::Attention` is decided from it, so it stays work-only.
    pub last_run: Option<Run>,
    /// When they last finished anything, in either lane: what "last worked" means.
    pub last_worked_at: Option<Ts>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChannelKind {
    Teams,
    Slack,
    Email,
    Jira,
    Asana,
    AzureDevOps,
    Github,
    Linear,
    Phone,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// Generated server-side when the client leaves it empty.
    #[serde(default)]
    pub id: String,
    pub kind: ChannelKind,
    pub address: String,
    pub when_to_use: String,
}

/// Who a person reports to. Employees need no `Top`: only people can sit above you.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PersonManager {
    /// You, the user.
    You,
    /// An employee or another person.
    Member { id: String },
    /// Nobody: the top of the company. Only the person at the top of the line above you.
    Top,
}

impl PersonManager {
    pub fn member_id(&self) -> Option<&str> {
        match self {
            PersonManager::Member { id } => Some(id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonDraft {
    pub name: String,
    pub title: String,
    pub description: String,
    pub strengths: String,
    pub responsibilities: String,
    pub channels: Vec<Channel>,
    pub notes: String,
    pub manager: PersonManager,
    pub hue: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub strengths: String,
    pub responsibilities: String,
    pub channels: Vec<Channel>,
    pub notes: String,
    pub manager: PersonManager,
    pub hue: u16,
    pub added_at: Ts,
}

impl PersonDraft {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(AppError::invalid("name is required"));
        }
        if self.hue > 359 {
            return Err(AppError::invalid("hue must be 0-359"));
        }
        Ok(())
    }

    pub fn into_person(self, id: String, added_at: Ts) -> Person {
        Person {
            id,
            name: self.name.trim().to_string(),
            title: self.title,
            description: self.description,
            strengths: self.strengths,
            responsibilities: self.responsibilities,
            channels: self.channels.into_iter().map(|mut c| {
                if c.id.trim().is_empty() {
                    c.id = uuid::Uuid::new_v4().to_string();
                }
                c
            }).collect(),
            notes: self.notes,
            manager: self.manager,
            hue: self.hue,
            added_at,
        }
    }
}

impl Person {
    pub fn draft(&self) -> PersonDraft {
        PersonDraft {
            name: self.name.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            strengths: self.strengths.clone(),
            responsibilities: self.responsibilities.clone(),
            channels: self.channels.clone(),
            notes: self.notes.clone(),
            manager: self.manager.clone(),
            hue: self.hue,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonView {
    #[serde(flatten)]
    pub person: Person,
    pub open_requests: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RequestKind {
    Question,
    Task,
    /// An FYI: created done, never watched, never followed up.
    Info,
}

impl RequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RequestKind::Question => "question",
            RequestKind::Task => "task",
            RequestKind::Info => "FYI",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RequestState {
    Open,
    Answered,
    Done,
    Cancelled,
}

impl RequestState {
    pub fn is_open(self) -> bool {
        matches!(self, RequestState::Open)
    }
    pub fn parse(s: &str) -> Result<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string()))
            .map_err(|_| AppError::invalid(format!("unknown request state '{s}'")))
    }
    pub fn as_str(self) -> &'static str {
        match self {
            RequestState::Open => "open",
            RequestState::Answered => "answered",
            RequestState::Done => "done",
            RequestState::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HumanRequest {
    pub id: String,
    pub person_id: String,
    pub employee_id: String,
    pub run_id: Option<String>,
    pub kind: RequestKind,
    pub message: String,
    pub due_at: Option<Ts>,
    pub channel_id: Option<String>,
    pub state: RequestState,
    pub response: Option<String>,
    pub created_at: Ts,
    pub resolved_at: Option<Ts>,
    /// How the ask went out. None for asks nobody reported sending.
    #[serde(default)]
    pub delivery: Option<Delivery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub locator: Locator,
    /// The sending account (Teams `from.user.id`). Its messages are never a reply: the user and every agent type as it.
    #[serde(default)]
    pub sender_id: Option<String>,
    pub sent_at: Ts,
    /// The tool that sent it, e.g. `mcp__ms365-local__send-chat-message`.
    pub via: String,
    pub source: DeliverySource,
    /// None exactly for Info. Only `Delivery::new` builds one.
    #[serde(default)]
    pub watch: Option<Watch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Locator {
    /// `server`: the employee's stdio MCP server the Office reads through; None = model checks instead.
    TeamsChat { chat_id: String, message_id: String, server: Option<String> },
    /// Anywhere else: the send's result or the employee's words, clipped to 300 chars.
    Other { text: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeliverySource {
    /// The employee called `sent`.
    Employee,
    /// Found in the asking run's transcript.
    Transcript,
}

/// Omega Star's watch on one delivered question or task. Only facts are stored; when the next check,
/// the nudge and the give-up fall is derived (`watch::plan`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub phase: WatchPhase,
    /// Your last Resume; the clocks restart from it.
    #[serde(default)]
    pub resumed_at: Option<Ts>,
    #[serde(default)]
    pub last_checked_at: Option<Ts>,
    /// Consecutive failed checks.
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub last_error: Option<String>,
    /// The newest message already judged; only later ones are judged.
    #[serde(default)]
    pub seen: Option<Seen>,
    /// The person's latest words about the ask and how they were read.
    #[serde(default)]
    pub heard: Option<Heard>,
    /// Any nudge, automatic or yours; the automatic one fires only while None.
    #[serde(default)]
    pub nudged_at: Option<Ts>,
    /// Filled on every read path by `Office::present`; never read back from storage.
    #[serde(skip_deserializing)]
    pub plan: Option<WatchPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WatchPhase {
    Watching,
    /// You stopped it.
    Stopped { at: Ts },
    /// Nobody replied in time; the employee was told to go on without it. The request stays open.
    GaveUp { at: Ts },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
    pub id: String,
    pub at: Ts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Heard {
    pub at: Ts,
    pub from: String,
    /// Clipped to 500 characters.
    pub text: String,
    pub verdict: Verdict,
    pub note: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// It answers the question.
    Answered,
    /// They did the task.
    Done,
    /// They asked something that must be answered first.
    AskedBack,
    /// They saw it and will get to it.
    Acknowledged,
    /// Nothing there is about this ask.
    NoReply,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchPlan {
    pub next_check_at: Ts,
    pub nudge_at: Option<Ts>,
    pub give_up_at: Ts,
}

/// What you can do to a watched request from its card.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WatchAction {
    Check,
    Nudge,
    Stop,
    Resume,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProposalState {
    Open,
    Approved,
    Declined,
    /// The proposer took it back, or left the company.
    Withdrawn,
}

impl ProposalState {
    pub fn parse(s: &str) -> Result<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string()))
            .map_err(|_| AppError::invalid(format!("unknown proposal state '{s}'")))
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalState::Open => "open",
            ProposalState::Approved => "approved",
            ProposalState::Declined => "declined",
            ProposalState::Withdrawn => "withdrawn",
        }
    }
}

/// A hire an employee proposed (`propose_hire`). You decide: approve it, as proposed or changed,
/// which hires the employee, or decline it. Either way the proposer hears back in a new run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HireProposal {
    pub id: String,
    /// The employee who proposed it.
    pub employee_id: String,
    pub run_id: Option<String>,
    /// The gap the hire fills, in the proposer's words.
    pub reason: String,
    /// The hire as proposed: supervised, reporting to the proposer or one of their direct reports.
    pub draft: EmployeeDraft,
    /// Skills they asked for that are not in your catalog.
    pub missing_skills: Vec<String>,
    pub state: ProposalState,
    /// Your note to the proposer when you decided.
    pub note: Option<String>,
    /// The employee hired when you approved it.
    pub hired_id: Option<String>,
    pub created_at: Ts,
    pub resolved_at: Option<Ts>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSnapshot {
    pub company: Company,
    pub employees: Vec<EmployeeView>,
    pub people: Vec<PersonView>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkillSource {
    User,
    Synced,
    Plugin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
}

/// Where the skill catalog is read from, and what the scan could not read.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSources {
    pub config_dir: String,
    pub skills_dir: String,
    /// Catalog entries.
    pub found: usize,
    pub unreadable: usize,
    /// The first I/O error, with its path.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Misfire {
    CatchUp,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDraft {
    #[serde(default)]
    pub id: Option<String>,
    pub employee_id: String,
    pub label: String,
    pub cron: String,
    pub instruction: String,
    pub enabled: bool,
    pub misfire: Misfire,
    /// Quiet routines reach the Inbox only when the run calls `notify` or does not succeed.
    #[serde(default)]
    pub quiet: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub employee_id: String,
    pub label: String,
    pub cron: String,
    pub instruction: String,
    pub enabled: bool,
    pub misfire: Misfire,
    #[serde(default)]
    pub quiet: bool,
    /// Computed on read, never stored.
    #[serde(default)]
    pub next_fire: Option<Ts>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CronPreview {
    Ok { description: String, next: Vec<Ts> },
    Err { error: String },
}

impl Serialize for CronPreview {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let v = match self {
            CronPreview::Ok { description, next } => {
                serde_json::json!({ "ok": true, "description": description, "next": next })
            }
            CronPreview::Err { error } => serde_json::json!({ "ok": false, "error": error }),
        };
        v.serialize(s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RunKind {
    Manual,
    Chat,
    Scheduled {
        schedule_id: String,
        slot: Ts,
        label: String,
        /// Copied from the schedule when the slot fires.
        #[serde(default)]
        quiet: bool,
    },
    Delegated { parent_run_id: String },
    /// A colleague (any employee, not only a manager) asked this employee a question, or with
    /// `task` handed them a task, through `ask_colleague`.
    Asked {
        parent_run_id: String,
        asked_by: String,
        #[serde(default)]
        task: bool,
    },
    /// Brings a colleague's answer (the finished Asked run `asked_run_id`, by employee `from`) to an
    /// asker whose own run ended before it arrived.
    Answered { asked_run_id: String, from: String, task: bool },
    FollowUp { request_id: String, person_id: String },
    /// Sorts skills you imported from Claude Code into the employee's family.
    Organize {
        imported: Vec<ImportedSkill>,
        /// You answered the "remove your originals?" offer (removed some, or kept them all).
        #[serde(default)]
        settled: bool,
    },
    /// You replied to a report in the Inbox; resumes that run's session. `label` names what the
    /// report was (a schedule's label, "Manual run", ...) so the reply's own report can say what it answers.
    /// `task`: the thread is a task's, so the reply continues the task.
    Reply {
        to_run_id: String,
        label: String,
        #[serde(default)]
        task: bool,
    },
    /// A task you started beside the ongoing chat: a conversation of its own, so it runs at the same
    /// time as the employee's other work. `label` names it. It starts from the conversation you were in:
    /// `context` is what was said there and `from` names it ("your chat"); both are empty when nothing was.
    Task {
        label: String,
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        context: String,
    },
    /// You approved or declined a hire this employee proposed.
    HireDecided { proposal_id: String, approved: bool },
    /// The employee tidies their own skills and notes: closes their open checks, confirms what they
    /// were taught is written down, and restructures what the health check flagged (see `tidy`).
    Tidy {
        reason: TidyReason,
        /// The findings it started from, so the next one only starts for a new problem.
        #[serde(default)]
        signature: Vec<String>,
        /// For an approved tidy: the proposals you approved, as the earlier tidy reported them.
        #[serde(default)]
        plan: Option<String>,
    },
    /// A quiet look for a reply on a channel the Office cannot read itself. haiku, low effort, fresh session.
    ReplyCheck { request_id: String, person_id: String },
    /// One short reminder on the same thread, in the employee's own model, fresh session. Quiet.
    Nudge { request_id: String, person_id: String },
    /// The give-up: go on without the answer or escalate. Resumes the asking run's session. Reaches the Inbox.
    Unanswered { request_id: String, person_id: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TidyReason {
    /// Omega Star saw something to tidy.
    Auto,
    /// You asked for one.
    Manual,
    /// You approved the bigger changes an earlier tidy proposed; this one may make them.
    Approved,
}

/// A catalog skill copied into an employee's `imports/` folder: `id` is its catalog id, `name`
/// the folder it was copied into.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedSkill {
    pub id: String,
    pub name: String,
}

/// An imported skill waiting in `imports/` to be organized.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StagedImport {
    pub name: String,
    pub description: String,
    pub files: Vec<String>,
}

/// One original behind an Organize run, as offered for removal from your Claude Code skills.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OriginalSkill {
    pub id: String,
    pub name: String,
    pub source: SkillSource,
    /// Still in your Claude Code skills.
    pub present: bool,
    /// Omega Star can move it to the Recycle Bin (user skills only).
    pub removable: bool,
    /// Why removing it could hurt ("Also granted to Leo", "Called by `recurring-weekly`").
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    Work,
    Chat,
}

impl Lane {
    pub fn as_str(self) -> &'static str {
        match self {
            Lane::Work => "work",
            Lane::Chat => "chat",
        }
    }
}

impl RunKind {
    pub fn lane(&self) -> Lane {
        match self {
            RunKind::Chat => Lane::Chat,
            _ => Lane::Work,
        }
    }

    /// Your conversations with the employee (the ongoing chat, tasks, replies in a thread) run beside
    /// everything else. The rest is background work, which an employee does one piece at a time.
    pub fn is_conversation(&self) -> bool {
        matches!(self, RunKind::Chat | RunKind::Task { .. } | RunKind::Reply { .. })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Lost,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Queued => "queued",
            RunState::Running => "running",
            RunState::Succeeded => "succeeded",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
            RunState::Lost => "lost",
        }
    }
    pub fn is_finished(self) -> bool {
        !matches!(self, RunState::Queued | RunState::Running)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Denial {
    pub tool: String,
    pub brief: String,
    pub suggested_rule: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Modified,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillChange {
    pub skill: String,
    pub file: String,
    pub change: ChangeKind,
}

/// How this copy of Omega Star was built: a published release, a test build, or a developer's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildChannel {
    Release,
    Dogfood,
    Dev,
}

impl BuildChannel {
    pub fn current() -> BuildChannel {
        match env!("OMEGA_CHANNEL") {
            "release" => BuildChannel::Release,
            "dogfood" => BuildChannel::Dogfood,
            _ => BuildChannel::Dev,
        }
    }
}

/// This copy of Omega Star: its version, the commit it was built from, and its channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Build {
    pub version: String,
    pub commit: String,
    pub channel: BuildChannel,
}

impl Build {
    pub fn new(version: String) -> Build {
        Build { version, commit: env!("OMEGA_COMMIT").into(), channel: BuildChannel::current() }
    }
}

/// The build a run started on, and the Claude Code and organisation around it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBuild {
    #[serde(flatten)]
    pub app: Build,
    #[serde(default)]
    pub claude: Option<String>,
    #[serde(default)]
    pub org: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub employee_id: String,
    pub kind: RunKind,
    pub prompt: String,
    pub state: RunState,
    pub created_at: Ts,
    pub started_at: Option<Ts>,
    pub ended_at: Option<Ts>,
    pub turns: Option<u32>,
    pub duration_ms: Option<u64>,
    pub summary: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub denials: Vec<Denial>,
    #[serde(default)]
    pub skill_changes: Vec<SkillChange>,
    /// What the employee flagged for you with `notify` during the run.
    #[serde(default)]
    pub needs_you: Vec<String>,
    /// When you archived this report: it leaves the Inbox for the Archive.
    #[serde(default)]
    pub archived_at: Option<Ts>,
    /// When you last replied to this report from the Inbox.
    #[serde(default)]
    pub replied_at: Option<Ts>,
    /// For a colleague's Asked run: when its answer reached the asker, through `await_reports` or an
    /// Answered run. Set once, so an answer is delivered exactly once.
    #[serde(default)]
    pub delivered_at: Option<Ts>,
    /// Permission rules you granted for this run only ("Allow once & retry"). They go into the run's
    /// settings and are never saved to the employee.
    #[serde(default)]
    pub allow_once: Vec<String>,
    /// For a Tidy run: the bigger changes it proposes and waits for your approval to make.
    #[serde(default)]
    pub proposed: Option<String>,
    /// Files you attached to a chat message, by name. They live in the employee's `attachments/<run id>/`.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// The build that ran it: app version, commit and channel, Claude Code's version and the signed-in organisation.
    #[serde(default)]
    pub build: Option<RunBuild>,
}

impl Run {
    pub fn new(employee_id: &str, kind: RunKind, prompt: String, now: Ts) -> Run {
        Run {
            id: uuid::Uuid::new_v4().to_string(),
            employee_id: employee_id.to_string(),
            kind,
            prompt,
            state: RunState::Queued,
            created_at: now,
            started_at: None,
            ended_at: None,
            turns: None,
            duration_ms: None,
            summary: None,
            error: None,
            denials: vec![],
            skill_changes: vec![],
            needs_you: vec![],
            archived_at: None,
            replied_at: None,
            delivered_at: None,
            allow_once: vec![],
            proposed: None,
            attachments: vec![],
            build: None,
        }
    }

    pub fn parent_run_id(&self) -> Option<&str> {
        match &self.kind {
            RunKind::Delegated { parent_run_id } | RunKind::Asked { parent_run_id, .. } => Some(parent_run_id),
            _ => None,
        }
    }

    /// For a Reply run: the thread's root, which it answers.
    pub fn reply_to(&self) -> Option<&str> {
        match &self.kind {
            RunKind::Reply { to_run_id, .. } => Some(to_run_id),
            _ => None,
        }
    }

    /// The run this one continues from in a chain of hand-offs: the parent of a delegated or asked
    /// run, and for an Answered run the answer it carries.
    pub fn upstream(&self) -> Option<&str> {
        match &self.kind {
            RunKind::Answered { asked_run_id, .. } => Some(asked_run_id),
            _ => self.parent_run_id(),
        }
    }
}

/// A report and the replies you exchanged on it: the root run and every Reply run chained to it,
/// oldest first. One chip on the employee's panel. In the Inbox and the Archive a thread is a row's
/// lead, with the row's other threads in `related`.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub root: Run,
    pub replies: Vec<Run>,
    /// In the Inbox and the Archive: the other threads filed under the same row, oldest first. That is
    /// the rest of a hand-off between employees, or a schedule's earlier failures in a row. Empty elsewhere.
    pub related: Vec<Thread>,
}

impl Thread {
    pub fn new(root: Run, replies: Vec<Run>) -> Thread {
        Thread { root, replies, related: vec![] }
    }
}

/// Where an employee's ongoing chat stands, for its chip: read from its current session's turns.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatHead {
    pub employee_id: String,
    /// When the newest finished turn ended; None before the first answer.
    pub last_ended_at: Option<Ts>,
    /// A turn is running.
    pub live: bool,
    /// A turn is waiting to start.
    pub queued: bool,
}

/// Every conversation across the office, for the Inbox and each employee's chips, in one read.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversations {
    /// Open threads of every employee, one per root, most recently active first: Inbox rows (with
    /// `related`), open tasks, reports you replied to, and top-level reports still being written.
    pub threads: Vec<Thread>,
    /// The root ids of the threads in `threads` that lead an Inbox row: reports that reached you.
    pub inbox: Vec<String>,
    /// One per current employee.
    pub chats: Vec<ChatHead>,
}

/// What an employee's ongoing chat is told about a thread you archived. Written when you archive it,
/// read by the next chat turn in that session, shown in the chat as a card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Brief {
    pub id: String,
    pub employee_id: String,
    pub root_run_id: String,
    /// What the report was.
    pub label: String,
    pub replies: u32,
    /// One line for the chat timeline: the last thing the employee said in the thread.
    pub excerpt: String,
    /// What the employee reads at the start of their next turn.
    pub text: String,
    pub created_at: Ts,
    /// When a chat turn was launched with it.
    #[serde(default)]
    pub consumed_at: Option<Ts>,
    /// The thread was a task's.
    #[serde(default)]
    pub task: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OwnSkill {
    pub name: String,
    pub description: String,
    pub role: bool,
    pub managed: bool,
    pub files: Vec<String>,
    pub updated_at: Ts,
}

/// What a health check can notice about an employee's skills and notes without a model. Ordered by
/// how much it matters: broken structure first, housekeeping last.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum HealthKind {
    /// A check the employee set for themselves is past its date and still open.
    OverdueCheck,
    /// A skill names another skill that does not exist.
    Dangling,
    /// No path from the role skill leads to this skill, so nobody will reach for it.
    Unreachable,
    /// Lines that read like an unfinished note ("TODO", "not yet verified", "revisit").
    LooseNotes,
    /// The same file was edited again and again within a day.
    Churn,
    /// A notes file repeats the same entry over and over.
    RepeatedEntries,
    /// A skill or reference file is long enough that it may be several things at once.
    TooLong,
    /// Dated lines sit in a procedure, where they age unnoticed.
    DatedLines,
    /// All descriptions together are heavy, and every turn loads them all.
    DescriptionsHeavy,
    /// A description is missing or too long.
    Description,
    /// Generated or leftover files (`__pycache__`, `.bak`) inside a skill folder.
    Junk,
    /// The frontmatter name is not the folder name.
    NameMismatch,
}

impl HealthKind {
    pub fn label(self) -> &'static str {
        match self {
            HealthKind::OverdueCheck => "Check overdue",
            HealthKind::Dangling => "Broken pointer",
            HealthKind::Unreachable => "Not reachable",
            HealthKind::LooseNotes => "Unfinished notes",
            HealthKind::Churn => "Rewritten repeatedly",
            HealthKind::RepeatedEntries => "Diary building up",
            HealthKind::TooLong => "Getting long",
            HealthKind::DatedLines => "Dated lines in a procedure",
            HealthKind::DescriptionsHeavy => "Descriptions are heavy",
            HealthKind::Description => "Description",
            HealthKind::Junk => "Leftover files",
            HealthKind::NameMismatch => "Name mismatch",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HealthFinding {
    pub kind: HealthKind,
    /// The skill it is about; none for the family as a whole or a notes file.
    pub skill: Option<String>,
    /// The file within the skill, or a notes file such as `workspace/state.md`.
    pub file: Option<String>,
    /// One sentence a person can act on.
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillHealth {
    /// Skills in the family, not counting the managed `omega-skill-craft`.
    pub skills: u32,
    /// Characters of every description together: the part loaded on every turn.
    pub description_chars: u32,
    pub findings: Vec<HealthFinding>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CheckState {
    Open,
    Done,
    /// The person you work for, or a tidy, decided it no longer matters.
    Dropped,
}

impl CheckState {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckState::Open => "open",
            CheckState::Done => "done",
            CheckState::Dropped => "dropped",
        }
    }
}

/// Something an employee said they would look at again, tracked so it is not left as a sentence in a
/// file. Without a schedule, no run of theirs happens later unless something starts it; an open check
/// past its date is that something (see `tidy`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    pub employee_id: String,
    /// What to find out.
    pub what: String,
    /// Where or how to find out: a command, a page, who to ask.
    pub how: String,
    pub due_at: Ts,
    pub state: CheckState,
    pub created_at: Ts,
    /// The run that set it.
    pub run_id: Option<String>,
    pub closed_at: Option<Ts>,
    /// What was found, once done.
    pub result: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillRevision {
    #[serde(flatten)]
    pub change: SkillChange,
    pub id: String,
    pub run_id: Option<String>,
    pub at: Ts,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RunEvent {
    Init { model: String, skills: Vec<String> },
    /// A message you sent into the run while it worked.
    User { text: String },
    Text { text: String },
    TextDelta { text: String },
    ToolCall { id: String, tool: String, brief: String },
    ToolResult { id: String, ok: bool, brief: String },
    Delegated { child_run_id: String, employee_id: String, task: String },
    Asked {
        child_run_id: String,
        employee_id: String,
        question: String,
        #[serde(default)]
        task: bool,
    },
    Requested { request_id: String, person_id: String, kind: RequestKind, message: String },
    ProposedHire { proposal_id: String, name: String, title: String },
    SkillChanged { skill: String, file: String, change: ChangeKind },
    /// A sub-agent started (an `Agent` call). `name` is the description it was launched with.
    TaskStarted { task_id: String, tool_use_id: String, name: String },
    /// A sub-agent's latest activity and running totals. Only the newest per task is worth keeping.
    TaskProgress { task_id: String, activity: String, last_tool: Option<String>, tool_uses: u32, tokens: u64, duration_ms: u64 },
    TaskEnded { task_id: String, outcome: TaskOutcome, tool_uses: u32, tokens: u64, duration_ms: u64 },
    Result { ok: bool, text: String, turns: u32, duration_ms: u64 },
    Stderr { text: String },
}

/// How a sub-agent ended.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskOutcome {
    Done,
    Failed,
    /// Killed, or otherwise ended without finishing.
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    pub run: Run,
    pub events: Vec<RunEvent>,
    pub children: Vec<Run>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeStatus {
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub logged_in: bool,
    pub email: Option<String>,
    pub org_name: Option<String>,
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub claude: ClaudeStatus,
    pub account: Option<Account>,
    pub company: Option<Company>,
    pub background: bool,
    pub build: Build,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub max_concurrent_runs: u32,
    pub claude_path: Option<String>,
    /// Informational: the data dir is chosen at startup (OMEGASTAR_DATA_DIR or the OS default).
    pub data_dir: String,
    pub paused: bool,
    /// Absent from settings saved before backups existed.
    pub backup: BackupSettings,
    /// Absent from settings saved before requests were watched.
    pub watch: WatchSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            max_concurrent_runs: 3,
            claude_path: None,
            data_dir: String::new(),
            paused: false,
            backup: BackupSettings::default(),
            watch: WatchSettings::default(),
        }
    }
}

/// How Omega Star watches requests to people for replies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WatchSettings {
    pub hours: BusinessHours,
    pub frequency: CheckFrequency,
    /// Business days until the automatic nudge when the ask has no due date. 0 = never nudge. Default 1. Max 10.
    pub nudge_after_days: u32,
    /// Business days until giving up, counted from max(start, due date, acknowledgement). Default 3. 1..=30,
    /// and greater than nudge_after_days when nudging is on.
    pub give_up_after_days: u32,
}

impl Default for WatchSettings {
    fn default() -> Self {
        WatchSettings { hours: BusinessHours::default(), frequency: CheckFrequency::Normal, nudge_after_days: 1, give_up_after_days: 3 }
    }
}

impl WatchSettings {
    /// Sorts the days and checks every field, as `save_settings` requires.
    pub fn validate(&mut self) -> Result<()> {
        let h = &mut self.hours;
        h.days.sort_unstable();
        h.days.dedup();
        if h.end_minute > 1440 || h.start_minute >= h.end_minute {
            return Err(AppError::invalid("business hours must start before they end, within the day"));
        }
        if h.days.is_empty() || h.days.iter().any(|d| !(1..=7).contains(d)) {
            return Err(AppError::invalid("pick at least one business day (1 = Monday to 7 = Sunday)"));
        }
        if self.nudge_after_days > 10 {
            return Err(AppError::invalid("nudge after must be 0 to 10 business days"));
        }
        if !(1..=30).contains(&self.give_up_after_days) {
            return Err(AppError::invalid("give up after must be 1 to 30 business days"));
        }
        if self.nudge_after_days > 0 && self.give_up_after_days <= self.nudge_after_days {
            return Err(AppError::invalid("give up after must be later than nudge after"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BusinessHours {
    /// Local minutes since midnight.
    pub start_minute: u16,
    pub end_minute: u16,
    /// ISO weekdays, 1 = Monday to 7 = Sunday.
    pub days: Vec<u8>,
}

impl Default for BusinessHours {
    fn default() -> Self {
        BusinessHours { start_minute: 540, end_minute: 1020, days: vec![1, 2, 3, 4, 5] }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CheckFrequency {
    Eager,
    #[default]
    Normal,
    Relaxed,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackupFrequency {
    #[default]
    Off,
    Hourly,
    Daily,
    Weekly,
}

impl BackupFrequency {
    pub fn interval(self) -> Option<chrono::Duration> {
        match self {
            BackupFrequency::Off => None,
            BackupFrequency::Hourly => Some(chrono::Duration::hours(1)),
            BackupFrequency::Daily => Some(chrono::Duration::days(1)),
            BackupFrequency::Weekly => Some(chrono::Duration::weeks(1)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupSettings {
    pub frequency: BackupFrequency,
    /// Where backup zips go. Never inside the office folder.
    pub dir: String,
    /// How many backups to keep; older ones are deleted after each successful backup.
    pub keep: u32,
}

impl Default for BackupSettings {
    fn default() -> Self {
        let dir = dirs::document_dir().or_else(dirs::home_dir).unwrap_or_default().join("Omega Star Backups");
        BackupSettings { frequency: BackupFrequency::Off, dir: dir.to_string_lossy().into_owned(), keep: 7 }
    }
}

/// The last backup attempt, stored in `meta`; `running` and `next_at` are filled in when read.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupStatus {
    pub running: bool,
    /// The last successful backup.
    pub last_at: Option<Ts>,
    pub last_file: Option<String>,
    pub last_bytes: Option<u64>,
    /// Files in the last successful backup that couldn't be read.
    pub skipped: u32,
    pub last_attempt_at: Option<Ts>,
    /// Set when the last attempt failed; cleared by the next success.
    pub last_error: Option<String>,
    pub next_at: Option<Ts>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// 0-100.
    pub percent: f64,
    pub resets_at: Option<Ts>,
    /// The CLI's own wording, e.g. "Sep 28, 2:59pm (Europe/London)".
    pub resets_text: Option<String>,
}

/// Claude subscription usage limits: the 5-hour session window and the 7-day window.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimits {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    pub observed_at: Option<Ts>,
    /// `false` when the account has no subscription limits (API key billing).
    pub subscription: bool,
}

impl Default for UsageLimits {
    fn default() -> Self {
        // Unknown until the first probe; assume a subscription so the UI waits rather than switching to spend.
        UsageLimits { five_hour: None, seven_day: None, observed_at: None, subscription: true }
    }
}

/// A file attached to a chat message: its name and its bytes in base64.
#[derive(Debug, Clone, Deserialize)]
pub struct NewAttachment {
    pub name: String,
    pub data: String,
}

pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn build() -> Build {
        Build { version: "1.2.3".into(), commit: "abc1234567".into(), channel: BuildChannel::Dogfood }
    }

    #[test]
    fn a_run_saved_before_builds_were_stamped_loads() {
        let mut run = serde_json::to_value(Run::new("e1", RunKind::Manual, "Do the thing".into(), Utc::now())).unwrap();
        run.as_object_mut().unwrap().remove("build");
        let run: Run = serde_json::from_value(run).unwrap();
        assert_eq!(run.build, None);
    }

    #[test]
    fn a_run_build_round_trips_flat() {
        let b = RunBuild { app: build(), claude: Some("2.1.0".into()), org: Some("Example Org".into()) };
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v, json!({ "version": "1.2.3", "commit": "abc1234567", "channel": "dogfood", "claude": "2.1.0", "org": "Example Org" }));
        assert_eq!(serde_json::from_value::<RunBuild>(v).unwrap(), b);
        let bare: RunBuild = serde_json::from_value(json!({ "version": "1.2.3", "commit": "abc1234567", "channel": "dogfood" })).unwrap();
        assert_eq!(bare, RunBuild { app: build(), claude: None, org: None });
    }

    #[test]
    fn a_build_channel_is_lower_case() {
        assert_eq!(serde_json::to_value(BuildChannel::Dogfood).unwrap(), json!("dogfood"));
    }
}
