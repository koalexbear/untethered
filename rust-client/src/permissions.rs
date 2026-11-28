//! Member posting presets are changes to real room power levels. Patch the
//! current document: roles, state-event rules and unknown fields belong to
//! the room, not to this editor.
use crate::{App, Request, moderation::Kind, room_state::RoomState, rooms::items};
use serde_json::{Value, json};

pub(crate) const POWER: &str = "m.room.power_levels";
const RESTORE: &str = "com.untethered.posting_restore.v1";
const POSTS: [&str; 4] = [
    "m.room.message",
    "m.room.encrypted",
    "m.sticker",
    "m.reaction",
];
// Matrix integers must survive a canonical JSON round trip.
pub(crate) const MAX: i64 = (1 << 53) - 1;
pub(crate) fn number(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|n| (-MAX..=MAX).contains(n))
}
pub(crate) fn level(map: &Value, key: &str, default: i64) -> Option<i64> {
    map.get(key).map_or(Some(default), number)
}
/// A Matrix state event as the App keeps it, without an engine's envelope.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StateDoc {
    pub event_id: String,
    pub sender: String,
    pub content: Value,
    /// Redacted, so its content no longer says what it did.
    pub redacted: bool,
}
impl StateDoc {}
#[derive(Default)]
pub struct Posting {
    create: Option<StateDoc>,
    power: Option<StateDoc>,
    /// Power levels events this one replaced. A replaced state event never
    /// becomes current again, so an older snapshot offering one is stale.
    superseded: Vec<String>,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Restrict,
    Restore,
    Allow,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Restrict => "Make read-only",
            Self::Restore => "Restore posting",
            Self::Allow => "Allow member messages",
        }
    }
}
impl Posting {
    pub fn new(create: Option<StateDoc>, power: Option<StateDoc>) -> Self {
        Self {
            create,
            power,
            superseded: Vec::new(),
        }
    }
    /// Take the documents of a fresher read, remembering what they replace.
    pub fn adopt(&mut self, fresh: Posting) {
        let Posting { create, power, .. } = fresh;
        if let Some(create) = create {
            self.create = Some(create);
        }
        self.replace(power);
    }
    /// The documents from a store snapshot, unless they are older than what
    /// is already known. Only the matrix-sdk engine has a store to read.
    pub fn store_snapshot(&mut self, create: Option<StateDoc>, power: Option<StateDoc>) {
        if create.is_some() {
            self.create = create;
        }
        if power
            .as_ref()
            .is_some_and(|p| self.superseded.contains(&p.event_id))
        {
            return;
        }
        self.replace(power);
    }
    fn replace(&mut self, power: Option<StateDoc>) {
        let old = self.id().map(str::to_owned);
        if old.as_deref() == power.as_ref().map(|p| p.event_id.as_str()) {
            return;
        }
        if let Some(old) = old {
            self.superseded.push(old);
            // Power level changes are rare; a short memory is enough.
            if self.superseded.len() > 16 {
                self.superseded.remove(0);
            }
        }
        self.power = power;
    }
    /// Our own power level change, once the homeserver accepted it.
    pub fn accepted(&mut self, event_id: String, sender: String, content: Value) {
        self.replace(Some(StateDoc {
            event_id,
            sender,
            content,
            redacted: false,
        }));
    }
    pub fn id(&self) -> Option<&str> {
        Some(&self.power.as_ref()?.event_id)
    }
    /// The power levels document, when it's one this client can read
    /// (roles.rs patches it too).
    pub(crate) fn content(&self) -> Option<&Value> {
        let doc = self.power.as_ref()?;
        let c = &doc.content;
        (!doc.redacted
            && c.is_object()
            && ["users", "events"]
                .iter()
                .all(|k| c.get(k).is_none_or(Value::is_object)))
        .then_some(c)
    }
    pub(crate) fn actor(&self, me: &str) -> Option<i64> {
        if me.is_empty() {
            return None;
        }
        let c = self.content()?;
        let create = self.create.as_ref()?;
        let modern = create.content["room_version"]
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            .is_some_and(|v| v >= 12);
        if modern
            && (create.sender == me
                || items(&create.content["additional_creators"]).any(|v| v == me))
        {
            return Some(MAX);
        }
        level(&c["users"], me, level(c, "users_default", 0)?)
    }
    pub fn can_post(&self, me: &str, encrypted: bool) -> Option<bool> {
        let c = self.content()?;
        let min = level(
            &c["events"],
            if encrypted {
                "m.room.encrypted"
            } else {
                "m.room.message"
            },
            level(c, "events_default", 0)?,
        )?;
        Some(self.actor(me)? >= min)
    }
    /// Reacting needs the room's `m.reaction` level. In an encrypted room a
    /// reaction goes out encrypted, and the homeserver checks
    /// `m.room.encrypted`'s level instead.
    pub fn can_react(&self, me: &str, encrypted: bool) -> Option<bool> {
        let c = self.content()?;
        let kind = if encrypted {
            "m.room.encrypted"
        } else {
            "m.reaction"
        };
        let min = level(&c["events"], kind, level(c, "events_default", 0)?)?;
        Some(self.actor(me)? >= min)
    }
    /// The sender needs permission to send a redaction. Deleting someone
    /// else's event also requires the room's redaction power level.
    pub(crate) fn can_redact_message(&self, me: &str, sender: &str) -> bool {
        let required = self.content().and_then(|content| {
            let send = level(
                &content["events"],
                "m.room.redaction",
                level(content, "events_default", 0)?,
            )?;
            if me == sender {
                Some(send)
            } else {
                Some(send.max(level(content, "redact", 50)?))
            }
        });
        required
            .zip(self.actor(me))
            .is_some_and(|(needed, actor)| actor >= needed)
    }

