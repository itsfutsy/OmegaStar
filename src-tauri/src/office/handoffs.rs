//! Hand-offs between employees: delegating, asking a colleague, sharing news and awaiting reports; the loop guard that
//! paces them; and how their reports are held, brought home and caught up on, with where each hand-off began.

use super::*;

/// The loop guard: once a chain holds PAIR_HANDOFFS asks between the same two employees within PAIR_WINDOW, or
/// CHAIN_HANDOFFS hand-offs in all, the next hand-off waits PAUSE and goes as a chain of its own. Answers never count,
/// and delegated work counts only toward the chain: the daily cap on tasks covers it. Once it paused PAUSES_PER_DAY
/// asks between the same two employees in a day, it stops the next one between them.
const PAIR_HANDOFFS: usize = 6;
const PAIR_WINDOW: chrono::Duration = chrono::Duration::hours(2);
const CHAIN_HANDOFFS: usize = 40;
const PAUSE: chrono::Duration = chrono::Duration::hours(1);
const PAUSES_PER_DAY: u32 = 2;
/// Most runs the walk up a chain of hand-offs goes through, against a damaged trail.
const CHAIN_HOPS: usize = 64;
/// Most tasks a manager hands one report in a day.
pub(super) const TASKS_PER_DAY: u32 = 4;
/// Reports held for a manager reach them in one run once this long passes without another, and never later than
/// HOLD_MAX after the first.
const HOLD: chrono::Duration = chrono::Duration::minutes(30);
const HOLD_MAX: chrono::Duration = chrono::Duration::hours(2);
/// An answer resumes the asker's session only while that is under a day old and its context under this.
const FRESH_TOKENS: u64 = 120_000;
/// An answer whose run failed goes again this many more times, each after REDELIVER_AFTER.
const REDELIVERIES: u32 = 2;
const REDELIVER_AFTER: chrono::Duration = chrono::Duration::minutes(10);
/// Under Claude Code's 300 s idle abort, so a wait never races it.
const AWAIT_LIMIT: Duration = Duration::from_secs(240);
/// Most runs `source_chain` walks back through hand-offs.
const SOURCE_HOPS: usize = 12;
/// What `ask_colleague` takes as `to` for news to every colleague.
const EVERYONE: &str = "everyone";

impl Office {
    /// Hands a direct report a task. Refuses one who is working further up this chain (they would wait on each
    /// other) and a fifth task in a day, except in a conversation you are in, and pauses one the loop guard stops.
    pub fn delegate(&self, caller_run: &str, caller: &Employee, to: &str, task: &str) -> Result<Value> {
        if task.trim().is_empty() {
            return Err(AppError::invalid("task is empty"));
        }
        let at = now();
        let (report, run, ahead, conversation) = {
            let db = self.db();
            let reports: Vec<Employee> = db.employees()?.into_iter().filter(|e| e.reports_to(&caller.id)).collect();
            let report = find_employee(&reports, to)
                .ok_or_else(|| AppError::invalid(format!("'{to}' is not one of your direct reports. Call team() for the list.")))?
                .clone();
            let (up, chain) = chain_up(&db, caller_run)?;
            if waiting_up(&up, &report.id) {
                return Err(AppError::invalid(format!(
                    "{} is already waiting on you further up this chain, so they cannot take this now. Finish with what you have, and say in your report what you still need from them.",
                    report.name
                )));
            }
            let conversation = up.first().is_some_and(|r| r.conversation_root().is_some());
            if !conversation && db.delegated_since(&caller.id, &report.id, day_start(at))? >= TASKS_PER_DAY {
                return Err(AppError::invalid(format!(
                    "{} already has {TASKS_PER_DAY} tasks from you today. Put the rest in your plan for tomorrow.",
                    report.name
                )));
            }
            let mut run = Run::new(&report.id, RunKind::Delegated { parent_run_id: caller_run.to_string() }, task.trim().to_string(), at);
            guard(&db, &mut run, chain, caller, &report)?;
            let ahead = queue_ahead(&db, &run, at)?;
            (report, run, ahead, conversation)
        };
        let session = run.id.clone();
        let run = self.enqueue(run, session)?;
        self.append_event(
            caller_run,
            RunEvent::Delegated { child_run_id: run.id.clone(), employee_id: report.id.clone(), task: run.prompt.clone() },
        );
        let note = match run.not_before {
            Some(t) => paused_note(&report.name, "Their report", t),
            None => handoff_note(&report.name, "Their report", ahead, conversation, !conversation),
        };
        Ok(json!({ "runId": run.id, "employee": report.name, "state": run.state, "note": note }))
    }

    /// Asks any other agent one question, with kind "task" hands them a task, or with kind "info" leaves them
    /// news that starts no run (`share_news`). Refuses a colleague who is working further up this chain (they
    /// would wait on each other), adds to a request of the caller's they have not started yet, and pauses a
    /// hand-off the loop guard stops.
    pub fn ask_colleague(&self, caller_run: &str, caller: &Employee, to: &str, question: &str, kind: &str) -> Result<Value> {
        let task = kind == "task";
        if question.trim().is_empty() {
            return Err(AppError::invalid(if task { "task is empty" } else if kind == "info" { "the news is empty" } else { "question is empty" }));
        }
        if kind == "info" {
            return self.share_news(caller, to, question.trim());
        }
        if to.trim().eq_ignore_ascii_case(EVERYONE) {
            return Err(AppError::invalid("Only news (kind info) goes to everyone. Ask one colleague at a time."));
        }
        let at = now();
        let what = if task { "Their report" } else { "The answer" };
        let (colleague, run, folded, ahead, conversation) = {
            let db = self.db();
            let others: Vec<Employee> = db.employees()?.into_iter().filter(|e| e.id != caller.id).collect();
            let colleague = colleague_in(&others, to)?;
            let (up, chain) = chain_up(&db, caller_run)?;
            if waiting_up(&up, &colleague.id) {
                return Err(AppError::invalid(format!(
                    "{} is already waiting on you further up this chain, so they cannot answer now. Answer with what you have, and say in your report what you still need from them.",
                    colleague.name
                )));
            }
            let here = up.first().and_then(|r| r.conversation_root().map(String::from));
            // A request of the caller's they have not started yet takes this one too, when both came from the same
            // place, so one answer covers both and goes where the first would have.
            let earlier = db.active_runs_of(&colleague.id)?.into_iter().find(|r| {
                untouched(r)
                    && matches!(&r.kind, RunKind::Asked { asked_by, .. } if *asked_by == caller.id)
                    && r.parent_run_id().and_then(|p| inbox::run_of(&db, p)).and_then(|p| p.conversation_root().map(String::from)) == here
            });
            let (run, folded) = match earlier {
                Some(mut r) => {
                    r.prompt = format!("{}\n\nLater, {} added:\n\n{}", r.prompt, caller.name, question.trim());
                    if let RunKind::Asked { task: t, .. } = &mut r.kind {
                        *t |= task;
                    }
                    db.save_run(&r)?;
                    (r, true)
                }
                None => {
                    let kind = RunKind::Asked { parent_run_id: caller_run.to_string(), asked_by: caller.id.clone(), task };
                    let mut run = Run::new(&colleague.id, kind, question.trim().to_string(), at);
                    guard(&db, &mut run, chain, caller, &colleague)?;
                    (run, false)
                }
            };
            let ahead = queue_ahead(&db, &run, at)?;
            (colleague, run, folded, ahead, here.is_some())
        };
        let (run, note) = if folded {
            self.sink().run_updated(&run);
            let back = handoff_note(&colleague.name, what, ahead, conversation, false);
            (run, format!("{} had not started your earlier request yet, so Omega Star added this one to it, and one answer covers both. {back}", colleague.name))
        } else {
            let session = run.id.clone();
            let run = self.enqueue(run, session)?;
            let note = match run.not_before {
                Some(t) => paused_note(&colleague.name, what, t),
                None => handoff_note(&colleague.name, what, ahead, conversation, false),
            };
            (run, note)
        };
        self.append_event(
            caller_run,
            RunEvent::Asked { child_run_id: run.id.clone(), employee_id: colleague.id.clone(), question: question.trim().to_string(), task },
        );
        Ok(json!({ "runId": run.id, "colleague": colleague.name, "state": run.state, "note": note }))
    }

    /// Leaves news for one colleague, or with `to` "everyone" for every colleague: it starts no run and nothing
    /// comes back. Each reads it at the top of their next run (`catch_up`).
    fn share_news(&self, caller: &Employee, to: &str, text: &str) -> Result<Value> {
        let names: Vec<String> = {
            let db = self.db();
            let others: Vec<Employee> = db.employees()?.into_iter().filter(|e| e.id != caller.id).collect();
            let to = if to.trim().eq_ignore_ascii_case(EVERYONE) { others } else { vec![colleague_in(&others, to)?] };
            if to.is_empty() {
                return Err(AppError::invalid("You have no colleagues to tell."));
            }
            let at = now();
            db.tx(|| {
                for e in &to {
                    let news = News { id: uuid::Uuid::new_v4().to_string(), employee_id: e.id.clone(), from: caller.id.clone(), text: text.to_string(), created_at: at, read_at: None, read_in: None };
                    db.save_news(&news)?;
                }
                Ok(())
            })?;
            to.into_iter().map(|e| e.name).collect()
        };
        let who = match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
            _ => names.join(""),
        };
        let note = format!("Omega Star shows this to {who} at the top of their next run. It starts no run, and nothing comes back.");
        Ok(json!({ "told": names, "note": note }))
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
                    "Some runs are still going. Each comes back to you when it finishes, so you can finish now; call await_reports again only if you need it in this run."
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
    /// asker, as an Answered run, or holds it for them (`hold`), and queues a failed answer again. Called
    /// whenever a run ends, since either side can end last; the check and `delivered_at` happen under one
    /// lock, so each is delivered exactly once.
    pub(super) fn deliver_answers(&self, ended: &Run) -> Result<()> {
        let due = due_answers(&self.db(), ended)?;
        for (run, session) in due {
            self.enqueue(run, session)?;
        }
        Ok(())
    }
}

