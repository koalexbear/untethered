//! Member posting presets are changes to real room power levels. Patch the
//! current document: roles, state-event rules and unknown fields belong to
//! the room, not to this editor.
use crate::{
    App, Request,
    moderation::Kind,
    room_state::RoomState,
    rooms::{items, server_confirmed},
};
use serde_json::{Value, json};

const CREATE: &str = "m.room.create";
const POWER: &str = "m.room.power_levels";
const RESTORE: &str = "com.untethered.posting_restore.v1";
const POSTS: [&str; 4] = [
    "m.room.message",
    "m.room.encrypted",
    "m.sticker",
    "m.reaction",
];
// Matrix integers must survive a canonical JSON round trip.
const MAX: i64 = (1 << 53) - 1;
fn number(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|n| (-MAX..=MAX).contains(n))
}
fn level(map: &Value, key: &str, default: i64) -> Option<i64> {
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
impl StateDoc {
    /// A confirmed state event of `room` from gomuks.
    pub fn from_gomuks(room: &str, e: &Value) -> Option<Self> {
        if e["room_id"] != room || !server_confirmed(e) {
            return None;
        }
        Some(Self {
            event_id: e["event_id"].as_str()?.into(),
            sender: e["sender"].as_str().unwrap_or_default().into(),
            content: e["content"].clone(),
            redacted: e["redacted_by"].is_string(),
        })
    }
}
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
    /// A state event from gomuks. `initial` keeps what is already known.
    pub fn ingest(&mut self, room: &str, e: &Value, initial: bool) {
        if e["state_key"] != "" {
            return;
        }
        let slot = match e["type"].as_str() {
            Some(CREATE) => &mut self.create,
            Some(POWER) => &mut self.power,
            _ => return,
        };
        if let Some(doc) = StateDoc::from_gomuks(room, e)
            && (!initial || slot.is_none())
        {
            *slot = Some(doc);
        }
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
    fn content(&self) -> Option<&Value> {
        let doc = self.power.as_ref()?;
        let c = &doc.content;
        (!doc.redacted
            && c.is_object()
            && ["users", "events"]
                .iter()
                .all(|k| c.get(k).is_none_or(Value::is_object)))
        .then_some(c)
    }
    fn actor(&self, me: &str) -> Option<i64> {
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
    pub fn can_link_channel(&self, me: &str) -> bool {
        self.content()
            .and_then(|c| {
                level(
                    &c["events"],
                    "m.space.child",
                    level(c, "state_default", 50)?,
                )
            })
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
    /// gomuks's answer to a posting check or write.
    pub fn posting_result(&mut self, f: Flight, data: &Value) {
        match f {
            Flight::Check(c) => {
                let state = RoomState::from_gomuks(&c.room, data);
                self.posting_checked(c, state);
            }
            Flight::Write(c, content) => match data.as_str().filter(|id| id.starts_with('$')) {
                Some(id) => self.posting_written(c, content, id.to_owned()),
                None => self.status = "Posting change was not confirmed.".into(),
            },
        }
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
    fn event(ty: &str, id: &str, sender: &str, c: Value) -> Value {
        json!({"room_id":"!r","type":ty,"state_key":"","sender":sender,"event_id":id,"content":c})
    }
    fn policy(version: &str, c: Value) -> Posting {
        let mut p = Posting::default();
        p.ingest(
            "!r",
            &event(
                CREATE,
                "$create",
                "@creator:h",
                json!({"room_version":version,"additional_creators":["@also:h"]}),
            ),
            false,
        );
        p.ingest("!r", &event(POWER, "$base", "@creator:h", c), false);
        p
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
        p.ingest("!r", &event(POWER, "$locked", "@also:h", restricted), false);
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
        p.ingest("!r", &event(POWER, "$changed", "@else:h", written), false);
        c["events"]["m.room.message"] = 87.into();
        c["kick"] = 70.into();
        assert_eq!(p.patch("@creator:h", Action::Restore), Some(c));
    }
    #[test]
    fn preflight_does_not_submit_a_stale_power_document() {
        let (tx, rx) = async_channel::bounded(8);
        let mut app = App::new(tx);
        app.connected = true;
        app.logged_in = true;
        app.verified = true;
        app.selected = "!r".into();
        app.matrix_id = "@creator:h".into();
        app.permission_panel = true;
        let p = policy("12", json!({}));
        let create = event(
            CREATE,
            "$create",
            "@creator:h",
            json!({"room_version":"12","additional_creators":["@also:h"]}),
        );
        app.live.list.entry("!r".into()).or_default().posting = p;
        app.change_posting(Change {
            room: "!r".into(),
            base: "$base".into(),
            action: Action::Restrict,
        });
        let req = rx.try_recv().unwrap().into_rpc();
        assert_eq!(req["command"], "get_room_state");
        assert_eq!(req["data"]["refetch"], true);
        let member = json!({"room_id":"!r","type":"m.room.member","event_id":"$join","state_key":"@creator:h","content":{"membership":"join"}});
        app.frame(json!({"command":"response","request_id":req["request_id"],"data":[create,member,event(POWER,"$new","@else:h",json!({"ban":90}))]}));
        assert!(rx.try_recv().is_err());
        assert_eq!(app.live.list["!r"].posting.id(), Some("$new"));
    }
}
