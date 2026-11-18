//! The Office: the single writer. Owns the store, the run queue and lanes, the scheduler tick,
//! conversations, delegation and people requests. It talks to the UI only through an `EventSink`.

mod conversations;
mod guarded;
#[cfg(test)]
mod fixtures;
mod handoffs;
mod inbox;
mod members;
mod phone;
mod requests;
mod runs;
mod schedules;
mod settings;
mod skills;
mod tidying;
mod tools;
mod visuals;

pub use visuals::{image_data_url, image_mime, PAGE_CSP};

use crate::backup;
use crate::catalog;
use crate::claude::{self, Limit, Outcome};
use crate::health;
use crate::log::log;
use crate::model::*;
use crate::org::Org;
use crate::own_skills;
use crate::proxy;
use crate::reader::{self, ReadError};
use crate::schedule;
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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
    /// A desktop notification: an employee flagged something that needs you. One titled "<name> needs you" is a
    /// decision of that employee's, which the sink may show together with their others (`notices`); the sink also
    /// holds notifications back in quiet hours, or drops them when you turned them off.
    fn notify(&self, _title: &str, _body: &str) {}
    /// Opens a full path or an https link on your screen (`open`), or says why it could not.
    fn open(&self, _target: &str) -> std::result::Result<(), String> {
        Err("there is no screen to open it on".into())
    }
    /// Shows a notes file (`cards::is_text_file`, by its full path) beside the conversation whose first run is
    /// `root_id` on your screen (`open`), or says why it could not.
    fn show_file(&self, _root_id: &str, _path: &str) -> std::result::Result<(), String> {
        Err("there is no screen to show it on".into())
    }
}

const TICK: Duration = Duration::from_secs(30);
const TIDY_FIRST: Duration = Duration::from_secs(120);
const TIDY_EVERY: Duration = Duration::from_secs(600);
const USAGE_EVERY: Duration = Duration::from_secs(300);
const BACKUP_EVERY: Duration = Duration::from_secs(60);
const WATCH_FIRST: Duration = Duration::from_secs(45);
const WATCH_EVERY: Duration = Duration::from_secs(60);
const ACCOUNT_EVERY: Duration = Duration::from_secs(60);
const COMPACT_FIRST: Duration = Duration::from_secs(300);
/// In the data dir while the app runs: a launch that finds it knows the last session did not quit cleanly.
pub(crate) const RUNNING_MARKER: &str = "omega.running";
/// In a run's folder while it works: its employee's skills as they were when it started, so a run the
/// app lost can still have its edits recorded.
const BEFORE_SNAPSHOT: &str = "before.json";
/// In a run's folder: its transcript, one JSON event per line.
const TRANSCRIPT: &str = "transcript.jsonl";
/// Times a run OmegaStar stopped mid-turn goes back to the queue before it is lost, so one that keeps
/// crashing the app cannot loop.
const RESTART_RETRIES: u32 = 3;
const DEFAULT_PROMPT: &str = "Work toward your objective. Start from your role skill, pick the most valuable next step, do it, and report.";

/// OMEGASTAR_DATA_DIR, or the channel's own folder: only a release build opens the real office, since an
/// older build that saves a row drops the fields a newer one wrote.
pub fn default_data_dir() -> PathBuf {
    match std::env::var_os("OMEGASTAR_DATA_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => dirs::data_dir().unwrap_or_else(std::env::temp_dir).join(data_folder(BuildChannel::current())),
    }
}

/// The folder a build of `channel` keeps its office in when OMEGASTAR_DATA_DIR is not set. Pure.
fn data_folder(channel: BuildChannel) -> &'static str {
    match channel {
        BuildChannel::Release => "OmegaStar",
        BuildChannel::Dogfood => "OmegaStar-dogfood",
        BuildChannel::Dev => "OmegaStar-dev",
    }
}