/// The note `delegate` and `ask_colleague` return: the hand-off comes back either way, so wait only for a quick
/// one. `ahead`: the runs ahead of it while its colleague is busy (`queue_ahead`). `conversation`: it was handed
/// over in a conversation you started, so it comes back there. `held`: a report held for your next run (`hold`).
fn handoff_note(name: &str, what: &str, ahead: usize, conversation: bool, held: bool) -> String {
    let wait = match ahead {
        0 => String::new(),
        1 => format!("{name} is busy with other work: 1 run is ahead of this one, so this starts when that finishes. "),
        n => format!("{name} is busy with other work: {n} runs are ahead of this one, so this starts when those finish. "),
    };
    let back = if held {
        format!(
            "Omega Star keeps it for your next run, or brings it with any other reports from your team in one run once {} minutes pass without a new one",
            HOLD.num_minutes()
        )
    } else if conversation {
        "Omega Star starts a new run for you when it arrives, as your next turn in this conversation".to_string()
    } else {
        "Omega Star starts a new run for you when it arrives".to_string()
    };
    format!(
        "{wait}{what} comes back to you either way. Wait for it with await_reports only if it should take a few minutes and you need it \
before you finish. Otherwise finish now: {back}."
    )
}

/// The note a hand-off the loop guard paused returns, in words that blame nobody: Omega Star paused it, not the colleague.
fn paused_note(name: &str, what: &str, at: Ts) -> String {
    format!(
        "Omega Star paused this hand-off to keep the chain short. It goes to {name} by itself at {}, and {} comes back to you as usual. \
Do not send it again or set a check for it: finish now.",
        at.with_timezone(&Local).format("%-I:%M %P"),
        what.to_lowercase()
    )
}

/// The runs up the chain of hand-offs from `run_id`, itself first, as far as the run a person or a schedule
/// started, and the chain a hand-off from it joins (`Run::chain`): the nearest one stored on the way, else that run.
fn chain_up(db: &Store, run_id: &str) -> Result<(Vec<Run>, String)> {
    let (mut up, mut chain): (Vec<Run>, Option<String>) = (vec![], None);
    let mut cur = db.run(run_id)?.map(|d| d.run);
    while let Some(r) = cur {
        if chain.is_none() {
            chain = r.chain.clone().or_else(|| r.upstream().is_none().then(|| r.id.clone()));
        }
        cur = match r.upstream() {
            Some(p) if up.len() < CHAIN_HOPS => db.run(p)?.map(|d| d.run),
            _ => None,
        };
        up.push(r);
    }
    let chain = chain.or_else(|| up.last().map(|r| r.id.clone())).unwrap_or_else(|| run_id.to_string());
    Ok((up, chain))
}

/// `employee_id` is still at work further up the chain `up`, waiting on its end: a hand-off to them would wait on itself.
fn waiting_up(up: &[Run], employee_id: &str) -> bool {
    up.iter().any(|r| !r.state.is_finished() && r.employee_id == employee_id)
}

/// The loop guard on a new hand-off `run` from `from` to `to` in the chain `chain`: it joins the chain, unless it is an
/// ask and the chain already holds PAIR_HANDOFFS asks between the same two employees within PAIR_WINDOW, or the chain
/// holds CHAIN_HANDOFFS hand-offs in all. Then it waits PAUSE and starts a chain of its own. An ask is refused, in words
/// that blame nobody, once PAUSES_PER_DAY asks between the two were paused today.
fn guard(db: &Store, run: &mut Run, chain: String, from: &Employee, to: &Employee) -> Result<()> {
    let ask = matches!(run.kind, RunKind::Asked { .. });
    if ask && db.paused_asks(&from.id, &to.id, day_start(run.created_at))? >= PAUSES_PER_DAY {
        return Err(AppError::invalid(format!(
            "Omega Star stopped this hand-off: {} and {} have passed this back and forth all day. Finish with what you have and say in your report what is still open.",
            from.name, to.name
        )));
    }
    let handoffs = db.chain_handoffs(&chain)?;
    let since = run.created_at - PAIR_WINDOW;
    let between = |a: &String, b: &String| (*a == from.id && *b == to.id) || (*a == to.id && *b == from.id);
    let pair = handoffs.iter().filter(|(a, b, at, delegated)| ask && !delegated && *at > since && between(a, b)).count();
    if pair < PAIR_HANDOFFS && handoffs.len() < CHAIN_HANDOFFS {
        run.chain = Some(chain);
    } else {
        run.chain = Some(run.id.clone());
        run.not_before = Some(run.created_at + PAUSE);
    }
    Ok(())
}

/// How many runs are ahead of `of` while its employee is busy: their background work running, and queued before
/// it and due. 0 when it starts without waiting: a question or an answer, which runs beside other work, or work
/// that finds one of its employee's lanes free (`runs::cap`).
fn queue_ahead(db: &Store, of: &Run, at: Ts) -> Result<usize> {
    if !runs::takes_a_lane(&of.kind) {
        return Ok(0);
    }
    let lanes = runs::cap(db)?;
    let work: Vec<Run> = db.active_runs_of(&of.employee_id)?.into_iter().filter(|r| r.id != of.id && runs::takes_a_lane(&r.kind)).collect();
    let ahead = work.iter().filter(|r| r.state == RunState::Running || (r.created_at <= of.created_at && r.not_before.is_none_or(|t| t <= at))).count();
    Ok(if ahead < lanes { 0 } else { ahead })
}

/// A queued run that never started, so what it was asked can still change.
fn untouched(r: &Run) -> bool {
    r.state == RunState::Queued && r.restarts == 0 && r.limit_retries == 0
}

/// The colleague `to` names: by id, slug or name, or by a first name only one of them has.
fn colleague_in(others: &[Employee], to: &str) -> Result<Employee> {
    let key = to.trim().to_lowercase();
    let by_first: Vec<&Employee> = others.iter().filter(|e| e.name.split_whitespace().next().map(str::to_lowercase) == Some(key.clone())).collect();
    find_employee(others, to)
        .or(if by_first.len() == 1 { Some(by_first[0]) } else { None })
        .cloned()
        .ok_or_else(|| AppError::invalid(format!("'{to}' is not a colleague. Call colleagues() for the list.")))
}

/// An employee's name, for a hand-off they may have left since.
fn colleague_name(db: &Store, id: &str) -> Result<String> {
    Ok(db.employee(id)?.map(|e| e.name).unwrap_or_else(|| "A former colleague".into()))
}

/// The start of the day `t` falls in, on your local calendar.
pub(super) fn day_start(t: Ts) -> Ts {
    let midnight = t.with_timezone(&Local).date_naive().and_time(chrono::NaiveTime::MIN);
    midnight.and_local_timezone(Local).earliest().map(|d| d.with_timezone(&Utc)).unwrap_or(t - chrono::Duration::days(1))
}

/// The employee's own work, which takes in the reports held for them: not a conversation with you, a tidy, or a
/// hand-off from someone else, which each do only what they were asked.
fn own_work(kind: &RunKind) -> bool {
    !kind.is_conversation() && !matches!(kind, RunKind::Tidy { .. } | RunKind::Asked { .. } | RunKind::Delegated { .. })
}

/// What came in for the employee since their last run, for this run's opening note: news from colleagues, now
/// read in this run (`News::read_in`); when the run is their own work (`own_work`), the reports held for them, which
/// it brings in place of their queued run (`Run::noted_in`); and their cards a newer card replaced. A tidy takes in
/// none of it.
pub(super) fn catch_up(db: &Store, employees: &[Employee], run: &Run, o: &mut claude::Opening) -> Result<()> {
    if !catches_up(&run.kind) {
        return Ok(());
    }
    let name = |id: &str| employees.iter().find(|e| e.id == id).map(|e| e.name.clone()).unwrap_or_else(|| "A former colleague".into());
    db.tx(|| {
        for mut n in db.unread_news(&run.employee_id)? {
            o.news.push((n.created_at, name(&n.from), n.text.clone()));
            n.read_at = Some(o.now);
            n.read_in = Some(run.id.clone());
            db.save_news(&n)?;
        }
        let batch = db.active_runs_of(&run.employee_id)?.into_iter().find(|r| r.id != run.id && untouched(r) && !r.reports.is_empty());
        if let Some(batch) = batch.filter(|_| own_work(&run.kind)) {
            for id in &batch.reports {
                let Some(mut h) = db.run(id)?.map(|r| r.run) else { continue };
                o.held.push((name(&h.employee_id), h.prompt.clone(), report_of(&h)));
                h.noted_in = Some(run.id.clone());
                db.save_run(&h)?;
            }
            db.delete_queued_run(&batch.id)?;
        }
        // Since their last run with a note started: it was told of anything replaced before then.
        let last = db
            .work_runs(&run.employee_id, 50)?
            .into_iter()
            .filter(|r| r.id != run.id && catches_up(&r.kind))
            .filter_map(|r| r.started_at)
            .max();
        if let Some(since) = last {
            o.replaced = db.replaced_cards(&run.employee_id, since)?;
        }
        Ok(())
    })
}

/// A run of this kind hears what came in since the last such run.
fn catches_up(kind: &RunKind) -> bool {
    runs::takes_opening(kind) && !matches!(kind, RunKind::Tidy { .. })
}

