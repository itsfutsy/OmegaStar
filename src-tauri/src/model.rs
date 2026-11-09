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
    /// Your display image (see `check_image`).
    #[serde(default)]
    pub image: Option<String>,
}

impl Company {
    const DEFAULT_TITLE: &'static str = "CEO";

    /// Before onboarding: no name, the default title, nobody above you.
    pub fn unset() -> Company {
        Company { name: String::new(), ceo_name: String::new(), title: Self::DEFAULT_TITLE.into(), email: None, reports_to: None, image: None }
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
    /// Tidies their skills and notes on their own when the health check finds something. On unless you turn it off.
    #[serde(default = "tidy_on")]
    pub auto_tidy: bool,
    pub workspaces: Vec<String>,
    pub hue: u16,
    /// Their display image (see `check_image`).
    #[serde(default)]
    pub image: Option<String>,
}

/// Employees hired before the browser switch existed could already drive Chrome, so they keep it.
fn browser_on() -> bool {
    true
}

/// Tidying on their own is on unless you turn it off, including for employees saved before it was.
fn tidy_on() -> bool {
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
    /// On unless you turn it off.
    #[serde(default = "tidy_on")]
    pub auto_tidy: bool,
    pub workspaces: Vec<String>,
    pub hue: u16,
    /// Their display image, shown in place of their initials (see `check_image`). None: their initials on their colour.
    #[serde(default)]
    pub image: Option<String>,
    /// The session of the one long chat older builds kept. Nothing reads it now, but a hire still gets one of
    /// its own, so an older build reading this employee never shares a session with another.
    pub chat_session: String,
    pub hired_at: Ts,
}

/// Largest display image, as its data URL. The app makes them a few dozen KB at most.
const MAX_IMAGE_CHARS: usize = 256 * 1024;

/// A display image, which anyone on the chart (you, an employee, a person) may have in place of their initials: a
/// small picture as a `data:image/` URL, so the app never fetches it from anywhere.
pub fn check_image(image: Option<&str>) -> Result<()> {
    match image {
        Some(i) if !i.starts_with("data:image/") || i.len() > MAX_IMAGE_CHARS => {
            Err(AppError::invalid("the display image must be a picture under 256 KB"))
        }
        _ => Ok(()),
    }
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
        check_image(self.image.as_deref())
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
            image: self.image,
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
            image: self.image.clone(),
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
    /// What they are working on: every queued or running run of theirs, running first, oldest first within each.
    pub live: Vec<Run>,
    /// How many runs they have going at once, and how many more wait to start.
    pub working: u32,
    pub waiting: u32,
    pub next_fire: Option<Ts>,
    /// Their latest work-lane run. `Presence::Attention` is decided from it, so it stays work-only.
    pub last_run: Option<Run>,
    /// When they last finished anything, in either lane: what "last worked" means.
    pub last_worked_at: Option<Ts>,
    pub status: EmployeeStatus,
}

/// How an employee is doing at a glance, for the org chart and their panel.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmployeeStatus {
    /// Their latest weekly review's number, as "metric value (change)".
    pub number: Option<String>,
    /// The first line of their last finished report, and when it finished.
    pub last_report: Option<Dated>,
    /// What they do next: their next schedule slot's label or their soonest open check, and when.
    pub next: Option<Dated>,
}

