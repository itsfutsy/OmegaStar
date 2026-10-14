//! The office tools employees call over MCP (dispatched from `mcp.rs`).

use super::*;

/// Longest chain of runs (delegations and questions) a new question may extend.
const ASK_DEPTH: usize = 6;
/// Under Claude Code's 300 s idle abort, so a wait never races it.
const AWAIT_LIMIT: Duration = Duration::from_secs(240);
/// Most runs `source_chain` walks back through hand-offs.
const SOURCE_HOPS: usize = 12;
/// CEO chat turns a chat origin quotes.
const SOURCE_TURNS: usize = 3;

impl Office {
    /// The live run a token belongs to, and its employee.
    pub fn caller(&self, token: &str) -> Option<(String, Employee)> {
        let run_id = lock(&self.0.active).iter().find(|(_, a)| a.token == token).map(|(id, _)| id.clone())?;
        let db = self.db();
        let run = db.run(&run_id).ok()??.run;
        let e = db.employee(&run.employee_id).ok()??;
        Some((run_id, e))
    }

    pub fn team(&self, caller: &Employee) -> Result<Value> {
        let db = self.db();
        let now = now();
        let reports: Vec<Value> = db
            .employees()?
            .iter()
            .filter(|e| e.manager_id.as_deref() == Some(&caller.id))
            .map(|e| {
                let v = Self::presence(&db, e, now, false)?;
                Ok(json!({
                    "id": e.id, "name": e.name, "slug": e.slug, "title": e.title, "objective": e.objective,
                    "presence": v.presence,
                    "lastSummary": v.last_run.and_then(|r| r.summary),
                }))
            })
            .collect::<Result<_>>()?;
        Ok(json!({ "reports": reports }))
    }

    pub fn delegate(&self, caller_run: &str, caller: &Employee, to: &str, task: &str) -> Result<Value> {
        if task.trim().is_empty() {
            return Err(AppError::invalid("task is empty"));
        }
        let (report, busy, chat) = {
            let db = self.db();
            let reports: Vec<Employee> = db.employees()?.into_iter().filter(|e| e.manager_id.as_deref() == Some(&caller.id)).collect();
            let report = find_member(&reports, to, |e| (&e.id, Some(&e.slug), &e.name))
                .ok_or_else(|| AppError::invalid(format!("'{to}' is not one of your direct reports. Call team() for the list.")))?
                .clone();
            let busy = db.active_runs_of(&report.id)?.iter().any(|r| r.state == RunState::Running && !r.kind.is_conversation());
            let chat = db.run(caller_run)?.map(|r| r.run.kind.lane()) == Some(Lane::Chat);
            (report, busy, chat)
        };
        let run = Run::new(&report.id, RunKind::Delegated { parent_run_id: caller_run.to_string() }, task.trim().to_string(), now());
        let session = run.id.clone();
        let run = self.enqueue(run, session)?;
        self.append_event(
            caller_run,
            RunEvent::Delegated { child_run_id: run.id.clone(), employee_id: report.id.clone(), task: run.prompt.clone() },
        );
        let note = handoff_note(&report.name, "Their report", busy, chat);
        Ok(json!({ "runId": run.id, "employee": report.name, "state": run.state, "note": note }))
    }

    /// Every other agent, with what they are for, who they report to and whether they are free.
    pub fn colleagues(&self, caller: &Employee) -> Result<Value> {
        let db = self.db();
        let now = now();
        let employees = db.employees()?;
        let people = db.people()?;
        let manager_name = |id: &Option<String>| {
            id.as_ref().and_then(|m| {
                employees.iter().find(|x| &x.id == m).map(|x| x.name.clone()).or_else(|| people.iter().find(|p| &p.id == m).map(|p| p.name.clone()))
            })
        };
        let list: Vec<Value> = employees
            .iter()
            .filter(|e| e.id != caller.id)
            .map(|e| {
                let v = Self::presence(&db, e, now, false)?;
                Ok(json!({
                    "id": e.id, "name": e.name, "slug": e.slug, "title": e.title,
                    "objective": e.objective, "description": e.description,
                    "reportsTo": manager_name(&e.manager_id),
                    "presence": v.presence,
                }))
            })
            .collect::<Result<_>>()?;
        Ok(json!({ "colleagues": list }))
    }

    /// Asks any other agent one question, or with `task` hands them a task. Refuses a colleague who is
    /// working further up this chain (they would wait on each other), and chains longer than ASK_DEPTH.
    pub fn ask_colleague(&self, caller_run: &str, caller: &Employee, to: &str, question: &str, task: bool) -> Result<Value> {
        if question.trim().is_empty() {
            return Err(AppError::invalid(if task { "task is empty" } else { "question is empty" }));
        }
        let (colleague, busy, chat) = {
            let db = self.db();
            let others: Vec<Employee> = db.employees()?.into_iter().filter(|e| e.id != caller.id).collect();
            let key = to.trim().to_lowercase();
            let by_first: Vec<&Employee> = others.iter().filter(|e| e.name.split_whitespace().next().map(str::to_lowercase) == Some(key.clone())).collect();
            let colleague = find_member(&others, to, |e| (&e.id, Some(&e.slug), &e.name))
                .or(if by_first.len() == 1 { Some(by_first[0]) } else { None })
                .ok_or_else(|| AppError::invalid(format!("'{to}' is not a colleague. Call colleagues() for the list.")))?
                .clone();
            // (employee, still running) for every run up the chain, the caller's first.
            let mut chain = vec![];
            let mut cur = db.run(caller_run)?.map(|d| d.run);
            while let Some(r) = cur {
                chain.push((r.employee_id.clone(), !r.state.is_finished()));
                cur = match r.upstream() {
                    Some(p) if chain.len() <= ASK_DEPTH => db.run(p)?.map(|d| d.run),
                    _ => None,
                };
            }
            if chain.iter().any(|(id, running)| *running && id == &colleague.id) {
                return Err(AppError::invalid(format!(
                    "{} is already waiting on you further up this chain, so they cannot answer now. Answer with what you have, and say in your report what you still need from them.",
                    colleague.name
                )));
            }
            if chain.len() >= ASK_DEPTH {
                return Err(AppError::invalid("This work is already several hand-offs deep. Answer with what you have, and say in your report what is still open."));
            }
            // Background work goes one piece at a time; their conversations with the CEO run beside it.
            let busy = db.active_runs_of(&colleague.id)?.iter().any(|r| r.state == RunState::Running && !r.kind.is_conversation());
            let chat = db.run(caller_run)?.map(|r| r.run.kind.lane()) == Some(Lane::Chat);
            (colleague, busy, chat)
        };
        let run = Run::new(
            &colleague.id,
            RunKind::Asked { parent_run_id: caller_run.to_string(), asked_by: caller.id.clone(), task },
            question.trim().to_string(),
            now(),
        );
        let session = run.id.clone();
        let run = self.enqueue(run, session)?;
        self.append_event(
            caller_run,
            RunEvent::Asked { child_run_id: run.id.clone(), employee_id: colleague.id.clone(), question: run.prompt.clone(), task },
        );
        let note = handoff_note(&colleague.name, if task { "Their report" } else { "The answer" }, busy, chat);
        Ok(json!({ "runId": run.id, "colleague": colleague.name, "state": run.state, "note": note }))
    }

