//! The phone link: pairing, the connection to the relay, and phone sessions (section 12 of
//! `docs/mobile-companion.md`). `Remote` is a handle of channels, made before the office; `start` runs its task once
//! the office is open, and the task alone owns the connection, the saved pairing and the session keys.

pub mod api;
#[cfg(test)]
mod fixture;
pub mod push;
pub mod store;
#[cfg(test)]
mod tests;
pub mod wire;

use crate::error::{AppError, Result};
use crate::log::log;
use crate::model::Ts;
use crate::office::Office;
use api::{Kind, Ops, Request, Room, Session};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use store::{Pairing, PushTarget};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// The relay every build talks to, unless `OMEGASTAR_RELAY` names another (host, or host:port).
pub const RELAY: &str = "relay.omegastar.app";
/// How long a pairing code works.
const PAIR_FOR: Duration = Duration::from_secs(5 * 60);
const PING_EVERY: Duration = Duration::from_secs(30);
const PONG_WITHIN: Duration = Duration::from_secs(10);
const CONNECT_WITHIN: Duration = Duration::from_secs(20);
const BACKOFF_FIRST: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// A connection that lasted this long starts the backoff over.
const STEADY: Duration = Duration::from_secs(30);
/// `changed` events go out at most this often, combined.
const CHANGED_EVERY: Duration = Duration::from_secs(1);
/// The longest phone name kept, in characters.
const MAX_DEVICE_NAME: usize = 60;
const UNREACHABLE: &str = "OmegaStar cannot reach the phone relay.";
const NO_LINK: &str = "This copy of OmegaStar cannot link a phone.";

/// The relay to use: `OMEGASTAR_RELAY` when set, else `RELAY`.
pub fn relay() -> String {
    std::env::var("OMEGASTAR_RELAY").ok().filter(|r| !r.is_empty()).unwrap_or_else(|| RELAY.into())
}

/// A relay on this computer, reached without TLS.
fn is_local(relay: &str) -> bool {
    let host = relay.split(':').next().unwrap_or(relay);
    host == "localhost" || host == "127.0.0.1"
}

/// The relay's room address, for the computer's side.
fn room_url(relay: &str) -> String {
    format!("{}://{relay}/v1/room?role=laptop", if is_local(relay) { "ws" } else { "wss" })
}

/// The relay's push route.
fn push_url(relay: &str) -> String {
    format!("{}://{relay}/v1/push", if is_local(relay) { "http" } else { "https" })
}

/// Where the phone link stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// No phone is paired and none is pairing.
    #[default]
    Off,
    /// A pairing code is showing.
    Pairing,
    /// Paired, and reaching for the relay.
    Connecting,
    /// In the room, with no phone there.
    Online,
    /// A phone is connected.
    Session,
}

/// The phone link, as Settings shows it (`remote_status` and `remote:updated`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: State,
    pub device_name: Option<String>,
    pub paired_at: Option<Ts>,
    pub last_seen_at: Option<Ts>,
    pub push_on: bool,
    pub error: Option<String>,
}

impl Default for Status {
    fn default() -> Status {
        Status { state: State::Off, device_name: None, paired_at: None, last_seen_at: None, push_on: true, error: None }
    }
}

/// A pairing code: the QR code as a data URL, the link it holds, and when it stops working.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairCode {
    pub qr: String,
    pub link: String,
    pub expires_at: Ts,
}

/// What a `changed` event tells the phone to fetch again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum What {
    Inbox,
    Proposals,
    Team,
}

impl What {
    const ALL: [What; 3] = [What::Inbox, What::Proposals, What::Team];

    /// Its bit in a set of changes.
    fn bit(self) -> u8 {
        1 << self as u8
    }

    /// Its name in the event.
    fn name(self) -> &'static str {
        match self {
            What::Inbox => "inbox",
            What::Proposals => "proposals",
            What::Team => "team",
        }
    }
}

/// What the handle asks of the task.
enum Cmd {
    PairStart(oneshot::Sender<Result<PairCode>>),
    PairCancel(oneshot::Sender<()>),
    Unpair(oneshot::Sender<Result<()>>),
    SetPush(bool, oneshot::Sender<Result<Status>>),
    Changed(What),
    Push(String),
    /// The relay said this push token no longer works.
    TokenGone(String),
}

