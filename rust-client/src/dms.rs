//! New message and blocking (ROADMAP J3).
//!
//! New message finds people in the homeserver's user directory and starts a
//! DM with one of them, or a group DM with several (engine/dms.rs). The DM
//! list's **+** opens it, as Discord's does.
//!
//! Blocking puts someone on the account's ignore list (`m.ignored_user_list`),
//! which homeservers and other clients honour: the homeserver stops sending
//! their messages and invites. Here their loaded messages, typing, invites,
//! alerts and calls are hidden as well, for what arrived before the block.
use crate::{
    App, Msg, Request, Section,
    engine::{Command, Failure, Person},
    rooms::Room,
};
use iced::{Task, widget::operation};
use std::collections::BTreeSet;

/// Up to ten people in a group DM, you included, as on Discord.
pub const MAX_PEOPLE: usize = 9;
pub const SEARCH: &str = "new-message-search";

#[derive(Default)]
pub struct NewMessage {
    pub query: String,
    /// The directory's latest answer, for `searched`. Kept while the next
    /// search runs, so the list doesn't flicker as you type.
    pub found: Vec<Person>,
    pub searched: String,
    pub picked: Vec<Person>,
    pub note: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub enum Input {
    Open,
    Close,
    Query(String),
    Toggle(Person),
    Remove(String),
    Start,
}

/// A full Matrix user ID, `@name:server`.
pub fn matrix_id(text: &str) -> bool {
    text.strip_prefix('@')
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(local, server)| !local.is_empty() && !server.is_empty())
        && !text.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl NewMessage {
    /// Who the list offers: the directory's people, after a full Matrix ID
    /// typed in that it didn't find (many directories list only people who
    /// share a room with you). Never you, or anyone you block.
    pub fn people(&self, me: &str, blocked: &BTreeSet<String>) -> Vec<Person> {
        let term = self.query.trim();
        let mut people = Vec::new();
        if matrix_id(term) && !self.found.iter().any(|p| p.id == term) {
            people.push(Person {
                id: term.to_owned(),
                name: None,
                avatar: None,
            });
        }
        if !term.is_empty() {
            people.extend(self.found.iter().cloned());
        }
        people.retain(|p| p.id != me && !blocked.contains(&p.id));
        people
    }

    pub fn searching(&self) -> bool {
        let term = self.query.trim();
        !term.is_empty() && term != self.searched
    }

    pub fn picked(&self, id: &str) -> bool {
        self.picked.iter().any(|p| p.id == id)
    }
}

impl App {
    pub fn new_message(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Open if self.logged_in && self.verified => {
                self.new_message = Some(NewMessage::default());
                self.server_menu = false;
                self.context_menu.dismiss();
                self.sdk_ui.profile.dismiss_card();
                return operation::focus(SEARCH);
            }
            Input::Close if !self.starting_dm() => self.new_message = None,
            Input::Query(query) => {
                let Some(form) = &mut self.new_message else {
                    return Task::none();
                };
                form.query = query;
                form.note = None;
                let term = form.query.trim().to_owned();
                if term.is_empty() {
                    form.found.clear();
                    form.searched.clear();
                } else if term != form.searched
                    && let Some(sdk) = &self.sdk
                {
                    let _ = sdk.try_send(Command::SearchPeople { term });
                }
            }
            Input::Toggle(person) => {
                let busy = self.starting_dm();
                let Some(form) = self.new_message.as_mut().filter(|_| !busy) else {
                    return Task::none();
                };
                if let Some(at) = form.picked.iter().position(|p| p.id == person.id) {
                    form.picked.remove(at);
                    form.note = None;
                } else if form.picked.len() < MAX_PEOPLE {
                    form.picked.push(person);
                    form.note = None;
                } else {
                    form.note = Some("A group DM has room for 9 people besides you.");
                }
                return operation::focus(SEARCH);
            }
            Input::Remove(id) => {
                let busy = self.starting_dm();
                if let Some(form) = self.new_message.as_mut().filter(|_| !busy) {
                    form.picked.retain(|p| p.id != id);
                    form.note = None;
                }
            }
            Input::Start => self.start_dm(),
            _ => {}
        }
        Task::none()
    }

    fn start_dm(&mut self) {
        if !self.connected || self.starting_dm() {
            return;
        }
        let Some(form) = &self.new_message else {
            return;
        };
        let users: Vec<String> = form.picked.iter().map(|p| p.id.clone()).collect();
        if users.is_empty() {
            return;
        }
        // The DM you already have with one person opens as it is.
        if let [user] = users.as_slice()
            && let Some(room) = self.direct_with(user).cloned()
        {
            self.new_message = None;
            self.started_dm = Some(room);
            self.open_started_dm();
            return;
        }
        self.command(move |id| Command::StartDm { id, users }, Request::StartDm);
    }