    pub async fn await_reports(&self, caller: &Employee, run_ids: &[String]) -> Result<Value> {
        let mut rx = self.0.finished.subscribe();
        let deadline = tokio::time::Instant::now() + AWAIT_LIMIT;
        loop {
            let runs = {
                let db = self.db();
                let mut v = vec![];
                for id in run_ids {
                    let r = db.run(id)?.ok_or_else(|| AppError::not_found(format!("run {id} not found")))?.run;
                    let parent_ok = r.parent_run_id().and_then(|p| inbox::run_of(&db, p)).map(|p| p.employee_id == caller.id);
                    if parent_ok != Some(true) {
                        return Err(AppError::invalid(format!("run {id} was not delegated or asked by you")));
                    }
                    v.push(r);
                }
                v
            };
            let done = runs.iter().all(|r| r.state.is_finished());
            if done || tokio::time::Instant::now() >= deadline {
                self.mark_delivered(runs.iter().filter(|r| r.state.is_finished()).map(|r| r.id.as_str()))?;
                let names: HashMap<String, String> = self.db().employees()?.into_iter().map(|e| (e.id, e.name)).collect();
                let reports: Vec<Value> = runs
                    .iter()
                    .map(|r| {
                        json!({
                            "runId": r.id, "employee": names.get(&r.employee_id), "state": r.state,
                            "summary": r.summary, "error": r.error,
                        })
                    })
                    .collect();
                let note = if done {
                    "All reports are in."
                } else {
                    "Some runs are still going. Each comes back to you in a new run when it finishes, so you can finish now; call await_reports again only if you need it in this run."
                };
                return Ok(json!({ "reports": reports, "note": note }));
            }
            let _ = tokio::time::timeout_at(deadline, rx.changed()).await;
        }
    }