    pub fn can_link_channel(&self, me: &str) -> bool {
        self.can_send_state(me, "m.space.child")
    }

    pub(crate) fn can_pin_message(&self, me: &str) -> bool {
        self.can_send_state(me, "m.room.pinned_events")
    }

    /// This account may send `event_type` state events here (server
    /// settings, ROADMAP C1, asks this of each control).
    pub(crate) fn can_send_state(&self, me: &str, event_type: &str) -> bool {
        self.content()
            .and_then(|c| level(&c["events"], event_type, level(c, "state_default", 50)?))
            .zip(self.actor(me))
            .is_some_and(|(needed, actor)| actor >= needed)
    }
    pub fn can_invite(&self, me: &str) -> bool {
        self.content()
            .and_then(|c| level(c, "invite", 0))
            .zip(self.actor(me))
            .is_some_and(|(needed, actor)| actor >= needed)
    }
    pub fn can_moderate(&self, me: &str, user: &str, action: crate::moderation::Action) -> bool {
        use crate::moderation::Action;
        let (Some(c), Some(actor), Some(target)) =
            (self.content(), self.actor(me), self.actor(user))
        else {
            return false;
        };
        if me == user {
            return false;
        }
        let enough = |name, default| level(c, name, default).is_some_and(|v| actor >= v);
        match action {
            Action::Invite => enough("invite", 0),
            Action::Kick => actor > target && enough("kick", 50),
            Action::Ban => actor > target && enough("ban", 50),
            // In Synapse unbanning another user is ban -> leave, so the
            // room's kick threshold also applies.
            Action::Unban => actor > target && enough("ban", 50) && enough("kick", 50),
        }
    }
    pub fn summary(&self) -> Option<(i64, i64, i64)> {
        let c = self.content()?;
        let d = level(c, "events_default", 0)?;
        Some((
            level(c, "users_default", 0)?,
            level(&c["events"], POSTS[0], d)?,
            level(&c["events"], POSTS[1], d)?,
        ))
    }
    pub fn action(&self) -> Option<Action> {
        let c = self.content()?;
        if c.get(RESTORE).is_some() {
            return Some(Action::Restore);
        }
        let (member, plain, cipher) = self.summary()?;
        Some(if plain > member && cipher > member {
            Action::Allow
        } else {
            Action::Restrict
        })
    }
    // Plan and authorize the exact field changes. A stricter existing posting
    // rule is never lowered when locking. Restore only the slots we changed
    // and only if they still contain our lock value.
    pub fn patch(&self, me: &str, action: Action) -> Option<Value> {
        if self.action()? != action {
            return None;
        }
        let c = self.content()?;
        let actor = self.actor(me)?;
        if actor < level(&c["events"], POWER, level(c, "state_default", 50)?)? {
            return None;
        }
        let default = level(c, "events_default", 0)?;
        let member = level(c, "users_default", 0)?;
        let mut out = c.clone();
        match action {
            Action::Restrict => {
                let locked = 50.max(member.checked_add(1)?);
                if locked > actor {
                    return None;
                }
                let mut saved = json!({"level":locked,"events":{}});
                if default < locked {
                    saved["events_default"] = c["events_default"].clone();
                    out["events_default"] = locked.into();
                }
                for ty in POSTS {
                    if let Some(old) = c["events"].get(ty)
                        && number(old)? < locked
                    {
                        saved["events"][ty] = old.clone();
                        out["events"][ty] = locked.into();
                    }
                }
                out[RESTORE] = saved;
            }
            Action::Restore => {
                let saved = &c[RESTORE];
                let locked = number(&saved["level"])?;
                if locked > actor || locked < 50 {
                    return None;
                }
                if let Some(old) = saved.get("events_default")
                    && c["events_default"].as_i64() == Some(locked)
                {
                    if old.is_null() {
                        out.as_object_mut()?.remove("events_default");
                    } else if number(old)? < locked {
                        out["events_default"] = old.clone();
                    } else {
                        return None;
                    }
                }
                for (ty, old) in saved["events"].as_object()? {
                    if !POSTS.contains(&ty.as_str()) || number(old)? >= locked {
                        return None;
                    }
                    if c["events"][ty].as_i64() == Some(locked) {
                        out["events"][ty] = old.clone();
                    }
                }
                out.as_object_mut()?.remove(RESTORE);
            }
            Action::Allow => {
                // A restriction created elsewhere has no restoration record.
                // Reopen messages, not unrelated defaults, stickers or reactions.
                for ty in ["m.room.message", "m.room.encrypted"] {
                    let old = level(&c["events"], ty, default)?;
                    if actor < old || actor < member {
                        return None;
                    }
                    out["events"][ty] = member.into();
                }
            }
        }
        Some(out)
    }
}

