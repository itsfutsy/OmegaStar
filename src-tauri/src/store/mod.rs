//! SQLite persistence. Each domain object is stored as its JSON form in `data`, with the columns
//! the Office queries on derived from it in exactly one place (each domain's save functions).

use crate::log::log;
use crate::model::*;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::Mutex;

pub use requests::{RequestFilter, RequestRun};

pub struct Store {
    conn: Connection,
}

/// The whole schema. Every statement is idempotent, so opening an existing office changes nothing.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS employees(id TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, departed_at INTEGER, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS people(id TEXT PRIMARY KEY, removed_at INTEGER, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS schedules(id TEXT PRIMARY KEY, employee_id TEXT NOT NULL, last_slot INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS runs(
  id TEXT PRIMARY KEY,
  employee_id TEXT NOT NULL,
  lane TEXT NOT NULL,
  state TEXT NOT NULL,
  schedule_id TEXT,
  slot INTEGER,
  parent_run_id TEXT,
  request_id TEXT,
  session TEXT NOT NULL,
  inited INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  data TEXT NOT NULL,
  UNIQUE(schedule_id, slot)
);
DROP INDEX IF EXISTS runs_one_running_per_lane;
CREATE UNIQUE INDEX IF NOT EXISTS runs_one_running_per_session ON runs(session) WHERE state = 'running';
CREATE UNIQUE INDEX IF NOT EXISTS runs_one_follow_up ON runs(request_id) WHERE request_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS runs_by_employee ON runs(employee_id, created_at);
CREATE INDEX IF NOT EXISTS runs_by_state ON runs(state, created_at);
CREATE INDEX IF NOT EXISTS runs_by_parent ON runs(parent_run_id);
CREATE TABLE IF NOT EXISTS requests(
  id TEXT PRIMARY KEY, person_id TEXT NOT NULL, employee_id TEXT NOT NULL, state TEXT NOT NULL,
  created_at INTEGER NOT NULL, data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS skill_revisions(
  id TEXT PRIMARY KEY, employee_id TEXT NOT NULL, run_id TEXT, at INTEGER NOT NULL, data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS revisions_by_employee ON skill_revisions(employee_id, at);
CREATE TABLE IF NOT EXISTS proposals(
  id TEXT PRIMARY KEY, employee_id TEXT NOT NULL, state TEXT NOT NULL, created_at INTEGER NOT NULL, data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS proposals_by_state ON proposals(state, created_at);
CREATE TABLE IF NOT EXISTS checks(
  id TEXT PRIMARY KEY, employee_id TEXT NOT NULL, state TEXT NOT NULL, due_at INTEGER NOT NULL, created_at INTEGER NOT NULL, data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS checks_by_employee ON checks(employee_id, state, due_at);
CREATE TABLE IF NOT EXISTS news(
  id TEXT PRIMARY KEY, employee_id TEXT NOT NULL, created_at INTEGER NOT NULL, read_at INTEGER, data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS news_by_employee ON news(employee_id, read_at, created_at);
CREATE TABLE IF NOT EXISTS skill_use(employee_id TEXT NOT NULL, skill TEXT NOT NULL, used_at INTEGER NOT NULL, PRIMARY KEY(employee_id, skill));
"#;

/// The file the office database lives in, inside the data folder.
pub const DB_FILE: &str = "office.db";

/// The meta key of the last backup's outcome, which the backup's own hash leaves out.
pub const BACKUP_STATUS: &str = "backup_status";

/// How many messages of your own you send before OmegaStar asks you, once, to star it on GitHub.
const STAR_ASK_AFTER: i64 = 30;

/// The one-time ask to star OmegaStar on GitHub, kept under the meta key `star_ask` (absent: `Counting`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum StarAsk {
    /// Fewer than `STAR_ASK_AFTER` of your own messages so far.
    #[default]
    Counting,
    /// The sidebar asks until you answer.
    Due,
    /// You starred it or said no thanks, so it never asks again.
    Answered,
}

/// The states of a run not finished yet (`RunState::is_finished`), as an SQL list.
macro_rules! active {
    () => {
        "('queued', 'running')"
    };
}

/// Each card of a run's row, as `c`.
macro_rules! cards {
    () => {
        "json_each(CASE WHEN json_valid(data) THEN data ELSE '{}' END, '$.cards') c"
    };
}

/// A card `c` still waiting on you (`Card::waiting`): no answer yet, or put off with Later.
macro_rules! card_waiting {
    () => {
        "(json_extract(c.value, '$.answer') IS NULL
                             OR json_extract(c.value, '$.backAt') IS NOT NULL OR json_extract(c.value, '$.parked') = 1)"
    };
}

/// The schedule ids of due checks (`CHECK_PREFIX`), as a LIKE pattern.
macro_rules! check_run {
    () => {
        "'check:%'"
    };
}

mod cards;
mod inbox;
mod org;
mod requests;
mod runs;
mod skills;
#[cfg(test)]
mod tests;

fn ms(t: Ts) -> i64 {
    t.timestamp_millis()
}

fn to_json<T: Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_string(v)?)
}

/// The error names the row by the start of its JSON, which begins with the row's id.
fn from_row<T: DeserializeOwned>(row: &Row, idx: usize) -> rusqlite::Result<T> {
    let s: String = row.get(idx)?;
    serde_json::from_str(&s).map_err(|e| {
        let head: String = s.chars().take(60).collect();
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(AppError::internal(format!("{e} in {head}"))))
    })
}

