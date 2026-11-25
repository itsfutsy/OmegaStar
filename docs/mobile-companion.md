# OmegaStar mobile companion: technical design

Status: draft for build. Version 1 of the design, written 18 November 2026.

## 1. Summary

A phone app that acts as a remote control for the OmegaStar Inbox on your computer. Three parts:

| Part | What it is | Where the code lives |
|---|---|---|
| Desktop | OmegaStar itself, with a new `remote` module | This repository (open source) |
| Relay | One Cloudflare Worker with one Durable Object per paired computer | Private repository |
| Phone app | iOS and Android app | Private repository |

```
Phone app  <== WebSocket ==>  Relay (Cloudflare)  <== WebSocket ==>  OmegaStar desktop
                              forwards sealed
                              frames, stores nothing
```

The computer stays the only home of the Inbox. The phone sends the same kind of commands the desktop interface
already sends, through the relay, sealed with a key the relay never has. Push notifications go from the desktop to
the relay to Apple or Google.

Sections 4 to 10 are the contract between the three codebases. Everything else describes how each side meets it.

## 2. Goals and non-goals

Goals for version 1:

- See the Inbox on the phone and act on it: answer cards, reply, allow blocked tools, approve tidies, decide hire
  proposals, mark done, retry a failed run.
- Push notifications when something needs you.
- Pair by scanning a QR code. No accounts, no logins.
- The relay cannot read or forge anything.
- The relay costs nothing while a computer sits idle.

Not in version 1:

- Anything while the computer is asleep or OmegaStar is closed. The phone shows the computer as offline.
- Watching a run's live transcript, or browsing every conversation.
- Starting new conversations, or attaching files and photos.
- More than one phone per computer, or more than one computer per phone.
- A web client.

## 3. How it fits together

1. **Pair once.** OmegaStar shows a QR code. The phone scans it and the two agree on a long-term room and key.
2. **Computer connects.** While OmegaStar runs, it keeps one WebSocket open to its room on the relay. The room is a
   Durable Object that hibernates when nothing is being sent.
3. **Phone connects.** When the app is opened, it joins the same room.
4. **Handshake.** The two exchange fresh random values and derive keys for this session.
5. **Commands.** The phone sends requests such as "show the Inbox" or "answer this card". The computer runs them
   through the same `Office` methods the desktop interface uses and replies.
6. **Changes.** While the phone is connected, the computer tells it when the Inbox changed, and the phone fetches it
   again.
7. **Push.** When the desktop would show a notification, it also asks the relay to send one to the phone.

## 4. Identifiers and secrets

All random values are 32 bytes from the operating system's secure random source, written as base64url without
padding (43 characters).

| Name | What it is | Who knows it |
|---|---|---|
| `pairRoom`, `pairKey` | One-time room and key carried in the QR code. Valid for 5 minutes and one use. | Computer, phone. The relay sees `pairRoom` only. |
| `roomId` | The long-term room. Knowing it is what lets a client join the room. | Computer, phone, relay |
| `key` | The long-term key. Seals everything. | Computer and phone only |
| Push token | The phone's address at Apple or Google | Phone, computer, and the relay for the moment it sends a push |

## 5. Pairing

The QR code holds one string:

```
omegastar://pair?v=1&relay=relay.example.com&room=<pairRoom>&key=<pairKey>
```

Steps:

1. The user clicks **Connect phone** in Settings. The desktop creates `pairRoom` and `pairKey`, connects to the relay
   in `pairRoom` as `laptop`, and shows the QR code, blurred until the user clicks it. The code goes away after 5
   minutes, with no countdown.
2. The phone scans the code, connects to `pairRoom` as `phone`, and runs the normal handshake (section 7) using
   `pairKey`.
3. The phone sends the request `pair` with `{ deviceName, platform }`, for example
   `{ "deviceName": "Maren's iPhone", "platform": "ios" }`.
4. The desktop creates the long-term `roomId` and `key`, saves the pairing, and answers with
   `{ roomId, key, companyName }`.
5. Both close the pairing room. The phone stores `relay`, `roomId` and `key` in the Keychain or Keystore. Both
   connect to the long-term room.
6. The desktop shows **Paired with Maren's iPhone**.