    pub fn starting_dm(&self) -> bool {
        self.pending.values().any(|r| matches!(r, Request::StartDm))
    }

    /// The directory's answer, if it's for what the search box says now.
    pub fn people_found(&mut self, term: String, result: Result<Vec<Person>, Failure>) {
        let Some(form) = self.new_message.as_mut().filter(|f| f.query.trim() == term) else {
            return;
        };
        form.searched = term;
        match result {
            Ok(people) => form.found = people,
            Err(_) => {
                form.found.clear();
                form.note = Some(
                    "Couldn't search the user directory. A full Matrix ID, like \
                     @alex:example.org, still works.",
                );
            }
        }
    }

    pub fn dm_started(&mut self, result: Result<String, Failure>) {
        match result {
            Ok(room) => {
                self.new_message = None;
                self.started_dm = Some(room);
                self.open_started_dm();
            }
            Err(_) => {
                if let Some(form) = &mut self.new_message {
                    form.note = Some(
                        "Couldn't start the conversation. Check that each Matrix ID is \
                         right, and try again.",
                    );
                }
            }
        }
    }

    /// Open the conversation New message started, once the room list has
    /// it as a DM (its `m.direct` entry can sync a moment after the room).
    pub fn open_started_dm(&mut self) {
        let Some(id) = &self.started_dm else {
            return;
        };
        if !self
            .live
            .list
            .get(id)
            .is_some_and(|r| r.has_metadata && r.dm)
        {
            return;
        }
        let id = self.started_dm.take().unwrap();
        self.account = false;
        self.room_preview = None;
        self.invite_open = None;
        self.section = Section::Direct;
        self.open(id);
        self.reconcile();
    }

    /// Who this account blocks, as the ignore list now says.
    pub fn blocked_changed(&mut self, users: Vec<String>) {
        let users: BTreeSet<String> = users.into_iter().collect();
        if users == self.blocked {
            return;
        }
        let added: Vec<String> = users.difference(&self.blocked).cloned().collect();
        self.blocked = users;
        for user in &added {
            self.hide_blocked(user);
        }
        // The SDK reloads every timeline from its newest messages when the
        // list changes: fill the open room's screen again (main.rs).
        self.filled.clear();
    }

    /// Hide what someone just blocked left on screen: the homeserver
    /// stops sending more, and the SDK reloads timelines without theirs.
    fn hide_blocked(&mut self, user: &str) {
        for room in self.live.list.values_mut() {
            room.forget_sender(user);
        }
        self.typing.forget_user(user);
        self.hide_blocked_invites();
        if self
            .invite_open
            .as_ref()
            .is_some_and(|id| self.invite(id).is_none())
        {
            self.invite_open = None;
        }
    }

    /// Invites from people this account blocks never show.
    pub fn hide_blocked_invites(&mut self) {
        let blocked = &self.blocked;
        self.live
            .invites
            .retain(|i| i.inviter.as_ref().is_none_or(|u| !blocked.contains(u)));
    }

    pub fn is_blocked(&self, user: &str) -> bool {
        self.blocked.contains(user)
    }

    /// Block or unblock someone. Blocking asks first (context_menu.rs).
    pub fn set_blocked(&mut self, user: String, block: bool) {
        if user == self.matrix_id || self.blocking(&user) || self.is_blocked(&user) == block {
            return;
        }
        let target = user.clone();
        self.command(
            move |id| Command::Block {
                id,
                user: target,
                block,
            },
            Request::Block(user, block),
        );
    }

    pub fn blocking(&self, user: &str) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Request::Block(u, _) if u == user))
    }

    pub fn block_answered(&mut self, user: String, block: bool, ok: bool) {
        if !ok {
            self.status = if block {
                "Couldn't block them. Try again."
            } else {
                "Couldn't unblock them. Try again."
            }
            .into();
            return;
        }
        // The ignore list syncs back too; this shows it at once.
        let mut users: Vec<String> = self.blocked.iter().cloned().collect();
        if block {
            users.push(user.clone());
        } else {
            users.retain(|u| *u != user);
        }
        self.blocked_changed(users);
        let name = self.someone(&user);
        self.status = if block {
            format!("Blocked {name}. Unblock them in User Settings, under Privacy.")
        } else {
            format!("Unblocked {name}.")
        };
    }

    /// A person's display name from any room that knows it, else their ID.
    pub fn someone(&self, user: &str) -> String {
        self.live
            .list
            .values()
            .find_map(|r| r.members.get(user).filter(|n| !n.is_empty()))
            .cloned()
            .unwrap_or_else(|| user.to_owned())
    }
}

