//! The remote API (section 9 of `docs/mobile-companion.md`): the only commands a phone may send, and the `match` that
//! runs each through the same `Office` methods the desktop's own commands use (`commands.rs`).

use super::wire;
use crate::error::{AppError, ErrorCode};
use crate::model::{CardReply, ProposalState};
use crate::office::Office;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The largest sealed frame the relay forwards.
pub const MAX_FRAME: usize = 4 * 1024 * 1024;
/// How many write results are kept for a resent `op`.
const OPS_KEPT: usize = 200;
/// How long a write result is kept for a resent `op`.
const OPS_FOR: Duration = Duration::from_secs(600);

const READS: [&str; 7] = ["inbox", "proposals", "team", "member_image", "thread", "card_file", "card_image"];
const WRITES: [&str; 9] =
    ["answer_card", "reply_to_run", "approve_blocked", "approve_tidy", "approve_proposal", "decline_proposal", "set_archived", "unarchive_runs", "retry_run"];

/// The room a request came in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Room {
    /// The one-use room of the QR code, where only `pair` is answered.
    Pairing,
    /// The long-term room of a paired phone.
    Paired,
}

/// The session commands, which the connection answers itself (`remote::Actor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Session {
    Pair,
    PushRegister,
    Unpair,
}

/// What kind of command a request is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Read,
    /// Carries an `op`, so a resent one is answered from the saved result.
    Write,
    Session(Session),
}

/// The remote layer's own error codes, beside the Office's.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCode {
    /// The command is not in the list.
    NotAllowed,
    BadRequest,
    /// The result would not fit in one sealed frame.
    TooLarge,
}

/// An error code as the phone reads it: the Office's (`ErrorCode`) or the remote layer's.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Code {
    Office(ErrorCode),
    Remote(RemoteCode),
}

/// Why a request failed, in the shape of the desktop's `ApiError`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Failure {
    pub code: Code,
    pub message: String,
}

impl Failure {
    /// A failure with one of the remote layer's codes.
    pub fn remote(code: RemoteCode, message: impl Into<String>) -> Failure {
        Failure { code: Code::Remote(code), message: message.into() }
    }
}

impl From<AppError> for Failure {
    fn from(e: AppError) -> Failure {
        Failure { code: Code::Office(e.code), message: e.message }
    }
}

/// What a request is answered with.
pub type Answer = Result<Value, Failure>;

/// A request from the phone.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: u64,
    pub cmd: String,
    pub args: Value,
    pub op: Option<String>,
}

/// The request in an opened frame. `None` when it is not one, or has no `id` to answer to, and is then ignored.
pub fn request(plain: &[u8]) -> Option<Request> {
    let v: Value = serde_json::from_slice(plain).ok()?;
    if v["t"] != "req" {
        return None;
    }
    Some(Request {
        id: v["id"].as_u64()?,
        cmd: v["cmd"].as_str().unwrap_or_default().into(),
        args: v["args"].clone(),
        op: v["op"].as_str().map(String::from),
    })
}

/// What kind of command `cmd` is, if a phone may send it in `room`.
pub fn check(room: Room, cmd: &str) -> Result<Kind, Failure> {
    let kind = match cmd {
        "pair" => Some(Kind::Session(Session::Pair)),
        "push_register" => Some(Kind::Session(Session::PushRegister)),
        "unpair" => Some(Kind::Session(Session::Unpair)),
        c if READS.contains(&c) => Some(Kind::Read),
        c if WRITES.contains(&c) => Some(Kind::Write),
        _ => None,
    };
    match (room, kind) {
        (Room::Pairing, Some(k @ Kind::Session(Session::Pair))) => Ok(k),
        (Room::Paired, Some(k)) if k != Kind::Session(Session::Pair) => Ok(k),
        (Room::Pairing, _) => Err(Failure::remote(RemoteCode::NotAllowed, "Only pairing is allowed here.")),
        _ => Err(Failure::remote(RemoteCode::NotAllowed, format!("'{cmd}' is not a remote command."))),
    }
}

/// The arguments of `req` as `T`.
pub fn args<T: DeserializeOwned>(req: &Request) -> Result<T, Failure> {
    let args = if req.args.is_null() { json!({}) } else { req.args.clone() };
    serde_json::from_value(args).map_err(|e| Failure::remote(RemoteCode::BadRequest, format!("Bad arguments for {}: {e}", req.cmd)))
}

/// The results of recent writes, by `op`: the last `OPS_KEPT`, for `OPS_FOR`.
#[derive(Default)]
pub struct Ops {
    saved: VecDeque<(String, Instant, Answer)>,
}

