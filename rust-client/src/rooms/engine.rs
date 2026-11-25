//! Typed updates from the matrix-sdk engine (`crate::engine`). The gomuks
//! path keeps feeding `Rooms::sync` and `Room::insert` with its JSON frames;
//! both fill the same fields the view reads.
use super::{Post, Quote, Room, Rooms};
use crate::engine::{Delivery, Kind, Message, RoomList};
use chrono::{DateTime, Local};
use serde_json::{Value, json};
use std::collections::BTreeSet;

impl Rooms {
    /// Replace the joined-room list. Returns rooms that are no longer joined.
    pub fn apply_engine(&mut self, list: RoomList) -> Vec<String> {
        let mut order_changed = false;
        if self.children != list.children {
            self.children = list.children;
            self.hierarchy_revision += 1;
            order_changed = true;
        }
        let joined: BTreeSet<&str> = list.rooms.iter().map(|r| r.id.as_str()).collect();
        let left: Vec<String> = self
            .list
            .keys()
            .filter(|id| !joined.contains(id.as_str()))
            .cloned()
            .collect();
        for id in &left {
            self.list.remove(id);
            self.hierarchy_revision += 1;
            order_changed = true;
        }
        for s in list.rooms {
            let space = s.space || self.children.contains_key(&s.id);
            let new = !self.list.contains_key(&s.id);
            let room = self.list.entry(s.id).or_default();
            order_changed |= new || room.name != s.name || room.space != space || room.dm != s.dm;
            if new || room.space != space {
                self.hierarchy_revision += 1;
            }
            room.has_metadata = true;
            room.name = s.name;
            room.topic = s.topic;
            room.space = space;
            room.dm = s.dm;
            room.voice = s.voice;
            room.encrypted = s.encrypted;
            #[cfg(target_os = "linux")]
            {
                room.dm_peer = s.dm_peer;
            }
            room.posting = crate::permissions::Posting::new(s.create, s.power);
            room.read.fully_read = s.fully_read;
            room.read.marked = s.marked_unread;
            room.read.messages = s.unread_messages;
            room.read.highlights = s.unread_mentions.max(s.highlights);
            room.unread = room.read.marked
                || room.read.messages > 0
                || room.read.highlights > 0
                || s.notifications > 0;
        }
        if order_changed {
            self.sort_rooms();
        }
        left
    }
}

impl Room {
    /// Replace the loaded timeline with the engine's snapshot.
    pub fn apply_engine(&mut self, messages: Vec<Message>, start_reached: bool) {
        self.history.clear();
        self.order.clear();
        self.event_rows.clear();
        self.related.clear();
        let sent = |m: &Message| m.event_id.clone().filter(|_| m.delivery == Delivery::Sent);
        let events: Vec<String> = messages.iter().filter_map(sent).collect();
        let receipts: Vec<(String, String, i64)> = messages
            .iter()
            .filter_map(|m| Some((sent(m)?, &m.receipts)))
            .flat_map(|(id, rs)| {
                rs.iter()
                    .map(move |(user, ts)| (id.clone(), user.clone(), *ts))
            })
            .collect();
        self.read.apply_engine(
            events.iter().map(String::as_str),
            receipts
                .iter()
                .map(|(e, u, t)| (e.as_str(), u.as_str(), *t)),
        );
        // `order` is sorted by (time, row). Keep the SDK's order even when
        // origin_server_ts goes backwards by never letting the key decrease.
        let mut floor = i64::MIN;
        for m in messages {
            if let Some(name) = &m.sender_name {
                self.members.insert(m.sender.clone(), name.clone());
            }
            if let Some((sender, body)) = m.reply_to.as_ref().and_then(|r| r.quoted.clone()) {
                let id = m.reply_to.as_ref().unwrap().event_id.clone();
                self.related.insert(
                    id.clone(),
                    Quote {
                        id,
                        sender,
                        excerpt: excerpt(&body),
                    },
                );
            }
            let time = m.time.max(floor);
            floor = time;
            let row = m.row;
            if let Some(id) = &m.event_id {
                self.event_rows.insert(id.clone(), row);
            }
            self.order.push((time, row));
            self.history.insert(row, post(m, time));
        }
        self.end = start_reached;
    }
}