/// A run claimed to start: registered as soon as it is marked running, so it can be cancelled
/// before its process exists.
#[derive(Default)]
struct Active {
    token: String,
    employee_id: String,
    /// Another run of the same employee worked at some point while this one did, so a skill edit this run
    /// saw may not be its own.
    beside: bool,
    pid: Option<u32>,
    cancelled: bool,
    /// What needs you from it (each card's message, and flags OmegaStar raised), copied onto the run when it ends.
    needs_you: Vec<String>,
    /// The cards it raised with `notify`, copied onto the run when it ends.
    cards: Vec<Card>,
    /// For a weekly review: the number it recorded with `report_number`.
    score: Option<Score>,
    /// For a Tidy run: the bigger changes it proposed with `propose_changes`.
    proposed: Option<String>,
    /// For a reply check or nudge: it recorded its finding with `check_result`.
    verdict: bool,
    /// The name it gave its own thread with `name_thread`, copied onto the run when it ends.
    title: Option<String>,
    /// Opened when the run starts preparing.
    transcript: Option<Arc<Mutex<File>>>,
    /// The run's open stdin, from its start until its turn ends: `Some` while it still takes messages.
    input: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    /// The ids of your messages sent in that it has not taken yet: it takes them at its next step.
    waiting: Vec<String>,
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
    /// The CLI proxy turned down the management key, so the usage monitor stops asking (each try
    /// brings the proxy's ban closer) until the proxy settings change or you refresh.
    proxy_refused: AtomicBool,
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
    /// Quit has begun: runs it kills stay running for the next start to put back in the queue, and none start.
    closing: AtomicBool,
    /// When you last saved the company page in You. The save writes no revision, so a run's guard asks here.
    company_saved: Mutex<Option<Ts>>,
    /// Rate limits in a row since a run last succeeded: each new hold for one waits longer (`runs::backoff`). Kept in
    /// the store with `UsageLimits::held_for_rate_limit` (`save_rate_limits`), so a restart keeps both.
    transient_limits: AtomicU32,
    /// The office's disk was too full to start runs when last checked.
    disk_full: AtomicBool,
    /// A test copy (`sandbox`): it never starts Claude Code, backs up or watches anyone.
    sandbox: bool,
}

#[derive(Clone)]
pub struct Office(Arc<Inner>);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> Ts {
    Utc::now()
}

/// The first message of the conversation `run` is a turn of (`Run::conversation_root`), counting an older build's
/// chat message as one: the message itself, or for a colleague's answer that came back into its chat, the message
/// whose ask it answers, through any answer turns there that asked on. None for anything else, or when the trail
/// is gone.
fn conversation_of(db: &Store, run: &Run) -> Result<Option<String>> {
    let mut at = run.clone();
    // Each step goes to an earlier run, so this only guards against a damaged trail.
    for _ in 0..64 {
        if matches!(at.kind, RunKind::Chat) {
            return Ok(Some(at.id));
        }
        if let Some(root) = at.conversation_root() {
            return Ok(Some(root.to_string()));
        }
        let RunKind::Answered { chat: true, asked_run_id, .. } = &at.kind else { return Ok(None) };
        let Some(asker) = db.run(asked_run_id)?.and_then(|a| a.run.parent_run_id().map(String::from)) else { return Ok(None) };
        let Some(next) = db.run(&asker)? else { return Ok(None) };
        at = next.run;
    }
    Ok(None)
}

/// The CEO is in this conversation: one they started, a reply, or a turn of an older build's chat. Not
/// is_conversation(): an answer that came back into a conversation starts from a colleague's report.
fn ceo_in(kind: &RunKind) -> bool {
    matches!(kind, RunKind::Chat | RunKind::Task { .. } | RunKind::Reply { .. })
}

/// You are in this run (`ceo_in`), unless it is a due check OmegaStar brought back into a conversation: that turn
/// runs with nobody watching, as a check run does.
fn ceo_present(run: &Run) -> bool {
    ceo_in(&run.kind) && run.check.is_none()
}

