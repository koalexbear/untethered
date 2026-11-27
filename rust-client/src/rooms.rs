use chrono::{DateTime, Local, NaiveDate};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
mod engine;
pub const VOICE_ROOM: &str = "org.matrix.msc3417.call";

pub fn items(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}
pub fn string(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}
pub fn short(id: &str) -> &str {
    id.trim_start_matches('@').split(':').next().unwrap_or(id)
}

#[derive(Default)]
pub struct Rooms {
    pub list: BTreeMap<String, Room>,
    pub(super) ordered_rooms: Vec<String>,
    pub(super) ordered_direct: Vec<String>,
    pub(super) ordered_spaces: BTreeMap<String, Vec<String>>,
    pub children: BTreeMap<String, BTreeSet<String>>,
    pub hierarchy_revision: u64,
    /// Old rooms and the joined rooms that replaced them in an upgrade; the
    /// old ones aren't in `list` (upgrades.rs).
    pub(crate) replaced: BTreeMap<String, String>,
}
#[derive(Default)]
pub struct Room {
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    pub space_alerts: crate::notifications::Mode,
    pub posting: crate::permissions::Posting,
    pub name: String,
    pub topic: String,
    pub space: bool,
    pub dm: bool,
    pub voice: bool,
    #[cfg(target_os = "linux")]
    pub voice_members: BTreeMap<String, (String, u64)>,
    #[cfg(target_os = "linux")]
    pub dm_peer: Option<String>,
    pub unread: bool,
    pub read: crate::reading::State,
    pub encrypted: bool,
    pub has_metadata: bool,
    pub end: bool,
    pub members: BTreeMap<String, String>,
    /// The room's picture (mxc URI).
    pub avatar: Option<String>,
    /// Members' profile pictures (mxc URIs), as the timeline shows them.
    pub avatars: BTreeMap<String, String>,
    /// The room that replaced this one, when this account hasn't joined it
    /// yet: the room is read-only and offers to join it (upgrades.rs).
    pub successor: Option<String>,
    /// Where older rooms' history meets the next room's in `order`.
    pub seams: Vec<crate::engine::Seam>,
    pub history: BTreeMap<i64, Post>,
    // Keep the timestamp and row-ID ordering used by the timeline.
    pub(super) order: Vec<(i64, i64)>,
    pub(super) event_rows: BTreeMap<String, i64>,
    pub(super) related: BTreeMap<String, Quote>,
    pub(super) edits: crate::edits::Replacements,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Quote {
    pub id: String,
    pub sender: String,
    pub excerpt: String,
}
impl Quote {
    pub fn relate(&self, params: &mut Value) {
        params["relates_to"] = json!({"m.in_reply_to":{"event_id":self.id}});
        params["mentions"] = json!({"user_ids":[self.sender]});
    }
}
pub struct Post {
    pub reactable: bool,
    pub chips: crate::reactions::Chips,
    /// The older room this message belongs to, above an upgrade's seam.
    /// Such messages are read-only (upgrades.rs).
    pub from: Option<String>,
    pub(super) text_message: bool,
    pub(super) encrypted: bool,
    pub(super) mentions: Value,
    pub revision: Option<std::sync::Arc<crate::edits::Revision>>,
    pub event_id: Option<String>,
    pub reply: Option<String>,
    pub media: Option<crate::media::Media>,
    pub sender: String,
    /// Plain text: what search, reply quotes, copying and notifications use.
    pub body: String,
    /// The `org.matrix.custom.html` body, kept to reuse `rich` while it and
    /// `body` stay the same across timeline snapshots.
    pub(super) formatted: Option<String>,
    /// The body with its formatting (rich.rs). None when it reads the same
    /// as plain text.
    pub rich: Option<std::sync::Arc<crate::rich::Doc>>,
    pub time: i64,
    pub day: NaiveDate,
    pub day_label: String,
    pub clock: String,
}
impl Post {
    pub fn quote(&self) -> Option<Quote> {
        let mut chars = self.text().trim().chars();
        let mut excerpt: String = chars
            .by_ref()
            .take(72)
            .map(|c| if c.is_whitespace() { ' ' } else { c })
            .collect();
        if chars.next().is_some() {
            excerpt.push('…');
        }
        Some(Quote {
            id: self.event_id.clone()?,
            sender: self.sender.clone(),
            excerpt,
        })
    }
}
// gomuks assigns ~txn IDs to local echoes. A $ ID comes from the homeserver.
// In v0.2611.1, actuallySend updates the ID but can leave the initial "not sent"
// sentinel on both the completion event and the persisted row.
pub fn server_confirmed(e: &Value) -> bool {
    e["event_id"]
        .as_str()
        .is_some_and(|s| s.starts_with('$') && s.len() > 1)
}
pub fn real_send_error(e: &Value) -> bool {
    !server_confirmed(e)
        && e["send_error"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s != "not sent")
}
impl Room {
    pub fn by_id(&self, id: &str) -> Option<&Post> {
        self.event_rows.get(id).and_then(|r| self.history.get(r))
    }
    pub fn quote(&self, id: &str) -> Option<Quote> {
        self.by_id(id)
            .and_then(Post::quote)
            .or_else(|| self.related.get(id).cloned())
    }
    #[cfg(target_os = "linux")]
    pub fn voice_users(&self) -> BTreeSet<&str> {
        let now = crate::desktop_call::now();
        self.voice_members
            .values()
            .filter(|(_, expires)| *expires > now)
            .map(|(user, _)| user.as_str())
            .collect()
    }
    /// `e` is a call member state event in client-server format.
    #[cfg(target_os = "linux")]
    pub fn voice_state(&mut self, e: &Value) {
        if !self.voice || e["type"] != untethered_iced::rtc_membership::EVENT_TYPE {
            return;
        }
        let Some(key) = e["state_key"].as_str() else {
            return;
        };
        self.voice_members.remove(key);
        if crate::desktop_call::active_member(e)
            && let Some(user) = e["sender"].as_str()
        {
            let expires = e["content"]["created_ts"]
                .as_u64()
                .or(e["origin_server_ts"].as_u64())
                .unwrap_or(0)
                .saturating_add(e["content"]["expires"].as_u64().unwrap_or(14_400_000));
            self.voice_members
                .insert(key.into(), (user.into(), expires));
        }
    }
    /// All of a voice room's call memberships, as the engine's store holds them.
    #[cfg(target_os = "linux")]
    pub fn replace_voice_members(&mut self, events: &[Value]) {
        self.voice_members.clear();
        for e in events {
            self.voice_state(e);
        }
    }
    pub fn already_confirmed(&self, e: &Value) -> bool {
        e["rowid"]
            .as_i64()
            .and_then(|id| self.history.get(&id))
            .is_some_and(|p| p.event_id.is_some())
    }
    pub fn insert(&mut self, e: &Value) {
        let Some(row) = e["rowid"].as_i64().filter(|&r| r > 0) else {
            return;
        };
        // An opaque encrypted row can later decrypt to a reaction, call signal
        // or edit. It must not remain an independent, replyable message.
        let edit = crate::edits::is_replacement(e);
        if e["type"] == "m.room.encrypted"
            && e["decrypted_type"]
                .as_str()
                .is_some_and(|ty| !matches!(ty, "m.room.message" | "m.sticker"))
            || edit
        {
            if let Some(old) = self.history.remove(&row) {
                self.remove_order(old.time, row);
                if let Some(id) = old.event_id {
                    self.event_rows.remove(&id);
                }
            }
            if edit
                && let Some(target) = self.edits.ingest(e)
                && let Some(p) = self
                    .event_rows
                    .get(&target)
                    .and_then(|row| self.history.get_mut(row))
            {
                self.edits.apply(p);
            }
            return;
        }
        let body = match e["type"].as_str() {
            Some("m.room.encrypted") => e["decrypted"]["body"]
                .as_str()
                .unwrap_or("Waiting for encryption keys…"),
            Some("m.room.message" | "m.sticker") => {
                e["content"]["body"].as_str().unwrap_or("[Attachment]")
            }
            _ => return,
        };
        let mut body = body.to_owned();
        let confirmed = server_confirmed(e);
        // A late local echo must not overwrite the synced event for this row.
        if !confirmed && self.already_confirmed(e) {
            return;
        }
        let redacted = e["redacted_by"].as_str().is_some_and(|s| !s.is_empty());
        let content = if e["type"] == "m.room.encrypted" {
            &e["decrypted"]
        } else {
            &e["content"]
        };
        let relation = &content["m.relates_to"];
        let reply = relation["m.in_reply_to"]["event_id"]
            .as_str()
            .filter(|id| {
                !redacted
                    && relation["is_falling_back"] != true
                    && id.starts_with('$')
                    && id.len() > 1
            })
            .map(str::to_owned);
        if reply.is_some()
            && body.starts_with("> <")
            && let Some((fallback, message)) = body.split_once("\n\n")
            && fallback.lines().all(|s| s.starts_with('>'))
        {
            body = message.to_owned();
        }
        if redacted {
            body = "Message removed".into();
        }
        let media = if redacted {
            None
        } else {
            crate::media::Media::parse(content)
        };
        let formatted = content["formatted_body"]
            .as_str()
            .filter(|_| content["format"] == "org.matrix.custom.html")
            .map(str::to_owned);
        let mut rich = (!redacted && media.is_none())
            .then(|| crate::rich::Doc::message(formatted.as_deref(), &body))
            .flatten();
        let note = if real_send_error(e) {
            Some("  ·  Failed to send")
        } else if !confirmed && e["send_error"] == "not sent" {
            Some("  ·  Sending…")
        } else {
            None
        };
        if let Some(note) = note {
            body.push_str(note);
            if let Some(doc) = &mut rich {
                doc.note(note);
            }
        }
        let event_id = confirmed.then(|| string(&e["event_id"]));
        if let Some(id) = &event_id {
            self.event_rows.insert(id.clone(), row);
        }
        let time = e["timestamp"].as_i64().unwrap_or(0);
        let stamp = DateTime::from_timestamp_millis(time)
            .unwrap_or_default()
            .with_timezone(&Local);
        let mut post = Post {
            reactable: confirmed
                && !redacted
                && e["state_key"].is_null()
                && (matches!(e["type"].as_str(), Some("m.room.message" | "m.sticker"))
                    || matches!(
                        e["decrypted_type"].as_str(),
                        Some("m.room.message" | "m.sticker")
                    )),
            chips: crate::reactions::chips(e),
            from: None,
            text_message: !redacted
                && e["state_key"].is_null()
                && content["msgtype"] == "m.text"
                && content["body"].is_string()
                && (e["type"] == "m.room.message" || e["decrypted_type"] == "m.room.message"),
            encrypted: e["type"] == "m.room.encrypted",
            mentions: content["m.mentions"].clone(),
            revision: None,
            event_id,
            reply,
            media,
            sender: string(&e["sender"]),
            body,
            formatted,
            rich: rich.map(std::sync::Arc::new),
            time,
            day: stamp.date_naive(),
            day_label: stamp.format("%B %-d, %Y").to_string(),
            clock: stamp.format("%H:%M").to_string(),
        };
        self.edits.apply(&mut post);
        let previous = self.history.insert(row, post);
        if let Some(old_id) = previous.as_ref().and_then(|p| p.event_id.as_ref())
            && self.history[&row].event_id.as_ref() != Some(old_id)
        {
            self.event_rows.remove(old_id);
        }
        if previous.as_ref().is_none_or(|p| p.time != time) {
            if let Some(previous) = previous {
                self.remove_order(previous.time, row);
            }
            let key = (time, row);
            let index = self.order.partition_point(|p| *p < key);
            self.order.insert(index, key);
        }
    }
    fn remove_order(&mut self, time: i64, row: i64) {
        if let Ok(index) = self.order.binary_search(&(time, row)) {
            self.order.remove(index);
        }
    }
    pub fn posts(&self) -> impl ExactSizeIterator<Item = &Post> + DoubleEndedIterator {
        self.posts_in(0..self.order.len())
    }
    pub fn posts_in(
        &self,
        range: std::ops::Range<usize>,
    ) -> impl ExactSizeIterator<Item = &Post> + DoubleEndedIterator {
        self.order[range]
            .iter()
            .map(|&(_, row)| &self.history[&row])
    }
    pub fn position(&self, event: &str) -> Option<usize> {
        let row = *self.event_rows.get(event)?;
        self.order
            .binary_search(&(self.history.get(&row)?.time, row))
            .ok()
    }
    pub fn key_at(&self, index: usize) -> Option<(i64, i64)> {
        self.order.get(index).copied()
    }
    pub fn index_at(&self, key: (i64, i64)) -> usize {
        self.order.partition_point(|p| *p < key)
    }
    pub fn display<'a>(&'a self, sender: &'a str) -> &'a str {
        self.members
            .get(sender)
            .map(String::as_str)
            .unwrap_or_else(|| short(sender))
    }
}
impl Rooms {
    /// Shortest path to each joined channel. A room may have several parents
    /// and spaces can contain cycles; never traverse through unjoined spaces.
    /// An upgraded child counts as its newest joined room.
    pub fn channels(&self, space: &str) -> BTreeMap<&str, usize> {
        let mut channels = BTreeMap::new();
        let Some((id, r)) = self.list.get_key_value(space) else {
            return channels;
        };
        if !r.space {
            return channels;
        }
        let mut visited = BTreeSet::from([id.as_str()]);
        let mut todo = std::collections::VecDeque::from([(id.as_str(), 0)]);
        while let Some((id, n)) = todo.pop_front() {
            for child in self.children.get(id).into_iter().flatten() {
                // A child that was upgraded is its newest room.
                let child = self.newest(child);
                if !visited.insert(child) {
                    continue;
                }
                if let Some(r) = self.list.get(child) {
                    if r.space {
                        todo.push_back((child, n + 1));
                    } else {
                        channels.insert(child, n + 1);
                    }
                }
            }
        }
        channels
    }
    fn sort_rooms(&mut self) {
        let mut rooms: Vec<_> = self.list.iter().filter(|(_, r)| !r.space).collect();
        rooms.sort_by_cached_key(|(id, r)| (r.name.to_lowercase(), *id));
        self.ordered_direct = rooms
            .iter()
            .filter(|(_, room)| room.dm)
            .map(|(id, _)| (*id).clone())
            .collect();
        let rank: BTreeMap<_, _> = rooms
            .iter()
            .enumerate()
            .map(|(index, (id, _))| (id.as_str(), index))
            .collect();
        self.ordered_spaces = self
            .children
            .iter()
            .map(|(space, _)| {
                let mut indexes: Vec<_> = self
                    .channels(space)
                    .into_keys()
                    .filter_map(|id| rank.get(id).copied())
                    .collect();
                indexes.sort_unstable();
                let children = indexes.into_iter().map(|i| rooms[i].0.clone()).collect();
                (space.clone(), children)
            })
            .collect();
        self.ordered_rooms = rooms.into_iter().map(|(id, _)| id.clone()).collect();
    }

