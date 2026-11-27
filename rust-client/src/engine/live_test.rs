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

/// SDK logs to a file when UNTETHERED_QA_LOG names a filter.
fn logs() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let (Ok(filter), Some(file)) = (
            env::var("UNTETHERED_QA_LOG"),
            env::var_os("UNTETHERED_QA_LOG_FILE"),
        ) else {
            return;
        };
        let file = std::fs::File::create(file).unwrap();
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .try_init();
    });
}

fn session_at(dir: &Path) -> (App, Receiver<Event>, Engine) {
    logs();
    let (transfers, backend) = async_channel::bounded(64);
    let (commands, rx): (Sender<Command>, _) = async_channel::bounded(64);
    let (tx, events) = async_channel::bounded(64);
    let mut app = App::new(transfers);
    app.sdk = Some(commands);
    let dir = dir.to_path_buf();
    (
        app,
        events,
        tokio::spawn(run_in(dir, browser(), rx, backend, tx)),
    )
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

/// The neighbouring posts that break the order, with their event IDs, to
/// look up in the SDK traces.
fn disorder(app: &App, room: &str) -> String {
    let posts: Vec<(usize, Option<String>)> = app.live.list[room]
        .posts()
        .filter_map(|p| {
            let n = p.text().strip_prefix("history ")?.parse().ok()?;
            Some((n, p.event_id.clone()))
        })
        .collect();
    posts
        .windows(2)
        .filter(|w| w[0].0 > w[1].0)
        .map(|w| format!("{} {:?} before {} {:?}", w[0].0, w[0].1, w[1].0, w[1].1))
        .collect::<Vec<_>>()
        .join("; ")
}

/// An image big enough for a thumbnail and a text file, sent encrypted
/// through the attachment tray with a caption; then an image from the gomuks
/// peer, shown inline and saved.
async fn attachments(app: &mut App, rx: &Receiver<Event>, room: &str, signals: &Path) {
    let files = tempfile::tempdir_in(signals).unwrap();
    let image_path = files.path().join("photo.png");
    let mut image = image::RgbImage::new(1200, 900);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = image::Rgb([(x % 256) as u8, (y % 256) as u8, 160]);
    }
    image.save(&image_path).unwrap();
    let file_path = files.path().join("notes.txt");
    std::fs::write(
        &file_path,
        format!("notes from matrix-sdk {}", uuid::Uuid::new_v4()),
    )
    .unwrap();
    let staged = [&image_path, &file_path]
        .into_iter()
        .map(|p| crate::media::LocalFile::from_path(p.clone()).unwrap())
        .collect();
    let _ = app.update(Msg::Files(room.into(), staged));
    let caption = format!("caption from matrix-sdk {}", uuid::Uuid::new_v4());
    app.editor = text_editor::Content::with_text(&caption);
    let _ = app.update(Msg::Send);
    assert!(app.batch.is_some(), "the tray started sending");
    wait(app, rx, "attachments delivered", |a| {
        a.batch.is_none() && !a.outbox.rooms.contains_key(room)
    })
    .await;
    assert!(
        app.editor.text().trim().is_empty(),
        "the caption left the composer"
    );
    // (text, media, event ID) of our post with that attachment.
    let mine = |a: &App, name: &str| {
        a.live.list[room]
            .posts()
            .find(|p| {
                p.sender == a.matrix_id
                    && p.event_id.is_some()
                    && p.media.as_ref().is_some_and(|m| m.name == name)
            })
            .map(|p| {
                (
                    p.text().to_owned(),
                    p.media.clone().unwrap(),
                    p.event_id.clone(),
                )
            })
    };
    wait(app, rx, "attachments in the timeline", |a| {
        mine(a, "photo.png").is_some() && mine(a, "notes.txt").is_some()
    })
    .await;
    let (text, media, photo) = mine(app, "photo.png").unwrap();
    let (_, _, notes) = mine(app, "notes.txt").unwrap();
    assert_eq!(text, caption, "the first file carries the caption");
    let preview = media.preview.clone().expect("the image has a preview");
    assert!(media.encrypted && preview.encrypted);
    assert_ne!(preview.mxc, media.mxc, "a thumbnail went with the image");
    assert!(
        inline(app, preview).await,
        "the encrypted thumbnail shows inline"
    );
    let saved = files.path().join("saved.png");
    save(app, rx, media, &saved).await;
    assert_eq!(
        std::fs::read(&saved).unwrap(),
        std::fs::read(&image_path).unwrap()
    );
    std::fs::write(
        signals.join("sdk-media"),
        serde_json::json!({
            "image": photo, "file": notes, "caption": caption,
            "image_path": image_path, "file_path": file_path,
        })
        .to_string(),
    )
    .unwrap();
    println!(
        "matrix-sdk: sent an encrypted image (with thumbnail) and a file; previewed and saved the image"
    );

    let theirs: Value = serde_json::from_str(&signal(signals, "peer-media").await).unwrap();
    let name = theirs["name"].as_str().unwrap().to_owned();
    let peer = |a: &App| {
        a.live.list[room]
            .posts()
            .find(|p| p.sender != a.matrix_id && p.media.as_ref().is_some_and(|m| m.name == name))
            .and_then(|p| p.media.clone())
    };
    wait(app, rx, "the peer's image", |a| peer(a).is_some()).await;
    let media = peer(app).unwrap();
    assert!(media.encrypted);
    assert!(
        inline(
            app,
            media
                .preview
                .clone()
                .expect("a small image previews itself")
        )
        .await,
        "the peer's image shows inline"
    );
    let saved = files.path().join("from-gomuks.png");
    save(app, rx, media, &saved).await;
    assert_eq!(
        std::fs::read(&saved).unwrap(),
        std::fs::read(theirs["path"].as_str().unwrap()).unwrap()
    );
    println!("matrix-sdk: showed and saved the gomuks peer's encrypted image");
}

