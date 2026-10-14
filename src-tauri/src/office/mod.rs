//! The Office: the single writer. Owns the store, the run queue and lanes, the scheduler tick,
//! chat, delegation and people requests. It talks to the UI only through an `EventSink`.

mod conversations;
mod inbox;
mod members;
mod requests;
mod runs;
mod schedules;
mod settings;
mod skills;
mod tools;

use crate::backup;
use crate::catalog;
use crate::claude::{self, Outcome};
use crate::health;
use crate::log::log;
use crate::model::*;
use crate::org::Org;
use crate::own_skills;
use crate::reader::{self, ReadError};
use crate::schedule::{self, Due};
use crate::store::{RequestFilter, RunRecord, Store};
use crate::threads;
use crate::tidy;
use crate::watch::{self, Applied, ChatMessage};
use chrono::{Local, NaiveDate, Utc};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub trait EventSink: Send + Sync + 'static {
    fn org_changed(&self);
    fn run_updated(&self, run: &Run);
    fn run_events(&self, run_id: &str, events: &[RunEvent]);
    fn request_updated(&self, request: &HumanRequest);
    fn proposal_updated(&self, _proposal: &HireProposal) {}
    fn settings_changed(&self, _settings: &AppSettings) {}
    fn usage_updated(&self, _usage: &UsageLimits) {}
    fn backup_updated(&self, _status: &BackupStatus) {}
    /// A desktop notification: an employee flagged something that needs you.
    fn notify(&self, _title: &str, _body: &str) {}
}

const TICK: Duration = Duration::from_secs(30);
const TIDY_FIRST: Duration = Duration::from_secs(120);
const TIDY_EVERY: Duration = Duration::from_secs(600);
const USAGE_EVERY: Duration = Duration::from_secs(300);
const BACKUP_EVERY: Duration = Duration::from_secs(60);
const WATCH_FIRST: Duration = Duration::from_secs(45);
const WATCH_EVERY: Duration = Duration::from_secs(60);
const ACCOUNT_EVERY: Duration = Duration::from_secs(60);
/// In the data dir while the app runs: a launch that finds it knows the last session did not quit cleanly.
pub(crate) const RUNNING_MARKER: &str = "omega.running";
/// In a run's folder while it works: its employee's skills as they were when it started, so a run the
/// app lost can still have its edits recorded.
const BEFORE_SNAPSHOT: &str = "before.json";
const DEFAULT_PROMPT: &str = "Work toward your objective. Start from your role skill, pick the most valuable next step, do it, and report.";

pub fn default_data_dir() -> PathBuf {
    match std::env::var_os("OMEGASTAR_DATA_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("OmegaStar"),
    }
}

/// A run claimed to start: registered as soon as it is marked running, so it can be cancelled
/// before its process exists.
struct Active {
    token: String,
    pid: Option<u32>,
    cancelled: bool,
    /// Flags raised with `notify`, copied onto the run when it ends.
    needs_you: Vec<String>,
    /// The card it raised with `notify`, copied onto the run when it ends.
    card: Option<Card>,
    /// For a Tidy run: the bigger changes it proposed with `propose_changes`.
    proposed: Option<String>,
    /// For a reply check or nudge: it recorded its finding with `check_result`.
    verdict: bool,
    /// Opened when the run starts preparing.
    transcript: Option<Arc<Mutex<File>>>,
    /// The run's open stdin, from its start until its turn ends: `Some` while it still takes messages.
    input: Option<tokio::sync::mpsc::UnboundedSender<String>>,
}