/// The answers and reports `ended` lets through: its finished Asked and Delegated children, and itself
/// when it was one, whose asker's run has finished too, each with the session it runs in. Each is
/// stamped delivered as it is found, so a second call finds nothing; a direct report's finished task is held
/// for its manager instead (`hold`). When `ended` failed or was lost, an answer it brought goes again
/// (`redeliver`); when it failed, was lost or you stopped it, the reports its opening note brought are held again.
pub(super) fn due_answers(db: &Store, ended: &Run) -> Result<Vec<(Run, String)>> {
    let ceo = ceo_of(db)?;
    let at = now();
    let mut ids: Vec<String> = db.children(&ended.id)?.into_iter().map(|r| r.id).collect();
    if ended.parent_run_id().is_some() {
        ids.push(ended.id.clone());
    }
    let mut due = vec![];
    // Your answer on the card of a colleague's ask or delegated work let them carry on: what came of it goes
    // to whoever handed it over, who already had their first report. Not when you stopped the reply.
    if let Some(RunKind::Reply { card: true, to_run_id, .. }) = (ended.state != RunState::Cancelled).then_some(&ended.kind) {
        if let Some(root) = db.run(to_run_id)?.map(|r| r.run).filter(|r| r.state.is_finished()) {
            let parent = match root.parent_run_id() {
                Some(p) => db.run(p)?.filter(|p| p.run.state.is_finished()),
                None => None,
            };
            if let Some(parent) = parent {
                let from = colleague_name(db, &root.employee_id)?;
                let answer = root.cards.iter().rev().find_map(|c| c.answer.as_ref()).map(|a| format!(" ({})", a.text)).unwrap_or_default();
                let note = format!("{ceo} answered {from}'s card{answer} and {from} carried on.");
                if let Some(d) = answered_run(db, &ceo, &root, &parent, &report_of(ended), Some(&note))? {
                    due.push(d);
                }
            }
        }
    }
    for id in ids {
        let Some(mut c) = db.run(&id)?.map(|d| d.run) else { continue };
        let Some(parent_run_id) = c.parent_run_id().map(String::from) else { continue };
        if c.delivered_at.is_some() || !c.state.is_finished() {
            continue;
        }
        let Some(parent) = db.run(&parent_run_id)? else { continue };
        if !parent.run.state.is_finished() {
            continue; // still working: it collects with await_reports, or gets it when it ends
        }
        c.delivered_at = Some(at);
        db.save_run(&c)?;
        if c.state == RunState::Cancelled {
            continue; // you stopped it
        }
        if held(db, &c, &parent)? {
            hold(db, &ceo, &c, &parent, at)?;
        } else if let Some(d) = answered_run(db, &ceo, &c, &parent, &report_of(&c), None)? {
            due.push(d);
        }
    }
    due.extend(redeliver(db, ended, at)?);
    // Taken in by a run that did not finish, they would reach nobody: the manager's next run brings them after all.
    if matches!(ended.state, RunState::Failed | RunState::Lost | RunState::Cancelled) {
        for mut r in db.noted_in(&ended.id)? {
            r.noted_in = None;
            db.save_run(&r)?;
            if let Some(parent) = r.parent_run_id().map(|p| db.run(p)).transpose()?.flatten() {
                hold(db, &ceo, &r, &parent, at)?;
            }
        }
    }
    Ok(due)
}

/// Whether a finished hand-off waits for its asker (`hold`) rather than coming back at once: a direct report's
/// task, handed over outside any conversation you started. Questions and tasks between colleagues come back at once.
fn held(db: &Store, handed: &Run, parent: &RunRecord) -> Result<bool> {
    Ok(matches!(handed.kind, RunKind::Delegated { .. }) && parent.run.kind.lane() == Lane::Work && super::conversation_of(db, &parent.run)?.is_none())
}

/// Holds a direct report's finished task for its manager, in the one queued run that brings them every report
/// held for them (`Run::reports`). It starts once HOLD passes without another, or HOLD_MAX after the first, so a
/// steady trickle never holds them for good, unless a run of the manager's own work takes them in first
/// (`catch_up`). Written here, under the lock that looks for it, so reports that come back together share one run.
fn hold(db: &Store, ceo: &str, handed: &Run, parent: &RunRecord, at: Ts) -> Result<()> {
    let manager = &parent.run.employee_id;
    if db.employee(manager)?.is_none() {
        return Ok(());
    }
    let (mut run, session, new) = match db.active_runs_of(manager)?.into_iter().find(|r| untouched(r) && !r.reports.is_empty()) {
        Some(b) => {
            let session = db.run(&b.id)?.map(|r| r.session).unwrap_or_else(|| b.id.clone());
            (b, session, false)
        }
        None => {
            let kind = RunKind::Answered { asked_run_id: handed.id.clone(), from: handed.employee_id.clone(), task: true, chat: false, thread: None };
            let mut run = Run::new(manager, kind, String::new(), at);
            run.omega_wrote = true;
            let session = answer_session(db, parent, at)?.0.unwrap_or_else(|| run.id.clone());
            (run, session, true)
        }
    };
    run.reports.push(handed.id.clone());
    run.not_before = Some((at + HOLD).min(run.created_at + HOLD_MAX));
    run.prompt = held_prompt(db, ceo, &run.reports, &session)?;
    if new {
        db.insert_run(&run, &session)?;
    } else {
        db.save_run(&run)?;
    }
    Ok(())
}

/// What the run bringing held reports says: each as a returned report says it, quoting where its manager's run
/// started unless this run resumes that run's session, and how many there are when there are several.
fn held_prompt(db: &Store, ceo: &str, reports: &[String], session: &str) -> Result<String> {
    let resumes = db.session_started(session)?;
    let (mut parts, mut quoted) = (vec![], HashSet::new());
    for id in reports {
        let Some(h) = db.run(id)?.map(|r| r.run) else { continue };
        let Some(parent) = h.parent_run_id().map(|p| db.run(p)).transpose()?.flatten() else { continue };
        let quote = !(resumes && parent.session == session) && quoted.insert(parent.run.id.clone());
        parts.push(answer_text(ceo, &colleague_name(db, &h.employee_id)?, &h, &parent.run, quote, &report_of(&h), None));
    }
    Ok(match parts.len() {
        1 => parts.remove(0),
        n => format!("{n} reports from your team came back after your run had ended.\n\n{}", parts.join("\n\n---\n\n")),
    })
}

/// An answer whose run failed or was lost, queued again after REDELIVER_AFTER as a new answered run for the same
/// ask, up to REDELIVERIES more times; after that it stays failed where you can see it. None for anything else,
/// and when the asker has left.
fn redeliver(db: &Store, ended: &Run, at: Ts) -> Result<Option<(Run, String)>> {
    let failed = matches!(ended.state, RunState::Failed | RunState::Lost);
    if !failed || !matches!(ended.kind, RunKind::Answered { .. }) || ended.redelivered >= REDELIVERIES || db.employee(&ended.employee_id)?.is_none() {
        return Ok(None);
    }
    let Some(rec) = db.run(&ended.id)? else { return Ok(None) };
    let mut again = Run::new(&ended.employee_id, ended.kind.clone(), ended.prompt.clone(), at);
    again.omega_wrote = ended.omega_wrote;
    again.reports = ended.reports.clone();
    again.redelivered = ended.redelivered + 1;
    again.not_before = Some(at + REDELIVER_AFTER);
    Ok(Some((again, rec.session)))
}

/// What a finished run reports back: its summary, or why it did not finish.
fn report_of(r: &Run) -> String {
    match (r.state, r.summary.as_deref()) {
        (RunState::Succeeded, Some(s)) => s.to_string(),
        (RunState::Succeeded, None) => "(no report)".to_string(),
        (state, _) => format!("They did not finish ({}): {}", state.as_str(), r.error.as_deref().unwrap_or("no details")),
    }
}

/// Whether a hand-off was a task (delegated, or asked as one) rather than a question.
fn is_task(handed: &Run) -> bool {
    !matches!(handed.kind, RunKind::Asked { task: false, .. })
}

/// The run that brings `handed`'s answer or report (`report`) to whoever handed it over, from
/// `parent`, and the session it runs in. When the asker handed it over in a conversation you started, it is
/// that conversation's next turn, in the session the conversation continues in. An older build's chat message
/// counts as one, and so does an answer turn of its chat that asked on (`conversation_of`), as `adopt_chat_turns`
/// makes them, though the message's first answer gets a fresh session rather than the old chat's. Otherwise it
/// resumes the asking run's session while that is fresh enough (`answer_session`). It quotes where the asker's run
/// started only when it does not resume it.
/// None when the asker has left.
fn answered_run(db: &Store, ceo: &str, handed: &Run, parent: &RunRecord, report: &str, note: Option<&str>) -> Result<Option<(Run, String)>> {
    let Some(asker) = db.employee(&parent.run.employee_id)? else { return Ok(None) };
    let from = colleague_name(db, &handed.employee_id)?;
    let into = super::conversation_of(db, &parent.run)?.map(|root| Office::thread_in(db, &root)).transpose()?;
    let (session, resumes) = match &into {
        Some(t) if matches!(t.root.kind, RunKind::Chat) && t.replies.is_empty() => (None, false),
        Some(t) => {
            let (rec, resumes) = Office::next_turn(db, t)?;
            (resumes.then_some(rec.session), resumes)
        }
        None if parent.run.kind.lane() == Lane::Chat => (None, false),
        None => answer_session(db, parent, now())?,
    };
    let prompt = answer_text(ceo, &from, handed, &parent.run, !resumes, report, note);
    let thread = into.map(|t| t.root.id);
    let kind = RunKind::Answered { asked_run_id: handed.id.clone(), from: handed.employee_id.clone(), task: is_task(handed), chat: false, thread };
    let mut run = Run::new(&asker.id, kind, prompt, now());
    run.omega_wrote = true;
    let session = session.unwrap_or_else(|| run.id.clone());
    Ok(Some((run, session)))
}

/// The session an answer to `parent`, a run outside any conversation you started, runs in, and whether it resumes
/// it. The asker's session holds everything it knew, but once it began over a day ago or its context passed
/// FRESH_TOKENS, the answer starts a fresh one (None) rather than compact it again. A session that never started is
/// the answer's to start.
fn answer_session(db: &Store, parent: &RunRecord, at: Ts) -> Result<(Option<String>, bool)> {
    if !db.session_started(&parent.session)? {
        return Ok((Some(parent.session.clone()), false));
    }
    let (began, context) = db.session_size(&parent.session)?;
    if began.is_some_and(|t| at - t > chrono::Duration::days(1)) || context.is_some_and(|c| c > FRESH_TOKENS) {
        return Ok((None, false));
    }
    Ok((Some(parent.session.clone()), true))
}

