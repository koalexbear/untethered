//! matrix-rust-sdk engine, the default since step 8. `UNTETHERED_ENGINE=gomuks`
//! (or `?engine=gomuks` on the web) still starts gomuks for one release.
//!
//! It owns a `matrix_sdk::Client` on the Iced tokio runtime, accepts typed
//! [`Command`]s and emits typed [`Event`]s, mirroring the channel shape of
//! `crate::backend` without a JSON boundary.
//! `docs/matrix-sdk-migration.md` tracks what moves over next.
// In the browser the SDK's types are neither Send nor Sync, and the engine's
// shared state (Arc) is only shared between tasks on one thread.
#![cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]

mod account;
pub mod account_ui;
mod alerts;
pub mod app;
pub mod browser;
#[cfg(target_os = "linux")]
mod calls;
mod compose;
mod dms;
pub mod explore;
mod invites;
mod joining;
#[cfg(not(target_arch = "wasm32"))]
mod keyring;
#[cfg(test)]
mod live_test;
mod media;
mod messaging;
pub mod pins;
pub mod polls;
mod presence;
mod previews;
pub mod profile;
#[cfg(debug_assertions)]
pub mod qa;
mod rooms;
pub mod rt;
mod search;
pub use search::{Has, Reach, RoomsFound, SearchQuery};
mod session;
pub mod threads;
// The desktop's own settings live beside the profile (tray.rs).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use session::profile_root;
mod state;
mod timeline;
mod typing;
mod upgrades;
mod verify;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
mod worker;
#[cfg(target_arch = "wasm32")]
mod xhr;

use async_channel::{Receiver, Sender};
use matrix_sdk::Client;
use matrix_sdk_ui::sync_service::{State as SyncState, SyncService};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

pub use browser::Opener;
pub use upgrades::Seam;

/// Events per room in the sliding sync room list.
const ROOM_LIST_TIMELINE: u32 = 20;

/// Shown on the sign-in screen when gomuks data is here but no matrix-sdk
/// session (docs/matrix-sdk-migration.md, "Existing gomuks profiles").
const UPGRADE: &str = "Untethered now uses a new Matrix engine. Sign in once more: this is a new device, and your recovery key restores your encrypted history. Afterwards you can remove the old session under Devices.";

