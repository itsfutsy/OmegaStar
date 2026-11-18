//! The phone link end to end: the real `Remote` against a fake relay on this computer and a test phone, and
//! `remote_e2e`, which pairs a real phone through a real relay.

use super::fixture::Fixture;
use super::*;
use futures_util::stream::{SplitSink, SplitStream};
use std::collections::HashMap;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{Callback, ErrorResponse, Request as Upgrade, Response as Upgraded};

/// Each room's sockets, by role.
type Rooms = Arc<Mutex<HashMap<String, HashMap<String, mpsc::UnboundedSender<Message>>>>>;

/// A relay with the real one's room rules: one socket per role, binary frames forwarded to the other role, and
/// `peer` text frames when the other role joins or leaves. Returns its host:port.
async fn fake_relay() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let rooms: Rooms = Arc::default();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            tokio::spawn(relay_socket(tcp, rooms.clone()));
        }
    });
    addr
}

/// A relay control frame.
fn peer(state: &str) -> Message {
    Message::Text(json!({ "t": "peer", "state": state }).to_string().into())
}

/// Reads the role and the room off a socket's upgrade request.
struct Joining<'a>(&'a mut Option<(String, String)>);

impl Callback for Joining<'_> {
    fn on_request(self, req: &Upgrade, res: Upgraded) -> std::result::Result<Upgraded, ErrorResponse> {
        let role = req.uri().query().and_then(|q| q.strip_prefix("role=")).map(String::from);
        let room = req.headers().get(AUTHORIZATION).and_then(|v| v.to_str().ok()?.strip_prefix("Bearer ").map(String::from));
        *self.0 = role.zip(room);
        Ok(res)
    }
}

