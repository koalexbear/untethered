//! Typing indicators (M3). While the composer holds a draft we tell the
//! room we're typing (the engine calls `Room::typing_notice`), renewing it
//! while the draft keeps changing, and stop on send, when the draft is
//! cleared, on switching rooms and after a few idle seconds. Who else is
//! typing in the open room reads "Mira is typing…" under the composer.
//! User Settings → Privacy has "Send typing indicators" (on by default, as
//! in Element), synced as account data like the read receipt preference.
use crate::{App, Msg, Request, engine::Command};
use iced::{
    Subscription,
    time::{Duration, Instant},
};
use std::collections::BTreeMap;

/// Still typing this long after the draft last changed.
pub const IDLE: Duration = Duration::from_secs(5);
/// How often the App repeats "typing" while it lasts. The SDK passes one
/// on at most every 3 seconds, each lasting 4 on the homeserver.
pub const RENEW: Duration = Duration::from_secs(1);
/// More people than this typing read "Several people are typing…".
const NAMED: usize = 3;

#[derive(Clone, Debug)]
pub enum Input {
    /// Time passed while we count as typing.
    Tick,
    /// The "Send typing indicators" switch.
    Share(bool),
}

#[derive(Default)]
pub struct Typing {
    pub sharing: Sharing,
    outgoing: Outgoing,
    incoming: Incoming,
}
impl Typing {
    /// Forget who is typing and our own typing, without notices: the
    /// engine or the sync has gone away.
    pub fn forget(&mut self) {
        self.outgoing = Outgoing::default();
        self.incoming = Incoming::default();
    }
}

/// A typing notice for the engine.
#[derive(Debug, PartialEq, Eq)]
pub struct Notice {
    pub room: String,
    pub typing: bool,
}

/// When to send typing notices for the composer.
#[derive(Default)]
pub struct Outgoing {
    current: Option<Current>,
}
struct Current {
    room: String,
    changed: Instant,
    renewed: Instant,
}
impl Outgoing {
    /// The draft in `room` changed; `empty` is whether nothing is left.
    pub fn changed(&mut self, room: &str, empty: bool, now: Instant) -> Vec<Notice> {
        let mut notices = Vec::new();
        if empty || self.current.as_ref().is_some_and(|c| c.room != room) {
            notices.extend(self.stop());
        }
        if empty {
            return notices;
        }
        match &mut self.current {
            Some(c) => {
                c.changed = now;
                if now.saturating_duration_since(c.renewed) >= RENEW {
                    c.renewed = now;
                    notices.push(start(room));
                }
            }
            None => {
                self.current = Some(Current {
                    room: room.to_owned(),
                    changed: now,
                    renewed: now,
                });
                notices.push(start(room));
            }
        }
        notices
    }
    /// Renew while the draft changed recently; stop once it hasn't for
    /// [`IDLE`].
    pub fn tick(&mut self, now: Instant) -> Option<Notice> {
        let c = self.current.as_mut()?;
        if now.saturating_duration_since(c.changed) >= IDLE {
            return self.stop();
        }
        if now.saturating_duration_since(c.renewed) < RENEW {
            return None;
        }
        c.renewed = now;
        Some(start(&c.room))
    }
    pub fn stop(&mut self) -> Option<Notice> {
        self.current.take().map(|c| Notice {
            room: c.room,
            typing: false,
        })
    }
    pub fn active(&self) -> bool {
        self.current.is_some()
    }
}
fn start(room: &str) -> Notice {
    Notice {
        room: room.to_owned(),
        typing: true,
    }
}

/// Who is typing in each room, as the homeserver last said, in the order
/// they started so names don't swap places when someone joins in.
#[derive(Default)]
pub struct Incoming(BTreeMap<String, Vec<String>>);
impl Incoming {
    pub fn set(&mut self, room: String, users: Vec<String>) {
        if users.is_empty() {
            self.0.remove(&room);
            return;
        }
        let list = self.0.entry(room).or_default();
        list.retain(|u| users.contains(u));
        for user in users {
            if !list.contains(&user) {
                list.push(user);
            }
        }
    }
    pub fn users(&self, room: &str) -> &[String] {
        self.0.get(room).map(Vec::as_slice).unwrap_or_default()
    }
}