    pub fn demo() -> Self {
        let mut rooms = Self::default();
        let space = "!workshop";
        let mut add = |id: &str, name: &str, topic: &str, dm: bool| {
            rooms.list.insert(
                id.into(),
                Room {
                    name: name.into(),
                    topic: topic.into(),
                    space: id == space,
                    dm,
                    end: true,
                    ..Room::default()
                },
            );
        };
        add(space, "The Workshop", "", false);
        add(
            "!general",
            "general",
            "The common room for ideas, updates, and hellos.",
            false,
        );
        add(
            "!design",
            "design-lab",
            "A place for work in progress.",
            false,
        );
        add(
            "!music",
            "off-topic",
            "Music, recommendations, and the occasional tangent.",
            false,
        );
        add("!dm", "Mira", "Direct message", true);
        rooms.children.insert(
            space.into(),
            ["!general", "!design", "!music"].map(String::from).into(),
        );
        let now = chrono::Utc::now().timestamp_millis();
        for (i,(room,sender,body)) in [
            ("!general","Mira","Morning! I made a little space for us to try out the new client."),
            ("!general","Jules","This already feels like our usual hangout. I found the room without thinking about it."),
            ("!general","Jules","And the design sketches have their own channel now."),
            ("!general","River","I’m here too. Try sending a message below, or jump over to #design-lab."),
            ("!design","River","First sketch: server rail on the left, the conversation front and center."),
            ("!music","Jules","What’s everyone listening to while they build?"),
            ("!dm","Mira","Found the direct messages! This conversation is just a local preview."),
        ].into_iter().enumerate() {
            rooms.list.get_mut(room).unwrap().insert(&json!({"rowid":i+1,"type":"m.room.message","sender":sender,"timestamp":now-((8-i) as i64)*60000,"content":{"body":body}}));
        }
        rooms.list.get_mut("!design").unwrap().unread = true;
        rooms.sort_rooms();
        rooms
    }
}

#[cfg(test)]
mod tests {}
