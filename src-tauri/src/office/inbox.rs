//! Threads and Inbox rows, replies, archiving and approving blocked tools.

use super::*;
use crate::cards;

/// How many archived reports the Archive reads, most recently archived first.
const ARCHIVE_ROWS: u32 = 200;

/// Who wrote a reply: you, or Omega Star from something you did in the app (an approval, or your answer
/// on a card).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ReplyBy {
    Ceo,
    App,
    Card,
}

impl Office {
    /// Colleague asks still in flight, for the Inbox's "With colleagues" list.
    pub fn open_asks(&self) -> Result<Vec<Run>> {
        self.db().open_asks()
    }

    /// The Archive: rows you archived, most recently archived first.
    pub fn archived(&self) -> Result<Vec<Thread>> {
        Self::rows(&self.db(), true)
    }

    /// The employees who report to you.
    pub(super) fn top_ids(employees: &[Employee]) -> HashSet<String> {
        employees.iter().filter(|e| e.manager_id.is_none()).map(|e| e.id.clone()).collect()
    }

    /// The run of failures in a row a scheduled run that broke belongs to: its schedule and the last
    /// success before it. None for anything else. `cache` keeps each schedule's successes.
    fn streak_of(db: &Store, cache: &std::cell::RefCell<HashMap<String, Vec<Ts>>>, r: &Run) -> Option<String> {
        let RunKind::Scheduled { schedule_id, .. } = &r.kind else { return None };
        if !threads::broke(r) {
            return None;
        }
        let mut cache = cache.borrow_mut();
        let ok = cache.entry(schedule_id.clone()).or_insert_with(|| db.successes_of_schedule(schedule_id).unwrap_or_default());
        let since = ok.iter().rev().find(|t| **t < r.created_at).map(|t| t.timestamp_millis()).unwrap_or(0);
        Some(format!("{schedule_id}@{since}"))
    }

    /// The Inbox, or with `archive` the Archive, as rows (`threads::file`): one row per piece of work, most
    /// recently active (or archived) first. A row is a thread (a report and your exchange on it) with the
    /// threads that belong with it in `related`: everything a hand-off between employees set off, or a
    /// schedule's earlier failures in a row.
    pub(super) fn rows(db: &Store, archive: bool) -> Result<Vec<Thread>> {
        let employees = db.employees()?;
        let top = Self::top_ids(&employees);
        let all: Vec<String> = employees.into_iter().map(|e| e.id).collect();
        let roots: Vec<String> = top.iter().cloned().collect();
        // The Archive reads open runs too, so a row with anything still open stays out of it.
        let mut starts = db.finished_root_runs(&roots, &all)?;
        starts.retain(|r| threads::reaches_you(r, false, top.contains(&r.employee_id)));
        if archive {
            let mut archived = db.archived_root_runs(&all, ARCHIVE_ROWS)?;
            archived.retain(|r| threads::reaches_you(r, false, true));
            starts.extend(archived);
        }
        let rows = threads::file(&starts, db.conversation_runs()?, |id| run_of(db, id), &top);
        let cache = std::cell::RefCell::new(HashMap::new());
        let rows = threads::merge_rows(rows, |row| Self::row_key(db, &cache, row));
        let mut out: Vec<Thread> = rows.into_iter().filter_map(|r| threads::present(r, archive)).collect();
        if archive {
            out.sort_by_key(|t| std::cmp::Reverse(threads::row_archived_at(t)));
        } else {
            out.sort_by_key(|t| std::cmp::Reverse(threads::activity_at(&t.root, &t.replies)));
        }
        Ok(out)
    }

    /// What a row belongs to, so rows with the same key become one: a chore waiting on you first, then a
    /// week's reviews, then a schedule's run of failures.
    fn row_key(db: &Store, cache: &std::cell::RefCell<HashMap<String, Vec<Ts>>>, row: &threads::Row) -> Option<String> {
        let lead = row.members.first().map(|(t, _)| &t.root);
        threads::chore_key(row)
            .or_else(|| lead.and_then(threads::review_week))
            .or_else(|| lead.and_then(|r| Self::streak_of(db, cache, r)))
    }