impl Room {
    /// Drop someone's loaded messages, as when they're blocked.
    pub fn forget_sender(&mut self, user: &str) {
        let rows: BTreeSet<i64> = self
            .history
            .iter()
            .filter(|(_, p)| p.sender == user)
            .map(|(row, _)| *row)
            .collect();
        if rows.is_empty() {
            return;
        }
        self.history.retain(|row, _| !rows.contains(row));
        self.forget_rows(&rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Event, Invite, Message, RoomList, RoomSummary, TimelineSnapshot};

    fn app() -> (App, async_channel::Receiver<Command>) {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, rx) = async_channel::bounded(8);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.matrix_id = "@me:example".into();
        (app, rx)
    }

    fn room(id: &str, dm: bool, peer: Option<&str>) -> RoomSummary {
        RoomSummary {
            id: id.into(),
            name: id.into(),
            dm,
            dm_peer: peer.map(Into::into),
            ..Default::default()
        }
    }

    fn rooms(app: &mut App, rooms: Vec<RoomSummary>, invites: Vec<Invite>) {
        app.engine_event(Event::Rooms(RoomList {
            invites,
            rooms,
            children: Default::default(),
        }));
    }

    fn message(row: i64, sender: &str) -> Message {
        Message {
            row,
            event_id: Some(format!("${row}")),
            sender: sender.into(),
            sender_name: None,
            sender_avatar: None,
            time: row,
            body: format!("message {row}"),
            formatted: None,
            kind: crate::engine::Kind::Text,
            encrypted: false,
            delivery: crate::engine::Delivery::Sent,
            edited: false,
            reply_to: None,
            reactions: Vec::new(),
            text: true,
            editable: false,
            mentions: None,
            edit_id: None,
            edit_time: None,
            receipts: Vec::new(),
            media: None,
            from: None,
            poll: None,
        }
    }

    fn pending(app: &App, wanted: impl Fn(&Request) -> bool) -> i64 {
        *app.pending.iter().find(|(_, r)| wanted(r)).unwrap().0
    }