impl Ops {
    /// The saved answer to `op`, if it is still kept at `now`.
    fn get(&mut self, op: &str, now: Instant) -> Option<Answer> {
        self.saved.retain(|(_, at, _)| now.duration_since(*at) < OPS_FOR);
        self.saved.iter().find(|(o, _, _)| o == op).map(|(_, _, a)| a.clone())
    }

    /// Whether `op` has a saved answer at `now`, so a request carrying it is a resend.
    pub fn has(&self, op: &str, now: Instant) -> bool {
        self.saved.iter().any(|(o, at, _)| o == op && now.duration_since(*at) < OPS_FOR)
    }

    /// Keeps `answer` to `op` from `now`.
    fn put(&mut self, op: &str, now: Instant, answer: Answer) {
        if self.saved.len() == OPS_KEPT {
            self.saved.pop_front();
        }
        self.saved.push_back((op.into(), now, answer));
    }
}

/// Answers a read or write in the paired room. A write runs once per `op`, a UUID: a resent one gets the saved
/// answer, which is `too_large` when the result would not fit in a frame.
pub fn run(office: &Office, ops: &mut Ops, kind: Kind, req: &Request, now: Instant) -> Answer {
    if kind != Kind::Write {
        return call(office, req);
    }
    let Some(op) = req.op.as_deref().filter(|o| uuid::Uuid::parse_str(o).is_ok()) else {
        return Err(Failure::remote(RemoteCode::BadRequest, format!("{} needs an op, a UUID.", req.cmd)));
    };
    if let Some(saved) = ops.get(op, now) {
        return saved;
    }
    let mut answer = call(office, req);
    if !fits(&res(req.id, &answer)) {
        answer = Err(too_large());
    }
    ops.put(op, now, answer.clone());
    answer
}

/// `runId`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunArgs {
    run_id: String,
}

/// `runId` and `card`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardArgs {
    run_id: String,
    card: usize,
}

/// `runId`, `card` and `index`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardImageArgs {
    run_id: String,
    card: usize,
    index: usize,
}

/// A member's `id`.
#[derive(Deserialize)]
struct MemberArgs {
    id: String,
}

/// `answer_card`'s arguments.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerArgs {
    run_id: String,
    card: usize,
    answer: CardReply,
}

/// `reply_to_run`'s arguments: never attachments.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplyArgs {
    run_id: String,
    text: String,
    archive: bool,
}

/// `approve_blocked`'s arguments.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockedArgs {
    run_id: String,
    rules: Vec<String>,
    always: bool,
}

/// A hire proposal decision.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProposalArgs {
    proposal_id: String,
    #[serde(default)]
    note: Option<String>,
}

/// `set_archived`'s arguments.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveArgs {
    run_id: String,
    archived: bool,
}

/// `unarchive_runs`'s arguments.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunIdsArgs {
    run_ids: Vec<String>,
}

/// The result of `T` as JSON.
fn ok<T: Serialize>(result: crate::error::Result<T>) -> Answer {
    Ok(serde_json::to_value(result?).map_err(AppError::from)?)
}

/// Runs a read or write on the Office, as the desktop's command of the same name does.
pub fn call(office: &Office, req: &Request) -> Answer {
    match req.cmd.as_str() {
        "inbox" => ok(office.inbox()),
        "proposals" => ok(office.proposals().map(|ps| ps.into_iter().filter(|p| p.state == ProposalState::Open).collect::<Vec<_>>())),
        "team" => ok(office.members()),
        "member_image" => ok(office.member_image(&args::<MemberArgs>(req)?.id)),
        "thread" => ok(office.thread(&args::<RunArgs>(req)?.run_id)),
        "card_file" => {
            let a: CardArgs = args(req)?;
            ok(office.card_file(&a.run_id, a.card))
        }
        "card_image" => {
            let a: CardImageArgs = args(req)?;
            ok(office.card_image(&a.run_id, a.card, a.index))
        }
        "answer_card" => {
            let a: AnswerArgs = args(req)?;
            ok(office.answer_card(&a.run_id, a.card, a.answer))
        }
        "reply_to_run" => {
            let a: ReplyArgs = args(req)?;
            ok(office.reply_to_run(&a.run_id, &a.text, a.archive, vec![]))
        }
        "approve_blocked" => {
            let a: BlockedArgs = args(req)?;
            ok(office.approve_blocked(&a.run_id, a.rules, a.always))
        }
        "approve_tidy" => ok(office.approve_tidy(&args::<RunArgs>(req)?.run_id)),
        "approve_proposal" => {
            let a: ProposalArgs = args(req)?;
            ok(office.approve_proposal_as_proposed(&a.proposal_id, a.note))
        }
        "decline_proposal" => {
            let a: ProposalArgs = args(req)?;
            ok(office.decline_proposal(&a.proposal_id, a.note))
        }
        "set_archived" => {
            let a: ArchiveArgs = args(req)?;
            ok(office.set_archived(&a.run_id, a.archived))
        }
        "unarchive_runs" => ok(office.unarchive(&args::<RunIdsArgs>(req)?.run_ids)),
        "retry_run" => ok(office.retry_run(&args::<RunArgs>(req)?.run_id)),
        cmd => Err(Failure::remote(RemoteCode::NotAllowed, format!("'{cmd}' is not a remote command."))),
    }
}

