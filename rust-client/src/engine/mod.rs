//! matrix-rust-sdk engine, selected with `UNTETHERED_ENGINE=matrix-sdk`.
//!
//! It owns a `matrix_sdk::Client` on the Iced tokio runtime, accepts typed
//! [`Command`]s and emits typed [`Event`]s, mirroring the channel shape of
//! `crate::backend` without a JSON boundary. gomuks stays the default engine.
//! `docs/matrix-sdk-migration.md` tracks what moves over next.

mod account;
pub mod account_ui;
pub mod app;
mod browser;
mod keyring;
#[cfg(test)]
mod live_test;
mod media;
mod messaging;
mod rooms;
mod session;
mod state;
mod timeline;
mod verify;

use async_channel::{Receiver, Sender};
use matrix_sdk::Client;
use matrix_sdk_ui::sync_service::{State as SyncState, SyncService};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

pub use browser::Opener;

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
    Recover {
        id: i64,
        secret: String,
    },
    /// Open the room's timeline if needed, then load older messages.
    Paginate {
        id: i64,
        room: String,
    },
    Send {
        id: i64,
        room: String,
        body: String,
        reply_to: Option<String>,
    },
    /// Ask this account's other devices to verify this one by comparing emoji.
    VerifyDevice {
        id: i64,
    },
    /// Accept the incoming verification request.
    AcceptVerification,
    /// The emoji on both devices match (true) or don't (false).
    ConfirmEmoji {
        matches: bool,
    },
    CancelVerification,
    /// Create cross-signing keys if the account has none, then a key backup
    /// and secret storage. The new key arrives once as [`Event::RecoveryKey`].
    SetUpRecovery {
        id: i64,
        password: String,
    },
    /// Replace the recovery key from a verified device.
    ChangeRecoveryKey {
        id: i64,
    },
    /// Replace the account's cross-signing identity, key backup and recovery
    /// key, for when every verified device and the old key are lost.
    ResetEncryption {
        id: i64,
        password: String,
    },
    /// List the account's devices as [`Event::Devices`].
    Devices {
        id: i64,
    },
    DeleteDevice {
        id: i64,
        device: String,
        password: String,
    },
    /// Sign out on the homeserver and remove this profile's session and stores.
    Logout {
        id: i64,
    },
    /// Find out how a homeserver signs users in, as [`Event::LoginOptions`].
    LoginOptions {
        id: i64,
        homeserver: String,
    },
    /// Sign in through the system browser.
    BrowserLogin {
        id: i64,
        homeserver: String,
        method: BrowserMethod,
    },
    CancelBrowserLogin,
    /// Add our reaction with `key` to an event, or remove it if present.
    React {
        id: i64,
        room: String,
        event: String,
        key: String,
    },
    /// Replace the text of one of our messages.
    Edit {
        id: i64,
        room: String,
        event: String,
        body: String,
    },
    Report {
        id: i64,
        room: String,
        event: String,
        reason: String,
    },
    /// Read receipt (public or private) and fully-read marker at `event`.
    MarkRead {
        id: i64,
        room: String,
        event: String,
        public: bool,
    },
    MarkUnread {
        id: i64,
        room: String,
    },
    /// Messages around an event outside the loaded timeline, as
    /// [`Event::Context`].
    Context {
        id: i64,
        room: String,
        event: String,
    },
    /// Whether read receipts are public, synced as account data.
    ShareReads {
        id: i64,
        public: bool,
    },
    /// The homeserver's upload size limit, as [`Event::UploadLimit`].
    UploadLimit {
        id: i64,
    },
    /// Send an attachment message whose content an upload produced (with
    /// the caption fields the App added). Done once the homeserver has it.
    SendMedia {
        id: i64,
        room: String,
        content: serde_json::Value,
        reply_to: Option<String>,
    },
    /// The room's state from the homeserver, as [`Event::RoomState`].
    RoomState {
        id: i64,
        room: String,
    },
    Moderate {
        id: i64,
        room: String,
        user: String,
        action: crate::moderation::Action,
        reason: String,
    },
    /// Replace the power levels document; answered with [`Event::Created`].
    SetPowerLevels {
        id: i64,
        room: String,
        content: serde_json::Value,
    },
    /// A Matrix `createRoom` body; answered with [`Event::Created`].
    CreateRoom {
        id: i64,
        body: serde_json::Value,
    },
    /// Link `child` into `space`; answered with [`Event::Created`].
    AddToSpace {
        id: i64,
        space: String,
        child: String,
        via: Vec<String>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BrowserMethod {
    /// Homeserver single sign-on, optionally with one identity provider.
    Sso { provider: Option<String> },
    /// OAuth 2.0 with the homeserver's authentication service (MAS).
    OAuth,
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
    /// The current emoji verification, or `None` when there is none.
    Verification(Option<Verification>),
    /// A new recovery key. The engine keeps no copy; show it once.
    RecoveryKey(String),
    Devices(Vec<DeviceInfo>),
    LoginOptions(LoginOptions),
    /// Messages around `event`, oldest first.
    Context {
        room: String,
        event: String,
        messages: Vec<Message>,
    },
    /// The account's read receipt preference.
    ReadSharing(bool),
    /// How much of an upload (`backend::Command::Upload`) has been sent.
    UploadProgress {
        id: i64,
        fraction: f64,
    },
    /// An upload finished: the attachment content to send, like gomuks's
    /// upload descriptor.
    Uploaded {
        id: i64,
        result: Result<serde_json::Value, Failure>,
    },
    UploadLimit {
        id: i64,
        result: Result<u64, Failure>,
    },
    RoomState {
        id: i64,
        result: Result<crate::room_state::RoomState, Failure>,
    },
    /// A request that made a room or an event, with its ID.
    Created {
        id: i64,
        result: Result<String, Failure>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verification {
    /// The other device asked; otherwise this device did.
    pub incoming: bool,
    /// The other device's display name and ID, once known.
    pub device: Option<String>,
    pub stage: Stage,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Another device asks to verify; accept or decline.
    Requested,
    /// Waiting for another device to accept.
    Waiting,
    /// Accepted; the devices are exchanging keys.
    Starting,
    /// Compare these (symbol, name) pairs with the other device.
    Emoji(Vec<(String, String)>),
    /// Confirmed here; waiting for the other device.
    Confirmed,
    Done,
    /// Engine-written reason.
    Cancelled(&'static str),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceInfo {
    pub id: String,
    pub name: Option<String>,
    /// Milliseconds since the epoch, as reported by the homeserver.
    pub last_seen: Option<i64>,
    /// Cross-signed by this account.
    pub verified: bool,
    /// This device.
    pub current: bool,
}

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct LoginOptions {
    pub homeserver: String,
    pub password: bool,
    /// Identity providers as (id, name); empty with `sso` means "any".
    pub providers: Vec<(String, String)>,
    pub sso: bool,
    pub oauth: bool,
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
    /// The account has published cross-signing keys, so another device can
    /// verify this one.
    pub identity: bool,
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
    /// The `m.fully_read` marker.
    pub fully_read: Option<String>,
    /// The documents behind permissions, from the store.
    pub create: Option<crate::permissions::StateDoc>,
    pub power: Option<crate::permissions::StateDoc>,
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
    /// (key, count, includes ours), in first-seen order.
    pub reactions: Vec<(String, u64, bool)>,
    /// An `m.text` message (only those can be edited here).
    pub text: bool,
    /// The SDK allows us to edit it.
    pub editable: bool,
    /// `m.mentions`: user IDs and `@room`.
    pub mentions: Option<(Vec<String>, bool)>,
    /// The newest edit's event ID, which reports refer to.
    pub edit_id: Option<String>,
    /// Read receipts on this event: (user, timestamp).
    pub receipts: Vec<(String, i64)>,
    /// An attachment's content (`msgtype`, `url` or `file`, `info`), for
    /// `media::Media::parse`.
    pub media: Option<serde_json::Value>,
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
    /// The homeserver rejected the account password.
    WrongPassword,
    /// The account has no cross-signing identity to verify against.
    NoIdentity,
    /// The homeserver wants approval in a browser (OAuth 2.0 accounts).
    NeedsBrowser,
    Cancelled,
    Failed,
}

/// `backend` carries the App's gomuks-shaped transfer commands (uploads,
/// inline previews, downloads), which this engine answers too.
pub async fn run(
    commands: Receiver<Command>,
    backend: Receiver<crate::backend::Command>,
    events: Sender<Event>,
) -> Result<(), String> {
    run_in(
        session::profile_root()?,
        browser::system(),
        commands,
        backend,
        events,
    )
    .await
}

/// Run on an explicit profile directory; tests run several engines at once
/// and replace the system browser.
pub async fn run_in(
    root: PathBuf,
    opener: Opener,
    commands: Receiver<Command>,
    backend: Receiver<crate::backend::Command>,
    events: Sender<Event>,
) -> Result<(), String> {
    // reqwest 0.13 is built without a crypto provider (see Cargo.toml).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let profile = Arc::new(session::Profile::open(root)?);
    let (notices, internal) = async_channel::unbounded();
    let mut engine = Engine {
        profile,
        events,
        notices,
        opener,
        browser: None,
        browser_id: None,
        live: None,
        sources: Default::default(),
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
            Ok(transfer) = backend.recv() => engine.transfer(transfer).await,
        }
    }
    engine.shutdown().await;
    Ok(())
}

/// Background tasks tell the command loop about session-level changes.
enum Notice {
    TokenRejected,
    /// A browser sign-in finished; on success the session is saved.
    SignedIn {
        id: i64,
        result: Result<Client, Failure>,
    },
}

struct Live {
    client: Client,
    sync: Arc<SyncService>,
    timelines: Arc<timeline::Timelines>,
    verifier: Arc<verify::Verifier>,
    tasks: Tasks,
}

struct Engine {
    profile: Arc<session::Profile>,
    events: Sender<Event>,
    notices: Sender<Notice>,
    opener: Opener,
    browser: Option<tokio::task::JoinHandle<()>>,
    browser_id: Option<i64>,
    live: Option<Live>,
    /// Attachments seen in timelines, for previews and downloads.
    sources: Arc<media::Sources>,
}

enum Key {
    SetUp(String),
    Change,
    Reset(String),
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
            self.sources.clone(),
        ));
        let verifier = verify::Verifier::new(client.clone(), self.events.clone());
        let mut tasks = Tasks::default();
        tasks.spawn(account_updates(client.clone(), self.events.clone()));
        tasks.spawn(rooms::updates(client.clone(), self.events.clone()));
        tasks.spawn(messaging::read_sharing(client.clone(), self.events.clone()));
        tasks.spawn(timeline::resume_after_errors(client.clone()));
        tasks.spawn(sync_status(sync.clone(), self.events.clone()));
        tasks.spawn(session_changes(
            client.clone(),
            self.profile.clone(),
            self.notices.clone(),
        ));
        self.live = Some(Live {
            client,
            sync,
            timelines,
            verifier,
            tasks,
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
                let browsing = self.browser.as_ref().is_some_and(|t| !t.is_finished());
                let result = if self.live.is_some() || browsing {
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
            Command::VerifyDevice { id } => {
                let Some(live) = &self.live else {
                    return self.done(id, Err(Failure::Unavailable)).await;
                };
                let (verifier, events) = (live.verifier.clone(), self.events.clone());
                tokio::spawn(async move {
                    let result = verifier.request().await;
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::AcceptVerification => {
                if let Some(live) = &self.live {
                    let verifier = live.verifier.clone();
                    tokio::spawn(async move { verifier.accept().await });
                }
            }
            Command::ConfirmEmoji { matches } => {
                if let Some(live) = &self.live {
                    let verifier = live.verifier.clone();
                    tokio::spawn(async move { verifier.confirm(matches).await });
                }
            }
            Command::CancelVerification => {
                if let Some(live) = &self.live {
                    let verifier = live.verifier.clone();
                    tokio::spawn(async move { verifier.cancel().await });
                }
            }
            Command::SetUpRecovery { id, password } => self.new_key(id, Key::SetUp(password)).await,
            Command::ChangeRecoveryKey { id } => self.new_key(id, Key::Change).await,
            Command::ResetEncryption { id, password } => {
                self.new_key(id, Key::Reset(password)).await
            }
            Command::Devices { id } => {
                let Some(live) = &self.live else {
                    return self.done(id, Err(Failure::Unavailable)).await;
                };
                let (client, events) = (live.client.clone(), self.events.clone());
                tokio::spawn(async move {
                    let result = match account::devices(&client).await {
                        Ok(list) => {
                            let _ = events.send(Event::Devices(list)).await;
                            Ok(())
                        }
                        Err(f) => Err(f),
                    };
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::DeleteDevice {
                id,
                device,
                password,
            } => {
                let Some(live) = &self.live else {
                    return self.done(id, Err(Failure::Unavailable)).await;
                };
                let (client, events) = (live.client.clone(), self.events.clone());
                tokio::spawn(async move {
                    let result = account::delete_device(&client, &device, &password).await;
                    drop(password);
                    if let Ok(list) = account::devices(&client).await {
                        let _ = events.send(Event::Devices(list)).await;
                    }
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::Logout { id } => self.logout(id).await,
            Command::LoginOptions { id, homeserver } => {
                let events = self.events.clone();
                tokio::spawn(async move {
                    let result = match browser::options(&homeserver).await {
                        Ok(options) => {
                            let _ = events.send(Event::LoginOptions(options)).await;
                            Ok(())
                        }
                        Err(f) => Err(f),
                    };
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::BrowserLogin {
                id,
                homeserver,
                method,
            } => {
                if self.live.is_some() || self.browser.as_ref().is_some_and(|t| !t.is_finished()) {
                    return self.done(id, Err(Failure::Unavailable)).await;
                }
                let (profile, opener, notices) = (
                    self.profile.clone(),
                    self.opener.clone(),
                    self.notices.clone(),
                );
                self.browser_id = Some(id);
                self.browser = Some(tokio::spawn(async move {
                    let result = browser::login(&profile, &homeserver, method, opener).await;
                    let _ = notices.send(Notice::SignedIn { id, result }).await;
                }));
            }
            Command::CancelBrowserLogin => {
                if let (Some(task), Some(id)) = (self.browser.take(), self.browser_id.take()) {
                    task.abort();
                    let _ = task.await;
                    session::forget(&self.profile).await;
                    self.done(id, Err(Failure::Cancelled)).await;
                }
            }
            Command::React {
                id,
                room,
                event,
                key,
            } => {
                self.job(id, move |_, timelines, _| async move {
                    timelines.react(&room, &event, &key).await
                })
                .await
            }
            Command::Edit {
                id,
                room,
                event,
                body,
            } => {
                self.job(id, move |_, timelines, _| async move {
                    timelines.edit(&room, &event, body).await
                })
                .await
            }
            Command::Report {
                id,
                room,
                event,
                reason,
            } => {
                self.job(id, move |client, _, _| async move {
                    messaging::report(&client, &room, &event, &reason).await
                })
                .await
            }
            Command::MarkRead {
                id,
                room,
                event,
                public,
            } => {
                self.job(id, move |client, _, _| async move {
                    messaging::mark_read(&client, &room, &event, public).await
                })
                .await
            }
            Command::MarkUnread { id, room } => {
                self.job(id, move |client, _, _| async move {
                    messaging::mark_unread(&client, &room).await
                })
                .await
            }
            Command::Context { id, room, event } => {
                let sources = self.sources.clone();
                self.job(id, move |client, _, events| async move {
                    let messages = messaging::context(&client, &room, &event, &sources).await?;
                    let _ = events
                        .send(Event::Context {
                            room,
                            event,
                            messages,
                        })
                        .await;
                    Ok(())
                })
                .await
            }
            Command::ShareReads { id, public } => {
                self.job(id, move |client, _, _| async move {
                    messaging::share_reads(&client, public).await
                })
                .await
            }
            Command::UploadLimit { id } => {
                let unavailable = Event::UploadLimit {
                    id,
                    result: Err(Failure::Unavailable),
                };
                self.answer(unavailable, move |client| async move {
                    let result = media::upload_limit(&client).await;
                    Event::UploadLimit { id, result }
                })
                .await
            }
            Command::RoomState { id, room } => {
                let unavailable = Event::RoomState {
                    id,
                    result: Err(Failure::Unavailable),
                };
                self.answer(unavailable, move |client| async move {
                    let result = state::room_state(&client, &room).await;
                    Event::RoomState { id, result }
                })
                .await
            }
            Command::Moderate {
                id,
                room,
                user,
                action,
                reason,
            } => {
                self.job(id, move |client, _, _| async move {
                    state::moderate(&client, &room, &user, action, &reason).await
                })
                .await
            }
            Command::SetPowerLevels { id, room, content } => {
                self.created(id, move |client| async move {
                    state::set_power_levels(&client, &room, content).await
                })
                .await
            }
            Command::CreateRoom { id, body } => {
                self.created(id, move |client| async move {
                    state::create_room(&client, body).await
                })
                .await
            }
            Command::AddToSpace {
                id,
                space,
                child,
                via,
            } => {
                self.created(id, move |client| async move {
                    state::add_to_space(&client, &space, &child, via).await
                })
                .await
            }
            Command::SendMedia {
                id,
                room,
                content,
                reply_to,
            } => {
                self.job(id, move |_, timelines, _| async move {
                    timelines.send_media(&room, content, reply_to).await
                })
                .await
            }
        }
    }

    /// The App's transfer commands, which go to the gomuks backend on that
    /// engine. Calls (`Voice`) aren't on this engine yet.
    async fn transfer(&self, command: crate::backend::Command) {
        use crate::backend::Command as Transfer;
        let live = self.live.as_ref().map(|l| l.client.clone());
        let (sources, events) = (self.sources.clone(), self.events.clone());
        match command {
            Transfer::Inline { source, result } => {
                tokio::spawn(async move {
                    let image = match live {
                        Some(client) => media::inline(client, sources, source).await,
                        None => None,
                    };
                    let _ = result.send(image).await;
                });
            }
            Transfer::Upload {
                id,
                cancel,
                file,
                encrypt,
                limit,
            } => {
                tokio::spawn(async move {
                    let result = match live {
                        Some(client) => {
                            media::upload(client, id, file, encrypt, limit, cancel, events.clone())
                                .await
                        }
                        None => Err(Failure::Unavailable),
                    };
                    let _ = events.send(Event::Uploaded { id, result }).await;
                });
            }
            Transfer::Download {
                id,
                media,
                destination,
            } => {
                tokio::spawn(async move {
                    let result = match live {
                        Some(client) => media::download(client, sources, media, destination).await,
                        None => Err(Failure::Unavailable),
                    };
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Transfer::Rpc(_) => {}
            #[cfg(target_os = "linux")]
            Transfer::Voice { .. } => {}
        }
    }

    /// Run a request against the signed-in session off the command loop and
    /// answer it with `Event::Done`.
    async fn job<F, Fut>(&self, id: i64, work: F)
    where
        F: FnOnce(Client, Arc<timeline::Timelines>, Sender<Event>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), Failure>> + Send + 'static,
    {
        let Some(live) = &self.live else {
            return self.done(id, Err(Failure::Unavailable)).await;
        };
        let (client, timelines, events) = (
            live.client.clone(),
            live.timelines.clone(),
            self.events.clone(),
        );
        tokio::spawn(async move {
            let result = work(client, timelines, events.clone()).await;
            let _ = events.send(Event::Done { id, result }).await;
        });
    }

    /// Run `work` against the signed-in session off the command loop; it
    /// answers with its own event. Without a session, `unavailable` does.
    async fn answer<F, Fut>(&self, unavailable: Event, work: F)
    where
        F: FnOnce(Client) -> Fut + Send + 'static,
        Fut: Future<Output = Event> + Send + 'static,
    {
        let Some(live) = &self.live else {
            return self.emit(unavailable).await;
        };
        let (client, events) = (live.client.clone(), self.events.clone());
        tokio::spawn(async move {
            let _ = events.send(work(client).await).await;
        });
    }

    /// Requests that make a room or an event, answered with its ID.
    async fn created<F, Fut>(&self, id: i64, work: F)
    where
        F: FnOnce(Client) -> Fut + Send + 'static,
        Fut: Future<Output = Result<String, Failure>> + Send + 'static,
    {
        let unavailable = Event::Created {
            id,
            result: Err(Failure::Unavailable),
        };
        self.answer(unavailable, move |client| async move {
            Event::Created {
                id,
                result: work(client).await,
            }
        })
        .await
    }

    /// Commands that end with a new recovery key.
    async fn new_key(&self, id: i64, kind: Key) {
        let Some(live) = &self.live else {
            return self.done(id, Err(Failure::Unavailable)).await;
        };
        let (client, events) = (live.client.clone(), self.events.clone());
        tokio::spawn(async move {
            let result = match kind {
                Key::SetUp(password) => account::set_up_recovery(&client, &password).await,
                Key::Change => account::change_recovery_key(&client).await,
                Key::Reset(password) => account::reset_encryption(&client, &password).await,
            };
            let result = match result {
                Ok(key) => {
                    let _ = events.send(Event::RecoveryKey(key)).await;
                    let _ = events
                        .send(Event::Account(account(&client, None).await))
                        .await;
                    Ok(())
                }
                Err(f) => Err(f),
            };
            let _ = events.send(Event::Done { id, result }).await;
        });
    }

    async fn logout(&mut self, id: i64) {
        let Some(live) = self.live.take() else {
            return self.done(id, Err(Failure::Unavailable)).await;
        };
        let Live {
            client,
            sync,
            tasks,
            ..
        } = live;
        // Our own sign-out must not also report a rejected token.
        tasks.stop().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), sync.stop()).await;
        let server = tokio::time::timeout(Duration::from_secs(15), client.logout()).await;
        drop((sync, client));
        session::forget(&self.profile).await;
        self.emit(Event::Account(Account::default())).await;
        if !matches!(server, Ok(Ok(()))) {
            self.emit(Event::Status(
                "Signed out on this device. The homeserver couldn’t be reached, so it may still list this device.",
            ))
            .await;
        }
        self.done(id, Ok(())).await;
    }

    async fn done(&self, id: i64, result: Result<(), Failure>) {
        self.emit(Event::Done { id, result }).await;
    }

    async fn notice(&mut self, notice: Notice) {
        match notice {
            Notice::SignedIn { id, result } => {
                self.browser = None;
                self.browser_id = None;
                match result {
                    Ok(client) => {
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
            Notice::TokenRejected => {
                if let Some(live) = self.live.take() {
                    live.tasks.stop().await;
                    live.sync.stop().await;
                }
                session::forget(&self.profile).await;
                self.emit(Event::Account(Account::default())).await;
                self.emit(Event::Status(
                    "The homeserver signed this device out. Sign in again.",
                ))
                .await;
            }
        }
    }

    async fn shutdown(&mut self) {
        if let Some(task) = self.browser.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(live) = self.live.take() {
            live.tasks.stop().await;
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
    let identity = match client.user_id() {
        Some(user) => encryption
            .get_user_identity(user)
            .await
            .ok()
            .flatten()
            .is_some(),
        None => false,
    };
    Account {
        logged_in: true,
        user_id: client
            .user_id()
            .map(ToString::to_string)
            .unwrap_or_default(),
        display_name,
        verified,
        identity,
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
    // Ask the homeserver once, so a fresh device knows whether the account
    // already has cross-signing keys before offering to create them.
    if let Some(user) = client.user_id() {
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            client.encryption().request_user_identity(user),
        )
        .await;
    }
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

async fn session_changes(client: Client, profile: Arc<session::Profile>, notices: Sender<Notice>) {
    let mut changes = client.subscribe_to_session_changes();
    while let Ok(change) = changes.recv().await {
        match change {
            matrix_sdk::SessionChange::UnknownToken(_) => {
                let _ = notices.send(Notice::TokenRejected).await;
                return;
            }
            matrix_sdk::SessionChange::TokensRefreshed => session::save_tokens(&profile, &client),
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

    /// Abort and wait. Aborting alone lets a running task finish its poll
    /// later, still holding what it captured (the profile, and with it the
    /// profile lock a restarted engine needs).
    async fn stop(mut self) {
        let tasks = std::mem::take(&mut self.0);
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
    }
}
impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}