#[derive(Clone)]
pub struct Change {
    pub room: String,
    pub base: String,
    pub action: Action,
}
pub enum Flight {
    Check(Change),
    Write(Change, Value),
}
impl Flight {
    fn change(&self) -> &Change {
        match self {
            Self::Check(c) | Self::Write(c, _) => c,
        }
    }
}
impl App {
    pub fn leave_permission_panel(&mut self) {
        self.permission_panel = false;
        // Drop preflight responses that could initiate an unwanted write after
        // a room switch. Already submitted writes still receive their ack.
        self.pending
            .retain(|_, p| !matches!(p, Request::Posting(Flight::Check(_))));
    }
    pub fn posting_busy(&self, room: &str) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p,Request::Posting(f) if f.change().room==room))
    }
    pub fn change_posting(&mut self, c: Change) {
        if !self.logged_in
            || !self.verified
            || !self.connected
            || !self.permission_panel
            || self.selected != c.room
            || self.posting_busy(&c.room)
        {
            return;
        }
        let Some(r) = self.live.list.get(&c.room) else {
            return;
        };
        if r.posting.id() != Some(&c.base) || r.posting.patch(&self.matrix_id, c.action).is_none() {
            return;
        }
        let room = c.room.clone();
        self.fetch_room_state(&room, Request::Posting(Flight::Check(c)));
    }
    /// Fresh room state for a posting change: write it only if nothing
    /// changed since the change was planned.
    pub fn posting_checked(&mut self, c: Change, state: RoomState) {
        if self.selected != c.room || !self.permission_panel || !self.verified || !self.connected {
            return;
        }
        let Some(r) = self.live.list.get_mut(&c.room) else {
            return;
        };
        if state.membership(&self.matrix_id) != Some(Kind::Joined) {
            self.status = "Cannot verify your membership in this room.".into();
            return;
        }
        let fresh = state.posting();
        if r.posting.id() != Some(&c.base) || fresh.id() != Some(&c.base) {
            if r.posting.id() == Some(&c.base) {
                r.posting.adopt(fresh);
            }
            self.status =
                "Room permissions changed. Review the current levels before trying again.".into();
            return;
        }
        let Some(content) = fresh.patch(&self.matrix_id, c.action) else {
            self.status = "You cannot make this permission change.".into();
            return;
        };
        let room = c.room.clone();
        self.send_power_levels(
            &room,
            content.clone(),
            Request::Posting(Flight::Write(c, content)),
        );
    }
    /// The homeserver accepted our power levels as `event_id`.
    pub fn posting_written(&mut self, c: Change, content: Value, event_id: String) {
        let me = self.matrix_id.clone();
        let Some(r) = self.live.list.get_mut(&c.room) else {
            return;
        };
        if r.posting.id() == Some(&c.base) {
            r.posting.accepted(event_id, me, content);
        }
        if self.selected == c.room {
            self.status.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc(id: &str, sender: &str, content: Value) -> StateDoc {
        StateDoc {
            event_id: id.into(),
            sender: sender.into(),
            content,
            redacted: false,
        }
    }
    fn policy(version: &str, c: Value) -> Posting {
        let create = json!({"room_version":version,"additional_creators":["@also:h"]});
        Posting::new(
            Some(doc("$create", "@creator:h", create)),
            Some(doc("$base", "@creator:h", c)),
        )
    }
    #[test]
    fn redacting_other_messages_requires_both_thresholds() {
        let p = policy(
            "12",
            json!({
                "users": {"@member:h": 10, "@moderator:h": 55},
                "events": {"m.room.redaction": 10},
                "redact": 55
            }),
        );
        assert!(!p.can_redact_message("@guest:h", "@guest:h"));
        assert!(p.can_redact_message("@member:h", "@member:h"));
        assert!(!p.can_redact_message("@member:h", "@guest:h"));
        assert!(p.can_redact_message("@moderator:h", "@guest:h"));
        assert!(p.can_redact_message("@also:h", "@moderator:h"));

        let stricter = policy(
            "11",
            json!({
                "users": {"@moderator:h": 55},
                "events": {"m.room.redaction": 60},
                "redact": 50
            }),
        );
        assert!(!stricter.can_redact_message("@moderator:h", "@guest:h"));
        assert!(!Posting::default().can_redact_message("@member:h", "@member:h"));
    }

    #[test]
    fn permissions_follow_room_version_and_state_event_threshold() {
        let c =
            json!({"events":{POWER:75,"m.sticker":90},"users":{"@mod:h":50},"state_default":40});
        let mut p = policy("12", c.clone());
        assert!(p.patch("@creator:h", Action::Restrict).is_some());
        let restricted = p.patch("@also:h", Action::Restrict).unwrap();
        assert_eq!(restricted["events"]["m.sticker"], 90);
        assert!(p.patch("@mod:h", Action::Restrict).is_none());
        assert!(p.patch("@member:h", Action::Restrict).is_none());
        p.store_snapshot(None, Some(doc("$locked", "@also:h", restricted)));
        assert_eq!(p.can_post("@member:h", true), Some(false));
        assert_eq!(p.can_post("@mod:h", true), Some(true));
        assert_eq!(p.patch("@creator:h", Action::Restore), Some(c.clone()));
        // In older rooms a creator cannot edit after their levels were revoked.
        assert!(
            policy("11", c)
                .patch("@creator:h", Action::Restrict)
                .is_none()
        );
    }
    #[test]
    fn reacting_follows_the_level_the_homeserver_checks() {
        let p = policy(
            "11",
            json!({"users": {"@helper:h": 25, "@mod:h": 50}, "events_default": 10,
                "events": {"m.reaction": 25, "m.room.encrypted": 50}}),
        );
        assert_eq!(p.can_react("@member:h", false), Some(false));
        assert_eq!(p.can_react("@helper:h", false), Some(true));
        // Encrypted, a reaction is an m.room.encrypted event.
        assert_eq!(p.can_react("@helper:h", true), Some(false));
        assert_eq!(p.can_react("@mod:h", true), Some(true));
        // Without its own level, reacting needs events_default.
        let plain = policy(
            "11",
            json!({"users": {"@helper:h": 25}, "events_default": 10}),
        );
        assert_eq!(plain.can_react("@member:h", false), Some(false));
        assert_eq!(plain.can_react("@helper:h", false), Some(true));
        assert_eq!(Posting::default().can_react("@member:h", false), None);
    }
    #[test]
    fn an_older_snapshot_never_brings_back_a_replaced_document() {
        let doc = |id: &str| StateDoc {
            event_id: id.into(),
            sender: "@creator:h".into(),
            content: json!({}),
            redacted: false,
        };
        let mut p = Posting::new(None, Some(doc("$a")));
        p.accepted("$b".into(), "@creator:h".into(), json!({}));
        p.store_snapshot(None, Some(doc("$a")));
        assert_eq!(p.id(), Some("$b"), "the store hasn't synced our change yet");
        p.store_snapshot(None, Some(doc("$c")));
        assert_eq!(p.id(), Some("$c"), "a newer change from elsewhere");
        p.adopt(Posting::new(None, Some(doc("$d"))));
        p.store_snapshot(None, Some(doc("$c")));
        assert_eq!(p.id(), Some("$d"));
    }
    #[test]
    fn restore_touches_only_unchanged_locked_fields() {
        let mut c = json!({"users_default":50,"events_default":5,"events":{"m.room.message":10,"m.room.encrypted":76,"m.sticker":0}});
        let mut p = policy("12", c.clone());
        let mut written = p.patch("@creator:h", Action::Restrict).unwrap();
        assert_eq!(written["events_default"], 51);
        assert_eq!(written["events"]["m.room.encrypted"], 76);
        written["events"]["m.room.message"] = 87.into();
        written["kick"] = 70.into();
        p.store_snapshot(None, Some(doc("$changed", "@else:h", written)));
        c["events"]["m.room.message"] = 87.into();
        c["kick"] = 70.into();
        assert_eq!(p.patch("@creator:h", Action::Restore), Some(c));
    }
}