/// The response to request `id`, as the JSON to seal. A result too large for one sealed frame is answered
/// `too_large` instead.
pub fn response(id: u64, answer: &Answer) -> Vec<u8> {
    let bytes = res(id, answer);
    if fits(&bytes) { bytes } else { res(id, &Err(too_large())) }
}

/// The response to request `id`, whatever its size.
fn res(id: u64, answer: &Answer) -> Vec<u8> {
    let json = match answer {
        Ok(ret) => json!({ "t": "res", "id": id, "ok": true, "ret": ret }),
        Err(err) => json!({ "t": "res", "id": id, "ok": false, "err": err }),
    };
    serde_json::to_vec(&json).unwrap_or_default()
}

/// Whether `response` fits in one sealed frame.
fn fits(response: &[u8]) -> bool {
    wire::sealed_len(response.len()) <= MAX_FRAME
}

/// The answer to a result too large for one sealed frame.
fn too_large() -> Failure {
    Failure::remote(RemoteCode::TooLarge, "That is too large to send to the phone. Open it on the computer.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::fixture::{self, Fixture};

    fn req(id: u64, cmd: &str, args: Value, op: Option<&str>) -> Request {
        Request { id, cmd: cmd.into(), args, op: op.map(String::from) }
    }

    fn code(f: Failure) -> Code {
        f.code
    }

    const NOT_ALLOWED: Code = Code::Remote(RemoteCode::NotAllowed);
    const OP: &str = "0b5c6d3e-8f1a-4b2c-9d7e-6a5f4e3d2c1b";

    #[test]
    fn every_command_outside_the_list_is_refused() {
        for cmd in ["", "conversations", "org", "settings", "update_settings", "run_now", "hire", "let_go", "read_image", "open_path", "employee_files", "Inbox", " inbox"] {
            assert_eq!(check(Room::Paired, cmd).map_err(code), Err(NOT_ALLOWED), "{cmd}");
        }
        for cmd in READS {
            assert_eq!(check(Room::Paired, cmd), Ok(Kind::Read));
        }
        for cmd in WRITES {
            assert_eq!(check(Room::Paired, cmd), Ok(Kind::Write));
        }
        assert_eq!(check(Room::Paired, "push_register"), Ok(Kind::Session(Session::PushRegister)));
        assert_eq!(check(Room::Paired, "unpair"), Ok(Kind::Session(Session::Unpair)));
    }

    #[test]
    fn pair_is_only_allowed_in_the_pairing_room_and_nothing_else_is() {
        assert_eq!(check(Room::Paired, "pair").map_err(code), Err(NOT_ALLOWED));
        assert_eq!(check(Room::Pairing, "pair"), Ok(Kind::Session(Session::Pair)));
        for cmd in READS.iter().chain(&WRITES).chain(&["push_register", "unpair", "hire"]) {
            assert_eq!(check(Room::Pairing, cmd).map_err(code), Err(NOT_ALLOWED), "{cmd}");
        }
    }

    #[test]
    fn requests_are_read_tolerantly() {
        let r = request(br#"{"t":"req","id":7,"cmd":"thread","args":{"runId":"r1"},"extra":1}"#).unwrap();
        assert_eq!(r, req(7, "thread", json!({"runId": "r1"}), None));
        assert!(request(br#"{"t":"evt","id":7}"#).is_none());
        assert!(request(br#"{"t":"req","cmd":"inbox"}"#).is_none(), "no id to answer to");
        assert!(request(b"not json").is_none());
    }

    #[tokio::test]
    async fn bad_arguments_and_writes_without_an_op_are_bad_requests() {
        let f = Fixture::new("remote-api-bad").await;
        let mut ops = Ops::default();
        let now = Instant::now();
        let bad = run(&f.office, &mut ops, Kind::Read, &req(1, "thread", json!({"run": "x"}), None), now).unwrap_err();
        assert_eq!(bad.code, Code::Remote(RemoteCode::BadRequest));
        for op in [None, Some(""), Some("op-1")] {
            let no_op = run(&f.office, &mut ops, Kind::Write, &req(2, "retry_run", json!({"runId": f.failed}), op), now).unwrap_err();
            assert_eq!(no_op.code, Code::Remote(RemoteCode::BadRequest), "{op:?}");
        }
        let missing = run(&f.office, &mut ops, Kind::Read, &req(3, "thread", json!({"runId": "nope"}), None), now).unwrap_err();
        assert_eq!(serde_json::to_value(&missing).unwrap()["code"], "notFound", "the Office's own codes keep their names");
    }

    #[tokio::test]
    async fn a_resent_op_gets_the_saved_answer_without_running_again() {
        let f = Fixture::new("remote-api-ops").await;
        let mut ops = Ops::default();
        let now = Instant::now();
        let archive = req(1, "set_archived", json!({"runId": f.done, "archived": true}), Some(OP));
        let first = run(&f.office, &mut ops, Kind::Write, &archive, now).unwrap();
        f.office.unarchive(std::slice::from_ref(&f.done)).unwrap();

        let again = run(&f.office, &mut ops, Kind::Write, &archive, now + Duration::from_secs(60)).unwrap();
        assert_eq!(again, first);
        assert!(f.office.run(&f.done).unwrap().archived_at.is_none(), "the resent op did not run again");

        let later = run(&f.office, &mut ops, Kind::Write, &archive, now + OPS_FOR + Duration::from_secs(1)).unwrap();
        assert_eq!(later, first);
        assert!(f.office.run(&f.done).unwrap().archived_at.is_some(), "after ten minutes the op runs again");
    }

    #[tokio::test]
    async fn a_write_whose_result_is_too_large_is_kept_as_too_large() {
        let f = Fixture::new("remote-api-big").await;
        let mut ops = Ops::default();
        let now = Instant::now();
        let reply = req(1, "reply_to_run", json!({"runId": f.done, "text": "x".repeat(MAX_FRAME), "archive": false}), Some(OP));
        let first = run(&f.office, &mut ops, Kind::Write, &reply, now).unwrap_err();
        assert_eq!(first.code, Code::Remote(RemoteCode::TooLarge));
        assert_eq!(ops.get(OP, now), Some(Err(first)), "the resent op hears too_large, not a result it cannot take");
    }

    #[test]
    fn only_the_last_200_ops_are_kept() {
        let mut ops = Ops::default();
        let now = Instant::now();
        for i in 0..=OPS_KEPT {
            ops.put(&i.to_string(), now, Ok(json!(i)));
        }
        assert!(ops.get("0", now).is_none());
        assert_eq!(ops.get("1", now), Some(Ok(json!(1))));
    }

    #[test]
    fn a_result_too_large_for_one_frame_is_answered_too_large() {
        let small: Value = serde_json::from_slice(&response(3, &Ok(json!("hi")))).unwrap();
        assert_eq!(small, json!({"t": "res", "id": 3, "ok": true, "ret": "hi"}));
        let big: Value = serde_json::from_slice(&response(4, &Ok(json!("x".repeat(MAX_FRAME))))).unwrap();
        assert_eq!(big["ok"], false);
        assert_eq!(big["err"]["code"], "too_large");
        let refused: Value = serde_json::from_slice(&response(5, &Err(Failure::remote(RemoteCode::NotAllowed, "no")))).unwrap();
        assert_eq!(refused["err"], json!({"code": "not_allowed", "message": "no"}));
    }

    #[tokio::test]
    async fn the_shape_of_every_remote_commands_result_matches_the_snapshot() {
        let f = Fixture::new("remote-api-shapes").await;
        let shapes = fixture::shapes(&f);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/remote/shapes.json");
        let text = serde_json::to_string_pretty(&shapes).unwrap() + "\n";
        if std::env::var_os("UPDATE_SHAPES").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        assert_eq!(
            text,
            std::fs::read_to_string(&path).unwrap(),
            "a remote command's result changed shape: if the phone can live with it, rerun with UPDATE_SHAPES=1 and \
             raise `wire::API` when a field it reads was removed or changed"
        );
    }
}
