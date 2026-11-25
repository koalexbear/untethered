//! Opt-in acceptance test for the matrix-sdk engine. `scripts/qa_matrix_sdk.py`
//! starts a disposable Synapse, seeds the account and recovery key through
//! gomuks, and runs a second account in gomuks as the peer.
use super::{Command, Event, run};
use crate::{App, Msg, Request, rooms};
use async_channel::{Receiver, Sender};
use iced::widget::text_editor;
use serde_json::Value;
use std::{env, path::Path, time::Duration};

fn session() -> (
    App,
    Receiver<Event>,
    tokio::task::JoinHandle<Result<(), String>>,
) {
    let (gomuks, _) = async_channel::bounded(1);
    let (commands, rx): (Sender<Command>, _) = async_channel::bounded(64);
    let (tx, events) = async_channel::bounded(64);
    let mut app = App::new(gomuks);
    app.sdk = Some(commands);
    (app, events, tokio::spawn(run(rx, tx)))
}

async fn wait(app: &mut App, rx: &Receiver<Event>, label: &str, until: impl Fn(&App) -> bool) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while !until(app) {
            let event = rx.recv().await.expect("engine exited");
            if let Event::Done { result: Err(f), id } = &event
                && !matches!(app.pending.get(id), Some(Request::Verify(_)))
            {
                panic!("engine request failed during {label}: {f:?}");
            }
            app.engine_event(event);
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out during {label}; status: {}", app.status));
}

