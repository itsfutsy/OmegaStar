//! Everything that knows the Claude Code CLI: locating it, account status, turning a run into an
//! invocation (argv, files), spawning and killing it, and probing usage. The handbook and framing
//! live in `prompt`, stream-json parsing in `stream`, and the reply judge in `judge`.

mod judge;
mod prompt;
mod stream;

pub use judge::*;
pub use prompt::*;
pub use stream::*;

use crate::model::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------------------------------------
// Locating the binary

/// PATH for child processes. A macOS/Linux GUI app does not inherit the login shell's PATH, where
/// npm/homebrew installs of `claude` (and the tools it runs) usually live.
pub fn child_path() -> Option<OsString> {
    static PATH: OnceLock<Option<OsString>> = OnceLock::new();
    PATH.get_or_init(|| {
        #[cfg(unix)]
        {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            let out = std::process::Command::new(shell).args(["-lc", "printf %s \"$PATH\""]).output().ok()?;
            let p = String::from_utf8(out.stdout).ok()?;
            (!p.trim().is_empty()).then(|| OsString::from(p.trim()))
        }
        #[cfg(not(unix))]
        {
            None
        }
    })
    .clone()
}

fn fallback_locations() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut v = vec![];
    if cfg!(windows) {
        v.push(home.join(".local/bin/claude.exe"));
        if let Some(appdata) = std::env::var_os("APPDATA") {
            v.push(PathBuf::from(appdata).join("npm").join("claude.cmd"));
        }
    } else {
        v.push(home.join(".local/bin/claude"));
        v.push(PathBuf::from("/opt/homebrew/bin/claude"));
        v.push(PathBuf::from("/usr/local/bin/claude"));
    }
    v.push(home.join(".claude/local").join(if cfg!(windows) { "claude.exe" } else { "claude" }));
    v
}

pub fn locate(override_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = override_path.map(str::trim).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let found = match child_path() {
        Some(path) => which::which_in("claude", Some(path), std::env::current_dir().unwrap_or_default()).ok(),
        None => which::which("claude").ok(),
    };
    found.or_else(|| fallback_locations().into_iter().find(|p| p.is_file()))
}

/// A child process with no console window, the login shell's PATH and none of the inherited
/// Anthropic credentials or provider settings, so a run always uses the signed-in account.
pub(crate) fn command(program: &Path) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    if let Some(p) = child_path() {
        c.env("PATH", p);
    }
    let names: Vec<OsString> = std::env::vars_os().map(|(k, _)| k).collect();
    for n in scrubbed_vars(names.iter().map(OsString::as_os_str)) {
        c.env_remove(n);
    }
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// The inherited variables a child must not see: Anthropic keys, base URLs and tokens, and the
/// switches to another provider. Network proxy variables and CLAUDE_CONFIG_DIR stay.
fn scrubbed_vars<'a>(names: impl Iterator<Item = &'a OsStr>) -> Vec<OsString> {
    const EXACT: [&str; 3] = ["CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR", "AWS_BEARER_TOKEN_BEDROCK"];
    names
        .filter(|n| n.to_str().is_some_and(|n| n.starts_with("ANTHROPIC_") || n.starts_with("CLAUDE_CODE_USE_") || EXACT.contains(&n)))
        .map(OsStr::to_os_string)
        .collect()
}

async fn capture(program: &Path, args: &[&str]) -> Option<String> {
    let fut = command(program).args(args).stdin(Stdio::null()).output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(20), fut).await.ok()?.ok()?;
    // `auth status` exits non-zero when logged out but still prints JSON.
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub async fn version(program: &Path) -> Option<String> {
    let s = capture(program, &["--version"]).await?;
    let v = s.split_whitespace().next()?.to_string();
    (!v.is_empty()).then_some(v)
}

fn parse_auth(stdout: &str) -> Option<Account> {
    let v: Value = serde_json::from_str(stdout.trim()).ok()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    Some(Account {
        logged_in: v.get("loggedIn").and_then(|x| x.as_bool()).unwrap_or(false),
        email: s("email"),
        org_name: s("orgName"),
        plan: s("subscriptionType"),
    })
}

pub async fn auth_status(program: &Path) -> Option<Account> {
    parse_auth(&capture(program, &["auth", "status"]).await?)
}

// ---------------------------------------------------------------------------------------------
// Preparing a run