/// Lock order: the store mutex (`Office::db()`) comes first. Never call `db()` while holding `usage`,
/// `account` or `active`; a function that needs both takes the store first, or releases the other
/// lock before taking the store. No subprocess runs while the store mutex is held.
struct Inner {
    data_dir: PathBuf,
    db: Mutex<Store>,
    sink: Arc<dyn EventSink>,
    rt: tokio::runtime::Handle,
    active: Mutex<HashMap<String, Active>>,
    mcp_port: u16,
    /// Bumped whenever a run finishes; `await_reports` waits on it.
    finished: tokio::sync::watch::Sender<u64>,
    usage: Mutex<UsageLimits>,
    /// Held while a `/usage` probe runs, so probes never overlap.
    probing: tokio::sync::Mutex<()>,
    /// Held while a backup runs, so backups never overlap.
    backing_up: tokio::sync::Mutex<()>,
    /// Held while the reply monitor makes a pass, so passes never overlap.
    watching: tokio::sync::Mutex<()>,
    build: Build,
    /// The last session did not quit cleanly.
    crashed: bool,
    /// The last `claude auth status` and when it was checked; `None` until the first check.
    account: Mutex<Option<(Ts, Option<Account>)>>,
    /// The Mac is in a full wake, not a Power Nap dark wake: background runs may start.
    awake: AtomicBool,
    /// When you last saved the company page in You. The save writes no revision, so a run's guard asks here.
    company_saved: Mutex<Option<Ts>>,
}

#[derive(Clone)]
pub struct Office(Arc<Inner>);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> Ts {
    Utc::now()
}

impl Office {
    /// Opens the data dir, recovers from a previous crash, starts the MCP endpoint, and dispatches
    /// queued work. Does not start the scheduler loop (see `spawn_scheduler`).
    pub async fn open(data_dir: PathBuf, sink: Arc<dyn EventSink>, build: Build) -> Result<Office> {
        fs::create_dir_all(data_dir.join("employees"))?;
        own_skills::seed_company_page(&data_dir.join("employees"))?;
        fs::create_dir_all(data_dir.join("runs"))?;
        let data_dir = long_path(&data_dir);
        let marker = data_dir.join(RUNNING_MARKER);
        let crashed = match fs::read_to_string(&marker) {
            Ok(contents) => {
                log!("the last session did not quit cleanly (a crash, a force quit or a power loss): {}", contents.trim());
                true
            }
            Err(_) => false,
        };
        if let Err(e) = fs::write(&marker, json!({ "pid": std::process::id(), "startedAt": now(), "build": build }).to_string()) {
            log!("could not write {RUNNING_MARKER}: {e}");
        }
        let store = Store::open(&data_dir.join("office.db"))?;
        let held_until = store.held_until().ok().flatten().filter(|t| *t > now());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let office = Office(Arc::new(Inner {
            data_dir,
            db: Mutex::new(store),
            sink,
            rt: tokio::runtime::Handle::current(),
            active: Mutex::new(HashMap::new()),
            mcp_port: port,
            finished: tokio::sync::watch::channel(0).0,
            usage: Mutex::new(UsageLimits { held_until, ..UsageLimits::default() }),
            probing: tokio::sync::Mutex::new(()),
            backing_up: tokio::sync::Mutex::new(()),
            watching: tokio::sync::Mutex::new(()),
            build,
            crashed,
            account: Mutex::new(None),
            awake: AtomicBool::new(true),
            company_saved: Mutex::new(None),
        }));
        for mut run in office.recover()? {
            // The run never reached its end, where its sends are normally found.
            if let Err(e) = office.capture_deliveries(&run.id, true) {
                log!("run {}: finding the requests it sent failed: {e}", run.id);
            }
            if run.state == RunState::Lost {
                continue;
            }
            // Finished from its transcript: what a run's end normally does.
            if matches!(run.kind, RunKind::Tidy { .. }) && run.state == RunState::Succeeded {
                office.review_tidy(&mut run);
                if let Err(e) = office.db().save_run(&run) {
                    log!("saving run {} failed: {e}", run.id);
                }
            }
            if let Err(e) = office.surface_thread(&run) {
                log!("surfacing the thread after run {} failed: {e}", run.id);
            }
            if let Err(e) = office.deliver_answers(&run) {
                log!("delivering answers after run {} failed: {e}", run.id);
            }
        }
        if let Err(e) = office.seed_reviews() {
            log!("seeding weekly reviews failed: {e}");
        }
        office.0.rt.spawn(crate::mcp::serve(listener, office.clone()));
        office.pump();
        Ok(office)
    }