/// What the handle and the task share.
struct Shared {
    tx: mpsc::UnboundedSender<Cmd>,
    /// Taken by `start`.
    rx: Mutex<Option<mpsc::UnboundedReceiver<Cmd>>>,
    status: watch::Sender<Status>,
    relay: String,
}

/// The handle to the phone link. Cheap to clone; until `start`, only channels.
#[derive(Clone)]
pub struct Remote(Arc<Shared>);

impl Remote {
    /// A handle that will talk to `relay` (host, or host:port) when started.
    pub fn new(relay: String) -> Remote {
        let (tx, rx) = mpsc::unbounded_channel();
        let (status, _) = watch::channel(Status::default());
        Remote(Arc::new(Shared { tx, rx: Mutex::new(Some(rx)), status, relay }))
    }

    /// Starts the link's task for `office`, connecting if a phone is paired. Never in a sandbox, where every request
    /// then fails.
    pub fn start(&self, office: Office) {
        let Some(rx) = self.0.rx.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        if office.is_sandbox() {
            log!("phone link: off in a sandbox office");
            return;
        }
        let dir = office.data_dir().to_path_buf();
        let actor = Actor {
            shared: self.0.clone(),
            pairing: store::load(&dir),
            dir,
            rx,
            client: crate::proxy::client(),
            pair: None,
            ops: Ops::default(),
            keys: None,
            link: State::Connecting,
            error: None,
            pending: 0,
            office: office.clone(),
        };
        actor.publish();
        office.spawn(actor.run());
    }

    /// Where the link stands now.
    pub fn status(&self) -> Status {
        self.0.status.borrow().clone()
    }

    /// Every change of status from now on.
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.0.status.subscribe()
    }

    /// Sends the task `cmd`, made around a reply channel, and waits for its answer.
    async fn ask<T>(&self, cmd: impl FnOnce(oneshot::Sender<T>) -> Cmd) -> Result<T> {
        let (reply, answer) = oneshot::channel();
        self.0.tx.send(cmd(reply)).map_err(|_| AppError::conflict(NO_LINK))?;
        answer.await.map_err(|_| AppError::conflict(NO_LINK))
    }

    /// Opens a pairing room and returns its code. Replaces any code still showing.
    pub async fn pair_start(&self) -> Result<PairCode> {
        self.ask(Cmd::PairStart).await?
    }

    /// Leaves the pairing room; its code stops working.
    pub async fn pair_cancel(&self) -> Result<()> {
        self.ask(Cmd::PairCancel).await
    }

    /// Forgets the paired phone, telling it first if it is connected.
    pub async fn unpair(&self) -> Result<()> {
        self.ask(Cmd::Unpair).await?
    }

    /// Turns phone pushes on or off.
    pub async fn set_push(&self, on: bool) -> Result<Status> {
        self.ask(|reply| Cmd::SetPush(on, reply)).await?
    }

    /// Tells a connected phone that `what` changed. Nothing without a phone session.
    pub fn changed(&self, what: What) {
        if self.0.status.borrow().state == State::Session {
            let _ = self.0.tx.send(Cmd::Changed(what));
        }
    }

    /// Sends the paired phone a push titled `title`, when pushes are on.
    pub fn push(&self, title: &str) {
        let paired = {
            let s = self.0.status.borrow();
            s.device_name.is_some() && s.push_on
        };
        if paired {
            let _ = self.0.tx.send(Cmd::Push(title.into()));
        }
    }
}

/// The one-use room of a pairing code.
struct PairRoom {
    relay: String,
    room: String,
    key: [u8; 32],
    deadline: Instant,
}

/// The room the task belongs in, and its key.
#[derive(Clone, PartialEq)]
struct Target {
    relay: String,
    room: String,
    key: [u8; 32],
    kind: Room,
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Sink = futures_util::stream::SplitSink<Ws, Message>;

/// The link's task: owns the saved pairing, the pairing room, the connection and the session.
struct Actor {
    shared: Arc<Shared>,
    office: Office,
    dir: PathBuf,
    rx: mpsc::UnboundedReceiver<Cmd>,
    client: reqwest::Client,
    pairing: Option<Pairing>,
    pair: Option<PairRoom>,
    ops: Ops,
    /// The live phone session's keys.
    keys: Option<wire::Keys>,
    /// The connection's own state, shown while paired: connecting, online or session.
    link: State,
    error: Option<String>,
    /// What changed since the last `changed` event (`What::bit`).
    pending: u8,
}

/// Waits until `at`, or forever when there is no `at`.
async fn until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at.into()).await,
        None => std::future::pending().await,
    }
}

