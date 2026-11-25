//! Counts and ownership come from gomuks's aggregate on the annotated message.
//! A click carries the rendered intent so stale/double clicks cannot invert it.
use crate::{
    App, Request,
    rooms::{self, Post, Room},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const QUICK: [(&str, &str); 6] = [
    ("👍", "Like"),
    ("❤️", "Love"),
    ("🎉", "Celebrate"),
    ("😂", "Laugh"),
    ("👀", "Eyes"),
    ("✅", "Done"),
];
pub struct Picker {
    pub target: String,
    pub custom: String,
}
pub fn label(key: &str) -> String {
    if let Some((_, name)) = QUICK.iter().find(|(k, _)| *k == key) {
        format!("{key} {name}")
    } else {
        key.into()
    }
}
#[derive(Default)]
pub struct Chip {
    pub count: u64,
    mine: BTreeSet<String>,
}
impl Chip {
    pub fn mine(&self) -> Option<&str> {
        self.mine.first().map(String::as_str)
    }
}
pub type Chips = BTreeMap<String, Chip>;
pub fn chips(event: &Value) -> Chips {
    let mut chips = Chips::new();
    if event["redacted_by"]
        .as_str()
        .is_some_and(|id| !id.is_empty())
    {
        return chips;
    }
    for (key, n) in event["reactions"].as_object().into_iter().flatten() {
        let Some(count) = n.as_u64().filter(|n| *n > 0) else {
            continue;
        };
        if !valid_key(key) {
            continue;
        }
        let mine = rooms::items(&event["own_reactions"][key])
            .filter_map(Value::as_str)
            .filter(|id| id.starts_with('$') && id.len() > 1)
            .map(str::to_owned)
            .collect();
        chips.insert(key.clone(), Chip { count, mine });
    }
    chips
}
fn valid_key(key: &str) -> bool {
    !key.trim().is_empty() && key.len() <= 255 && !key.chars().any(char::is_control)
}

/// A chip from an engine that aggregates reactions itself (matrix-sdk-ui).
/// It toggles by key, so our own reaction needs no event ID.
pub fn engine_chip(count: u64, mine: bool) -> Chip {
    Chip {
        count,
        mine: if mine {
            BTreeSet::from(["own".to_owned()])
        } else {
            BTreeSet::new()
        },
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Toggle {
    room: String,
    target: String,
    key: String,
    remove: Option<String>,
}
impl Toggle {
    /// Room, reacted-to event and key.
    pub fn parts(&self) -> (&str, &str, &str) {
        (&self.room, &self.target, &self.key)
    }
    pub fn for_post(room: &str, post: &Post, key: &str) -> Option<Self> {
        if !post.reactable || !valid_key(key) {
            return None;
        }
        Some(Self {
            room: room.into(),
            target: post.event_id.clone()?,
            key: key.into(),
            remove: post.chips.get(key).and_then(Chip::mine).map(str::to_owned),
        })
    }
    // A synchronous server acknowledgement and the aggregate sync can arrive
    // in either order. Apply only the missing ownership transition.
    fn confirm(&self, room: &mut Room, ack_id: &str) {
        let Some(p) = room
            .event_rows
            .get(&self.target)
            .and_then(|r| room.history.get_mut(r))
        else {
            return;
        };
        if !p.reactable {
            return;
        }
        if let Some(removed) = &self.remove {
            if let Some(chip) = p.chips.get_mut(&self.key)
                && chip.mine.remove(removed)
            {
                chip.count = chip.count.saturating_sub(1);
                if chip.count == 0 {
                    p.chips.remove(&self.key);
                }
            }
        } else {
            let chip = p.chips.entry(self.key.clone()).or_default();
            if chip.mine.insert(ack_id.into()) {
                chip.count = chip.count.saturating_add(1);
            }
        }
    }
}
impl App {
    pub fn reaction_pending(&self, target: &str, key: &str) -> bool {
        self.pending.values().any(|r|matches!(r,Request::Reaction(op) if op.room==self.selected && op.target==target && op.key==key))
    }
    pub fn toggle_reaction(&mut self, op: Toggle) {
        if self.selected != op.room
            || !self.logged_in
            || !self.verified
            || !self.connected
            || self.reaction_pending(&op.target, &op.key)
        {
            return;
        }
        let Some(p) = self.visible_post(&op.target) else {
            return;
        };
        if Toggle::for_post(&op.room, p, &op.key).as_ref() != Some(&op) {
            return;
        }
        self.reaction_picker = None;
        let (cmd, params) = if let Some(id) = &op.remove {
            ("redact_event", json!({"room_id":op.room,"event_id":id}))
        } else {
            (
                "send_event",
                json!({"room_id":op.room,"type":"m.reaction","synchronous":true,"content":{
                "m.relates_to":{"rel_type":"m.annotation","event_id":op.target,"key":op.key}}}),
            )
        };
        self.rpc(cmd, params, Request::Reaction(op));
    }
    pub fn finish_reaction(&mut self, op: Toggle, data: &Value) {
        let rel = &data["content"]["m.relates_to"];
        let accepted = rooms::server_confirmed(data)
            && (op.remove.is_some()
                || (data["type"] == "m.reaction"
                    && data["sender"] == self.matrix_id
                    && data["room_id"] == op.room
                    && rel["rel_type"] == "m.annotation"
                    && rel["event_id"] == op.target
                    && rel["key"] == op.key));
        if !accepted {
            if self.selected == op.room {
                self.status = "Could not change this reaction. Try again.".into();
            }
            return;
        }
        let id = data["event_id"].as_str().unwrap();
        if let Some(room) = self.live.list.get_mut(&op.room) {
            op.confirm(room, id);
        }
        if let Some(slice) = self
            .original
            .as_mut()
            .filter(|c| c.room == op.room)
            .and_then(|c| c.slice.as_mut())
        {
            op.confirm(slice, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_and_ack_in_either_order_keep_peer_and_own_annotations_distinct() {
        for sync_first in [true, false] {
            let (tx, rx) = async_channel::bounded(8);
            let mut app = App::new(tx);
            app.logged_in = true;
            app.verified = true;
            app.connected = true;
            app.matrix_id = "@me:h".into();
            app.selected = "!r".into();
            let base = json!({"room_id":"!r","event_id":"$reply","rowid":2,"sender":"@them:h","type":"m.room.encrypted","decrypted_type":"m.room.message",
               "decrypted":{"msgtype":"m.text","body":"reply","m.relates_to":{"m.in_reply_to":{"event_id":"$parent"}}},"reactions":{"👍":1}});
            app.live.sync(&json!({"rooms": {"!r": {
                "timeline": [{"event_rowid": 2}],
                "events": [&base]
            }}}));
            let op = Toggle::for_post("!r", app.visible_post("$reply").unwrap(), "👍").unwrap();
            app.toggle_reaction(op.clone());
            app.toggle_reaction(op.clone());
            let rpc = rx.try_recv().unwrap().into_rpc();
            assert!(rx.try_recv().is_err());
            let sent = json!({"event_id":"$mine","room_id":"!r","sender":"@me:h","type":"m.reaction","content":rpc["data"]["content"]});
            let ack = json!({"command":"response","request_id":rpc["request_id"],"data":sent});
            let mut both = base.clone();
            both["reactions"]["👍"] = 2.into();
            both["own_reactions"] = json!({"👍":["$mine"]});
            let sync = |base: &Value| json!({"command":"sync_complete","data":{"rooms":{"!r":{"events":[base]}}}});
            if sync_first {
                app.frame(sync(&both));
                app.frame(ack)
            } else {
                app.frame(ack);
                app.frame(sync(&both))
            }
            assert_eq!(app.visible_post("$reply").unwrap().chips["👍"].count, 2);
            app.toggle_reaction(op); // stale Add button is not a Remove instruction
            assert!(rx.try_recv().is_err());
            let remove = Toggle::for_post("!r", app.visible_post("$reply").unwrap(), "👍").unwrap();
            app.toggle_reaction(remove);
            let rpc = rx.try_recv().unwrap().into_rpc();
            assert_eq!(rpc["command"], "redact_event");
            assert_eq!(rpc["data"]["event_id"], "$mine");
            let ack = json!({"command":"response","request_id":rpc["request_id"],"data":{"event_id":"$redaction"}});
            if sync_first {
                app.frame(sync(&base));
                app.frame(ack)
            } else {
                app.frame(ack);
                app.frame(sync(&base))
            }
            let chip = &app.visible_post("$reply").unwrap().chips["👍"];
            assert_eq!(chip.count, 1);
            assert!(chip.mine().is_none());
        }
    }
}
