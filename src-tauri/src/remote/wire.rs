//! What crosses the relay: hello frames, the session keys they lead to, and sealed frames, as section 7 of
//! `docs/mobile-companion.md` sets out. Pure: randomness comes in as a parameter wherever a test needs it fixed.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
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
    api: ApiRange,
}

/// The computer's half of the handshake.
#[derive(Serialize)]
struct LaptopHello<'a> {
    v: u32,
    nonce: &'a str,
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

/// What the computer does with a phone's hello.
pub enum Answer {
    /// Send `frame` and start a session with `keys`.
    Start { frame: Vec<u8>, keys: Box<Keys> },
    /// Send `frame` (`update_phone` or `update_desktop`) and start no session.
    Refuse { frame: Vec<u8> },
}

/// Answers the hello JSON a phone sent (`body`, without the type byte) in `room`, sealed with `key`, using `nonce` as
/// this side's fresh value. `None` when it is not a hello this version understands, which is then ignored.
pub fn answer_hello(body: &[u8], key: &[u8; 32], room: &str, version: &str, nonce: &[u8; 32]) -> Option<Answer> {
    let hello: PhoneHello = serde_json::from_slice(body).ok()?;
    if hello.v != 1 || decode32(&hello.nonce).is_none() {
        return None;
    }
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
    let nonce = encode(nonce);
    let json = serde_json::to_vec(&LaptopHello { v: 1, nonce: &nonce, api: API, desktop: Desktop { version } }).ok()?;
    let (k_phone, k_laptop) = derive(key, body, &json);
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

/// The session's two keys from the long-term `key` and both hellos: `(kPhone, kLaptop)`.
pub fn derive(key: &[u8; 32], phone_hello: &[u8], laptop_hello: &[u8]) -> ([u8; 32], [u8; 32]) {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt(phone_hello, laptop_hello)).extract(key);
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

    fn phone_hello(min: u32, max: u32) -> Vec<u8> {
        json!({"v": 1, "nonce": encode(&run(0x40)), "api": {"min": min, "max": max}, "app": {"platform": "ios", "version": "1.0.0"}})
            .to_string()
            .into_bytes()
    }

    /// The shared test vectors, in the key order the other implementations write.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Vectors {
        key: String,
        room_id: String,
        phone_hello: String,
        laptop_hello: String,
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
    /// 0x60.. for the phone's and computer's nonces.
    fn vectors() -> String {
        let key = run(0);
        let room = encode(&run(0x20));
        let phone_hello = format!(
            r#"{{"v":1,"nonce":"{}","api":{{"min":1,"max":1}},"app":{{"platform":"ios","version":"1.0.0"}}}}"#,
            encode(&run(0x40))
        );
        let laptop_hello = format!(r#"{{"v":1,"nonce":"{}","api":1,"desktop":{{"version":"0.7.0"}}}}"#, encode(&run(0x60)));
        let (k_phone, k_laptop) = derive(&key, phone_hello.as_bytes(), laptop_hello.as_bytes());
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
            phone_hello,
            laptop_hello,
            k_phone: hex(&k_phone),
            k_laptop: hex(&k_laptop),
            frames,
        };
        serde_json::to_string_pretty(&out).unwrap() + "\n"
    }

    #[test]
    fn the_shared_vectors_match_the_committed_file_and_each_frame_opens() {
        let committed = include_str!("vectors.json");
        assert_eq!(vectors(), committed);

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

    #[test]
    fn a_hello_starts_a_session_both_sides_can_use() {
        let key = run(0);
        let hello = phone_hello(1, 1);
        let Some(Answer::Start { frame, mut keys }) = answer_hello(&hello, &key, "room", "0.7.0", &run(0x60)) else {
            panic!("expected a session");
        };
        assert_eq!(frame[0], HELLO);
        let sent: Value = serde_json::from_slice(&frame[1..]).unwrap();
        assert_eq!(sent, json!({"v": 1, "nonce": encode(&run(0x60)), "api": 1, "desktop": {"version": "0.7.0"}}));

        let (k_phone, k_laptop) = derive(&key, &hello, &frame[1..]);
        let mut phone = Keys::new(&k_phone, &k_laptop, "room");
        assert_eq!(keys.open(&phone.seal(b"ping")).unwrap(), b"ping");
        assert_eq!(phone.open(&keys.seal(b"pong")).unwrap(), b"pong");
    }

    #[test]
    fn a_phone_outside_this_api_is_told_which_side_to_update() {
        let refusal = |min, max| match answer_hello(&phone_hello(min, max), &run(0), "room", "0.7.0", &run(0x60)) {
            Some(Answer::Refuse { frame }) => serde_json::from_slice::<Value>(&frame[1..]).unwrap(),
            _ => panic!("expected a refusal"),
        };
        assert_eq!(refusal(2, 3), json!({"v": 1, "error": "update_desktop"}));
        assert_eq!(refusal(0, 0), json!({"v": 1, "error": "update_phone"}));
    }

    #[test]
    fn a_malformed_hello_is_ignored() {
        let answer = |body: &[u8]| answer_hello(body, &run(0), "room", "0.7.0", &run(0x60)).is_some();
        assert!(!answer(b"not json"));
        assert!(!answer(br#"{"v":2,"nonce":"x","api":{"min":1,"max":1}}"#));
        assert!(!answer(br#"{"v":1,"nonce":"short","api":{"min":1,"max":1}}"#));
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