/// Joins `target`'s room at the relay as the computer.
async fn connect(target: &Target) -> std::result::Result<Ws, String> {
    let mut req = room_url(&target.relay).into_client_request().map_err(|e| e.to_string())?;
    let bearer = format!("Bearer {}", target.room).parse().map_err(|_| "the room id is not a valid header".to_string())?;
    req.headers_mut().insert(AUTHORIZATION, bearer);
    let config = WebSocketConfig::default().max_message_size(Some(api::MAX_FRAME)).max_frame_size(Some(api::MAX_FRAME));
    match tokio::time::timeout(CONNECT_WITHIN, tokio_tungstenite::connect_async_with_config(req, Some(config), true)).await {
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("timed out".into()),
    }
}

/// The QR code for `link`, as an SVG data URL.
fn qr_data_url(link: &str) -> Result<String> {
    let code = qrcode::QrCode::new(link.as_bytes()).map_err(|e| AppError::internal(e.to_string()))?;
    let svg = code.render::<qrcode::render::svg::Color>().min_dimensions(240, 240).build();
    Ok(format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg)))
}

/// The arguments of `pair`. The phone's platform is sent too, and not kept.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairArgs {
    device_name: String,
}

/// A relay control frame.
#[derive(Deserialize)]
struct Peer {
    t: String,
    state: String,
}

impl Actor {
    /// Runs the link for as long as the app does: in the room it belongs in while there is one, reconnecting with
    /// backoff, and otherwise waiting for a command.
    async fn run(mut self) {
        let mut backoff = Backoff::default();
        loop {
            let Some(target) = self.target() else {
                backoff = Backoff::default();
                self.error = None;
                self.publish();
                match self.rx.recv().await {
                    Some(cmd) => self.handle(cmd),
                    None => return,
                }
                continue;
            };
            self.link = State::Connecting;
            self.publish();
            let ws = match self.meanwhile(&target, connect(&target)).await {
                None => continue,
                Some(Ok(ws)) => ws,
                Some(Err(e)) => {
                    log!("phone link: cannot reach the relay: {e}");
                    self.error = Some(UNREACHABLE.into());
                    self.publish();
                    if self.meanwhile(&target, tokio::time::sleep(backoff.next())).await.is_none() {
                        backoff = Backoff::default();
                    }
                    continue;
                }
            };
            self.error = None;
            let since = Instant::now();
            self.serve(ws, &target).await;
            self.keys = None;
            self.pending = 0;
            if since.elapsed() >= STEADY {
                backoff = Backoff::default();
            }
            if self.target().as_ref() == Some(&target) {
                self.link = State::Connecting;
                self.publish();
                self.meanwhile(&target, tokio::time::sleep(backoff.next())).await;
            }
        }
    }

    /// The room the task belongs in: the pairing room while a code is showing, else the paired phone's.
    fn target(&self) -> Option<Target> {
        if let Some(p) = &self.pair {
            return Some(Target { relay: p.relay.clone(), room: p.room.clone(), key: p.key, kind: Room::Pairing });
        }
        let p = self.pairing.as_ref()?;
        Some(Target { relay: p.relay.clone(), room: p.room_id.clone(), key: wire::decode32(&p.key)?, kind: Room::Paired })
    }

    /// When the pairing code stops working, if one is showing.
    fn pair_deadline(&self) -> Option<Instant> {
        self.pair.as_ref().map(|p| p.deadline)
    }

    /// Waits for `fut` while answering commands. `None` when the task no longer belongs in `target`'s room.
    async fn meanwhile<T>(&mut self, target: &Target, fut: impl Future<Output = T>) -> Option<T> {
        tokio::pin!(fut);
        loop {
            let deadline = self.pair_deadline();
            tokio::select! {
                out = &mut fut => return Some(out),
                Some(cmd) = self.rx.recv() => self.handle(cmd),
                _ = until(deadline) => self.pair = None,
            }
            if self.target().as_ref() != Some(target) {
                return None;
            }
        }
    }