/// A line of text and the time it is about.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dated {
    pub text: String,
    pub at: Ts,
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
    /// Their display image (see `check_image`).
    #[serde(default)]
    pub image: Option<String>,
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
    /// Their display image, shown in place of their initials (see `check_image`). None: their initials.
    #[serde(default)]
    pub image: Option<String>,
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
        check_image(self.image.as_deref())
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
            image: self.image,
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
            image: self.image.clone(),
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Misfire {
    #[default]
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
    /// Quiet routines reach the Inbox only when the run calls `notify` or does not succeed.
    #[serde(default)]
    pub quiet: bool,
    /// The model and effort the routine runs on; `None` = the employee's own.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    /// Only during business hours: a slot outside them is skipped.
    #[serde(default)]
    pub active_hours: bool,
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
    /// Kept only so older builds, which require it, can read schedules this build saves. Always CatchUp; nothing reads it.
    #[serde(default)]
    pub misfire: Misfire,
    #[serde(default)]
    pub quiet: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    /// The weekly review Omega Star seeds for every employee.
    #[serde(default)]
    pub review: bool,
    /// Runs only during business hours (`WatchSettings::hours`): a slot outside them is skipped, not caught up.
    #[serde(default)]
    pub active_hours: bool,
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

/// The schedule_id prefix of a due check's run (see `RunKind::check_id`).
pub const CHECK_PREFIX: &str = "check:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RunKind {
    Manual,
    /// A turn of the one long chat older builds had. Nothing starts one now; old ones still load.
    Chat,
    Scheduled {
        schedule_id: String,
        slot: Ts,
        label: String,
        /// Copied from the schedule when the slot fires.
        #[serde(default)]
        quiet: bool,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<Effort>,
        #[serde(default)]
        review: bool,
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
    /// Brings a colleague's answer or a report (the finished Asked or Delegated run `asked_run_id`, by
    /// employee `from`) to whoever handed it over, once their own run had ended before it arrived.
    Answered {
        asked_run_id: String,
        from: String,
        task: bool,
        /// A turn in the one long chat of an older build. Nothing sets it now.
        #[serde(default)]
        chat: bool,
        /// The asker handed the work over in a conversation you started, so this is its next turn: the id of
        /// that conversation's first message.
        #[serde(default)]
        thread: Option<String>,
    },
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
        /// The reply answers a card, written by Omega Star from your tap.
        #[serde(default)]
        card: bool,
    },
    /// A conversation you started with the employee, under the hood a task: a thread of its own, so it runs at
    /// the same time as their other work. `label` names it. `from` and `context` held what an older build's chat
    /// said before it began; nothing sets them now.
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
    /// The lane a run is stored in, written once when it is queued: Chat only for the turns of an older
    /// build's chat, so those rows keep to themselves.
    pub fn lane(&self) -> Lane {
        match self {
            RunKind::Chat | RunKind::Answered { chat: true, .. } => Lane::Chat,
            _ => Lane::Work,
        }
    }

    /// Your conversations with the employee (a thread you started, a reply in a thread, an answer that came
    /// back into one, and an older build's chat) run beside everything else. The rest is background work,
    /// which an employee does as many pieces of at once as the office's cap allows.
    pub fn is_conversation(&self) -> bool {
        matches!(self, RunKind::Chat | RunKind::Task { .. } | RunKind::Reply { .. } | RunKind::Answered { chat: true, .. } | RunKind::Answered { thread: Some(_), .. })
    }

    /// A due check runs as a Scheduled run whose schedule_id is check:<check id>; no schedules row has that id.
    pub fn check_id(&self) -> Option<&str> {
        match self {
            RunKind::Scheduled { schedule_id, .. } => schedule_id.strip_prefix(CHECK_PREFIX),
            _ => None,
        }
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

/// One thing a run asks you with the answer built in: a choice, a list to approve, a chore or a task only you can
/// do, a question only you can answer, or news to read (`notify`). A run can ask several.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    /// The question, as the Inbox item's title.
    pub message: String,
    /// One line on why it matters, under the question.
    #[serde(default)]
    pub why: Option<String>,
    /// What it is about, so the same question from anyone is one Inbox item and asking again replaces it:
    /// `ask:<what the employee named it>`, `chore:<subject>`, or for a proposal Omega Star raised `company:`,
    /// `owns:` or `company-page:` and what it is about. None on cards asked before keys.
    #[serde(default)]
    pub key: Option<String>,
    /// The file it is about, e.g. a draft to approve, which the card shows as it is now: a full path in the
    /// employee's folder or one of their workspaces.
    #[serde(default)]
    pub file: Option<String>,
    /// A web page it is about, e.g. the pull request: http or https only.
    #[serde(default)]
    pub link: Option<String>,
    /// Up to three images it shows, as full paths in the employee's folder or one of their workspaces.
    #[serde(default)]
    pub images: Vec<String>,
    pub ask: CardAsk,
    #[serde(default)]
    pub answer: Option<CardAnswer>,
    /// You said Later: the card opens again then, by itself.
    #[serde(default)]
    pub back_at: Option<Ts>,
    /// Times it came back after a Later.
    #[serde(default)]
    pub times_back: u32,
    /// You said Later again once it came back: it waits in your Parked list, answerable, and never comes back by itself.
    #[serde(default)]
    pub parked: bool,
    /// What Omega Star does when you go ahead: the card is its proposal, not an employee's ask.
    #[serde(default)]
    pub proposal: Option<Proposal>,
}

impl Card {
    pub fn new(message: &str, why: Option<String>, key: Option<String>, ask: CardAsk) -> Card {
        let key = match &ask {
            CardAsk::Chore { subject, .. } => Some(format!("chore:{subject}")),
            _ => key,
        };
        Card {
            message: message.to_string(),
            why,
            key,
            file: None,
            link: None,
            images: vec![],
            ask,
            answer: None,
            back_at: None,
            times_back: 0,
            parked: false,
            proposal: None,
        }
    }

    /// Waiting on your answer. News waits on nothing: it is only unseen until you see it.
    pub fn is_open(&self) -> bool {
        self.answer.is_none() && self.ask != CardAsk::News
    }

    /// No answer yet: open, or news you have not seen.
    pub fn pending(&self) -> bool {
        self.answer.is_none()
    }

    /// You put it off with Later: it comes back by itself (`back_at`), or waits in your Parked list.
    pub fn put_off(&self) -> bool {
        self.back_at.is_some() || self.parked
    }

    /// Still waiting on you: no answer yet, or put off with Later. A newer card about the same thing replaces it,
    /// `close_ask` withdraws it, and any answer settles it.
    pub fn waiting(&self) -> bool {
        self.pending() || self.put_off()
    }

    /// News you have not seen yet.
    pub fn unseen_news(&self) -> bool {
        self.answer.is_none() && self.ask == CardAsk::News
    }

    /// What the card is about: its key, or for a chore asked before keys `chore:<subject>`.
    pub fn topic(&self) -> Option<String> {
        match (&self.key, &self.ask) {
            (Some(k), _) => Some(k.clone()),
            (None, CardAsk::Chore { subject, .. }) => Some(format!("chore:{subject}")),
            _ => None,
        }
    }
}

/// What a card Omega Star raised for an employee does when you go ahead.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Proposal {
    /// Adds `line` as a bullet under the company page's `## <section>` (`propose_company_line`).
    CompanyLine { section: String, line: String },
    /// Writes the employee's version of what they own, saved as the card's file, into their what-you-own.md.
    Owns,
    /// Writes the employee's version of the company page, saved as the card's file, into the page.
    CompanyPage,
}

/// A run's cards as stored: a list, or one card (or none) as a run saved before a run could ask several.
fn cards_compat<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<Card>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Many(Vec<Card>),
        One(Box<Card>),
    }
    Ok(match Option::<Stored>::deserialize(d)? {
        None => vec![],
        Some(Stored::Many(v)) => v,
        Some(Stored::One(c)) => vec![*c],
    })
}

