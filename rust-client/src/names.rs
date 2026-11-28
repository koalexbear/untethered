//! Names (ROADMAP M12): per-server nicknames and role badges, and per-server
//! pictures (M15).
//!
//! Matrix has no per-space profile. A server nickname is this account's
//! display name, and a server picture its avatar, in each room of the space:
//! the space itself, its joined subspaces and channels (`m.room.member`,
//! engine/names.rs), so Element and other clients show them in those rooms
//! only. A channel joined after they were set gets them too, while it still
//! shows the global name or picture it joined with. Clearing them puts the
//! global name or picture back everywhere in the space.
//!
//! Badges stand in for Discord's role colours: Matrix's own named power
//! levels, from the room's power levels (Admin at 100, Moderator at 50, the
//! creators of a version 12 room above every level), and the custom roles
//! its server names (ROADMAP C2, roles.rs), shown wherever the person's
//! name is.
use crate::{
    App, Msg, Request, Section,
    engine::{
        Command,
        names::{Change, Own, Picture},
    },
    media::LocalFile,
    rooms::{Room, Rooms},
};
use iced::{Task, widget::image::Handle, widget::operation};
use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

pub const FIELD: &str = "server-nickname";
/// The file picker's answer for a server picture (`Msg::Files`).
pub const PICKER: &str = "server-picture:";
/// Discord's limit on a nickname.
pub const LENGTH: usize = 32;
/// A version 12 room's creators outrank every power level (permissions.rs).
const CREATOR: i64 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    Moderator,
    /// A level the room's server names (roles.rs).
    Custom {
        name: crate::roles::RoleName,
        color: u32,
    },
}

impl Role {
    pub fn label(&self) -> &str {
        match self {
            Self::Owner => "Owner",
            Self::Admin => "Admin",
            Self::Moderator => "Moderator",
            Self::Custom { name, .. } => name.as_str(),
        }
    }
    /// The badge's text.
    pub fn tag(&self) -> String {
        match self {
            Self::Owner => "OWNER".into(),
            Self::Admin => "ADMIN".into(),
            Self::Moderator => "MOD".into(),
            Self::Custom { name, .. } => name.as_str().to_uppercase(),
        }
    }
}

/// [`badge`] without custom roles.
#[cfg(test)]
pub fn role(level: i64, everyone: i64) -> Option<Role> {
    badge(level, everyone, None)
}

/// What a power level makes someone, as Element names levels: only above
/// what every member has, so a room where everyone moderates shows no one.
/// The server's custom roles name the levels between: the highest named
/// level someone reaches, custom or Matrix's own, is theirs.
pub fn badge(level: i64, everyone: i64, titles: Option<&crate::roles::Titles>) -> Option<Role> {
    if level >= CREATOR {
        return Some(Role::Owner);
    } else if level <= everyone {
        return None;
    }
    let named = if level >= 100 {
        Some((100, Role::Admin))
    } else if level >= 50 {
        Some((50, Role::Moderator))
    } else {
        None
    };
    let custom = titles
        .and_then(|t| t.reached(level))
        .filter(|t| t.level > everyone);
    match (custom, named) {
        (Some(t), named) if named.is_none_or(|(at, _)| t.level > at) => Some(Role::Custom {
            name: crate::roles::RoleName::new(&t.name),
            color: t.color,
        }),
        (_, named) => named.map(|(_, role)| role),
    }
}

