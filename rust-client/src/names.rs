//! Names (ROADMAP M12): per-server nicknames and role badges.
//!
//! Matrix has no per-space name. A server nickname is this account's display
//! name in each room of the space: the space itself, its joined subspaces
//! and channels (`m.room.member`, engine/names.rs), so Element and other
//! clients show it in those rooms only. A channel joined after the nickname
//! was set gets it too, while it still shows the global name it joined with.
//! Clearing the nickname puts the global name back everywhere in the space.
//!
//! Badges stand in for Discord's role colours: Matrix's own named power
//! levels, from the room's power levels (Admin at 100, Moderator at 50, the
//! creators of a version 12 room above every level), shown wherever the
//! person's name is.
use crate::{
    App, Msg, Request, Section,
    engine::{Command, names::Own},
    rooms::{Room, Rooms},
};
use iced::{Task, widget::operation};
use std::collections::{BTreeSet, VecDeque};

pub const FIELD: &str = "server-nickname";
/// Discord's limit on a nickname.
pub const LENGTH: usize = 32;
/// A version 12 room's creators outrank every power level (permissions.rs).
const CREATOR: i64 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    Moderator,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Self::Owner => "Owner",
            Self::Admin => "Admin",
            Self::Moderator => "Moderator",
        }
    }
    /// The badge's text.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Owner => "OWNER",
            Self::Admin => "ADMIN",
            Self::Moderator => "MOD",
        }
    }
}

/// What a power level makes someone, as Element names levels: only above
/// what every member has, so a room where everyone moderates shows no one.
pub fn role(level: i64, everyone: i64) -> Option<Role> {
    if level >= CREATOR {
        Some(Role::Owner)
    } else if level <= everyone {
        None
    } else if level >= 100 {
        Some(Role::Admin)
    } else if level >= 50 {
        Some(Role::Moderator)
    } else {
        None
    }
}

impl Room {
    /// The badge `user` has here, from the room's power levels.
    pub fn role(&self, user: &str) -> Option<Role> {
        // `summary` starts with users_default, what every member has.
        role(self.posting.actor(user)?, self.posting.summary()?.0)
    }
}

impl Rooms {
    /// A server's rooms: the space, its joined subspaces and channels, each
    /// upgraded room as its newest joined one.
    pub fn server(&self, space: &str) -> Vec<&str> {
        let mut rooms = Vec::new();
        let Some((id, r)) = self.list.get_key_value(space) else {
            return rooms;
        };
        if !r.space {
            return rooms;
        }
        let mut seen = BTreeSet::from([id.as_str()]);
        let mut todo = VecDeque::from([id.as_str()]);
        while let Some(id) = todo.pop_front() {
            rooms.push(id);
            for child in self.children.get(id).into_iter().flatten() {
                let child = self.newest(child);
                if !seen.insert(child) {
                    continue;
                }
                match self.list.get(child) {
                    Some(r) if r.space => todo.push_back(child),
                    Some(_) => rooms.push(child),
                    None => {}
                }
            }
        }
        rooms
    }

    /// The joined spaces that list `room` as a child.
    fn parents<'a>(&'a self, room: &'a str) -> impl Iterator<Item = &'a str> {
        self.children
            .iter()
            .filter(move |(space, children)| {
                self.list.get(*space).is_some_and(|r| r.space)
                    && children.iter().any(|c| self.newest(c) == room)
            })
            .map(|(space, _)| space.as_str())
    }
}

/// Edit Server Profile, for one space.
pub struct Form {
    pub space: String,
    pub input: String,
    pub problem: Option<&'static str>,
}

#[derive(Default)]
pub struct State {
    pub form: Option<Form>,
    /// Rooms given their server's nickname after joining, this session:
    /// each is tried once.
    followed: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub enum Input {
    /// Edit Server Profile for this space.
    Open(String),
    Edit(String),
    Save,
    /// Clear the nickname: the global name shows again.
    Reset,
    Close,
}

/// A nickname write in flight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    /// From Edit Server Profile.
    Server(String),
    /// A room joined after its server's nickname was set.
    Follow(String),
}

