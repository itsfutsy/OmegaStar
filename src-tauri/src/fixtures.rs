//! Test fixtures shared across the crate's test modules. Tests change what they need with struct update syntax.

use crate::model::{Build, Clearance, Company, Employee, HumanRequest, Person, PersonManager, Run, RunEvent, RunKind, RunState, SkillGrant};
use crate::office::{EventSink, Office};
use crate::store::Store;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A sink for an Office in tests, which drops every event.
pub struct NoSink;

impl EventSink for NoSink {
    fn org_changed(&self) {}
    fn run_updated(&self, _run: &Run) {}
    fn run_events(&self, _run_id: &str, _events: &[RunEvent]) {}
    fn request_updated(&self, _request: &HumanRequest) {}
}

/// Ada, a tester who reports to you, with a fresh id and nothing granted.
pub fn employee() -> Employee {
    Employee {
        id: uuid::Uuid::new_v4().to_string(),
        slug: "ada".into(),
        name: "Ada".into(),
        title: "Tester".into(),
        description: String::new(),
        objective: String::new(),
        manager_id: None,
        skills: SkillGrant::default(),
        clearance: Clearance::Supervised,
        allow_rules: vec![],
        model: None,
        effort: None,
        browser: false,
        auto_tidy: false,
        workspaces: vec![],
        hue: 0,
        image: None,
        chat_session: "chat-ada".into(),
        hired_at: chrono::Utc::now(),
        desk: false,
    }
}

/// A tester like Ada, called `name`.
pub fn employee_named(name: &str) -> Employee {
    Employee { slug: name.to_lowercase(), name: name.into(), chat_session: format!("chat-{name}"), ..employee() }
}

/// Sam, a bookkeeper who reports to you, with a fresh id.
pub fn person() -> Person {
    Person {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Sam".into(),
        title: "Bookkeeper".into(),
        description: String::new(),
        strengths: String::new(),
        responsibilities: String::new(),
        channels: vec![],
        notes: String::new(),
        manager: PersonManager::You,
        hue: 0,
        image: None,
        added_at: chrono::Utc::now(),
    }
}

/// A slot of the "Inbox sweep" schedule `id`, due now.
pub fn scheduled(id: &str, quiet: bool) -> RunKind {
    RunKind::Scheduled { schedule_id: id.into(), slot: chrono::Utc::now(), label: "Inbox sweep".into(), quiet, model: None, effort: None, review: false }
}

/// Pat's company, Acme.
pub fn acme() -> Company {
    Company { name: "Acme".into(), ceo_name: "Pat".into(), ..Company::unset() }
}

/// A task's kind.
pub fn task(label: &str) -> RunKind {
    RunKind::Task { label: label.into(), from: None, context: String::new() }
}

/// A path of its own for one test's folder, named after `tag`, not made yet.
pub fn temp_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("omegastar-{tag}-{}", uuid::Uuid::new_v4()))
}

/// A new, empty folder for one test, named after `tag`.
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = temp_path(tag);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The office kept in `dir`, telling no one what changes.
pub async fn open_office(dir: &Path) -> Office {
    Office::open(dir.to_path_buf(), Arc::new(NoSink), Build::new("0.0.0".into())).await.unwrap()
}

/// Saves a run of `e` in `state`, created `ago` minutes back, in `session` (its own id when None). A finished run
/// ended then, and a successful one reports "<name> did it."
pub fn saved_run(db: &Store, e: &Employee, kind: RunKind, prompt: &str, state: RunState, ago: i64, session: Option<&str>) -> Run {
    let at = chrono::Utc::now() - chrono::Duration::minutes(ago);
    let mut r = Run::new(&e.id, kind, prompt.into(), at);
    r.state = state;
    if state.is_finished() {
        r.ended_at = Some(at);
        r.summary = (state == RunState::Succeeded).then(|| format!("{} did it.", e.name));
    }
    let session = session.map(String::from).unwrap_or_else(|| r.id.clone());
    assert!(db.insert_run(&r, &session).unwrap());
    r
}
