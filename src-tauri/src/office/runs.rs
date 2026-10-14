//! The run queue: enqueueing, claiming, gathering a snapshot, executing and cancelling runs.

use super::*;

const EVENT_BATCH: Duration = Duration::from_millis(100);

impl Office {
    pub(super) fn enqueue(&self, run: Run, session: String) -> Result<Run> {
        self.db().insert_run(&run, &session)?;
        self.sink().run_updated(&run);
        self.pump();
        self.sink().org_changed();
        Ok(self.db().run(&run.id)?.map(|r| r.run).unwrap_or(run))
    }

    pub fn run_now(&self, employee_id: &str, instruction: Option<String>) -> Result<Run> {
        self.db().employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
        let prompt = instruction.filter(|i| !i.trim().is_empty()).unwrap_or_else(|| DEFAULT_PROMPT.into());
        let run = Run::new(employee_id, RunKind::Manual, prompt, now());
        let session = run.id.clone();
        self.enqueue(run, session)
    }

    /// Decodes and checks the files attached to a message: at most `MAX_ATTACHMENTS`, each within
    /// `MAX_ATTACHMENT_BYTES`, under safe de-duplicated names.
    pub(super) fn decode_attachments(attachments: &[NewAttachment]) -> Result<Vec<(String, Vec<u8>)>> {
        use base64::Engine;
        if attachments.len() > MAX_ATTACHMENTS {
            return Err(AppError::invalid(format!("attach at most {MAX_ATTACHMENTS} files at a time")));
        }
        let names = attachment_names(attachments.iter().map(|a| a.name.as_str()));
        let mut files = vec![];
        for (a, name) in attachments.iter().zip(names) {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&a.data)
                .map_err(|_| AppError::invalid(format!("{name} could not be read")))?;
            if bytes.len() > MAX_ATTACHMENT_BYTES {
                return Err(AppError::invalid(format!("{name} is larger than {} MB", MAX_ATTACHMENT_BYTES / (1024 * 1024))));
            }
            files.push((name, bytes));
        }
        Ok(files)
    }

    /// Saves decoded attachments under the employee's `attachments/<run id>/` and names them on the run.
    pub(super) fn attach(&self, run: &mut Run, files: &[(String, Vec<u8>)]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let dir = claude::attachments_dir(&self.employee_dir(&run.employee_id), &run.id);
        let written = fs::create_dir_all(&dir).and_then(|_| files.iter().try_for_each(|(name, bytes)| fs::write(dir.join(name), bytes)));
        if let Err(err) = written {
            let _ = fs::remove_dir_all(&dir);
            return Err(AppError::internal(format!("Could not save the attachments: {err}")));
        }
        run.attachments = files.iter().map(|(name, _)| name.clone()).collect();
        Ok(())
    }

    /// Your message in the ongoing chat. It goes into the turn that is working, or else runs as the next
    /// turn. A message with attachments is always a turn of its own.
    pub fn send_chat(&self, employee_id: &str, text: &str, attachments: Vec<NewAttachment>) -> Result<Run> {
        if text.trim().is_empty() && attachments.is_empty() {
            return Err(AppError::invalid("message is empty"));
        }
        let files = Self::decode_attachments(&attachments)?;
        let (e, working) = {
            let db = self.db();
            let e = db.employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            // Only the newest turn takes messages, so a message never overtakes one queued before it.
            let working = db.chat_runs(&e.id, &e.chat_session)?.pop().filter(|r| r.state == RunState::Running);
            (e, working)
        };
        if let Some(r) = working.filter(|r| files.is_empty() && self.steer(&r.id, text)) {
            return Ok(r);
        }
        let mut run = Run::new(employee_id, RunKind::Chat, text.to_string(), now());
        self.attach(&mut run, &files)?;
        self.enqueue(run, e.chat_session)
    }

    /// Starts a task: a conversation of its own, so the employee works on it at the same time as
    /// everything else. It starts from what you discussed in the ongoing chat.
    pub fn start_task(&self, employee_id: &str, text: &str, attachments: Vec<NewAttachment>) -> Result<Run> {
        let text = text.trim();
        if text.is_empty() && attachments.is_empty() {
            return Err(AppError::invalid("message is empty"));
        }
        let files = Self::decode_attachments(&attachments)?;
        let mut run = {
            let db = self.db();
            let e = db.employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            let context = threads::context(&ceo_of(&db)?, &db.chat_runs(&e.id, &e.chat_session)?);
            let from = (!context.is_empty()).then(|| "your chat".to_string());
            let names: Vec<String> = attachments.iter().map(|a| a.name.clone()).collect();
            Run::new(&e.id, RunKind::Task { label: threads::task_label(text, &names), from, context }, text.to_string(), now())
        };
        self.attach(&mut run, &files)?;
        let session = run.id.clone();
        self.enqueue(run, session)
    }

    /// The file behind one of a run's attachments.
    pub fn attachment_path(&self, run_id: &str, name: &str) -> Result<PathBuf> {
        let run = self.run(run_id)?;
        if !run.attachments.iter().any(|a| a == name) {
            return Err(AppError::not_found("attachment not found"));
        }
        let path = claude::attachments_dir(&self.employee_dir(&run.employee_id), run_id).join(name);
        if !path.is_file() {
            return Err(AppError::not_found(format!("{name} is no longer there")));
        }
        Ok(path)
    }

    pub fn new_chat(&self, employee_id: &str) -> Result<()> {
        {
            let db = self.db();
            let mut e = db.employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            e.chat_session = uuid::Uuid::new_v4().to_string();
            db.save_employee(&e)?;
        }
        self.sink().org_changed();
        Ok(())
    }

    pub fn chat(&self, employee_id: &str) -> Result<Vec<Run>> {
        let db = self.db();
        let e = db.employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
        db.chat_runs(&e.id, &e.chat_session)
    }

    pub fn runs(&self, employee_id: Option<&str>, limit: u32) -> Result<Vec<Run>> {
        self.db().recent_runs(employee_id, limit.clamp(1, 1000))
    }

    pub fn run(&self, run_id: &str) -> Result<Run> {
        self.db().run(run_id)?.map(|r| r.run).ok_or_else(|| AppError::not_found("run not found"))
    }

    pub fn run_detail(&self, run_id: &str) -> Result<RunDetail> {
        let run = self.run(run_id)?;
        let transcript = fs::read_to_string(self.run_dir(run_id).join("transcript.jsonl")).unwrap_or_default();
        let children = self.db().children(run_id)?;
        let events = claude::replay(&transcript, &self.employee_dir(&run.employee_id));
        Ok(RunDetail { events, run, children })
    }

    pub fn cancel_run(&self, run_id: &str) -> Result<()> {
        let queued = {
            let db = self.db();
            let mut r = db.run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
            match r.state {
                RunState::Queued => {
                    r.state = RunState::Cancelled;
                    r.ended_at = Some(now());
                    db.save_run(&r)?;
                    Some(r)
                }
                _ => None,
            }
        };
        match queued {
            Some(r) => {
                self.sink().run_updated(&r);
                self.0.finished.send_modify(|n| *n += 1);
                self.sink().org_changed();
            }
            None => {
                let pid = {
                    let mut active = lock(&self.0.active);
                    active.get_mut(run_id).and_then(|a| {
                        a.cancelled = true;
                        a.pid
                    })
                };
                if let Some(pid) = pid {
                    claude::kill_tree(pid);
                }
            }
        }
        Ok(())
    }

    /// Kills every live run (on Quit): children would otherwise outlive the Office.
    pub fn shutdown(&self) {
        let pids: Vec<u32> = lock(&self.0.active).values_mut().filter_map(|a| {
            a.cancelled = true;
            a.pid
        }).collect();
        for pid in pids {
            claude::kill_tree(pid);
        }
        let _ = fs::remove_file(self.0.data_dir.join(RUNNING_MARKER));
    }

    /// Starts every queued run that is free to go, within the global work cap. A conversation takes
    /// turns (one run per session at a time), and an employee does one piece of background work at a
    /// time; their conversations with you run beside it. Delegated runs whose parent is running bypass
    /// the cap: the parent is blocked on them, so capping them could deadlock.
    pub(super) fn pump(&self) {
        let (started, dropped) = match self.claim_runnable() {
            Ok(v) => v,
            Err(e) => {
                log!("dispatch failed: {e}");
                return;
            }
        };
        for run in &dropped {
            self.sink().run_updated(run);
        }
        if !dropped.is_empty() {
            self.0.finished.send_modify(|n| *n += 1);
            self.sink().org_changed();
        }
        for run in &started {
            self.sink().run_updated(run);
            let office = self.clone();
            let run = run.clone();
            self.0.rt.spawn(async move { office.execute(run).await });
        }
        if !started.is_empty() {
            self.sink().org_changed();
        }
    }

    /// The queued runs to start now, and the reply checks and nudges no longer needed, cancelled.
    fn claim_runnable(&self) -> Result<(Vec<Run>, Vec<Run>)> {
        let stamp = self.run_build();
        // Each read takes its own lock and lets it go before the store is locked (see Inner).
        let at = now();
        let usage = self.usage_limits();
        let held = usage.held_until.is_some_and(|t| t > at);
        let account_ok = self.account_cached().is_some() && usage.account_issue.is_none();
        let awake = self.0.awake.load(Ordering::Relaxed);
        let db = self.db();
        // One unit: a run marked running here must be returned, or it would never start.
        let claimed = db.tx(|| {
            let cap = db.settings()?.map(|s| s.max_concurrent_runs).unwrap_or(3).max(1) as usize;
            let running = db.records_in_state(RunState::Running)?;
            let mut sessions: HashSet<String> = running.iter().map(|r| r.session.clone()).collect();
            let mut working: HashSet<String> = running.iter().filter(|r| !r.run.kind.is_conversation()).map(|r| r.run.employee_id.clone()).collect();
            let running_ids: HashSet<String> = running.iter().map(|r| r.run.id.clone()).collect();
            let mut talking: HashSet<String> = running.iter().filter(|r| r.run.kind.is_conversation()).map(|r| r.run.employee_id.clone()).collect();
            let mut work = running.iter().filter(|r| r.run.kind.lane() == Lane::Work).count();
            let mut started = vec![];
            let mut dropped = vec![];
            for RunRecord { run: mut r, session } in db.records_in_state(RunState::Queued)? {
                if let RunKind::ReplyCheck { request_id, .. } | RunKind::Nudge { request_id, .. } = &r.kind {
                    if db.request(request_id)?.as_ref().and_then(watch::Facts::of).is_none() {
                        r.state = RunState::Cancelled;
                        r.ended_at = Some(now());
                        r.error = Some("No longer needed: the request was answered or stopped.".into());
                        db.save_run(&r)?;
                        dropped.push(r);
                        continue;
                    }
                }
                if gate(&r.kind, held, account_ok, talking.contains(&r.employee_id), awake) == Gate::Skip {
                    continue;
                }
                let lane = r.kind.lane();
                let background = !r.kind.is_conversation();
                if sessions.contains(&session) || (background && working.contains(&r.employee_id)) {
                    continue;
                }
                let exempt = r.parent_run_id().map(|p| running_ids.contains(p)).unwrap_or(false);
                if lane == Lane::Work && work >= cap && !exempt {
                    continue;
                }
                r.state = RunState::Running;
                r.started_at = Some(now());
                r.build = Some(stamp.clone());
                db.save_run(&r)?;
                sessions.insert(session);
                if background {
                    working.insert(r.employee_id.clone());
                } else {
                    talking.insert(r.employee_id.clone());
                }
                if lane == Lane::Work {
                    work += 1;
                }
                started.push(r);
            }
            Ok((started, dropped))
        })?;
        // Registered while the store is still locked, so a cancel that sees a run as running finds it.
        let mut active = lock(&self.0.active);
        for r in &claimed.0 {
            let token = uuid::Uuid::new_v4().simple().to_string();
            active.insert(r.id.clone(), Active { token, pid: None, cancelled: false, needs_you: vec![], card: None, proposed: None, verdict: false, transcript: None, input: None });
        }
        Ok(claimed)
    }

    /// Sends your message into a run that is still working: the employee sees it at their next step.
    /// `false` when the run no longer takes messages (its turn is over, or it has not started).
    pub(super) fn steer(&self, run_id: &str, text: &str) -> bool {
        let ev = RunEvent::User { text: text.to_string() };
        {
            let active = lock(&self.0.active);
            let Some(a) = active.get(run_id) else { return false };
            if !a.input.as_ref().is_some_and(|i| i.send(claude::user_line(text)).is_ok()) {
                return false;
            }
            if let Some(f) = &a.transcript {
                append_line(f, &claude::omega_line(&ev));
            }
        }
        self.sink().run_events(run_id, &[ev]);
        true
    }

    /// Appends a synthetic event to a live run's transcript and forwards it to the UI.
    pub(super) fn append_event(&self, run_id: &str, ev: RunEvent) {
        let file = lock(&self.0.active).get(run_id).and_then(|a| a.transcript.clone());
        if let Some(f) = file {
            append_line(&f, &claude::omega_line(&ev));
        }
        self.sink().run_events(run_id, &[ev]);
    }

    fn gather(&self, run: &Run, token: &str) -> Result<claude::Snapshot> {
        // Locating claude can start a login shell, so it happens before the store is locked.
        let settings = self.db().settings()?;
        let claude_path = claude::locate(settings.as_ref().and_then(|s| s.claude_path.as_deref()))
            .ok_or_else(|| AppError::invalid("Claude Code was not found. Install it, or set its path in Settings."))?;
        let db = self.db();
        let e = db.employee(&run.employee_id)?.ok_or_else(|| AppError::not_found("This employee has left the company."))?;
        let company = db
            .company()
            ?
            .unwrap_or_else(|| Company { name: "the company".into(), ceo_name: "the person you work for".into(), ..Company::unset() });
        let employees = db.employees()?;
        let people = db.people()?;
        let manager = e.manager_id.as_ref().and_then(|m| {
            employees
                .iter()
                .find(|x| &x.id == m)
                .map(|x| (x.name.clone(), x.title.clone()))
                .or_else(|| people.iter().find(|p| &p.id == m).map(|p| (p.name.clone(), p.title.clone())))
        });
        let boss = company.reports_to.as_ref().and_then(|b| people.iter().find(|p| &p.id == b)).map(|p| (p.name.clone(), p.title.clone()));
        let reports = employees
            .iter()
            .filter(|x| x.manager_id.as_deref() == Some(&e.id))
            .map(|x| claude::Report { name: x.name.clone(), slug: x.slug.clone(), title: x.title.clone(), objective: x.objective.clone() })
            .collect();
        let colleagues = employees
            .iter()
            .filter(|x| x.id != e.id && x.manager_id.as_deref() != Some(&e.id))
            .map(|x| claude::Colleague {
                name: x.name.clone(),
                slug: x.slug.clone(),
                title: x.title.clone(),
                objective: x.objective.clone(),
                description: x.description.clone(),
            })
            .collect();
        let counterpart = match &run.kind {
            RunKind::Asked { asked_by: id, .. } | RunKind::Answered { from: id, .. } => {
                employees.iter().find(|x| &x.id == id).map(|x| (x.name.clone(), x.title.clone()))
            }
            _ => None,
        };
        let person_name = |id: &str| people.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| "a former member".into());
        let waiting_on = db
            .requests(&RequestFilter { employee_id: Some(e.id.clone()), state: Some(RequestState::Open), ..Default::default() })
            ?
            .into_iter()
            .map(|r| {
                let n = person_name(&r.person_id);
                (r, n)
            })
            .collect();
        let recent_work = db
            .work_runs(&e.id, 30)
            ?
            .into_iter()
            .filter(|r| r.id != run.id && r.state.is_finished() && !matches!(r.kind, RunKind::Tidy { .. } | RunKind::ReplyCheck { .. }))
            .filter_map(|r| Some((r.ended_at.unwrap_or(r.created_at), claude::kind_label(&r.kind), r.state, r.summary.clone().filter(|s| !s.trim().is_empty())?)))
            .take(5)
            .collect();
        let schedules = db.schedules(&e.id)?;
        let proposals = db.proposals(Some(&e.id), Some(ProposalState::Open))?;
        let checks = db.checks(&e.id, Some(CheckState::Open))?;
        let replier = match &run.kind {
            RunKind::FollowUp { person_id, .. }
            | RunKind::ReplyCheck { person_id, .. }
            | RunKind::Nudge { person_id, .. }
            | RunKind::Unanswered { person_id, .. } => Some(person_name(person_id)),
            _ => None,
        };
        let watched = match &run.kind {
            RunKind::ReplyCheck { request_id, person_id } | RunKind::Nudge { request_id, person_id } | RunKind::Unanswered { request_id, person_id } => {
                db.request(request_id)?.map(|request| {
                    let channels = people.iter().find(|p| &p.id == person_id).map(|p| p.channels.as_slice()).unwrap_or_default();
                    let channel = request.channel_id.as_ref().and_then(|c| channels.iter().find(|x| &x.id == c)).map(claude::channel_name);
                    let days = settings.as_ref().map_or(WatchSettings::default(), |s| s.watch.clone()).give_up_after_days;
                    claude::Watched { person: person_name(person_id), channel, days, request }
                })
            }
            _ => None,
        };
        let session = db.run(&run.id)?.map(|r| r.session).unwrap_or_else(|| run.id.clone());
        let resume = db.session_started(&session)?;
        drop(db);
        let tidy = self.tidy_brief(run, &e)?;
        let catalog = self.skill_catalog();
        let own = own_skills::own_skill_names(&self.skills_dir(&e.id));
        let granted = catalog.iter().filter(|s| e.skills.keys.contains(&s.id)).cloned().collect();
        Ok(claude::Snapshot {
            granted,
            claude: claude_path,
            settings: catalog::render_settings(&e.skills, &catalog, &own, &allowed_rules(&e, &run.allow_once)),
            employee_dir: self.employee_dir(&e.id),
            run_dir: self.run_dir(&run.id),
            mcp_url: format!("http://127.0.0.1:{}/mcp/{token}", self.0.mcp_port),
            company,
            employee: e,
            manager,
            boss,
            reports,
            colleagues,
            people,
            counterpart,
            waiting_on,
            recent_work,
            schedules,
            proposals,
            checks,
            tidy,
            replier,
            watched,
            resume,
            session,
        })
    }

    async fn execute(self, mut run: Run) {
        let ended = self.drive(&run).await;
        let (cancelled, needs_you, card, proposed, verdict) =
            lock(&self.0.active).remove(&run.id).map(|a| (a.cancelled, a.needs_you, a.card, a.proposed, a.verdict)).unwrap_or_default();
        run.needs_you.extend(needs_you);
        // Before the limit requeue, so a card raised before the limit stopped the run survives it.
        if card.is_some() {
            run.card = card;
        }
        // Added to what an attempt the usage limit stopped proposed, so nothing it proposed is lost.
        if let Some(p) = proposed.filter(|p| !p.trim().is_empty()) {
            run.proposed = Some(match run.proposed.take() {
                Some(earlier) => format!("{earlier}\n\n{p}"),
                None => p,
            });
        }
        // Held before the run is saved back to the queue, so no pump claims it straight into the limit,
        // and held too when it is out of retries, so other queued runs do not start into it.
        if hit_limit(cancelled, &ended) {
            let reset = ended.as_ref().ok().and_then(|e| e.limited);
            self.hold_until(reset.unwrap_or_else(|| now() + chrono::Duration::minutes(15)));
        }
        if requeue_for_limit(cancelled, &ended, run.limit_retries) {
            if let Ok(e) = ended {
                self.requeue_after_limit(run, e);
            }
            return;
        }
        if matches!(run.kind, RunKind::Organize { .. }) {
            let left: Vec<String> = own_skills::staged_imports(&own_skills::imports_dir(&self.employee_dir(&run.employee_id))).into_iter().map(|s| s.name).collect();
            if !left.is_empty() {
                let what = if left.len() == 1 { "skill is" } else { "skills are" };
                run.needs_you.push(format!("{} imported {what} still waiting to be organized: {}.", left.len(), left.join(", ")));
            }
        }
        let (state, error) = final_state(cancelled, &ended);
        run.state = state;
        run.error = error;
        let ended_at = now();
        run.ended_at = Some(ended_at);
        run.duration_ms = run.started_at.map(|s| (ended_at - s).num_milliseconds().max(0) as u64);
        if let Ok(e) = &ended {
            if let Some(o) = &e.outcome {
                run.turns = Some(o.turns);
                run.summary = Some(o.text.clone()).filter(|t| !t.trim().is_empty());
            }
            run.denials = e.blocks.clone();
            if !e.notes.is_empty() {
                let notes = e.notes.join("\n");
                run.summary = Some(match run.summary.take() {
                    Some(s) => format!("{s}\n\n{notes}"),
                    None => notes,
                });
            }
            run.context_tokens = e.context;
            if let (Some(b), Some(v)) = (run.build.as_mut(), e.claude_version.clone()) {
                b.claude = Some(v);
            }
            // Extended, not replaced: a run the usage limit stopped before reports those attempts' changes too.
            run.skill_changes.extend(e.changes.iter().map(|c| c.change.clone()));
        }
        if matches!(run.kind, RunKind::Tidy { .. }) && run.state == RunState::Succeeded {
            self.review_tidy(&mut run);
        }
        // Before the run is seen to finish, so whoever waits for it also sees what it sent.
        if let Err(e) = self.capture_deliveries(&run.id, false) {
            log!("finding sends in run {} failed: {e}", run.id);
        }
        if let RunKind::ReplyCheck { request_id, .. } = &run.kind {
            if !verdict && run.state != RunState::Cancelled {
                if let Err(e) = self.check_missed(request_id, run.error.clone().unwrap_or_else(|| "The check ended without a result".into())) {
                    log!("recording a missed check failed: {e}");
                }
            }
        }
        let saved = {
            let db = self.db();
            db.tx(|| {
                if let Ok(e) = ended {
                    for fc in e.changes {
                        db.insert_revision(&run.employee_id, &own_skills::revision(fc, Some(run.id.clone()), now()))?;
                    }
                }
                db.save_run(&run)
            })
        };
        if let Err(e) = saved {
            log!("saving run {} failed: {e}", run.id);
        }
        if run.card.as_ref().is_some_and(|c| c.is_open() && !matches!(c.ask, CardAsk::Chore { .. })) {
            if let Err(e) = self.replace_older_cards(&run) {
                log!("replacing the cards before run {} failed: {e}", run.id);
            }
        }
        if let RunKind::Scheduled { schedule_id, quiet, label, .. } = &run.kind {
            if threads::broke(&run) {
                // A quiet routine that breaks is exactly what needs you: nobody reads its reports
                // otherwise. Once per run of failures: the rest join the same Inbox row.
                let again = self.db().previous_finished(schedule_id, run.created_at).ok().flatten().is_some_and(|p| threads::broke(&p));
                if *quiet && !again {
                    let name = self.db().employee(&run.employee_id).ok().flatten().map(|e| e.name).unwrap_or_default();
                    self.sink().notify(&format!("{name}: '{label}' failed"), run.error.as_deref().unwrap_or("Open Omega Star for details."));
                }
            } else if run.state == RunState::Succeeded {
                if let Err(e) = self.archive_streak(schedule_id, &run) {
                    log!("archiving the failures before run {} failed: {e}", run.id);
                }
            }
        }
        self.sink().run_updated(&run);
        if let Err(e) = self.surface_thread(&run) {
            log!("surfacing the thread after run {} failed: {e}", run.id);
        }
        if let Err(e) = self.deliver_answers(&run) {
            log!("delivering answers after run {} failed: {e}", run.id);
        }
        self.0.finished.send_modify(|n| *n += 1);
        self.pump();
        self.sink().org_changed();
    }

    /// Puts a run the usage limit stopped back in the queue with what it changed so far. `execute` has
    /// already held the office until the limit resets.
    fn requeue_after_limit(&self, mut run: Run, e: Ended) {
        let changes: Vec<SkillChange> = e.changes.iter().map(|c| c.change.clone()).collect();
        reset_for_requeue(&mut run, &changes);
        let saved = {
            let db = self.db();
            db.tx(|| {
                for fc in e.changes {
                    db.insert_revision(&run.employee_id, &own_skills::revision(fc, Some(run.id.clone()), now()))?;
                }
                db.save_run(&run)?;
                let session = db.run(&run.id)?.map(|r| r.session).unwrap_or_else(|| run.id.clone());
                db.session_started(&session)
            })
        };
        match saved {
            Ok(resumes) => log!("requeued {} after the usage limit (attempt {}, resumes {resumes})", run.id, run.limit_retries),
            Err(err) => log!("requeueing run {} after the usage limit failed: {err}", run.id),
        }
        self.sink().run_updated(&run);
        self.sink().org_changed();
    }

    /// A schedule that works again closes its run of failures: the ones since its last success before
    /// `ok` move to the Archive, unless you replied to one.
    fn archive_streak(&self, schedule_id: &str, ok: &Run) -> Result<()> {
        let cleared = {
            let db = self.db();
            db.tx(|| {
                let since = db.successes_of_schedule(schedule_id)?.into_iter().filter(|t| *t < ok.created_at).max();
                let mut cleared = vec![];
                for mut f in db.failures_of_schedule(schedule_id)? {
                    let in_streak = f.created_at < ok.created_at && since.is_none_or(|s| f.created_at > s);
                    let open_card = f.card.as_ref().is_some_and(Card::is_open);
                    if in_streak && f.archived_at.is_none() && f.replied_at.is_none() && !open_card {
                        f.archived_at = Some(now());
                        db.save_run(&f)?;
                        cleared.push(f);
                    }
                }
                Ok(cleared)
            })?
        };
        for f in &cleared {
            self.sink().run_updated(f);
        }
        Ok(())
    }

    /// A newer card closes the open decision and item cards it supersedes, so a routine never stacks them.
    fn replace_older_cards(&self, newer: &Run) -> Result<()> {
        let replaced = {
            let db = self.db();
            db.tx(|| replace_older_cards_in(&db, newer, now()))?
        };
        for r in &replaced {
            self.sink().run_updated(r);
        }
        Ok(())
    }

    /// Writes into a run's open stdin without recording it as your message: for answers Omega Star
    /// writes, such as a chore you did. `false` when the run no longer takes messages.
    pub(super) fn tell(&self, run_id: &str, text: &str) -> bool {
        let active = lock(&self.0.active);
        active.get(run_id).and_then(|a| a.input.as_ref()).is_some_and(|i| i.send(claude::user_line(text)).is_ok())
    }

    /// Prepares files, spawns claude, streams its output into the transcript and the UI, and diffs
    /// the employee's skills afterwards. `Err` means the process never started.
    async fn drive(&self, run: &Run) -> Result<Ended> {
        let run_dir = self.run_dir(&run.id);
        fs::create_dir_all(&run_dir)?;
        let transcript = OpenOptions::new()
            .create(true)
            .append(true)
            .open(run_dir.join("transcript.jsonl"))?;
        let transcript = Arc::new(Mutex::new(transcript));
        let token = {
            let mut active = lock(&self.0.active);
            let a = active.get_mut(&run.id).ok_or_else(|| AppError::internal("the run was not claimed"))?;
            a.transcript = Some(transcript.clone());
            a.token.clone()
        };

        let snap = self.gather(run, &token)?;
        let inv = claude::prepare(&snap, run);
        let skills_dir = own_skills::skills_dir(&snap.employee_dir);
        own_skills::ensure_skill_link(&snap.employee_dir)?;
        own_skills::install_craft(&skills_dir)?;
        for (path, text) in &inv.files {
            // Left alone when unchanged, so an unchanged handbook keeps its modified time.
            if fs::read_to_string(path).ok().as_deref() == Some(text) {
                continue;
            }
            // Written whole and swapped in: the employee's other runs may be reading the same handbook.
            let tmp = path.with_extension(format!("{}.tmp", run.id));
            fs::write(&tmp, text)
                .and_then(|_| fs::rename(&tmp, path))
                .map_err(|e| AppError::internal(format!("writing {}: {e}", path.display())))?;
        }
        if matches!(run.kind, RunKind::Organize { .. }) {
            own_skills::backup_imports(&own_skills::imports_dir(&snap.employee_dir), &run_dir.join("imports"))?;
        }
        let is_tidy = matches!(run.kind, RunKind::Tidy { .. });
        let notes_dir = snap.employee_dir.join("workspace");
        if is_tidy {
            tidy::clean_generated(&skills_dir);
        }
        let mut before = own_skills::snapshot(&skills_dir);
        if is_tidy {
            // A tidy compacts notes too, so its changes to them are recorded and can be reverted.
            before.extend(own_skills::notes_snapshot(&notes_dir));
        }

        if lock(&self.0.active).get(&run.id).is_some_and(|a| a.cancelled) {
            return Err(AppError::conflict("cancelled before it started"));
        }
        let mut child = claude::spawn(&inv).map_err(|e| AppError::internal(format!("could not start Claude Code: {e}")))?;
        let pid = child.id();
        if let Some(pid) = pid {
            crate::power::stay_awake_while(pid);
        }
        let before_path = run_dir.join(BEFORE_SNAPSHOT);
        if let Err(e) = own_skills::save_snapshot(&before_path, &before) {
            log!("run {}: could not keep its skills snapshot: {e}", run.id);
        }
        let mut stdin = child.stdin.take().expect("piped");
        let (input, mut messages) = tokio::sync::mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = messages.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
            let _ = stdin.shutdown().await; // claude -p exits once its stdin is closed and its turn is done
        });
        let _ = input.send(claude::user_line(&inv.stdin));
        // Active holds the only sender, so stdin closes when the turn ends or the run is removed.
        let cancelled_early = {
            let mut active = lock(&self.0.active);
            active.get_mut(&run.id).map(|a| {
                a.pid = pid;
                a.input = Some(input);
                a.cancelled
            })
        };
        if let (Some(true), Some(pid)) = (cancelled_early, pid) {
            claude::kill_tree(pid);
        }

        let mut stderr = child.stderr.take().expect("piped");
        let stderr_path = run_dir.join("stderr.log");
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            let _ = fs::write(stderr_path, &buf);
            String::from_utf8_lossy(&buf).into_owned()
        });

        let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
        let mut pending: Vec<RunEvent> = vec![];
        let mut tasks = claude::TaskWatch::default();
        let mut signals = claude::RunSignals::default();
        let mut outcome: Option<Outcome> = None;
        let (mut limited, mut context, mut claude_version) = (None, None, None);
        let mut flush = tokio::time::interval(EVENT_BATCH);
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { break };
                    let parsed = claude::parse_line(&line, &snap.employee_dir);
                    signals.observe(&parsed);
                    limited = parsed.limited.or(limited);
                    context = parsed.context.or(context);
                    claude_version = parsed.claude_version.clone().or(claude_version);
                    if !parsed.partial {
                        append_line(&transcript, &line);
                    }
                    if parsed.events.iter().any(|e| matches!(e, RunEvent::Init { .. })) {
                        if let Err(e) = self.db().mark_inited(&run.id) {
                            log!("run {}: could not record that its session started, so the next turn may not resume it: {e}", run.id);
                        }
                    }
                    if let Some(o) = parsed.outcome {
                        // A message written before this point still runs, as one more turn of this run.
                        if let Some(a) = lock(&self.0.active).get_mut(&run.id) {
                            a.input = None;
                        }
                        outcome = Some(match outcome.take() {
                            Some(prev) => claude::merge_outcome(prev, o),
                            None => o,
                        });
                    }
                    if let Some(u) = parsed.usage {
                        self.observe_usage(u, false);
                    }
                    pending.extend(parsed.events.into_iter().filter(|e| tasks.keep(e)));
                }
                _ = flush.tick() => {
                    claude::coalesce_progress(&mut pending);
                    if !pending.is_empty() {
                        self.sink().run_events(&run.id, &std::mem::take(&mut pending));
                    }
                }
            }
        }
        let exit = child.wait().await.ok().and_then(|s| s.code());
        let stderr = stderr_task.await.unwrap_or_default();

        let mut after = own_skills::snapshot(&skills_dir);
        if is_tidy {
            after.extend(own_skills::notes_snapshot(&notes_dir));
        }
        if let RunKind::Tidy { reason, .. } = &run.kind {
            // What you taught is never lost to a tidy. An approved tidy may prune the rules its plan names.
            if *reason != TidyReason::Approved {
                let lost = tidy::dropped(&before, &after);
                let put_back = if lost.is_empty() { vec![] } else { tidy::restore(&skills_dir, &snap.employee.slug, &lost).unwrap_or_default() };
                if !put_back.is_empty() {
                    after = own_skills::snapshot(&skills_dir);
                    after.extend(own_skills::notes_snapshot(&notes_dir));
                    let quoted = put_back.iter().take(3).map(|t| format!("\"{}\"", claude::clip(t.line.trim_start_matches(['-', '*', ' ']), 80))).collect::<Vec<_>>().join("; ");
                    self.flag(&run.id, format!("The tidy dropped {} of the rules you taught; Omega Star put them back: {quoted}.", put_back.len()));
                }
            }
        }
        // Only an employee's first tidies are checked line by line, and never a plan you approved.
        let checked = matches!(&run.kind, RunKind::Tidy { reason, .. } if *reason != TidyReason::Approved) && self.earlier_tidies(run) < tidy::REVIEWED_FIRST;
        if checked {
            let gone = tidy::vanished(&before, &after);
            if !gone.is_empty() {
                let in_notes = gone.iter().filter(|(s, _, _)| s == own_skills::NOTES).count();
                let where_ = if in_notes > 0 { format!(" ({in_notes} in workspace notes)") } else { String::new() };
                self.flag(
                    &run.id,
                    format!("{} lines from before no longer appear anywhere{where_}. Check the run's changes for anything that mattered, and revert the run if it did.", gone.len()),
                );
            }
        }
        let changes = own_skills::diff(&before, &after);
        let _ = fs::remove_file(&before_path);
        for c in &changes {
            let ev = RunEvent::SkillChanged { skill: c.change.skill.clone(), file: c.change.file.clone(), change: c.change.change };
            append_line(&transcript, &claude::omega_line(&ev));
            pending.push(ev);
        }
        if !stderr.trim().is_empty() {
            let ev = RunEvent::Stderr { text: stderr.trim().to_string() };
            append_line(&transcript, &claude::omega_line(&ev));
            pending.push(ev);
        }
        claude::coalesce_progress(&mut pending);
        if !pending.is_empty() {
            self.sink().run_events(&run.id, &pending);
        }
        let rules = allowed_rules(&snap.employee, &run.allow_once);
        let denied = outcome.as_ref().map(|o| o.denials.clone()).unwrap_or_default();
        let (blocks, notes) = signals.sort_denials(denied, |d| covered(&snap.employee, &rules, d), !run.kind.is_conversation());
        Ok(Ended { outcome, exit, stderr, changes, limited, blocks, notes, context, claude_version })
    }
}