/// The name shown in a room, as this account's member event has it.
fn shown(me: &Own) -> Option<&str> {
    me.name.as_deref()
}

impl App {
    /// This account's global name, once known: None inside when it has none.
    fn global_name(&self) -> Option<Option<&str>> {
        if self.profile.is_empty() {
            None
        } else if self.profile == self.matrix_id {
            Some(None)
        } else {
            Some(Some(&self.profile))
        }
    }

    /// This account's nickname in a server: its name in the space, where
    /// that isn't the global name.
    pub fn nickname(&self, space: &str) -> Option<&str> {
        let global = self.global_name()?;
        let r = self.live.list.get(space).filter(|r| r.space)?;
        shown(r.me.as_ref()?).filter(|name| Some(*name) != global)
    }

    /// The server a room belongs to: the open one if it's there, or else
    /// the first joined space that has it. None for rooms in no space.
    pub fn server_of(&self, room: &str) -> Option<String> {
        if let Section::Space(space) = &self.section
            && self.live.server(space).contains(&room)
        {
            return Some(space.clone());
        }
        self.live
            .list
            .iter()
            .filter(|(_, r)| r.space)
            .find(|(space, _)| self.live.server(space).contains(&room))
            .map(|(space, _)| space.clone())
    }

    /// Edit Server Profile is offered: the matrix-sdk engine, signed in and
    /// verified (the rail and menus need no more).
    pub fn may_edit_server_profile(&self) -> bool {
        self.logged_in && self.verified && self.sdk.is_some()
    }

