//! Opt-in acceptance tests for the matrix-sdk engine. `scripts/qa_matrix_sdk.py`
//! starts a disposable Synapse, seeds the account and recovery key through
//! gomuks, and runs a second account in gomuks as the peer.
use super::{
    BrowserMethod, Command, Event, Stage, account_ui::Action, run_in, session::profile_root,
};
use crate::{App, Msg, Request, rooms};
use async_channel::{Receiver, Sender};
use iced::widget::text_editor;
use serde_json::Value;
use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

type Engine = tokio::task::JoinHandle<Result<(), String>>;

/// Follows a sign-in page's redirects like a browser would, so a mock
/// identity provider can complete single sign-on without a person.
fn browser() -> super::Opener {
    Arc::new(|url: url::Url| {
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .unwrap();
            let _ = client.get(url).send().await;
        });
    })
}

fn session_at(dir: &Path) -> (App, Receiver<Event>, Engine) {
    let (gomuks, _) = async_channel::bounded(1);
    let (commands, rx): (Sender<Command>, _) = async_channel::bounded(64);
    let (tx, events) = async_channel::bounded(64);
    let mut app = App::new(gomuks);
    app.sdk = Some(commands);
    let dir = dir.to_path_buf();
    (app, events, tokio::spawn(run_in(dir, browser(), rx, tx)))
}

fn session() -> (App, Receiver<Event>, Engine) {
    session_at(&profile_root().unwrap())
}

async fn wait(app: &mut App, rx: &Receiver<Event>, label: &str, until: impl Fn(&App) -> bool) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while !until(app) {
            let event = rx.recv().await.expect("engine exited");
            if let Event::Done { result: Err(f), id } = &event
                && app
                    .pending
                    .get(id)
                    .is_some_and(|p| !matches!(p, Request::Verify(_)))
            {
                panic!("engine request failed during {label}: {f:?}");
            }
            app.engine_event(event);
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out during {label}; status: {}; note: {}",
            app.status, app.sdk_ui.note
        )
    });
}

/// Drive two engines at once, for flows between two devices.
async fn wait2(
    a: &mut App,
    ra: &Receiver<Event>,
    b: &mut App,
    rb: &Receiver<Event>,
    label: &str,
    until: impl Fn(&App, &App) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(90), async {
        while !until(a, b) {
            tokio::select! {
                e = ra.recv() => a.engine_event(e.expect("first engine exited")),
                e = rb.recv() => b.engine_event(e.expect("second engine exited")),
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out during {label}; first: {:?} {}; second: {:?} {}",
            a.sdk_ui.verification, a.sdk_ui.note, b.sdk_ui.verification, b.sdk_ui.note
        )
    });
}

async fn sign_in(
    dir: &Path,
    server: &str,
    user: &str,
    password: &str,
) -> (App, Receiver<Event>, Engine) {
    let (mut app, rx, engine) = session_at(dir);
    wait(&mut app, &rx, "startup", |a| a.connected).await;
    app.server = server.into();
    app.username = user.into();
    app.password = password.into();
    let _ = app.update(Msg::Login);
    wait(&mut app, &rx, "login", |a| a.logged_in).await;
    (app, rx, engine)
}

fn saved_session(dir: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(dir.join("matrix-sdk/session.json")).ok()?).ok()
}