Rules:

- In the pairing room the only request allowed is `pair`, and only the first valid one is answered.
- After 5 minutes, or on Cancel, the desktop leaves the pairing room and forgets `pairRoom` and `pairKey`. The 5
  minutes are wall-clock time, so a computer that slept through them refuses `pair` when it wakes.
- Whoever scans the code first is paired, so it is a secret for its 5 minutes. Settings keeps it blurred, well past
  what a scanner can read, until the user clicks it, so it is not caught on a shared or recorded screen before then.
- A photo of the QR code is useless once pairing has finished or the 5 minutes are up, even to someone who recorded
  what crossed the relay: the answer that carries `roomId` and `key` is sealed with a session key the code alone does
  not give (section 7).
- Pairing again replaces the previous phone.

**Unpair.** From the desktop: delete the saved pairing, then, if a phone session is live, send the event `unpaired`
so the app wipes its copy, and leave the room. The phone hears `unpaired` only once the computer has forgotten it. From the phone: send the request `unpair`, then wipe. Either way the old
`roomId` and `key` are never used again. If the saved pairing cannot be deleted it is emptied instead, and if that
fails too, Unpair fails and says so, still paired.

## 6. Connecting to the relay

```
GET wss://<relay>/v1/room?role=laptop        (or role=phone)
Authorization: Bearer <roomId>
Upgrade: websocket
```

The room id travels in a header so it stays out of URLs.

| Response | Meaning |
|---|---|
| 101 | Connected |
| 400 | Missing or malformed role or room id |
| 426 | Not a WebSocket upgrade |
| 429 | Too many connections from this address |

**One socket per role.** A new `laptop` connection replaces the old `laptop` socket, and the same for `phone`. The
replaced socket is closed with code 4001.

**Relay control frames** are WebSocket *text* messages, sent only by the relay:

```json
{ "t": "peer", "state": "joined" }
{ "t": "peer", "state": "left" }
```

"Peer" is always the other role. On connect, a client is told `joined` straight away if the other side is already
there. Clients never send text messages, and the relay ignores any it receives.

**Sealed frames** are WebSocket *binary* messages. The relay forwards each one unchanged to the other role, or drops
it if the other role is absent.

**Close codes:** 4001 replaced, 1009 frame too large, 1008 sending too fast.

**Keep-alive.** Each client sends a WebSocket protocol ping every 30 seconds. Cloudflare answers these itself without
waking the room, and they are not billed. If no pong arrives within 10 seconds, the client drops the socket and
reconnects. The desktop does the same when a send to the relay does not finish within 10 seconds, plus a second for
every 256 KiB it carries, so a big image is not cut off on a slow upload.

**Reconnect.** Wait 1 second, then double each time up to 60 seconds, with 20% random jitter. Reset after a
connection has lasted 30 seconds. The desktop reconnects for as long as a pairing exists, unless its socket in the
long-term room is closed with 4001: another computer holds the room, so it stays out and Settings says so until the
next pairing or app start. The phone connects when
the app comes to the foreground and disconnects when it leaves.

## 7. Encryption

Algorithms: AES-256-GCM, HKDF-SHA256, SHA-256 and X25519. All four are built into `ring` (already in OmegaStar's
dependencies) and Apple CryptoKit. Android's `javax.crypto` has AES-GCM, SHA-256 and HMAC-SHA256 built in, and HKDF
is a few lines on top of that HMAC. X25519 on Android is in section 19.

Every binary frame starts with one type byte.

### Handshake

A session starts each time the phone learns the computer is present: on connect if told `joined`, or when `joined`
arrives later. When the computer hears `joined` it ends any session it had, since that was the old socket's, and shows
the phone as connected again only once the new socket's first sealed frame opens.

| Step | Sender | Frame |
|---|---|---|
| 1 | Phone | `0x01` then JSON: `{ "v": 1, "nonce": "<32 random bytes>", "pub": "<X25519 public key>", "api": { "min": 1, "max": 1 }, "app": { "platform": "ios", "version": "1.0.0" } }` |
| 2 | Computer | `0x01` then JSON: `{ "v": 1, "nonce": "<32 random bytes>", "pub": "<X25519 public key>", "api": 1, "desktop": { "version": "0.7.0" } }` |