pub struct Report {
    pub name: String,
    pub slug: String,
    pub title: String,
    pub objective: String,
}

/// Another agent in the company, outside your own reports, as the handbook introduces them.
pub struct Colleague {
    pub name: String,
    pub slug: String,
    pub title: String,
    pub objective: String,
    pub description: String,
}

/// Everything a launch depends on, gathered by the Office. `prepare` is a pure function of it.
pub struct Snapshot {
    pub claude: PathBuf,
    pub company: Company,
    pub employee: Employee,
    /// The catalog skills granted to the employee right now.
    pub granted: Vec<SkillInfo>,
    /// Name and title of the employee's manager; `None` = reports to you (the user) directly.
    pub manager: Option<(String, String)>,
    /// Name and title of the person you (the user) report to, if anyone.
    pub boss: Option<(String, String)>,
    pub reports: Vec<Report>,
    /// Every other agent except the employee's direct reports.
    pub colleagues: Vec<Colleague>,
    pub people: Vec<Person>,
    /// Name and title of the colleague on the other end: who asked (Asked runs) or who answered (Answered runs).
    pub counterpart: Option<(String, String)>,
    /// The employee's own recurring schedules.
    pub schedules: Vec<Schedule>,
    /// What the opening note at the top of the first message is made from. `None` for reply checks,
    /// nudges and Organize runs, which get no note.
    pub opening: Option<Opening>,
    /// What the employee owns: their `what-you-own.md`, trimmed; empty when there is none.
    pub owns: String,
    /// For Delegated, Asked and Answered runs: where the hand-off began, in one passage.
    pub source: Option<String>,
    /// The employee's manager is an agent, not a person.
    pub manager_is_agent: bool,
    /// For Tidy runs: what to work from.
    pub tidy: Option<TidyBrief>,
    /// For follow-up runs, reply checks, nudges and give-ups: the person the request went to.
    pub replier: Option<String>,
    /// For reply checks, nudges and give-ups: the request they are about.
    pub watched: Option<Watched>,
    pub settings: Value,
    pub employee_dir: PathBuf,
    pub run_dir: PathBuf,
    pub mcp_url: String,
    /// `true` when the session already exists and must be resumed.
    pub resume: bool,
    pub session: String,
}

/// What changed around the employee that a run should know before it starts: what waits, what they
/// did lately, what their team finished, what else is running and which notes changed. With `since`,
/// only what is new since then.
#[derive(Default)]
pub struct Opening {
    pub now: Ts,
    /// The end of the last turn in this conversation (or of the routine's last run); `None` = a fresh start.
    pub since: Option<Ts>,
    /// Open checks, soonest first.
    pub checks: Vec<Check>,
    /// Cards still waiting on an answer: (when raised, message), oldest first.
    pub cards: Vec<(Ts, String)>,
    /// Open requests to people, with the person's name.
    pub waiting_on: Vec<(HumanRequest, String)>,
    /// Hire proposals waiting on a decision.
    pub proposals: Vec<HireProposal>,
    /// The employee's finished runs in the window, other conversations only, newest first.
    pub work: Vec<Run>,
    /// What the employee's direct reports finished, with their names, newest first.
    pub team: Vec<(String, Run)>,
    /// The employee's other runs in progress.
    pub running: Vec<Run>,
    /// Notes in `workspace/` changed in the window: (path inside it, when), newest first.
    pub notes: Vec<(String, Ts)>,
    /// Runs that asked or delegated something: a quiet run among them reached someone.
    pub parents: HashSet<String>,
}

/// The request a reply check, nudge or give-up is about.
pub struct Watched {
    pub request: HumanRequest,
    pub person: String,
    /// The channel it went out on, as the handbook names it ("Teams jordan@example.com").
    pub channel: Option<String>,
    /// How many business days the watch waited before giving up.
    pub days: u32,
}

/// What a Tidy run works from: what the health check found, what the person taught the employee
/// since the last tidy, and for an approved tidy the plan they approved.
pub struct TidyBrief {
    pub findings: Vec<HealthFinding>,
    /// (when, what they said) in chat and in Inbox replies.
    pub taught: Vec<(Ts, String)>,
    pub plan: Option<String>,
}

pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// The first message: the prompt and any attached files.
    pub stdin: String,
    /// Files to write before spawning.
    pub files: Vec<(PathBuf, String)>,
}

