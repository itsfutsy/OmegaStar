use super::*;
use crate::{fixtures, threads};
use chrono::Utc;
use std::path::PathBuf;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("omegastar-store-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An employee reporting to `manager_id`, or to you.
fn employee(name: &str, manager_id: Option<&str>) -> Employee {
    let id = uuid::Uuid::new_v4().to_string();
    Employee {
        id: id.clone(),
        slug: name.to_lowercase(),
        name: name.into(),
        manager_id: manager_id.map(String::from),
        workspaces: vec![format!("/office/employees/{id}")],
        chat_session: format!("chat-{name}"),
        ..fixtures::employee()
    }
}

/// A run in `state`, ended now if it is finished.
fn run(e: &Employee, kind: RunKind, state: RunState) -> Run {
    let mut r = Run::new(&e.id, kind, "go".into(), Utc::now());
    r.state = state;
    if state.is_finished() {
        r.ended_at = Some(r.created_at);
    }
    r
}

/// Inserts a work-lane row of `e` as it is, readable or not.
fn raw(db: &Store, e: &Employee, id: &str, state: &str, session: &str, data: &str) {
    db.conn
        .execute(
            "INSERT INTO runs(id, employee_id, lane, state, session, created_at, data) VALUES (?1, ?2, 'work', ?3, ?4, ?5, ?6)",
            params![id, e.id, state, session, ms(Utc::now()), data],
        )
        .unwrap();
}

/// A run's JSON with a kind this build does not know.
fn future_kind(e: &Employee, state: RunState) -> (String, String) {
    let mut v = serde_json::to_value(run(e, RunKind::Manual, state)).unwrap();
    v["kind"] = serde_json::json!({ "type": "someFutureKind", "detail": "from a newer build" });
    (v["id"].as_str().unwrap().to_string(), v.to_string())
}

fn ids(runs: &[Run]) -> BTreeSet<String> {
    runs.iter().map(|r| r.id.clone()).collect()
}

#[test]
fn the_opening_note_queries() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let at = |mins: i64| Utc::now() - chrono::Duration::minutes(mins);
    let insert = |mut r: Run, mins: i64, session: &str| {
        r.created_at = at(mins);
        if r.state.is_finished() {
            r.ended_at = Some(at(mins - 1));
        }
        assert!(db.insert_run(&r, session).unwrap());
        r
    };
    let old = insert(run(&ada, RunKind::Chat, RunState::Succeeded), 120, "chat-1");
    let newer = insert(run(&ada, RunKind::Chat, RunState::Failed), 60, "chat-1");
    let now_turn = insert(run(&ada, RunKind::Chat, RunState::Running), 5, "chat-1");
    let report = insert(run(&ada, RunKind::Manual, RunState::Succeeded), 30, "m1");
    insert(run(&ada, RunKind::Manual, RunState::Queued), 2, "m2");

    let found: Vec<(String, String)> = db.finished_since(&ada.id, at(90)).unwrap().into_iter().map(|r| (r.run.id, r.session)).collect();
    assert_eq!(found, vec![(report.id.clone(), "m1".to_string()), (newer.id.clone(), "chat-1".to_string())]);
    assert_eq!(db.finished_since(&ada.id, at(200)).unwrap().len(), 3);

    assert_eq!(db.last_turn_ended("chat-1", &now_turn.id).unwrap(), newer.ended_at);
    assert_eq!(db.last_turn_ended("chat-1", &newer.id).unwrap(), old.ended_at);
    assert_eq!(db.last_turn_ended("m2", "x").unwrap(), None);

    let choices = || CardAsk::Choices { choices: vec!["Not now".into(), "Go".into()], recommended: 0 };
    let open = insert(with_card(run(&ada, RunKind::Manual, RunState::Succeeded), choices(), false), 50, "c1");
    let chore = insert(with_card(run(&ada, RunKind::Manual, RunState::Failed), CardAsk::Chore { subject: "chrome".into(), url: None }, false), 40, "c2");
    insert(with_card(run(&ada, RunKind::Manual, RunState::Succeeded), choices(), true), 45, "c3");
    insert(with_card(run(&ada, RunKind::Manual, RunState::Running), choices(), false), 3, "c4");
    assert_eq!(db.open_cards(&ada.id).unwrap().iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![open.id, chore.id]);

    let asker = insert(run(&ada, RunKind::Manual, RunState::Succeeded), 100, "a1");
    insert(run(&ada, RunKind::Asked { parent_run_id: asker.id.clone(), asked_by: ada.id.clone(), task: false }, RunState::Succeeded), 20, "a2");
    insert(run(&ada, RunKind::Delegated { parent_run_id: "long-ago".into() }, RunState::Succeeded), 300, "a3");
    assert_eq!(db.parents_since(at(60)).unwrap(), HashSet::from([asker.id.clone()]));
    assert_eq!(db.parents_since(at(400)).unwrap(), HashSet::from([asker.id, "long-ago".to_string()]));

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unreadable_row_is_left_out() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let queued = run(&ada, RunKind::Manual, RunState::Queued);
    let mut done = run(&ada, RunKind::Manual, RunState::Succeeded);
    done.needs_you = vec!["Look at this.".into()];
    assert!(db.insert_run(&queued, &queued.id).unwrap());
    assert!(db.insert_run(&done, &done.id).unwrap());
    for state in [RunState::Queued, RunState::Succeeded] {
        let (id, data) = future_kind(&ada, state);
        raw(&db, &ada, &id, state.as_str(), &id, &data);
        let id = uuid::Uuid::new_v4().to_string();
        raw(&db, &ada, &id, state.as_str(), &id, "not json");
    }

    let queued_id = queued.id.clone();
    let both = ids(&[queued.clone(), done.clone()]);
    assert_eq!(ids(&db.recent_runs(None, 100).unwrap()), both);
    let records: Vec<Run> = db.records_in_state(RunState::Queued).unwrap().into_iter().map(|r| r.run).collect();
    assert_eq!(ids(&records), ids(std::slice::from_ref(&queued)));
    assert_eq!(ids(&db.runs_in_state(RunState::Queued).unwrap()), ids(&[queued]));
    assert_eq!(ids(&db.runs_in_state(RunState::Succeeded).unwrap()), ids(&[done.clone()]));
    let ada_only = std::slice::from_ref(&ada.id);
    assert_eq!(ids(&db.finished_root_runs(ada_only, ada_only).unwrap()), ids(&[done.clone()]));
    assert_eq!(ids(&db.reached_root_runs(ada_only, ada_only, 10).unwrap()), ids(&[done.clone()]));
    assert_eq!(ids(&db.open_cards(&ada.id).unwrap()), ids(&[]));
    assert!(db.open_topic_runs("chore:chrome").unwrap().is_empty());
    // The Inbox's other queries read the JSON in SQL too.
    assert!(db.reply_runs(None).unwrap().is_empty());
    assert!(db.open_tasks(&ada.id).unwrap().is_empty());
    assert!(db.archived_tasks(10).unwrap().is_empty());
    assert!(db.conversation_runs().unwrap().is_empty());
    assert!(db.open_asks().unwrap().is_empty());
    // So do the reply watch and Tidy.
    assert!(!db.has_run_for("req-1", RequestRun::Nudge).unwrap());
    assert!(db.active_run_for("req-1", RequestRun::Nudge).unwrap().is_none());
    assert!(db.tidy_runs(&ada.id).unwrap().is_empty());
    assert!(db.taught_since(&ada.id, None, 10).unwrap().is_empty());
    // And the hand-off queries.
    assert!(db.replaced_cards(&ada.id, Utc::now() - chrono::Duration::days(1)).unwrap().is_empty());
    raw(&db, &ada, "bad-handoff", "succeeded", "s-bad", "not json");
    db.conn.execute("UPDATE runs SET parent_run_id = ?1 WHERE id = 'bad-handoff'", [&queued_id]).unwrap();
    assert!(db.chain_handoffs("c1").unwrap().is_empty());
    assert!(db.noted_in("r1").unwrap().is_empty());
    assert_eq!(db.delegated_since(&ada.id, &ada.id, Utc::now() - chrono::Duration::days(1)).unwrap(), 0);
    assert_eq!(db.session_size("s-bad").unwrap().1, None);
    let now = ms(Utc::now());
    db.conn
        .execute(
            "INSERT INTO requests(id, person_id, employee_id, state, created_at, data) VALUES ('req-1', 'p1', ?1, 'open', ?2, 'not json')",
            params![ada.id, now],
        )
        .unwrap();
    assert!(db.watched_requests().unwrap().is_empty());
    assert!(db.requests_of_run(&queued_id).unwrap().is_empty());

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unreadable_running_rows_are_lost() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let (future, data) = future_kind(&ada, RunState::Running);
    raw(&db, &ada, &future, "running", "s1", &data);
    raw(&db, &ada, "corrupt", "running", "s2", "not json");

    let at = Utc::now();
    assert_eq!(db.lose_unreadable_running(at, "Stopped.").unwrap(), 2);
    let state = |id: &str| -> String { db.conn.query_row("SELECT state FROM runs WHERE id = ?", [id], |r| r.get(0)).unwrap() };
    assert_eq!(state(&future), "lost");
    assert_eq!(state("corrupt"), "lost");
    let (json_state, ended, error): (String, String, String) = db
        .conn
        .query_row(
            "SELECT json_extract(data, '$.state'), json_extract(data, '$.endedAt'), json_extract(data, '$.error') FROM runs WHERE id = ?",
            [&future],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(json_state, "lost");
    assert_eq!(ended.parse::<Ts>().unwrap(), at);
    assert_eq!(error, "Stopped.");
    let corrupt: String = db.conn.query_row("SELECT data FROM runs WHERE id = 'corrupt'", [], |r| r.get(0)).unwrap();
    assert_eq!(corrupt, "not json");

    for session in ["s1", "s2"] {
        let next = run(&ada, RunKind::Manual, RunState::Running);
        assert!(db.insert_run(&next, session).unwrap(), "a new run takes {session}");
    }

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_old_office_still_opens() {
    let dir = temp_dir();
    let path = dir.join("office.db");
    let old = Connection::open(&path).unwrap();
    old.execute_batch(
        r#"
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE employees(id TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, departed_at INTEGER, data TEXT NOT NULL);
CREATE TABLE people(id TEXT PRIMARY KEY, removed_at INTEGER, data TEXT NOT NULL);
CREATE TABLE schedules(id TEXT PRIMARY KEY, employee_id TEXT NOT NULL, last_slot INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE runs(
  id TEXT PRIMARY KEY,
  employee_id TEXT NOT NULL,
  lane TEXT NOT NULL,
  state TEXT NOT NULL,
  schedule_id TEXT,
  slot INTEGER,
  parent_run_id TEXT,
  request_id TEXT,
  session TEXT NOT NULL,
  inited INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  data TEXT NOT NULL,
  UNIQUE(schedule_id, slot)
);
CREATE UNIQUE INDEX runs_one_running_per_lane ON runs(employee_id, lane) WHERE state = 'running';
INSERT INTO meta(key, value) VALUES ('settings', '{"maxConcurrentRuns":2,"claudePath":null,"dataDir":"/office","paused":false}');
INSERT INTO employees(id, slug, data) VALUES ('e1', 'ada', '{"id":"e1","slug":"ada","name":"Ada","title":"Analyst","description":"","objective":"Keep the numbers right.","managerId":null,"skills":{"builtins":true,"keys":[]},"clearance":"supervised","allowRules":[],"model":null,"effort":null,"workspaces":["/office/employees/e1/workspace"],"hue":200,"chatSession":"chat-e1","hiredAt":"2026-09-20T09:00:00Z"}');
INSERT INTO runs(id, employee_id, lane, state, session, inited, created_at, data) VALUES ('r1', 'e1', 'work', 'succeeded', 'r1', 1, 1759050000000, '{"id":"r1","employeeId":"e1","kind":{"type":"manual"},"prompt":"Check the numbers.","state":"succeeded","createdAt":"2026-09-28T09:00:00Z","startedAt":"2026-09-28T09:00:01Z","endedAt":"2026-09-28T09:05:00Z","turns":4,"durationMs":299000,"costUsd":0.12,"summary":"All good.","error":null,"needsYou":[]}');
"#,
    )
    .unwrap();
    drop(old);

    let db = Store::open(&path).unwrap();
    let mut st = db.conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap();
    let tables: BTreeSet<String> = st.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    drop(st);
    for t in ["checks", "proposals", "skill_revisions", "requests", "news", "skill_use"] {
        assert!(tables.contains(t), "{t} created");
    }
    let lane_index: Option<String> =
        db.conn.query_row("SELECT name FROM sqlite_master WHERE name = 'runs_one_running_per_lane'", [], |r| r.get(0)).optional().unwrap();
    assert_eq!(lane_index, None);

    let r = db.run("r1").unwrap().unwrap().run;
    assert!(r.allow_once.is_empty() && r.denials.is_empty() && r.skill_changes.is_empty() && r.attachments.is_empty());
    assert_eq!(r.proposed, None);
    let e = db.employee("e1").unwrap().unwrap();
    assert!(e.browser);
    assert!(e.auto_tidy);
    let s = db.settings().unwrap().unwrap();
    assert_eq!(s.max_concurrent_runs, 2);
    assert_eq!(s.backup, BackupSettings::default());
    assert_eq!(s.watch, WatchSettings::default());
    assert_eq!((s.notifications.mode, s.notifications.quiet), (NotifyMode::Batched, None));

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn news_is_unread_until_marked() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let (ada, bo) = (employee("Ada", None), employee("Bo", None));
    let news = |text: &str, mins: i64| News {
        id: uuid::Uuid::new_v4().to_string(),
        employee_id: bo.id.clone(),
        from: ada.id.clone(),
        text: text.into(),
        created_at: Utc::now() - chrono::Duration::minutes(mins),
        read_at: None,
        read_in: None,
    };
    let (later, earlier) = (news("The till is counted.", 5), news("The shelf is counted.", 10));
    db.save_news(&later).unwrap();
    db.save_news(&earlier).unwrap();
    let unread: Vec<String> = db.unread_news(&bo.id).unwrap().into_iter().map(|n| n.text).collect();
    assert_eq!(unread, vec!["The shelf is counted.", "The till is counted."], "oldest first");
    assert!(db.unread_news(&ada.id).unwrap().is_empty());
    db.save_news(&News { read_at: Some(Utc::now()), read_in: Some("r1".into()), ..earlier.clone() }).unwrap();
    assert_eq!(db.unread_news(&bo.id).unwrap(), vec![later.clone()]);

    // The run that showed it never started, so the next one shows it again.
    assert_eq!(db.unread_news_of("r2").unwrap(), 0);
    assert_eq!(db.unread_news_of("r1").unwrap(), 1);
    assert_eq!(db.unread_news(&bo.id).unwrap(), vec![earlier, later]);

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn answered_cards_are_your_decisions_this_session_has_not_heard() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    let card = |question: &str, ask: CardAsk, how: AnswerHow, mins: i64, held: bool| {
        let mut c = Card::new(question, None, Some(format!("ask:{question}")), ask);
        c.answer = Some(CardAnswer { at: Utc::now() - chrono::Duration::minutes(mins), how, text: "Chose: Book.".into(), held });
        c
    };
    let choices = || CardAsk::Choices { choices: vec!["Book".into()], recommended: 0 };
    // This build first opened the office 25 minutes ago.
    db.set_meta("typed_answers_since", &(Utc::now() - chrono::Duration::minutes(25))).unwrap();
    let mut r = run(&ada, RunKind::Manual, RunState::Succeeded);
    r.cards = vec![
        card("Book the room?", choices(), AnswerHow::Tapped, 30, false),
        card("Order lunch?", choices(), AnswerHow::Rejected, 10, false),
        card("Which caterer?", choices(), AnswerHow::Typed, 20, false),
        card("Which florist?", choices(), AnswerHow::Typed, 40, false),
        card("Print menus?", choices(), AnswerHow::Dismissed, 5, false),
        card("Move chairs?", choices(), AnswerHow::Replaced, 5, false),
        card("Hang the sign?", choices(), AnswerHow::Withdrawn, 5, false),
        card("Count the till?", choices(), AnswerHow::Tapped, 2, true),
        card("Sign in?", CardAsk::Chore { subject: "sign-in:example.com".into(), url: None }, AnswerHow::Tapped, 2, false),
        card("Paint the wall?", choices(), AnswerHow::Tapped, 60 * 24 * 8, false),
    ];
    assert!(db.insert_run(&r, "s1").unwrap());
    let week = Utc::now() - chrono::Duration::days(7);
    let found: Vec<String> = db.answered_cards(&ada.id, week, "s2").unwrap().into_iter().map(|c| c.message).collect();
    assert_eq!(found, ["Order lunch?", "Which caterer?", "Book the room?"], "your taps, words and noes this week, newest first");
    assert!(!found.contains(&"Which florist?".to_string()), "words typed before this build may be an older build's reply put on every card");
    assert!(db.answered_cards(&ada.id, week, "s1").unwrap().is_empty(), "the session they came back to heard them");
    assert!(db.answered_cards(&employee("Bo", None).id, week, "s2").unwrap().is_empty());

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_skill_keeps_its_latest_use() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let (used, since) = db.skill_use("e1").unwrap();
    assert!(used.is_empty() && since <= Utc::now());
    let at = |days: i64| Utc::now() - chrono::Duration::days(days);
    let (three, one, five) = (at(3), at(1), at(5));
    let skills = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<HashSet<_>>();
    db.record_skill_use("e1", &skills(&["ada-invoices", "ada-payroll"]), three).unwrap();
    db.record_skill_use("e1", &skills(&["ada-invoices"]), one).unwrap();
    db.record_skill_use("e1", &skills(&["ada-payroll"]), five).unwrap();
    let mut used: Vec<(String, i64)> = db.skill_use("e1").unwrap().0.into_iter().map(|(s, t)| (s, t.timestamp_millis())).collect();
    used.sort();
    assert_eq!(used, [("ada-invoices".to_string(), one.timestamp_millis()), ("ada-payroll".to_string(), three.timestamp_millis())], "an older use never hides a newer one");
    assert!(db.skill_use("e2").unwrap().0.is_empty());
    drop(db);
    let db = Store::open(&dir.join("office.db")).unwrap();
    assert_eq!(db.skill_use("e1").unwrap().1, since, "counting began once");

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn taught_since_keeps_only_your_own_words() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let report = run(&ada, RunKind::Manual, RunState::Succeeded);
    db.insert_run(&report, &report.id).unwrap();
    let reply = |prompt: &str, omega_wrote: bool| {
        let mut r = Run::new(&ada.id, RunKind::Reply { to_run_id: report.id.clone(), label: "Manual run".into(), task: false, card: false }, prompt.into(), Utc::now());
        r.omega_wrote = omega_wrote;
        db.insert_run(&r, &r.id).unwrap();
    };
    // An approval from the app, and a reply of yours wrapped because the report's session never started.
    reply("Pat allowed the build for this retry.", true);
    reply("Your report (Manual run):\n\nIt failed.\n\nPat's reply:\n\nTry the other folder.", false);
    let came = RunKind::Answered { asked_run_id: "r1".into(), from: "e0".into(), task: false, chat: false, thread: Some(report.id.clone()) };
    let mut answer = Run::new(&ada.id, came, "Bo answered your question.".into(), Utc::now());
    answer.omega_wrote = true;
    db.insert_run(&answer, &report.id).unwrap();

    let taught: Vec<String> = db.taught_since(&ada.id, None, 10).unwrap().into_iter().map(|(_, t)| t).collect();
    assert_eq!(taught.len(), 1, "{taught:?}");
    assert!(taught[0].ends_with("Try the other folder."));

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

fn tables(db: &Store) -> BTreeSet<String> {
    let mut st = db.conn.prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'index')").unwrap();
    let v = st.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    v
}

#[test]
fn a_briefs_table_is_left_alone() {
    let dir = temp_dir();
    let path = dir.join("office.db");
    let db = Store::open(&path).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let done = run(&ada, RunKind::Manual, RunState::Succeeded);
    assert!(db.insert_run(&done, &done.id).unwrap());
    // As an older build left it.
    let brief = r#"{"id":"b1","employeeId":"e1","rootRunId":"r1","label":"Manual run","replies":1,"excerpt":"Done.","text":"Settled."}"#;
    db.conn
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS briefs(
  id TEXT PRIMARY KEY, employee_id TEXT NOT NULL, session TEXT NOT NULL, created_at INTEGER NOT NULL, consumed_at INTEGER, data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS briefs_by_session ON briefs(employee_id, session, created_at);",
        )
        .unwrap();
    db.conn.execute("INSERT INTO briefs(id, employee_id, session, created_at, data) VALUES ('b1', 'e1', 's1', 1, ?)", [brief]).unwrap();
    drop(db);

    let db = Store::open(&path).unwrap();
    assert_eq!(db.run(&done.id).unwrap().unwrap().run, done);
    assert_eq!(ids(&db.recent_runs(None, 10).unwrap()), ids(&[done]));
    let t = tables(&db);
    assert!(t.contains("briefs") && t.contains("briefs_by_session"), "the briefs table stays");
    let rows: Vec<(String, String)> = {
        let mut st = db.conn.prepare("SELECT id, data FROM briefs").unwrap();
        let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        v
    };
    assert_eq!(rows, vec![("b1".to_string(), brief.to_string())]);

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fresh_office_has_no_briefs_table() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let t = tables(&db);
    assert!(t.contains("runs"));
    assert!(!t.contains("briefs") && !t.contains("briefs_by_session"));

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

fn with_card(mut r: Run, ask: CardAsk, answered: bool) -> Run {
    let mut card = Card::new("Needs you.", None, Some("ask:boost".into()), ask);
    card.answer = answered.then(|| CardAnswer { at: Utc::now(), how: AnswerHow::Tapped, text: "Chose: Not now.".into(), held: false });
    r.cards = vec![card];
    r
}

#[test]
fn a_runs_one_card_becomes_its_list_once() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let old = |id: &str, card: serde_json::Value| {
        let mut v = serde_json::to_value(run(&ada, RunKind::Manual, RunState::Succeeded)).unwrap();
        v["id"] = id.into();
        v.as_object_mut().unwrap().remove("cards");
        v["card"] = card;
        raw(&db, &ada, id, "succeeded", id, &v.to_string());
    };
    old("chore", serde_json::json!({ "message": "Open Chrome.", "ask": { "type": "chore", "subject": "chrome" }, "answer": null }));
    old("choice", serde_json::json!({ "message": "Boost?", "ask": { "type": "choices", "choices": ["No", "Yes"], "recommended": 0 }, "answer": null }));
    old("none", serde_json::Value::Null);
    db.conn.execute("DELETE FROM meta WHERE key = 'cards_listed'", []).unwrap();
    db.migrate_cards().unwrap();
    let get = |id: &str| db.run(id).unwrap().unwrap().run;
    assert_eq!(get("chore").cards[0].key.as_deref(), Some("chore:chrome"), "a chore gets its key");
    assert_eq!(get("choice").cards.len(), 1);
    assert!(get("none").cards.is_empty());
    let raw_json: String = db.conn.query_row("SELECT data FROM runs WHERE id = 'choice'", [], |r| r.get(0)).unwrap();
    assert!(!raw_json.contains("\"card\""), "{raw_json}");
    assert_eq!(ids(&db.open_topic_runs("chore:chrome").unwrap()), ids(&[get("chore")]));
    assert_eq!(ids(&db.open_cards(&ada.id).unwrap()), ids(&[get("chore"), get("choice")]));
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn revisions_of_dependency_and_data_files_are_dropped_once() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let revise = |skill: &str, file: &str| {
        let change = SkillChange { skill: skill.into(), file: file.into(), change: ChangeKind::Added };
        db.insert_revision(&ada.id, &SkillRevision { change, id: uuid::Uuid::new_v4().to_string(), run_id: None, at: Utc::now(), before: None, after: Some("x".into()) }).unwrap();
    };
    let left_out = ["node_modules/left-pad/index.js", "scripts/__pycache__/a.pyc", ".venv/lib/x.py", ".git/HEAD", "data/pull.json", "log.JSONL", "rates.csv"];
    let kept = ["SKILL.md", "scripts/fetch.py", "examples.md", ".github/notes.md", "jsonfeed.md"];
    for f in left_out.iter().chain(&kept) {
        revise("ada-reports", f);
    }
    revise(crate::own_skills::NOTES, "rates.csv.md");
    db.conn.execute("DELETE FROM meta WHERE key = 'unrecorded_revisions_dropped'", []).unwrap();
    db.drop_unrecorded_revisions().unwrap();
    let files = || db.revisions(&ada.id).unwrap().into_iter().map(|r| r.change.file).collect::<BTreeSet<_>>();
    let mut want: BTreeSet<String> = kept.iter().map(|f| f.to_string()).collect();
    want.insert("rates.csv.md".into());
    assert_eq!(files(), want);
    revise("ada-reports", "late.json");
    db.drop_unrecorded_revisions().unwrap();
    assert!(files().contains("late.json"), "it runs once");
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_cards_are_found_by_chore_and_by_schedule() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let chrome = || CardAsk::Chore { subject: "chrome".into(), url: None };
    let choices = || CardAsk::Choices { choices: vec!["Not now".into(), "Go".into()], recommended: 0 };
    let insert = |r: Run| {
        assert!(db.insert_run(&r, &r.id).unwrap());
        r
    };
    let waiting = insert(with_card(run(&ada, RunKind::Manual, RunState::Failed), chrome(), false));
    insert(with_card(run(&ada, RunKind::Manual, RunState::Succeeded), chrome(), true));
    let held = insert(with_card(run(&ada, RunKind::Manual, RunState::Queued), chrome(), false));
    insert(with_card(run(&ada, RunKind::Manual, RunState::Running), chrome(), false));
    insert(with_card(run(&ada, RunKind::Chat, RunState::Succeeded), chrome(), false));
    insert(with_card(run(&ada, RunKind::Manual, RunState::Succeeded), CardAsk::Chore { subject: "sign-in:acme".into(), url: None }, false));
    insert(with_card(run(&ada, RunKind::Manual, RunState::Succeeded), choices(), false));
    assert_eq!(ids(&db.open_topic_runs("chore:chrome").unwrap()), ids(&[waiting, held]), "a run held for the usage limit still waits; a chat turn never does");

    let at = |mins: i64| Utc::now() - chrono::Duration::minutes(mins);
    let scheduled = |id: &str, mins: i64, ask: CardAsk, answered: bool| {
        let kind = RunKind::Scheduled { schedule_id: id.into(), slot: at(mins), label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
        let mut r = run(&ada, kind, RunState::Succeeded);
        r.created_at = at(mins);
        insert(with_card(r, ask, answered))
    };
    let older = scheduled("s1", 30, choices(), false);
    let items = CardAsk::Items { verb: "Post".into(), items: vec![CardItem { id: "d1".into(), text: "Draft".into(), about: None, recommended: true }] };
    let older_items = scheduled("s1", 20, items, false);
    scheduled("s1", 15, choices(), true);
    // Put off with Later, it still waits on you.
    let mut put_off = scheduled("s1", 14, choices(), true);
    put_off.cards[0].back_at = Some(at(-60));
    db.save_run(&put_off).unwrap();
    scheduled("s1", 12, chrome(), false);
    scheduled("s2", 10, choices(), false);
    let newer = scheduled("s1", 5, choices(), false);
    let found = db.open_cards_of_schedule("s1", newer.created_at).unwrap();
    assert_eq!(found.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![older.id, older_items.id, put_off.id]);

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cards_said_later_to_parked_and_withdrawn_are_found() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let (ada, bo) = (employee("Ada", None), employee("Bo", None));
    db.save_employee(&ada).unwrap();
    db.save_employee(&bo).unwrap();
    let now = Utc::now();
    let insert = |e: &Employee, state: RunState, mins: i64, f: &dyn Fn(&mut Card)| {
        let mut r = with_card(run(e, RunKind::Manual, state), CardAsk::Choices { choices: vec!["Go".into()], recommended: 0 }, true);
        r.created_at = now - chrono::Duration::minutes(mins);
        f(&mut r.cards[0]);
        assert!(db.insert_run(&r, &r.id).unwrap());
        r
    };
    // Due a moment ago, a fraction of a second from now, and tomorrow; one is due but still working.
    let due = insert(&ada, RunState::Succeeded, 30, &|c| c.back_at = Some(now - chrono::Duration::milliseconds(1500)));
    let just = insert(&ada, RunState::Succeeded, 20, &|c| c.back_at = Some(now + chrono::Duration::milliseconds(300)));
    let tomorrow = insert(&ada, RunState::Succeeded, 10, &|c| c.back_at = Some(now + chrono::Duration::days(1)));
    insert(&ada, RunState::Running, 5, &|c| c.back_at = Some(now - chrono::Duration::hours(1)));
    assert_eq!(ids(&db.cards_due_back(now).unwrap()), ids(std::slice::from_ref(&due)));
    assert_eq!(ids(&db.cards_due_back(now + chrono::Duration::seconds(1)).unwrap()), ids(&[due.clone(), just.clone()]));

    let older = insert(&ada, RunState::Succeeded, 40, &|c| c.parked = true);
    let newer = insert(&bo, RunState::Succeeded, 1, &|c| c.parked = true);
    // Put off or parked, a card still waits on you: a newer one about the same thing replaces it, and an answer reaches it.
    assert_eq!(ids(&db.open_topic_runs("ask:boost").unwrap()), ids(&[due, just, tomorrow, older.clone(), newer.clone()]));
    assert_eq!(db.parked_runs().unwrap().iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![newer.id.clone(), older.id.clone()], "newest first");
    db.depart_employee(&bo.id, now).unwrap();
    assert_eq!(ids(&db.parked_runs().unwrap()), ids(&[older]), "only people still here");

    let withdrawn = |mins: i64| move |c: &mut Card| c.answer = Some(CardAnswer { at: now - chrono::Duration::minutes(mins), how: AnswerHow::Withdrawn, text: "Settled.".into(), held: false });
    insert(&ada, RunState::Succeeded, 50, &withdrawn(30));
    assert!(!db.withdrew_since(&ada.id, "ask:boost", now - chrono::Duration::minutes(15)).unwrap(), "half an hour ago");
    insert(&ada, RunState::Succeeded, 50, &withdrawn(5));
    assert!(db.withdrew_since(&ada.id, "ask:boost", now - chrono::Duration::minutes(15)).unwrap());
    assert!(!db.withdrew_since(&bo.id, "ask:boost", now - chrono::Duration::minutes(15)).unwrap(), "someone else's");
    assert!(!db.withdrew_since(&ada.id, "ask:other", now - chrono::Duration::minutes(15)).unwrap());
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_due_check_is_run_once() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let now = Utc::now();
    let check = |state: CheckState, due_at: Ts| {
        let c = Check {
            id: uuid::Uuid::new_v4().to_string(),
            employee_id: ada.id.clone(),
            what: "Did the fix hold".into(),
            how: "Rerun the weekly report".into(),
            due_at,
            state,
            created_at: now - chrono::Duration::days(3),
            run_id: None,
            closed_at: None,
            result: None,
        };
        db.save_check(&c).unwrap();
        c
    };
    let hour = chrono::Duration::hours(1);
    let due = check(CheckState::Open, now - hour);
    check(CheckState::Open, now + hour);
    check(CheckState::Done, now - hour);
    check(CheckState::Dropped, now - hour);
    let to_run = |db: &Store| db.checks_to_run(now).unwrap().into_iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(to_run(&db), vec![due.id.clone()]);

    let kind = RunKind::Scheduled { schedule_id: format!("{CHECK_PREFIX}{}", due.id), slot: due.due_at, label: "Check: Did the fix hold".into(), quiet: true, model: None, effort: None, review: false };
    let first = Run::new(&ada.id, kind.clone(), "go".into(), now);
    assert!(db.insert_run(&first, &first.id).unwrap());
    assert!(to_run(&db).is_empty());
    let again = Run::new(&ada.id, kind, "go".into(), now);
    assert!(!db.insert_run(&again, &again.id).unwrap());

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

/// One of each kind of run. `i` makes its ids unique, since `insert_run` skips a second run of the
/// same schedule slot or follow-up.
fn every_kind(i: usize) -> Vec<RunKind> {
    let slot = Utc::now();
    let person = || "p1".to_string();
    vec![
        RunKind::Manual,
        RunKind::Chat,
        RunKind::Scheduled { schedule_id: format!("loud-{i}"), slot, label: "Sweep".into(), quiet: false, model: None, effort: None, review: false },
        RunKind::Scheduled { schedule_id: format!("quiet-{i}"), slot, label: "Sweep".into(), quiet: true, model: None, effort: None, review: false },
        RunKind::Scheduled { schedule_id: format!("review-{i}"), slot, label: "Weekly review".into(), quiet: false, model: None, effort: None, review: true },
        RunKind::Scheduled { schedule_id: format!("{CHECK_PREFIX}short-{i}"), slot, label: "Check: Did it ship".into(), quiet: true, model: None, effort: None, review: false },
        RunKind::Scheduled { schedule_id: format!("{CHECK_PREFIX}long-{i}"), slot, label: "Check: Did it ship".into(), quiet: true, model: None, effort: None, review: false },
        RunKind::Delegated { parent_run_id: format!("parent-{i}") },
        RunKind::Asked { parent_run_id: format!("parent-{i}"), asked_by: "e0".into(), task: false },
        RunKind::Answered { asked_run_id: format!("asked-{i}"), from: "e0".into(), task: false, chat: false, thread: None },
        RunKind::Answered { asked_run_id: format!("asked-chat-{i}"), from: "e0".into(), task: true, chat: true, thread: None },
        RunKind::Answered { asked_run_id: format!("asked-thread-{i}"), from: "e0".into(), task: false, chat: false, thread: Some(format!("task-{i}")) },
        RunKind::FollowUp { request_id: format!("request-{i}"), person_id: person() },
        RunKind::Organize { imported: vec![], settled: false },
        RunKind::Reply { to_run_id: format!("root-{i}"), label: "Manual run".into(), task: false, card: false },
        RunKind::Reply { to_run_id: format!("card-{i}"), label: "Sweep".into(), task: false, card: true },
        RunKind::Task { label: "Draft it".into(), from: None, context: String::new() },
        RunKind::HireDecided { proposal_id: format!("proposal-{i}"), approved: true },
        RunKind::Tidy { reason: TidyReason::Auto, signature: vec![], plan: None },
        RunKind::ReplyCheck { request_id: format!("request-{i}"), person_id: person() },
        RunKind::Nudge { request_id: format!("request-{i}"), person_id: person() },
        RunKind::Unanswered { request_id: format!("request-{i}"), person_id: person() },
    ]
}

#[test]
fn the_inbox_query_matches_reaches_you() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    let bo = employee("Bo", Some(&ada.id));
    db.save_employee(&ada).unwrap();
    db.save_employee(&bo).unwrap();
    let states = [RunState::Queued, RunState::Running, RunState::Succeeded, RunState::Failed, RunState::Cancelled, RunState::Lost];
    let denial = Denial { tool: "Bash".into(), brief: "rm -rf build".into(), suggested_rule: "Bash(rm:*)".into(), ..Default::default() };

    let mut all = vec![];
    let mut i = 0;
    for e in [&ada, &bo] {
        for state in states {
            for flag in [false, true] {
                for blocked in [false, true] {
                    i += 1;
                    for (j, kind) in every_kind(i).into_iter().enumerate() {
                        let mut r = run(e, kind, state);
                        // A due check's run that worked for long reports like other work.
                        if r.kind.check_id().is_some_and(|c| c.starts_with("long")) {
                            r.duration_ms = Some(threads::CHECK_QUIET_MS + 1);
                        }
                        if flag {
                            r.needs_you = vec!["Look at this.".into()];
                        }
                        if j % 2 == 0 {
                            r.replied_at = Some(Utc::now());
                        }
                        r.steered = (i + j) % 3 == 0;
                        if blocked {
                            r.denials = vec![denial.clone()];
                        }
                        assert!(db.insert_run(&r, &r.id).unwrap(), "inserted {:?}", r.kind);
                        all.push(r);
                    }
                }
            }
        }
    }

    let got = ids(&db.finished_root_runs(std::slice::from_ref(&ada.id), &[ada.id.clone(), bo.id.clone()]).unwrap());
    let want: BTreeSet<String> =
        all.iter().filter(|r| threads::reaches_you(r, false, r.employee_id == ada.id) && threads::could_need_you(r)).map(|r| r.id.clone()).collect();
    let differ: Vec<String> = all
        .iter()
        .filter(|r| got.contains(&r.id) != want.contains(&r.id))
        .map(|r| format!("{:?} {:?} needs_you {} denials {} top {}", r.kind, r.state, r.needs_you.len(), r.denials.len(), r.employee_id == ada.id))
        .collect();
    assert!(differ.is_empty(), "the query and reaches_you differ on {differ:#?}");
    assert!(!want.is_empty());

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_weekly_review_counts() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let (ada, bo) = (employee("Ada", None), employee("Bo", Some("x")));
    db.save_employee(&ada).unwrap();
    db.save_employee(&bo).unwrap();
    let at = |hours: i64| Utc::now() - chrono::Duration::hours(hours);
    let since = at(24 * 7);
    let insert = |e: &Employee, kind: RunKind, state: RunState, hours: i64| {
        let mut r = run(e, kind, state);
        r.created_at = at(hours);
        assert!(db.insert_run(&r, &r.id).unwrap());
        r
    };
    let scheduled = |id: &str, label: &str, quiet: bool, review: bool, hours: i64| RunKind::Scheduled {
        schedule_id: id.into(),
        slot: at(hours),
        label: label.into(),
        quiet,
        model: None,
        effort: None,
        review,
    };
    insert(&ada, scheduled("s1", "Sweep", true, false, 1), RunState::Succeeded, 1);
    let mut notified = run(&ada, scheduled("s1", "Sweep", true, false, 2), RunState::Succeeded);
    notified.created_at = at(2);
    notified.needs_you = vec!["Look at this.".into()];
    assert!(db.insert_run(&notified, &notified.id).unwrap());
    let asker = insert(&ada, scheduled("s1", "Sweep", true, false, 3), RunState::Succeeded, 3);
    insert(&bo, RunKind::Asked { parent_run_id: asker.id.clone(), asked_by: ada.id.clone(), task: false }, RunState::Succeeded, 3);
    insert(&ada, scheduled("s1", "Sweep", true, false, 4), RunState::Failed, 4);
    insert(&ada, scheduled("s1", "Sweep", true, false, 300), RunState::Succeeded, 300);
    insert(&ada, scheduled("s1", "Sweep", true, false, 0), RunState::Queued, 0);
    insert(&ada, scheduled("s2", "Post", false, false, 5), RunState::Succeeded, 5);
    insert(&ada, scheduled("r1", "Weekly review", false, true, 6), RunState::Succeeded, 6);
    insert(&ada, scheduled("check:c1", "Check: Did it hold", true, false, 7), RunState::Succeeded, 7);
    insert(&bo, scheduled("s3", "Bo's sweep", true, false, 8), RunState::Succeeded, 8);

    let counts: Vec<(String, bool, u32, u32, u32)> =
        db.routine_counts(Some(&ada.id), since).unwrap().into_iter().map(|c| (c.label, c.quiet, c.runs, c.reached, c.failed)).collect();
    assert_eq!(counts, vec![("Sweep".to_string(), true, 4, 2, 1), ("Post".to_string(), false, 1, 0, 0)]);

    let delegated = insert(&ada, RunKind::Delegated { parent_run_id: "p1".into() }, RunState::Succeeded, 10);
    let chat = insert(&ada, RunKind::Chat, RunState::Succeeded, 10);
    let tidy = insert(&ada, RunKind::Tidy { reason: TidyReason::Auto, signature: vec![], plan: None }, RunState::Succeeded, 10);
    let revise = |r: &Run, skill: &str, hours: i64| {
        let change = SkillChange { skill: skill.into(), file: "SKILL.md".into(), change: ChangeKind::Modified };
        let rev = SkillRevision { change, id: uuid::Uuid::new_v4().to_string(), run_id: Some(r.id.clone()), at: at(hours), before: None, after: Some("x".into()) };
        db.insert_revision(&ada.id, &rev).unwrap();
        rev.id
    };
    let older = revise(&delegated, "ops", 9);
    revise(&chat, "ops", 8);
    revise(&tidy, "ops", 7);
    revise(&delegated, "@company", 6);
    let newer = revise(&asker, "ops", 2);
    revise(&delegated, "ops", 400);
    let found: Vec<(String, String)> = db.unattended_revisions(&ada.id, since).unwrap().into_iter().map(|(rev, r)| (rev.id, r.id)).collect();
    assert_eq!(found, vec![(newer, asker.id.clone()), (older, delegated.id.clone())]);
    assert!(db.unattended_revisions(&bo.id, since).unwrap().is_empty());

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_conversation_with_you_is_found_while_it_runs() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let (ada, bo) = (employee("Ada", None), employee("Bo", None));
    db.save_employee(&ada).unwrap();
    db.save_employee(&bo).unwrap();
    let insert = |r: Run| assert!(db.insert_run(&r, &r.id).unwrap());
    let came = |chat: bool, thread: Option<String>| RunKind::Answered { asked_run_id: "r1".into(), from: bo.id.clone(), task: false, chat, thread };
    insert(run(&ada, came(true, None), RunState::Running));
    insert(run(&ada, came(false, Some("t1".into())), RunState::Running));
    insert(run(&ada, RunKind::Chat, RunState::Succeeded));
    insert(run(&ada, RunKind::Manual, RunState::Running));
    assert!(!db.ceo_conversation_running(None).unwrap(), "an answer coming back into a conversation is not you");
    insert(run(&bo, RunKind::Task { label: "Draft it".into(), from: None, context: String::new() }, RunState::Running));
    assert!(db.ceo_conversation_running(None).unwrap());
    assert!(db.ceo_conversation_running(Some(&bo.id)).unwrap());
    assert!(!db.ceo_conversation_running(Some(&ada.id)).unwrap());

    let start = Utc::now();
    let revise = |e: &Employee, skill: &str, file: &str, at| {
        let change = SkillChange { skill: skill.into(), file: file.into(), change: ChangeKind::Modified };
        let rev = SkillRevision { change, id: uuid::Uuid::new_v4().to_string(), run_id: None, at, before: None, after: Some("x".into()) };
        db.insert_revision(&e.id, &rev).unwrap();
    };
    revise(&ada, "@company", "CLAUDE.md", start - chrono::Duration::minutes(5));
    revise(&ada, "@own", "what-you-own.md", start + chrono::Duration::minutes(1));
    assert!(!db.revised_since("@company", "CLAUDE.md", None, start).unwrap(), "changed before the run started");
    assert!(db.revised_since("@own", "what-you-own.md", Some(&ada.id), start).unwrap());
    assert!(!db.revised_since("@own", "what-you-own.md", Some(&bo.id), start).unwrap());
    revise(&bo, "@company", "CLAUDE.md", start + chrono::Duration::minutes(2));
    assert!(db.revised_since("@company", "CLAUDE.md", None, start).unwrap());

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}


fn sweep(id: &str, label: &str, quiet: bool, review: bool, slot: Ts) -> RunKind {
    RunKind::Scheduled { schedule_id: id.into(), slot, label: label.into(), quiet, model: None, effort: None, review }
}

#[test]
fn usage_counts_what_runs_did_and_cost() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let (ada, bo) = (employee("Ada", None), employee("Bo", None));
    db.save_employee(&ada).unwrap();
    db.save_employee(&bo).unwrap();
    let at = |hours: i64| Utc::now() - chrono::Duration::hours(hours);
    let since = at(24);
    let insert = |e: &Employee, kind: RunKind, state: RunState, hours: i64, cost: Option<f64>, edit: &dyn Fn(&mut Run)| {
        let mut r = run(e, kind, state);
        r.created_at = at(hours);
        r.cost_usd = cost;
        edit(&mut r);
        assert!(db.insert_run(&r, &r.id).unwrap());
        r
    };
    insert(&ada, sweep("s1", "Sweep", true, false, at(1)), RunState::Succeeded, 1, Some(0.1), &|_| {});
    insert(&ada, sweep("s1", "Sweep", true, false, at(2)), RunState::Succeeded, 2, Some(0.2), &|r| {
        r.skill_changes = vec![SkillChange { skill: "ops".into(), file: "SKILL.md".into(), change: ChangeKind::Modified }];
    });
    insert(&ada, sweep("s1", "Sweep", true, false, at(3)), RunState::Failed, 3, None, &|_| {});
    let asker = insert(&ada, sweep("s1", "Sweep", true, false, at(4)), RunState::Succeeded, 4, Some(0.3), &|_| {});
    insert(&ada, sweep("s1", "Sweep", true, false, at(30)), RunState::Succeeded, 30, Some(5.0), &|_| {});
    insert(&ada, sweep("s1", "Sweep", true, false, at(0)), RunState::Queued, 0, None, &|_| {});
    insert(&ada, RunKind::Manual, RunState::Succeeded, 5, Some(1.0), &|r| {
        let ask = || CardAsk::Choices { choices: vec!["Go".into()], recommended: 0 };
        r.cards = vec![Card::new("Post it?", None, Some("ask:post".into()), ask()), Card::new("Boost it?", None, Some("ask:boost".into()), ask())];
    });
    insert(&ada, sweep("r1", "Weekly review", false, true, at(6)), RunState::Succeeded, 6, Some(0.4), &|_| {});
    insert(&bo, RunKind::Asked { parent_run_id: asker.id.clone(), asked_by: ada.id.clone(), task: false }, RunState::Succeeded, 4, Some(0.05), &|_| {});
    insert(&bo, sweep("s2", "Digest", false, false, at(7)), RunState::Succeeded, 7, None, &|_| {});

    let round = |c: f64| (c * 100.0).round() / 100.0;
    let routines: Vec<(String, String, u32, u32, u32, u32, f64)> = db
        .routine_counts(None, since)
        .unwrap()
        .into_iter()
        .map(|c| (c.schedule_id, c.employee_id, c.runs, c.reached, c.failed, c.useful, round(c.cost_usd)))
        .collect();
    assert_eq!(
        routines,
        vec![("s1".to_string(), ada.id.clone(), 4, 1, 1, 3, 0.6), ("s2".to_string(), bo.id.clone(), 1, 0, 0, 0, 0.0)],
        "the review is not a routine; the hand-off, the skill change and the failure did something"
    );
    assert_eq!(db.routine_counts(Some(&bo.id), since).unwrap().len(), 1);

    let usage: Vec<(String, u32, u32, u32, u32, f64)> =
        db.employee_usage(since).unwrap().into_iter().map(|u| (u.employee_id, u.runs, u.decisions, u.handoffs, u.failed, round(u.cost_usd))).collect();
    let of = |e: &Employee| usage.iter().find(|u| u.0 == e.id).cloned();
    assert_eq!(of(&ada), Some((ada.id.clone(), 6, 2, 1, 1, 2.0)), "finished runs since then, everything they did and cost");
    assert_eq!(of(&bo), Some((bo.id.clone(), 2, 0, 0, 0, 0.05)));
    assert_eq!(usage.len(), 2);

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_finds_runs_by_their_words_newest_first() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let (ada, bo, cy) = (employee("Ada", None), employee("Bo", None), employee("Cy", None));
    for e in [&ada, &bo, &cy] {
        db.save_employee(e).unwrap();
    }
    let insert = |e: &Employee, days: i64, edit: &dyn Fn(&mut Run)| {
        let mut r = run(e, RunKind::Manual, RunState::Succeeded);
        r.created_at = Utc::now() - chrono::Duration::days(days);
        edit(&mut r);
        assert!(db.insert_run(&r, &r.id).unwrap());
        r.id
    };
    let asked = insert(&ada, 90, &|r| r.prompt = "Send the Quarterly Invoice to Acme.".into());
    let reported = insert(&bo, 30, &|r| r.summary = Some("Done.\n\nThe INVOICE is paid.".into()));
    let carded = insert(&ada, 2, &|r| {
        r.cards = vec![Card::new("Chase the invoice with Acme?", None, Some("ask:chase".into()), CardAsk::Choices { choices: vec!["Chase".into()], recommended: 0 })];
    });
    insert(&ada, 1, &|r| r.summary = Some("Nothing new in the inbox.".into()));
    insert(&cy, 1, &|r| r.prompt = "Find the invoice.".into());
    db.depart_employee(&cy.id, Utc::now()).unwrap();
    raw(&db, &ada, "broken", "succeeded", "broken", "not json, an invoice");

    let found = |q: &str, limit: u32| db.search_runs(q, limit).unwrap().into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(found("invoice", 10), vec![carded.clone(), reported.clone(), asked], "newest first; a departed employee's are left out");
    assert_eq!(found("  Invoice ", 2), vec![carded, reported]);
    assert!(found("refund", 10).is_empty());
    assert!(found("  ", 10).is_empty());

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_latest_number_and_report_are_found() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let t = Utc::now();
    let hours = |h: i64| t - chrono::Duration::hours(h);
    let schedule = |id: &str, review: bool| Schedule {
        id: id.into(),
        employee_id: ada.id.clone(),
        label: if review { "Weekly review".into() } else { "Sweep".into() },
        cron: "0 7 * * 1".into(),
        instruction: "Go.".into(),
        enabled: true,
        misfire: Misfire::CatchUp,
        quiet: !review,
        model: None,
        effort: None,
        review,
        active_hours: false,
        next_fire: None,
    };
    db.save_schedule(&schedule("r1", true), t).unwrap();
    db.save_schedule(&schedule("s1", false), t).unwrap();
    let insert = |kind: RunKind, h: i64, edit: &dyn Fn(&mut Run)| {
        let mut r = run(&ada, kind, RunState::Succeeded);
        r.created_at = hours(h);
        r.ended_at = Some(hours(h));
        edit(&mut r);
        assert!(db.insert_run(&r, &r.id).unwrap());
        r.id
    };
    assert_eq!(db.latest_score(&ada.id).unwrap(), None);
    let score = |value: &str| Score { metric: "Signups".into(), value: value.into(), change: String::new(), note: String::new() };
    insert(sweep("r1", "Weekly review", false, true, hours(24 * 14)), 24 * 14, &|r| r.score = Some(score("30")));
    insert(sweep("r1", "Weekly review", false, true, hours(24 * 7)), 24 * 7, &|r| r.score = Some(score("42")));
    insert(sweep("r1", "Weekly review", false, true, hours(2)), 2, &|_| {});
    assert_eq!(db.latest_score(&ada.id).unwrap(), Some(score("42")), "the newest review that recorded one");

    let since = hours(24 * 30);
    let report = insert(RunKind::Manual, 10, &|r| r.summary = Some("Paused both campaigns.".into()));
    insert(sweep("s1", "Sweep", true, false, hours(5)), 5, &|r| r.summary = Some("Nothing new.".into()));
    insert(RunKind::Task { label: "Plan".into(), from: None, context: String::new() }, 4, &|r| r.summary = Some("Sure.".into()));
    insert(RunKind::ReplyCheck { request_id: "q1".into(), person_id: "p1".into() }, 3, &|r| r.summary = Some("No reply yet.".into()));
    insert(RunKind::Manual, 1, &|r| r.summary = Some("  ".into()));
    assert_eq!(db.last_report(&ada.id, since).unwrap().map(|r| r.id), Some(report), "the newest that says something to you");
    let raised = insert(sweep("s1", "Sweep", true, false, hours(0)), 0, &|r| {
        r.summary = Some("Found a refund request.".into());
        r.needs_you = vec!["Refund it?".into()];
    });
    assert_eq!(db.last_report(&ada.id, since).unwrap().map(|r| r.id), Some(raised), "a quiet run that raised something");
    assert_eq!(db.last_report(&ada.id, t + chrono::Duration::hours(1)).unwrap(), None);

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runs_about_a_request_are_found_by_kind() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let ada = employee("Ada", None);
    db.save_employee(&ada).unwrap();
    let nudge = run(&ada, RunKind::Nudge { request_id: "q1".into(), person_id: "p1".into() }, RunState::Queued);
    let check = run(&ada, RunKind::ReplyCheck { request_id: "q1".into(), person_id: "p1".into() }, RunState::Succeeded);
    let unanswered = run(&ada, RunKind::Unanswered { request_id: "q2".into(), person_id: "p1".into() }, RunState::Running);
    for r in [&nudge, &check, &unanswered] {
        assert!(db.insert_run(r, &r.id).unwrap());
    }

    assert_eq!(db.active_run_for("q1", RequestRun::Nudge).unwrap().map(|r| r.id), Some(nudge.id));
    assert!(db.has_run_for("q1", RequestRun::Nudge).unwrap());
    assert!(db.active_run_for("q1", RequestRun::ReplyCheck).unwrap().is_none(), "a finished check is not active");
    assert!(db.has_run_for("q1", RequestRun::ReplyCheck).unwrap(), "but it was made");
    assert!(!db.has_run_for("q1", RequestRun::Unanswered).unwrap());
    assert_eq!(db.active_run_for("q2", RequestRun::Unanswered).unwrap().map(|r| r.id), Some(unanswered.id));
    assert!(!db.has_run_for("q2", RequestRun::Nudge).unwrap(), "another request's kind");

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn each_request_run_tag_is_its_run_kinds_type() {
    for (run, kind) in [
        (RequestRun::ReplyCheck, RunKind::ReplyCheck { request_id: "q1".into(), person_id: "p1".into() }),
        (RequestRun::Nudge, RunKind::Nudge { request_id: "q1".into(), person_id: "p1".into() }),
        (RequestRun::Unanswered, RunKind::Unanswered { request_id: "q1".into(), person_id: "p1".into() }),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap()["type"], run.tag());
    }
}

#[test]
fn sql_fragments_match_the_rules_they_mirror() {
    for state in [RunState::Queued, RunState::Running, RunState::Succeeded, RunState::Failed, RunState::Cancelled, RunState::Lost] {
        assert_eq!(active!().contains(&format!("'{}'", state.as_str())), !state.is_finished(), "{state:?}");
    }
    assert_eq!(check_run!(), format!("'{CHECK_PREFIX}%'"));
}

#[test]
fn team_stats_counts_finished_work_per_day_and_keeps_who_left() {
    let dir = temp_dir();
    let db = Store::open(&dir.join("office.db")).unwrap();
    let (ada, bo) = (employee("Ada", None), employee("Bo", None));
    for e in [&ada, &bo] {
        db.save_employee(e).unwrap();
    }
    db.depart_employee(&bo.id, Utc::now()).unwrap();
    let insert = |e: &Employee, days: i64, state: RunState, ms: u64| {
        let mut r = run(e, RunKind::Manual, state);
        r.created_at = Utc::now() - chrono::Duration::days(days);
        r.duration_ms = Some(ms);
        assert!(db.insert_run(&r, &r.id).unwrap());
    };
    insert(&ada, 0, RunState::Succeeded, 60_000);
    insert(&ada, 0, RunState::Failed, 30_000);
    insert(&ada, 0, RunState::Running, 0);
    insert(&ada, 3, RunState::Succeeded, 1_000);
    insert(&bo, 0, RunState::Succeeded, 5_000);

    let s = db.team_stats().unwrap();
    assert_eq!(s.staff.len(), 2, "everyone ever hired, so headcount can be drawn for any day");
    assert!(s.staff.iter().find(|x| x.employee_id == bo.id).unwrap().departed_at.is_some());
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let ada_today = s.days.iter().find(|d| d.employee_id == ada.id && d.day == today).unwrap();
    assert_eq!((ada_today.runs, ada_today.work_ms), (2, 90_000), "finished runs only, failures counted");
    assert_eq!(s.days.iter().filter(|d| d.employee_id == ada.id).count(), 2, "one row per day worked");
    assert!(s.days.iter().any(|d| d.employee_id == bo.id), "work by someone who left still counts");

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}
