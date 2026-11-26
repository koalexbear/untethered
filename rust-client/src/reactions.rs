//! Counts and ownership come from gomuks's aggregate on the annotated message.
//! A click carries the rendered intent so stale/double clicks cannot invert it.
use crate::{
    App, Request,
    rooms::{self, Post},
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
}

#[cfg(test)]
mod tests {}
