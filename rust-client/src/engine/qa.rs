//! QA fixtures only (debug builds): `UNTETHERED_QA_SEED` signs the profile
//! in `UNTETHERED_PROFILE` in on this engine and verifies it, then the
//! process exits instead of opening a window. The call fixtures use it the
//! way they use gomuks to prepare a gomuks profile.
//!
//! `UNTETHERED_QA_SEED` is `{"server","username","password","key"}` (`key`:
//! the account's recovery key). Run it with `UNTETHERED_KEYRING=off`, so the
//! store passphrase stays in the session file and the client opens the
//! profile in the fixture's own isolated session.
//!
//! The QA event log is what this device synced, one JSON object per entry
//! (see [`watch`]). The fixtures read it where they read gomuks's database
//! before. On the desktop `UNTETHERED_QA_EVENTS` names the file that gets
//! one entry per line; in the browser every entry is a `{"Qa": entry}`
//! message from the engine's worker (`worker.rs`), which browser fixtures
//! watch on the page.
use super::Alert;
use matrix_sdk::{
    Client, Room,
    deserialized_responses::EncryptionInfo,
    ruma::{
        events::{AnyGlobalAccountDataEvent, AnySyncTimelineEvent},
        push::Action,
        serde::Raw,
    },
};
use serde_json::{Value, json};

#[cfg(not(target_arch = "wasm32"))]
type Sink = std::sync::Arc<std::path::Path>;
#[cfg(target_arch = "wasm32")]
type Sink = ();

#[cfg(not(target_arch = "wasm32"))]
fn sink() -> Option<Sink> {
    std::env::var_os("UNTETHERED_QA_EVENTS").map(|p| std::path::Path::new(&p).into())
}
#[cfg(target_arch = "wasm32")]
fn sink() -> Option<Sink> {
    Some(())
}

#[cfg(not(target_arch = "wasm32"))]
fn append(path: &Sink, entry: Value) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{entry}");
    }
}
#[cfg(target_arch = "wasm32")]
fn append(_: &Sink, entry: Value) {
    super::worker::qa(entry);
}

/// Log every timeline event and global account data event sync brings:
///
/// - `{"kind":"event", room_id, event_id, sender, type, content, encrypted,
///   notify, highlight, sound}`: `type` and `content` are decrypted when the
///   SDK could decrypt the event (an undecryptable one keeps
///   `m.room.encrypted`); `notify`, `highlight` and `sound` are the actions the
///   SDK's push rule evaluation gave it, i.e. what the account's push rules
///   say, before the engine's own filters (see [`alert`]). An event the SDK
///   decrypts later, when its room key arrives, gets a second entry from
///   [`redecrypted`] with `"redecrypted": true`.
/// - `{"kind":"account_data", type, content}`.
///
/// Back-paginated history isn't logged; sync does bring each room's latest
/// events when it first lists the room.
pub fn watch(client: &Client) {
    let Some(path) = sink() else {
        return;
    };
    let timeline = path.clone();
    client.add_event_handler(
        move |raw: Raw<AnySyncTimelineEvent>,
              room: Room,
              info: Option<EncryptionInfo>,
              actions: Vec<Action>| {
            let path = timeline.clone();
            async move {
                if let Some(entry) = entry(room.room_id(), &raw, info.is_some(), &actions) {
                    append(&path, entry);
                }
            }
        },
    );
    client.add_event_handler(move |raw: Raw<AnyGlobalAccountDataEvent>| {
        let path = path.clone();
        async move {
            if let Ok(event) = raw.deserialize_as::<Value>() {
                let line = json!({"kind": "account_data", "type": event["type"],
                    "content": event["content"]});
                append(&path, line);
            }
        }
    });
}

/// Log the events the SDK's event cache decrypted after sync brought them
/// (the room key came later, often on the separate encryption sync), with the
/// push actions it evaluated for them then. Runs for the session.
pub async fn redecrypted(client: Client) {
    use futures_util::StreamExt;
    use matrix_sdk::event_cache::RedecryptorReport;
    let (Some(path), Some(cache)) = (sink(), client.event_cache().cloned()) else {
        return;
    };
    // The cache, not the client, so this doesn't keep the session alive.
    drop(client);
    let mut reports = std::pin::pin!(cache.subscribe_to_decryption_reports());
    while let Some(report) = reports.next().await {
        let Ok(RedecryptorReport::ResolvedUtds { room_id, events }) = report else {
            continue;
        };
        let Ok((room, _handles)) = cache.room(&room_id).await else {
            continue;
        };
        for id in events {
            let Ok(Some(event)) = room.find_event(&id).await else {
                continue;
            };
            let encrypted = event.encryption_info().is_some();
            let actions = event.push_actions().unwrap_or_default();
            if let Some(mut entry) = entry(&room_id, event.raw(), encrypted, actions) {
                entry["redecrypted"] = true.into();
                append(&path, entry);
            }
        }
    }
}