impl Room {
    /// The badge `user` has here, from the room's power levels.
    pub fn role(&self, user: &str) -> Option<Role> {
        // `summary` starts with users_default, what every member has.
        badge(
            self.posting.actor(user)?,
            self.posting.summary()?.0,
            self.titles.as_deref(),
        )
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
    /// A new server picture to save, shown in the form until then.
    pub picture: Option<Arc<Picked>>,
    /// The file picker is open, as this request.
    pub picking: Option<i64>,
    pub problem: Option<&'static str>,
}

/// A picked picture's bytes, and how it looks once cut round.
pub struct Picked {
    pub bytes: Vec<u8>,
    pub preview: Option<Handle>,
}

impl std::fmt::Debug for Picked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picked({} bytes)", self.bytes.len())
    }
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
    /// Choose a server picture with the file picker.
    PickPicture,
    Picked(i64, Result<Option<Arc<Picked>>, &'static str>),
    /// Clear the server picture: the global one shows again.
    ResetPicture,
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

    /// This account's picture in a server: its picture in the space, where
    /// that isn't the global one (ROADMAP M15).
    pub fn server_picture(&self, space: &str) -> Option<&str> {
        // The global profile is known once its name is.
        self.global_name()?;
        let r = self.live.list.get(space).filter(|r| r.space)?;
        let global = self.own_avatar.as_deref();
        r.me.as_ref()?
            .avatar
            .as_deref()
            .filter(|avatar| Some(*avatar) != global)
    }

    /// The picture this account shows in a room: its server picture there,
    /// or the global one.
    pub fn own_picture<'a>(&'a self, r: &'a Room) -> Option<&'a str> {
        r.me.as_ref()
            .and_then(|me| me.avatar.as_deref())
            .or(self.own_avatar.as_deref())
    }

    /// Someone's picture (mxc URI) in a room, as its members see it.
    pub fn member_avatar<'a>(&'a self, r: &'a Room, user: &str) -> Option<&'a str> {
        if user == self.matrix_id {
            self.own_picture(r)
        } else {
            r.avatars.get(user).map(String::as_str)
        }
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
                    picture: None,
                    picking: None,
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
                    let change = Change {
                        name: (name.as_deref() != self.nickname(&form.space)).then_some(name),
                        avatar: form
                            .picture
                            .as_ref()
                            .map(|p| Some(Picture::Upload(p.bytes.clone()))),
                    };
                    let space = form.space.clone();
                    self.save_server_profile(space, change);
                }
            }
            Input::Reset => {
                if let Some(form) = &self.names.form {
                    let space = form.space.clone();
                    let change = Change {
                        name: Some(None),
                        ..Change::default()
                    };
                    self.save_server_profile(space, change);
                }
            }
            Input::ResetPicture => {
                if let Some(form) = &self.names.form {
                    let space = form.space.clone();
                    let change = Change {
                        avatar: Some(None),
                        ..Change::default()
                    };
                    self.save_server_profile(space, change);
                }
            }
            Input::PickPicture if self.connected && !self.nickname_saving() => {
                if let Some(form) = &mut self.names.form {
                    self.seq += 1;
                    form.picking = Some(self.seq);
                    form.problem = None;
                    let token = format!("{PICKER}{}", self.seq);
                    return crate::profile_fields::pick_picture(token, "Choose a server picture");
                }
            }
            Input::Picked(id, result) => {
                if let Some(form) = &mut self.names.form
                    && form.picking == Some(id)
                {
                    form.picking = None;
                    match result {
                        Ok(Some(picked)) => form.picture = Some(picked),
                        Ok(None) => {}
                        Err(problem) => form.problem = Some(problem),
                    }
                }
            }
            Input::Close if !self.nickname_saving() => self.names.form = None,
            Input::Edit(_) | Input::Close | Input::PickPicture => {}
        }
        Task::none()
    }

    /// The file picker's answer for a server picture: read and checked off
    /// the UI thread, with a round preview.
    pub fn server_picture_picked(&mut self, id: i64, file: Option<LocalFile>) -> Task<Msg> {
        if !self
            .names
            .form
            .as_ref()
            .is_some_and(|f| f.picking == Some(id))
        {
            return Task::none();
        }
        let Some(file) = file else {
            return self.names(Input::Picked(id, Ok(None)));
        };
        Task::perform(crate::profile_fields::read_picture(file, true), move |r| {
            Msg::Names(Input::Picked(id, r.map(Some)))
        })
    }

    fn save_server_profile(&mut self, space: String, change: Change) {
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
        if change == Change::default() {
            return;
        }
        if let Some(form) = &mut self.names.form {
            form.problem = None;
        }
        self.command(
            move |id| Command::SetServerProfile { id, rooms, change },
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
                        Some("Couldn't change your server profile in every channel. Try again.");
                }
            }
            // Tried once; the room keeps the global name it joined with.
            Write::Follow(_) => {}
        }
    }

    /// Give rooms joined after their server's nickname or picture was set
    /// that profile, as Discord does for a server's new channels. Only while
    /// a room still shows the global name or picture it joined with: one set
    /// there since, here or in another client, is left alone.
    pub fn follow_server_profiles(&mut self) {
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
            if me.joined == Some(false) || self.names.followed.contains(id) {
                continue;
            }
            let name = (shown(me) == global)
                .then(|| self.inherited(id, me.since, App::nickname))
                .flatten();
            let avatar = (me.avatar.as_deref() == self.own_avatar.as_deref())
                .then(|| self.inherited(id, me.since, App::server_picture))
                .flatten();
            if name.is_some() || avatar.is_some() {
                todo.push((
                    id.clone(),
                    name.map(str::to_owned),
                    avatar.map(str::to_owned),
                ));
            }
        }
        for (room, name, avatar) in todo {
            self.names.followed.insert(room.clone());
            let target = room.clone();
            self.command(
                move |id| Command::FollowServerProfile {
                    id,
                    room: target,
                    name,
                    avatar,
                },
                Request::Nickname(Write::Follow(room)),
            );
        }
    }

    /// What `field` gives the nearest space above `room` that has it set,
    /// set before `joined`.
    fn inherited(
        &self,
        room: &str,
        joined: i64,
        field: for<'a> fn(&'a App, &str) -> Option<&'a str>,
    ) -> Option<&str> {
        let mut seen = BTreeSet::from([room]);
        let mut todo = VecDeque::from([room]);
        while let Some(id) = todo.pop_front() {
            for space in self.live.parents(id) {
                if !seen.insert(space) {
                    continue;
                }
                let set = self.live.list.get(space).and_then(|r| r.me.as_ref());
                if let Some(value) = field(self, space)
                    && set.is_some_and(|me| me.since < joined)
                {
                    return Some(value);
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
            avatar: None,
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
        // A server's custom roles name the levels between (roles.rs).
        let titles = crate::roles::Titles::parse(&serde_json::json!({"roles": {
            "25": {"name": "Helper", "color": "#e67e22"},
            "75": {"name": "Senior", "color": "#3498db"}
        }}));
        let named = |level, everyone| badge(level, everyone, Some(&titles)).map(|r| r.tag());
        assert_eq!(named(25, 0).as_deref(), Some("HELPER"));
        assert_eq!(named(30, 0).as_deref(), Some("HELPER"));
        assert_eq!(named(60, 0).as_deref(), Some("MOD"));
        assert_eq!(named(80, 0).as_deref(), Some("SENIOR"));
        assert_eq!(named(100, 0).as_deref(), Some("ADMIN"));
        assert_eq!(named(30, 25), None, "a role everyone has names no one");
        assert_eq!(named(10, 0), None);
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
        let Ok(Command::SetServerProfile {
            id,
            mut rooms,
            change,
        }) = rx.try_recv()
        else {
            panic!("expected a nickname");
        };
        rooms.sort_unstable();
        assert_eq!(
            rooms,
            ["!deep:x", "!general:x", "!new:x", "!space:x", "!sub:x"]
        );
        assert_eq!(change.name, Some(Some("Captain".into())));
        assert_eq!(change.avatar, None, "the picture stays");
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
        let Ok(Command::SetServerProfile { id, change, .. }) = rx.try_recv() else {
            panic!("expected a reset");
        };
        assert_eq!(change.name, Some(None));
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
        app.follow_server_profiles();
        let Ok(Command::FollowServerProfile {
            room, name, avatar, ..
        }) = rx.try_recv()
        else {
            panic!("expected the new channel to follow");
        };
        assert_eq!(
            (room.as_str(), name.as_deref()),
            ("!new:x", Some("Captain"))
        );
        assert_eq!(avatar, None);
        // Rooms joined before it, and rooms in no space, are left alone.
        assert!(rx.try_recv().is_err());
        app.follow_server_profiles();
        assert!(rx.try_recv().is_err(), "once per room");

        // A member event the store can't place goes to the engine, which
        // checks the event it replaced.
        let (mut app, rx) = super::tests::app();
        let mut unknown = me("Me", 9, true);
        unknown.as_mut().unwrap().joined = None;
        app.live
            .apply_engine(list(me("Captain", 5, false), unknown));
        app.follow_server_profiles();
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::FollowServerProfile { room, .. }) if room == "!new:x"
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
            app.follow_server_profiles();
            assert!(rx.try_recv().is_err());
        }
        // A subspace's own nickname is nearer than the server's.
        let (mut app, rx) = app();
        let mut rooms = list(me("Captain", 5, false), None);
        rooms.rooms[1].me = me("Sub name", 6, false);
        rooms.rooms[3].me = me("Me", 9, true);
        app.live.apply_engine(rooms);
        app.follow_server_profiles();
        let Ok(Command::FollowServerProfile { room, name, .. }) = rx.try_recv() else {
            panic!("expected the deep channel to follow");
        };
        assert_eq!(
            (room.as_str(), name.as_deref()),
            ("!deep:x", Some("Sub name"))
        );
    }

    fn pictured(name: &str, avatar: Option<&str>, since: i64, joined: bool) -> Option<Own> {
        Some(Own {
            name: Some(name.into()),
            avatar: avatar.map(Into::into),
            since,
            joined: Some(joined),
        })
    }

    fn picked() -> Arc<Picked> {
        Arc::new(Picked {
            bytes: vec![1, 2, 3],
            preview: None,
        })
    }

    #[test]
    fn the_server_picture_is_the_space_picture_unless_it_is_the_global_one() {
        let (mut app, _) = app();
        app.own_avatar = Some("mxc://h/global".into());
        app.live
            .apply_engine(list(pictured("Me", Some("mxc://h/global"), 1, true), None));
        assert_eq!(app.server_picture("!space:x"), None);
        app.live.apply_engine(list(
            pictured("Me", Some("mxc://h/server"), 5, false),
            pictured("Me", Some("mxc://h/global"), 1, true),
        ));
        assert_eq!(app.server_picture("!space:x"), Some("mxc://h/server"));
        assert_eq!(app.server_picture("!general:x"), None, "not a server");
        // Where members see it: the space's picture there, the global one
        // in a room with no picture of its own, someone else's from the room.
        let space = &app.live.list["!space:x"];
        assert_eq!(
            app.member_avatar(space, "@me:example.org"),
            Some("mxc://h/server")
        );
        let general = &app.live.list["!general:x"];
        assert_eq!(
            app.member_avatar(general, "@me:example.org"),
            Some("mxc://h/global")
        );
        assert_eq!(app.member_avatar(general, "@mira:x"), None);
    }

    #[test]
    fn a_picked_picture_saves_with_the_name_in_one_change() {
        let (mut app, rx) = app();
        app.live
            .apply_engine(list(me("Captain", 5, false), me("Me", 1, true)));
        let _ = app.names(Input::Open("!space:x".into()));
        let _ = app.names(Input::PickPicture);
        let picking = app.names.form.as_ref().unwrap().picking.unwrap();
        // A stale picker's answer is ignored.
        let _ = app.names(Input::Picked(picking + 1, Ok(Some(picked()))));
        assert!(app.names.form.as_ref().unwrap().picture.is_none());
        let _ = app.names(Input::Picked(picking, Ok(Some(picked()))));
        assert!(app.names.form.as_ref().unwrap().picture.is_some());
        let _ = app.names(Input::Save);
        let Ok(Command::SetServerProfile { change, .. }) = rx.try_recv() else {
            panic!("expected the picture to save");
        };
        // The nickname didn't change, so only the picture is written.
        assert_eq!(change.name, None);
        assert_eq!(change.avatar, Some(Some(Picture::Upload(vec![1, 2, 3]))));
    }

    #[test]
    fn reset_picture_puts_the_global_one_back_and_keeps_the_name() {
        let (mut app, rx) = app();
        app.own_avatar = Some("mxc://h/global".into());
        app.live.apply_engine(list(
            pictured("Captain", Some("mxc://h/server"), 5, false),
            None,
        ));
        let _ = app.names(Input::Open("!space:x".into()));
        let _ = app.names(Input::ResetPicture);
        let Ok(Command::SetServerProfile {
            id,
            mut rooms,
            change,
        }) = rx.try_recv()
        else {
            panic!("expected a picture reset");
        };
        rooms.sort_unstable();
        assert_eq!(
            rooms,
            ["!deep:x", "!general:x", "!new:x", "!space:x", "!sub:x"]
        );
        assert_eq!(change.avatar, Some(None));
        assert_eq!(change.name, None);
        app.engine_event(crate::engine::Event::Done { id, result: Ok(()) });
        assert!(app.names.form.is_none());
        // Nothing to save: no write.
        let _ = app.names(Input::Open("!space:x".into()));
        let _ = app.names(Input::Edit("Captain".into()));
        let _ = app.names(Input::Save);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_channel_joined_after_the_server_picture_gets_it() {
        let (mut app, rx) = app();
        app.own_avatar = Some("mxc://h/global".into());
        // No nickname, a server picture set at 5; joined at 9 with the
        // global picture.
        app.live.apply_engine(list(
            pictured("Me", Some("mxc://h/server"), 5, false),
            pictured("Me", Some("mxc://h/global"), 9, true),
        ));
        app.follow_server_profiles();
        let Ok(Command::FollowServerProfile {
            room, name, avatar, ..
        }) = rx.try_recv()
        else {
            panic!("expected the new channel to take the picture");
        };
        assert_eq!(room, "!new:x");
        assert_eq!(name, None, "no nickname to take");
        assert_eq!(avatar.as_deref(), Some("mxc://h/server"));
        assert!(rx.try_recv().is_err(), "rooms joined earlier keep theirs");

        // A picture set there since joining stays.
        let (mut app, rx) = super::tests::app();
        app.own_avatar = Some("mxc://h/global".into());
        app.live.apply_engine(list(
            pictured("Me", Some("mxc://h/server"), 5, false),
            pictured("Me", Some("mxc://h/mine"), 9, true),
        ));
        app.follow_server_profiles();
        assert!(rx.try_recv().is_err());
    }
}
