//! Typed updates from the matrix-sdk engine (`crate::engine`). The gomuks
//! path keeps feeding `Rooms::sync` and `Room::insert` with its JSON frames;
//! both fill the same fields the view reads.
use super::{Post, Quote, Room, Rooms};
use crate::engine::{Delivery, Kind, Message, RoomList};
use chrono::{DateTime, Local};
use serde_json::{Value, json};
use std::collections::BTreeSet;

impl Rooms {
    /// Replace the joined-room list. Returns rooms that are no longer joined,
    /// or were replaced by a joined room in an upgrade (upgrades.rs).
    pub fn apply_engine(&mut self, mut list: RoomList) -> Vec<String> {
        self.invites = std::mem::take(&mut list.invites);
        let mut order_changed = false;
        let replaced = crate::upgrades::replaced(&list.rooms);
        list.rooms.retain(|r| !replaced.contains_key(&r.id));
        if self.replaced != replaced {
            self.replaced = replaced;
            self.hierarchy_revision += 1;
            order_changed = true;
        }
        // Edges written here show until sync brings them back (channel_order.rs).
        let edges = list
            .rooms
            .iter_mut()
            .filter(|r| r.space)
            .map(|r| (r.id.clone(), std::mem::take(&mut r.child_order)))
            .collect();
        let (children, edges) = self.arranging.synced(list.children, edges);
        if self.children != children {
            self.children = children;
            self.hierarchy_revision += 1;
            order_changed = true;
        }
        if self.edges != edges {
            self.edges = edges;
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
            room.avatar = s.avatar;
            room.topic = s.topic;
            room.space = space;
            room.dm = s.dm;
            room.voice = s.voice;
            room.encrypted = s.encrypted;
            room.link_previews = s.link_previews;
            room.successor = s.successor;
            room.dm_peer = s.dm_peer;
            #[cfg(target_os = "linux")]
            room.replace_voice_members(&s.call_members);
            room.posting.store_snapshot(s.create, s.power);
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            {
                let alerts = s
                    .space_alerts
                    .as_ref()
                    .map(crate::notifications::spaces::mode)
                    .unwrap_or_default();
                if room.space_alerts != alerts {
                    room.space_alerts = alerts;
                    self.hierarchy_revision += 1;
                }
            }
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

/// Sent events in the order reading reaches them: the messages in timeline
/// order, and each message's latest edit after the last message sent before
/// it. An edit has no message of its own, but a read marker can be on one.
fn read_order(messages: &[Message], sent: impl Fn(&Message) -> Option<String>) -> Vec<String> {
    let mut order: Vec<(String, i64)> = messages
        .iter()
        .filter_map(|m| Some((sent(m)?, m.time)))
        .collect();
    let mut edits: Vec<(String, i64)> = messages
        .iter()
        .filter(|m| sent(m).is_some())
        .filter_map(|m| Some((m.edit_id.clone()?, m.edit_time?)))
        .collect();
    edits.sort_by_key(|(_, time)| *time);
    for (edit, time) in edits {
        let at = order
            .iter()
            .rposition(|(_, t)| *t <= time)
            .map_or(0, |p| p + 1);
        order.insert(at, (edit, time));
    }
    order.into_iter().map(|(id, _)| id).collect()
}

impl Room {
    /// Replace the loaded timeline with the engine's snapshot.
    pub fn apply_engine(&mut self, messages: Vec<Message>, start_reached: bool) {
        // The previous posts by row, to keep their parsed formatting.
        let previous = std::mem::take(&mut self.history);
        self.order.clear();
        self.event_rows.clear();
        self.related.clear();
        // Read state is the live room's alone; older rooms' messages have none.
        let sent = |m: &Message| {
            m.event_id
                .clone()
                .filter(|_| m.delivery == Delivery::Sent && m.from.is_none())
        };
        let events = read_order(&messages, sent);
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
            if !self.profile_members.contains(&m.sender) {
                if let Some(name) = &m.sender_name {
                    self.members.insert(m.sender.clone(), name.clone());
                }
                if let Some(avatar) = &m.sender_avatar {
                    self.avatars.insert(m.sender.clone(), avatar.clone());
                }
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
            self.history.insert(row, post(m, time, previous.get(&row)));
        }
        self.end = start_reached;
    }
}

/// What an undecryptable message says: waiting for its key, or why it
/// won't come.
fn unreadable(why: crate::engine::Unreadable) -> &'static str {
    use crate::engine::Unreadable;
    match why {
        Unreadable::Waiting => "Waiting for encryption keys…",
        Unreadable::BeforeJoined => "Can't decrypt · sent before you joined",
        Unreadable::NoBackup => "Can't decrypt · sent before this device, and key backup is off",
        Unreadable::Unverified => "Can't decrypt · verify this device to read older messages",
        Unreadable::Withheld => "Can't decrypt · the sender didn't share the key with this device",
        Unreadable::Untrusted => "Can't decrypt · sent from a device you don't trust",
        Unreadable::Malformed => "Can't decrypt · the message is malformed",
    }
}

fn post(m: Message, time: i64, previous: Option<&Post>) -> Post {
    let stamp = DateTime::from_timestamp_millis(m.time)
        .unwrap_or_default()
        .with_timezone(&Local);
    let media = m
        .media
        .as_ref()
        .filter(|_| m.kind == Kind::Attachment)
        .and_then(crate::media::Media::parse);
    let mut body = match m.kind {
        // A poll's card shows it; the body is its question.
        Kind::Text | Kind::Sticker | Kind::Poll => m.body,
        // The file card shows the attachment; the body is its name or caption.
        Kind::Attachment if media.is_some() => m.body,
        Kind::Attachment => format!("Attachment · {}", m.body),
        Kind::Redacted => "Message removed".into(),
        Kind::Undecryptable(why) => unreadable(why).into(),
    };
    let content = body.len();
    let note = match m.delivery {
        Delivery::Sent => None,
        Delivery::Sending => Some("  ·  Sending…"),
        Delivery::Failed => Some("  ·  Failed to send"),
    };
    if let Some(note) = note {
        body.push_str(note);
    }
    // Every snapshot carries every loaded message; parse each body once.
    let rich = match previous {
        _ if m.kind != Kind::Text => None,
        Some(p) if p.body == body && p.formatted == m.formatted => p.rich.clone(),
        _ => crate::rich::Doc::message(m.formatted.as_deref(), &body[..content]).map(|mut doc| {
            if let Some(note) = note {
                doc.note(note);
            }
            std::sync::Arc::new(doc)
        }),
    };
    let event_id = m.event_id.filter(|_| m.delivery == Delivery::Sent);
    // Older rooms' messages are read-only: no reactions, edits or replies.
    let live = m.from.is_none();
    let revision = (m.edited && m.kind == Kind::Text).then(|| {
        let id = m.edit_id.or_else(|| event_id.clone()).unwrap_or_default();
        std::sync::Arc::new(crate::edits::Revision::applied(body.clone(), id))
    });
    let message = event_id.is_some()
        && matches!(
            m.kind,
            Kind::Text | Kind::Attachment | Kind::Sticker | Kind::Poll
        );
    Post {
        reactable: live && message,
        // Element doesn't forward polls either: its votes stay with it.
        forwardable: message && m.kind != Kind::Poll,
        chips: m
            .reactions
            .into_iter()
            .map(|(key, count, mine)| (key, crate::reactions::engine_chip(count, mine)))
            .collect(),
        // Only gates the Edit button, so follow the SDK's own rule.
        text_message: live && m.text && m.kind == Kind::Text && m.editable,
        from: m.from,
        encrypted: m.encrypted,
        mentions: m.mentions.map_or(
            Value::Null,
            |(users, room)| json!({"user_ids": users, "room": room}),
        ),
        revision,
        event_id,
        reply: m.reply_to.map(|r| r.event_id),
        media,
        poll: m.poll.filter(|_| m.kind == Kind::Poll),
        sender: m.sender,
        body,
        formatted: m.formatted,
        rich,
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
            sender_avatar: None,
            time,
            body: body.into(),
            formatted: None,
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
            edit_time: None,
            receipts: Vec::new(),
            media: None,
            from: None,
            poll: None,
        }
    }

    #[test]
    fn keeping_the_newest_posts_moves_the_seams_with_them() {
        let mut r = Room::default();
        r.apply_engine(
            (1..=10).map(|n| message(n, n, &n.to_string())).collect(),
            true,
        );
        let seam = |index| crate::engine::Seam {
            index,
            older: "!old:example".into(),
            newer: "!new:example".into(),
            time: None,
            readable: true,
        };
        r.seams = vec![seam(3), seam(8)];
        r.keep_newest(4);
        let bodies: Vec<_> = r.posts().map(|p| p.body.as_str()).collect();
        assert_eq!(bodies, ["7", "8", "9", "10"]);
        assert_eq!(r.seams.iter().map(|s| s.index).collect::<Vec<_>>(), [2]);
        assert!(!r.end, "what went can load again");
        assert!(r.by_id("$3").is_none() && r.by_id("$9").is_some());
    }

    #[test]
    fn an_undecryptable_message_says_whether_its_key_may_still_come() {
        use crate::engine::Unreadable;
        let mut m = message(1, 1, "");
        m.kind = Kind::Undecryptable(Unreadable::Waiting);
        assert_eq!(
            post(m.clone(), 1, None).body,
            "Waiting for encryption keys…"
        );
        m.kind = Kind::Undecryptable(Unreadable::BeforeJoined);
        assert_eq!(
            post(m, 1, None).body,
            "Can't decrypt · sent before you joined"
        );
    }

    /// A read marker on an edit, which has no row, keeps its place: the
    /// divider goes before the first message sent after the edit.
    #[test]
    fn read_marker_on_an_edit_keeps_its_place() {
        let mut room = Room::default();
        let edited = |mut m: Message, time| {
            m.edit_id = Some(format!("{}-edit", m.event_id.clone().unwrap()));
            m.edit_time = Some(time);
            m
        };
        // The newest message was edited, then another message arrived.
        room.apply_engine(
            vec![
                message(1, 100, "first"),
                edited(message(2, 200, "revised"), 250),
                message(3, 300, "after"),
            ],
            true,
        );
        room.read.fully_read = Some("$2-edit".into());
        assert_eq!(room.read.divider(&room), Some("$3"));
        // An older message edited after newer ones: they are read too.
        room.apply_engine(
            vec![
                edited(message(1, 100, "first"), 350),
                message(2, 200, "second"),
                message(3, 300, "third"),
            ],
            true,
        );
        room.read.fully_read = Some("$1-edit".into());
        assert_eq!(room.read.divider(&room), None);
    }

    /// Messages from an older room above an upgrade's seam (upgrades.rs):
    /// shown, but read-only and outside the live room's read order.
    #[test]
    fn older_rooms_messages_are_read_only_and_never_unread() {
        let mut room = Room::default();
        let editable = |m: Message| Message {
            editable: true,
            ..m
        };
        let old = Message {
            from: Some("!old".into()),
            ..editable(message(1, 100, "before"))
        };
        room.apply_engine(vec![old, editable(message(2, 200, "after"))], true);
        let (before, after) = (&room.history[&1], &room.history[&2]);
        assert_eq!(before.from.as_deref(), Some("!old"));
        assert!(!before.reactable && !before.text_message);
        assert!(after.reactable && after.text_message && after.from.is_none());
        room.read.fully_read = Some("$1".into());
        assert_eq!(room.read.divider(&room), None);
        room.read.fully_read = Some("$0".into());
        room.read.set_latest(Some("$2".into()));
        assert_eq!(room.read.target(), Some("$2"));
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
            invites: Vec::new(),
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
            invites: Vec::new(),
            rooms: vec![room("!b", "bravo"), room("!s", "Space")],
            children: BTreeMap::from([("!s".into(), ["!a".into(), "!b".into()].into())]),
        });
        assert_eq!(left, ["!a", "!c"]);
        assert_eq!(rooms.ordered_spaces["!s"], ["!b"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn voice_room_members_follow_store_snapshots() {
        let now = crate::desktop_call::now();
        let member = |user: &str, device: &str, ts: u64, content: serde_json::Value| {
            serde_json::json!({"type":untethered_iced::rtc_membership::EVENT_TYPE,
                "state_key":format!("_{user}_{device}_m.call"),"sender":user,
                "event_id":format!("${device}"),"origin_server_ts":ts,"content":content})
        };
        let joined = |device: &str| {
            serde_json::json!({"application":"m.call","scope":"m.room","call_id":"",
                "device_id":device,"focus_active":{"type":"livekit"}})
        };
        let voice = |members: Vec<serde_json::Value>| RoomList {
            invites: Vec::new(),
            rooms: vec![RoomSummary {
                id: "!v".into(),
                name: "Lounge".into(),
                voice: true,
                call_members: members,
                ..Default::default()
            }],
            children: BTreeMap::new(),
        };
        let mut rooms = Rooms::default();
        // Element Call omits created_ts on a first join: the event's own
        // timestamp starts the lease. An expired lease and a left member
        // ({}) don't count.
        rooms.apply_engine(voice(vec![
            member("@a:x", "A", now - 1_000, joined("A")),
            member("@b:x", "B", now - 15_000_000, joined("B")),
            member("@c:x", "C", now - 1_000, serde_json::json!({})),
        ]));
        assert_eq!(rooms.list["!v"].voice_users(), ["@a:x"].into());
        rooms.apply_engine(voice(vec![member("@a:x", "A", now, serde_json::json!({}))]));
        assert!(rooms.list["!v"].voice_users().is_empty());
    }

    #[test]
    fn formatted_bodies_render_rich_and_parse_once() {
        let mut room = Room::default();
        let bold = |row| Message {
            formatted: Some("<b>hi</b> there".into()),
            ..message(row, 100, "**hi** there")
        };
        let mut sending = message(2, 200, "plain");
        sending.formatted = Some("<i>plain</i>".into());
        sending.delivery = Delivery::Sending;
        room.apply_engine(vec![bold(1), sending, message(3, 300, "just text")], true);
        let first = room.history[&1].rich.clone().expect("formatted");
        assert_eq!(room.history[&1].text(), "**hi** there");
        let Some(crate::rich::Block::Paragraph(runs)) =
            room.history[&2].rich.as_ref().map(|d| &d.blocks[0])
        else {
            panic!("a paragraph");
        };
        assert_eq!(runs.last().unwrap().text, "  ·  Sending…");
        assert!(room.history[&3].rich.is_none());
        // The next snapshot keeps the parsed body; a changed one parses again.
        room.apply_engine(vec![bold(1), message(2, 200, "edited")], true);
        assert!(std::sync::Arc::ptr_eq(
            &first,
            room.history[&1].rich.as_ref().unwrap()
        ));
        assert!(room.history[&2].rich.is_none());
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