/// The number an employee answers for, as their weekly review measured it (`report_number`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    /// What is counted, e.g. "Open support tickets".
    pub metric: String,
    /// This week's value, e.g. "0" or "12".
    pub value: String,
    /// Against last week, e.g. "+3%", "down 1", "first count".
    pub change: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CardAsk {
    /// Up to four buttons; `recommended` is the index of the one the employee recommends.
    Choices { choices: Vec<String>, recommended: usize },
    /// A list you tick one by one; `verb` is what a tick means, e.g. "Post".
    Items { verb: String, items: Vec<CardItem> },
    /// `chrome` or `sign-in:<site>`: every run waiting on the same subject carries on when you press Done. `url`: the
    /// page to do it on.
    Chore {
        subject: String,
        #[serde(default)]
        url: Option<String>,
    },
    /// A to-do only you can do: the steps, and the page to do it on. Done or Can't.
    Task {
        steps: String,
        #[serde(default)]
        url: Option<String>,
    },
    /// A fact only you know, answered in your own words.
    Question,
    /// Something you should know that needs no decision. Seen sends nothing.
    News,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardItem {
    pub id: String,
    /// What would go out, in full, e.g. the post.
    pub text: String,
    /// One line on what it is about, e.g. the review it answers, shown above the text.
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardAnswer {
    pub at: Ts,
    pub how: AnswerHow,
    /// The fixed answer sent to the employee, e.g. "Chose: Not now."; empty when nothing was sent.
    pub text: String,
    /// Not sent yet: its run still asks something else, and hears all your answers in one message once nothing is
    /// left open.
    #[serde(default)]
    pub held: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnswerHow {
    /// You pressed one of its buttons.
    Tapped,
    /// You answered in your own words on the card. An older build counted any reply in the thread as one.
    Typed,
    /// You pressed Later (Not now): busy, not a no. The card comes back once by itself (`Card::back_at`), then is parked.
    /// On a chore it sends nothing and never comes back.
    Dismissed,
    /// You pressed No: drop it.
    Rejected,
    /// A newer card from the same routine, or about the same thing, took its place.
    Replaced,
    /// The employee closed it: it was settled without you.
    Withdrawn,
}

/// Your answer to a card, from the Inbox.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CardReply {
    Choice { index: usize },
    Items { picks: Vec<ItemPick> },
    /// You did the chore or the task.
    Done,
    /// Not now: on a chore it sends nothing; on anything else it is Later until 9 am tomorrow.
    Dismiss,
    /// No, on a decision.
    Reject,
    /// An answer in your own words, to this card only.
    Text { text: String },
    /// A question about the card: it goes to the employee now, and the card stays open.
    Ask { text: String },
    /// Busy, not a no: the card comes back by itself at `until`, or is parked if it came back once already.
    Later { until: Ts },
    /// You cannot do the task.
    Cant,
    /// You read the news.
    Seen,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemPick {
    pub id: String,
    pub tick: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub employee_id: String,
    pub kind: RunKind,
    pub prompt: String,
    /// Omega Star wrote this prompt from something you did or something that came back (an approval, a
    /// returned report, a card answer); it is not your own words, except where it quotes them.
    #[serde(default)]
    pub omega_wrote: bool,
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
    /// What needs you from this run: each card's message, and flags Omega Star raised.
    #[serde(default)]
    pub needs_you: Vec<String>,
    /// The decisions the run asked you with cards, and your answers. Each message is also in `needs_you`.
    #[serde(default, alias = "card", deserialize_with = "cards_compat")]
    pub cards: Vec<Card>,
    /// When you archived this report: it leaves the Inbox for the Archive.
    #[serde(default)]
    pub archived_at: Option<Ts>,
    /// For a thread's first run: the name the employee gave the thread with `name_thread`. It wins over the
    /// name its kind gives it.
    #[serde(default)]
    pub title: Option<String>,
    /// When you last replied to this report from the Inbox.
    #[serde(default)]
    pub replied_at: Option<Ts>,
    /// You wrote into it while it worked or waited to start, so its report waits for you in the Inbox like a reply,
    /// whatever kind of run it is.
    #[serde(default)]
    pub steered: bool,
    /// For a reply: your question about one of the thread's cards (`CardReply::Ask`). Your own words, but it leaves the
    /// thread as it was: it never makes it a conversation you are in (`threads::talking`), so a later tap still files it.
    #[serde(default)]
    pub about_card: bool,
    /// For an Asked or Delegated run: when its answer or report reached whoever handed it over, through
    /// await_reports or an Answered run. Set once, so it is delivered exactly once.
    #[serde(default)]
    pub delivered_at: Option<Ts>,
    /// For a due check that came back into the conversation it was set in, as a reply Omega Star wrote there:
    /// the check's id. A check's own run names it in its kind instead (`RunKind::check_id`).
    #[serde(default)]
    pub check: Option<String>,
    /// Permission rules you granted for this run only ("Allow once & retry"). They go into the run's
    /// settings and are never saved to the employee.
    #[serde(default)]
    pub allow_once: Vec<String>,
    /// For a Tidy run: the bigger changes it proposes and waits for your approval to make.
    #[serde(default)]
    pub proposed: Option<String>,
    /// For a Tidy run: when you approved what it proposed. Replying to it is no decision.
    #[serde(default)]
    pub approved_at: Option<Ts>,
    /// Files you attached to a message, by name. They live in the employee's `attachments/<run id>/`.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// The build that ran it: app version, commit and channel, Claude Code's version and the signed-in organisation.
    #[serde(default)]
    pub build: Option<RunBuild>,
    /// Times the run went back to the queue because a limit stopped it: the usage limit, or a rate limit.
    #[serde(default)]
    pub limit_retries: u32,
    /// Of `limit_retries`, the times Claude or a proxy in front of it turned requests away for now (HTTP 429 or 529).
    /// Counted apart, so a rate limit never uses up the retries the usage limit gets.
    #[serde(default)]
    pub transient_retries: u32,
    /// It failed because the usage limit stopped it once too often: nothing you can do, so it never needs you.
    #[serde(default)]
    pub limited: bool,
    /// Times the run went back to the queue because Omega Star stopped while it was in progress.
    #[serde(default)]
    pub restarts: u32,
    /// It last went back to the queue because Omega Star stopped, not the usage limit; its next attempt is told which.
    #[serde(default)]
    pub restarted: bool,
    /// Where its next attempt's lines start in its transcript, which every attempt appends to: set when it goes back to
    /// the queue, so a restart that finds that attempt working judges it by its own lines alone.
    #[serde(default)]
    pub attempt_from: u64,
    /// For a weekly review: the number it measured, for the team's scoreboard.
    #[serde(default)]
    pub score: Option<Score>,
    /// The context size in tokens after its last top-level message.
    #[serde(default)]
    pub context_tokens: Option<u64>,
    /// What the run cost in US dollars as Claude Code reckons it, summed over its result segments and attempts.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// Not to start before then: a hand-off Omega Star paused to keep its chain short, an answer queued again
    /// after it failed, or the reports held for a manager.
    #[serde(default)]
    pub not_before: Option<Ts>,
    /// For a delegated or asked run: the run its chain of hand-offs started from, one a person or a schedule
    /// started, or a paused hand-off, which starts a chain of its own. The loop guard counts by it.
    #[serde(default)]
    pub chain: Option<String>,
    /// For an answered run that brings a manager the reports held for them: the delegated runs it carries.
    #[serde(default)]
    pub reports: Vec<String>,
    /// For a held report: the run whose opening note brought it to the manager instead.
    #[serde(default)]
    pub noted_in: Option<String>,
    /// For an answered run: times it was queued again because an earlier attempt failed.
    #[serde(default)]
    pub redelivered: u32,
}

impl Run {
    pub fn new(employee_id: &str, kind: RunKind, prompt: String, now: Ts) -> Run {
        Run {
            id: uuid::Uuid::new_v4().to_string(),
            employee_id: employee_id.to_string(),
            kind,
            prompt,
            omega_wrote: false,
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
            cards: vec![],
            archived_at: None,
            title: None,
            replied_at: None,
            steered: false,
            about_card: false,
            delivered_at: None,
            check: None,
            allow_once: vec![],
            proposed: None,
            approved_at: None,
            attachments: vec![],
            build: None,
            limit_retries: 0,
            transient_retries: 0,
            limited: false,
            restarts: 0,
            restarted: false,
            attempt_from: 0,
            score: None,
            context_tokens: None,
            cost_usd: None,
            not_before: None,
            chain: None,
            reports: vec![],
            noted_in: None,
            redelivered: 0,
        }
    }

    /// The cards still waiting on your answer. Unseen news waits on nothing, so it holds back no answer and no Done.
    pub fn open_cards(&self) -> impl Iterator<Item = &Card> {
        self.cards.iter().filter(|c| c.is_open())
    }

    pub fn has_open_card(&self) -> bool {
        self.open_cards().next().is_some()
    }

    /// It has news you have not seen, which keeps its row in the Inbox.
    pub fn has_news(&self) -> bool {
        self.cards.iter().any(Card::unseen_news)
    }

    /// What needs you that is not a card: flags Omega Star raised, and `notify` messages from before cards.
    pub fn flags(&self) -> impl Iterator<Item = &String> {
        self.needs_you.iter().filter(|m| !self.cards.iter().any(|c| c.message == **m))
    }

    pub fn parent_run_id(&self) -> Option<&str> {
        match &self.kind {
            RunKind::Delegated { parent_run_id } | RunKind::Asked { parent_run_id, .. } => Some(parent_run_id),
            _ => None,
        }
    }

    /// The root of the thread this run is a later turn of: the run a Reply answers, or the first message of
    /// the conversation an answer came back into.
    pub fn reply_to(&self) -> Option<&str> {
        match &self.kind {
            RunKind::Reply { to_run_id, .. } | RunKind::Answered { thread: Some(to_run_id), .. } => Some(to_run_id),
            _ => None,
        }
    }

    /// The conversation you started that this run is a turn of, by its first message's id: a task itself, a
    /// reply in one, or an answer that came back into one. None for anything else.
    pub fn conversation_root(&self) -> Option<&str> {
        match &self.kind {
            RunKind::Task { .. } => Some(&self.id),
            RunKind::Reply { to_run_id, task: true, .. } | RunKind::Answered { thread: Some(to_run_id), .. } => Some(to_run_id),
            _ => None,
        }
    }

    /// The check this run does: a due check's own run, or its turn in the conversation it was set in.
    pub fn check_id(&self) -> Option<&str> {
        self.kind.check_id().or(self.check.as_deref())
    }

    /// A turn of a conversation with you, which never waits for or counts against the cap on runs at once: a
    /// conversation you started, a reply you typed, an answer that came back into a conversation, and a turn of
    /// an older build's chat. A reply Omega Star wrote from your tap (a card's answer, an approval) is capped:
    /// one tap can answer many runs at once.
    pub fn uncapped(&self) -> bool {
        self.kind.is_conversation() && !(self.omega_wrote && matches!(self.kind, RunKind::Reply { .. }))
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

/// A report and the replies you exchanged on it: the root run and every later turn chained to it
/// (`Run::reply_to`), oldest first. One chip on the employee's panel. In the Inbox and the Archive a thread is a row's
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

/// Every conversation across the office, for the Inbox and each employee's chips, in one read.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversations {
    /// Open threads of every employee, one per root, most recently active first: Inbox rows (with
    /// `related`), open conversations you started, reports you replied to, and top-level reports still being written.
    pub threads: Vec<Thread>,
    /// The root ids of the threads in `threads` that lead an Inbox row: reports that reached you.
    pub inbox: Vec<String>,
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
    /// No run has used the skill for a month.
    Unused,
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
            HealthKind::Unused => "Not used lately",
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

/// The page every employee reads at the start of every run, with what is worth a look on it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyPage {
    pub text: String,
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
/// file. An open check past its date starts a run of its own, see `Office::tick`.
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

/// News a colleague sent with `ask_colleague` kind info. It starts no run: the receiver reads it at the top of
/// their next run, and then it is read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct News {
    pub id: String,
    /// Who it is for.
    pub employee_id: String,
    /// The employee who sent it.
    pub from: String,
    pub text: String,
    pub created_at: Ts,
    #[serde(default)]
    pub read_at: Option<Ts>,
    /// The run whose opening note showed it, so a run that never started gives it back for the next one.
    #[serde(default)]
    pub read_in: Option<String>,
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
    /// A message you sent into the run while it worked. `id` tags it on its way in, so `Heard` can say when the
    /// employee took it; a message from an older build has none and counts as taken where it was sent.
    User {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// The employee took your message `id` (a `User`), at their next step: it reads from here, as Claude Code
    /// shows a queued message once it goes in.
    Heard { id: String },
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
    /// A page the employee showed (`show`), saved as `runs/<run id>/visuals/<id>.html`.
    Visual { id: String, title: String },
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
    /// The run's folder was deleted after `AppSettings::transcript_days`: no events, its report is on the run.
    pub transcript_gone: bool,
}

/// What runs did and used since a time, for Activity's digest of today and its usage this week. Usage is what
/// runs cost in US dollars at API list price, as Claude Code reckons it; a run with no cost recorded adds nothing.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunUsage {
    pub employees: Vec<EmployeeUsage>,
    pub routines: Vec<RoutineUsage>,
}

/// An employee's finished runs since a time.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmployeeUsage {
    pub employee_id: String,
    pub runs: u32,
    /// Cards their runs raised for you.
    pub decisions: u32,
    /// Work they handed to colleagues: the asked and delegated runs their runs started.
    pub handoffs: u32,
    pub failed: u32,
    pub cost_usd: f64,
}

/// One routine's finished runs since a time. Weekly reviews and due checks are not routines.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RoutineUsage {
    pub schedule_id: String,
    pub employee_id: String,
    pub label: String,
    pub quiet: bool,
    pub runs: u32,
    /// Runs that reached someone: notified you or handed work to a colleague.
    pub reached: u32,
    pub failed: u32,
    /// Runs that did something: reached someone, changed a skill or failed. The rest did nothing.
    pub useful: u32,
    pub cost_usd: f64,
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
    /// Absent from settings saved before the CLI proxy existed.
    pub proxy: ProxySettings,
    /// Days a finished run's folder (its transcript, log and the pages it showed) is kept; `None` = forever.
    /// Its row and report stay either way.
    pub transcript_days: Option<u32>,
    /// Absent from settings saved before notifications could be batched.
    pub notifications: NotificationSettings,
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
            proxy: ProxySettings::default(),
            transcript_days: None,
            notifications: NotificationSettings::default(),
        }
    }
}

