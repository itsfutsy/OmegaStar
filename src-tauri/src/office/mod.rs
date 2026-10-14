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
    /// For a Tidy run: the bigger changes it proposed with `propose_changes`.
    proposed: Option<String>,
    /// For a reply check or nudge: it recorded its finding with `check_result`.
    verdict: bool,
    /// Opened when the run starts preparing.
    transcript: Option<Arc<Mutex<File>>>,
    /// The run's open stdin, from its start until its turn ends: `Some` while it still takes messages.
    input: Option<tokio::sync::mpsc::UnboundedSender<String>>,
}

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
            usage: Mutex::new(UsageLimits::default()),
            probing: tokio::sync::Mutex::new(()),
            backing_up: tokio::sync::Mutex::new(()),
            watching: tokio::sync::Mutex::new(()),
            build,
            crashed,
            account: Mutex::new(None),
        }));
        for run_id in office.recover()? {
            // The run never reached its end, where its sends are normally found.
            if let Err(e) = office.capture_deliveries(&run_id, true) {
                log!("run {run_id}: finding the requests it sent failed: {e}");
            }
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

    /// Marks the runs a previous session left running as lost, and returns their ids.
    fn recover(&self) -> Result<Vec<String>> {
        let db = self.db();
        let mut lost = vec![];
        for mut r in db.runs_in_state(RunState::Running)? {
            r.state = RunState::Lost;
            r.ended_at = Some(now());
            r.error = Some(if self.0.crashed { "Omega Star closed unexpectedly while this run was in progress." } else { "Omega Star stopped while this run was in progress." }.into());
            db.save_run(&r)?;
            lost.push(r.id);
        }
        db.lose_unreadable_running(now(), "Omega Star stopped while this run was in progress.")?;
        Ok(lost)
    }
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