    /// Serves one connection to `target`'s room until it drops or the task belongs elsewhere.
    async fn serve(&mut self, ws: Ws, target: &Target) {
        let (mut tx, mut rx) = ws.split();
        self.link = State::Online;
        self.publish();
        let start = tokio::time::Instant::now();
        let mut ping = tokio::time::interval_at(start + PING_EVERY, PING_EVERY);
        let mut flush = tokio::time::interval(CHANGED_EVERY);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut pong_by: Option<Instant> = None;
        loop {
            let deadline = self.pair_deadline();
            let flushing = self.keys.is_some() && self.pending != 0;
            let alive = tokio::select! {
                msg = rx.next() => match msg {
                    Some(Ok(Message::Pong(_))) => {
                        pong_by = None;
                        true
                    }
                    Some(Ok(msg)) => self.frame(msg, &mut tx, target).await,
                    Some(Err(e)) => {
                        log!("phone link: the connection dropped: {e}");
                        false
                    }
                    None => false,
                },
                _ = ping.tick() => {
                    pong_by.get_or_insert(Instant::now() + PONG_WITHIN);
                    tx.send(Message::Ping(Default::default())).await.is_ok()
                }
                _ = until(pong_by) => {
                    log!("phone link: the relay stopped answering");
                    false
                }
                Some(cmd) = self.rx.recv() => {
                    let told = !matches!(cmd, Cmd::Unpair(_)) || self.event(&mut tx, json!({ "t": "evt", "name": "unpaired" })).await;
                    self.handle(cmd);
                    told
                }
                _ = until(deadline) => {
                    self.pair = None;
                    true
                }
                _ = flush.tick(), if flushing => {
                    let what: Vec<&str> = What::ALL.iter().filter(|w| self.pending & w.bit() != 0).map(|w| w.name()).collect();
                    self.pending = 0;
                    self.event(&mut tx, json!({ "t": "evt", "name": "changed", "what": what })).await
                }
            };
            if !alive {
                return;
            }
            if self.target().as_ref() != Some(target) {
                let _ = tx.send(Message::Close(None)).await;
                return;
            }
        }
    }

    /// Seals `event` to the phone, if a session is live. False when the connection is gone.
    async fn event(&mut self, tx: &mut Sink, event: Value) -> bool {
        let Some(keys) = self.keys.as_mut() else { return true };
        let frame = keys.seal(event.to_string().as_bytes());
        tx.send(Message::Binary(frame.into())).await.is_ok()
    }

    /// Handles one message from the relay in `target`'s room. False when the connection is gone.
    async fn frame(&mut self, msg: Message, tx: &mut Sink, target: &Target) -> bool {
        let bytes = match msg {
            Message::Text(text) => {
                if let Ok(peer) = serde_json::from_str::<Peer>(&text) {
                    if peer.t == "peer" && peer.state == "left" {
                        self.end_session();
                    }
                }
                return true;
            }
            Message::Binary(bytes) => bytes,
            Message::Close(_) => return false,
            _ => return true,
        };
        match bytes.first() {
            Some(&wire::HELLO) => {
                let answer = wire::answer_hello(&bytes[1..], &target.key, &target.room, self.office.version(), &wire::random_bytes());
                match answer {
                    None => true,
                    Some(wire::Answer::Refuse { frame }) => {
                        self.end_session();
                        tx.send(Message::Binary(frame.into())).await.is_ok()
                    }
                    Some(wire::Answer::Start { frame, keys }) => {
                        if tx.send(Message::Binary(frame.into())).await.is_err() {
                            return false;
                        }
                        self.start_session(*keys, target);
                        true
                    }
                }
            }
            Some(&wire::SEALED) => {
                let Some(keys) = self.keys.as_mut() else { return true };
                let Some(plain) = keys.open(&bytes) else {
                    log!("phone link: a frame did not open; the session ends");
                    self.end_session();
                    return true;
                };
                let Some(req) = api::request(&plain) else { return true };
                let answer = self.answer(target, &req);
                let Some(keys) = self.keys.as_mut() else { return true };
                let frame = keys.seal(&api::response(req.id, &answer));
                tx.send(Message::Binary(frame.into())).await.is_ok()
            }
            _ => true,
        }
    }

