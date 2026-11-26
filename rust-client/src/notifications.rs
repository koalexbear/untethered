//! gomuks evaluates Matrix push rules on decrypted content. A room rule can
//! mute ordinary chat without overriding the higher-priority mention rule.
use crate::{App, Msg, Request, rooms::items};
#[cfg(target_arch = "wasm32")]
pub mod browser;
#[cfg(target_os = "linux")]
mod desktop;
pub mod spaces;
use iced::Task;
use iced::time::Instant;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
#[cfg(target_os = "linux")]
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Default,
    All,
    Quiet,
}
impl Mode {
    pub const CHOICES: [Self; 3] = [Self::Default, Self::All, Self::Quiet];
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Account default",
            Self::All => "All messages",
            Self::Quiet => "Mute ordinary",
        }
    }
}
#[derive(Clone)]
pub struct Change {
    room: String,
    mode: Mode,
    enable_next: bool,
}
/// The Matrix push rule kinds the App writes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum RuleKind {
    Override,
    Room,
    Underride,
}
impl RuleKind {}
/// A change to one push rule, as the Matrix push rules API makes it.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum RuleChange {
    Enabled(bool),
    Delete,
    /// Create or replace the rule with these Matrix actions, and conditions
    /// for override and underride rules.
    Put {
        actions: Value,
        conditions: Option<Value>,
    },
}
#[derive(Default)]
pub struct Alerts {
    pub spaces: spaces::Spaces,
    pub panel: bool,
    #[cfg(target_arch = "wasm32")]
    pub asking: bool,
    pub ready: bool,
    // None if the homeserver hasn't supplied an unmodified suppress-all master rule.
    pub dnd: Option<bool>,
    dnd_revision: u64,
    rules: BTreeMap<String, (Mode, bool)>,
    recent: VecDeque<String>,
    waiting: VecDeque<(String, String, Instant)>,
    outgoing: Vec<Notice>,
    pub delivery_failed: bool,
}
struct Notice {
    title: String,
    body: String,
    sound: bool,
    #[cfg(target_arch = "wasm32")]
    room: String,
    #[cfg(target_arch = "wasm32")]
    event: String,
}
impl Alerts {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// The account's push rules: the `m.push_rules` account data content.
    pub fn ruleset(&mut self, content: &Value) {
        let Some(rules) = content["global"].as_object() else {
            return;
        };
        self.ready = true;
        self.spaces.rules(rules.get("underride"));
        self.set_dnd(
            rules
                .get("override")
                .and_then(Value::as_array)
                .and_then(|rs| {
                    rs.iter().find(|r| {
                        r["rule_id"] == MASTER
                            && r["default"] == true
                            && r["conditions"].as_array().is_some_and(Vec::is_empty)
                            && r["actions"]
                                .as_array()
                                .is_some_and(|a| a.is_empty() || a.as_slice() == ["dont_notify"])
                    })
                })
                .and_then(|r| r["enabled"].as_bool()),
        );
        self.rules.clear();
        for r in rules
            .get("room")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = r["rule_id"].as_str() {
                self.rules.insert(
                    id.into(),
                    (
                        if items(&r["actions"]).any(|a| a == "notify") {
                            Mode::All
                        } else {
                            Mode::Quiet
                        },
                        r["enabled"] == true,
                    ),
                );
            }
        }
    }
    pub fn mode(&self, room: &str) -> Mode {
        self.rules
            .get(room)
            .filter(|(_, on)| *on)
            .map_or(Mode::Default, |(mode, _)| *mode)
    }
    fn set_dnd(&mut self, value: Option<bool>) {
        if self.dnd != value {
            self.dnd_revision += 1;
            self.dnd = value;
        }
        if value == Some(true) {
            self.waiting.clear();
            self.outgoing.clear();
        }
    }
}
fn single_line(s: &str, limit: usize) -> String {
    s.chars()
        .take(limit)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
const MASTER: &str = ".m.rule.master";
impl App {
    pub fn changing_dnd(&self) -> bool {
        self.pending.values().any(|r| matches!(r, Request::Dnd(..)))
    }
    pub fn quiet_alerts(&self) -> bool {
        self.alerts.dnd == Some(true)
            || self
                .pending
                .values()
                .any(|r| matches!(r, Request::Dnd(true, _)))
    }
    pub fn change_dnd(&mut self, on: bool) {
        if !self.connected
            || !self.logged_in
            || !self.verified
            || self.alerts.dnd != Some(!on)
            || self.changing_dnd()
        {
            return;
        }
        let request = Request::Dnd(on, self.alerts.dnd_revision);
        self.send_push_rule(RuleKind::Override, MASTER, RuleChange::Enabled(on), request);
        if on && self.pending.contains_key(&self.seq) {
            self.apply_quiet();
        }
    }
    fn apply_quiet(&mut self) {
        self.alerts.outgoing.clear();
        self.alerts.waiting.clear();
        #[cfg(target_os = "linux")]
        self.inbox.silence_chime();
    }
    pub fn saved_dnd(&mut self, on: bool, revision: u64) {
        // A later change synchronized from another device wins over a late RPC response.
        if revision == self.alerts.dnd_revision {
            self.alerts.set_dnd(Some(on));
        }
        if self.quiet_alerts() {
            self.apply_quiet();
        }
    }
    pub fn notifications_pending(&self, room: &str) -> bool {
        self.pending
            .values()
            .any(|req| matches!(req,Request::NotifyRule(c) if c.room==room))
    }
    pub fn set_notifications(&mut self, room: String, mode: Mode) {
        if self.selected != room
            || !self.logged_in
            || !self.verified
            || !self.connected
            || !self.alerts.ready
            || !self.live.list.contains_key(&room)
            || self.notifications_pending(&room)
            || self.alerts.mode(&room) == mode
        {
            return;
        }
        let change = match mode {
            Mode::Default => RuleChange::Delete,
            Mode::All => RuleChange::Put {
                actions: json!(["notify"]),
                conditions: None,
            },
            Mode::Quiet => RuleChange::Put {
                actions: json!([]),
                conditions: None,
            },
        };
        let enable_next =
            mode != Mode::Default && self.alerts.rules.get(&room).is_some_and(|(_, on)| !*on);
        let id = room.clone();
        self.send_push_rule(
            RuleKind::Room,
            &id,
            change,
            Request::NotifyRule(Change {
                room,
                mode,
                enable_next,
            }),
        );
    }
    pub fn saved_notifications(&mut self, change: Change) {
        if change.enable_next {
            let room = change.room.clone();
            self.send_push_rule(
                RuleKind::Room,
                &room,
                RuleChange::Enabled(true),
                Request::NotifyRule(Change {
                    enable_next: false,
                    ..change
                }),
            );
        } else if change.mode == Mode::Default {
            self.alerts.rules.remove(&change.room);
        } else {
            self.alerts.rules.insert(change.room, (change.mode, true));
        }
    }
    /// Alerts are quiet, or the user is reading this room right now.
    fn reading_room(&self, id: &str) -> bool {
        self.quiet_alerts()
            || self.reading_focused()
                && self.selected == id
                && !self.account
                && self.setup.is_none()
                && self.mentions.is_none()
                && self.search.is_none()
                && self.original.is_none()
    }
    /// A message the matrix-sdk engine's push rule evaluation says to
    /// notify about.
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    pub fn engine_notice(&mut self, notice: crate::engine::Alert) {
        if !self.logged_in
            || !self.verified
            || self.closing
            || notice.sender == self.matrix_id
            || self.alerts.recent.iter().any(|old| *old == notice.event_id)
        {
            return;
        }
        let Some(room) = self.live.list.get(&notice.room) else {
            return;
        };
        self.alerts.recent.push_back(notice.event_id.clone());
        if self.alerts.recent.len() > 512 {
            self.alerts.recent.pop_front();
        }
        if self.reading_room(&notice.room) {
            return;
        }
        let title = format!(
            "{} · {}",
            single_line(room.display(&notice.sender), 64),
            single_line(&room.name, 80)
        );
        self.alerts.outgoing.push(Notice {
            title,
            body: single_line(&notice.body, 240),
            sound: notice.sound,
            #[cfg(target_arch = "wasm32")]
            room: notice.room,
            #[cfg(target_arch = "wasm32")]
            event: notice.event_id,
        });
    }
    /// Alerts waiting to be shown, as (title, body).
    #[cfg(test)]
    pub fn queued_alerts(&self) -> Vec<(String, String)> {
        self.alerts
            .outgoing
            .iter()
            .map(|n| (n.title.clone(), n.body.clone()))
            .collect()
    }
    /// The account's push rules arrived or changed (matrix-sdk engine).
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    pub fn engine_push_rules(&mut self, content: &Value) {
        self.alerts.ruleset(content);
        // The engine reports the rules after its first sync, with the room
        // list in place.
        self.alerts.spaces.initialized = true;
        if self.alerts.dnd == Some(true) {
            self.apply_quiet();
        }
        self.sync_space_rules();
    }
    pub fn send_push_rule(
        &mut self,
        kind: RuleKind,
        rule_id: &str,
        change: RuleChange,
        request: Request,
    ) {
        let rule_id = rule_id.to_owned();
        self.command(
            |id| crate::engine::Command::PushRule {
                id,
                kind,
                rule_id,
                change,
            },
            request,
        );
    }
    pub fn send_room_account_data(
        &mut self,
        room: &str,
        kind: &str,
        content: Value,
        request: Request,
    ) {
        let (room, kind) = (room.to_owned(), kind.to_owned());
        self.command(
            |id| crate::engine::Command::RoomAccountData {
                id,
                room,
                kind,
                content,
            },
            request,
        );
    }
    pub fn send_alerts(&mut self) -> Task<Msg> {
        let notices = std::mem::take(&mut self.alerts.outgoing);
        if notices.is_empty() || self.quiet_alerts() {
            return Task::none();
        }
        #[cfg(target_arch = "wasm32")]
        {
            if browser::permission() != "granted" {
                return Task::none();
            }
            self.alerts.delivery_failed = false;
            for n in notices {
                if browser::show(
                    &self.matrix_id,
                    &n.room,
                    &n.event,
                    &n.title,
                    &n.body,
                    !n.sound,
                )
                .is_err()
                {
                    self.alerts.delivery_failed = true;
                }
            }
            Task::none()
        }
        #[cfg(target_os = "linux")]
        Task::perform(
            async move {
                matches!(
                    tokio::time::timeout(Duration::from_secs(5), desktop::deliver(notices)).await,
                    Ok(Ok(()))
                )
            },
            Msg::DesktopDelivery,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rooms;
    #[cfg(target_os = "linux")]
    #[test]
    fn engine_alerts_skip_own_repeated_and_quiet_messages_and_read_typed_rules() {
        let (tx, _) = async_channel::bounded(4);
        let mut app = App::new(tx);
        app.logged_in = true;
        app.verified = true;
        app.matrix_id = "@me:h".into();
        app.live.list.insert(
            "!r".into(),
            rooms::Room {
                name: "general".into(),
                ..Default::default()
            },
        );
        let alert = |event: &str, sender: &str| crate::engine::Alert {
            room: "!r".into(),
            event_id: event.into(),
            sender: sender.into(),
            body: "hello\nthere".into(),
            sound: true,
        };
        app.engine_notice(alert("$1", "@bob:h"));
        app.engine_notice(alert("$1", "@bob:h"));
        app.engine_notice(alert("$2", "@me:h"));
        assert_eq!(app.alerts.outgoing.len(), 1, "once, and never our own");
        let n = &app.alerts.outgoing[0];
        assert_eq!(n.title, "bob · general");
        assert_eq!(n.body, "hello there");
        assert!(n.sound);
        // The account's rules as Matrix defines them, with DND on.
        app.engine_push_rules(
            &json!({"global":{"override":[{"rule_id":MASTER,"default":true,
            "conditions":[],"actions":[],"enabled":true}],
            "room":[{"rule_id":"!r","actions":[],"enabled":true}]}}),
        );
        assert_eq!(app.alerts.dnd, Some(true));
        assert!(app.alerts.outgoing.is_empty(), "DND drops what was waiting");
        assert!(app.alerts.mode("!r") == Mode::Quiet);
        app.engine_notice(alert("$3", "@bob:h"));
        assert!(app.alerts.outgoing.is_empty(), "nothing while DND is on");
    }
}
