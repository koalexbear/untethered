# Moving the Matrix engine from gomuks to matrix-rust-sdk

Untethered talks to Matrix through gomuks: a Go child process on the desktop
(`src/backend/native.rs`) and a WASM worker on the web (`src/backend/web.rs`,
`web/worker/`). Everything crosses that boundary as `serde_json::Value`. This
document records the first step of replacing it with
[matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk) and maps
what is left.

Planned order: core sync and timeline → account and verification →
messaging features on the SDK Timeline → media and send queue →
notifications, moderation, permissions and spaces → call signaling → web
build → remove gomuks. Throughout, the UI keeps reading
`rooms::{Rooms, Room, Post}`; that model is the seam.

## Status: steps 1 to 4 are in

The native client has a second engine built on `matrix-sdk` and
`matrix-sdk-ui` **0.20.0**. gomuks stays the default. The wasm32 build doesn't
compile the SDK.

**0.20.0 since step 4.** Steps 1 to 3 stayed on 0.19.1 because 0.20.0 failed
the restart check every time. That failure was the engine's own pagination
race (see [restart history](#restart-history-check-corrects-steps-1-and-2)),
which 0.20.0's timing hit more often. With the race fixed, 0.20.0 passed the
round trip 10 times in a row, then the full acceptance script. The upgrade
needed four small API changes. The call port needs APIs that only 0.20 has.

```sh
UNTETHERED_ENGINE=matrix-sdk bash scripts/rust-client.sh
```

Any other value, or no value, starts gomuks. An unknown value also prints a
warning. The variable is read at startup, so one binary can run either
engine for side-by-side testing.

**Why an environment variable rather than a Cargo feature:** QA and
day-to-day comparison need both engines in the same build, and the default
must stay gomuks. The cost is that every native build compiles the SDK (see
[measurements](#c-build-time-and-binary-size)). Make it a feature only if
that cost blocks someone before the SDK becomes the default.

| Flow | Implementation | State |
| --- | --- | --- |
| Password login (existing account form) | `matrix_auth().login_username()`, device name "Untethered" | Done |
| Session persisted in `UNTETHERED_PROFILE` and restored | `matrix-sdk/session.json` (mode 0600) + SQLite stores encrypted with a random passphrase; `Client::restore_session` with the saved homeserver URL, no network needed | Done |
| Recovery key or passphrase (existing verify form) | `encryption().recovery().recover()`; waits until the device is cross-signed; errors map to the existing messages | Done |
| Sync | `SyncService` (Simplified Sliding Sync, offline mode, restarts after errors) | Done |
| Rooms, DMs, spaces, unread/highlight counts | Room snapshots from the SDK store, coalesced per sync response; `m.space.child` from local state | Done |
| Room timeline mapped into `Post`s, back-pagination | matrix-sdk-ui `Timeline` per opened room, snapshot per diff batch; `paginate_backwards(50)` | Done |
| Sending text | `Timeline::send` / `send_reply` (plain text) | Done |

### How it fits together

- `src/engine/mod.rs`: `Command`/`Event` types and the engine loop. It
  owns the `matrix_sdk::Client` on the Iced tokio runtime, like
  `backend::run`, but with typed messages. Requests carry the `App::pending`
  id and are answered by exactly one `Event::Done { id, result }`.
- `src/engine/session.rs`: profile directory (the same `rust-client.lock`
  as gomuks, so only one engine can open a profile at a time), session file,
  login, restore and recovery.
- `src/engine/rooms.rs`: joined-room summaries and space edges.
  `src/engine/timeline.rs`: timelines, mapping, pagination and sends. The
  room list syncs the newest 20 events of every room, so opened timelines
  stay live without per-room subscriptions (see the sliding sync
  recommendation for why).
- `src/engine/app.rs`: the App side. It starts the engine, routes
  login/verify/paginate/send to typed commands, and applies events. It reuses
  `App::pending` and the existing `Request` variants, so the view's busy
  states work unchanged.
- `src/rooms/engine.rs`: typed updates into `Rooms`/`Room`/`Post`. The
  gomuks path keeps `Rooms::sync`/`Room::insert`; neither path needed a new
  model field.
- Shared changes: `client_state` handling moved into
  `account::apply_account`, which both engines call.
  `edits::Revision::applied` shows the "edited" label for edits the SDK has
  already applied. `main.rs` has a few cfg-gated hooks and adds no view code.

Things worth knowing:

- **TLS:** matrix-sdk's default TLS feature adds aws-lc-rs beside the ring
  provider that livekit and reqwest 0.12 already use. With both compiled in,
  rustls can't pick a process default, and anything relying on one (such as
  tokio-tungstenite's connector under livekit) panics unless the app installs
  one. The engine therefore builds reqwest 0.13 with `rustls-no-provider` and
  installs ring. This keeps one TLS stack and avoids a C build of aws-lc,
  including in the Windows cross-build.
- **`decancer`:** matrix-sdk-base depends on it. Its `impl Add<_> for String`
  breaks `String + &String` inference everywhere in the crate, which is why
  `backend/native.rs` now uses `.as_str()`.
- **`#![recursion_limit = "256"]`:** rustc needs it to prove the SDK's
  futures are `Send`.
- **Homeserver requirement:** sign-in checks `/versions` and refuses
  servers without Simplified Sliding Sync, with a message pointing back to
  gomuks.
- **Session secrets:** `session.json` holds the access token and the store
  passphrase (mode 0600, in a 0700 directory). That matches the trust
  level of gomuks's profile. Moving the passphrase to the OS keyring is
  listed under next steps.
- **Unsupported requests:** gomuks RPCs this engine doesn't implement yet
  never reach a backend. Background reads (media config, members) are
  skipped; user actions show "This isn’t available with the matrix-sdk engine
  yet." The mentions, search, report and community setup flows get their
  normal failure handling.

Known differences on the new engine, each owned by a later step:
- Unread badges don't clear, because no receipts are sent yet.
- There are no desktop notifications.
- Reaction and edit controls are hidden.
- Attachments show as "Attachment · name", and staged files can't be sent.
- Sends are plain text; gomuks renders Markdown.
- Calls, moderation, permissions, search and the mentions inbox are
  unavailable.

## Step 2: account and verification

Also on the new engine only. gomuks, its screens and its labels behave as
before.

| Flow | Implementation | How it was verified |
| --- | --- | --- |
| Emoji (SAS) verification with another of the account's devices: ask from "Verify this device", or accept a request | `UserIdentity::request_verification_with_methods([SasV1])`; incoming requests arrive through a to-device event handler; one flow at a time. Once our identity is verified, matrix-sdk-crypto requests the cross-signing and backup keys from the other device, so this device ends up cross-signed | Two engine instances against Synapse showed the same seven emoji, and the new device ended up cross-signed |
| Recovery setup for an account without keys | `bootstrap_cross_signing` (the password goes out only if the server asks for it), then `recovery().enable().wait_for_backups_to_upload()`. The key is shown once and never stored | A fresh account got a key and a verified device; a second device verified with that key |
| Change recovery key | `recovery().reset_key()`, or `enable()` if a verified device never set recovery up | The new key verified a new device; the old key was refused |
| Reset encryption | `recovery().reset_identity()` with the password, then a new recovery key | New identity and key; the earlier device then reported itself unverified |
| Devices | `Client::devices` plus `get_user_devices` for the cross-signing state; `delete_devices` with the password | The new device showed as verified; a wrong password was refused; removing it signed that device out |
| Sign out | `Client::logout`, then the session file, stores and keyring entry are removed | Nothing restored afterwards |
| Store passphrase in the OS keyring | keyring-core with the Secret Service store (Linux, over zbus) or the Credential Manager store (Windows); falls back to the 0600 file. `UNTETHERED_KEYRING=off` forces the file | Round trip and restart against a private GNOME Keyring on a private session bus; the account test ran with the keyring off |
| Single sign-on | `get_sso_login_url`, a loopback redirect on 127.0.0.1, then `login_token` | Synapse's CAS login against a mock CAS server in the QA script; the session restored after a restart |
| OAuth 2.0 / MAS | Dynamic client registration as a native app with a loopback redirect, `oauth().login()` and `finish_login`; refreshed tokens are saved | **Not tested end to end.** MAS needs PostgreSQL, which this machine doesn't have. Tested: discovery reports no OAuth on plain Synapse; the loopback listener returns the redirect; only http(s) URLs reach the browser |

### Screens, and merging with the Discord-style pass

- **New screens** are in `src/view/sdk_account.rs`: emoji comparison, the
  one-time recovery key, verification choices under the recovery form,
  Encryption/Devices/Session groups on the account page, and "Other ways to
  sign in".
- **Shared helpers only:** they import just what both layouts share
  (`surface`, `label`, the base colours, `El`, `BOLD`) and keep local copies of
  the Discord-style button and field styles. The file compiles unchanged on
  `discord-ui-pass`.
- **Hooks in existing files:**
  - `view.rs`: a module line; a full-window screen at the top of `view()` for
    verification and the recovery key; the gomuks emoji note behind
    `self.sdk.is_none()`; and one block after the account form that adds the
    SDK sections.
  - `main.rs`: the `sdk_ui` field, `Msg::Sdk`, and one match arm.
- **On `discord-ui-pass`,** the account-page sections become extra groups in
  `settings_view`, and the login choices go under its Sign in button.
- **No gomuks label changed.**

The live tests drive these screens' state and actions. Only the "Verify this
device" screen was also checked in the running app, in a private virtual
KWin. The other screens were not rendered, because nothing could click in
that compositor.

### Things worth knowing

- **Own devices only:** verification covers just the user's own devices.
  Requests from other people, and a second request while one runs, are
  ignored rather than cancelled, because a cancel from this device would also
  end the request for the user's other clients.
- **No QR codes:** QR verification isn't offered. If the other device
  switches to it, the flow stops with a message.
- **Secrets:** the recovery key stays in App memory only until "I’ve saved
  it". Actions that need the account password take it from a field on the
  screen and clear it after each request.
- **Session file format 2** records whether the session is a Matrix or an
  OAuth session, and where the passphrase is (`"keyring"` with the entry
  name, or `"passphrase"`). Step-1 files (format 1) still restore, with their
  passphrase in the file.
- **Keyring entries** are named "matrix-sdk store <user> <device>" under the
  service "Untethered".
- **Browser sign-in** opens `xdg-open` (or `rundll32` on Windows) without a
  shell, and only for http(s) URLs.
- **OAuth client URI:** client registration uses
  `https://github.com/koalexbear/untethered` as the client URI. Change it if
  the project gets a homepage.
- **MAS accounts:** resetting encryption on an OAuth (MAS) account needs
  approval in the browser. That isn't wired yet, and the screen says so.
- **Test shutdown noise:** when a test ends, the tokio runtime cancels SDK
  background tasks that still hold the SQLite store, and matrix-sdk-sqlite
  panics on the cancelled pool task ("Cancelled"). The tests still pass. The
  SDK offers no shutdown to wait for.

### Restart history check (corrects steps 1 and 2)

The earlier diagnosis was wrong: the SDK doesn't lose history after a
restart. Traces from the live test (`UNTETHERED_QA_LOG`, see
[Verification](#verification)) show the event cache loading the older events
from its store and reporting the room start correctly.

The fault was in this engine. `Timeline::paginate_backwards` returns once
the event cache has the events, but the Timeline adds them to its items
later, on its own task. The engine reported "start reached" as soon as the
call returned, so the App could be told the room started above a hole. The
next snapshot filled the hole in, but the test had already checked, and the
UI would already have hidden "Earlier messages". Events from the store
arrive faster than from the network, which is why it showed up after a
restart.

Fixed in step 3 (`src/engine/timeline.rs`): when a back-pagination reaches
the start, the engine waits until the oldest events the event cache holds
show in the Timeline, then records the Timeline's own start item. A
snapshot reports the start only while that item is present, so a timeline
that the SDK clears drops it too.

Two rarer failures turned up while looping the test:
- **"history out of order"** was the fixture. gomuks sent the 55 seed
  messages in parallel, so the server sometimes stored "history 13" before
  "history 12", and the SDK showed the server's order. The QA script now
  waits for each seed message before sending the next.
- **"engine exited"** right after the restart: the stopped engine's session
  tasks were aborted but not awaited, so one could still hold the profile
  lock when the new engine opened the profile. Shutdown, sign-out and a
  rejected token now wait for those tasks.

### Step 2 verification

On this branch, matrix-sdk 0.19.1, Synapse 1.166.0:
- `cargo test` (54 unit tests);
- `scripts/qa_matrix_sdk.py`, which runs both live tests. One full pass:

```text
Synapse 1.166.0; Simplified Sliding Sync advertised: True
Private GNOME Keyring on a private session bus
matrix-sdk: password login as @sdk479421:test.invalid
matrix-sdk: store passphrase saved in the Secret Service keyring
matrix-sdk: recovery key verified this device; encrypted room synced
matrix-sdk: space child edge, DM and highlight count (1) mapped
matrix-sdk: back-paginated to the room start (20 then 55 posts); 55 of 55 earlier messages decrypted from key backup
matrix-sdk: sent an encrypted message; waiting for the gomuks peer
gomuks peer decrypted the matrix-sdk message $Fi8mm8QB2tjXu0Js1FMXknXxTl61dDg5CTyN5AEuJPM
matrix-sdk: decrypted and rendered the gomuks peer's message
matrix-sdk: restart restored the session and both encrypted messages
matrix-sdk: both devices show 🏁 Flag, 🍕 Pizza, 🌽 Corn, 🦋 Butterfly, ☁️ Cloud, 🔑 Key, 🔥 Fire
matrix-sdk: emoji verification cross-signed the new device
matrix-sdk: device list showed it verified; removing it (password checked) signed it out
matrix-sdk: recovery setup created keys, verified the device and showed a key once
matrix-sdk: the changed key verifies a new device and the old one doesn't
matrix-sdk: sign-out removed the session and stores
matrix-sdk: reset encryption gave a new identity and recovery key
matrix-sdk: the earlier device reports itself unverified after the reset
matrix-sdk: single sign-on ([("cas", "CAS")]) signed in through the loopback redirect and restored
PASS matrix-sdk engine: login, recovery, sync, pagination, encrypted send/receive, restart, keyring, emoji verification, devices, recovery setup/change/reset and sign-out
```

The last step-2 run passed the account test and failed only the restart
history check. That was the engine's pagination race, fixed in step 3 (see
above).

## Step 3: messaging on the SDK Timeline

The App still builds these requests as gomuks RPCs at their existing call
sites. On the new engine, `engine_rpc` (`src/engine/app.rs`) turns them into
typed commands, so `reactions.rs`, `edits.rs`, `reports.rs`, `reading.rs` and
`replies.rs` keep one code path for both engines. The message toolbar's
React, Edit and Report controls now appear on the new engine.

| Feature | Implementation | How it was verified |
| --- | --- | --- |
| Reactions | `Timeline::toggle_reaction`. Chips come from the Timeline's aggregation, including whether one is ours | Our chip showed one reaction, ours; the gomuks peer decrypted the reaction |
| Edits | `Timeline::edit` with Markdown. A post carries the newest edit's event ID, which reports refer to. Only messages the SDK lets us edit get the Edit control | The peer decrypted the edit's `m.new_content`; the edited text survived a restart |
| Reports | `Room::report_content`, with the reason if one was given | Synapse's `event_reports` holds the report and its reason |
| Read state | One request for the read receipt (public or private) and the fully-read marker; the marked-unread flag; other people's receipts from the Timeline; Timeline order for the automatic read marker; the read-sharing choice as account data (`com.untethered.read_receipts.v1`, the same key as on gomuks) | Synapse's `receipts_linearized` holds our private receipt |
| View original | `Room::event_with_context` (12 events each side) into the existing original slice | Opened a message outside the loaded timeline |
| Markdown | matrix-sdk's `markdown` feature for messages and edits. Replies stay literal text, as with gomuks | The peer received the formatted body |

### Step 3 verification

On this branch, matrix-sdk 0.19.1, Synapse 1.166.0:
- `cargo test` (57 unit tests), `cargo clippy --all-targets`, the wasm32
  check and the Windows cross-build;
- `qa_matrix_sdk.py`, which now also checks each change from the gomuks
  peer's side (formatted body, edit, reaction) and the report and private
  read receipt in Synapse's database. After the fixes above, the round trip
  passed 10 times in a row (`UNTETHERED_QA_ONLY=round-trip` runs only that
  test), then the full script passed with both live tests.

## Step 4: attachments and the send queue

The App keeps its attachment tray (`transfers.rs`, `media.rs`) and still
sends the gomuks backend's transfer commands (upload, inline preview,
download) and the `get_media_config` RPC. On the new engine, `boot_engine`
hands those commands to the engine instead of a closed channel, and
`src/engine/media.rs` answers them. No view code changed: attachment posts
get the same file card, inline preview and Save button as on gomuks.

| Feature | Implementation | How it was verified |
| --- | --- | --- |
| Upload | Reads the file (path, portal or clipboard paste) and checks its size again. The MIME type comes from the content for images and from the name otherwise. `Encryption::upload_encrypted_file` in encrypted rooms, `Media::upload` elsewhere; progress in whole percents from the request's send-progress observable; dropping the App's lease cancels it. Images get `w`/`h` (after EXIF rotation) and, above 800×600, a JPEG thumbnail uploaded the same way. The result is the content gomuks's `/_gomuks/upload` returns | The gomuks peer decrypted the image, its thumbnail and the file, byte for byte |
| Sending | `send_message` with `base_content` becomes `Command::SendMedia`: the content goes through the room's send queue via the Timeline (`send`, or `send_reply` for replies), so it shows as a local echo. The command ends when the send queue reports the event sent; on a send error the event leaves the queue and the tray offers the retry | As above; the caption went with the first file and left the composer |
| Received attachments | Timeline items carry their content (`Message::media`), so posts get a `media::Media`. Each attachment's `MediaSource`, with the keys of an encrypted file, is remembered by mxc URI, because the App's `Media` carries only the URI | The peer's encrypted image showed inline and saved byte for byte |
| Inline previews | Encrypted: the file or its thumbnail, decrypted, through the SDK media cache. Plain: the homeserver's 800×600 thumbnail, or the file if there is none | Both, in the encrypted room and an unencrypted one |
| Downloads | `Media::get_media_content` without the cache, written to a temporary file beside the destination, then renamed | Byte for byte, encrypted and plain |
| Upload limit | `Client::load_or_fetch_max_upload_size` | Unit test |
| Send queue after errors | matrix-sdk disables a room's send queue after any send error, and nothing turned it back on: one network error stopped sending in that room until a restart. After a recoverable error the engine now re-enables the queue 5 seconds later, and the queue resends under the same transaction ID | Not tested live; it needs a network failure mid-send |

### Things worth knowing (step 4)

- **Memory:** previews and downloads load the whole file into memory. The
  App asks for a preview only when the event claims at most 4 MiB, and the
  engine discards anything larger after downloading it, but a sender can lie
  about the size; gomuks capped the stream instead. Downloads are bounded by
  the 64 MiB upload limit only in practice.
- **Failed text messages:** an unrecoverable send error (the homeserver
  refuses the event) leaves a text message in the queue as "Failed to send",
  and it holds back later messages in that room until a restart. Failed
  messages need Retry and Remove controls.
- **Not sent:** a blurhash, and durations or sizes for audio and video.

## (a) gomuks calls and their matrix-sdk equivalents

Status: **Done** = implemented on the new engine; **Available**
= the SDK has it, not wired yet; **Partial**/**Gap** as noted. API names are
from matrix-sdk/matrix-sdk-ui 0.20.0, which the engine uses since step 4.
Those marked *(0.20)* are new in 0.20.

### RPCs

| gomuks call | Where the client uses it | matrix-sdk equivalent | Status |
| --- | --- | --- | --- |
| `create_room` | Space and channel setup (`community.rs`) | `Client::create_room(create_room::v3::Request)` → `Room`; space edges with `Room::send_state_event(SpaceChildEventContent)` or `SpaceService::add_child_to_space` | Available |
| `get_event_context` | Opening a reply's original or a mention beyond loaded history (`replies.rs`) | `Room::event_with_context`; better, a second `Timeline` with `TimelineFocus::Event` that paginates both ways | Done (`event_with_context`) |
| `get_media_config` | Upload size limit (`account.rs`, `transfers.rs`) | `Client::load_or_fetch_max_upload_size()` (cached) | Done |
| `get_room_state` | Members and power levels (`main.rs`, `moderation.rs`, `permissions.rs`), call memberships (`desktop_call.rs`, `incoming.rs`), community setup | `Room::members`/`sync_members`, `Room::power_levels`, `get_state_events_static::<T>()` or raw `get_state_events`, and `Room::subscribe_to_state_events` *(0.20)*. `Posting::ingest` and `Room::voice_state` take JSON today and need typed inputs | Available |
| `get_rtc_transports` | "Check call server support" (`main.rs`), call setup (`desktop_call.rs`) | gomuks calls the MSC4143 `/rtc/transports` endpoint: `client.send(ruma::api::client::rtc::transports::…::Request)`; the well-known variant is `Client::rtc_foci()` | Available |
| `get_state` | Own user and device ID (`desktop_call.rs`), refresh after verification (`main.rs`) | `Client::user_id`/`device_id`, `Encryption::get_own_device`, `verification_state()`, `recovery().state()` | Done (as `Event::Account`) |
| `listen_to_device` | Receiving call encryption keys (`desktop_call.rs`) | `Client::subscribe_to_custom_to_device_messages([types])` *(0.20)*: Olm-decrypted, with the sender's device. In 0.19.1, a to-device event handler | Available |
| `login` | Account form | `matrix_auth().login_username()`; `Client::restore_session` on start | Done |
| `mark_read` | Read markers and Mark read (`reading.rs`) | `Timeline::send_single_receipt` / `mark_as_read(ReceiptType::Read or ReadPrivate)`, `Room::send_multiple_receipts` for fully-read; per-item receipts in `EventTimelineItem::read_receipts()` | Done (`send_multiple_receipts`) |
| `paginate` | History (`main.rs`) | `Timeline::paginate_backwards(50)` | Done |
| `report_event` | Report message (`reports.rs`) | `Room::report_content` | Done |
| `request_openid_token` | LiveKit JWT service auth (`desktop_call.rs`) | `client.send(request_openid_token::v3::Request::new(user_id))` | Available (raw ruma request) |
| `search_local` | Room search (`search.rs`), mentions inbox (`mentions.rs`) | No equivalent. The `experimental-search` feature (Tantivy, `Room::search_messages`, matrix-sdk-ui `SearchService`) covers part of room search | Gap; see (b) |
| `send_event` | Edits (`edits.rs`), reactions (`reactions.rs`), call decline (`incoming.rs`), call events (`desktop_call.rs`) | `Timeline::edit`, `Timeline::toggle_reaction`, `Room::send`, `Room::send_raw` | Done for edits and reactions; calls open |
| `send_message` | Composer and replies (`main.rs`), media captions (`transfers.rs`) | `Timeline::send`, `send_reply`, `send_attachment` (send queue: local echo, retry, wedged-queue handling) | Done for text (Markdown) and replies; media open |
| `send_to_device` | Sending call encryption keys (`desktop_call/channel.rs`) | `Encryption::encrypt_and_send_raw_to_device` behind the `experimental-send-custom-to-device` feature | Available behind a feature |
| `set_account_data` | Marked unread (`reading.rs`), space alert preferences (`notifications/spaces.rs`) | `Room::set_unread_flag`, `Room::set_account_data_raw`, `Account::set_account_data_raw` | Done for the unread flag and read sharing; space alerts open |
| `set_membership` | Kick, ban, unban, invite (`moderation.rs`), joins (`community.rs`) | `Room::kick_user`, `ban_user`, `unban_user`, `invite_user_by_id`, `leave`; `Client::join_room_by_id` | Available |
| `set_state` | Power levels (`permissions.rs`), space edges (`community.rs`), call membership with delayed leave (`rtc_membership.rs`, `desktop_call.rs`) | `Room::send_state_event`/`send_state_event_raw`; MSC4140 delayed events with `SendRawStateEvent::into_delayed` *(0.20)* | Available |
| `update_delayed_event`¹ | Restart/send/cancel the delayed call leave (`rtc_membership.rs`) | `Client::update_delayed_event(delay_id, action, …)` *(0.20)*; `Client::rtc_livekit_delegate_delayed_leave` *(0.20)* hands it to the homeserver | Available |
| `update_push_rule` | Room and space alerts, Do Not Disturb (`notifications.rs`, `notifications/spaces.rs`) | `Client::notification_settings()` → `set_room_notification_mode`, `set_push_rule_enabled`, `create_custom_conditional_push_rule`, `delete_user_defined_room_rules`; anything else (the per-room rules space alerts write) with ruma's `push::set_pushrule` through `Client::send` | Available |
| `verify` | Verify form | `Recovery::recover(key or passphrase)` | Done |

¹ Not in the original list; `rtc_membership.rs` calls it. The QA scripts also
call `generate_recovery_key`, `reset_encryption` and the `/_gomuks/keys/export`
endpoint, which is covered under recommendations (b).

### HTTP endpoints

| gomuks endpoint | Where | matrix-sdk equivalent | Status |
| --- | --- | --- | --- |
| `POST /_gomuks/upload` | Desktop uploads with progress and encryption (`backend/native.rs`) | `Encryption::upload_encrypted_file` / `Media::upload` with their send-progress observables, plus a thumbnail for large images; `Timeline::send_attachment` would also do it, but would bypass the App's attachment tray | Done (`engine/media.rs`) |
| `GET /_gomuks/media/…` | Inline thumbnails and downloads (`backend/native.rs`) | `Media::get_media_content(&MediaRequestParameters { source, format }, use_cache)`. gomuks looks an encrypted file's keys up by mxc in its database; the engine remembers each attachment's `MediaSource` from timeline items | Done |

### Frames

| gomuks frame | Used for | matrix-sdk equivalent | Status |
| --- | --- | --- | --- |
| `sync_complete` | Rooms, spaces, unread counts, timeline deltas, receipts, account data | `SyncService`/`RoomListService` drive sync; store reads on `subscribe_to_all_room_updates` and `room_info_notable_update_receiver` → `Event::Rooms`; Timeline diffs → `Event::Timeline`; receipts from `EventTimelineItem::read_receipts()`; account data through `Account` | Done for rooms, DMs, spaces, counts and timelines |
| `events_decrypted` | Late decryption | The Timeline replaces undecryptable items in place when keys arrive, including from key backup | Done (snapshots pick it up) |
| `send_complete` | Local echo confirmation or failure | `EventSendState` on timeline items; `Client::send_queue()` for retry and unwedging | Done ("Sending…"/"Failed to send") |
| `client_state` | Sign-in, verification, recovery availability | `Event::Account` from `verification_state()` and `recovery().state_stream()` | Done |
| `upload_progress` | Attachment progress | `Event::UploadProgress` from the upload request's send progress | Done |
| `response` / `error` | Request completion | `Event::Done { id, result: Result<(), Failure> }`; `Failure` is a small enum that never carries server text | Done |

## (b) Recommendations

### Simplified Sliding Sync or classic `/sync`

**Use `SyncService` (Simplified Sliding Sync, MSC4186) as the only sync
path, and fail clearly on servers that don't support it.**

- matrix-sdk-ui is built around it. `RoomListService`, the separate
  encryption sync, latest events and room subscriptions all assume it, and
  it is what Element X ships. The upstream work happens there.
- Synapse supports it natively since 1.114. The disposable Synapse 1.166.0
  used here advertised `org.matrix.simplified_msc3575: true`. 0.20.0 adds
  `VersionBuilder::DiscoverStable` for servers that advertise the stable
  endpoint.
- Startup is fast: the list loads in growing batches. In 0.19 and 0.20 the
  default `required_state` includes `m.space.child`/`m.space.parent`, so
  space edges are local reads.
- Don't subscribe rooms the list already holds. A room subscription answers
  with a `limited` timeline, so the room gets a second gap next to the
  list's. Back-pagination then returns older pages out of order. The
  acceptance test saw history 4–53, then 0–3, then 54, on both 0.19.1 and
  0.20.0, and waiting for the subscription response first didn't help. The
  engine instead raises the list's timeline to 20 events per room
  (`SyncServiceBuilder::with_room_list_timeline_limit`). That costs up to 20
  events per room on the first sync. Worth reporting upstream with the
  test; revisit room subscriptions once it is fixed.
- Unread counts behave differently. The SDK computes message and mention
  counts locally from the events it has seen; with 20 events per room in
  the list, a room with more unread messages than that can show a low count
  until it is opened. The engine uses
  the larger of the local mention count and the server highlight count, and
  marks a room unread if the server reports notifications. gomuks counted
  over everything it synced, so compare the badges on a large account before
  the switch.
- Before `SyncService` starts, check `Client::available_sliding_sync_versions()`
  at sign-in. If the server supports none, say so. A classic fallback is
  cheap to add later if real users need it: the room list here already reads
  from the store and room-update broadcasts, and the Timeline works with
  `Client::sync()`. Don't build it speculatively.

### Local search and the mentions inbox

**Keep our own index of decrypted events in a separate SQLite database in
the profile, fed by the SDK. Track `experimental-search` but don't build on
it yet.**

- gomuks's `search_local` queries its own SQLite database of decrypted
  events: room search with coverage counts, and the cross-room mentions
  inbox. The SDK has no counterpart.
- `experimental-search` (matrix-sdk-search) is a Tantivy index per room over
  the event cache, with optional encryption at rest. 0.20 adds a
  low-priority sweep that back-paginates joined rooms to fill it
  (`event_cache::search_backfill`). It is still experimental and a large
  dependency, it doesn't suit the browser, and its result order changed in
  0.20 (newest first instead of by score). It has no notion of "events that
  mention me".
- The index: FTS5 is compiled into the bundled SQLite the SDK already
  links (`libsqlite3-sys` defines `SQLITE_ENABLE_FTS5` for the bundled
  build). Feed it from each joined room's `RoomEventCache` updates, not just
  open timelines. They deliver decrypted events, and the event cache's
  re-decryptor replaces items when late keys arrive. For the
  mentions inbox, store the SDK's push-rule result with each event
  (`TimelineEvent::push_actions`, or `EventTimelineItem::is_highlighted`),
  so "mentions" means what Matrix push rules say, as it does in gomuks
  today. A background backfill that paginates each room replaces gomuks's
  growing coverage.
- The SDK encrypts its own stores; this index would hold plaintext. That
  matches gomuks's unencrypted database, but encrypt the index or keep
  message bodies out of it if the threat model changes. Keep `search.rs`
  and `mentions.rs` behind an interface so a different backend can replace
  the index later.

### Existing gomuks profiles

**Don't migrate the crypto store. Upgrading means signing in on the new
engine (a new device), verifying with the recovery key, then signing the
old device out.**

- gomuks keeps mautrix-go's Olm/Megolm state in its own database. A
  migration would have to carry the device identity, Olm sessions, inbound
  group sessions and cross-signing keys into matrix-sdk-crypto, and keep the
  device ID. That is possible in principle (vodozemac reads libolm
  pickles), but it is a large, risky one-off with no upstream support.
- Recovery restores history. In `scripts/qa_matrix_sdk.py` a gomuks device
  writes 55 encrypted messages before the SDK device exists. After
  `recover()`, the new device decrypted all 55 from server-side key backup
  (the engine uses `BackupDownloadStrategy::AfterDecryptionFailure`).
- Keys that never reached backup can still come across. gomuks's
  `POST /_gomuks/keys/export` writes a standard passphrase-protected Megolm
  key export, and `Encryption::import_room_keys` reads it. A one-time
  upgrade step can run the pinned gomuks against the old profile and import
  the export.
- Upgrade flow:
  1. On first start of an SDK-default build, detect gomuks data in the
     profile and explain the new sign-in.
  2. After verification, optionally import the gomuks key export.
  3. Offer to delete the old device. This needs authentication, normally
     the account password.
  4. Keep the gomuks files for one release so the user can roll back.
- Accounts without recovery are stuck either way; today's client can't
  verify them either. Recovery setup (`recovery().enable()`), SAS emoji
  verification and OIDC belong in step 2. The engine already turns off
  automatic cross-signing and backup creation, so signing in never replaces
  existing keys.

### Web

**Run the SDK in a dedicated Web Worker with IndexedDB, keep the Iced UI on
the main thread, and talk across a serde-derived version of the same
`Command`/`Event` types.**

- It builds. A probe crate with `matrix-sdk` 0.19.1 (`e2e-encryption`,
  `indexeddb`, `js`), `matrix-sdk-ui` (`js`) and getrandom's `wasm_js` feature
  builds a `Client` on `indexeddb_store` and starts a `SyncService`. It passes
  `cargo check --target wasm32-unknown-unknown` with `wasm-bindgen` resolved
  to the pinned 0.2.130. The pin doesn't block the move, but check every SDK
  bump against it until the pin can go.
- Use a worker, not the main thread. Sync processing, Megolm decryption and
  IndexedDB transactions on Iced's thread would stall rendering on large
  accounts. gomuks already runs in a worker. The SDK's web dependencies
  work there too: the IndexedDB layer (`matrix_indexed_db_futures`) falls
  back to `WorkerGlobalScope.indexedDB`, and reqwest's wasm client calls the
  global `fetch`. The engine can move over almost unchanged.
- The engine's `Command`/`Event` types are plain data on purpose. Derive
  serde for them and post them as structured clones or JSON. The boundary
  stays typed on both sides, unlike the hand-built JSON frames today.
- Keep one tab per device, as `web/worker/profile-lock.js` does today with
  the Web Locks API. The SDK's cross-process store lock exists for
  multi-process use, but one owner is simpler and matches the native
  profile lock.
- Do this after the native client has moved over (step 7, as planned).
  Meanwhile, keep engine code free of native-only APIs: file paths stay in
  `session.rs`, and media will need a byte-stream abstraction.

### Calls

**Keep the LiveKit media stack in `src/desktop_call/` as it is. Port only
the Matrix signaling, behind the existing `rtc_membership::Signal` seam,
whose callers already pass in a request closure.**

Most of these need 0.20.0 or later, which the engine uses since step 4.

| Signaling need | Today (gomuks) | matrix-sdk 0.20 |
| --- | --- | --- |
| RTC transport discovery | `get_rtc_transports` → MSC4143 `/rtc/transports` | `client.send(ruma::api::client::rtc::transports::…)`, or `Client::rtc_foci()` for well-known |
| LiveKit authorization | `request_openid_token`, then the JWT service | Same flow with `request_openid_token::v3::Request`. `Client::rtc_livekit_get_token` covers the newer homeserver endpoint where it exists (it returns `M_UNRECOGNIZED` otherwise) |
| Membership with delayed leave | `set_state` + `delay_ms`, `update_delayed_event` (`rtc_membership.rs`) | `Room::send_state_event_raw(…).into_delayed(…)` returns the delay ID; `Client::update_delayed_event` restarts, sends or cancels it; `rtc_livekit_delegate_delayed_leave` hands it to the homeserver |
| Encrypted per-sender keys (`rtc_keys.rs`) | `send_to_device` / `listen_to_device` | `Encryption::encrypt_and_send_raw_to_device` (feature `experimental-send-custom-to-device`); `Client::subscribe_to_custom_to_device_messages([io.element.call.encryption_keys])` |
| Memberships and ringing (`incoming.rs`) | `get_room_state`, decrypted `m.rtc.notification`/decline events | `Room::subscribe_to_state_events` for memberships; the Timeline already surfaces `RtcNotification { call_intent, declined_by, active_call_info }` |

`Signal::request(cmd, json)` lets `rtc_membership.rs` move without changes
by implementing it over the engine at first. Then replace the JSON commands
with typed methods (`join`, `heartbeat`, `leave`, `send_keys`) and delete the
string commands with gomuks. Repeat the Element Call interop runs from
`scripts/test-media-interop.py` (`--rtc-membership`, `--rtc-audio`,
`--desktop-dm`) on the new engine before switching the default. The
to-device feature flag is the one place where an experimental SDK API sits
on a user-facing path. Pin the SDK version, and treat each bump as a call
regression test.

### Account seeding in the QA scripts

**Replace gomuks-based seeding with a small Rust seeding binary on the
SDK, and keep gomuks only as an interop peer while it exists.**

- The scripts use gomuks for exactly two things: creating cross-signing and
  recovery (`generate_recovery_key` + `reset_encryption`), and acting as an
  encrypted peer. Plain HTTP covers registration and room setup.
- Add `src/bin/untethered-seed.rs` (non-wasm, built only for QA). It
  signs in with a password, bootstraps cross-signing
  (`EncryptionSettings::auto_enable_cross_signing`, or
  `bootstrap_cross_signing` with the password), runs
  `recovery().enable()`, prints the recovery key, and sends encrypted
  messages from a second device or user. It
  takes the same JSON fixture the scripts already write (server, user,
  password → recovery key, room). Replace `seed()` in
  `scripts/test-media-interop.py` with a call to it; most `qa_*.py` modules
  receive `seed` as a parameter, so few call sites change.
- Keep Element Web as the independent peer for interop. Same-library peers
  can hide symmetric bugs. Keep the gomuks peer in `qa_matrix_sdk.py` until
  gomuks is removed: it is what proves the switch-over path, including
  history from a gomuks device decrypting on the new one.

## (c) Build time and binary size

Measured on this branch against `70d5d0a`, the commit it builds on. The
setup:
- a 32-core x86-64 Linux machine, rustc 1.102 nightly, `-j 16`, fresh
  target directories, and the default release profile (no LTO);
- the local compiler cache off (`RUSTC_WRAPPER=` and clang called
  directly);
- the two builds back to back.

The machine was shared: load average 6.5 when the base started and 15.5
for the branch. An earlier attempt with the branch at load 90 is
discarded. Both binaries are the Linux `untethered-iced`.

| | Base | This branch | Change |
| --- | --- | --- | --- |
| Crates compiled for Linux (normal and build dependencies) | 441 | 579 | +138 |
| `Cargo.lock` packages (all targets) | 616 | 802 | +186 |
| Release clean build, wall | 54.7 s | 114.3 s | +59.6 s (2.1×) |
| Release clean build, CPU (user + sys) | 728 s | 1,488 s | +760 s (2.0×) |
| Dev clean build, wall | 34.6 s | 50.0 s | +15.4 s (1.4×) |
| Dev clean build, CPU | 347 s | 403 s | +56 s (1.2×) |
| Dev rebuild after touching `main.rs` | 1.3 s | 1.9 s | +0.6 s |
| Release binary | 72.3 MB | 163.8 MB | +91.5 MB (2.3×) |
| Release binary, stripped | 53.7 MB | 116.9 MB | +63.2 MB (2.2×) |
| Dev binary | 151.3 MB | 295.5 MB | +144.2 MB |

What this means:
- Most of the build cost is optimizing the SDK in release builds. Dev
  builds and edit-compile cycles barely change. The wasm32 build doesn't
  compile the SDK, so it is unaffected.
- Every native build pays this while gomuks stays the default, because the
  engine is chosen at runtime. If that becomes a problem before the switch,
  put the SDK behind a Cargo feature.
- The binary doubles, but it replaces the pinned gomuks release binary
  that the Arch package installs next to it (84 MB). Once gomuks is
  removed, an installed client gets smaller: 117 MB stripped instead of
  54 + 84 MB. A release profile with LTO and
  one codegen unit would likely cut this further; it wasn't measured
  here.

## Verification

All of the following were run on this branch:

- `cargo build --locked`; `cargo test --locked` (52 unit tests, 5 of them
  new, plus 4 ignored live tests); `cargo clippy --all-targets` (one existing
  lint in `Rooms::sort_rooms`); `cargo check --target wasm32-unknown-unknown`;
  and `cargo build --target x86_64-pc-windows-gnu`, which links.
- `scripts/qa_matrix_sdk.py` against a disposable Synapse 1.166.0, with
  gomuks 0.2611.1 as the seeding tool and the peer. It runs the ignored test
  `engine::live_test::matrix_sdk_login_restore_and_encrypted_round_trip`,
  which drives the real `App` through the same `Msg`s the UI sends. Final
  run on 0.19.1:

  ```text
  Synapse 1.166.0; Simplified Sliding Sync advertised: True
  matrix-sdk: password login as @sdkca3129:test.invalid
  matrix-sdk: recovery key verified this device; encrypted room synced
  matrix-sdk: space child edge, DM and highlight count (1) mapped
  matrix-sdk: back-paginated to the room start (20 then 55 posts); 55 of 55 earlier messages decrypted from key backup
  matrix-sdk: sent an encrypted message; waiting for the gomuks peer
  gomuks peer decrypted the matrix-sdk message $KbYXlxLCsemvUFoc1x7oByO30jUI0Crv--So79P1JPM
  matrix-sdk: decrypted and rendered the gomuks peer's message
  matrix-sdk: restart restored the session and both encrypted messages
  PASS matrix-sdk engine: login, recovery, sync, pagination, encrypted send/receive and restart
  ```

  The test also checks:
  - a wrong password and a wrong recovery entry report errors;
  - the paginated history is complete and in order, live and after the
    restart;
  - restoring doesn't log in again;
  - a second engine can't open a locked profile;
  - Synapse holds no plaintext `m.room.message` in the encrypted room.

  With per-room subscriptions the in-order check fails on both versions.
  The restart check used to fail sometimes; that was the engine's
  pagination race, fixed in step 3.
- The real binary, started with `UNTETHERED_ENGINE=matrix-sdk` in a private
  virtual KWin on a profile restored by the test, opened without signing
  in. It showed the space in the rail, the DM, the unread badge, and the
  encrypted room with history decrypted from backup and both exchanged
  messages.

To repeat it:

```sh
UNTETHERED_BACKEND=/path/to/gomuks uv run --python 3.13 --with matrix-synapse \
  --with requests --with websocket-client python scripts/qa_matrix_sdk.py
```

## Next steps

0. **Report the event cache issue upstream** with the acceptance test: pages
   come back out of order after a room subscription. Re-run
   `qa_matrix_sdk.py` on each SDK release.
1. **Finish account work:**
   - test OAuth sign-in against a real MAS (it needs PostgreSQL);
   - approve MAS identity resets in the browser;
   - verify other people (in-room requests) and support QR codes;
   - rename devices;
   - move step-1 passphrases from the session file into the keyring.
2. **Messaging on the Timeline:** done in step 3 except typing
   notifications. Once gomuks is gone, delete the hand-rolled aggregation in
   `rooms.rs`/`edits.rs`/`reactions.rs`/`replies.rs`/`reading.rs`.
3. **Media and send queue:** carry `MediaSource` in `media::Media`, send
   attachments with progress, and add thumbnail/download commands.
4. Notifications, moderation, permissions and spaces; then calls (above);
   then the web worker; then delete `backend/` and the gomuks scripts.