/// The words that bring `handed`'s answer or report (`report`) from `from` to whoever handed it over in `parent`:
/// `note` when there is one, where `parent` had started when `quote` (the run does not resume it), the ask and the report.
fn answer_text(ceo: &str, from: &str, handed: &Run, parent: &Run, quote: bool, report: &str, note: Option<&str>) -> String {
    let (did, yours, theirs) =
        if is_task(handed) { ("finished the task you handed them", "Your task", "Their report") } else { ("answered your question", "Your question", "Their answer") };
    let mut prompt = format!("{from} {did}.");
    if let Some(n) = note {
        prompt += &format!("\n\n{n}");
    }
    if quote {
        let origin = if ceo_in(&parent.kind) && !parent.omega_wrote {
            format!("You were talking with {ceo}, whose message was:")
        } else {
            "Your run had started from:".to_string()
        };
        prompt += &format!("\n\n{origin}\n\n{}", claude::clip(&parent.prompt, 2000));
    }
    prompt + &format!("\n\n{yours}:\n\n{}\n\n{theirs}:\n\n{}", handed.prompt, claude::clip(report, 20000))
}

/// Where a hand-off began, read from the store under its lock; `source_text` turns it into a note
/// from the transcripts once the lock is released.
pub(super) struct SourceChain {
    ceo: String,
    /// Whoever `from` belongs to, who sent the hand-off on.
    sender: String,
    /// The hand-off runs walked back through, nearest first, with their employees' names.
    handoffs: Vec<(Run, String)>,
    /// The first run that did not come from a hand-off, with its employee's name; None when it is gone.
    origin: Option<(Run, String)>,
    /// For a hire decision: your note on the proposal.
    note: Option<String>,
    /// For a follow-up: the person who replied.
    person: Option<String>,
}

/// Walks back from the run `from` (the upstream of a hand-off) to the run the work began in.
pub(super) fn source_chain(db: &Store, ceo: &str, from: &str) -> Result<SourceChain> {
    let mut chain = SourceChain { ceo: ceo.to_string(), sender: String::new(), handoffs: vec![], origin: None, note: None, person: None };
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
        chain.sender = "the sender".into();
    }
    Ok(chain)
}

