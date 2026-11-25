//! Opt-in acceptance test against a disposable local homeserver.
use super::*;
use async_channel::Receiver;
use std::{env, time::Duration};

async fn wait(app: &mut App, rx: &Receiver<Value>, label: &str, until: impl Fn(&App) -> bool) {
    tokio::time::timeout(Duration::from_secs(45), async {
        while !until(app) {
            let f = rx.recv().await.expect("backend exited");
            assert_ne!(
                f["command"], "error",
                "gomuks request failed during {label}"
            );
            app.frame(f);
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out during {label}"));
}
fn session() -> (
    App,
    Receiver<Value>,
    tokio::task::JoinHandle<Result<(), String>>,
) {
    let (tx, cmd) = async_channel::bounded(64);
    let (output, rx) = async_channel::bounded(64);
    (App::new(tx), rx, tokio::spawn(backend::run(cmd, output)))
}

#[tokio::test]
#[ignore = "run scripts/test-media-interop.py --rtc-membership with local Element Web"]
async fn rtc_membership_seen_by_element() {
    use untethered_iced::rtc_membership::Membership;
    let path = std::path::PathBuf::from(env::var_os("UNTETHERED_RTC_QA").unwrap());
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(path.join("rtc.json")).unwrap()).unwrap();
    let room = fixture["room"].as_str().unwrap();
    let service = url::Url::parse(fixture["service"].as_str().unwrap()).unwrap();
    assert_eq!(service.host_str(), Some("127.0.0.1"));
    let (mut app, rx, engine) = session();
    wait(&mut app, &rx, "verified member of encrypted room", |a| {
        a.verified && a.live.list.get(room).is_some_and(|r| r.encrypted)
    })
    .await;
    let mut seq = 9000;
    let mut rpc = async |cmd: &'static str, body: Value| -> Result<Value, String> {
        seq += 1;
        let frame = json!({"command":cmd,"request_id":seq,"data":body});
        app.commands
            .send(backend::Command::Rpc(frame))
            .await
            .map_err(|_| "closed")?;
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let f = rx.recv().await.map_err(|_| "backend exited")?;
                if f["request_id"] == seq {
                    match f["command"].as_str() {
                        Some("response") => return Ok(f["data"].clone()),
                        Some("error") => return Err(format!("{cmd} rejected: {}", f["data"])),
                        _ => (),
                    }
                }
            }
        })
        .await
        .map_err(|_| format!("{cmd} timed out"))?
    };
    let identity = rpc("get_state", json!({})).await.unwrap();
    let user = identity["user_id"].as_str().unwrap();
    let device = identity["device_id"].as_str().unwrap();
    std::fs::write(path.join("rtc-device"), device).unwrap();
    async fn signal(path: &std::path::Path, name: &str) {
        tokio::time::timeout(Duration::from_secs(25), async {
            while !path.join(name).is_file() {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        })
        .await
        .expect("Element observer did not acknowledge membership");
    }
    let member = Membership::join(room, user, device, &service, 8000, &mut rpc)
        .await
        .unwrap();
    signal(&path, "restart").await;
    member.heartbeat(&mut rpc).await.unwrap();
    std::fs::write(path.join("restarted"), "").unwrap();
    signal(&path, "leave").await;
    member.leave(&mut rpc).await.unwrap();
    signal(&path, "rejoin").await;
    let _abandoned = Membership::join(room, user, device, &service, 8000, &mut rpc)
        .await
        .unwrap();
    // Stop the actual transport without sending a leave or any more heartbeats.
    app.commands.close();
    engine.await.unwrap().unwrap();
}

