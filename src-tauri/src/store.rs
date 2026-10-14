//! SQLite persistence. Each domain object is stored as its JSON form in `data`, with the columns
//! the Office queries on derived from it in exactly one place (the save functions below).

use crate::log::log;
use crate::model::*;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;

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
CREATE TABLE IF NOT EXISTS briefs(
  id TEXT PRIMARY KEY, employee_id TEXT NOT NULL, session TEXT NOT NULL, created_at INTEGER NOT NULL, consumed_at INTEGER, data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS briefs_by_session ON briefs(employee_id, session, created_at);
"#;

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

#[derive(Default)]
pub struct RequestFilter {
    pub person_id: Option<String>,
    pub employee_id: Option<String>,
    pub state: Option<RequestState>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(&format!("BEGIN; {SCHEMA} COMMIT;"))?;
        Ok(Store { conn })
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
        self.meta("backup_status")
    }
    pub fn set_backup_status(&self, s: &BackupStatus) -> Result<()> {
        self.set_meta("backup_status", s)
    }
    /// When the usage limit that holds the office resets; `None` when nothing holds it.
    pub fn held_until(&self) -> Result<Option<Ts>> {
        Ok(self.meta::<Option<Ts>>("held_until")?.flatten())
    }
    pub fn set_held_until(&self, t: Option<Ts>) -> Result<()> {
        self.set_meta("held_until", &t)
    }

    /// Writes a consistent copy of the whole database to `path`, which must not exist.
    pub fn snapshot_to(&self, path: &Path) -> Result<()> {
        self.conn.execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
        Ok(())
    }

    // ---- employees ----

    pub fn employees(&self) -> Result<Vec<Employee>> {
        let mut st = self.conn.prepare("SELECT data FROM employees WHERE departed_at IS NULL")?;
        let mut v: Vec<Employee> = readable("employees", st.query_map([], |r| from_row(r, 0))?)?;
        v.sort_by_key(|e| e.hired_at);
        Ok(v)
    }

    pub fn employee(&self, id: &str) -> Result<Option<Employee>> {
        Ok(self
            .conn
            .query_row("SELECT data FROM employees WHERE id = ? AND departed_at IS NULL", [id], |r| from_row(r, 0))
            .optional()?)
    }

    /// Every slug ever used, departed employees included (the column is UNIQUE).
    pub fn slugs(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT slug FROM employees")?;
        let v = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn save_employee(&self, e: &Employee) -> Result<()> {
        self.conn.execute(
            "INSERT INTO employees(id, slug, data) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET data = ?3",
            params![e.id, e.slug, to_json(e)?],
        )?;
        Ok(())
    }

    pub fn depart_employee(&self, id: &str, at: Ts) -> Result<()> {
        self.conn.execute("UPDATE employees SET departed_at = ? WHERE id = ?", params![ms(at), id])?;
        Ok(())
    }

    // ---- people ----

    pub fn people(&self) -> Result<Vec<Person>> {
        let mut st = self.conn.prepare("SELECT data FROM people WHERE removed_at IS NULL")?;
        let mut v: Vec<Person> = readable("people", st.query_map([], |r| from_row(r, 0))?)?;
        v.sort_by_key(|p| p.added_at);
        Ok(v)
    }

    pub fn person(&self, id: &str) -> Result<Option<Person>> {
        Ok(self
            .conn
            .query_row("SELECT data FROM people WHERE id = ? AND removed_at IS NULL", [id], |r| from_row(r, 0))
            .optional()?)
    }

    pub fn save_person(&self, p: &Person) -> Result<()> {
        self.conn.execute(
            "INSERT INTO people(id, data) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET data = ?2",
            params![p.id, to_json(p)?],
        )?;
        Ok(())
    }

    pub fn remove_person(&self, id: &str, at: Ts) -> Result<()> {
        self.conn.execute("UPDATE people SET removed_at = ? WHERE id = ?", params![ms(at), id])?;
        Ok(())
    }

    // ---- schedules ----

    fn schedules_where(&self, clause: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<(Schedule, Ts)>> {
        let mut st = self.conn.prepare(&format!("SELECT data, last_slot FROM schedules {clause}"))?;
        let rows = st.query_map(p, |r| {
            let s: Schedule = from_row(r, 0)?;
            let last: i64 = r.get(1)?;
            Ok((s, Ts::from_timestamp_millis(last).unwrap_or_default()))
        })?;
        let v = readable("schedules", rows)?;
        Ok(v)
    }

    /// Schedules with the slot they last fired (or were anchored) at.
    pub fn all_schedules(&self) -> Result<Vec<(Schedule, Ts)>> {
        self.schedules_where("", &[])
    }

    pub fn schedules(&self, employee_id: &str) -> Result<Vec<Schedule>> {
        Ok(self.schedules_where("WHERE employee_id = ?", &[&employee_id])?.into_iter().map(|(s, _)| s).collect())
    }

    pub fn schedule(&self, id: &str) -> Result<Option<(Schedule, Ts)>> {
        Ok(self.schedules_where("WHERE id = ?", &[&id])?.into_iter().next())
    }

    pub fn save_schedule(&self, s: &Schedule, last_slot: Ts) -> Result<()> {
        self.conn.execute(
            "INSERT INTO schedules(id, employee_id, last_slot, data) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET last_slot = ?3, data = ?4",
            params![s.id, s.employee_id, ms(last_slot), to_json(s)?],
        )?;
        Ok(())
    }

    pub fn set_last_slot(&self, schedule_id: &str, slot: Ts) -> Result<()> {
        self.conn.execute("UPDATE schedules SET last_slot = ? WHERE id = ?", params![ms(slot), schedule_id])?;
        Ok(())
    }

    pub fn delete_schedule(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM schedules WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn delete_schedules_of(&self, employee_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM schedules WHERE employee_id = ?", [employee_id])?;
        Ok(())
    }

    // ---- runs ----

    /// Inserts a queued run. Returns false when an equivalent run already exists (same schedule slot,
    /// or a follow-up for the same request): that unique constraint is the claim.
    pub fn insert_run(&self, run: &Run, session: &str) -> Result<bool> {
        let (schedule_id, slot) = match &run.kind {
            RunKind::Scheduled { schedule_id, slot, .. } => (Some(schedule_id.as_str()), Some(ms(*slot))),
            _ => (None, None),
        };
        let request_id = match &run.kind {
            RunKind::FollowUp { request_id, .. } => Some(request_id.as_str()),
            _ => None,
        };
        let n = self.conn.execute(
            "INSERT INTO runs(id, employee_id, lane, state, schedule_id, slot, parent_run_id, request_id, session, created_at, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT DO NOTHING",
            params![
                run.id,
                run.employee_id,
                run.kind.lane().as_str(),
                run.state.as_str(),
                schedule_id,
                slot,
                run.parent_run_id(),
                request_id,
                session,
                ms(run.created_at),
                to_json(run)?
            ],
        )?;
        Ok(n == 1)
    }

    pub fn save_run(&self, run: &Run) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET state = ?2, data = ?3 WHERE id = ?1",
            params![run.id, run.state.as_str(), to_json(run)?],
        )?;
        Ok(())
    }

    /// Marks every run still running as lost, in SQL, so it reaches rows this build cannot read; JSON
    /// that is not valid is left as it is. `recover` calls it after saving the readable ones, so a row
    /// from another build never holds its session's running slot.
    pub fn lose_unreadable_running(&self, at: Ts, error: &str) -> Result<usize> {
        let at = serde_json::to_value(at)?;
        Ok(self.conn.execute(
            "UPDATE runs SET state = 'lost',
               data = CASE WHEN json_valid(data) THEN json_set(data, '$.state', 'lost', '$.endedAt', ?1, '$.error', ?2) ELSE data END
             WHERE state = 'running'",
            params![at.as_str(), error],
        )?)
    }

    pub fn mark_inited(&self, run_id: &str) -> Result<()> {
        self.conn.execute("UPDATE runs SET inited = 1 WHERE id = ?", [run_id])?;
        Ok(())
    }

    /// Whether Claude Code has already started this session (so the next turn must --resume it).
    pub fn session_started(&self, session: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM runs WHERE session = ? AND inited = 1 LIMIT 1", [session], |_| Ok(()))
            .optional()?
            .is_some())
    }

    pub fn run(&self, id: &str) -> Result<Option<RunRecord>> {
        Ok(self
            .conn
            .query_row("SELECT data, session FROM runs WHERE id = ?", [id], |r| {
                Ok(RunRecord { run: from_row(r, 0)?, session: r.get(1)? })
            })
            .optional()?)
    }

    fn runs_where(&self, clause: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Run>> {
        let mut st = self.conn.prepare(&format!("SELECT data FROM runs {clause}"))?;
        let v = readable("runs", st.query_map(p, |r| from_row(r, 0))?)?;
        Ok(v)
    }

    pub fn runs_in_state(&self, state: RunState) -> Result<Vec<Run>> {
        self.runs_where("WHERE state = ? ORDER BY created_at, rowid", &[&state.as_str()])
    }

    /// The same runs with their sessions.
    pub fn records_in_state(&self, state: RunState) -> Result<Vec<RunRecord>> {
        let mut st = self.conn.prepare("SELECT data, session FROM runs WHERE state = ? ORDER BY created_at, rowid")?;
        let v = readable("runs", st.query_map([state.as_str()], |r| Ok(RunRecord { run: from_row(r, 0)?, session: r.get(1)? }))?)?;
        Ok(v)
    }

    pub fn recent_runs(&self, employee_id: Option<&str>, limit: u32) -> Result<Vec<Run>> {
        match employee_id {
            Some(e) => self.runs_where("WHERE employee_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?", &[&e, &limit]),
            None => self.runs_where(
                "WHERE employee_id IN (SELECT id FROM employees WHERE departed_at IS NULL)
                 ORDER BY created_at DESC, rowid DESC LIMIT ?",
                &[&limit],
            ),
        }
    }

    pub fn children(&self, parent_run_id: &str) -> Result<Vec<Run>> {
        self.runs_where("WHERE parent_run_id = ? ORDER BY created_at, rowid", &[&parent_run_id])
    }

    pub fn chat_runs(&self, employee_id: &str, session: &str) -> Result<Vec<Run>> {
        self.runs_where(
            "WHERE employee_id = ? AND lane = 'chat' AND session = ? ORDER BY created_at, rowid",
            &[&employee_id, &session],
        )
    }

    /// Latest work-lane runs of an employee, newest first.
    pub fn work_runs(&self, employee_id: &str, limit: u32) -> Result<Vec<Run>> {
        self.runs_where(
            "WHERE employee_id = ? AND lane = 'work' ORDER BY created_at DESC, rowid DESC LIMIT ?",
            &[&employee_id, &limit],
        )
    }

    pub fn active_runs_of(&self, employee_id: &str) -> Result<Vec<Run>> {
        self.runs_where(
            "WHERE employee_id = ? AND state IN ('queued', 'running') ORDER BY created_at, rowid",
            &[&employee_id],
        )
    }

    /// Raw SQL on purpose: a queued row this build cannot read is a newer build's run, which that build will start.
    pub fn schedule_has_queued_run(&self, schedule_id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM runs WHERE schedule_id = ? AND state = 'queued' LIMIT 1",
                [schedule_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The runs that may reach the Inbox on their own, not archived, newest first. `roots` are the
    /// employees who report to you, `flagged` everyone. Reply runs never appear: they belong to the thread
    /// of the run they answer (`reply_runs`).
    pub fn finished_root_runs(&self, roots: &[String], flagged: &[String]) -> Result<Vec<Run>> {
        self.reports(roots, flagged, None, "json_extract(data, '$.archivedAt') IS NULL", "created_at DESC, rowid DESC")
    }

    /// The same runs once archived: the most recently archived `limit`, newest first.
    pub fn archived_root_runs(&self, employee_ids: &[String], limit: u32) -> Result<Vec<Run>> {
        self.reports(employee_ids, employee_ids, Some(limit), "json_extract(data, '$.archivedAt') IS NOT NULL", "json_extract(data, '$.archivedAt') DESC, rowid DESC")
    }

    /// A fast pre-filter for `threads::reaches_you` with `quiet_chain` false and `top` meaning one of
    /// `roots`, so the Inbox does not load every quiet run ever made. The Office still applies
    /// `reaches_you` to what comes back, and `the_inbox_query_matches_reaches_you` keeps the two equal.
    /// `json_valid` goes first, and again in the OR branch SQLite may run on its own: a JSON function on a
    /// row that is not JSON fails the whole query.
    fn reports(&self, roots: &[String], flagged: &[String], limit: Option<u32>, archived: &str, order: &str) -> Result<Vec<Run>> {
        let (roots, flagged) = (serde_json::to_string(roots)?, serde_json::to_string(flagged)?);
        self.runs_where(
            &format!(
                "WHERE json_valid(data)
                 AND lane = 'work'
                 AND json_extract(data, '$.kind.type') IS NOT 'reply'
                 AND json_extract(data, '$.kind.type') IS NOT 'replyCheck'
                 AND json_extract(data, '$.kind.type') IS NOT 'task'
                 AND ((employee_id IN (SELECT value FROM json_each(?)) AND parent_run_id IS NULL)
                      OR (employee_id IN (SELECT value FROM json_each(?))
                          AND json_valid(data)
                          AND (COALESCE(json_array_length(data, '$.needsYou'), 0) > 0
                               OR (COALESCE(json_array_length(data, '$.denials'), 0) > 0 AND json_extract(data, '$.kind.type') IS NOT 'tidy')
                               OR (parent_run_id IS NOT NULL AND state IN ('failed', 'lost')))))
                 AND state NOT IN ('queued', 'running')
                 AND NOT (json_extract(data, '$.kind.type') = 'nudge' AND state = 'cancelled')
                 AND NOT ((COALESCE(json_extract(data, '$.kind.quiet'), 0) = 1 OR json_extract(data, '$.kind.type') IN ('tidy', 'nudge'))
                          AND state = 'succeeded'
                          AND COALESCE(json_array_length(data, '$.needsYou'), 0) = 0
                          AND (COALESCE(json_array_length(data, '$.denials'), 0) = 0 OR json_extract(data, '$.kind.type') = 'tidy'))
                 AND {archived}
                 ORDER BY {order} LIMIT ?"
            ),
            &[&roots, &flagged, &limit.map_or(-1, i64::from)],
        )
    }

    /// Colleague asks still in flight: queued or running, or answered while the asker's own run is
    /// still going and has not collected the answer yet. Oldest first.
    pub fn open_asks(&self) -> Result<Vec<Run>> {
        self.runs_where(
            "WHERE json_valid(data)
               AND json_extract(data, '$.kind.type') = 'asked'
               AND (state IN ('queued', 'running')
                    OR (json_valid(data) AND json_extract(data, '$.deliveredAt') IS NULL
                        AND parent_run_id IN (SELECT id FROM runs WHERE state IN ('queued', 'running'))))
             ORDER BY created_at, rowid",
            &[],
        )
    }

    // ---- human requests ----

    pub fn save_request(&self, r: &HumanRequest) -> Result<()> {
        self.conn.execute(
            "INSERT INTO requests(id, person_id, employee_id, state, created_at, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET state = ?4, data = ?6",
            params![r.id, r.person_id, r.employee_id, r.state.as_str(), ms(r.created_at), to_json(r)?],
        )?;
        Ok(())
    }

    pub fn request(&self, id: &str) -> Result<Option<HumanRequest>> {
        Ok(self
            .conn
            .query_row("SELECT data FROM requests WHERE id = ?", [id], |r| from_row(r, 0))
            .optional()?)
    }

    /// Open requests Omega Star is watching for a reply, oldest first.
    pub fn watched_requests(&self) -> Result<Vec<HumanRequest>> {
        self.requests_where("WHERE state = 'open' AND json_valid(data) AND json_extract(data, '$.delivery.watch.phase.type') = 'watching'", &[])
    }

    /// The requests a run made, oldest first.
    pub fn requests_of_run(&self, run_id: &str) -> Result<Vec<HumanRequest>> {
        self.requests_where("WHERE json_valid(data) AND json_extract(data, '$.runId') = ?", &[&run_id])
    }

    fn requests_where(&self, clause: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<HumanRequest>> {
        let mut st = self.conn.prepare(&format!("SELECT data FROM requests {clause} ORDER BY created_at, rowid"))?;
        let v = readable("requests", st.query_map(p, |r| from_row(r, 0))?)?;
        Ok(v)
    }

    /// The queued or running run of `kind` (its `type` tag) about a request.
    pub fn active_run_for(&self, request_id: &str, kind: &str) -> Result<Option<Run>> {
        Ok(self
            .runs_where(
                "WHERE state IN ('queued', 'running') AND json_valid(data) AND json_extract(data, '$.kind.type') = ? AND json_extract(data, '$.kind.requestId') = ? LIMIT 1",
                &[&kind, &request_id],
            )?
            .into_iter()
            .next())
    }

    /// Whether a run of `kind` about a request was ever made, in any state.
    pub fn has_run_for(&self, request_id: &str, kind: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM runs WHERE json_valid(data) AND json_extract(data, '$.kind.type') = ? AND json_extract(data, '$.kind.requestId') = ? LIMIT 1",
                [kind, request_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn requests(&self, f: &RequestFilter) -> Result<Vec<HumanRequest>> {
        let mut st = self.conn.prepare(
            "SELECT data FROM requests WHERE (?1 IS NULL OR person_id = ?1) AND (?2 IS NULL OR employee_id = ?2)
             AND (?3 IS NULL OR state = ?3) ORDER BY created_at DESC, rowid DESC",
        )?;
        let v = readable("requests", st.query_map(params![f.person_id, f.employee_id, f.state.map(|s| s.as_str())], |r| from_row(r, 0))?)?;
        Ok(v)
    }

    // ---- hire proposals ----

    pub fn save_proposal(&self, p: &HireProposal) -> Result<()> {
        self.conn.execute(
            "INSERT INTO proposals(id, employee_id, state, created_at, data) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET state = ?3, data = ?5",
            params![p.id, p.employee_id, p.state.as_str(), ms(p.created_at), to_json(p)?],
        )?;
        Ok(())
    }

    pub fn proposal(&self, id: &str) -> Result<Option<HireProposal>> {
        Ok(self
            .conn
            .query_row("SELECT data FROM proposals WHERE id = ?", [id], |r| from_row(r, 0))
            .optional()?)
    }

    /// Newest first.
    pub fn proposals(&self, employee_id: Option<&str>, state: Option<ProposalState>) -> Result<Vec<HireProposal>> {
        let mut st = self.conn.prepare(
            "SELECT data FROM proposals WHERE (?1 IS NULL OR employee_id = ?1) AND (?2 IS NULL OR state = ?2)
             ORDER BY created_at DESC, rowid DESC",
        )?;
        let v = readable("proposals", st.query_map(params![employee_id, state.map(|s| s.as_str())], |r| from_row(r, 0))?)?;
        Ok(v)
    }

    // ---- tidy ----

    /// An employee's Tidy runs, newest first.
    pub fn tidy_runs(&self, employee_id: &str) -> Result<Vec<Run>> {
        self.runs_where("WHERE employee_id = ? AND json_valid(data) AND json_extract(data, '$.kind.type') = 'tidy' ORDER BY created_at DESC, rowid DESC", &[&employee_id])
    }

    /// What you told this employee since `since`, in chat, tasks and Inbox replies: (when, your words),
    /// oldest first, at most `limit` of the newest.
    pub fn taught_since(&self, employee_id: &str, since: Option<Ts>, limit: usize) -> Result<Vec<(Ts, String)>> {
        let mut st = self.conn.prepare(
            "SELECT created_at, json_extract(data, '$.prompt') FROM runs
             WHERE employee_id = ?1 AND created_at > ?2 AND json_valid(data) AND json_extract(data, '$.kind.type') IN ('chat', 'reply', 'task')
               AND COALESCE(json_extract(data, '$.omegaWrote'), 0) = 0
             ORDER BY created_at DESC, rowid DESC LIMIT ?3",
        )?;
        let rows = st.query_map(params![employee_id, since.map(ms).unwrap_or(0), limit as i64], |r| {
            Ok((Ts::from_timestamp_millis(r.get::<_, i64>(0)?).unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default()))
        })?;
        let mut v: Vec<(Ts, String)> = rows.collect::<rusqlite::Result<_>>()?;
        v.retain(|(_, t)| !t.trim().is_empty() && !t.contains("approved what was blocked"));
        v.reverse();
        Ok(v)
    }

    // ---- threads and briefs ----

    /// Every Reply run, of one employee or of everyone, oldest first. Threads are built from them.
    pub fn reply_runs(&self, employee_id: Option<&str>) -> Result<Vec<Run>> {
        self.runs_where(
            "WHERE json_valid(data) AND (?1 IS NULL OR employee_id = ?1) AND json_extract(data, '$.kind.type') = 'reply' ORDER BY created_at, rowid",
            &[&employee_id],
        )
    }

    /// An employee's tasks you have not archived, oldest first.
    pub fn open_tasks(&self, employee_id: &str) -> Result<Vec<Run>> {
        self.runs_where(
            "WHERE employee_id = ? AND json_valid(data) AND json_extract(data, '$.kind.type') = 'task' AND json_extract(data, '$.archivedAt') IS NULL
             ORDER BY created_at, rowid",
            &[&employee_id],
        )
    }

    /// Every run that continues a conversation (replies, colleagues' asks and answers, delegated
    /// work), oldest first: what `threads::file` follows to put a hand-off in one Inbox row. An answer
    /// that came back as a chat turn belongs to that chat, never to a row.
    pub fn conversation_runs(&self) -> Result<Vec<Run>> {
        self.runs_where(
            "WHERE json_valid(data) AND json_extract(data, '$.kind.type') IN ('reply', 'answered', 'asked', 'delegated')
               AND NOT (json_extract(data, '$.kind.type') = 'answered' AND COALESCE(json_extract(data, '$.kind.chat'), 0) = 1)
             ORDER BY created_at, rowid",
            &[],
        )
    }

    /// When each successful run of a schedule was created, oldest first.
    pub fn successes_of_schedule(&self, schedule_id: &str) -> Result<Vec<Ts>> {
        let mut st = self.conn.prepare("SELECT created_at FROM runs WHERE schedule_id = ? AND state = 'succeeded' ORDER BY created_at")?;
        let v = st.query_map([schedule_id], |r| Ok(Ts::from_timestamp_millis(r.get::<_, i64>(0)?).unwrap_or_default()))?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    /// A schedule's runs that failed or were lost, oldest first.
    pub fn failures_of_schedule(&self, schedule_id: &str) -> Result<Vec<Run>> {
        self.runs_where("WHERE schedule_id = ? AND state IN ('failed', 'lost') ORDER BY created_at, rowid", &[&schedule_id])
    }

    /// The last run of a schedule that finished before `before`.
    pub fn previous_finished(&self, schedule_id: &str, before: Ts) -> Result<Option<Run>> {
        Ok(self
            .runs_where(
                "WHERE schedule_id = ? AND created_at < ? AND state NOT IN ('queued', 'running') ORDER BY created_at DESC, rowid DESC LIMIT 1",
                &[&schedule_id, &ms(before)],
            )?
            .pop())
    }

    /// The runs with these ids, in no particular order.
    pub fn runs_by_id(&self, ids: &[String]) -> Result<Vec<Run>> {
        let ids = serde_json::to_string(ids)?;
        self.runs_where("WHERE id IN (SELECT value FROM json_each(?))", &[&ids])
    }

    pub fn insert_brief(&self, b: &Brief, session: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO briefs(id, employee_id, session, created_at, consumed_at, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![b.id, b.employee_id, session, ms(b.created_at), b.consumed_at.map(ms), to_json(b)?],
        )?;
        Ok(())
    }

    /// The briefs written for one chat session, oldest first; `unconsumed_only` keeps those no turn has read.
    pub fn briefs(&self, employee_id: &str, session: &str, unconsumed_only: bool) -> Result<Vec<Brief>> {
        let mut st = self.conn.prepare(
            "SELECT data FROM briefs WHERE employee_id = ?1 AND session = ?2 AND (?3 = 0 OR consumed_at IS NULL) ORDER BY created_at, rowid",
        )?;
        let v = readable("briefs", st.query_map(params![employee_id, session, unconsumed_only as i64], |r| from_row(r, 0))?)?;
        Ok(v)
    }

    /// Drops the briefs about a thread that no chat turn has read yet: the thread came back to the Inbox.
    pub fn drop_unread_briefs(&self, root_run_id: &str) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM briefs WHERE consumed_at IS NULL AND json_valid(data) AND json_extract(data, '$.rootRunId') = ?", [root_run_id])?)
    }

    /// Marks briefs as read by a chat turn launched at `at`.
    pub fn consume_briefs(&self, ids: &[String], at: Ts) -> Result<()> {
        for id in ids {
            let found = self.conn.query_row("SELECT data FROM briefs WHERE id = ?", [id], |r| from_row::<Brief>(r, 0)).optional()?;
            if let Some(mut b) = found {
                b.consumed_at = Some(at);
                self.conn.execute("UPDATE briefs SET consumed_at = ?2, data = ?3 WHERE id = ?1", params![id, ms(at), to_json(&b)?])?;
            }
        }
        Ok(())
    }

    // ---- checks ----

    pub fn save_check(&self, c: &Check) -> Result<()> {
        self.conn.execute(
            "INSERT INTO checks(id, employee_id, state, due_at, created_at, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET state = ?3, due_at = ?4, data = ?6",
            params![c.id, c.employee_id, c.state.as_str(), ms(c.due_at), ms(c.created_at), to_json(c)?],
        )?;
        Ok(())
    }

    pub fn check(&self, id: &str) -> Result<Option<Check>> {
        Ok(self.conn.query_row("SELECT data FROM checks WHERE id = ?", [id], |r| from_row(r, 0)).optional()?)
    }

    /// Soonest due first; open ones by date, finished ones newest first.
    pub fn checks(&self, employee_id: &str, state: Option<CheckState>) -> Result<Vec<Check>> {
        let mut st = self.conn.prepare(
            "SELECT data FROM checks WHERE employee_id = ?1 AND (?2 IS NULL OR state = ?2)
             ORDER BY (state = 'open') DESC, CASE WHEN state = 'open' THEN due_at END ASC, created_at DESC",
        )?;
        let v = readable("checks", st.query_map(params![employee_id, state.map(|s| s.as_str())], |r| from_row(r, 0))?)?;
        Ok(v)
    }

    // ---- skill revisions ----

    pub fn insert_revision(&self, employee_id: &str, r: &SkillRevision) -> Result<()> {
        self.conn.execute(
            "INSERT INTO skill_revisions(id, employee_id, run_id, at, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![r.id, employee_id, r.run_id, ms(r.at), to_json(r)?],
        )?;
        Ok(())
    }

    pub fn revisions(&self, employee_id: &str) -> Result<Vec<SkillRevision>> {
        let mut st = self
            .conn
            .prepare("SELECT data FROM skill_revisions WHERE employee_id = ? ORDER BY at DESC, rowid DESC")?;
        let v = readable("skill_revisions", st.query_map([employee_id], |r| from_row(r, 0))?)?;
        Ok(v)
    }

    /// (skill, file, when) of every edit since `since`, without the file texts.
    pub fn edits_since(&self, employee_id: &str, since: Ts) -> Result<Vec<(String, String, Ts)>> {
        let mut st = self.conn.prepare(
            "SELECT json_extract(data, '$.skill'), json_extract(data, '$.file'), at FROM skill_revisions WHERE employee_id = ? AND at >= ?",
        )?;
        let rows = st.query_map(params![employee_id, ms(since)], |r| {
            let at: i64 = r.get(2)?;
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, Ts::from_timestamp_millis(at).unwrap_or_default()))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A revision with the employee it belongs to.
    pub fn revision(&self, id: &str) -> Result<Option<(String, SkillRevision)>> {
        Ok(self
            .conn
            .query_row("SELECT employee_id, data FROM skill_revisions WHERE id = ?", [id], |r| {
                Ok((r.get(0)?, from_row(r, 1)?))
            })
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::threads;
    use chrono::Utc;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omegastar-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An employee reporting to `manager_id`, or to you.
    fn employee(name: &str, manager_id: Option<&str>) -> Employee {
        let id = uuid::Uuid::new_v4().to_string();
        Employee {
            id: id.clone(),
            slug: name.to_lowercase(),
            name: name.into(),
            title: "Tester".into(),
            description: String::new(),
            objective: String::new(),
            manager_id: manager_id.map(String::from),
            skills: SkillGrant { builtins: false, keys: vec![] },
            clearance: Clearance::Supervised,
            allow_rules: vec![],
            model: None,
            effort: None,
            browser: false,
            auto_tidy: false,
            workspaces: vec![format!("/office/employees/{id}")],
            hue: 0,
            chat_session: format!("chat-{name}"),
            hired_at: Utc::now(),
        }
    }

    /// A run in `state`, ended now if it is finished.
    fn run(e: &Employee, kind: RunKind, state: RunState) -> Run {
        let mut r = Run::new(&e.id, kind, "go".into(), Utc::now());
        r.state = state;
        if state.is_finished() {
            r.ended_at = Some(r.created_at);
        }
        r
    }

    /// Inserts a work-lane row of `e` as it is, readable or not.
    fn raw(db: &Store, e: &Employee, id: &str, state: &str, session: &str, data: &str) {
        db.conn
            .execute(
                "INSERT INTO runs(id, employee_id, lane, state, session, created_at, data) VALUES (?1, ?2, 'work', ?3, ?4, ?5, ?6)",
                params![id, e.id, state, session, ms(Utc::now()), data],
            )
            .unwrap();
    }

    /// A run's JSON with a kind this build does not know.
    fn future_kind(e: &Employee, state: RunState) -> (String, String) {
        let mut v = serde_json::to_value(run(e, RunKind::Manual, state)).unwrap();
        v["kind"] = serde_json::json!({ "type": "someFutureKind", "detail": "from a newer build" });
        (v["id"].as_str().unwrap().to_string(), v.to_string())
    }

    fn ids(runs: &[Run]) -> BTreeSet<String> {
        runs.iter().map(|r| r.id.clone()).collect()
    }

    #[test]
    fn an_unreadable_row_is_left_out() {
        let dir = temp_dir();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let ada = employee("Ada", None);
        db.save_employee(&ada).unwrap();
        let queued = run(&ada, RunKind::Manual, RunState::Queued);
        let done = run(&ada, RunKind::Manual, RunState::Succeeded);
        assert!(db.insert_run(&queued, &queued.id).unwrap());
        assert!(db.insert_run(&done, &done.id).unwrap());
        for state in [RunState::Queued, RunState::Succeeded] {
            let (id, data) = future_kind(&ada, state);
            raw(&db, &ada, &id, state.as_str(), &id, &data);
            let id = uuid::Uuid::new_v4().to_string();
            raw(&db, &ada, &id, state.as_str(), &id, "not json");
        }

        let queued_id = queued.id.clone();
        let both = ids(&[queued.clone(), done.clone()]);
        assert_eq!(ids(&db.recent_runs(None, 100).unwrap()), both);
        let records: Vec<Run> = db.records_in_state(RunState::Queued).unwrap().into_iter().map(|r| r.run).collect();
        assert_eq!(ids(&records), ids(&[queued.clone()]));
        assert_eq!(ids(&db.runs_in_state(RunState::Queued).unwrap()), ids(&[queued]));
        assert_eq!(ids(&db.runs_in_state(RunState::Succeeded).unwrap()), ids(&[done.clone()]));
        assert_eq!(ids(&db.finished_root_runs(&[ada.id.clone()], &[ada.id.clone()]).unwrap()), ids(&[done]));
        // The Inbox's other queries read the JSON in SQL too.
        assert!(db.reply_runs(None).unwrap().is_empty());
        assert!(db.open_tasks(&ada.id).unwrap().is_empty());
        assert!(db.conversation_runs().unwrap().is_empty());
        assert!(db.open_asks().unwrap().is_empty());
        // So do the reply watch and Tidy.
        assert!(!db.has_run_for("req-1", "nudge").unwrap());
        assert!(db.active_run_for("req-1", "nudge").unwrap().is_none());
        assert!(db.tidy_runs(&ada.id).unwrap().is_empty());
        assert!(db.taught_since(&ada.id, None, 10).unwrap().is_empty());
        let now = ms(Utc::now());
        db.conn
            .execute(
                "INSERT INTO requests(id, person_id, employee_id, state, created_at, data) VALUES ('req-1', 'p1', ?1, 'open', ?2, 'not json')",
                params![ada.id, now],
            )
            .unwrap();
        db.conn
            .execute("INSERT INTO briefs(id, employee_id, session, created_at, data) VALUES ('b1', ?1, 's1', ?2, 'not json')", params![ada.id, now])
            .unwrap();
        assert!(db.watched_requests().unwrap().is_empty());
        assert!(db.requests_of_run(&queued_id).unwrap().is_empty());
        assert_eq!(db.drop_unread_briefs("root-1").unwrap(), 0);

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_running_rows_are_lost() {
        let dir = temp_dir();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let ada = employee("Ada", None);
        db.save_employee(&ada).unwrap();
        let (future, data) = future_kind(&ada, RunState::Running);
        raw(&db, &ada, &future, "running", "s1", &data);
        raw(&db, &ada, "corrupt", "running", "s2", "not json");

        let at = Utc::now();
        assert_eq!(db.lose_unreadable_running(at, "Stopped.").unwrap(), 2);
        let state = |id: &str| -> String { db.conn.query_row("SELECT state FROM runs WHERE id = ?", [id], |r| r.get(0)).unwrap() };
        assert_eq!(state(&future), "lost");
        assert_eq!(state("corrupt"), "lost");
        let (json_state, ended, error): (String, String, String) = db
            .conn
            .query_row(
                "SELECT json_extract(data, '$.state'), json_extract(data, '$.endedAt'), json_extract(data, '$.error') FROM runs WHERE id = ?",
                [&future],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(json_state, "lost");
        assert_eq!(ended.parse::<Ts>().unwrap(), at);
        assert_eq!(error, "Stopped.");
        let corrupt: String = db.conn.query_row("SELECT data FROM runs WHERE id = 'corrupt'", [], |r| r.get(0)).unwrap();
        assert_eq!(corrupt, "not json");

        for session in ["s1", "s2"] {
            let next = run(&ada, RunKind::Manual, RunState::Running);
            assert!(db.insert_run(&next, session).unwrap(), "a new run takes {session}");
        }

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_office_still_opens() {
        let dir = temp_dir();
        let path = dir.join("office.db");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            r#"
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE employees(id TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, departed_at INTEGER, data TEXT NOT NULL);
CREATE TABLE people(id TEXT PRIMARY KEY, removed_at INTEGER, data TEXT NOT NULL);
CREATE TABLE schedules(id TEXT PRIMARY KEY, employee_id TEXT NOT NULL, last_slot INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE runs(
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
CREATE UNIQUE INDEX runs_one_running_per_lane ON runs(employee_id, lane) WHERE state = 'running';
INSERT INTO meta(key, value) VALUES ('settings', '{"maxConcurrentRuns":2,"claudePath":null,"dataDir":"/office","paused":false}');
INSERT INTO employees(id, slug, data) VALUES ('e1', 'ada', '{"id":"e1","slug":"ada","name":"Ada","title":"Analyst","description":"","objective":"Keep the numbers right.","managerId":null,"skills":{"builtins":true,"keys":[]},"clearance":"supervised","allowRules":[],"model":null,"effort":null,"workspaces":["/office/employees/e1/workspace"],"hue":200,"chatSession":"chat-e1","hiredAt":"2026-09-20T09:00:00Z"}');
INSERT INTO runs(id, employee_id, lane, state, session, inited, created_at, data) VALUES ('r1', 'e1', 'work', 'succeeded', 'r1', 1, 1759050000000, '{"id":"r1","employeeId":"e1","kind":{"type":"manual"},"prompt":"Check the numbers.","state":"succeeded","createdAt":"2026-09-28T09:00:00Z","startedAt":"2026-09-28T09:00:01Z","endedAt":"2026-09-28T09:05:00Z","turns":4,"durationMs":299000,"costUsd":0.12,"summary":"All good.","error":null,"needsYou":[]}');
"#,
        )
        .unwrap();
        drop(old);

        let db = Store::open(&path).unwrap();
        let mut st = db.conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap();
        let tables: BTreeSet<String> = st.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        drop(st);
        for t in ["checks", "proposals", "skill_revisions", "requests"] {
            assert!(tables.contains(t), "{t} created");
        }
        let lane_index: Option<String> =
            db.conn.query_row("SELECT name FROM sqlite_master WHERE name = 'runs_one_running_per_lane'", [], |r| r.get(0)).optional().unwrap();
        assert_eq!(lane_index, None);

        let r = db.run("r1").unwrap().unwrap().run;
        assert!(r.allow_once.is_empty() && r.denials.is_empty() && r.skill_changes.is_empty() && r.attachments.is_empty());
        assert_eq!(r.proposed, None);
        let e = db.employee("e1").unwrap().unwrap();
        assert!(e.browser);
        assert!(!e.auto_tidy);
        let s = db.settings().unwrap().unwrap();
        assert_eq!(s.max_concurrent_runs, 2);
        assert_eq!(s.backup, BackupSettings::default());
        assert_eq!(s.watch, WatchSettings::default());

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn taught_since_keeps_only_your_own_words() {
        let dir = temp_dir();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let ada = employee("Ada", None);
        db.save_employee(&ada).unwrap();
        let report = run(&ada, RunKind::Manual, RunState::Succeeded);
        db.insert_run(&report, &report.id).unwrap();
        let reply = |prompt: &str, omega_wrote: bool| {
            let mut r = Run::new(&ada.id, RunKind::Reply { to_run_id: report.id.clone(), label: "Manual run".into(), task: false }, prompt.into(), Utc::now());
            r.omega_wrote = omega_wrote;
            db.insert_run(&r, &r.id).unwrap();
        };
        // An approval from the app, and a reply of yours wrapped because the report's session never started.
        reply("Pat allowed the build for this retry.", true);
        reply("Your report (Manual run):\n\nIt failed.\n\nPat's reply:\n\nTry the other folder.", false);
        let came = RunKind::Answered { asked_run_id: "r1".into(), from: "e0".into(), task: false, chat: true };
        let mut answer = Run::new(&ada.id, came, "Bo answered your question.".into(), Utc::now());
        answer.omega_wrote = true;
        db.insert_run(&answer, &ada.chat_session).unwrap();

        let taught: Vec<String> = db.taught_since(&ada.id, None, 10).unwrap().into_iter().map(|(_, t)| t).collect();
        assert_eq!(taught.len(), 1, "{taught:?}");
        assert!(taught[0].ends_with("Try the other folder."));

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One of each kind of run. `i` makes its ids unique, since `insert_run` skips a second run of the
    /// same schedule slot or follow-up.
    fn every_kind(i: usize) -> Vec<RunKind> {
        let slot = Utc::now();
        let person = || "p1".to_string();
        vec![
            RunKind::Manual,
            RunKind::Chat,
            RunKind::Scheduled { schedule_id: format!("loud-{i}"), slot, label: "Sweep".into(), quiet: false, model: None, effort: None },
            RunKind::Scheduled { schedule_id: format!("quiet-{i}"), slot, label: "Sweep".into(), quiet: true, model: None, effort: None },
            RunKind::Delegated { parent_run_id: format!("parent-{i}") },
            RunKind::Asked { parent_run_id: format!("parent-{i}"), asked_by: "e0".into(), task: false },
            RunKind::Answered { asked_run_id: format!("asked-{i}"), from: "e0".into(), task: false, chat: false },
            RunKind::Answered { asked_run_id: format!("asked-chat-{i}"), from: "e0".into(), task: true, chat: true },
            RunKind::FollowUp { request_id: format!("request-{i}"), person_id: person() },
            RunKind::Organize { imported: vec![], settled: false },
            RunKind::Reply { to_run_id: format!("root-{i}"), label: "Manual run".into(), task: false },
            RunKind::Task { label: "Draft it".into(), from: None, context: String::new() },
            RunKind::HireDecided { proposal_id: format!("proposal-{i}"), approved: true },
            RunKind::Tidy { reason: TidyReason::Auto, signature: vec![], plan: None },
            RunKind::ReplyCheck { request_id: format!("request-{i}"), person_id: person() },
            RunKind::Nudge { request_id: format!("request-{i}"), person_id: person() },
            RunKind::Unanswered { request_id: format!("request-{i}"), person_id: person() },
        ]
    }

    #[test]
    fn the_inbox_query_matches_reaches_you() {
        let dir = temp_dir();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let ada = employee("Ada", None);
        let bo = employee("Bo", Some(&ada.id));
        db.save_employee(&ada).unwrap();
        db.save_employee(&bo).unwrap();
        let states = [RunState::Queued, RunState::Running, RunState::Succeeded, RunState::Failed, RunState::Cancelled, RunState::Lost];
        let denial = Denial { tool: "Bash".into(), brief: "rm -rf build".into(), suggested_rule: "Bash(rm:*)".into() };

        let mut all = vec![];
        let mut i = 0;
        for e in [&ada, &bo] {
            for state in states {
                for flag in [false, true] {
                    for blocked in [false, true] {
                        i += 1;
                        for kind in every_kind(i) {
                            let mut r = run(e, kind, state);
                            if flag {
                                r.needs_you = vec!["Look at this.".into()];
                            }
                            if blocked {
                                r.denials = vec![denial.clone()];
                            }
                            assert!(db.insert_run(&r, &r.id).unwrap(), "inserted {:?}", r.kind);
                            all.push(r);
                        }
                    }
                }
            }
        }

        let got = ids(&db.finished_root_runs(&[ada.id.clone()], &[ada.id.clone(), bo.id.clone()]).unwrap());
        let want: BTreeSet<String> =
            all.iter().filter(|r| threads::reaches_you(r, false, r.employee_id == ada.id)).map(|r| r.id.clone()).collect();
        let differ: Vec<String> = all
            .iter()
            .filter(|r| got.contains(&r.id) != want.contains(&r.id))
            .map(|r| format!("{:?} {:?} needs_you {} denials {} top {}", r.kind, r.state, r.needs_you.len(), r.denials.len(), r.employee_id == ada.id))
            .collect();
        assert!(differ.is_empty(), "the query and reaches_you differ on {differ:#?}");
        assert!(!want.is_empty());

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