`pub` is the public half of an X25519 key pair, 32 bytes as base64url. Each side makes a new pair for every hello it
sends, keeps the private half in memory for that handshake only, and never saves it. The computer ignores a hello
whose `pub` is missing or gives no shared secret.

If the versions cannot work together, the computer answers `{ "v": 1, "error": "update_phone" }` or
`{ "v": 1, "error": "update_desktop" }` and no session starts.

Both sides then derive two keys:

```
shared   = X25519( own private key, the other side's pub )
salt     = SHA-256( phone hello JSON bytes || computer hello JSON bytes )
prk      = HKDF-Extract( salt, key || shared )
kPhone   = HKDF-Expand( prk, "omegastar-remote v1 phone",  32 )    seals phone -> computer
kLaptop  = HKDF-Expand( prk, "omegastar-remote v1 laptop", 32 )    seals computer -> phone
```

The hello bytes are the JSON exactly as sent, without the type byte. A new phone hello at any time starts a new
session and discards the old keys.

### Sealed frames

```
0x02 || AES-256-GCM( key = kPhone or kLaptop, nonce, aad, plaintext )
```

- **Nonce:** 12 bytes. Four zero bytes, then a counter as an unsigned 64-bit big-endian number. Each direction has
  its own counter, starting at 0 for each session and going up by one per frame.
- **AAD:** the room id as its 43 ASCII characters.
- **Plaintext:** one UTF-8 JSON message (section 8).
- **Output:** ciphertext followed by the 16-byte tag.

A frame that fails to open ends the session. The computer drops its keys and closes its socket, then reconnects with
the usual backoff, so the phone hears `left` and then `joined` and starts a new handshake. After three failures in a
row the phone shows **Pair again**.

### Why this is enough

- The relay never has `key`, so it cannot read or write frames.
- Each session mixes in fresh randomness from both sides, so frames recorded from an old session cannot be replayed.
- Counters stop frames being replayed or reordered within a session.
- The hello frames are not sealed, but they feed the key derivation, so changing them only makes the session fail.
  That covers `pub` too: a relay that swaps in its own public key gets a session neither side can use, since it
  lacks `key`.
- A session's keys need `shared`, and so one of the two private halves, which are gone once the handshake is done.
  Someone who records the traffic and later gets `key`, or the code of a pairing, still cannot open what they
  recorded. That holds for the pairing room as well, where the answer to `pair` carries the long-term `key`.

Known limit: `key` itself does not change. Someone who holds it can join the room as the phone for as long as the
pairing lasts. Section 14 covers a stolen key.

## 8. Messages inside sealed frames

**Request**, phone to computer:

```json
{ "t": "req", "id": 12, "cmd": "answer_card", "args": { "runId": "...", "card": 0, "answer": { "type": "choice", "index": 1 } }, "op": "5f0c..." }
```

**Response**, computer to phone:

```json
{ "t": "res", "id": 12, "ok": true, "ret": [ ] }
{ "t": "res", "id": 12, "ok": false, "err": { "code": "invalid", "message": "That card was already answered." } }
```

**Event**, computer to phone:

```json
{ "t": "evt", "name": "changed", "what": ["inbox"] }
```

Rules:

- `id` is a number the phone picks, unique within the session. The response carries the same `id`.
- `err` has the shape of OmegaStar's existing `ApiError`. The remote layer adds three codes: `not_allowed` (the
  command is not in the list), `bad_request` and `too_large`.
- **Writes carry `op`,** a UUID the phone creates once per user action. The computer remembers the results of the
  last 200 ops for 10 minutes, failures included. If the same `op` arrives again, it returns the saved result without
  running the command again. This makes it safe for the phone to resend a write whose response was lost. A write
  without `op`, or whose `op` is not a UUID, gets `bad_request`.
- One message is at most 4 MiB once sealed. The computer answers `too_large` if a result would not fit, and keeps
  `too_large` as a write's saved result, never the result itself.

### Compatibility

The remote API has one version number, `api`, starting at 1.

- Adding a command, or adding a field to a result, does not change the number.
- Removing or changing a command, or a field the phone reads, raises the number.
- The phone ignores fields it does not know. For a type it does not know, such as a new kind of card, it shows
  **Open this on your computer**.
