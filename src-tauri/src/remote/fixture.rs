//! An invented office for the remote tests and the `remote_e2e` harness: Pat's company Acme, Ada the tester, Sam the
//! bookkeeper, and one Inbox item of each kind the phone acts on. Held for an hour, so nothing it queues starts.

use super::api::{self, Kind, Ops, Request};
use crate::fixtures::{acme, employee, employee_named, open_office, person, temp_dir};
use crate::model::*;
use crate::office::Office;
use crate::store::Store;
use base64::Engine;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

/// A 1 by 1 PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

/// The invented office, and the ids of what is in it.
pub struct Fixture {
    pub dir: PathBuf,
    pub office: Office,
    pub ada: String,
    /// Ada asks "Order the spare parts?" with a choice card that shows a file and an image.
    pub carded: String,
    /// Ada was blocked from publishing the menu.
    pub blocked: String,
    /// Ada's tidy proposes a change.
    pub tidy: String,
    /// Ada's run that failed.
    pub failed: String,
    /// Ada's finished run with nothing open.
    pub done: String,
    /// Ada's open proposals to hire Cy and Dee.
    pub proposals: [String; 2],
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A finished run of `e`'s, in `state`.
fn finished(e: &Employee, kind: RunKind, prompt: &str, state: RunState) -> Run {
    let mut r = Run::new(&e.id, kind, prompt.into(), chrono::Utc::now());
    r.state = state;
    r.ended_at = Some(r.created_at);
    r.summary = (state == RunState::Succeeded).then(|| format!("{} did it.", e.name));
    r
}

/// An open proposal of `from`'s to hire `name`.
fn proposal(from: &Employee, name: &str) -> HireProposal {
    HireProposal {
        id: uuid::Uuid::new_v4().to_string(),
        employee_id: from.id.clone(),
        run_id: None,
        reason: "Design work piles up.".into(),
        draft: employee_named(name).draft(),
        missing_skills: vec![],
        state: ProposalState::Open,
        note: None,
        hired_id: None,
        created_at: chrono::Utc::now(),
        resolved_at: None,
    }
}

impl Fixture {
    /// The invented office in a new folder named after `tag`.
    pub async fn new(tag: &str) -> Fixture {
        let dir = fs::canonicalize(temp_dir(tag)).unwrap();
        let png = base64::engine::general_purpose::STANDARD.decode(PNG).unwrap();
        let image = format!("data:image/png;base64,{PNG}");
        fs::write(dir.join("parts.png"), &png).unwrap();
        fs::write(dir.join("order.md"), "Two belts and a pump.").unwrap();

        let ada = Employee { image: Some(image.clone()), hue: 200, ..employee() };
        let db = Store::open(&dir.join("office.db")).unwrap();
        db.set_held_until(Some(chrono::Utc::now() + chrono::Duration::hours(1))).unwrap();
        db.set_company(&Company { image: Some(image), ..acme() }).unwrap();
        db.save_employee(&ada).unwrap();
        db.save_person(&person()).unwrap();

        let mut carded = finished(&ada, RunKind::Manual, "Check the supplier site.", RunState::Succeeded);
        let mut card = Card::new("Order the spare parts?", Some("The pump is wearing out.".into()), None, CardAsk::Choices { choices: vec!["Not now".into(), "Order".into()], recommended: 1 });
        card.file = Some(dir.join("order.md").display().to_string());
        card.images = vec![dir.join("parts.png").display().to_string()];
        card.brief = vec![BriefRow { label: "The part".into(), text: "A new seal kit.".into(), shows: Some(Shows::Images) }];
        carded.needs_you = vec!["Order the spare parts?".into()];
        carded.cards = vec![card];

        let mut blocked = finished(&ada, RunKind::Manual, "Publish the menu.", RunState::Succeeded);
        blocked.denials = vec![Denial { tool: "Bash".into(), brief: "npm publish".into(), suggested_rule: "Bash(npm publish)".into() }];

        let mut tidy = finished(&ada, RunKind::Tidy { reason: TidyReason::Auto, signature: vec![], plan: None }, "Tidy your skills.", RunState::Succeeded);
        tidy.proposed = Some("Split the long pricing skill in two.".into());
        tidy.needs_you = vec!["Proposes bigger changes that need your OK.".into()];

        let mut failed = finished(&ada, RunKind::Manual, "Count the stock.", RunState::Failed);
        failed.error = Some("The stock sheet was locked.".into());
        let done = finished(&ada, RunKind::Manual, "Water the plants.", RunState::Succeeded);

        for r in [&carded, &blocked, &tidy, &failed, &done] {
            assert!(db.insert_run(r, &r.id).unwrap());
        }
        let proposals = [proposal(&ada, "Cy"), proposal(&ada, "Dee")];
        for p in &proposals {
            db.save_proposal(p).unwrap();
        }
        drop(db);

        Fixture {
            office: open_office(&dir).await,
            dir,
            ada: ada.id,
            carded: carded.id,
            blocked: blocked.id,
            tidy: tidy.id,
            failed: failed.id,
            done: done.id,
            proposals: proposals.map(|p| p.id),
        }
    }
}

/// The shape of a JSON value: each scalar replaced by its type's name, and each array by the merged shape of its items.
fn shape(v: &Value) -> Value {
    match v {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("boolean"),
        Value::Number(_) => json!("number"),
        Value::String(_) => json!("string"),
        Value::Array(items) => Value::Array(items.iter().map(shape).reduce(merge).into_iter().collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), shape(v))).collect()),
    }
}