    /// Ticks now and every 30 s for as long as the runtime lives.
    pub fn spawn_scheduler(&self) {
        let office = self.clone();
        self.0.rt.spawn(async move {
            let mut every = tokio::time::interval(TICK);
            loop {
                every.tick().await; // the first tick completes immediately
                // Read here, off any lock, so claiming runs never waits for ioreg.
                let awake = tokio::task::spawn_blocking(crate::power::full_wake).await.unwrap_or(true);
                if office.0.awake.swap(awake, Ordering::Relaxed) != awake {
                    log!("{}", if awake { "full wake: background runs can start" } else { "dark wake: background runs wait" });
                }
                if let Err(e) = office.tick(now()) {
                    log!("scheduler tick failed: {e}");
                }
            }
        });
    }

    /// Probes `/usage` now and every 5 minutes for as long as the runtime lives.
    pub fn spawn_usage_monitor(&self) {
        let office = self.clone();
        self.0.rt.spawn(async move {
            let mut every = tokio::time::interval(USAGE_EVERY);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                every.tick().await;
                if let Ok(_probing) = office.0.probing.try_lock() {
                    office.probe_usage().await;
                }
            }
        });
    }

    /// Checks which account Claude Code is signed in to now and every minute.
    pub fn spawn_account_monitor(&self) {
        let office = self.clone();
        self.0.rt.spawn(async move {
            let mut every = tokio::time::interval(ACCOUNT_EVERY);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                every.tick().await;
                office.refresh_account().await;
            }
        });
    }

    /// Checks every minute (starting a minute after launch) whether an automatic backup is due.
    pub fn spawn_backup_monitor(&self) {
        let office = self.clone();
        self.0.rt.spawn(async move {
            let mut every = tokio::time::interval_at(tokio::time::Instant::now() + BACKUP_EVERY, BACKUP_EVERY);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                every.tick().await;
                if let Err(e) = office.backup_tick(now()).await {
                    log!("backup check failed: {e}");
                }
            }
        });
    }

    /// Watches requests to people for replies every minute, starting 45 seconds after launch.
    pub fn spawn_watch_monitor(&self) {
        let office = self.clone();
        self.0.rt.spawn(async move {
            let mut every = tokio::time::interval_at(tokio::time::Instant::now() + WATCH_FIRST, WATCH_EVERY);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                every.tick().await;
                if let Err(e) = office.watch_tick(now()).await {
                    log!("watching requests failed: {e}");
                }
            }
        });
    }

    /// Looks for employees due a tidy every ten minutes, starting two minutes after launch.
    pub fn spawn_tidy_monitor(&self) {
        let office = self.clone();
        self.0.rt.spawn(async move {
            let mut every = tokio::time::interval_at(tokio::time::Instant::now() + TIDY_FIRST, TIDY_EVERY);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                every.tick().await;
                if let Err(e) = office.tidy_tick(now()) {
                    log!("tidy check failed: {e}");
                }
            }
        });
    }

    fn db(&self) -> MutexGuard<'_, Store> {
        lock(&self.0.db)
    }

    fn sink(&self) -> &dyn EventSink {
        &*self.0.sink
    }

    pub fn data_dir(&self) -> &Path {
        &self.0.data_dir
    }

    fn employee_dir(&self, id: &str) -> PathBuf {
        self.0.data_dir.join("employees").join(id)
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.0.data_dir.join("runs").join(id)
    }

    fn skills_dir(&self, employee_id: &str) -> PathBuf {
        own_skills::skills_dir(&self.employee_dir(employee_id))
    }

    /// Settles the runs a previous session left running: a run whose transcript ends in a result is
    /// finished from it, the rest are lost. Either way the skill edits since its start are recorded
    /// when its snapshot survived. Returns the runs.
    fn recover(&self) -> Result<Vec<Run>> {
        let lost = if self.0.crashed { "Omega Star closed unexpectedly while this run was in progress." } else { "Omega Star stopped while this run was in progress." };
        let db = self.db();
        let mut settled = vec![];
        for mut r in db.runs_in_state(RunState::Running)? {
            let run_dir = self.run_dir(&r.id);
            let home = self.employee_dir(&r.employee_id);
            let path = run_dir.join("transcript.jsonl");
            let transcript = fs::read_to_string(&path).unwrap_or_default();
            let supervised = db.employee(&r.employee_id)?.is_some_and(|e| e.clearance == Clearance::Supervised);
            let before_path = run_dir.join(BEFORE_SNAPSHOT);
            let skills = own_skills::load_snapshot(&before_path).map(|before| {
                let mut after = own_skills::snapshot(&own_skills::skills_dir(&home));
                if matches!(r.kind, RunKind::Tidy { .. }) {
                    after.extend(own_skills::notes_snapshot(&home.join("workspace")));
                }
                (before, after)
            });
            let left = Leftover {
                transcript: &transcript,
                home: &home,
                supervised,
                modified: fs::metadata(&path).and_then(|m| m.modified()).ok().map(Ts::from),
                now: now(),
                stderr_tail: fs::read_to_string(run_dir.join("stderr.log")).ok().and_then(|s| runs::stderr_tail(&s)),
                skills,
            };
            let changes = settle(&mut r, left, lost);
            db.tx(|| {
                for fc in changes {
                    db.insert_revision(&r.employee_id, &own_skills::revision(fc, Some(r.id.clone()), now()))?;
                }
                db.save_run(&r)
            })?;
            let _ = fs::remove_file(&before_path);
            settled.push(r);
        }
        db.lose_unreadable_running(now(), lost)?;
        Ok(settled)
    }
}

