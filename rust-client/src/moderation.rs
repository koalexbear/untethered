//! Room membership actions are addressed by MXID and current membership
//! event, never by display name or by a message in the timeline.
use crate::{App, Request, room_state::RoomState};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Joined,
    Invited,
    Knocking,
    Banned,
    Left,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Joined => "Joined",
            Self::Invited => "Invited",
            Self::Knocking => "Requested",
            Self::Banned => "Banned",
            Self::Left => "Left",
        }
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Member {
    pub kind: Kind,
    pub name: String,
    /// The membership event, which a change is checked against.
    pub event: String,
}
impl Member {}
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Action {
    Kick,
    Ban,
    Unban,
    Invite,
}
impl Action {
    pub const ALL: [Self; 4] = [Self::Kick, Self::Ban, Self::Unban, Self::Invite];
    pub fn label(self) -> &'static str {
        match self {
            Self::Kick => "Kick",
            Self::Ban => "Ban",
            Self::Unban => "Unban",
            Self::Invite => "Invite",
        }
    }
    fn applies(self, kind: Kind) -> bool {
        match self {
            Self::Kick => kind == Kind::Joined,
            Self::Ban => kind != Kind::Banned,
            Self::Unban => kind == Kind::Banned,
            Self::Invite => matches!(kind, Kind::Left | Kind::Knocking),
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct Change {
    room: String,
    user: String,
    event: String,
    power: String,
    reason: String,
    action: Action,
}
pub enum Flight {
    Read {
        room: String,
        prior_power: Option<String>,
        change: Option<Change>,
    },
    Write(Change),
}
impl Flight {
    fn room(&self) -> &str {
        match self {
            Self::Read { room, .. } => room,
            Self::Write(c) => &c.room,
        }
    }
}
#[derive(Default)]
pub struct Directory {
    pub users: BTreeMap<String, Member>,
    pub filter: String,
    pub selected: Option<String>,
    pub reason: String,
    pub note: String,
    // Do not let a delayed snapshot undo a membership transition already
    // observed in sync while that snapshot was in flight.
    changed: BTreeSet<String>,
}
impl Directory {
    /// A membership transition seen in sync.
    pub fn observe(&mut self, user: &str, member: Member) {
        self.changed.insert(user.into());
        self.users.insert(user.into(), member);
    }
    pub fn choose(&mut self, id: Option<String>) {
        self.selected = id.filter(|u| self.users.contains_key(u));
        self.reason.clear();
        self.note.clear();
    }
}
impl App {
    pub fn reset_directory(&mut self) {
        self.directory = Directory::default();
        self.pending
            .retain(|_, p| !matches!(p, Request::Membership(Flight::Read { .. })));
    }
    pub fn membership_busy(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p,Request::Membership(f) if f.room()==self.selected))
    }
    pub fn read_directory(&mut self, change: Option<Change>) {
        if !self.people
            || !self.verified
            || !self.logged_in
            || !self.connected
            || self.account
            || self.membership_busy()
        {
            return;
        }
        let room = self.selected.clone();
        let Some(r) = self.live.list.get(&room) else {
            return;
        };
        let prior_power = r.posting.id().map(str::to_owned);
        self.directory.changed.clear();
        let target = room.clone();
        self.fetch_room_state(
            &target,
            Request::Membership(Flight::Read {
                room,
                prior_power,
                change,
            }),
        );
    }
    pub fn member_change(&self, action: Action) -> Option<Change> {
        if !self.people || self.account || !self.connected || !self.verified || !self.logged_in {
            return None;
        }
        let room = self.live.list.get(&self.selected)?;
        let d = &self.directory;
        let user = d.selected.as_ref()?;
        let m = d.users.get(user)?;
        if d.users.get(&self.matrix_id)?.kind != Kind::Joined
            || !action.applies(m.kind)
            || !room.posting.can_moderate(&self.matrix_id, user, action)
        {
            return None;
        }
        Some(Change {
            room: self.selected.clone(),
            user: user.clone(),
            event: m.event.clone(),
            power: room.posting.id()?.into(),
            reason: d.reason.trim().into(),
            action,
        })
    }
    pub fn moderate(&mut self, c: Change) {
        if self.current_member_change(&c) {
            self.read_directory(Some(c));
        }
    }
    fn current_member_change(&self, c: &Change) -> bool {
        self.member_change(c.action).as_ref() == Some(c)
    }
    pub fn membership_error(&mut self, f: Flight) {
        if self.people && f.room() == self.selected {
            self.directory.note = match f {
                Flight::Read { .. } => "Could not verify room membership. Refresh to try again.",
                Flight::Write(_) => "Membership change failed. Refresh to check your permissions.",
            }
            .into();
        }
    }
    /// Fresh room state for the directory, and for a change waiting on it.
    pub fn membership_state(&mut self, f: Flight, state: RoomState) {
        let Flight::Read {
            room,
            prior_power,
            change,
        } = f
        else {
            return;
        };
        if !self.people || self.selected != room {
            return;
        }
        let Some(r) = self.live.list.get_mut(&room) else {
            return;
        };
        let fresh = state.posting();
        let mut users = state.members;
        let checked = change.as_ref().is_some_and(|c| {
            fresh.id() == Some(&c.power)
                && fresh.can_moderate(&self.matrix_id, &c.user, c.action)
                && users
                    .get(&self.matrix_id)
                    .is_some_and(|m| m.kind == Kind::Joined)
                && users
                    .get(&c.user)
                    .is_some_and(|m| m.event == c.event && c.action.applies(m.kind))
        });
        if r.posting.id() == prior_power.as_deref() {
            r.posting.adopt(fresh);
        }
        for id in std::mem::take(&mut self.directory.changed) {
            if let Some(m) = self.directory.users.remove(&id) {
                users.insert(id, m);
            }
        }
        self.directory.users = users;
        if let Some(c) = change {
            if !checked || !self.current_member_change(&c) {
                self.directory.note =
                    "Membership or permissions changed. Review the member before trying again."
                        .into();
                return;
            }
            let (room, user, reason) = (c.room.clone(), c.user.clone(), c.reason.clone());
            self.send_membership(
                &room,
                &user,
                c.action,
                &reason,
                Request::Membership(Flight::Write(c)),
            );
        }
    }
    /// The homeserver accepted a membership change.
    pub fn membership_written(&mut self, c: Change) {
        if !self.people || self.selected != c.room {
            return;
        }
        if self.directory.selected.as_ref() == Some(&c.user) {
            self.directory.reason.clear();
            self.directory.note = format!("{} accepted by homeserver.", c.action.label());
        }
        // A successful membership change returns no event. If sync has not
        // delivered the transition yet, reload the actual state.
        if self
            .directory
            .users
            .get(&c.user)
            .is_none_or(|m| m.event == c.event)
        {
            self.read_directory(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{permissions::StateDoc, rooms::Room};
    use serde_json::json;
    fn doc(id: &str, sender: &str, content: serde_json::Value) -> StateDoc {
        StateDoc {
            event_id: id.into(),
            sender: sender.into(),
            content,
            redacted: false,
        }
    }
    fn power(id: &str, kick: u64, ban: u64) -> StateDoc {
        let levels = json!({"users":{"@mod:h":60,"@equal:h":60},"invite":10,"ban":ban,"kick":kick});
        doc(id, "@owner:h", levels)
    }
    fn state() -> RoomState {
        let member = |event: &str| Member {
            kind: Kind::Joined,
            name: "Same display name".into(),
            event: event.into(),
        };
        RoomState {
            create: Some(doc("$cp", "@owner:h", json!({"room_version":"12"}))),
            power: Some(power("$pl", 65, 55)),
            members: [
                ("@mod:h".to_owned(), member("$mod")),
                ("@other:h".to_owned(), member("$peer")),
                ("@equal:h".to_owned(), member("$equal")),
            ]
            .into(),
        }
    }
    fn app() -> (App, async_channel::Receiver<crate::backend::Command>) {
        let (tx, rx) = async_channel::bounded(8);
        let mut a = App::new(tx);
        a.selected = "!room".into();
        a.live.list.insert(a.selected.clone(), Room::default());
        a.people = true;
        a.logged_in = true;
        a.verified = true;
        a.connected = true;
        a.matrix_id = "@mod:h".into();
        a.membership_state(
            Flight::Read {
                room: a.selected.clone(),
                prior_power: None,
                change: None,
            },
            state(),
        );
        (a, rx)
    }
    #[test]
    fn actor_and_target_powers_and_banned_state_constrain_controls() {
        let (mut a, _) = app();
        for user in ["@mod:h", "@equal:h"] {
            a.directory.choose(Some(user.into()));
            assert!(a.member_change(Action::Ban).is_none());
        }
        a.directory.choose(Some("@other:h".into()));
        assert!(a.member_change(Action::Ban).is_some());
        a.directory.users.get_mut("@other:h").unwrap().kind = Kind::Banned;
        assert!(
            a.member_change(Action::Unban).is_none(),
            "unban needs the room's higher kick threshold"
        );
        assert!(
            a.member_change(Action::Invite).is_none(),
            "an invite cannot undo a ban"
        );
        a.directory.users.get_mut("@other:h").unwrap().kind = Kind::Left;
        assert!(a.member_change(Action::Invite).is_some());
        a.directory.users.get_mut("@mod:h").unwrap().kind = Kind::Left;
        assert!(a.member_change(Action::Invite).is_none());
    }
    #[test]
    fn kick_uses_its_own_threshold_and_only_targets_joined_lower_power_members() {
        let (mut a, _) = app();
        a.directory.choose(Some("@other:h".into()));
        // Level 60 can ban at 55 but cannot kick at 65.
        assert!(a.member_change(Action::Kick).is_none());
        a.live
            .list
            .get_mut("!room")
            .unwrap()
            .posting
            .store_snapshot(None, Some(power("$pl2", 60, 70)));
        assert!(a.member_change(Action::Kick).is_some());
        assert!(a.member_change(Action::Ban).is_none());
        for target in ["@mod:h", "@equal:h"] {
            a.directory.choose(Some(target.into()));
            assert!(a.member_change(Action::Kick).is_none());
        }
        a.directory.choose(Some("@other:h".into()));
        for kind in [Kind::Left, Kind::Banned, Kind::Invited, Kind::Knocking] {
            a.directory.users.get_mut("@other:h").unwrap().kind = kind;
            assert!(a.member_change(Action::Kick).is_none());
        }
    }
}
