//! matrix-rust-sdk engine, selected with `UNTETHERED_ENGINE=matrix-sdk`.
//!
//! It owns a `matrix_sdk::Client` on the Iced tokio runtime, accepts typed
//! [`Command`]s and emits typed [`Event`]s, mirroring the channel shape of
//! `crate::backend` without a JSON boundary. gomuks stays the default engine.
//! `docs/matrix-sdk-migration.md` tracks what moves over next.

pub mod app;
#[cfg(test)]
mod live_test;
mod rooms;
mod session;
mod timeline;

use async_channel::{Receiver, Sender};
use matrix_sdk::Client;
use matrix_sdk_ui::sync_service::{State as SyncState, SyncService};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

pub const ENV: &str = "UNTETHERED_ENGINE";
/// Events per room in the sliding sync room list.
const ROOM_LIST_TIMELINE: u32 = 20;

/// Whether this launch uses matrix-sdk instead of gomuks.
pub fn selected() -> bool {
    match std::env::var(ENV).as_deref() {
        Ok("matrix-sdk") => true,
        Ok("gomuks") | Ok("") | Err(_) => false,
        Ok(other) => {
            eprintln!(
                "Unknown {ENV}={other:?}; using gomuks. Set {ENV}=matrix-sdk to try the new engine."
            );
            false
        }
    }
}

/// Requests from the UI. `id` matches `App::pending`; every command with an
/// `id` is answered by exactly one [`Event::Done`].
pub enum Command {
    Login {
        id: i64,
        homeserver: String,
        username: String,
        password: String,
    },
    /// Unlock secret storage with a recovery key or passphrase.
    Recover { id: i64, secret: String },
    /// Open the room's timeline if needed, then load older messages.
    Paginate { id: i64, room: String },
    Send {
        id: i64,
        room: String,
        body: String,
        reply_to: Option<String>,
    },
}

#[derive(Clone)]
pub enum Event {
    Account(Account),
    Rooms(RoomList),
    Timeline(TimelineSnapshot),
    Done {
        id: i64,
        result: Result<(), Failure>,
    },
    Connection(Connection),
    /// Engine-written text for the status line. Never contains server text.
    Status(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Connection {
    Online,
    /// SyncService's offline mode is polling `/versions` until the server answers.
    Offline,
    /// A sync error stopped the service; it restarts shortly.
    Retrying,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub logged_in: bool,
    pub user_id: String,
    pub display_name: Option<String>,
    /// This device is cross-signed by its owner.
    pub verified: bool,
    pub recovery: Recovery,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Recovery {
    #[default]
    Unknown,
    Enabled,
    /// No secret storage: a recovery key cannot verify this device.
    Disabled,
    Incomplete,
}

#[derive(Clone, Default)]
pub struct RoomList {
    pub rooms: Vec<RoomSummary>,
    /// Current `m.space.child` edges (non-empty `via`) of joined spaces.
    pub children: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone, Default)]
pub struct RoomSummary {
    pub id: String,
    pub name: String,
    pub topic: String,
    pub space: bool,
    pub dm: bool,
    /// Direct-call target; only the Linux call UI reads it today.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub dm_peer: Option<String>,
    pub voice: bool,
    pub encrypted: bool,
    /// Client-side counts computed by the SDK from read receipts.
    pub unread_messages: u64,
    pub unread_mentions: u64,
    /// Server-side push rule counts.
    pub notifications: u64,
    pub highlights: u64,
    pub marked_unread: bool,
}

#[derive(Clone)]
pub struct TimelineSnapshot {
    pub room: String,
    /// Messages in timeline order, oldest first.
    pub messages: Vec<Message>,
    pub start_reached: bool,
}

#[derive(Clone)]
pub struct Message {
    /// Stable for the lifetime of the timeline item, including local echoes
    /// that later receive an event ID. Always positive.
    pub row: i64,
    pub event_id: Option<String>,
    pub sender: String,
    pub sender_name: Option<String>,
    pub time: i64,
    pub body: String,
    pub kind: Kind,
    pub encrypted: bool,
    pub delivery: Delivery,
    pub edited: bool,
    pub reply_to: Option<Reply>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text,
    /// A file, image, audio or video message; `body` is its caption or name.
    Attachment,
    Sticker,
    Redacted,
    Undecryptable,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Delivery {
    Sent,
    Sending,
    Failed,
}

#[derive(Clone)]
pub struct Reply {
    pub event_id: String,
    /// Present once the timeline has the replied-to event.
    pub quoted: Option<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Failure {
    /// The homeserver rejected the username or password.
    Forbidden,
    /// No homeserver answered at that address.
    Unreachable,
    /// The homeserver doesn't offer Simplified Sliding Sync (MSC4186).
    NoSlidingSync,
    /// The key or passphrase didn't unlock secret storage.
    WrongSecret,
    /// The account has no usable secret storage.
    NoRecovery,
    /// Not signed in, or the room isn't known yet.
    Unavailable,
    Failed,
}

pub async fn run(commands: Receiver<Command>, events: Sender<Event>) -> Result<(), String> {
    // reqwest 0.13 is built without a crypto provider (see Cargo.toml).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let profile = session::Profile::open()?;
    let (notices, internal) = async_channel::unbounded();
    let mut engine = Engine {
        profile,
        events,
        notices,
        live: None,
    };
    match session::restore(&engine.profile).await {
        Ok(Some(client)) => engine.go_live(client).await,
        Ok(None) => engine.emit(Event::Account(Account::default())).await,
        Err(why) => {
            engine.emit(Event::Account(Account::default())).await;
            engine.emit(Event::Status(why)).await;
        }
    }
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Ok(command) => engine.handle(command).await,
                Err(_) => break,
            },
            Ok(notice) = internal.recv() => engine.notice(notice).await,
        }
    }
    engine.shutdown().await;
    Ok(())
}

/// Background tasks tell the command loop about session-level changes.
enum Notice {
    TokenRejected,
}

struct Live {
    client: Client,
    sync: Arc<SyncService>,
    timelines: Arc<timeline::Timelines>,
    _tasks: Tasks,
}

struct Engine {
    profile: session::Profile,
    events: Sender<Event>,
    notices: Sender<Notice>,
    live: Option<Live>,
}

impl Engine {
    async fn emit(&self, event: Event) {
        let _ = self.events.send(event).await;
    }

