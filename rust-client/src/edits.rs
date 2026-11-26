//! Matrix replacements change the body of a timeline event, not its identity
//! or reply relation. The composer draft stays separate from an in-place edit.
use crate::{
    App, Request,
    rooms::{self, Post},
};
use iced::widget::text_editor;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

pub struct Revision {
    pub(super) event_id: String,
    sender: String,
    encrypted: bool,
    timestamp: i64,
    body: String,
    mentions: Value,
}
impl Revision {
    /// A replacement the engine has already applied: matrix-sdk-ui's timeline
    /// aggregates edits and reports the latest text.
    /// `event_id` is the newest edit, which reports refer to.
    pub fn applied(body: String, event_id: String) -> Self {
        Self {
            event_id,
            sender: String::new(),
            encrypted: false,
            timestamp: 0,
            body,
            mentions: Value::Null,
        }
    }
}
#[derive(Default)]
pub struct Replacements(BTreeMap<String, BTreeMap<String, Arc<Revision>>>);

fn content(e: &Value) -> &Value {
    if e["type"] == "m.room.encrypted" {
        &e["decrypted"]
    } else {
        &e["content"]
    }
}
pub fn is_replacement(e: &Value) -> bool {
    e["relation_type"] == "m.replace" || content(e)["m.relates_to"]["rel_type"] == "m.replace"
}
fn target(e: &Value) -> Option<&str> {
    content(e)["m.relates_to"]["event_id"]
        .as_str()
        .or(e["relates_to"].as_str())
}
impl Replacements {
    // Cache independently of timeline order: sync may deliver an edit before
    // its message; on reload gomuks may return it only in related_events.
    pub fn ingest(&mut self, e: &Value) -> Option<String> {
        let c = content(e);
        let rel = &c["m.relates_to"];
        let id = e["event_id"]
            .as_str()
            .filter(|_| rooms::server_confirmed(e))?;
        let retracted = e["redacted_by"].as_str().is_some_and(|s| !s.is_empty());
        let target = if retracted {
            target(e)?
        } else {
            rel["event_id"].as_str()?
        };
        if retracted {
            if let Some(revs) = self.0.get_mut(target) {
                revs.remove(id);
            }
        } else {
            let encrypted = e["type"] == "m.room.encrypted";
            if rel["rel_type"] != "m.replace"
                || !target.starts_with('$')
                || !e["state_key"].is_null()
                || (if encrypted {
                    &e["decrypted_type"]
                } else {
                    &e["type"]
                }) != "m.room.message"
                || c["m.new_content"]["msgtype"] != "m.text"
            {
                return None;
            }
            let new = &c["m.new_content"];
            let rev = Revision {
                event_id: id.into(),
                sender: e["sender"].as_str()?.into(),
                encrypted,
                timestamp: e["timestamp"].as_i64()?,
                body: new["body"].as_str()?.into(),
                mentions: new["m.mentions"].clone(),
            };
            self.0
                .entry(target.to_owned())
                .or_default()
                .insert(id.into(), Arc::new(rev));
        }
        Some(target.to_owned())
    }
    pub fn apply(&self, p: &mut Post) {
        p.revision = p
            .event_id
            .as_ref()
            .filter(|_| p.text_message)
            .and_then(|id| self.0.get(id))
            .into_iter()
            .flatten()
            .filter(|(_, r)| r.sender == p.sender && r.encrypted == p.encrypted)
            .max_by(|(a, x), (b, y)| x.timestamp.cmp(&y.timestamp).then(a.cmp(b)))
            .map(|(_, r)| r.clone());
    }
}
impl Post {
    pub fn current_mentions(&self) -> &Value {
        self.revision
            .as_ref()
            .map_or(&self.mentions, |r| &r.mentions)
    }
    pub fn text(&self) -> &str {
        self.revision.as_ref().map_or(&self.body, |r| &r.body)
    }
    pub fn may_edit(&self, me: &str) -> bool {
        self.text_message && !me.is_empty() && self.sender == me && self.event_id.is_some()
    }
}

