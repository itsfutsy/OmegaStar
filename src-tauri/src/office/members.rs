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
        let active = db.active_runs_of(&e.id)?;
        // Talking to them is how you usually start work, so a chat turn in flight counts as working
        // just like a work run does; the work lane is only preferred as the run to look at.
        let in_state = |state: RunState| {
            active.iter().filter(|r| r.state == state).min_by_key(|r| r.kind.lane() == Lane::Chat)
        };
        let live = in_state(RunState::Running);
        let waiting = in_state(RunState::Queued);
        let shown = live.or(waiting);
        let recent = db.work_runs(&e.id, 10)?;
        // A reply check that failed shows on its request's card; it is not the employee's problem.
        let last_finished = recent.iter().filter(|r| !matches!(r.kind, RunKind::ReplyCheck { .. })).find(|r| r.state.is_finished());
        let last_worked_at = db
            .recent_runs(Some(&e.id), 10)?
            .iter()
            .filter(|r| r.state.is_finished())
            .map(|r| r.ended_at.unwrap_or(r.created_at))
            .max();
        let presence = if live.is_some() {
            Presence::Working
        } else if waiting.is_some() {
            Presence::Queued
        } else if last_finished.map(|r| matches!(r.state, RunState::Failed | RunState::Lost) || !r.denials.is_empty()).unwrap_or(false) {
            Presence::Attention
        } else {
            Presence::Idle
        };
        let next_fire = if paused {
            None
        } else {
            db.schedules(&e.id)?.iter().filter(|s| s.enabled).filter_map(|s| schedule::next_fire(&s.cron, now, &Local)).min()
        };
        Ok(EmployeeView {
            employee: e.clone(),
            presence,
            active_run_id: shown.map(|r| r.id.clone()),
            active_lane: shown.map(|r| r.kind.lane()),
            working: active.iter().filter(|r| r.state == RunState::Running).count() as u32,
            waiting: active.iter().filter(|r| r.state == RunState::Queued).count() as u32,
            next_fire,
            last_run: recent.first().cloned(),
            last_worked_at,
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
        Ok(OrgSnapshot { company, employees, people })
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

    pub fn allow_rule(&self, employee_id: &str, rule: &str) -> Result<Employee> {
        let rule = rule.trim();
        if rule.is_empty() {
            return Err(AppError::invalid("rule is empty"));
        }
        let e = {
            let db = self.db();
            let mut e = db.employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            if !e.allow_rules.iter().any(|r| r == rule) {
                e.allow_rules.push(rule.to_string());
                db.save_employee(&e)?;
            }
            e
        };
        self.sink().org_changed();
        Ok(e)
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

    pub fn remove_person(&self, person_id: &str) -> Result<()> {
        {
            let db = self.db();
            let p = db.person(person_id)?.ok_or_else(|| AppError::not_found("person not found"))?;
            Self::edit_org(&db, |org| {
                org.remove_member(&p.id);
                Ok(())
            })?;
            db.remove_person(&p.id, now())?;
        }
        self.cancel_open_requests(RequestFilter { person_id: Some(person_id.into()), ..Default::default() })?;
        self.sink().org_changed();
        Ok(())
    }

    // ------------------------------------------------------------------ hire proposals

    pub fn proposals(&self, state: Option<String>) -> Result<Vec<HireProposal>> {
        let state = state.as_deref().map(ProposalState::parse).transpose()?;
        self.db().proposals(None, state)
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
    /// from, which still holds the context; a proposal made in chat gets a fresh session.
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
            let from = p.run_id.as_deref().map(|id| db.run(id)).transpose()?.flatten();
            let session = match from {
                Some(rec) if rec.run.kind.lane() == Lane::Work && db.session_started(&rec.session)? => rec.session,
                _ => run.id.clone(),
            };
            (run, session)
        };
        self.enqueue(run, session)?;
        Ok(())
    }
}
