//! Presence (N4): online, idle, Do Not Disturb and offline dots on people's
//! pictures, their custom status, and our own status from the user panel,
//! where the homeserver shares presence (`m.presence`). Many large
//! homeservers turn it off: the engine then finds that our own status doesn't
//! read back, and no dots show anywhere rather than everyone offline.
//!
//! Our status is a choice (Online, Idle or Invisible) with a custom status
//! message, both kept in account data for all our devices, and the existing
//! Do Not Disturb (alerts paused) over it. Online turns Idle after five
//! minutes without input in the window, as on Discord, and back on the next
//! input. The spec has no Do Not Disturb, so others see it as online;
//! Invisible publishes offline, without the custom status.
//!
//! The engine (`engine/presence.rs`) publishes what [`App::sync_presence`]
//! asks for and reads the presence of the people the window shows: the
//! profile card, the DM list and the member list.
use crate::{App, Msg, Request, Section, engine::Command};
use iced::{Subscription, Task, time::Instant, widget::operation};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

/// Account data holding our choice and custom status, for all our devices.
pub const SAVED: &str = "io.github.koalexbear.untethered.status";
/// Online turns Idle after this long without input, as on Discord.
pub const AWAY_AFTER: Duration = Duration::from_secs(5 * 60);
/// A custom status is at most this many characters, as on Discord.
pub const MAX_MESSAGE: usize = 128;
/// The engine reads at most this many people's presence at a time.
const MAX_WATCHED: usize = 100;
/// The member list shows this many members at most (view/room_people.rs).
const MEMBERS_SHOWN: usize = 80;
/// The custom status field, focused when its dialog opens.
pub const FIELD: &str = "custom-status";

/// What a dot shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Status {
    Online,
    Idle,
    /// Do Not Disturb: ours, or someone's MSC3026 "busy".
    Busy,
    Offline,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Idle => "Idle",
            Self::Busy => "Do Not Disturb",
            Self::Offline => "Offline",
        }
    }
    /// The spec's presence state that publishes this status: it has no Do
    /// Not Disturb, so that reads as online.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Online | Self::Busy => "online",
            Self::Idle => "unavailable",
            Self::Offline => "offline",
        }
    }
    /// A presence state as the homeserver reports it. MSC3026's busy (where
    /// a homeserver enables it) is Do Not Disturb; other unknown states read
    /// as online rather than hiding someone who is there.
    pub fn from_wire(state: &str) -> Self {
        match state {
            "offline" => Self::Offline,
            "unavailable" => Self::Idle,
            "busy" | "org.matrix.msc3026.busy" => Self::Busy,
            _ => Self::Online,
        }
    }
    /// Shown as online in the member list (Discord's Online and Offline groups).
    pub fn here(self) -> bool {
        self != Self::Offline
    }
}

/// Someone's presence: what the homeserver reports, or what we publish.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Presence {
    pub status: Status,
    /// Their custom status (`status_msg`).
    pub message: Option<String>,
}

/// Whether the homeserver shares presence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Support {
    /// Not known yet: no dots show. The engine says this when it starts.
    #[default]
    Unknown,
    On,
    Off,
}

/// The status we pick in the user panel, saved in account data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Choice {
    #[default]
    Online,
    Idle,
    Invisible,
}
impl Choice {
    pub fn status(self) -> Status {
        match self {
            Self::Online => Status::Online,
            Self::Idle => Status::Idle,
            Self::Invisible => Status::Offline,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Idle => "idle",
            Self::Invisible => "invisible",
        }
    }
}

/// The choice and custom status in the account data's content. Missing or
/// unknown means Online without a custom status.
pub fn read_saved(content: &Value) -> (Choice, String) {
    let choice = match content["status"].as_str() {
        Some("idle") => Choice::Idle,
        Some("invisible") => Choice::Invisible,
        _ => Choice::Online,
    };
    let message = content["message"].as_str().map_or_else(String::new, clean);
    (choice, message)
}

/// The account data's content for a choice and custom status.
pub fn saved(choice: Choice, message: &str) -> Value {
    json!({ "status": choice.name(), "message": message })
}

/// A custom status as typed: one line, trimmed, at most [`MAX_MESSAGE`]
/// characters.
pub fn clean(message: &str) -> String {
    message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_MESSAGE)
        .collect()
}