    /// Every thread in the row `root` (a thread's root) is filed under, oldest first: its hand-off, for a
    /// schedule's failure the rest of that run of failures, and for a weekly review the team's other
    /// reviews that week.
    fn row_of(db: &Store, root: &Run) -> Result<Vec<Thread>> {
        let top = Self::top_ids(&db.employees()?);
        let conversation = db.conversation_runs()?;
        let lookup = |id: &str| run_of(db, id);
        let cache = std::cell::RefCell::new(HashMap::new());
        let mut starts = vec![root.clone()];
        let first = threads::file(&starts, conversation.clone(), lookup, &top).into_iter().next().and_then(|r| r.members.into_iter().next());
        if let Some((t, _)) = first {
            if let Some(week) = threads::review_week(&t.root) {
                let same = |r: &Run| r.id != t.root.id && threads::review_week(r).as_ref() == Some(&week);
                starts.extend(db.review_runs(t.root.created_at - chrono::Duration::days(8))?.into_iter().filter(same));
            } else if let (Some(key), RunKind::Scheduled { schedule_id, .. }) = (Self::streak_of(db, &cache, &t.root), &t.root.kind) {
                let same = |f: &Run| f.id != t.root.id && Self::streak_of(db, &cache, f).as_ref() == Some(&key);
                starts.extend(db.failures_of_schedule(schedule_id)?.into_iter().filter(same));
            }
        }
        // Keyed as the Inbox keys them, so a review held in a chore's row neither joins the week nor brings it along.
        let rows = threads::merge_rows(threads::file(&starts, conversation, lookup, &top), |row| Self::row_key(db, &cache, row));
        let at = rows.iter().position(|r| r.members.iter().any(|(t, _)| t.root.id == root.id)).unwrap_or(0);
        Ok(rows.into_iter().nth(at).map(|r| r.members.into_iter().map(|(t, _)| t).collect()).unwrap_or_default())
    }

    /// The thread a run belongs to: the report or task (root) and every reply on it, oldest first.
    pub(super) fn thread_in(db: &Store, run_id: &str) -> Result<Thread> {
        let run = db.run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
        let root = match run.reply_to() {
            Some(id) => db.run(id)?.map(|r| r.run).ok_or_else(|| AppError::not_found("the report this reply answers is gone"))?,
            None => run,
        };
        let replies = threads::group(db.reply_runs(Some(&root.employee_id))?).remove(&root.id).unwrap_or_default();
        Ok(Thread::new(root, replies))
    }

    pub fn thread(&self, run_id: &str) -> Result<Thread> {
        Self::thread_in(&self.db(), run_id)
    }

    /// A reply that lands on an archived thread brings it back to the Inbox only when it needs you:
    /// "Send & archive" meant you did not want the answer back, unless the answer raises something.
    pub(super) fn surface_thread(&self, reply: &Run) -> Result<()> {
        if reply.reply_to().is_none() {
            return Ok(());
        }
        let flagged = !reply.needs_you.is_empty() || !reply.denials.is_empty() || matches!(reply.state, RunState::Failed | RunState::Lost);
        if !flagged {
            return Ok(());
        }
        let root = {
            let db = self.db();
            let Some(root_id) = reply.reply_to() else { return Ok(()) };
            let Some(mut root) = db.run(root_id)?.map(|r| r.run) else { return Ok(()) };
            if root.archived_at.is_none() {
                return Ok(());
            }
            root.archived_at = None;
            db.save_run(&root)?;
            root
        };
        self.sink().run_updated(&root);
        Ok(())
    }

    /// Archives a report with its thread and the rest of its Inbox row (they leave the Inbox and the
    /// employee's chips), or moves them all back. `run_id` may be any run in the row.
    pub fn set_archived(&self, run_id: &str, archived: bool) -> Result<Run> {
        let (root, changed) = {
            let db = self.db();
            let t = Self::thread_in(&db, run_id)?;
            if !t.root.state.is_finished() {
                return Err(AppError::conflict("it is still working, so it cannot be archived yet"));
            }
            let row = Self::row_of(&db, &t.root)?;
            if archived && threads::card_open(&row) {
                return Err(AppError::conflict("Answer the card first, or dismiss it."));
            }
            let mut changed = vec![];
            for m in row {
                // A run still going joins the row when it finishes, and shows only if it needs you.
                let mut root = m.root;
                if !root.state.is_finished() || root.archived_at.is_some() == archived {
                    continue;
                }
                root.archived_at = archived.then(now);
                db.save_run(&root)?;
                changed.push(root);
            }
            let root = db.run(&t.root.id)?.map(|r| r.run).unwrap_or(t.root);
            (root, changed)
        };
        for r in &changed {
            self.sink().run_updated(r);
        }
        Ok(root)
    }

