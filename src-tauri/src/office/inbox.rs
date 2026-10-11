//! The Inbox: threads, rows, replies, archiving and approving blocked tools.

use super::*;

/// How many archived reports the Archive reads, most recently archived first.
const ARCHIVE_ROWS: u32 = 200;

impl Office {
    /// The Inbox: one row per piece of work, most recently active first. A row is a thread (a report and
    /// your exchange on it) with the threads that belong with it in `related`: everything a hand-off
    /// between employees set off, or a schedule's earlier failures in a row.
    pub fn inbox(&self) -> Result<Vec<Thread>> {
        Self::rows(&self.db(), false)
    }

    /// Colleague asks still in flight, for the Inbox's "With colleagues" list.
    pub fn open_asks(&self) -> Result<Vec<Run>> {
        self.db().open_asks()
    }

    /// The Archive: rows you archived, most recently archived first.
    pub fn archived(&self) -> Result<Vec<Thread>> {
        Self::rows(&self.db(), true)
    }

    /// The employees who report to you.
    fn top_ids(employees: &[Employee]) -> HashSet<String> {
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

    /// The Inbox, or with `archive` the Archive, as rows (`threads::file`).
    fn rows(db: &Store, archive: bool) -> Result<Vec<Thread>> {
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
        let rows = threads::merge_streaks(rows, |r| Self::streak_of(db, &cache, r));
        let mut out: Vec<Thread> = rows.into_iter().filter_map(|r| threads::present(r, archive)).collect();
        if archive {
            out.sort_by_key(|t| std::cmp::Reverse(threads::row_archived_at(t)));
        } else {
            out.sort_by_key(|t| std::cmp::Reverse(threads::activity_at(&t.root, &t.replies)));
        }
        Ok(out)
    }

    /// Every thread in the row `root` (a thread's root) is filed under, oldest first: its hand-off, and
    /// for a schedule's failure the rest of that run of failures.
    fn row_of(db: &Store, root: &Run) -> Result<Vec<Thread>> {
        let top = Self::top_ids(&db.employees()?);
        let conversation = db.conversation_runs()?;
        let lookup = |id: &str| run_of(db, id);
        let cache = std::cell::RefCell::new(HashMap::new());
        let mut starts = vec![root.clone()];
        let first = threads::file(&starts, conversation.clone(), lookup, &top).into_iter().next().and_then(|r| r.members.into_iter().next());
        if let Some((t, _)) = first {
            if let (Some(key), RunKind::Scheduled { schedule_id, .. }) = (Self::streak_of(db, &cache, &t.root), &t.root.kind) {
                let same = |f: &Run| f.id != t.root.id && Self::streak_of(db, &cache, f).as_ref() == Some(&key);
                starts.extend(db.failures_of_schedule(schedule_id)?.into_iter().filter(same));
            }
        }
        let rows = threads::merge_streaks(threads::file(&starts, conversation, lookup, &top), |r| Self::streak_of(db, &cache, r));
        Ok(rows.into_iter().next().map(|r| r.members.into_iter().map(|(t, _)| t).collect()).unwrap_or_default())
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

    /// The employee's open threads, for their Chat tab: their tasks, oldest first so each keeps its
    /// place, then reports you replied to, most recently active first. Archived ones are left out.
    pub fn threads(&self, employee_id: &str) -> Result<Vec<Thread>> {
        let db = self.db();
        let mut grouped = threads::group(db.reply_runs(Some(employee_id))?);
        let mut out: Vec<Thread> = db
            .open_tasks(employee_id)?
            .into_iter()
            .map(|root| {
                let replies = grouped.remove(&root.id).unwrap_or_default();
                Thread::new(root, replies)
            })
            .collect();
        let ids: Vec<String> = grouped.keys().cloned().collect();
        let mut reports: Vec<Thread> = db
            .runs_by_id(&ids)?
            .into_iter()
            .filter(|r| r.archived_at.is_none() && !matches!(r.kind, RunKind::Task { .. }))
            .map(|root| {
                let replies = grouped.remove(&root.id).unwrap_or_default();
                Thread::new(root, replies)
            })
            .collect();
        reports.sort_by_key(|t| std::cmp::Reverse(threads::activity_at(&t.root, &t.replies)));
        out.extend(reports);
        Ok(out)
    }

    /// The briefs left for the employee's current chat conversation, oldest first.
    pub fn chat_briefs(&self, employee_id: &str) -> Result<Vec<Brief>> {
        let db = self.db();
        let e = db.employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
        db.briefs(&e.id, &e.chat_session, false)
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
    /// Chat tab), or moves them all back. Archiving a thread you replied in, or a task, leaves a brief
    /// for its employee's ongoing chat, unless `brief` is `Some(false)`. `run_id` may be any run in the row.
    pub fn set_archived(&self, run_id: &str, archived: bool, brief: Option<bool>) -> Result<Run> {
        let (root, changed, briefed) = {
            let db = self.db();
            let t = Self::thread_in(&db, run_id)?;
            if !t.root.state.is_finished() {
                return Err(AppError::conflict("it is still working, so it cannot be archived yet"));
            }
            let ceo = ceo_of(&db)?;
            let (mut changed, mut briefed) = (vec![], false);
            for m in Self::row_of(&db, &t.root)? {
                // A run still going joins the row when it finishes, and shows only if it needs you.
                let mut root = m.root;
                if !root.state.is_finished() || root.archived_at.is_some() == archived {
                    continue;
                }
                root.archived_at = archived.then(now);
                db.save_run(&root)?;
                if !archived {
                    // Back in the Inbox: a brief no turn has read would tell the chat the thread was closed.
                    db.drop_unread_briefs(&root.id)?;
                }
                let task = matches!(root.kind, RunKind::Task { .. });
                if archived && (task || !m.replies.is_empty()) && brief.unwrap_or(true) {
                    if let Some(e) = db.employee(&root.employee_id)? {
                        let label = threads::report_label(&root.kind);
                        let (text, excerpt) = threads::brief(&ceo, &label, &root, &m.replies);
                        let b = Brief {
                            id: uuid::Uuid::new_v4().to_string(),
                            employee_id: e.id.clone(),
                            root_run_id: root.id.clone(),
                            label,
                            replies: m.replies.len() as u32,
                            excerpt,
                            text,
                            created_at: now(),
                            consumed_at: None,
                            task,
                        };
                        db.insert_brief(&b, &e.chat_session)?;
                        briefed = true;
                    }
                }
                changed.push(root);
            }
            let root = db.run(&t.root.id)?.map(|r| r.run).unwrap_or(t.root);
            (root, changed, briefed)
        };
        for r in &changed {
            self.sink().run_updated(r);
        }
        if briefed {
            self.sink().org_changed();
        }
        Ok(root)
    }

    /// Your reply in a report's thread. It goes into the thread's run while that works, or else starts a
    /// run that resumes the thread's session, so the employee answers with everything it saw. The reply
    /// joins the thread and the report stays in the Inbox, unless `archive`: "send, and bring the answer
    /// back only if it needs me".
    pub fn reply_to_run(&self, run_id: &str, text: &str, archive: bool, attachments: Vec<NewAttachment>) -> Result<Run> {
        self.reply_with(run_id, text, archive, vec![], attachments)
    }

    /// A blocked run's way forward: you allow what it was denied, and it resumes to retry. `always`
    /// saves the rules to the employee; otherwise they apply to the retry run only. The rules must
    /// come from what the run was denied: its own suggested rule, or every tool of the same MCP server.
    pub fn approve_blocked(&self, run_id: &str, rules: Vec<String>, always: bool) -> Result<Run> {
        let (original, employee, ceo, asker) = {
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
            let asker = match &original.kind {
                RunKind::Asked { asked_by, .. } => db.employee(asked_by)?.map(|a| a.name),
                _ => None,
            };
            (original, employee, ceo, asker)
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
        let mut text = format!(
            "{ceo} approved what was blocked:\n{blocked}\n\nYou may now use {} {scope}. Retry it and finish the task. \
Approval covers only what is listed, so if something else is blocked, stop and say what.",
            rules.join(", ")
        );
        if let Some(who) = asker {
            text.push_str(&format!(" When you are done, tell {who} the outcome with `ask_colleague`."));
        }
        self.reply_with(run_id, &text, false, rules, vec![])
    }

    /// `run_id` may be any run in the thread. The reply answers the thread's root and resumes the
    /// thread's newest session (the root's, or a fresh one an earlier reply had to start). A plain reply
    /// goes into that session's run while it works; one that carries files or rules is a run of its own.
    fn reply_with(&self, run_id: &str, text: &str, archive: bool, allow_once: Vec<String>, attachments: Vec<NewAttachment>) -> Result<Run> {
        let text = text.trim();
        if text.is_empty() && attachments.is_empty() {
            return Err(AppError::invalid("reply is empty"));
        }
        let files = Self::decode_attachments(&attachments)?;
        let plain = files.is_empty() && allow_once.is_empty();
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
            let mut reply = Run::new(&e.id, RunKind::Reply { to_run_id: root.id.clone(), label, task }, prompt, now());
            reply.allow_once = allow_once;
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
