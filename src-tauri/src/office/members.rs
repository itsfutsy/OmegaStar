//! The org: employees, the real people next to them, and hire proposals.

use super::*;

impl Office {
    /// Applies `change` to the whole org in memory and validates the result, writing nothing.
    fn plan_org<T>(db: &Store, change: impl FnOnce(&mut Org) -> Result<T>) -> Result<(Org, Org, T)> {
        let before = Org { company: db.company()?.unwrap_or_else(Company::unset), employees: db.employees()?, people: db.people()? };
        let mut after = before.clone();
        let out = change(&mut after)?;
        after.validate()?;
        Ok((before, after, out))
    }

    /// Saves every company, employee and person row a plan changed or added. Members the plan
    /// removed are left to the caller, which keeps their rows for history.
    fn save_org(db: &Store, before: &Org, after: &Org) -> Result<()> {
        db.tx(|| {
            if after.company != before.company {
                db.set_company(&after.company)?;
            }
            for e in after.employees.iter().filter(|e| !before.employees.contains(e)) {
                db.save_employee(e)?;
            }
            for p in after.people.iter().filter(|p| !before.people.contains(p)) {
                db.save_person(p)?;
            }
            Ok(())
        })
    }

    /// The one path for org mutations: plan, validate, save.
    pub(super) fn edit_org<T>(db: &Store, change: impl FnOnce(&mut Org) -> Result<T>) -> Result<T> {
        let (before, after, out) = Self::plan_org(db, change)?;
        Self::save_org(db, &before, &after)?;
        Ok(out)
    }

    pub(super) fn presence(db: &Store, e: &Employee, now: Ts, paused: bool) -> Result<EmployeeView> {
        // Talking to them is how you usually start work, so a conversation in flight counts as working just like
        // background work does. Running first; the sort is stable, so each keeps the oldest first.
        let mut live = db.active_runs_of(&e.id)?;
        live.sort_by_key(|r| r.state != RunState::Running);
        let working = live.iter().filter(|r| r.state == RunState::Running).count() as u32;
        let waiting = live.len() as u32 - working;
        let recent = db.work_runs(&e.id, 10)?;
        // A reply check that failed shows on its request's card; it is not the employee's problem.
        let last_finished = recent.iter().filter(|r| !matches!(r.kind, RunKind::ReplyCheck { .. })).find(|r| r.state.is_finished());
        let last_worked_at = db
            .recent_runs(Some(&e.id), 10)?
            .iter()
            .filter(|r| r.state.is_finished())
            .map(|r| r.ended_at.unwrap_or(r.created_at))
            .max();
        let presence = if working > 0 {
            Presence::Working
        } else if waiting > 0 {
            Presence::Queued
        } else if last_finished.map(|r| matches!(r.state, RunState::Failed | RunState::Lost) || !r.denials.is_empty()).unwrap_or(false) {
            Presence::Attention
        } else {
            Presence::Idle
        };
        // Nothing runs while schedules are paused, checks included. A routine kept to business hours runs inside them.
        let hours = db.settings()?.unwrap_or_default().watch.hours;
        let routines = if paused { vec![] } else { db.schedules(&e.id)? };
        let slots: Vec<Dated> =
            routines.iter().filter(|s| s.enabled).filter_map(|s| Some(Dated { at: schedules::next_slot(s, &hours, now, &Local)?, text: s.label.clone() })).collect();
        let next_fire = slots.iter().map(|d| d.at).min();
        let check = if paused { None } else { db.checks(&e.id, Some(CheckState::Open))?.into_iter().next() };
        let check = check.map(|c| Dated { text: format!("Check: {}", claude::clip(&claude::one_line(&c.what), 60)), at: c.due_at });
        let report = db.last_report(&e.id, now - chrono::Duration::days(REPORT_DAYS))?;
        let status = EmployeeStatus {
            number: db.latest_score(&e.id)?.map(|s| number(&s)),
            last_report: report.and_then(|r| Some(Dated { text: headline(r.summary.as_deref()?)?, at: r.ended_at.unwrap_or(r.created_at) })),
            next: slots.into_iter().chain(check).min_by_key(|d| d.at),
        };
        Ok(EmployeeView {
            employee: e.clone(),
            presence,
            live,
            working,
            next_fire,
            last_run: recent.first().cloned(),
            last_worked_at,
            status,
        })
    }

