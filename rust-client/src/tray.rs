//! The system tray (ROADMAP N6), as on Discord: an icon with an unread and
//! mention badge and the call's state, a menu with Open, Mute, Deafen, Do Not
//! Disturb and Quit, and closing the window to the tray so sync,
//! notifications, ringing and calls carry on. A StatusNotifierItem on Linux
//! (`tray/linux.rs`), the notification area on Windows (`tray/windows.rs`).
//! Browsers have no tray, so none of this is built for the web.
//!
//! Wayland can't hide a window, so "closing to the tray" really closes it and
//! the tray opens a new one: on the desktop the app is an iced daemon that may
//! have no window (main.rs). Everything the window shows lives in `App`, so
//! only widget state such as scroll offsets and focus starts over.
mod autostart;
mod icon;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    //! No tray on other systems yet: closing the window quits.
    #[derive(Clone)]
    pub struct Link;
    impl Link {
        pub fn show(&self, _: super::Snapshot) -> iced::Task<crate::Msg> {
            iced::Task::none()
        }
    }
    impl Link {
        pub fn shutdown(&self) {}
    }
    pub async fn start(_: async_channel::Sender<super::Event>, _: bool) -> Option<Link> {
        None
    }
}
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(windows)]
use windows as platform;

use crate::{App, Msg, rooms::Room};
use iced::{Task, window};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

pub use autostart::HIDDEN_FLAG;

/// Settings kept on this computer, in the profile directory: unlike read
/// receipts they describe this desktop, not the Matrix account.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Closing the window keeps Untethered running in the tray.
    pub close_to_tray: bool,
}
impl Default for Settings {
    fn default() -> Self {
        // On by default, as on Discord.
        Self {
            close_to_tray: true,
        }
    }
}
impl Settings {
    fn path() -> Option<PathBuf> {
        let root = crate::engine::profile_root().ok();
        #[cfg(windows)]
        let root = root
            .or_else(|| std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("Untethered")));
        Some(root?.join("desktop.json"))
    }
    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    fn save(self) -> Result<(), &'static str> {
        const FAILED: &str = "Couldn’t save the setting in the profile directory.";
        let path = Self::path().ok_or(FAILED)?;
        let dir = path.parent().ok_or(FAILED)?;
        std::fs::create_dir_all(dir).map_err(|_| FAILED)?;
        // Replace the file whole, so a crash never leaves half a file.
        let next = path.with_extension("json.new");
        std::fs::write(&next, serde_json::to_vec_pretty(&self).map_err(|_| FAILED)?)
            .map_err(|_| FAILED)?;
        std::fs::rename(&next, &path).map_err(|_| FAILED)
    }
}

/// Whether a tray is showing the icon.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Host {
    /// Still asking the desktop.
    #[default]
    Starting,
    Ready,
    /// The desktop has no tray (no StatusNotifierItem host), or it went away.
    Missing,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OnClose {
    Hide,
    Quit,
}
/// What closing the window does. Only hide it while a tray shows the icon,
/// so Untethered never keeps running where nobody can see it, and quit once
/// the engine has stopped: a tray icon for a dead client helps nobody.
pub fn on_close(settings: Settings, host: Host, running: bool) -> OnClose {
    if settings.close_to_tray && host == Host::Ready && running {
        OnClose::Hide
    } else {
        OnClose::Quit
    }
}
/// Whether the window must come back on its own: the tray went away (or never
/// came up for a start at sign-in), or the engine stopped and its error needs
/// reading. Not while quitting.
pub fn must_show(window_open: bool, quitting: bool, host: Host, running: bool) -> bool {
    !window_open && !quitting && (host == Host::Missing || !running)
}

