//! Roles and channel permissions (ROADMAP C2): Discord's Roles page on
//! Matrix power levels, as a tab of Server Settings (server_settings.rs).
//!
//! - **Roles** are named power levels: Matrix's own Admin (100) and
//!   Moderator (50), Member for what everyone has (`users_default`), and
//!   custom roles at the levels between, named and coloured in our own
//!   state event on the space ([`ROLES`]). Only Untethered shows those
//!   names; every client enforces the levels. The creators of a version 12
//!   room outrank every level (Owner) and can't be given a role.
//! - **Giving a role** writes the member's level into the power levels of
//!   the space and of each of its rooms this account may change: Matrix
//!   has power levels per room, not per space.
//! - **Permissions** are the levels actions need. The space's own levels
//!   are the server's defaults; a channel whose levels differ has limits of
//!   its own, such as an announcement channel only moderators post in.
//!   Changing a default changes it in the channels that had it.
//!
//! Every change is patched onto the room's current power levels (the
//! engine reads them just before writing, engine/space_settings.rs), so
//! what this page doesn't edit stays: other users, other event types,
//! unknown fields, the posting panel's restore record (permissions.rs).
//! Only what the homeserver's authorization rules allow is offered:
//! nothing above this account's own level, no one at or above it, nothing
//! in a room where it can't send `m.room.power_levels`.
use crate::{
    App, Msg, Request,
    engine::{
        Command, Failure,
        space_settings::{self as engine, Member, Reply},
    },
    permissions::{MAX, POWER, Posting, level},
    server_settings::{Note, Op},
};
use iced::Task;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fmt, sync::Arc};

pub use crate::engine::space_settings::ROLES;

/// Matrix's named power levels, as Element names them.
pub const ADMIN: i64 = 100;
pub const MODERATOR: i64 = 50;
/// Discord's limit on a role's name.
pub const NAME_LIMIT: usize = 32;
pub const NAME_FIELD: &str = "role-name";
pub const LEVEL_FIELD: &str = "role-level";
pub const FILTER_FIELD: &str = "role-members";
/// Role colours to pick from: Discord's, light enough to read as a name on
/// every surface here.
pub const COLORS: [u32; 10] = [
    0x1abc9c, 0x2ecc71, 0x3498db, 0x9b59b6, 0xe91e63, 0xf1c40f, 0xe67e22, 0xe74c3c, 0x95a5a6,
    0x99aab5,
];
/// A custom role saved without a colour we can read.
const PLAIN: u32 = 0x99aab5;
/// Members listed at once; the search finds the rest.
pub const SHOWN: usize = 100;

/// A level a custom role may name: those between Member and Admin, other
/// than Moderator's.
pub fn custom_level(level: i64) -> bool {
    (1..ADMIN).contains(&level) && level != MODERATOR
}

fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.chars().count() <= NAME_LIMIT
}

pub fn hex(color: u32) -> String {
    format!("#{:06x}", color & 0xff_ffff)
}

fn parse_hex(s: &str) -> Option<u32> {
    let digits = s.strip_prefix('#')?;
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

/// A custom role: a name and colour for a power level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Title {
    pub level: i64,
    pub name: String,
    pub color: u32,
}

/// A space's custom roles, highest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Titles(Vec<Title>);

impl Titles {
    /// The roles in a [`ROLES`] event; what isn't a custom role is left out.
    pub fn parse(content: &Value) -> Self {
        let mut out: Vec<Title> = content["roles"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, role)| {
                let level = key.parse::<i64>().ok().filter(|l| custom_level(*l))?;
                let name = role["name"].as_str()?.trim();
                if name.is_empty() {
                    return None;
                }
                Some(Title {
                    level,
                    name: name.chars().take(NAME_LIMIT).collect(),
                    color: role["color"].as_str().and_then(parse_hex).unwrap_or(PLAIN),
                })
            })
            .collect();
        out.sort_by_key(|t| std::cmp::Reverse(t.level));
        Self(out)
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Title> {
        self.0.iter()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn get(&self, level: i64) -> Option<&Title> {
        self.0.iter().find(|t| t.level == level)
    }
    /// The highest custom role `level` reaches.
    pub fn reached(&self, level: i64) -> Option<&Title> {
        self.0.iter().find(|t| t.level <= level)
    }
}

/// A custom role's name where a badge needs a `Copy` value (names.rs).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RoleName {
    len: u8,
    bytes: [u8; 127],
}

impl RoleName {
    pub fn new(name: &str) -> Self {
        let mut end = name.len().min(127);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        let mut bytes = [0; 127];
        bytes[..end].copy_from_slice(&name.as_bytes()[..end]);
        Self {
            len: end as u8,
            bytes,
        }
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
    }
}

impl fmt::Debug for RoleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TitleEdit {
    /// Name a level: a new role, or the one at `from` changed (an edit
    /// may move it to another level).
    Set {
        from: Option<i64>,
        level: i64,
        name: String,
        color: u32,
    },
    Remove {
        level: i64,
    },
}

/// Apply `edit` to a [`ROLES`] event's content (None: there's none yet).
/// Only the role it names changes; unknown fields stay.
pub fn apply_titles(content: Option<Value>, edit: &TitleEdit) -> Result<Value, Failure> {
    let mut content = content
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if !content["roles"].is_object() {
        content["roles"] = json!({});
    }
    let roles = content["roles"].as_object_mut().expect("an object");
    match edit {
        TitleEdit::Set {
            from,
            level,
            name,
            color,
        } => {
            if !custom_level(*level) || !valid_name(name) {
                return Err(Failure::Failed);
            }
            let key = level.to_string();
            let mut role = match from {
                Some(from) => roles
                    .get(&from.to_string())
                    .filter(|r| r.is_object())
                    .cloned()
                    .ok_or(Failure::Failed)?,
                None => json!({}),
            };
            if *from != Some(*level) && roles.contains_key(&key) {
                return Err(Failure::Failed);
            }
            if let Some(from) = from {
                roles.remove(&from.to_string());
            }
            role["name"] = name.trim().into();
            role["color"] = hex(*color).into();
            roles.insert(key, role);
        }
        TitleEdit::Remove { level } => {
            roles.remove(&level.to_string()).ok_or(Failure::Failed)?;
        }
    }
    Ok(content)
}

