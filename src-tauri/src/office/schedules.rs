//! Schedules and the checks employees leave themselves for later.

use super::*;

/// Checks an employee may hold open at once; more means they are using the tool as a notebook.
const MAX_OPEN_CHECKS: usize = 20;

impl Office {
    fn with_next_fire(mut s: Schedule) -> Schedule {
        s.next_fire = if s.enabled { schedule::next_fire(&s.cron, now(), &Local) } else { None };
        s
    }

    pub fn schedules(&self, employee_id: &str) -> Result<Vec<Schedule>> {
        Ok(self.db().schedules(employee_id)?.into_iter().map(Self::with_next_fire).collect())
    }

    pub fn save_schedule(&self, d: ScheduleDraft) -> Result<Schedule> {
        schedule::parse(&d.cron).map_err(|e| AppError::invalid(format!("cron: {e}")))?;
        let s = {
            let db = self.db();
            db.employee(&d.employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            let existing = match &d.id {
                Some(id) => Some(db.schedule(id)?.ok_or_else(|| AppError::not_found("schedule not found"))?),
                None => None,
            };
            let s = Schedule {
                id: d.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                employee_id: d.employee_id,
                label: if d.label.trim().is_empty() { d.cron.trim().to_string() } else { d.label.trim().to_string() },
                cron: d.cron.trim().to_string(),
                instruction: d.instruction,
                enabled: d.enabled,
                misfire: Misfire::CatchUp,
                quiet: d.quiet,
                model: d.model,
                effort: d.effort,
                next_fire: None,
            };
            // A new, re-timed or re-enabled schedule starts counting from now: it never fires for
            // slots that passed before the edit.
            let last = match existing {
                Some((old, last)) if old.cron == s.cron && (old.enabled || !s.enabled) => last,
                _ => now(),
            };
            db.save_schedule(&s, last)?;
            s
        };
        self.sink().org_changed();
        Ok(Self::with_next_fire(s))
    }

    pub fn delete_schedule(&self, id: &str) -> Result<()> {
        self.db().delete_schedule(id)?;
        self.sink().org_changed();
        Ok(())
    }

    pub fn cron_preview(&self, cron: &str) -> CronPreview {
        schedule::preview(cron, now(), &Local)
    }

    /// One scheduler pass at `now`. Inserting the run row is the claim, so a slot fires once
    /// however often this runs.
    pub fn tick(&self, now: Ts) -> Result<()> {
        let created = {
            let db = self.db();
            if db.settings()?.map(|s| s.paused).unwrap_or(false) {
                return Ok(());
            }
            let mut created = vec![];
            for (s, last) in db.all_schedules()? {
                if !s.enabled {
                    continue;
                }
                let Ok(cron) = schedule::parse(&s.cron) else { continue };
                match schedule::due_slot(&cron, last, now, &Local) {
                    Due::Nothing => {}
                    Due::Fire(slot) => {
                        db.set_last_slot(&s.id, slot)?;
                        if db.schedule_has_queued_run(&s.id)? || db.employee(&s.employee_id)?.is_none() {
                            continue;
                        }
                        let prompt = if s.instruction.trim().is_empty() { DEFAULT_PROMPT.to_string() } else { s.instruction.clone() };
                        let kind = RunKind::Scheduled { schedule_id: s.id.clone(), slot, label: s.label.clone(), quiet: s.quiet, model: s.model.clone(), effort: s.effort };
                        let run = Run::new(&s.employee_id, kind, prompt, now);
                        if db.insert_run(&run, &run.id)? {
                            created.push(run);
                        }
                    }
                }
            }
            created
        };
        for r in &created {
            self.sink().run_updated(r);
        }
        if !created.is_empty() {
            self.sink().org_changed();
        }
        // Every tick, not only one that queued work: a hold may have ended, or a dark wake turned full.
        self.release_hold(now);
        self.pump();
        Ok(())
    }

    // ------------------------------------------------------------------ checks

    pub fn checks(&self, employee_id: &str) -> Result<Vec<Check>> {
        self.db().checks(employee_id, None)
    }

    /// The person you work for decides a check no longer matters.
    pub fn drop_check(&self, check_id: &str) -> Result<Check> {
        let c = {
            let db = self.db();
            let mut c = db.check(check_id)?.ok_or_else(|| AppError::not_found("check not found"))?;
            if c.state != CheckState::Open {
                return Err(AppError::conflict("that check is already closed"));
            }
            c.state = CheckState::Dropped;
            c.closed_at = Some(now());
            db.save_check(&c)?;
            c
        };
        self.sink().org_changed();
        Ok(c)
    }

    pub fn check_later_tool(&self, run_id: &str, caller: &Employee, args: &Value) -> Result<Value> {
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let (what, how) = (text("what"), text("how"));
        if what.is_empty() || how.is_empty() {
            return Err(AppError::invalid("what and how are both required: what you will find out, and where or how you will look"));
        }
        let days = args.get("inDays").and_then(|v| v.as_f64()).unwrap_or(7.0);
        if !(0.0..=180.0).contains(&days) {
            return Err(AppError::invalid("inDays must be between 0 and 180"));
        }
        let c = {
            let db = self.db();
            if db.checks(&caller.id, Some(CheckState::Open))?.len() >= MAX_OPEN_CHECKS {
                return Err(AppError::conflict(format!("you already hold {MAX_OPEN_CHECKS} open checks: close some with check_done before adding more")));
            }
            let at = now();
            let c = Check {
                id: uuid::Uuid::new_v4().to_string(),
                employee_id: caller.id.clone(),
                what,
                how,
                due_at: at + chrono::Duration::minutes((days * 24.0 * 60.0) as i64),
                state: CheckState::Open,
                created_at: at,
                run_id: Some(run_id.to_string()),
                closed_at: None,
                result: None,
            };
            db.save_check(&c)?;
            c
        };
        self.sink().org_changed();
        Ok(json!({
            "check": c,
            "note": "Omega Star brings this back to you once it is due, even if nothing else starts a run. Do not also leave it as a note in a skill."
        }))
    }

    pub fn my_checks(&self, caller: &Employee) -> Result<Value> {
        let at = now();
        let open: Vec<Value> = self
            .db()
            .checks(&caller.id, Some(CheckState::Open))?
            .into_iter()
            .map(|c| json!({ "id": c.id, "what": c.what, "how": c.how, "due": c.due_at, "overdue": c.due_at <= at }))
            .collect();
        Ok(json!({ "now": at, "open": open }))
    }

    pub fn check_done_tool(&self, caller: &Employee, id: &str, result: &str) -> Result<Value> {
        if result.trim().is_empty() {
            return Err(AppError::invalid("result is required: say what you found"));
        }
        let c = {
            let db = self.db();
            let mut c = db.check(id)?.ok_or_else(|| AppError::not_found("check not found"))?;
            if c.employee_id != caller.id {
                return Err(AppError::invalid("that is not one of your checks"));
            }
            if c.state != CheckState::Open {
                return Err(AppError::conflict("that check is already closed"));
            }
            c.state = CheckState::Done;
            c.closed_at = Some(now());
            c.result = Some(result.trim().to_string());
            db.save_check(&c)?;
            c
        };
        self.sink().org_changed();
        Ok(json!({ "check": c, "note": "Now write what you found into the skill it belongs to, with today's date, so it is not lost." }))
    }
}