/// Requests from the UI. `id` matches `App::pending`; every command with an
/// `id` is answered by exactly one [`Event::Done`].
#[derive(serde::Serialize, serde::Deserialize)]
pub enum Command {
    Discussion {
        id: i64,
        room: String,
        action: threads::Request,
    },
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
    SaveProfile {
        id: i64,
        edit: profile::Edit,
    },
    DirectProfile {
        id: i64,
        user: String,
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
        /// Mentions and room links picked in the composer (pills.rs).
        pills: Vec<crate::pills::Pill>,
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
        pills: Vec<crate::pills::Pill>,
    },
    Redact {
        id: i64,
        room: String,
        event: String,
    },
    /// Send a message `room` shows again in the room `to` (ROADMAP M8),
    /// with `note` after it.
    Forward {
        id: i64,
        room: String,
        event: String,
        to: String,
        note: Option<String>,
    },
    /// Start a poll in `room` (ROADMAP M9).
    CreatePoll {
        id: i64,
        room: String,
        draft: polls::Draft,
    },
    /// Vote in the poll at `event`: `answers` replace an earlier vote, and
    /// none takes it back.
    Vote {
        id: i64,
        room: String,
        event: String,
        answers: Vec<String>,
    },
    EndPoll {
        id: i64,
        room: String,
        event: String,
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
    /// Read receipt and fully-read marker at the newest event the event
    /// cache holds, for a room whose timeline the App hasn't opened.
    MarkRoomRead {
        id: i64,
        room: String,
        public: bool,
    },
    /// Leave a room; it drops out of the next room list.
    LeaveRoom {
        id: i64,
        room: String,
    },
    /// Join the room that replaced `room` in an upgrade (its tombstone's
    /// successor); the next room list then leaves `room` out.
    FollowUpgrade {
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
    /// Tell the room we're typing, or that we stopped. No answer.
    Typing {
        room: String,
        typing: bool,
    },
    /// People in the user directory, answered with `Event::People` (dms.rs).
    SearchPeople {
        term: String,
    },
    /// A DM with one person, or a group DM with several, answered with
    /// `Event::Created` and the room's ID.
    StartDm {
        id: i64,
        users: Vec<String>,
    },
    /// Block or unblock someone (`m.ignored_user_list`).
    Block {
        id: i64,
        user: String,
        block: bool,
    },
    /// What the homeserver says about a room by ID or address, answered
    /// with `Event::RoomPreview` (joining.rs).
    PreviewRoom {
        target: String,
        via: Vec<String>,
    },
    /// Join a room by ID or address.
    JoinRoom {
        id: i64,
        target: String,
        via: Vec<String>,
    },
    /// A page of the homeserver's public room directory, answered with
    /// `Event::PublicRooms` (explore.rs).
    PublicRooms {
        id: i64,
        query: explore::DirectoryQuery,
    },
    /// The rooms in a space, answered with `Event::SpaceRooms`.
    SpaceRooms {
        space: String,
    },
    /// Join a room this account is invited to (invites.rs). Declining is
    /// `LeaveRoom`.
    AcceptInvite {
        id: i64,
        room: String,
    },
    /// The App no longer shows this room: drop its timeline (timeline.rs).
    CloseTimeline {
        room: String,
    },
    /// A link's preview, answered with `Event::LinkPreview` (previews.rs).
    LinkPreview {
        url: String,
    },
    /// Turn a room's link previews on or off.
    SetLinkPreviews {
        id: i64,
        room: String,
        on: bool,
    },
    /// Whether to send typing notices, synced as account data.
    ShareTyping {
        id: i64,
        send: bool,
    },
    /// Publish our status (N4) and keep it published; no answer.
    PublishPresence {
        presence: crate::presence::Presence,
    },
    /// Read these people's presence (N4) from now on, most visible first;
    /// changes come as [`Event::Presence`]. No answer.
    WatchPresence {
        users: Vec<String>,
    },
    /// Save our status choice and custom status (N4) as account data.
    SaveStatus {
        id: i64,
        choice: crate::presence::Choice,
        message: String,
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
    /// Change one of the account's push rules.
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    PushRule {
        id: i64,
        kind: crate::notifications::RuleKind,
        rule_id: String,
        change: crate::notifications::RuleChange,
    },
    /// Replace a room's account data of `kind` (Matrix event type).
    /// Only the alert settings send it today.
    #[cfg_attr(
        not(any(target_os = "linux", target_arch = "wasm32")),
        allow(dead_code)
    )]
    RoomAccountData {
        id: i64,
        room: String,
        kind: String,
        content: serde_json::Value,
    },
    /// Search rooms (search.rs), answered with [`Event::RoomsFound`].
    SearchRooms {
        id: i64,
        query: SearchQuery,
    },
    /// Load older history of the encrypted ones among `rooms` onto this
    /// device, so searches here reach further back.
    IndexRooms {
        id: i64,
        rooms: Vec<String>,
    },
    /// Stored messages mentioning us in these rooms, as [`Event::Mentions`].
    Mentions {
        id: i64,
        rooms: Vec<String>,
    },
    /// The homeserver's MatrixRTC transports, as [`Event::RtcTransports`].
    RtcTransports {
        id: i64,
    },
    /// A room's state for checking an incoming call, as [`Event::CallState`].
    #[cfg(target_os = "linux")]
    CallState {
        id: i64,
        room: String,
    },
    #[cfg(target_os = "linux")]
    DeclineRing {
        id: i64,
        room: String,
        ring: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum BrowserMethod {
    /// Homeserver single sign-on, optionally with one identity provider.
    Sso { provider: Option<String> },
    /// OAuth 2.0 with the homeserver's authentication service (MAS).
    OAuth,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Event {
    Discussion {
        id: i64,
        result: Result<threads::Response, Failure>,
    },
    Account(Account),
    ProfileSaved {
        id: i64,
        result: Result<profile::Saved, Failure>,
    },
    ProfileDirect {
        id: i64,
        result: Result<String, Failure>,
    },
    Rooms(RoomList),
    Timeline(TimelineSnapshot),
    Done {
        id: i64,
        result: Result<(), Failure>,
    },
    Connection(Connection),
    /// Engine-written text for the status line. Never contains server text.
    Status(String),
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
    /// Who is typing in a room now, never us.
    Typing {
        room: String,
        users: Vec<String>,
    },
    /// The account's "Send typing indicators" preference.
    TypingSharing(bool),
    /// People's presence (N4), when it changed.
    Presence(Vec<(String, crate::presence::Presence)>),
    /// Whether the homeserver shares presence (N4): Unknown when the
    /// engine starts asking.
    PresenceSupport(crate::presence::Support),
    /// Our saved status choice and custom status (N4), at startup and
    /// whenever it syncs.
    OwnStatus {
        choice: crate::presence::Choice,
        message: String,
    },
    /// A room's preview for a link or address.
    RoomPreview {
        target: String,
        result: Result<RoomPreview, Failure>,
    },
    /// The user directory's answer to a search for `term`.
    People {
        term: String,
        result: Result<Vec<Person>, Failure>,
    },
    /// A page of the public room directory.
    PublicRooms {
        id: i64,
        result: Result<explore::Directory, Failure>,
    },
    /// What a space contains, at any depth.
    SpaceRooms {
        space: String,
        result: Result<Vec<explore::SpaceChild>, Failure>,
    },
    /// Who this account blocks, at startup and whenever it changes.
    Blocked(Vec<String>),
    /// A link's preview, or None when the homeserver has none.
    LinkPreview {
        url: String,
        preview: Option<LinkPreview>,
    },
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
    /// A membership change synced.
    Member {
        room: String,
        user: String,
        member: crate::moderation::Member,
    },
    /// The account's push rules (`m.push_rules` content), at startup and
    /// whenever they change. The App's alerts run on Linux and the web.
    #[cfg_attr(
        not(any(target_os = "linux", target_arch = "wasm32")),
        allow(dead_code)
    )]
    PushRules(serde_json::Value),
    /// A message the push rules say to notify about.
    #[cfg_attr(
        not(any(target_os = "linux", target_arch = "wasm32")),
        allow(dead_code)
    )]
    Notification(Alert),
    RoomsFound {
        id: i64,
        result: Result<RoomsFound, Failure>,
    },
    Mentions {
        id: i64,
        result: Result<Vec<Found>, Failure>,
    },
    /// MSC4143 transports as the homeserver describes them.
    RtcTransports {
        id: i64,
        result: Result<Vec<serde_json::Value>, Failure>,
    },
    /// A status frame from a running call (`desktop_call`'s own protocol,
    /// the same on both engines).
    #[cfg(target_os = "linux")]
    CallFrame(serde_json::Value),
    /// A call-related room event, as a (decrypted) Matrix event, for
    /// incoming calls.
    #[cfg(target_os = "linux")]
    CallActivity {
        room: String,
        event: serde_json::Value,
    },
    /// A room's state as Matrix state events (`Command::CallState`).
    #[cfg(target_os = "linux")]
    CallState {
        id: i64,
        result: Result<Vec<serde_json::Value>, Failure>,
    },
}