- OmegaStar keeps a snapshot test of the JSON shape of every remote command, so a change to those shapes cannot slip
  through unnoticed.

## 9. Remote API, version 1

Only these commands are accepted. Anything else returns `not_allowed`. Type names refer to
[`src/api/types.ts`](../src/api/types.ts).

### Read

| Command | Arguments | Result | Notes |
|---|---|---|---|
| `inbox` | none | `{ threads: Thread[] }` | New. Only the threads that lead an Inbox row, taken from `Office::conversations`. |
| `proposals` | none | `HireProposal[]` | Open proposals only. |
| `team` | none | `{ id, name, title, hue, hasImage }[]` | New. You, employees and people, without the image data. |
| `member_image` | `{ id }` | data URL | New. One member's picture. The phone caches it. |
| `thread` | `{ runId }` | `Thread` | Existing. |
| `card_file` | `{ runId, card }` | text | Existing. Up to 200 KB. |
| `card_image` | `{ runId, card, index }` | data URL | New. Reads only an image listed on that card. Refuses files over 2.5 MB. |

### Write (each carries `op`)

| Command | Arguments | Result | Notes |
|---|---|---|---|
| `answer_card` | `{ runId, card, answer: CardReply }` | `Run[]` | Existing. Includes Later, through `{ type: "later", until }`. |
| `reply_to_run` | `{ runId, text, archive }` | `Run` | Existing, always with no attachments. |
| `approve_blocked` | `{ runId, rules, always }` | `Run` | Existing. Allow once or Always allow. |
| `approve_tidy` | `{ runId }` | `Run` | Existing. |
| `approve_proposal` | `{ proposalId, note }` | `Employee` | New wrapper. Hires exactly as proposed, using the draft the computer already holds. |
| `decline_proposal` | `{ proposalId, note }` | `HireProposal` | Existing. |
| `set_archived` | `{ runId, archived }` | `string[]` | Existing. Done. |
| `unarchive_runs` | `{ runIds }` | `null` | Existing. Undo for Done. |
| `retry_run` | `{ runId }` | `Run` | Existing. |

### Session

| Command | Arguments | Result | Notes |
|---|---|---|---|
| `pair` | `{ deviceName, platform }` | `{ roomId, key, companyName }` | Pairing room only. |
| `push_register` | `{ platform, token, sandbox }` | `null` | Sent after a handshake whenever the token is new or changed. |
| `unpair` | none | `null` | The phone wipes its copy after the response. |

### Events

| Event | Meaning |
|---|---|
| `changed` with `what: ["inbox" \| "proposals" \| "team"]` | Fetch those again. The computer sends at most one per second, combining what changed. |
| `unpaired` | The computer removed the pairing. Wipe and show the pairing screen. |

The phone never works out for itself what belongs in the Inbox. It asks.

## 10. Push notifications

**Registering.** The phone gets its device token from Apple or Google and sends `push_register` after the next
handshake. The computer saves the token with the pairing, and answers with an error when it cannot.