/// What a run's process left behind.
struct Ended {
    /// Every result segment of the run, merged.
    outcome: Option<Outcome>,
    exit: Option<i32>,
    stderr: String,
    changes: Vec<own_skills::FileChange>,
    /// The usage limit stopped it; it resets then.
    limited: Option<Ts>,
    /// The denials that really blocked it.
    blocks: Vec<Denial>,
    /// Lines for its report, such as a safety check that stopped a step.
    notes: Vec<String>,
    /// The context size after its last top-level message.
    context: Option<u64>,
    claude_version: Option<String>,
}

/// Whether a queued run may start now, by what holds it back.
#[derive(Debug, PartialEq)]
enum Gate {
    Claim,
    Skip,
}

/// The office-wide holds on a queued run, in order: the usage limit holds every run (a chat would fail
/// the same way); work you are not in waits for the right account; a tidy waits while its employee is
/// talking with you, so it never tidies a skill the conversation is still changing; and a dark wake
/// starts no work you are not in. A report coming back as a chat turn is not you: nobody may be
/// watching when it lands. Pure.
fn gate(kind: &RunKind, held: bool, account_ok: bool, talking: bool, awake: bool) -> Gate {
    let background = !matches!(kind, RunKind::Chat | RunKind::Task { .. } | RunKind::Reply { .. });
    if held || (background && !account_ok) || (matches!(kind, RunKind::Tidy { .. }) && talking) || (background && !awake) {
        Gate::Skip
    } else {
        Gate::Claim
    }
}