fn entry(
    room: &matrix_sdk::ruma::RoomId,
    raw: &Raw<AnySyncTimelineEvent>,
    decrypted: bool,
    actions: &[Action],
) -> Option<Value> {
    let event = raw.deserialize_as::<Value>().ok()?;
    let kind = event["type"].as_str().unwrap_or_default();
    Some(json!({
        "kind": "event",
        "room_id": room,
        "event_id": event["event_id"],
        "sender": event["sender"],
        "type": kind,
        "content": event["content"],
        "encrypted": decrypted || kind == "m.room.encrypted",
        "notify": actions.iter().any(Action::should_notify),
        "highlight": actions.iter().any(Action::is_highlight),
        "sound": actions.iter().any(|a| a.sound().is_some()),
    }))
}

/// Log a notification the engine passed to the App (`Event::Notification`):
/// `{"kind":"alert", room_id, event_id, sender, body, sound}`. These are the
/// push rule notifications left after `alerts.rs` drops our own messages,
/// edits, non-messages and history from before the session started; the App
/// still decides whether to show a popup (focused room, do not disturb).
pub fn alert(alert: &Alert) {
    if let Some(path) = sink() {
        let line = json!({"kind": "alert", "room_id": alert.room, "event_id": alert.event_id,
            "sender": alert.sender, "body": alert.body, "sound": alert.sound});
        append(&path, line);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn seed(seed: &str) -> i32 {
    use super::{Command, Event, run};
    use std::time::Duration;
    let Ok(seed) = serde_json::from_str::<serde_json::Value>(seed) else {
        eprintln!("UNTETHERED_QA_SEED isn't JSON");
        return 2;
    };
    let Ok(runtime) = tokio::runtime::Runtime::new() else {
        return 2;
    };
    let text = |key: &str| seed[key].as_str().unwrap_or_default().to_owned();
    let code = runtime.block_on(async move {
        let (commands, rx) = async_channel::unbounded();
        let (_transfers, backend) = async_channel::unbounded();
        let (tx, events) = async_channel::unbounded();
        let engine = tokio::spawn(async move {
            if let Err(why) = run(rx, backend, tx).await {
                eprintln!("QA seed: {why}");
            }
        });
        // Account updates can come before or after a command's answer.
        let mut verified = false;
        let mut until = async |want: i64, verify: bool| -> Option<bool> {
            let mut done = None;
            loop {
                let Ok(event) = events.recv().await else {
                    eprintln!("QA seed: the engine stopped");
                    return None;
                };
                match event {
                    Event::Done { id, result } if id == want => {
                        if let Err(failure) = result {
                            eprintln!("QA seed: command {want} failed: {failure:?}");
                        }
                        done = Some(result.is_ok());
                    }
                    Event::Account(a) => verified = a.logged_in && a.verified,
                    Event::Status(status) => eprintln!("QA seed: {status}"),
                    _ => {}
                }
                match done {
                    Some(false) => return Some(false),
                    Some(true) if !verify || verified => return Some(true),
                    _ => {}
                }
            }
        };
        let login = Command::Login {
            id: 1,
            homeserver: text("server"),
            username: text("username"),
            password: text("password"),
        };
        let signed_in = commands.send(login).await.is_ok()
            && matches!(
                tokio::time::timeout(Duration::from_secs(150), until(1, false)).await,
                Ok(Some(true))
            );
        if !signed_in {
            eprintln!("QA seed: sign-in failed");
            return 3;
        }
        let recover = Command::Recover {
            id: 2,
            secret: text("key"),
        };
        let recovered = commands.send(recover).await.is_ok()
            && matches!(
                tokio::time::timeout(Duration::from_secs(150), until(2, true)).await,
                Ok(Some(true))
            );
        if !recovered {
            eprintln!("QA seed: the recovery key didn't verify the device");
            return 4;
        }
        drop(commands);
        let _ = tokio::time::timeout(Duration::from_secs(20), engine).await;
        0
    });
    // Exit without shutting the runtime down: matrix-sdk-crypto panics when
    // its key query tasks are cancelled, which is only noise here.
    std::process::exit(code)
}