#[derive(Clone, Debug)]
pub enum Input {
    /// Time passed: are we idle yet?
    Tick,
    /// Input in the window after we went idle on our own.
    Back,
    /// Open or close the status menu in the user panel.
    Menu(bool),
    Choose(Choice),
    /// Do Not Disturb from the status menu: the existing alerts pause.
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    Dnd,
    /// The custom status dialog: open it, edit, save, clear or close it.
    Edit,
    Draft(String),
    Save,
    Clear,
    Close,
}

/// Inputs seen in the window so far. The listener counts them without a
/// message (which would rebuild the view on every mouse move); the tick
/// notices that the count moved.
static INPUTS: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub struct State {
    pub support: Support,
    /// Other people's presence, as last read.
    people: HashMap<String, Presence>,
    pub choice: Choice,
    pub message: String,
    /// The account data has been read: publishing now won't override a
    /// choice saved on another device, such as Invisible.
    loaded: bool,
    /// Saves on their way to the account data. A synced value doesn't
    /// replace the choice while one is.
    saving: usize,
    /// Online, but idle for [`AWAY_AFTER`]: published as idle.
    pub away: bool,
    inputs: u64,
    last_input: Option<Instant>,
    /// What the engine was last asked to publish and read.
    published: Option<Presence>,
    watched: Vec<String>,
    /// The status menu is open.
    pub menu: bool,
    /// The custom status dialog, with what's typed.
    pub dialog: Option<String>,
}
impl State {
    /// Someone's presence as a dot shows it: nothing while the homeserver
    /// doesn't share presence or we haven't read theirs yet.
    pub fn of(&self, user: &str) -> Option<&Presence> {
        (self.support == Support::On)
            .then(|| self.people.get(user))
            .flatten()
    }
    /// Our status: Do Not Disturb (`dnd`) over the choice, and Idle when
    /// Online went idle.
    pub fn own(&self, dnd: bool) -> Status {
        match self.choice {
            _ if dnd => Status::Busy,
            Choice::Online if self.away => Status::Idle,
            choice => choice.status(),
        }
    }
    /// What to publish: the custom status shows unless we're invisible.
    fn publish(&self, dnd: bool) -> Presence {
        let status = self.own(dnd);
        let message =
            (status != Status::Offline && !self.message.is_empty()).then(|| self.message.clone());
        Presence { status, message }
    }
    /// Seen as idle after `now`? Input counted since the last check makes
    /// it the last input.
    fn idle_check(&mut self, inputs: u64, now: Instant) -> bool {
        if inputs != self.inputs || self.last_input.is_none() {
            self.inputs = inputs;
            self.last_input = Some(now);
        }
        self.last_input
            .is_some_and(|last| now.saturating_duration_since(last) >= AWAY_AFTER)
    }
    /// The engine (re)started, perhaps for another account: what it knew
    /// is gone, and the saved status comes again.
    fn restart(&mut self) {
        *self = Self {
            menu: self.menu,
            ..Self::default()
        };
    }
}

fn count_input(event: iced::Event, _: iced::event::Status, _: iced::window::Id) -> Option<Msg> {
    if matches!(
        event,
        iced::Event::Keyboard(_) | iced::Event::Mouse(_) | iced::Event::Touch(_)
    ) {
        INPUTS.fetch_add(1, Ordering::Relaxed);
    }
    None
}

fn come_back(event: iced::Event, _: iced::event::Status, _: iced::window::Id) -> Option<Msg> {
    use iced::{Event, keyboard, mouse};
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { .. })
        | Event::Mouse(
            mouse::Event::CursorMoved { .. }
            | mouse::Event::ButtonPressed(_)
            | mouse::Event::WheelScrolled { .. },
        )
        | Event::Touch(_) => Some(Msg::Presence(Input::Back)),
        _ => None,
    }
}

impl App {
    fn dnd(&self) -> bool {
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        return self.logged_in && self.alerts.dnd == Some(true);
        #[cfg(not(any(target_os = "linux", target_arch = "wasm32")))]
        false
    }

    /// Our status as the user panel shows it.
    pub fn own_status(&self) -> Status {
        self.presence.own(self.dnd())
    }

    /// Someone's presence for a dot, ours included.
    pub fn presence_of(&self, user: &str) -> Option<Presence> {
        if self.presence.support != Support::On {
            return None;
        }
        if user == self.matrix_id {
            return Some(self.presence.publish(self.dnd()));
        }
        self.presence.of(user).cloned()
    }

    /// Someone's status for a dot, ours included.
    pub fn status_of(&self, user: &str) -> Option<Status> {
        if user == self.matrix_id {
            return (self.presence.support == Support::On).then(|| self.own_status());
        }
        self.presence.of(user).map(|p| p.status)
    }

