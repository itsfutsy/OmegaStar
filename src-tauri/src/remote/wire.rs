//! What crosses the relay: hello frames, the session keys they lead to, and sealed frames, as section 7 of
//! `docs/mobile-companion.md` sets out. Pure: randomness comes in as a parameter wherever a test needs it fixed.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::agreement::{agree_ephemeral, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use ring::digest::{digest, SHA256};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

/// The remote API version this computer speaks (section 8).
pub const API: u32 = 1;
/// The type byte of a hello frame.
pub const HELLO: u8 = 0x01;
/// The type byte of a sealed frame.
pub const SEALED: u8 = 0x02;

const INFO_PHONE: &[u8] = b"omegastar-remote v1 phone";
const INFO_LAPTOP: &[u8] = b"omegastar-remote v1 laptop";

/// 32 bytes from the system's secure random source.
pub fn random_bytes() -> [u8; 32] {
    let mut out = [0u8; 32];
    SystemRandom::new().fill(&mut out).expect("the system has no secure random source");
    out
}

/// 32 random bytes as base64url without padding: every room id, key and nonce (section 4).
pub fn random_id() -> String {
    encode(&random_bytes())
}

/// Base64url without padding.
pub fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The 32 bytes a base64url room id, key or nonce stands for, if that is what it is.
pub fn decode32(text: &str) -> Option<[u8; 32]> {
    URL_SAFE_NO_PAD.decode(text).ok()?.try_into().ok()
}

/// The range of API versions a phone speaks.
#[derive(Deserialize)]
struct ApiRange {
    min: u32,
    max: u32,
}

/// The parts of a phone's hello the computer reads.
#[derive(Deserialize)]
struct PhoneHello {
    v: u32,
    nonce: String,
    /// The phone's public key for this session's exchange.
    #[serde(rename = "pub")]
    public: String,
    api: ApiRange,
}

/// The computer's half of the handshake.
#[derive(Serialize)]
struct LaptopHello<'a> {
    v: u32,
    nonce: &'a str,
    #[serde(rename = "pub")]
    public: &'a str,
    api: u32,
    desktop: Desktop<'a>,
}

/// The computer's app version, inside its hello.
#[derive(Serialize)]
struct Desktop<'a> {
    version: &'a str,
}

/// The computer's answer when the two cannot work together.
#[derive(Serialize)]
struct Refusal {
    v: u32,
    error: &'static str,
}

/// One side's key pair for one session's key exchange (X25519). It is used once and never saved, so a session's keys
/// cannot be worked out later from the long-term key and what crossed the relay.
pub struct Exchange {
    private: EphemeralPrivateKey,
    public: [u8; 32],
}

impl Exchange {
    /// A fresh key pair from the system's secure random source.
    pub fn generate() -> Exchange {
        let private = EphemeralPrivateKey::generate(&X25519, &SystemRandom::new()).expect("the system has no secure random source");
        let public = private.compute_public_key().expect("an X25519 key has a public half");
        Exchange { public: public.as_ref().try_into().expect("X25519 public keys are 32 bytes"), private }
    }

    /// The public half, which goes in the hello.
    pub fn public(&self) -> &[u8; 32] {
        &self.public
    }

    /// The secret shared with the side whose public half is `theirs`. `None` when `theirs` gives no secret.
    pub fn shared(self, theirs: &[u8; 32]) -> Option<[u8; 32]> {
        agree_ephemeral(self.private, &UnparsedPublicKey::new(&X25519, theirs), |secret| secret.try_into().ok()).ok()?
    }
}

/// What the computer does with a phone's hello.
pub enum Answer {
    /// Send `frame` and start a session with `keys`.
    Start { frame: Vec<u8>, keys: Box<Keys> },
    /// Send `frame` (`update_phone` or `update_desktop`) and start no session.
    Refuse { frame: Vec<u8> },
}