pub fn kind_label(k: &RunKind) -> String {
    match k {
        RunKind::Manual => "manual".into(),
        RunKind::Chat => "chat".into(),
        RunKind::Scheduled { label, .. } => format!("scheduled '{label}'"),
        RunKind::Delegated { .. } => "delegated by your manager".into(),
        RunKind::Asked { task: false, .. } => "question from a colleague".into(),
        RunKind::Asked { task: true, .. } => "task from a colleague".into(),
        RunKind::Answered { .. } => "a colleague's answer".into(),
        RunKind::FollowUp { .. } => "follow-up on a reply".into(),
        RunKind::Organize { .. } => "organized imported skills".into(),
        RunKind::Tidy { reason: TidyReason::Approved, .. } => "tidy of your skills (approved changes)".into(),
        RunKind::Tidy { .. } => "tidy of your skills and notes".into(),
        RunKind::Reply { task: false, .. } => "reply to your report".into(),
        RunKind::Reply { task: true, label, .. } => format!("reply in your task '{label}'"),
        RunKind::Task { label, .. } => format!("task '{label}'"),
        RunKind::HireDecided { approved: true, .. } => "hire proposal approved".into(),
        RunKind::HireDecided { approved: false, .. } => "hire proposal declined".into(),
        RunKind::ReplyCheck { .. } => "reply check".into(),
        RunKind::Nudge { .. } => "nudge to a person".into(),
        RunKind::Unanswered { .. } => "no reply to a request".into(),
    }
}

pub fn clip(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(n).collect::<String>().trim_end())
    }
}

/// The start and the end of a long text, so a report's closing lines survive: one line, and when it is
/// longer than `2n + 5` characters, its first and last `n` around " ... ".
pub(crate) fn ends(s: &str, n: usize) -> String {
    let s = one_line(s);
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 2 * n + 5 {
        return s;
    }
    let head: String = chars[..n].iter().collect();
    let tail: String = chars[chars.len() - n..].iter().collect();
    format!("{} ... {}", head.trim_end(), tail.trim_start())
}

pub(crate) fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn channel_kind(k: ChannelKind) -> &'static str {
    match k {
        ChannelKind::Teams => "Teams",
        ChannelKind::Slack => "Slack",
        ChannelKind::Email => "Email",
        ChannelKind::Jira => "Jira",
        ChannelKind::Asana => "Asana",
        ChannelKind::AzureDevOps => "Azure DevOps",
        ChannelKind::Github => "GitHub",
        ChannelKind::Linear => "Linear",
        ChannelKind::Phone => "Phone",
        ChannelKind::Other => "Other",
    }
}

/// A channel as a run names it: "Teams nina@example.com".
pub fn channel_name(c: &Channel) -> String {
    format!("{} {}", channel_kind(c.kind), c.address.trim())
}

pub fn channel_line(c: &Channel) -> String {
    format!("{} {}: use for {} (channel id `{}`)", channel_kind(c.kind), c.address.trim(), one_line(&c.when_to_use), c.id)
}