/// Where a permission's level is kept in the power levels.
enum Slot {
    /// A top-level key, and the spec's default for it.
    Key(&'static str, i64),
    /// An event type, state or not (their defaults differ).
    Event(&'static str, bool),
}

fn slot_level(c: &Value, slot: &Slot) -> Option<i64> {
    match slot {
        Slot::Key(key, default) => level(c, key, *default),
        Slot::Event(kind, state) => {
            let default = if *state {
                level(c, "state_default", 50)?
            } else {
                level(c, "events_default", 0)?
            };
            level(&c["events"], kind, default)
        }
    }
}

/// What a role may do in a room, each kept by the homeserver as a power
/// level and enforced by its authorization rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Perm {
    Post,
    React,
    Pin,
    Redact,
    Invite,
    Kick,
    Ban,
    Manage,
    Roles,
}

impl Perm {
    pub const ALL: [Self; 9] = [
        Self::Post,
        Self::React,
        Self::Pin,
        Self::Redact,
        Self::Invite,
        Self::Kick,
        Self::Ban,
        Self::Manage,
        Self::Roles,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Post => "Send messages",
            Self::React => "Add reactions",
            Self::Pin => "Pin messages",
            Self::Redact => "Delete others' messages",
            Self::Invite => "Invite people",
            Self::Kick => "Kick members",
            Self::Ban => "Ban members",
            Self::Manage => "Manage channel",
            Self::Roles => "Manage roles",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::Post => {
                "Messages, stickers and polls. In an encrypted channel the server can't tell a \
                 reaction from a message, so this covers reactions too."
            }
            Self::React => "Reacting to messages with emoji.",
            Self::Pin => "Pinning and unpinning messages.",
            Self::Redact => "Removing messages other people sent.",
            Self::Invite => "Inviting people to join.",
            Self::Kick => "Removing members below their own role.",
            Self::Ban => "Banning and unbanning members below their own role.",
            Self::Manage => "Changing the name, topic and picture.",
            Self::Roles => "Giving roles and changing these permissions, up to their own role.",
        }
    }
    /// Every slot the permission sets.
    fn slots(self) -> &'static [Slot] {
        match self {
            Self::Post => &[
                Slot::Event("m.room.message", false),
                Slot::Event("m.room.encrypted", false),
                Slot::Event("m.sticker", false),
                Slot::Event("m.poll.start", false),
                Slot::Event("org.matrix.msc3381.poll.start", false),
            ],
            Self::React => &[Slot::Event("m.reaction", false)],
            Self::Pin => &[Slot::Event("m.room.pinned_events", true)],
            Self::Redact => &[Slot::Key("redact", 50)],
            Self::Invite => &[Slot::Key("invite", 0)],
            Self::Kick => &[Slot::Key("kick", 50)],
            Self::Ban => &[Slot::Key("ban", 50)],
            Self::Manage => &[
                Slot::Event("m.room.name", true),
                Slot::Event("m.room.topic", true),
                Slot::Event("m.room.avatar", true),
            ],
            Self::Roles => &[Slot::Event(POWER, true)],
        }
    }
    /// The level the permission needs: what lets someone do all of it.
    /// Posting reads messages, plain and encrypted, as the composer does.
    pub fn level(self, c: &Value) -> Option<i64> {
        let slots = match self {
            Self::Post => &self.slots()[..2],
            _ => self.slots(),
        };
        slots
            .iter()
            .map(|s| slot_level(c, s))
            .try_fold(i64::MIN, |most, l| Some(most.max(l?)))
    }
}

/// A power level change, made the same way in each room it's sent to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// Give `user` a level; None: Member, what everyone has.
    User { user: String, level: Option<i64> },
    /// The level each permission needs.
    Perms(Vec<(Perm, i64)>),
}

/// The room's power levels with `change` made, if `me` may make it there;
/// the same document when there's nothing to change. As the homeserver's
/// authorization rules say: `me` must be allowed to send power levels, and
/// may neither set anything above their own level nor change anyone, or
/// any permission, at or above it.
pub fn patch(posting: &Posting, me: &str, change: &Change) -> Option<Value> {
    let c = posting.content()?;
    let actor = posting.actor(me)?;
    if !posting.can_send_state(me, POWER) {
        return None;
    }
    let mut out = c.clone();
    match change {
        Change::User { user, level } => {
            // Your own role is someone else's to change: losing it can't
            // be undone from here.
            let current = posting.actor(user)?;
            if user == me || current >= actor || current >= MAX {
                return None;
            }
            match *level {
                Some(level) => {
                    if level > actor || level < -MAX {
                        return None;
                    }
                    if current == level {
                        return Some(out);
                    }
                    if !out["users"].is_object() {
                        out["users"] = json!({});
                    }
                    out["users"][user] = level.into();
                }
                None => {
                    if let Some(users) = out["users"].as_object_mut() {
                        users.remove(user);
                    }
                }
            }
        }
        Change::Perms(perms) => {
            for (perm, new) in perms {
                if *new > actor || *new < -MAX {
                    return None;
                }
                for slot in perm.slots() {
                    let old = slot_level(c, slot)?;
                    if old == *new {
                        continue;
                    }
                    if old > actor {
                        return None;
                    }
                    match slot {
                        Slot::Key(key, _) => out[*key] = (*new).into(),
                        Slot::Event(kind, _) => {
                            if !out["events"].is_object() {
                                out["events"] = json!({});
                            }
                            out["events"][*kind] = (*new).into();
                        }
                    }
                }
            }
        }
    }
    Some(out)
}

/// A level's name in a room: Owner for creators, Matrix's named levels,
/// the space's custom roles, Member for what everyone has, or its number.
pub fn level_name(level: i64, everyone: i64, titles: Option<&Titles>) -> String {
    if level >= MAX {
        "Owner".into()
    } else if level == ADMIN {
        "Admin".into()
    } else if level == MODERATOR {
        "Moderator".into()
    } else if let Some(t) = titles.and_then(|t| t.get(level)) {
        t.name.clone()
    } else if level <= everyone {
        "Member".into()
    } else {
        format!("Level {level}")
    }
}

/// One entry of a role picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The level; None for Member when giving a role (the member's entry
    /// is removed, so the room's default applies).
    pub level: Option<i64>,
    pub label: String,
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// One entry of the channel picker: a channel, or None for the server's
/// defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub room: Option<String>,
    pub label: String,
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// Spaces' custom roles, for badges and the Roles tab: asked once a
/// session for the open space, and again whenever its settings load or
/// its roles change here. Empty where a space has none.
#[derive(Default)]
pub struct Cache {
    titles: BTreeMap<String, Arc<Titles>>,
}