/// Answers the hello JSON a phone sent (`body`, without the type byte) in `room`, sealed with `key`, using `nonce` as
/// this side's fresh value and `exchange` as its half of the key exchange. `None` when it is not a hello this version
/// understands, which is then ignored.
pub fn answer_hello(body: &[u8], key: &[u8; 32], room: &str, version: &str, nonce: &[u8; 32], exchange: Exchange) -> Option<Answer> {
    let hello: PhoneHello = serde_json::from_slice(body).ok()?;
    if hello.v != 1 || decode32(&hello.nonce).is_none() {
        return None;
    }
    let theirs = decode32(&hello.public)?;
    let refused = if API < hello.api.min {
        Some("update_desktop")
    } else if API > hello.api.max {
        Some("update_phone")
    } else {
        None
    };
    if let Some(error) = refused {
        let json = serde_json::to_vec(&Refusal { v: 1, error }).ok()?;
        return Some(Answer::Refuse { frame: framed(HELLO, &json) });
    }
    let (nonce, public) = (encode(nonce), encode(exchange.public()));
    let json = serde_json::to_vec(&LaptopHello { v: 1, nonce: &nonce, public: &public, api: API, desktop: Desktop { version } }).ok()?;
    let (k_phone, k_laptop) = derive(key, &exchange.shared(&theirs)?, body, &json);
    Some(Answer::Start { frame: framed(HELLO, &json), keys: Box::new(Keys::new(&k_laptop, &k_phone, room)) })
}

/// A frame: its type byte, then `body`.
fn framed(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(kind);
    out.extend_from_slice(body);
    out
}

/// The handshake's salt: SHA-256 of the phone's hello then the computer's, as sent.
pub fn salt(phone_hello: &[u8], laptop_hello: &[u8]) -> [u8; 32] {
    let mut both = phone_hello.to_vec();
    both.extend_from_slice(laptop_hello);
    digest(&SHA256, &both).as_ref().try_into().expect("SHA-256 is 32 bytes")
}

/// The session's two keys from the long-term `key`, the exchange's `shared` secret and both hellos:
/// `(kPhone, kLaptop)`. The long-term key proves who is talking; the shared secret is what only this session has.
pub fn derive(key: &[u8; 32], shared: &[u8; 32], phone_hello: &[u8], laptop_hello: &[u8]) -> ([u8; 32], [u8; 32]) {
    let mut secret = [0u8; 64];
    secret[..32].copy_from_slice(key);
    secret[32..].copy_from_slice(shared);
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt(phone_hello, laptop_hello)).extract(&secret);
    let expand = |info: &[u8]| {
        let mut out = [0u8; 32];
        prk.expand(&[info], hkdf::HKDF_SHA256).and_then(|okm| okm.fill(&mut out)).expect("32 bytes is a valid HKDF length");
        out
    };
    (expand(INFO_PHONE), expand(INFO_LAPTOP))
}

/// One side of a session: the key it seals with, the key it opens with, and a counter for each direction.
pub struct Keys {
    seal: LessSafeKey,
    open: LessSafeKey,
    room: String,
    sent: u64,
    received: u64,
}

/// The 12-byte nonce for frame `counter`: four zero bytes, then the counter big-endian.
fn nonce(counter: u64) -> Nonce {
    let mut bytes = [0u8; 12];
    bytes[4..].copy_from_slice(&counter.to_be_bytes());
    Nonce::assume_unique_for_key(bytes)
}

/// An AES-256-GCM key from its 32 bytes.
fn aes(key: &[u8; 32]) -> LessSafeKey {
    LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).expect("AES-256 keys are 32 bytes"))
}

impl Keys {
    /// Keys that seal with `seal` and open with `open` in `room`, both counters at 0.
    pub fn new(seal: &[u8; 32], open: &[u8; 32], room: &str) -> Keys {
        Keys { seal: aes(seal), open: aes(open), room: room.into(), sent: 0, received: 0 }
    }

