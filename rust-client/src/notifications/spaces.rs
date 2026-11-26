//! Matrix has no space push-rule scope. Store the user's preference on the
//! space, then maintain ordinary *underride* rules on joined descendants.
//! Room, mention and DND rules retain their native Matrix priority.
use super::{Mode, RuleChange, RuleKind};
use crate::{
    App, Request, Section,
    rooms::{Rooms, items},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PREF: &str = "com.untethered.space_alerts.v1";
const PREFIX: &str = "com.untethered.space_alerts.v1.";
pub fn mode(content: &Value) -> Mode {
    match content["mode"].as_str() {
        Some("all") => Mode::All,
        Some("quiet") => Mode::Quiet,
        _ => Mode::Default,
    }
}
fn rule_id(room: &str) -> String {
    format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(room))
}
fn conditions(room: &str) -> Value {
    json!([{"kind":"event_property_is","key":"room_id","value":room}])
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Installed {
    mode: Mode,
    enabled: bool,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Write {
    room: String,
    mode: Mode,
    enable: bool,
}
impl Write {
    /// The underride rule change this write makes.
    fn change(&self) -> RuleChange {
        if self.enable {
            RuleChange::Enabled(true)
        } else if self.mode != Mode::Default {
            RuleChange::Put {
                actions: if self.mode == Mode::All {
                    json!(["notify"])
                } else {
                    json!([])
                },
                conditions: Some(conditions(&self.room)),
            }
        } else {
            RuleChange::Delete
        }
    }
}
#[derive(Default)]
pub struct Spaces {
    pub panel: bool,
    pub initialized: bool,
    revision: Option<u64>,
    desired: BTreeMap<String, (usize, Mode)>,
    installed: BTreeMap<String, Installed>,
    conflicts: BTreeSet<String>,
    failed: BTreeMap<String, Write>,
    dirty: bool,
}
impl Spaces {
    pub(super) fn rules(&mut self, rules: Option<&Value>) {
        self.installed.clear();
        self.conflicts.clear();
        self.dirty = true;
        for rule in rules.into_iter().flat_map(items) {
            let Some(key) = rule["rule_id"]
                .as_str()
                .and_then(|s| s.strip_prefix(PREFIX))
            else {
                continue;
            };
            let Some(room) = URL_SAFE_NO_PAD
                .decode(key)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .filter(|s| s.starts_with('!'))
            else {
                continue;
            };
            if rule["default"] == true
                || rule["conditions"] != conditions(&room)
                || (rule["actions"] != json!(["notify"]) && rule["actions"] != json!([]))
            {
                self.conflicts.insert(room);
                continue;
            }
            self.installed.insert(
                room,
                Installed {
                    mode: if rule["actions"] == json!([]) {
                        Mode::Quiet
                    } else {
                        Mode::All
                    },
                    enabled: rule["enabled"] == true,
                },
            );
        }
    }
    fn update_graph(&mut self, rooms: &Rooms) {
        if self.revision == Some(rooms.hierarchy_revision) {
            return;
        }
        self.revision = Some(rooms.hierarchy_revision);
        self.dirty = true;
        self.desired.clear();
        for (space, r) in &rooms.list {
            if !r.space || r.space_alerts == Mode::Default {
                continue;
            }
            for (id, distance) in rooms.channels(space) {
                let entry = self
                    .desired
                    .entry(id.into())
                    .or_insert((distance, r.space_alerts));
                if distance < entry.0 || (distance == entry.0 && r.space_alerts == Mode::Quiet) {
                    *entry = (distance, r.space_alerts);
                }
            }
        }
    }
    fn action(&self, room: &str) -> Option<Write> {
        if self.conflicts.contains(room) {
            return None;
        }
        let stored = self.installed.get(room);
        let mode = self.inherited(room);
        let change = match mode {
            Mode::Default => stored.is_some(),
            _ => stored.is_none_or(|r| r.mode != mode || !r.enabled),
        };
        change.then(|| Write {
            room: room.into(),
            mode,
            enable: stored.is_some_and(|r| r.mode == mode && !r.enabled),
        })
    }
    fn pending_write(&mut self) -> Option<Write> {
        // One server rejection mustn't prevent an unrelated room from
        // updating. Revisit the rejected operation only on retry or a change.
        let failed = std::mem::take(&mut self.failed);
        self.failed = failed
            .into_iter()
            .filter(|(id, w)| self.action(id).as_ref() == Some(w))
            .collect();
        self.desired
            .keys()
            .chain(self.installed.keys())
            .filter(|id| !self.failed.contains_key(*id))
            .find_map(|id| self.action(id))
    }
    pub fn inherited(&self, room: &str) -> Mode {
        self.desired.get(room).map_or(Mode::Default, |(_, m)| *m)
    }
    pub fn conflict(&self) -> bool {
        self.conflicts
            .iter()
            .any(|id| self.desired.contains_key(id))
    }
    pub fn failed(&self) -> bool {
        !self.failed.is_empty()
    }
    /// The space alert rule installed for a room, as the push rules say.
    #[cfg(test)]
    pub fn installed(&self, room: &str) -> Option<Mode> {
        self.installed
            .get(room)
            .filter(|r| r.enabled)
            .map(|r| r.mode)
    }
}
impl App {
    pub fn retry_space_rules(&mut self) {
        self.alerts.spaces.failed.clear();
        self.alerts.spaces.dirty = true;
        self.sync_space_rules();
    }
    pub fn set_space_mode(&mut self, space: String, m: Mode) {
        if !self.verified
            || !self.connected
            || !self.logged_in
            || !self.alerts.spaces.initialized
            || !self.alerts.ready
            || !matches!(&self.section,Section::Space(id) if *id==space)
            || self
                .pending
                .values()
                .any(|p| matches!(p,Request::SpacePref(id) if *id==space))
            || !self
                .live
                .list
                .get(&space)
                .is_some_and(|r| r.space && r.space_alerts != m)
        {
            return;
        }
        self.alerts.spaces.failed.clear();
        let content = json!({"mode": match m {
            Mode::All => "all",
            Mode::Quiet => "quiet",
            Mode::Default => "default",
        }});
        let target = space.clone();
        self.send_room_account_data(&target, PREF, content, Request::SpacePref(space));
    }
    pub fn sync_space_rules(&mut self) {
        let s = &mut self.alerts.spaces;
        if !s.initialized
            || !self.alerts.ready
            || !self.logged_in
            || !self.connected
            || !self.verified
            || self.closing
        {
            return;
        }
        s.update_graph(&self.live);
        if !s.dirty
            || self
                .pending
                .values()
                .any(|p| matches!(p, Request::SpaceRule(_)))
        {
            return;
        }
        s.dirty = false;
        let Some(write) = s.pending_write() else {
            return;
        };
        let rule = rule_id(&write.room);
        self.send_push_rule(
            RuleKind::Underride,
            &rule,
            write.change(),
            Request::SpaceRule(write.clone()),
        );
        if !self.pending.contains_key(&self.seq) {
            self.finish_space_rule(write, false);
        }
    }
    pub fn finish_space_rule(&mut self, write: Write, ok: bool) {
        let s = &mut self.alerts.spaces;
        s.dirty = true;
        if !ok {
            s.failed.insert(write.room.clone(), write);
            return;
        }
        if write.mode == Mode::Default {
            s.installed.remove(&write.room);
        } else {
            // PUT on an existing disabled rule keeps its enabled flag.
            let enabled = write.enable || s.installed.get(&write.room).is_none_or(|r| r.enabled);
            s.installed.insert(
                write.room,
                Installed {
                    mode: write.mode,
                    enabled,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rooms::Room;
    #[test]
    fn shortest_joined_path_wins_and_quiet_breaks_ties_even_with_cycles() {
        let mut rooms = Rooms::default();
        for (id, pref) in [
            ("!a", Mode::Quiet),
            ("!b", Mode::All),
            ("!sub", Mode::Default),
        ] {
            rooms.list.insert(
                id.into(),
                Room {
                    space: true,
                    space_alerts: pref,
                    ..Default::default()
                },
            );
        }
        rooms.list.insert("!chat".into(), Room::default());
        for (space, kids) in [
            ("!a", vec!["!sub", "!absent"]),
            ("!sub", vec!["!a", "!chat"]),
            ("!b", vec!["!chat"]),
            ("!absent", vec!["!chat"]),
        ] {
            rooms
                .children
                .insert(space.into(), kids.into_iter().map(str::to_owned).collect());
        }
        let mut s = Spaces::default();
        s.update_graph(&rooms);
        assert_eq!(rooms.channels("!a")["!chat"], 2);
        assert!(s.inherited("!chat") == Mode::All);
        // "!a" now holds the channel directly.
        rooms
            .children
            .insert("!a".into(), ["!chat", "!sub"].map(String::from).into());
        rooms.hierarchy_revision += 1;
        s.update_graph(&rooms);
        assert!(s.inherited("!chat") == Mode::Quiet);
        for left in ["!a", "!b"] {
            rooms.list.remove(left);
            rooms.children.remove(left);
        }
        rooms.hierarchy_revision += 1;
        s.update_graph(&rooms);
        assert!(s.inherited("!chat") == Mode::Default);
    }

    #[test]
    fn preserve_customized_rules_and_continue_after_an_unrelated_rejection() {
        let mut s = Spaces::default();
        for id in ["!a", "!b", "!c"] {
            s.desired.insert(id.into(), (1, Mode::Quiet));
        }
        s.rules(Some(
            &json!([{"rule_id":rule_id("!a"),"conditions":conditions("!a"),
            "enabled":true,"actions":[{"set_tweak":"sound","value":"custom"}]}]),
        ));
        let write = s.pending_write().unwrap();
        assert_eq!(write.room, "!b");
        s.failed.insert(write.room.clone(), write);
        assert_eq!(s.pending_write().unwrap().room, "!c");
        assert!(s.conflict());
        s.desired.clear();
        assert!(s.pending_write().is_none() && !s.failed());
    }
}
