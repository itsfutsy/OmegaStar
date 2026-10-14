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
            // Re-timing the weekly review keeps it the review.
            let review = existing.as_ref().map(|(o, _)| o.review).unwrap_or(false);
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
                review,
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

    /// Gives every employee the weekly review, once per office: a review someone deletes never returns.
    pub(super) fn seed_reviews(&self) -> Result<()> {
        let db = self.db();
        if db.reviews_seeded()? {
            return Ok(());
        }
        let ceo = ceo_of(&db)?;
        db.tx(|| {
            for e in db.employees()? {
                self.seed_review(&db, &e, &ceo)?;
            }
            db.set_reviews_seeded()
        })
    }

    /// Adds `e`'s weekly review, Monday 7 am. It counts from now, so it never fires for a slot already past.
    pub(super) fn seed_review(&self, db: &Store, e: &Employee, ceo: &str) -> Result<()> {
        let s = Schedule {
            id: uuid::Uuid::new_v4().to_string(),
            employee_id: e.id.clone(),
            label: "Weekly review".into(),
            cron: "0 7 * * 1".into(),
            instruction: review_instruction(ceo),
            enabled: true,
            misfire: Misfire::CatchUp,
            quiet: false,
            model: None,
            effort: None,
            review: true,
            next_fire: None,
        };
        db.save_schedule(&s, now())
    }

    pub fn cron_preview(&self, cron: &str) -> CronPreview {
        schedule::preview(cron, now(), &Local)
    }

    /// One scheduler pass at `now`: due schedule slots, then due checks. Inserting the run row is the
    /// claim, so a slot or a check's due date fires once however often this runs.
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
                        let kind = RunKind::Scheduled { schedule_id: s.id.clone(), slot, label: s.label.clone(), quiet: s.quiet, model: s.model.clone(), effort: s.effort, review: s.review };
                        let run = Run::new(&s.employee_id, kind, prompt, now);
                        if db.insert_run(&run, &run.id)? {
                            created.push(run);
                        }
                    }
                }
            }
            let checks = db.checks_to_run(now)?;
            let (mut busy, mut departed) = (HashSet::new(), HashSet::new());
            for id in checks.iter().map(|c| &c.employee_id).collect::<HashSet<_>>() {
                if db.employee(id)?.is_none() {
                    departed.insert(id.clone());
                } else if db.active_runs_of(id)?.iter().any(|r| r.kind.check_id().is_some()) {
                    busy.insert(id.clone());
                }
            }
            for run in due_check_runs(checks, &busy, &departed, &ceo_of(&db)?, now) {
                if db.insert_run(&run, &run.id)? {
                    created.push(run);
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
            "note": "On its due date Omega Star starts a quiet run of yours to do it. Do not also leave it as a note in a skill or promise it in your report."
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

/// The runs for due checks, oldest first: at most one per employee per pass, and none for someone who
/// has left or already has a check run queued or running (`busy`). Pure.
fn due_check_runs(mut checks: Vec<Check>, busy: &HashSet<String>, departed: &HashSet<String>, ceo: &str, now: Ts) -> Vec<Run> {
    checks.sort_by_key(|c| c.due_at);
    let mut taken = HashSet::new();
    checks
        .into_iter()
        .filter(|c| !busy.contains(&c.employee_id) && !departed.contains(&c.employee_id) && taken.insert(c.employee_id.clone()))
        .map(|c| {
            let kind = RunKind::Scheduled {
                schedule_id: format!("{CHECK_PREFIX}{}", c.id),
                slot: c.due_at,
                label: format!("Check: {}", claude::clip(&claude::one_line(&c.what), 60)),
                quiet: true,
                model: None,
                effort: None,
                review: false,
            };
            Run::new(&c.employee_id, kind, check_prompt(&c, ceo), now)
        })
        .collect()
}

/// The instruction of a due check's run.
fn check_prompt(c: &Check, ceo: &str) -> String {
    format!(
        "Your check is due (check id `{}`, set {}): {}\nHow: {}\n\nDo it now. Close it with check_done and what you found, and write the finding \
where it belongs. Then act on it within your remit: fix it, hand it to whoever owns it, or put a card to {ceo} with notify if only they \
can decide. If it cannot be settled yet, close it saying why and set a new check_later for the day it can.",
        c.id,
        c.created_at.with_timezone(&Local).format("%Y-%m-%d"),
        c.what,
        c.how
    )
}

/// The instruction of the weekly review Omega Star seeds for every employee.
fn review_instruction(ceo: &str) -> String {
    format!(
        "Your weekly review. Omega Star adds what it counted this week below.
1. Measure your number with a command you can rerun, and compare it with last week. If you have no number yet, pick one that shows your objective moving.
2. For each routine, decide to keep it, reshape it or stop it, and change your own schedules to match.
3. Act on the biggest lever your remit allows, or put the decision to {ceo} as a card with your recommendation.
4. Close every open check with check_done, or close it and set a new check_later for the day it can be settled.
5. Remove each blocker you hit twice: fix it, hand it to a colleague, or raise one chore card.
6. Check your skills and routing rules against the current team, and remove any rule your runs wrote this week that a run next month would not need. Leave every rule {ceo} taught you (marked taught or confirmed) as it is: to merge or drop one, record it with propose_changes.
7. Read what-you-own.md in your folder. If it is empty or out of date, write the version you propose there: the number you answer for, your limits, and what you decide alone. Omega Star puts the file back and shows {ceo} your version: only they set it.
End with at most three lines. The first opens with your number."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoSink;
    impl EventSink for NoSink {
        fn org_changed(&self) {}
        fn run_updated(&self, _run: &Run) {}
        fn run_events(&self, _run_id: &str, _events: &[RunEvent]) {}
        fn request_updated(&self, _request: &HumanRequest) {}
    }

    fn check(employee_id: &str, what: &str, due_at: Ts) -> Check {
        Check {
            id: uuid::Uuid::new_v4().to_string(),
            employee_id: employee_id.into(),
            what: what.into(),
            how: "Rerun the weekly report".into(),
            due_at,
            state: CheckState::Open,
            created_at: due_at - chrono::Duration::days(7),
            run_id: None,
            closed_at: None,
            result: None,
        }
    }

    #[test]
    fn due_checks_run_one_per_employee() {
        let t = now();
        let day = chrono::Duration::days(1);
        let newer = check("ada", "Did the newer fix hold", t - day);
        let older = check("ada", "Did the older fix hold", t - day * 2);
        let bo = check("bo", "Did signups move", t - day);
        let none = HashSet::new();
        let runs = due_check_runs(vec![newer, older.clone(), bo.clone()], &none, &none, "Pat", t);
        let got: Vec<(&str, Option<&str>)> = runs.iter().map(|r| (r.employee_id.as_str(), r.kind.check_id())).collect();
        assert_eq!(got, vec![("ada", Some(older.id.as_str())), ("bo", Some(bo.id.as_str()))]);
        match &runs[0].kind {
            RunKind::Scheduled { slot, label, quiet, review, .. } => {
                assert_eq!((*slot, label.as_str(), *quiet, *review), (older.due_at, "Check: Did the older fix hold", true, false));
            }
            k => panic!("{k:?}"),
        }
        assert!(runs[0].prompt.contains(&format!("check id `{}`", older.id)));
        assert!(runs[0].prompt.contains("put a card to Pat with notify"));

        let ada: HashSet<String> = ["ada".to_string()].into();
        let bo_only = |busy: &HashSet<String>, departed: &HashSet<String>| -> Vec<String> {
            due_check_runs(vec![older.clone(), bo.clone()], busy, departed, "Pat", t).into_iter().map(|r| r.employee_id).collect()
        };
        assert_eq!(bo_only(&ada, &none), vec!["bo"]);
        assert_eq!(bo_only(&none, &ada), vec!["bo"]);
    }

    #[test]
    fn a_check_label_is_one_short_line() {
        let long = format!("Did the fix\nhold {}", "and more ".repeat(20));
        let runs = due_check_runs(vec![check("ada", &long, now())], &HashSet::new(), &HashSet::new(), "Pat", now());
        let RunKind::Scheduled { label, .. } = &runs[0].kind else { panic!() };
        assert!(label.starts_with("Check: Did the fix hold and more"));
        assert!(!label.contains('\n') && label.chars().count() <= "Check: ".len() + 63);
    }

    #[test]
    fn the_review_instruction_names_the_ceo_without_dashes() {
        let text = review_instruction("Pat");
        assert!(text.contains("put the decision to Pat as a card"));
        assert!(!text.contains('\u{2014}') && !check_prompt(&check("ada", "x", now()), "Pat").contains('\u{2014}'));
    }

    #[tokio::test]
    async fn every_employee_gets_one_review_once() {
        let dir = std::env::temp_dir().join(format!("omegastar-schedules-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let ada = Employee {
            id: uuid::Uuid::new_v4().to_string(),
            slug: "ada".into(),
            name: "Ada".into(),
            title: "Tester".into(),
            description: String::new(),
            objective: String::new(),
            manager_id: None,
            skills: SkillGrant { builtins: false, keys: vec![] },
            clearance: Clearance::Supervised,
            allow_rules: vec![],
            model: None,
            effort: None,
            browser: false,
            auto_tidy: false,
            workspaces: vec![],
            hue: 0,
            chat_session: "chat-ada".into(),
            hired_at: now(),
        };
        {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.set_company(&Company { name: "Acme".into(), ceo_name: "Pat".into(), ..Company::unset() }).unwrap();
            db.save_employee(&ada).unwrap();
        }

        let office = Office::open(dir.clone(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        let reviews = office.schedules(&ada.id).unwrap();
        assert_eq!(reviews.len(), 1);
        let r = &reviews[0];
        assert!(r.review && r.enabled && !r.quiet);
        assert_eq!((r.label.as_str(), r.cron.as_str()), ("Weekly review", "0 7 * * 1"));
        assert_eq!(r.instruction, review_instruction("Pat"));

        // Re-timing keeps it the review.
        let draft = ScheduleDraft {
            id: Some(r.id.clone()),
            employee_id: ada.id.clone(),
            label: r.label.clone(),
            cron: "0 8 * * 1".into(),
            instruction: r.instruction.clone(),
            enabled: true,
            quiet: false,
            model: None,
            effort: None,
        };
        assert!(office.save_schedule(draft).unwrap().review);

        office.seed_reviews().unwrap();
        assert_eq!(office.schedules(&ada.id).unwrap().len(), 1, "seeded once");
        office.delete_schedule(&r.id).unwrap();
        office.seed_reviews().unwrap();
        assert!(office.schedules(&ada.id).unwrap().is_empty(), "a deleted review stays deleted");

        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }
}