/// The usage limit stopped the run, and you did not. Pure.
fn hit_limit(cancelled: bool, ended: &Result<Ended>) -> bool {
    !cancelled && matches!(ended, Ok(Ended { outcome: Some(o), .. }) if !o.ok && o.limit)
}

/// A run the usage limit stopped goes back to the queue, up to three times. Pure.
fn requeue_for_limit(cancelled: bool, ended: &Result<Ended>, retries: u32) -> bool {
    retries < 3 && hit_limit(cancelled, ended)
}

/// A run back in the queue after the usage limit: as if it never started, keeping what it changed.
fn reset_for_requeue(run: &mut Run, changes: &[SkillChange]) {
    run.state = RunState::Queued;
    run.started_at = None;
    run.ended_at = None;
    run.error = None;
    run.summary = None;
    run.turns = None;
    run.duration_ms = None;
    run.denials.clear();
    run.limit_retries += 1;
    run.skill_changes.extend(changes.iter().cloned());
}

/// The open decision and item cards `newer` replaces, marked Replaced and saved: those of the same routine's
/// earlier runs, which also leave the Inbox unless you replied to them, and those earlier in its thread,
/// which stay. A due check's run replaces nothing of its routine. Call inside a transaction.
fn replace_older_cards_in(db: &Store, newer: &Run, at: Ts) -> Result<Vec<Run>> {
    let replaceable = |r: &Run| r.id != newer.id && r.card.as_ref().is_some_and(|c| c.is_open() && !matches!(c.ask, CardAsk::Chore { .. }));
    let mut out: Vec<Run> = vec![];
    if let (RunKind::Scheduled { schedule_id, .. }, None) = (&newer.kind, newer.kind.check_id()) {
        for mut r in db.open_cards_of_schedule(schedule_id, newer.created_at)?.into_iter().filter(|r| replaceable(r)) {
            if r.replied_at.is_none() {
                r.archived_at = Some(at);
            }
            out.push(r);
        }
    }
    let t = Office::thread_in(db, &newer.id)?;
    for r in std::iter::once(t.root).chain(t.replies) {
        if r.created_at < newer.created_at && replaceable(&r) && !out.iter().any(|o| o.id == r.id) {
            out.push(r);
        }
    }
    for r in &mut out {
        if let Some(c) = r.card.as_mut() {
            c.answer = Some(CardAnswer { at, how: AnswerHow::Replaced, text: String::new() });
        }
        db.save_run(r)?;
    }
    Ok(out)
}