/// Two shapes as one: objects with every key of both, and differing scalars as `a|b`.
fn merge(a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v) in b {
                let merged = match a.remove(&k) {
                    Some(old) => merge(old, v),
                    None => v,
                };
                a.insert(k, merged);
            }
            Value::Object(a)
        }
        (Value::Array(a), Value::Array(b)) => Value::Array(a.into_iter().chain(b).reduce(merge).into_iter().collect()),
        (a, b) if a == b => a,
        (Value::String(a), Value::String(b)) => {
            let mut names: Vec<&str> = a.split('|').chain(b.split('|')).collect();
            names.sort_unstable();
            names.dedup();
            json!(names.join("|"))
        }
        (Value::String(n), other) | (other, Value::String(n)) if n == "null" => other,
        (a, _) => a,
    }
}

/// The shape of every remote command's result in `f`: reads first, each write once, then `team` again, once the writes
/// have given an employee work, merged into its first shape.
pub fn shapes(f: &Fixture) -> Value {
    let calls = [
        ("inbox", json!({})),
        ("proposals", json!({})),
        ("team", json!({})),
        ("member_image", json!({ "id": f.ada })),
        ("thread", json!({ "runId": f.carded })),
        ("card_file", json!({ "runId": f.carded, "card": 0 })),
        ("card_image", json!({ "runId": f.carded, "card": 0, "index": 0 })),
        ("start_task", json!({ "employeeId": f.ada, "text": "Draft the spring menu." })),
        ("answer_card", json!({ "runId": f.carded, "card": 0, "answer": { "type": "choice", "index": 1 } })),
        ("reply_to_run", json!({ "runId": f.done, "text": "Thanks.", "archive": false })),
        ("approve_blocked", json!({ "runId": f.blocked, "rules": ["Bash(npm publish)"], "always": false })),
        ("approve_tidy", json!({ "runId": f.tidy })),
        ("approve_proposal", json!({ "proposalId": f.proposals[0], "note": "Welcome aboard." })),
        ("decline_proposal", json!({ "proposalId": f.proposals[1], "note": null })),
        ("set_archived", json!({ "runId": f.failed, "archived": true })),
        ("unarchive_runs", json!({ "runIds": [f.failed] })),
        ("retry_run", json!({ "runId": f.failed })),
        ("team", json!({})),
    ];
    let mut ops = Ops::default();
    let mut out = Map::new();
    for (i, (cmd, args)) in calls.into_iter().enumerate() {
        let kind = api::check(api::Room::Paired, cmd).unwrap();
        let op = (kind == Kind::Write).then(|| format!("00000000-0000-4000-8000-{i:012}"));
        let req = Request { id: i as u64, cmd: cmd.into(), args, op };
        let ret = api::run(&f.office, &mut ops, kind, &req, Instant::now()).unwrap_or_else(|e| panic!("{cmd}: {}", e.message));
        let shaped = match out.remove(cmd) {
            Some(before) => merge(before, shape(&ret)),
            None => shape(&ret),
        };
        out.insert(cmd.into(), shaped);
    }
    Value::Object(out)
}