impl Office {
    /// Opens the data dir, recovers from a previous crash, starts the MCP endpoint, and dispatches
    /// queued work. Does not start the scheduler loop (see `spawn_scheduler`).
    pub async fn open(data_dir: PathBuf, sink: Arc<dyn EventSink>, build: Build) -> Result<Office> {
        crate::sandbox::check_office(build.channel, &data_dir, crate::sandbox::release_dir().as_deref())?;
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
        let store = Store::open(&data_dir.join(crate::store::DB_FILE))?;
        let held_until = store.held_until().ok().flatten().filter(|t| *t > now());
        let (rate_limits, rate_hold) = store.rate_limits().unwrap_or_default();
        let sandbox = crate::sandbox::is_sandbox(&data_dir);
        if sandbox {
            log!("this office is a sandbox: it never starts Claude Code, backs up or watches anyone");
        }
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
            usage: Mutex::new(UsageLimits { held_until, held_for_rate_limit: rate_hold && held_until.is_some(), ..UsageLimits::default() }),
            probing: tokio::sync::Mutex::new(()),
            proxy_refused: AtomicBool::new(false),
            backing_up: tokio::sync::Mutex::new(()),
            watching: tokio::sync::Mutex::new(()),
            build,
            crashed,
            account: Mutex::new(None),
            awake: AtomicBool::new(true),
            closing: AtomicBool::new(false),
            company_saved: Mutex::new(None),
            transient_limits: AtomicU32::new(rate_limits),
            disk_full: AtomicBool::new(false),
            sandbox,
        }));
        office.adopt_chat_turns()?;
        for mut run in office.recover()? {
            // Back in the queue to carry on: its end finds its sends and reports it, as for any run.
            if run.state == RunState::Queued {
                continue;
            }
            // The run never reached its end, where its sends are normally found.
            if let Err(e) = office.capture_deliveries(&run.id, true) {
                log!("run {}: finding the requests it sent failed: {e}", run.id);
            }
            // A lost run still hands on the reports that waited for it, and reports back to whoever handed it over.
            if let Err(e) = office.deliver_answers(&run) {
                log!("delivering answers after run {} failed: {e}", run.id);
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
                    log!("{}", if awake { "full wake" } else { "dark wake" });
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
                if office.0.proxy_refused.load(Ordering::Relaxed) {
                    continue;
                }
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

    /// Once per office, five minutes after launch: takes the image data out of the transcripts older builds saved
    /// whole, in the background (`compact_transcripts`). A launch that stops it before the end starts it again.
    pub fn spawn_transcript_compaction(&self) {
        let office = self.clone();
        self.0.rt.spawn(async move {
            if office.db().transcripts_compacted().unwrap_or(true) {
                return;
            }
            tokio::time::sleep(COMPACT_FIRST).await;
            let compacting = office.clone();
            match tokio::task::spawn_blocking(move || compacting.compact_transcripts()).await {
                Ok(saved) => {
                    log!("took the images out of finished runs' transcripts, saving {} MB", saved / (1024 * 1024));
                    if let Err(e) = office.db().set_transcripts_compacted() {
                        log!("could not record that transcripts were compacted: {e}");
                    }
                }
                Err(e) => log!("taking the images out of transcripts failed: {e}"),
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

    /// A test copy (`sandbox`), which never links a phone either.
    pub fn is_sandbox(&self) -> bool {
        self.0.sandbox
    }

    /// This build's version, as the app shows it.
    pub fn version(&self) -> &str {
        &self.0.build.version
    }

    /// Runs `task` on the runtime the office runs on, for as long as the app runs.
    pub fn spawn(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        self.0.rt.spawn(task);
    }

    fn employee_dir(&self, id: &str) -> PathBuf {
        self.0.data_dir.join("employees").join(id)
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.0.data_dir.join("runs").join(id)
    }

    fn transcript_path(&self, id: &str) -> PathBuf {
        self.run_dir(id).join(TRANSCRIPT)
    }

    fn skills_dir(&self, employee_id: &str) -> PathBuf {
        own_skills::skills_dir(&self.employee_dir(employee_id))
    }

    /// Moves the turns an older build's chat left queued or running into the work lane, as this build makes
    /// them, since nothing shows a chat-lane run once it ends: your message there becomes a conversation you
    /// started, and a colleague's answer that came back into the chat the next turn of the chat message that
    /// asked, so it waits for you in that conversation whoever the employee reports to.
    fn adopt_chat_turns(&self) -> Result<()> {
        let db = self.db();
        for state in [RunState::Queued, RunState::Running] {
            for mut r in db.runs_in_state(state)?.into_iter().filter(|r| r.kind.lane() == Lane::Chat) {
                let message = conversation_of(&db, &r)?;
                r.kind = match r.kind {
                    RunKind::Answered { asked_run_id, from, task, .. } => {
                        let thread = match message {
                            Some(id) if db.run(&id)?.is_some() => Some(id),
                            _ => None,
                        };
                        RunKind::Answered { asked_run_id, from, task, chat: false, thread }
                    }
                    _ => RunKind::Task { label: threads::task_label(&r.prompt, &r.attachments), from: None, context: String::new() },
                };
                db.save_run_moved(&r)?;
            }
        }
        Ok(())
    }

    /// Settles the runs a previous session left running: a run whose transcript ends in a result is
    /// finished from it, the rest go back to the queue to carry on in their session, or are lost once they
    /// have too often. Either way the skill edits since its start are recorded when its snapshot survived.
    /// Returns the runs.
    fn recover(&self) -> Result<Vec<Run>> {
        let lost = if self.0.crashed { "OmegaStar closed unexpectedly while this run was in progress." } else { "OmegaStar stopped while this run was in progress." };
        let db = self.db();
        let mut settled = vec![];
        for mut r in db.runs_in_state(RunState::Running)? {
            let run_dir = self.run_dir(&r.id);
            let home = self.employee_dir(&r.employee_id);
            let path = run_dir.join(TRANSCRIPT);
            let transcript = fs::read_to_string(&path).unwrap_or_default();
            let employee = db.employee(&r.employee_id)?;
            let supervised = employee.as_ref().is_some_and(|e| e.clearance == Clearance::Supervised);
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
            let since = r.started_at.unwrap_or(r.created_at);
            let changes = settle(&mut r, left, lost);
            let slug = employee.as_ref().map_or_else(|| r.employee_id.clone(), |e| e.slug.clone());
            // A card's file and images as notify checks them, so a card read back from the transcript shows them only if they pass.
            let workspaces = employee.map(|e| e.workspaces).unwrap_or_default();
            for c in &mut r.cards {
                c.file = c.file.take().and_then(|f| tools::card_file_path(&home, &workspaces, &f).ok()).map(|p| p.to_string_lossy().into_owned());
                c.images = c.images.iter().filter_map(|i| tools::card_image_path(&home, &workspaces, i).ok()).map(|p| p.to_string_lossy().into_owned()).collect();
            }
            db.tx(|| {
                runs::record_changes(&db, &r.employee_id, &r.id, since, changes)?;
                db.save_run(&r)
            })?;
            let _ = fs::remove_file(&before_path);
            match r.state {
                RunState::Queued => log!("requeued {} after OmegaStar stopped (attempt {})", r.id, r.restarts),
                RunState::Failed | RunState::Lost => runs::log_failed(&r, &slug, None),
                _ => {}
            }
            settled.push(r);
        }
        let unreadable = db.lose_unreadable_running(now(), lost)?;
        if unreadable > 0 {
            log!("lost {unreadable} running runs this build cannot read");
        }
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

/// Finishes a run from its transcript when its turn had ended. Otherwise puts it back in the queue, as if it
/// never started, to carry on in its session, or marks it lost with `lost` once it has been back
/// `RESTART_RETRIES` times. A run back in the queue before is judged by its last attempt's lines alone
/// (`Run::attempt_from`): an earlier attempt's result, such as the limit that sent it back, is not how this one
/// ended. Returns the skill edits it saw, its own being those its calls wrote. Pure.
fn settle(run: &mut Run, left: Leftover<'_>, lost: &str) -> Vec<runs::Seen> {
    let attempt = left.transcript.get(run.attempt_from as usize..).unwrap_or(left.transcript);
    match claude::transcript_outcome(attempt, left.home) {
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
            // The cards it raised, as `notify` took them: a later card about the same thing replaces an earlier one.
            let mut cards: Vec<Card> = vec![];
            for call in claude::office_calls(left.transcript, "mcp__omega__notify") {
                let Ok(card) = crate::cards::card(&call) else { continue };
                cards.retain(|c| c.topic().is_none() || c.topic() != card.topic());
                cards.push(card);
            }
            run.needs_you = cards.iter().map(|c| c.message.clone()).collect();
            run.cards = cards;
            if matches!(run.kind, RunKind::Scheduled { review: true, .. }) {
                run.score = claude::office_calls(left.transcript, "mcp__omega__report_number").last().and_then(|c| {
                    Some(Score { metric: text(c, "metric")?, value: text(c, "value")?, change: text(c, "change").unwrap_or_default(), note: text(c, "note").unwrap_or_default() })
                });
            }
            if matches!(run.kind, RunKind::Tidy { .. }) {
                run.proposed = claude::office_calls(left.transcript, "mcp__omega__propose_changes").last().and_then(|c| text(c, "plan"));
            }
        }
        None if run.restarts < RESTART_RETRIES => {
            run.state = RunState::Queued;
            run.started_at = None;
            run.restarts += 1;
            run.restarted = true;
            run.attempt_from = left.transcript.len() as u64;
        }
        None => {
            run.state = RunState::Lost;
            run.ended_at = Some(left.now);
            run.error = Some(lost.into());
        }
    }
    // Who else worked beside it before the app stopped is not known, so only what its calls show is its own.
    let changes = left.skills.map(|(before, after)| own_skills::diff(&before, &after)).unwrap_or_default();
    let moved = if changes.is_empty() { HashSet::new() } else { runs::moved_skills(left.transcript) };
    let changes: Vec<runs::Seen> = changes
        .into_iter()
        .map(|c| {
            let wrote = moved.contains(&c.change.skill)
                || own_skills::recorded_path(left.home, &c.change.skill, &c.change.file).is_ok_and(|p| skills::wrote_file(left.transcript, &p, left.home));
            runs::Seen { own: wrote, wrote, change: c }
        })
        .collect();
    run.skill_changes.extend(runs::own_changes(&changes));
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

/// Below this much free space on the office's disk no run starts.
const MIN_FREE_GB: u64 = 2;
const MIN_FREE_BYTES: u64 = MIN_FREE_GB * 1024 * 1024 * 1024;

/// Too little space to start runs. Unknown free space never stops them. Pure.
fn too_full(free: Option<u64>) -> bool {
    free.is_some_and(|b| b < MIN_FREE_BYTES)
}

/// The space free to this user on the disk that holds `path`; `None` when it cannot be read.
#[cfg(unix)]
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs only writes into the zeroed struct, and `c` is a NUL-terminated path.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}

#[cfg(windows)]
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, free_to_caller: *mut u64, total: *mut u64, total_free: *mut u64) -> i32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    // SAFETY: `wide` is a NUL-terminated path; the call writes only `free`, and the other outputs may be null.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

#[cfg(not(any(unix, windows)))]
fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// Finds a member by id, slug or name (case-insensitive).
fn find_member<'a, T>(items: &'a [T], key: &str, f: impl Fn(&T) -> (&String, Option<&String>, &String)) -> Option<&'a T> {
    let k = key.trim().to_lowercase();
    items.iter().find(|x| {
        let (id, slug, name) = f(x);
        id.to_lowercase() == k || slug.is_some_and(|s| s.to_lowercase() == k) || name.to_lowercase() == k
    })
}

fn find_employee<'a>(items: &'a [Employee], key: &str) -> Option<&'a Employee> {
    find_member(items, key, |e| (&e.id, Some(&e.slug), &e.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{task, temp_path};

    const LOST: &str = "OmegaStar stopped while this run was in progress.";

    fn home() -> &'static Path {
        Path::new("/tmp/office/employees/tester")
    }

    fn assistant(text: &str) -> String {
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "model": "claude-sonnet-4-5", "content": [{ "type": "text", "text": text }] } }).to_string()
    }

    fn notify(id: &str, input: Value) -> String {
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "model": "claude-sonnet-4-5", "content": [{ "type": "tool_use", "id": id, "name": "mcp__omega__notify", "input": input }] } }).to_string()
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
    fn only_a_release_build_opens_the_real_office() {
        assert_eq!(data_folder(BuildChannel::Release), "OmegaStar");
        assert_eq!(data_folder(BuildChannel::Dogfood), "OmegaStar-dogfood");
        assert_eq!(data_folder(BuildChannel::Dev), "OmegaStar-dev");
    }

    #[test]
    fn a_transcript_that_ended_finishes_the_run() {
        let ask = |message: &str| json!({ "message": message, "key": "totals", "choices": [{ "label": "Recheck", "recommended": true }, { "label": "Leave it" }] });
        let transcript = [
            assistant("Reading."),
            result("Line one.\nLine two.", 3),
            notify("t1", ask("Check the totals?")),
            notify("t2", json!({ "message": "FYI, totals are in." })),
            notify("t3", ask("Recheck the totals?")),
            result("Done.", 2),
        ]
        .join("\n");
        let mut r = running();
        assert!(settle(&mut r, left(&transcript, None), LOST).is_empty());
        assert_eq!(r.state, RunState::Succeeded);
        assert_eq!(r.summary.as_deref(), Some("Line one.\nLine two.\n\nDone."));
        assert_eq!(r.turns, Some(5));
        assert_eq!(r.needs_you, vec!["Recheck the totals?".to_string()], "the card notify took, the later one about the same thing");
        assert_eq!(r.cards.iter().map(|c| c.message.as_str()).collect::<Vec<_>>(), vec!["Recheck the totals?"]);
        assert_eq!(r.duration_ms, Some(8 * 60 * 1000));
        assert_eq!(r.error, None);
    }

    #[test]
    fn a_transcript_cut_mid_turn_goes_back_to_the_queue() {
        let transcript = [assistant("Reading."), result("Line one.", 1), assistant("Now the second part.")].join("\n");
        let mut r = running();
        r.limit_retries = 1;
        settle(&mut r, left(&transcript, None), LOST);
        assert_eq!((r.state, r.started_at, r.ended_at, r.error, r.summary), (RunState::Queued, None, None, None, None));
        assert_eq!((r.restarts, r.restarted, r.limit_retries), (1, true, 1), "it is told it was the restart, not the limit");
    }

    #[test]
    fn a_requeued_run_is_judged_by_its_last_attempt_alone() {
        let limited = json!({ "type": "result", "subtype": "success", "is_error": true, "num_turns": 1, "duration_ms": 500,
            "result": "API Error: Server is temporarily limiting requests.", "permission_denials": [], "api_error_status": 429 })
        .to_string();
        let first = format!("{}\n{limited}\n", assistant("Reading."));
        // The limit sent it back to the queue, and OmegaStar stopped before its next attempt wrote a line.
        let mut r = running();
        r.limit_retries = 1;
        r.attempt_from = first.len() as u64;
        settle(&mut r, left(&first, None), LOST);
        assert_eq!((r.state, r.error, r.restarts), (RunState::Queued, None, 1), "the 429 was the last attempt's, not this one's");
        assert_eq!(r.attempt_from, first.len() as u64, "its next attempt starts after it");

        // An attempt that ended is finished from its own lines.
        let both = format!("{first}{}\n{}", assistant("Reading again."), result("Swept the inbox.", 2));
        let mut r = running();
        r.limit_retries = 1;
        r.attempt_from = first.len() as u64;
        settle(&mut r, left(&both, None), LOST);
        assert_eq!((r.state, r.summary.as_deref(), r.turns), (RunState::Succeeded, Some("Swept the inbox."), Some(2)));
    }

    #[test]
    fn a_run_cut_off_too_often_is_lost() {
        let mut r = running();
        r.restarts = RESTART_RETRIES;
        settle(&mut r, left(&assistant("Reading."), None), LOST);
        assert_eq!((r.state, r.error.as_deref(), r.restarts), (RunState::Lost, Some(LOST), RESTART_RETRIES));
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
        let dir = temp_path("recover");
        let skill = dir.join("skills").join("tester-notes");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "Check the inbox every morning.\n").unwrap();
        let path = dir.join(BEFORE_SNAPSHOT);
        own_skills::save_snapshot(&path, &own_skills::snapshot(&dir.join("skills"))).unwrap();
        fs::write(skill.join("SKILL.md"), "Check the inbox every morning and evening.\n").unwrap();

        let before = own_skills::load_snapshot(&path).unwrap();
        let after = own_skills::snapshot(&dir.join("skills"));
        let mut r = running();
        let changes = settle(&mut r, left("", Some((before.clone(), after.clone()))), LOST);
        assert_eq!(r.state, RunState::Queued, "edits made before the restart are recorded, though the run goes again");
        assert_eq!(changes.len(), 1);
        assert!(!changes[0].own && r.skill_changes.is_empty(), "nothing on record says this run made it");

        let edit = json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "content": [{ "type": "tool_use", "id": "t1", "name": "Edit",
            "input": { "file_path": home().join("skills/tester-notes/SKILL.md"), "old_string": "morning", "new_string": "morning and evening" } }] } });
        let done = json!({ "type": "user", "parent_tool_use_id": null, "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "ok" }] } });
        let mut r = running();
        let changes = settle(&mut r, left(&format!("{edit}\n{done}"), Some((before, after))), LOST);
        assert!(changes[0].own && changes[0].wrote);
        assert_eq!(r.skill_changes, vec![SkillChange { skill: "tester-notes".into(), file: "SKILL.md".into(), change: ChangeKind::Modified }]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn too_little_free_space_holds_work() {
        assert!(too_full(Some(MIN_FREE_BYTES - 1)));
        assert!(!too_full(Some(MIN_FREE_BYTES)));
        assert!(!too_full(None), "unknown free space never stops work");
        let dir = std::env::temp_dir();
        assert!(free_bytes(&dir).is_some_and(|b| b > 0), "the temp folder's disk has room");
        assert_eq!(free_bytes(&dir.join(format!("omegastar-missing-{}", uuid::Uuid::new_v4()))), None);
    }

    #[test]
    fn only_conversations_and_replies_have_you_in_them() {
        let scheduled = RunKind::Scheduled { schedule_id: "s1".into(), slot: now(), label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
        assert!(!ceo_in(&RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat: true, thread: None }));
        assert!(!ceo_in(&RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat: false, thread: Some("t1".into()) }));
        assert!(!ceo_in(&scheduled));
        assert!(!ceo_in(&RunKind::Delegated { parent_run_id: "r1".into() }));
        assert!(ceo_in(&RunKind::Chat));
        assert!(ceo_in(&task("Draft it")));
        assert!(ceo_in(&RunKind::Reply { to_run_id: "r1".into(), label: "Sweep".into(), task: false, card: false }));
    }
}