/// An allow rule already covers the denied call, so the denial is not a block. Every tool is allowed in
/// bypassPermissions, so no rule helps an autonomous employee either.
fn covered(e: &Employee, rules: &[String], d: &Denial) -> bool {
    e.clearance == Clearance::Autonomous
        || rules.iter().any(|r| *r == d.suggested_rule || *r == d.tool || claude::mcp_server_rule(&d.tool).as_deref() == Some(r.as_str()))
}

/// Safe, distinct file names for attachments: the last path component, with characters Windows
/// forbids replaced, and repeats numbered (`image.png`, `image (2).png`).
fn attachment_names<'a>(raw: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for r in raw {
        let base = r.rsplit(['/', '\\']).next().unwrap_or("");
        let clean: String = base
            .chars()
            .map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c })
            .collect();
        let clean = clean.trim().trim_end_matches('.').trim();
        let clean: String = if clean.is_empty() || clean.chars().all(|c| c == '.') { "attachment".into() } else { clean.chars().take(120).collect() };
        let (stem, ext) = match clean.rfind('.') {
            Some(i) if i > 0 => (&clean[..i], &clean[i..]),
            _ => (clean.as_str(), ""),
        };
        let mut name = clean.clone();
        let mut n = 2;
        while out.iter().any(|o| o.eq_ignore_ascii_case(&name)) {
            name = format!("{stem} ({n}){ext}");
            n += 1;
        }
        out.push(name);
    }
    out
}