/// The "Send typing indicators" preference.
pub struct Sharing {
    pub send: bool,
    revision: u64,
}
impl Default for Sharing {
    fn default() -> Self {
        Self {
            send: true,
            revision: 0,
        }
    }
}
impl Sharing {
    fn update(&mut self, send: bool) {
        self.revision += 1;
        self.send = send;
    }
    /// A save finished; a newer synced value wins over it.
    fn saved(&mut self, send: bool, revision: u64) {
        if revision == self.revision {
            self.update(send);
        }
    }
}

/// One piece of the typing line: names are bold, as on Discord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part<'a> {
    Name(&'a str),
    Text(&'static str),
}

/// "Mira is typing…", "Mira and Jo are typing…", "Mira, Jo and Sam are
/// typing…", then "Several people are typing…". Empty when nobody is.
pub fn phrase<'a>(names: &[&'a str]) -> Vec<Part<'a>> {
    use Part::{Name, Text};
    match names {
        [] => Vec::new(),
        [a] => vec![Name(a), Text(" is typing…")],
        [a, b] => vec![Name(a), Text(" and "), Name(b), Text(" are typing…")],
        [a, b, c] => vec![
            Name(a),
            Text(", "),
            Name(b),
            Text(" and "),
            Name(c),
            Text(" are typing…"),
        ],
        _ => {
            debug_assert!(names.len() > NAMED);
            vec![Text("Several people are typing…")]
        }
    }
}

impl App {
    /// The composer's text changed: typed, pasted or an emoji picked.
    pub fn composer_changed(&mut self) {
        let posting = self
            .rooms()
            .list
            .get(&self.selected)
            .is_some_and(|r| r.posting.can_post(&self.matrix_id, r.encrypted) != Some(false));
        if !self.typing.sharing.send
            || !posting
            || !self.logged_in
            || !self.verified
            || !self.connected
        {
            return self.stop_typing();
        }
        let notices =
            self.typing
                .outgoing
                .changed(&self.selected, self.editor.is_empty(), Instant::now());
        self.send_notices(notices);
    }

    /// Sending, switching rooms or turning the switch off.
    pub fn stop_typing(&mut self) {
        let notice = self.typing.outgoing.stop();
        self.send_notices(notice);
    }

    fn send_notices(&self, notices: impl IntoIterator<Item = Notice>) {
        let Some(sdk) = &self.sdk else { return };
        for Notice { room, typing } in notices {
            // A full queue drops it: the next change or tick sends another,
            // and the homeserver times a start out by itself.
            let _ = sdk.try_send(Command::Typing { room, typing });
        }
    }

    pub fn typing_input(&mut self, input: Input) {
        match input {
            Input::Tick => {
                let notice = self.typing.outgoing.tick(Instant::now());
                self.send_notices(notice);
            }
            Input::Share(send) => self.share_typing(send),
        }
    }