fn path_arg(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Where a run's attachments live: inside the employee's home, so reading them needs no permission.
pub fn attachments_dir(employee_dir: &Path, run_id: &str) -> PathBuf {
    employee_dir.join("attachments").join(run_id)
}

/// The prompt, followed by the absolute path of each attached file.
pub(super) fn with_attachments(prompt: &str, dir: &Path, names: &[String]) -> String {
    if names.is_empty() {
        return prompt.to_string();
    }
    let list: Vec<String> = names.iter().map(|n| format!("- {}", path_arg(&dir.join(n)))).collect();
    let note = format!("Attached files (read each one before you reply; the Read tool opens images and PDFs too):\n{}", list.join("\n"));
    if prompt.trim().is_empty() { note } else { format!("{prompt}\n\n{note}") }
}

/// Claude Code tools whose effect dies with the `claude -p` process or bypasses Omega Star. The
/// `omega` MCP tools `schedule` and `notify` replace them.
const SESSION_ONLY_TOOLS: [&str; 6] = ["CronCreate", "CronDelete", "CronList", "ScheduleWakeup", "RemoteTrigger", "PushNotification"];

/// `--model` and `--effort` for a run: the employee's own, unless its kind says otherwise.
fn model_args(kind: &RunKind, e: &Employee) -> Vec<String> {
    let (model, effort) = match kind {
        // A reply check only looks and reports: the smallest model does it.
        RunKind::ReplyCheck { .. } => (Some("haiku".to_string()), Some(Effort::Low)),
        RunKind::Scheduled { model, effort, .. } => (model.clone().or_else(|| e.model.clone()), effort.or(e.effort)),
        _ => (e.model.clone(), e.effort),
    };
    let mut args = vec![];
    if let Some(m) = model {
        args.extend(["--model".into(), m]);
    }
    if let Some(eff) = effort {
        args.extend(["--effort".into(), eff.as_str().into()]);
    }
    args
}

/// The first message of a run that goes again after the usage limit stopped it: the conversation it
/// resumes already holds the message it was answering.
pub const LIMIT_RETRY_NOTE: &str = "Omega Star: the usage limit stopped you before you could answer. It has reset now, so carry on with the message above.";

pub fn prepare(s: &Snapshot, run: &Run) -> Invocation {
    let e = &s.employee;
    let settings_path = s.run_dir.join("settings.json");
    let prompt_path = s.run_dir.join("system-prompt.md");
    let mcp_path = s.run_dir.join("mcp.json");
    let mut args: Vec<String> = ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages"]
        .map(String::from)
        .to_vec();
    args.push(if s.resume { "--resume" } else { "--session-id" }.into());
    args.push(s.session.clone());
    args.extend(["--settings".into(), path_arg(&settings_path)]);
    args.extend(["--append-system-prompt-file".into(), path_arg(&prompt_path)]);
    let mode = match e.clearance {
        Clearance::Supervised => "acceptEdits",
        Clearance::Autonomous => "bypassPermissions",
    };
    args.extend(["--permission-mode".into(), mode.into(), "--permission-prompts".into(), "none".into()]);
    args.extend(["--mcp-config".into(), path_arg(&mcp_path)]);
    // `claude -p` leaves Claude in Chrome off unless asked, so only employees you allowed it get it.
    if e.browser {
        args.push("--chrome".into());
    }
    args.extend(["--disallowedTools".into(), SESSION_ONLY_TOOLS.join(",")]);
    args.extend(model_args(&run.kind, e));
    for w in e.workspaces.iter().filter(|w| !w.trim().is_empty()) {
        args.extend(["--add-dir".into(), w.clone()]);
    }
    let mcp = json!({ "mcpServers": { "omega": { "type": "http", "url": s.mcp_url } } });
    Invocation {
        program: s.claude.clone(),
        args,
        cwd: s.employee_dir.clone(),
        env: vec![("MCP_TOOL_TIMEOUT".into(), "900000".into())],
        stdin: first_message(s, run),
        files: vec![
            (s.employee_dir.join("CLAUDE.md"), handbook(s)),
            (settings_path, serde_json::to_string_pretty(&s.settings).unwrap()),
            (prompt_path, system_prompt(s, run)),
            (mcp_path, serde_json::to_string_pretty(&mcp).unwrap()),
        ],
    }
}

/// A snapshot of `employee` at `company` with nothing else around them, for tests.
#[cfg(test)]
pub(crate) fn test_snapshot(company: Company, employee: Employee, opening: Option<Opening>, resume: bool) -> Snapshot {
    let employee_dir = PathBuf::from("/office/employees").join(&employee.id);
    Snapshot {
        claude: PathBuf::from("/usr/local/bin/claude"),
        company,
        granted: vec![],
        manager: None,
        boss: None,
        reports: vec![],
        colleagues: vec![],
        people: vec![],
        counterpart: None,
        schedules: vec![],
        opening,
        owns: String::new(),
        source: None,
        manager_is_agent: false,
        tidy: None,
        replier: None,
        watched: None,
        settings: json!({}),
        run_dir: PathBuf::from("/office/runs/r1"),
        employee_dir,
        mcp_url: "http://127.0.0.1:1/mcp/t".into(),
        resume,
        session: employee.chat_session.clone(),
        employee,
    }
}

/// One message for a run's stdin. A run keeps reading these until its stdin closes, so a message sent
/// while it works reaches it at its next step.
pub fn user_line(text: &str) -> String {
    format!("{}\n", json!({ "type": "user", "message": { "role": "user", "content": text } }))
}

// ---------------------------------------------------------------------------------------------
// Process control

pub fn spawn(inv: &Invocation) -> std::io::Result<tokio::process::Child> {
    let mut c = command(&inv.program);
    c.args(&inv.args)
        .current_dir(&inv.cwd)
        .envs(inv.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    c.process_group(0);
    c.spawn()
}

/// Kills the process and everything it started (tools, MCP servers, shells).
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // The child leads its own process group (process_group(0) at spawn).
        let _ = std::process::Command::new("kill").args(["-KILL", "--", &format!("-{pid}")]).status();
    }
}