/// A stored message found by search or for the mentions inbox, with its
/// current (edited) text.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Found {
    pub room: String,
    pub event_id: String,
    pub sender: String,
    pub body: String,
    pub time: i64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Alert {
    pub room: String,
    pub event_id: String,
    pub sender: String,
    /// The message text, or an attachment's name or caption.
    pub body: String,
    pub sound: bool,
    /// The push rules highlight it: a mention (sounds.rs).
    #[serde(default)]
    pub highlight: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Verification {
    /// The other device asked; otherwise this device did.
    pub incoming: bool,
    /// The other device's display name and ID, once known.
    pub device: Option<String>,
    pub stage: Stage,
}

#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
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
    Cancelled(String),
}

#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
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

#[derive(Clone, Default, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct LoginOptions {
    pub homeserver: String,
    pub password: bool,
    /// Identity providers as (id, name); empty with `sso` means "any".
    pub providers: Vec<(String, String)>,
    pub sso: bool,
    pub oauth: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Connection {
    Online,
    /// SyncService's offline mode is polling `/versions` until the server answers.
    Offline,
    /// A sync error stopped the service; it restarts shortly.
    Retrying,
}

#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Account {
    pub logged_in: bool,
    pub user_id: String,
    pub display_name: Option<String>,
    /// The account's profile picture (mxc URI).
    #[serde(default)]
    pub avatar: Option<String>,
    /// This device is cross-signed by its owner.
    pub verified: bool,
    /// The account has published cross-signing keys, so another device can
    /// verify this one.
    pub identity: bool,
    pub recovery: Recovery,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Recovery {
    #[default]
    Unknown,
    Enabled,
    /// No secret storage: a recovery key cannot verify this device.
    Disabled,
    Incomplete,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RoomList {
    pub rooms: Vec<RoomSummary>,
    /// Current `m.space.child` edges (non-empty `via`) of joined spaces.
    pub children: BTreeMap<String, BTreeSet<String>>,
    /// Rooms, spaces and DMs this account is invited to (invites.rs).
    #[serde(default)]
    pub invites: Vec<Invite>,
}

/// Someone found in the user directory (dms.rs).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Person {
    pub id: String,
    pub name: Option<String>,
    /// Their profile picture (mxc URI).
    pub avatar: Option<String>,
}

/// A room reached by a link or address, before joining (joining.rs).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoomPreview {
    pub room_id: String,
    pub alias: Option<String>,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub avatar: Option<String>,
    pub members: u64,
    pub space: bool,
    /// This account is in it already.
    pub joined: bool,
    pub join: JoinRule,
}

/// Who may join a previewed room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JoinRule {
    /// Anyone.
    Open,
    /// Members of certain spaces or rooms (restricted).
    Members,
    /// By asking first (knocking).
    Knock,
    /// By invitation only.
    Invite,
}

/// An invite, as its stripped state and the invite event tell it.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Invite {
    pub id: String,
    /// The room's name; a DM's is the person who invited.
    pub name: String,
    pub topic: String,
    pub avatar: Option<String>,
    pub space: bool,
    pub dm: bool,
    pub inviter: Option<String>,
    pub inviter_name: Option<String>,
    pub inviter_avatar: Option<String>,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RoomSummary {
    pub id: String,
    pub name: String,
    /// The room's picture (mxc URI); a DM without one uses the peer's.
    #[serde(default)]
    pub avatar: Option<String>,
    pub topic: String,
    pub space: bool,
    pub dm: bool,
    /// The other user in a direct-message room.
    pub dm_peer: Option<String>,
    pub voice: bool,
    /// A voice room's call member state events from the store, in
    /// client-server format; empty for other rooms. Only the Linux call UI
    /// reads them today.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub call_members: Vec<serde_json::Value>,
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
    /// The space's alert preference (room account data content).
    #[cfg_attr(
        not(any(target_os = "linux", target_arch = "wasm32")),
        allow(dead_code)
    )]
    pub space_alerts: Option<serde_json::Value>,
    /// The room that replaced this one in an upgrade (`m.room.tombstone`).
    #[serde(default)]
    pub successor: Option<String>,
    /// The room this one replaced (`m.room.create`'s `predecessor`).
    #[serde(default)]
    pub predecessor: Option<String>,
    /// This account shows link previews in the room (previews.rs).
    #[serde(default)]
    pub link_previews: bool,
}

