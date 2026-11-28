//! Runs: saving them, finding them, and what they did and cost.

use super::*;

impl Store {
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
        if n == 1 && run.kind.is_conversation() && !run.omega_wrote {
            self.count_star_ask()?;
        }
        Ok(n == 1)
    }

    pub fn save_run(&self, run: &Run) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET state = ?2, data = ?3 WHERE id = ?1",
            params![run.id, run.state.as_str(), to_json(run)?],
        )?;
        Ok(())
    }

    /// Saves a run whose kind moved it to another lane, with that lane.
    pub fn save_run_moved(&self, run: &Run) -> Result<()> {
        self.conn.execute("UPDATE runs SET lane = ?2, data = ?3 WHERE id = ?1", params![run.id, run.kind.lane().as_str(), to_json(run)?])?;
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
        Ok(self.conn.query_row("SELECT data, session FROM runs WHERE id = ?", [id], record).optional()?)
    }

    pub fn runs_in_state(&self, state: RunState) -> Result<Vec<Run>> {
        self.runs_where("WHERE state = ? ORDER BY created_at, rowid", &[&state.as_str()])
    }

    /// The same runs with their sessions.
    pub fn records_in_state(&self, state: RunState) -> Result<Vec<RunRecord>> {
        self.records_where("WHERE state = ? ORDER BY created_at, rowid", &[&state.as_str()])
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

    /// Latest work-lane runs of an employee, newest first.
    pub fn work_runs(&self, employee_id: &str, limit: u32) -> Result<Vec<Run>> {
        self.runs_where(
            "WHERE employee_id = ? AND lane = 'work' ORDER BY created_at DESC, rowid DESC LIMIT ?",
            &[&employee_id, &limit],
        )
    }

    /// An employee's finished runs created since `since`, both lanes, with their sessions, newest first.
    pub fn finished_since(&self, employee_id: &str, since: Ts) -> Result<Vec<RunRecord>> {
        self.records_where(
            concat!("WHERE employee_id = ?1 AND created_at >= ?2 AND state NOT IN ", active!(), "
             ORDER BY created_at DESC, rowid DESC"),
            &[&employee_id, &ms(since)],
        )
    }

    /// When the last turn of a conversation ended: its newest finished run other than `except`.
    pub fn last_turn_ended(&self, session: &str, except: &str) -> Result<Option<Ts>> {
        Ok(self
            .runs_where(
                concat!("WHERE session = ? AND id != ? AND state NOT IN ", active!(), " ORDER BY created_at DESC, rowid DESC LIMIT 1"),
                &[&session, &except],
            )?
            .pop()
            .and_then(|r| r.ended_at))
    }

    /// The finished runs of a session other than `except`, newest first.
    pub fn turns_of_session(&self, session: &str, except: &str) -> Result<Vec<Run>> {
        self.runs_where(concat!("WHERE session = ? AND id != ? AND state NOT IN ", active!(), " ORDER BY created_at DESC, rowid DESC"), &[&session, &except])
    }

    /// The ids of runs that asked a colleague something or delegated work since `since`.
    pub fn parents_since(&self, since: Ts) -> Result<HashSet<String>> {
        let mut st = self.conn.prepare("SELECT DISTINCT parent_run_id FROM runs WHERE parent_run_id IS NOT NULL AND created_at >= ?")?;
        let v = st.query_map([ms(since)], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn active_runs_of(&self, employee_id: &str) -> Result<Vec<Run>> {
        self.runs_where(
            concat!("WHERE employee_id = ? AND state IN ", active!(), " ORDER BY created_at, rowid"),
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

    /// Finished routine runs since `since`, of one employee or with `None` everyone's, one count per schedule,
    /// busiest first. Reviews and due checks are not routines. A run reached someone when it notified you or handed
    /// work to a colleague, and did something when it reached someone, changed a skill or failed.
    pub fn routine_counts(&self, employee_id: Option<&str>, since: Ts) -> Result<Vec<RoutineUsage>> {
        let mut st = self.conn.prepare(concat!(
            "SELECT schedule_id, MAX(employee_id), MAX(json_extract(data, '$.kind.label')), MAX(COALESCE(json_extract(data, '$.kind.quiet'), 0)), COUNT(*),
               SUM(reached), SUM(failed), SUM(reached OR failed OR COALESCE(json_array_length(data, '$.skillChanges'), 0) > 0), TOTAL(json_extract(data, '$.costUsd'))
             FROM (SELECT r.*, r.state IN ('failed', 'lost') AS failed,
                     COALESCE(json_array_length(r.data, '$.needsYou'), 0) > 0 OR EXISTS (SELECT 1 FROM runs c WHERE c.parent_run_id = r.id) AS reached
                   FROM runs r
                   WHERE (?1 IS NULL OR r.employee_id = ?1) AND r.schedule_id IS NOT NULL AND r.schedule_id NOT LIKE ", check_run!(), " AND r.created_at >= ?2
                     AND r.state NOT IN ", active!(), " AND json_valid(r.data)
                     AND COALESCE(json_extract(r.data, '$.kind.review'), 0) = 0)
             GROUP BY schedule_id ORDER BY 5 DESC",
        ))?;
        let rows = st.query_map(params![employee_id, ms(since)], |r| {
            Ok(RoutineUsage {
                schedule_id: r.get(0)?,
                employee_id: r.get(1)?,
                label: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                quiet: r.get::<_, i64>(3)? != 0,
                runs: r.get(4)?,
                reached: r.get(5)?,
                failed: r.get(6)?,
                useful: r.get(7)?,
                cost_usd: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Each employee's finished runs since `since`: how many, the cards they raised, the work they handed to
    /// colleagues since then, how many failed, and what they cost.
    pub fn employee_usage(&self, since: Ts) -> Result<Vec<EmployeeUsage>> {
        let mut st = self.conn.prepare(concat!(
            "SELECT r.employee_id, COUNT(*), SUM(COALESCE(json_array_length(r.data, '$.cards'), 0)), MAX(COALESCE(h.n, 0)),
               SUM(r.state IN ('failed', 'lost')), TOTAL(json_extract(r.data, '$.costUsd'))
             FROM runs r
             LEFT JOIN (SELECT p.employee_id, COUNT(*) AS n FROM runs c JOIN runs p ON p.id = c.parent_run_id
                        WHERE c.created_at >= ?1 GROUP BY p.employee_id) h ON h.employee_id = r.employee_id
             WHERE r.created_at >= ?1 AND r.state NOT IN ", active!(), " AND json_valid(r.data)
             GROUP BY r.employee_id",
        ))?;
        let rows = st.query_map([ms(since)], |r| {
            Ok(EmployeeUsage { employee_id: r.get(0)?, runs: r.get(1)?, decisions: r.get(2)?, handoffs: r.get(3)?, failed: r.get(4)?, cost_usd: r.get(5)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The Analytics page's numbers: everyone hired, finished runs per employee per local day, every card that asked you
    /// something, each employee's latest weekly number, and the tokens finished runs used. A run's night is judged by when it was made, as its day is.
    pub fn team_stats(&self) -> Result<TeamStats> {
        let mut st = self.conn.prepare("SELECT data, departed_at FROM employees")?;
        let staff = readable(
            "employees",
            st.query_map([], |r| {
                let e: Employee = from_row(r, 0)?;
                let departed_at = r.get::<_, Option<i64>>(1)?.and_then(Ts::from_timestamp_millis);
                Ok(StaffSpan { employee_id: e.id, name: e.name, hired_at: e.hired_at, departed_at })
            })?,
        )?;
        let mut st = self.conn.prepare(concat!(
            "SELECT date(created_at / 1000, 'unixepoch', 'localtime') AS d, employee_id, COUNT(*),
               TOTAL(COALESCE(json_extract(data, '$.durationMs'), 0)),
               TOTAL(CASE WHEN CAST(strftime('%H', created_at / 1000, 'unixepoch', 'localtime') AS INTEGER) NOT BETWEEN 7 AND 21
                 THEN COALESCE(json_extract(data, '$.durationMs'), 0) ELSE 0 END)
             FROM runs WHERE state NOT IN ", active!(), " AND json_valid(data)
             GROUP BY d, employee_id ORDER BY d",
        ))?;
        let days = st
            .query_map([], |r| {
                Ok(TeamDay { day: r.get(0)?, employee_id: r.get(1)?, runs: r.get(2)?, work_ms: r.get::<_, f64>(3)? as i64, night_ms: r.get::<_, f64>(4)? as i64 })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut st = self.conn.prepare(
            "SELECT COALESCE(json_extract(r.data, '$.endedAt'), json_extract(r.data, '$.createdAt')) AS at,
               json_extract(c.value, '$.answer.at'), json_extract(c.value, '$.answer.how')
             FROM runs r, json_each(r.data, '$.cards') c
             WHERE json_valid(r.data) AND json_extract(c.value, '$.ask.type') <> 'news'
             ORDER BY at, r.rowid, c.key",
        )?;
        let time = |s: Option<String>| s.and_then(|s| s.parse::<Ts>().ok());
        let asks = st
            .query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter_map(|(at, answered, how)| {
                let yours = matches!(how.as_deref(), Some("tapped" | "typed" | "dismissed" | "rejected"));
                Some(TeamAsk { at: time(at)?, answered_at: if yours { time(answered) } else { None }, open: how.is_none() })
            })
            .collect();
        // SQLite takes the other columns from the row MAX picked, so each is the newest review's.
        let mut st = self.conn.prepare(
            "SELECT employee_id, MAX(created_at), json_extract(data, '$.score.metric'), json_extract(data, '$.score.value'),
               json_extract(data, '$.score.change')
             FROM runs WHERE json_valid(data) AND json_type(data, '$.score') = 'object'
             GROUP BY employee_id",
        )?;
        let text = |r: &Row, i: usize| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
        let numbers = st
            .query_map([], |r| Ok(TeamNumber { employee_id: r.get(0)?, metric: text(r, 2)?, value: text(r, 3)?, change: text(r, 4)? }))?
            .collect::<rusqlite::Result<_>>()?;
        // A run a colleague asked for or delegated has that colleague's run as its parent.
        let mut st = self.conn.prepare(concat!(
            "SELECT date(r.created_at / 1000, 'unixepoch', 'localtime') AS d, r.employee_id, p.employee_id, json_extract(t.value, '$.model') AS m,
               TOTAL(json_extract(t.value, '$.tokens'))
             FROM runs r JOIN json_each(r.data, '$.tokens') t LEFT JOIN runs p ON p.id = r.parent_run_id
             WHERE r.state NOT IN ", active!(), " AND json_valid(r.data)
             GROUP BY d, r.employee_id, p.employee_id, m ORDER BY d",
        ))?;
        let tokens = st
            .query_map([], |r| {
                Ok(TeamTokens { day: r.get(0)?, employee_id: r.get(1)?, handed_over_by: r.get(2)?, model: r.get(3)?, tokens: r.get::<_, f64>(4)? as u64 })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(TeamStats { staff, days, asks, numbers, tokens })
    }

    /// The newest `limit` runs of the employees still here whose instruction, report or a card's question
    /// contains `query`, ignoring case in ASCII letters as SQLite's `lower` does.
    pub fn search_runs(&self, query: &str, limit: u32) -> Result<Vec<Run>> {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return Ok(vec![]);
        }
        self.runs_where(
            "WHERE employee_id IN (SELECT id FROM employees WHERE departed_at IS NULL)
               AND CASE WHEN json_valid(data) THEN
                     instr(lower(COALESCE(json_extract(data, '$.prompt'), '')), ?1) > 0
                     OR instr(lower(COALESCE(json_extract(data, '$.summary'), '')), ?1) > 0
                     OR EXISTS (SELECT 1 FROM json_each(data, '$.cards') c WHERE instr(lower(COALESCE(json_extract(c.value, '$.message'), '')), ?1) > 0)
                   ELSE 0 END
             ORDER BY created_at DESC, rowid DESC LIMIT ?2",
            &[&q, &limit],
        )
    }

    /// Deletes a run that never started: reports held for a manager, once a run of theirs took them in instead.
    pub fn delete_queued_run(&self, id: &str) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM runs WHERE id = ? AND state = 'queued'", [id])? == 1)
    }

    /// When a session's first run was made, and the context its newest finished run that measured one ended with.
    pub fn session_size(&self, session: &str) -> Result<(Option<Ts>, Option<u64>)> {
        let began: Option<i64> = self.conn.query_row("SELECT MIN(created_at) FROM runs WHERE session = ?", [session], |r| r.get(0))?;
        let context: Option<i64> = self
            .conn
            .query_row(
                concat!("SELECT json_extract(data, '$.contextTokens') FROM runs
                 WHERE session = ? AND state NOT IN ", active!(), " AND json_valid(data) AND json_extract(data, '$.contextTokens') IS NOT NULL
                 ORDER BY created_at DESC, rowid DESC LIMIT 1"),
                [session],
                |r| r.get(0),
            )
            .optional()?;
        Ok((began.and_then(Ts::from_timestamp_millis), context.map(|c| c.max(0) as u64)))
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
                concat!("WHERE schedule_id = ? AND created_at < ? AND state NOT IN ", active!(), " ORDER BY created_at DESC, rowid DESC LIMIT 1"),
                &[&schedule_id, &ms(before)],
            )?
            .pop())
    }

    /// The runs with these ids, in no particular order.
    pub fn runs_by_id(&self, ids: &[String]) -> Result<Vec<Run>> {
        let ids = serde_json::to_string(ids)?;
        self.runs_where("WHERE id IN (SELECT value FROM json_each(?))", &[&ids])
    }

    /// A conversation with you is working: one you started, a reply, or a turn of an older build's chat, of
    /// one employee or of anyone. An answer coming back into a conversation is not one: nobody may be watching.
    pub fn ceo_conversation_running(&self, employee_id: Option<&str>) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM runs WHERE state = 'running' AND json_valid(data) AND json_extract(data, '$.kind.type') IN ('chat', 'task', 'reply')
                   AND (?1 IS NULL OR employee_id = ?1) LIMIT 1",
                [employee_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
}