    /// Records that these finished answers and reports reached whoever handed them over.
    fn mark_delivered<'a>(&self, run_ids: impl Iterator<Item = &'a str>) -> Result<()> {
        let db = self.db();
        for id in run_ids {
            if let Some(mut r) = db.run(id)?.map(|d| d.run) {
                if matches!(r.kind, RunKind::Asked { .. } | RunKind::Delegated { .. }) && r.delivered_at.is_none() {
                    r.delivered_at = Some(now());
                    db.save_run(&r)?;
                }
            }
        }
        Ok(())
    }

    /// Brings every finished answer or report whose asker's run ended without collecting it to the
    /// asker, as an Answered run. Called whenever a run ends, since either side can end last; the
    /// check and `delivered_at` happen under one lock, so each is delivered exactly once.
    pub(super) fn deliver_answers(&self, ended: &Run) -> Result<()> {
        let due = due_answers(&self.db(), ended)?;
        for (run, session) in due {
            self.enqueue(run, session)?;
        }
        Ok(())
    }

    pub fn people_list(&self) -> Result<Value> {
        let people: Vec<Value> = self
            .db()
            .people()?
            .iter()
            .map(|p| {
                json!({
                    "id": p.id, "name": p.name, "title": p.title, "description": p.description,
                    "strengths": p.strengths, "responsibilities": p.responsibilities,
                    "channels": p.channels, "notes": p.notes,
                })
            })
            .collect();
        Ok(json!({ "people": people }))
    }

    pub fn request_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
        let kind: RequestKind = serde_json::from_value(json!(s("kind").unwrap_or("question")))
            .map_err(|_| AppError::invalid("kind must be question, task or info"))?;
        let due_at = s("dueAt").map(parse_due).transpose()?;
        let (req, person) = self.create_request(
            &caller.id,
            Some(caller_run),
            s("person").ok_or_else(|| AppError::invalid("person is required"))?,
            kind,
            s("message").unwrap_or(""),
            s("channelId"),
            due_at,
        )?;
        let channel = req.channel_id.as_ref().and_then(|c| person.channels.iter().find(|x| &x.id == c));
        let relay = self.db().company()?.map(|c| c.ceo_name).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "the person you work for".into());
        let then = "then call `sent` with the request id, the tool you sent it with and exactly what that tool returned";
        let hint = match channel {
            Some(c) => format!(
                "Recorded. Now deliver it: {}. Use a connector or skill you have for that channel, {then}. If you cannot reach it, say so in your report and {relay} will relay it from Omega Star.",
                claude::channel_line(c)
            ),
            None => format!("Recorded without a channel. Deliver it through whichever of their channels fits the ask if you can reach one, {then}; otherwise say so in your report and {relay} will relay it from Omega Star."),
        };
        Ok(json!({
            "request": req,
            "person": { "id": person.id, "name": person.name, "title": person.title, "channels": person.channels, "notes": person.notes },
            "channel": channel,
            "hint": hint,
        }))
    }

    pub fn my_requests(&self, caller: &Employee) -> Result<Value> {
        let watch = self.settings()?.watch;
        let mut reqs = self.db().requests(&RequestFilter { employee_id: Some(caller.id.clone()), ..Default::default() })?;
        reqs.truncate(50);
        for r in &mut reqs {
            Self::present_with(&watch, r);
        }
        Ok(json!({ "requests": reqs }))
    }

    /// Records that the caller delivered one of its requests, from what the send tool returned.
    pub fn sent_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).unwrap_or("").to_string();
        let (id, tool) = (text("requestId"), text("tool"));
        let result = match args.get("result") {
            Some(Value::String(s)) => s.clone(),
            Some(v) if !v.is_null() => v.to_string(),
            _ => String::new(),
        };
        if id.is_empty() || tool.is_empty() {
            return Err(AppError::invalid("requestId and tool are required: the request you sent and the tool you sent it with"));
        }
        if result.trim().is_empty() {
            return Err(AppError::invalid("result is required: exactly what the send tool returned"));
        }
        let (r, person, lane) = {
            let db = self.db();
            let mut r = match db.request(&id)? {
                Some(r) if r.employee_id == caller.id => r,
                _ => return Err(AppError::not_found(format!("'{id}' is not one of your requests. Call my_requests() for the list."))),
            };
            if r.state == RequestState::Cancelled {
                return Err(AppError::invalid("that request was cancelled, so there is nothing to watch"));
            }
            if r.delivery.is_some() {
                return Err(AppError::conflict("that request is already recorded as sent"));
            }
            r.delivery = Some(Delivery::new(watch::parse_sent(&tool, &result, now()), DeliverySource::Employee, r.kind));
            db.save_request(&r)?;
            let person = Self::person_name(&db, &r.person_id)?;
            (r, person, db.run(caller_run)?.map(|x| x.run.kind.lane()))
        };
        self.announce(&r);
        let back = if lane == Some(Lane::Work) { "starts a follow-up run that resumes this session" } else { "starts a follow-up run for you" };
        let note = match &r.delivery {
            Some(Delivery { watch: None, .. }) => "Recorded as sent. An FYI needs no reply, so nothing watches it.".to_string(),
            _ if !r.state.is_open() => "Recorded as sent.".to_string(),
            Some(Delivery { locator: Locator::TeamsChat { server: Some(_), .. }, .. }) => format!(
                "Omega Star now watches that Teams chat for {person}'s reply, nudges them once when it is due, and {back} when they reply. Do not watch for it yourself."
            ),
            _ => format!("Omega Star now checks for {person}'s reply in business hours, nudges them once when it is due, and {back} when they reply. Do not watch for it yourself."),
        };
        Ok(json!({ "request": self.present(r), "note": note }))
    }

    /// What a reply check or a nudge found about its own request, recorded once per run. A reply
    /// resolves the request as recorded by nobody, so the employee's one follow-up run starts. On a
    /// Teams chat, a finding that keeps watching leaves the native reader's cursor where it was: the
    /// run may have missed a message, so the reader still judges everything after the cursor.
    pub fn check_result_tool(&self, caller_run: &str, args: &Value) -> Result<Value> {
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).unwrap_or("").to_string();
        let id = text("requestId");
        let verdict: Verdict = serde_json::from_value(json!(text("verdict")))
            .map_err(|_| AppError::invalid("verdict must be answered, done, askedBack, acknowledged or noReply"))?;
        let kind = self.run(caller_run)?.kind;
        let nudging = matches!(kind, RunKind::Nudge { .. });
        if !matches!(&kind, RunKind::ReplyCheck { request_id, .. } | RunKind::Nudge { request_id, .. } if *request_id == id) {
            return Err(AppError::invalid("check_result is only for a reply check or a nudge that Omega Star started, about its own request"));
        }
        {
            let mut active = lock(&self.0.active);
            let a = active.get_mut(caller_run).ok_or_else(|| AppError::invalid("this run has ended"))?;
            if a.verdict {
                return Err(AppError::conflict("you already recorded what you found in this run"));
            }
            a.verdict = true;
        }
        let at = now();
        let (words, note) = (text("response"), text("note"));
        let (applied, r) = {
            let db = self.db();
            let mut r = db.request(&id)?.ok_or_else(|| AppError::not_found("request not found"))?;
            let said = if !words.is_empty() { words.clone() } else if !note.is_empty() { note.clone() } else { text("verdict") };
            let heard = Heard { at, from: Self::person_name(&db, &r.person_id)?, text: said.clone(), verdict, note };
            let chat = matches!(r.delivery, Some(Delivery { locator: Locator::TeamsChat { .. }, .. }));
            let seen = watch_of(&mut r).and_then(|w| w.seen.clone());
            let applied = watch::apply_verdict(&mut r, &Seen { id: caller_run.to_string(), at }, heard, &said);
            if chat && applied == Applied::Kept {
                if let Some(w) = watch_of(&mut r) {
                    w.seen = seen;
                }
            }
            if applied != Applied::Stale {
                db.save_request(&r)?;
            }
            (applied, r)
        };
        let next = match (&applied, verdict) {
            (Applied::Stale, _) => "Nothing changed: the request was already answered or is no longer watched. Send nothing; end now with one line.",
            (Applied::Kept, Verdict::NoReply) if nudging => "Recorded: no reply yet. Send the reminder now, as your instructions say.",
            (Applied::Kept, _) if nudging => "Recorded. Send nothing; end now with one line.",
            (Applied::Kept, _) => "Recorded. Omega Star keeps watching. End now with one line.",
            (Applied::Resolve { .. }, _) => "Recorded. A follow-up run carries on with their reply. Send nothing; end now with one line.",
        };
        if applied != Applied::Stale {
            self.announce(&r);
        }
        if let Applied::Resolve { state, response } = applied {
            self.resolve_request(&id, state.as_str(), Some(response), None)?;
        }
        Ok(json!({ "recorded": verdict, "note": next }))
    }

    /// `update_request` from an employee's run. A reply check or a nudge records with `check_result`
    /// instead: recording as the requester would start no follow-up run.
    pub fn update_request_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        if matches!(self.run(caller_run)?.kind, RunKind::ReplyCheck { .. } | RunKind::Nudge { .. }) {
            return Err(AppError::invalid("in a reply check or a nudge, record what you found with check_result"));
        }
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let response = args.get("response").and_then(|v| v.as_str()).map(String::from);
        let r = self.resolve_request(text("id"), text("state"), response, Some(&caller.id))?;
        Ok(json!({ "request": self.present(r) }))
    }

    fn own_schedule(&self, caller: &Employee, id: &str) -> Result<Schedule> {
        match self.db().schedule(id)? {
            Some((s, _)) if s.employee_id == caller.id => Ok(s),
            _ => Err(AppError::not_found(format!("'{id}' is not one of your schedules. Call schedules() for the list."))),
        }
    }

    pub fn my_schedules(&self, caller: &Employee) -> Result<Value> {
        Ok(json!({
            "schedules": self.schedules(&caller.id)?,
            "localTime": Local::now().format("%Y-%m-%d %-I:%M %P (UTC%:z)").to_string(),
        }))
    }

    /// Creates a schedule, or with `id` changes only the fields given.
    pub fn schedule_tool(&self, caller: &Employee, args: &Value) -> Result<Value> {
        let id = args.get("id").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty());
        let mut d = match id {
            Some(id) => {
                let s = self.own_schedule(caller, id)?;
                ScheduleDraft {
                    id: Some(s.id),
                    employee_id: s.employee_id,
                    label: s.label,
                    cron: s.cron,
                    instruction: s.instruction,
                    enabled: s.enabled,
                    quiet: s.quiet,
                    model: s.model,
                    effort: s.effort,
                }
            }
            None => ScheduleDraft {
                id: None,
                employee_id: caller.id.clone(),
                label: String::new(),
                cron: String::new(),
                instruction: String::new(),
                enabled: true,
                quiet: false,
                model: None,
                effort: None,
            },
        };
        let text = |k: &str| args.get(k).and_then(|v| v.as_str());
        let flag = |k: &str| args.get(k).and_then(|v| v.as_bool());
        if let Some(v) = text("label") {
            d.label = v.to_string();
        }
        if let Some(v) = text("cron") {
            d.cron = v.to_string();
        }
        if let Some(v) = text("instruction") {
            d.instruction = v.to_string();
        }
        if let Some(v) = flag("quiet") {
            d.quiet = v;
        }
        if let Some(v) = flag("enabled") {
            d.enabled = v;
        }
        if let Some(v) = text("model") {
            d.model = Some(v.trim()).filter(|m| !m.is_empty() && *m != "default").map(String::from);
        }
        if let Some(v) = text("effort") {
            d.effort = effort_arg(v)?;
        }
        if d.cron.trim().is_empty() {
            return Err(AppError::invalid("cron is required"));
        }
        if d.instruction.trim().is_empty() {
            return Err(AppError::invalid("instruction is required: what you do each time it fires"));
        }
        let s = self.save_schedule(d)?;
        let when = match schedule::preview(&s.cron, now(), &Local) {
            CronPreview::Ok { description, .. } => description,
            CronPreview::Err { error } => error,
        };
        Ok(json!({ "schedule": s, "when": when }))
    }

    pub fn unschedule_tool(&self, caller: &Employee, id: &str) -> Result<Value> {
        let s = self.own_schedule(caller, id)?;
        self.delete_schedule(&s.id)?;
        Ok(json!({ "deleted": s.id }))
    }

    /// Flags the calling run for you: a desktop notification now, and its report in the Inbox.
    pub fn notify_tool(&self, caller_run: &str, caller: &Employee, message: &str) -> Result<Value> {
        let message = message.trim();
        if message.is_empty() {
            return Err(AppError::invalid("message is empty"));
        }
        let mut active = lock(&self.0.active);
        let a = active.get_mut(caller_run).ok_or_else(|| AppError::invalid("this run has ended"))?;
        a.needs_you.push(message.to_string());
        drop(active);
        self.sink().notify(&format!("{} needs you", caller.name), message);
        Ok(json!({ "notified": true }))
    }

    /// Your Claude Code skill catalog, for choosing the skills a proposed hire needs.
    pub fn catalog_tool(&self) -> Result<Value> {
        let skills: Vec<Value> = self
            .skill_catalog()
            .iter()
            .map(|s| json!({ "name": s.name, "description": claude::clip(&s.description, 300), "source": s.source, "skills": s.skills }))
            .collect();
        Ok(json!({ "skills": skills }))
    }

    /// Records a hire the caller proposes. The hire reports to the caller unless they name one of
    /// their direct reports; skills are matched against your catalog by name.
    pub fn propose_hire_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).unwrap_or("").to_string();
        let (name, title, reason) = (s("name"), s("title"), s("reason"));
        if title.is_empty() {
            return Err(AppError::invalid("title is required: the role you want hired"));
        }
        if name.is_empty() {
            return Err(AppError::invalid("name is required: suggest a first name for the new hire"));
        }
        if reason.is_empty() {
            return Err(AppError::invalid("reason is required: the gap this hire fills and what they would own"));
        }
        let asked: Vec<String> = args
            .get("skills")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default();
        let catalog = self.skill_catalog();
        let (mut keys, mut missing) = (vec![], vec![]);
        for n in &asked {
            match catalog.iter().find(|c| c.name.eq_ignore_ascii_case(n) || c.id.eq_ignore_ascii_case(n)) {
                Some(c) if !keys.contains(&c.id) => keys.push(c.id.clone()),
                Some(_) => {}
                None => missing.push(n.clone()),
            }
        }
        let (proposal, lane, ceo) = {
            let db = self.db();
            let to = s("reportsTo");
            let manager_id = if to.is_empty() || find_member(std::slice::from_ref(caller), &to, |e| (&e.id, Some(&e.slug), &e.name)).is_some() {
                caller.id.clone()
            } else {
                let reports: Vec<Employee> = db.employees()?.into_iter().filter(|e| e.manager_id.as_deref() == Some(&caller.id)).collect();
                find_member(&reports, &to, |e| (&e.id, Some(&e.slug), &e.name)).map(|e| e.id.clone()).ok_or_else(|| {
                    AppError::invalid(format!("'{to}' is neither you nor one of your direct reports. Leave reportsTo out and the hire reports to you."))
                })?
            };
            if let Some(dup) = db.proposals(Some(&caller.id), Some(ProposalState::Open))?.into_iter().find(|p| p.draft.title.eq_ignore_ascii_case(&title)) {
                return Err(AppError::conflict(format!(
                    "You already proposed a {} (proposal `{}`) and it is waiting on a decision. To change it, withdraw_proposal it and propose again.",
                    dup.draft.title, dup.id
                )));
            }
            let hue = hue_from_name(&name);
            let p = HireProposal {
                id: uuid::Uuid::new_v4().to_string(),
                employee_id: caller.id.clone(),
                run_id: Some(caller_run.to_string()),
                reason,
                draft: EmployeeDraft {
                    name,
                    title,
                    description: s("description"),
                    objective: s("objective"),
                    manager_id: Some(manager_id),
                    skills: SkillGrant { builtins: false, keys },
                    clearance: Clearance::Supervised,
                    allow_rules: vec![],
                    model: None,
                    effort: None,
                    browser: false,
                    auto_tidy: true,
                    workspaces: vec![],
                    hue,
                },
                missing_skills: missing,
                state: ProposalState::Open,
                note: None,
                hired_id: None,
                created_at: now(),
                resolved_at: None,
            };
            db.save_proposal(&p)?;
            let lane = db.run(caller_run)?.map(|r| r.run.kind.lane());
            (p, lane, ceo_of(&db)?)
        };
        let d = &proposal.draft;
        self.append_event(caller_run, RunEvent::ProposedHire { proposal_id: proposal.id.clone(), name: d.name.clone(), title: d.title.clone() });
        self.sink().proposal_updated(&proposal);
        if lane != Some(Lane::Chat) {
            self.sink().notify(&format!("{} proposes a hire", caller.name), &format!("{}, {}", d.name, d.title));
        }
        let mut note = format!(
            "Recorded. {ceo} reviews it in their Inbox, may change it, and approves or declines it. Their decision comes back to \
you as a new run. Never create the hire any other way."
        );
        if !proposal.missing_skills.is_empty() {
            note += &format!(
                " Not in the catalog, so not granted: {}. Say what the hire needs them for; {ceo} can install them.",
                proposal.missing_skills.join(", ")
            );
        }
        Ok(json!({ "proposal": proposal, "note": note }))
    }

    pub fn my_proposals(&self, caller: &Employee) -> Result<Value> {
        let v = self.db().proposals(Some(&caller.id), None)?;
        Ok(json!({ "proposals": v.into_iter().take(20).collect::<Vec<_>>() }))
    }

    pub fn withdraw_proposal_tool(&self, caller: &Employee, id: &str) -> Result<Value> {
        let p = {
            let db = self.db();
            let mut p = match db.proposal(id)? {
                Some(p) if p.employee_id == caller.id => p,
                _ => return Err(AppError::not_found(format!("'{id}' is not one of your proposals. Call my_proposals() for the list."))),
            };
            if p.state != ProposalState::Open {
                return Err(AppError::conflict(format!("that proposal is already {}", p.state.as_str())));
            }
            p.state = ProposalState::Withdrawn;
            p.resolved_at = Some(now());
            db.save_proposal(&p)?;
            p
        };
        self.sink().proposal_updated(&p);
        Ok(json!({ "withdrawn": p.id }))
    }
}