/// A link's preview as the homeserver gives it (previews.rs).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site: Option<String>,
    /// The homeserver's copy of the page's image (mxc URI).
    pub image: Option<String>,
    pub image_size: Option<(u32, u32)>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct TimelineSnapshot {
    pub room: String,
    /// Messages in timeline order, oldest first.
    pub messages: Vec<Message>,
    pub start_reached: bool,
    /// The latest remote event, also when it has no message of its own (an
    /// edit, a reaction): what reading the room marks read.
    #[serde(default)]
    pub latest_event: Option<String>,
    /// Where the history of an older room this one was upgraded from
    /// continues above, oldest first (upgrades.rs).
    #[serde(default)]
    pub seams: Vec<Seam>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    /// Stable for the lifetime of the timeline item, including local echoes
    /// that later receive an event ID. Always positive.
    pub row: i64,
    pub event_id: Option<String>,
    pub sender: String,
    pub sender_name: Option<String>,
    /// The sender's profile picture (mxc URI) as the timeline knows it.
    #[serde(default)]
    pub sender_avatar: Option<String>,
    pub time: i64,
    pub body: String,
    /// The `org.matrix.custom.html` body of a text, notice or emote, as
    /// matrix-sdk-ui sanitised it (reply fallback removed).
    #[serde(default)]
    pub formatted: Option<String>,
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
    /// When the newest edit was sent (ms since the epoch), to place it in the
    /// room's read order: it has no message of its own.
    #[serde(default)]
    pub edit_time: Option<i64>,
    /// Read receipts on this event: (user, timestamp).
    pub receipts: Vec<(String, i64)>,
    /// An attachment's content (`msgtype`, `url` or `file`, `info`), for
    /// `media::Media::parse`.
    pub media: Option<serde_json::Value>,
    /// The older room this message belongs to, when the snapshot continues
    /// into a room this one was upgraded from. Such messages are read-only.
    #[serde(default)]
    pub from: Option<String>,
    /// A poll's question, answers and votes (ROADMAP M9).
    #[serde(default)]
    pub poll: Option<polls::Poll>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Text,
    /// A file, image, audio or video message; `body` is its caption or name.
    Attachment,
    Sticker,
    /// A poll (`poll` says what it asks); `body` is its question.
    Poll,
    Redacted,
    Undecryptable(Unreadable),
}