// ---------------------------------------------------------------------------------------------
// Usage limits

/// Reads the `/usage` report, whose lines look like
/// `Current session: 34% used · resets Sep 28, 2:59pm (Europe/London)`.
/// "Current session" is the 5-hour window and "Current week (all models)" the 7-day one.
fn parse_usage_text(text: &str) -> UsageSample {
    let window = |line: &str| -> Option<UsageWindow> {
        let at = line.find("% used")?;
        let head = &line[..at];
        let start = head.char_indices().rev().find(|(_, c)| !(c.is_ascii_digit() || *c == '.')).map(|(i, c)| i + c.len_utf8()).unwrap_or(0);
        let percent: f64 = head[start..].parse().ok()?;
        let resets_text = line[at..].find("resets ").map(|i| line[at + i + "resets ".len()..].trim().to_string()).filter(|t| !t.is_empty());
        Some(UsageWindow { percent: percent.clamp(0.0, 100.0), resets_at: None, resets_text })
    };
    let mut sample = UsageSample::default();
    for line in text.lines() {
        if sample.five_hour.is_none() && line.contains("Current session") {
            sample.five_hour = window(line);
        } else if sample.seven_day.is_none() && line.contains("Current week (all models)") {
            sample.seven_day = window(line);
        }
    }
    sample
}

/// The `/usage` report from a probe's stream-json stdout; `None` when there was no result line.
fn parse_usage_output(stdout: &str) -> Option<UsageSample> {
    let result = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|v| v["type"] == "result")?;
    Some(parse_usage_text(result["result"].as_str().unwrap_or("")))
}