**Sending.** The desktop already decides when a notification is shown, after batching and quiet hours, in
`TauriSink::show` in [`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs). That same point also sends the push, so the
phone follows the desktop's notification settings. A switch in Settings turns phone pushes off. A computer that
another has replaced in the room sends none.

```
POST https://<relay>/v1/push
{ "platform": "ios", "token": "<device token>", "sandbox": false, "title": "Juno needs you" }
```

| Response | Meaning | What the desktop does |
|---|---|---|
| 204 | Sent | Nothing |
| 410 | The token is no longer valid | Forgets the token. The phone registers again at its next session. |
| 429 | Too many pushes | Drops this one |
| Other | Failed | Logs it and drops this one. No retry queue. |

**What the push says.** The title is the desktop notification's title, at most 120 characters. The body is always
"Open OmegaStar to reply." The desktop notification's own body is never sent. The title is the only content that
leaves the computer unsealed: the relay, and Apple or Google, can see it.

**Tapping it** opens the app at the Inbox. Opening the exact item needs the thread id passed through
`EventSink::notify`, which is left for later.

**How the relay sends it:**

| | iOS | Android |
|---|---|---|
| Endpoint | `https://api.push.apple.com/3/device/<token>`, or `api.sandbox.push.apple.com` when `sandbox` is true | `https://fcm.googleapis.com/v1/projects/<project>/messages:send` |
| Auth | A JWT signed with ES256 from the APNs key. Header `kid` is the key id, claim `iss` is the team id. | An access token from Google, obtained with a JWT signed with RS256 from the service account, scope `https://www.googleapis.com/auth/firebase.messaging` |
| Headers | `apns-topic: <bundle id>`, `apns-push-type: alert`, `apns-priority: 10` | none extra |
| Body | `{ "aps": { "alert": { "title": "...", "body": "..." }, "sound": "default" } }` | `{ "message": { "token": "...", "notification": { "title": "...", "body": "..." }, "android": { "priority": "HIGH" } } }` |

Both signing steps use the Worker's built-in Web Crypto. The APNs JWT is reused for 45 minutes and the Google access
token until shortly before it expires, each held in the Worker's memory.

## 11. The relay

A Cloudflare Worker using Hono, TypeScript, Wrangler and Vitest, the same stack as the Vibe Typer backend.

### Routes

| Route | Purpose |
|---|---|
| `GET /v1/room?role=` | Checks the upgrade header, the role, and that the bearer value is 43 base64url characters. Applies the connection rate limit. Hands the request to the room: `env.ROOM.getByName(roomId).fetch(request)`. |
| `POST /v1/push` | Checks the body, applies the push rate limit, sends to Apple or Google. A plain Worker request that never touches a Durable Object. |
| `GET /health` | Returns 200. |

### The room

One Durable Object class, `Room`, named by `roomId`. A sketch of everything it does:

```ts
import { DurableObject } from "cloudflare:workers";

const MAX_BYTES = 4 * 1024 * 1024;
type Role = "laptop" | "phone";
const other = (role: Role): Role => (role === "laptop" ? "phone" : "laptop");
const peer = (state: "joined" | "left") => JSON.stringify({ t: "peer", state });

export class Room extends DurableObject<Env> {
  async fetch(request: Request): Promise<Response> {
    const role = new URL(request.url).searchParams.get("role") as Role;
    const [client, server] = Object.values(new WebSocketPair());

    for (const old of this.ctx.getWebSockets(role)) old.close(4001, "replaced");
    this.ctx.acceptWebSocket(server, [role]);          // the hibernating kind, never server.accept()

    const others = this.ctx.getWebSockets(other(role));
    if (others.length > 0) server.send(peer("joined"));
    for (const ws of others) ws.send(peer("joined"));

    return new Response(null, { status: 101, webSocket: client });
  }

  webSocketMessage(ws: WebSocket, message: ArrayBuffer | string) {
    if (typeof message === "string") return;           // clients send binary only
    if (message.byteLength > MAX_BYTES) return ws.close(1009, "too large");
    const [role] = this.ctx.getTags(ws) as Role[];
    for (const to of this.ctx.getWebSockets(other(role))) to.send(message);
  }

  webSocketClose(ws: WebSocket) {
    const [role] = this.ctx.getTags(ws) as Role[];
    const replaced = this.ctx.getWebSockets(role).some((s) => s !== ws && s.readyState === 1);   // 1 = open
    if (replaced) return;                              // a newer socket took this role
    for (const to of this.ctx.getWebSockets(other(role))) to.send(peer("left"));
  }
}
```

The sending-rate cap (300 frames per 10 seconds per socket, then close with 1008) is a counter held in memory and is
left out of the sketch.

### What the room must never do

These are what keep it free while idle. Any one of them stops it hibernating, and it is then billed for every second
a computer is connected.

- Never call `server.accept()`. Only `this.ctx.acceptWebSocket()`.
- No `setTimeout`, `setInterval` or alarms.
- No `fetch` or other outgoing connection from inside the room. Pushes go through the plain Worker route.
- No storage reads or writes.
- No work in the constructor.

### Configuration

```jsonc
{
  "name": "omegastar-relay",
  "main": "src/index.ts",
  "compatibility_date": "2026-11-01",
  "durable_objects": { "bindings": [{ "name": "ROOM", "class_name": "Room" }] },
  "migrations": [{ "tag": "v1", "new_sqlite_classes": ["Room"] }]
}
```

Plus two rate-limit bindings, set up the way the Vibe Typer backend's are: connections keyed by client address (60 a
minute), and pushes keyed by device token (30 a minute).