/// The unread state the icon shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Badge {
    #[default]
    None,
    Unread,
    Mentions(u64),
}
impl Badge {
    /// Unread rooms and their mentions, by the server rail's rules (view.rs,
    /// `Mark`): spaces don't count, and every unread direct message counts as
    /// a mention, as on Discord.
    pub fn of<'a>(rooms: impl IntoIterator<Item = &'a Room>) -> Self {
        let mut unread = false;
        let mut mentions = 0;
        for r in rooms.into_iter().filter(|r| !r.space && r.unread) {
            unread = true;
            mentions += if r.dm {
                r.read.messages.max(r.read.highlights)
            } else {
                r.read.highlights
            };
        }
        match (mentions, unread) {
            (0, false) => Self::None,
            (0, true) => Self::Unread,
            (n, _) => Self::Mentions(n),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Call {
    /// The room or person, as the call dock names it.
    pub place: String,
    pub muted: bool,
    pub deafened: bool,
}

/// Everything the tray shows. The platform side redraws when it changes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Snapshot {
    pub badge: Badge,
    pub call: Option<Call>,
    /// Who is calling, while an incoming call rings.
    pub ringing: Option<String>,
    /// None while Do Not Disturb can't be changed: signed out, or the
    /// homeserver's rule isn't one Untethered can switch.
    pub dnd: Option<bool>,
}
impl Snapshot {
    /// The tooltip's lines under the app's name.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        if let Some(caller) = &self.ringing {
            lines.push(format!("Incoming call from {caller}"));
        }
        if let Some(c) = &self.call {
            let state = if c.deafened {
                " (deafened)"
            } else if c.muted {
                " (muted)"
            } else {
                ""
            };
            lines.push(format!("In a call: {}{state}", c.place));
        }
        match self.badge {
            Badge::Mentions(1) => lines.push("1 mention".into()),
            Badge::Mentions(n) => lines.push(format!("{n} mentions")),
            Badge::Unread => lines.push("Unread messages".into()),
            Badge::None => {}
        }
        if self.dnd == Some(true) {
            lines.push("Do Not Disturb".into());
        }
        lines.join("\n")
    }
}