    /// Your reply in a report's thread. It goes into the thread's run while that works, or else starts a
    /// run that resumes the thread's session, so the employee answers with everything it saw. The reply
    /// joins the thread and the report stays in the Inbox, unless `archive`: "send, and bring the answer
    /// back only if it needs me".
    /// A reply you type answers the thread's open cards too, once it has gone, so their buttons go and the row
    /// can be archived.
    pub fn reply_to_run(&self, run_id: &str, text: &str, archive: bool, attachments: Vec<NewAttachment>) -> Result<Run> {
        let reply = self.reply_with(run_id, text, archive, vec![], attachments, ReplyBy::Ceo)?;
        let typed = {
            let db = self.db();
            db.tx(|| answer_typed(&db, run_id, text, now()))?
        };
        for r in &typed {
            self.sink().run_updated(r);
        }
        Ok(reply)
    }

    /// Your answer on a card. It goes to the run as a fixed message through `reply_with` and archives its row;
    /// Done on a chore goes to every run waiting on it, and those still working hear it straight away. Returns
    /// the reply runs it started.
    pub fn answer_card(&self, run_id: &str, reply: CardReply) -> Result<Vec<Run>> {
        let at = now();
        let (ask, text, targets) = {
            let db = self.db();
            let run = db.run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
            let card = run.card.clone().ok_or_else(|| AppError::invalid("That run has no card."))?;
            if !card.is_open() {
                return Err(AppError::conflict("That card was already answered."));
            }
            if !run.state.is_finished() {
                return Err(AppError::conflict("It is still working, so answer the card when it finishes."));
            }
            let text = cards::answer_text(&card.ask, &reply, &ceo_of(&db)?)?;
            let how = if reply == CardReply::Dismiss { AnswerHow::Dismissed } else { AnswerHow::Tapped };
            let mut targets = vec![run];
            if let (CardAsk::Chore { subject }, CardReply::Done) = (&card.ask, &reply) {
                targets.extend(db.open_chore_runs(subject)?.into_iter().filter(|r| r.id != run_id));
            }
            let answer = CardAnswer { at, how, text: text.clone().unwrap_or_default() };
            db.tx(|| answer_cards(&db, &mut targets, &answer))?;
            (card.ask, text, targets)
        };
        let mut replies = self.send_card_answers(&targets, &ask, text.as_deref());
        if let (CardAsk::Chore { subject }, Some(text)) = (&ask, &text) {
            let is_chore = |c: &Card| c.is_open() && matches!(&c.ask, CardAsk::Chore { subject: s } if s == subject);
            let answer = CardAnswer { at, how: AnswerHow::Tapped, text: text.clone() };
            // Runs still working on the same chore hear it now, in the turn they are in. Told and marked under one
            // lock, so a run ending meanwhile either keeps the answer or is found by the read below.
            for a in lock(&self.0.active).values_mut() {
                let Some(c) = a.card.as_mut().filter(|c| is_chore(c)) else { continue };
                if a.input.as_ref().is_some_and(|i| i.send(claude::user_line(text)).is_ok()) {
                    c.answer = Some(answer.clone());
                }
            }
            // A run that finished while the answers above went out was in neither pass.
            let late = {
                let db = self.db();
                db.open_chore_runs(subject).and_then(|mut late| db.tx(|| answer_cards(&db, &mut late, &answer)).map(|_| late))
            };
            match late {
                Ok(late) => replies.extend(self.send_card_answers(&late, &ask, Some(text))),
                Err(e) => log!("answering the runs that finished waiting on the chore failed: {e}"),
            }
        }
        Ok(replies)
    }