fn post(m: Message, time: i64) -> Post {
    let stamp = DateTime::from_timestamp_millis(m.time)
        .unwrap_or_default()
        .with_timezone(&Local);
    let media = m
        .media
        .as_ref()
        .filter(|_| m.kind == Kind::Attachment)
        .and_then(crate::media::Media::parse);
    let mut body = match m.kind {
        Kind::Text | Kind::Sticker => m.body,
        // The file card shows the attachment; the body is its name or caption.
        Kind::Attachment if media.is_some() => m.body,
        Kind::Attachment => format!("Attachment · {}", m.body),
        Kind::Redacted => "Message removed".into(),
        Kind::Undecryptable => "Waiting for encryption keys…".into(),
    };
    match m.delivery {
        Delivery::Sent => {}
        Delivery::Sending => body.push_str("  ·  Sending…"),
        Delivery::Failed => body.push_str("  ·  Failed to send"),
    }
    let event_id = m.event_id.filter(|_| m.delivery == Delivery::Sent);
    let revision = (m.edited && m.kind == Kind::Text).then(|| {
        let id = m.edit_id.or_else(|| event_id.clone()).unwrap_or_default();
        std::sync::Arc::new(crate::edits::Revision::applied(body.clone(), id))
    });
    Post {
        reactable: event_id.is_some()
            && matches!(m.kind, Kind::Text | Kind::Attachment | Kind::Sticker),
        chips: m
            .reactions
            .into_iter()
            .map(|(key, count, mine)| (key, crate::reactions::engine_chip(count, mine)))
            .collect(),
        // Only gates the Edit button, so follow the SDK's own rule.
        text_message: m.text && m.kind == Kind::Text && m.editable,
        encrypted: m.encrypted,
        mentions: m.mentions.map_or(
            Value::Null,
            |(users, room)| json!({"user_ids": users, "room": room}),
        ),
        revision,
        event_id,
        reply: m.reply_to.map(|r| r.event_id),
        media,
        sender: m.sender,
        body,
        time,
        day: stamp.date_naive(),
        day_label: stamp.format("%B %-d, %Y").to_string(),
        clock: stamp.format("%H:%M").to_string(),
    }
}

// Same excerpt as Post::quote.
fn excerpt(text: &str) -> String {
    let mut chars = text.trim().chars();
    let mut excerpt: String = chars
        .by_ref()
        .take(72)
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    if chars.next().is_some() {
        excerpt.push('…');
    }
    excerpt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Reply, RoomSummary};
    use std::collections::BTreeMap;

    fn message(row: i64, time: i64, body: &str) -> Message {
        Message {
            row,
            event_id: Some(format!("${row}")),
            sender: "@a:example".into(),
            sender_name: Some("Alice".into()),
            time,
            body: body.into(),
            kind: Kind::Text,
            encrypted: true,
            delivery: Delivery::Sent,
            edited: false,
            reply_to: None,
            reactions: Vec::new(),
            text: true,
            editable: false,
            mentions: None,
            edit_id: None,
            receipts: Vec::new(),
            media: None,
        }
    }

    #[test]
    fn room_list_fills_spaces_dms_and_unread_state() {
        let mut rooms = Rooms::default();
        let room = |id: &str, name: &str| RoomSummary {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let left = rooms.apply_engine(RoomList {
            rooms: vec![
                room("!a", "Zulu"),
                RoomSummary {
                    dm: true,
                    unread_messages: 2,
                    ..room("!b", "bravo")
                },
                RoomSummary {
                    space: true,
                    ..room("!s", "Space")
                },
                RoomSummary {
                    highlights: 1,
                    ..room("!c", "charlie")
                },
            ],
            children: BTreeMap::from([("!s".into(), ["!a".into(), "!b".into()].into())]),
        });
        assert!(left.is_empty());
        assert_eq!(rooms.ordered_rooms, ["!b", "!c", "!a"]);
        assert_eq!(rooms.ordered_direct, ["!b"]);
        assert_eq!(rooms.ordered_spaces["!s"], ["!b", "!a"]);
        assert!(rooms.list["!b"].unread && rooms.list["!b"].read.messages == 2);
        assert_eq!(rooms.list["!c"].read.highlights, 1);
        assert!(!rooms.list["!a"].unread);

        let left = rooms.apply_engine(RoomList {
            rooms: vec![room("!b", "bravo"), room("!s", "Space")],
            children: BTreeMap::from([("!s".into(), ["!a".into(), "!b".into()].into())]),
        });
        assert_eq!(left, ["!a", "!c"]);
        assert_eq!(rooms.ordered_spaces["!s"], ["!b"]);
    }

    #[test]
    fn timeline_snapshot_keeps_sdk_order_and_states() {
        let mut room = Room::default();
        let mut reply = message(3, 50, "reply");
        reply.reply_to = Some(Reply {
            event_id: "$0".into(),
            quoted: Some(("@b:example".into(), "older\nmessage".into())),
        });
        let mut sending = message(4, 300, "pending");
        sending.delivery = Delivery::Sending;
        let mut edited = message(5, 400, "new text");
        edited.edited = true;
        room.apply_engine(
            vec![
                message(1, 100, "first"),
                message(2, 200, "second"),
                reply,
                sending,
                edited,
            ],
            false,
        );
        let posts: Vec<_> = room.posts().map(|p| p.text().to_owned()).collect();
        assert_eq!(
            posts,
            [
                "first",
                "second",
                "reply",
                "pending  ·  Sending…",
                "new text"
            ]
        );
        // The out-of-order timestamp keeps its place and its own clock time.
        assert_eq!(room.position("$3"), Some(2));
        assert_eq!(room.history[&3].time, 200);
        assert!(room.history[&4].event_id.is_none());
        assert!(room.history[&5].revision.is_some());
        assert_eq!(room.quote("$0").unwrap().excerpt, "older message");
        assert_eq!(room.display("@a:example"), "Alice");
        assert!(!room.end);
        room.apply_engine(vec![message(1, 100, "first")], true);
        assert_eq!(room.posts().len(), 1);
        assert!(room.end);
    }
}