/// The Roles tab's state while the page is open.
#[derive(Default)]
pub struct Draft {
    pub view: View,
    pub filter: String,
    /// Whose permissions show: a channel, or None for the server's.
    pub channel: Option<String>,
    /// A role being made or edited.
    pub form: Option<Form>,
}

/// The part of the Roles tab on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Roles,
    Members,
    Permissions,
}

impl View {
    pub const ALL: [Self; 3] = [Self::Roles, Self::Members, Self::Permissions];
    pub fn label(self) -> &'static str {
        match self {
            Self::Roles => "Roles",
            Self::Members => "Members",
            Self::Permissions => "Permissions",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    /// The custom role being edited, by level; None for a new one.
    pub from: Option<i64>,
    pub name: String,
    pub level: String,
    pub color: u32,
}

#[derive(Clone, Debug)]
pub enum Input {
    View(View),
    Filter(String),
    /// Give a member a role: its level, or None for Member.
    Assign(String, Option<i64>),
    /// Show a channel's permissions, or None for the server's defaults.
    Channel(Option<String>),
    /// What a permission needs in a room: the space for the server's
    /// default.
    Perm(String, Perm, i64),
    /// Give a channel the server's defaults back.
    Sync(String),
    /// Make a role (None), or edit the custom role at this level.
    Edit(Option<i64>),
    Name(String),
    Level(String),
    Color(u32),
    Save,
    Cancel,
    Remove(i64),
}

/// A roles change in flight, for its note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    /// `who` given `role`; `skipped` rooms this account can't change.
    Assign {
        who: String,
        role: String,
        skipped: usize,
    },
    /// `what` set; `skipped` rooms this account can't change. `rooms`:
    /// say in how many rooms (a server default).
    Perms {
        what: String,
        skipped: usize,
        rooms: bool,
    },
    Title(TitleEdit),
}

impl Write {
    fn note(&self, written: usize, failed: usize) -> Note {
        let rooms = |n: usize| {
            if n == 1 {
                "1 room".into()
            } else {
                format!("{n} rooms")
            }
        };
        let mut text = match self {
            Self::Assign { who, role, .. } if written > 0 => {
                format!("{who} is now {role} in {}.", rooms(written))
            }
            Self::Assign { who, role, .. } => format!("{who} was already {role}."),
            Self::Perms {
                what, rooms: true, ..
            } if written > 0 => format!("{what}, in {}.", rooms(written)),
            Self::Perms { what, .. } if written > 0 => format!("{what}."),
            Self::Perms { what, .. } => format!("{what} already."),
            Self::Title(TitleEdit::Set { name, .. }) => format!("Role {} saved.", name.trim()),
            Self::Title(TitleEdit::Remove { .. }) => {
                "Role removed. Its members keep their power level.".into()
            }
        };
        if let Self::Assign { skipped, .. } | Self::Perms { skipped, .. } = self
            && *skipped > 0
        {
            text.push_str(&format!(
                " {} you can't change kept {}.",
                if *skipped == 1 {
                    "One room".into()
                } else {
                    format!("{skipped} rooms")
                },
                if *skipped == 1 {
                    "its levels"
                } else {
                    "theirs"
                }
            ));
        }
        if failed > 0 {
            text.push_str(&format!(" Couldn't change {}.", rooms(failed)));
        }
        Note {
            text,
            problem: failed > 0,
        }
    }
}

fn input(input: Input) -> Msg {
    Msg::ServerSettings(crate::server_settings::Input::Roles(input))
}

impl App {
    fn posting(&self, room: &str) -> Option<&Posting> {
        self.live.list.get(room).map(|r| &r.posting)
    }

    /// This account's power level in a room.
    pub fn my_level(&self, room: &str) -> Option<i64> {
        self.posting(room)?.actor(&self.matrix_id)
    }

    /// Someone's power level in a room.
    pub fn level_in(&self, room: &str, user: &str) -> Option<i64> {
        self.posting(room)?.actor(user)
    }

    /// What everyone has in a room (`users_default`).
    pub fn everyone_in(&self, room: &str) -> i64 {
        self.posting(room)
            .and_then(|p| p.summary())
            .map_or(0, |(everyone, ..)| everyone)
    }

    /// A space's custom roles, once known.
    pub fn titles(&self, space: &str) -> Option<&Titles> {
        self.server_settings.roles.titles.get(space).map(|t| &**t)
    }

    /// A level's name in a room of `space`.
    pub fn level_label(&self, space: &str, room: &str, level: i64) -> String {
        level_name(level, self.everyone_in(room), self.titles(space))
    }

    /// This account may change power levels in `room` now.
    pub fn may_roles(&self, room: &str) -> bool {
        self.connected
            && self
                .posting(room)
                .is_some_and(|p| p.can_send_state(&self.matrix_id, POWER))
    }

    /// This account may give `user` a role in the space.
    pub fn may_assign(&self, space: &str, user: &str) -> bool {
        let change = Change::User {
            user: user.to_owned(),
            level: None,
        };
        self.connected
            && self
                .posting(space)
                .is_some_and(|p| patch(p, &self.matrix_id, &change).is_some())
    }

    /// This account may name roles in the space at `level`.
    pub fn may_title(&self, space: &str, level: i64) -> bool {
        self.may_set(space, ROLES) && self.my_level(space).is_some_and(|me| level <= me)
    }

    /// The roles this account may give in a room, lowest first: Member,
    /// the space's custom roles, Moderator and Admin, up to its own level.
    /// For a permission, Member is the room's `users_default`.
    pub fn role_choices(&self, space: &str, room: &str, permission: bool) -> Vec<Choice> {
        let Some(me) = self.my_level(room) else {
            return Vec::new();
        };
        let everyone = self.everyone_in(room);
        let mut out = vec![Choice {
            level: permission.then_some(everyone),
            label: "Member".into(),
        }];
        let mut levels: Vec<i64> = self
            .titles(space)
            .into_iter()
            .flat_map(Titles::iter)
            .map(|t| t.level)
            .chain([MODERATOR, ADMIN])
            .filter(|l| *l > everyone && *l <= me)
            .collect();
        levels.sort_unstable();
        levels.dedup();
        out.extend(levels.into_iter().map(|l| Choice {
            level: Some(l),
            label: self.level_label(space, room, l),
        }));
        out
    }