    /// Sends each answered card's `text` to its run and archives its row, carrying on past a run that fails.
    /// Returns the reply runs it started.
    fn send_card_answers(&self, targets: &[Run], ask: &CardAsk, text: Option<&str>) -> Vec<Run> {
        let mut replies = vec![];
        for t in targets {
            self.sink().run_updated(t);
            if let Some(text) = text {
                let sent = match (ask, &t.card) {
                    (CardAsk::Chore { .. }, _) | (_, None) => text.to_string(),
                    (_, Some(c)) => format!("Your card: \"{}\"\n{text}", claude::clip(&c.message, 300)),
                };
                match self.reply_with(&t.id, &sent, false, vec![], vec![], ReplyBy::Card) {
                    Ok(r) => replies.push(r),
                    Err(e) => {
                        log!("sending your answer on the card of run {} failed: {e}", t.id);
                        continue;
                    }
                }
            }
            match self.set_archived(&t.id, true) {
                Err(e) if e.code != crate::error::ErrorCode::Conflict => log!("archiving run {} after its card failed: {e}", t.id),
                _ => {}
            }
        }
        replies
    }

    /// A blocked run's way forward: you allow what it was denied, and it resumes to retry. `always`
    /// saves the rules to the employee; otherwise they apply to the retry run only. The rules must
    /// come from what the run was denied: its own suggested rule, or every tool of the same MCP server.
    pub fn approve_blocked(&self, run_id: &str, rules: Vec<String>, always: bool) -> Result<Run> {
        let (original, employee, ceo) = {
            let db = self.db();
            let original = db.run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
            if original.denials.is_empty() {
                return Err(AppError::conflict("nothing was blocked in that run"));
            }
            if !original.state.is_finished() {
                return Err(AppError::conflict("that run is still working"));
            }
            let employee = db.employee(&original.employee_id)?.ok_or_else(|| AppError::not_found("that employee no longer works here"))?;
            let ceo = ceo_of(&db)?;
            (original, employee, ceo)
        };
        let mut rules: Vec<String> = rules.into_iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
        rules.dedup();
        if rules.is_empty() {
            return Err(AppError::invalid("choose what to allow"));
        }
        for r in &rules {
            let asked_for = original.denials.iter().any(|d| d.suggested_rule == *r || claude::mcp_server_rule(&d.tool).as_deref() == Some(r.as_str()));
            if !asked_for {
                return Err(AppError::invalid(format!("'{r}' is not something this run was blocked on")));
            }
        }
        if always {
            for r in &rules {
                self.allow_rule(&employee.id, r)?;
            }
        }
        let blocked = original.denials.iter().map(|d| format!("- {}: {}", d.tool, d.brief)).collect::<Vec<_>>().join("\n");
        let scope = if always { "from now on" } else { "for this retry" };
        let text = format!(
            "{ceo} approved what was blocked:\n{blocked}\n\nYou may now use {} {scope}. Retry it and finish the task. \
Approval covers only what is listed, so if something else is blocked, stop and say what.",
            rules.join(", ")
        );
        self.reply_with(run_id, &text, false, rules, vec![], ReplyBy::App)
    }