fn record(r: &Row) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord { run: from_row(r, 0)?, session: r.get(1)? })
}

/// The rows this build can read. A row it cannot (written by a newer build, or damaged) is left out
/// and logged once per process, so one bad row never hides every other one.
fn readable<T>(table: &str, rows: impl Iterator<Item = rusqlite::Result<T>>) -> Result<Vec<T>> {
    static LOGGED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let mut v = vec![];
    for row in rows {
        match row {
            Ok(r) => v.push(r),
            Err(err @ rusqlite::Error::FromSqlConversionFailure(..)) => {
                if LOGGED.lock().unwrap_or_else(|e| e.into_inner()).insert(err.to_string()) {
                    log!("left out a {table} row this build cannot read: {err}");
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(v)
}

pub struct RunRecord {
    pub run: Run,
    pub session: String,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(&format!("BEGIN; {SCHEMA} COMMIT;"))?;
        let store = Store { conn };
        store.migrate_cards()?;
        store.drop_unrecorded_revisions()?;
        store.surface_hidden_cards()?;
        if store.meta::<Ts>("skill_use_since")?.is_none() {
            store.set_meta("skill_use_since", &chrono::Utc::now())?;
        }
        if store.meta::<Ts>("typed_answers_since")?.is_none() {
            store.set_meta("typed_answers_since", &chrono::Utc::now())?;
        }
        store.count_star_ask()?;
        Ok(store)
    }

    /// Once: deletes the revisions older builds recorded of files that skill snapshots now leave out, as
    /// `own_skills` does: anything in a dependency or cache folder, and data files. Nobody wrote them, and they
    /// were most of the table.
    fn drop_unrecorded_revisions(&self) -> Result<()> {
        const DIRS: [&str; 5] = ["node_modules", "__pycache__", ".cache", ".venv", ".git"];
        const EXTS: [&str; 3] = ["json", "jsonl", "csv"];
        if self.meta::<bool>("unrecorded_revisions_dropped")?.unwrap_or(false) {
            return Ok(());
        }
        let left_out: Vec<String> = DIRS.iter().map(|d| format!("instr(f, '/{d}/') > 0")).chain(EXTS.iter().map(|x| format!("f LIKE '%.{x}'"))).collect();
        self.tx(|| {
            let n = self.conn.execute(
                &format!(
                    "DELETE FROM skill_revisions WHERE id IN (
                       SELECT id FROM (SELECT id, '/' || json_extract(data, '$.file') AS f FROM skill_revisions
                                       WHERE json_valid(data) AND json_extract(data, '$.skill') NOT LIKE '@%')
                       WHERE {})",
                    left_out.join(" OR ")
                ),
                [],
            )?;
            if n > 0 {
                log!("deleted {n} skill revisions of dependency and data files, which skills no longer record");
            }
            self.set_meta("unrecorded_revisions_dropped", &true)
        })
    }

    /// Once: an approved tidy filed with a card you never answered comes back to the Inbox. Older builds filed a tidy
    /// the moment you approved it, whatever else it asked, so the card left your Inbox while its employee was still
    /// told it waited on you. Your approval answered the tidy's own flags, so only its cards keep it there.
    fn surface_hidden_cards(&self) -> Result<()> {
        if self.meta::<bool>("hidden_cards_surfaced")?.unwrap_or(false) {
            return Ok(());
        }
        self.tx(|| {
            let filed = self.runs_where(
                concat!("WHERE json_valid(data) AND json_extract(data, '$.approvedAt') IS NOT NULL AND json_extract(data, '$.archivedAt') IS NOT NULL
                   AND EXISTS (SELECT 1 FROM ", cards!(), " WHERE json_extract(c.value, '$.answer') IS NULL)"),
                &[],
            )?;
            let mut n = 0;
            for mut r in filed.into_iter().filter(Run::has_open_card) {
                r.needs_you.retain(|m| r.cards.iter().any(|c| c.message == *m));
                r.archived_at = None;
                self.save_run(&r)?;
                n += 1;
            }
            if n > 0 {
                log!("brought back {n} approved tidies that were filed with a card you never answered");
            }
            self.set_meta("hidden_cards_surfaced", &true)
        })
    }

    /// Once: a run's one `card` becomes its list of `cards`, and a chore card gets the key every card now
    /// has, so every card query reads one shape.
    fn migrate_cards(&self) -> Result<()> {
        if self.meta::<bool>("cards_listed")?.unwrap_or(false) {
            return Ok(());
        }
        self.tx(|| {
            self.conn.execute(
                "UPDATE runs SET data = json_set(json_remove(data, '$.card'), '$.cards',
                   CASE
                     WHEN json_type(data, '$.card') != 'object' THEN json_array()
                     WHEN json_extract(data, '$.card.ask.type') = 'chore'
                       THEN json_array(json_set(json(json_extract(data, '$.card')), '$.key', 'chore:' || json_extract(data, '$.card.ask.subject')))
                     ELSE json_array(json(json_extract(data, '$.card')))
                   END)
                 WHERE json_valid(data) AND json_type(data, '$.card') IS NOT NULL",
                [],
            )?;
            self.set_meta("cards_listed", &true)
        })
    }

    /// Runs `f` as one unit: every write it makes lands, or none does. A savepoint, so units nest.
    pub fn tx<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("SAVEPOINT tx")?;
        match f() {
            Ok(v) => {
                self.conn.execute_batch("RELEASE tx")?;
                Ok(v)
            }
            Err(e) => {
                if let Err(undo) = self.conn.execute_batch("ROLLBACK TO tx; RELEASE tx") {
                    log!("rolling back a failed write failed: {undo}");
                }
                Err(e)
            }
        }
    }

    // ---- meta ----

    fn meta<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?", [key], |r| from_row(r, 0))
            .optional()?)
    }

    fn set_meta<T: Serialize>(&self, key: &str, v: &T) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, to_json(v)?],
        )?;
        Ok(())
    }

    pub fn company(&self) -> Result<Option<Company>> {
        self.meta("company")
    }
    pub fn set_company(&self, c: &Company) -> Result<()> {
        self.set_meta("company", c)
    }
    pub fn settings(&self) -> Result<Option<AppSettings>> {
        self.meta("settings")
    }
    pub fn set_settings(&self, s: &AppSettings) -> Result<()> {
        self.set_meta("settings", s)
    }
    pub fn backup_status(&self) -> Result<Option<BackupStatus>> {
        self.meta(BACKUP_STATUS)
    }
    pub fn set_backup_status(&self, s: &BackupStatus) -> Result<()> {
        self.set_meta(BACKUP_STATUS, s)
    }
    /// When the usage limit that holds the office resets; `None` when nothing holds it.
    pub fn held_until(&self) -> Result<Option<Ts>> {
        Ok(self.meta::<Option<Ts>>("held_until")?.flatten())
    }
    pub fn set_held_until(&self, t: Option<Ts>) -> Result<()> {
        self.set_meta("held_until", &t)
    }
    /// Rate limits in a row since a run last succeeded, and whether the hold on work is for one, so a restart keeps
    /// the backoff and a usage check cannot end the hold.
    pub fn rate_limits(&self) -> Result<(u32, bool)> {
        Ok(self.meta::<(u32, bool)>("rate_limits")?.unwrap_or_default())
    }
    pub fn set_rate_limits(&self, in_a_row: u32, holding: bool) -> Result<()> {
        self.set_meta("rate_limits", &(in_a_row, holding))
    }
    /// Whether every employee hired before weekly reviews existed has been given one.
    pub fn reviews_seeded(&self) -> Result<bool> {
        Ok(self.meta::<bool>("reviewsSeeded")?.unwrap_or(false))
    }
    pub fn set_reviews_seeded(&self) -> Result<()> {
        self.set_meta("reviewsSeeded", &true)
    }
    /// Whether the transcripts runs saved before images were left out of them have had theirs taken out.
    pub fn transcripts_compacted(&self) -> Result<bool> {
        Ok(self.meta::<bool>("transcriptsCompacted")?.unwrap_or(false))
    }
    pub fn set_transcripts_compacted(&self) -> Result<()> {
        self.set_meta("transcriptsCompacted", &true)
    }

    /// Whether the sidebar asks you to star OmegaStar on GitHub now.
    pub fn star_ask_due(&self) -> Result<bool> {
        Ok(self.star_ask()? == StarAsk::Due)
    }

    /// You starred OmegaStar or said no thanks, so the ask never comes back. Safe at any time and more than once.
    pub fn answer_star_ask(&self) -> Result<()> {
        self.set_meta("star_ask", &StarAsk::Answered)
    }

    fn star_ask(&self) -> Result<StarAsk> {
        Ok(self.meta("star_ask")?.unwrap_or_default())
    }

    /// While still counting, makes the ask due once you have sent `STAR_ASK_AFTER` messages of your own: conversation
    /// runs OmegaStar did not write. Answers that come back into a conversation are always OmegaStar's, so only a chat
    /// turn, a task or a reply can be yours. The count stops at the threshold, and nothing is counted once it is due.
    /// Runs when a message of yours is queued, and when the office opens, so messages from older builds count too.
    fn count_star_ask(&self) -> Result<()> {
        if self.star_ask()? != StarAsk::Counting {
            return Ok(());
        }
        let own: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM runs WHERE json_valid(data) AND json_extract(data, '$.kind.type') IN ('chat', 'task', 'reply')
               AND COALESCE(json_extract(data, '$.omegaWrote'), 0) = 0 LIMIT ?1)",
            [STAR_ASK_AFTER],
            |r| r.get(0),
        )?;
        if own >= STAR_ASK_AFTER {
            self.set_meta("star_ask", &StarAsk::Due)?;
        }
        Ok(())
    }

    /// Writes a consistent copy of the whole database to `path`, which must not exist.
    pub fn snapshot_to(&self, path: &Path) -> Result<()> {
        self.conn.execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
        Ok(())
    }

    fn runs_where(&self, clause: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Run>> {
        let mut st = self.conn.prepare(&format!("SELECT data FROM runs {clause}"))?;
        let v = readable("runs", st.query_map(p, |r| from_row(r, 0))?)?;
        Ok(v)
    }

    /// `runs_where` with each run's session.
    fn records_where(&self, clause: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<RunRecord>> {
        let mut st = self.conn.prepare(&format!("SELECT data, session FROM runs {clause}"))?;
        let v = readable("runs", st.query_map(p, record)?)?;
        Ok(v)
    }
}