fn append_line(file: &Mutex<File>, line: &str) {
    let mut f = lock(file);
    let _ = f.write_all(line.as_bytes()).and_then(|_| f.write_all(b"\n"));
}

/// What an employee may do without asking: their own rules, Chrome when the browser switch is on, and
/// whatever you allowed for this one run.
fn allowed_rules(e: &Employee, once: &[String]) -> Vec<String> {
    let mut rules = e.allow_rules.clone();
    if e.browser {
        rules.push(claude::CHROME_RULE.to_string());
    }
    rules.extend(once.iter().cloned());
    let mut seen = HashSet::new();
    rules.retain(|r| seen.insert(r.clone()));
    rules
}

/// Decides how a run ended. Pure.
fn final_state(cancelled: bool, ended: &Result<Ended>) -> (RunState, Option<String>) {
    if cancelled {
        return (RunState::Cancelled, None);
    }
    let e = match ended {
        Err(e) => return (RunState::Failed, Some(e.message.clone())),
        Ok(e) => e,
    };
    match &e.outcome {
        Some(o) => state_of(o, stderr_tail(&e.stderr)),
        None => (
            RunState::Failed,
            Some(stderr_tail(&e.stderr).unwrap_or_else(|| format!("Claude Code exited without a result (exit code {:?}).", e.exit))),
        ),
    }
}