/// How desktop notifications reach you (`notices::Notices`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationSettings {
    pub mode: NotifyMode,
    /// No notification shows between these times; one summary of what came in shows when they end. `None` = never quiet.
    pub quiet: Option<QuietHours>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NotifyMode {
    /// Each one as it comes.
    All,
    /// An employee's decisions in the 90 seconds after their first show as one.
    #[default]
    Batched,
    /// None at all.
    Off,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuietHours {
    /// Local minutes since midnight, under 1440. A start later than the end runs overnight, e.g. 10 pm to 8 am.
    pub start_minute: u16,
    pub end_minute: u16,
}

impl QuietHours {
    /// Whether `minute` (local minutes since midnight) falls in them. Pure.
    pub fn contains(&self, minute: u16) -> bool {
        if self.start_minute <= self.end_minute {
            self.start_minute <= minute && minute < self.end_minute
        } else {
            minute >= self.start_minute || minute < self.end_minute
        }
    }
}

/// A CLI proxy (CLIProxyAPI) that Claude Code goes through instead of the signed-in account, so runs
/// share the proxy's pool of accounts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxySettings {
    /// Its address, e.g. `http://127.0.0.1:8317`. Empty = no proxy.
    pub url: String,
    /// One of its API keys, which Claude Code sends in an `x-api-key` header.
    pub api_key: String,
    /// Its management key, to read each account's usage limits. Empty = none are shown.
    pub management_key: String,
}