/// Asks Claude Code for `/usage`. Verified to cost nothing: no model turn, no session saved.
pub async fn probe_usage(program: &Path, cwd: &Path) -> Option<UsageSample> {
    let mut c = command(program);
    c.args(["-p", "--output-format", "stream-json", "--verbose", "--no-session-persistence"])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    c.process_group(0);
    let mut child = c.spawn().ok()?;
    let pid = child.id();
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"/usage").await;
        let _ = stdin.shutdown().await;
    }
    let mut out = String::new();
    let mut stdout = child.stdout.take()?;
    let read = tokio::time::timeout(Duration::from_secs(90), stdout.read_to_string(&mut out)).await;
    if read.is_err() {
        if let Some(pid) = pid {
            kill_tree(pid);
        }
    }
    let _ = child.wait().await;
    parse_usage_output(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schedule_can_run_on_its_own_model_and_effort() {
        let e = Employee {
            id: "e1".into(),
            slug: "ada".into(),
            name: "Ada".into(),
            title: "Tester".into(),
            description: String::new(),
            objective: String::new(),
            manager_id: None,
            skills: SkillGrant::default(),
            clearance: Clearance::Supervised,
            allow_rules: vec![],
            model: Some("opus".into()),
            effort: Some(Effort::High),
            browser: false,
            auto_tidy: false,
            workspaces: vec![],
            hue: 0,
            chat_session: "chat-ada".into(),
            hired_at: chrono::Utc::now(),
        };
        let scheduled = |model: Option<&str>, effort| RunKind::Scheduled {
            schedule_id: "s1".into(),
            slot: chrono::Utc::now(),
            label: "Inbox watch".into(),
            quiet: true,
            model: model.map(String::from),
            effort,
            review: false,
        };
        assert_eq!(model_args(&scheduled(Some("haiku"), Some(Effort::Low)), &e), ["--model", "haiku", "--effort", "low"]);
        assert_eq!(model_args(&scheduled(None, None), &e), ["--model", "opus", "--effort", "high"]);
        assert_eq!(model_args(&RunKind::Reply { to_run_id: "r1".into(), label: "Inbox watch".into(), task: false, card: false }, &e), ["--model", "opus", "--effort", "high"]);
    }

    fn ada() -> Employee {
        Employee {
            id: "e1".into(),
            slug: "ada".into(),
            name: "Ada".into(),
            title: "Analyst".into(),
            description: String::new(),
            objective: "Keep the numbers right.".into(),
            manager_id: None,
            skills: SkillGrant::default(),
            clearance: Clearance::Supervised,
            allow_rules: vec![],
            model: None,
            effort: None,
            browser: false,
            auto_tidy: false,
            workspaces: vec![],
            hue: 0,
            chat_session: "chat-ada".into(),
            hired_at: chrono::Utc::now(),
        }
    }

    fn acme() -> Company {
        Company { name: "Acme".into(), ceo_name: "Pat".into(), ..Company::unset() }
    }

    fn snapshot(opening: Option<Opening>, resume: bool) -> Snapshot {
        test_snapshot(acme(), ada(), opening, resume)
    }

    fn now_opening(since: Option<Ts>) -> Opening {
        Opening { now: chrono::Utc::now(), since, ..Opening::default() }
    }

    fn file<'a>(inv: &'a Invocation, name: &str) -> &'a str {
        &inv.files.iter().find(|(p, _)| p.ends_with(name)).unwrap().1
    }

    fn scheduled(id: &str, quiet: bool) -> RunKind {
        RunKind::Scheduled { schedule_id: id.into(), slot: chrono::Utc::now(), label: "Inbox sweep".into(), quiet, model: None, effort: None, review: false }
    }

    /// A finished run of Ada's, `mins` minutes ago.
    fn done(kind: RunKind, mins: i64, summary: &str) -> Run {
        let at = chrono::Utc::now() - chrono::Duration::minutes(mins);
        let mut r = Run::new("e1", kind, "Go.".into(), at);
        r.state = RunState::Succeeded;
        r.started_at = Some(at);
        r.ended_at = Some(at);
        r.summary = Some(summary.into());
        r
    }

    #[test]
    fn the_frame_reaches_a_resumed_run_in_its_first_message() {
        let fresh = Run::new("e1", scheduled("s1", false), "Sweep the inbox.".into(), chrono::Utc::now());
        let before = fresh.prompt.clone();
        let inv = prepare(&snapshot(Some(now_opening(None)), false), &fresh);
        assert!(file(&inv, "system-prompt.md").contains("Scheduled routine 'Inbox sweep'"));
        assert!(inv.stdin.starts_with("<omega-star>\nOmega Star wrote this note. It is not a message from Pat.\n\nNow: "), "{}", inv.stdin);
        assert!(inv.stdin.ends_with("</omega-star>\n\nSweep the inbox."));
        assert!(!inv.stdin.contains("Scheduled routine"), "a new session reads its frame in the system prompt");
        assert_eq!(fresh.prompt, before);

        let reply = Run::new("e1", RunKind::Reply { to_run_id: "r0".into(), label: "Inbox sweep".into(), task: false, card: false }, "Archive it.".into(), chrono::Utc::now());
        let inv = prepare(&snapshot(Some(now_opening(Some(chrono::Utc::now()))), true), &reply);
        let frame_text = "replied in the thread on your report 'Inbox sweep'";
        assert!(!file(&inv, "system-prompt.md").contains(frame_text));
        let (note, body) = inv.stdin.split_once("</omega-star>\n\n").unwrap();
        assert!(note.contains(frame_text), "{note}");
        assert_eq!(body, "Archive it.");
        assert_eq!(reply.prompt, "Archive it.", "the note is never written into the prompt");

        let chat = Run::new("e1", RunKind::Chat, "Where are we on the launch?".into(), chrono::Utc::now());
        let inv = prepare(&snapshot(Some(now_opening(Some(chrono::Utc::now()))), true), &chat);
        assert!(!inv.stdin.contains("You are talking with Pat"));
        assert!(!file(&inv, "system-prompt.md").contains("You are talking with Pat"));
        assert!(inv.stdin.ends_with("Where are we on the launch?"));

        let mut retried = reply.clone();
        retried.limit_retries = 1;
        let inv = prepare(&snapshot(None, true), &retried);
        assert!(inv.stdin.contains(frame_text));
        assert!(inv.stdin.ends_with(&format!("</omega-star>\n\n{LIMIT_RETRY_NOTE}")));
        assert!(!inv.stdin.contains("Archive it."));
    }

    #[test]
    fn the_handbook_stays_the_same_whatever_happened_lately() {
        let mut busy = now_opening(None);
        busy.work = vec![done(RunKind::Manual, 30, "Paid the invoice.")];
        busy.notes = vec![("campaign.md".into(), chrono::Utc::now())];
        assert_eq!(handbook(&snapshot(Some(busy), false)), handbook(&snapshot(Some(now_opening(None)), false)));
        assert_eq!(handbook(&snapshot(None, false)), handbook(&snapshot(Some(now_opening(Some(chrono::Utc::now()))), true)));
    }

    #[test]
    fn quiet_runs_fold_into_one_line_per_routine() {
        let mut o = now_opening(None);
        for i in 0..47 {
            o.work.push(done(scheduled("s1", true), 10 + i, &format!("Nothing new in run {i}.")));
        }
        let mut flagged = done(scheduled("s1", true), 5, "Found a renewal notice.");
        flagged.needs_you = vec!["The domain renews on Friday.".into()];
        o.work.insert(0, flagged);
        o.parents = [o.work[3].id.clone(), o.work[9].id.clone()].into();
        let run = Run::new("e1", RunKind::Manual, "Go.".into(), chrono::Utc::now());
        let text = first_message(&snapshot(Some(o), false), &run);
        let fold = text.lines().find(|l| l.contains("quiet runs")).unwrap();
        assert!(fold.starts_with("- Inbox sweep: 47 quiet runs since "), "{fold}");
        assert!(fold.contains(", 2 reached someone. Latest (") && fold.ends_with("): Nothing new in run 0."), "{fold}");
        let raised = text.lines().find(|l| l.contains("(you raised: The domain renews on Friday.)")).unwrap();
        assert!(raised.contains("scheduled 'Inbox sweep': Found a renewal notice."), "{raised}");
        assert_eq!(text.lines().filter(|l| l.starts_with("- ")).count(), 2, "{text}");

        let mut o = now_opening(None);
        o.work = (0..3).map(|i| done(scheduled("s1", true), 10 + i, "Nothing new.")).collect();
        let text = first_message(&snapshot(Some(o), false), &run);
        assert!(text.contains("3 quiet runs since ") && text.contains(", none reached anyone. Latest ("), "{text}");
    }

    #[test]
    fn a_long_report_keeps_its_start_and_its_end() {
        let mut o = now_opening(None);
        let summary = format!("Reconciled the accounts. {}**Still open:** the bank has not confirmed the transfer.", "Checked every line twice. ".repeat(35));
        assert!(summary.chars().count() > 900);
        o.work = vec![done(RunKind::Manual, 30, &summary)];
        let run = Run::new("e1", RunKind::Manual, "Go.".into(), chrono::Utc::now());
        let text = first_message(&snapshot(Some(o), false), &run);
        let line = text.lines().find(|l| l.contains("Reconciled the accounts.")).unwrap();
        assert!(line.contains(" ... ") && line.ends_with("**Still open:** the bank has not confirmed the transfer."), "{line}");
        assert_eq!(ends("short", 150), "short");
    }

    #[test]
    fn a_quiet_routine_with_nothing_new_gets_no_note() {
        let quiet = Run::new("e1", scheduled("s1", true), "Sweep the inbox.".into(), chrono::Utc::now());
        let since = Some(chrono::Utc::now() - chrono::Duration::minutes(15));
        assert_eq!(first_message(&snapshot(Some(now_opening(since)), false), &quiet), quiet.prompt);
        let loud = Run::new("e1", scheduled("s2", false), "Write the weekly report.".into(), chrono::Utc::now());
        let text = first_message(&snapshot(Some(now_opening(since)), false), &loud);
        let note = text.split_once("\n\n").unwrap().1;
        assert!(note.starts_with("Now: ") && note.contains(".\n</omega-star>"), "only the time: {text}");
        assert_eq!(first_message(&snapshot(None, false), &quiet), quiet.prompt, "no note at all");
    }

    #[test]
    fn runs_lose_inherited_anthropic_settings_but_keep_the_network() {
        let names = [
            "PATH", "HOME", "CLAUDE_CONFIG_DIR", "SHELL", "HTTPS_PROXY", "no_proxy",
            "ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CODE_USE_BEDROCK",
        ];
        let scrubbed = scrubbed_vars(names.iter().map(OsStr::new));
        assert_eq!(scrubbed, ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CODE_USE_BEDROCK"]);
    }
}