Secrets: `APNS_KEY_P8`, `APNS_KEY_ID`, `APNS_TEAM_ID`, `APNS_BUNDLE_ID`, `FCM_SERVICE_ACCOUNT`.

### Limits

| Limit | Value | Source |
|---|---|---|
| Sealed frame size | 4 MiB | Ours. Cloudflare allows 32 MiB. |
| Sockets per room | 2, one per role | Ours |
| Frames per socket | 300 per 10 seconds | Ours |
| Connections per address | 60 a minute | Ours |
| Pushes per device token | 30 a minute | Ours |
| Push title | 120 characters | Ours |

### Logging

Log counts and outcomes only. Never log a room id, a push token or a push title.

## 12. Desktop changes

All in this repository, behind one setting that is off until the user pairs a phone.

### New module: `src-tauri/src/remote/`

| File | Job |
|---|---|
| `mod.rs` | The `Remote` handle and its task: connect, reconnect, pairing, sessions |
| `wire.rs` | Hello frames, key derivation, sealing and opening frames |
| `api.rs` | The list of allowed commands and the `match` that calls the `Office` |
| `push.rs` | The `POST /v1/push` call |
| `store.rs` | Reading and writing the saved pairing |

**States:** `off` -> `pairing` -> `connecting` -> `online` (no phone) -> `session` (phone connected). A phone's
hello only derives the keys; `session`, and the saved `lastSeenAt`, wait for its first sealed frame to open.

**Start-up.** The sink is created before the `Office`, and the remote needs both, so:

1. Create the `Remote` handle first. It is only channels at this point.
2. Give a clone to `TauriSink`.
3. After `Office::open`, call `remote.start(office.clone())`, beside the existing `office.spawn_*` calls in
   [`lib.rs`](../src-tauri/src/lib.rs). It connects only if a pairing is saved.

It never starts in a sandbox office.

**Hooks in `TauriSink`:**

| Existing method | Added call |
|---|---|
| `run_updated` | `remote.changed(Inbox)` |
| `proposal_updated` | `remote.changed(Proposals)` |
| `org_changed` | `remote.changed(Team)` and `remote.changed(Inbox)`: who is in the Inbox follows the team |
| `show` | `remote.push(title)` |

`remote.changed` does nothing unless a phone session is live, so an idle computer sends nothing to the relay.

**Phone activity.** Each session the phone starts, and each change it makes (a write that worked, counted once per
`op`), is kept with the pairing as `{ at, what }`, the latest 10, and written to the log, which outlasts the pairing.
`what` is `connected`, the write's command (`set_archived` to false counts as `unarchive_runs`), or `push_register`
when the phone changes where its pushes go. Settings lists them under the paired phone, so use of the link that is
not the owner's shows on the computer.

**Commands.** `api.rs` calls the same `Office` methods as the adapters in
[`commands.rs`](../src-tauri/src/commands.rs). Four reads are new and belong on the `Office`: `inbox`, `members`
(the `team` command; `Office::team` already exists), `member_image` and `card_image`, plus `company_name` for the
`pair` answer and `approve_proposal_as_proposed` for `approve_proposal`. `card_image` must take the path from the card
itself, never from the phone. `card_image` and `card_file` read a regular file only, through the one handle they
checked, opened without waiting so a pipe in its place is refused, and stop at their size limit. Reads run off the link's
task, on a blocking thread, and are answered with an error after 10 seconds, so a slow disk never holds up the link.

**The saved pairing** lives in `remote.json` in the office folder:

```json
{ "relay": "relay.example.com", "roomId": "...", "key": "...", "deviceName": "Maren's iPhone",
  "pairedAt": "2026-11-18T09:00:00Z", "push": { "platform": "ios", "token": "...", "sandbox": false },
  "pushOn": true, "lastSeenAt": "2026-11-18T09:05:00Z",
  "activity": [{ "at": "2026-11-18T09:05:00Z", "what": "connected" }] }
```