    pub fn presence_input(&mut self, input: Input) -> Task<Msg> {
        let task = self.presence_change(input);
        // The update returns early with the task, before its usual tail.
        self.sync_presence();
        task
    }

    fn presence_change(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Tick => {
                let idle = self
                    .presence
                    .idle_check(INPUTS.load(Ordering::Relaxed), Instant::now());
                if idle && self.goes_idle() {
                    self.presence.away = true;
                }
            }
            Input::Back => {
                self.presence.away = false;
                self.presence.last_input = Some(Instant::now());
            }
            Input::Menu(open) => self.presence.menu = open,
            Input::Choose(choice) => {
                self.presence.menu = false;
                self.presence.away = false;
                #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
                if self.dnd() {
                    self.change_dnd(false);
                }
                let message = self.presence.message.clone();
                self.save_status(choice, message);
            }
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Input::Dnd => {
                self.presence.menu = false;
                self.change_dnd(true);
            }
            Input::Edit => {
                self.presence.menu = false;
                self.presence.dialog = Some(self.presence.message.clone());
                return operation::focus(FIELD);
            }
            Input::Draft(text) => {
                if let Some(draft) = &mut self.presence.dialog {
                    *draft = text.chars().take(MAX_MESSAGE).collect();
                }
            }
            Input::Save => {
                if let Some(draft) = self.presence.dialog.take() {
                    self.save_status(self.presence.choice, clean(&draft));
                }
            }
            Input::Clear => {
                self.presence.dialog = None;
                self.presence.menu = false;
                self.save_status(self.presence.choice, String::new());
            }
            Input::Close => self.presence.dialog = None,
        }
        Task::none()
    }

    /// Online turns Idle on its own; a chosen status, Do Not Disturb and a
    /// call don't (someone in a call is there).
    fn goes_idle(&self) -> bool {
        #[cfg(target_os = "linux")]
        if self.call.is_some() {
            return false;
        }
        self.presence.choice == Choice::Online && !self.dnd()
    }

    fn save_status(&mut self, choice: Choice, message: String) {
        if choice == self.presence.choice && message == self.presence.message {
            return;
        }
        let before = (self.presence.choice, self.presence.message.clone());
        // Shown at once; a failed save puts it back.
        self.presence.choice = choice;
        self.presence.message = message.clone();
        self.presence.saving += 1;
        self.command(
            |id| Command::SaveStatus {
                id,
                choice,
                message,
            },
            Request::SaveStatus(before.0, before.1.clone()),
        );
        if !matches!(self.pending.get(&self.seq), Some(Request::SaveStatus(..))) {
            // Not queued: nothing changed.
            self.presence.saving -= 1;
            (self.presence.choice, self.presence.message) = before;
        }
    }

    /// The account data save finished; a failed one puts back what was.
    pub fn status_saved(&mut self, before: (Choice, String), ok: bool) {
        self.presence.saving = self.presence.saving.saturating_sub(1);
        if !ok {
            (self.presence.choice, self.presence.message) = before;
            self.status = "Could not save your status. Try again.".into();
        }
    }

    /// The choice and custom status as synced, from this device or another.
    pub fn status_synced(&mut self, choice: Choice, message: String) {
        self.presence.loaded = true;
        if self.presence.saving == 0 {
            self.presence.choice = choice;
            self.presence.message = clean(&message);
        }
    }

    pub fn presence_support(&mut self, support: Support) {
        if support == Support::Unknown {
            self.presence.restart();
        }
        self.presence.support = support;
        if support == Support::Off {
            self.presence.menu &= self.dnd_available();
        }
    }

    pub fn presence_seen(&mut self, changes: Vec<(String, Presence)>) {
        for (user, presence) in changes {
            self.presence.people.insert(user, presence);
        }
    }

    /// Do Not Disturb works here (it pauses alerts, notifications.rs).
    pub fn dnd_available(&self) -> bool {
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        return self.alerts.dnd.is_some();
        #[cfg(not(any(target_os = "linux", target_arch = "wasm32")))]
        false
    }

    /// The people the window shows with a dot, most visible first: the
    /// profile card, the member list, then the DM list. Never us.
    fn watch_list(&self) -> Vec<&str> {
        if !self.logged_in || !self.verified || !self.window_shown() {
            return Vec::new();
        }
        let mut users: Vec<&str> = Vec::new();
        if let Some((_, user)) = &self.sdk_ui.profile.card {
            users.push(user);
        }
        if self.people {
            // As the member list filters them.
            let query = self.directory.filter.trim().to_lowercase();
            users.extend(
                self.directory
                    .users
                    .iter()
                    .filter(|(id, m)| {
                        m.kind == crate::moderation::Kind::Joined
                            && (query.is_empty()
                                || id.to_lowercase().contains(&query)
                                || m.name.to_lowercase().contains(&query))
                    })
                    .map(|(id, _)| id.as_str())
                    .take(MEMBERS_SHOWN),
            );
        }
        if self.section == Section::Direct {
            let store = &self.live;
            users.extend(
                store
                    .ordered_direct
                    .iter()
                    .filter_map(|id| store.list.get(id)?.dm_peer.as_deref()),
            );
        }
        let mut seen = BTreeSet::new();
        users.retain(|u| *u != self.matrix_id && seen.insert(*u));
        users.truncate(MAX_WATCHED);
        users
    }

    /// Whether the window is open, not only the tray. Tests have no window.
    fn window_shown(&self) -> bool {
        #[cfg(not(any(target_arch = "wasm32", test)))]
        return self.window_open();
        #[cfg(any(target_arch = "wasm32", test))]
        true
    }

    /// Ask the engine to publish our status and read the people shown,
    /// when either changed. Runs after every update, so it only compares
    /// until something did.
    pub fn sync_presence(&mut self) {
        if self.sdk.is_none()
            || !self.connected
            || !self.logged_in
            || !self.verified
            || self.presence.support == Support::Off
        {
            return;
        }
        // Not before the saved choice is known: an Invisible account
        // mustn't show online while it loads.
        if self.presence.loaded {
            let mut publish = self.presence.publish(self.dnd());
            // Do Not Disturb publishes as online: no need to tell the
            // homeserver again when it changes.
            if publish.status == Status::Busy {
                publish.status = Status::Online;
            }
            if self.presence.published.as_ref() != Some(&publish)
                && self.engine_send(Command::PublishPresence {
                    presence: publish.clone(),
                })
            {
                self.presence.published = Some(publish);
            }
        }
        let watch = self.watch_list();
        if watch != self.presence.watched {
            let users: Vec<String> = watch.into_iter().map(str::to_owned).collect();
            if self.engine_send(Command::WatchPresence {
                users: users.clone(),
            }) {
                self.presence.watched = users;
            }
        }
    }

    /// A command without an answer. A full queue drops it: the next update
    /// sends it again.
    fn engine_send(&self, command: Command) -> bool {
        self.sdk
            .as_ref()
            .is_some_and(|sdk| sdk.try_send(command).is_ok())
    }

    /// While Online could turn Idle, a tick to notice it and a listener
    /// counting input; once idle, the first input brings us back.
    pub fn presence_subscription(&self) -> Subscription<Msg> {
        if self.presence.support != Support::On || !self.logged_in {
            return Subscription::none();
        }
        if self.presence.away {
            iced::event::listen_with(come_back)
        } else if self.goes_idle() {
            Subscription::batch([
                iced::event::listen_with(count_input),
                iced::time::every(Duration::from_secs(10)).map(|_| Msg::Presence(Input::Tick)),
            ])
        } else {
            Subscription::none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Account, Event, Recovery, RoomList, RoomSummary};

    fn signed_in() -> (App, async_channel::Receiver<Command>) {
        let (transfers, _) = async_channel::bounded(1);
        let (tx, rx) = async_channel::bounded(64);
        let mut app = App::new(transfers);
        app.sdk = Some(tx);
        app.engine_event(Event::Account(Account {
            logged_in: true,
            user_id: "@me:x".into(),
            display_name: Some("Me".into()),
            avatar: None,
            verified: true,
            identity: true,
            recovery: Recovery::Enabled,
        }));
        app.engine_event(Event::Rooms(RoomList {
            invites: Vec::new(),
            rooms: vec![
                RoomSummary {
                    id: "!dm:x".into(),
                    name: "Mira".into(),
                    dm: true,
                    dm_peer: Some("@mira:x".into()),
                    ..Default::default()
                },
                RoomSummary {
                    id: "!group:x".into(),
                    name: "Mira and Jo".into(),
                    dm: true,
                    ..Default::default()
                },
            ],
            children: Default::default(),
        }));
        app.engine_event(Event::PresenceSupport(Support::Unknown));
        while rx.try_recv().is_ok() {}
        (app, rx)
    }

    fn sent(rx: &async_channel::Receiver<Command>) -> Vec<Command> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|c| {
                matches!(
                    c,
                    Command::PublishPresence { .. }
                        | Command::WatchPresence { .. }
                        | Command::SaveStatus { .. }
                )
            })
            .collect()
    }

    fn published(rx: &async_channel::Receiver<Command>) -> Vec<Presence> {
        sent(rx)
            .into_iter()
            .filter_map(|c| match c {
                Command::PublishPresence { presence } => Some(presence),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nothing_is_published_before_the_saved_choice_is_known() {
        let (mut app, rx) = signed_in();
        app.sync_presence();
        assert!(published(&rx).is_empty());
        // An Invisible account never shows online while it loads.
        app.engine_event(Event::OwnStatus {
            choice: Choice::Invisible,
            message: "brb".into(),
        });
        app.sync_presence();
        assert_eq!(
            published(&rx),
            [Presence {
                status: Status::Offline,
                message: None
            }]
        );
        // Unchanged, it isn't asked for again.
        app.sync_presence();
        assert!(published(&rx).is_empty());
    }

    #[test]
    fn the_dm_list_and_member_list_are_watched_without_us() {
        let (mut app, rx) = signed_in();
        let _ = app.update(Msg::Section(Section::Direct));
        let watched: Vec<_> = sent(&rx)
            .into_iter()
            .filter_map(|c| match c {
                Command::WatchPresence { users } => Some(users),
                _ => None,
            })
            .collect();
        // A group DM has no one peer to show.
        assert_eq!(watched.last(), Some(&vec!["@mira:x".to_owned()]));
        // Presence off: nothing more is asked, and nothing shows.
        app.engine_event(Event::PresenceSupport(Support::Off));
        app.engine_event(Event::Presence(vec![(
            "@mira:x".into(),
            Presence {
                status: Status::Idle,
                message: None,
            },
        )]));
        let _ = app.update(Msg::Section(Section::All));
        assert!(sent(&rx).is_empty());
        assert_eq!(app.status_of("@mira:x"), None);
        assert_eq!(app.status_of("@me:x"), None);
    }

    #[test]
    fn choosing_a_status_saves_it_and_publishes_it_and_a_failure_puts_it_back() {
        let (mut app, rx) = signed_in();
        app.engine_event(Event::OwnStatus {
            choice: Choice::Online,
            message: String::new(),
        });
        app.engine_event(Event::PresenceSupport(Support::On));
        app.sync_presence();
        let _ = sent(&rx);
        let _ = app.update(Msg::Presence(Input::Edit));
        let _ = app.update(Msg::Presence(Input::Draft("  At the   workshop ".into())));
        let _ = app.update(Msg::Presence(Input::Save));
        let _ = app.update(Msg::Presence(Input::Choose(Choice::Idle)));
        let commands = sent(&rx);
        let saves: Vec<_> = commands
            .iter()
            .filter_map(|c| match c {
                Command::SaveStatus {
                    id,
                    choice,
                    message,
                } => Some((*id, *choice, message.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            saves
                .iter()
                .map(|s| (s.1, s.2.as_str()))
                .collect::<Vec<_>>(),
            [
                (Choice::Online, "At the workshop"),
                (Choice::Idle, "At the workshop")
            ]
        );
        assert!(commands.iter().any(|c| matches!(
            c,
            Command::PublishPresence { presence } if presence.status == Status::Idle
                && presence.message.as_deref() == Some("At the workshop")
        )));
        assert_eq!(app.own_status(), Status::Idle);
        // A synced value doesn't override while a save is on its way.
        app.engine_event(Event::OwnStatus {
            choice: Choice::Online,
            message: String::new(),
        });
        assert_eq!(app.presence.choice, Choice::Idle);
        app.engine_event(Event::Done {
            id: saves[0].0,
            result: Ok(()),
        });
        app.engine_event(Event::Done {
            id: saves[1].0,
            result: Err(crate::engine::Failure::Failed),
        });
        assert_eq!(app.presence.choice, Choice::Online);
        assert_eq!(app.presence.message, "At the workshop");
    }

    #[test]
    fn idle_publishes_idle_and_input_brings_us_back() {
        let (mut app, rx) = signed_in();
        app.engine_event(Event::OwnStatus {
            choice: Choice::Online,
            message: String::new(),
        });
        app.engine_event(Event::PresenceSupport(Support::On));
        app.sync_presence();
        assert_eq!(
            published(&rx).last().map(|p| p.status),
            Some(Status::Online)
        );
        app.presence.away = true;
        app.sync_presence();
        assert_eq!(published(&rx).last().map(|p| p.status), Some(Status::Idle));
        let _ = app.update(Msg::Presence(Input::Back));
        assert_eq!(
            published(&rx).last().map(|p| p.status),
            Some(Status::Online)
        );
        // Someone else's presence shows once read; ours is what we publish.
        app.engine_event(Event::Presence(vec![(
            "@mira:x".into(),
            Presence {
                status: Status::Idle,
                message: Some("lunch".into()),
            },
        )]));
        assert_eq!(app.status_of("@mira:x"), Some(Status::Idle));
        assert_eq!(app.status_of("@me:x"), Some(Status::Online));
        // The engine restarting forgets what it read.
        app.engine_event(Event::PresenceSupport(Support::Unknown));
        assert_eq!(app.status_of("@mira:x"), None);
    }

    #[test]
    fn states_read_as_the_spec_names_them() {
        assert_eq!(Status::from_wire("online"), Status::Online);
        assert_eq!(Status::from_wire("unavailable"), Status::Idle);
        assert_eq!(Status::from_wire("offline"), Status::Offline);
        assert_eq!(Status::from_wire("org.matrix.msc3026.busy"), Status::Busy);
        // Someone there in a state we don't know isn't hidden.
        assert_eq!(Status::from_wire("org.example.dancing"), Status::Online);
        // Do Not Disturb publishes as online: the spec has nothing else.
        assert_eq!(Status::Busy.wire(), "online");
        assert_eq!(Status::Idle.wire(), "unavailable");
        assert!(Status::Idle.here() && Status::Busy.here() && !Status::Offline.here());
    }

    #[test]
    fn the_saved_choice_round_trips_and_defaults_to_online() {
        assert_eq!(read_saved(&Value::Null), (Choice::Online, String::new()));
        assert_eq!(
            read_saved(&json!({ "status": "dancing" })).0,
            Choice::Online
        );
        for choice in [Choice::Online, Choice::Idle, Choice::Invisible] {
            assert_eq!(
                read_saved(&saved(choice, "At the workshop")),
                (choice, "At the workshop".into())
            );
        }
    }

    #[test]
    fn a_custom_status_is_one_trimmed_line_of_at_most_128_characters() {
        assert_eq!(clean("  Out\nfor   lunch "), "Out for lunch");
        assert_eq!(clean(&"é".repeat(200)).chars().count(), MAX_MESSAGE);
    }

    #[test]
    fn our_status_puts_do_not_disturb_over_the_choice_and_idles_only_online() {
        let mut state = State::default();
        assert_eq!(state.own(false), Status::Online);
        assert_eq!(state.own(true), Status::Busy);
        state.away = true;
        assert_eq!(state.own(false), Status::Idle);
        state.choice = Choice::Invisible;
        assert_eq!(state.own(false), Status::Offline);
        state.message = "brb".into();
        // Invisible hides the custom status too; otherwise it's published.
        assert_eq!(state.publish(false).message, None);
        state.choice = Choice::Idle;
        assert_eq!(
            state.publish(false),
            Presence {
                status: Status::Idle,
                message: Some("brb".into())
            }
        );
        assert_eq!(state.publish(true).status, Status::Busy);
    }

    #[test]
    fn idle_comes_five_minutes_after_the_last_input() {
        let mut state = State::default();
        let start = Instant::now();
        assert!(!state.idle_check(0, start));
        assert!(!state.idle_check(0, start + AWAY_AFTER / 2));
        // Input counted in between moves the last input to this check.
        assert!(!state.idle_check(3, start + AWAY_AFTER - Duration::from_secs(1)));
        assert!(!state.idle_check(3, start + AWAY_AFTER + Duration::from_secs(1)));
        assert!(state.idle_check(3, start + AWAY_AFTER * 2));
    }

    #[test]
    fn nothing_shows_until_the_homeserver_shares_presence() {
        let mut state = State::default();
        let mira = Presence {
            status: Status::Idle,
            message: None,
        };
        state.people.insert("@mira:x".into(), mira.clone());
        assert_eq!(state.of("@mira:x"), None);
        state.support = Support::On;
        assert_eq!(state.of("@mira:x"), Some(&mira));
        state.support = Support::Off;
        assert_eq!(state.of("@mira:x"), None);
    }
}