impl ProxySettings {
    /// The variables that send Claude Code through the proxy; none when there is no proxy. The key
    /// goes in a header because ANTHROPIC_AUTH_TOKEN would replace the claude.ai sign-in, and
    /// Claude in Chrome and claude.ai connectors only load with that sign-in.
    pub fn env(&self) -> Vec<(String, String)> {
        if self.url.is_empty() {
            return vec![];
        }
        let mut env = vec![("ANTHROPIC_BASE_URL".to_string(), self.url.clone())];
        if !self.api_key.is_empty() {
            env.push(("ANTHROPIC_CUSTOM_HEADERS".into(), format!("x-api-key: {}", self.api_key)));
        }
        env
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
    /// backup::fingerprint() of the office at the last successful backup.
    pub fingerprint: Option<String>,
    /// The last time an automatic backup was due but nothing had changed.
    pub checked_at: Option<Ts>,
    /// Large files and build folders the last successful backup left out.
    pub left_out: u32,
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
    /// The office waits for the usage limit to reset until then: no run starts.
    pub held_until: Option<Ts>,
    /// Claude Code is signed out or on another account than the one set up: routines wait.
    pub account_issue: Option<AccountIssue>,
    /// Why the CLI proxy could not report its accounts' usage, when it could not.
    pub proxy_issue: Option<String>,
}

impl Default for UsageLimits {
    fn default() -> Self {
        // Unknown until the first probe; assume a subscription so the UI waits rather than switching to spend.
        UsageLimits { five_hour: None, seven_day: None, observed_at: None, subscription: true, held_until: None, account_issue: None, proxy_issue: None }
    }
}

/// Why routines are paused for the account, and the account Claude Code is signed in to instead.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountIssue {
    pub message: String,
    pub signed_in_as: Option<String>,
}