    pub fn changing_typing_sharing(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::ShareTyping(..)))
    }

    fn share_typing(&mut self, send: bool) {
        if !self.verified
            || !self.connected
            || !self.logged_in
            || self.changing_typing_sharing()
            || self.typing.sharing.send == send
        {
            return;
        }
        // Stop at once rather than after the homeserver has the setting.
        if !send {
            self.stop_typing();
        }
        let revision = self.typing.sharing.revision;
        self.command(
            |id| Command::ShareTyping { id, send },
            Request::ShareTyping(send, revision),
        );
    }

    /// The switch as synced, from this device or another.
    pub fn typing_shared(&mut self, send: bool) {
        if !send {
            self.stop_typing();
        }
        self.typing.sharing.update(send);
    }

    pub fn typing_saved(&mut self, send: bool, revision: u64, ok: bool) {
        if ok {
            self.typing.sharing.saved(send, revision);
        } else {
            self.status = "Could not save your typing indicator preference. Try again.".into();
        }
    }

    pub fn typing_seen(&mut self, room: String, mut users: Vec<String>) {
        users.retain(|u| *u != self.matrix_id);
        self.typing.incoming.set(room, users);
    }

    /// The typing line for the open room, with the timeline's names.
    pub fn typing_line(&self) -> Vec<Part<'_>> {
        let Some(r) = self.rooms().list.get(&self.selected) else {
            return Vec::new();
        };
        let names: Vec<&str> = self
            .typing
            .incoming
            .users(&self.selected)
            .iter()
            .map(|u| r.display(u))
            .collect();
        phrase(&names)
    }

    pub fn typing_subscription(&self) -> Subscription<Msg> {
        if self.typing.outgoing.active() {
            iced::time::every(Duration::from_millis(500)).map(|_| Msg::Typing(Input::Tick))
        } else {
            Subscription::none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Account, Event, Recovery, RoomList, RoomSummary};
    use iced::widget::text_editor::{Action, Content, Edit};

    fn text(parts: &[Part]) -> String {
        parts
            .iter()
            .map(|p| match p {
                Part::Name(s) | Part::Text(s) => *s,
            })
            .collect()
    }

    #[test]
    fn the_typing_line_names_up_to_three_people() {
        assert!(phrase(&[]).is_empty());
        assert_eq!(text(&phrase(&["Mira"])), "Mira is typing…");
        assert_eq!(text(&phrase(&["Mira", "Jo"])), "Mira and Jo are typing…");
        assert_eq!(
            text(&phrase(&["Mira", "Jo", "Sam"])),
            "Mira, Jo and Sam are typing…"
        );
        assert_eq!(
            text(&phrase(&["Mira", "Jo", "Sam", "Ana"])),
            "Several people are typing…"
        );
        assert_eq!(
            phrase(&["Mira", "Jo"])
                .into_iter()
                .filter(|p| matches!(p, Part::Name(_)))
                .count(),
            2,
            "names are bold"
        );
    }

    #[test]
    fn typing_starts_renews_and_stops_when_idle() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut out = Outgoing::default();
        assert_eq!(out.tick(t0), None, "nothing to renew before typing");
        assert_eq!(out.changed("!a", false, t0), vec![start("!a")]);
        assert!(out.active());
        // Keystrokes within RENEW don't repeat it...
        assert!(out.changed("!a", false, at(300)).is_empty());
        assert_eq!(out.tick(at(600)), None);
        // ...after it they do, from a keystroke or a tick.
        assert_eq!(out.changed("!a", false, at(1000)), vec![start("!a")]);
        assert_eq!(out.tick(at(2000)), Some(start("!a")));
        assert_eq!(out.tick(at(2500)), None);
        // A pause shorter than IDLE keeps renewing; IDLE without a change stops.
        assert_eq!(out.tick(at(5900)), Some(start("!a")));
        let stop = Notice {
            room: "!a".into(),
            typing: false,
        };
        assert_eq!(out.tick(at(6000)), Some(stop));
        assert!(!out.active());
        assert_eq!(out.tick(at(7000)), None, "stopped once");
    }

    #[test]
    fn clearing_the_draft_sending_or_switching_rooms_stops_typing() {
        let t0 = Instant::now();
        let stop = |room: &str| Notice {
            room: room.into(),
            typing: false,
        };
        let mut out = Outgoing::default();
        assert!(
            out.changed("!a", true, t0).is_empty(),
            "an empty draft isn't typing"
        );
        out.changed("!a", false, t0);
        assert_eq!(out.changed("!a", true, t0), vec![stop("!a")]);
        assert!(!out.active());
        // The next character starts again at once.
        assert_eq!(out.changed("!a", false, t0), vec![start("!a")]);
        // A change in another room's draft moves the notice there.
        assert_eq!(out.changed("!b", false, t0), vec![stop("!a"), start("!b")]);
        assert_eq!(out.stop(), Some(stop("!b")));
        assert_eq!(out.stop(), None);
    }

    #[test]
    fn people_keep_their_place_in_the_line() {
        let mut seen = Incoming::default();
        seen.set("!a".into(), vec!["@jo:x".into(), "@mira:x".into()]);
        seen.set(
            "!a".into(),
            vec!["@sam:x".into(), "@mira:x".into(), "@jo:x".into()],
        );
        assert_eq!(seen.users("!a"), ["@jo:x", "@mira:x", "@sam:x"]);
        seen.set("!a".into(), vec!["@sam:x".into()]);
        assert_eq!(seen.users("!a"), ["@sam:x"]);
        seen.set("!a".into(), vec![]);
        assert!(seen.users("!a").is_empty() && seen.0.is_empty());
    }

    fn signed_in_app() -> (App, async_channel::Receiver<Command>) {
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
            rooms: ["!a:x", "!b:x"]
                .map(|id| RoomSummary {
                    id: id.into(),
                    name: id.into(),
                    ..Default::default()
                })
                .into(),
            children: Default::default(),
        }));
        let _ = app.update(Msg::Open("!a:x".into()));
        while rx.try_recv().is_ok() {}
        (app, rx)
    }
    fn typing(rx: &async_channel::Receiver<Command>) -> Vec<(String, bool)> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|c| match c {
                Command::Typing { room, typing } => Some((room, typing)),
                _ => None,
            })
            .collect()
    }
    fn key(app: &mut App, c: char) {
        let _ = app.update(Msg::Draft(Action::Edit(Edit::Insert(c))));
    }

    #[test]
    fn the_composer_sends_typing_notices() {
        let (mut app, rx) = signed_in_app();
        let a = || "!a:x".to_owned();
        // Moving the cursor isn't typing.
        let _ = app.update(Msg::Draft(Action::Move(
            iced::widget::text_editor::Motion::Left,
        )));
        assert!(typing(&rx).is_empty());
        key(&mut app, 'h');
        key(&mut app, 'i');
        assert_eq!(typing(&rx), [(a(), true)], "one start for both keys");
        let _ = app.update(Msg::Send);
        assert_eq!(typing(&rx), [(a(), false)], "sending stops it");

        // Clearing the draft stops it.
        app.editor = Content::new();
        key(&mut app, 'x');
        let _ = app.update(Msg::Draft(Action::Edit(Edit::Backspace)));
        assert_eq!(typing(&rx), [(a(), true), (a(), false)]);

        // Switching rooms stops it, and the other room's draft doesn't
        // start until it changes.
        key(&mut app, 'y');
        let _ = app.update(Msg::Open("!b:x".into()));
        let _ = app.update(Msg::Open("!a:x".into()));
        assert_eq!(typing(&rx), [(a(), true), (a(), false)]);
        assert!(!app.editor.is_empty());

        // With the switch off nothing goes out, and turning it off stops a
        // notice that's running.
        key(&mut app, 'z');
        let _ = app.update(Msg::Typing(Input::Share(false)));
        let sent: Vec<Command> = std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|c| matches!(c, Command::Typing { .. } | Command::ShareTyping { .. }))
            .collect();
        assert!(matches!(
            sent.as_slice(),
            [
                Command::Typing { typing: true, .. },
                Command::Typing { typing: false, .. },
                Command::ShareTyping { send: false, .. }
            ]
        ));
        let Command::ShareTyping { id, .. } = sent[2] else {
            unreachable!()
        };
        app.engine_event(Event::Done { id, result: Ok(()) });
        assert!(!app.typing.sharing.send);
        key(&mut app, '!');
        assert!(typing(&rx).is_empty());
        // The synced value turns it back on.
        app.engine_event(Event::TypingSharing(true));
        key(&mut app, '?');
        assert_eq!(typing(&rx), [(a(), true)]);
    }

    #[test]
    fn others_typing_show_with_timeline_names_never_us() {
        let (mut app, _rx) = signed_in_app();
        app.live
            .list
            .get_mut("!a:x")
            .unwrap()
            .members
            .insert("@mira:x".into(), "Mira".into());
        app.engine_event(Event::Typing {
            room: "!a:x".into(),
            users: vec!["@mira:x".into(), "@me:x".into(), "@jo:x".into()],
        });
        assert_eq!(text(&app.typing_line()), "Mira and jo are typing…");
        // A member's name from their membership, before they post.
        app.engine_event(Event::Member {
            room: "!a:x".into(),
            user: "@jo:x".into(),
            member: crate::moderation::Member {
                kind: crate::moderation::Kind::Joined,
                name: "Jo".into(),
                avatar: None,
                event: "$jo".into(),
            },
        });
        assert_eq!(text(&app.typing_line()), "Mira and Jo are typing…");
        // Only the open room's.
        app.engine_event(Event::Typing {
            room: "!b:x".into(),
            users: vec!["@sam:x".into()],
        });
        assert_eq!(text(&app.typing_line()), "Mira and Jo are typing…");
        app.engine_event(Event::Typing {
            room: "!a:x".into(),
            users: vec![],
        });
        assert!(app.typing_line().is_empty());
    }
}