    async fn go_live(&mut self, client: Client) {
        let sync = match SyncService::builder(client.clone())
            .with_offline_mode()
            // Every room syncs its latest events, so opened timelines stay
            // live without per-room subscriptions. Adding a subscription to a
            // room the list already holds gives it a second gap, and
            // back-pagination then returns older pages out of order
            // (0.19.1 and 0.20.0; see docs/matrix-sdk-migration.md).
            .with_room_list_timeline_limit(ROOM_LIST_TIMELINE)
            .build()
            .await
        {
            Ok(sync) => Arc::new(sync),
            Err(_) => {
                self.emit(Event::Account(Account::default())).await;
                self.emit(Event::Status(
                    "The matrix-sdk engine couldn’t start syncing.",
                ))
                .await;
                return;
            }
        };
        sync.start().await;
        let timelines = Arc::new(timeline::Timelines::new(
            client.clone(),
            self.events.clone(),
        ));
        let mut tasks = Tasks::default();
        tasks.spawn(account_updates(client.clone(), self.events.clone()));
        tasks.spawn(rooms::updates(client.clone(), self.events.clone()));
        tasks.spawn(sync_status(sync.clone(), self.events.clone()));
        tasks.spawn(session_changes(client.clone(), self.notices.clone()));
        self.live = Some(Live {
            client,
            sync,
            timelines,
            _tasks: tasks,
        });
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Login {
                id,
                homeserver,
                username,
                password,
            } => {
                let result = if self.live.is_some() {
                    Err(Failure::Unavailable)
                } else {
                    session::login(&self.profile, &homeserver, &username, &password).await
                };
                match result {
                    Ok(client) => {
                        // Signed in before the request completes, so the form
                        // never offers a second sign-in in between.
                        let now = account(&client, None).await;
                        self.go_live(client).await;
                        if self.live.is_some() {
                            self.emit(Event::Account(now)).await;
                        }
                        self.done(id, Ok(())).await;
                    }
                    Err(failure) => self.done(id, Err(failure)).await,
                }
            }
            Command::Recover { id, secret } => {
                let Some(live) = &self.live else {
                    return self.done(id, Err(Failure::Unavailable)).await;
                };
                let (client, events) = (live.client.clone(), self.events.clone());
                tokio::spawn(async move {
                    let result = session::recover(&client, &secret).await;
                    drop(secret);
                    if result.is_ok() {
                        // Report the verified state before completing the request.
                        let _ = events
                            .send(Event::Account(account(&client, None).await))
                            .await;
                    }
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::Paginate { id, room } => {
                let Some(live) = &self.live else {
                    return self.done(id, Err(Failure::Unavailable)).await;
                };
                let (timelines, events) = (live.timelines.clone(), self.events.clone());
                tokio::spawn(async move {
                    let result = timelines.paginate(&room).await;
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::Send {
                id,
                room,
                body,
                reply_to,
            } => {
                let Some(live) = &self.live else {
                    return self.done(id, Err(Failure::Unavailable)).await;
                };
                let (timelines, events) = (live.timelines.clone(), self.events.clone());
                tokio::spawn(async move {
                    let result = timelines.send(&room, body, reply_to).await;
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
        }
    }

    async fn done(&self, id: i64, result: Result<(), Failure>) {
        self.emit(Event::Done { id, result }).await;
    }

    async fn notice(&mut self, notice: Notice) {
        match notice {
            Notice::TokenRejected => {
                if let Some(live) = self.live.take() {
                    live.sync.stop().await;
                }
                session::forget(&self.profile);
                self.emit(Event::Account(Account::default())).await;
                self.emit(Event::Status(
                    "The homeserver signed this device out. Sign in again.",
                ))
                .await;
            }
        }
    }

    async fn shutdown(&mut self) {
        if let Some(live) = self.live.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), live.sync.stop()).await;
        }
    }
}

pub(super) async fn account(client: &Client, display_name: Option<String>) -> Account {
    let encryption = client.encryption();
    let verified = encryption
        .get_own_device()
        .await
        .ok()
        .flatten()
        .is_some_and(|device| device.is_cross_signed_by_owner());
    use matrix_sdk::encryption::recovery::RecoveryState;
    Account {
        logged_in: true,
        user_id: client
            .user_id()
            .map(ToString::to_string)
            .unwrap_or_default(),
        display_name,
        verified,
        recovery: match encryption.recovery().state() {
            RecoveryState::Unknown => Recovery::Unknown,
            RecoveryState::Enabled => Recovery::Enabled,
            RecoveryState::Disabled => Recovery::Disabled,
            RecoveryState::Incomplete => Recovery::Incomplete,
        },
    }
}

/// Report sign-in, device verification and recovery changes.
async fn account_updates(client: Client, events: Sender<Event>) {
    use futures_util::StreamExt;
    let mut verification = client.encryption().verification_state();
    let mut recovery = client.encryption().recovery().state_stream();
    let mut name = None;
    let mut last = None;
    loop {
        if name.is_none() {
            name =
                tokio::time::timeout(Duration::from_secs(10), client.account().get_display_name())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten();
        }
        let now = account(&client, name.clone()).await;
        if last.as_ref() != Some(&now) {
            if events.send(Event::Account(now.clone())).await.is_err() {
                return;
            }
            last = Some(now);
        }
        tokio::select! {
            v = verification.next() => if v.is_none() { return },
            r = recovery.next() => if r.is_none() { return },
            // Own-device signatures can land without a state transition.
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
    }
}

async fn sync_status(sync: Arc<SyncService>, events: Sender<Event>) {
    let mut state = sync.state();
    while let Some(now) = state.next().await {
        let connection = match now {
            SyncState::Offline => Connection::Offline,
            SyncState::Error(_) => Connection::Retrying,
            SyncState::Running => Connection::Online,
            SyncState::Idle | SyncState::Terminated => continue,
        };
        if events.send(Event::Connection(connection)).await.is_err() {
            return;
        }
        if matches!(now, SyncState::Error(_)) {
            tokio::time::sleep(Duration::from_secs(3)).await;
            sync.start().await;
        }
    }
}

async fn session_changes(client: Client, notices: Sender<Notice>) {
    let mut changes = client.subscribe_to_session_changes();
    while let Ok(change) = changes.recv().await {
        if let matrix_sdk::SessionChange::UnknownToken(_) = change {
            let _ = notices.send(Notice::TokenRejected).await;
            return;
        }
    }
}

/// Aborts its tasks when the session ends.
#[derive(Default)]
struct Tasks(Vec<tokio::task::JoinHandle<()>>);
impl Tasks {
    fn spawn(&mut self, task: impl Future<Output = ()> + Send + 'static) {
        self.0.push(tokio::spawn(task));
    }
}
impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}