    /// A permission's level in a room.
    pub fn perm_level(&self, room: &str, perm: Perm) -> Option<i64> {
        perm.level(self.posting(room)?.content()?)
    }

    /// The permissions a channel has its own levels for.
    pub fn channel_limits(&self, space: &str, room: &str) -> Vec<Perm> {
        Perm::ALL
            .into_iter()
            .filter(|p| {
                let (here, server) = (self.perm_level(room, *p), self.perm_level(space, *p));
                here.is_some() && server.is_some() && here != server
            })
            .collect()
    }

    /// The space's channels for the permissions picker, by name (not its
    /// subspaces: those are categories here).
    pub fn role_channels(&self, space: &str) -> Vec<(&str, &crate::rooms::Room)> {
        let mut out: Vec<_> = self
            .live
            .server(space)
            .into_iter()
            .filter_map(|id| Some((id, self.live.list.get(id)?)))
            .filter(|(_, r)| !r.space)
            .collect();
        out.sort_by_cached_key(|(id, r)| (r.name.to_lowercase(), id.to_string()));
        out
    }

    /// The space's members as the Roles tab lists them: matching the
    /// search, highest level first, then by name.
    pub fn role_members(&self) -> Vec<&Member> {
        let Some(page) = &self.server_settings.page else {
            return Vec::new();
        };
        let Some(snap) = &page.snapshot else {
            return Vec::new();
        };
        let filter = page.roles.filter.trim().to_lowercase();
        let mut out: Vec<(i64, String, &Member)> = snap
            .members
            .iter()
            .filter(|m| {
                filter.is_empty()
                    || m.id.to_lowercase().contains(&filter)
                    || m.name.to_lowercase().contains(&filter)
            })
            .map(|m| {
                let level = self.level_in(&page.space, &m.id).unwrap_or(0);
                let name = if m.name.is_empty() { &m.id } else { &m.name };
                (level, name.to_lowercase(), m)
            })
            .collect();
        out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        out.into_iter().map(|(.., m)| m).collect()
    }

    /// The rooms of the space where `user`'s level isn't the space's.
    pub fn member_differs(&self, space: &str, user: &str) -> Vec<(&str, i64)> {
        let Some(home) = self.level_in(space, user) else {
            return Vec::new();
        };
        self.role_channels(space)
            .into_iter()
            .filter_map(|(id, r)| {
                let level = self.level_in(id, user)?;
                (level != home).then_some((r.name.as_str(), level))
            })
            .collect()
    }