/// Why a message can't be read: its key may still come, or the SDK's best
/// guess (`UtdCause`) says it won't.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Unreadable {
    /// From the sender's devices, or from key backup.
    #[default]
    Waiting,
    /// Sent before we joined; its key was never shared with us.
    BeforeJoined,
    /// Sent before this device existed, with no key backup to restore it.
    NoBackup,
    /// Sent before this device existed; key backup needs it verified.
    Unverified,
    /// The sender's device didn't share the key with this device.
    Withheld,
    /// Sent from a device or identity this account doesn't trust.
    Untrusted,
    Malformed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Delivery {
    Sent,
    Sending,
    Failed,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub event_id: String,
    /// Present once the timeline has the replied-to event.
    pub quoted: Option<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
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

/// Attachment work besides commands. On the desktop these are the App's
/// gomuks-shaped transfer commands, which the engine answers too.
#[cfg(not(target_arch = "wasm32"))]
pub type Transfer = crate::backend::Command;

/// Attachment work the page hands the browser worker (`worker.rs`). Images
/// decode and downloads save on the page, so these answer with bytes.
#[cfg(target_arch = "wasm32")]
pub enum Transfer {
    Inline {
        source: crate::media::PreviewRef,
        result: Sender<Option<Vec<u8>>>,
    },
    Upload {
        id: i64,
        cancel: Receiver<()>,
        file: crate::media::LocalFile,
        encrypt: bool,
        limit: u64,
    },
    Download {
        media: crate::media::Media,
        result: Sender<Result<Vec<u8>, Failure>>,
    },
}

/// `backend` carries the App's gomuks-shaped transfer commands (uploads,
/// inline previews, downloads), which this engine answers too.
#[cfg(not(target_arch = "wasm32"))]
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

/// In the browser the engine runs in a worker; this starts it from the page.
#[cfg(target_arch = "wasm32")]
pub use worker::run;

/// Run on an explicit profile directory; tests run several engines at once
/// and replace the system browser.
#[cfg(not(target_arch = "wasm32"))]
pub async fn run_in(
    root: PathBuf,
    opener: Opener,
    commands: Receiver<Command>,
    backend: Receiver<crate::backend::Command>,
    events: Sender<Event>,
) -> Result<(), String> {
    // reqwest 0.13 is built without a crypto provider (see Cargo.toml).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let profile = session::Profile::open(root)?;
    run_with(profile, opener, commands, backend, events).await
}

async fn run_with(
    profile: session::Profile,
    opener: Opener,
    commands: Receiver<Command>,
    backend: Receiver<Transfer>,
    events: Sender<Event>,
) -> Result<(), String> {
    let profile = Arc::new(profile);
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
        Ok(None) => {
            engine.emit(Event::Account(Account::default())).await;
            if session::gomuks_data(&engine.profile).await {
                engine.emit(Event::Status(UPGRADE.into())).await;
            }
        }
        Err(why) => {
            engine.emit(Event::Account(Account::default())).await;
            engine.emit(Event::Status(why.into())).await;
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
    /// Typing notices for `typing::notices`, in order.
    typing: Sender<(String, bool)>,
    /// Our status and the people shown, for `presence::run`.
    presence: Sender<presence::Request>,
    profile: tokio::sync::watch::Sender<profile::Changes>,
    tasks: Tasks,
}

struct Engine {
    profile: Arc<session::Profile>,
    events: Sender<Event>,
    notices: Sender<Notice>,
    opener: Opener,
    browser: Option<rt::JoinHandle<()>>,
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
        // Before the first sync, so the fixtures' log misses nothing.
        #[cfg(debug_assertions)]
        qa::watch(&client);
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
                    "The matrix-sdk engine couldn’t start syncing.".into(),
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
        let (profile, changes) = tokio::sync::watch::channel(profile::Changes::default());
        tasks.spawn(account_updates(
            client.clone(),
            self.events.clone(),
            changes,
        ));
        tasks.spawn(rooms::updates(client.clone(), self.events.clone()));
        tasks.spawn(messaging::account_data(client.clone(), self.events.clone()));
        state::member_changes(&client, self.events.clone());
        typing::watch(&client, self.events.clone());
        let (typing, notices) = async_channel::unbounded();
        tasks.spawn(typing::notices(client.clone(), notices));
        let (presence, requests) = async_channel::unbounded();
        tasks.spawn(presence::run(client.clone(), self.events.clone(), requests));
        #[cfg(target_os = "linux")]
        calls::watch(&client, self.events.clone());
        alerts::register(&client, self.events.clone()).await;
        tasks.spawn(timeline::resume_after_errors(client.clone()));
        #[cfg(debug_assertions)]
        tasks.spawn(qa::redecrypted(client.clone()));
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
            typing,
            presence,
            profile,
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
                        let now = account(&client, None, None).await;
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
                rt::spawn(async move {
                    let result = session::recover(&client, &secret).await;
                    drop(secret);
                    if result.is_ok() {
                        // Report the verified state before completing the request.
                        let _ = events
                            .send(Event::Account(account(&client, None, None).await))
                            .await;
                    }
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::SaveProfile { id, edit } => {
                let Some(live) = &self.live else {
                    return self
                        .emit(Event::ProfileSaved {
                            id,
                            result: Err(Failure::Unavailable),
                        })
                        .await;
                };
                let (client, changes, events) = (
                    live.client.clone(),
                    live.profile.clone(),
                    self.events.clone(),
                );
                rt::spawn(async move {
                    let result = profile::save(&client, edit).await;
                    if let Ok(saved) = &result {
                        changes.send_modify(|state| state.record(saved));
                    }
                    let _ = events.send(Event::ProfileSaved { id, result }).await;
                });
            }
            Command::DirectProfile { id, user } => {
                self.answer(
                    Event::ProfileDirect {
                        id,
                        result: Err(Failure::Unavailable),
                    },
                    move |client| async move {
                        Event::ProfileDirect {
                            id,
                            result: profile::direct(&client, &user).await,
                        }
                    },
                )
                .await
            }
            Command::Paginate { id, room } => {
                let Some(live) = &self.live else {
                    return self.done(id, Err(Failure::Unavailable)).await;
                };
                let (timelines, events) = (live.timelines.clone(), self.events.clone());
                rt::spawn(async move {
                    let result = timelines.paginate(&room).await;
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::Send {
                id,
                room,
                body,
                reply_to,
                pills,
            } => {
                let Some(live) = &self.live else {
                    return self.done(id, Err(Failure::Unavailable)).await;
                };
                let (timelines, events) = (live.timelines.clone(), self.events.clone());
                rt::spawn(async move {
                    let result = timelines.send(&room, body, &pills, reply_to).await;
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::VerifyDevice { id } => {
                let Some(live) = &self.live else {
                    return self.done(id, Err(Failure::Unavailable)).await;
                };
                let (verifier, events) = (live.verifier.clone(), self.events.clone());
                rt::spawn(async move {
                    let result = verifier.request().await;
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            Command::AcceptVerification => {
                if let Some(live) = &self.live {
                    let verifier = live.verifier.clone();
                    rt::spawn(async move { verifier.accept().await });
                }
            }
            Command::ConfirmEmoji { matches } => {
                if let Some(live) = &self.live {
                    let verifier = live.verifier.clone();
                    rt::spawn(async move { verifier.confirm(matches).await });
                }
            }
            Command::CancelVerification => {
                if let Some(live) = &self.live {
                    let verifier = live.verifier.clone();
                    rt::spawn(async move { verifier.cancel().await });
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
                rt::spawn(async move {
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
                rt::spawn(async move {
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
                rt::spawn(async move {
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
                self.browser = Some(rt::spawn(async move {
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
            Command::Discussion { id, room, action } => {
                let sources = self.sources.clone();
                self.answer(
                    Event::Discussion {
                        id,
                        result: Err(Failure::Unavailable),
                    },
                    move |client| async move {
                        Event::Discussion {
                            id,
                            result: threads::request(&client, &room, action, &sources).await,
                        }
                    },
                )
                .await;
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
                pills,
            } => {
                self.job(id, move |_, timelines, _| async move {
                    timelines.edit(&room, &event, body, &pills).await
                })
                .await
            }
            Command::Redact { id, room, event } => {
                self.job(id, move |client, _, _| async move {
                    messaging::redact(&client, &room, &event).await
                })
                .await
            }
            Command::Forward {
                id,
                room,
                event,
                to,
                note,
            } => {
                self.job(id, move |_, timelines, _| async move {
                    timelines.forward(&room, &event, &to, note).await
                })
                .await
            }
            Command::CreatePoll { id, room, draft } => {
                self.job(id, move |_, timelines, _| async move {
                    timelines.create_poll(&room, &draft).await
                })
                .await
            }
            Command::Vote {
                id,
                room,
                event,
                answers,
            } => {
                self.job(id, move |_, timelines, _| async move {
                    timelines.vote(&room, &event, answers).await
                })
                .await
            }
            Command::EndPoll { id, room, event } => {
                self.job(id, move |_, timelines, _| async move {
                    timelines.end_poll(&room, &event).await
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
            Command::MarkRoomRead { id, room, public } => {
                self.job(id, move |client, _, _| async move {
                    rooms::mark_latest_read(&client, &room, public).await
                })
                .await
            }
            Command::SearchPeople { term } => {
                if let Some(live) = &self.live {
                    let (client, events) = (live.client.clone(), self.events.clone());
                    rt::spawn(async move {
                        let result = dms::search(&client, &term).await;
                        let _ = events.send(Event::People { term, result }).await;
                    });
                }
            }
            Command::StartDm { id, users } => {
                self.created(id, move |client| async move {
                    dms::start(&client, &users).await
                })
                .await
            }
            Command::Block { id, user, block } => {
                self.job(id, move |client, _, _| async move {
                    dms::block(&client, &user, block).await
                })
                .await
            }
            Command::PreviewRoom { target, via } => {
                if let Some(live) = &self.live {
                    let (client, events) = (live.client.clone(), self.events.clone());
                    rt::spawn(async move {
                        let result = joining::preview(&client, &target, &via).await;
                        let _ = events.send(Event::RoomPreview { target, result }).await;
                    });
                }
            }
            Command::JoinRoom { id, target, via } => {
                self.job(id, move |client, _, _| async move {
                    joining::join(&client, &target, &via).await
                })
                .await
            }
            Command::PublicRooms { id, query } => {
                let unavailable = Event::PublicRooms {
                    id,
                    result: Err(Failure::Unavailable),
                };
                self.answer(unavailable, move |client| async move {
                    let result = explore::public_rooms(&client, query).await;
                    Event::PublicRooms { id, result }
                })
                .await
            }
            Command::SpaceRooms { space } => {
                if let Some(live) = &self.live {
                    let (client, events) = (live.client.clone(), self.events.clone());
                    rt::spawn(async move {
                        let result = explore::space_rooms(&client, &space).await;
                        let _ = events.send(Event::SpaceRooms { space, result }).await;
                    });
                }
            }
            Command::AcceptInvite { id, room } => {
                self.job(id, move |client, _, _| async move {
                    invites::accept(&client, &room).await
                })
                .await
            }
            Command::LeaveRoom { id, room } => {
                self.job(id, move |client, _, _| async move {
                    rooms::leave(&client, &room).await
                })
                .await
            }
            Command::FollowUpgrade { id, room } => {
                self.job(id, move |client, _, _| async move {
                    upgrades::follow(&client, &room).await
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
            Command::Typing { room, typing } => {
                if let Some(live) = &self.live {
                    let _ = live.typing.try_send((room, typing));
                }
            }
            // Before the next command: a Paginate after it opens the room
            // afresh rather than paging the timeline that's going away.
            Command::CloseTimeline { room } => {
                if let Some(live) = &self.live {
                    live.timelines.close(&room).await;
                }
            }
            Command::LinkPreview { url } => {
                if let Some(live) = &self.live {
                    let (client, events) = (live.client.clone(), self.events.clone());
                    rt::spawn(async move {
                        let preview = previews::fetch(&client, &url).await;
                        let _ = events.send(Event::LinkPreview { url, preview }).await;
                    });
                }
            }
            Command::SetLinkPreviews { id, room, on } => {
                self.job(id, move |client, _, _| async move {
                    previews::set(&client, &room, on).await
                })
                .await
            }
            Command::ShareTyping { id, send } => {
                self.job(id, move |client, _, _| async move {
                    typing::share(&client, send).await
                })
                .await
            }
            Command::PublishPresence { presence } => {
                if let Some(live) = &self.live {
                    let _ = live.presence.try_send(presence::Request::Publish(presence));
                }
            }
            Command::WatchPresence { users } => {
                if let Some(live) = &self.live {
                    let _ = live.presence.try_send(presence::Request::Watch(users));
                }
            }
            Command::SaveStatus {
                id,
                choice,
                message,
            } => {
                self.job(id, move |client, _, _| async move {
                    presence::save(&client, choice, &message).await
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
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Command::PushRule {
                id,
                kind,
                rule_id,
                change,
            } => {
                self.job(id, move |client, _, _| async move {
                    alerts::push_rule(&client, kind, &rule_id, change).await
                })
                .await
            }
            Command::RoomAccountData {
                id,
                room,
                kind,
                content,
            } => {
                self.job(id, move |client, _, _| async move {
                    alerts::room_account_data(&client, &room, &kind, content).await
                })
                .await
            }
            Command::SearchRooms { id, query } => {
                let unavailable = Event::RoomsFound {
                    id,
                    result: Err(Failure::Unavailable),
                };
                self.answer(unavailable, move |client| async move {
                    let result = search::search_rooms(&client, query).await;
                    Event::RoomsFound { id, result }
                })
                .await
            }
            Command::IndexRooms { id, rooms } => {
                self.job(id, move |client, _, _| async move {
                    search::index(&client, &rooms).await
                })
                .await
            }
            Command::RtcTransports { id } => {
                let unavailable = Event::RtcTransports {
                    id,
                    result: Err(Failure::Unavailable),
                };
                self.answer(unavailable, move |client| async move {
                    let result = match client.discover_rtc_transports().await {
                        Ok(found) => Ok(found
                            .unwrap_or_default()
                            .iter()
                            .filter_map(|t| serde_json::to_value(t).ok())
                            .collect()),
                        Err(_) => Err(Failure::Unreachable),
                    };
                    Event::RtcTransports { id, result }
                })
                .await
            }
            #[cfg(target_os = "linux")]
            Command::CallState { id, room } => {
                let unavailable = Event::CallState {
                    id,
                    result: Err(Failure::Unavailable),
                };
                self.answer(unavailable, move |client| async move {
                    let result = calls::call_state(&client, &room).await;
                    Event::CallState { id, result }
                })
                .await
            }
            #[cfg(target_os = "linux")]
            Command::DeclineRing { id, room, ring } => {
                self.job(id, move |client, _, _| async move {
                    calls::decline_ring(&client, &room, &ring).await
                })
                .await
            }
            Command::Mentions { id, rooms } => {
                let unavailable = Event::Mentions {
                    id,
                    result: Err(Failure::Unavailable),
                };
                self.answer(unavailable, move |client| async move {
                    let result = search::mentions(&client, &rooms).await;
                    Event::Mentions { id, result }
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
    async fn transfer(&self, command: Transfer) {
        let live = self.live.as_ref().map(|l| l.client.clone());
        let (sources, events) = (self.sources.clone(), self.events.clone());
        match command {
            Transfer::Inline { source, result } => {
                rt::spawn(async move {
                    let Some(client) = live else {
                        return drop(result.send(None).await);
                    };
                    #[cfg(not(target_arch = "wasm32"))]
                    let image = media::inline(client, sources, source).await;
                    #[cfg(target_arch = "wasm32")]
                    let image = media::inline_bytes(client, sources, source).await;
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
                rt::spawn(async move {
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
            #[cfg(not(target_arch = "wasm32"))]
            Transfer::Download {
                id,
                media,
                destination,
            } => {
                rt::spawn(async move {
                    let result = match live {
                        Some(client) => media::download(client, sources, media, destination).await,
                        None => Err(Failure::Unavailable),
                    };
                    let _ = events.send(Event::Done { id, result }).await;
                });
            }
            #[cfg(target_os = "linux")]
            Transfer::Fetch {
                media,
                destination,
                result,
            } => {
                rt::spawn(async move {
                    let fetched = match live {
                        Some(client) => media::download(client, sources, media, destination)
                            .await
                            .is_ok(),
                        None => false,
                    };
                    let _ = result.send(fetched).await;
                });
            }
            #[cfg(target_arch = "wasm32")]
            Transfer::Download { media, result } => {
                rt::spawn(async move {
                    let bytes = match live {
                        Some(client) => media::whole_file(client, sources, &media).await,
                        None => Err(Failure::Unavailable),
                    };
                    let _ = result.send(bytes).await;
                });
            }
            #[cfg(target_os = "linux")]
            Transfer::Voice {
                room,
                target,
                controls,
            } => {
                // The call reports in desktop_call's frames; pass them on.
                let (frames, from_call) = async_channel::unbounded::<serde_json::Value>();
                let forward = events.clone();
                rt::spawn(async move {
                    while let Ok(frame) = from_call.recv().await {
                        if forward.send(Event::CallFrame(frame)).await.is_err() {
                            return;
                        }
                    }
                });
                rt::spawn(async move {
                    let result = match live {
                        Some(client) => match calls::SdkSignal::new(client, &room) {
                            Ok(signal) => {
                                crate::desktop_call::run_with(
                                    signal, &room, target, controls, &frames,
                                )
                                .await
                            }
                            Err(why) => Err(why),
                        },
                        None => Err("Sign in before calling."),
                    };
                    let label = result.unwrap_or_else(|e| e);
                    let _ = frames
                        .send(serde_json::json!({"command":"desktop_call","data":{
                            "room":room,"ended":true,"label":label}}))
                        .await;
                });
            }
        }
    }

    /// Run a request against the signed-in session off the command loop and
    /// answer it with `Event::Done`.
    async fn job<F, Fut>(&self, id: i64, work: F)
    where
        F: FnOnce(Client, Arc<timeline::Timelines>, Sender<Event>) -> Fut
            + matrix_sdk::SendOutsideWasm
            + 'static,
        Fut: Future<Output = Result<(), Failure>> + matrix_sdk::SendOutsideWasm + 'static,
    {
        let Some(live) = &self.live else {
            return self.done(id, Err(Failure::Unavailable)).await;
        };
        let (client, timelines, events) = (
            live.client.clone(),
            live.timelines.clone(),
            self.events.clone(),
        );
        rt::spawn(async move {
            let result = work(client, timelines, events.clone()).await;
            let _ = events.send(Event::Done { id, result }).await;
        });
    }

    /// Run `work` against the signed-in session off the command loop; it
    /// answers with its own event. Without a session, `unavailable` does.
    async fn answer<F, Fut>(&self, unavailable: Event, work: F)
    where
        F: FnOnce(Client) -> Fut + matrix_sdk::SendOutsideWasm + 'static,
        Fut: Future<Output = Event> + matrix_sdk::SendOutsideWasm + 'static,
    {
        let Some(live) = &self.live else {
            return self.emit(unavailable).await;
        };
        let (client, events) = (live.client.clone(), self.events.clone());
        rt::spawn(async move {
            let _ = events.send(work(client).await).await;
        });
    }

    /// Requests that make a room or an event, answered with its ID.
    async fn created<F, Fut>(&self, id: i64, work: F)
    where
        F: FnOnce(Client) -> Fut + matrix_sdk::SendOutsideWasm + 'static,
        Fut: Future<Output = Result<String, Failure>> + matrix_sdk::SendOutsideWasm + 'static,
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
        rt::spawn(async move {
            let result = match kind {
                Key::SetUp(password) => account::set_up_recovery(&client, &password).await,
                Key::Change => account::change_recovery_key(&client).await,
                Key::Reset(password) => account::reset_encryption(&client, &password).await,
            };
            let result = match result {
                Ok(key) => {
                    let _ = events.send(Event::RecoveryKey(key)).await;
                    let _ = events
                        .send(Event::Account(account(&client, None, None).await))
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
        let _ = rt::timeout(Duration::from_secs(5), sync.stop()).await;
        let server = rt::timeout(Duration::from_secs(15), client.logout()).await;
        drop((sync, client));
        session::forget(&self.profile).await;
        self.emit(Event::Account(Account::default())).await;
        if !matches!(server, Ok(Ok(()))) {
            self.emit(Event::Status(
                "Signed out on this device. The homeserver couldn’t be reached, so it may still list this device."
                    .into(),
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
                        let now = account(&client, None, None).await;
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
                    // As on sign-out: a sync service stuck retrying with the
                    // rejected token must not keep the notice from the UI.
                    let _ = rt::timeout(Duration::from_secs(5), live.sync.stop()).await;
                }
                session::forget(&self.profile).await;
                self.emit(Event::Account(Account::default())).await;
                self.emit(Event::Status(
                    "The homeserver signed this device out. Sign in again.".into(),
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
            let _ = rt::timeout(Duration::from_secs(5), live.sync.stop()).await;
        }
    }
}

pub(super) async fn account(
    client: &Client,
    display_name: Option<String>,
    avatar: Option<String>,
) -> Account {
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
        avatar,
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
async fn account_updates(
    client: Client,
    events: Sender<Event>,
    mut changes: tokio::sync::watch::Receiver<profile::Changes>,
) {
    use futures_util::StreamExt;
    let mut verification = client.encryption().verification_state();
    let mut recovery = client.encryption().recovery().state_stream();
    let mut name = None;
    let mut avatar = None;
    let mut last = None;
    // Ask the homeserver once, so a fresh device knows whether the account
    // already has cross-signing keys before offering to create them.
    if let Some(user) = client.user_id() {
        let _ = rt::timeout(
            Duration::from_secs(10),
            client.encryption().request_user_identity(user),
        )
        .await;
    }
    loop {
        let saved = changes.borrow_and_update().clone();
        if let Some(value) = saved.name {
            name = value;
        } else if name.is_none() {
            name = rt::timeout(Duration::from_secs(10), client.account().get_display_name())
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();
        }
        if let Some(value) = saved.avatar {
            avatar = value;
        } else if avatar.is_none() {
            avatar = rt::timeout(Duration::from_secs(10), client.account().get_avatar_url())
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .map(|u| u.to_string());
        }
        let now = account(&client, name.clone(), avatar.clone()).await;
        if changes.has_changed().unwrap_or(false) {
            continue;
        }
        if last.as_ref() != Some(&now) {
            if events.send(Event::Account(now.clone())).await.is_err() {
                return;
            }
            last = Some(now);
        }
        tokio::select! {
            c = changes.changed() => if c.is_err() { return },
            v = verification.next() => if v.is_none() { return },
            r = recovery.next() => if r.is_none() { return },
            // Own-device signatures can land without a state transition.
            _ = rt::sleep(Duration::from_secs(30)) => {}
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
            rt::sleep(Duration::from_secs(3)).await;
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
struct Tasks(Vec<rt::JoinHandle<()>>);
impl Tasks {
    fn spawn(&mut self, task: impl Future<Output = ()> + matrix_sdk::SendOutsideWasm + 'static) {
        self.0.push(rt::spawn(task));
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