    pub fn nickname_saving(&self) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Request::Nickname(Write::Server(_))))
    }

    pub fn names(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Open(space) => {
                if !self.may_edit_server_profile()
                    || !self.live.list.get(&space).is_some_and(|r| r.space)
                {
                    return Task::none();
                }
                let input = self.nickname(&space).unwrap_or_default().to_owned();
                self.names.form = Some(Form {
                    space,
                    input,
                    problem: None,
                });
                self.server_menu = false;
                self.sdk_ui.profile.dismiss_card();
                return operation::focus(FIELD);
            }
            Input::Edit(text) if !self.nickname_saving() => {
                if let Some(form) = &mut self.names.form {
                    form.input = text.chars().take(LENGTH).collect();
                    form.problem = None;
                }
            }
            Input::Save => {
                if let Some(form) = &self.names.form {
                    let name = Some(form.input.trim().to_owned()).filter(|n| !n.is_empty());
                    let space = form.space.clone();
                    self.save_nickname(space, name);
                }
            }
            Input::Reset => {
                if let Some(form) = &self.names.form {
                    let space = form.space.clone();
                    self.save_nickname(space, None);
                }
            }
            Input::Close if !self.nickname_saving() => self.names.form = None,
            Input::Edit(_) | Input::Close => {}
        }
        Task::none()
    }

    fn save_nickname(&mut self, space: String, name: Option<String>) {
        if self.nickname_saving() || !self.connected {
            return;
        }
        let rooms: Vec<String> = self
            .live
            .server(&space)
            .into_iter()
            .map(str::to_owned)
            .collect();
        if rooms.is_empty() {
            return;
        }
        if let Some(form) = &mut self.names.form {
            form.problem = None;
        }
        self.command(
            move |id| Command::SetNickname { id, rooms, name },
            Request::Nickname(Write::Server(space)),
        );
    }

    pub fn nickname_done(&mut self, write: Write, ok: bool) {
        match write {
            Write::Server(space) => {
                let Some(form) = self.names.form.as_mut().filter(|f| f.space == space) else {
                    return;
                };
                if ok {
                    self.names.form = None;
                } else {
                    form.problem =
                        Some("Couldn't change your nickname in every channel. Try again.");
                }
            }
            // Tried once; the room keeps the global name it joined with.
            Write::Follow(_) => {}
        }
    }

    /// Give rooms joined after their server's nickname was set that
    /// nickname, as Discord does for a server's new channels. Only while a
    /// room still shows the global name it joined with: a name set there
    /// since, here or in another client, is left alone.
    pub fn follow_nicknames(&mut self) {
        if !self.logged_in || !self.verified || !self.connected || self.sdk.is_none() {
            return;
        }
        let Some(global) = self.global_name() else {
            return;
        };
        let mut todo = Vec::new();
        for (id, r) in &self.live.list {
            let Some(me) = &r.me else {
                continue;
            };
            // Whether a member event the store can't place is the join, the
            // engine finds out.
            if me.joined == Some(false) || shown(me) != global || self.names.followed.contains(id) {
                continue;
            }
            if let Some(name) = self.inherited(id, me.since) {
                todo.push((id.clone(), name.to_owned()));
            }
        }
        for (room, name) in todo {
            self.names.followed.insert(room.clone());
            let target = room.clone();
            self.command(
                move |id| Command::FollowNickname {
                    id,
                    room: target,
                    name,
                },
                Request::Nickname(Write::Follow(room)),
            );
        }
    }

    /// The nickname of the nearest space above `room` that has one, set
    /// before `joined`.
    fn inherited(&self, room: &str, joined: i64) -> Option<&str> {
        let mut seen = BTreeSet::from([room]);
        let mut todo = VecDeque::from([room]);
        while let Some(id) = todo.pop_front() {
            for space in self.live.parents(id) {
                if !seen.insert(space) {
                    continue;
                }
                let set = self.live.list.get(space).and_then(|r| r.me.as_ref());
                if let Some(name) = self.nickname(space)
                    && set.is_some_and(|me| me.since < joined)
                {
                    return Some(name);
                }
                todo.push_back(space);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{RoomList, RoomSummary};
    use std::collections::BTreeMap;

    fn app() -> (App, async_channel::Receiver<Command>) {
        let (backend, _) = async_channel::bounded(1);
        let (sdk, rx) = async_channel::bounded(16);
        let mut app = App::new(backend);
        app.sdk = Some(sdk);
        app.logged_in = true;
        app.connected = true;
        app.verified = true;
        app.matrix_id = "@me:example.org".into();
        app.profile = "Me".into();
        (app, rx)
    }

    fn me(name: &str, since: i64, joined: bool) -> Option<Own> {
        Some(Own {
            name: Some(name.into()),
            since,
            joined: Some(joined),
        })
    }

    fn room(id: &str, space: bool, me: Option<Own>) -> RoomSummary {
        RoomSummary {
            id: id.into(),
            name: id.into(),
            space,
            me,
            ..RoomSummary::default()
        }
    }

    /// A server with a subspace, three channels and an unrelated room.
    fn list(space_me: Option<Own>, new_me: Option<Own>) -> RoomList {
        RoomList {
            rooms: vec![
                room("!space:x", true, space_me),
                room("!sub:x", true, me("Me", 1, true)),
                room("!general:x", false, me("Me", 1, true)),
                room("!deep:x", false, me("Me", 1, true)),
                room("!new:x", false, new_me),
                room("!elsewhere:x", false, me("Me", 1, true)),
            ],
            children: BTreeMap::from([
                (
                    "!space:x".into(),
                    BTreeSet::from(["!sub:x".into(), "!general:x".into(), "!new:x".into()]),
                ),
                ("!sub:x".into(), BTreeSet::from(["!deep:x".into()])),
            ]),
            ..RoomList::default()
        }
    }

    #[test]
    fn roles_follow_matrix_named_levels_above_everyone() {
        assert_eq!(role(100, 0), Some(Role::Admin));
        assert_eq!(role(75, 0), Some(Role::Moderator));
        assert_eq!(role(50, 0), Some(Role::Moderator));
        assert_eq!(role(49, 0), None);
        assert_eq!(role(0, 0), None);
        assert_eq!(role(CREATOR, 0), Some(Role::Owner));
        // Where every member moderates, no one stands out.
        assert_eq!(role(50, 50), None);
        assert_eq!(role(100, 50), Some(Role::Admin));
    }

    #[test]
    fn a_room_badge_comes_from_its_own_power_levels() {
        use crate::permissions::StateDoc;
        use serde_json::json;
        let doc = |id: &str, content| StateDoc {
            event_id: id.into(),
            sender: "@creator:x".into(),
            content,
            redacted: false,
        };
        let mut r = Room::default();
        r.posting.store_snapshot(
            Some(doc("$c", json!({"room_version": "11"}))),
            Some(doc(
                "$p",
                json!({"users": {"@admin:x": 100, "@mod:x": 50}, "users_default": 0}),
            )),
        );
        assert_eq!(r.role("@admin:x"), Some(Role::Admin));
        assert_eq!(r.role("@mod:x"), Some(Role::Moderator));
        assert_eq!(r.role("@someone:x"), None);
        r.posting.store_snapshot(
            Some(doc("$c12", json!({"room_version": "12"}))),
            Some(doc("$p2", json!({"users": {"@mod:x": 50}}))),
        );
        assert_eq!(r.role("@creator:x"), Some(Role::Owner));
        assert_eq!(r.role("@admin:x"), None);
    }

    #[test]
    fn a_server_is_its_space_subspaces_and_channels() {
        let (mut app, _) = app();
        app.live
            .apply_engine(list(me("Me", 1, true), me("Me", 1, true)));
        let mut server = app.live.server("!space:x");
        server.sort_unstable();
        assert_eq!(
            server,
            ["!deep:x", "!general:x", "!new:x", "!space:x", "!sub:x"]
        );
        assert!(app.live.server("!general:x").is_empty());
        assert_eq!(app.server_of("!deep:x").as_deref(), Some("!space:x"));
        assert_eq!(app.server_of("!elsewhere:x"), None);
        app.section = Section::Space("!sub:x".into());
        assert_eq!(app.server_of("!deep:x").as_deref(), Some("!sub:x"));
    }

    #[test]
    fn saving_sets_the_name_in_every_room_of_the_server_and_reset_clears_it() {
        let (mut app, rx) = app();
        app.live
            .apply_engine(list(me("Me", 1, true), me("Me", 1, true)));
        let _ = app.names(Input::Open("!space:x".into()));
        assert_eq!(app.names.form.as_ref().unwrap().input, "");
        let _ = app.names(Input::Edit(" Captain ".into()));
        let _ = app.names(Input::Save);
        let Ok(Command::SetNickname {
            id,
            mut rooms,
            name,
        }) = rx.try_recv()
        else {
            panic!("expected a nickname");
        };
        rooms.sort_unstable();
        assert_eq!(
            rooms,
            ["!deep:x", "!general:x", "!new:x", "!space:x", "!sub:x"]
        );
        assert_eq!(name.as_deref(), Some("Captain"));
        // One save at a time, and the form stays until it's done.
        let _ = app.names(Input::Save);
        let _ = app.names(Input::Close);
        assert!(rx.try_recv().is_err());
        assert!(app.names.form.is_some());
        app.engine_event(crate::engine::Event::Done {
            id,
            result: Err(crate::engine::Failure::Failed),
        });
        assert!(app.names.form.as_ref().unwrap().problem.is_some());

        let _ = app.names(Input::Reset);
        let Ok(Command::SetNickname { id, name, .. }) = rx.try_recv() else {
            panic!("expected a reset");
        };
        assert_eq!(name, None);
        app.engine_event(crate::engine::Event::Done { id, result: Ok(()) });
        assert!(app.names.form.is_none());
    }

    #[test]
    fn your_own_name_in_a_server_offers_edit_server_profile() {
        use crate::context_menu::Entry;
        let (mut app, _) = app();
        app.live.apply_engine(list(me("Me", 1, true), None));
        let offers = |app: &App, room: &str, user: &str| {
            app.user_items(room, user)
                .iter()
                .find(|i| i.entry == Entry::EditServerProfile)
                .map(|i| i.msg.is_some())
        };
        assert_eq!(offers(&app, "!deep:x", "@me:example.org"), Some(true));
        assert_eq!(offers(&app, "!elsewhere:x", "@me:example.org"), None);
        assert_eq!(offers(&app, "!deep:x", "@other:example.org"), None);
        app.connected = false;
        assert_eq!(offers(&app, "!deep:x", "@me:example.org"), Some(false));
        // gomuks has no command for it.
        app.sdk = None;
        assert_eq!(offers(&app, "!deep:x", "@me:example.org"), None);
    }

    #[test]
    fn the_nickname_is_the_space_name_unless_it_is_the_global_one() {
        let (mut app, _) = app();
        app.live.apply_engine(list(me("Me", 1, true), None));
        assert_eq!(app.nickname("!space:x"), None);
        app.live.apply_engine(list(me("Captain", 5, false), None));
        assert_eq!(app.nickname("!space:x"), Some("Captain"));
        let _ = app.names(Input::Open("!space:x".into()));
        assert_eq!(app.names.form.as_ref().unwrap().input, "Captain");
        // Not a server.
        assert_eq!(app.nickname("!general:x"), None);
        // Unknown global name: no nickname to tell apart yet.
        app.profile.clear();
        assert_eq!(app.nickname("!space:x"), None);
    }

    #[test]
    fn a_channel_joined_after_the_nickname_gets_it_once() {
        let (mut app, rx) = app();
        // Joined after the nickname was set (5), still with the global name.
        app.live
            .apply_engine(list(me("Captain", 5, false), me("Me", 9, true)));
        app.follow_nicknames();
        let Ok(Command::FollowNickname { room, name, .. }) = rx.try_recv() else {
            panic!("expected the new channel to follow");
        };
        assert_eq!((room.as_str(), name.as_str()), ("!new:x", "Captain"));
        // Rooms joined before it, and rooms in no space, are left alone.
        assert!(rx.try_recv().is_err());
        app.follow_nicknames();
        assert!(rx.try_recv().is_err(), "once per room");

        // A member event the store can't place goes to the engine, which
        // checks the event it replaced.
        let (mut app, rx) = super::tests::app();
        let mut unknown = me("Me", 9, true);
        unknown.as_mut().unwrap().joined = None;
        app.live
            .apply_engine(list(me("Captain", 5, false), unknown));
        app.follow_nicknames();
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::FollowNickname { room, .. }) if room == "!new:x"
        ));
    }

    #[test]
    fn a_channel_named_since_joining_or_joined_earlier_keeps_its_name() {
        for new in [
            me("Me", 3, true),         // joined before the nickname
            me("Me", 9, false),        // set back to the global name since
            me("Other name", 9, true), // joined with another name
        ] {
            let (mut app, rx) = app();
            app.live.apply_engine(list(me("Captain", 5, false), new));
            app.follow_nicknames();
            assert!(rx.try_recv().is_err());
        }
        // A subspace's own nickname is nearer than the server's.
        let (mut app, rx) = app();
        let mut rooms = list(me("Captain", 5, false), None);
        rooms.rooms[1].me = me("Sub name", 6, false);
        rooms.rooms[3].me = me("Me", 9, true);
        app.live.apply_engine(rooms);
        app.follow_nicknames();
        let Ok(Command::FollowNickname { room, name, .. }) = rx.try_recv() else {
            panic!("expected the deep channel to follow");
        };
        assert_eq!((room.as_str(), name.as_str()), ("!deep:x", "Sub name"));
    }
}
