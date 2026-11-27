//! Threads and Inbox rows, replies, archiving and approving blocked tools.

use super::*;
use crate::cards;
use crate::threads::View;

/// How many rows the Archive shows, and how many of each kind of run it reads for them, newest first.
const ARCHIVE_ROWS: u32 = 200;

/// Who wrote a reply: you, or OmegaStar from something you did in the app (an approval, or your answer
/// on a card). `Question` is you too, asking about a card (`Run::about_card`).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ReplyBy {
    Ceo,
    Question,
    App,
    Card,
}

impl Office {
    /// Colleague asks still in flight, for Activity's "Waiting on others".
    pub fn open_asks(&self) -> Result<Vec<Run>> {
        self.db().open_asks()
    }

    /// Weekly reviews from the last eight days, for Activity's Numbers.
    pub fn week_reviews(&self) -> Result<Vec<Run>> {
        self.db().review_runs(now() - chrono::Duration::days(8))
    }

    /// The Archive: every row that reached you and needs nothing from you now, the rows you archived, and the
    /// conversations you started and marked done, newest first (`threads::archive_at`).
    pub fn archived(&self) -> Result<Vec<Thread>> {
        Self::rows(&self.db(), View::Archive)
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

    /// The Inbox or the Archive (`view`) as rows (`threads::file`): one row per piece of work, most recently
    /// active (or archived) first. A row is a thread (a report and your exchange on it) with the threads that
    /// belong with it in `related`: everything a hand-off between employees set off, a schedule's earlier
    /// failures in a row, or the team's other reviews that week. The Inbox reads only runs that might need
    /// you; the Archive reads the newest of everything that reached you, and adds the conversations you
    /// started and marked done, each a line of its own.
    pub(super) fn rows(db: &Store, view: View) -> Result<Vec<Thread>> {
        let employees = db.employees()?;
        let top = Self::top_ids(&employees);
        let all: Vec<String> = employees.into_iter().map(|e| e.id).collect();
        let roots: Vec<String> = top.iter().cloned().collect();
        // The Archive reads the runs that might need you too, so a row that needs you is whole and stays out of it.
        let mut starts = db.finished_root_runs(&roots, &all)?;
        if view == View::Archive {
            starts.extend(db.reached_root_runs(&roots, &all, ARCHIVE_ROWS)?);
        }
        // A review brings the team's other reviews that week, as `row_of` does, so a week that needs you is one
        // whole row in the Inbox and none of it is left in the Archive. Only current employees': a review by
        // someone you let go would lead a row nobody can show.
        let weeks: HashSet<String> = starts.iter().filter_map(threads::review_week).collect();
        if let Some(first) = starts.iter().filter(|r| threads::review_week(r).is_some()).map(|r| r.created_at).min() {
            let have: HashSet<String> = starts.iter().map(|r| r.id.clone()).collect();
            let same = |r: &Run| !have.contains(&r.id) && all.contains(&r.employee_id) && threads::review_week(r).is_some_and(|w| weeks.contains(&w));
            starts.extend(db.review_runs(first - chrono::Duration::days(8))?.into_iter().filter(same));
        }
        starts.retain(|r| threads::reaches_you(r, false, top.contains(&r.employee_id)));
        if view == View::Archive {
            let mut archived = db.archived_root_runs(&all, ARCHIVE_ROWS)?;
            archived.retain(|r| threads::reaches_you(r, false, true));
            starts.extend(archived);
        }
        let rows = threads::file(&starts, db.conversation_runs()?, |id| run_of(db, id), &top);
        let cache = std::cell::RefCell::new(HashMap::new());
        let rows = threads::merge_rows(rows, |row| Self::row_key(db, &cache, row));
        let mut out: Vec<Thread> = rows.into_iter().filter_map(|r| threads::present(r, view)).collect();
        if view == View::Archive {
            // A conversation you started is never filed into a row: once done, it is a line of its own.
            let mut replies = threads::group(db.reply_runs(None)?);
            out.extend(db.archived_tasks(ARCHIVE_ROWS)?.into_iter().map(|root| {
                let turns = replies.remove(&root.id).unwrap_or_default();
                Thread::new(root, turns)
            }));
            out.sort_by_key(|t| std::cmp::Reverse(threads::archive_at(t)));
            out.truncate(ARCHIVE_ROWS as usize);
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
        let employees = db.employees()?;
        let top = Self::top_ids(&employees);
        let current: HashSet<&str> = employees.iter().map(|e| e.id.as_str()).collect();
        let conversation = db.conversation_runs()?;
        let lookup = |id: &str| run_of(db, id);
        let cache = std::cell::RefCell::new(HashMap::new());
        let mut starts = vec![root.clone()];
        let first = threads::file(&starts, conversation.clone(), lookup, &top).into_iter().next().and_then(|r| r.members.into_iter().next());
        if let Some((t, _)) = first {
            if let Some(week) = threads::review_week(&t.root) {
                let same = |r: &Run| r.id != t.root.id && current.contains(r.employee_id.as_str()) && threads::review_week(r).as_ref() == Some(&week);
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

    /// The thread a run belongs to: the report or conversation you started (root) and every later turn, oldest first.
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
    /// "Send & done" meant you did not want the answer back, unless the answer raises something.
    pub(super) fn surface_thread(&self, reply: &Run) -> Result<()> {
        let Some(root_id) = reply.reply_to() else { return Ok(()) };
        let flagged = !reply.needs_you.is_empty() || !reply.denials.is_empty() || matches!(reply.state, RunState::Failed | RunState::Lost);
        if !flagged {
            return Ok(());
        }
        let root = {
            let db = self.db();
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
    /// employee's open conversations), or moves them all back. Archiving leaves alone any other conversation
    /// you are in in the row (`threads::talking`), cards and all, and any other thread in it that broke or
    /// still asks you something. `run_id` may be any run in the row. Returns the ids of the runs it moved, for Undo.
    pub fn set_archived(&self, run_id: &str, archived: bool) -> Result<Vec<String>> {
        Ok(self.archive_row(run_id, archived, false)?.into_iter().map(|r| r.id).collect())
    }

    /// Moves exactly these runs back out of the Archive: Done's Undo, so what you archived before stays there. News that
    /// archiving counted as seen is unseen again.
    pub fn unarchive(&self, run_ids: &[String]) -> Result<()> {
        let changed = {
            let db = self.db();
            db.tx(|| {
                let mut changed = vec![];
                for id in run_ids {
                    let Some(mut r) = db.run(id)?.map(|r| r.run) else { continue };
                    if r.archived_at.is_none() {
                        continue;
                    }
                    let turns = Self::thread_in(&db, id)?.replies;
                    changed.extend(file_news(&db, &mut r, &turns, false, now())?);
                    r.archived_at = None;
                    db.save_run(&r)?;
                    changed.push(r);
                }
                Ok(changed)
            })?
        };
        for r in &changed {
            self.sink().run_updated(r);
        }
        Ok(())
    }

    /// `set_archived`, but when `spare_own` (a tap on a card) and `run_id`'s thread is a conversation you are in
    /// (`yours`), it is left going and only the rest of its row is archived: what a typed reply leaves alone, a tap
    /// there does too. A tap answers only its card, so a thread that still asks you something else stays too, news you
    /// have not seen included. Unseen news never holds up Done: archiving a thread counts as seeing it.
    /// Returns the runs it moved.
    pub(super) fn archive_row(&self, run_id: &str, archived: bool, spare_own: bool) -> Result<Vec<Run>> {
        let (changed, turns) = {
            let db = self.db();
            let t = Self::thread_in(&db, run_id)?;
            if !t.root.state.is_finished() {
                return Err(AppError::conflict("it is still working, so it cannot be archived yet"));
            }
            let own = archived && !(spare_own && yours(&t));
            // Done ends only this conversation: another one you are in goes on by itself, so neither it
            // nor a card in it is Done's to settle. `t` holds every turn, answers that came back into it too.
            let row: Vec<Thread> = Self::row_of(&db, &t.root)?.into_iter().filter(|m| !archived || (m.root.id != t.root.id && !threads::talking(m))).collect();
            if archived && ((own && threads::open_card(&t).is_some()) || threads::card_open(&row)) {
                return Err(AppError::conflict("Answer the card first, or dismiss it."));
            }
            // A tap answers only its card, so its thread stays while it asks you something else.
            let own = own && !(spare_own && threads::asks_you(&t));
            // Nor does the rest of the row go while it needs you: a thread that asks you something, or broke, unless it
            // is another of the same routine's failures in a row, which you dealt with as one.
            let routine = |m: &Thread| matches!((&m.root.kind, &t.root.kind), (RunKind::Scheduled { schedule_id: a, .. }, RunKind::Scheduled { schedule_id: b, .. }) if a == b);
            let needs_you = |m: &Thread| threads::asks_you(m) || (threads::broken(m) && !routine(m));
            let at = now();
            let (mut changed, mut turns) = (vec![], vec![]);
            for m in row.into_iter().filter(|m| !archived || !needs_you(m)).chain(own.then(|| t.clone())) {
                // A run still going joins the row when it finishes, and shows only if it needs you.
                let mut root = m.root;
                if !root.state.is_finished() || root.archived_at.is_some() == archived {
                    continue;
                }
                turns.extend(file_news(&db, &mut root, &m.replies, archived, at)?);
                root.archived_at = archived.then_some(at);
                db.save_run(&root)?;
                changed.push(root);
            }
            (changed, turns)
        };
        for r in changed.iter().chain(&turns) {
            self.sink().run_updated(r);
        }
        Ok(changed)
    }

    /// Your reply in a report's thread. It goes into the thread's run while that works, or else starts a
    /// run that resumes the thread's session, so the employee answers with everything it saw. The reply
    /// joins the thread and the report stays in the Inbox, unless `archive`: "send, and bring the answer
    /// back only if it needs me", which waits until no card in the thread is open.
    /// A reply you type answers no card: each stays open until you answer it on the card. Answers you tapped that
    /// were held back for them go with it, ahead of what you typed.
    pub fn reply_to_run(&self, run_id: &str, text: &str, archive: bool, attachments: Vec<NewAttachment>) -> Result<Run> {
        let held: Vec<String> = {
            let t = Self::thread_in(&self.db(), run_id)?;
            if archive && threads::open_card(&t).is_some() {
                return Err(AppError::conflict("Answer the card first, or dismiss it."));
            }
            std::iter::once(&t.root).chain(&t.replies).flat_map(held_said).collect()
        };
        let said = held.into_iter().chain(Some(text.trim().to_string()).filter(|t| !t.is_empty())).collect::<Vec<_>>().join("\n\n");
        let reply = self.reply_with(run_id, &said, archive, vec![], attachments, ReplyBy::Ceo)?;
        let sent = {
            let db = self.db();
            db.tx(|| send_held(&db, run_id))?
        };
        for r in &sent {
            self.sink().run_updated(r);
        }
        Ok(reply)
    }

    /// Tries again a finished run that failed, was lost or was stopped. A hand-off (a colleague's ask, delegated work,
    /// or an answer brought back) runs again as it was, in its session if that started, so its report still goes to
    /// whoever handed it over: an answer OmegaStar already queued again by itself (`queued_again`) goes now instead.
    /// Anything else carries on in its thread, told how the last attempt stopped. Either way it leaves the Inbox, unless
    /// you are in it or it still asks you something, and the retry brings it back only if it needs you. Returns the new run.
    pub fn retry_run(&self, run_id: &str) -> Result<Run> {
        let (run, session, archive, queued) = {
            let db = self.db();
            let rec = db.run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?;
            if !matches!(rec.run.state, RunState::Failed | RunState::Lost | RunState::Cancelled) {
                return Err(AppError::conflict("Only a run that did not finish can be retried."));
            }
            let queued = queued_again(&db, &rec.run)?;
            let t = Self::thread_in(&db, run_id)?;
            // In a conversation, the answer queued again is the later turn.
            if queued.is_none() && t.replies.last().unwrap_or(&t.root).id != run_id {
                return Err(AppError::conflict("It already carried on in a later turn."));
            }
            db.employee(&rec.run.employee_id)?.ok_or_else(|| AppError::not_found("that employee no longer works here"))?;
            if let Some(q) = &queued {
                db.save_run(q)?;
            }
            let session = db.session_started(&rec.session)?.then_some(rec.session);
            (rec.run, session, !yours(&t) && !threads::asks_you(&t), queued)
        };
        let again = match queued {
            Some(q) => {
                self.sink().run_updated(&q);
                self.pump();
                q
            }
            None if !matches!(run.kind, RunKind::Asked { .. } | RunKind::Delegated { .. } | RunKind::Answered { .. }) => {
                let stopped = match run.error.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
                    Some(e) => format!("Your last attempt stopped with an error: {}", claude::clip(&claude::one_line(e), 500)),
                    None => "Your last attempt was stopped before it finished.".to_string(),
                };
                let text = format!("{stopped}\n\nCarry on from where you left off and finish.");
                return self.reply_with(run_id, &text, archive, vec![], vec![], ReplyBy::App);
            }
            None => {
                let mut again = Run::new(&run.employee_id, run.kind.clone(), run.prompt.clone(), now());
                again.omega_wrote = run.omega_wrote;
                let session = session.unwrap_or_else(|| again.id.clone());
                self.enqueue(again, session)?
            }
        };
        // The new run reports on its own, so the one it replaces leaves the Inbox: a turn of a conversation stays in it.
        if archive && run.reply_to().is_none() {
            let filed = {
                let db = self.db();
                match db.run(run_id)?.map(|r| r.run).filter(|r| r.archived_at.is_none()) {
                    Some(mut r) => {
                        r.archived_at = Some(now());
                        db.save_run(&r)?;
                        Some(r)
                    }
                    None => None,
                }
            };
            if let Some(r) = filed {
                self.sink().run_updated(&r);
            }
        }
        Ok(again)
    }

    /// The text of the file card `index` of a run shows, as it is now, so you read what would go out.
    pub fn card_file(&self, run_id: &str, index: usize) -> Result<String> {
        let run = self.db().run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
        let path = run.cards.get(index).and_then(|c| c.file.clone()).ok_or_else(|| AppError::invalid("That card shows no file."))?;
        // Still the file the card was given, not a link put in its place since.
        if fs::canonicalize(&path).ok().as_deref() != Some(Path::new(&path)) {
            return Err(AppError::not_found(format!("Nothing is at {path} any more.")));
        }
        if fs::metadata(&path)?.len() > cards::FILE_BYTES {
            return Err(AppError::invalid(format!("It is now over {} KB, too long to show here. Open it instead.", cards::FILE_BYTES / 1024)));
        }
        fs::read_to_string(&path).map_err(|_| AppError::invalid("It is no longer plain text. Open it instead."))
    }

    /// Your answer on card `index` of a run. A run still asking something else holds it back; once its last card is
    /// answered it hears every answer at once, in one fixed message through `reply_with`, and its row is archived.
    /// Everyone waiting on the same thing (a chore, or a card with the same key) hears the same answer where it fits
    /// their card (a choice only where they offer it, `cards::same_reply`), and those still working hear it straight
    /// away. Returns the reply runs it started.
    /// Later is not a no: the card comes back by itself on the day you picked (`bring_back_cards`), and Later on one
    /// that came back already parks it. A parked card still takes any answer. Going ahead with a proposal carries it
    /// out first (`carry_out`). An answer in your own words goes the same way as a tap. A question about the card
    /// answers nothing: it goes now (`ask_about_card`).
    pub fn answer_card(&self, run_id: &str, index: usize, reply: CardReply) -> Result<Vec<Run>> {
        if let CardReply::Ask { text } = &reply {
            return Ok(vec![self.ask_about_card(run_id, index, text)?]);
        }
        let at = now();
        let (card, ceo, targets) = {
            let db = self.db();
            let run = db.run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
            let card = run.cards.get(index).cloned().ok_or_else(|| AppError::invalid("That run has no such card."))?;
            if !run.state.is_finished() {
                return Err(AppError::conflict("It is still working, so answer the card when it finishes."));
            }
            let ceo = ceo_of(&db)?;
            // Tried first, so an answer that does not fit changes nothing.
            cards::answer(&mut card.clone(), &reply, &ceo, at)?;
            if card.proposal.is_some() && matches!(reply, CardReply::Choice { .. }) {
                self.carry_out(&db, &run, &card)?;
            }
            let mut targets = vec![run];
            if let Some(topic) = card.topic() {
                targets.extend(db.open_topic_runs(&topic)?.into_iter().filter(|r| r.id != run_id));
            }
            let said = db.tx(|| answer_cards(&db, &mut targets, Some(index), &card, &reply, &ceo, at))?;
            (card, ceo, targets.into_iter().zip(said).collect::<Vec<_>>())
        };
        let mut replies = self.send_card_answers(&targets);
        if let Some(topic) = card.topic() {
            // Runs still working on the same thing hear it now, in the turn they are in, where it fits their card. Told
            // and marked under one lock, so a run ending meanwhile either keeps the answer or is found by the read below.
            for a in lock(&self.0.active).values_mut() {
                let Some(i) = a.cards.iter().position(|c| c.waiting() && c.topic().as_deref() == Some(topic.as_str())) else { continue };
                let mut c = a.cards[i].clone();
                let Some(fits) = cards::same_reply(&card.ask, &reply, &c.ask) else { continue };
                let Some(answer) = cards::answer(&mut c, &fits, &ceo, at).ok().and(c.answer.clone()) else { continue };
                let told = answer.text.is_empty() || a.input.as_ref().is_some_and(|input| input.send(claude::user_line(&card_reply_text(&c, &answer))).is_ok());
                if told {
                    // Answered, so it no longer needs you for it: the run ends unflagged unless something else does.
                    a.needs_you.retain(|m| *m != c.message);
                    a.cards[i] = c;
                }
            }
            // A run that finished while the answers above went out was in neither pass. One the first pass answered
            // may still wait (Later puts a card off), so it is left out.
            let late = {
                let db = self.db();
                db.open_topic_runs(&topic).and_then(|mut late| {
                    late.retain(|r| !targets.iter().any(|(t, _)| t.id == r.id));
                    let said = db.tx(|| answer_cards(&db, &mut late, None, &card, &reply, &ceo, at))?;
                    Ok(late.into_iter().zip(said).collect::<Vec<_>>())
                })
            };
            match late {
                Ok(late) => replies.extend(self.send_card_answers(&late)),
                Err(e) => log!("answering the runs that finished waiting on {topic} failed: {e}"),
            }
        }
        Ok(replies)
    }

    /// Opens again, with no model run, every card you said Later to whose day has come: it waits on you as before, its
    /// message back in what needs you and its row back in the Inbox (or its conversation open again), and
    /// `Card::times_back` counts it. Your answer then reaches the employee as any answer does. Returns how many came back.
    pub fn bring_back_cards(&self, at: Ts) -> Result<usize> {
        let (back, changed) = {
            let db = self.db();
            db.tx(|| {
                let (mut back, mut changed) = (0, vec![]);
                for mut r in db.cards_due_back(at)? {
                    let mut messages = vec![];
                    for c in r.cards.iter_mut().filter(|c| c.back_at.is_some_and(|t| t <= at)) {
                        (c.answer, c.back_at) = (None, None);
                        c.times_back += 1;
                        messages.push(c.message.clone());
                    }
                    back += messages.len();
                    for m in messages {
                        if !r.needs_you.contains(&m) {
                            r.needs_you.push(m);
                        }
                    }
                    match r.reply_to().map(String::from) {
                        Some(id) => {
                            if let Some(mut root) = db.run(&id)?.map(|x| x.run).filter(|x| x.archived_at.is_some()) {
                                root.archived_at = None;
                                db.save_run(&root)?;
                                changed.push(root);
                            }
                        }
                        None => r.archived_at = None,
                    }
                    db.save_run(&r)?;
                    changed.push(r);
                }
                Ok((back, changed))
            })?
        };
        for r in &changed {
            self.sink().run_updated(r);
        }
        Ok(back)
    }

    /// The runs holding a card you parked, newest first: the parked cards in your Later list.
    pub fn parked_runs(&self) -> Result<Vec<Run>> {
        self.db().parked_runs()
    }

    /// Your question about card `index` of a run, sent now as your reply in its thread. The card stays open, and the
    /// answers its run holds back stay held for it. The question leaves the thread as it was (`Run::about_card`), so a
    /// later tap still files it. Returns the reply. The Inbox reads your words back out of its prompt to show them
    /// under the card (`askedText` in `src/lib/threads.ts`), so the line before them changes there too.
    fn ask_about_card(&self, run_id: &str, index: usize, text: &str) -> Result<Run> {
        let text = text.trim();
        if text.is_empty() {
            return Err(AppError::invalid("Type your question first."));
        }
        let card = self.run(run_id)?.cards.get(index).cloned().ok_or_else(|| AppError::invalid("That run has no such card."))?;
        if !card.waiting() {
            return Err(AppError::conflict("That card was already answered."));
        }
        let said = format!("About your card \"{}\", which stays open until I answer it:\n\n{text}", claude::clip(&card.message, 300));
        self.reply_with(run_id, &said, false, vec![], vec![], ReplyBy::Question)
    }

    /// Sends each run what it no longer holds back (`said`: its answers, each under its own card, from `take_held`)
    /// and archives its row (but not a conversation you are in, nor what still needs you), carrying on past a run that
    /// fails. A run still asking something else waits for its last card. A run back in the queue after the usage limit
    /// only keeps its answered card: its retry carries on anyway. Returns the reply runs it started.
    pub(super) fn send_card_answers(&self, targets: &[(Run, Option<String>)]) -> Vec<Run> {
        let mut replies = vec![];
        for (t, said) in targets {
            self.sink().run_updated(t);
            if t.state == RunState::Queued || t.has_open_card() {
                continue;
            }
            if let Some(said) = said {
                match self.reply_with(&t.id, said, false, vec![], vec![], ReplyBy::Card) {
                    Ok(r) => replies.push(r),
                    Err(e) => {
                        log!("sending your answers on the cards of run {} failed: {e}", t.id);
                        continue;
                    }
                }
            }
            // A tap in a conversation you are in answers the card, and the conversation goes on.
            match self.archive_row(&t.id, true, true) {
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
        let (original, employee, ceo, stays) = {
            let db = self.db();
            let original = db.run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
            let t = Self::thread_in(&db, run_id)?;
            let stays = yours(&t) || std::iter::once(&t.root).chain(&t.replies).any(|r| r.has_open_card() || r.has_news());
            if original.denials.is_empty() {
                return Err(AppError::conflict("nothing was blocked in that run"));
            }
            if !original.state.is_finished() {
                return Err(AppError::conflict("that run is still working"));
            }
            let employee = db.employee(&original.employee_id)?.ok_or_else(|| AppError::not_found("that employee no longer works here"))?;
            let ceo = ceo_of(&db)?;
            (original, employee, ceo, stays)
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
        // Decided, so it leaves the Inbox: the retry comes back only if it needs you again. A conversation you are
        // in goes on instead, and a card you have not answered or news you have not seen keeps the thread where you
        // will see it.
        self.reply_with(run_id, &text, !stays, rules, vec![], ReplyBy::App)
    }

    /// A thread's newest turn as stored, and whether the next turn resumes its session: yes while that turn is
    /// still to come (it has the session, even before it starts) or once its session started. No when it never
    /// started (it failed first), so there is nothing to resume and the next turn starts a fresh one.
    pub(super) fn next_turn(db: &Store, t: &Thread) -> Result<(RunRecord, bool)> {
        let latest = t.replies.last().unwrap_or(&t.root);
        let rec = db.run(&latest.id)?.ok_or_else(|| AppError::not_found("run not found"))?;
        let resumes = !rec.run.state.is_finished() || db.session_started(&rec.session)?;
        Ok((rec, resumes))
    }

    /// `run_id` may be any run in the thread. The reply answers the thread's root and resumes the
    /// thread's newest session (the root's, or a fresh one an earlier reply had to start). A plain reply
    /// goes into that session's run while it works; one that carries files or rules, or that OmegaStar
    /// wrote (`by`), is a run of its own.
    fn reply_with(&self, run_id: &str, text: &str, archive: bool, allow_once: Vec<String>, attachments: Vec<NewAttachment>, by: ReplyBy) -> Result<Run> {
        let text = text.trim();
        if text.is_empty() && attachments.is_empty() {
            return Err(AppError::invalid("reply is empty"));
        }
        let files = Self::decode_attachments(&attachments)?;
        let own_words = matches!(by, ReplyBy::Ceo | ReplyBy::Question);
        let plain = files.is_empty() && allow_once.is_empty() && own_words;
        let (root, mut reply, session, turns) = {
            let db = self.db();
            let t = Self::thread_in(&db, run_id)?;
            let mut root = t.root.clone();
            let e = db.employee(&root.employee_id)?.ok_or_else(|| AppError::not_found("that employee no longer works here"))?;
            let latest = t.replies.last().cloned().unwrap_or_else(|| root.clone());
            // Sending and archiving counts as seeing the thread's news.
            let at = now();
            let turns = if archive { file_news(&db, &mut root, &t.replies, true, at)? } else { vec![] };
            if plain && latest.state == RunState::Running && self.steer(&latest.id, text) {
                root.replied_at = Some(at);
                root.archived_at = archive.then_some(at);
                // A report you wrote into as it worked waits for you like a reply, unless you sent it away.
                root.steered |= !archive && latest.id == root.id;
                db.save_run(&root)?;
                drop(db);
                for r in std::iter::once(&root).chain(&turns) {
                    self.sink().run_updated(r);
                }
                return Ok(latest);
            }
            let (rec, resumes) = Self::next_turn(&db, &t)?;
            let (label, task) = threads::reply_label(&root);
            let (prompt, session) = if resumes {
                (text.to_string(), rec.session)
            } else {
                let report = rec.run.summary.clone().or_else(|| rec.run.error.clone()).unwrap_or_else(|| "(no report)".into());
                let ceo = db.company()?.map(|c| c.ceo_name).unwrap_or_else(|| "Your manager".into());
                let before = if task {
                    format!("{ceo} started this conversation ({label}):\n\n{}\n\nWhat came of it:\n\n{report}", root.prompt)
                } else {
                    format!("Your report ({label}):\n\n{report}")
                };
                (format!("{before}\n\n{ceo}'s reply:\n\n{text}"), String::new())
            };
            let card = by == ReplyBy::Card;
            let mut reply = Run::new(&e.id, RunKind::Reply { to_run_id: root.id.clone(), label, task, card }, prompt, now());
            reply.allow_once = allow_once;
            // The never-started wrapper quotes your reply in full, so it still counts as your words.
            reply.omega_wrote = !own_words;
            reply.about_card = by == ReplyBy::Question;
            let session = if session.is_empty() { reply.id.clone() } else { session };
            root.replied_at = Some(at);
            root.archived_at = archive.then_some(at);
            db.save_run(&root)?;
            (root, reply, session, turns)
        };
        self.attach(&mut reply, &files)?;
        for r in std::iter::once(&root).chain(&turns) {
            self.sink().run_updated(r);
        }
        self.enqueue(reply, session)
    }
}

/// Archiving a thread (`archived`, at `at`) counts as seeing its news, and moving it back out of the Archive unsees
/// what archiving it marked (at the root's `archived_at`). Marks `root`, which the caller saves, and the finished
/// `turns`, saved here. Call before changing the root's `archived_at`. Returns the turns it changed.
fn file_news(db: &Store, root: &mut Run, turns: &[Run], archived: bool, at: Ts) -> Result<Vec<Run>> {
    let Some(stamp) = (if archived { Some(at) } else { root.archived_at }) else { return Ok(vec![]) };
    see_news(root, archived, stamp);
    let mut changed = vec![];
    for t in turns.iter().filter(|t| t.state.is_finished()) {
        let mut t = t.clone();
        if see_news(&mut t, archived, stamp) {
            db.save_run(&t)?;
            changed.push(t);
        }
    }
    Ok(changed)
}

/// Marks `r`'s unseen news seen at `at`, or with `seen` false unsees the news marked seen at exactly `at` by archiving.
/// Returns whether anything changed.
fn see_news(r: &mut Run, seen: bool, at: Ts) -> bool {
    let mut changed = false;
    for c in r.cards.iter_mut().filter(|c| c.ask == CardAsk::News) {
        let by_archiving = c.answer.as_ref().is_some_and(|a| a.at == at && a.how == AnswerHow::Tapped && a.text.is_empty());
        if seen && c.answer.is_none() {
            c.answer = Some(CardAnswer { at, how: AnswerHow::Tapped, text: String::new(), held: false });
            changed = true;
        } else if !seen && by_archiving {
            c.answer = None;
            changed = true;
        }
    }
    changed
}

/// A conversation you are in: one you started (an older build's chat message an answer came back into too), a
/// report you typed into (`threads::talking`), or a run you wrote into as it worked (`Run::steered`).
pub(super) fn yours(t: &Thread) -> bool {
    matches!(t.root.kind, RunKind::Task { .. } | RunKind::Chat) || threads::talking(t) || t.root.steered
}

/// For an answer brought back (`RunKind::Answered`) that did not finish: the copy OmegaStar queued again by itself
/// (`handoffs::redeliver`), made due now, so a retry sends that rather than a second copy. Err once a copy is running or
/// brought it, as a retry would bring the same answer twice. None for anything else. Save the run it returns.
fn queued_again(db: &Store, failed: &Run) -> Result<Option<Run>> {
    let RunKind::Answered { asked_run_id, .. } = &failed.kind else { return Ok(None) };
    let copies: Vec<Run> =
        db.answers_to(asked_run_id)?.into_iter().filter(|r| r.id != failed.id && r.prompt == failed.prompt && r.created_at >= failed.created_at).collect();
    if copies.iter().any(|r| matches!(r.state, RunState::Running | RunState::Succeeded)) {
        return Err(AppError::conflict("This answer already went again, so there is nothing to retry."));
    }
    Ok(copies.into_iter().find(|r| r.state == RunState::Queued).map(|r| Run { not_before: None, ..r }))
}

/// Marks the answers the finished runs of `run_id`'s thread held back sent, since they went with your typed reply.
/// Their open cards stay open. Returns those runs, saved. Call inside a transaction.
fn send_held(db: &Store, run_id: &str) -> Result<Vec<Run>> {
    let t = Office::thread_in(db, run_id)?;
    let mut out = vec![];
    for mut r in std::iter::once(t.root).chain(t.replies) {
        if !r.state.is_finished() || held_said(&r).is_empty() {
            continue;
        }
        for a in r.cards.iter_mut().filter_map(|c| c.answer.as_mut()) {
            a.held = false;
        }
        db.save_run(&r)?;
        out.push(r);
    }
    Ok(out)
}

/// Puts your `reply` on `card`, card `index` of the first run, and on every other card about the same thing still waiting
/// on you in all of them, put off with Later included, where it fits (`cards::same_reply`), each worded for its own card
/// (`cards::answer`), and saves them. A finished run still asking something else holds it back. Returns, in the runs'
/// order, what each now sends (`take_held`). Call inside a transaction.
fn answer_cards(db: &Store, runs: &mut [Run], index: Option<usize>, card: &Card, reply: &CardReply, ceo: &str, at: Ts) -> Result<Vec<Option<String>>> {
    let topic = card.topic();
    let mut said = vec![];
    for (n, r) in runs.iter_mut().enumerate() {
        // A run back in the queue after the usage limit is never sent it: its retry carries on anyway.
        let finished = r.state.is_finished();
        for (i, c) in r.cards.iter_mut().enumerate() {
            if n == 0 && Some(i) == index {
                cards::answer(c, reply, ceo, at)?;
            } else if topic.is_some() && c.waiting() && c.topic() == topic {
                let Some(fits) = cards::same_reply(&card.ask, reply, &c.ask) else { continue };
                if cards::answer(c, &fits, ceo, at).is_err() {
                    continue;
                }
            } else {
                continue;
            }
            if let Some(a) = c.answer.as_mut() {
                a.held = finished && !a.text.is_empty();
            }
        }
        said.push(take_held(r));
        db.save_run(r)?;
    }
    Ok(said)
}

/// What a run's held-back answers say, each under the question it answers.
fn held_said(run: &Run) -> Vec<String> {
    run.cards.iter().filter_map(|c| c.answer.as_ref().filter(|a| a.held).map(|a| card_reply_text(c, a))).collect()
}

/// A run's held-back answers as one message, now marked sent, once it has no open card left. None while it still
/// asks something, or when it holds nothing. Save the run after.
pub(super) fn take_held(run: &mut Run) -> Option<String> {
    if run.has_open_card() {
        return None;
    }
    let said = held_said(run);
    for a in run.cards.iter_mut().filter_map(|c| c.answer.as_mut()) {
        a.held = false;
    }
    (!said.is_empty()).then(|| said.join("\n\n"))
}

/// What your answer on `card` says to the run that raised it: a chore's Done as it is, anything else under the
/// question it answers. Your own words are quoted, and since they close the card even when they decide nothing ("I'll
/// look later"), the employee hears that it has left your Inbox.
fn card_reply_text(card: &Card, answer: &CardAnswer) -> String {
    let typed = answer.how == AnswerHow::Typed;
    let said = if typed { format!("Answered: \"{}\"\n{TYPED_CLOSES}", answer.text) } else { answer.text.clone() };
    match card.ask {
        CardAsk::Chore { .. } if !typed => said,
        _ => format!("Your card: \"{}\"\n{said}", claude::clip(&card.message, 300)),
    }
}

/// What follows your own words on a card: they closed it, so a decision still needed is asked again.
const TYPED_CLOSES: &str = "This answer closed the card, so it is no longer in their Inbox. If you still need their decision, raise the card again.";

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
    use crate::fixtures::{employee_named, open_office, task, temp_dir, temp_path};

    /// A finished report of `e` asking a decision with a card.
    fn carded(e: &Employee) -> Run {
        let mut r = Run::new(&e.id, RunKind::Manual, "go".into(), now());
        r.state = RunState::Succeeded;
        r.ended_at = Some(now());
        r.needs_you = vec!["Boost the post?".into()];
        r.cards = vec![Card::new("Boost the post?", None, Some("ask:boost".into()), CardAsk::Choices { choices: vec!["Not now".into(), "Go".into()], recommended: 0 })];
        r
    }

    #[test]
    fn a_reply_you_type_sends_what_was_held_and_answers_no_card() {
        let dir = temp_dir("typed");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let e = employee_named("Cy");
        db.save_employee(&e).unwrap();
        let mut report = two_cards(&e);
        report.cards[1].answer = Some(CardAnswer { at: now(), how: AnswerHow::Tapped, text: "Chose: Post.".into(), held: true });
        assert!(db.insert_run(&report, &report.id).unwrap());
        let mut working = carded(&e);
        working.kind = RunKind::Reply { to_run_id: report.id.clone(), label: "Manual run".into(), task: false, card: false };
        working.state = RunState::Running;
        assert!(db.insert_run(&working, &report.id).unwrap());

        let sent = db.tx(|| send_held(&db, &working.id)).unwrap();
        assert_eq!(sent.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![report.id.as_str()]);
        let s = db.run(&report.id).unwrap().unwrap().run;
        assert!(s.cards[0].is_open(), "the card you did not answer stays open");
        assert!(s.cards[1].answer.as_ref().is_some_and(|a| !a.held), "the tap went with the reply");
        assert!(db.run(&working.id).unwrap().unwrap().run.has_open_card());
        assert!(db.tx(|| send_held(&db, &report.id)).unwrap().is_empty(), "nothing goes twice");
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_weeks_reviews_share_one_row() {
        let dir = temp_dir("team-week");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let hire = |slug: &str, manager: Option<&Employee>| {
            let e = Employee { manager_id: manager.map(|m| m.id.clone()), ..employee_named(slug) };
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
        d.cards = vec![Card::new("Open Chrome.", None, None, CardAsk::Chore { subject: "chrome".into(), url: None })];
        db.save_run(&d).unwrap();

        let ids = |t: &Thread| -> HashSet<String> { std::iter::once(t).chain(&t.related).map(|x| x.root.id.clone()).collect() };
        let week: HashSet<String> = [&a, &b, &c].iter().map(|r| r.id.clone()).collect();
        let rows = Office::rows(&db, View::Inbox).unwrap();
        assert_eq!(
            rows.iter().map(ids).collect::<Vec<_>>(),
            vec![HashSet::from([d.id.clone()])],
            "only the review waiting on a chore needs you: plain reviews are in the Archive, and one failure is not a streak"
        );
        assert!(!rows.iter().any(|t| ids(t).contains(&later.id)));
        let chored = Office::row_of(&db, &d).unwrap();
        assert_eq!(chored.iter().map(|t| t.root.id.clone()).collect::<Vec<_>>(), vec![d.id.clone()], "answering the chore leaves the week alone");

        let row = Office::row_of(&db, &b).unwrap();
        assert_eq!(row.iter().map(|t| t.root.id.clone()).collect::<HashSet<_>>(), week, "a second-level employee's review leads to the whole week");
        for t in row {
            let mut r = t.root;
            r.archived_at = Some(now());
            db.save_run(&r).unwrap();
        }
        let rows = Office::rows(&db, View::Inbox).unwrap();
        assert_eq!(rows.iter().map(ids).collect::<Vec<_>>(), vec![HashSet::from([d.id.clone()])]);
        let archived: Vec<HashSet<String>> = Office::rows(&db, View::Archive).unwrap().iter().map(ids).collect();
        assert_eq!(archived.len(), 2, "the week you archived, and next week's review, which needs nothing");
        assert!(archived.contains(&week) && archived.contains(&HashSet::from([later.id.clone()])));
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_week_that_needs_you_is_one_inbox_row_with_every_review() {
        let dir = temp_dir("week-needs");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let slot = "2026-10-07T12:00:00Z".parse::<Ts>().unwrap();
        let review = |slug: &str, flag: bool| {
            let e = employee_named(slug);
            db.save_employee(&e).unwrap();
            let kind = RunKind::Scheduled { schedule_id: format!("review-{slug}"), slot, label: "Weekly review".into(), quiet: false, model: None, effort: None, review: true };
            let mut r = Run::new(&e.id, kind, "Review your week.".into(), slot);
            r.state = RunState::Succeeded;
            r.ended_at = Some(slot);
            r.summary = Some("Signups 40, up 5.".into());
            if flag {
                r.needs_you = vec!["Approve the budget.".into()];
            }
            assert!(db.insert_run(&r, &r.id).unwrap());
            r.id
        };
        let week: HashSet<String> = [review("ada", false), review("bo", true), review("cy", false)].into();
        let ids = |t: &Thread| -> HashSet<String> { std::iter::once(t).chain(&t.related).map(|x| x.root.id.clone()).collect() };
        let inbox: Vec<HashSet<String>> = Office::rows(&db, View::Inbox).unwrap().iter().map(ids).collect();
        assert_eq!(inbox, vec![week], "the reviews that need nothing come along with the one that does");
        assert!(Office::rows(&db, View::Archive).unwrap().is_empty(), "so none of them is lost between the two");
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A week of plain reviews by `slugs`, reporting to you, at noon UTC on a Wednesday of one ISO week.
    fn plain_week(db: &Store, slugs: &[&str]) -> Vec<Run> {
        let slot = "2026-10-07T12:00:00Z".parse::<Ts>().unwrap();
        slugs
            .iter()
            .enumerate()
            .map(|(i, slug)| {
                let e = employee_named(slug);
                db.save_employee(&e).unwrap();
                let at = slot + chrono::Duration::hours(i as i64);
                let kind = RunKind::Scheduled { schedule_id: format!("review-{slug}"), slot: at, label: "Weekly review".into(), quiet: false, model: None, effort: None, review: true };
                let mut r = Run::new(&e.id, kind, "Review your week.".into(), at);
                r.state = RunState::Succeeded;
                r.ended_at = Some(at);
                r.summary = Some("Signups 40, up 5.".into());
                assert!(db.insert_run(&r, &r.id).unwrap());
                r
            })
            .collect()
    }

    /// Your reply on `report` and the employee's answer, a later turn of its thread. Returns the turn.
    fn talk_about(db: &Store, report: &Run) -> Run {
        let mut root = report.clone();
        let at = root.created_at + chrono::Duration::hours(4);
        root.replied_at = Some(at);
        db.save_run(&root).unwrap();
        let kind = RunKind::Reply { to_run_id: root.id.clone(), label: threads::report_label(&root.kind), task: false, card: false };
        let mut turn = Run::new(&root.employee_id, kind, "Why up?".into(), at);
        turn.state = RunState::Succeeded;
        turn.ended_at = Some(at);
        assert!(db.insert_run(&turn, &root.id).unwrap());
        turn
    }

    #[tokio::test]
    async fn done_on_one_review_conversation_leaves_another_in_the_same_week_going() {
        let dir = temp_dir("week-done");
        let week = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            let week = plain_week(&db, &["ada", "bo", "cy"]);
            talk_about(&db, &week[0]);
            talk_about(&db, &week[1]);
            week
        };
        let office = open_office(&dir).await;
        office.set_archived(&week[0].id, true).unwrap();
        let archived = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run.archived_at.is_some();
        assert!(archived(&week[0]), "Done ends the conversation you pressed it on");
        assert!(!archived(&week[1]), "another conversation on the week goes on");
        assert!(archived(&week[2]), "the rest of the week goes with it");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn done_leaves_what_still_needs_you_and_undo_brings_back_only_what_it_filed() {
        let dir = temp_dir("done-spares");
        let week = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            let mut week = plain_week(&db, &["ada", "bo", "cy", "dee", "eve"]);
            // Bo's review broke and Cy's flags something, so both still need you. Dee's you archived before.
            week[1].state = RunState::Failed;
            week[2].needs_you = vec!["Approve the budget.".into()];
            week[3].archived_at = Some(now() - chrono::Duration::hours(1));
            for r in &week[1..4] {
                db.save_run(r).unwrap();
            }
            week
        };
        let office = open_office(&dir).await;
        let archived = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run.archived_at.is_some();
        let filed = office.set_archived(&week[0].id, true).unwrap();
        assert_eq!(filed.iter().cloned().collect::<HashSet<_>>(), HashSet::from([week[0].id.clone(), week[4].id.clone()]), "yours, and what needs nothing");
        assert!(!archived(&week[1]) && !archived(&week[2]), "a failure and a flag stay in the Inbox");
        office.unarchive(&filed).unwrap();
        assert!(!archived(&week[0]) && !archived(&week[4]), "Undo brings back what Done filed");
        assert!(archived(&week[3]), "and leaves what you archived before");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn done_on_a_routine_that_keeps_failing_files_its_failures_together() {
        let dir = temp_dir("done-streak");
        let e = employee_named("Cy");
        let failed = |mins: i64| {
            let at = now() - chrono::Duration::minutes(mins);
            let kind = RunKind::Scheduled { schedule_id: "sweep".into(), slot: at, label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
            let mut r = Run::new(&e.id, kind, "go".into(), at);
            r.state = RunState::Failed;
            r.ended_at = Some(at);
            r
        };
        let runs = [failed(30), failed(20), failed(10)];
        let office = held_office(&dir, &e, &runs.iter().collect::<Vec<_>>()).await;
        assert_eq!(office.set_archived(&runs[2].id, true).unwrap().len(), 3, "one routine's failures in a row are one thing you dealt with");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_tap_files_only_what_needs_nothing_more() {
        let dir = temp_dir("tap-spares");
        let week = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            let mut week = plain_week(&db, &["ada", "bo", "cy"]);
            week[0] = with_card(week[0].clone(), "Boost the post?");
            week[1].state = RunState::Failed;
            // Cy's review asks something else besides its card.
            week[2] = with_card(week[2].clone(), "Order the stock?");
            week[2].needs_you.push("A rule you taught was dropped.".into());
            for r in &week {
                db.save_run(r).unwrap();
            }
            week
        };
        let office = open_office(&dir).await;
        let archived = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run.archived_at.is_some();
        office.answer_card(&week[0].id, 0, CardReply::Choice { index: 1 }).unwrap();
        assert!(!archived(&week[0]), "Cy's card is still open in the row");
        office.answer_card(&week[2].id, 0, CardReply::Choice { index: 1 }).unwrap();
        assert!(archived(&week[0]), "it needs nothing more");
        assert!(!archived(&week[1]), "a failure stays");
        assert!(!archived(&week[2]), "the tap answered only the card, and the flag still needs you");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn retry_carries_a_failed_report_on_in_its_thread() {
        let dir = temp_dir("retry");
        let e = employee_named("Cy");
        let report = |state: RunState| {
            let mut r = Run::new(&e.id, RunKind::Manual, "Count the stock.".into(), now());
            r.state = state;
            r.ended_at = Some(now());
            r
        };
        let mut broke = report(RunState::Failed);
        broke.error = Some("API Error: 500\nInternal server error".into());
        let fine = report(RunState::Succeeded);
        let stopped = report(RunState::Cancelled);
        let office = held_office(&dir, &e, &[&broke, &fine, &stopped]).await;
        assert_eq!(office.retry_run(&fine.id).unwrap_err().message, "Only a run that did not finish can be retried.");
        let again = office.retry_run(&broke.id).unwrap();
        assert_eq!(again.kind, RunKind::Reply { to_run_id: broke.id.clone(), label: "Manual run".into(), task: false, card: false });
        assert!(again.omega_wrote);
        let told = "Your last attempt stopped with an error: API Error: 500 Internal server error\n\nCarry on from where you left off and finish.";
        assert!(again.prompt.ends_with(told), "{}", again.prompt);
        assert!(office.db().run(&broke.id).unwrap().unwrap().run.archived_at.is_some(), "it leaves the Inbox, and the retry brings it back if it needs you");
        assert_eq!(office.retry_run(&broke.id).unwrap_err().message, "It already carried on in a later turn.");
        let again = office.retry_run(&stopped.id).unwrap();
        assert!(again.prompt.ends_with("Your last attempt was stopped before it finished.\n\nCarry on from where you left off and finish."), "{}", again.prompt);
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn retry_runs_a_lost_hand_off_again_so_its_report_still_goes_back() {
        let dir = temp_dir("retry-handoff");
        let ada = employee_named("Cy");
        let bo = employee_named("Bo");
        let mut plan = Run::new(&ada.id, RunKind::Manual, "Plan the week.".into(), now());
        plan.state = RunState::Succeeded;
        plan.ended_at = Some(now());
        let mut asked = Run::new(&bo.id, RunKind::Asked { parent_run_id: plan.id.clone(), asked_by: ada.id.clone(), task: true }, "Book the venue.".into(), now());
        asked.state = RunState::Lost;
        asked.ended_at = Some(now());
        asked.delivered_at = Some(now());
        let office = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&bo).unwrap();
            drop(db);
            held_office(&dir, &ada, &[&plan, &asked]).await
        };
        office.db().mark_inited(&asked.id).unwrap();
        let again = office.retry_run(&asked.id).unwrap();
        assert_ne!(again.id, asked.id);
        assert_eq!((&again.kind, again.prompt.as_str(), again.state), (&asked.kind, "Book the venue.", RunState::Queued), "the same hand-off, from the same run");
        assert_eq!(office.db().run(&again.id).unwrap().unwrap().session, asked.id, "it picks up the session it started");
        assert!(office.db().run(&asked.id).unwrap().unwrap().run.archived_at.is_some());
        // Once it finishes, its report goes to whoever handed the work over.
        let mut done = office.db().run(&again.id).unwrap().unwrap().run;
        done.state = RunState::Succeeded;
        done.ended_at = Some(now());
        done.summary = Some("Booked for Friday.".into());
        office.db().save_run(&done).unwrap();
        let due = super::super::handoffs::due_answers(&office.db(), &done).unwrap();
        assert!(matches!(&due[..], [(r, _)] if r.employee_id == ada.id && matches!(&r.kind, RunKind::Answered { asked_run_id, .. } if *asked_run_id == again.id)), "{due:?}");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_check_that_worked_for_long_reports_to_the_archive() {
        let dir = temp_dir("long-check");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let e = employee_named("Cy");
        db.save_employee(&e).unwrap();
        let check = |id: &str, mins: u64| {
            let kind = RunKind::Scheduled { schedule_id: format!("{CHECK_PREFIX}{id}"), slot: now(), label: "Check: Did it ship".into(), quiet: true, model: None, effort: None, review: false };
            let mut r = Run::new(&e.id, kind, "go".into(), now());
            r.state = RunState::Succeeded;
            r.ended_at = Some(now());
            r.duration_ms = Some(mins * 60 * 1000);
            r.summary = Some("Shipped.".into());
            assert!(db.insert_run(&r, &r.id).unwrap());
            r
        };
        let (_look, work) = (check("c1", 5), check("c2", 45));
        let archive: Vec<String> = Office::rows(&db, View::Archive).unwrap().into_iter().map(|t| t.root.id).collect();
        assert_eq!(archive, vec![work.id.clone()], "a quick look stays quiet");
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A card on `run` with no answer yet.
    fn with_card(mut run: Run, message: &str) -> Run {
        run.needs_you = vec![message.into()];
        run.cards = vec![Card::new(message, None, None, CardAsk::Choices { choices: vec!["Not now".into(), "Go".into()], recommended: 0 })];
        run
    }

    #[tokio::test]
    async fn answering_the_last_card_of_a_week_archives_a_review_whose_card_was_answered_first() {
        let dir = temp_dir("week-cards");
        let week = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            let week = plain_week(&db, &["ada", "bo"]);
            for r in &week {
                db.save_run(&with_card(r.clone(), "Boost the post?")).unwrap();
            }
            week
        };
        let office = open_office(&dir).await;
        let archived = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run.archived_at.is_some();
        assert_eq!(office.answer_card(&week[0].id, 0, CardReply::Choice { index: 1 }).unwrap().len(), 1);
        assert!(!archived(&week[0]), "the week still waits on the other card");
        office.answer_card(&week[1].id, 0, CardReply::Choice { index: 1 }).unwrap();
        assert!(archived(&week[0]) && archived(&week[1]), "the answer to a tap is no conversation of yours, so the whole week goes");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_card_in_another_conversation_of_the_row_does_not_hold_up_done() {
        let dir = temp_dir("week-other-card");
        let week = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            let week = plain_week(&db, &["ada", "bo"]);
            talk_about(&db, &week[0]);
            let turn = talk_about(&db, &week[1]);
            db.save_run(&with_card(turn, "Boost the post?")).unwrap();
            week
        };
        let office = open_office(&dir).await;
        office.set_archived(&week[0].id, true).unwrap();
        let archived = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run.archived_at.is_some();
        assert!(archived(&week[0]) && !archived(&week[1]), "the card waits in its own conversation");
        assert_eq!(office.set_archived(&week[1].id, true).unwrap_err().message, "Answer the card first, or dismiss it.");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_card_in_an_answer_that_came_back_into_a_conversation_holds_up_done() {
        let dir = temp_dir("answer-card");
        let e = employee_named("Cy");
        let mut task = Run::new(&e.id, task("Draft it"), "Draft it.".into(), now());
        task.state = RunState::Succeeded;
        task.ended_at = Some(now());
        let kind = RunKind::Answered { asked_run_id: "asked-1".into(), from: e.id.clone(), task: false, chat: false, thread: Some(task.id.clone()) };
        let mut answer = with_card(Run::new(&e.id, kind, "Bo answered your question.".into(), now()), "Send it to the printer?");
        answer.state = RunState::Succeeded;
        answer.ended_at = Some(now());
        answer.omega_wrote = true;
        let office = held_office(&dir, &e, &[&task, &answer]).await;
        assert_eq!(office.set_archived(&task.id, true).unwrap_err().message, "Answer the card first, or dismiss it.");
        assert!(office.db().run(&task.id).unwrap().unwrap().run.archived_at.is_none());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_tap_or_an_approval_in_a_conversation_you_started_keeps_it_going() {
        let dir = temp_dir("task-tap");
        let e = employee_named("Cy");
        let start = |label: &str| {
            let mut r = Run::new(&e.id, task(label), format!("{label}."), now());
            r.state = RunState::Succeeded;
            r.ended_at = Some(now());
            r
        };
        let carded = with_card(start("Check the supplier site"), "Order the spare parts?");
        let mut blocked = start("Publish the menu");
        blocked.denials = vec![Denial { tool: "Bash".into(), brief: "npm publish".into(), suggested_rule: "Bash(npm publish)".into() }];
        let office = held_office(&dir, &e, &[&carded, &blocked]).await;
        let archived = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run.archived_at.is_some();
        assert_eq!(office.answer_card(&carded.id, 0, CardReply::Choice { index: 1 }).unwrap().len(), 1);
        assert!(!archived(&carded), "the answer goes on in the conversation, as a typed one does");
        office.approve_blocked(&blocked.id, vec!["Bash(npm publish)".into()], false).unwrap();
        assert!(!archived(&blocked), "the retry goes on in the conversation");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_reply_in_an_older_chat_message_is_a_turn_of_your_conversation() {
        let dir = temp_dir("old-chat-reply");
        let e = employee_named("Cy");
        let mut message = Run::new(&e.id, RunKind::Chat, "Can you check the rota with Bo?".into(), now());
        message.state = RunState::Succeeded;
        message.ended_at = Some(now());
        let came = RunKind::Answered { asked_run_id: "asked-1".into(), from: e.id.clone(), task: false, chat: false, thread: Some(message.id.clone()) };
        let mut answer = Run::new(&e.id, came, "Bo answered your question.".into(), now());
        answer.state = RunState::Succeeded;
        answer.ended_at = Some(now());
        answer.omega_wrote = true;
        let office = held_office(&dir, &e, &[&message, &answer]).await;
        let reply = office.reply_to_run(&message.id, "Thanks, book it.", false, vec![]).unwrap();
        assert_eq!(reply.kind, RunKind::Reply { to_run_id: message.id.clone(), label: "Check the rota with Bo".into(), task: true, card: false });
        assert!(reply.prompt.starts_with("Your manager started this conversation (Check the rota with Bo)"), "{}", reply.prompt);
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_tap_on_an_answer_in_an_older_chat_message_keeps_its_conversation_going() {
        let dir = temp_dir("old-chat-tap");
        let e = employee_named("Cy");
        let mut message = Run::new(&e.id, RunKind::Chat, "Can you check the rota with Bo?".into(), now());
        message.state = RunState::Succeeded;
        message.ended_at = Some(now());
        let came = RunKind::Answered { asked_run_id: "asked-1".into(), from: e.id.clone(), task: false, chat: false, thread: Some(message.id.clone()) };
        let mut answer = with_card(Run::new(&e.id, came, "Bo answered your question.".into(), now()), "Swap the Friday shift?");
        answer.state = RunState::Succeeded;
        answer.ended_at = Some(now());
        answer.omega_wrote = true;
        let office = held_office(&dir, &e, &[&message, &answer]).await;
        assert_eq!(office.answer_card(&answer.id, 0, CardReply::Choice { index: 1 }).unwrap().len(), 1);
        assert!(office.db().run(&message.id).unwrap().unwrap().run.archived_at.is_none(), "the answer goes on in the conversation");
        assert!(office.conversations().unwrap().threads.iter().any(|t| t.root.id == message.id), "and it stays open");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn turns_an_older_chat_left_waiting_move_to_the_work_lane() {
        let dir = temp_dir("old-chat");
        let e = employee_named("Cy");
        let message = Run::new(&e.id, RunKind::Chat, "Can you draft the launch note?".into(), now());
        let came = RunKind::Answered { asked_run_id: "asked-1".into(), from: e.id.clone(), task: false, chat: true, thread: None };
        let answer = Run::new(&e.id, came, "Bo answered your question.".into(), now());
        let mut done = Run::new(&e.id, RunKind::Chat, "Hi.".into(), now());
        done.state = RunState::Succeeded;
        done.ended_at = Some(now());
        let office = held_office(&dir, &e, &[&message, &answer, &done]).await;
        let stored = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run.kind;
        assert_eq!(stored(&message), task("Draft the launch note"), "your message is a conversation");
        assert_eq!(stored(&answer), RunKind::Answered { asked_run_id: "asked-1".into(), from: e.id.clone(), task: false, chat: false, thread: None });
        assert_eq!(stored(&done), RunKind::Chat, "a finished turn stays as it was");
        let work: HashSet<String> = office.db().work_runs(&e.id, 10).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(work, HashSet::from([message.id.clone(), answer.id.clone()]));
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_older_chat_answer_comes_back_into_the_chat_message_that_asked() {
        let dir = temp_dir("old-chat-answer");
        let ana = employee_named("Ana");
        let bo = employee_named("Bo");
        let cy = Employee { manager_id: Some(ana.id.clone()), ..employee_named("Cy") };
        let mut message = Run::new(&cy.id, RunKind::Chat, "Can you check the rota with Bo?".into(), now());
        message.state = RunState::Succeeded;
        message.ended_at = Some(now());
        let mut asked = Run::new(&bo.id, RunKind::Asked { parent_run_id: message.id.clone(), asked_by: cy.id.clone(), task: false }, "Is the rota final?".into(), now());
        asked.state = RunState::Succeeded;
        asked.ended_at = Some(now());
        let came = RunKind::Answered { asked_run_id: asked.id.clone(), from: bo.id.clone(), task: false, chat: true, thread: None };
        let answer = Run::new(&cy.id, came, "Bo answered your question.".into(), now());
        {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&ana).unwrap();
            db.save_employee(&bo).unwrap();
        }
        let office = held_office(&dir, &cy, &[&message, &asked, &answer]).await;
        let kind = office.db().run(&answer.id).unwrap().unwrap().run.kind;
        assert_eq!(kind, RunKind::Answered { asked_run_id: asked.id.clone(), from: bo.id.clone(), task: false, chat: false, thread: Some(message.id.clone()) });
        let open = office.conversations().unwrap().threads;
        let chat = open.iter().find(|t| t.root.id == message.id).expect("the chat message is a conversation to follow");
        assert_eq!(chat.replies.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![answer.id.clone()], "even though Cy reports to Ana");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_review_you_are_talking_about_leaves_the_rest_of_its_week_in_the_archive() {
        let dir = temp_dir("week-talking");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let week = plain_week(&db, &["ada", "bo", "cy"]);
        talk_about(&db, &week[0]);
        let ids = |t: &Thread| -> HashSet<String> { std::iter::once(t).chain(&t.related).map(|x| x.root.id.clone()).collect() };
        let archived: Vec<HashSet<String>> = Office::rows(&db, View::Archive).unwrap().iter().map(ids).collect();
        assert_eq!(archived, vec![HashSet::from([week[1].id.clone(), week[2].id.clone()])], "the conversation stays open, the rest of the week is here");
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_week_leaves_out_the_reviews_of_someone_you_let_go() {
        let dir = temp_dir("week-departed");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let week = plain_week(&db, &["bo", "dee"]);
        // Both need you, and the review by the one you let go is the newer.
        for r in &week {
            let mut r = r.clone();
            r.needs_you = vec!["Approve the order.".into()];
            db.save_run(&r).unwrap();
        }
        db.depart_employee(&week[1].employee_id, now()).unwrap();
        let ids = |t: &Thread| -> HashSet<String> { std::iter::once(t).chain(&t.related).map(|x| x.root.id.clone()).collect() };
        let inbox = Office::rows(&db, View::Inbox).unwrap();
        assert_eq!(inbox.iter().map(ids).collect::<Vec<_>>(), vec![HashSet::from([week[0].id.clone()])]);
        assert_eq!(inbox[0].root.id, week[0].id, "a current employee's decision leads the row");
        let row = Office::row_of(&db, &week[0]).unwrap();
        assert_eq!(row.iter().map(|t| t.root.id.clone()).collect::<Vec<_>>(), vec![week[0].id.clone()], "Done reaches only what the row shows");
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_archive_holds_what_reached_you_and_needs_nothing_and_the_inbox_what_does() {
        let dir = temp_dir("archive");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let e = employee_named("Cy");
        db.save_employee(&e).unwrap();
        let ago = |hours: f64| now() - chrono::Duration::minutes((hours * 60.0) as i64);
        let report = |hours: f64, f: &dyn Fn(&mut Run)| {
            let at = ago(hours);
            let mut r = Run::new(&e.id, RunKind::Manual, "go".into(), at);
            r.state = RunState::Succeeded;
            r.ended_at = Some(at);
            r.summary = Some("Done.".into());
            f(&mut r);
            assert!(db.insert_run(&r, &r.id).unwrap());
            r
        };
        let plain = report(1.0, &|_| {});
        let carded = report(2.0, &|r| *r = Run { id: r.id.clone(), created_at: r.created_at, ended_at: r.ended_at, ..carded(&e) });
        let limited = report(3.0, &|r| {
            r.state = RunState::Failed;
            r.limited = true;
        });
        let broken = report(4.0, &|r| r.state = RunState::Failed);
        let old = report(24.0 * 10.0, &|_| {});
        let done = report(30.0, &|r| r.archived_at = Some(ago(0.5)));
        let talking = report(5.0, &|_| {});
        talk_about(&db, &talking);
        // A reply an older build let you type into a report while it worked, with no later turn, then it finished
        // needing nothing. Now such a report is marked `steered` and waits in the Inbox.
        let steered = report(7.0, &|r| r.replied_at = Some(ago(6.5)));
        // A conversation you started and marked done, with the employee's answer in it, and one still going.
        let task = |hours: f64, archived: Option<Ts>| {
            let at = ago(hours);
            let mut r = Run::new(&e.id, task("Draft it"), "Draft it.".into(), at);
            r.state = RunState::Succeeded;
            r.ended_at = Some(at);
            r.archived_at = archived;
            assert!(db.insert_run(&r, &r.id).unwrap());
            r
        };
        let finished = task(50.0, Some(ago(2.0)));
        let mut turn = Run::new(&e.id, RunKind::Reply { to_run_id: finished.id.clone(), label: "Draft it".into(), task: true, card: false }, "Shorter.".into(), ago(49.0));
        turn.state = RunState::Succeeded;
        turn.ended_at = Some(ago(49.0));
        assert!(db.insert_run(&turn, &finished.id).unwrap());
        let going = task(6.0, None);

        let roots = |v: &[Thread]| v.iter().map(|t| t.root.id.clone()).collect::<Vec<_>>();
        let archive = Office::rows(&db, View::Archive).unwrap();
        assert_eq!(
            roots(&archive),
            vec![done.id.clone(), plain.id.clone(), finished.id.clone(), limited.id.clone(), steered.id.clone(), old.id.clone()],
            "newest first by when each was done or last moved; what needs you, a conversation still going and an open task stay out"
        );
        assert_eq!(archive[2].replies.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![turn.id.clone()], "a finished conversation keeps its turns");
        assert!(!roots(&archive).contains(&talking.id) && !roots(&archive).contains(&going.id), "those are open conversations");
        let inbox = Office::rows(&db, View::Inbox).unwrap();
        assert_eq!(roots(&inbox), vec![carded.id.clone(), broken.id.clone()], "the usage limit is never yours to fix");
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_quiet_routine_you_wrote_into_waits_in_the_inbox_until_you_mark_it_done() {
        let dir = temp_dir("steered-row");
        let db = Store::open(&dir.join("office.db")).unwrap();
        let e = employee_named("Cy");
        db.save_employee(&e).unwrap();
        let sweep = |mins: i64, steered: bool| {
            let at = now() - chrono::Duration::minutes(mins);
            let kind = RunKind::Scheduled { schedule_id: "sweep".into(), slot: at, label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
            let mut r = Run::new(&e.id, kind, "Sweep the inbox.".into(), at);
            r.state = RunState::Succeeded;
            r.ended_at = Some(at);
            r.summary = Some("Filed 12 emails.".into());
            r.steered = steered;
            assert!(db.insert_run(&r, &r.id).unwrap());
            r
        };
        let (_plain, watched) = (sweep(20, false), sweep(10, true));
        let roots = |view: View| Office::rows(&db, view).unwrap().into_iter().map(|t| t.root.id).collect::<Vec<_>>();
        assert_eq!(roots(View::Inbox), vec![watched.id.clone()]);
        assert!(roots(View::Archive).is_empty(), "a quiet run that needed nothing reaches you nowhere");
        let mut done = db.run(&watched.id).unwrap().unwrap().run;
        done.archived_at = Some(now());
        db.save_run(&done).unwrap();
        assert!(roots(View::Inbox).is_empty());
        assert_eq!(roots(View::Archive), vec![watched.id.clone()]);
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn typing_into_a_working_report_keeps_it_for_you_unless_you_send_it_away() {
        let dir = temp_dir("steered-report");
        let e = employee_named("Cy");
        let mut broke = Run::new(&e.id, RunKind::Manual, "Count the stock.".into(), now());
        broke.state = RunState::Failed;
        broke.ended_at = Some(now());
        broke.steered = true;
        let office = held_office(&dir, &e, &[&broke]).await;
        // Saved once the office is open, so they are not taken for runs a restart cut off.
        let live = |prompt: &str| {
            let mut r = Run::new(&e.id, RunKind::Manual, prompt.into(), now());
            r.state = RunState::Running;
            assert!(office.db().insert_run(&r, &r.id).unwrap());
            let (input, heard) = tokio::sync::mpsc::unbounded_channel();
            let a = Active { token: "t".into(), employee_id: e.id.clone(), input: Some(input), ..Default::default() };
            lock(&office.0.active).insert(r.id.clone(), a);
            (r, heard)
        };
        let ((kept, _kept_heard), (sent, _sent_heard)) = (live("Sweep the inbox."), live("Pay the invoices."));
        assert_eq!(office.reply_to_run(&kept.id, "Use last week's numbers.", false, vec![]).unwrap().id, kept.id, "it went in as it worked");
        office.reply_to_run(&sent.id, "Only the overdue ones.", true, vec![]).unwrap();
        let steered = |r: &Run| office.run(&r.id).unwrap().steered;
        assert!(steered(&kept), "its report waits for you like a reply");
        assert!(!steered(&sent), "Send & done brings it back only if it needs you");
        // A retry of a run you wrote into goes on with you, not to the Archive.
        office.retry_run(&broke.id).unwrap();
        assert!(office.run(&broke.id).unwrap().archived_at.is_none());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_open_card_keeps_its_row_in_the_inbox() {
        let dir = temp_dir("card-row");
        let e = employee_named("Cy");
        let report = carded(&e);
        {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&e).unwrap();
            assert!(db.insert_run(&report, &report.id).unwrap());
        }
        let office = open_office(&dir).await;
        assert_eq!(office.set_archived(&report.id, true).unwrap_err().message, "Answer the card first, or dismiss it.");
        assert!(office.set_archived(&report.id, false).is_ok(), "moving it back is never blocked");
        assert_eq!(office.answer_card(&report.id, 0, CardReply::Choice { index: 5 }).unwrap_err().message, "That answer does not fit this card.");
        let unreadable = vec![NewAttachment { name: "notes.txt".into(), data: "not base64!".into() }];
        assert_eq!(office.reply_to_run(&report.id, "Not now.", false, unreadable).unwrap_err().message, "notes.txt could not be read");
        assert!(office.db().run(&report.id).unwrap().unwrap().run.has_open_card(), "a reply that did not go leaves the card open");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A finished run of `e` waiting on the Chrome chore, in `state`.
    fn chrome_wait(e: &Employee, state: RunState) -> Run {
        let mut r = Run::new(&e.id, RunKind::Manual, "go".into(), now());
        r.state = state;
        r.ended_at = state.is_finished().then(now);
        r.needs_you = vec!["Open Chrome.".into()];
        r.cards = vec![Card::new("Open Chrome.", None, None, CardAsk::Chore { subject: cards::CHROME.into(), url: None })];
        r
    }

    /// An office on `dir` whose work is held for the usage limit, so nothing it queues starts.
    async fn held_office(dir: &Path, e: &Employee, runs: &[&Run]) -> Office {
        {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(e).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            for r in runs {
                assert!(db.insert_run(r, &r.id).unwrap());
            }
        }
        open_office(dir).await
    }

    #[tokio::test]
    async fn one_answer_reaches_everyone_asked_the_same_thing_and_close_ask_withdraws_one() {
        let dir = temp_dir("same-ask");
        let ada = employee_named("Cy");
        let bo = employee_named("Bo");
        let ask = |e: &Employee, message: &str, key: &str| {
            let mut r = Run::new(&e.id, RunKind::Manual, "go".into(), now());
            r.state = RunState::Succeeded;
            r.ended_at = Some(now());
            r.needs_you = vec![message.into()];
            r.cards = vec![Card::new(message, Some("Top page.".into()), Some(key.into()), CardAsk::Choices { choices: vec!["Merge".into(), "Not now".into()], recommended: 0 })];
            r
        };
        let (a, b, c) = (ask(&ada, "Merge PR #12?", "ask:pr:12"), ask(&bo, "Can #12 go live?", "ask:pr:12"), ask(&bo, "Merge PR #13?", "ask:pr:13"));
        let office = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&bo).unwrap();
            drop(db);
            held_office(&dir, &ada, &[&a, &b, &c]).await
        };
        let replies = office.answer_card(&a.id, 0, CardReply::Choice { index: 0 }).unwrap();
        // Each run hears it under its own question (quoted whole here, since these sessions never started).
        let heard = |q: &str| replies.iter().filter(|r| r.prompt.ends_with(&format!("Your card: \"{q}\"\nChose: Merge."))).count();
        assert_eq!((replies.len(), heard("Merge PR #12?"), heard("Can #12 go live?")), (2, 1, 1));
        let stored = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run;
        for r in [&a, &b] {
            let s = stored(r);
            assert_eq!(s.cards[0].answer.as_ref().map(|x| (x.how, x.text.as_str())), Some((AnswerHow::Tapped, "Chose: Merge.")));
            assert!(s.archived_at.is_some(), "answered, so it leaves the Inbox");
        }
        assert!(stored(&c).has_open_card(), "another key is another question");

        let err = office.close_ask_tool(&bo, "pr:14", "No such thing.").unwrap_err().message;
        assert_eq!(err, "You have no open card about 'pr:14'. Your open cards: pr:13.");
        assert_eq!(office.close_ask_tool(&bo, "PR:13", "Merged by hand.").unwrap()["closed"], 1);
        let c = stored(&c);
        assert_eq!(c.cards[0].answer.as_ref().map(|x| (x.how, x.text.as_str())), Some((AnswerHow::Withdrawn, "Merged by hand.")));
        assert!(c.archived_at.is_some());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A finished report of `e` asking two decisions at once: boost the post (Not now / Go), and post the recap.
    fn two_cards(e: &Employee) -> Run {
        let mut r = carded(e);
        r.needs_you.push("Post the recap?".into());
        r.cards.push(Card::new("Post the recap?", None, Some("ask:recap".into()), CardAsk::Choices { choices: vec!["Post".into(), "Not now".into()], recommended: 0 }));
        r
    }

    #[tokio::test]
    async fn a_runs_answers_go_together_once_its_last_card_is_answered() {
        let dir = temp_dir("card-set");
        let e = employee_named("Cy");
        let report = two_cards(&e);
        let office = held_office(&dir, &e, &[&report]).await;
        let stored = || office.db().run(&report.id).unwrap().unwrap().run;
        assert!(office.answer_card(&report.id, 1, CardReply::Choice { index: 0 }).unwrap().is_empty(), "held for the other card");
        let s = stored();
        assert!(s.cards[1].answer.as_ref().is_some_and(|a| a.held) && s.archived_at.is_none());
        let replies = office.answer_card(&report.id, 0, CardReply::Choice { index: 1 }).unwrap();
        assert_eq!(replies.len(), 1, "one message for both");
        let both = "Your card: \"Boost the post?\"\nChose: Go.\n\nYour card: \"Post the recap?\"\nChose: Post.";
        assert!(replies[0].prompt.ends_with(both), "{}", replies[0].prompt);
        let s = stored();
        assert!(s.cards.iter().all(|c| c.answer.as_ref().is_some_and(|a| !a.held)));
        assert!(s.archived_at.is_some(), "nothing left to ask, so it leaves the Inbox");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_reply_you_type_carries_the_answers_held_for_it_and_leaves_the_rest_open() {
        let dir = temp_dir("card-typed");
        let e = employee_named("Cy");
        let report = two_cards(&e);
        let office = held_office(&dir, &e, &[&report]).await;
        office.answer_card(&report.id, 1, CardReply::Choice { index: 0 }).unwrap();
        assert_eq!(office.reply_to_run(&report.id, "Thanks.", true, vec![]).unwrap_err().message, "Answer the card first, or dismiss it.");
        assert!(office.thread(&report.id).unwrap().replies.is_empty(), "Send & done sent nothing while a card is open");
        let reply = office.reply_to_run(&report.id, "What does a boost cost?", false, vec![]).unwrap();
        assert!(reply.prompt.ends_with("Your card: \"Post the recap?\"\nChose: Post.\n\nWhat does a boost cost?"), "{}", reply.prompt);
        let s = office.db().run(&report.id).unwrap().unwrap().run;
        assert!(s.cards[0].is_open(), "a reply you type answers no card");
        assert!(s.cards[1].answer.as_ref().is_some_and(|a| !a.held), "it went with the reply");
        assert!(s.archived_at.is_none());
        let replies = office.answer_card(&report.id, 0, CardReply::Choice { index: 1 }).unwrap();
        assert!(replies[0].prompt.ends_with("Your card: \"Boost the post?\"\nChose: Go."), "only the new answer goes: {}", replies[0].prompt);
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn your_own_words_on_a_card_answer_it_and_wait_for_the_rest_like_a_tap() {
        let dir = temp_dir("card-text");
        let e = employee_named("Cy");
        let report = two_cards(&e);
        // Someone else asking the same thing hears the same answer.
        let mut same = two_cards(&e);
        same.cards.remove(0);
        let office = held_office(&dir, &e, &[&report, &same]).await;
        let stored = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run;
        let replies = office.answer_card(&report.id, 1, CardReply::Text { text: " Only on Friday. ".into() }).unwrap();
        assert_eq!(replies.len(), 1, "the other run has nothing else to ask, so it hears it now");
        assert!(replies[0].prompt.ends_with("Your card: \"Post the recap?\"\nAnswered: \"Only on Friday.\"\nThis answer closed the card, so it is no longer in their Inbox. If you still need their decision, raise the card again."), "{}", replies[0].prompt);
        let s = stored(&report);
        assert_eq!(s.cards[1].answer.as_ref().map(|a| (a.how, a.text.as_str(), a.held)), Some((AnswerHow::Typed, "Only on Friday.", true)));
        assert!(s.cards[0].is_open(), "it answers that card only");
        let replies = office.answer_card(&report.id, 0, CardReply::Choice { index: 1 }).unwrap();
        let both = "Your card: \"Boost the post?\"\nChose: Go.\n\nYour card: \"Post the recap?\"\nAnswered: \"Only on Friday.\"\nThis answer closed the card, so it is no longer in their Inbox. If you still need their decision, raise the card again.";
        assert!(replies[0].prompt.ends_with(both), "{}", replies[0].prompt);
        assert_eq!(office.answer_card(&same.id, 0, CardReply::Text { text: "Again.".into() }).unwrap_err().message, "That card was already answered.");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_question_about_a_card_goes_now_and_keeps_it_open() {
        let dir = temp_dir("card-ask");
        let e = employee_named("Cy");
        let report = two_cards(&e);
        let office = held_office(&dir, &e, &[&report]).await;
        office.answer_card(&report.id, 1, CardReply::Choice { index: 0 }).unwrap();
        assert_eq!(office.answer_card(&report.id, 0, CardReply::Ask { text: " ".into() }).unwrap_err().message, "Type your question first.");
        let asked = office.answer_card(&report.id, 0, CardReply::Ask { text: "What does a boost cost?".into() }).unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].kind, RunKind::Reply { to_run_id: report.id.clone(), label: "Manual run".into(), task: false, card: false });
        assert!(!asked[0].omega_wrote, "your own words");
        assert!(asked[0].about_card, "about the card, so it leaves the thread as it was");
        assert!(asked[0].prompt.ends_with("About your card \"Boost the post?\", which stays open until I answer it:\n\nWhat does a boost cost?"), "{}", asked[0].prompt);
        let s = office.db().run(&report.id).unwrap().unwrap().run;
        assert!(s.cards[0].is_open(), "still yours to answer");
        assert!(s.cards[1].answer.as_ref().is_some_and(|a| a.held), "the tap still waits for it");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_question_about_a_card_leaves_a_later_tap_to_file_the_row() {
        let dir = temp_dir("card-ask-tap");
        let e = employee_named("Cy");
        let report = carded(&e);
        let office = held_office(&dir, &e, &[&report]).await;
        office.answer_card(&report.id, 0, CardReply::Ask { text: "What does a boost cost?".into() }).unwrap();
        assert!(!yours(&office.thread(&report.id).unwrap()), "a question about the card is no conversation you are in");
        office.answer_card(&report.id, 0, CardReply::Choice { index: 1 }).unwrap();
        assert!(office.db().run(&report.id).unwrap().unwrap().run.archived_at.is_some(), "the tap files it, as it would without the question");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_card_put_off_still_waits_until_it_is_settled() {
        let dir = temp_dir("put-off");
        let e = employee_named("Cy");
        let keyed = |key: &str| {
            let mut r = carded(&e);
            r.cards[0].key = Some(key.into());
            r
        };
        let (put_off, asked_again, settled) = (keyed("ask:boost"), keyed("ask:boost"), keyed("ask:recap"));
        let mut news = with_news(&e, true);
        news.cards[0].key = Some("ask:ads".into());
        let office = held_office(&dir, &e, &[&put_off, &asked_again, &settled, &news]).await;
        let stored = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run;
        let until = now() + chrono::Duration::days(2);
        office.answer_card(&put_off.id, 0, CardReply::Later { until }).unwrap();
        office.answer_card(&settled.id, 0, CardReply::Later { until }).unwrap();
        assert_eq!(stored(&asked_again).cards[0].back_at, Some(until), "Later reaches the other card asked the same thing too");

        // An answer about the same thing reaches a card put off, and close_ask withdraws one.
        office.answer_card(&asked_again.id, 0, CardReply::Choice { index: 1 }).unwrap();
        let c = stored(&put_off).cards.remove(0);
        assert_eq!((c.answer.map(|a| a.text), c.back_at), (Some("Chose: Go.".into()), None));
        assert_eq!(office.close_ask_tool(&e, "recap", "Posted it myself.").unwrap()["closed"], 1);
        let c = stored(&settled).cards.remove(0);
        assert_eq!((c.answer.map(|a| a.how), c.back_at), (Some(AnswerHow::Withdrawn), None));
        assert_eq!(office.bring_back_cards(until + chrono::Duration::minutes(1)).unwrap(), 0, "settled, so nothing comes back");

        // Closing a card leaves news you have not seen where you will see it.
        assert_eq!(office.close_ask_tool(&e, "ads", "Paused the ads.").unwrap()["closed"], 1);
        let n = stored(&news);
        assert!(n.archived_at.is_none() && n.has_news(), "{:?}", n.cards);
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn approving_a_block_keeps_news_you_have_not_seen() {
        let dir = temp_dir("approve-news");
        let e = employee_named("Cy");
        let mut blocked = with_news(&e, false);
        blocked.denials = vec![Denial { tool: "Bash".into(), brief: "git push".into(), suggested_rule: "Bash(git push:*)".into() }];
        let office = held_office(&dir, &e, &[&blocked]).await;
        office.approve_blocked(&blocked.id, vec!["Bash(git push:*)".into()], false).unwrap();
        let s = office.db().run(&blocked.id).unwrap().unwrap().run;
        assert!(s.archived_at.is_none() && s.has_news(), "the retry goes, and the news waits to be seen");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn approving_a_block_keeps_a_card_you_have_not_answered() {
        let dir = temp_dir("approve-card");
        let e = employee_named("Cy");
        let mut blocked = carded(&e);
        blocked.denials = vec![Denial { tool: "Bash".into(), brief: "git push".into(), suggested_rule: "Bash(git push:*)".into() }];
        let office = held_office(&dir, &e, &[&blocked]).await;
        office.approve_blocked(&blocked.id, vec!["Bash(git push:*)".into()], false).unwrap();
        let s = office.db().run(&blocked.id).unwrap().unwrap().run;
        assert!(s.archived_at.is_none() && s.has_open_card(), "the retry goes, and the card waits for your answer");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn retry_sends_an_answer_once_whatever_omega_star_queued_again() {
        let dir = temp_dir("retry-answer");
        let ada = employee_named("Cy");
        let answer = |asked: &str, mins: i64, state: RunState| {
            let came = RunKind::Answered { asked_run_id: asked.into(), from: "e-bo".into(), task: false, chat: false, thread: None };
            let mut r = Run::new(&ada.id, came, format!("Bo answered your question {asked}."), now() - chrono::Duration::minutes(mins));
            (r.state, r.omega_wrote) = (state, true);
            r.ended_at = state.is_finished().then(now);
            r
        };
        // The answer failed, and OmegaStar queued it again for ten minutes on.
        let failed = answer("a1", 2, RunState::Failed);
        let mut copy = answer("a1", 1, RunState::Queued);
        copy.not_before = Some(now() + chrono::Duration::minutes(10));
        // Here the copy failed too, and Retry all retries both.
        let (first, second) = (answer("a2", 2, RunState::Failed), answer("a2", 1, RunState::Failed));
        let office = held_office(&dir, &ada, &[&failed, &copy, &first, &second]).await;

        let again = office.retry_run(&failed.id).unwrap();
        assert_eq!((again.id.as_str(), again.not_before), (copy.id.as_str(), None), "the copy goes now, rather than a second one");
        assert!(office.db().run(&failed.id).unwrap().unwrap().run.archived_at.is_some());
        let mut done = office.run(&copy.id).unwrap();
        (done.state, done.ended_at) = (RunState::Succeeded, Some(now()));
        office.db().save_run(&done).unwrap();
        assert_eq!(office.retry_run(&failed.id).unwrap_err().message, "This answer already went again, so there is nothing to retry.");

        let one = office.retry_run(&first.id).unwrap();
        let two = office.retry_run(&second.id).unwrap();
        assert_eq!(one.id, two.id, "one new run brings it");
        assert_eq!(office.db().answers_to("a2").unwrap().len(), 3);
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn closing_the_last_open_card_sends_the_answers_held_for_it() {
        let dir = temp_dir("card-closed");
        let e = employee_named("Cy");
        let report = two_cards(&e);
        let office = held_office(&dir, &e, &[&report]).await;
        office.answer_card(&report.id, 1, CardReply::Choice { index: 0 }).unwrap();
        assert_eq!(office.close_ask_tool(&e, "boost", "Boosted it myself.").unwrap()["closed"], 1);
        let t = office.thread(&report.id).unwrap();
        assert_eq!(t.replies.len(), 1);
        assert!(t.replies[0].prompt.ends_with("Your card: \"Post the recap?\"\nChose: Post."), "{}", t.replies[0].prompt);
        assert!(t.root.cards[1].answer.as_ref().is_some_and(|a| !a.held));
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn later_brings_a_card_back_by_itself_once_then_parks_it() {
        let dir = temp_dir("later");
        let e = employee_named("Cy");
        let report = carded(&e);
        let office = held_office(&dir, &e, &[&report]).await;
        let stored = || office.db().run(&report.id).unwrap().unwrap().run;
        let until = now() + chrono::Duration::days(2);
        let told = office.answer_card(&report.id, 0, CardReply::Later { until }).unwrap();
        assert_eq!(told.len(), 1);
        assert!(told[0].prompt.ends_with(&format!("Your card: \"Boost the post?\"\n{}", cards::later_text("the person you work for", until))), "{}", told[0].prompt);
        let s = stored();
        assert_eq!((s.cards[0].answer.as_ref().map(|a| a.how), s.cards[0].back_at), (Some(AnswerHow::Dismissed), Some(until)));
        assert!(s.archived_at.is_some(), "answered for now, so it leaves the Inbox");
        assert!(office.db().checks(&e.id, None).unwrap().is_empty(), "no run brings it back");

        assert_eq!(office.bring_back_cards(now()).unwrap(), 0, "not yet");
        assert_eq!(office.bring_back_cards(until + chrono::Duration::minutes(1)).unwrap(), 1);
        let s = stored();
        assert!(s.cards[0].is_open() && s.cards[0].back_at.is_none() && s.cards[0].times_back == 1);
        assert!(s.archived_at.is_none() && s.needs_you == vec!["Boost the post?".to_string()], "back in the Inbox");
        assert!(Office::rows(&office.db(), View::Inbox).unwrap().iter().any(|t| t.root.id == report.id));
        assert_eq!(office.bring_back_cards(until + chrono::Duration::minutes(2)).unwrap(), 0, "once");

        // Later again parks it, where it waits for any answer.
        let parked = office.answer_card(&report.id, 0, CardReply::Later { until: until + chrono::Duration::days(1) }).unwrap();
        assert!(parked[0].prompt.ends_with(&cards::parked_text("the person you work for")), "{}", parked[0].prompt);
        let s = stored();
        assert!(s.cards[0].parked && s.cards[0].back_at.is_none());
        assert!(!Office::rows(&office.db(), View::Inbox).unwrap().iter().any(|t| t.root.id == report.id), "a parked card never needs you");
        assert_eq!(office.parked_runs().unwrap().iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![report.id.clone()]);
        let went = office.answer_card(&report.id, 0, CardReply::Choice { index: 1 }).unwrap();
        assert!(went[0].prompt.ends_with("Your card: \"Boost the post?\"\nChose: Go."), "{}", went[0].prompt);
        assert!(!stored().cards[0].parked && office.parked_runs().unwrap().is_empty());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn not_now_from_an_older_inbox_is_later_until_tomorrow_and_no_drops_it() {
        let dir = temp_dir("not-now");
        let e = employee_named("Cy");
        let later = carded(&e);
        let mut no = carded(&e);
        no.cards[0].key = Some("ask:recap".into());
        let chore = chrome_wait(&e, RunState::Succeeded);
        let office = held_office(&dir, &e, &[&later, &no, &chore]).await;
        let at = now();
        office.answer_card(&later.id, 0, CardReply::Dismiss).unwrap();
        office.answer_card(&no.id, 0, CardReply::Reject).unwrap();
        office.answer_card(&chore.id, 0, CardReply::Dismiss).unwrap();
        let card = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run.cards.remove(0);
        let back = cards::tomorrow_morning(at);
        assert_eq!(card(&later).answer.map(|a| (a.how, a.text)), Some((AnswerHow::Dismissed, cards::later_text("the person you work for", back))));
        assert_eq!(card(&later).back_at, Some(back));
        assert_eq!(card(&no).answer.map(|a| (a.how, a.text)), Some((AnswerHow::Rejected, cards::REJECTED.into())));
        assert_eq!(card(&chore).answer.map(|a| a.text), Some(String::new()), "a chore's Not now sends nothing");
        assert_eq!(card(&chore).back_at, None, "and never comes back");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A finished report of `e` telling you news, and with `decision` asking something too.
    fn with_news(e: &Employee, decision: bool) -> Run {
        let mut r = if decision { carded(e) } else { Run { needs_you: vec![], cards: vec![], ..carded(e) } };
        r.needs_you.push("The spring sale is live.".into());
        r.cards.push(Card::new("The spring sale is live.", None, Some("ask:sale".into()), CardAsk::News));
        r
    }

    #[tokio::test]
    async fn news_keeps_its_row_until_seen_and_holds_up_no_answer() {
        let dir = temp_dir("news");
        let e = employee_named("Cy");
        let report = with_news(&e, true);
        let office = held_office(&dir, &e, &[&report]).await;
        let stored = || office.db().run(&report.id).unwrap().unwrap().run;
        let in_inbox = || Office::rows(&office.db(), View::Inbox).unwrap().iter().any(|t| t.root.id == report.id);
        assert!(in_inbox());
        let told = office.answer_card(&report.id, 0, CardReply::Choice { index: 1 }).unwrap();
        assert_eq!(told.len(), 1, "unseen news holds no answer back");
        assert!(stored().archived_at.is_none() && in_inbox(), "but keeps its row until you see it");
        assert!(office.answer_card(&report.id, 1, CardReply::Done).is_err(), "news is seen, or followed up in words");
        assert!(office.answer_card(&report.id, 1, CardReply::Seen).unwrap().is_empty(), "seeing it sends nothing");
        assert!(stored().archived_at.is_some() && !in_inbox());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn your_words_on_news_follow_it_up_and_count_as_seeing_it() {
        let dir = temp_dir("news-words");
        let e = employee_named("Cy");
        let report = with_news(&e, false);
        let office = held_office(&dir, &e, &[&report]).await;
        let told = office.answer_card(&report.id, 0, CardReply::Text { text: " Does it cover the outlet store? ".into() }).unwrap();
        assert!(matches!(&told[..], [r] if r.prompt.ends_with("Answered: \"Does it cover the outlet store?\"\nThis answer closed the card, so it is no longer in their Inbox. If you still need their decision, raise the card again.")), "{told:?}");
        assert!(office.db().run(&report.id).unwrap().unwrap().run.archived_at.is_some(), "it counts as seen");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn done_counts_as_seeing_news_and_undo_unsees_it() {
        let dir = temp_dir("news-done");
        let e = employee_named("Cy");
        let report = with_news(&e, false);
        let office = held_office(&dir, &e, &[&report]).await;
        let stored = || office.db().run(&report.id).unwrap().unwrap().run;
        let filed = office.set_archived(&report.id, true).unwrap();
        assert_eq!(filed, vec![report.id.clone()], "news never holds up Done");
        let s = stored();
        assert_eq!(s.cards[0].answer.as_ref().map(|a| (a.at, a.how)), Some((s.archived_at.unwrap(), AnswerHow::Tapped)), "seen as you archived it");
        office.unarchive(&filed).unwrap();
        let s = stored();
        assert!(s.archived_at.is_none() && s.has_news(), "Undo brings it back unseen");
        assert!(Office::rows(&office.db(), View::Inbox).unwrap().iter().any(|t| t.root.id == report.id));
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_shared_key_hears_a_choice_only_where_it_is_offered() {
        let dir = temp_dir("shared-key");
        let ada = employee_named("Cy");
        let bo = employee_named("Bo");
        let ask = |e: &Employee, choices: &[&str]| {
            let mut r = Run::new(&e.id, RunKind::Manual, "go".into(), now());
            r.state = RunState::Succeeded;
            r.ended_at = Some(now());
            r.needs_you = vec!["What about PR #12?".into()];
            r.cards = vec![Card::new("What about PR #12?", None, Some("ask:pr:12".into()), CardAsk::Choices { choices: choices.iter().map(|c| c.to_string()).collect(), recommended: 0 })];
            r
        };
        let (merge, also, hold) = (ask(&ada, &["Merge"]), ask(&bo, &["Hold it", "merge"]), ask(&bo, &["Hold it"]));
        let office = {
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&bo).unwrap();
            drop(db);
            held_office(&dir, &ada, &[&merge, &also, &hold]).await
        };
        let replies = office.answer_card(&merge.id, 0, CardReply::Choice { index: 0 }).unwrap();
        assert_eq!(replies.len(), 2);
        let card = |r: &Run| office.db().run(&r.id).unwrap().unwrap().run.cards.remove(0);
        assert_eq!(card(&also).answer.map(|a| a.text), Some("Chose: merge.".into()), "its own label");
        assert!(card(&hold).is_open(), "Merge is not one of its choices, so it stays open");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn going_ahead_with_a_company_line_adds_it_to_the_page() {
        let dir = temp_dir("company-line");
        let e = employee_named("Cy");
        let proposed = |line: &str| {
            let mut r = Run::new(&e.id, RunKind::Manual, "go".into(), now());
            r.state = RunState::Succeeded;
            r.ended_at = Some(now());
            let card = cards::company_line_card("Right now", line, "Everyone books rooms.").unwrap();
            r.needs_you = vec![card.message.clone()];
            r.cards = vec![card];
            r
        };
        let (yes, no) = (proposed("The big room is closed until May."), proposed("The kitchen is open late."));
        let office = held_office(&dir, &e, &[&yes, &no]).await;
        let told = office.answer_card(&yes.id, 0, CardReply::Choice { index: 0 }).unwrap();
        assert!(told[0].prompt.ends_with("Your card: \"Add to the company page?\"\nAdded to the company page under Right now."), "{}", told[0].prompt);
        let page = office.company_page().unwrap().text;
        assert!(page.contains("## Right now\n\nDated lines, newest first.\n\n- The big room is closed until May.\n"), "{page}");
        let told = office.answer_card(&no.id, 0, CardReply::Reject).unwrap();
        assert!(told[0].prompt.ends_with("\nNot added."), "{}", told[0].prompt);
        assert!(!office.company_page().unwrap().text.contains("kitchen"));
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn chrome_dropping_in_a_chat_raises_no_card() {
        let dir = temp_dir("chrome-chat");
        let e = employee_named("Cy");
        let mut chat = Run::new(&e.id, RunKind::Chat, "hi".into(), now());
        chat.state = RunState::Succeeded;
        chat.ended_at = Some(now());
        let office = held_office(&dir, &e, &[&chat]).await;
        office.chrome_down(&chat, &e);
        let stored = office.db().run(&chat.id).unwrap().unwrap().run;
        assert!(stored.cards.is_empty() && stored.needs_you.is_empty(), "the chat says it itself");
        assert!(office.db().open_topic_runs("chore:chrome").unwrap().is_empty());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn done_reaches_a_run_held_for_the_usage_limit_without_a_reply() {
        let dir = temp_dir("chore-held");
        let e = employee_named("Cy");
        let done = chrome_wait(&e, RunState::Succeeded);
        let held = chrome_wait(&e, RunState::Queued);
        let office = held_office(&dir, &e, &[&done, &held]).await;
        let replies = office.answer_card(&done.id, 0, CardReply::Done).unwrap();
        assert_eq!(replies.len(), 1, "only the finished run gets a reply");
        assert!(matches!(&replies[0].kind, RunKind::Reply { to_run_id, .. } if *to_run_id == done.id));
        let held = office.db().run(&held.id).unwrap().unwrap().run;
        assert_eq!(held.state, RunState::Queued);
        assert!(!held.cards.is_empty() && !held.has_open_card(), "its retry carries the answered card");
        assert!(held.archived_at.is_none());
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn done_on_a_working_run_clears_its_chore_headline() {
        let dir = temp_dir("chore-live");
        let e = employee_named("Cy");
        let done = chrome_wait(&e, RunState::Succeeded);
        let office = held_office(&dir, &e, &[&done]).await;
        let (input, _heard) = tokio::sync::mpsc::unbounded_channel();
        let live = chrome_wait(&e, RunState::Running);
        lock(&office.0.active).insert(
            live.id.clone(),
            Active { token: "t".into(), employee_id: e.id.clone(), needs_you: vec!["Open Chrome.".into(), "Totals look off.".into()], cards: live.cards.clone(), input: Some(input), ..Default::default() },
        );
        office.answer_card(&done.id, 0, CardReply::Done).unwrap();
        let active = lock(&office.0.active);
        let a = &active[&live.id];
        assert!(!a.cards.is_empty() && a.cards.iter().all(|c| !c.is_open()));
        assert_eq!(a.needs_you, vec!["Totals look off.".to_string()], "only the answered chore leaves");
        drop(active);
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
        let dir = temp_path("inbox");
        let path = dir.join("office.db");
        fs::create_dir_all(&dir).unwrap();
        let ada = employee_named("Ada");
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

        let office = open_office(&dir).await;
        let before = every_row(&path);
        assert_eq!(office.set_archived(&reply.id, true).unwrap(), vec![report.id.clone()]);
        assert!(office.db().run(&report.id).unwrap().unwrap().run.archived_at.is_some());
        assert_eq!(every_row(&path), before, "nothing but archivedAt changed");
        let c = rusqlite::Connection::open(&path).unwrap();
        let mut st = c.prepare("SELECT id FROM runs WHERE json_extract(data, '$.archivedAt') IS NOT NULL").unwrap();
        let stamped: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(stamped, vec![report.id.clone()]);

        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }
}
