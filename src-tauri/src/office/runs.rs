//! The run queue: enqueueing, claiming, gathering a snapshot, executing and cancelling runs.

use super::*;
use crate::cards;

const EVENT_BATCH: Duration = Duration::from_millis(100);
/// Told to a run whose Chrome dropped, with `{ceo}` filled in by `chrome_note`.
const CHROME_NOTE: &str = "Omega Star: Chrome is not connected, so {ceo} has been asked to reconnect it. Do not ask {ceo} about it yourself. Carry on with what does not need Chrome. If the rest needs Chrome, finish with what you have and say what is left; you will be picked up again once it is back.";

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

    /// Starts a new conversation with the employee (a task under the hood): a thread of its own, so they
    /// work on it at the same time as everything else. Every next message in it is a reply (`reply_to_run`).
    pub fn start_task(&self, employee_id: &str, text: &str, attachments: Vec<NewAttachment>) -> Result<Run> {
        let text = text.trim();
        if text.is_empty() && attachments.is_empty() {
            return Err(AppError::invalid("message is empty"));
        }
        let files = Self::decode_attachments(&attachments)?;
        let mut run = {
            let e = self.db().employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            let names: Vec<String> = attachments.iter().map(|a| a.name.clone()).collect();
            Run::new(&e.id, RunKind::Task { label: threads::task_label(text, &names), from: None, context: String::new() }, text.to_string(), now())
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

    pub fn runs(&self, employee_id: Option<&str>, limit: u32) -> Result<Vec<Run>> {
        self.db().recent_runs(employee_id, limit.clamp(1, 1000))
    }

    /// The newest runs whose instruction, report or a card's question contains `query`, for the command palette.
    pub fn search_runs(&self, query: &str, limit: u32) -> Result<Vec<Run>> {
        self.db().search_runs(query, limit.clamp(1, 200))
    }

    /// What each employee's runs and each routine did and used since `since`, for Activity.
    pub fn run_usage(&self, since: Ts) -> Result<RunUsage> {
        let db = self.db();
        Ok(RunUsage { employees: db.employee_usage(since)?, routines: db.routine_counts(None, since)? })
    }

    pub fn run(&self, run_id: &str) -> Result<Run> {
        self.db().run(run_id)?.map(|r| r.run).ok_or_else(|| AppError::not_found("run not found"))
    }

    pub fn run_detail(&self, run_id: &str) -> Result<RunDetail> {
        let run = self.run(run_id)?;
        let dir = self.run_dir(run_id);
        let transcript = fs::read_to_string(dir.join("transcript.jsonl")).unwrap_or_default();
        let children = self.db().children(run_id)?;
        let events = claude::replay(&transcript, &self.employee_dir(&run.employee_id));
        // Every run that started has a transcript until the transcript sweep deletes it.
        let transcript_gone = run.state.is_finished() && run.started_at.is_some() && !dir.join("transcript.jsonl").exists();
        Ok(RunDetail { events, run, children, transcript_gone })
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
                // As a run's end does: an ask it brought is stamped, and reports an earlier attempt took in are held again.
                if let Err(e) = self.deliver_answers(&r) {
                    log!("delivering answers after cancelling run {} failed: {e}", r.id);
                }
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
        self.0.closing.store(true, Ordering::SeqCst);
        let pids: Vec<u32> = lock(&self.0.active).values_mut().filter_map(|a| {
            a.cancelled = true;
            a.pid
        }).collect();
        for pid in pids {
            claude::kill_tree(pid);
        }
        let _ = fs::remove_file(self.0.data_dir.join(RUNNING_MARKER));
    }

    /// Starts every queued run that is free to go, within the global cap (`cap`). A conversation takes turns (one
    /// run per session at a time), and an employee does as many pieces of background work at once as the cap, each
    /// in one of their lanes; a colleague's answer or question runs beside it (`fast_lane`), and so do their
    /// conversations with you, which never wait for the cap (`Run::uncapped`). A routine never runs twice at once,
    /// though, whatever the lanes. Delegated runs whose parent is running bypass the cap too, though not their
    /// employee's lanes: the parent is blocked on them, so capping them could deadlock. Nothing starts while the
    /// disk is nearly full.
    pub(super) fn pump(&self) {
        if self.0.closing.load(Ordering::SeqCst) || self.disk_full() {
            return;
        }
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
        let awake = !GATE_DARK_WAKE || self.0.awake.load(Ordering::Relaxed);
        let db = self.db();
        // One unit: a run marked running here must be returned, or it would never start.
        let claimed = db.tx(|| {
            let cap = cap(&db)?;
            let running = db.records_in_state(RunState::Running)?;
            let mut sessions: HashSet<String> = running.iter().map(|r| r.session.clone()).collect();
            // Background runs going per employee, each in one of their lanes.
            let mut working: HashMap<String, usize> = HashMap::new();
            for r in running.iter().filter(|r| takes_a_lane(&r.run.kind)) {
                *working.entry(r.run.employee_id.clone()).or_default() += 1;
            }
            let running_ids: HashSet<String> = running.iter().map(|r| r.run.id.clone()).collect();
            // Employees with anything going, which a tidy waits out, and the routines going.
            let mut busy: HashSet<String> = running.iter().map(|r| r.run.employee_id.clone()).collect();
            let mut routines: HashSet<String> = running.iter().filter_map(|r| routine_of(&r.run.kind)).collect();
            let mut capped = running.iter().filter(|r| !r.run.uncapped()).count();
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
                // Not due yet: a paused hand-off, an answer queued again, or reports held for a manager.
                if r.not_before.is_some_and(|t| t > at) {
                    continue;
                }
                if gate(&r, held, account_ok, busy.contains(&r.employee_id), awake) == Gate::Skip {
                    continue;
                }
                let lane = takes_a_lane(&r.kind);
                let free = working.get(&r.employee_id).copied().unwrap_or(0) < cap;
                let routine = routine_of(&r.kind);
                if sessions.contains(&session) || (lane && !free) || routine.as_ref().is_some_and(|s| routines.contains(s)) {
                    continue;
                }
                let exempt = r.uncapped() || r.parent_run_id().map(|p| running_ids.contains(p)).unwrap_or(false);
                if capped >= cap && !exempt {
                    // Its session's later turns wait behind it, so a reply typed after a tap never overtakes the tap.
                    sessions.insert(session);
                    continue;
                }
                r.state = RunState::Running;
                r.started_at = Some(now());
                r.build = Some(stamp.clone());
                db.save_run(&r)?;
                sessions.insert(session);
                if lane {
                    *working.entry(r.employee_id.clone()).or_default() += 1;
                }
                busy.insert(r.employee_id.clone());
                routines.extend(routine);
                if !r.uncapped() {
                    capped += 1;
                }
                started.push(r);
            }
            Ok((started, dropped))
        })?;
        // Registered while the store is still locked, so a cancel that sees a run as running finds it. A card
        // raised before a usage limit requeued the run is still its one card.
        let mut active = lock(&self.0.active);
        for r in &claimed.0 {
            let token = uuid::Uuid::new_v4().simple().to_string();
            let mut beside = false;
            for a in active.values_mut().filter(|a| a.employee_id == r.employee_id) {
                a.beside = true;
                beside = true;
            }
            active.insert(
                r.id.clone(),
                Active {
                    token,
                    employee_id: r.employee_id.clone(),
                    beside,
                    pid: None,
                    cancelled: false,
                    needs_you: vec![],
                    cards: r.cards.clone(),
                    score: r.score.clone(),
                    proposed: None,
                    verdict: false,
                    title: None,
                    transcript: None,
                    input: None,
                    waiting: vec![],
                },
            );
        }
        Ok(claimed)
    }

    /// The office's disk has less than `MIN_FREE_BYTES` free, so no run starts: one that fills it fails, and so does
    /// what it was doing. Tells you once when it fills, and logs when there is room again; the next tick starts work.
    fn disk_full(&self) -> bool {
        let full = too_full(free_bytes(&self.0.data_dir));
        if self.0.disk_full.swap(full, Ordering::SeqCst) != full {
            if full {
                log!("paused work: less than 2 GB is free on the office's disk");
                self.sink().notify("Omega Star paused work: your disk is nearly full", "Work starts again by itself once more than 2 GB is free.");
            } else {
                log!("resumed work: more than 2 GB is free on the office's disk again");
            }
        }
        full
    }

    /// Sends your message into a run that is still working: the employee takes it at their next step, as Claude Code
    /// takes a message typed while it works, and `RunEvent::Heard` marks when. `false` when the run no longer takes
    /// messages (its turn is over, or it has not started).
    pub(super) fn steer(&self, run_id: &str, text: &str) -> bool {
        self.steer_as(run_id, text, text)
    }

    /// `steer`, where the employee hears `said` and the run's events keep your own words, `text`.
    fn steer_as(&self, run_id: &str, text: &str, said: &str) -> bool {
        let id = uuid::Uuid::new_v4().to_string();
        let ev = RunEvent::User { text: text.to_string(), id: Some(id.clone()) };
        {
            let mut active = lock(&self.0.active);
            let Some(a) = active.get_mut(run_id) else { return false };
            if !a.input.as_ref().is_some_and(|i| i.send(claude::steer_line(said, &id)).is_ok()) {
                return false;
            }
            a.waiting.push(id);
            if let Some(f) = &a.transcript {
                append_line(f, &claude::omega_line(&ev));
            }
        }
        self.sink().run_events(run_id, &[ev]);
        true
    }

    /// Writes into a run's open stdin without recording it as your message: for notes Omega Star writes,
    /// such as Chrome dropping. `false` when the run no longer takes messages.
    pub(super) fn tell(&self, run_id: &str, text: &str) -> bool {
        let active = lock(&self.0.active);
        active.get(run_id).and_then(|a| a.input.as_ref()).is_some_and(|i| i.send(claude::user_line(text)).is_ok())
    }

    /// Chrome dropped in a run: you get the one Chrome chore card (or the run joins the open one), and the run
    /// hears that you were asked, so it carries on without Chrome instead of asking you itself. Not in a turn of
    /// an older build's chat: a card there never reaches you, so the run tells you itself.
    pub(super) fn chrome_down(&self, run: &Run, e: &Employee) {
        if run.kind.lane() == Lane::Chat {
            return;
        }
        let why = format!("{} is waiting. Open Chrome, check the Claude extension is on and signed in, then press Done.", e.name);
        if let Err(err) = self.raise_chore(&run.id, cards::CHROME, "Reconnect Claude in Chrome", &why) {
            log!("run {}: asking you to reconnect Chrome failed: {err}", run.id);
        }
        let ceo = ceo_of(&self.db()).unwrap_or_else(|err| {
            log!("run {}: could not read the company for the Chrome note: {err}", run.id);
            "the person you work for".into()
        });
        self.tell(&run.id, &chrome_note(&ceo));
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
        let manager_is_agent = e.manager_id.as_ref().is_some_and(|m| employees.iter().any(|x| &x.id == m));
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
        let schedules = db.schedules(&e.id)?;
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
        let mut opening = opening(&db, &e, &employees, run, &session, resume, now())?;
        if let Some(o) = opening.as_mut() {
            o.waiting_on = db
                .requests(&RequestFilter { employee_id: Some(e.id.clone()), state: Some(RequestState::Open), ..Default::default() })?
                .into_iter()
                .map(|r| {
                    let n = person_name(&r.person_id);
                    (r, n)
                })
                .collect();
        }
        if let Some(o) = opening.as_mut() {
            tools::catch_up(&db, &employees, run, o)?;
        }
        let chain = match (&run.kind, run.upstream()) {
            (RunKind::Delegated { .. } | RunKind::Asked { .. } | RunKind::Answered { .. }, Some(up)) => Some(tools::source_chain(&db, &company.ceo_name, up)?),
            _ => None,
        };
        drop(db);
        let employee_dir = self.employee_dir(&e.id);
        if let Some(o) = opening.as_mut() {
            o.notes = own_skills::changed_notes(&employee_dir.join("workspace"), o.since.unwrap_or(o.now - chrono::Duration::days(7)));
        }
        let source = chain.map(|c| tools::source_text(&c, &self.0.data_dir.join("runs")));
        let owns = fs::read_to_string(employee_dir.join(own_skills::OWN_FILE)).map(|t| t.trim().to_string()).unwrap_or_default();
        let tidy = self.tidy_brief(run, &e)?;
        let catalog = self.skill_catalog();
        let own = own_skills::own_skill_names(&self.skills_dir(&e.id));
        let granted = catalog.iter().filter(|s| e.skills.keys.contains(&s.id)).cloned().collect();
        Ok(claude::Snapshot {
            granted,
            claude: claude_path,
            settings: catalog::render_settings(&e.skills, &catalog, &own, &allowed_rules(&e, &run.allow_once)),
            employee_dir,
            run_dir: self.run_dir(&run.id),
            mcp_url: format!("http://127.0.0.1:{}/mcp/{token}", self.0.mcp_port),
            proxy_env: settings.as_ref().map(|s| s.proxy.env()).unwrap_or_default(),
            company,
            employee: e,
            manager,
            boss,
            reports,
            colleagues,
            people,
            counterpart,
            schedules,
            opening,
            owns,
            source,
            manager_is_agent,
            tidy,
            replier,
            watched,
            resume,
            session,
        })
    }

    async fn execute(self, mut run: Run) {
        let ended = self.drive(&run).await;
        // It never started, so the news its opening note took in reaches the next run instead.
        if ended.is_err() {
            if let Err(e) = self.db().unread_news_of(&run.id) {
                log!("run {}: could not give back the news it never showed: {e}", run.id);
            }
        }
        // Quit killed it: saved as cancelled it would report back and never carry on, so the next start settles it.
        if self.0.closing.load(Ordering::SeqCst) {
            return;
        }
        let (cancelled, needs_you, cards, score, proposed, verdict, title) =
            lock(&self.0.active).remove(&run.id).map(|a| (a.cancelled, a.needs_you, a.cards, a.score, a.proposed, a.verdict, a.title)).unwrap_or_default();
        if let Some(stored) = self.db().run(&run.id).ok().flatten() {
            keep_yours(&mut run, &stored.run);
        }
        run.needs_you.extend(needs_you);
        // Before the limit requeue, so cards raised before the limit stopped the run survive it. The live run
        // started with the run's own cards, so its list holds them all.
        if !cards.is_empty() {
            run.cards = cards;
        }
        if score.is_some() {
            run.score = score;
        }
        if title.is_some() {
            run.title = title;
        }
        // Added to what an attempt the usage limit stopped proposed, so nothing it proposed is lost.
        if let Some(p) = proposed.filter(|p| !p.trim().is_empty()) {
            run.proposed = Some(match run.proposed.take() {
                Some(earlier) => format!("{earlier}\n\n{p}"),
                None => p,
            });
        }
        // Before the limit requeue, so what every attempt cost adds up.
        if let Ok(e) = &ended {
            run.cost_usd = [run.cost_usd, e.cost].into_iter().flatten().reduce(|a, b| a + b);
        }
        // Held before the run is saved back to the queue, so no pump claims it straight into the limit,
        // and held too when it is out of retries, so other queued runs do not start into it.
        let limit = hit_limit(cancelled, &ended);
        match limit {
            Some(Limit::Usage) => {
                let reset = ended.as_ref().ok().and_then(|e| e.limited);
                self.hold_until(reset.unwrap_or_else(|| now() + chrono::Duration::minutes(15)), Limit::Usage);
            }
            Some(Limit::Transient) => self.hold_for_transient_limit(),
            None => {}
        }
        if let Some(limit) = requeue_for_limit(limit, &run) {
            if let Ok(e) = ended {
                self.requeue_after_limit(run, e, limit);
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
        // A rate limit that outlasts the retries breaks the run as any failure does, so it reaches you.
        run.limited = limit == Some(Limit::Usage);
        if state == RunState::Succeeded && self.0.transient_limits.swap(0, Ordering::SeqCst) != 0 {
            self.save_rate_limits();
        }
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
            run.skill_changes.extend(own_changes(&e.changes));
        }
        if matches!(run.kind, RunKind::Tidy { .. }) && run.state == RunState::Succeeded {
            self.review_tidy(&mut run);
            // What it left, so a file that grows well past it starts the next tidy early (`tidy::due`).
            let slug = self.db().employee(&run.employee_id).ok().flatten().map(|e| e.slug).unwrap_or_default();
            let dir = self.employee_dir(&run.employee_id);
            if let RunKind::Tidy { signature, .. } = &mut run.kind {
                tidy::note_sizes(signature, &dir, &slug);
            }
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
                    record_changes(&db, &run.employee_id, &run.id, run.started_at.unwrap_or(run.created_at), e.changes)?;
                }
                db.save_run(&run)?;
                db.employee(&run.employee_id)
            })
        };
        match saved {
            Ok(e) if matches!(run.state, RunState::Failed | RunState::Lost) => log_failed(&run, e.as_ref().map_or(&run.employee_id, |e| &e.slug), limit),
            Ok(_) => {}
            Err(e) => log!("saving run {} failed: {e}", run.id),
        }
        if run.open_cards().any(|c| !matches!(c.ask, CardAsk::Chore { .. })) {
            if let Err(e) = self.replace_older_cards(&run) {
                log!("replacing the cards before run {} failed: {e}", run.id);
            }
        }
        if let RunKind::Scheduled { schedule_id, label, .. } = &run.kind {
            if threads::broke(&run) {
                // A routine that keeps breaking is exactly what needs you: say so once, as it reaches the
                // Inbox. A failure from the usage limit is Omega Star's to wait out, never yours.
                if !run.limited && self.failures_in_a_row(schedule_id, &run) == threads::STREAK {
                    let name = self.db().employee(&run.employee_id).ok().flatten().map(|e| e.name).unwrap_or_default();
                    self.sink().notify(&format!("{name}: '{label}' keeps failing"), run.error.as_deref().unwrap_or("Open Omega Star for details."));
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

    /// Puts a run a limit stopped back in the queue with what it changed so far. `execute` has already held
    /// the office until the limit resets or the rate limit has had time to ease.
    fn requeue_after_limit(&self, mut run: Run, e: Ended, limit: Limit) {
        // Recorded from the attempt that saw them, which `reset_for_requeue` forgets.
        let since = run.started_at.unwrap_or(run.created_at);
        reset_for_requeue(&mut run, &own_changes(&e.changes).collect::<Vec<_>>(), limit);
        // The next attempt's lines follow this one's, which ended in the limit.
        run.attempt_from = fs::metadata(self.run_dir(&run.id).join("transcript.jsonl")).map_or(0, |m| m.len());
        let saved = {
            let db = self.db();
            db.tx(|| {
                record_changes(&db, &run.employee_id, &run.id, since, e.changes)?;
                db.save_run(&run)?;
                let session = db.run(&run.id)?.map(|r| r.session).unwrap_or_else(|| run.id.clone());
                db.session_started(&session)
            })
        };
        let attempt = if limit == Limit::Transient { run.transient_retries } else { run.limit_retries.saturating_sub(run.transient_retries) };
        match saved {
            Ok(resumes) => log!("requeued {} after {} (attempt {attempt}, resumes {resumes})", run.id, limit.as_str()),
            Err(err) => log!("requeueing run {} after {} failed: {err}", run.id, limit.as_str()),
        }
        self.sink().run_updated(&run);
        self.sink().org_changed();
    }

    /// Holds the office for a rate limit: 5 minutes for the first since a run last succeeded, then longer each
    /// time (`backoff`). A run that meets one while the office is already held waits with the rest.
    fn hold_for_transient_limit(&self) {
        let at = now();
        if self.usage_limits().held_until.is_some_and(|t| t > at) {
            return;
        }
        let n = self.0.transient_limits.fetch_add(1, Ordering::SeqCst);
        self.hold_until(at + backoff(n), Limit::Transient);
    }

    /// How many of a schedule's runs up to `last` broke for a reason you can act on since it last worked.
    fn failures_in_a_row(&self, schedule_id: &str, last: &Run) -> usize {
        let db = self.db();
        let since = db.successes_of_schedule(schedule_id).unwrap_or_default().into_iter().filter(|t| *t < last.created_at).max();
        db.failures_of_schedule(schedule_id)
            .unwrap_or_default()
            .iter()
            .filter(|f| f.created_at <= last.created_at && since.is_none_or(|s| f.created_at > s) && !f.limited)
            .count()
    }

    /// A schedule that works again closes its run of failures: the ones since its last success before
    /// `ok` move to the Archive, unless you replied to one or wrote into it.
    fn archive_streak(&self, schedule_id: &str, ok: &Run) -> Result<()> {
        let cleared = {
            let db = self.db();
            db.tx(|| {
                // The store keeps milliseconds, so compare in them: else the success that ends the streak counts as its own last one.
                let since = db.successes_of_schedule(schedule_id)?.into_iter().filter(|t| t.timestamp_millis() < ok.created_at.timestamp_millis()).max();
                let mut cleared = vec![];
                for mut f in db.failures_of_schedule(schedule_id)? {
                    let in_streak = f.created_at < ok.created_at && since.is_none_or(|s| f.created_at > s);
                    if in_streak && f.archived_at.is_none() && f.replied_at.is_none() && !f.steered && !f.has_open_card() {
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
    /// A run left with nothing open hears the answers you gave its other cards, as a tap would have sent them.
    fn replace_older_cards(&self, newer: &Run) -> Result<()> {
        let replaced = {
            let db = self.db();
            db.tx(|| {
                let mut out = vec![];
                for mut r in replace_older_cards_in(&db, newer, now())? {
                    let said = super::inbox::take_held(&mut r);
                    if said.is_some() {
                        db.save_run(&r)?;
                    }
                    out.push((r, said));
                }
                Ok(out)
            })?
        };
        let (sends, rest): (Vec<_>, Vec<_>) = replaced.into_iter().partition(|(_, s)| s.is_some());
        for (r, _) in &rest {
            self.sink().run_updated(r);
        }
        self.send_card_answers(&sends);
        Ok(())
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
        // A run requeued after the usage limit appends to the same transcript; this attempt's lines start here.
        let attempt_start = transcript.metadata().map(|m| m.len()).unwrap_or(0);
        let transcript = Arc::new(Mutex::new(transcript));
        let token = {
            let mut active = lock(&self.0.active);
            let a = active.get_mut(&run.id).ok_or_else(|| AppError::internal("the run was not claimed"))?;
            a.transcript = Some(transcript.clone());
            a.token.clone()
        };

        let snap = self.gather(run, &token)?;
        let inv = claude::prepare(&snap, run);
        // Behind a CLI proxy a run reports only the pool account it used: the proxy reports the pool.
        let proxied = !snap.proxy_env.is_empty();
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
        let guarded_before = own_skills::guarded_snapshot(&snap.employee_dir);

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
        // Your messages an earlier attempt never took, as Omega Star stopped before its next step, go in again now.
        let unheard = if attempt_start > 0 { claude::unheard(&fs::read_to_string(run_dir.join("transcript.jsonl")).unwrap_or_default()) } else { vec![] };
        for (id, text) in &unheard {
            let _ = input.send(claude::steer_line(&steer_words(run, &snap.company.ceo_name, text), id));
        }
        // Active holds the only sender, so stdin closes when the turn ends or the run is removed.
        let cancelled_early = {
            let mut active = lock(&self.0.active);
            active.get_mut(&run.id).map(|a| {
                a.pid = pid;
                a.input = Some(input);
                a.waiting.extend(unheard.into_iter().map(|(id, _)| id));
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
        let mut spent: Vec<claude::Spend> = vec![];
        let (mut limited, mut context, mut claude_version) = (None, None, None);
        let mut flush = tokio::time::interval(EVENT_BATCH);
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { break };
                    let parsed = claude::parse_line(&line, &snap.employee_dir);
                    signals.observe(&parsed);
                    if signals.chrome_down(&parsed.events, now()) {
                        self.chrome_down(run, &snap.employee);
                    }
                    limited = parsed.limited.or(limited);
                    context = parsed.context.or(context);
                    claude_version = parsed.claude_version.clone().or(claude_version);
                    // Its echo of a message it took from stdin is not kept: one of yours is marked heard where it went in.
                    if let Some(id) = &parsed.replayed {
                        let yours = lock(&self.0.active).get_mut(&run.id).is_some_and(|a| {
                            let before = a.waiting.len();
                            a.waiting.retain(|w| w != id);
                            a.waiting.len() < before
                        });
                        if yours {
                            let ev = RunEvent::Heard { id: id.clone() };
                            append_line(&transcript, &claude::omega_line(&ev));
                            pending.push(ev);
                        }
                    } else if !parsed.partial {
                        append_line(&transcript, &claude::without_images(&line));
                    }
                    if parsed.events.iter().any(|e| matches!(e, RunEvent::Init { .. })) {
                        if let Err(e) = self.db().mark_inited(&run.id) {
                            log!("run {}: could not record that its session started, so the next turn may not resume it: {e}", run.id);
                        }
                    }
                    spent.extend(parsed.spend);
                    if let Some(mut o) = parsed.outcome {
                        // A message written before this point still runs, as one more turn of this run.
                        if let Some(a) = lock(&self.0.active).get_mut(&run.id) {
                            a.input = None;
                        }
                        o.limit = signals.limit(&o);
                        outcome = Some(match outcome.take() {
                            Some(prev) => claude::merge_outcome(prev, o),
                            None => o,
                        });
                    }
                    if let Some(u) = parsed.usage.filter(|_| !proxied) {
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
        // A resumed session's running total starts from its earlier turns, which only matters when a result
        // reports no cost of its own.
        let before_cost = match spent.first() {
            _ if !snap.resume => Some(0.0),
            Some(s) if s.own.is_none() => self.session_total_before(run, &snap.session, &run_dir.join("transcript.jsonl"), attempt_start),
            _ => None,
        };
        let cost = claude::run_cost(&spent, before_cost);

        let mut after = own_skills::snapshot(&skills_dir);
        if is_tidy {
            after.extend(own_skills::notes_snapshot(&notes_dir));
        }
        // What Omega Star did that the report should say, after the employee's own words.
        let mut omega_notes: Vec<String> = vec![];
        if snap.employee.browser && signals.chrome_missing() {
            log!("run {}: Claude in Chrome did not load, so the run had none of its tools", run.id);
            omega_notes.push("Claude in Chrome did not load in this run, so it worked without your browser.".into());
        }
        omega_notes.extend(claude::stopped_helpers(&stderr));
        if let RunKind::Tidy { reason, .. } = &run.kind {
            // What you taught is never lost to a tidy. An approved tidy may prune the rules its plan names. It is
            // already put back, so it is a line in the Archive rather than something for you to do.
            if *reason != TidyReason::Approved {
                let lost = tidy::dropped(&before, &after);
                let put_back = if lost.is_empty() { vec![] } else { tidy::restore(&skills_dir, &snap.employee.slug, &lost).unwrap_or_default() };
                if !put_back.is_empty() {
                    after = own_skills::snapshot(&skills_dir);
                    after.extend(own_skills::notes_snapshot(&notes_dir));
                    let n = put_back.len();
                    omega_notes.push(format!("Omega Star put back {n} {} you taught that the tidy dropped.", if n == 1 { "rule" } else { "rules" }));
                }
            }
        }
        let changes = own_skills::diff(&before, &after);
        // Only a conversation with you may change the company page or what the employee owns; any other run
        // that wrote one has it put back, or only flagged when you changed it too, and never records it. Decided
        // by what this attempt's own calls wrote, so a change made elsewhere meanwhile is never recorded as its own.
        let mut guarded_after = own_skills::guarded_snapshot(&snap.employee_dir);
        let mut transcript_text: Option<String> = None;
        let mut recorded: Vec<(String, String)> = vec![];
        let mut put_back = false;
        for (skill, file) in [(own_skills::COMPANY, own_skills::COMPANY_FILE), (own_skills::OWN, own_skills::OWN_FILE)] {
            let key = (skill.to_string(), file.to_string());
            let (was, is) = (guarded_before.get(&key), guarded_after.get(&key));
            let Ok(path) = own_skills::recorded_path(&snap.employee_dir, skill, file) else { continue };
            if was == is {
                continue;
            }
            let text = transcript_text.get_or_insert_with(|| attempt_text(&run_dir.join("transcript.jsonl"), attempt_start));
            let wrote = skills::wrote_file(text, &path, &snap.employee_dir);
            if !skills::ceo_present(&run) {
                let who = (skill == own_skills::OWN).then_some(snap.employee.id.as_str());
                let since = run.started_at.unwrap_or(run.created_at);
                let seen = {
                    let db = self.db();
                    db.ceo_conversation_running(who).and_then(|talking| Ok((talking, db.revised_since(skill, file, who, since)?)))
                };
                // When the store cannot tell, nothing is put back: that could lose your own edit.
                let (talking, revised) = seen.unwrap_or_else(|err| {
                    log!("run {}: could not tell whether you were changing {skill}/{file}: {err}", run.id);
                    (true, true)
                });
                // Saving the page in You writes no revision, so it is asked about here.
                let saved = skill == own_skills::COMPANY && self.company_saved_since(since);
                let action = skills::guarded_action(wrote, talking, revised || saved);
                let putting_back = action == skills::GuardedAction::PutBack;
                match self.put_back_guarded(run, &snap.employee, &key, was, is, action) {
                    Ok(()) => put_back |= putting_back,
                    Err(err) => log!("run {}: putting back {skill}/{file} failed: {err}", run.id),
                }
                // A flagged change is not recorded either: undoing it would write over your own edit.
                continue;
            }
            if wrote {
                recorded.push(key);
            }
        }
        if put_back {
            guarded_after = own_skills::guarded_snapshot(&snap.employee_dir);
        }
        // A run working alone made every skill edit it saw. Beside another run of the employee, an edit is its own only
        // when its own calls show it: a write of the file, or Omega Star's tools moving or retiring the skill.
        let alone = !lock(&self.0.active).get(&run.id).is_some_and(|a| a.beside);
        let mut moved: Option<HashSet<String>> = None;
        let mut changes: Vec<Seen> = changes
            .into_iter()
            .map(|c| {
                let wrote = !alone && {
                    let text = transcript_text.get_or_insert_with(|| attempt_text(&run_dir.join("transcript.jsonl"), attempt_start));
                    moved.get_or_insert_with(|| moved_skills(text)).contains(&c.change.skill)
                        || own_skills::recorded_path(&snap.employee_dir, &c.change.skill, &c.change.file).is_ok_and(|path| skills::wrote_file(text, &path, &snap.employee_dir))
                };
                Seen { own: alone || wrote, wrote, change: c }
            })
            .collect();
        changes.extend(
            own_skills::diff(&guarded_before, &guarded_after)
                .into_iter()
                .filter(|c| recorded.iter().any(|(s, f)| *s == c.change.skill && *f == c.change.file))
                .map(|c| Seen { own: true, wrote: true, change: c }),
        );
        // Which of its skills it used, so a skill no run uses shows up (`health`). Looking after them is not using them.
        if !matches!(run.kind, RunKind::Tidy { .. } | RunKind::Organize { .. }) {
            let text = transcript_text.get_or_insert_with(|| attempt_text(&run_dir.join("transcript.jsonl"), attempt_start));
            let own = own_skills::own_skill_names(&skills_dir);
            let used: HashSet<String> = skills_used(text).into_iter().filter(|s| own.contains(s)).collect();
            if let Err(e) = self.db().record_skill_use(&run.employee_id, &used, now()) {
                log!("run {}: could not record the skills it used: {e}", run.id);
            }
        }
        let _ = fs::remove_file(&before_path);
        for c in own_changes(&changes) {
            let ev = RunEvent::SkillChanged { skill: c.skill, file: c.file, change: c.change };
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
        let (blocks, mut notes) = signals.sort_denials(denied, |d| covered(&snap.employee, &rules, d), !run.kind.is_conversation());
        notes.extend(omega_notes);
        Ok(Ended { outcome, exit, stderr, changes, limited, blocks, notes, context, claude_version, cost })
    }

    /// The session's running cost total before this attempt, as Claude Code last reported it: in an earlier attempt
    /// of the run, or else in the session's latest earlier turn that reported one. `None` when none did.
    fn session_total_before(&self, run: &Run, session: &str, transcript: &Path, attempt_start: u64) -> Option<f64> {
        let earlier = fs::read(transcript).unwrap_or_default();
        let earlier = String::from_utf8_lossy(earlier.get(..attempt_start as usize).unwrap_or_default()).into_owned();
        if let Some(total) = claude::last_total(&earlier) {
            return Some(total);
        }
        let turns = self.db().turns_of_session(session, &run.id).ok()?;
        turns.iter().find_map(|r| claude::last_total(&fs::read_to_string(self.run_dir(&r.id).join("transcript.jsonl")).ok()?))
    }

    /// Takes the image data out of finished runs' transcripts, which older builds saved whole: one file at a time,
    /// pausing between them. A file is only swapped for its stripped copy, so stopping halfway loses nothing, and a
    /// file already stripped is left as it is. Returns the bytes saved.
    pub(super) fn compact_transcripts(&self) -> u64 {
        let mut saved = 0;
        for dir in fs::read_dir(self.0.data_dir.join("runs")).into_iter().flatten().flatten() {
            let id = dir.file_name().to_string_lossy().into_owned();
            if !self.db().run(&id).ok().flatten().is_some_and(|r| r.run.state.is_finished()) {
                continue;
            }
            match compact_transcript(&dir.path().join("transcript.jsonl")) {
                Ok(n) => saved += n,
                Err(e) => log!("run {id}: could not take the images out of its transcript: {e}"),
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        saved
    }
}

/// Rewrites a transcript without its image data (`claude::without_images`) through a copy beside it, swapped in
/// only when something changed and the transcript did not grow meanwhile. Returns the bytes saved.
fn compact_transcript(path: &Path) -> std::io::Result<u64> {
    use std::io::BufRead;
    let Ok(file) = File::open(path) else { return Ok(0) };
    let len = file.metadata()?.len();
    let tmp = path.with_extension("jsonl.compacting");
    let copied = (|| -> std::io::Result<bool> {
        let (mut reader, mut out) = (std::io::BufReader::new(file), std::io::BufWriter::new(File::create(&tmp)?));
        let (mut line, mut changed) = (Vec::new(), false);
        while reader.read_until(b'\n', &mut line)? > 0 {
            let body = line.strip_suffix(b"\n").unwrap_or(&line);
            match std::str::from_utf8(body).ok().map(claude::without_images) {
                Some(std::borrow::Cow::Owned(stripped)) => {
                    changed = true;
                    out.write_all(stripped.as_bytes())?;
                    out.write_all(&line[body.len()..])?;
                }
                _ => out.write_all(&line)?,
            }
            line.clear();
        }
        out.flush()?;
        Ok(changed)
    })();
    // Gone meanwhile, or grown: the copy is left out rather than swapped in.
    let unchanged = fs::metadata(path).is_ok_and(|m| m.len() == len);
    match copied {
        Ok(true) if unchanged => {
            let size = fs::metadata(&tmp)?.len();
            fs::rename(&tmp, path)?;
            Ok(len.saturating_sub(size))
        }
        other => {
            let _ = fs::remove_file(&tmp);
            other.map(|_| 0)
        }
    }
}

/// The transcript from byte `start` on: what one attempt of a run wrote. Empty when it cannot be read.
fn attempt_text(path: &Path, start: u64) -> String {
    let bytes = fs::read(path).unwrap_or_default();
    String::from_utf8_lossy(bytes.get(start as usize..).unwrap_or_default()).into_owned()
}

/// A skill change a run saw between its snapshots, which may be another run's.
pub(super) struct Seen {
    pub(super) change: own_skills::FileChange,
    /// The run's own calls show it made the change, so it takes the credit for it even when another run
    /// recorded the change first.
    pub(super) wrote: bool,
    /// It is the run's own: its calls show it, or nothing else of its employee ran beside it.
    pub(super) own: bool,
}

/// The skills a run's own calls moved or retired with Omega Star's tools, which write no file themselves.
pub(super) fn moved_skills(transcript: &str) -> HashSet<String> {
    let moves = claude::office_calls(transcript, "mcp__omega__move_skill").into_iter().flat_map(|c| [c["from"].clone(), c["to"].clone()]);
    let retires = claude::office_calls(transcript, "mcp__omega__retire_skill").into_iter().map(|c| c["name"].clone());
    moves.chain(retires).filter_map(|n| n.as_str().map(String::from)).collect()
}

/// The skills a run used, its subagents included: invoked with the Skill tool, or a `SKILL.md` read in a skills folder.
pub(super) fn skills_used(transcript: &str) -> HashSet<String> {
    let mut used = HashSet::new();
    let lines = transcript.lines().filter(|l| l.contains("\"tool_use\"")).filter_map(|l| serde_json::from_str::<Value>(l).ok());
    for v in lines.filter(|v| v["type"] == "assistant") {
        for call in v["message"]["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_use") {
            used.extend(match call["name"].as_str() {
                Some("Skill") => call["input"]["skill"].as_str().map(String::from),
                Some("Read") => call["input"]["file_path"].as_str().and_then(skill_read),
                _ => None,
            });
        }
    }
    used
}

/// The skill whose `SKILL.md` is at `path`, when it is one in a skills folder.
fn skill_read(path: &str) -> Option<String> {
    let path = Path::new(path);
    let skill = path.parent()?;
    if path.file_name()? != "SKILL.md" || skill.parent()?.file_name()? != "skills" {
        return None;
    }
    skill.file_name()?.to_str().map(String::from)
}

/// The changes that are the run's own, as its report lists them.
pub(super) fn own_changes(changes: &[Seen]) -> impl Iterator<Item = SkillChange> + '_ {
    changes.iter().filter(|c| c.own).map(|c| c.change.change.clone())
}

/// Records the skill changes a run saw since `since`, when it started. One already recorded since then is the same
/// edit seen by another run, so it is never recorded twice: a run whose own calls wrote it only takes the credit.
/// Any other is recorded once, as the run's when it is its own, else with no run. Call inside a transaction.
pub(super) fn record_changes(db: &Store, employee_id: &str, run_id: &str, since: Ts, changes: Vec<Seen>) -> Result<()> {
    for c in changes {
        let fc = &c.change;
        match db.same_revision(employee_id, &fc.change, fc.before.as_deref(), fc.after.as_deref(), since)? {
            Some(r) if c.wrote && r.run_id.is_none() => db.credit_revision(&r.id, run_id)?,
            Some(_) => {}
            None => db.insert_revision(employee_id, &own_skills::revision(c.change, c.own.then(|| run_id.to_string()), now()))?,
        }
    }
    Ok(())
}

/// What a run's process left behind.
struct Ended {
    /// Every result segment of the run, merged.
    outcome: Option<Outcome>,
    exit: Option<i32>,
    stderr: String,
    /// The skill changes it saw, its own and others'.
    changes: Vec<Seen>,
    /// The usage limit stopped it; it resets then.
    limited: Option<Ts>,
    /// The denials that really blocked it.
    blocks: Vec<Denial>,
    /// Lines for its report, such as a safety check that stopped a step.
    notes: Vec<String>,
    /// The context size after its last top-level message.
    context: Option<u64>,
    claude_version: Option<String>,
    /// What this attempt cost, by Claude Code's reckoning.
    cost: Option<f64>,
}

/// What you did to a run while it worked, after the copy its end holds was taken: you wrote into it, or replied in
/// its thread (both `reply_to_run`). Kept when its end saves that copy over `stored`. Pure.
fn keep_yours(run: &mut Run, stored: &Run) {
    run.steered |= stored.steered;
    run.replied_at = run.replied_at.max(stored.replied_at);
}

/// A colleague's answer coming back, and a colleague's question (not a task): short runs that never wait behind
/// their employee's other background work, though they still keep to the cap and to their session. Pure.
pub(super) fn fast_lane(kind: &RunKind) -> bool {
    matches!(kind, RunKind::Answered { .. } | RunKind::Asked { task: false, .. })
}

/// Runs at the same time, which is also how many lanes each employee has for background work.
pub(super) fn cap(db: &Store) -> Result<usize> {
    Ok(db.settings()?.unwrap_or_default().max_concurrent_runs.max(1) as usize)
}

/// Background work that takes one of its employee's lanes (`cap`). Pure.
pub(super) fn takes_a_lane(kind: &RunKind) -> bool {
    !kind.is_conversation() && !fast_lane(kind)
}

/// The routine a run is a slot of: its schedule, or a due check's own run's check, so it never runs twice at once.
/// None for anything else. Pure.
fn routine_of(kind: &RunKind) -> Option<String> {
    match kind {
        RunKind::Scheduled { schedule_id, .. } => Some(schedule_id.clone()),
        _ => None,
    }
}

/// Whether a queued run may start now, by what holds it back.
#[derive(Debug, PartialEq)]
enum Gate {
    Claim,
    Skip,
}

/// The office-wide holds on a queued run, in order: the usage limit holds every run (a conversation would
/// fail the same way); work you are not in waits for the right account; a tidy waits while anything else of its
/// employee's runs (`busy`), so it never tidies a skill a conversation, an answer or the other lane is still using
/// or changing; and a dark wake starts no work you are not in. An answer coming back into a conversation is not
/// you: nobody may be watching when it lands. Pure.
/// Off until omega.log shows its "dark wake" lines only during real dark wakes (`pmset -g log`), never
/// while the display merely sleeps in a full wake: a wrong reading would hold every routine overnight.
const GATE_DARK_WAKE: bool = false;

fn gate(run: &Run, held: bool, account_ok: bool, busy: bool, awake: bool) -> Gate {
    let background = !skills::ceo_present(run);
    if held || (background && !account_ok) || (matches!(run.kind, RunKind::Tidy { .. }) && busy) || (background && !awake) {
        Gate::Skip
    } else {
        Gate::Claim
    }
}

/// The limit that stopped the run, unless you did. A limit with a known reset time is the usage limit, however
/// the result names it: older Claude Code builds report the usage limit as a bare 429. Pure.
fn hit_limit(cancelled: bool, ended: &Result<Ended>) -> Option<Limit> {
    match ended {
        Ok(Ended { outcome: Some(o), limited, .. }) if !cancelled && !o.ok => match o.limit? {
            Limit::Transient if limited.is_some() => Some(Limit::Usage),
            limit => Some(limit),
        },
        _ => None,
    }
}

/// Times a run goes back to the queue for the usage limit, and, counted apart, for a rate limit.
const USAGE_RETRIES: u32 = 3;
const TRANSIENT_RETRIES: u32 = 8;

/// The limit a run goes back to the queue for, while it has retries left for that kind. Pure.
fn requeue_for_limit(limit: Option<Limit>, run: &Run) -> Option<Limit> {
    limit.filter(|l| match l {
        Limit::Usage => run.limit_retries.saturating_sub(run.transient_retries) < USAGE_RETRIES,
        Limit::Transient => run.transient_retries < TRANSIENT_RETRIES,
    })
}

/// How long the office waits out its `n`th rate limit in a row, from 0: 5, 10, 20 and 40 minutes, then 60. Pure.
fn backoff(n: u32) -> chrono::Duration {
    chrono::Duration::minutes([5, 10, 20, 40, 60][n.min(4) as usize])
}

/// A run back in the queue after a limit: as if it never started, keeping what it changed and what it cost.
fn reset_for_requeue(run: &mut Run, changes: &[SkillChange], limit: Limit) {
    run.state = RunState::Queued;
    run.started_at = None;
    run.ended_at = None;
    run.error = None;
    run.summary = None;
    run.turns = None;
    run.duration_ms = None;
    run.denials.clear();
    run.limit_retries += 1;
    if limit == Limit::Transient {
        run.transient_retries += 1;
    }
    run.restarted = false;
    run.skill_changes.extend(changes.iter().cloned());
}

/// Writes a failed or lost run to omega.log: its id, its employee, its kind and what broke it.
pub(super) fn log_failed(run: &Run, slug: &str, limit: Option<Limit>) {
    let kind = serde_json::to_value(&run.kind).ok().and_then(|k| k["type"].as_str().map(String::from)).unwrap_or_default();
    log!("run {} {} ({slug}, {kind}): {}", run.id, run.state.as_str(), error_class(run, limit));
}

/// What broke a failed or lost run, in a few words. Pure.
fn error_class(run: &Run, limit: Option<Limit>) -> &'static str {
    let error = run.error.as_deref().unwrap_or("");
    if run.state == RunState::Lost {
        "lost when Omega Star stopped"
    } else if run.limited || limit == Some(Limit::Usage) {
        "usage limit"
    } else if limit == Some(Limit::Transient) {
        "rate limit"
    } else if error.contains("No space left on device") {
        "disk full"
    } else if error.starts_with("could not start Claude Code") || error.starts_with("Claude Code was not found") {
        "could not start"
    } else if error.starts_with("Stopped: the run reached its turn limit") {
        "turn limit"
    } else if error.starts_with("Claude Code exited without a result") {
        "no result"
    } else {
        "error"
    }
}

/// The decision and item cards still waiting on you that `newer` replaces, put off with Later too, marked Replaced and
/// saved, so none comes back: those of the same routine's earlier runs, which also leave the Inbox unless you replied
/// to them, wrote into them or have not seen their news, or they still wait in the queue; and the employee's earlier
/// cards about the same thing as one of its own, in its thread or anywhere else. A card about something else stays
/// open, so answers that come back into one conversation leave each other's cards alone. A due check's run replaces
/// nothing of its routine. Call inside a transaction.
fn replace_older_cards_in(db: &Store, newer: &Run, at: Ts) -> Result<Vec<Run>> {
    let decision = |c: &Card| (c.is_open() || c.put_off()) && !matches!(c.ask, CardAsk::Chore { .. });
    // What still needs you, or has yet to run, stays where it is.
    let files = |r: &Run| r.replied_at.is_none() && !r.steered && !r.has_news() && r.state.is_finished();
    let mut out: Vec<(Run, bool)> = vec![];
    if let (RunKind::Scheduled { schedule_id, .. }, None) = (&newer.kind, newer.kind.check_id()) {
        for mut r in db.open_cards_of_schedule(schedule_id, newer.created_at)?.into_iter().filter(|r| r.id != newer.id && r.cards.iter().any(decision)) {
            if files(&r) {
                r.archived_at = Some(at);
            }
            out.push((r, true));
        }
    }
    let topics: Vec<String> = newer.open_cards().filter(|c| decision(c)).filter_map(Card::topic).collect();
    for topic in &topics {
        for r in db.open_topic_runs(topic)? {
            if r.employee_id == newer.employee_id && r.created_at < newer.created_at && r.id != newer.id && !out.iter().any(|(o, _)| o.id == r.id) {
                out.push((r, false));
            }
        }
    }
    // Its own thread stays whatever it replaced: the newer card is asked there.
    let t = Office::thread_in(db, &newer.id)?;
    let thread: HashSet<String> = std::iter::once(t.root).chain(t.replies).map(|r| r.id).collect();
    for (r, all) in &mut out {
        for c in r.cards.iter_mut().filter(|c| decision(c) && (*all || c.topic().is_some_and(|k| topics.contains(&k)))) {
            c.answer = Some(CardAnswer { at, how: AnswerHow::Replaced, text: String::new(), held: false });
            (c.back_at, c.parked) = (None, false);
        }
        // A run whose cards all went to a newer one has nothing left to ask, unless you replied to it or wrote into it.
        if !*all && !r.has_open_card() && files(r) && !thread.contains(&r.id) {
            r.archived_at = Some(at);
        }
        db.save_run(r)?;
    }
    Ok(out.into_iter().map(|(r, _)| r).collect())
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

fn chrome_note(ceo: &str) -> String {
    CHROME_NOTE.replace("{ceo}", ceo)
}

/// What an employee at work on `run` hears of your words: a conversation with you needs nobody named. Pure.
fn steer_words(run: &Run, ceo: &str, text: &str) -> String {
    if run.kind.is_conversation() { text.to_string() } else { format!("{ceo} is watching this run and says: {text}") }
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

/// What a run's opening note is made from, read under the store lock: `waiting_on` and `notes` are
/// left for the caller. `None` for reply checks, nudges and Organize runs. A resumed conversation, and a
/// quiet routine, get only what is new since their last turn.
fn opening(db: &Store, e: &Employee, employees: &[Employee], run: &Run, session: &str, resume: bool, now: Ts) -> Result<Option<claude::Opening>> {
    if matches!(run.kind, RunKind::ReplyCheck { .. } | RunKind::Nudge { .. } | RunKind::Organize { .. }) {
        return Ok(None);
    }
    let (day, week) = (chrono::Duration::days(1), chrono::Duration::days(7));
    let since = if resume {
        db.last_turn_ended(session, &run.id)?
    } else if let RunKind::Scheduled { schedule_id, quiet: true, .. } = &run.kind {
        db.previous_finished(schedule_id, run.created_at)?.and_then(|r| r.ended_at)
    } else {
        None
    };
    let window_start = since.unwrap_or(now - week);
    let finished_after = |employee_id: &str, start: Ts| -> Result<Vec<RunRecord>> {
        Ok(db
            .finished_since(employee_id, start - day)?
            .into_iter()
            .filter(|r| r.run.ended_at.is_some_and(|t| t > start))
            .filter(|r| !matches!(r.run.kind, RunKind::Tidy { .. } | RunKind::ReplyCheck { .. } | RunKind::Nudge { .. }))
            .collect())
    };
    let mut work: Vec<Run> =
        finished_after(&e.id, window_start)?.into_iter().filter(|r| r.run.id != run.id && r.session != session).map(|r| r.run).collect();
    work.sort_by(|a, b| b.ended_at.cmp(&a.ended_at));

    let mut team = vec![];
    let mut team_since = window_start;
    let reports: Vec<&Employee> = employees.iter().filter(|x| x.manager_id.as_deref() == Some(e.id.as_str())).collect();
    if !reports.is_empty() {
        // From the last run that was not a quiet routine, so a frequent routine does not swallow the lines.
        team_since = since
            .or_else(|| work.iter().find(|r| !matches!(r.kind, RunKind::Scheduled { quiet: true, .. })).map(|r| r.started_at.unwrap_or(r.created_at)))
            .unwrap_or(now - week);
        for x in reports {
            for r in finished_after(&x.id, team_since)? {
                // Their conversations with you are between you and them.
                if r.run.kind.is_conversation() {
                    continue;
                }
                // Work this employee handed over comes back to them on its own.
                if let Some(parent) = r.run.parent_run_id() {
                    if db.run(parent)?.is_some_and(|p| p.run.employee_id == e.id) {
                        continue;
                    }
                }
                team.push((x.name.clone(), r.run));
            }
        }
        team.sort_by(|a, b| b.1.ended_at.cmp(&a.1.ended_at));
    }

    let cards = db
        .open_cards(&e.id)?
        .into_iter()
        .flat_map(|r| {
            let at = r.ended_at.unwrap_or(r.created_at);
            r.cards.into_iter().filter(Card::is_open).map(move |c| (at, c)).collect::<Vec<_>>()
        })
        .collect();
    Ok(Some(claude::Opening {
        now,
        since,
        checks: db.checks(&e.id, Some(CheckState::Open))?,
        cards,
        waiting_on: vec![],
        proposals: db.proposals(Some(&e.id), Some(ProposalState::Open))?,
        work,
        team,
        running: db.active_runs_of(&e.id)?.into_iter().filter(|r| r.state == RunState::Running && r.id != run.id).collect(),
        notes: vec![],
        parents: db.parents_since(window_start.min(team_since) - day)?,
        news: vec![],
        held: vec![],
        replaced: vec![],
        answers: db.answered_cards(&e.id, now - week, session)?,
    }))
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
            image: None,
            chat_session: "chat-ada".into(),
            hired_at: now(),
        }
    }

    fn outcome(ok: bool, subtype: &str, text: &str, limit: Option<Limit>) -> Outcome {
        Outcome { ok, text: text.into(), subtype: subtype.into(), turns: 2, duration_ms: 10, denials: vec![], limit, notice: false }
    }

    fn ended(o: Option<Outcome>, stderr: &str) -> Result<Ended> {
        Ok(Ended { outcome: o, exit: Some(1), stderr: stderr.into(), changes: vec![], limited: None, blocks: vec![], notes: vec![], context: None, claude_version: None, cost: None })
    }

    /// A temp store in its own folder.
    fn opening_store() -> (PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!("omegastar-opening-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Store::open(&dir.join("office.db")).unwrap();
        (dir, db)
    }

    /// A finished run of `e`'s that ended `mins` minutes ago, saved in `session`.
    fn finished(db: &Store, e: &Employee, kind: RunKind, mins: i64, session: &str, summary: &str) -> Run {
        let at = now() - chrono::Duration::minutes(mins);
        let mut r = Run::new(&e.id, kind, format!("Prompt {mins}."), at - chrono::Duration::minutes(1));
        r.state = RunState::Succeeded;
        r.started_at = Some(r.created_at);
        r.ended_at = Some(at);
        r.summary = Some(summary.into());
        assert!(db.insert_run(&r, session).unwrap());
        r
    }

    fn note(e: &Employee, o: claude::Opening, run: &Run) -> String {
        let company = Company { name: "Acme".into(), ceo_name: "Pat".into(), ..Company::unset() };
        claude::first_message(&claude::test_snapshot(company, e.clone(), Some(o), false), run)
    }

    fn task(label: &str) -> RunKind {
        RunKind::Task { label: label.into(), from: None, context: String::new() }
    }

    struct NoSink;
    impl EventSink for NoSink {
        fn org_changed(&self) {}
        fn run_updated(&self, _run: &Run) {}
        fn run_events(&self, _run_id: &str, _events: &[RunEvent]) {}
        fn request_updated(&self, _request: &HumanRequest) {}
    }

    #[tokio::test]
    async fn a_newer_card_sends_the_answers_its_run_held() {
        let dir = std::env::temp_dir().join(format!("omegastar-replace-held-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let ada = employee(Clearance::Supervised);
        let card = |q: &str, key: &str| Card::new(q, None, Some(key.into()), CardAsk::Choices { choices: vec!["Go".into(), "Not now".into()], recommended: 0 });
        let (root, newer) = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&ada).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            let mut root = finished(&db, &ada, task("Plan the launch"), 30, "s1", "Two things.");
            root.cards = vec![card("Launch on Friday?", "ask:launch"), card("Email the list?", "ask:email")];
            root.cards[1].answer = Some(CardAnswer { at: now(), how: AnswerHow::Tapped, text: "Chose: Go.".into(), held: true });
            db.save_run(&root).unwrap();
            let mut newer = finished(&db, &ada, RunKind::Reply { to_run_id: root.id.clone(), label: "Plan the launch".into(), task: true, card: false }, 5, "s1", "One more.");
            newer.cards = vec![card("Launch on Monday instead?", "ask:launch")];
            db.save_run(&newer).unwrap();
            (root, newer)
        };
        let office = Office::open(dir.clone(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        office.replace_older_cards(&newer).unwrap();
        let t = office.thread(&root.id).unwrap();
        assert_eq!(t.root.cards[0].answer.as_ref().map(|a| a.how), Some(AnswerHow::Replaced));
        assert!(t.root.cards[1].answer.as_ref().is_some_and(|a| !a.held));
        let sent: Vec<&str> = t.replies.iter().map(|r| r.prompt.as_str()).filter(|p| p.ends_with("Your card: \"Email the list?\"\nChose: Go.")).collect();
        assert_eq!(sent.len(), 1, "{:?}", t.replies.iter().map(|r| &r.prompt).collect::<Vec<_>>());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    /// An office for Pat at Acme whose queue stays held, so nothing a test queues starts Claude Code. `runs` are
    /// saved once it is open, so one saved as working is not taken for one a restart cut off.
    async fn held_office(dir: &Path, employees: &[&Employee], runs: &[&Run]) -> Office {
        fs::create_dir_all(dir).unwrap();
        {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.set_company(&Company { name: "Acme".into(), ceo_name: "Pat".into(), ..Company::unset() }).unwrap();
            for e in employees {
                db.save_employee(e).unwrap();
            }
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
        }
        let office = Office::open(dir.to_path_buf(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        for r in runs {
            assert!(office.db().insert_run(r, &r.id).unwrap());
        }
        office
    }

    #[tokio::test]
    async fn a_routine_run_you_wrote_into_waits_for_you_when_a_later_one_moves_on() {
        let dir = std::env::temp_dir().join(format!("omegastar-steered-routine-{}", uuid::Uuid::new_v4()));
        let ada = employee(Clearance::Supervised);
        let routine = |mins: i64, state: RunState, steered: bool| {
            // To the millisecond, as the store keeps when a run was made.
            let at = Ts::from_timestamp_millis((now() - chrono::Duration::minutes(mins)).timestamp_millis()).unwrap();
            let kind = RunKind::Scheduled { schedule_id: "s1".into(), slot: at, label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
            let mut r = Run::new(&ada.id, kind, "Sweep the inbox.".into(), at);
            r.state = state;
            r.ended_at = Some(at);
            r.steered = steered;
            r
        };
        let carded = |mut r: Run| {
            r.cards = vec![Card::new("Post the draft?", None, Some("ask:draft".into()), CardAsk::Choices { choices: vec!["Post".into()], recommended: 0 })];
            r
        };
        let (asked, watched_asked) = (carded(routine(50, RunState::Succeeded, false)), carded(routine(45, RunState::Succeeded, true)));
        let (failed, watched_failed) = (routine(20, RunState::Failed, false), routine(15, RunState::Failed, true));
        let newer = carded(routine(5, RunState::Succeeded, false));
        let office = held_office(&dir, &[&ada], &[&asked, &watched_asked, &failed, &watched_failed, &newer]).await;
        office.replace_older_cards(&newer).unwrap();
        office.archive_streak("s1", &newer).unwrap();
        let archived = |r: &Run| office.run(&r.id).unwrap().archived_at.is_some();
        assert!(archived(&asked) && archived(&failed), "the newer card and the run that worked again file these");
        assert!(!archived(&watched_asked) && !archived(&watched_failed), "what you wrote into stays until you mark it done");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_turn_held_by_the_cap_keeps_its_place_in_its_session() {
        let dir = std::env::temp_dir().join(format!("omegastar-cap-order-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let ada = employee(Clearance::Supervised);
        {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&ada).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
        }
        let office = Office::open(dir.clone(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        {
            let db = office.db();
            for i in 0..3 {
                let mut busy = Run::new(&ada.id, RunKind::Manual, format!("Sweep {i}."), now());
                busy.state = RunState::Running;
                assert!(db.insert_run(&busy, &busy.id).unwrap());
            }
            let root = finished(&db, &ada, task("Book the venue"), 10, "s1", "Which room?");
            let reply = |text: &str, card: bool| {
                let mut r = Run::new(&ada.id, RunKind::Reply { to_run_id: root.id.clone(), label: "Book the venue".into(), task: true, card }, text.into(), now());
                r.omega_wrote = card;
                assert!(db.insert_run(&r, "s1").unwrap());
                r
            };
            reply("Chose: The big room.", true);
            reply("Thanks, book it.", false);
        }
        lock(&office.0.usage).held_until = None;
        let (started, _) = office.claim_runnable().unwrap();
        assert!(started.is_empty(), "the typed reply waits for the tap before it: {started:?}");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    /// An office whose employees are saved and whose queue is held at open, so a test claims runs itself.
    async fn claiming_office(dir: &Path, employees: &[&Employee]) -> Office {
        fs::create_dir_all(dir).unwrap();
        {
            let db = Store::open(&dir.join("office.db")).unwrap();
            for e in employees {
                db.save_employee(e).unwrap();
            }
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
        }
        let office = Office::open(dir.to_path_buf(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        lock(&office.0.usage).held_until = None;
        // Background work waits for the first account check.
        *lock(&office.0.account) = Some((now(), None));
        office
    }

    /// Queues a run of `e`'s in `session` (its own when `None`), `state` for one already going.
    fn queue(office: &Office, e: &Employee, kind: RunKind, session: Option<&str>, state: RunState) -> Run {
        let mut r = Run::new(&e.id, kind, "Go.".into(), now());
        r.state = state;
        assert!(office.db().insert_run(&r, session.unwrap_or(&r.id)).unwrap());
        r
    }

    fn scheduled(slot: i64) -> RunKind {
        RunKind::Scheduled { schedule_id: "s1".into(), slot: now() + chrono::Duration::minutes(slot), label: "Sweep".into(), quiet: true, model: None, effort: None, review: false }
    }

    /// Sets how many runs go at the same time.
    fn set_cap(office: &Office, cap: u32) {
        let mut s = office.settings().unwrap();
        s.max_concurrent_runs = cap;
        office.db().set_settings(&s).unwrap();
    }

    #[tokio::test]
    async fn a_colleagues_question_never_waits_for_a_lane() {
        let dir = std::env::temp_dir().join(format!("omegastar-fast-lane-{}", uuid::Uuid::new_v4()));
        let (ada, bo) = (employee(Clearance::Supervised), Employee { id: uuid::Uuid::new_v4().to_string(), slug: "bo".into(), ..employee(Clearance::Supervised) });
        let office = claiming_office(&dir, &[&ada, &bo]).await;
        set_cap(&office, 1);
        // Bo waits on what they handed Ada, so it skips the cap, but it fills Ada's one lane.
        let parent = queue(&office, &bo, RunKind::Manual, None, RunState::Running);
        queue(&office, &ada, RunKind::Delegated { parent_run_id: parent.id.clone() }, None, RunState::Running);
        let asked = |task: bool| RunKind::Asked { parent_run_id: parent.id.clone(), asked_by: bo.id.clone(), task };
        let sweep = queue(&office, &ada, scheduled(0), None, RunState::Queued);
        let task = queue(&office, &ada, asked(true), None, RunState::Queued);
        let question = queue(&office, &ada, asked(false), None, RunState::Queued);
        let (started, _) = office.claim_runnable().unwrap();
        assert_eq!(started.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![question.id.as_str()]);
        assert_eq!(office.run(&sweep.id).unwrap().state, RunState::Queued, "the cap is full");
        assert_eq!(office.run(&task.id).unwrap().state, RunState::Queued, "a task waits for Ada's lane");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_paused_handoff_waits_for_its_time() {
        let dir = std::env::temp_dir().join(format!("omegastar-not-before-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let ada = employee(Clearance::Supervised);
        let named = |name: &str| Employee { name: name.into(), slug: name.to_lowercase(), chat_session: format!("chat-{name}"), ..employee(Clearance::Supervised) };
        let (bo, cy) = (named("Bo"), named("Cy"));
        let (paused, due) = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            for e in [&ada, &bo, &cy] {
                db.save_employee(e).unwrap();
            }
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            let asker = finished(&db, &ada, RunKind::Manual, 30, "m1", "Asked around.");
            let ask = |e: &Employee, mins: i64| {
                let mut r = Run::new(&e.id, RunKind::Asked { parent_run_id: asker.id.clone(), asked_by: ada.id.clone(), task: false }, "Which room?".into(), now());
                r.not_before = Some(now() + chrono::Duration::minutes(mins));
                assert!(db.insert_run(&r, &r.id).unwrap());
                r
            };
            (ask(&bo, 60), ask(&cy, -1))
        };
        let office = Office::open(dir.clone(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        lock(&office.0.usage).held_until = None;
        *lock(&office.0.account) = Some((now(), None));
        let (started, _) = office.claim_runnable().unwrap();
        let ids: Vec<&str> = started.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec![due.id.as_str()], "{paused:?} waits for its time");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_employee_does_as_much_background_work_at_once_as_the_cap_allows() {
        let dir = std::env::temp_dir().join(format!("omegastar-lanes-{}", uuid::Uuid::new_v4()));
        let ada = employee(Clearance::Supervised);
        let office = claiming_office(&dir, &[&ada]).await;
        set_cap(&office, 3);
        queue(&office, &ada, RunKind::Manual, None, RunState::Running);
        let work: Vec<Run> = (0..3).map(|_| queue(&office, &ada, RunKind::Manual, None, RunState::Queued)).collect();
        let (started, _) = office.claim_runnable().unwrap();
        assert_eq!(started.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![work[0].id.as_str(), work[1].id.as_str()]);
        assert_eq!(office.run(&work[2].id).unwrap().state, RunState::Queued, "three runs are going");
        set_cap(&office, 4);
        let (started, _) = office.claim_runnable().unwrap();
        assert_eq!(started.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![work[2].id.as_str()], "a fourth lane opens with the cap");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_routine_never_runs_twice_at_once_and_a_tidy_waits_for_quiet() {
        let dir = std::env::temp_dir().join(format!("omegastar-one-routine-{}", uuid::Uuid::new_v4()));
        let ada = employee(Clearance::Supervised);
        let bo = Employee { id: uuid::Uuid::new_v4().to_string(), slug: "bo".into(), chat_session: "chat-bo".into(), ..employee(Clearance::Supervised) };
        let office = claiming_office(&dir, &[&ada, &bo]).await;
        let tidy = || RunKind::Tidy { reason: TidyReason::Auto, signature: vec![], plan: None };
        let routine = |id: &str, slot: i64| RunKind::Scheduled { schedule_id: id.into(), slot: now() + chrono::Duration::minutes(slot), label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
        queue(&office, &ada, routine("s1", 0), None, RunState::Running);
        let again = queue(&office, &ada, routine("s1", 1), None, RunState::Queued);
        let other = queue(&office, &ada, routine("s2", 2), None, RunState::Queued);
        // Bo has only a colleague's answer going, beside which a tidy would once have started.
        let answer = RunKind::Answered { asked_run_id: "a1".into(), from: ada.id.clone(), task: false, chat: false, thread: None };
        queue(&office, &bo, answer, None, RunState::Running);
        let tidying = queue(&office, &bo, tidy(), None, RunState::Queued);
        let (started, _) = office.claim_runnable().unwrap();
        assert_eq!(started.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![other.id.as_str()], "{started:?}");
        assert_eq!(office.run(&again.id).unwrap().state, RunState::Queued, "another lane never runs the same routine twice at once");
        assert_eq!(office.run(&tidying.id).unwrap().state, RunState::Queued, "a tidy waits until nothing else of Bo's runs");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_runs_end_keeps_your_reply_and_your_words_from_while_it_worked() {
        let at = now();
        let taken = Run::new("e1", RunKind::Manual, "Sweep.".into(), at - chrono::Duration::minutes(5));
        let stored = Run { replied_at: Some(at), steered: true, ..taken.clone() };
        let mut ended = taken.clone();
        keep_yours(&mut ended, &stored);
        assert_eq!((ended.replied_at, ended.steered), (Some(at), true));
        let mut later = Run { replied_at: Some(at + chrono::Duration::minutes(1)), ..taken };
        keep_yours(&mut later, &stored);
        assert_eq!(later.replied_at, Some(at + chrono::Duration::minutes(1)), "the newer reply");
    }

    #[tokio::test]
    async fn finished_transcripts_lose_their_images_once() {
        let dir = std::env::temp_dir().join(format!("omegastar-compact-{}", uuid::Uuid::new_v4()));
        let ada = employee(Clearance::Supervised);
        let office = claiming_office(&dir, &[&ada]).await;
        let done = queue(&office, &ada, RunKind::Manual, None, RunState::Succeeded);
        let working = queue(&office, &ada, RunKind::Manual, None, RunState::Running);
        let data = "iVBORw0KGgo".repeat(400);
        let image = json!({
            "type": "user", "parent_tool_use_id": null,
            "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "toolu_img1", "content": [
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": data } },
            ] }] },
        });
        let said = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","content":[{"type":"text","text":"Made the chart."}]},"parent_tool_use_id":null}"#;
        let old = format!("{said}\n{image}\n{said}\n");
        let transcript = |r: &Run| office.run_dir(&r.id).join("transcript.jsonl");
        for r in [&done, &working] {
            fs::create_dir_all(office.run_dir(&r.id)).unwrap();
            fs::write(transcript(r), &old).unwrap();
        }
        assert!(office.compact_transcripts() > 3000);
        let now_text = fs::read_to_string(transcript(&done)).unwrap();
        assert!(!now_text.contains(&data) && now_text.ends_with('\n') && now_text.lines().count() == 3, "{now_text}");
        let home = office.employee_dir(&ada.id);
        assert_eq!(claude::replay(&now_text, &home), claude::replay(&old, &home), "the app reads the same");
        assert_eq!(fs::read_to_string(transcript(&working)).unwrap(), old, "a run still working is left alone");
        assert_eq!(office.compact_transcripts(), 0, "safe to repeat");
        assert!(!office.run_dir(&done.id).join("transcript.jsonl.compacting").exists());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skills_moved_with_omega_stars_tools_are_the_runs_own() {
        let call = |id: &str, tool: &str, input: Value| json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "content": [{ "type": "tool_use", "id": id, "name": tool, "input": input }] } });
        let result = |id: &str, error: bool| json!({ "type": "user", "parent_tool_use_id": null, "message": { "content": [{ "type": "tool_result", "tool_use_id": id, "is_error": error, "content": "ok" }] } });
        let transcript = [
            call("t1", "mcp__omega__move_skill", json!({ "from": "imported-notes", "to": "ada-notes" })),
            result("t1", false),
            call("t2", "mcp__omega__retire_skill", json!({ "name": "ada-old-sweep" })),
            result("t2", false),
            call("t3", "mcp__omega__retire_skill", json!({ "name": "ada-kept" })),
            result("t3", true),
        ]
        .map(|l| l.to_string())
        .join("\n");
        let moved = moved_skills(&transcript);
        assert_eq!(moved, HashSet::from(["imported-notes".to_string(), "ada-notes".to_string(), "ada-old-sweep".to_string()]));
    }

    #[test]
    fn a_run_uses_the_skills_it_invokes_or_reads() {
        let call = |id: &str, tool: &str, input: Value| json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "content": [{ "type": "tool_use", "id": id, "name": tool, "input": input }] } });
        let transcript = [
            call("t1", "Skill", json!({ "skill": "ada" })),
            call("t2", "Read", json!({ "file_path": "/office/employees/e1/skills/ada-invoices/SKILL.md" })),
            call("t3", "Read", json!({ "file_path": "/office/employees/e1/.claude/skills/ada-payroll/SKILL.md" })),
            call("t4", "Read", json!({ "file_path": "/office/employees/e1/skills/ada-close/facts.md" })),
            call("t5", "Read", json!({ "file_path": "/office/employees/e1/workspace/SKILL.md" })),
            json!({ "type": "assistant", "parent_tool_use_id": "t0", "message": { "content": [{ "type": "tool_use", "id": "t6", "name": "Read", "input": { "file_path": "/office/employees/e1/skills/ada-voice/SKILL.md" } }] } }),
        ]
        .map(|l| l.to_string())
        .join("\n");
        let used = skills_used(&transcript);
        assert_eq!(used, HashSet::from(["ada", "ada-invoices", "ada-payroll", "ada-voice"].map(String::from)), "a subagent's read counts too");
    }

    #[test]
    fn a_skill_edit_is_recorded_once_and_credited_to_the_run_that_wrote_it() {
        let (dir, db) = opening_store();
        let e = employee(Clearance::Supervised);
        db.save_employee(&e).unwrap();
        let since = now() - chrono::Duration::minutes(1);
        let edit = |before: &str, after: &str| own_skills::FileChange {
            change: SkillChange { skill: "ada-notes".into(), file: "SKILL.md".into(), change: ChangeKind::Modified },
            before: Some(before.into()),
            after: Some(after.into()),
        };
        let seen = |change, wrote, own| vec![Seen { change, wrote, own }];
        let runs_of = || db.revisions(&e.id).unwrap().into_iter().map(|r| r.run_id).collect::<Vec<_>>();
        // A check that worked beside a conversation ends first, having seen the conversation's edit.
        db.tx(|| record_changes(&db, &e.id, "check", since, seen(edit("one", "two"), false, false))).unwrap();
        assert_eq!(runs_of(), vec![None]);
        // The conversation that wrote it ends next and takes the credit, without a second revision.
        db.tx(|| record_changes(&db, &e.id, "talk", since, seen(edit("one", "two"), true, true))).unwrap();
        assert_eq!(runs_of(), vec![Some("talk".to_string())]);
        // A run working alone that saw it, say your save in the app, takes nothing.
        db.tx(|| record_changes(&db, &e.id, "sweep", since, seen(edit("one", "two"), false, true))).unwrap();
        assert_eq!(runs_of(), vec![Some("talk".to_string())]);
        // A run alone whose calls do not show the edit, a script it ran, still owns it.
        db.tx(|| record_changes(&db, &e.id, "sweep", since, seen(edit("two", "three"), false, true))).unwrap();
        assert_eq!(runs_of(), vec![Some("sweep".to_string()), Some("talk".to_string())]);
        // The same edit made again after both is a new one.
        let later = now() + chrono::Duration::seconds(1);
        db.tx(|| record_changes(&db, &e.id, "later", later, seen(edit("one", "two"), true, true))).unwrap();
        assert_eq!(runs_of().len(), 3);
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_conversation_knows_the_last_one() {
        let (dir, db) = opening_store();
        let ada = employee(Clearance::Supervised);
        db.save_employee(&ada).unwrap();
        finished(&db, &ada, task("Book the venue"), 90, "t1", "Booked the venue for Thursday.");
        let turn = Run::new(&ada.id, task("The offsite"), "Where are we on the offsite?".into(), now());
        let o = opening(&db, &ada, std::slice::from_ref(&ada), &turn, &turn.id, false, now()).unwrap().unwrap();
        assert_eq!(o.since, None);
        let text = note(&ada, o, &turn);
        assert!(text.contains(", your conversation 'Book the venue': Pat: \"Prompt 90.\" You: Booked the venue for Thursday."), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_routine_hears_your_answers_to_cards_raised_elsewhere() {
        let (dir, db) = opening_store();
        let ada = employee(Clearance::Supervised);
        db.save_employee(&ada).unwrap();
        let mut asked = finished(&db, &ada, task("Tidy the shop"), 90, "t1", "One question.");
        let mut card = Card::new("Repaint the sign?", None, Some("ask:sign".into()), CardAsk::Choices { choices: vec!["Repaint".into()], recommended: 0 });
        card.answer = Some(CardAnswer { at: now() - chrono::Duration::minutes(60), how: AnswerHow::Tapped, text: "Chose: Repaint.".into(), held: false });
        asked.cards = vec![card];
        db.save_run(&asked).unwrap();
        let sweep = Run::new(&ada.id, scheduled(0), "Sweep.".into(), now());
        let o = opening(&db, &ada, std::slice::from_ref(&ada), &sweep, &sweep.id, false, now()).unwrap().unwrap();
        assert_eq!(o.answers.iter().map(|c| c.message.as_str()).collect::<Vec<_>>(), ["Repaint the sign?"]);
        let text = note(&ada, o, &sweep);
        assert!(text.contains("Pat's answers to your cards in the last 7 days, newest first:\n- Your card \"Repaint the sign?\": Chose: Repaint."), "{text}");
        let turn = Run::new(&ada.id, task("Tidy the shop"), "And the door?".into(), now());
        assert!(opening(&db, &ada, std::slice::from_ref(&ada), &turn, "t1", true, now()).unwrap().unwrap().answers.is_empty(), "the conversation heard it");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_resumed_turn_hears_only_what_is_new_elsewhere() {
        let (dir, db) = opening_store();
        let ada = employee(Clearance::Supervised);
        db.save_employee(&ada).unwrap();
        let sweep = RunKind::Scheduled { schedule_id: "s1".into(), slot: now(), label: "Sweep".into(), quiet: false, model: None, effort: None, review: false };
        let first = finished(&db, &ada, task("Plan the offsite"), 180, "t1", "Earlier turn.");
        let reply = || RunKind::Reply { to_run_id: first.id.clone(), label: "Plan the offsite".into(), task: true, card: false };
        let last = finished(&db, &ada, reply(), 60, "t1", "Last turn.");
        finished(&db, &ada, sweep, 120, "s1-run", "Swept before the last turn.");
        let manual = finished(&db, &ada, RunKind::Manual, 30, "m1", "Paid the invoice.");
        let turn = Run::new(&ada.id, reply(), "And now?".into(), now());
        let o = opening(&db, &ada, std::slice::from_ref(&ada), &turn, "t1", true, now()).unwrap().unwrap();
        assert_eq!(o.since, last.ended_at);
        assert_eq!(o.work.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![manual.id]);
        let text = note(&ada, o, &turn);
        assert!(text.contains("What you did elsewhere since your last turn here, newest first.\n- "), "{text}");
        assert!(!text.contains("Swept") && !text.contains("Last turn."), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_manager_hears_what_her_reports_did_on_their_own() {
        let (dir, db) = opening_store();
        let ada = employee(Clearance::Supervised);
        let mut bo = employee(Clearance::Supervised);
        bo.name = "Bo".into();
        bo.slug = "bo".into();
        bo.chat_session = "chat-bo".into();
        bo.manager_id = Some(ada.id.clone());
        db.save_employee(&ada).unwrap();
        db.save_employee(&bo).unwrap();
        let handed = finished(&db, &ada, RunKind::Manual, 240, "m0", "Handed Bo the audit.");
        finished(&db, &bo, RunKind::Delegated { parent_run_id: handed.id.clone() }, 100, "d1", "Audit done.");
        let report = RunKind::Scheduled { schedule_id: "s9".into(), slot: now(), label: "Weekly numbers".into(), quiet: false, model: None, effort: None, review: false };
        finished(&db, &bo, report, 50, "s9-run", "Sales are up 4 percent.");
        finished(&db, &bo, task("Draft the memo"), 45, "t-bo", "Drafted the memo for Pat.");
        let run = Run::new(&ada.id, RunKind::Manual, "Go.".into(), now());
        let employees = vec![ada.clone(), bo.clone()];
        let o = opening(&db, &ada, &employees, &run, "m-now", false, now()).unwrap().unwrap();
        assert_eq!(o.team.len(), 1, "her own hand-off comes back to her directly");
        let text = note(&ada, o, &run);
        assert!(text.contains("From your team since your last run:\n- Bo, scheduled 'Weekly numbers' ("), "{text}");
        assert!(text.contains("): Sales are up 4 percent.") && !text.contains("Audit done."), "{text}");
        assert!(!text.contains("Drafted the memo"), "Bo's conversation with Pat stays between them: {text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_routine_that_works_again_files_its_failures() {
        let dir = std::env::temp_dir().join(format!("omegastar-streak-{}", uuid::Uuid::new_v4()));
        let ada = employee(Clearance::Supervised);
        let office = claiming_office(&dir, &[&ada]).await;
        let run = |mins: i64, state: RunState| {
            let at = now() - chrono::Duration::minutes(mins);
            let kind = RunKind::Scheduled { schedule_id: "s1".into(), slot: at, label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
            let mut r = Run::new(&ada.id, kind, "Go.".into(), at);
            r.state = state;
            r.ended_at = Some(at);
            assert!(office.db().insert_run(&r, &r.id).unwrap());
            r
        };
        let failures: Vec<Run> = [40, 30, 20].into_iter().map(|m| run(m, RunState::Failed)).collect();
        let ok = run(10, RunState::Succeeded);
        office.archive_streak("s1", &ok).unwrap();
        for f in &failures {
            assert!(office.run(&f.id).unwrap().archived_at.is_some(), "the failures before it are filed, times to the nanosecond and all");
        }
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_gate_holds_runs_by_kind() {
        let scheduled = RunKind::Scheduled { schedule_id: "s1".into(), slot: now(), label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
        let tidy = RunKind::Tidy { reason: TidyReason::Auto, signature: vec![], plan: None };
        let talk = task("Draft it");
        let gate = |kind: &RunKind, held, account_ok, busy, awake| gate(&Run::new("e1", kind.clone(), "Go.".into(), now()), held, account_ok, busy, awake);
        // gate(run, held, account_ok, busy, awake)
        assert_eq!(gate(&talk, true, true, false, true), Gate::Skip);
        assert_eq!(gate(&talk, false, false, false, true), Gate::Claim);
        assert_eq!(gate(&scheduled, false, false, false, true), Gate::Skip);
        assert_eq!(gate(&scheduled, false, true, false, false), Gate::Skip);
        assert_eq!(gate(&scheduled, false, true, false, true), Gate::Claim);
        assert_eq!(gate(&talk, false, true, false, false), Gate::Claim);
        assert_eq!(gate(&tidy, false, true, true, true), Gate::Skip);
        assert_eq!(gate(&tidy, false, true, false, true), Gate::Claim);
        let came = RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat: false, thread: Some("t1".into()) };
        assert_eq!(gate(&came, false, false, false, true), Gate::Skip);
        assert_eq!(gate(&came, false, true, false, false), Gate::Skip);
        assert_eq!(gate(&came, false, true, false, true), Gate::Claim);
        // A due check back in a conversation is a turn of it, but nobody is watching: it waits for the account.
        let check = Run { check: Some("c1".into()), ..Run::new("e1", RunKind::Reply { to_run_id: "t1".into(), label: "Draft it".into(), task: true, card: false }, "Check.".into(), now()) };
        assert_eq!(super::gate(&check, false, false, false, true), Gate::Skip);
        assert_eq!(super::gate(&check, false, true, false, true), Gate::Claim);
    }

    #[test]
    fn each_limit_has_its_own_retries() {
        let usage = ended(Some(outcome(false, "success", "You have hit your limit.", Some(Limit::Usage))), "");
        let rate = ended(Some(outcome(false, "success", "API Error: Server is temporarily limiting requests.", Some(Limit::Transient))), "");
        let mut run = Run::new("e1", RunKind::Manual, "go".into(), now());
        for _ in 0..TRANSIENT_RETRIES {
            assert_eq!(requeue_for_limit(hit_limit(false, &rate), &run), Some(Limit::Transient));
            reset_for_requeue(&mut run, &[], Limit::Transient);
        }
        assert_eq!(requeue_for_limit(hit_limit(false, &rate), &run), None, "eight rate limits, then it fails");
        for _ in 0..USAGE_RETRIES {
            assert_eq!(requeue_for_limit(hit_limit(false, &usage), &run), Some(Limit::Usage), "rate limits never use up the usage limit's retries");
            reset_for_requeue(&mut run, &[], Limit::Usage);
        }
        assert_eq!(requeue_for_limit(hit_limit(false, &usage), &run), None);
        assert_eq!((run.limit_retries, run.transient_retries), (11, 8));
        assert_eq!(hit_limit(false, &usage), Some(Limit::Usage), "out of retries, it still holds the office");
        assert_eq!(hit_limit(true, &usage), None, "you stopped it");
        assert_eq!(hit_limit(false, &ended(Some(outcome(true, "success", "Done.", None)), "")), None);
        assert_eq!(hit_limit(false, &Err(AppError::internal("could not start"))), None);
        // An older build reports the usage limit as a bare 429, but the rate limit before it names the reset.
        let mut older = ended(Some(outcome(false, "success", "You've hit your session limit.", Some(Limit::Transient))), "");
        if let Ok(e) = older.as_mut() {
            e.limited = Some(now() + chrono::Duration::hours(2));
        }
        assert_eq!(hit_limit(false, &older), Some(Limit::Usage));
    }

    #[test]
    fn rate_limits_wait_longer_each_time_in_a_row() {
        assert_eq!((0..7).map(|n| backoff(n).num_minutes()).collect::<Vec<_>>(), vec![5, 10, 20, 40, 60, 60, 60]);
    }

    fn window(percent: f64) -> Option<UsageWindow> {
        Some(UsageWindow { percent, resets_at: None, resets_text: None })
    }

    #[tokio::test]
    async fn a_rate_limit_holds_the_office_longer_each_time_and_a_usage_check_cannot_end_it() {
        let dir = std::env::temp_dir().join(format!("omegastar-rate-hold-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let office = Office::open(dir.clone(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        let held_for = |office: &Office| office.usage_limits().held_until.map(|t| ((t - now()).num_seconds() as f64 / 60.0).round() as i64);
        office.hold_for_transient_limit();
        assert_eq!(held_for(&office), Some(5));
        office.hold_for_transient_limit();
        assert_eq!(held_for(&office), Some(5), "a run that meets it while held waits with the rest");
        let room = || claude::UsageSample { five_hour: window(12.0), seven_day: window(30.0) };
        office.observe_usage(room(), true);
        assert_eq!(held_for(&office), Some(5), "the usage windows say nothing about a rate limit");
        assert!(office.release_hold(now() + chrono::Duration::minutes(6)));
        office.hold_for_transient_limit();
        assert_eq!(held_for(&office), Some(10), "it came back before any run succeeded");
        assert_eq!(office.db().held_until().unwrap(), office.usage_limits().held_until, "kept across a restart");

        // After a restart it is still a rate limit's hold, which a usage check cannot end, and the next waits longer.
        drop(office);
        let office = Office::open(dir.clone(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        office.observe_usage(room(), true);
        assert_eq!(held_for(&office), Some(10));
        assert!(office.release_hold(now() + chrono::Duration::minutes(11)));
        office.hold_for_transient_limit();
        assert_eq!(held_for(&office), Some(20));

        assert!(office.release_hold(now() + chrono::Duration::minutes(21)));
        office.hold_until(now() + chrono::Duration::hours(2), Limit::Usage);
        office.observe_usage(room(), true);
        assert_eq!(office.usage_limits().held_until, None, "a usage check still ends a usage hold");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_is_logged_by_what_broke_it() {
        let failed = |error: &str| Run { state: RunState::Failed, error: Some(error.into()), ..Run::new("e1", RunKind::Manual, "go".into(), now()) };
        assert_eq!(error_class(&failed("API Error: Server is temporarily limiting requests."), Some(Limit::Transient)), "rate limit");
        assert_eq!(error_class(&Run { limited: true, ..failed("You have hit your limit.") }, None), "usage limit");
        assert_eq!(error_class(&failed("io: No space left on device (os error 28)"), None), "disk full");
        assert_eq!(error_class(&failed("Stopped: the run reached its turn limit."), None), "turn limit");
        assert_eq!(error_class(&failed("could not start Claude Code: no such file"), None), "could not start");
        assert_eq!(error_class(&failed("Claude Code exited without a result (exit code Some(1))."), None), "no result");
        assert_eq!(error_class(&Run { state: RunState::Lost, ..failed("Omega Star stopped while this run was in progress.") }, None), "lost when Omega Star stopped");
        assert_eq!(error_class(&failed("Something broke."), None), "error");
    }

    #[test]
    fn a_retry_reads_only_its_own_attempt() {
        let dir = std::env::temp_dir().join(format!("omegastar-attempt-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        assert_eq!(attempt_text(&path, 0), "", "a missing transcript is empty");
        fs::write(&path, "first attempt\n").unwrap();
        let start = fs::metadata(&path).unwrap().len();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut f, b"second attempt\n").unwrap();
        assert_eq!(attempt_text(&path, start), "second attempt\n");
        assert_eq!(attempt_text(&path, 0), "first attempt\nsecond attempt\n");
        let _ = fs::remove_dir_all(&dir);
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
        run.restarts = 1;
        run.restarted = true;
        db.save_run(&run).unwrap();

        let fc = own_skills::FileChange {
            change: SkillChange { skill: "ada-notes".into(), file: "SKILL.md".into(), change: ChangeKind::Modified },
            before: Some("old".into()),
            after: Some("new".into()),
        };
        run.cost_usd = Some(0.5);
        reset_for_requeue(&mut run, std::slice::from_ref(&fc.change), Limit::Usage);
        db.tx(|| {
            db.insert_revision(&e.id, &own_skills::revision(fc, Some(run.id.clone()), now()))?;
            db.save_run(&run)
        })
        .unwrap();

        let saved = db.run(&run.id).unwrap().unwrap().run;
        assert_eq!((saved.state, saved.limit_retries, saved.started_at, saved.summary), (RunState::Queued, 1, None, None));
        assert_eq!((saved.restarts, saved.restarted), (1, false), "it is told it was the limit, not the earlier restart");
        assert!(saved.denials.is_empty());
        assert_eq!(saved.skill_changes.len(), 1);
        assert_eq!(saved.cost_usd, Some(0.5), "what it cost so far stays with it");
        assert_eq!(db.revisions(&e.id).unwrap().iter().filter(|r| r.run_id.as_deref() == Some(run.id.as_str())).count(), 1);
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_run_cut_off_by_a_restart_carries_on_without_reporting_back() {
        let dir = std::env::temp_dir().join(format!("omegastar-restart-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let ada = employee(Clearance::Supervised);
        let child = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&ada).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            let parent = finished(&db, &ada, task("Plan the launch"), 30, "s1", "Handed the post over.");
            let mut child = Run::new(&ada.id, RunKind::Delegated { parent_run_id: parent.id.clone() }, "Draft the launch post.".into(), now());
            assert!(db.insert_run(&child, &child.id).unwrap());
            child.state = RunState::Running;
            child.started_at = Some(now());
            db.save_run(&child).unwrap();
            child
        };
        let office = Office::open(dir.clone(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        let saved = office.run(&child.id).unwrap();
        assert_eq!((saved.state, saved.restarts, saved.restarted, saved.delivered_at), (RunState::Queued, 1, true, None));
        let runs = office.runs(None, 100).unwrap();
        assert!(!runs.iter().any(|r| matches!(r.kind, RunKind::Answered { .. })), "it reports back when it really ends: {runs:?}");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_routine_card_replaces_the_older_ones() {
        let dir = std::env::temp_dir().join(format!("omegastar-cards-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let e = employee(Clearance::Supervised);
        db.save_employee(&e).unwrap();
        let items = || CardAsk::Items { verb: "Post".into(), items: vec![CardItem { id: "d1".into(), text: "Draft one".into(), about: None, recommended: true }] };
        let card = |r: &mut Run, ask: CardAsk| r.cards = vec![Card::new("Post these?", None, Some("ask:drafts".into()), ask)];
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
        let chore = scheduled("s1", 40, CardAsk::Chore { subject: "chrome".into(), url: None }, false);
        let newer = scheduled("s1", 10, items(), false);

        let replaced = db.tx(|| replace_older_cards_in(&db, &newer, now())).unwrap();
        assert_eq!(replaced.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![first.id.as_str(), replied.id.as_str()]);
        let saved = |r: &Run| db.run(&r.id).unwrap().unwrap().run;
        let how = |r: &Run| saved(r).cards.remove(0).answer.map(|a| (a.how, a.text));
        assert_eq!(how(&first), Some((AnswerHow::Replaced, String::new())));
        assert!(saved(&first).archived_at.is_some());
        assert_eq!(how(&replied), Some((AnswerHow::Replaced, String::new())));
        assert!(saved(&replied).archived_at.is_none(), "you replied to it, so it stays");
        assert_eq!(how(&chore), None, "a chore is never replaced");
        assert_eq!(how(&newer), None);

        // A due check's runs share no routine.
        let keyed = |r: Run, key: &str| {
            let mut r = r;
            r.cards[0].key = Some(key.into());
            db.save_run(&r).unwrap();
            r
        };
        let check = keyed(scheduled("check:c1", 30, items(), false), "ask:check-a");
        let later = keyed(scheduled("check:c1", 5, items(), false), "ask:check-b");
        assert!(db.tx(|| replace_older_cards_in(&db, &later, now())).unwrap().is_empty());
        assert_eq!(how(&check), None);

        // A card about the same thing replaces the employee's earlier one wherever it was asked, and its run
        // leaves the Inbox once nothing is left to ask.
        let manual = |key: &str| {
            let mut r = Run::new(&e.id, RunKind::Manual, "go".into(), now() - chrono::Duration::minutes(20));
            r.state = RunState::Succeeded;
            r.cards = vec![Card::new("Merge it?", None, Some(key.into()), items())];
            assert!(db.insert_run(&r, &r.id).unwrap());
            r
        };
        let asked = manual("ask:pr:12");
        let other = manual("ask:pr:13");
        let mut again = Run::new(&e.id, RunKind::Manual, "go".into(), now());
        again.cards = vec![Card::new("Merge #12?", None, Some("ask:pr:12".into()), items())];
        assert!(db.insert_run(&again, &again.id).unwrap());
        let replaced = db.tx(|| replace_older_cards_in(&db, &again, now())).unwrap();
        assert_eq!(replaced.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![asked.id.as_str()]);
        assert_eq!(how(&asked).map(|h| h.0), Some(AnswerHow::Replaced));
        assert!(saved(&asked).archived_at.is_some());
        assert_eq!(how(&other), None, "another key is another question");
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_card_replaces_ones_put_off_and_files_only_what_needs_nothing() {
        let (dir, db) = opening_store();
        let e = employee(Clearance::Supervised);
        db.save_employee(&e).unwrap();
        let choices = || CardAsk::Choices { choices: vec!["Post".into()], recommended: 0 };
        let routine = |ago: i64, state: RunState| {
            let at = now() - chrono::Duration::minutes(ago);
            let kind = RunKind::Scheduled { schedule_id: "s1".into(), slot: at, label: "Drafts".into(), quiet: true, model: None, effort: None, review: false };
            let mut r = Run::new(&e.id, kind, "go".into(), at);
            r.state = state;
            r.ended_at = state.is_finished().then_some(at);
            r.cards = vec![Card::new("Post these?", None, Some("ask:drafts".into()), choices())];
            r
        };
        let put_off = |r: &mut Run, parked: bool| {
            r.cards[0].answer = Some(CardAnswer { at: now(), how: AnswerHow::Dismissed, text: "Later.".into(), held: false });
            (r.cards[0].back_at, r.cards[0].parked) = if parked { (None, true) } else { (Some(now() + chrono::Duration::days(1)), false) };
        };
        let insert = |r: Run| {
            assert!(db.insert_run(&r, &r.id).unwrap());
            r
        };
        // Put off with Later, back in the queue after a limit, and with news you have not seen.
        let mut later = routine(60, RunState::Succeeded);
        put_off(&mut later, false);
        let later = insert(later);
        let waiting = insert(routine(50, RunState::Queued));
        let mut news = routine(40, RunState::Succeeded);
        news.cards.push(Card::new("The sale is live.", None, Some("ask:sale".into()), CardAsk::News));
        let news = insert(news);
        // Parked elsewhere, about the same thing.
        let mut parked = Run::new(&e.id, RunKind::Manual, "go".into(), now() - chrono::Duration::minutes(30));
        parked.state = RunState::Succeeded;
        parked.cards = vec![Card::new("Post the drafts?", None, Some("ask:drafts".into()), choices())];
        put_off(&mut parked, true);
        let parked = insert(parked);
        let newer = insert(routine(5, RunState::Succeeded));

        let replaced = db.tx(|| replace_older_cards_in(&db, &newer, now())).unwrap();
        assert_eq!(replaced.len(), 4);
        let saved = |r: &Run| db.run(&r.id).unwrap().unwrap().run;
        for r in [&later, &waiting, &news, &parked] {
            let c = saved(r).cards.remove(0);
            assert_eq!((c.answer.map(|a| a.how), c.back_at, c.parked), (Some(AnswerHow::Replaced), None, false), "it never comes back: {:?}", r.kind);
        }
        assert!(saved(&later).archived_at.is_some() && saved(&parked).archived_at.is_some());
        assert!(saved(&waiting).archived_at.is_none(), "still to run, so not in the Archive");
        assert!(saved(&news).archived_at.is_none() && saved(&news).has_news(), "news you have not seen keeps its row");
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn answers_that_come_back_into_a_conversation_keep_each_others_cards() {
        let (dir, db) = opening_store();
        let e = employee(Clearance::Supervised);
        db.save_employee(&e).unwrap();
        let talk = finished(&db, &e, task("Plan the launch"), 60, "s1", "Asking Bo and Cy.");
        // Each colleague's answer comes back as a turn of the conversation, with a card about its own thing.
        let answer = |mins: i64, key: &str| {
            let kind = RunKind::Answered { asked_run_id: format!("asked-{key}"), from: "e2".into(), task: false, chat: false, thread: Some(talk.id.clone()) };
            let mut r = finished(&db, &e, kind, mins, "s1", "They answered.");
            r.cards = vec![Card::new("Go ahead?", None, Some(key.into()), CardAsk::Choices { choices: vec!["Go".into()], recommended: 0 })];
            db.save_run(&r).unwrap();
            r
        };
        let posts = answer(30, "ask:posts");
        let venue = answer(20, "ask:venue");
        let open = |r: &Run| db.run(&r.id).unwrap().unwrap().run.has_open_card();

        assert!(db.tx(|| replace_older_cards_in(&db, &venue, now())).unwrap().is_empty());
        assert!(open(&posts) && open(&venue), "another key is another decision, so both stay open");

        let again = answer(10, "ask:posts");
        let replaced = db.tx(|| replace_older_cards_in(&db, &again, now())).unwrap();
        assert_eq!(replaced.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![posts.id.as_str()], "the same key still replaces");
        assert!(!open(&posts) && open(&venue));
        assert!(db.run(&posts.id).unwrap().unwrap().run.archived_at.is_none(), "a turn of the conversation stays in it");
        assert!(db.run(&talk.id).unwrap().unwrap().run.archived_at.is_none());
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
            (outcome(true, "success", "Done.", None), "", (RunState::Succeeded, None)),
            (outcome(false, "error_max_turns", "", None), "", failed("Stopped: the run reached its turn limit.")),
            (outcome(false, "error_during_execution", "Something broke.", None), "noise", failed("Something broke.")),
            (outcome(false, "error_during_execution", "", None), "warning: one\nerror: the real cause\n", failed("warning: one\nerror: the real cause")),
            (outcome(false, "error_during_execution", "", None), "", failed("Claude Code reported error_during_execution")),
        ];
        for (o, stderr, want) in cases {
            assert_eq!(state_of(&o, stderr_tail(stderr)), want);
            assert_eq!(final_state(false, &ended(Some(o), stderr)), want);
        }
    }
}