/// What a run the last session left running left behind, read by `recover`.
struct Leftover<'a> {
    transcript: &'a str,
    /// The employee's folder, which the transcript's paths are relative to.
    home: &'a Path,
    supervised: bool,
    /// When the transcript was last written.
    modified: Option<Ts>,
    now: Ts,
    stderr_tail: Option<String>,
    /// The skills as the run started and as they are now, when its snapshot survived.
    skills: Option<(own_skills::Snapshot, own_skills::Snapshot)>,
}

/// Finishes a run from its transcript when its turn had ended, or marks it lost with `lost`, and returns
/// the skill edits it made. Pure.
fn settle(run: &mut Run, left: Leftover<'_>, lost: &str) -> Vec<own_skills::FileChange> {
    match claude::transcript_outcome(left.transcript, left.home) {
        Some(o) => {
            let (state, error) = runs::state_of(&o, left.stderr_tail);
            let ended = left.modified.unwrap_or(left.now);
            run.state = state;
            run.error = error;
            run.ended_at = Some(ended);
            run.duration_ms = run.started_at.map(|s| (ended - s).num_milliseconds().max(0) as u64);
            run.turns = Some(o.turns);
            run.summary = Some(o.text).filter(|t| !t.trim().is_empty());
            run.denials = if left.supervised { o.denials.into_iter().map(|d| d.denial).collect() } else { vec![] };
            let text = |call: &Value, key: &str| call[key].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
            run.needs_you = claude::office_calls(left.transcript, "mcp__omega__notify").iter().filter_map(|c| text(c, "message")).collect();
            run.proposed = claude::office_calls(left.transcript, "mcp__omega__propose_changes").last().and_then(|c| text(c, "plan"));
        }
        None => {
            run.state = RunState::Lost;
            run.ended_at = Some(left.now);
            run.error = Some(lost.into());
        }
    }
    let changes = left.skills.map(|(before, after)| own_skills::diff(&before, &after)).unwrap_or_default();
    run.skill_changes.extend(changes.iter().map(|c| c.change.clone()));
    changes
}

/// The watch of a delivered request.
fn watch_of(r: &mut HumanRequest) -> Option<&mut Watch> {
    r.delivery.as_mut().and_then(|d| d.watch.as_mut())
}

/// The name agents use for you, with a fallback before onboarding.
fn ceo_of(db: &Store) -> Result<String> {
    Ok(db.company()?.map(|c| c.ceo_name).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "the person you work for".into()))
}

/// The canonical, long form of a directory. Claude Code resolves the paths it is given, so an 8.3
/// short name (common in Windows %TEMP%) in the working directory makes its own files look foreign.
fn long_path(p: &Path) -> PathBuf {
    let Ok(c) = fs::canonicalize(p) else { return p.to_path_buf() };
    let s = c.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => c,
    }
}