/// A note on where a hand-off began, quoting your own words where there are some, with what you
/// wrote into its runs on the way. Reads the runs' transcripts under `runs_dir`.
pub(super) fn source_text(chain: &SourceChain, runs_dir: &Path) -> String {
    let ceo = &chain.ceo;
    let steered = |id: &str| claude::steered(&fs::read_to_string(runs_dir.join(id).join(TRANSCRIPT)).unwrap_or_default());
    let quote = |s: &str| format!("> \"{}\"", claude::clip(&claude::one_line(s), 600));
    let missing = format!("Where this began is no longer on record, so treat it as {}'s own request.", chain.sender);
    let mut text = match &chain.origin {
        None => missing,
        Some((r, _)) if r.omega_wrote => format!(
            "This began with a message Omega Star wrote in the app from something {ceo} did there. It is not {ceo}'s own words, except where it quotes them:\n{}",
            quote(&r.prompt)
        ),
        Some((r, employee)) => match &r.kind {
            RunKind::Manual if r.prompt == DEFAULT_PROMPT => format!("This began when {ceo} started a run for {employee} from the app, with no instructions."),
            RunKind::Manual | RunKind::Chat | RunKind::Task { .. } | RunKind::Reply { .. } => {
                let words: Vec<String> = std::iter::once(r.prompt.clone()).chain(steered(&r.id)).collect();
                format!("This began with {ceo}'s own message to {employee}:\n{}", quote(&words.join(" ")))
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{open_office, saved_run, task};
    use crate::office::fixtures::{employee, held_office, office, saved, standup, Office0};

    #[tokio::test]
    async fn the_front_desk_is_set_up_once_stays_off_the_team_and_passes_messages_on() {
        let o = office();
        let mo = Employee { title: "Coordinator".into(), ..employee(&o.db, "Mo", None) };
        o.db.save_employee(&mo).unwrap();
        let office = held_office(&o).await;
        let desk = office.front_desk().unwrap();
        assert!(desk.employee.desk && desk.employee.name == "OmegaStar" && !desk.employee.auto_tidy && !desk.employee.browser);
        assert_eq!(office.front_desk().unwrap().employee.id, desk.employee.id, "opening it again finds the same desk");
        let org = office.org().unwrap();
        assert_eq!(org.employees.iter().map(|e| e.employee.name.as_str()).collect::<Vec<_>>(), ["Mo"]);
        assert_eq!(org.desk.map(|d| d.employee.id), Some(desk.employee.id.clone()));
        assert!(o.db.schedules(&desk.employee.id).unwrap().is_empty(), "no weekly review for the desk");
        assert!(office.colleagues(&mo).unwrap()["colleagues"].as_array().unwrap().is_empty(), "colleagues never see the desk");
        assert!(office.update_employee(desk.employee.clone()).is_err());

        let talk = saved_run(&o.db, &desk.employee, task("Plan the launch"), "Get the launch planned.", RunState::Running, 0, None);
        office.ask_colleague(&talk.id, &desk.employee, "Mo", "Pat asks: plan the launch.", "task").unwrap();
        let handed = o.db.active_runs_of(&mo.id).unwrap();
        assert!(matches!(&handed[..], [r] if matches!(&r.kind, RunKind::Asked { asked_by, task: true, .. } if *asked_by == desk.employee.id)), "{handed:?}");
        assert!(office.ask_colleague(&talk.id, &mo, "OmegaStar", "Anything?", "question").is_err(), "nobody can hand the desk work");
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
        let parent = saved_run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Succeeded, 30, None);
        o.db.mark_inited(&parent.id).unwrap();
        let child = saved_run(&o.db, &bo, asked(&parent, true), "Count the stock.", RunState::Succeeded, 20, None);

        let due = due_answers(&o.db, &child).unwrap();
        assert_eq!(due.len(), 1);
        let (a, session) = &due[0];
        assert_eq!(a.employee_id, ada.id);
        assert_eq!(a.kind, RunKind::Answered { asked_run_id: child.id.clone(), from: bo.id.clone(), task: true, chat: false, thread: None });
        assert_eq!(session, &parent.id, "it resumes the manager's session");
        assert!(a.omega_wrote);
        assert!(a.prompt.starts_with("Bo finished the task you handed them.\n\nYour task:\n\nCount the stock.\n\nTheir report:\n\nBo did it."), "{}", a.prompt);
        assert!(delivered(&o.db, &child));
        assert!(due_answers(&o.db, &child).unwrap().is_empty(), "delivered once");
        assert!(due_answers(&o.db, &parent).unwrap().is_empty());
    }

    #[tokio::test]
    async fn reports_come_home_when_a_run_is_lost_at_startup() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        // Ada was waiting on Bo's report when the app quit; Bo's own hand-off to Cy was lost with it.
        let parent = saved_run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Running, 30, None);
        let child = saved_run(&o.db, &bo, delegated(&parent), "Count the stock.", RunState::Succeeded, 20, None);
        let cy = employee(&o.db, "Cy", Some(&bo.id));
        let asker = saved_run(&o.db, &bo, RunKind::Manual, "Price the stock.", RunState::Succeeded, 25, None);
        let lost = saved_run(&o.db, &cy, delegated(&asker), "Check the prices.", RunState::Running, 20, None);
        // Both already went back to the queue as often as a run may, so this restart loses them.
        for mut r in [parent.clone(), lost.clone()] {
            r.restarts = RESTART_RETRIES;
            o.db.save_run(&r).unwrap();
        }

        let office = open_office(&o.dir).await;
        assert!(delivered(&o.db, &child), "the lost manager still gets the report that waited for it");
        assert!(delivered(&o.db, &lost), "the lost hand-off reports back to who handed it over");
        let answered: Vec<Run> = o.db.runs_in_state(RunState::Queued).unwrap();
        let to = |id: &str| answered.iter().find(|r| matches!(&r.kind, RunKind::Answered { asked_run_id, .. } if asked_run_id == id)).map(|r| r.employee_id.clone());
        assert_eq!(to(&child.id), Some(ada.id.clone()));
        assert_eq!(to(&lost.id), Some(bo.id.clone()));
        drop(office);
    }

    #[tokio::test]
    async fn a_report_waits_for_a_manager_the_restart_put_back_in_the_queue() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let parent = saved_run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Running, 30, None);
        let child = saved_run(&o.db, &bo, delegated(&parent), "Count the stock.", RunState::Succeeded, 20, None);

        let office = open_office(&o.dir).await;
        assert_eq!(o.db.run(&parent.id).unwrap().unwrap().run.state, RunState::Queued, "it carries on");
        assert!(!delivered(&o.db, &child), "it collects the report with await_reports, or gets it when it ends");
        drop(office);
    }

    #[test]
    fn a_report_waits_for_its_manager_to_finish() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let mut parent = saved_run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Running, 30, None);
        let child = saved_run(&o.db, &bo, asked(&parent, true), "Count the stock.", RunState::Succeeded, 20, None);
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
        let parent = saved_run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Succeeded, 30, None);
        let stopped = saved_run(&o.db, &bo, delegated(&parent), "Count the stock.", RunState::Cancelled, 20, None);
        assert!(due_answers(&o.db, &stopped).unwrap().is_empty());
        assert!(delivered(&o.db, &stopped), "stamped all the same");

        let mut collected = saved_run(&o.db, &bo, delegated(&parent), "Count the till.", RunState::Succeeded, 20, None);
        collected.delivered_at = Some(now());
        o.db.save_run(&collected).unwrap();
        assert!(due_answers(&o.db, &collected).unwrap().is_empty());
        assert!(due_answers(&o.db, &parent).unwrap().is_empty());
    }

    #[test]
    fn an_answer_to_an_ask_from_a_conversation_is_its_next_turn() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let task = task("The numbers");
        let parent = saved_run(&o.db, &ada, task, "Ask Bo for the numbers.", RunState::Succeeded, 30, None);
        o.db.mark_inited(&parent.id).unwrap();
        let mut child = saved_run(&o.db, &bo, asked(&parent, false), "What are the numbers?", RunState::Succeeded, 20, None);
        child.needs_you = vec!["Look at this.".into()];
        o.db.save_run(&child).unwrap();

        let (a, session) = due_answers(&o.db, &child).unwrap().remove(0);
        assert_eq!(a.kind, RunKind::Answered { asked_run_id: child.id.clone(), from: bo.id.clone(), task: false, chat: false, thread: Some(parent.id.clone()) });
        assert_eq!(session, parent.id, "the conversation's session");
        assert!(a.kind.is_conversation());
        assert_eq!(a.kind.lane(), Lane::Work);
        assert!(a.prompt.starts_with("Bo answered your question.\n\nYour question:"), "it resumes the conversation, so it quotes nothing: {}", a.prompt);

        assert!(o.db.insert_run(&a, &session).unwrap());
        assert_eq!(Office::thread_in(&o.db, &parent.id).unwrap().replies.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![a.id.clone()]);
        assert!(!o.db.conversation_runs().unwrap().iter().any(|r| r.id == a.id));
        let rows = Office::rows(&o.db, crate::threads::View::Inbox).unwrap();
        let filed: Vec<&str> = rows.iter().flat_map(|t| std::iter::once(t).chain(&t.related)).map(|t| t.root.id.as_str()).collect();
        assert!(filed.contains(&child.id.as_str()), "the flagged answer has its row: {filed:?}");
        assert!(!filed.contains(&a.id.as_str()), "the turn is in no row");

        // An ask from your reply in the conversation comes back into it too, after the turn before it.
        let reply = RunKind::Reply { to_run_id: parent.id.clone(), label: "The numbers".into(), task: true, card: false };
        let replied = saved_run(&o.db, &ada, reply, "And the totals?", RunState::Succeeded, 5, Some(&parent.id));
        let again = saved_run(&o.db, &bo, asked(&replied, false), "And the totals?", RunState::Succeeded, 2, None);
        let (b, session) = due_answers(&o.db, &again).unwrap().remove(0);
        assert!(matches!(&b.kind, RunKind::Answered { thread: Some(t), .. } if *t == parent.id), "{:?}", b.kind);
        assert_eq!(session, parent.id);
    }

    #[test]
    fn an_answer_to_an_older_builds_chat_comes_back_into_its_message_in_a_fresh_session() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let parent = saved_run(&o.db, &ada, RunKind::Chat, "Ask Bo for the numbers.", RunState::Succeeded, 30, Some(&ada.chat_session));
        o.db.mark_inited(&parent.id).unwrap();
        let child = saved_run(&o.db, &bo, asked(&parent, false), "What are the numbers?", RunState::Succeeded, 20, None);
        let (a, session) = due_answers(&o.db, &child).unwrap().remove(0);
        assert_eq!(a.kind, RunKind::Answered { asked_run_id: child.id.clone(), from: bo.id.clone(), task: false, chat: false, thread: Some(parent.id.clone()) });
        assert_eq!(session, a.id, "never into the old chat");
        assert!(a.prompt.starts_with("Bo answered your question.\n\nYou were talking with Pat, whose message was:\n\nAsk Bo for the numbers."), "{}", a.prompt);
    }

    #[test]
    fn an_ask_from_an_older_builds_chat_answer_turn_comes_back_into_the_chat_message() {
        let o = office();
        let di = employee(&o.db, "Di", None);
        let ada = employee(&o.db, "Ada", Some(&di.id));
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let cy = employee(&o.db, "Cy", None);
        let message = saved_run(&o.db, &ada, RunKind::Chat, "Ask Bo for the numbers.", RunState::Succeeded, 40, Some(&ada.chat_session));
        o.db.mark_inited(&message.id).unwrap();
        let first = saved_run(&o.db, &bo, asked(&message, false), "What are the numbers?", RunState::Succeeded, 30, None);
        let came = RunKind::Answered { asked_run_id: first.id.clone(), from: bo.id.clone(), task: false, chat: true, thread: None };
        let turn = saved_run(&o.db, &ada, came, "Bo answered your question.", RunState::Succeeded, 20, Some(&ada.chat_session));
        let child = saved_run(&o.db, &cy, asked(&turn, false), "Do they include the till?", RunState::Succeeded, 10, None);
        let (a, _) = due_answers(&o.db, &child).unwrap().remove(0);
        assert_eq!(a.kind, RunKind::Answered { asked_run_id: child.id.clone(), from: cy.id.clone(), task: false, chat: false, thread: Some(message.id.clone()) });
    }

    #[test]
    fn a_report_quotes_where_its_run_started_only_when_it_does_not_resume_it() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let never_started = saved_run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Failed, 30, None);
        let child = saved_run(&o.db, &bo, asked(&never_started, true), "Count the stock.", RunState::Succeeded, 20, None);
        let (a, session) = due_answers(&o.db, &child).unwrap().remove(0);
        assert_eq!(session, never_started.id);
        assert!(a.prompt.contains("\n\nYour run had started from:\n\nPlan the launch.\n\nYour task:"), "{}", a.prompt);

        let came = RunKind::Answered { asked_run_id: child.id.clone(), from: bo.id.clone(), task: true, chat: false, thread: None };
        let answered = saved_run(&o.db, &ada, came, "Bo finished the task you handed them.", RunState::Succeeded, 10, None);
        let child = saved_run(&o.db, &bo, asked(&answered, true), "Count the till.", RunState::Failed, 5, None);
        let (a, _) = due_answers(&o.db, &child).unwrap().remove(0);
        assert!(a.prompt.contains("\n\nYour run had started from:\n\nBo finished the task"), "{}", a.prompt);
        assert!(a.prompt.contains("Their report:\n\nThey did not finish (failed): no details"), "{}", a.prompt);
    }

    #[test]
    fn an_answered_card_on_handed_work_reports_to_whoever_handed_it() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let parent = saved_run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Succeeded, 30, None);
        o.db.mark_inited(&parent.id).unwrap();
        let mut child = saved_run(&o.db, &bo, delegated(&parent), "Boost the post.", RunState::Succeeded, 20, None);
        let answer = CardAnswer { at: now(), how: AnswerHow::Tapped, text: "Chose: Not now.".into(), held: false };
        let mut card = Card::new("Boost it?", None, Some("ask:boost".into()), CardAsk::Choices { choices: vec!["Not now".into(), "Go".into()], recommended: 0 });
        card.answer = Some(answer);
        child.cards = vec![card];
        o.db.save_run(&child).unwrap();
        assert!(due_answers(&o.db, &child).unwrap().is_empty(), "the first report is held for Ada");
        assert!(delivered(&o.db, &child));

        let reply = |card: bool, state: RunState| {
            let kind = RunKind::Reply { to_run_id: child.id.clone(), label: "Delegated".into(), task: false, card };
            saved_run(&o.db, &bo, kind, "Your card: \"Boost it?\"\nChose: Not now.", state, 5, Some(&child.id))
        };
        assert!(due_answers(&o.db, &reply(false, RunState::Succeeded)).unwrap().is_empty(), "a reply you typed stays with Bo");
        assert!(due_answers(&o.db, &reply(true, RunState::Cancelled)).unwrap().is_empty(), "a reply you stopped stays with Bo");
        let tapped = reply(true, RunState::Succeeded);
        let due = due_answers(&o.db, &tapped).unwrap();
        assert_eq!(due.len(), 1);
        let (a, session) = &due[0];
        assert_eq!(a.employee_id, ada.id);
        assert_eq!(session, &parent.id);
        assert_eq!(a.kind, RunKind::Answered { asked_run_id: child.id.clone(), from: bo.id.clone(), task: true, chat: false, thread: None });
        assert!(
            a.prompt.starts_with("Bo finished the task you handed them.\n\nPat answered Bo's card (Chose: Not now.) and Bo carried on.\n\nYour task:\n\nBoost the post.\n\nTheir report:\n\nBo did it."),
            "{}",
            a.prompt
        );
    }

    #[test]
    fn older_runs_load_without_the_new_fields() {
        let kind: RunKind = serde_json::from_value(json!({ "type": "answered", "askedRunId": "r1", "from": "e1", "task": true })).unwrap();
        assert_eq!(kind, RunKind::Answered { asked_run_id: "r1".into(), from: "e1".into(), task: true, chat: false, thread: None });
        let mut v = serde_json::to_value(Run::new("e1", RunKind::Manual, "go".into(), now())).unwrap();
        for field in ["omegaWrote", "notBefore", "chain", "reports", "notedIn", "redelivered"] {
            v.as_object_mut().unwrap().remove(field);
        }
        let r: Run = serde_json::from_value(v).unwrap();
        assert!(!r.omega_wrote);
        assert_eq!((r.not_before, r.chain, r.reports, r.noted_in, r.redelivered), (None, None, vec![], None, 0));
    }

    #[test]
    fn a_handoff_note_says_when_to_wait() {
        let plain = handoff_note("Bo", "Their report", 0, false, false);
        assert_eq!(
            plain,
            "Their report comes back to you either way. Wait for it with await_reports only if it should take a few minutes and you need it \
before you finish. Otherwise finish now: Omega Star starts a new run for you when it arrives."
        );
        let both = handoff_note("Bo", "The answer", 1, true, false);
        assert!(both.starts_with("Bo is busy with other work: 1 run is ahead of this one, so this starts when that finishes. The answer comes back"), "{both}");
        assert!(both.ends_with("when it arrives, as your next turn in this conversation."));
        let queue = handoff_note("Bo", "The answer", 3, false, false);
        assert!(queue.starts_with("Bo is busy with other work: 3 runs are ahead of this one, so this starts when those finish. "), "{queue}");
        let held = handoff_note("Bo", "Their report", 0, false, true);
        assert!(held.ends_with("Otherwise finish now: Omega Star keeps it for your next run, or brings it with any other reports from your team in one run once 30 minutes pass without a new one."), "{held}");
        let paused = paused_note("Bo", "Their report", now());
        assert!(paused.starts_with("Omega Star paused this hand-off to keep the chain short. It goes to Bo by itself at "), "{paused}");
        assert!(!paused.contains("declin") && !paused.contains("refus"), "nothing suggests Bo said no: {paused}");
        for t in [&plain, &both, &queue, &held, &paused] {
            assert!(!t.contains('\u{2014}'), "{t}");
        }
    }

    #[test]
    fn only_work_that_waits_for_a_lane_has_runs_ahead_of_it() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", None);
        let cap = |n: u32| o.db.set_settings(&AppSettings { max_concurrent_runs: n, ..AppSettings::default() }).unwrap();
        cap(1);
        let asker = saved_run(&o.db, &ada, RunKind::Manual, "Ask Bo.", RunState::Succeeded, 30, None);
        saved_run(&o.db, &bo, RunKind::Manual, "A long build.", RunState::Running, 20, None);
        let earlier = saved_run(&o.db, &bo, RunKind::Asked { parent_run_id: asker.id.clone(), asked_by: ada.id.clone(), task: true }, "Fix it.", RunState::Queued, 10, None);
        let task = saved_run(&o.db, &bo, RunKind::Asked { parent_run_id: asker.id.clone(), asked_by: ada.id.clone(), task: true }, "Ship it.", RunState::Queued, 5, None);
        let question = saved_run(&o.db, &bo, RunKind::Asked { parent_run_id: asker.id.clone(), asked_by: ada.id.clone(), task: false }, "Which room?", RunState::Queued, 1, None);
        assert_eq!(queue_ahead(&o.db, &task, now()).unwrap(), 2, "the build, and the task queued before it");
        assert_eq!(queue_ahead(&o.db, &earlier, now()).unwrap(), 1);
        assert_eq!(queue_ahead(&o.db, &question, now()).unwrap(), 0, "a question runs beside background work");
        cap(2);
        assert_eq!(queue_ahead(&o.db, &earlier, now()).unwrap(), 0, "a second lane opens with the cap");
        assert_eq!(queue_ahead(&o.db, &task, now()).unwrap(), 2);
    }

    /// Writes your messages into a run's transcript, as if steered into it while it worked.
    fn steer(o: &Office0, r: &Run, said: &[&str]) {
        let dir = o.dir.join("runs").join(&r.id);
        fs::create_dir_all(&dir).unwrap();
        let lines: Vec<String> = said.iter().map(|t| claude::omega_line(&RunEvent::User { text: t.to_string(), id: None })).collect();
        fs::write(dir.join("transcript.jsonl"), lines.join("\n") + "\n").unwrap();
    }

    /// Where the hand-off `r` began, as the run being prepared for it is told: walked from its upstream.
    fn source(o: &Office0, r: &Run) -> String {
        source_text(&source_chain(&o.db, "Pat", r.upstream().unwrap()).unwrap(), &o.dir.join("runs"))
    }

    #[test]
    fn a_handoff_from_a_conversation_quotes_what_you_wrote_into_it() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let cy = employee(&o.db, "Cy", None);
        let task = task("The stock");
        let delegating = saved_run(&o.db, &ada, task, "Get Bo to count the stock.", RunState::Succeeded, 35, None);
        steer(&o, &delegating, &["Only the warehouse."]);

        let d = saved_run(&o.db, &bo, delegated(&delegating), "Count the stock.", RunState::Running, 30, None);
        steer(&o, &d, &["Use last week's sheet."]);
        assert_eq!(source(&o, &d), "This began with Pat's own message to Ada:\n> \"Get Bo to count the stock. Only the warehouse.\"");
        // Bo asks Cy on: what Pat wrote into Bo's run on the way comes with it.
        let a = saved_run(&o.db, &cy, asked(&d, false), "Where is last week's sheet?", RunState::Running, 20, None);
        assert_eq!(
            source(&o, &a),
            "This began with Pat's own message to Ada:\n> \"Get Bo to count the stock. Only the warehouse.\"\n\
Pat also wrote into Bo's run on the way:\n\
> \"Use last week's sheet.\""
        );
        // A hand-off from an older build's chat quotes the message it began with.
        let chat = saved_run(&o.db, &ada, RunKind::Chat, "Ask Bo for a date.", RunState::Succeeded, 10, Some(&ada.chat_session));
        let d = saved_run(&o.db, &bo, delegated(&chat), "Find a date.", RunState::Running, 5, None);
        assert_eq!(source(&o, &d), "This began with Pat's own message to Ada:\n> \"Ask Bo for a date.\"");
    }

    #[test]
    fn a_handoff_says_when_it_did_not_begin_with_your_words() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let from = |origin: &Run| {
            let d = saved_run(&o.db, &bo, delegated(origin), "Count the stock.", RunState::Running, 0, None);
            source(&o, &d)
        };

        let sweep = RunKind::Scheduled { schedule_id: "s1".into(), slot: now(), label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
        let scheduled = saved_run(&o.db, &ada, sweep, "Sweep the inbox.", RunState::Running, 5, None);
        assert_eq!(from(&scheduled), "This began with Ada's scheduled 'Sweep', not a message from Pat.");

        let root = saved_run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Succeeded, 60, None);
        assert_eq!(from(&root), "This began with Pat's own message to Ada:\n> \"Plan the launch.\"");
        let reply = RunKind::Reply { to_run_id: root.id.clone(), label: "Manual run".into(), task: false, card: false };
        let mut approval = Run::new(&ada.id, reply, "Pat approved what was blocked:\n- Bash: npm publish".into(), now());
        approval.omega_wrote = true;
        o.db.insert_run(&approval, &root.id).unwrap();
        assert_eq!(
            from(&approval),
            "This began with a message Omega Star wrote in the app from something Pat did there. It is not Pat's own words, except where it \
quotes them:\n> \"Pat approved what was blocked: - Bash: npm publish\""
        );

        let bare = saved_run(&o.db, &ada, RunKind::Manual, DEFAULT_PROMPT, RunState::Running, 5, None);
        assert_eq!(from(&bare), "This began when Pat started a run for Ada from the app, with no instructions.");

        let lost = saved_run(&o.db, &bo, RunKind::Delegated { parent_run_id: "gone".into() }, "Count the stock.", RunState::Running, 0, None);
        assert_eq!(source(&o, &lost), "Where this began is no longer on record, so treat it as the sender's own request.");
        let cy = employee(&o.db, "Cy", None);
        let on = saved_run(&o.db, &cy, asked(&lost, false), "Which stock?", RunState::Running, 0, None);
        assert_eq!(source(&o, &on), "Where this began is no longer on record, so treat it as Bo's own request.");
    }

    /// A finished, delivered hand-off of `kind` to `to` in the chain `root` started, `ago` minutes back.
    fn handed(db: &Store, to: &Employee, kind: RunKind, root: &Run, ago: i64) -> Run {
        let mut r = saved_run(db, to, kind, "Over to you.", RunState::Succeeded, ago, None);
        r.chain = Some(root.id.clone());
        r.delivered_at = Some(now());
        db.save_run(&r).unwrap();
        r
    }

    fn run_id(v: &Value) -> String {
        v["runId"].as_str().unwrap().to_string()
    }

    /// What `catch_up` puts in the opening note of `r`.
    fn caught_up(db: &Store, employees: &[Employee], r: &Run) -> claude::Opening {
        let mut o = claude::Opening { now: now(), ..Default::default() };
        catch_up(db, employees, r, &mut o).unwrap();
        o
    }

    #[tokio::test]
    async fn back_and_forth_pauses_the_next_handoff_instead_of_refusing_it() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let cy = employee(&o.db, "Cy", None);
        let root = saved_run(&o.db, &ada, standup("s1"), "Sweep the inbox.", RunState::Succeeded, 180, None);
        for _ in 0..3 {
            handed(&o.db, &cy, asked(&root, false), &root, 150);
        }
        // Five hand-offs between Ada and Cy in the last two hours, either way. Older ones and answers do not count.
        let h1 = handed(&o.db, &cy, asked(&root, false), &root, 60);
        let h2 = handed(&o.db, &ada, asked(&h1, false), &root, 50);
        let h3 = handed(&o.db, &cy, asked(&h2, false), &root, 40);
        let h4 = handed(&o.db, &ada, asked(&h3, false), &root, 30);
        let h5 = handed(&o.db, &cy, asked(&h4, false), &root, 20);
        let answer = |h: &Run, state| {
            let came = RunKind::Answered { asked_run_id: h.id.clone(), from: cy.id.clone(), task: false, chat: false, thread: None };
            saved_run(&o.db, &ada, came, "Cy answered your question.", state, 10, None)
        };
        answer(&h3, RunState::Succeeded);
        let caller = answer(&h5, RunState::Running);
        let office = held_office(&o).await;

        let first = office.ask_colleague(&caller.id, &ada, "Cy", "And the totals?", "question").unwrap();
        let mut r = saved(&o.db, &run_id(&first));
        assert_eq!((r.not_before, r.chain.clone()), (None, Some(root.id.clone())), "{first}");
        r.state = RunState::Succeeded;
        o.db.save_run(&r).unwrap();

        let second = office.ask_colleague(&caller.id, &ada, "Cy", "And the till?", "question").unwrap();
        let r = saved(&o.db, &run_id(&second));
        assert_eq!(r.state, RunState::Queued, "paused, not refused");
        let wait = r.not_before.unwrap() - now();
        assert!(wait > chrono::Duration::minutes(59) && wait <= chrono::Duration::hours(1), "{wait:?}");
        assert_eq!(r.chain, Some(r.id.clone()), "it starts a chain of its own");
        assert_eq!(chain_up(&o.db, &r.id).unwrap().1, r.id, "what it hands on counts there");
        let note = second["note"].as_str().unwrap();
        assert!(note.starts_with("Omega Star paused this hand-off to keep the chain short. It goes to Cy by itself at "), "{note}");
        drop(office);
    }

    #[tokio::test]
    async fn a_pair_paused_twice_today_is_stopped_and_tasks_count_only_toward_the_chain() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let cy = employee(&o.db, "Cy", None);
        let root = saved_run(&o.db, &ada, standup("s1"), "Sweep the inbox.", RunState::Running, 0, None);
        // Six tasks Ada handed Bo in this chain just now: the daily cap covers those, so they never pause an ask.
        for _ in 0..6 {
            handed(&o.db, &bo, delegated(&root), &root, 0);
        }
        let theirs = saved_run(&o.db, &cy, RunKind::Manual, "Price the stock.", RunState::Running, 0, None);
        // Two asks between Ada and Cy were paused today, one each way: each started a chain of its own.
        for (from, to) in [(&root, &cy), (&theirs, &ada)] {
            let mut r = saved_run(&o.db, to, asked(from, false), "Which supplier?", RunState::Succeeded, 0, None);
            r.chain = Some(r.id.clone());
            o.db.save_run(&r).unwrap();
        }
        let office = held_office(&o).await;
        let ask = office.ask_colleague(&root.id, &ada, "Bo", "Which shelf?", "question").unwrap();
        assert_eq!(saved(&o.db, &run_id(&ask)).not_before, None, "{ask}");

        let stopped = office.ask_colleague(&root.id, &ada, "Cy", "And the till?", "question").unwrap_err();
        assert_eq!(
            stopped.message,
            "Omega Star stopped this hand-off: Ada and Cy have passed this back and forth all day. Finish with what you have and say in your report what is still open."
        );
        let back = office.ask_colleague(&theirs.id, &cy, "Ada", "Can you check the till?", "task").unwrap_err();
        assert!(back.message.starts_with("Omega Star stopped this hand-off: Cy and Ada have passed this back and forth all day."), "{}", back.message);
        drop(office);
    }

    #[test]
    fn held_reports_wait_two_hours_at_most_however_often_one_comes() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let morning = saved_run(&o.db, &ada, standup("s1"), "Stand-up.", RunState::Succeeded, 300, None);
        let parent = o.db.run(&morning.id).unwrap().unwrap();
        let report = |task: &str| saved_run(&o.db, &bo, delegated(&morning), task, RunState::Succeeded, 0, None);
        let batch = || o.db.active_runs_of(&ada.id).unwrap().remove(0);
        let first = now() - chrono::Duration::minutes(110);
        hold(&o.db, "Pat", &report("Count the shelf."), &parent, first).unwrap();
        assert_eq!(batch().not_before, Some(first + HOLD));
        // Another every 20 minutes would put it off for good: it goes two hours after the first.
        hold(&o.db, "Pat", &report("Count the till."), &parent, first + chrono::Duration::minutes(100)).unwrap();
        let b = batch();
        assert_eq!((b.not_before, b.reports.len()), (Some(first + HOLD_MAX), 2));
    }

    #[tokio::test]
    async fn reports_a_run_took_in_are_held_again_when_you_stop_it() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let morning = saved_run(&o.db, &ada, standup("s1"), "Stand-up.", RunState::Succeeded, 60, None);
        let shelf = saved_run(&o.db, &bo, delegated(&morning), "Count the shelf.", RunState::Succeeded, 20, None);
        assert!(due_answers(&o.db, &shelf).unwrap().is_empty(), "held for Ada");
        // Ada's next run took it in, then went back to the queue for the usage limit, where you stop it.
        let next = saved_run(&o.db, &ada, RunKind::Manual, "Go.", RunState::Running, 0, None);
        assert_eq!(caught_up(&o.db, &[ada.clone(), bo.clone()], &next).held.len(), 1);
        let mut back = saved(&o.db, &next.id);
        (back.state, back.limit_retries) = (RunState::Queued, 1);
        o.db.save_run(&back).unwrap();
        let office = held_office(&o).await;
        office.cancel_run(&next.id).unwrap();
        let held: Vec<Vec<String>> = o.db.active_runs_of(&ada.id).unwrap().into_iter().map(|r| r.reports).collect();
        assert_eq!(held, vec![vec![shelf.id.clone()]], "Ada's next run brings it after all");
        assert_eq!(saved(&o.db, &shelf.id).noted_in, None);
        drop(office);
    }

    #[tokio::test]
    async fn a_long_chain_pauses_a_task_and_a_report_takes_four_a_day() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        employee(&o.db, "Bo", Some(&ada.id));
        let (cy, dee) = (employee(&o.db, "Cy", None), employee(&o.db, "Dee", None));
        let root = saved_run(&o.db, &ada, standup("s1"), "Hand out the week's work.", RunState::Running, 300, None);
        let theirs = saved_run(&o.db, &cy, RunKind::Manual, "Ask Dee.", RunState::Succeeded, 290, None);
        for _ in 0..39 {
            handed(&o.db, &dee, asked(&theirs, false), &root, 280);
        }
        let office = held_office(&o).await;
        let hand = |n: usize| office.delegate(&root.id, &ada, "Bo", &format!("Count shelf {n}."));
        let first = saved(&o.db, &run_id(&hand(1).unwrap()));
        assert_eq!((first.not_before, first.chain.clone()), (None, Some(root.id.clone())), "39 hand-offs came before it");
        let second = hand(2).unwrap();
        assert!(second["note"].as_str().unwrap().starts_with("Omega Star paused this hand-off to keep the chain short. It goes to Bo by itself at "), "{second}");
        assert!(saved(&o.db, &run_id(&second)).not_before.is_some(), "the 41st waits");
        hand(3).unwrap();
        hand(4).unwrap();
        assert_eq!(hand(5).unwrap_err().message, "Bo already has 4 tasks from you today. Put the rest in your plan for tomorrow.");
        let team = office.team(&ada).unwrap();
        assert_eq!((team["reports"][0]["tasksToday"].clone(), team["tasksPerDay"].clone()), (json!(4), json!(4)));
        // One you stopped no longer counts, and a conversation with Pat may hand out more.
        office.cancel_run(&first.id).unwrap();
        hand(5).unwrap();
        let talking = saved_run(&o.db, &ada, task("The stock"), "Get Bo on the shelves.", RunState::Running, 5, None);
        let v = office.delegate(&talking.id, &ada, "Bo", "Count shelf 6.").unwrap();
        assert!(v["note"].as_str().unwrap().ends_with("as your next turn in this conversation."), "{v}");
        drop(office);
    }

    #[tokio::test]
    async fn a_colleague_waiting_on_you_up_the_chain_is_still_refused() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let waiting = saved_run(&o.db, &bo, RunKind::Manual, "Price the stock.", RunState::Running, 10, None);
        let asked_ada = saved_run(&o.db, &ada, asked(&waiting, false), "Which supplier?", RunState::Running, 5, None);
        let office = held_office(&o).await;
        let ask = office.ask_colleague(&asked_ada.id, &ada, "Bo", "Which shelf?", "question").unwrap_err();
        assert!(ask.message.starts_with("Bo is already waiting on you further up this chain, so they cannot answer now."), "{}", ask.message);
        let task = office.delegate(&asked_ada.id, &ada, "Bo", "Count the shelf.").unwrap_err();
        assert!(task.message.starts_with("Bo is already waiting on you further up this chain, so they cannot take this now."), "{}", task.message);
        drop(office);
    }

    #[tokio::test]
    async fn a_repeat_request_joins_the_one_not_started_yet() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let cy = employee(&o.db, "Cy", None);
        let sweep = saved_run(&o.db, &ada, standup("s1"), "Sweep.", RunState::Succeeded, 30, None);
        let later = saved_run(&o.db, &ada, RunKind::Manual, "Plan.", RunState::Running, 5, None);
        let office = held_office(&o).await;
        let first = run_id(&office.ask_colleague(&sweep.id, &ada, "Cy", "Which room?", "question").unwrap());
        let again = office.ask_colleague(&later.id, &ada, "Cy", "Book it for Friday.", "task").unwrap();
        assert_eq!(run_id(&again), first, "{again}");
        let note = again["note"].as_str().unwrap();
        assert!(note.starts_with("Cy had not started your earlier request yet, so Omega Star added this one to it, and one answer covers both."), "{note}");
        let r = saved(&o.db, &first);
        assert_eq!(r.prompt, "Which room?\n\nLater, Ada added:\n\nBook it for Friday.");
        assert!(matches!(r.kind, RunKind::Asked { task: true, .. }), "a task added makes it one");
        assert_eq!(o.db.active_runs_of(&cy.id).unwrap().len(), 1);

        // Not from a conversation with Pat, where its answer would land elsewhere, nor once it has started.
        let talking = saved_run(&o.db, &ada, task("Rooms"), "Find a room.", RunState::Running, 2, None);
        assert_ne!(run_id(&office.ask_colleague(&talking.id, &ada, "Cy", "Which floor?", "question").unwrap()), first);
        let mut started = r.clone();
        started.state = RunState::Running;
        o.db.save_run(&started).unwrap();
        assert_ne!(run_id(&office.ask_colleague(&later.id, &ada, "Cy", "And the projector?", "question").unwrap()), first);
        drop(office);
    }

    #[tokio::test]
    async fn news_starts_no_run_and_tops_the_next_one() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", None);
        let cy = employee(&o.db, "Cy", None);
        let sender = saved_run(&o.db, &ada, RunKind::Manual, "Ship it.", RunState::Running, 5, None);
        let office = held_office(&o).await;
        let one = office.ask_colleague(&sender.id, &ada, "Bo", "The new opening hours are up.", "info").unwrap();
        assert_eq!(one["note"], "Omega Star shows this to Bo at the top of their next run. It starts no run, and nothing comes back.");
        let all = office.ask_colleague(&sender.id, &ada, "everyone", "We close at noon on Friday.", "info").unwrap();
        assert_eq!(all["told"], json!(["Bo", "Cy"]));
        assert!(all["note"].as_str().unwrap().starts_with("Omega Star shows this to Bo and Cy at the top of their next run."), "{all}");
        assert!(o.db.active_runs_of(&bo.id).unwrap().is_empty() && o.db.active_runs_of(&cy.id).unwrap().is_empty(), "news starts no run");
        let err = office.ask_colleague(&sender.id, &ada, "everyone", "Who is free?", "question").unwrap_err();
        assert!(err.message.starts_with("Only news (kind info) goes to everyone."), "{}", err.message);
        drop(office);

        let employees = vec![ada.clone(), bo.clone(), cy.clone()];
        let tidy = Run::new(&bo.id, RunKind::Tidy { reason: TidyReason::Auto, signature: vec![], plan: None }, "Tidy.".into(), now());
        assert!(caught_up(&o.db, &employees, &tidy).news.is_empty(), "a tidy leaves it for real work");
        let next = Run::new(&bo.id, RunKind::Manual, "Go.".into(), now());
        let news: Vec<(String, String)> = caught_up(&o.db, &employees, &next).news.into_iter().map(|(_, who, text)| (who, text)).collect();
        assert_eq!(news, vec![("Ada".into(), "The new opening hours are up.".into()), ("Ada".into(), "We close at noon on Friday.".into())]);
        assert!(o.db.unread_news(&bo.id).unwrap().is_empty(), "read once shown");
        assert_eq!(o.db.unread_news(&cy.id).unwrap().len(), 1);
        // A run that never started gives back the news it took in, so the next one shows it.
        assert_eq!(o.db.unread_news_of(&next.id).unwrap(), 2);
        let again = Run::new(&bo.id, RunKind::Manual, "Go.".into(), now());
        assert_eq!(caught_up(&o.db, &employees, &again).news.len(), 2);
    }

    #[test]
    fn a_reports_finished_task_waits_for_the_managers_next_run() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let cy = employee(&o.db, "Cy", Some(&ada.id));
        let morning = saved_run(&o.db, &ada, standup("s1"), "Stand-up.", RunState::Succeeded, 60, None);
        o.db.mark_inited(&morning.id).unwrap();
        let shelf = saved_run(&o.db, &bo, delegated(&morning), "Count the shelf.", RunState::Succeeded, 20, None);
        let till = saved_run(&o.db, &cy, delegated(&morning), "Count the till.", RunState::Succeeded, 10, None);
        let queued = |db: &Store| db.active_runs_of(&ada.id).unwrap();

        assert!(due_answers(&o.db, &shelf).unwrap().is_empty(), "held, not brought back at once");
        assert!(delivered(&o.db, &shelf));
        let first = queued(&o.db).remove(0);
        assert_eq!(first.kind, RunKind::Answered { asked_run_id: shelf.id.clone(), from: bo.id.clone(), task: true, chat: false, thread: None });
        assert_eq!(first.reports, vec![shelf.id.clone()]);
        assert_eq!(o.db.run(&first.id).unwrap().unwrap().session, morning.id, "it resumes the stand-up");
        assert!(first.prompt.starts_with("Bo finished the task you handed them.\n\nYour task:\n\nCount the shelf.\n\nTheir report:\n\nBo did it."), "{}", first.prompt);
        let wait = first.not_before.unwrap() - now();
        assert!(wait > chrono::Duration::minutes(29) && wait <= chrono::Duration::minutes(30), "{wait:?}");

        assert!(due_answers(&o.db, &till).unwrap().is_empty());
        let both = queued(&o.db);
        assert_eq!(both.len(), 1, "one run brings them all");
        let both = &both[0];
        assert_eq!(both.reports, vec![shelf.id.clone(), till.id.clone()]);
        assert!(both.prompt.starts_with("2 reports from your team came back after your run had ended.\n\nBo finished the task"), "{}", both.prompt);
        assert!(both.prompt.contains("\n\n---\n\nCy finished the task you handed them.\n\nYour task:\n\nCount the till."), "{}", both.prompt);
        assert!(both.not_before >= first.not_before, "a new report pushes it back");

        // A conversation with Pat leaves them, and the next run of Ada's own work takes them in, in place of the held run.
        let employees = vec![ada.clone(), bo.clone(), cy.clone()];
        let chat = Run::new(&ada.id, task("Stock"), "How is the count going?".into(), now());
        assert!(caught_up(&o.db, &employees, &chat).held.is_empty());
        let next = saved_run(&o.db, &ada, RunKind::Manual, "Go.", RunState::Running, 0, None);
        let took = caught_up(&o.db, &employees, &next).held;
        let said = |who: &str, task: &str| (who.to_string(), task.to_string(), format!("{who} did it."));
        assert_eq!(took, vec![said("Bo", "Count the shelf."), said("Cy", "Count the till.")]);
        assert!(o.db.run(&both.id).unwrap().is_none(), "the held run is gone");
        assert_eq!(saved(&o.db, &shelf.id).noted_in, Some(next.id.clone()));

        // That run failed, so they are held again.
        let mut failed = saved(&o.db, &next.id);
        failed.state = RunState::Failed;
        o.db.save_run(&failed).unwrap();
        assert!(due_answers(&o.db, &failed).unwrap().is_empty());
        assert_eq!(queued(&o.db).into_iter().map(|r| r.reports).collect::<Vec<_>>(), vec![vec![shelf.id.clone(), till.id.clone()]]);
        assert_eq!(saved(&o.db, &shelf.id).noted_in, None);
    }

    #[test]
    fn a_task_handed_over_in_a_conversation_comes_back_at_once() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", Some(&ada.id));
        let talking = saved_run(&o.db, &ada, task("The stock"), "Get Bo to count the shelf.", RunState::Succeeded, 30, None);
        let counted = saved_run(&o.db, &bo, delegated(&talking), "Count the shelf.", RunState::Succeeded, 20, None);
        let due = due_answers(&o.db, &counted).unwrap();
        assert!(matches!(&due[0].0.kind, RunKind::Answered { thread: Some(t), .. } if *t == talking.id), "{due:?}");
    }

    #[test]
    fn an_answer_to_a_long_or_old_session_starts_a_fresh_one() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", None);
        let asker = |kind: RunKind, prompt: &str, ago: i64, context: u64| {
            let mut r = saved_run(&o.db, &ada, kind, prompt, RunState::Succeeded, ago, None);
            r.context_tokens = Some(context);
            o.db.save_run(&r).unwrap();
            o.db.mark_inited(&r.id).unwrap();
            r
        };
        let answer_to = |parent: &Run| {
            let child = saved_run(&o.db, &bo, asked(parent, false), "Which room?", RunState::Succeeded, 5, None);
            due_answers(&o.db, &child).unwrap().remove(0)
        };
        let (a, session) = answer_to(&asker(RunKind::Manual, "Plan the launch.", 30, 150_000));
        assert_eq!(session, a.id, "a fresh session");
        assert!(a.prompt.contains("\n\nYour run had started from:\n\nPlan the launch.\n\nYour question:"), "{}", a.prompt);
        let (a, session) = answer_to(&asker(RunKind::Manual, "Plan the week.", 2 * 24 * 60, 20_000));
        assert_eq!(session, a.id, "a session over a day old");
        let recent = asker(RunKind::Manual, "Plan the day.", 30, 20_000);
        assert_eq!(answer_to(&recent).1, recent.id, "a short, recent one resumes");
        // Never for a conversation with Pat.
        let talking = asker(task("The offsite"), "Plan the offsite.", 3 * 24 * 60, 150_000);
        let (a, session) = answer_to(&talking);
        assert_eq!(session, talking.id);
        assert!(matches!(&a.kind, RunKind::Answered { thread: Some(t), .. } if *t == talking.id));
    }

    #[test]
    fn a_failed_answer_goes_again_twice_then_stays_failed() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let bo = employee(&o.db, "Bo", None);
        let parent = saved_run(&o.db, &ada, RunKind::Manual, "Plan the launch.", RunState::Succeeded, 30, None);
        o.db.mark_inited(&parent.id).unwrap();
        let answer_for = |question: &str| {
            let child = saved_run(&o.db, &bo, asked(&parent, false), question, RunState::Succeeded, 20, None);
            due_answers(&o.db, &child).unwrap().remove(0)
        };
        let (mut answer, session) = answer_for("Which room?");
        answer.reports = vec!["r9".into()];
        let mut tries = 0;
        loop {
            assert!(o.db.insert_run(&answer, &session).unwrap());
            answer.state = if tries == 1 { RunState::Lost } else { RunState::Failed };
            answer.ended_at = Some(now());
            o.db.save_run(&answer).unwrap();
            let Some((again, s)) = due_answers(&o.db, &answer).unwrap().pop() else { break };
            tries += 1;
            assert_eq!((&again.kind, &again.prompt, &again.reports, &s), (&answer.kind, &answer.prompt, &answer.reports, &session));
            assert_eq!(again.redelivered, tries);
            let wait = again.not_before.unwrap() - now();
            assert!(wait > chrono::Duration::minutes(9) && wait <= chrono::Duration::minutes(10), "{wait:?}");
            answer = again;
        }
        assert_eq!(tries, 2, "two more tries, then it stays failed");

        let (mut stopped, session) = answer_for("Which floor?");
        assert!(o.db.insert_run(&stopped, &session).unwrap());
        stopped.state = RunState::Cancelled;
        o.db.save_run(&stopped).unwrap();
        assert!(due_answers(&o.db, &stopped).unwrap().is_empty(), "you stopped it");
    }

    #[test]
    fn the_next_run_hears_which_cards_were_replaced_since_the_last() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let replaced = |message: &str, mins: i64| {
            let mut r = saved_run(&o.db, &ada, RunKind::Manual, "Go.", RunState::Succeeded, 300, None);
            let mut card = Card::new(message, None, Some(format!("ask:card-{mins}")), CardAsk::Choices { choices: vec!["Go".into()], recommended: 0 });
            card.answer = Some(CardAnswer { at: now() - chrono::Duration::minutes(mins), how: AnswerHow::Replaced, text: String::new(), held: false });
            r.cards = vec![card];
            o.db.save_run(&r).unwrap();
        };
        replaced("Order more paper?", 180);
        replaced("Book the bigger room?", 60);
        let started = |kind: RunKind, mins: i64| {
            let mut r = saved_run(&o.db, &ada, kind, "Go.", RunState::Succeeded, mins, None);
            r.started_at = Some(now() - chrono::Duration::minutes(mins));
            o.db.save_run(&r).unwrap();
        };
        started(RunKind::Manual, 120);
        started(RunKind::Tidy { reason: TidyReason::Auto, signature: vec![], plan: None }, 30);
        let next = saved_run(&o.db, &ada, RunKind::Manual, "Go.", RunState::Running, 0, None);
        let said: Vec<String> = caught_up(&o.db, std::slice::from_ref(&ada), &next).replaced.into_iter().map(|(_, c)| c.message).collect();
        assert_eq!(said, vec!["Book the bigger room?".to_string()], "the last run heard of the earlier one, and a tidy hears of none");
    }
}