async fn signal(dir: &Path, name: &str) -> String {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(dir.join(name)) {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the peer did not write {name}"))
}

/// Indexes of the seeded "history N" posts, in display order.
fn history_order(app: &App, room: &str) -> Vec<usize> {
    app.live.list[room]
        .posts()
        .filter_map(|p| p.text().strip_prefix("history ")?.parse().ok())
        .collect()
}

async fn paginate_to_start(app: &mut App, rx: &Receiver<Event>, room: &str) {
    // A page can hold only state events, which have no Post; allow a few.
    for _ in 0..12 {
        if app.live.list[room].end {
            break;
        }
        let _ = app.update(Msg::More);
        wait(app, rx, "back-pagination", |a| {
            !a.pending.values().any(|p| matches!(p, Request::Page(_)))
        })
        .await;
    }
    assert!(
        app.live.list[room].end,
        "pagination never reached the start of the room"
    );
}

fn has(app: &App, room: &str, body: &str) -> bool {
    app.live.list.get(room).is_some_and(|r| {
        r.posts()
            .any(|p| p.text() == body && p.event_id.is_some() && p.encrypted)
    })
}

async fn stop(app: App, engine: tokio::task::JoinHandle<Result<(), String>>) {
    app.sdk.as_ref().unwrap().close();
    drop(app);
    tokio::time::timeout(Duration::from_secs(20), engine)
        .await
        .expect("engine shut down")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run scripts/qa_matrix_sdk.py (disposable Synapse and a gomuks peer)"]
async fn matrix_sdk_login_restore_and_encrypted_round_trip() {
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(env::var("UNTETHERED_QA_FIXTURE").unwrap()).unwrap())
            .unwrap();
    let signals = std::path::PathBuf::from(env::var_os("UNTETHERED_QA_SIGNALS").unwrap());
    let server = fixture["server"].as_str().unwrap();
    assert_eq!(
        url::Url::parse(server).unwrap().host_str(),
        Some("127.0.0.1")
    );
    assert!(
        !Path::new(&env::var_os("UNTETHERED_PROFILE").unwrap()).exists(),
        "Use a fresh profile."
    );
    let room = fixture["room"].as_str().unwrap();
    let mine = format!("matrix-sdk to gomuks {}", uuid::Uuid::new_v4());

    let (mut app, rx, engine) = session();
    wait(&mut app, &rx, "startup", |a| a.connected).await;
    assert!(!app.logged_in, "a fresh profile has no session");
    app.server = server.into();
    app.username = rooms::string(&fixture["username"]);
    app.password = "wrong password".into();
    let _ = app.update(Msg::Login);
    tokio::time::timeout(Duration::from_secs(30), async {
        while app.pending.values().any(|p| matches!(p, Request::Login)) {
            app.engine_event(rx.recv().await.unwrap());
        }
    })
    .await
    .expect("rejected login returned");
    assert!(!app.logged_in && app.status.starts_with("Sign-in failed"));
    app.password = rooms::string(&fixture["password"]);
    let _ = app.update(Msg::Login);
    wait(&mut app, &rx, "login", |a| a.logged_in).await;
    println!("matrix-sdk: password login as {}", app.matrix_id);

    // A failed attempt must report its error and leave the form usable.
    app.recovery = "not a recovery key".into();
    let _ = app.update(Msg::Verify);
    wait(&mut app, &rx, "failed verification", |a| {
        !a.verifying() && !a.verification_note.is_empty()
    })
    .await;
    assert!(
        app.verification_feedback().contains("as a passphrase"),
        "{}",
        app.verification_feedback()
    );
    app.recovery = rooms::string(&fixture["key"]).replace(' ', "\u{202f}");
    let _ = app.update(Msg::Verify);
    wait(&mut app, &rx, "verified room sync", |a| {
        a.verified && a.live.list.get(room).is_some_and(|r| r.encrypted)
    })
    .await;
    println!("matrix-sdk: recovery key verified this device; encrypted room synced");
    let space = fixture["space"].as_str().unwrap();
    wait(&mut app, &rx, "space hierarchy", |a| {
        a.live.list.get(space).is_some_and(|s| s.space) && a.live.channels(space).contains_key(room)
    })
    .await;
    let dm = fixture["dm"].as_str().unwrap();
    wait(&mut app, &rx, "direct message", |a| {
        a.live.list.get(dm).is_some_and(|r| r.dm)
    })
    .await;
    let unread = fixture["unread_room"].as_str().unwrap();
    wait(&mut app, &rx, "unread counts", |a| {
        a.live
            .list
            .get(unread)
            .is_some_and(|r| r.unread && r.read.highlights > 0)
    })
    .await;
    println!(
        "matrix-sdk: space child edge, DM and highlight count ({}) mapped",
        app.live.list[unread].read.highlights
    );

    let _ = app.update(Msg::Open(room.into()));
    wait(&mut app, &rx, "first history page", |a| {
        a.selected == room && !a.pending.values().any(|p| matches!(p, Request::Page(_)))
    })
    .await;
    let older = fixture["history"].as_u64().unwrap() as usize;
    let first = app.live.list[room].posts().len();
    paginate_to_start(&mut app, &rx, room).await;
    let loaded = app.live.list[room].posts().len();
    assert!(
        loaded >= older && loaded > first,
        "only {loaded} of {older} earlier posts loaded"
    );
    // The gomuks seed device wrote this history before this device existed;
    // it can only decrypt through the key backup unlocked by recovery.
    let decrypted = |app: &App| {
        app.live.list[room]
            .posts()
            .filter(|p| p.text().starts_with("history "))
            .count()
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while decrypted(&app) < older && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            app.engine_event(event);
        }
    }
    println!(
        "matrix-sdk: back-paginated to the room start ({first} then {loaded} posts); {} of {older} earlier messages decrypted from key backup",
        decrypted(&app)
    );
    let order = history_order(&app, room);
    assert!(
        order.is_sorted(),
        "history out of order after pagination: {order:?}"
    );

    app.editor = text_editor::Content::with_text(&mine);
    let _ = app.update(Msg::Send);
    wait(&mut app, &rx, "encrypted send", |a| has(a, room, &mine)).await;
    assert!(
        app.editor.text().trim().is_empty(),
        "the sent draft was cleared"
    );
    std::fs::write(signals.join("sdk-sent"), &mine).unwrap();
    println!("matrix-sdk: sent an encrypted message; waiting for the gomuks peer");

    let theirs = signal(&signals, "peer-sent").await;
    wait(&mut app, &rx, "decrypt the peer's message", |a| {
        has(a, room, &theirs)
    })
    .await;
    let sender = app.live.list[room]
        .posts()
        .find(|p| p.text() == theirs)
        .map(|p| p.sender.clone())
        .unwrap();
    assert_eq!(sender, fixture["peer"].as_str().unwrap());
    println!("matrix-sdk: decrypted and rendered the gomuks peer's message");
    stop(app, engine).await;

    let (mut app, rx, engine) = session();
    wait(&mut app, &rx, "restore", |a| a.connected).await;
    assert!(
        app.logged_in && app.verified,
        "the session was restored, not created"
    );
    assert!(
        !app.pending.values().any(|p| matches!(p, Request::Login)),
        "restoring must not log in again"
    );
    wait(&mut app, &rx, "restored room list", |a| {
        a.live.list.contains_key(room)
    })
    .await;
    let _ = app.update(Msg::Open(room.into()));
    wait(&mut app, &rx, "restored history", |a| {
        has(a, room, &mine) && has(a, room, &theirs)
    })
    .await;
    paginate_to_start(&mut app, &rx, room).await;
    let order = history_order(&app, room);
    assert!(
        order.len() == older && order.is_sorted(),
        "restored history incomplete or out of order: {order:?}"
    );
    println!("matrix-sdk: restart restored the session and both encrypted messages");
    stop(app, engine).await;

    // A second engine on the same profile is refused while one runs.
    let (mut held, rx, first) = session();
    wait(&mut held, &rx, "first engine", |a| a.connected).await;
    let (_second_app, _second_rx, second) = session();
    let refused = tokio::time::timeout(Duration::from_secs(10), second)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refused, Err("This profile is already open.".into()));
    drop(rx);
    stop(held, first).await;
}