    pub fn roles_busy(&self) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Request::ServerSettings(Op::Roles { .. })))
    }

    /// The Roles tab shows nothing this account can change.
    pub fn roles_read_only(&self, space: &str) -> bool {
        !self.may_set(space, ROLES) && !self.live.server(space).iter().any(|r| self.may_roles(r))
    }

    /// The Roles tab's input (server_settings.rs routes it here).
    pub fn roles_input(&mut self, space: String, input: Input) -> Task<Msg> {
        let busy = self.roles_busy();
        let Some(page) = self.server_settings.page.as_mut() else {
            return Task::none();
        };
        let draft = &mut page.roles;
        match input {
            Input::View(view) => {
                draft.view = view;
                draft.form = None;
                page.note = None;
            }
            Input::Filter(v) => draft.filter = v,
            Input::Channel(room) => draft.channel = room,
            Input::Name(v) => {
                if let Some(f) = &mut draft.form {
                    f.name = v.chars().take(NAME_LIMIT).collect();
                }
            }
            Input::Level(v) => {
                if let Some(f) = &mut draft.form {
                    f.level = v.chars().filter(char::is_ascii_digit).take(2).collect();
                }
            }
            Input::Color(c) => {
                if let Some(f) = &mut draft.form {
                    f.color = c;
                }
            }
            Input::Cancel => {
                draft.form = None;
                page.note = None;
            }
            Input::Edit(from) => {
                let title = from.and_then(|l| self.titles(&space)?.get(l).cloned());
                let allowed = match &title {
                    Some(t) => self.may_title(&space, t.level),
                    None => from.is_none() && self.may_set(&space, ROLES),
                };
                if !allowed || busy {
                    return Task::none();
                }
                let page = self.server_settings.page.as_mut().expect("open");
                page.note = None;
                page.roles.form = Some(match title {
                    Some(t) => Form {
                        from: Some(t.level),
                        name: t.name,
                        level: t.level.to_string(),
                        color: t.color,
                    },
                    None => Form {
                        from: None,
                        name: String::new(),
                        level: String::new(),
                        color: COLORS[0],
                    },
                });
                return iced::widget::operation::focus(NAME_FIELD);
            }
            Input::Save if !busy => self.save_title(space),
            Input::Remove(level) if !busy && self.may_title(&space, level) => {
                if self.titles(&space).is_some_and(|t| t.get(level).is_some()) {
                    self.edit_titles(space, TitleEdit::Remove { level });
                }
            }
            Input::Assign(user, level) if !busy => self.assign(space, user, level),
            Input::Perm(room, perm, level) if !busy => self.set_perm(space, room, perm, level),
            Input::Sync(room) if !busy => self.sync_channel(space, room),
            _ => {}
        }
        Task::none()
    }

    fn roles_note(&mut self, text: String) {
        if let Some(page) = &mut self.server_settings.page {
            page.note = Some(Note {
                text,
                problem: true,
            });
        }
    }

    fn save_title(&mut self, space: String) {
        let Some(form) = self
            .server_settings
            .page
            .as_ref()
            .and_then(|p| p.roles.form.clone())
        else {
            return;
        };
        let name = form.name.trim().to_owned();
        let level = form.level.parse::<i64>().ok();
        let mine = self.my_level(&space).unwrap_or(0);
        let taken = level
            .and_then(|l| self.titles(&space)?.get(l))
            .filter(|t| Some(t.level) != form.from)
            .map(|t| t.name.clone());
        let problem = if !valid_name(&name) {
            Some("Give the role a name.".to_owned())
        } else if !level.is_some_and(custom_level) {
            Some("A custom role's power level is from 1 to 99, other than 50 (Moderator).".into())
        } else if level.is_some_and(|l| l > mine) {
            Some(format!(
                "You can make roles up to your own power level, {mine}."
            ))
        } else {
            taken.map(|other| format!("{other} already has that power level."))
        };
        if let Some(text) = problem {
            return self.roles_note(text);
        }
        let level = level.expect("checked");
        if !self.may_title(&space, level) || form.from.is_some_and(|l| !self.may_title(&space, l)) {
            return;
        }
        self.edit_titles(
            space,
            TitleEdit::Set {
                from: form.from,
                level,
                name,
                color: form.color,
            },
        );
    }

    fn edit_titles(&mut self, space: String, edit: TitleEdit) {
        let request = engine::Request::EditTitles {
            space: space.clone(),
            edit: edit.clone(),
        };
        self.roles_command(space, request, Write::Title(edit));
    }

    fn roles_command(&mut self, space: String, request: engine::Request, write: Write) {
        if let Some(page) = &mut self.server_settings.page {
            page.note = None;
        }
        self.command(
            move |id| Command::SpaceSettings { id, request },
            Request::ServerSettings(Op::Roles { space, write }),
        );
    }

    /// Send `change` to each of `rooms` where this account may make it and
    /// it changes something; the rest are skipped.
    fn send_changes(
        &mut self,
        space: String,
        rooms: Vec<String>,
        change: impl Fn(&str) -> Change,
        write: impl FnOnce(usize) -> Write,
    ) {
        let mut changes = Vec::new();
        let mut skipped = 0;
        for room in rooms {
            let Some(posting) = self.posting(&room) else {
                continue;
            };
            let change = change(&room);
            match patch(posting, &self.matrix_id, &change) {
                None => skipped += 1,
                Some(new) if Some(&new) == posting.content() => {}
                Some(_) => changes.push((room, change)),
            }
        }
        let write = write(skipped);
        if changes.is_empty() {
            let note = write.note(0, 0);
            if let Some(page) = &mut self.server_settings.page {
                page.note = Some(note);
            }
            return;
        }
        self.roles_command(space, engine::Request::Power { changes }, write);
    }

    fn assign(&mut self, space: String, user: String, level: Option<i64>) {
        if !self.may_assign(&space, &user) || level.is_some_and(|l| l == MAX) {
            return;
        }
        let who = self
            .server_settings
            .page
            .as_ref()
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.members.iter().find(|m| m.id == user))
            .map(|m| m.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| user.clone());
        let role = match level {
            Some(l) => self.level_label(&space, &space, l),
            None => "Member".into(),
        };
        let rooms = self
            .live
            .server(&space)
            .into_iter()
            .map(str::to_owned)
            .collect();
        let change = Change::User { user, level };
        self.send_changes(
            space,
            rooms,
            |_| change.clone(),
            |skipped| Write::Assign { who, role, skipped },
        );
    }

    /// A permission for one channel, or as the server's default: then for
    /// the space and every room that had the old default.
    fn set_perm(&mut self, space: String, room: String, perm: Perm, level: i64) {
        let rooms: Vec<String> = if room == space {
            let Some(old) = self.perm_level(&space, perm) else {
                return;
            };
            self.live
                .server(&space)
                .into_iter()
                .filter(|r| *r == space || self.perm_level(r, perm) == Some(old))
                .map(str::to_owned)
                .collect()
        } else if self.live.server(&space).contains(&room.as_str()) {
            vec![room.clone()]
        } else {
            return;
        };
        let role = self.level_label(&space, &room, level);
        let what = if room == space {
            format!("{}: {role} by default", perm.label())
        } else {
            let name = self.live.list.get(&room).map_or("", |r| r.name.as_str());
            format!("{}: {role} in #{name}", perm.label())
        };
        let change = Change::Perms(vec![(perm, level)]);
        let server = room == space;
        self.send_changes(
            space,
            rooms,
            |_| change.clone(),
            |skipped| Write::Perms {
                what,
                skipped,
                rooms: server,
            },
        );
    }

    /// Give a channel the server's levels for each permission it differs
    /// in.
    fn sync_channel(&mut self, space: String, room: String) {
        let perms: Vec<(Perm, i64)> = self
            .channel_limits(&space, &room)
            .into_iter()
            .filter_map(|p| Some((p, self.perm_level(&space, p)?)))
            .collect();
        if perms.is_empty() || !self.live.server(&space).contains(&room.as_str()) {
            return;
        }
        let name = self.live.list.get(&room).map_or("", |r| r.name.as_str());
        let what = format!("#{name} has the server's permissions");
        let change = Change::Perms(perms);
        self.send_changes(
            space,
            vec![room],
            |_| change.clone(),
            |skipped| Write::Perms {
                what,
                skipped,
                rooms: false,
            },
        );
    }

    /// The engine's answer to a roles change.
    pub fn roles_answer(&mut self, space: String, write: Write, result: Result<Reply, Failure>) {
        let me = self.matrix_id.clone();
        let note = match (result, &write) {
            (Ok(Reply::Power(results)), _) => {
                let (mut written, mut failed) = (0, 0);
                for (room, result) in results {
                    match result {
                        Ok(Some((event_id, content))) => {
                            written += 1;
                            if let Some(r) = self.live.list.get_mut(&room) {
                                r.posting.accepted(event_id, me.clone(), content);
                            }
                        }
                        Ok(None) => {}
                        Err(_) => failed += 1,
                    }
                }
                write.note(written, failed)
            }
            (Ok(Reply::Titles(content)), Write::Title(_)) => {
                self.titles_loaded(&space, content.as_ref());
                if let Some(page) = &mut self.server_settings.page {
                    page.roles.form = None;
                }
                write.note(1, 0)
            }
            (_, Write::Title(_)) => Note {
                text: "Couldn't save the role. Check your role and connection.".into(),
                problem: true,
            },
            _ => Note {
                text: "Couldn't change the power levels. Check your connection.".into(),
                problem: true,
            },
        };
        if let Some(page) = self
            .server_settings
            .page
            .as_mut()
            .filter(|p| p.space == space)
        {
            page.note = Some(note);
        }
    }

    /// A space's custom roles as read: for badges and the Roles tab.
    pub fn titles_loaded(&mut self, space: &str, content: Option<&Value>) {
        let titles = content.map(Titles::parse).unwrap_or_default();
        let known = self.server_settings.roles.titles.get(space);
        if known.is_some_and(|k| **k == titles) {
            return;
        }
        self.server_settings
            .roles
            .titles
            .insert(space.to_owned(), Arc::new(titles));
        self.spread_titles();
    }

    /// The answer to asking for a space's roles (for badges): a failure
    /// counts as none, so it's asked once.
    pub fn titles_answer(&mut self, space: String, result: Result<Reply, Failure>) {
        match result {
            Ok(Reply::Titles(content)) => self.titles_loaded(&space, content.as_ref()),
            _ => self.titles_loaded(&space, None),
        }
    }

    /// Give each room its server's custom roles, for its badges (names.rs).
    /// A room in several spaces takes the first one's with any.
    pub fn spread_titles(&mut self) {
        let mut by_room: BTreeMap<String, Arc<Titles>> = BTreeMap::new();
        for (space, titles) in &self.server_settings.roles.titles {
            if titles.is_empty() {
                continue;
            }
            for room in self.live.server(space) {
                by_room
                    .entry(room.to_owned())
                    .or_insert_with(|| titles.clone());
            }
        }
        for (id, r) in &mut self.live.list {
            let titles = by_room.remove(id);
            if r.titles != titles {
                r.titles = titles;
            }
        }
    }

    /// Ask for the open space's custom roles once a session.
    pub fn ask_titles(&mut self) {
        let crate::Section::Space(space) = &self.section else {
            return;
        };
        let asked = self.server_settings.roles.titles.contains_key(space)
            || self.pending.values().any(
                |r| matches!(r, Request::ServerSettings(Op::Titles(s) | Op::Load(s)) if s == space),
            );
        if asked
            || !self.may_open_server_settings()
            || !self.connected
            || !self.live.list.get(space).is_some_and(|r| r.space)
        {
            return;
        }
        let space = space.clone();
        let request = engine::Request::Titles {
            space: space.clone(),
        };
        self.command(
            move |id| Command::SpaceSettings { id, request },
            Request::ServerSettings(Op::Titles(space)),
        );
    }
}