    /// Starts a phone session with `keys`, discarding any before it.
    fn start_session(&mut self, keys: wire::Keys, target: &Target) {
        self.keys = Some(keys);
        self.pending = 0;
        self.link = State::Session;
        if target.kind == Room::Paired {
            if let Some(p) = self.pairing.as_mut() {
                p.last_seen_at = Some(chrono::Utc::now());
                let p = p.clone();
                self.save(&p);
            }
        }
        self.publish();
    }

    /// Ends the phone session; the connection stays.
    fn end_session(&mut self) {
        self.keys = None;
        self.pending = 0;
        self.link = State::Online;
        self.publish();
    }

    /// Answers a phone's request in `target`'s room.
    fn answer(&mut self, target: &Target, req: &Request) -> api::Answer {
        let kind = api::check(target.kind, &req.cmd)?;
        match kind {
            Kind::Session(Session::Pair) => self.paired(target, api::args(req)?),
            Kind::Session(Session::PushRegister) => {
                let push: PushTarget = api::args(req)?;
                if let Some(mut p) = self.pairing.clone().filter(|p| p.push.as_ref() != Some(&push)) {
                    p.push = Some(push);
                    self.save(&p);
                    self.pairing = Some(p);
                }
                Ok(Value::Null)
            }
            Kind::Session(Session::Unpair) => {
                self.forget()?;
                Ok(Value::Null)
            }
            Kind::Read | Kind::Write => api::run(&self.office, &mut self.ops, kind, req, Instant::now()),
        }
    }

    /// Pairs the phone that sent `pair` in the pairing room: a new long-term room and key, saved, replacing any phone
    /// paired before. The pairing room is left once this is answered.
    fn paired(&mut self, target: &Target, args: PairArgs) -> api::Answer {
        let name: String = args.device_name.trim().chars().take(MAX_DEVICE_NAME).collect();
        if name.is_empty() {
            return Err(api::Failure::remote(api::RemoteCode::BadRequest, "The phone sent no name."));
        }
        let company = self.office.company_name()?;
        let pairing = Pairing {
            relay: target.relay.clone(),
            room_id: wire::random_id(),
            key: wire::random_id(),
            device_name: name,
            paired_at: chrono::Utc::now(),
            push: None,
            push_on: true,
            last_seen_at: None,
        };
        store::save(&self.dir, &pairing).map_err(AppError::from)?;
        let ret = json!({ "roomId": pairing.room_id, "key": pairing.key, "companyName": company });
        self.pairing = Some(pairing);
        self.pair = None;
        self.ops = Ops::default();
        self.publish();
        Ok(ret)
    }

    /// Saves `pairing`, logging a failure: the link keeps working from memory.
    fn save(&self, pairing: &Pairing) {
        if let Err(e) = store::save(&self.dir, pairing) {
            log!("phone link: could not save {}: {e}", store::FILE);
        }
    }

    /// Forgets the paired phone: its room and key are never used again. Fails, still paired, when the saved pairing
    /// cannot be removed, since the next start would load it again.
    fn forget(&mut self) -> Result<()> {
        store::delete(&self.dir).map_err(|e| AppError::internal(format!("OmegaStar could not forget the phone: {e}")))?;
        self.pairing = None;
        self.ops = Ops::default();
        self.publish();
        Ok(())
    }

