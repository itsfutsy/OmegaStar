//! The office tools employees call over MCP (dispatched from `mcp.rs`).

use super::*;

/// Longest chain of runs (delegations and questions) a new question may extend.
const ASK_DEPTH: usize = 6;
const AWAIT_LIMIT: Duration = Duration::from_secs(300);

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
        let reports: Vec<Employee> = self.db().employees()?.into_iter().filter(|e| e.manager_id.as_deref() == Some(&caller.id)).collect();
        let report = find_member(&reports, to, |e| (&e.id, Some(&e.slug), &e.name)).ok_or_else(|| {
            AppError::invalid(format!("'{to}' is not one of your direct reports. Call team() for the list."))
        })?;
        let run = Run::new(&report.id, RunKind::Delegated { parent_run_id: caller_run.to_string() }, task.trim().to_string(), now());
        let session = run.id.clone();
        let run = self.enqueue(run, session)?;
        self.append_event(
            caller_run,
            RunEvent::Delegated { child_run_id: run.id.clone(), employee_id: report.id.clone(), task: run.prompt.clone() },
        );
        Ok(json!({ "runId": run.id, "employee": report.name, "state": run.state }))
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
        let (colleague, busy) = {
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
            (colleague, busy)
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
        let what = if task { "Their report" } else { "The answer" };
        let wait = if busy { format!("{} is busy with other work, so this starts when that finishes. ", colleague.name) } else { String::new() };
        let note = format!(
            "{wait}{what} comes back to you either way: collect it with await_reports if you need it now, or finish \
without it and Omega Star starts a new run for you when it arrives."
        );
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
                let note = if done { "All reports are in." } else { "Some runs are still going: call await_reports again for those." };
                return Ok(json!({ "reports": reports, "note": note }));
            }
            let _ = tokio::time::timeout_at(deadline, rx.changed()).await;
        }
    }

    /// Records that these finished colleague answers reached their asker.
    fn mark_delivered<'a>(&self, run_ids: impl Iterator<Item = &'a str>) -> Result<()> {
        let db = self.db();
        for id in run_ids {
            if let Some(mut r) = db.run(id)?.map(|d| d.run) {
                if matches!(r.kind, RunKind::Asked { .. }) && r.delivered_at.is_none() {
                    r.delivered_at = Some(now());
                    db.save_run(&r)?;
                }
            }
        }
        Ok(())
    }

    /// Brings every finished colleague answer whose asker's run ended without collecting it to the
    /// asker, as an Answered run. Called whenever a run ends, since either side can end last; the
    /// check and `delivered_at` happen under one lock, so each answer is delivered exactly once.
    pub(super) fn deliver_answers(&self, ended: &Run) -> Result<()> {
        let due = {
            let db = self.db();
            let ceo = db.company()?.map(|c| c.ceo_name).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "the person you work for".into());
            let mut ids: Vec<String> = db.children(&ended.id)?.into_iter().map(|r| r.id).collect();
            if ended.parent_run_id().is_some() {
                ids.push(ended.id.clone());
            }
            let mut due = vec![];
            for id in ids {
                let Some(mut c) = db.run(&id)?.map(|d| d.run) else { continue };
                let RunKind::Asked { parent_run_id, asked_by, task } = c.kind.clone() else { continue };
                if c.delivered_at.is_some() || !c.state.is_finished() {
                    continue;
                }
                let Some(parent) = db.run(&parent_run_id)? else { continue };
                if !parent.run.state.is_finished() {
                    continue; // still working: it collects with await_reports, or gets it when it ends
                }
                c.delivered_at = Some(now());
                db.save_run(&c)?;
                if c.state == RunState::Cancelled || db.employee(&asked_by)?.is_none() {
                    continue; // you stopped it, or the asker has left
                }
                let from = db.employee(&c.employee_id)?.map(|e| e.name).unwrap_or_else(|| "A former colleague".into());
                let (did, yours, theirs) = if task {
                    ("finished the task you handed them", "Your task", "Their report")
                } else {
                    ("answered your question", "Your question", "Their answer")
                };
                let reply = match (c.state, c.summary.as_deref()) {
                    (RunState::Succeeded, Some(s)) => s.to_string(),
                    (RunState::Succeeded, None) => "(no report)".to_string(),
                    (state, _) => format!("They did not finish ({}): {}", state.as_str(), c.error.as_deref().unwrap_or("no details")),
                };
                let origin = match parent.run.kind.lane() {
                    Lane::Chat => format!("You were talking with {ceo}, whose message was:"),
                    Lane::Work => "Your run had started from:".to_string(),
                };
                let prompt = format!(
                    "{from} {did}.\n\n{origin}\n\n{}\n\n{yours}:\n\n{}\n\n{theirs}:\n\n{}",
                    claude::clip(&parent.run.prompt, 2000),
                    c.prompt,
                    claude::clip(&reply, 20000)
                );
                let run = Run::new(&asked_by, RunKind::Answered { asked_run_id: c.id.clone(), from: c.employee_id.clone(), task }, prompt, now());
                // A work run's session still holds everything it knew; a chat's belongs to the chat lane.
                let session = if parent.run.kind.lane() == Lane::Work { parent.session.clone() } else { run.id.clone() };
                due.push((run, session));
            }
            due
        };
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

    #[test]
    fn schedule_effort_takes_a_level_or_default() {
        assert_eq!(effort_arg("low").unwrap(), Some(Effort::Low));
        assert_eq!(effort_arg(" xhigh ").unwrap(), Some(Effort::Xhigh));
        assert_eq!(effort_arg("default").unwrap(), None);
        assert_eq!(effort_arg("quick").unwrap_err().message, "effort must be low, medium, high, xhigh or max");
    }
}