pub struct Editing {
    pub room: String,
    pub event: String,
    pub input: text_editor::Content,
    pub pending: Option<i64>,
}
impl App {
    pub fn start_edit(&mut self, id: String) -> bool {
        let Some(p) = self
            .visible_post(&id)
            .filter(|p| p.may_edit(&self.matrix_id))
        else {
            return false;
        };
        if !self.connected
            || !self.verified
            || self.editing.is_some()
            || !self
                .live
                .list
                .get(&self.selected)
                .is_some_and(|r| r.has_metadata && r.encrypted == p.encrypted)
        {
            return false;
        }
        self.editing = Some(Editing {
            room: self.selected.clone(),
            input: text_editor::Content::with_text(p.text()),
            event: id,
            pending: None,
        });
        true
    }
    pub fn save_edit(&mut self) {
        let Some(d) = self
            .editing
            .as_ref()
            .filter(|d| d.room == self.selected && d.pending.is_none())
        else {
            return;
        };
        let Some(post) = self
            .visible_post(&d.event)
            .filter(|p| p.may_edit(&self.matrix_id))
        else {
            return;
        };
        let Some(room) = self.live.list.get(&d.room) else {
            return;
        };
        if !self.verified
            || !self.connected
            || !room.has_metadata
            || post.encrypted != room.encrypted
        {
            return;
        }
        let body = d.input.text();
        if body.trim().is_empty() || body == post.text() {
            return;
        }
        let mentions = post.current_mentions();
        let mut new = json!({"body":body,"msgtype":"m.text"});
        if mentions.is_object() {
            new["m.mentions"] = mentions.clone();
        }
        let params = json!({"room_id":d.room,"type":"m.room.message","synchronous":true,
            "content":{"msgtype":"m.text","body":format!("* {body}"),"m.new_content":new,
                "m.mentions":{},"m.relates_to":{"rel_type":"m.replace","event_id":d.event}}});
        self.rpc(
            "send_event",
            params,
            Request::Edit(d.room.clone(), d.event.clone()),
        );
        if self.pending.contains_key(&self.seq) {
            self.editing.as_mut().unwrap().pending = Some(self.seq);
        }
    }
    pub fn finish_edit(&mut self, seq: i64, room: &str, id: &str, event: &Value) {
        let ok = rooms::server_confirmed(event)
            && event["room_id"] == room
            && event["sender"] == self.matrix_id
            && self
                .live
                .list
                .get(room)
                .is_some_and(|r| r.encrypted == (event["type"] == "m.room.encrypted"))
            && content(event)["m.relates_to"]["event_id"] == id
            && content(event)["m.relates_to"]["rel_type"] == "m.replace";
        if ok {
            if let Some(r) = self.live.list.get_mut(room) {
                r.insert(event);
            }
            if let Some(slice) = self
                .original
                .as_mut()
                .filter(|c| c.room == room)
                .and_then(|c| c.slice.as_mut())
            {
                slice.insert(event);
            }
        }
        if let Some(d) = &mut self.editing
            && d.pending == Some(seq)
        {
            if ok {
                self.editing = None;
                self.status.clear();
            } else {
                d.pending = None;
                self.status = "Could not save the edit. Your changes have been kept.".into();
            }
        }
    }
}

impl crate::replies::Original {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rooms::Room;

    fn base() -> Value {
        json!({"room_id":"!r","rowid":1,"event_id":"$reply","timestamp":1,"sender":"@alice:h","type":"m.room.encrypted",
        "decrypted_type":"m.room.message","decrypted":{"msgtype":"m.text","body":"first","m.relates_to":{"m.in_reply_to":{"event_id":"$original"}}}})
    }
    fn edit(row: i64, body: &str) -> Value {
        json!({"room_id":"!r","rowid":row,"event_id":format!("$e{row}"),"timestamp":row,"sender":"@alice:h","type":"m.room.encrypted",
        "decrypted_type":"m.room.message","decrypted":{"msgtype":"m.text","body":"* fallback","m.relates_to":{"rel_type":"m.replace","event_id":"$reply"},
            "m.new_content":{"msgtype":"m.text","body":body,"m.relates_to":{"m.in_reply_to":{"event_id":"$wrong"}}}}})
    }
    #[test]
    fn replacements_change_only_text_and_require_the_original_sender_and_encryption() {
        let mut room = Room::default();
        room.insert(&edit(3, "newest"));
        room.insert(&edit(2, "previous"));
        room.insert(&base());
        let mut foreign = edit(4, "foreign");
        foreign["sender"] = json!("@other:h");
        room.insert(&foreign);
        let mut plain = edit(5, "clear");
        plain["type"] = json!("m.room.message");
        plain["content"] = plain["decrypted"].take();
        room.insert(&plain);
        let mut unsent = edit(6, "pending");
        unsent["event_id"] = json!("~txn");
        room.insert(&unsent);
        let check = |room: &Room, expected: &str| {
            assert_eq!(room.history.len(), 1);
            let p = room.by_id("$reply").unwrap();
            assert_eq!(p.text(), expected);
            assert_eq!(p.reply.as_deref(), Some("$original"));
            assert_eq!(p.time, 1);
            assert!(!p.may_edit("@other:h"));
        };
        check(&room, "newest");
        let mut retracted = edit(3, "newest");
        retracted["redacted_by"] = json!("$removed");
        room.insert(&retracted);
        check(&room, "previous");
    }
}