/// Room search over stored history, with edits applied, and the mentions
/// inbox. `edit` is (original text, its event, the edited text).
async fn search_and_mentions(
    app: &mut App,
    rx: &Receiver<Event>,
    room: &str,
    mentions_room: &str,
    edit: (&str, &str, &str),
    peer: &str,
) {
    let _ = app.search_ready(room.into(), crate::search::Bookmark::default());
    assert!(app.search.is_some(), "search opened");
    async fn find(app: &mut App, rx: &Receiver<Event>, term: &str) -> Vec<(String, String)> {
        app.search.as_mut().unwrap().input = term.into();
        app.submit_search();
        wait(app, rx, "search", |a| {
            a.search.as_ref().is_some_and(|s| !s.busy)
        })
        .await;
        let s = app.search.as_ref().unwrap();
        assert!(!s.failed, "search failed");
        s.hits
            .iter()
            .map(|h| (h.id.clone(), h.snippet.clone()))
            .collect()
    }
    let hits = find(app, rx, "history 4").await;
    assert_eq!(hits.len(), 11, "history 4 and 40-49: {hits:?}");
    assert!(
        hits.iter()
            .all(|(_, snippet)| snippet.contains("history 4"))
    );
    // Our edit is found under the original event once its echo is stored;
    // the original text no longer matches.
    let (original, id, edited) = edit;
    let new_word = edited.rsplit(' ').next().unwrap();
    let mut found = false;
    for _ in 0..20 {
        if find(app, rx, new_word)
            .await
            .iter()
            .any(|(hit, _)| hit == id)
        {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        found,
        "the edited text is searchable under the original event"
    );
    let old_word = original.rsplit(' ').next().unwrap();
    assert!(
        find(app, rx, old_word).await.is_empty(),
        "replaced text is not found"
    );
    // The encrypted room was searched on this device, back to its start.
    let reach = app.search.as_ref().unwrap().reach.clone();
    assert!(
        reach
            .iter()
            .any(|r| r.room == room && !r.server && r.oldest.is_some()),
        "searched on this device: {reach:?}"
    );
    app.clear_search();
    println!("matrix-sdk: search found stored history and edited text, on this device");

    app.open_mentions();
    wait(app, rx, "mentions inbox", |a| {
        !a.pending.values().any(|p| matches!(p, Request::Mentions))
            && a.mentions.as_ref().is_some_and(|i| {
                i.hits
                    .iter()
                    .any(|h| h.room == mentions_room && h.sender == peer)
            })
    })
    .await;
    app.close_mentions();
    println!("matrix-sdk: the mentions inbox found the peer's mention");
}

/// An image in an unencrypted room: a plain upload, previewed through the
/// homeserver's thumbnail.
async fn plain_attachment(app: &mut App, rx: &Receiver<Event>, room: &str, signals: &Path) {
    let _ = app.update(Msg::Open(room.into()));
    wait(app, rx, "unencrypted room open", |a| {
        a.selected == room && a.live.list[room].has_metadata
    })
    .await;
    assert!(!app.live.list[room].encrypted);
    let files = tempfile::tempdir_in(signals).unwrap();
    let path = files.path().join("plain.png");
    image::RgbImage::from_pixel(64, 48, image::Rgb([20, 120, 200]))
        .save(&path)
        .unwrap();
    let staged = vec![crate::media::LocalFile::from_path(path.clone()).unwrap()];
    let _ = app.update(Msg::Files(room.into(), staged));
    let _ = app.update(Msg::Send);
    wait(app, rx, "plain attachment delivered", |a| {
        a.batch.is_none() && !a.outbox.rooms.contains_key(room)
    })
    .await;
    let found = |a: &App| {
        a.live.list[room]
            .posts()
            .find(|p| {
                p.event_id.is_some() && p.media.as_ref().is_some_and(|m| m.name == "plain.png")
            })
            .and_then(|p| p.media.clone())
    };
    wait(app, rx, "plain attachment in the timeline", |a| {
        found(a).is_some()
    })
    .await;
    let media = found(app).unwrap();
    assert!(!media.encrypted && !media.mxc.is_empty());
    assert!(inline(app, media.preview.clone().unwrap()).await);
    let saved = files.path().join("saved-plain.png");
    save(app, rx, media, &saved).await;
    assert_eq!(
        std::fs::read(&saved).unwrap(),
        std::fs::read(&path).unwrap()
    );
    println!("matrix-sdk: sent, previewed and saved an image in an unencrypted room");
}

/// Ask the engine for an inline preview, as the timeline does.
async fn inline(app: &App, source: crate::media::PreviewRef) -> bool {
    let (result, image) = async_channel::bounded(1);
    app.commands
        .send(crate::backend::Command::Inline { source, result })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), image.recv())
        .await
        .expect("inline preview answered")
        .unwrap()
        .is_some()
}