/// The note `delegate` and `ask_colleague` return: the hand-off comes back either way, so wait only
/// for a quick one.
fn handoff_note(name: &str, what: &str, busy: bool, chat: bool) -> String {
    let wait = if busy { format!("{name} is busy with other work, so this starts when that finishes. ") } else { String::new() };
    let turn = if chat { ", as your next turn in this chat" } else { "" };
    format!(
        "{wait}{what} comes back to you either way. Wait for it with await_reports only if it should take a few minutes and you need it \
before you finish. Otherwise finish now: Omega Star starts a new run for you when it arrives{turn}."
    )
}

/// The answers and reports `ended` lets through: its finished Asked and Delegated children, and itself
/// when it was one, whose asker's run has finished too, each with the session it runs in. Each is
/// stamped delivered as it is found, so a second call finds nothing.
pub(super) fn due_answers(db: &Store, ended: &Run) -> Result<Vec<(Run, String)>> {
    let ceo = ceo_of(db)?;
    let mut ids: Vec<String> = db.children(&ended.id)?.into_iter().map(|r| r.id).collect();
    if ended.parent_run_id().is_some() {
        ids.push(ended.id.clone());
    }
    let mut due = vec![];
    for id in ids {
        let Some(mut c) = db.run(&id)?.map(|d| d.run) else { continue };
        let (parent_run_id, task) = match &c.kind {
            RunKind::Asked { parent_run_id, task, .. } => (parent_run_id.clone(), *task),
            RunKind::Delegated { parent_run_id } => (parent_run_id.clone(), true),
            _ => continue,
        };
        if c.delivered_at.is_some() || !c.state.is_finished() {
            continue;
        }
        let Some(parent) = db.run(&parent_run_id)? else { continue };
        if !parent.run.state.is_finished() {
            continue; // still working: it collects with await_reports, or gets it when it ends
        }
        c.delivered_at = Some(now());
        db.save_run(&c)?;
        if c.state == RunState::Cancelled {
            continue; // you stopped it
        }
        let report = match (c.state, c.summary.as_deref()) {
            (RunState::Succeeded, Some(s)) => s.to_string(),
            (RunState::Succeeded, None) => "(no report)".to_string(),
            (state, _) => format!("They did not finish ({}): {}", state.as_str(), c.error.as_deref().unwrap_or("no details")),
        };
        if let Some(d) = answered_run(db, &ceo, &c, &parent, task, &report, None)? {
            due.push(d);
        }
    }
    Ok(due)
}