    /// The sealed frame for `plaintext`, using the next send counter.
    pub fn seal(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let mut out = framed(SEALED, plaintext);
        let tag = self
            .seal
            .seal_in_place_separate_tag(nonce(self.sent), Aad::from(self.room.as_bytes()), &mut out[1..])
            .expect("a frame fits AES-GCM's limits");
        self.sent += 1;
        out.extend_from_slice(tag.as_ref());
        out
    }

    /// The plaintext of a sealed `frame`, using the next receive counter. `None` when it does not open, which ends
    /// the session.
    pub fn open(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
        let (&kind, body) = frame.split_first()?;
        if kind != SEALED {
            return None;
        }
        let mut body = body.to_vec();
        let plain = self.open.open_in_place(nonce(self.received), Aad::from(self.room.as_bytes()), &mut body).ok()?;
        let len = plain.len();
        self.received += 1;
        body.truncate(len);
        Some(body)
    }
}

/// How many bytes the sealed frame for a plaintext of `len` bytes takes: the type byte and the tag.
pub fn sealed_len(len: usize) -> usize {
    len + 1 + AES_256_GCM.tag_len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// Bytes `from` up to `from + 31`.
    fn run(from: u8) -> [u8; 32] {
        std::array::from_fn(|i| from + i as u8)
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
    }

    /// The fixed key pairs of the shared vectors: the private halves are bytes 0x80.. (phone) and 0xa0.. (computer).
    /// ring takes no private key from outside, so the public halves and the secret the two share were worked out
    /// from them once, with two other X25519 implementations, and every other implementation checks them again.
    const PHONE_PUBLIC: &str = "493e82fc74464a59268817623d2053c5eb8e2cc4a988b4fee179ec6b010d531d";
    const LAPTOP_PUBLIC: &str = "605a725d2a4adfeeb1a29e17edd621c1b7593ee8cdbc44ac6c4ab6e2f805d23c";
    const SHARED: &str = "c6dea8dd115ef27b7e0953539b2b19e59b7abf3ffd57985ec76de86ec31d1b42";

    fn phone_hello(public: &[u8; 32], min: u32, max: u32) -> Vec<u8> {
        json!({"v": 1, "nonce": encode(&run(0x40)), "pub": encode(public), "api": {"min": min, "max": max}, "app": {"platform": "ios", "version": "1.0.0"}})
            .to_string()
            .into_bytes()
    }

    /// The shared test vectors, in the key order the other implementations write.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Vectors {
        key: String,
        room_id: String,
        phone_private: String,
        laptop_private: String,
        phone_hello: String,
        laptop_hello: String,
        shared: String,
        salt: String,
        k_phone: String,
        k_laptop: String,
        frames: Vec<VectorFrame>,
    }

    /// One sealed frame of the shared vectors.
    #[derive(Serialize)]
    struct VectorFrame {
        from: &'static str,
        counter: u64,
        plaintext: &'static str,
        frame: String,
    }

    /// The shared vectors from their fixed inputs: bytes 0x00.. for the key, 0x20.. for the room id, 0x40.. and
    /// 0x60.. for the phone's and computer's nonces, and 0x80.. and 0xa0.. for their private keys.
    fn vectors() -> String {
        let key = run(0);
        let room = encode(&run(0x20));
        let shared: [u8; 32] = unhex(SHARED).try_into().unwrap();
        let phone_hello = format!(
            r#"{{"v":1,"nonce":"{}","pub":"{}","api":{{"min":1,"max":1}},"app":{{"platform":"ios","version":"1.0.0"}}}}"#,
            encode(&run(0x40)),
            encode(&unhex(PHONE_PUBLIC))
        );
        let laptop_hello =
            format!(r#"{{"v":1,"nonce":"{}","pub":"{}","api":1,"desktop":{{"version":"0.7.0"}}}}"#, encode(&run(0x60)), encode(&unhex(LAPTOP_PUBLIC)));
        let (k_phone, k_laptop) = derive(&key, &shared, phone_hello.as_bytes(), laptop_hello.as_bytes());
        let mut phone = Keys::new(&k_phone, &k_laptop, &room);
        let mut laptop = Keys::new(&k_laptop, &k_phone, &room);
        let sends = [
            ("phone", r#"{"t":"req","id":1,"cmd":"inbox","args":{}}"#),
            (
                "phone",
                r#"{"t":"req","id":2,"cmd":"set_archived","args":{"runId":"run-1","archived":true},"op":"00000000-0000-4000-8000-000000000001"}"#,
            ),
            ("laptop", r#"{"t":"res","id":1,"ok":true,"ret":{"threads":[]}}"#),
        ];
        let frames = sends
            .iter()
            .map(|&(from, plaintext)| {
                let keys = if from == "phone" { &mut phone } else { &mut laptop };
                let counter = keys.sent;
                VectorFrame { from, counter, plaintext, frame: hex(&keys.seal(plaintext.as_bytes())) }
            })
            .collect();
        let out = Vectors {
            key: encode(&key),
            salt: hex(&salt(phone_hello.as_bytes(), laptop_hello.as_bytes())),
            room_id: room,
            phone_private: hex(&run(0x80)),
            laptop_private: hex(&run(0xa0)),
            phone_hello,
            laptop_hello,
            shared: hex(&shared),
            k_phone: hex(&k_phone),
            k_laptop: hex(&k_laptop),
            frames,
        };
        serde_json::to_string_pretty(&out).unwrap() + "\n"
    }

    #[test]
    fn the_shared_vectors_match_the_committed_file_and_each_frame_opens() {
        // Git on Windows may check the file out with its own line endings.
        let committed = &include_str!("vectors.json").replace("\r\n", "\n");
        assert_eq!(&vectors(), committed);

        let v: Value = serde_json::from_str(committed).unwrap();
        let k_phone: [u8; 32] = unhex(v["kPhone"].as_str().unwrap()).try_into().unwrap();
        let k_laptop: [u8; 32] = unhex(v["kLaptop"].as_str().unwrap()).try_into().unwrap();
        let room = v["roomId"].as_str().unwrap();
        let mut at_laptop = Keys::new(&k_laptop, &k_phone, room);
        let mut at_phone = Keys::new(&k_phone, &k_laptop, room);
        for f in v["frames"].as_array().unwrap() {
            let opener = if f["from"] == "phone" { &mut at_laptop } else { &mut at_phone };
            let plain = opener.open(&unhex(f["frame"].as_str().unwrap())).expect("a vector frame opens");
            assert_eq!(plain, f["plaintext"].as_str().unwrap().as_bytes());
        }
    }

    /// The computer's public key in its hello `frame`.
    fn public_in(frame: &[u8]) -> [u8; 32] {
        let hello: Value = serde_json::from_slice(&frame[1..]).unwrap();
        decode32(hello["pub"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn a_hello_starts_a_session_both_sides_can_use() {
        let key = run(0);
        let exchange = Exchange::generate();
        let hello = phone_hello(exchange.public(), 1, 1);
        let Some(Answer::Start { frame, mut keys }) = answer_hello(&hello, &key, "room", "0.7.0", &run(0x60), Exchange::generate()) else {
            panic!("expected a session");
        };
        assert_eq!(frame[0], HELLO);
        let theirs = public_in(&frame);
        let sent: Value = serde_json::from_slice(&frame[1..]).unwrap();
        assert_eq!(sent, json!({"v": 1, "nonce": encode(&run(0x60)), "pub": encode(&theirs), "api": 1, "desktop": {"version": "0.7.0"}}));

        let (k_phone, k_laptop) = derive(&key, &exchange.shared(&theirs).unwrap(), &hello, &frame[1..]);
        let mut phone = Keys::new(&k_phone, &k_laptop, "room");
        assert_eq!(keys.open(&phone.seal(b"ping")).unwrap(), b"ping");
        assert_eq!(phone.open(&keys.seal(b"pong")).unwrap(), b"pong");
    }

    #[test]
    fn the_long_term_key_and_both_hellos_do_not_give_a_sessions_keys() {
        let key = run(0);
        let exchange = Exchange::generate();
        let hello = phone_hello(exchange.public(), 1, 1);
        let Some(Answer::Start { frame, mut keys }) = answer_hello(&hello, &key, "room", "0.7.0", &run(0x60), Exchange::generate()) else {
            panic!("expected a session");
        };
        // Someone who recorded both hellos and later learns the key still lacks both private halves.
        let guess = Exchange::generate().shared(&public_in(&frame)).unwrap();
        let (k_phone, k_laptop) = derive(&key, &guess, &hello, &frame[1..]);
        assert!(keys.open(&Keys::new(&k_phone, &k_laptop, "room").seal(b"ping")).is_none());
    }

    #[test]
    fn both_sides_of_an_exchange_share_one_secret() {
        let (phone, laptop) = (Exchange::generate(), Exchange::generate());
        let (phone_public, laptop_public) = (*phone.public(), *laptop.public());
        assert_ne!(phone_public, laptop_public);
        let secret = phone.shared(&laptop_public).unwrap();
        assert_eq!(laptop.shared(&phone_public).unwrap(), secret);
        assert_ne!(Exchange::generate().shared(&laptop_public).unwrap(), secret);
    }

    #[test]
    fn a_phone_outside_this_api_is_told_which_side_to_update() {
        let refusal = |min, max| match answer_hello(&phone_hello(&run(0x80), min, max), &run(0), "room", "0.7.0", &run(0x60), Exchange::generate()) {
            Some(Answer::Refuse { frame }) => serde_json::from_slice::<Value>(&frame[1..]).unwrap(),
            _ => panic!("expected a refusal"),
        };
        assert_eq!(refusal(2, 3), json!({"v": 1, "error": "update_desktop"}));
        assert_eq!(refusal(0, 0), json!({"v": 1, "error": "update_phone"}));
    }

    #[test]
    fn a_malformed_hello_is_ignored() {
        let answer = |body: &[u8]| answer_hello(body, &run(0), "room", "0.7.0", &run(0x60), Exchange::generate()).is_some();
        assert!(answer(&phone_hello(Exchange::generate().public(), 1, 1)));
        assert!(!answer(b"not json"));
        assert!(!answer(br#"{"v":2,"nonce":"x","api":{"min":1,"max":1}}"#));
        assert!(!answer(br#"{"v":1,"nonce":"short","api":{"min":1,"max":1}}"#));
        let nonce = encode(&run(0x40));
        assert!(!answer(format!(r#"{{"v":1,"nonce":"{nonce}","api":{{"min":1,"max":1}}}}"#).as_bytes()), "no public key");
        assert!(!answer(format!(r#"{{"v":1,"nonce":"{nonce}","pub":"short","api":{{"min":1,"max":1}}}}"#).as_bytes()));
        assert!(!answer(&phone_hello(&[0; 32], 1, 1)), "a public key that gives no secret");
    }

    #[test]
    fn a_frame_opens_only_once_in_order_in_its_room_and_unaltered() {
        let (a, b) = (run(0), run(0x20));
        let mut sender = Keys::new(&a, &b, "room");
        let first = sender.seal(b"one");
        let second = sender.seal(b"two");

        let mut out_of_order = Keys::new(&b, &a, "room");
        assert!(out_of_order.open(&second).is_none());

        let mut other_room = Keys::new(&b, &a, "elsewhere");
        assert!(other_room.open(&first).is_none());

        let mut altered = first.clone();
        *altered.last_mut().unwrap() ^= 1;
        assert!(Keys::new(&b, &a, "room").open(&altered).is_none());

        let mut receiver = Keys::new(&b, &a, "room");
        assert_eq!(receiver.open(&first).unwrap(), b"one");
        assert!(receiver.open(&first).is_none(), "a replayed frame must not open");
    }

    #[test]
    fn sealed_len_counts_the_type_byte_and_the_tag() {
        assert_eq!(Keys::new(&run(0), &run(1), "room").seal(b"abc").len(), sealed_len(3));
    }
}