async fn save(app: &mut App, rx: &Receiver<Event>, media: crate::media::Media, to: &Path) {
    let _ = app.update(Msg::SaveTo(media, Some(to.to_path_buf())));
    assert!(app.pending.values().any(|p| matches!(p, Request::Download)));
    wait(app, rx, "download", |a| {
        !a.pending.values().any(|p| matches!(p, Request::Download))
    })
    .await;
    assert_eq!(app.status, "Attachment saved.");
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

/// The seeded device in UNTETHERED_PROFILE (opened with UNTETHERED_KEYRING=off)
/// sends a 16 MiB file through the fixture's throttling gateway, cancels it
/// mid-upload, then retries. `scripts/qa_upload_cancel.py` checks the
/// transport, Synapse's media and events, and the decrypted event (from the
/// QA event log); files in UNTETHERED_UPLOAD_QA pace both sides
/// (`native.cancelled`, `native.retry`, `native.sent`, `native.checked`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run scripts/test-media-interop.py --upload-cancel (disposable throttled server)"]
async fn throttled_encrypted_upload_cancel_retry() {
    let dir = PathBuf::from(env::var_os("UNTETHERED_UPLOAD_QA").expect("throttle fixture"));
    let f: Value =
        serde_json::from_slice(&std::fs::read(dir.join("fixture.json")).unwrap()).unwrap();
    let server = url::Url::parse(f["server"].as_str().unwrap()).unwrap();
    assert_eq!(server.host_str(), Some("127.0.0.1"));
    let room = f["room"].as_str().unwrap();
    let (mut app, rx, engine) = session();
    wait(&mut app, &rx, "the seeded device synced its room", |a| {
        a.verified && a.live.list.contains_key(room) && !a.media_config_pending()
    })
    .await;
    app.open(room.into());
    app.editor = text_editor::Content::with_text("native cancel proof");
    let file = crate::media::LocalFile::from_path(dir.join("throttled.bin")).unwrap();
    let _ = app.update(Msg::Files(room.into(), vec![file]));
    assert!(
        app.outbox.rooms.get(room).is_some_and(|q| q.len() == 1),
        "the file staged; limit={}",
        app.media_limit
    );
    let _ = app.update(Msg::Send);
    wait(&mut app, &rx, "upload progress short of complete", |a| {
        a.outbox.rooms[room][0]
            .upload
            .as_ref()
            .and_then(|u| u.percent)
            .is_some_and(|p| (1..100).contains(&p))
    })
    .await;
    let a = &app.outbox.rooms[room][0];
    println!("Native attachment card: {}", a.status_label());
    let (id, request) = (a.id, a.upload.as_ref().unwrap().request);
    let _ = app.update(Msg::CancelUpload(room.into(), id, request));
    assert!(app.outbox.rooms[room][0].cancelled && app.batch.is_none());
    assert_eq!(app.editor.text(), "native cancel proof");
    // The fixture sees the truncated request body and asks Synapse before
    // letting this retry.
    std::fs::write(dir.join("native.cancelled"), "").unwrap();
    signal(&dir, "native.retry").await;
    let _ = app.update(Msg::Send);
    wait(&mut app, &rx, "one confirmed retry", |a| {
        a.batch.is_none()
            && a.outbox.rooms.is_empty()
            && a.live.list[room].posts().any(|p| {
                p.text() == "native cancel proof"
                    && p.event_id.is_some()
                    && p.media.as_ref().is_some_and(|m| m.encrypted)
            })
    })
    .await;
    // The fixture then reads the event as this device's sync brought it back
    // (the QA event log), so keep syncing until it has.
    std::fs::write(dir.join("native.sent"), "").unwrap();
    signal(&dir, "native.checked").await;
    stop(app, engine).await;
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
    // Markdown: the peer checks the formatted body.
    let mine = format!("**matrix-sdk** to gomuks {}", uuid::Uuid::new_v4());

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
    let first = app.live.list[room].posts().len();
    // "View original" for an event outside the loaded window uses /context.
    // Ask for it directly: jump() would show it locally once paginated.
    let old = fixture["old_event"].as_str().unwrap();
    app.engine_rpc(
        "get_event_context",
        Value::Null,
        Request::Context(room.into(), old.into()),
    );
    wait(&mut app, &rx, "event context", |a| a.original.is_some()).await;
    let context = app.original.as_ref().unwrap();
    let found = context
        .slice
        .as_ref()
        .and_then(|s| s.by_id(old))
        .map(|p| p.text().to_owned());
    assert_eq!(found.as_deref(), Some("history 3"));
    println!("matrix-sdk: View original loaded an event outside the timeline through /context");
    app.close_original();
    let older = fixture["history"].as_u64().unwrap() as usize;
    paginate_to_start(&mut app, &rx, room).await;
    let loaded = app.live.list[room].posts().len();
    assert!(
        loaded >= older,
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
        "history out of order after pagination: {order:?} ({})",
        disorder(&app, room)
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
    #[cfg(target_os = "linux")]
    {
        wait(&mut app, &rx, "an alert for the peer's message", |a| {
            a.queued_alerts().iter().any(|(_, body)| *body == theirs)
        })
        .await;
        println!(
            "matrix-sdk: the account's push rules raised an alert for the peer's encrypted message"
        );
    }

    // Mark the room read at the peer's message, before our own reaction and
    // edit count as reading it.
    wait(&mut app, &rx, "unread from the peer", |a| {
        a.live.list[room].unread
    })
    .await;
    let target = app.live.list[room].read.target().map(str::to_owned);
    app.mark_room(room.to_owned(), target);
    wait(&mut app, &rx, "room read", |a| {
        !a.live.list[room].unread
            && !a
                .pending
                .values()
                .any(|p| matches!(p, Request::ReadState(..)))
    })
    .await;
    // React to the peer's message, edit ours, and let gomuks check both.
    let theirs_id = app.live.list[room]
        .posts()
        .find(|p| p.text() == theirs)
        .and_then(|p| p.event_id.clone())
        .unwrap();
    let mine_id = app.live.list[room]
        .posts()
        .find(|p| p.text() == mine)
        .and_then(|p| p.event_id.clone())
        .unwrap();
    let op = crate::reactions::Toggle::for_post(room, app.visible_post(&theirs_id).unwrap(), "👍")
        .expect("the peer's message is reactable");
    app.toggle_reaction(op);
    wait(&mut app, &rx, "own reaction", |a| {
        a.visible_post(&theirs_id)
            .and_then(|p| p.chips.get("👍"))
            .is_some_and(|c| c.count == 1 && c.mine().is_some())
    })
    .await;
    let edited = format!("edited from matrix-sdk {}", uuid::Uuid::new_v4());
    assert!(app.start_edit(mine_id.clone()), "our message is editable");
    app.editing.as_mut().unwrap().input = text_editor::Content::with_text(&edited);
    app.save_edit();
    wait(&mut app, &rx, "edit applied", |a| {
        a.editing.is_none()
            && a.visible_post(&mine_id)
                .is_some_and(|p| p.text() == edited && p.revision.is_some())
    })
    .await;
    std::fs::write(
        signals.join("sdk-acted"),
        serde_json::json!({"edit": edited, "edited": mine_id, "reacted": theirs_id, "key": "👍"})
            .to_string(),
    )
    .unwrap();
    signal(&signals, "peer-confirmed").await;
    println!("matrix-sdk: reaction and edit shown here and decrypted by the gomuks peer");

    search_and_mentions(
        &mut app,
        &rx,
        room,
        unread,
        (&mine, &mine_id, &edited),
        fixture["peer"].as_str().unwrap(),
    )
    .await;

    attachments(&mut app, &rx, room, &signals).await;
    plain_attachment(&mut app, &rx, unread, &signals).await;
    let _ = app.update(Msg::Open(room.into()));

    assert!(app.begin_report(room, &theirs_id));
    let _ = app.update(Msg::ReportReason("qa report".into()));
    app.submit_report();
    wait(&mut app, &rx, "report sent", |a| a.reporting.is_none()).await;
    assert_eq!(app.status, "Report sent to your homeserver.");
    std::fs::write(signals.join("sdk-reported"), &theirs_id).unwrap();
    println!("matrix-sdk: marked the room read and reported the peer's message");
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
        has(a, room, &edited) && has(a, room, &theirs)
    })
    .await;
    paginate_to_start(&mut app, &rx, room).await;
    let order = history_order(&app, room);
    assert!(
        order.len() == older && order.is_sorted(),
        "restored history incomplete or out of order: {order:?} ({})",
        disorder(&app, room)
    );
    println!("matrix-sdk: restart restored the session, both encrypted messages and the edit");
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
#[ignore = "run UNTETHERED_QA_ONLY=delete-drafts scripts/qa_matrix_sdk.py with disposable Synapse"]
async fn matrix_sdk_delete_and_drafts() {
    use crate::context_menu::Event as Menu;
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(env::var("UNTETHERED_QA_FIXTURE").unwrap()).unwrap())
            .unwrap();
    let base = PathBuf::from(env::var_os("UNTETHERED_QA_PROFILES").unwrap());
    let server = fixture["server"].as_str().unwrap();
    let room = fixture["room"].as_str().unwrap();
    let owner_dir = base.join("delete-owner");
    let (mut owner, owner_events, owner_engine) = sign_in(
        &owner_dir,
        server,
        fixture["username"].as_str().unwrap(),
        fixture["password"].as_str().unwrap(),
    )
    .await;
    owner.recovery = rooms::string(&fixture["key"]);
    let _ = owner.update(Msg::Verify);
    let (mut peer, peer_events, peer_engine) = sign_in(
        &base.join("delete-peer"),
        server,
        fixture["peer"].as_str().unwrap(),
        fixture["peer_password"].as_str().unwrap(),
    )
    .await;
    peer.recovery = rooms::string(&fixture["peer_key"]);
    let _ = peer.update(Msg::Verify);
    wait2(
        &mut owner,
        &owner_events,
        &mut peer,
        &peer_events,
        "both verified",
        |a, b| {
            a.verified
                && b.verified
                && a.live.list.contains_key(room)
                && b.live.list.contains_key(room)
        },
    )
    .await;
    let _ = owner.update(Msg::Open(room.into()));
    let _ = peer.update(Msg::Open(room.into()));

    let body = "Message to remove from both clients";
    peer.editor = text_editor::Content::with_text(body);
    let _ = peer.update(Msg::Send);
    wait2(
        &mut owner,
        &owner_events,
        &mut peer,
        &peer_events,
        "sent message on both clients",
        |a, b| has(a, room, body) && has(b, room, body),
    )
    .await;
    let event = owner.live.list[room]
        .posts()
        .find(|p| p.text() == body)
        .and_then(|p| p.event_id.clone())
        .unwrap();
    assert!(peer.can_delete(room, &event));
    assert!(owner.can_delete(room, &event));
    let _ = owner.update(Msg::ContextMenu(Menu::Delete {
        room: room.into(),
        event: event.clone(),
    }));
    assert!(owner.context_menu.deleting.is_some());
    let _ = owner.update(Msg::ContextMenu(Menu::ConfirmDelete));
    wait2(
        &mut owner,
        &owner_events,
        &mut peer,
        &peer_events,
        "redaction on both clients",
        |a, b| {
            [a, b].into_iter().all(|app| {
                app.live.list[room]
                    .by_id(&event)
                    .is_none_or(|post| !post.reactable && post.text() == "Message removed")
            })
        },
    )
    .await;

    let own = "Remove my own message";
    owner.editor = text_editor::Content::with_text(own);
    let _ = owner.update(Msg::Send);
    wait2(
        &mut owner,
        &owner_events,
        &mut peer,
        &peer_events,
        "own message on both clients",
        |a, b| has(a, room, own) && has(b, room, own),
    )
    .await;
    let own_event = owner.live.list[room]
        .posts()
        .find(|p| p.text() == own)
        .and_then(|p| p.event_id.clone())
        .unwrap();
    assert!(!peer.can_delete(room, &own_event));
    let _ = owner.update(Msg::ContextMenu(Menu::Delete {
        room: room.into(),
        event: own_event.clone(),
    }));
    let _ = owner.update(Msg::ContextMenu(Menu::ConfirmDelete));
    wait2(
        &mut owner,
        &owner_events,
        &mut peer,
        &peer_events,
        "own redaction on both clients",
        |a, b| {
            [a, b].into_iter().all(|app| {
                app.live.list[room]
                    .by_id(&own_event)
                    .is_none_or(|post| !post.reactable && post.text() == "Message removed")
            })
        },
    )
    .await;

    let attachment = base.join("captioned.txt");
    std::fs::write(&attachment, b"Test attachment").unwrap();
    let file = crate::media::LocalFile::from_path(attachment).unwrap();
    let _ = owner.update(Msg::Files(room.into(), vec![file]));
    assert_eq!(owner.outbox.rooms[room].len(), 1);
    let draft = "Caption retained across restart";
    let _ = owner.update(Msg::Draft(text_editor::Action::Edit(
        text_editor::Edit::Paste(Arc::new(draft.to_owned())),
    )));
    owner.persist_drafts();
    drop(owner_events);
    stop(owner, owner_engine).await;
    let (mut restored, restored_events, restored_engine) = session_at(&owner_dir);
    wait(
        &mut restored,
        &restored_events,
        "restored account and room",
        |a| a.verified && a.live.list.contains_key(room),
    )
    .await;
    let _ = restored.update(Msg::Open(room.into()));
    assert_eq!(restored.editor.text(), draft);
    stop(restored, restored_engine).await;
    stop(peer, peer_engine).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[ignore = "run UNTETHERED_QA_ONLY=profiles scripts/qa_matrix_sdk.py with disposable Synapse"]
async fn matrix_sdk_profile_interop() {
    use super::profile::Action as ProfileAction;
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(env::var("UNTETHERED_QA_FIXTURE").unwrap()).unwrap())
            .unwrap();
    let base = PathBuf::from(env::var_os("UNTETHERED_QA_PROFILES").unwrap());
    let server = fixture["server"].as_str().unwrap();
    let room = fixture["room"].as_str().unwrap();
    let dm = fixture["dm"].as_str().unwrap();
    let peer = fixture["peer"].as_str().unwrap();
    let (mut owner, owner_events, owner_engine) = sign_in(
        &base.join("profile-owner"),
        server,
        fixture["username"].as_str().unwrap(),
        fixture["password"].as_str().unwrap(),
    )
    .await;
    owner.recovery = rooms::string(&fixture["key"]);
    let _ = owner.update(Msg::Verify);
    let (mut observer, observer_events, observer_engine) = sign_in(
        &base.join("profile-observer"),
        server,
        peer,
        fixture["peer_password"].as_str().unwrap(),
    )
    .await;
    wait2(
        &mut owner,
        &owner_events,
        &mut observer,
        &observer_events,
        "verified owner and both joined room lists",
        |a, b| {
            a.verified
                && a.live.list.contains_key(room)
                && a.live.list.contains_key(dm)
                && b.live.list.contains_key(room)
        },
    )
    .await;
    let owner_id = owner.matrix_id.clone();
    let profile = |action| Msg::Sdk(Action::Profile(action));
    owner.account = true;
    let _ = owner.update(profile(ProfileAction::Name("Profile QA".into())));
    let _ = owner.update(profile(ProfileAction::SaveName));
    wait2(
        &mut owner,
        &owner_events,
        &mut observer,
        &observer_events,
        "saved display name arrives in the other device's member list",
        |a, b| {
            a.profile == "Profile QA"
                && b.live.list[room]
                    .members
                    .get(&owner_id)
                    .is_some_and(|name| name == "Profile QA")
        },
    )
    .await;

    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    // Supply the picker's result without requiring a desktop file dialog.
    owner.sdk_ui.profile.picking = true;
    let _ = owner.update(profile(ProfileAction::Picked(
        0,
        Ok(Some(png.into_inner())),
    )));
    wait2(
        &mut owner,
        &owner_events,
        &mut observer,
        &observer_events,
        "uploaded avatar arrives in the other device's member list",
        |a, b| {
            a.own_avatar
                .as_ref()
                .is_some_and(|uri| b.live.list[room].avatars.get(&owner_id) == Some(uri))
        },
    )
    .await;
    let _ = owner.update(profile(ProfileAction::RemoveAvatar));
    wait2(
        &mut owner,
        &owner_events,
        &mut observer,
        &observer_events,
        "removed avatar clears on both devices",
        |a, b| a.own_avatar.is_none() && !b.live.list[room].avatars.contains_key(&owner_id),
    )
    .await;

    owner.account = false;
    let dm_count = owner.live.list.values().filter(|room| room.dm).count();
    for _ in 0..2 {
        let _ = owner.update(profile(ProfileAction::Open {
            room: room.into(),
            user: peer.into(),
        }));
        let _ = owner.update(profile(ProfileAction::Message));
        assert_eq!(owner.selected, dm);
        assert!(!owner.sdk_ui.profile.opening_dm());
    }
    assert_eq!(
        owner.live.list.values().filter(|room| room.dm).count(),
        dm_count
    );
    stop(owner, owner_engine).await;
    stop(observer, observer_engine).await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run scripts/qa_matrix_sdk.py (disposable Synapse and a gomuks peer)"]
async fn matrix_sdk_rooms_spaces_and_moderation() {
    use crate::{community::Input, moderation, permissions};
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(env::var("UNTETHERED_QA_FIXTURE").unwrap()).unwrap())
            .unwrap();
    let base = PathBuf::from(env::var_os("UNTETHERED_QA_PROFILES").unwrap());
    let server = fixture["server"].as_str().unwrap();
    let (user, pass) = (
        rooms::string(&fixture["username"]),
        rooms::string(&fixture["password"]),
    );
    let room = fixture["unread_room"].as_str().unwrap();
    let peer = fixture["peer"].as_str().unwrap();
    let (mut app, rx, engine) = sign_in(&base.join("rooms"), server, &user, &pass).await;
    app.recovery = rooms::string(&fixture["key"]);
    let _ = app.update(Msg::Verify);
    wait(&mut app, &rx, "verified", |a| a.verified).await;
    wait(&mut app, &rx, "room permissions from sync", |a| {
        a.live
            .list
            .get(room)
            .is_some_and(|r| r.posting.id().is_some())
    })
    .await;
    let _ = app.update(Msg::Open(room.into()));

    // Posting: read-only for members, then restored, each checked against a
    // fresh read of the room's state.
    app.permission_panel = true;
    for (action, members_may_post) in [
        (permissions::Action::Restrict, false),
        (permissions::Action::Restore, true),
    ] {
        let posting = &app.live.list[room].posting;
        assert_eq!(posting.action(), Some(action));
        let base_event = posting.id().unwrap().to_owned();
        app.change_posting(permissions::Change {
            room: room.into(),
            base: base_event.clone(),
            action,
        });
        wait(&mut app, &rx, "posting change", |a| {
            !a.posting_busy(room) && a.live.list[room].posting.id() != Some(&base_event)
        })
        .await;
        assert_eq!(
            app.live.list[room].posting.can_post(peer, false),
            Some(members_may_post)
        );
    }
    app.leave_permission_panel();
    println!("matrix-sdk: made a room read-only for members and restored posting");

    // Moderation: kick the peer, then invite them back.
    app.people = true;
    app.read_directory(None);
    let kind = |a: &App| a.directory.users.get(peer).map(|m| m.kind);
    wait(&mut app, &rx, "member directory", |a| {
        kind(a) == Some(moderation::Kind::Joined)
    })
    .await;
    for (action, after) in [
        (moderation::Action::Kick, moderation::Kind::Left),
        (moderation::Action::Invite, moderation::Kind::Invited),
    ] {
        app.directory.choose(Some(peer.into()));
        let change = app.member_change(action).expect("an admin may do this");
        app.moderate(change);
        wait(&mut app, &rx, "membership change", |a| {
            kind(a) == Some(after) && !a.membership_busy()
        })
        .await;
    }
    app.people = false;
    println!(
        "matrix-sdk: kicked the gomuks peer and invited them back, checked against fresh room state"
    );

    // Alerts: each change waits for the push rules to come back from sync.
    #[cfg(target_os = "linux")]
    {
        use crate::notifications::Mode;
        wait(&mut app, &rx, "push rules", |a| a.alerts.dnd.is_some()).await;
        for mode in [Mode::Quiet, Mode::All, Mode::Default] {
            app.set_notifications(room.into(), mode);
            wait(&mut app, &rx, "room alert mode", |a| {
                a.alerts.mode(room) == mode && !a.notifications_pending(room)
            })
            .await;
        }
        for on in [true, false] {
            app.change_dnd(on);
            wait(&mut app, &rx, "do not disturb", |a| {
                a.alerts.dnd == Some(on) && !a.changing_dnd()
            })
            .await;
        }
        println!(
            "matrix-sdk: room alert modes and Do Not Disturb round-tripped through push rules"
        );
    }

    // Community: a space, then an encrypted channel linked into it.
    let _ = app.update(Msg::Setup(Input::Start(None)));
    let _ = app.update(Msg::Setup(Input::Name("SDK made space".into())));
    let _ = app.update(Msg::Setup(Input::Submit));
    wait(&mut app, &rx, "space created", |a| {
        a.setup.as_ref().is_some_and(|f| f.done)
    })
    .await;
    let space = app.setup.as_ref().unwrap().created.clone().unwrap();
    let _ = app.update(Msg::Setup(Input::Close));
    wait(&mut app, &rx, "space synced", |a| {
        a.live.list.get(&space).is_some_and(|r| r.space)
    })
    .await;
    let _ = app.update(Msg::Setup(Input::Start(Some(space.clone()))));
    let _ = app.update(Msg::Setup(Input::Name("sdk-channel".into())));
    let _ = app.update(Msg::Setup(Input::Submit));
    wait(&mut app, &rx, "channel created and linked", |a| {
        a.setup.as_ref().is_some_and(|f| f.done)
    })
    .await;
    let channel = app.setup.as_ref().unwrap().created.clone().unwrap();
    wait(&mut app, &rx, "channel in the space", |a| {
        a.live
            .children
            .get(&space)
            .is_some_and(|c| c.contains(&channel))
            && a.live.list.get(&channel).is_some_and(|r| r.encrypted)
    })
    .await;
    println!("matrix-sdk: created a space and an encrypted channel linked into it");

    // Space alerts: the preference lives on the space, and the App keeps an
    // underride rule on each joined room in it.
    #[cfg(target_os = "linux")]
    {
        use crate::notifications::Mode;
        app.section = crate::Section::Space(space.clone());
        app.set_space_mode(space.clone(), Mode::Quiet);
        wait(&mut app, &rx, "space alert rule on the channel", |a| {
            a.live.list[&space].space_alerts == Mode::Quiet
                && a.alerts.spaces.installed(&channel) == Some(Mode::Quiet)
        })
        .await;
        println!("matrix-sdk: a space alert preference installed its push rule on the channel");
    }
    stop(app, engine).await;
}

