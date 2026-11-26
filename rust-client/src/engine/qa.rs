//! QA fixtures only (debug builds): `UNTETHERED_QA_SEED` signs the profile
//! in `UNTETHERED_PROFILE` in on this engine and verifies it, then the
//! process exits instead of opening a window. The call fixtures use it the
//! way they use gomuks to prepare a gomuks profile.
//!
//! `UNTETHERED_QA_SEED` is `{"server","username","password","key"}` (`key`:
//! the account's recovery key). Run it with `UNTETHERED_KEYRING=off`, so the
//! store passphrase stays in the session file and the client opens the
//! profile in the fixture's own isolated session.
use super::{Command, Event, run};
use std::time::Duration;

pub fn seed(seed: &str) -> i32 {
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