fn stage(app: &App) -> Option<&Stage> {
    app.sdk_ui.verification.as_ref().map(|v| &v.stage)
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
    let saved = saved_session(&profile_root().unwrap()).expect("session saved");
    if env::var_os("UNTETHERED_QA_KEYRING").is_some() {
        assert!(
            saved["keyring"].is_string() && saved["passphrase"].is_null(),
            "the store passphrase belongs in the keyring"
        );
        println!("matrix-sdk: store passphrase saved in the Secret Service keyring");
    }

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

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[ignore = "run scripts/qa_matrix_sdk.py (disposable Synapse)"]
async fn matrix_sdk_account_flows() {
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(env::var("UNTETHERED_QA_FIXTURE").unwrap()).unwrap())
            .unwrap();
    let base = PathBuf::from(env::var_os("UNTETHERED_QA_PROFILES").unwrap());
    let server = fixture["server"].as_str().unwrap();
    let (user, pass) = (
        rooms::string(&fixture["username"]),
        rooms::string(&fixture["password"]),
    );
    let keyring = env::var_os("UNTETHERED_QA_KEYRING").is_some();

    // Emoji verification: a new device (b) verified by a verified one (a).
    let (mut a, ra, ea) = sign_in(&base.join("a"), server, &user, &pass).await;
    a.recovery = rooms::string(&fixture["key"]);
    let _ = a.update(Msg::Verify);
    wait(&mut a, &ra, "verify the first device", |a| a.verified).await;
    let (mut b, rb, eb) = sign_in(&base.join("b"), server, &user, &pass).await;
    wait(&mut b, &rb, "own identity known", |b| b.sdk_ui.identity).await;
    assert!(!b.verified);
    let _ = b.update(Msg::Sdk(Action::VerifyWithDevice));
    wait2(&mut a, &ra, &mut b, &rb, "incoming request", |a, _| {
        stage(a) == Some(&Stage::Requested)
    })
    .await;
    assert!(a.sdk_ui.verification.as_ref().unwrap().incoming);
    let _ = a.update(Msg::Sdk(Action::AcceptVerification));
    wait2(&mut a, &ra, &mut b, &rb, "emoji on both devices", |a, b| {
        matches!(stage(a), Some(Stage::Emoji(_))) && matches!(stage(b), Some(Stage::Emoji(_)))
    })
    .await;
    let (Some(Stage::Emoji(left)), Some(Stage::Emoji(right))) = (stage(&a), stage(&b)) else {
        unreachable!()
    };
    assert_eq!(left.len(), 7);
    assert_eq!(left, right, "both devices show the same emoji");
    let shown: Vec<_> = left.iter().map(|(s, n)| format!("{s} {n}")).collect();
    println!("matrix-sdk: both devices show {}", shown.join(", "));
    let _ = a.update(Msg::Sdk(Action::EmojiMatch(true)));
    let _ = b.update(Msg::Sdk(Action::EmojiMatch(true)));
    wait2(&mut a, &ra, &mut b, &rb, "verified by emoji", |a, b| {
        stage(a) == Some(&Stage::Done) && stage(b) == Some(&Stage::Done) && b.verified
    })
    .await;
    println!("matrix-sdk: emoji verification cross-signed the new device");

    // Devices: the new device shows as verified, then is signed out remotely.
    let b_id = saved_session(&base.join("b")).unwrap()["session"]["device_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let _ = a.update(Msg::Sdk(Action::CloseVerification));
    let _ = b.update(Msg::Sdk(Action::CloseVerification));
    for _ in 0..20 {
        let _ = a.update(Msg::Sdk(Action::RefreshDevices));
        wait(&mut a, &ra, "device list", |a| a.sdk_ui.jobs.is_empty()).await;
        let listed = a.sdk_ui.devices.iter().flatten().find(|d| d.id == b_id);
        if listed.is_some_and(|d| d.verified) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let devices = a.sdk_ui.devices.clone().unwrap();
    assert!(devices.iter().any(|d| d.current && d.verified));
    assert!(
        devices
            .iter()
            .any(|d| d.id == b_id && d.verified && !d.current)
    );
    let _ = a.update(Msg::Sdk(Action::AskRemove(Some(b_id.clone()))));
    let _ = a.update(Msg::Sdk(Action::Password("not the password".into())));
    let _ = a.update(Msg::Sdk(Action::RemoveDevice));
    wait(&mut a, &ra, "wrong password", |a| a.sdk_ui.jobs.is_empty()).await;
    assert!(
        a.sdk_ui.note.contains("didn’t accept that password"),
        "{}",
        a.sdk_ui.note
    );
    let _ = a.update(Msg::Sdk(Action::Password(pass.clone())));
    let _ = a.update(Msg::Sdk(Action::RemoveDevice));
    wait(&mut a, &ra, "device signed out", |a| {
        a.sdk_ui.jobs.is_empty()
    })
    .await;
    assert!(a.sdk_ui.note.starts_with("Signed out"), "{}", a.sdk_ui.note);
    assert!(!a.sdk_ui.devices.iter().flatten().any(|d| d.id == b_id));
    wait(&mut b, &rb, "removed device notices", |b| {
        !b.logged_in && !b.status.is_empty()
    })
    .await;
    assert!(b.status.contains("signed this device out"), "{}", b.status);
    assert!(
        saved_session(&base.join("b")).is_none(),
        "local session removed"
    );
    println!(
        "matrix-sdk: device list showed it verified; removing it (password checked) signed it out"
    );
    stop(a, ea).await;
    stop(b, eb).await;

    // Recovery for an account that has no encryption keys yet.
    let (fresh, fresh_pass) = (
        rooms::string(&fixture["fresh_username"]),
        rooms::string(&fixture["fresh_password"]),
    );
    let (mut c, rc, ec) = sign_in(&base.join("c"), server, &fresh, &fresh_pass).await;
    wait(&mut c, &rc, "recovery state known", |c| {
        c.sdk_ui.recovery != super::Recovery::Unknown
    })
    .await;
    assert!(!c.sdk_ui.identity && !c.verified);
    let _ = c.update(Msg::Sdk(Action::Password(fresh_pass.clone())));
    let _ = c.update(Msg::Sdk(Action::SetUpRecovery));
    wait(&mut c, &rc, "recovery set up", |c| {
        c.sdk_ui.recovery_key.is_some() && c.verified && c.sdk_ui.jobs.is_empty()
    })
    .await;
    let first = c.sdk_ui.recovery_key.clone().unwrap();
    assert!(c.sdk_ui.password.is_empty(), "the password isn't kept");
    let _ = c.update(Msg::Sdk(Action::SavedRecoveryKey));
    assert!(c.sdk_ui.recovery_key.is_none());
    println!("matrix-sdk: recovery setup created keys, verified the device and showed a key once");
    let _ = c.update(Msg::Sdk(Action::ChangeRecoveryKey));
    wait(&mut c, &rc, "new recovery key", |c| {
        c.sdk_ui.recovery_key.is_some() && c.sdk_ui.jobs.is_empty()
    })
    .await;
    let second = c.sdk_ui.recovery_key.clone().unwrap();
    assert_ne!(first, second);
    let _ = c.update(Msg::Sdk(Action::SavedRecoveryKey));

    // Only the new key verifies another device.
    let (mut d, rd, ed) = sign_in(&base.join("d"), server, &fresh, &fresh_pass).await;
    d.recovery = first.clone();
    let _ = d.update(Msg::Verify);
    wait(&mut d, &rd, "old key refused", |d| {
        !d.verifying() && !d.verification_note.is_empty()
    })
    .await;
    assert!(!d.verified);
    d.recovery = second.clone();
    let _ = d.update(Msg::Verify);
    wait(&mut d, &rd, "new key verifies", |d| d.verified).await;
    println!("matrix-sdk: the changed key verifies a new device and the old one doesn't");
    if keyring {
        assert!(saved_session(&base.join("d")).unwrap()["keyring"].is_string());
    } else {
        assert!(saved_session(&base.join("d")).unwrap()["passphrase"].is_string());
    }

    // Sign out removes the session, the stores and the keyring entry.
    let _ = d.update(Msg::Sdk(Action::AskSignOut(true)));
    let _ = d.update(Msg::Sdk(Action::SignOut));
    wait(&mut d, &rd, "signed out", |d| {
        !d.logged_in && d.sdk_ui.jobs.is_empty()
    })
    .await;
    assert!(saved_session(&base.join("d")).is_none());
    assert!(!base.join("d/matrix-sdk/store").exists());
    stop(d, ed).await;
    let (mut d, rd, ed) = session_at(&base.join("d"));
    wait(&mut d, &rd, "restart after sign-out", |d| d.connected).await;
    assert!(!d.logged_in, "nothing to restore after signing out");
    stop(d, ed).await;
    println!("matrix-sdk: sign-out removed the session and stores");

    // Reset encryption from a new device that has neither key nor device.
    let (mut e, re, ee) = sign_in(&base.join("e"), server, &fresh, &fresh_pass).await;
    wait(&mut e, &re, "identity known", |e| e.sdk_ui.identity).await;
    let _ = e.update(Msg::Sdk(Action::ShowReset(true)));
    let _ = e.update(Msg::Sdk(Action::Password(fresh_pass.clone())));
    let _ = e.update(Msg::Sdk(Action::ResetEncryption));
    wait(&mut e, &re, "encryption reset", |e| {
        e.sdk_ui.recovery_key.is_some() && e.verified && e.sdk_ui.jobs.is_empty()
    })
    .await;
    assert_ne!(e.sdk_ui.recovery_key.as_deref(), Some(second.as_str()));
    println!("matrix-sdk: reset encryption gave a new identity and recovery key");
    wait(&mut c, &rc, "old device loses its verification", |c| {
        !c.verified
    })
    .await;
    println!("matrix-sdk: the earlier device reports itself unverified after the reset");
    stop(e, ee).await;
    stop(c, ec).await;

    // Single sign-on: Synapse's CAS login against the script's mock CAS
    // server, followed to the loopback listener like a browser would.
    let (mut f, rf, ef) = session_at(&base.join("f"));
    wait(&mut f, &rf, "startup", |f| f.connected).await;
    f.server = server.into();
    let _ = f.update(Msg::Sdk(Action::LoginOptions));
    wait(&mut f, &rf, "sign-in options", |f| {
        f.sdk_current_options().is_some()
    })
    .await;
    let options = f.sdk_current_options().unwrap().clone();
    assert!(
        options.password && options.sso && !options.oauth,
        "{options:?}"
    );
    let provider = options.providers.first().map(|(id, _)| id.clone());
    let _ = f.update(Msg::Sdk(Action::BrowserLogin(BrowserMethod::Sso {
        provider,
    })));
    assert!(f.sdk_ui.browser);
    wait(&mut f, &rf, "single sign-on", |f| {
        f.logged_in && !f.sdk_ui.browser
    })
    .await;
    assert_eq!(f.matrix_id, fixture["sso_user"].as_str().unwrap());
    assert!(!f.sdk_ui.browser && f.status.is_empty(), "{}", f.status);
    stop(f, ef).await;
    let (mut f, rf, ef) = session_at(&base.join("f"));
    wait(&mut f, &rf, "restore the SSO session", |f| f.connected).await;
    assert!(f.logged_in, "the SSO session restores");
    stop(f, ef).await;
    println!(
        "matrix-sdk: single sign-on ({:?}) signed in through the loopback redirect and restored",
        options.providers
    );
}