/// Upgraded rooms as one channel (upgrades.rs); `scripts/qa_room_upgrades.py`
/// seeds the rooms and documents them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run scripts/qa_room_upgrades.py (disposable Synapse)"]
async fn matrix_sdk_room_upgrades() {
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(env::var("UNTETHERED_QA_FIXTURE").unwrap()).unwrap())
            .unwrap();
    let base = PathBuf::from(env::var_os("UNTETHERED_QA_PROFILES").unwrap());
    let ids = |key: &str| -> Vec<String> {
        fixture[key]
            .as_array()
            .unwrap()
            .iter()
            .map(rooms::string)
            .collect()
    };
    let (general, before, after) = (ids("general"), ids("before"), ids("after"));
    let (lounge, archive) = (ids("lounge"), ids("archive"));
    let space = rooms::string(&fixture["space"]);
    let newest = general[2].clone();
    let (mut app, rx, engine) = sign_in(
        &base.join("upgrades"),
        fixture["server"].as_str().unwrap(),
        &rooms::string(&fixture["username"]),
        &rooms::string(&fixture["password"]),
    )
    .await;
    wait(&mut app, &rx, "recovery state known", |a| {
        a.sdk_ui.recovery != super::Recovery::Unknown
    })
    .await;
    let _ = app.update(Msg::Sdk(Action::Password(rooms::string(
        &fixture["password"],
    ))));
    let _ = app.update(Msg::Sdk(Action::SetUpRecovery));
    wait(&mut app, &rx, "recovery set up", |a| {
        a.sdk_ui.recovery_key.is_some() && a.verified && a.sdk_ui.jobs.is_empty()
    })
    .await;
    std::fs::write(
        base.join("upgrades/recovery-key"),
        app.sdk_ui.recovery_key.take().unwrap(),
    )
    .unwrap();

    // The list: one "general", also in the space that lists its first
    // version; the lounge waits for us to join its new room.
    wait(&mut app, &rx, "rooms listed", |a| {
        [&newest, &space, &lounge[0], &archive[1]]
            .iter()
            .all(|id| a.live.list.contains_key(*id))
            && a.live.list[&lounge[0]].successor.is_some()
            && a.live
                .ordered_spaces
                .get(&space)
                .is_some_and(|c| !c.is_empty())
    })
    .await;
    assert!(!app.live.list.contains_key(&general[0]) && !app.live.list.contains_key(&general[1]));
    assert_eq!(app.live.newest(&general[0]), newest);
    assert_eq!(
        app.live.ordered_spaces[&space],
        std::slice::from_ref(&newest)
    );
    let names: Vec<_> = app
        .live
        .ordered_rooms
        .iter()
        .map(|id| app.live.list[id].name.as_str())
        .collect();
    assert_eq!(
        names.iter().filter(|n| **n == "general").count(),
        1,
        "{names:?}"
    );
    assert_eq!(
        app.live.list[&lounge[0]].successor.as_deref(),
        Some(lounge[1].as_str())
    );
    assert!(!app.live.list.contains_key(&lounge[1]), "only invited");
    println!(
        "matrix-sdk: the list shows one general (also in the space that lists its first version); the lounge offers its new room"
    );

    // A link into the first version opens the newest at that message, which
    // the engine finds in the older room before any history is loaded.
    let _ = app.update(Msg::Section(crate::Section::All));
    let link = format!(
        "https://matrix.to/#/{}/{}?via=test.invalid",
        general[0], before[9]
    );
    let _ = app.update(Msg::OpenLink(link));
    assert_eq!(app.selected, newest);
    wait(
        &mut app,
        &rx,
        "message context from the first version",
        |a| a.original.as_ref().and_then(|o| o.slice.as_ref()).is_some(),
    )
    .await;
    let slice = app.original.as_ref().unwrap().slice.as_ref().unwrap();
    let target = slice.by_id(&before[9]).expect("the linked message");
    assert_eq!(target.text(), "before 10");
    assert_eq!(target.from.as_deref(), Some(general[0].as_str()));
    assert!(!target.reactable, "older rooms' messages are read-only");
    let _ = app.update(Msg::Latest);
    println!("matrix-sdk: a link into the first version opened the merged channel at that message");

    // Scrolling back past the newest room's start continues into both
    // older rooms, with a divider at each upgrade.
    paginate_to_start(&mut app, &rx, &newest).await;
    let r = &app.live.list[&newest];
    let posts: Vec<_> = r.posts().collect();
    let text = |i: usize| posts[i].text().to_owned();
    assert_eq!(text(0), "before 1");
    assert_eq!(posts.last().unwrap().text(), "after 5");
    assert_eq!(posts.len(), 60 + 3 + 5, "every message once");
    assert_eq!(r.seams.len(), 2, "a divider at each upgrade");
    assert!(r.seams.iter().all(|s| s.readable && s.time.is_some()));
    assert_eq!(
        (r.seams[0].older.as_str(), r.seams[0].index),
        (general[0].as_str(), 60)
    );
    assert_eq!(
        (r.seams[1].newer.as_str(), r.seams[1].index),
        (newest.as_str(), 63)
    );
    assert_eq!(text(60), "between 1");
    assert_eq!(text(63), "after 1");
    assert_eq!(posts[0].from.as_deref(), Some(general[0].as_str()));
    assert_eq!(posts[60].from.as_deref(), Some(general[1].as_str()));
    assert!(posts[63].from.is_none() && posts[63].reactable);
    assert!(posts[..63].iter().all(|p| !p.reactable && !p.text_message));
    let count = posts.len();
    // Read state follows the live room only: a marker on an older room's
    // message is in no read order, one on the live room's is.
    let read = |a: &mut App, marker: &str| {
        let r = a.live.list.get_mut(&newest).unwrap();
        let saved = r.read.fully_read.replace(marker.to_owned());
        let divider = r.read.divider(r).map(str::to_owned);
        r.read.fully_read = saved;
        divider
    };
    assert_eq!(read(&mut app, &before[0]), None);
    assert_eq!(
        read(&mut app, &after[0]).as_deref(),
        Some(after[1].as_str())
    );
    println!(
        "matrix-sdk: scrolling back reached the first version past two dividers ({count} messages)"
    );

    // Search covers the older rooms, now that their history is stored.
    let _ = app.search_ready(newest.clone(), crate::search::Bookmark::default());
    app.search.as_mut().unwrap().input = "before 4".into();
    app.submit_search();
    wait(&mut app, &rx, "search", |a| {
        a.search.as_ref().is_some_and(|s| !s.busy)
    })
    .await;
    let hits: Vec<_> = app
        .search
        .as_ref()
        .unwrap()
        .hits
        .iter()
        .map(|h| h.id.clone())
        .collect();
    assert!(hits.contains(&before[3]), "{hits:?}");
    assert!(hits.contains(&before[39]), "{hits:?}");
    app.clear_search();
    println!("matrix-sdk: search found the first version's messages from the newest room");

    // New messages go to the newest room.
    app.editor = text_editor::Content::with_text("hello from the newest room");
    let _ = app.update(Msg::Send);
    wait(&mut app, &rx, "sent to the newest room", |a| {
        a.live.list[&newest].posts().any(|p| {
            p.text() == "hello from the newest room" && p.event_id.is_some() && p.from.is_none()
        })
    })
    .await;

    // The lounge: joining its new room hides it and opens the new one.
    let _ = app.update(Msg::Open(lounge[0].clone()));
    assert_eq!(app.selected, lounge[0]);
    let _ = app.update(Msg::FollowUpgrade(lounge[0].clone()));
    wait(&mut app, &rx, "joined the lounge's new room", |a| {
        a.live.list.contains_key(&lounge[1]) && !a.live.list.contains_key(&lounge[0])
    })
    .await;
    assert_eq!(app.selected, lounge[1]);
    paginate_to_start(&mut app, &rx, &lounge[1]).await;
    let r = &app.live.list[&lounge[1]];
    assert!(
        r.posts()
            .any(|p| p.text() == "lounge before" && p.from.is_some())
    );
    assert_eq!(r.seams.len(), 1);
    println!(
        "matrix-sdk: Join the new room joined the lounge's successor, hid the old room and kept its history"
    );

    // The patio keeps its offer for a look at the UI.
    let patio = ids("patio");
    assert_eq!(
        app.live.list[&patio[0]].successor.as_deref(),
        Some(patio[1].as_str())
    );

    // The archive was upgraded before we joined: no history to read.
    let _ = app.update(Msg::Open(archive[1].clone()));
    paginate_to_start(&mut app, &rx, &archive[1]).await;
    let r = &app.live.list[&archive[1]];
    assert!(r.posts().all(|p| p.text() != "archive secret"));
    assert!(r.posts().any(|p| p.text() == "archive after"));
    assert_eq!(r.seams.len(), 1);
    assert!(!r.seams[0].readable && r.seams[0].index == 0);
    println!("matrix-sdk: the archive's divider says its older history isn't available");
    stop(app, engine).await;
}