/// Serves one socket of the fake relay.
async fn relay_socket(tcp: TcpStream, rooms: Rooms) {
    let mut joined = None;
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(tcp, Joining(&mut joined)).await else { return };
    let Some((role, room)) = joined else { return };
    let other = if role == "laptop" { "phone" } else { "laptop" };
    let (mut sink, mut stream) = ws.split();
    let (tx, mut outbox) = mpsc::unbounded_channel();
    {
        let mut rooms = rooms.lock().unwrap();
        let sockets = rooms.entry(room.clone()).or_default();
        sockets.insert(role.clone(), tx.clone());
        if let Some(them) = sockets.get(other) {
            let _ = them.send(peer("joined"));
            let _ = tx.send(peer("joined"));
        }
    }
    let writer = tokio::spawn(async move {
        while let Some(msg) = outbox.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(msg)) = stream.next().await {
        if let Message::Binary(bytes) = msg {
            if let Some(them) = rooms.lock().unwrap().get(&room).and_then(|s| s.get(other)) {
                let _ = them.send(Message::Binary(bytes));
            }
        }
    }
    let mut rooms = rooms.lock().unwrap();
    if let Some(sockets) = rooms.get_mut(&room) {
        if sockets.get(&role).is_some_and(|s| s.same_channel(&tx)) {
            sockets.remove(&role);
            if let Some(them) = sockets.get(other) {
                let _ = them.send(peer("left"));
            }
        }
    }
    writer.abort();
}

type PhoneStream = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

/// The next message from the relay, within 10 seconds.
async fn next(rx: &mut PhoneStream) -> Message {
    tokio::time::timeout(Duration::from_secs(10), rx.next()).await.expect("the relay sends something").unwrap().unwrap()
}

/// A test phone in a room, with a session open.
struct Phone {
    tx: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    rx: PhoneStream,
    keys: wire::Keys,
    next_id: u64,
}

impl Phone {
    /// Joins `room` at `relay` as the phone, waits for the computer, and runs the handshake with `key`.
    async fn join(relay: &str, room: &str, key: &[u8; 32]) -> Phone {
        let mut req = format!("ws://{relay}/v1/room?role=phone").into_client_request().unwrap();
        req.headers_mut().insert(AUTHORIZATION, format!("Bearer {room}").parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        let (mut tx, mut rx) = ws.split();
        while next(&mut rx).await != peer("joined") {}
        let hello = json!({ "v": 1, "nonce": wire::encode(&wire::random_bytes()), "api": { "min": 1, "max": 1 }, "app": { "platform": "ios", "version": "1.0.0" } }).to_string();
        let mut frame = vec![wire::HELLO];
        frame.extend_from_slice(hello.as_bytes());
        tx.send(Message::Binary(frame.into())).await.unwrap();
        let Message::Binary(reply) = next(&mut rx).await else { panic!("the computer's hello is binary") };
        assert_eq!(reply[0], wire::HELLO);
        let (k_phone, k_laptop) = wire::derive(key, hello.as_bytes(), &reply[1..]);
        Phone { tx, rx, keys: wire::Keys::new(&k_phone, &k_laptop, room), next_id: 0 }
    }

    /// The next sealed message from the computer.
    async fn recv(&mut self) -> Value {
        loop {
            if let Message::Binary(frame) = next(&mut self.rx).await {
                return serde_json::from_slice(&self.keys.open(&frame).expect("the frame opens")).unwrap();
            }
        }
    }

    /// The next event from the computer.
    async fn event(&mut self) -> Value {
        loop {
            let msg = self.recv().await;
            if msg["t"] == "evt" {
                return msg;
            }
        }
    }

    /// Sends request `cmd` and returns its response.
    async fn call(&mut self, cmd: &str, args: Value, op: Option<&str>) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let req = json!({ "t": "req", "id": id, "cmd": cmd, "args": args, "op": op });
        let frame = self.keys.seal(req.to_string().as_bytes());
        self.tx.send(Message::Binary(frame.into())).await.unwrap();
        loop {
            let msg = self.recv().await;
            if msg["t"] == "res" && msg["id"] == id {
                return msg;
            }
        }
    }
}

/// Waits until the link's status passes `ok`.
async fn wait_for(remote: &Remote, ok: impl Fn(&Status) -> bool) -> Status {
    let mut rx = remote.subscribe();
    let status = tokio::time::timeout(Duration::from_secs(10), rx.wait_for(|s| ok(s))).await;
    let status = status.unwrap_or_else(|_| panic!("the status stayed {:?}", remote.status())).unwrap().clone();
    status
}

/// The pairing room and key in a pairing link.
fn room_and_key(link: &str) -> (String, [u8; 32]) {
    let query: HashMap<&str, &str> = link.split_once('?').unwrap().1.split('&').filter_map(|kv| kv.split_once('=')).collect();
    (query["room"].into(), wire::decode32(query["key"]).unwrap())
}

/// Pairs a test phone through `relay` and returns it connected to the long-term room.
async fn paired_phone(remote: &Remote, relay: &str) -> Phone {
    let code = remote.pair_start().await.unwrap();
    assert_eq!(remote.status().state, State::Pairing);
    assert!(code.link.contains(&format!("relay={relay}&")));
    let (room, key) = room_and_key(&code.link);
    let mut phone = Phone::join(relay, &room, &key).await;
    let refused = phone.call("inbox", json!({}), None).await;
    assert_eq!(refused["err"]["code"], "not_allowed", "only pair is answered in the pairing room");
    let paired = phone.call("pair", json!({ "deviceName": "Test Phone", "platform": "ios" }), None).await;
    assert_eq!(paired["ret"]["companyName"], "Acme");
    let room = paired["ret"]["roomId"].as_str().unwrap().to_string();
    let key = wire::decode32(paired["ret"]["key"].as_str().unwrap()).unwrap();
    wait_for(remote, |s| s.device_name.as_deref() == Some("Test Phone") && s.state == State::Online).await;
    let phone = Phone::join(relay, &room, &key).await;
    wait_for(remote, |s| s.state == State::Session && s.last_seen_at.is_some()).await;
    phone
}

#[tokio::test]
async fn a_phone_pairs_reads_acts_hears_changes_and_is_unpaired() {
    let f = Fixture::new("remote-link").await;
    let relay = fake_relay().await;
    let remote = Remote::new(relay.clone());
    remote.start(f.office.clone());
    assert_eq!(remote.status().state, State::Off);

    let mut phone = paired_phone(&remote, &relay).await;
    let saved = store::load(&f.dir).unwrap();
    assert_eq!((saved.device_name.as_str(), saved.relay.as_str()), ("Test Phone", relay.as_str()));

    let inbox = phone.call("inbox", json!({}), None).await;
    assert_eq!(inbox["ok"], true);
    assert!(inbox["ret"]["threads"].as_array().unwrap().iter().any(|t| t["root"]["id"] == f.carded.as_str()));
    let answer = json!({ "runId": f.carded, "card": 0, "answer": { "type": "choice", "index": 1 } });
    assert_eq!(phone.call("answer_card", answer, Some("op-1")).await["ok"], true);
    assert!(f.office.run(&f.carded).unwrap().cards[0].answer.is_some());

    let registered = phone.call("push_register", json!({ "platform": "ios", "token": "t1", "sandbox": true }), None).await;
    assert_eq!(registered["ret"], Value::Null);
    assert_eq!(store::load(&f.dir).unwrap().push.unwrap().token, "t1");

    remote.changed(What::Inbox);
    remote.changed(What::Team);
    let mut heard = vec![];
    while heard.len() < 2 {
        let evt = phone.event().await;
        assert_eq!(evt["name"], "changed");
        heard.extend(evt["what"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()));
    }
    assert_eq!(heard, ["inbox", "team"]);

    let status = remote.set_push(false).await.unwrap();
    assert!(!status.push_on);
    assert!(!store::load(&f.dir).unwrap().push_on);

    remote.unpair().await.unwrap();
    assert_eq!(phone.event().await, json!({ "t": "evt", "name": "unpaired" }));
    assert_eq!(remote.status(), Status::default());
    assert!(!f.dir.join(store::FILE).exists());
}

#[tokio::test]
async fn the_phone_can_unpair_and_a_cancelled_code_stops_pairing() {
    let f = Fixture::new("remote-unpair").await;
    let relay = fake_relay().await;
    let remote = Remote::new(relay.clone());
    remote.start(f.office.clone());

    let mut phone = paired_phone(&remote, &relay).await;
    assert_eq!(phone.call("unpair", json!({}), None).await["ret"], Value::Null);
    wait_for(&remote, |s| s.state == State::Off).await;
    assert!(store::load(&f.dir).is_none());

    remote.pair_start().await.unwrap();
    remote.pair_cancel().await.unwrap();
    assert_eq!(remote.status().state, State::Off);
}

#[tokio::test]
async fn a_code_past_its_time_pairs_nothing() {
    let f = Fixture::new("remote-expired").await;
    let remote = Remote::new(RELAY.into());
    let rx = remote.0.rx.lock().unwrap().take().unwrap();
    let mut actor = Actor::new(remote.0.clone(), f.office.clone(), rx);
    actor.pair_start().unwrap();
    let target = actor.target().unwrap();
    actor.pair.as_mut().unwrap().expires_at = chrono::Utc::now();

    let refused = actor.paired(&target, PairArgs { device_name: "Test Phone".into() }).unwrap_err();
    assert_eq!(refused.code, api::Code::Remote(api::RemoteCode::NotAllowed));
    assert!(actor.pair.is_none() && actor.pairing.is_none());
    assert!(store::load(&f.dir).is_none());
}

#[tokio::test]
async fn a_sandbox_never_links_a_phone() {
    let dir = crate::fixtures::temp_dir("remote-sandbox");
    std::fs::write(dir.join(crate::sandbox::MARKER), "").unwrap();
    let remote = Remote::new(fake_relay().await);
    remote.start(crate::fixtures::open_office(&dir).await);
    assert_eq!(remote.pair_start().await.unwrap_err().message, NO_LINK);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Pairs a real phone through the relay at `OMEGASTAR_RELAY` and serves it the fixture office for `REMOTE_E2E_SECS`
/// seconds (300 unless set), then prints the answer the phone gave Ada's card.
#[tokio::test]
#[ignore = "needs a relay and a phone: OMEGASTAR_RELAY=<host[:port]> cargo test --locked remote_e2e -- --ignored --nocapture"]
async fn remote_e2e() {
    assert!(std::env::var_os("OMEGASTAR_RELAY").is_some(), "set OMEGASTAR_RELAY to the relay's host[:port]");
    let secs = std::env::var("REMOTE_E2E_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let f = Fixture::new("remote-e2e").await;
    let remote = Remote::new(relay());
    remote.start(f.office.clone());
    let code = remote.pair_start().await.unwrap();
    let svg = base64::engine::general_purpose::STANDARD.decode(code.qr.strip_prefix("data:image/svg+xml;base64,").unwrap()).unwrap();
    let qr = std::env::temp_dir().join("omegastar-remote-e2e-pair.svg");
    std::fs::write(&qr, svg).unwrap();
    println!("PAIR_LINK {}", code.link);
    println!("PAIR_QR {}", qr.display());

    let mut status = remote.subscribe();
    let end = tokio::time::sleep(Duration::from_secs(secs));
    tokio::pin!(end);
    loop {
        tokio::select! {
            _ = &mut end => break,
            Ok(()) = status.changed() => println!("STATUS {}", serde_json::to_string(&*status.borrow_and_update()).unwrap()),
        }
    }
    let answer = &f.office.run(&f.carded).unwrap().cards[0].answer;
    println!("CARD_ANSWER {}", serde_json::to_string(answer).unwrap());
}