/// A file attached to a message: its name and its bytes in base64.
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
    use serde_json::{json, Value};

    fn build() -> Build {
        Build { version: "1.2.3".into(), commit: "abc1234567".into(), channel: BuildChannel::Dogfood }
    }

    fn ada_draft() -> EmployeeDraft {
        EmployeeDraft {
            name: "Ada".into(),
            title: "Analyst".into(),
            description: String::new(),
            objective: String::new(),
            manager_id: None,
            skills: SkillGrant { builtins: false, keys: vec![] },
            clearance: Clearance::Supervised,
            allow_rules: vec![],
            model: None,
            effort: None,
            browser: false,
            auto_tidy: false,
            workspaces: vec![],
            hue: 0,
            image: None,
        }
    }

    #[test]
    fn a_display_image_is_only_ever_a_small_embedded_picture() {
        let with = |image: String| EmployeeDraft { image: Some(image), ..ada_draft() }.validate();
        assert!(with("data:image/webp;base64,UklGR".into()).is_ok());
        assert!(with("https://example.com/face.png".into()).is_err(), "never fetched from the network");
        assert!(with(format!("data:image/png;base64,{}", "A".repeat(300 * 1024))).is_err(), "too large");
    }

    #[test]
    fn tidy_on_its_own_is_on_unless_turned_off() {
        let draft = ada_draft();
        let employee = draft.clone().into_employee("e1".into(), "ada".into(), "chat-ada".into(), Utc::now());
        let mut draft = serde_json::to_value(draft).unwrap();
        let mut employee = serde_json::to_value(employee).unwrap();
        assert!(!serde_json::from_value::<EmployeeDraft>(draft.clone()).unwrap().auto_tidy, "an explicit off stays off");
        assert!(!serde_json::from_value::<Employee>(employee.clone()).unwrap().auto_tidy, "an explicit off stays off");
        draft.as_object_mut().unwrap().remove("autoTidy");
        employee.as_object_mut().unwrap().remove("autoTidy");
        assert!(serde_json::from_value::<EmployeeDraft>(draft).unwrap().auto_tidy);
        assert!(serde_json::from_value::<Employee>(employee).unwrap().auto_tidy);
    }

    #[test]
    fn an_employee_saved_with_their_own_lanes_still_loads() {
        let mut employee = serde_json::to_value(Employee {
            id: "e1".into(),
            slug: "ada".into(),
            name: "Ada".into(),
            title: "Analyst".into(),
            description: String::new(),
            objective: String::new(),
            manager_id: None,
            skills: SkillGrant::default(),
            clearance: Clearance::Supervised,
            allow_rules: vec![],
            model: None,
            effort: None,
            browser: false,
            auto_tidy: true,
            workspaces: vec![],
            hue: 0,
            image: None,
            chat_session: "chat-ada".into(),
            hired_at: Utc::now(),
        })
        .unwrap();
        employee.as_object_mut().unwrap().insert("lanes".into(), 2.into());
        assert_eq!(serde_json::from_value::<Employee>(employee).unwrap().name, "Ada", "older builds kept a lane count per employee");
    }

    #[test]
    fn a_run_saved_before_builds_were_stamped_loads() {
        let mut run = serde_json::to_value(Run::new("e1", RunKind::Manual, "Do the thing".into(), Utc::now())).unwrap();
        run.as_object_mut().unwrap().remove("build");
        let run: Run = serde_json::from_value(run).unwrap();
        assert_eq!(run.build, None);
    }

    #[test]
    fn a_run_saved_before_cards_loads() {
        let mut run = serde_json::to_value(Run::new("e1", RunKind::Reply { to_run_id: "r1".into(), label: "Sweep".into(), task: false, card: true }, "Chose: Not now.".into(), Utc::now())).unwrap();
        run.as_object_mut().unwrap().remove("cards");
        run["kind"].as_object_mut().unwrap().remove("card");
        let run: Run = serde_json::from_value(run).unwrap();
        assert!(run.cards.is_empty());
        assert_eq!(run.kind, RunKind::Reply { to_run_id: "r1".into(), label: "Sweep".into(), task: false, card: false });
    }

    #[test]
    fn a_run_saved_with_one_card_loads_it_as_its_cards() {
        let mut run = serde_json::to_value(Run::new("e1", RunKind::Manual, "Go".into(), Utc::now())).unwrap();
        let o = run.as_object_mut().unwrap();
        o.remove("cards");
        o.insert("card".into(), json!({ "message": "Open Chrome.", "ask": { "type": "chore", "subject": "chrome" }, "answer": null }));
        let loaded: Run = serde_json::from_value(run.clone()).unwrap();
        assert_eq!(loaded.cards.len(), 1);
        assert_eq!(loaded.cards[0].topic().as_deref(), Some("chore:chrome"), "a chore asked before keys still has its topic");
        run["card"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<Run>(run).unwrap().cards.is_empty());
    }

    #[test]
    fn a_card_reads_its_own_shapes() {
        let card = Card::new("Boost it?", None, None, CardAsk::Choices { choices: vec!["Not now".into(), "Try it".into()], recommended: 0 });
        let v = serde_json::to_value(&card).unwrap();
        assert_eq!(
            v,
            json!({
                "message": "Boost it?", "why": null, "key": null, "file": null, "link": null, "images": [],
                "ask": { "type": "choices", "choices": ["Not now", "Try it"], "recommended": 0 }, "answer": null,
                "backAt": null, "timesBack": 0, "parked": false, "proposal": null
            })
        );
        assert_eq!(serde_json::from_value::<Card>(v).unwrap(), card);
        let chrome = CardAsk::Chore { subject: "chrome".into(), url: None };
        assert_eq!(Card::new("Open Chrome.", None, Some("ignored".into()), chrome).key.as_deref(), Some("chore:chrome"));
        let reply: CardReply = serde_json::from_value(json!({ "type": "items", "picks": [{ "id": "d1", "tick": true }] })).unwrap();
        assert_eq!(reply, CardReply::Items { picks: vec![ItemPick { id: "d1".into(), tick: true, note: None }] });
        assert_eq!(serde_json::from_value::<CardReply>(json!({ "type": "done" })).unwrap(), CardReply::Done);
    }

    #[test]
    fn the_new_card_shapes_read_as_the_inbox_sends_them() {
        let ask = |v: Value| serde_json::from_value::<CardAsk>(v).unwrap();
        assert_eq!(ask(json!({ "type": "task", "steps": "Renew the domain.", "url": "https://example.com" })), CardAsk::Task { steps: "Renew the domain.".into(), url: Some("https://example.com".into()) });
        assert_eq!(ask(json!({ "type": "question" })), CardAsk::Question);
        assert_eq!(ask(json!({ "type": "news" })), CardAsk::News);
        assert_eq!(ask(json!({ "type": "chore", "subject": "chrome" })), CardAsk::Chore { subject: "chrome".into(), url: None }, "a chore saved before urls");
        assert_eq!(serde_json::to_value(CardAsk::Chore { subject: "sign-in:acme".into(), url: None }).unwrap(), json!({ "type": "chore", "subject": "sign-in:acme", "url": null }));
        assert_eq!(serde_json::to_value(CardAsk::Question).unwrap(), json!({ "type": "question" }));
        let line = Proposal::CompanyLine { section: "Who to ask".into(), line: "Bo signs off refunds.".into() };
        assert_eq!(serde_json::to_value(&line).unwrap(), json!({ "type": "companyLine", "section": "Who to ask", "line": "Bo signs off refunds." }));
        assert_eq!(serde_json::to_value(Proposal::Owns).unwrap(), json!({ "type": "owns" }));
        assert_eq!(serde_json::to_value(Proposal::CompanyPage).unwrap(), json!({ "type": "companyPage" }));
        let reply = |v: Value| serde_json::from_value::<CardReply>(v).unwrap();
        let until: Ts = "2026-11-02T09:00:00Z".parse().unwrap();
        assert_eq!(reply(json!({ "type": "later", "until": "2026-11-02T09:00:00Z" })), CardReply::Later { until });
        assert_eq!(reply(json!({ "type": "cant" })), CardReply::Cant);
        assert_eq!(reply(json!({ "type": "seen" })), CardReply::Seen);
        // A card an older build saved loads with none of what came later.
        let old: Card = serde_json::from_value(json!({ "message": "Boost it?", "ask": { "type": "choices", "choices": ["Go"], "recommended": 0 }, "answer": null })).unwrap();
        assert_eq!((old.link, old.images.len(), old.back_at, old.times_back, old.parked, old.proposal), (None, 0, None, 0, false, None));
    }

    #[test]
    fn news_is_unseen_until_answered_and_waits_on_nothing() {
        let mut run = Run::new("e1", RunKind::Manual, "Go".into(), Utc::now());
        run.cards = vec![Card::new("The spring sale is live.", None, Some("ask:sale".into()), CardAsk::News)];
        assert!(run.has_news() && !run.has_open_card(), "it holds back no answer");
        assert!(run.cards[0].pending() && !run.cards[0].is_open());
        run.cards[0].answer = Some(CardAnswer { at: Utc::now(), how: AnswerHow::Tapped, text: String::new(), held: false });
        assert!(!run.has_news());
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

    fn schedule_json(misfire: Option<&str>) -> String {
        let misfire = misfire.map(|m| format!(r#","misfire":"{m}""#)).unwrap_or_default();
        format!(r#"{{"id":"s1","employeeId":"e1","label":"Sweep","cron":"0 9 * * *","instruction":"Look","enabled":true{misfire}}}"#)
    }

    #[test]
    fn schedules_read_old_misfire_values_and_save_catch_up() {
        let skip: Schedule = serde_json::from_str(&schedule_json(Some("skip"))).unwrap();
        assert_eq!(skip.misfire, Misfire::Skip);
        let none: Schedule = serde_json::from_str(&schedule_json(None)).unwrap();
        assert_eq!(none.misfire, Misfire::CatchUp);
        assert_eq!((none.model, none.effort, none.review), (None, None, false));
        let saved = serde_json::to_string(&Schedule { misfire: Misfire::CatchUp, ..skip }).unwrap();
        assert!(saved.contains(r#""misfire":"catchUp""#), "{saved}");
    }

    #[test]
    fn check_id_reads_only_check_runs() {
        let scheduled = |id: &str| RunKind::Scheduled { schedule_id: id.into(), slot: Utc::now(), label: "x".into(), quiet: true, model: None, effort: None, review: false };
        assert_eq!(scheduled("check:abc").check_id(), Some("abc"));
        assert_eq!(scheduled("6f1c0e2a").check_id(), None);
        assert_eq!(RunKind::Chat.check_id(), None);
    }

    #[test]
    fn a_check_that_came_back_into_a_conversation_is_still_the_check() {
        let own = Run::new("e1", RunKind::Scheduled { schedule_id: "check:abc".into(), slot: Utc::now(), label: "x".into(), quiet: true, model: None, effort: None, review: false }, "go".into(), Utc::now());
        assert_eq!(own.check_id(), Some("abc"));
        let back = Run { check: Some("abc".into()), ..Run::new("e1", RunKind::Reply { to_run_id: "r0".into(), label: "Draft it".into(), task: true, card: false }, "go".into(), Utc::now()) };
        assert_eq!(back.check_id(), Some("abc"));
        assert_eq!(Run::new("e1", RunKind::Manual, "go".into(), Utc::now()).check_id(), None);
    }

    #[test]
    fn a_run_saved_before_titles_and_check_turns_loads() {
        let mut run = serde_json::to_value(Run { title: Some("Spring sale emails".into()), ..Run::new("e1", RunKind::Manual, "Go".into(), Utc::now()) }).unwrap();
        assert_eq!(run["title"], json!("Spring sale emails"));
        let o = run.as_object_mut().unwrap();
        o.remove("title");
        o.remove("check");
        let run: Run = serde_json::from_value(run).unwrap();
        assert_eq!((run.title, run.check), (None, None));
    }

    #[test]
    fn a_conversation_you_are_in_is_never_capped() {
        let answered = |thread: Option<&str>, chat: bool| RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat, thread: thread.map(String::from) };
        let reply = |card: bool| RunKind::Reply { to_run_id: "r0".into(), label: "Draft it".into(), task: true, card };
        let run = |kind: RunKind, omega_wrote: bool| Run { omega_wrote, ..Run::new("e1", kind, "go".into(), Utc::now()) };
        for kind in [RunKind::Task { label: "Draft it".into(), from: None, context: String::new() }, reply(false), answered(Some("r0"), false), RunKind::Chat, answered(None, true)] {
            assert!(run(kind.clone(), false).uncapped(), "{kind:?}");
        }
        assert!(run(answered(Some("r0"), false), true).uncapped(), "Omega Star writes every answer");
        assert!(!run(reply(true), true).uncapped(), "a card's answer waits its turn");
        assert!(!run(reply(false), true).uncapped(), "so does an approval");
        for kind in [RunKind::Manual, answered(None, false), RunKind::Delegated { parent_run_id: "r0".into() }] {
            assert!(!run(kind.clone(), false).uncapped(), "{kind:?}");
        }
    }

    #[test]
    fn an_answer_in_a_conversation_is_its_next_turn() {
        let task = Run::new("e1", RunKind::Task { label: "Draft it".into(), from: None, context: String::new() }, "Draft it.".into(), Utc::now());
        let came = RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat: false, thread: Some(task.id.clone()) };
        let answer = Run::new("e1", came, "Bo answered your question.".into(), Utc::now());
        assert_eq!(answer.reply_to(), Some(task.id.as_str()));
        assert_eq!(answer.conversation_root(), Some(task.id.as_str()));
        assert_eq!(task.conversation_root(), Some(task.id.as_str()));
        assert!(answer.kind.is_conversation());
        assert_eq!(answer.kind.lane(), Lane::Work);
        let on_a_report = Run::new("e1", RunKind::Reply { to_run_id: "r0".into(), label: "Sweep".into(), task: false, card: false }, "ok".into(), Utc::now());
        assert_eq!((on_a_report.reply_to(), on_a_report.conversation_root()), (Some("r0"), None));
    }

    #[test]
    fn rows_from_an_older_builds_chat_still_load() {
        let chat: Run = serde_json::from_value(serde_json::to_value(Run::new("e1", RunKind::Chat, "Hi.".into(), Utc::now())).unwrap()).unwrap();
        assert_eq!(chat.kind, RunKind::Chat);
        let came: RunKind = serde_json::from_value(json!({ "type": "answered", "askedRunId": "r1", "from": "e2", "task": false, "chat": true })).unwrap();
        assert_eq!(came, RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat: true, thread: None });
        assert_eq!(came.lane(), Lane::Chat);
        let task: RunKind = serde_json::from_value(json!({ "type": "task", "label": "Draft it", "from": "your chat", "context": "- 2:15 pm, Pat: Hi.\n" })).unwrap();
        assert_eq!(task, RunKind::Task { label: "Draft it".into(), from: Some("your chat".into()), context: "- 2:15 pm, Pat: Hi.\n".into() });
    }

    #[test]
    fn a_proxy_key_goes_in_a_header_so_runs_keep_the_claude_ai_sign_in() {
        let proxy = ProxySettings { url: "http://127.0.0.1:8317".into(), api_key: "k1".into(), management_key: String::new() };
        let env = |vars: &[(&str, &str)]| vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        assert_eq!(proxy.env(), env(&[("ANTHROPIC_BASE_URL", "http://127.0.0.1:8317"), ("ANTHROPIC_CUSTOM_HEADERS", "x-api-key: k1")]));
        let keyless = ProxySettings { api_key: String::new(), ..proxy };
        assert_eq!(keyless.env(), env(&[("ANTHROPIC_BASE_URL", "http://127.0.0.1:8317")]));
        assert!(ProxySettings::default().env().is_empty());
    }
}