- Written with owner-only permissions where the system supports it.
- Left out of backups, with its temporary `remote.json.partial`, by adding it beside the database files that
  [`backup.rs`](../src-tauri/src/backup.rs) already skips at the top of the office folder. A restored backup must
  not bring a second computer into the same room.
- Left out of sandbox copies too ([`office-copy.sh`](../scripts/office-copy.sh)). Whoever is given a copy must not
  be able to act as the phone in the office it was copied from.
- Not in the system keychain. Release builds are not signed with a stable identity, so the keychain would ask again
  after every update.

### New dependencies

| Crate | Why |
|---|---|
| `tokio-tungstenite`, with rustls | The WebSocket client. Use the `ring` provider, as [`proxy.rs`](../src-tauri/src/proxy.rs) already installs it. |
| `ring`, as a direct dependency | AES-GCM, HKDF, SHA-256 and secure random. Already in `Cargo.lock` through rustls. |
| A QR generator | To draw the pairing code, in Rust or in the interface |

### Interface

New Tauri commands, added to `Commands` in [`types.ts`](../src/api/types.ts):

| Command | Result |
|---|---|
| `remote_status` | `{ state, deviceName, pairedAt, lastSeenAt, pushOn, error, code, activity }`; `code` is the pairing code while one shows, so Settings shows it again when you come back, and `activity` is what the phone did lately, newest first |
| `remote_pair_start` | `{ qr, link, expiresAt }`; `link` is what the QR code holds |
| `remote_pair_cancel` | `null` |
| `remote_unpair` | `null` |
| `remote_set_push` with `{ on }` | the status, or an error when it cannot be saved. Off still holds until restart. |

New event: `remote:updated`, carrying the status.

A new **Phone** section in Settings, beside the existing ones in `src/screens/settings/`. It shows the state, the QR
code while pairing (blurred until clicked), the paired phone's name, the push switch, what the phone did lately and
**Unpair**. The browser preview in `src/preview/` needs the same commands with invented data.

## 13. What the phone app must do

- **Store** `relay`, `roomId` and `key` in the Keychain on iOS and Keystore-backed storage on Android, readable only
  on that device.
- **Connect** when the app comes to the foreground, and disconnect when it leaves.
- **Make a new X25519 key pair for every hello**, and never store its private half.
- **Raise the incoming message limit** of its WebSocket client to at least 4 MiB.
- **Show one of these states** at all times: not paired, connecting, computer offline, connected, update needed,
  pair again.
- **Fetch, don't compute.** On connect and on each `changed` event, fetch `inbox`, `proposals` or `team` again.
- **Give every write an `op`** and reuse the same `op` when resending after a lost connection.
- **Decode tolerantly**, as section 8 describes.
- **Register for push** and send `push_register` when the token is new or changed.

Screens for version 1: pairing scanner, Inbox list, item (report, cards and their actions), reply, blocked tool,
hire proposal, settings.

## 14. Security and privacy

| Threat | What stops it |
|---|---|
| The relay, or Cloudflare, reads content | Everything is sealed with a key the relay never has |
| Someone records the traffic and later gets `key` or a pairing code | Each session's keys also need a private key that lived only for that handshake |
| The relay replays, reorders or alters frames | Fresh keys per session, counters, and authenticated encryption |
| A stranger joins a room | They would need the 256-bit room id. Even with it they cannot read or write frames. The worst they can do is knock a client off, which reconnects. |
| The QR code is seen or photographed | Settings blurs it until clicked, so it is on screen only when the user chooses. It works once, for 5 minutes, and whoever scans it first is paired; the desktop shows which phone that was. After that it opens nothing. |
| The phone is lost | Unpair on the desktop. The room and key are abandoned. |
| The phone app is taken over | It can do what the API allows and nothing else: no file reads, no settings, no skills, no arbitrary commands. It can allow tools and approve hires, so an app lock is worth adding later. |
| Someone reads the computer's disk | `remote.json` is as exposed as the office database beside it. Whoever copies it can act as the phone until Unpair. Settings lists what the phone did lately and the log keeps it, so that use shows on the computer. |
| Strangers use the relay for free or to run up a bill | Rate limits, the frame cap, two sockets per room |