/// The run that brings `handed`'s answer or report (`report`) to whoever handed it over, from
/// `parent`, and the session it runs in. A chat turn's answer is the asker's next turn in their
/// current chat; a work run's resumes that run's session, which still holds everything it knew. It
/// quotes where the asker's run started only when it does not resume it. None when the asker has left.
fn answered_run(db: &Store, ceo: &str, handed: &Run, parent: &RunRecord, task: bool, report: &str, note: Option<&str>) -> Result<Option<(Run, String)>> {
    let Some(asker) = db.employee(&parent.run.employee_id)? else { return Ok(None) };
    let from = db.employee(&handed.employee_id)?.map(|e| e.name).unwrap_or_else(|| "A former colleague".into());
    let chat = parent.run.kind.lane() == Lane::Chat;
    let session = if chat { asker.chat_session.clone() } else { parent.session.clone() };
    let resumes = session == parent.session && db.session_started(&session)?;
    let (did, yours, theirs) =
        if task { ("finished the task you handed them", "Your task", "Their report") } else { ("answered your question", "Your question", "Their answer") };
    let mut prompt = format!("{from} {did}.");
    if let Some(n) = note {
        prompt += &format!("\n\n{n}");
    }
    if !resumes {
        let origin = match parent.run.kind {
            RunKind::Chat => format!("You were talking with {ceo}, whose message was:"),
            _ => "Your run had started from:".to_string(),
        };
        prompt += &format!("\n\n{origin}\n\n{}", claude::clip(&parent.run.prompt, 2000));
    }
    prompt += &format!("\n\n{yours}:\n\n{}\n\n{theirs}:\n\n{}", handed.prompt, claude::clip(report, 20000));
    let kind = RunKind::Answered { asked_run_id: handed.id.clone(), from: handed.employee_id.clone(), task, chat };
    let mut run = Run::new(&asker.id, kind, prompt, now());
    run.omega_wrote = true;
    Ok(Some((run, session)))
}

/// Where a hand-off began, read from the store under its lock; `source_text` turns it into one line
/// from the transcripts once the lock is released.
pub(super) struct SourceChain {
    ceo: String,
    /// Whoever `from` belongs to, who sent the hand-off on.
    sender: String,
    /// The hand-off runs walked back through, nearest first, with their employees' names.
    handoffs: Vec<(Run, String)>,
    /// The first run that did not come from a hand-off, with its employee's name; None when it is gone.
    origin: Option<(Run, String)>,
    /// For a chat origin: your last chat turns there up to and including it, oldest first.
    turns: Vec<Run>,
    /// For a hire decision: your note on the proposal.
    note: Option<String>,
    /// For a follow-up: the person who replied.
    person: Option<String>,
}

