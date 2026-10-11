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
use std::ffi::OsString;
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

/// A child process with no console window and the login shell's PATH.
pub(crate) fn command(program: &Path) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    if let Some(p) = child_path() {
        c.env("PATH", p);
    }
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c
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
    /// The employee's own open requests, with the person's name.
    pub waiting_on: Vec<(HumanRequest, String)>,
    /// Summaries of recent finished work runs, newest first: (when, kind label, state, summary).
    pub recent_work: Vec<(Ts, String, RunState, String)>,
    /// The employee's own recurring schedules.
    pub schedules: Vec<Schedule>,
    /// The employee's hire proposals still waiting on a decision.
    pub proposals: Vec<HireProposal>,
    /// The employee's open checks, soonest first.
    pub checks: Vec<Check>,
    /// For Tidy runs: what to work from.
    pub tidy: Option<TidyBrief>,
    /// For follow-up runs, reply checks, nudges and give-ups: the person the request went to.
    pub replier: Option<String>,
    /// For chat turns: what the threads you closed since the last turn decided, oldest first.
    pub briefs: Vec<String>,
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
fn with_attachments(prompt: &str, dir: &Path, names: &[String]) -> String {
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
    // A reply check only looks and reports: the smallest model does it.
    let (model, effort) = match run.kind {
        RunKind::ReplyCheck { .. } => (Some("haiku".to_string()), Some(Effort::Low)),
        _ => (e.model.clone(), e.effort),
    };
    if let Some(m) = model {
        args.extend(["--model".into(), m]);
    }
    if let Some(eff) = effort {
        args.extend(["--effort".into(), eff.as_str().into()]);
    }
    for w in e.workspaces.iter().filter(|w| !w.trim().is_empty()) {
        args.extend(["--add-dir".into(), w.clone()]);
    }
    let mcp = json!({ "mcpServers": { "omega": { "type": "http", "url": s.mcp_url } } });
    Invocation {
        program: s.claude.clone(),
        args,
        cwd: s.employee_dir.clone(),
        env: vec![("MCP_TOOL_TIMEOUT".into(), "900000".into())],
        stdin: with_attachments(&run.prompt, &attachments_dir(&s.employee_dir, &run.id), &run.attachments),
        files: vec![
            (s.employee_dir.join("CLAUDE.md"), handbook(s)),
            (settings_path, serde_json::to_string_pretty(&s.settings).unwrap()),
            (prompt_path, system_prompt(s, run)),
            (mcp_path, serde_json::to_string_pretty(&mcp).unwrap()),
        ],
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