    pub fn org(&self) -> Result<OrgSnapshot> {
        let db = self.db();
        let paused = db.settings()?.map(|s| s.paused).unwrap_or(false);
        let company = db.company()?.unwrap_or_else(Company::unset);
        let now = now();
        let employees = db.employees()?.iter().map(|e| Self::presence(&db, e, now, paused)).collect::<Result<_>>()?;
        let open = db.requests(&RequestFilter { state: Some(RequestState::Open), ..Default::default() })?;
        let people = db
            .people()?
            .into_iter()
            .map(|p| PersonView { open_requests: open.iter().filter(|r| r.person_id == p.id).count() as u32, person: p })
            .collect();
        Ok(OrgSnapshot { company, employees, people, star_ask: db.star_ask_due()? })
    }

    /// You starred OmegaStar on GitHub or said no thanks: the sidebar's ask goes for good.
    pub fn answer_star_ask(&self) -> Result<()> {
        self.db().answer_star_ask()?;
        self.sink().org_changed();
        Ok(())
    }

    pub fn hire(&self, mut draft: EmployeeDraft, imports: Vec<String>) -> Result<Employee> {
        draft.validate()?;
        let catalog = self.skill_catalog();
        let picked = Self::resolve_imports(&catalog, &imports)?;
        draft.skills.keys.retain(|k| !imports.contains(k));
        let e = {
            let db = self.db();
            let mut taken: HashSet<String> = db.slugs()?.into_iter().collect();
            taken.extend(catalog.iter().map(|s| s.name.clone()));
            let slug = own_skills::slugify(&draft.name, &taken);
            let e = draft.into_employee(uuid::Uuid::new_v4().to_string(), slug, uuid::Uuid::new_v4().to_string(), now());
            let (before, after, ()) = Self::plan_org(&db, |org| {
                org.employees.push(e.clone());
                Ok(())
            })?;
            let dir = self.employee_dir(&e.id);
            fs::create_dir_all(dir.join("workspace"))?;
            let granted: Vec<SkillInfo> = catalog.iter().filter(|s| e.skills.keys.contains(&s.id)).cloned().collect();
            own_skills::ensure_skill_link(&dir)?;
            let skills_dir = own_skills::skills_dir(&dir);
            own_skills::scaffold_role_skill(&skills_dir, &e, &granted)?;
            own_skills::install_craft(&skills_dir)?;
            Self::save_org(&db, &before, &after)?;
            Self::seed_review(&db, &e, &ceo_of(&db)?, db.employees()?.len().saturating_sub(1))?;
            e
        };
        if !picked.is_empty() {
            self.stage_and_organize(&e, &picked)?;
        }
        self.sink().org_changed();
        Ok(e)
    }