/// What the tray and the settings page send the app.
#[derive(Clone)]
pub enum Event {
    /// The tray came up, or the desktop has none.
    Started(Option<platform::Link>),
    /// The desktop's tray went away (false) or came back. Windows puts the
    /// icon back itself when Explorer restarts.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Host(bool),
    Open,
    Mute,
    Deafen,
    Dnd,
    Answer,
    Decline,
    Quit,
    /// Quitting took too long: exit anyway.
    Exit,
    CloseToTray(bool),
    StartAtLogin(bool),
    Saved(Change, Result<(), &'static str>),
}
#[derive(Clone, Copy)]
pub enum Change {
    CloseToTray(bool),
    StartAtLogin(bool),
}

#[derive(Default)]
pub struct Tray {
    pub settings: Settings,
    /// This computer starts Untethered at sign-in: the autostart entry or the
    /// Run key is the record, so removing it in the desktop's own settings
    /// shows here too.
    pub start_at_login: bool,
    /// A setting is being written.
    pub saving: bool,
    /// Why the last change to a setting didn't save.
    pub note: Option<&'static str>,
    pub host: Host,
    link: Option<platform::Link>,
    shown: Option<Snapshot>,
    /// The main window, None while Untethered is in the tray.
    window: Option<window::Id>,
    /// Started at sign-in: stay in the tray once it shows the icon.
    hidden_start: bool,
}

impl App {
    /// Load the settings, open the window unless this is a start at sign-in,
    /// and put the icon in the tray.
    pub fn boot_tray(&mut self) -> Task<Msg> {
        self.tray.settings = Settings::load();
        self.tray.start_at_login = autostart::enabled();
        self.tray.hidden_start = std::env::args().any(|a| a == HIDDEN_FLAG);
        let (tx, rx) = async_channel::unbounded();
        let start = Task::perform(platform::start(tx, self.tray.hidden_start), |link| {
            Msg::Tray(Event::Started(link))
        });
        let window = if self.tray.hidden_start {
            Task::none()
        } else {
            self.open_window()
        };
        Task::batch([window, start, Task::run(rx, Msg::Tray)])
    }
    pub fn tray_event(&mut self, event: Event) -> Task<Msg> {
        let task = self.handle_tray(event);
        Task::batch([task, self.sync_tray()])
    }
    fn handle_tray(&mut self, event: Event) -> Task<Msg> {
        match event {
            Event::Started(link) => {
                self.tray.host = if link.is_some() {
                    Host::Ready
                } else {
                    Host::Missing
                };
                self.tray.link = link;
                self.tray.shown = None;
            }
            Event::Host(up) => {
                self.tray.host = if up { Host::Ready } else { Host::Missing };
                // A panel that comes back starts with a blank item.
                self.tray.shown = None;
            }
            Event::Open => return self.open_window(),
            #[cfg(target_os = "linux")]
            Event::Mute => return self.update(Msg::ToggleMic),
            #[cfg(target_os = "linux")]
            Event::Deafen => return self.update(Msg::ToggleDeafen),
            #[cfg(target_os = "linux")]
            Event::Dnd => {
                if let Some(on) = self.alerts.dnd {
                    return self.update(Msg::SetDnd(!on));
                }
            }
            #[cfg(target_os = "linux")]
            Event::Answer | Event::Decline => {
                let accept = matches!(event, Event::Answer);
                if let Some((offer, _)) = &self.inbox.ringing {
                    let id = offer.event.clone();
                    let decided = self.update(Msg::DecideRing(id, accept));
                    // The call opens in the window, as answering on Discord does.
                    return if accept {
                        Task::batch([decided, self.open_window()])
                    } else {
                        decided
                    };
                }
            }
            #[cfg(not(target_os = "linux"))]
            Event::Mute | Event::Deafen | Event::Dnd | Event::Answer | Event::Decline => {}
            Event::Quit => return self.leave(),
            Event::Exit => return iced::exit(),
            Event::CloseToTray(on) => return self.save_tray_setting(Change::CloseToTray(on)),
            Event::StartAtLogin(on) => return self.save_tray_setting(Change::StartAtLogin(on)),
            Event::Saved(change, result) => {
                self.tray.saving = false;
                self.tray.note = result.err();
                if result.is_ok() {
                    match change {
                        Change::CloseToTray(on) => self.tray.settings.close_to_tray = on,
                        Change::StartAtLogin(on) => self.tray.start_at_login = on,
                    }
                }
            }
        }
        Task::none()
    }
    fn save_tray_setting(&mut self, change: Change) -> Task<Msg> {
        if self.tray.saving {
            return Task::none();
        }
        self.tray.saving = true;
        let mut settings = self.tray.settings;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || match change {
                    Change::CloseToTray(on) => {
                        settings.close_to_tray = on;
                        settings.save()
                    }
                    Change::StartAtLogin(on) => autostart::set(on),
                })
                .await
                .unwrap_or(Err("Couldn’t save the setting."))
            },
            move |result| Msg::Tray(Event::Saved(change, result)),
        )
    }
    /// The window's close button: to the tray, or quit.
    pub fn close_requested(&mut self) -> Task<Msg> {
        match on_close(self.tray.settings, self.tray.host, !self.stopped) {
            OnClose::Hide if !self.closing => self.hide_window(),
            _ => self.leave(),
        }
    }
    /// Quit from the tray or the window. The icon goes at once, and a call
    /// that won't end can't keep an unseen Untethered running for long.
    fn leave(&mut self) -> Task<Msg> {
        if let Some(link) = self.tray.link.take() {
            link.shutdown();
        }
        let deadline = Task::perform(tokio::time::sleep(Duration::from_secs(10)), |_| {
            Msg::Tray(Event::Exit)
        });
        Task::batch([self.quit(), deadline])
    }
    fn hide_window(&mut self) -> Task<Msg> {
        let Some(id) = self.tray.window.take() else {
            return Task::none();
        };
        // A video would play on with nothing on screen.
        self.close_viewer();
        self.context_menu.open = None;
        // Nobody is reading now: alerts for the open room come back, and
        // push-to-talk held in the window lets go.
        let unfocused = self.update(Msg::WindowFocus(false));
        Task::batch([unfocused, window::close(id)])
    }
    /// Show the window: open a new one from the tray, or raise the open one.
    pub fn open_window(&mut self) -> Task<Msg> {
        if let Some(id) = self.tray.window {
            return Task::batch([window::minimize(id, false), window::gain_focus(id)]);
        }
        let (id, opened) = window::open(crate::main_window());
        self.tray.window = Some(id);
        // The new window's timeline starts at the newest messages, so the
        // reading position follows; a message opened in context keeps its place.
        let anchor = if self.original.is_some() {
            self.anchor_history()
        } else {
            self.timeline_end = None;
            self.scrolled_away = false;
            self.scrolled_far = false;
            Task::none()
        };
        opened.discard().chain(anchor)
    }
    /// Bring the window back if it must, and send the tray what changed.
    /// Runs after every update.
    pub fn sync_tray(&mut self) -> Task<Msg> {
        let show = if must_show(
            self.tray.window.is_some(),
            self.closing,
            self.tray.host,
            !self.stopped,
        ) {
            self.open_window()
        } else {
            Task::none()
        };
        let now = self.tray_snapshot();
        let Some(link) = &self.tray.link else {
            return show;
        };
        if self.tray.shown.as_ref() == Some(&now) {
            return show;
        }
        self.tray.shown = Some(now.clone());
        Task::batch([show, link.show(now)])
    }
    fn tray_snapshot(&self) -> Snapshot {
        #[cfg(target_os = "linux")]
        let (call, ringing, dnd) = {
            let name = |room: &str| {
                self.live
                    .list
                    .get(room)
                    .map_or_else(|| "a room".to_owned(), |r| r.name.clone())
            };
            let call = self.call.as_ref().map(|c| {
                let mode = c.audio.borrow();
                Call {
                    place: name(&c.room),
                    muted: mode.muted,
                    deafened: mode.deafened,
                }
            });
            let ringing = self.inbox.ringing.as_ref().map(|(offer, _)| {
                self.live
                    .list
                    .get(&offer.room)
                    .map_or_else(|| offer.peer.clone(), |r| r.display(&offer.peer).to_owned())
            });
            let dnd = self
                .alerts
                .dnd
                .filter(|_| self.connected && self.logged_in && self.verified);
            (call, ringing, dnd)
        };
        // Calls and notifications are Linux-only so far.
        #[cfg(not(target_os = "linux"))]
        let (call, ringing, dnd) = (None, None, None);
        Snapshot {
            badge: if self.logged_in {
                Badge::of(self.live.list.values())
            } else {
                Badge::None
            },
            call,
            ringing,
            dnd,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(dm: bool, unread: bool, messages: u64, highlights: u64) -> Room {
        let mut r = Room {
            dm,
            unread,
            ..Default::default()
        };
        r.read.messages = messages;
        r.read.highlights = highlights;
        r
    }

    #[test]
    fn closing_hides_only_while_a_tray_shows_the_icon() {
        let on = Settings::default();
        assert!(on.close_to_tray, "on by default, as on Discord");
        assert_eq!(on_close(on, Host::Ready, true), OnClose::Hide);
        let off = Settings {
            close_to_tray: false,
        };
        assert_eq!(on_close(off, Host::Ready, true), OnClose::Quit);
        // No tray host, or not yet known: quitting beats an invisible app.
        assert_eq!(on_close(on, Host::Missing, true), OnClose::Quit);
        assert_eq!(on_close(on, Host::Starting, true), OnClose::Quit);
        // The engine stopped: nothing left to keep running.
        assert_eq!(on_close(on, Host::Ready, false), OnClose::Quit);
    }

    #[test]
    fn the_window_comes_back_when_nothing_else_shows_untethered() {
        // Hidden in a working tray: stay there.
        assert!(!must_show(false, false, Host::Ready, true));
        // A start at sign-in waits for the tray's answer.
        assert!(!must_show(false, false, Host::Starting, true));
        // The tray went away or never came: show the window.
        assert!(must_show(false, false, Host::Missing, true));
        // The engine stopped while hidden: show its error.
        assert!(must_show(false, false, Host::Ready, false));
        // Already open, or on the way out: nothing to do.
        assert!(!must_show(true, false, Host::Missing, false));
        assert!(!must_show(false, true, Host::Missing, false));
    }

    #[test]
    fn badge_follows_unread_rooms_and_counts_direct_messages_as_mentions() {
        assert_eq!(Badge::of([]), Badge::None);
        // Counts on a read room don't show.
        assert_eq!(Badge::of([&room(false, false, 4, 2)]), Badge::None);
        assert_eq!(Badge::of([&room(false, true, 4, 0)]), Badge::Unread);
        let rooms = [
            room(false, true, 9, 2),
            // Every unread direct message is a mention.
            room(true, true, 3, 1),
            room(false, true, 1, 0),
        ];
        assert_eq!(Badge::of(&rooms), Badge::Mentions(5));
        // Spaces carry their children's state; they don't count again.
        let space = Room {
            space: true,
            ..room(false, true, 7, 7)
        };
        assert_eq!(Badge::of([&space]), Badge::None);
    }

    #[test]
    fn signed_out_shows_no_badge_and_a_live_room_does() {
        let (tx, _) = async_channel::bounded(1);
        let mut app = App::new(tx);
        // The signed-out preview's demo rooms have unread ones.
        assert_eq!(app.tray_snapshot().badge, Badge::None);
        app.logged_in = true;
        app.live.list.insert("!r".into(), room(false, true, 2, 1));
        assert_eq!(app.tray_snapshot().badge, Badge::Mentions(1));
    }

    #[test]
    fn summary_names_the_ring_the_call_and_the_badge() {
        let s = Snapshot {
            badge: Badge::Mentions(3),
            call: Some(Call {
                place: "general".into(),
                muted: true,
                deafened: false,
            }),
            ringing: Some("Mira".into()),
            dnd: Some(true),
        };
        assert_eq!(
            s.summary(),
            "Incoming call from Mira\nIn a call: general (muted)\n3 mentions\nDo Not Disturb"
        );
        assert_eq!(Snapshot::default().summary(), "");
        let one = Snapshot {
            badge: Badge::Mentions(1),
            ..Default::default()
        };
        assert_eq!(one.summary(), "1 mention");
    }

    #[test]
    fn settings_read_missing_fields_as_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
        let s: Settings = serde_json::from_str(r#"{"close_to_tray":false}"#).unwrap();
        assert!(!s.close_to_tray);
    }
}
