//! Runs: their kinds, states, events, builds, threads and usage.

use super::*;
use serde::{Deserialize, Serialize};

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
        /// The reply answers a card, written by OmegaStar from your tap.
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

/// How this copy of OmegaStar was built: a published release, a test build, or a developer's own.
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

/// This copy of OmegaStar: its version, the commit it was built from, and its channel.
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
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub employee_id: String,
    pub kind: RunKind,
    pub prompt: String,
    /// OmegaStar wrote this prompt from something you did or something that came back (an approval, a
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
    /// What needs you from this run: each card's message, and flags OmegaStar raised.
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
    /// You wrote into it while it worked, so its report waits for you in the Inbox like a reply, whatever kind of run
    /// it is.
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
    /// For a due check that came back into the conversation it was set in, as a reply OmegaStar wrote there:
    /// the check's id. A check's own run names it in its kind instead (`RunKind::check_id`).
    #[serde(default)]
    pub check: Option<String>,
    /// Permission rules you granted for this run only ("Allow once"). They go into the run's
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
    /// Times the run went back to the queue because OmegaStar stopped while it was in progress.
    #[serde(default)]
    pub restarts: u32,
    /// It last went back to the queue because OmegaStar stopped, not the usage limit; its next attempt is told which.
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
    /// The tokens it used, model by model, summed over its result segments and attempts. Empty for a run from
    /// before builds kept them.
    #[serde(default)]
    pub tokens: Vec<ModelTokens>,
    /// Not to start before then: a hand-off OmegaStar paused to keep its chain short, an answer queued again
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
            tokens: vec![],
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
        self.cards.iter().any(|c| c.answer.is_none() && c.ask == CardAsk::News)
    }

    /// What needs you that is not a card: flags OmegaStar raised, and `notify` messages from before cards.
    pub fn flags(&self) -> impl Iterator<Item = &String> {
        self.needs_you.iter().filter(|m| !self.cards.iter().any(|c| c.message == **m))
    }

    /// For a Tidy run: the bigger changes it proposed that wait for your approval. A reply in its thread may still
    /// change them.
    pub fn waiting_plan(&self) -> Option<&str> {
        match (&self.kind, &self.proposed, self.approved_at) {
            (RunKind::Tidy { .. }, Some(plan), None) => Some(plan),
            _ => None,
        }
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
    /// an older build's chat. A reply OmegaStar wrote from your tap (a card's answer, an approval) is capped:
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
/// (`Run::reply_to`), oldest first. In the Inbox and the Archive a thread is a row's lead, with the row's other threads
/// in `related`.
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

/// Every conversation across the office, for the Inbox and the sidebar, in one read.
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
}

/// The tokens a run used on one model, as Claude Code counts them: what it read fresh, wrote to the prompt cache and
/// wrote out. Re-reads of the cache are left out: they are most of the volume, at a tenth of the price.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelTokens {
    /// The model's id, e.g. `claude-opus-4-5-20251101`.
    pub model: String,
    pub tokens: u64,
}

/// Adds `more` to `total`, model by model.
pub fn add_tokens(total: &mut Vec<ModelTokens>, more: &[ModelTokens]) {
    for m in more {
        match total.iter_mut().find(|t| t.model == m.model) {
            Some(t) => t.tokens += m.tokens,
            None => total.push(m.clone()),
        }
    }
}

/// What runs did and used since a time, for Activity's digest of today and its usage this week. Usage is what
/// runs cost in US dollars at API list price, as Claude Code reckons it; a run with no cost recorded adds nothing.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunUsage {
    pub employees: Vec<EmployeeUsage>,
    pub routines: Vec<RoutineUsage>,
}

/// What the team got done and what it asked of you, for the Analytics page, all time: the page slices it by range.
/// Days are this computer's local dates.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamStats {
    /// Everyone ever hired, including those who have left, so the page knows the team's size and age.
    pub staff: Vec<StaffSpan>,
    /// One row per employee per day they finished runs.
    pub days: Vec<TeamDay>,
    /// Every card that asked you something (never news), oldest first.
    pub asks: Vec<TeamAsk>,
    /// Each employee's latest weekly number, from their newest weekly review that reported one.
    pub numbers: Vec<TeamNumber>,
    /// Tokens used, one row per day, employee, model and who handed the work over.
    pub tokens: Vec<TeamTokens>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StaffSpan {
    pub employee_id: String,
    pub name: String,
    pub hired_at: Ts,
    pub departed_at: Option<Ts>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamDay {
    /// `YYYY-MM-DD`, local.
    pub day: String,
    pub employee_id: String,
    pub runs: u32,
    /// Time the runs spent working, in milliseconds.
    pub work_ms: i64,
    /// The part of `work_ms` from runs begun between 10 pm and 7 am local: work done while you were likely asleep.
    pub night_ms: i64,
}

/// Tokens an employee's finished runs used on one model in a local day.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamTokens {
    /// `YYYY-MM-DD`, local.
    pub day: String,
    /// Who did the work, and so used the tokens.
    pub employee_id: String,
    /// The colleague who asked for it or delegated it; `None` for their own work.
    pub handed_over_by: Option<String>,
    pub model: String,
    pub tokens: u64,
}

/// One card that asked you something. A card has no time of its own, so `at` is when its run ended, or began if it
/// has not ended.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamAsk {
    pub at: Ts,
    /// When you answered it yourself, Later and No included. None when it is still open, or when a newer card replaced
    /// it or the employee withdrew it, since then you never answered.
    pub answered_at: Option<Ts>,
    /// Still waiting on your answer.
    pub open: bool,
}

/// The number an employee answers for, as their newest weekly review reported it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamNumber {
    pub employee_id: String,
    pub metric: String,
    pub value: String,
    /// Against last week, in the employee's own words, e.g. "+3" or "first count".
    pub change: String,
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
    use serde_json::json;

    fn build() -> Build {
        Build { version: "1.2.3".into(), commit: "abc1234567".into(), channel: BuildChannel::Dogfood }
    }

    #[test]
    fn tokens_add_up_model_by_model() {
        let m = |model: &str, tokens: u64| ModelTokens { model: model.into(), tokens };
        let mut total = vec![m("opus", 10)];
        add_tokens(&mut total, &[m("haiku", 5), m("opus", 7)]);
        assert_eq!(total, vec![m("opus", 17), m("haiku", 5)]);
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
        assert!(run(answered(Some("r0"), false), true).uncapped(), "OmegaStar writes every answer");
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
}