    /// `run_id` may be any run in the thread. The reply answers the thread's root and resumes the
    /// thread's newest session (the root's, or a fresh one an earlier reply had to start). A plain reply
    /// goes into that session's run while it works; one that carries files or rules, or that Omega Star
    /// wrote (`by`), is a run of its own.
    fn reply_with(&self, run_id: &str, text: &str, archive: bool, allow_once: Vec<String>, attachments: Vec<NewAttachment>, by: ReplyBy) -> Result<Run> {
        let text = text.trim();
        if text.is_empty() && attachments.is_empty() {
            return Err(AppError::invalid("reply is empty"));
        }
        let files = Self::decode_attachments(&attachments)?;
        let plain = files.is_empty() && allow_once.is_empty() && by == ReplyBy::Ceo;
        let (root, mut reply, session) = {
            let db = self.db();
            let t = Self::thread_in(&db, run_id)?;
            let mut root = t.root;
            let e = db.employee(&root.employee_id)?.ok_or_else(|| AppError::not_found("that employee no longer works here"))?;
            let latest = t.replies.last().cloned().unwrap_or_else(|| root.clone());
            if plain && latest.state == RunState::Running && self.steer(&latest.id, text) {
                root.replied_at = Some(now());
                root.archived_at = archive.then(now);
                db.save_run(&root)?;
                drop(db);
                self.sink().run_updated(&root);
                return Ok(latest);
            }
            let rec = db.run(&latest.id)?.ok_or_else(|| AppError::not_found("run not found"))?;
            let label = threads::report_label(&root.kind);
            let task = matches!(root.kind, RunKind::Task { .. });
            // A turn still to come has the session, even before it starts.
            let (prompt, session) = if !rec.run.state.is_finished() || db.session_started(&rec.session)? {
                (text.to_string(), rec.session)
            } else {
                // The session never started (it failed first), so there is nothing to resume.
                let report = rec.run.summary.clone().or_else(|| rec.run.error.clone()).unwrap_or_else(|| "(no report)".into());
                let ceo = db.company()?.map(|c| c.ceo_name).unwrap_or_else(|| "Your manager".into());
                let before = if task {
                    format!("{ceo}'s task ({label}):\n\n{}\n\nWhat came of it:\n\n{report}", root.prompt)
                } else {
                    format!("Your report ({label}):\n\n{report}")
                };
                (format!("{before}\n\n{ceo}'s reply:\n\n{text}"), String::new())
            };
            let card = by == ReplyBy::Card;
            let mut reply = Run::new(&e.id, RunKind::Reply { to_run_id: root.id.clone(), label, task, card }, prompt, now());
            reply.allow_once = allow_once;
            // The never-started wrapper quotes your reply in full, so it still counts as your words.
            reply.omega_wrote = by != ReplyBy::Ceo;
            let session = if session.is_empty() { reply.id.clone() } else { session };
            root.replied_at = Some(now());
            root.archived_at = archive.then(now);
            db.save_run(&root)?;
            (root, reply, session)
        };
        self.attach(&mut reply, &files)?;
        self.sink().run_updated(&root);
        self.enqueue(reply, session)
    }
}

/// Marks every open card on a finished run of `run_id`'s thread answered by your typed reply (`text`,
/// clipped), and returns those runs, saved. Call inside a transaction.
fn answer_typed(db: &Store, run_id: &str, text: &str, at: Ts) -> Result<Vec<Run>> {
    let t = Office::thread_in(db, run_id)?;
    let mut out = vec![];
    for mut r in std::iter::once(t.root).chain(t.replies) {
        if !r.state.is_finished() {
            continue;
        }
        let Some(c) = r.card.as_mut().filter(|c| c.is_open()) else { continue };
        c.answer = Some(CardAnswer { at, how: AnswerHow::Typed, text: claude::clip(text, 300) });
        db.save_run(&r)?;
        out.push(r);
    }
    Ok(out)
}

/// Puts `answer` on each run's card and saves them. Call inside a transaction.
fn answer_cards(db: &Store, runs: &mut [Run], answer: &CardAnswer) -> Result<()> {
    for r in runs {
        if let Some(c) = r.card.as_mut() {
            c.answer = Some(answer.clone());
        }
        db.save_run(r)?;
    }
    Ok(())
}