**What the relay can see:** client addresses, room ids, when frames pass and how large they are, and for a push the
device token and the title.

**What it cannot see:** anything in the Inbox, any answer, any name beyond what a push title contains.

The README's promise of no cloud service needs a line for this opt-in feature.

## 15. When things go wrong

| Situation | Behaviour |
|---|---|
| Computer asleep, off, or OmegaStar closed | The phone connects, hears no `joined`, and shows **Your computer is offline**. It updates the moment `joined` arrives. |
| Relay unreachable | Both sides retry with backoff. The phone says it cannot reach the relay. |
| Relay redeployed | Sockets drop. Both sides reconnect, spread out by the jitter. |
| A frame fails to open | The computer reconnects, and the phone starts a new handshake on `joined`. Three in a row: **Pair again**. |
| Versions too far apart | The hello answers with `update_phone` or `update_desktop`, and the phone says which to update. |
| A write's response is lost | The phone resends with the same `op`, or fetches the Inbox again. |
| The item changed in the meantime | The `Office` returns its usual error. The phone shows the message and fetches again. |
| Push token no longer valid | The relay answers 410, the desktop forgets the token, the phone registers again later. |
| The desktop is unpaired while the phone is away | The phone finds the room empty forever and shows the computer as offline. Pairing again fixes it. |

## 16. Cost guardrails

- **The room hibernates.** The rules in section 11 are the whole cost story. Idle time is free only while they hold.
- **An idle computer is silent.** Keep-alive pings are free, and the desktop sends frames only during a phone session.
- **Caps** on frame size, frame rate, sockets per room and connections per address.
- **Measure before launch.** Run three test computers for a week on the free plan and read the Durable Objects
  duration figure in the Cloudflare dashboard. It should be close to zero.
- **Watch after launch.** A billing alert, and a weekly look at duration per connected room.

Expected bill: about $5 a month until several thousand customers, on the earlier estimates.

## 17. Testing

- **Shared test vectors.** One JSON file with a fixed key, both sides' private keys, both hello frames, the shared
  secret, the two derived keys and a few sealed frames. The Rust, iOS and Android code must all reproduce it.
- **Relay.** Vitest in the Workers pool: two clients in a room, binary frames forwarded, text ignored, `joined` and
  `left`, replacement, the size and rate caps, and the push route against mocked Apple and Google.
- **Desktop.** Unit tests for `wire.rs`. A fake relay in the test process to drive pairing, handshake, a request and
  an event. A test that every command outside the list is refused. The JSON shape snapshot from section 8. Invented
  names and data only.
- **A fake phone.** A small command-line client that pairs and sends requests, so the desktop and relay can be
  proven end to end before the app exists.

## 18. Build order

1. **Relay.** Worker, room, tests. Deploy to the free plan.
2. **Desktop core.** `wire.rs`, the connection, pairing and `api.rs`, tested with the fake phone.
3. **Measure hibernation** for a week. This can run alongside step 4.
4. **Desktop interface.** The Phone section in Settings.
5. **Phone app, read only.** Pairing and the Inbox.
6. **Phone app, actions.** Answers, replies, approvals.
7. **Push.** The relay route, registration, and the hook in `show`.
8. **Launch guards.** Rate limits switched on, alerts set.

## 19. To check while building

These are assumptions in this design that were not confirmed against documentation or code:

- Whether `webSocketClose` fires for a socket the room itself closed, which the replacement logic relies on.
- That Cloudflare's request logs do not record the `Authorization` header.
- That several Worker instances each signing their own APNs token does not trigger Apple's limit on token refreshes.
  If it does, share the token through a cache.
- The default incoming message limit of each phone WebSocket client.
- Which Android versions have X25519 built in, and which library to use on the ones that do not.

## 20. Decisions still open

| Decision | Proposed |
|---|---|
| How the phone apps are built | Not decided: native Swift and Kotlin as for Vibe Typer, or one shared codebase |
| What a push says | The notification title only, as in section 10 |
| Keep the computer awake while a phone is paired | No for version 1 |
| Where the relay lives | A private repository, at `relay.abramindustries.com` |
| Phones per computer | One |
| How the app is sold | Paid upfront, which needs no accounts |