    #[test]
    fn new_message_searches_picks_people_and_opens_their_group_dm() {
        let (mut app, rx) = app();
        let _ = app.update(Msg::Dm(Input::Open));
        let _ = app.update(Msg::Dm(Input::Query("mi".into())));
        assert!(matches!(rx.try_recv(), Ok(Command::SearchPeople { term }) if term == "mi"));
        let found = vec![
            person("@mira:example"),
            person("@milo:example"),
            person("@me:example"),
        ];
        // An answer for an older search is dropped.
        app.engine_event(Event::People {
            term: "m".into(),
            result: Ok(vec![person("@max:example")]),
        });
        app.engine_event(Event::People {
            term: "mi".into(),
            result: Ok(found.clone()),
        });
        let form = app.new_message.as_ref().unwrap();
        let ids: Vec<String> = form
            .people(&app.matrix_id, &app.blocked)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["@mira:example", "@milo:example"]);
        let _ = app.update(Msg::Dm(Input::Toggle(found[0].clone())));
        let _ = app.update(Msg::Dm(Input::Toggle(found[1].clone())));
        let _ = app.update(Msg::Dm(Input::Start));
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::StartDm { users, .. }) if users == ["@mira:example", "@milo:example"]
        ));
        // Starting, the dialog stays and Escape can't close it.
        let _ = app.update(Msg::Escape);
        assert!(app.new_message.is_some());
        let id = pending(&app, |r| matches!(r, Request::StartDm));
        app.engine_event(Event::Created {
            id,
            result: Ok("!group:example".into()),
        });
        assert!(app.new_message.is_none());
        // Listed before its `m.direct` entry syncs, it waits.
        rooms(
            &mut app,
            vec![room("!group:example", false, None)],
            Vec::new(),
        );
        assert!(app.started_dm.is_some());
        assert!(app.section != Section::Direct);
        rooms(
            &mut app,
            vec![room("!group:example", true, None)],
            Vec::new(),
        );
        assert_eq!(app.selected, "!group:example");
        assert!(app.section == Section::Direct);
    }

    #[test]
    fn new_message_opens_the_dm_you_have_with_one_person() {
        let (mut app, rx) = app();
        rooms(
            &mut app,
            vec![room("!dm:example", true, Some("@mira:example"))],
            Vec::new(),
        );
        let _ = app.update(Msg::Dm(Input::Open));
        let _ = app.update(Msg::Dm(Input::Toggle(person("@mira:example"))));
        while rx.try_recv().is_ok() {}
        let _ = app.update(Msg::Dm(Input::Start));
        assert!(rx.try_recv().is_err(), "no new room");
        assert!(app.new_message.is_none());
        assert_eq!(app.selected, "!dm:example");
    }

    #[test]
    fn blocking_hides_their_messages_and_invites() {
        let (mut app, _rx) = app();
        let invite = |id: &str, from: &str| Invite {
            id: id.into(),
            name: id.into(),
            inviter: Some(from.into()),
            ..Default::default()
        };
        rooms(
            &mut app,
            vec![room("!r:example", false, None)],
            vec![
                invite("!a:example", "@mira:example"),
                invite("!b:example", "@bob:example"),
            ],
        );
        let snapshot = |rows: std::ops::RangeInclusive<i64>| {
            Event::Timeline(TimelineSnapshot {
                room: "!r:example".into(),
                messages: rows
                    .map(|row| {
                        message(
                            row,
                            if row % 2 == 0 {
                                "@mira:example"
                            } else {
                                "@bob:example"
                            },
                        )
                    })
                    .collect(),
                start_reached: true,
                latest_event: None,
                seams: Vec::new(),
            })
        };
        app.engine_event(snapshot(1..=4));
        assert_eq!(app.live.list["!r:example"].posts().len(), 4);
        app.engine_event(Event::Blocked(vec!["@mira:example".into()]));
        let senders = |app: &App| -> Vec<String> {
            app.live.list["!r:example"]
                .posts()
                .map(|p| p.sender.clone())
                .collect()
        };
        assert_eq!(senders(&app), ["@bob:example", "@bob:example"]);
        let invites: Vec<&str> = app.live.invites.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(invites, ["!b:example"]);
        // What syncs later stays hidden too.
        app.engine_event(snapshot(1..=6));
        assert_eq!(senders(&app).len(), 3);
        rooms(
            &mut app,
            vec![room("!r:example", false, None)],
            vec![invite("!c:example", "@mira:example")],
        );
        assert!(app.live.invites.is_empty());
    }

    #[test]
    fn block_asks_first_and_unblock_does_not() {
        use crate::context_menu::{Entry, Event as Menu, Target};
        let (mut app, rx) = app();
        rooms(&mut app, vec![room("!r:example", false, None)], Vec::new());
        let entries = |app: &App| -> Vec<Entry> {
            app.menu_items(&Target::User {
                room: "!r:example".into(),
                user: "@mira:example".into(),
            })
            .into_iter()
            .map(|i| i.entry)
            .collect()
        };
        assert!(entries(&app).contains(&Entry::Block));
        while rx.try_recv().is_ok() {}
        let _ = app.context_menu_event(Menu::Block("@mira:example".into()));
        assert_eq!(app.context_menu.blocking.as_deref(), Some("@mira:example"));
        assert!(rx.try_recv().is_err(), "nothing until confirmed");
        let _ = app.context_menu_event(Menu::ConfirmBlock);
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::Block { user, block: true, .. }) if user == "@mira:example"
        ));
        let id = pending(&app, |r| matches!(r, Request::Block(..)));
        app.engine_event(Event::Done { id, result: Ok(()) });
        assert!(app.is_blocked("@mira:example"));
        assert!(app.status.starts_with("Blocked"));
        assert!(entries(&app).contains(&Entry::Unblock));
        let _ = app.context_menu_event(Menu::Unblock("@mira:example".into()));
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::Block { block: false, .. })
        ));
        // You can't block yourself.
        let _ = app.context_menu_event(Menu::Block("@me:example".into()));
        assert!(app.context_menu.blocking.is_none());
    }

    fn person(id: &str) -> Person {
        Person {
            id: id.into(),
            name: None,
            avatar: None,
        }
    }

    #[test]
    fn matrix_ids_need_a_name_and_a_server() {
        assert!(matrix_id("@mira:example.org"));
        for text in [
            "mira",
            "@mira",
            "@:example.org",
            "@mira:",
            "#room:example.org",
            "@mi ra:x",
        ] {
            assert!(!matrix_id(text), "{text}");
        }
    }

    #[test]
    fn the_list_offers_a_typed_id_and_never_you_or_anyone_blocked() {
        let blocked = BTreeSet::from(["@bob:example.org".to_owned()]);
        let mut form = NewMessage {
            query: "@zoe:other.org".into(),
            found: vec![
                person("@me:example.org"),
                person("@bob:example.org"),
                person("@mira:example.org"),
            ],
            searched: "@zoe:other.org".into(),
            ..Default::default()
        };
        let ids = |people: Vec<Person>| people.into_iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(
            ids(form.people("@me:example.org", &blocked)),
            ["@zoe:other.org", "@mira:example.org"]
        );
        // A typed ID the directory found isn't listed twice.
        form.query = "@mira:example.org".into();
        assert_eq!(
            ids(form.people("@me:example.org", &blocked)),
            ["@mira:example.org"]
        );
        // An empty search lists no one.
        form.query = " ".into();
        assert!(form.people("@me:example.org", &blocked).is_empty());
        assert!(!form.searching());
        form.query = "mi".into();
        assert!(form.searching());
    }
}