/// A run by id while walking threads. A store error is logged and the run treated as missing, since
/// the thread walkers take a plain lookup.
pub(super) fn run_of(db: &Store, id: &str) -> Option<Run> {
    match db.run(id) {
        Ok(r) => r.map(|r| r.run),
        Err(e) => {
            log!("reading run {id}: {e}");
            None
        }
    }
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

    fn tester() -> Employee {
        Employee {
            id: uuid::Uuid::new_v4().to_string(),
            slug: "cy".into(),
            name: "Cy".into(),
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
            chat_session: "chat-cy".into(),
            hired_at: now(),
        }
    }

    /// A finished report of `e` asking a decision with a card.
    fn carded(e: &Employee) -> Run {
        let mut r = Run::new(&e.id, RunKind::Manual, "go".into(), now());
        r.state = RunState::Succeeded;
        r.ended_at = Some(now());
        r.needs_you = vec!["Boost the post?".into()];
        r.card = Some(Card { message: "Boost the post?".into(), ask: CardAsk::Choices { choices: vec!["Not now".into(), "Go".into()], recommended: 0 }, answer: None });
        r
    }

    #[test]
    fn a_reply_you_type_answers_the_threads_open_cards() {
        let dir = std::env::temp_dir().join(format!("omegastar-typed-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let e = tester();
        db.save_employee(&e).unwrap();
        let report = carded(&e);
        assert!(db.insert_run(&report, &report.id).unwrap());
        let mut working = carded(&e);
        working.kind = RunKind::Reply { to_run_id: report.id.clone(), label: "Manual run".into(), task: false, card: false };
        working.state = RunState::Running;
        assert!(db.insert_run(&working, &report.id).unwrap());

        let long = format!("Not now. {}", "x".repeat(400));
        let typed = db.tx(|| answer_typed(&db, &working.id, &long, now())).unwrap();
        assert_eq!(typed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![report.id.as_str()]);
        let answer = db.run(&report.id).unwrap().unwrap().run.card.unwrap().answer.unwrap();
        assert_eq!(answer.how, AnswerHow::Typed);
        assert_eq!(answer.text, claude::clip(&long, 300));
        assert!(db.run(&working.id).unwrap().unwrap().run.card.unwrap().is_open(), "a run still working keeps its card");
        assert!(db.tx(|| answer_typed(&db, &report.id, "Again.", now())).unwrap().is_empty());
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_weeks_reviews_share_one_row() {
        let dir = std::env::temp_dir().join(format!("omegastar-team-week-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let hire = |slug: &str, manager: Option<&Employee>| {
            let e = Employee { slug: slug.into(), name: slug.into(), chat_session: format!("chat-{slug}"), manager_id: manager.map(|m| m.id.clone()), ..tester() };
            db.save_employee(&e).unwrap();
            e
        };
        let ada = hire("ada", None);
        let bo = hire("bo", Some(&ada));
        let cy = hire("cy", None);
        // Wednesdays at noon UTC, in ISO weeks 41 and 42 in every time zone.
        let (this_week, next_week) = ("2026-10-07T12:00:00Z".parse::<Ts>().unwrap(), "2026-10-14T12:00:00Z".parse::<Ts>().unwrap());
        let review = |e: &Employee, slot: Ts, state: RunState| {
            let kind = RunKind::Scheduled { schedule_id: format!("review-{}", e.slug), slot, label: "Weekly review".into(), quiet: false, model: None, effort: None, review: true };
            let mut r = Run::new(&e.id, kind, "Review your week.".into(), slot);
            r.state = state;
            r.ended_at = Some(slot);
            r.summary = Some("Signups 40, up 5.".into());
            assert!(db.insert_run(&r, &r.id).unwrap());
            r
        };
        let a = review(&ada, this_week, RunState::Succeeded);
        let b = review(&bo, this_week, RunState::Succeeded);
        let c = review(&cy, this_week, RunState::Failed);
        let later = review(&ada, next_week, RunState::Succeeded);
        // A review waiting on a chore sits in the chore's row, as the Inbox files it.
        let dee = hire("dee", None);
        let mut d = review(&dee, "2026-10-08T12:00:00Z".parse::<Ts>().unwrap(), RunState::Succeeded);
        d.needs_you = vec!["Open Chrome.".into()];
        d.card = Some(Card { message: "Open Chrome.".into(), ask: CardAsk::Chore { subject: "chrome".into() }, answer: None });
        db.save_run(&d).unwrap();

        let ids = |t: &Thread| -> HashSet<String> { std::iter::once(t).chain(&t.related).map(|x| x.root.id.clone()).collect() };
        let week: HashSet<String> = [&a, &b, &c].iter().map(|r| r.id.clone()).collect();
        let rows = Office::rows(&db, false).unwrap();
        assert_eq!(rows.len(), 3, "the second week is a row of its own");
        assert_eq!(ids(&rows[0]), HashSet::from([later.id.clone()]));
        assert_eq!(ids(&rows[1]), HashSet::from([d.id.clone()]));
        assert_eq!(ids(&rows[2]), week, "the failed review is in the team's row, not a streak");
        let chored = Office::row_of(&db, &d).unwrap();
        assert_eq!(chored.iter().map(|t| t.root.id.clone()).collect::<Vec<_>>(), vec![d.id.clone()], "answering the chore leaves the week alone");

        let row = Office::row_of(&db, &b).unwrap();
        assert_eq!(row.iter().map(|t| t.root.id.clone()).collect::<HashSet<_>>(), week, "a second-level employee's review leads to the whole week");
        for t in row {
            let mut r = t.root;
            r.archived_at = Some(now());
            db.save_run(&r).unwrap();
        }
        let rows = Office::rows(&db, false).unwrap();
        assert_eq!(rows.iter().map(ids).collect::<Vec<_>>(), vec![HashSet::from([later.id.clone()]), HashSet::from([d.id.clone()])]);
        let archived = Office::rows(&db, true).unwrap();
        assert_eq!(archived.iter().map(ids).collect::<Vec<_>>(), vec![week]);
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_open_card_keeps_its_row_in_the_inbox() {
        let dir = std::env::temp_dir().join(format!("omegastar-card-row-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let e = tester();
        let report = carded(&e);
        {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&e).unwrap();
            assert!(db.insert_run(&report, &report.id).unwrap());
        }
        let office = Office::open(dir.clone(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        assert_eq!(office.set_archived(&report.id, true).unwrap_err().message, "Answer the card first, or dismiss it.");
        assert!(office.set_archived(&report.id, false).is_ok(), "moving it back is never blocked");
        assert_eq!(office.answer_card(&report.id, CardReply::Choice { index: 5 }).unwrap_err().message, "That answer does not fit this card.");
        let unreadable = vec![NewAttachment { name: "notes.txt".into(), data: "not base64!".into() }];
        assert_eq!(office.reply_to_run(&report.id, "Not now.", false, unreadable).unwrap_err().message, "notes.txt could not be read");
        assert!(office.db().run(&report.id).unwrap().unwrap().run.card.unwrap().is_open(), "a reply that did not go leaves the card open");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Every row of every table, each column as text, with `archivedAt` left out of run data.
    fn every_row(path: &Path) -> Vec<(String, Vec<String>)> {
        let c = rusqlite::Connection::open(path).unwrap();
        let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").unwrap();
        let tables: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let mut out = vec![];
        for t in tables {
            let mut st = c.prepare(&format!("SELECT * FROM {t} ORDER BY rowid")).unwrap();
            let names: Vec<String> = st.column_names().into_iter().map(String::from).collect();
            let rows = st
                .query_map([], |r| {
                    let mut cols = vec![];
                    for (i, name) in names.iter().enumerate() {
                        let v: rusqlite::types::Value = r.get(i)?;
                        cols.push(match v {
                            rusqlite::types::Value::Text(s) if t == "runs" && name == "data" => {
                                let mut j: Value = serde_json::from_str(&s).unwrap();
                                j.as_object_mut().unwrap().remove("archivedAt");
                                j.to_string()
                            }
                            v => format!("{v:?}"),
                        });
                    }
                    Ok(cols)
                })
                .unwrap();
            for row in rows {
                out.push((t.clone(), row.unwrap()));
            }
        }
        out
    }

    #[tokio::test]
    async fn archiving_a_replied_thread_writes_only_archived_at() {
        let dir = std::env::temp_dir().join(format!("omegastar-inbox-{}", uuid::Uuid::new_v4()));
        let path = dir.join("office.db");
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
        let mut report = Run::new(&ada.id, RunKind::Manual, "go".into(), now());
        report.state = RunState::Succeeded;
        report.ended_at = Some(now());
        report.summary = Some("All good.".into());
        report.replied_at = Some(now());
        let mut reply = Run::new(&ada.id, RunKind::Reply { to_run_id: report.id.clone(), label: "Manual run".into(), task: false, card: false }, "Thanks.".into(), now());
        reply.state = RunState::Succeeded;
        reply.ended_at = Some(now());
        reply.summary = Some("Noted.".into());
        {
            let db = Store::open(&path).unwrap();
            db.save_employee(&ada).unwrap();
            assert!(db.insert_run(&report, &report.id).unwrap());
            assert!(db.insert_run(&reply, &report.id).unwrap());
        }

        let office = Office::open(dir.clone(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap();
        let before = every_row(&path);
        let archived = office.set_archived(&reply.id, true).unwrap();
        assert_eq!(archived.id, report.id);
        assert!(archived.archived_at.is_some());
        assert_eq!(every_row(&path), before, "nothing but archivedAt changed");
        let c = rusqlite::Connection::open(&path).unwrap();
        let mut st = c.prepare("SELECT id FROM runs WHERE json_extract(data, '$.archivedAt') IS NOT NULL").unwrap();
        let stamped: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(stamped, vec![report.id.clone()]);

        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }
}