/// Walks back from the run `from` (the upstream of a hand-off) to the run the work began in.
pub(super) fn source_chain(db: &Store, ceo: &str, from: &str) -> Result<SourceChain> {
    let mut chain = SourceChain { ceo: ceo.to_string(), sender: String::new(), handoffs: vec![], origin: None, turns: vec![], note: None, person: None };
    let mut seen = HashSet::new();
    let mut next = Some(from.to_string());
    for _ in 0..SOURCE_HOPS {
        let Some(id) = next.take().filter(|id| seen.insert(id.clone())) else { break };
        let Some(rec) = db.run(&id)? else { break };
        let name = db.employee(&rec.run.employee_id)?.map(|e| e.name).unwrap_or_else(|| "a former colleague".into());
        if chain.sender.is_empty() {
            chain.sender = name.clone();
        }
        if let Some(up) = rec.run.upstream() {
            next = Some(up.to_string());
            chain.handoffs.push((rec.run, name));
            continue;
        }
        match &rec.run.kind {
            RunKind::Chat => {
                let mut turns: Vec<Run> = db
                    .chat_runs(&rec.run.employee_id, &rec.session)?
                    .into_iter()
                    .filter(|t| t.created_at <= rec.run.created_at && matches!(t.kind, RunKind::Chat) && !t.omega_wrote)
                    .collect();
                chain.turns = turns.split_off(turns.len().saturating_sub(SOURCE_TURNS));
            }
            RunKind::HireDecided { proposal_id, .. } => chain.note = db.proposal(proposal_id)?.and_then(|p| p.note),
            RunKind::FollowUp { person_id, .. } => {
                chain.person = Some(db.person(person_id)?.map(|p| p.name).unwrap_or_else(|| "a former member".into()))
            }
            _ => {}
        }
        chain.origin = Some((rec.run, name));
        break;
    }
    if chain.sender.is_empty() {
        chain.sender = "the colleague who sent it".into();
    }
    Ok(chain)
}

/// One line on where a hand-off began, quoting your own words where there are some, with what you
/// wrote into its runs on the way. Reads the runs' transcripts under `runs_dir`.
pub(super) fn source_text(chain: &SourceChain, runs_dir: &Path) -> String {
    let ceo = &chain.ceo;
    let steered = |id: &str| claude::steered(&fs::read_to_string(runs_dir.join(id).join("transcript.jsonl")).unwrap_or_default());
    let quote = |s: &str| format!("> \"{}\"", claude::clip(&claude::one_line(s), 600));
    let missing = format!("Where this began is no longer on record, so treat it as {}'s own request.", chain.sender);
    let mut text = match &chain.origin {
        None => missing,
        Some((r, _)) if r.omega_wrote => format!(
            "This began with a message Omega Star wrote in the app from something {ceo} did there. It is not {ceo}'s own words, except where it quotes them:\n{}",
            quote(&r.prompt)
        ),
        Some((r, employee)) => match &r.kind {
            RunKind::Chat => {
                let mut t = format!("This began in {ceo}'s chat with {employee}. {ceo}'s last messages there, oldest first:");
                for turn in &chain.turns {
                    let words: Vec<String> = std::iter::once(turn.prompt.clone()).chain(steered(&turn.id)).collect();
                    t += &format!("\n{}", quote(&words.join(" ")));
                }
                t
            }
            RunKind::Manual if r.prompt == DEFAULT_PROMPT => format!("This began when {ceo} started a run for {employee} from the app, with no instructions."),
            RunKind::Manual | RunKind::Task { .. } | RunKind::Reply { .. } => format!("This began with {ceo}'s own message to {employee}:\n{}", quote(&r.prompt)),
            RunKind::HireDecided { .. } => {
                let mut t = format!("This began with {ceo}'s decision on a hire {employee} proposed; Omega Star wrote the message.");
                if let Some(n) = &chain.note {
                    t += &format!(" {ceo}'s note: \"{}\"", claude::clip(&claude::one_line(n), 600));
                }
                t
            }
            RunKind::Scheduled { label, .. } => format!("This began with {employee}'s scheduled '{label}', not a message from {ceo}."),
            RunKind::FollowUp { .. } => format!(
                "This began with {}'s reply to a request from {employee}, not a message from {ceo}.",
                chain.person.as_deref().unwrap_or("a former member")
            ),
            RunKind::Tidy { .. } | RunKind::Organize { .. } | RunKind::ReplyCheck { .. } | RunKind::Nudge { .. } | RunKind::Unanswered { .. } => {
                format!("This began with {employee}'s {}, which Omega Star started, not a message from {ceo}.", claude::kind_label(&r.kind))
            }
            // A hand-off is never an origin: the walk goes on past it.
            RunKind::Delegated { .. } | RunKind::Asked { .. } | RunKind::Answered { .. } => missing,
        },
    };
    for (r, employee) in chain.handoffs.iter().rev() {
        let said = steered(&r.id);
        if !said.is_empty() {
            text += &format!("\n{ceo} also wrote into {employee}'s run on the way:");
            for s in said {
                text += &format!("\n{}", quote(&s));
            }
        }
    }
    text
}

/// The avatar hue the hire sheet derives from a name (`hueFromName` in EmployeeFields.tsx).
fn hue_from_name(name: &str) -> u16 {
    let n = name.trim().to_lowercase();
    if n.is_empty() {
        return 250;
    }
    n.encode_utf16().fold(0u32, |h, c| (h * 31 + c as u32) % 360) as u16
}

fn parse_due(s: &str) -> Result<Ts> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(t.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(|d| d.and_hms_opt(12, 0, 0).unwrap().and_utc())
        .map_err(|_| AppError::invalid("dueAt must be an ISO date or date-time"))
}