/// Finds a member by id, slug or name (case-insensitive).
fn find_member<'a, T>(items: &'a [T], key: &str, f: impl Fn(&T) -> (&String, Option<&String>, &String)) -> Option<&'a T> {
    let k = key.trim().to_lowercase();
    items.iter().find(|x| {
        let (id, slug, name) = f(x);
        id.to_lowercase() == k || slug.map(|s| s.to_lowercase() == k).unwrap_or(false) || name.to_lowercase() == k
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOST: &str = "Omega Star stopped while this run was in progress.";

    fn home() -> &'static Path {
        Path::new("/tmp/office/employees/tester")
    }

    fn assistant(text: &str) -> String {
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "model": "claude-sonnet-4-5", "content": [{ "type": "text", "text": text }] } }).to_string()
    }

    fn notify(id: &str, message: &str) -> String {
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "model": "claude-sonnet-4-5", "content": [{ "type": "tool_use", "id": id, "name": "mcp__omega__notify", "input": { "message": message } }] } }).to_string()
    }

    fn result(text: &str, turns: u32) -> String {
        json!({ "type": "result", "subtype": "success", "is_error": false, "num_turns": turns, "duration_ms": 900, "result": text, "permission_denials": [] }).to_string()
    }

    fn started() -> Ts {
        "2026-10-14T09:00:00Z".parse().unwrap()
    }

    fn running() -> Run {
        let mut r = Run::new("e1", RunKind::Manual, "go".into(), started());
        r.state = RunState::Running;
        r.started_at = Some(r.created_at);
        r
    }

    fn left<'a>(transcript: &'a str, skills: Option<(own_skills::Snapshot, own_skills::Snapshot)>) -> Leftover<'a> {
        Leftover { transcript, home: home(), supervised: true, modified: Some(started() + chrono::Duration::minutes(8)), now: now(), stderr_tail: None, skills }
    }

    #[test]
    fn a_transcript_that_ended_finishes_the_run() {
        let transcript = [assistant("Reading."), result("Line one.\nLine two.", 3), notify("t1", "Please check the totals."), result("Done.", 2)].join("\n");
        let mut r = running();
        assert!(settle(&mut r, left(&transcript, None), LOST).is_empty());
        assert_eq!(r.state, RunState::Succeeded);
        assert_eq!(r.summary.as_deref(), Some("Line one.\nLine two.\n\nDone."));
        assert_eq!(r.turns, Some(5));
        assert_eq!(r.needs_you, vec!["Please check the totals.".to_string()]);
        assert_eq!(r.duration_ms, Some(8 * 60 * 1000));
        assert_eq!(r.error, None);
    }

    #[test]
    fn a_transcript_cut_mid_turn_is_lost() {
        let transcript = [assistant("Reading."), result("Line one.", 1), assistant("Now the second part.")].join("\n");
        let mut r = running();
        settle(&mut r, left(&transcript, None), LOST);
        assert_eq!((r.state, r.error.as_deref(), r.summary), (RunState::Lost, Some(LOST), None));
    }

    #[test]
    fn a_half_written_last_line_does_not_reopen_the_turn() {
        let transcript = format!("{}\n{}\n{{\"type\":\"assistant\",\"mess", assistant("Reading."), result("All done.", 2));
        let mut r = running();
        settle(&mut r, left(&transcript, None), LOST);
        assert_eq!((r.state, r.summary.as_deref()), (RunState::Succeeded, Some("All done.")));
    }

    #[test]
    fn a_kept_snapshot_records_the_edits() {
        let dir = std::env::temp_dir().join(format!("omegastar-recover-{}", uuid::Uuid::new_v4()));
        let skill = dir.join("skills").join("tester-notes");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "Check the inbox every morning.\n").unwrap();
        let path = dir.join(BEFORE_SNAPSHOT);
        own_skills::save_snapshot(&path, &own_skills::snapshot(&dir.join("skills"))).unwrap();
        fs::write(skill.join("SKILL.md"), "Check the inbox every morning and evening.\n").unwrap();

        let before = own_skills::load_snapshot(&path).unwrap();
        let after = own_skills::snapshot(&dir.join("skills"));
        let mut r = running();
        let changes = settle(&mut r, left("", Some((before, after))), LOST);
        assert_eq!(r.state, RunState::Lost);
        assert_eq!(changes.len(), 1);
        assert_eq!(r.skill_changes, vec![SkillChange { skill: "tester-notes".into(), file: "SKILL.md".into(), change: ChangeKind::Modified }]);
        let _ = fs::remove_dir_all(&dir);
    }
}