    /// Handles one command from the handle.
    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::PairStart(reply) => {
                let _ = reply.send(self.pair_start());
            }
            Cmd::PairCancel(reply) => {
                self.pair = None;
                let _ = reply.send(());
            }
            Cmd::Unpair(reply) => {
                let _ = reply.send(self.forget());
            }
            Cmd::SetPush(on, reply) => {
                let result = match self.pairing.as_mut() {
                    Some(p) => {
                        p.push_on = on;
                        let p = p.clone();
                        self.save(&p);
                        Ok(self.status())
                    }
                    None => Err(AppError::conflict("No phone is paired.")),
                };
                let _ = reply.send(result);
            }
            Cmd::Changed(what) => {
                if self.keys.is_some() {
                    self.pending |= what.bit();
                }
            }
            Cmd::Push(title) => self.push(title),
            Cmd::TokenGone(token) => {
                if let Some(mut p) = self.pairing.clone().filter(|p| p.push.as_ref().is_some_and(|t| t.token == token)) {
                    p.push = None;
                    self.save(&p);
                    self.pairing = Some(p);
                }
            }
        }
        self.publish();
    }

    /// Opens a new pairing room and returns its code.
    fn pair_start(&mut self) -> Result<PairCode> {
        let relay = self.shared.relay.clone();
        let room = wire::random_id();
        let key = wire::random_bytes();
        let link = format!("omegastar://pair?v=1&relay={relay}&room={room}&key={}", wire::encode(&key));
        let qr = qr_data_url(&link)?;
        let expires_at = chrono::Utc::now() + chrono::Duration::from_std(PAIR_FOR).unwrap_or_default();
        self.pair = Some(PairRoom { relay, room, key, deadline: Instant::now() + PAIR_FOR });
        Ok(PairCode { qr, link, expires_at })
    }

    /// Sends a push titled `title` to the paired phone, when it has a token and pushes are on.
    fn push(&self, title: String) {
        let Some(p) = self.pairing.as_ref().filter(|p| p.push_on) else { return };
        let Some(target) = p.push.clone() else { return };
        let (client, url, tx) = (self.client.clone(), push_url(&p.relay), self.shared.tx.clone());
        tokio::spawn(async move {
            if push::send(&client, &url, &target, &title).await {
                let _ = tx.send(Cmd::TokenGone(target.token));
            }
        });
    }

    /// The status as Settings shows it.
    fn status(&self) -> Status {
        let state = match (&self.pair, &self.pairing) {
            (Some(_), _) => State::Pairing,
            (None, None) => State::Off,
            (None, Some(_)) => self.link,
        };
        let p = self.pairing.as_ref();
        Status {
            state,
            device_name: p.map(|p| p.device_name.clone()),
            paired_at: p.map(|p| p.paired_at),
            last_seen_at: p.and_then(|p| p.last_seen_at),
            push_on: p.is_none_or(|p| p.push_on),
            error: self.error.clone().filter(|_| state != State::Off),
        }
    }

    /// Tells the handle the status, if it changed.
    fn publish(&self) {
        let now = self.status();
        self.shared.status.send_if_modified(|s| {
            let changed = *s != now;
            *s = now;
            changed
        });
    }
}

/// Reconnect waits: 1 second doubling to 60, each with 20% jitter either way.
struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Backoff {
        Backoff { next: BACKOFF_FIRST }
    }
}

impl Backoff {
    /// The next wait.
    fn next(&mut self) -> Duration {
        let base = self.next;
        self.next = (base * 2).min(BACKOFF_MAX);
        let r = wire::random_bytes();
        let unit = f64::from(u16::from_le_bytes([r[0], r[1]])) / f64::from(u16::MAX);
        base.mul_f64(0.8 + 0.4 * unit)
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn a_relay_on_this_computer_goes_without_tls() {
        assert_eq!(room_url("localhost:8787"), "ws://localhost:8787/v1/room?role=laptop");
        assert_eq!(room_url("127.0.0.1"), "ws://127.0.0.1/v1/room?role=laptop");
        assert_eq!(room_url(RELAY), "wss://relay.omegastar.app/v1/room?role=laptop");
        assert_eq!(push_url("127.0.0.1:9000"), "http://127.0.0.1:9000/v1/push");
        assert_eq!(push_url("relay.example.com"), "https://relay.example.com/v1/push");
        assert!(!is_local("localhost.example.com"));
    }

    #[test]
    fn backoff_doubles_to_a_minute_with_a_fifth_of_jitter() {
        let mut b = Backoff::default();
        for base in [1, 2, 4, 8, 16, 32, 60, 60] {
            let wait = b.next().as_secs_f64();
            let base = base as f64;
            assert!((base * 0.8..=base * 1.2).contains(&wait), "{wait} is not within 20% of {base}");
        }
    }

    #[test]
    fn the_qr_code_is_an_svg_data_url() {
        let url = qr_data_url("omegastar://pair?v=1&relay=relay.example.com&room=r&key=k").unwrap();
        let svg = base64::engine::general_purpose::STANDARD.decode(url.strip_prefix("data:image/svg+xml;base64,").unwrap()).unwrap();
        assert!(String::from_utf8(svg).unwrap().contains("<svg"));
    }
}