#[tokio::test]
#[ignore = "run scripts/test-media-interop.py --upload-cancel (disposable throttled server)"]
async fn throttled_encrypted_upload_cancel_retry() {
    let dir =
        std::path::PathBuf::from(env::var_os("UNTETHERED_UPLOAD_QA").expect("throttle fixture"));
    let f: Value =
        serde_json::from_slice(&std::fs::read(dir.join("fixture.json")).unwrap()).unwrap();
    assert_eq!(
        url::Url::parse(f["server"].as_str().unwrap())
            .unwrap()
            .host_str(),
        Some("127.0.0.1")
    );
    let room = f["room"].as_str().unwrap();
    let (mut app, rx, backend) = session();
    wait(&mut app, &rx, "synced seeded test device", |a| {
        a.verified && a.live.list.contains_key(room) && !a.media_config_pending()
    })
    .await;
    app.open(room.into());
    app.editor = text_editor::Content::with_text("native cancel proof");
    let path = dir.join("throttled.bin");
    let _ = app.update(Msg::Files(
        room.into(),
        vec![media::LocalFile::from_path(path).unwrap()],
    ));
    assert!(
        app.outbox.rooms.get(room).is_some_and(|q| q.len() == 1),
        "file must stage before test; limit={}",
        app.media_limit
    );
    let _ = app.update(Msg::Send);
    wait(&mut app, &rx, "nonterminal progress from gomuks", |a| {
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
    // The driver independently observes a truncated TCP body and queries
    // Synapse before letting this test retry.
    std::fs::write(dir.join("native.cancelled"), "").unwrap();
    tokio::time::timeout(Duration::from_secs(40), async {
        while !dir.join("native.retry").is_file() {
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
    })
    .await
    .expect("driver allowed retry");
    let _ = app.update(Msg::Send);
    wait(&mut app, &rx, "single confirmed retry", |a| {
        a.batch.is_none()
            && a.outbox.rooms.is_empty()
            && a.live.list[room].history.values().any(|p| {
                p.body == "native cancel proof" && p.media.as_ref().is_some_and(|m| m.encrypted)
            })
    })
    .await;
    app.commands.close();
    backend.await.unwrap().unwrap();
}
#[tokio::test]
#[ignore = "requires a disposable loopback homeserver and a new, separate gomuks profile"]
async fn encrypted_round_trip_and_restart() {
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(env::var("UNTETHERED_QA_FIXTURE").unwrap()).unwrap())
            .unwrap();
    let server = fixture["server"].as_str().unwrap();
    assert!(url::Url::parse(server).unwrap().host_str() == Some("127.0.0.1"));
    assert!(
        !std::path::Path::new(&env::var_os("UNTETHERED_PROFILE").unwrap()).exists(),
        "Use a fresh profile."
    );
    let id = fixture["room"].as_str().unwrap();
    let body = format!("Rust client round trip {}", uuid::Uuid::new_v4());
    let (mut app, rx, engine) = session();
    wait(&mut app, &rx, "startup", |a| a.connected).await;
    assert!(!app.logged_in);
    app.server = server.into();
    app.username = rooms::string(&fixture["username"]);
    app.password = rooms::string(&fixture["password"]);
    let _ = app.update(Msg::Login);
    wait(&mut app, &rx, "login", |a| a.logged_in).await;
    // Exercise the error path before retrying. The failed request also
    // dispatches client_state; it must not make the feedback disappear.
    app.recovery = "not a recovery key".into();
    let _ = app.update(Msg::Verify);
    tokio::time::timeout(Duration::from_secs(25), async {
        while app.verifying() {
            app.frame(rx.recv().await.unwrap());
        }
    })
    .await
    .expect("failed verification returned");
    assert!(app.verification_feedback().contains("invalid"));
    app.recovery = rooms::string(&fixture["key"]).replace(' ', "\u{202f}");
    let _ = app.update(Msg::Verify);
    wait(&mut app, &rx, "verified room sync", |a| {
        a.verified && a.live.list.contains_key(id)
    })
    .await;
    app.open(id.into());
    assert!(app.live.list[id].encrypted);
    app.editor = text_editor::Content::with_text(&body);
    let _ = app.update(Msg::Send);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let f = rx.recv().await.unwrap();
            let done = f["command"] == "send_complete" && f["data"]["event"]["room_id"] == id;
            if done {
                assert!(f["data"]["error"].is_null());
            }
            app.frame(f);
            if done {
                break;
            }
        }
    })
    .await
    .expect("encrypted send completion");
    app.commands.close();
    engine.await.unwrap().unwrap();
    let (mut app, rx, engine) = session();
    wait(&mut app, &rx, "restore session and decrypt history", |a| {
        a.verified
            && !a.media_config_pending()
            && a.live
                .list
                .get(id)
                .is_some_and(|r| r.history.values().any(|p| p.body == body))
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let original = b"Untethered attachment integrity fixture\n\x00\x01\xfe\xff";
    let path = dir.path().join("round trip.bin");
    std::fs::write(&path, original).unwrap();
    let _ = app.update(Msg::Files(
        id.into(),
        vec![media::LocalFile::from_path(path).unwrap()],
    ));
    app.editor = text_editor::Content::with_text("desktop attachment");
    let _ = app.update(Msg::Send);
    wait(&mut app, &rx, "encrypted media event", |a| {
        a.batch.is_none()
            && a.live.list[id].history.values().any(|p| {
                p.media
                    .as_ref()
                    .is_some_and(|m| m.name == "round trip.bin" && m.encrypted)
            })
    })
    .await;
    assert!(
        !app.outbox.rooms.contains_key(id),
        "delivery succeeded but the retry card is still visible"
    );
    assert!(
        !app.live.list[id]
            .history
            .values()
            .any(|p| p.body.contains("Failed to send"))
    );
    let file = app.live.list[id]
        .history
        .values()
        .find_map(|p| p.media.clone())
        .unwrap();
    let saved = dir.path().join("saved.bin");
    let _ = app.update(Msg::SaveTo(file, Some(saved.clone())));
    wait(&mut app, &rx, "decrypt and save attachment", |a| {
        !a.pending.values().any(|p| matches!(p, Request::Download))
    })
    .await;
    assert_eq!(std::fs::read(saved).unwrap(), original);
    // Send a recognizable image, then read it through the same encrypted
    // preview path used by the timeline. A second browser device checks the
    // rendered pixels independently in scripts/test-media-interop.py.
    let path = dir.path().join("inline-proof.png");
    image::RgbImage::from_fn(640, 240, |x, _| {
        image::Rgb(if x < 320 {
            [226, 36, 139]
        } else {
            [12, 217, 174]
        })
    })
    .save(&path)
    .unwrap();
    let _ = app.update(Msg::Files(
        id.into(),
        vec![media::LocalFile::from_path(path).unwrap()],
    ));
    let _ = app.update(Msg::Send);
    wait(&mut app, &rx, "confirmed encrypted image", |a| {
        a.batch.is_none()
            && a.outbox.rooms.is_empty()
            && a.live.list[id].history.values().any(|p| {
                p.media
                    .as_ref()
                    .is_some_and(|m| m.name == "inline-proof.png")
            })
    })
    .await;
    let source = app.live.list[id]
        .history
        .values()
        .filter_map(|p| p.media.as_ref())
        .find(|m| m.name == "inline-proof.png")
        .unwrap()
        .preview
        .clone()
        .unwrap();
    assert!(source.encrypted);
    let (result, received) = async_channel::bounded(1);
    assert!(
        app.commands
            .send(backend::Command::Inline { source, result })
            .await
            .is_ok()
    );
    let handle = tokio::time::timeout(Duration::from_secs(35), received.recv())
        .await
        .unwrap()
        .unwrap()
        .expect("native encrypted thumbnail");
    let iced::widget::image::Handle::Rgba {
        width,
        height,
        pixels,
        ..
    } = handle
    else {
        panic!("not a thumbnail")
    };
    assert_eq!((width, height), (400, 150));
    assert_eq!(&pixels[..3], &[226, 36, 139]);
    assert_eq!(&pixels[(width as usize - 1) * 4..][..3], &[12, 217, 174]);
    println!(
        "Native: confirmed encrypted upload, byte-identical file save and verified 400x150 two-colour thumbnail."
    );
    app.commands.close();
    engine.await.unwrap().unwrap();
}
