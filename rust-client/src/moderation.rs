//! Room membership actions are addressed by MXID and current membership
//! event, never by display name or by a message in the timeline.
use crate::{
    App, Request,
    room_state::RoomState,
    rooms::{items, server_confirmed},
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub kind: Kind,
    pub name: String,
    /// The membership event, which a change is checked against.
    pub event: String,
}
impl Member {
    /// A confirmed membership event of `room` from gomuks.
    pub(crate) fn read<'a>(room: &str, e: &'a Value) -> Option<(&'a str, Self)> {
        if e["room_id"] != room || e["type"] != "m.room.member" || !server_confirmed(e) {
            return None;
        }
        let id = e["state_key"]
            .as_str()
            .filter(|u| u.starts_with('@') && u.contains(':'))?;
        let kind = match e["content"]["membership"].as_str()? {
            "join" => Kind::Joined,
            "invite" => Kind::Invited,
            "knock" => Kind::Knocking,
            "ban" => Kind::Banned,
            "leave" => Kind::Left,
            _ => return None,
        };
        Some((
            id,
            Self {
                kind,
                event: e["event_id"].as_str()?.into(),
                name: e["content"]["displayname"].as_str().unwrap_or("").into(),
            },
        ))
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Kick,
    Ban,
    Unban,
    Invite,
}
impl Action {
    pub const ALL: [Self; 4] = [Self::Kick, Self::Ban, Self::Unban, Self::Invite];
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Kick => "kick",
            Self::Ban => "ban",
            Self::Unban => "unban",
            Self::Invite => "invite",
        }
    }
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
    pub fn sync(&mut self, room: &str, delta: &Value) {
        for e in items(&delta["events"]) {
            if let Some((id, m)) = Member::read(room, e)
                && e["rowid"]
                    .as_i64()
                    .is_some_and(|r| r > 0 && delta["state"]["m.room.member"][id] == r)
            {
                self.observe(id, m);
            }
        }
    }
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
    /// gomuks's answer to a membership read or write.
    pub fn membership_result(&mut self, f: Flight, data: &Value) {
        match f {
            Flight::Read { .. } => {
                let state = RoomState::from_gomuks(f.room(), data);
                self.membership_state(f, state);
            }
            Flight::Write(c) => self.membership_written(c),
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
    use crate::rooms::Room;
    use serde_json::json;
    fn membership(user: &str, ev: &str, kind: &str) -> Value {
        json!({"room_id":"!room","event_id":ev,"type":"m.room.member","state_key":user,
            "content":{"membership":kind,"displayname":"Same display name"}})
    }
    fn state() -> Value {
        json!([{"room_id":"!room","event_id":"$cp","type":"m.room.create","state_key":"","sender":"@owner:h","content":{"room_version":"12"}},
            {"room_id":"!room","event_id":"$pl","type":"m.room.power_levels","state_key":"","content":{
                "users":{"@mod:h":60,"@equal:h":60},"invite":10,"ban":55,"kick":65}},
            membership("@mod:h","$mod","join"),membership("@other:h","$peer","join"),membership("@equal:h","$equal","join")])
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
        a.membership_result(
            Flight::Read {
                room: a.selected.clone(),
                prior_power: None,
                change: None,
            },
            &state(),
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
        let mut changed = state();
        changed[1]["content"]["kick"] = json!(60);
        changed[1]["content"]["ban"] = json!(70);
        a.live
            .list
            .get_mut("!room")
            .unwrap()
            .posting
            .ingest("!room", &changed[1], false);
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
    #[test]
    fn stale_target_membership_cannot_be_overwritten_or_moderated_by_a_late_snapshot() {
        let (mut a, rx) = app();
        a.directory.choose(Some("@other:h".into()));
        let ban = a.member_change(Action::Ban).unwrap();
        a.moderate(ban);
        let req = rx.try_recv().unwrap().into_rpc();
        assert_eq!(req["command"], "get_room_state");
        let mut banned = membership("@other:h", "$ban", "ban");
        banned["rowid"] = json!(42);
        a.directory.sync(
            "!room",
            &json!({"state":{"m.room.member":{"@other:h":42}},"events":[banned]}),
        );
        a.frame(json!({"command":"response","request_id":req["request_id"],"data":state()}));
        assert!(
            rx.try_recv().is_err(),
            "no write may follow a stale preflight"
        );
        assert!(a.directory.users["@other:h"].kind == Kind::Banned);
    }
}