    pub fn update_employee(&self, employee: Employee) -> Result<Employee> {
        let draft = employee.draft();
        draft.validate()?;
        let e = {
            let db = self.db();
            let old = db.employee(&employee.id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            let e = draft.into_employee(old.id, old.slug, old.chat_session, old.hired_at);
            Self::edit_org(&db, |org| {
                org.employees.retain(|x| x.id != e.id);
                org.employees.push(e.clone());
                Ok(())
            })?;
            e
        };
        self.sink().org_changed();
        Ok(e)
    }

    fn cancel_open_requests(&self, f: RequestFilter) -> Result<()> {
        let cancelled = {
            let db = self.db();
            let mut v = vec![];
            for mut r in db.requests(&RequestFilter { state: Some(RequestState::Open), ..f })? {
                r.state = RequestState::Cancelled;
                r.resolved_at = Some(now());
                db.save_request(&r)?;
                v.push(r);
            }
            v
        };
        for r in &cancelled {
            self.announce(r);
        }
        Ok(())
    }

    pub fn let_go(&self, employee_id: &str) -> Result<()> {
        let active = {
            let db = self.db();
            let e = db.employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            db.tx(|| {
                Self::edit_org(&db, |org| {
                    org.remove_member(&e.id);
                    Ok(())
                })?;
                db.delete_schedules_of(&e.id)?;
                db.depart_employee(&e.id, now())
            })?;
            db.active_runs_of(&e.id)?
        };
        for r in active {
            self.cancel_run(&r.id)?;
        }
        self.cancel_open_requests(RequestFilter { employee_id: Some(employee_id.into()), ..Default::default() })?;
        let withdrawn = {
            let db = self.db();
            db.tx(|| {
                for mut c in db.checks(employee_id, Some(CheckState::Open))? {
                    c.state = CheckState::Dropped;
                    c.closed_at = Some(now());
                    db.save_check(&c)?;
                }
                let mut v = vec![];
                for mut p in db.proposals(Some(employee_id), Some(ProposalState::Open))? {
                    p.state = ProposalState::Withdrawn;
                    p.resolved_at = Some(now());
                    db.save_proposal(&p)?;
                    v.push(p);
                }
                Ok(v)
            })?
        };
        for p in &withdrawn {
            self.sink().proposal_updated(p);
        }
        self.sink().org_changed();
        Ok(())
    }

    pub(super) fn allow_rule(&self, employee_id: &str, rule: &str) -> Result<()> {
        {
            let db = self.db();
            let mut e = db.employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            if !e.allow_rules.iter().any(|r| r == rule) {
                e.allow_rules.push(rule.to_string());
                db.save_employee(&e)?;
            }
        }
        self.sink().org_changed();
        Ok(())
    }

    // ------------------------------------------------------------------ people

    pub fn add_person(&self, draft: PersonDraft) -> Result<Person> {
        draft.validate()?;
        let p = {
            let db = self.db();
            let p = draft.into_person(uuid::Uuid::new_v4().to_string(), now());
            Self::edit_org(&db, |org| {
                org.people.push(p.clone());
                Ok(())
            })?;
            p
        };
        self.sink().org_changed();
        Ok(p)
    }

    pub fn update_person(&self, person: Person) -> Result<Person> {
        let draft = person.draft();
        draft.validate()?;
        let p = {
            let db = self.db();
            let old = db.person(&person.id)?.ok_or_else(|| AppError::not_found("person not found"))?;
            let p = draft.into_person(old.id, old.added_at);
            Self::edit_org(&db, |org| {
                org.people.retain(|x| x.id != p.id);
                org.people.push(p.clone());
                Ok(())
            })?;
            p
        };
        self.sink().org_changed();
        Ok(p)
    }

    /// Removes a person from the org, and starts a run for each employee whose skills, notes or what they own still
    /// name them (`names_in_files`), to take them out.
    pub fn remove_person(&self, person_id: &str) -> Result<()> {
        let (p, employees, ceo) = {
            let db = self.db();
            let p = db.person(person_id)?.ok_or_else(|| AppError::not_found("person not found"))?;
            Self::edit_org(&db, |org| {
                org.remove_member(&p.id);
                Ok(())
            })?;
            db.remove_person(&p.id, now())?;
            (p, db.employees()?, ceo_of(&db)?)
        };
        self.cancel_open_requests(RequestFilter { person_id: Some(person_id.into()), ..Default::default() })?;
        for e in employees.iter().filter(|e| names_in_files(&self.employee_dir(&e.id), &p.name)) {
            if let Err(err) = self.run_now(&e.id, Some(left_instruction(&p.name, &ceo))) {
                log!("starting {}'s clean-up after a person left failed: {err}", e.slug);
            }
        }
        self.sink().org_changed();
        Ok(())
    }

    // ------------------------------------------------------------------ hire proposals

    pub fn proposals(&self) -> Result<Vec<HireProposal>> {
        self.db().proposals(None, None)
    }

    /// Takes an open proposal out of the queue, so a second approve or decline is refused.
    fn claim_proposal(&self, proposal_id: &str, to: ProposalState, note: Option<String>) -> Result<HireProposal> {
        let db = self.db();
        let mut p = db.proposal(proposal_id)?.ok_or_else(|| AppError::not_found("proposal not found"))?;
        if p.state != ProposalState::Open {
            return Err(AppError::conflict(format!("this proposal is already {}", p.state.as_str())));
        }
        p.state = to;
        p.note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        p.resolved_at = Some(now());
        db.save_proposal(&p)?;
        Ok(p)
    }

    /// Hires from a proposal, with your changes in `draft`, and tells the proposer.
    pub fn approve_proposal(&self, proposal_id: &str, draft: EmployeeDraft, imports: Vec<String>, note: Option<String>) -> Result<Employee> {
        let mut p = self.claim_proposal(proposal_id, ProposalState::Approved, note)?;
        let e = match self.hire(draft, imports) {
            Ok(e) => e,
            Err(err) => {
                p.state = ProposalState::Open;
                p.note = None;
                p.resolved_at = None;
                self.db().save_proposal(&p)?;
                return Err(err);
            }
        };
        p.hired_id = Some(e.id.clone());
        self.db().save_proposal(&p)?;
        self.sink().proposal_updated(&p);
        self.enqueue_decision(&p, Some(&e))?;
        Ok(e)
    }

    pub fn decline_proposal(&self, proposal_id: &str, note: Option<String>) -> Result<HireProposal> {
        let p = self.claim_proposal(proposal_id, ProposalState::Declined, note)?;
        self.sink().proposal_updated(&p);
        self.enqueue_decision(&p, None)?;
        self.sink().org_changed();
        Ok(p)
    }

    /// The proposer's run that hears your decision. It resumes the work run the proposal came
    /// from, which still holds the context; a proposal made in an older build's chat gets a fresh session.
    fn enqueue_decision(&self, p: &HireProposal, hired: Option<&Employee>) -> Result<()> {
        let (run, session) = {
            let db = self.db();
            if db.employee(&p.employee_id)?.is_none() {
                return Ok(());
            }
            let ceo = ceo_of(&db)?;
            let d = &p.draft;
            let note = p.note.as_deref().map(|n| format!("\n\n{ceo}'s note:\n{n}"));
            let prompt = match hired {
                Some(e) => {
                    // None: the new hire reports to the proposer.
                    let manager = match e.manager_id.as_deref() {
                        Some(m) if m == p.employee_id => None,
                        Some(m) => Some(db.employee(m)?.map(|x| x.name).or(db.person(m)?.map(|x| x.name)).unwrap_or_else(|| "someone else".into())),
                        None => Some(ceo.clone()),
                    };
                    let changed = if e.name != d.name || e.title != d.title { format!(" (you proposed {}, {})", d.name, d.title) } else { String::new() };
                    let next = match &manager {
                        None => format!(
                            "{} reports to you. They start with only their role skill and know nothing of your work. Brief them now: \
`delegate` them a first task that gives them the context they need from you (what your area does, what you expect of them, \
where things live). Do not wait for their report: finish with a short report of what you handed over. Their report comes back \
to you as a new run when they are done.",
                            e.name
                        ),
                        Some(m) => format!(
                            "{} reports to {m}. If they need context from you, hand it over with `ask_colleague` (kind task) and finish with a \
short report; do not wait for their answer, which comes back to you as a new run.",
                            e.name
                        ),
                    };
                    format!(
                        "{ceo} approved your hire proposal: {} joined as {}{changed}.{}\n\nWhy you proposed it:\n{}\n\n{next}",
                        e.name,
                        e.title,
                        note.unwrap_or_default(),
                        p.reason
                    )
                }
                None => format!(
                    "{ceo} declined your proposal to hire {} as {}.{}\n\nWhy you proposed it:\n{}\n\nDo not propose this role again unless \
something changes. Adjust your plan for the gap, and say in a line or two how you will cover it.",
                    d.name,
                    d.title,
                    note.unwrap_or_else(|| " No reason given.".into()),
                    p.reason
                ),
            };
            let run = Run::new(&p.employee_id, RunKind::HireDecided { proposal_id: p.id.clone(), approved: hired.is_some() }, prompt, now());
            let (session, _) = Self::origin_session(&db, p.run_id.as_deref(), &run.id)?;
            (run, session)
        };
        self.enqueue(run, session)?;
        Ok(())
    }
}

/// What a run of an employee who still names someone who left is told.
fn left_instruction(name: &str, ceo: &str) -> String {
    format!(
        "{name} has left. Remove them from your skills and notes. If your what-you-own.md names them, write the change there: \
OmegaStar shows it to {ceo} to approve."
    )
}

/// The employee's skills, notes (top-level markdown in `workspace/`) or what they own name the person called `name`.
fn names_in_files(employee_dir: &Path, name: &str) -> bool {
    let skills = own_skills::snapshot(&own_skills::skills_dir(employee_dir));
    let notes = own_skills::notes_snapshot(&employee_dir.join("workspace"));
    let own = fs::read_to_string(employee_dir.join(own_skills::OWN_FILE)).unwrap_or_default();
    skills.values().chain(notes.values()).chain(std::iter::once(&own)).any(|text| mentions(text, name))
}

/// `text` names the person called `name`: their first name, which their full name starts with too, as a word of its
/// own with its capitals. Pure.
fn mentions(text: &str, name: &str) -> bool {
    let Some(first) = name.split_whitespace().next() else { return false };
    text.match_indices(first).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + first.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// How far back an employee's status looks for their last report: an older one says little about how they are doing.
const REPORT_DAYS: i64 = 30;

/// A weekly review's number as a status line shows it: "Open tickets 12 (down 3)". Pure.
fn number(s: &Score) -> String {
    let (metric, value, change) = (s.metric.trim(), s.value.trim(), s.change.trim());
    if change.is_empty() { format!("{metric} {value}") } else { format!("{metric} {value} ({change})") }
}

/// The first line of a report that has words in it, without markdown marks, short enough for a status line. Pure.
fn headline(report: &str) -> Option<String> {
    let marks: &[char] = &['#', '>', '-', '*', '|', ' '];
    report
        .lines()
        .map(|l| l.trim().trim_start_matches(marks).replace(['*', '_', '`'], ""))
        .find(|l| l.chars().any(char::is_alphanumeric))
        .map(|l| claude::clip(&l, 160))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, acme, employee_named, open_office, saved_run, task, temp_dir};

    #[test]
    fn presence_lists_live_work_running_first_then_oldest_first() {
        let dir = temp_dir("presence");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let ada = employee_named("Ada");
        let bo = employee_named("Bo");
        db.save_employee(&ada).unwrap();
        db.save_employee(&bo).unwrap();

        let task = task("Draft the plan");
        let queued_old = saved_run(&db, &ada, RunKind::Manual, "go", RunState::Queued, 30, None);
        let running_new = saved_run(&db, &ada, RunKind::Manual, "go", RunState::Running, 5, None);
        let running_old = saved_run(&db, &ada, task, "go", RunState::Running, 20, None);
        let queued_new = saved_run(&db, &ada, RunKind::Manual, "go", RunState::Queued, 1, None);
        saved_run(&db, &ada, RunKind::Manual, "go", RunState::Succeeded, 60, None);
        saved_run(&db, &bo, RunKind::Manual, "go", RunState::Succeeded, 10, None);

        let v = Office::presence(&db, &ada, now(), false).unwrap();
        let ids: Vec<&str> = v.live.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, [&running_old.id, &running_new.id, &queued_old.id, &queued_new.id]);
        assert_eq!((v.presence, v.working), (Presence::Working, 2));

        let idle = Office::presence(&db, &bo, now(), false).unwrap();
        assert!(idle.live.is_empty());
        assert_eq!(idle.presence, Presence::Idle);
        saved_run(&db, &bo, RunKind::Manual, "go", RunState::Queued, 1, None);
        assert_eq!(Office::presence(&db, &bo, now(), false).unwrap().presence, Presence::Queued);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_counts_as_a_word_of_its_own_with_its_capitals() {
        for (text, named) in [
            ("Ask Sam before a launch.", true),
            ("Sam Lee signs off posts.", true),
            ("(Sam's list)", true),
            ("Samples go to Samantha.", false),
            ("ask sam first", false),
            ("BigSam", false),
            ("", false),
        ] {
            assert_eq!(mentions(text, "Sam Lee"), named, "{text}");
        }
        assert!(!mentions("Ask Sam.", " "), "no name, no mention");
    }

    #[tokio::test]
    async fn someone_who_leaves_is_cleaned_out_of_whoever_still_names_them() {
        let dir = temp_dir("left");
        let (ada, bo, cy, dee) = (employee_named("Ada"), employee_named("Bo"), employee_named("Cy"), employee_named("Dee"));
        let sam = Person { name: "Sam Lee".into(), title: "Designer".into(), ..fixtures::person() };
        {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.set_company(&acme()).unwrap();
            for e in [&ada, &bo, &cy, &dee] {
                db.save_employee(e).unwrap();
            }
            db.save_person(&sam).unwrap();
            // Held, so the runs it starts wait.
            db.set_held_until(Some(now() + chrono::Duration::days(1))).unwrap();
        }
        let office = open_office(&dir).await;
        let write = |e: &Employee, file: &str, text: &str| {
            let path = office.employee_dir(&e.id).join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(&ada, "skills/ada-launch/SKILL.md", "Sam signs off every launch.\n");
        write(&bo, "workspace/logo.md", "Waiting on Sam Lee for the new logo.\n");
        write(&cy, own_skills::OWN_FILE, "Spend over 100 goes to Sam first.\n");
        write(&dee, "skills/dee-ops/SKILL.md", "Samples go to the shared drive.\n");

        office.remove_person(&sam.id).unwrap();
        let told = left_instruction("Sam Lee", "Pat");
        assert_eq!(told, "Sam Lee has left. Remove them from your skills and notes. If your what-you-own.md names them, write the change there: OmegaStar shows it to Pat to approve.");
        let cleanup = |e: &Employee| office.db().active_runs_of(&e.id).unwrap().into_iter().map(|r| (r.kind, r.prompt)).collect::<Vec<_>>();
        for e in [&ada, &bo, &cy] {
            assert_eq!(cleanup(e), vec![(RunKind::Manual, told.clone())], "{}", e.name);
        }
        assert!(cleanup(&dee).is_empty());
        assert!(office.org().unwrap().people.is_empty());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_hire_decision_resumes_the_started_work_session_it_was_proposed_in_and_starts_fresh_otherwise() {
        let dir = temp_dir("decision");
        let ada = employee_named("Ada");
        let (started, unstarted, chat) = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&ada).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::days(1))).unwrap();
            let started = saved_run(&db, &ada, RunKind::Manual, "go", RunState::Succeeded, 30, None);
            db.mark_inited(&started.id).unwrap();
            let unstarted = saved_run(&db, &ada, RunKind::Manual, "go", RunState::Failed, 20, None);
            let mut chat = Run::new(&ada.id, RunKind::Chat, "Find us a designer.".into(), now() - chrono::Duration::minutes(10));
            chat.state = RunState::Succeeded;
            chat.ended_at = Some(chat.created_at);
            assert!(db.insert_run(&chat, &ada.chat_session).unwrap());
            db.mark_inited(&chat.id).unwrap();
            (started, unstarted, chat)
        };
        let office = open_office(&dir).await;
        let decided = |from: Option<&Run>| {
            let p = HireProposal {
                id: uuid::Uuid::new_v4().to_string(),
                employee_id: ada.id.clone(),
                run_id: from.map(|r| r.id.clone()),
                reason: "Design work piles up.".into(),
                draft: employee_named("Cy").draft(),
                missing_skills: vec![],
                state: ProposalState::Open,
                note: None,
                hired_id: None,
                created_at: now(),
                resolved_at: None,
            };
            office.db().save_proposal(&p).unwrap();
            office.decline_proposal(&p.id, None).unwrap();
            let db = office.db();
            let told = db.active_runs_of(&ada.id).unwrap().into_iter().find(|r| matches!(&r.kind, RunKind::HireDecided { proposal_id, .. } if *proposal_id == p.id)).unwrap();
            let session = db.run(&told.id).unwrap().unwrap().session;
            (told.id, session)
        };
        let (_, session) = decided(Some(&started));
        assert_eq!(session, started.id, "a started work session carries on");
        for from in [Some(&unstarted), Some(&chat), None] {
            let (id, session) = decided(from);
            assert_eq!(session, id, "starts fresh: {:?}", from.map(|r| &r.kind));
        }
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_status_says_their_number_their_last_report_and_what_is_next() {
        let dir = temp_dir("status");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let ada = employee_named("Ada");
        db.save_employee(&ada).unwrap();
        use chrono::TimeZone;
        // On this computer's clock, away from Monday's weekly review. 4 November 2026 is a Wednesday.
        let t = Local.with_ymd_and_hms(2026, 11, 4, 10, 15, 0).unwrap().with_timezone(&Utc);
        assert_eq!(Office::presence(&db, &ada, t, false).unwrap().status, EmployeeStatus::default());

        let schedule = |id: &str, label: &str, cron: &str, review: bool| Schedule {
            id: id.into(),
            employee_id: ada.id.clone(),
            label: label.into(),
            cron: cron.into(),
            instruction: "Go.".into(),
            enabled: true,
            misfire: Misfire::CatchUp,
            quiet: !review,
            model: None,
            effort: None,
            review,
            active_hours: false,
            next_fire: None,
        };
        db.save_schedule(&schedule("r1", "Weekly review", "30 7 * * 1", true), t).unwrap();
        db.save_schedule(&schedule("s1", "Inbox sweep", "0 * * * *", false), t).unwrap();
        let day = chrono::Duration::days(1);
        let kind = RunKind::Scheduled { schedule_id: "r1".into(), slot: t - day, label: "Weekly review".into(), quiet: false, model: None, effort: None, review: true };
        let mut review = Run::new(&ada.id, kind, "go".into(), t - day);
        (review.state, review.ended_at) = (RunState::Succeeded, Some(t - day));
        review.score = Some(Score { metric: "Signups".into(), value: "42".into(), change: "+3".into(), note: String::new() });
        assert!(db.insert_run(&review, &review.id).unwrap());
        let mut report = Run::new(&ada.id, RunKind::Manual, "go".into(), t - chrono::Duration::hours(2));
        (report.state, report.ended_at) = (RunState::Succeeded, Some(t - chrono::Duration::hours(1)));
        report.summary = Some("\n**Paused** both `campaigns`.\n- More below.".into());
        assert!(db.insert_run(&report, &report.id).unwrap());

        let v = Office::presence(&db, &ada, t, false).unwrap();
        let hour = schedules::next_slot(&schedule("s1", "Inbox sweep", "0 * * * *", false), &BusinessHours::default(), t, &Local).unwrap();
        assert_eq!(v.status.number.as_deref(), Some("Signups 42 (+3)"));
        assert_eq!(v.status.last_report, Some(Dated { text: "Paused both campaigns.".into(), at: t - chrono::Duration::hours(1) }));
        assert_eq!((v.next_fire, v.status.next), (Some(hour), Some(Dated { text: "Inbox sweep".into(), at: hour })), "the next whole hour");

        let due = t - chrono::Duration::hours(1);
        let c = Check { id: "c1".into(), employee_id: ada.id.clone(), what: "Did the\nfix hold?".into(), how: "Rerun it".into(), due_at: due, state: CheckState::Open, created_at: due - day, run_id: None, closed_at: None, result: None };
        db.save_check(&c).unwrap();
        assert_eq!(Office::presence(&db, &ada, t, false).unwrap().status.next, Some(Dated { text: "Check: Did the fix hold?".into(), at: due }), "a check due sooner");

        let paused = Office::presence(&db, &ada, t, true).unwrap();
        assert_eq!((paused.next_fire, paused.status.next), (None, None), "nothing runs while schedules are paused");
        assert_eq!(paused.status.number.as_deref(), Some("Signups 42 (+3)"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_status_line_reads_as_plain_text() {
        let score = Score { metric: " Open tickets ".into(), value: "12".into(), change: " ".into(), note: "Down from 15.".into() };
        assert_eq!(number(&score), "Open tickets 12");
        assert_eq!(headline("# Weekly report\nAll quiet."), Some("Weekly report".into()));
        assert_eq!(headline("```\n---\n| **Spend** | 40 |"), Some("Spend | 40 |".into()));
        assert_eq!(headline("  \n---\n"), None);
        assert_eq!(headline(&"word ".repeat(60)).map(|l| l.chars().count()), Some(162), "clipped at a word, with an ellipsis");
    }
}