/// How a run that reached a result ended. `stderr_tail` explains a failure the result does not. Pure.
pub(super) fn state_of(o: &Outcome, stderr_tail: Option<String>) -> (RunState, Option<String>) {
    if o.ok {
        return (RunState::Succeeded, None);
    }
    let msg = match o.subtype.as_str() {
        "error_max_turns" => "Stopped: the run reached its turn limit.".to_string(),
        _ if !o.text.trim().is_empty() => o.text.trim().to_string(),
        _ => stderr_tail.unwrap_or_else(|| format!("Claude Code reported {}", o.subtype)),
    };
    (RunState::Failed, Some(msg))
}

/// The end of a run's stderr, at most 1500 characters; `None` when it said nothing.
pub(super) fn stderr_tail(stderr: &str) -> Option<String> {
    let t = stderr.trim();
    (!t.is_empty()).then(|| t.chars().rev().take(1500).collect::<Vec<_>>().into_iter().rev().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn employee(clearance: Clearance) -> Employee {
        Employee {
            id: uuid::Uuid::new_v4().to_string(),
            slug: "ada".into(),
            name: "Ada".into(),
            title: "Tester".into(),
            description: String::new(),
            objective: String::new(),
            manager_id: None,
            skills: SkillGrant { builtins: false, keys: vec![] },
            clearance,
            allow_rules: vec![],
            model: None,
            effort: None,
            browser: false,
            auto_tidy: false,
            workspaces: vec![],
            hue: 0,
            chat_session: "chat-ada".into(),
            hired_at: now(),
        }
    }

    fn outcome(ok: bool, subtype: &str, text: &str, limit: bool) -> Outcome {
        Outcome { ok, text: text.into(), subtype: subtype.into(), turns: 2, duration_ms: 10, denials: vec![], limit }
    }

    fn ended(o: Option<Outcome>, stderr: &str) -> Result<Ended> {
        Ok(Ended { outcome: o, exit: Some(1), stderr: stderr.into(), changes: vec![], limited: None, blocks: vec![], notes: vec![], context: None, claude_version: None })
    }

    #[test]
    fn the_gate_holds_runs_by_kind() {
        let scheduled = RunKind::Scheduled { schedule_id: "s1".into(), slot: now(), label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
        let tidy = RunKind::Tidy { reason: TidyReason::Auto, signature: vec![], plan: None };
        // gate(kind, held, account_ok, talking, awake)
        assert_eq!(gate(&RunKind::Chat, true, true, false, true), Gate::Skip);
        assert_eq!(gate(&RunKind::Chat, false, false, false, true), Gate::Claim);
        assert_eq!(gate(&scheduled, false, false, false, true), Gate::Skip);
        assert_eq!(gate(&scheduled, false, true, false, false), Gate::Skip);
        assert_eq!(gate(&scheduled, false, true, false, true), Gate::Claim);
        assert_eq!(gate(&RunKind::Chat, false, true, false, false), Gate::Claim);
        assert_eq!(gate(&tidy, false, true, true, true), Gate::Skip);
        assert_eq!(gate(&tidy, false, true, false, true), Gate::Claim);
        let came = RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat: true };
        assert_eq!(gate(&came, false, false, false, true), Gate::Skip);
        assert_eq!(gate(&came, false, true, false, false), Gate::Skip);
        assert_eq!(gate(&came, false, true, false, true), Gate::Claim);
    }

    #[test]
    fn a_limit_requeues_up_to_three_times() {
        let limited = ended(Some(outcome(false, "success", "You have hit your limit.", true)), "");
        for retries in 0..3 {
            assert!(requeue_for_limit(false, &limited, retries));
        }
        assert!(!requeue_for_limit(false, &limited, 3));
        assert!(!requeue_for_limit(true, &limited, 0));
        assert!(!requeue_for_limit(false, &ended(Some(outcome(true, "success", "Done.", false)), ""), 0));
        assert!(!requeue_for_limit(false, &Err(AppError::internal("could not start")), 0));
        assert!(hit_limit(false, &limited), "out of retries, it still holds the office");
        assert!(!hit_limit(true, &limited));
    }

    #[test]
    fn a_requeued_run_is_queued_with_its_changes() {
        let dir = std::env::temp_dir().join(format!("omegastar-requeue-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let e = employee(Clearance::Supervised);
        db.save_employee(&e).unwrap();
        let mut run = Run::new(&e.id, RunKind::Manual, "go".into(), now());
        assert!(db.insert_run(&run, &run.id).unwrap());
        run.state = RunState::Running;
        run.started_at = Some(now());
        run.summary = Some("You have hit your limit.".into());
        run.denials = vec![Denial { tool: "Bash".into(), brief: "ls".into(), suggested_rule: "Bash(ls:*)".into() }];
        db.save_run(&run).unwrap();

        let fc = own_skills::FileChange {
            change: SkillChange { skill: "ada-notes".into(), file: "SKILL.md".into(), change: ChangeKind::Modified },
            before: Some("old".into()),
            after: Some("new".into()),
        };
        reset_for_requeue(&mut run, std::slice::from_ref(&fc.change));
        db.tx(|| {
            db.insert_revision(&e.id, &own_skills::revision(fc, Some(run.id.clone()), now()))?;
            db.save_run(&run)
        })
        .unwrap();

        let saved = db.run(&run.id).unwrap().unwrap().run;
        assert_eq!((saved.state, saved.limit_retries, saved.started_at, saved.summary), (RunState::Queued, 1, None, None));
        assert!(saved.denials.is_empty());
        assert_eq!(saved.skill_changes.len(), 1);
        assert_eq!(db.revisions(&e.id).unwrap().iter().filter(|r| r.run_id.as_deref() == Some(run.id.as_str())).count(), 1);
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_routine_card_replaces_the_older_ones() {
        let dir = std::env::temp_dir().join(format!("omegastar-cards-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let e = employee(Clearance::Supervised);
        db.save_employee(&e).unwrap();
        let items = || CardAsk::Items { verb: "Post".into(), items: vec![CardItem { id: "d1".into(), text: "Draft one".into(), recommended: true }] };
        let card = |r: &mut Run, ask: CardAsk| r.card = Some(Card { message: "Post these?".into(), ask, answer: None });
        let scheduled = |schedule: &str, ago: i64, ask: CardAsk, replied: bool| {
            let at = now() - chrono::Duration::minutes(ago);
            let kind = RunKind::Scheduled { schedule_id: schedule.into(), slot: at, label: "Drafts".into(), quiet: true, model: None, effort: None, review: false };
            let mut r = Run::new(&e.id, kind, "go".into(), at);
            r.state = RunState::Succeeded;
            r.ended_at = Some(at);
            r.replied_at = replied.then_some(at);
            card(&mut r, ask);
            assert!(db.insert_run(&r, &r.id).unwrap());
            r
        };
        let first = scheduled("s1", 60, items(), false);
        let replied = scheduled("s1", 50, items(), true);
        let chore = scheduled("s1", 40, CardAsk::Chore { subject: "chrome".into() }, false);
        let newer = scheduled("s1", 10, items(), false);

        let replaced = db.tx(|| replace_older_cards_in(&db, &newer, now())).unwrap();
        assert_eq!(replaced.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![first.id.as_str(), replied.id.as_str()]);
        let saved = |r: &Run| db.run(&r.id).unwrap().unwrap().run;
        let how = |r: &Run| saved(r).card.unwrap().answer.map(|a| (a.how, a.text));
        assert_eq!(how(&first), Some((AnswerHow::Replaced, String::new())));
        assert!(saved(&first).archived_at.is_some());
        assert_eq!(how(&replied), Some((AnswerHow::Replaced, String::new())));
        assert!(saved(&replied).archived_at.is_none(), "you replied to it, so it stays");
        assert_eq!(how(&chore), None, "a chore is never replaced");
        assert_eq!(how(&newer), None);

        // A due check's runs share no routine.
        let check = scheduled("check:c1", 30, items(), false);
        let later = scheduled("check:c1", 5, items(), false);
        assert!(db.tx(|| replace_older_cards_in(&db, &later, now())).unwrap().is_empty());
        assert_eq!(how(&check), None);
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_matching_rule_covers_a_supervised_denial() {
        let d = Denial { tool: "mcp__claude-in-chrome__get_page_text".into(), brief: String::new(), suggested_rule: "mcp__claude-in-chrome__get_page_text".into() };
        assert!(covered(&employee(Clearance::Autonomous), &[], &d));
        let supervised = employee(Clearance::Supervised);
        assert!(!covered(&supervised, &["Bash(git:*)".into()], &d));
        assert!(covered(&supervised, &[claude::CHROME_RULE.into()], &d));
        assert!(covered(&supervised, &[d.tool.clone()], &d));
        let git = Denial { tool: "Bash".into(), brief: "git push".into(), suggested_rule: "Bash(git:*)".into() };
        assert!(covered(&supervised, &["Bash(git:*)".into()], &git));
        assert!(!covered(&supervised, &["Bash(ls:*)".into()], &git));
    }

    #[test]
    fn state_of_matches_how_runs_ended_before() {
        let failed = |m: &str| (RunState::Failed, Some(m.to_string()));
        let cases = [
            (outcome(true, "success", "Done.", false), "", (RunState::Succeeded, None)),
            (outcome(false, "error_max_turns", "", false), "", failed("Stopped: the run reached its turn limit.")),
            (outcome(false, "error_during_execution", "Something broke.", false), "noise", failed("Something broke.")),
            (outcome(false, "error_during_execution", "", false), "warning: one\nerror: the real cause\n", failed("warning: one\nerror: the real cause")),
            (outcome(false, "error_during_execution", "", false), "", failed("Claude Code reported error_during_execution")),
        ];
        for (o, stderr, want) in cases {
            assert_eq!(state_of(&o, stderr_tail(stderr)), want);
            assert_eq!(final_state(false, &ended(Some(o), stderr)), want);
        }
    }
}
