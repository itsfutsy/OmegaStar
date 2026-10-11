//! The sidebar's conversations: every open thread across the office, the threads archived today, and
//! each employee's ongoing chat, composed from the Inbox's queries and the open tasks in one read.

use super::*;
use std::cmp::Reverse;

impl Office {
    /// Every conversation, for the sidebar's shelves. `done_since` (RFC 3339) is where `done` starts:
    /// local midnight in the UI.
    pub fn conversations(&self, done_since: &str) -> Result<Conversations> {
        let since = chrono::DateTime::parse_from_rfc3339(done_since)
            .map_err(|_| AppError::invalid("doneSince must be an RFC 3339 time"))?
            .with_timezone(&Utc);
        Self::conversations_in(&self.db(), since)
    }

    /// `conversations` over a store. An Inbox row stands for every thread filed under it, so those
    /// threads never show again on their own.
    fn conversations_in(db: &Store, since: Ts) -> Result<Conversations> {
        let employees = db.employees()?;
        let current: HashSet<&str> = employees.iter().map(|e| e.id.as_str()).collect();
        let top = Self::top_ids(&employees);
        let ids = |ts: &[Thread]| -> Vec<String> { ts.iter().flat_map(|t| std::iter::once(t).chain(&t.related)).map(|t| t.root.id.clone()).collect() };

        let mut open = Self::rows(db, false)?;
        let mut filed: HashSet<String> = ids(&open).into_iter().collect();
        let mut grouped = threads::group(db.reply_runs(None)?);
        let mut roots: Vec<Run> = vec![];
        for e in &employees {
            roots.extend(db.open_tasks(&e.id)?);
        }
        let replied: Vec<String> = grouped.keys().cloned().collect();
        roots.extend(db.runs_by_id(&replied)?.into_iter().filter(|r| r.archived_at.is_none()));
        for state in [RunState::Queued, RunState::Running] {
            roots.extend(db.runs_in_state(state)?.into_iter().filter(|r| top.contains(&r.employee_id) && Self::live_report(r)));
        }
        for root in roots {
            if !current.contains(root.employee_id.as_str()) || !filed.insert(root.id.clone()) {
                continue;
            }
            let replies = grouped.remove(&root.id).unwrap_or_default();
            open.push(Thread::new(root, replies));
        }
        open.sort_by_key(|t| Reverse(threads::activity_at(&t.root, &t.replies)));

        let mut done: Vec<(Ts, Thread)> = Self::rows(db, true)?
            .into_iter()
            .filter_map(|t| threads::row_archived_at(&t).filter(|at| *at >= since).map(|at| (at, t)))
            .collect();
        filed.extend(done.iter().flat_map(|(_, t)| ids(std::slice::from_ref(t))));
        for root in db.archived_roots_since(since)? {
            let Some(at) = root.archived_at.filter(|at| *at >= since) else { continue };
            if !current.contains(root.employee_id.as_str()) || !filed.insert(root.id.clone()) {
                continue;
            }
            let replies = grouped.remove(&root.id).unwrap_or_default();
            done.push((at, Thread::new(root, replies)));
        }
        done.sort_by_key(|(at, _)| Reverse(*at));

        let chats = employees
            .iter()
            .map(|e| {
                let turns = db.chat_runs(&e.id, &e.chat_session)?;
                Ok(ChatHead {
                    employee_id: e.id.clone(),
                    last_ended_at: turns.iter().filter_map(|r| r.ended_at).max(),
                    live: turns.iter().any(|r| r.state == RunState::Running),
                    queued: turns.iter().any(|r| r.state == RunState::Queued),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Conversations { threads: open, done: done.into_iter().map(|(_, t)| t).collect(), chats })
    }

    /// A queued or running run that will be a report of its own once it finishes, under the same root
    /// id: not a conversation turn, a hand-off, a task, or a quiet run.
    fn live_report(r: &Run) -> bool {
        r.kind.lane() == Lane::Work && !threads::continues(&r.kind) && !threads::quiet(&r.kind) && !matches!(r.kind, RunKind::Task { .. } | RunKind::ReplyCheck { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An employee reporting to `manager_id`, or to you.
    fn employee(name: &str, manager_id: Option<&str>) -> Employee {
        Employee {
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
        }
    }

    /// Saves a run in `state`; a finished one ended `ago` minutes back. Returns it as saved.
    fn run(db: &Store, e: &Employee, kind: RunKind, state: RunState, ago: i64, session: Option<&str>) -> Run {
        let at = now() - chrono::Duration::minutes(ago);
        let mut r = Run::new(&e.id, kind, "go".into(), at);
        r.state = state;
        if state.is_finished() {
            r.ended_at = Some(at);
        }
        let session = session.map(String::from).unwrap_or_else(|| r.id.clone());
        assert!(db.insert_run(&r, &session).unwrap());
        r
    }

    /// A task's kind.
    fn task(label: &str) -> RunKind {
        RunKind::Task { label: label.into(), from: None, context: String::new() }
    }

    #[test]
    fn conversations_gather_every_open_thread_once() {
        let dir = std::env::temp_dir().join(format!("omegastar-conversations-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let ada = employee("Ada", None);
        let bo = employee("Bo", Some(&ada.id));
        db.save_employee(&ada).unwrap();
        db.save_employee(&bo).unwrap();

        let report = run(&db, &ada, RunKind::Manual, RunState::Succeeded, 60, None);
        let replied = run(&db, &ada, RunKind::Manual, RunState::Succeeded, 50, None);
        let reply = Run::new(&ada.id, RunKind::Reply { to_run_id: replied.id.clone(), label: "Manual run".into(), task: false }, "ok".into(), now());
        db.insert_run(&reply, &replied.id).unwrap();
        let open_task = run(&db, &ada, task("Draft it"), RunState::Running, 0, None);
        let sub_report = run(&db, &bo, RunKind::Manual, RunState::Succeeded, 40, None);
        let sub_reply = Run::new(&bo.id, RunKind::Reply { to_run_id: sub_report.id.clone(), label: "Manual run".into(), task: false }, "and?".into(), now());
        db.insert_run(&sub_reply, &sub_report.id).unwrap();
        let live = run(&db, &ada, RunKind::Manual, RunState::Running, 0, None);
        let quiet = RunKind::Scheduled { schedule_id: "s".into(), slot: now(), label: "Sweep".into(), quiet: true };
        let live_quiet = run(&db, &ada, quiet, RunState::Running, 0, None);
        let live_sub = run(&db, &bo, RunKind::Manual, RunState::Queued, 0, None);

        let mut today = Run::new(&ada.id, task("Ship it"), "go".into(), now());
        today.state = RunState::Succeeded;
        today.ended_at = Some(now());
        today.archived_at = Some(now());
        db.insert_run(&today, &today.id).unwrap();
        let mut old = Run::new(&ada.id, task("Old"), "go".into(), now() - chrono::Duration::days(2));
        old.state = RunState::Succeeded;
        old.archived_at = Some(now() - chrono::Duration::days(1));
        db.insert_run(&old, &old.id).unwrap();

        let chat = run(&db, &ada, RunKind::Chat, RunState::Succeeded, 5, Some(&ada.chat_session));

        let since = now() - chrono::Duration::hours(1);
        let c = Office::conversations_in(&db, since).unwrap();
        let ids: Vec<&str> = c.threads.iter().map(|t| t.root.id.as_str()).collect();
        for want in [&report.id, &replied.id, &open_task.id, &sub_report.id, &live.id] {
            assert_eq!(ids.iter().filter(|id| **id == want.as_str()).count(), 1, "{want} once in threads: {ids:?}");
        }
        for unwanted in [&live_quiet.id, &live_sub.id, &today.id, &old.id, &chat.id] {
            assert!(!ids.contains(&unwanted.as_str()), "{unwanted} not in threads");
        }
        let with_reply = c.threads.iter().find(|t| t.root.id == replied.id).unwrap();
        assert_eq!(with_reply.replies.len(), 1, "the inbox row keeps its reply");
        assert_eq!(c.threads.iter().find(|t| t.root.id == sub_report.id).unwrap().replies.len(), 1);

        let done: Vec<&str> = c.done.iter().map(|t| t.root.id.as_str()).collect();
        assert_eq!(done, vec![today.id.as_str()], "only today's archive is done");

        let head = |e: &Employee| c.chats.iter().find(|h| h.employee_id == e.id).unwrap();
        assert_eq!(head(&ada).last_ended_at, chat.ended_at);
        assert!(!head(&ada).live && !head(&ada).queued);
        assert_eq!(head(&bo).last_ended_at, None);

        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }
}