/// The schedule tool's `effort`: a level, or 'default' (or nothing) to go back to the employee's own.
fn effort_arg(v: &str) -> Result<Option<Effort>> {
    match v.trim() {
        "" | "default" => Ok(None),
        v => serde_json::from_value(json!(v)).map(Some).map_err(|_| AppError::invalid("effort must be low, medium, high, xhigh or max")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Office0 {
        dir: PathBuf,
        db: Store,
    }

    impl Drop for Office0 {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// A temp store whose CEO is called Pat.
    fn office() -> Office0 {
        let dir = std::env::temp_dir().join(format!("omegastar-tools-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("runs")).unwrap();
        let db = Store::open(&dir.join("office.db")).unwrap();
        db.set_company(&Company { name: "Acme".into(), ceo_name: "Pat".into(), ..Company::unset() }).unwrap();
        Office0 { dir, db }
    }

    /// An employee reporting to `manager_id`, or to you.
    fn employee(db: &Store, name: &str, manager_id: Option<&str>) -> Employee {
        let e = Employee {
            id: uuid::Uuid::new_v4().to_string(),
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
            workspaces: vec![],
            hue: 0,
            chat_session: format!("chat-{name}"),
            hired_at: now(),
        };
        db.save_employee(&e).unwrap();
        e
    }

    /// Saves a run of `e` in `state`, created `ago` minutes back, in `session` (its own id when None).
    fn run(db: &Store, e: &Employee, kind: RunKind, prompt: &str, state: RunState, ago: i64, session: Option<&str>) -> Run {
        let at = now() - chrono::Duration::minutes(ago);
        let mut r = Run::new(&e.id, kind, prompt.into(), at);
        r.state = state;
        if state.is_finished() {
            r.ended_at = Some(at);
            r.summary = Some(format!("{} did it.", e.name)).filter(|_| state == RunState::Succeeded);
        }
        let session = session.map(String::from).unwrap_or_else(|| r.id.clone());
        assert!(db.insert_run(&r, &session).unwrap());
        r
    }

    fn delegated(parent: &Run) -> RunKind {
        RunKind::Delegated { parent_run_id: parent.id.clone() }
    }

    fn asked(parent: &Run, task: bool) -> RunKind {
        RunKind::Asked { parent_run_id: parent.id.clone(), asked_by: parent.employee_id.clone(), task }
    }

    fn delivered(db: &Store, r: &Run) -> bool {
        db.run(&r.id).unwrap().unwrap().run.delivered_at.is_some()
    }

    #[test]
    fn a_late_report_comes_home_once() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let parent = run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Succeeded, 30, None);
        o.db.mark_inited(&parent.id).unwrap();
        let child = run(&o.db, &bo, delegated(&parent), "Count the stock.", RunState::Succeeded, 20, None);

        let due = due_answers(&o.db, &child).unwrap();
        assert_eq!(due.len(), 1);
        let (a, session) = &due[0];
        assert_eq!(a.employee_id, ada.id);
        assert_eq!(a.kind, RunKind::Answered { asked_run_id: child.id.clone(), from: bo.id.clone(), task: true, chat: false });
        assert_eq!(session, &parent.id, "it resumes the manager's session");
        assert!(a.omega_wrote);
        assert!(a.prompt.starts_with("Bo finished the task you handed them.\n\nYour task:\n\nCount the stock.\n\nTheir report:\n\nBo did it."), "{}", a.prompt);
        assert!(delivered(&o.db, &child));
        assert!(due_answers(&o.db, &child).unwrap().is_empty(), "delivered once");
        assert!(due_answers(&o.db, &parent).unwrap().is_empty());
    }

    #[test]
    fn a_report_waits_for_its_manager_to_finish() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let mut parent = run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Running, 30, None);
        let child = run(&o.db, &bo, delegated(&parent), "Count the stock.", RunState::Succeeded, 20, None);
        assert!(due_answers(&o.db, &child).unwrap().is_empty(), "the manager can still collect it");
        assert!(!delivered(&o.db, &child));

        parent.state = RunState::Succeeded;
        parent.ended_at = Some(now());
        o.db.save_run(&parent).unwrap();
        let due = due_answers(&o.db, &parent).unwrap();
        assert_eq!(due.len(), 1);
        assert!(matches!(&due[0].0.kind, RunKind::Answered { asked_run_id, .. } if *asked_run_id == child.id));
    }

    #[test]
    fn a_stopped_or_collected_report_is_not_brought_back() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let parent = run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Succeeded, 30, None);
        let stopped = run(&o.db, &bo, delegated(&parent), "Count the stock.", RunState::Cancelled, 20, None);
        assert!(due_answers(&o.db, &stopped).unwrap().is_empty());
        assert!(delivered(&o.db, &stopped), "stamped all the same");

        let mut collected = run(&o.db, &bo, delegated(&parent), "Count the till.", RunState::Succeeded, 20, None);
        collected.delivered_at = Some(now());
        o.db.save_run(&collected).unwrap();
        assert!(due_answers(&o.db, &collected).unwrap().is_empty());
        assert!(due_answers(&o.db, &parent).unwrap().is_empty());
    }

    #[test]
    fn a_chat_turns_report_comes_back_as_its_next_chat_turn() {
        let o = office();
        let mut ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        // You talked with Ada in an older chat, and have started a new one since.
        let parent = run(&o.db, &ada, RunKind::Chat, "Ask Bo for the numbers.", RunState::Succeeded, 30, Some("chat-old"));
        o.db.mark_inited(&parent.id).unwrap();
        let mut child = run(&o.db, &bo, asked(&parent, false), "What are the numbers?", RunState::Succeeded, 20, None);
        child.needs_you = vec!["Look at this.".into()];
        o.db.save_run(&child).unwrap();

        let due = due_answers(&o.db, &child).unwrap();
        let (a, session) = &due[0];
        assert_eq!(a.kind, RunKind::Answered { asked_run_id: child.id.clone(), from: bo.id.clone(), task: false, chat: true });
        assert_eq!(session, &ada.chat_session, "the chat Ada is in now");
        assert!(a.kind.is_conversation());
        assert_eq!(a.kind.lane(), Lane::Chat);
        assert!(a.prompt.starts_with("Bo answered your question.\n\nYou were talking with Pat, whose message was:\n\nAsk Bo for the numbers."), "{}", a.prompt);

        assert!(o.db.insert_run(a, session).unwrap());
        assert!(o.db.chat_runs(&ada.id, &ada.chat_session).unwrap().iter().any(|r| r.id == a.id));
        assert!(!o.db.conversation_runs().unwrap().iter().any(|r| r.id == a.id));
        let rows = Office::rows(&o.db, false).unwrap();
        let filed: Vec<&str> = rows.iter().flat_map(|t| std::iter::once(t).chain(&t.related)).map(|t| t.root.id.as_str()).collect();
        assert!(filed.contains(&child.id.as_str()), "the flagged answer has its row: {filed:?}");
        assert!(!filed.contains(&a.id.as_str()), "the chat turn is in no row");

        // In the same chat, the turn resumes it and quotes nothing again.
        ada.chat_session = "chat-old".into();
        o.db.save_employee(&ada).unwrap();
        let again = run(&o.db, &bo, asked(&parent, false), "And the totals?", RunState::Succeeded, 10, None);
        let (a, session) = due_answers(&o.db, &again).unwrap().remove(0);
        assert_eq!(session, "chat-old");
        assert!(!a.prompt.contains("You were talking with"), "{}", a.prompt);
    }

    #[test]
    fn a_report_quotes_where_its_run_started_only_when_it_does_not_resume_it() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let never_started = run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Failed, 30, None);
        let child = run(&o.db, &bo, delegated(&never_started), "Count the stock.", RunState::Succeeded, 20, None);
        let (a, session) = due_answers(&o.db, &child).unwrap().remove(0);
        assert_eq!(session, never_started.id);
        assert!(a.prompt.contains("\n\nYour run had started from:\n\nPlan the launch.\n\nYour task:"), "{}", a.prompt);

        let came = RunKind::Answered { asked_run_id: child.id.clone(), from: bo.id.clone(), task: true, chat: false };
        let answered = run(&o.db, &ada, came, "Bo finished the task you handed them.", RunState::Succeeded, 10, None);
        let child = run(&o.db, &bo, asked(&answered, true), "Count the till.", RunState::Failed, 5, None);
        let (a, _) = due_answers(&o.db, &child).unwrap().remove(0);
        assert!(a.prompt.contains("\n\nYour run had started from:\n\nBo finished the task"), "{}", a.prompt);
        assert!(a.prompt.contains("Their report:\n\nThey did not finish (failed): no details"), "{}", a.prompt);
    }

    #[test]
    fn older_runs_load_without_the_new_fields() {
        let kind: RunKind = serde_json::from_value(json!({ "type": "answered", "askedRunId": "r1", "from": "e1", "task": true })).unwrap();
        assert_eq!(kind, RunKind::Answered { asked_run_id: "r1".into(), from: "e1".into(), task: true, chat: false });
        let mut v = serde_json::to_value(Run::new("e1", RunKind::Manual, "go".into(), now())).unwrap();
        v.as_object_mut().unwrap().remove("omegaWrote");
        let r: Run = serde_json::from_value(v).unwrap();
        assert!(!r.omega_wrote);
    }

    #[test]
    fn a_handoff_note_says_when_to_wait() {
        let plain = handoff_note("Bo", "Their report", false, false);
        assert_eq!(
            plain,
            "Their report comes back to you either way. Wait for it with await_reports only if it should take a few minutes and you need it \
before you finish. Otherwise finish now: Omega Star starts a new run for you when it arrives."
        );
        let both = handoff_note("Bo", "The answer", true, true);
        assert!(both.starts_with("Bo is busy with other work, so this starts when that finishes. The answer comes back"));
        assert!(both.ends_with("when it arrives, as your next turn in this chat."));
        assert!(!plain.contains('\u{2014}') && !both.contains('\u{2014}'));
    }

    /// Writes your messages into a run's transcript, as if steered into it while it worked.
    fn steer(o: &Office0, r: &Run, said: &[&str]) {
        let dir = o.dir.join("runs").join(&r.id);
        fs::create_dir_all(&dir).unwrap();
        let lines: Vec<String> = said.iter().map(|t| claude::omega_line(&RunEvent::User { text: t.to_string() })).collect();
        fs::write(dir.join("transcript.jsonl"), lines.join("\n") + "\n").unwrap();
    }

    fn source(o: &Office0, from: &Run) -> String {
        source_text(&source_chain(&o.db, "Pat", &from.id).unwrap(), &o.dir.join("runs"))
    }

    #[test]
    fn a_handoff_from_a_chat_quotes_the_chat() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let chat = Some(ada.chat_session.as_str());
        let turn = |prompt: &str, ago: i64| run(&o.db, &ada, RunKind::Chat, prompt, RunState::Succeeded, ago, chat);
        turn("First, hello.", 50);
        turn("Second, the plan.", 45);
        turn("Third, the budget.", 40);
        let came = RunKind::Answered { asked_run_id: "elsewhere".into(), from: bo.id.clone(), task: false, chat: true };
        run(&o.db, &ada, came, "Bo answered your question.", RunState::Succeeded, 38, chat);
        let delegating = turn("Fourth, get Bo to count the stock.", 35);
        turn("Fifth, after it.", 10);
        steer(&o, &delegating, &["Only the warehouse."]);

        let d = run(&o.db, &bo, delegated(&delegating), "Count the stock.", RunState::Running, 30, None);
        steer(&o, &d, &["Use last week's sheet."]);
        let text = source(&o, &d);
        assert_eq!(
            text,
            "This began in Pat's chat with Ada. Pat's last messages there, oldest first:\n\
> \"Second, the plan.\"\n\
> \"Third, the budget.\"\n\
> \"Fourth, get Bo to count the stock. Only the warehouse.\"\n\
Pat also wrote into Bo's run on the way:\n\
> \"Use last week's sheet.\""
        );
    }

    #[test]
    fn a_handoff_says_when_it_did_not_begin_with_your_words() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let from = |origin: &Run| {
            let d = run(&o.db, &bo, delegated(origin), "Count the stock.", RunState::Running, 0, None);
            source(&o, &d)
        };

        let sweep = RunKind::Scheduled { schedule_id: "s1".into(), slot: now(), label: "Sweep".into(), quiet: true, model: None, effort: None };
        let scheduled = run(&o.db, &ada, sweep, "Sweep the inbox.", RunState::Running, 5, None);
        assert_eq!(from(&scheduled), "This began with Ada's scheduled 'Sweep', not a message from Pat.");

        let root = run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Succeeded, 60, None);
        assert_eq!(from(&root), "This began with Pat's own message to Ada:\n> \"Plan the launch.\"");
        let reply = RunKind::Reply { to_run_id: root.id.clone(), label: "Manual run".into(), task: false };
        let mut approval = Run::new(&ada.id, reply, "Pat approved what was blocked:\n- Bash: npm publish".into(), now());
        approval.omega_wrote = true;
        o.db.insert_run(&approval, &root.id).unwrap();
        assert_eq!(
            from(&approval),
            "This began with a message Omega Star wrote in the app from something Pat did there. It is not Pat's own words, except where it \
quotes them:\n> \"Pat approved what was blocked: - Bash: npm publish\""
        );

        let bare = run(&o.db, &ada, RunKind::Manual, DEFAULT_PROMPT, RunState::Running, 5, None);
        assert_eq!(from(&bare), "This began when Pat started a run for Ada from the app, with no instructions.");

        let lost = run(&o.db, &bo, RunKind::Delegated { parent_run_id: "gone".into() }, "Count the stock.", RunState::Running, 0, None);
        assert_eq!(source(&o, &lost), "Where this began is no longer on record, so treat it as Bo's own request.");
    }

    #[test]
    fn schedule_effort_takes_a_level_or_default() {
        assert_eq!(effort_arg("low").unwrap(), Some(Effort::Low));
        assert_eq!(effort_arg(" xhigh ").unwrap(), Some(Effort::Xhigh));
        assert_eq!(effort_arg("default").unwrap(), None);
        assert_eq!(effort_arg("quick").unwrap_err().message, "effort must be low, medium, high, xhigh or max");
    }
}