/// The message a role picker sends for `user`.
pub fn assign_msg(user: &str) -> impl Fn(Choice) -> Msg + '_ {
    move |c: Choice| input(Input::Assign(user.to_owned(), c.level))
}

/// The message a permission picker sends; Member is `everyone`'s level.
pub fn perm_msg(room: &str, perm: Perm, everyone: i64) -> impl Fn(Choice) -> Msg + '_ {
    move |c: Choice| {
        input(Input::Perm(
            room.to_owned(),
            perm,
            c.level.unwrap_or(everyone),
        ))
    }
}

pub fn msg(i: Input) -> Msg {
    input(i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::StateDoc;

    fn doc(content: Value) -> StateDoc {
        StateDoc {
            event_id: "$p".into(),
            sender: "@creator:h".into(),
            content,
            redacted: false,
        }
    }

    fn posting(version: &str, power: Value) -> Posting {
        Posting::new(
            Some(doc(json!({"room_version": version}))),
            Some(doc(power)),
        )
    }

    fn levels() -> Value {
        json!({
            "users": {"@admin:h": 100, "@mod:h": 50, "@helper:h": 25},
            "users_default": 0,
            "events": {"m.room.power_levels": 100, "m.sticker": 70, "org.example.audit": 80},
            "kick": 50,
            "org.example.policy": {"keep": true},
            "com.untethered.posting_restore.v1": {"level": 50, "events": {}}
        })
    }

    #[test]
    fn giving_a_role_patches_only_that_member() {
        let p = posting("10", levels());
        let give = |user: &str, level| Change::User {
            user: user.into(),
            level,
        };
        let out = patch(&p, "@admin:h", &give("@new:h", Some(50))).unwrap();
        let mut expected = levels();
        expected["users"]["@new:h"] = 50.into();
        assert_eq!(out, expected, "everything else stays, unknown fields too");
        // Member removes the entry, so the room's default applies.
        let out = patch(&p, "@admin:h", &give("@helper:h", None)).unwrap();
        assert!(out["users"].get("@helper:h").is_none());
        // Nothing to change: the same document.
        assert_eq!(
            patch(&p, "@admin:h", &give("@mod:h", Some(50))),
            Some(levels())
        );
        // Not above your own level, not someone at it, not yourself.
        assert!(patch(&p, "@admin:h", &give("@new:h", Some(101))).is_none());
        let mut equal = levels();
        equal["users"]["@other:h"] = 100.into();
        let peer = posting("10", equal);
        assert!(patch(&peer, "@admin:h", &give("@other:h", Some(50))).is_none());
        assert!(patch(&p, "@admin:h", &give("@admin:h", Some(50))).is_none());
        // A moderator can't send power levels here at all.
        assert!(patch(&p, "@mod:h", &give("@new:h", Some(25))).is_none());
    }

    #[test]
    fn version_12_creators_outrank_everyone_and_keep_out_of_users() {
        let p = posting("12", levels());
        let give = |user: &str| Change::User {
            user: user.into(),
            level: Some(50),
        };
        // The creator may change an admin, and nobody may change them.
        assert!(patch(&p, "@creator:h", &give("@admin:h")).is_some());
        assert!(patch(&p, "@admin:h", &give("@creator:h")).is_none());
        let old = posting("11", levels());
        assert!(
            patch(&old, "@creator:h", &give("@mod:h")).is_none(),
            "before version 12 a creator is only their level"
        );
    }

    #[test]
    fn a_permission_sets_each_of_its_slots_within_the_actors_level() {
        let mut c = levels();
        c["users"]["@roles:h"] = 75.into();
        c["events"]["m.room.power_levels"] = 75.into();
        let p = posting("10", c.clone());
        assert_eq!(Perm::Post.level(&c), Some(0));
        let out = patch(&p, "@roles:h", &Change::Perms(vec![(Perm::Post, 50)])).unwrap();
        for kind in [
            "m.room.message",
            "m.room.encrypted",
            "m.sticker",
            "m.poll.start",
            "org.matrix.msc3381.poll.start",
        ] {
            assert_eq!(out["events"][kind], 50, "{kind}");
        }
        assert_eq!(Perm::Post.level(&out), Some(50));
        assert_eq!(out["events"]["org.example.audit"], 80);
        assert_eq!(out["org.example.policy"], json!({"keep": true}));
        assert!(out.get("events_default").is_none(), "defaults stay");
        // A level above the actor's, or a slot above it, is refused.
        assert!(patch(&p, "@roles:h", &Change::Perms(vec![(Perm::Kick, 80)])).is_none());
        let mut strict = c.clone();
        strict["ban"] = 90.into();
        let strict = posting("10", strict);
        assert!(patch(&strict, "@roles:h", &Change::Perms(vec![(Perm::Ban, 50)])).is_none());
        // Unchanged slots aren't written.
        let same = patch(&p, "@roles:h", &Change::Perms(vec![(Perm::Kick, 50)])).unwrap();
        assert_eq!(same, c);
        let invite = patch(&p, "@roles:h", &Change::Perms(vec![(Perm::Invite, 50)])).unwrap();
        assert_eq!(invite["invite"], 50);
        assert_eq!(Perm::Invite.level(&invite), Some(50));
        // State events fall back to state_default.
        assert_eq!(Perm::Pin.level(&c), Some(50));
        assert_eq!(Perm::Roles.level(&c), Some(75));
    }

    #[test]
    fn custom_roles_keep_unknown_fields_and_their_levels_apart() {
        let set = |from, level, name: &str| TitleEdit::Set {
            from,
            level,
            name: name.into(),
            color: 0xe67e22,
        };
        let made = apply_titles(None, &set(None, 25, " Helper ")).unwrap();
        assert_eq!(
            made,
            json!({"roles": {"25": {"name": "Helper", "color": "#e67e22"}}})
        );
        let mut extra = made.clone();
        extra["x.other"] = 1.into();
        extra["roles"]["25"]["x.icon"] = "mxc://h/i".into();
        let moved = apply_titles(Some(extra.clone()), &set(Some(25), 30, "Helpers")).unwrap();
        assert_eq!(moved["x.other"], 1);
        assert!(moved["roles"].get("25").is_none());
        assert_eq!(moved["roles"]["30"]["x.icon"], "mxc://h/i");
        assert_eq!(moved["roles"]["30"]["name"], "Helpers");
        // Matrix's own levels and taken ones are refused.
        for level in [0, 50, 100, 150] {
            assert!(
                apply_titles(Some(extra.clone()), &set(None, level, "X")).is_err(),
                "{level}"
            );
        }
        assert!(apply_titles(Some(extra.clone()), &set(None, 25, "Again")).is_err());
        assert!(apply_titles(Some(extra.clone()), &set(None, 10, "  ")).is_err());
        let removed = apply_titles(Some(extra), &TitleEdit::Remove { level: 25 }).unwrap();
        assert_eq!(removed["roles"], json!({}));

        let titles = Titles::parse(&json!({"roles": {
            "25": {"name": "Helper", "color": "#E67E22"},
            "75": {"name": "Senior", "color": "red"},
            "50": {"name": "Not Moderator"},
            "x": {"name": "Bad"},
            "10": {"name": ""}
        }}));
        let parsed: Vec<_> = titles
            .iter()
            .map(|t| (t.level, t.name.as_str(), t.color))
            .collect();
        assert_eq!(parsed, [(75, "Senior", PLAIN), (25, "Helper", 0xe67e22)]);
        assert_eq!(titles.reached(60).map(|t| t.level), Some(25));
        assert_eq!(titles.reached(80).map(|t| t.level), Some(75));
        assert_eq!(level_name(25, 0, Some(&titles)), "Helper");
        assert_eq!(level_name(0, 0, Some(&titles)), "Member");
        assert_eq!(level_name(60, 0, Some(&titles)), "Level 60");
        assert_eq!(level_name(MAX, 0, None), "Owner");
    }

    mod app {
        use super::*;
        use crate::{
            engine::{RoomList, RoomSummary, space_settings::Snapshot},
            server_settings::Input as Settings,
        };

        /// A space with general and announcements (its own posting rule)
        /// where this account has `level`, and vault where it's a member.
        fn app(level: i64) -> (App, async_channel::Receiver<Command>) {
            let (backend, _) = async_channel::bounded(8);
            let (sdk, rx) = async_channel::bounded(16);
            let mut app = App::new(backend);
            app.sdk = Some(sdk);
            app.logged_in = true;
            app.connected = true;
            app.verified = true;
            app.matrix_id = "@me:h".into();
            let power = |me: i64, post: i64| {
                doc(
                    json!({"users": {"@me:h": me, "@mod:h": 50}, "users_default": 0,
                    "events": {"m.room.message": post, "x.keep": 7}}),
                )
            };
            let room = |id: &str, space: bool, power| RoomSummary {
                id: id.into(),
                name: id[1..id.find(':').unwrap()].into(),
                space,
                create: Some(doc(json!({"room_version": "10"}))),
                power: Some(power),
                ..Default::default()
            };
            let rooms = vec![
                room("!space:h", true, power(level, 0)),
                room("!general:h", false, power(level, 0)),
                room("!announcements:h", false, power(level, 50)),
                room("!vault:h", false, power(0, 0)),
            ];
            let children = [(
                "!space:h".to_owned(),
                ["!general:h", "!announcements:h", "!vault:h"]
                    .map(str::to_owned)
                    .into(),
            )]
            .into();
            app.live.apply_engine(RoomList {
                rooms,
                children,
                ..Default::default()
            });
            let _ = app.server_settings(Settings::Open("!space:h".into()));
            let Ok(Command::SpaceSettings { id, .. }) = rx.try_recv() else {
                panic!("no load");
            };
            let members = ["@me:h", "@mod:h", "@bob:h"]
                .map(|id| Member {
                    id: id.into(),
                    name: String::new(),
                })
                .into();
            let snapshot = Snapshot {
                name: "Space".into(),
                members,
                roles: Some(json!({"roles": {"25": {"name": "Helper", "color": "#e67e22"}}})),
                ..Default::default()
            };
            app.space_settings_answer(id, Ok(Reply::Loaded(snapshot)));
            (app, rx)
        }

        fn roles(app: &mut App, i: Input) {
            let _ = app.server_settings(Settings::Roles(i));
        }

        fn sent(rx: &async_channel::Receiver<Command>) -> Option<(i64, engine::Request)> {
            match rx.try_recv() {
                Ok(Command::SpaceSettings { id, request }) => Some((id, request)),
                _ => None,
            }
        }

        #[test]
        fn a_role_goes_to_each_room_this_account_may_change() {
            let (mut app, rx) = app(100);
            assert!(app.titles("!space:h").is_some_and(|t| t.get(25).is_some()));
            let labels: Vec<_> = app
                .role_choices("!space:h", "!space:h", false)
                .into_iter()
                .map(|c| c.label)
                .collect();
            assert_eq!(labels, ["Member", "Helper", "Moderator", "Admin"]);
            roles(&mut app, Input::Assign("@bob:h".into(), Some(25)));
            let Some((id, engine::Request::Power { changes })) = sent(&rx) else {
                panic!("no power levels change");
            };
            let rooms: std::collections::BTreeSet<_> =
                changes.iter().map(|(r, _)| r.as_str()).collect();
            assert_eq!(rooms, ["!space:h", "!general:h", "!announcements:h"].into());
            assert!(app.roles_busy());
            // Another change waits for this one.
            roles(&mut app, Input::Assign("@mod:h".into(), None));
            assert!(sent(&rx).is_none());
            let results = changes
                .iter()
                .map(|(room, change)| {
                    let posting = &app.live.list[room].posting;
                    let content = patch(posting, "@me:h", change).unwrap();
                    (room.clone(), Ok(Some(("$new".to_owned(), content))))
                })
                .collect();
            app.space_settings_answer(id, Ok(Reply::Power(results)));
            assert_eq!(app.level_in("!general:h", "@bob:h"), Some(25));
            assert_eq!(app.level_in("!vault:h", "@bob:h"), Some(0));
            let note = app
                .server_settings
                .page
                .as_ref()
                .unwrap()
                .note
                .clone()
                .unwrap();
            assert!(!note.problem);
            assert!(note.text.contains("Helper in 3 rooms"), "{}", note.text);
            assert!(
                note.text.contains("One room you can't change"),
                "{}",
                note.text
            );
            // The badge names the custom role.
            let badge = app.live.list["!general:h"].role("@bob:h").unwrap();
            assert_eq!(badge.tag(), "HELPER");
        }

        #[test]
        fn a_default_changes_the_channels_that_had_it() {
            let (mut app, rx) = app(100);
            assert_eq!(
                app.channel_limits("!space:h", "!announcements:h"),
                [Perm::Post]
            );
            roles(&mut app, Input::Perm("!space:h".into(), Perm::Post, 25));
            let Some((_, engine::Request::Power { changes })) = sent(&rx) else {
                panic!("no power levels change");
            };
            let rooms: Vec<_> = changes.iter().map(|(r, _)| r.as_str()).collect();
            assert_eq!(
                rooms,
                ["!space:h", "!general:h"],
                "announcements keeps its own"
            );
            assert_eq!(changes[0].1, Change::Perms(vec![(Perm::Post, 25)]));
        }

        #[test]
        fn a_channel_takes_the_servers_permissions_back() {
            let (mut app, rx) = app(100);
            roles(&mut app, Input::Sync("!announcements:h".into()));
            let Some((_, engine::Request::Power { changes })) = sent(&rx) else {
                panic!("no power levels change");
            };
            assert_eq!(
                changes,
                [(
                    "!announcements:h".to_owned(),
                    Change::Perms(vec![(Perm::Post, 0)])
                )]
            );
        }

        #[test]
        fn a_member_without_the_right_changes_nothing() {
            let (mut app, rx) = app(0);
            assert!(app.roles_read_only("!space:h"));
            assert!(!app.may_assign("!space:h", "@bob:h"));
            roles(&mut app, Input::Assign("@bob:h".into(), Some(25)));
            roles(&mut app, Input::Perm("!space:h".into(), Perm::Post, 0));
            roles(&mut app, Input::Edit(None));
            roles(&mut app, Input::Remove(25));
            assert!(sent(&rx).is_none());
            assert!(
                app.server_settings
                    .page
                    .as_ref()
                    .unwrap()
                    .roles
                    .form
                    .is_none()
            );
        }

        #[test]
        fn escape_closes_the_page_after_using_the_pickers() {
            let (mut app, rx) = app(100);
            roles(&mut app, Input::View(View::Permissions));
            roles(&mut app, Input::Channel(Some("!announcements:h".into())));
            roles(
                &mut app,
                Input::Perm("!announcements:h".into(), Perm::React, 25),
            );
            let Some((id, engine::Request::Power { .. })) = sent(&rx) else {
                panic!("no power levels change");
            };
            app.space_settings_answer(id, Ok(Reply::Power(Vec::new())));
            let _ = app.update(Msg::FreeEscape);
            assert!(app.server_settings.page.is_none());
        }

        #[test]
        fn a_moderator_gives_only_roles_below_their_own() {
            let (mut app, rx) = app(50);
            // Power levels need 50 to change here (state_default).
            assert!(app.may_assign("!space:h", "@bob:h"));
            assert!(!app.may_assign("!space:h", "@mod:h"), "an equal");
            let labels: Vec<_> = app
                .role_choices("!space:h", "!space:h", false)
                .into_iter()
                .map(|c| c.label)
                .collect();
            assert_eq!(labels, ["Member", "Helper", "Moderator"]);
            roles(&mut app, Input::Assign("@bob:h".into(), Some(100)));
            assert!(sent(&rx).is_none(), "above their own level");
        }

        #[test]
        fn the_role_form_checks_its_level_before_saving() {
            let (mut app, rx) = app(100);
            roles(&mut app, Input::Edit(None));
            roles(&mut app, Input::Name("Senior".into()));
            roles(&mut app, Input::Level("50".into()));
            roles(&mut app, Input::Save);
            assert!(sent(&rx).is_none());
            roles(&mut app, Input::Level("25".into()));
            roles(&mut app, Input::Save);
            assert!(sent(&rx).is_none(), "Helper has 25");
            let note = app
                .server_settings
                .page
                .as_ref()
                .unwrap()
                .note
                .clone()
                .unwrap();
            assert!(note.text.contains("Helper"), "{}", note.text);
            roles(&mut app, Input::Level("75".into()));
            roles(&mut app, Input::Color(COLORS[2]));
            roles(&mut app, Input::Save);
            let Some((id, engine::Request::EditTitles { edit, .. })) = sent(&rx) else {
                panic!("no role saved");
            };
            assert_eq!(
                edit,
                TitleEdit::Set {
                    from: None,
                    level: 75,
                    name: "Senior".into(),
                    color: COLORS[2]
                }
            );
            let content = json!({"roles": {"25": {"name": "Helper"}, "75": {"name": "Senior"}}});
            app.space_settings_answer(id, Ok(Reply::Titles(Some(content))));
            assert!(
                app.server_settings
                    .page
                    .as_ref()
                    .unwrap()
                    .roles
                    .form
                    .is_none()
            );
            assert_eq!(app.level_label("!space:h", "!general:h", 75), "Senior");
        }
    }

    #[test]
    fn a_role_name_keeps_whole_characters() {
        let name = RoleName::new(&"é".repeat(70));
        assert_eq!(name.as_str(), "é".repeat(63));
        assert_eq!(RoleName::new("Helper").as_str(), "Helper");
    }
}
