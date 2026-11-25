//! Use gomuks timeline order (not sender timestamps, DB event IDs, or context
//! rows) for read markers. Private unless the account opts into public receipts.
use crate::{
    App, Msg, Request,
    rooms::{self, items},
};
use iced::Task;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct State {
    pub marked: bool,
    pub messages: u64,
    pub highlights: u64,
    pub fully_read: Option<String>,
    timeline: BTreeMap<i64, String>,
    preview: Option<String>,
    attempted: Option<String>,
    // One current main-timeline receipt per reader, even across re-pagination.
    readers: BTreeMap<String, (String, i64)>,
}
impl State {
    pub fn position(&mut self, t: i64, e: &Value) {
        if t != 0 && rooms::server_confirmed(e) {
            self.timeline.insert(t, rooms::string(&e["event_id"]));
        }
    }
    pub fn target(&self) -> Option<&str> {
        self.timeline
            .last_key_value()
            .map(|(_, e)| e.as_str())
            .or(self.preview.as_deref())
    }
    pub fn sync(&mut self, delta: &Value, id: &str) {
        if delta["reset"] == true {
            self.timeline.clear();
            self.preview = None;
            self.attempted = None;
            self.readers.clear();
        }
        self.receipts(delta, id);
        let m = &delta["meta"];
        if m.is_object() {
            self.marked = m["marked_unread"] == true;
            self.messages = m["unread_messages"].as_u64().unwrap_or(0);
            self.highlights = m["unread_highlights"].as_u64().unwrap_or(0);
            if self.messages == 0
                && self.highlights == 0
                && !self.marked
                && m["unread_notifications"].as_u64().unwrap_or(0) == 0
            {
                self.attempted = None;
            }
        }
        if let Some(data) = delta["account_data"].get("m.fully_read") {
            self.fully_read = data["content"]["event_id"]
                .as_str()
                .filter(|e| e.starts_with('$'))
                .map(str::to_owned);
        }
        let rows: BTreeMap<_, _> = items(&delta["timeline"])
            .filter_map(|t| Some((t["event_rowid"].as_i64()?, t["timeline_rowid"].as_i64()?)))
            .collect();
        for e in
            items(&delta["events"]).filter(|e| e["room_id"] == id && rooms::server_confirmed(e))
        {
            if let Some(row) = e["rowid"].as_i64() {
                if let Some(t) = rows.get(&row) {
                    self.position(*t, e);
                }
                if m["preview_event_rowid"] == row {
                    self.preview = Some(rooms::string(&e["event_id"]));
                }
            }
        }
    }
    pub fn receipts(&mut self, data: &Value, room: &str) {
        if let Some(receipts) = data["receipts"].as_object() {
            for (event, rows) in receipts {
                for r in items(rows) {
                    if r["receipt_type"] != "m.read"
                        || r["room_id"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty() && id != room)
                        || !matches!(r["thread_id"].as_str(), None | Some("" | "main"))
                        || !event.starts_with('$')
                        || r["event_id"] != *event
                    {
                        continue;
                    }
                    if let (Some(user), Some(ts)) = (r["user_id"].as_str(), r["timestamp"].as_i64())
                        && self.readers.get(user).is_none_or(|(_, old)| ts >= *old)
                    {
                        self.readers.insert(user.into(), (event.clone(), ts));
                    }
                }
            }
        }
    }
    pub fn seen(&self, me: &str) -> BTreeMap<&str, Vec<&str>> {
        let mut by_event = BTreeMap::new();
        for (user, (id, _)) in &self.readers {
            if user != me {
                by_event
                    .entry(id.as_str())
                    .or_insert_with(Vec::new)
                    .push(user.as_str());
            }
        }
        by_event
    }
    pub fn divider<'a>(&'a self, room: &rooms::Room) -> Option<&'a str> {
        let marker = self.fully_read.as_ref()?;
        // A fully-read marker may be on an edit, reaction or room state,
        // none of which has its own row in the rendered message list.
        let mut later = self
            .timeline
            .values()
            .skip_while(|id| *id != marker)
            .skip(1);
        later
            .find(|id| room.by_id(id).is_some())
            .map(String::as_str)
    }
}
const PREF: &str = "com.untethered.read_receipts.v1";
#[derive(Default)]
pub struct Sharing {
    pub public: bool,
    revision: u64,
}
impl Sharing {
    pub fn sync(&mut self, d: &Value) {
        if let Some(p) = d["account_data"].get(PREF) {
            self.update(p["content"]["public"] == true)
        } else if d["clear_state"] == true {
            self.update(false);
        }
    }
    fn update(&mut self, value: bool) {
        self.revision += 1;
        self.public = value;
    }
    pub fn saved(&mut self, value: bool, revision: u64) {
        if revision == self.revision {
            self.update(value);
        }
    }
}
impl App {
    pub fn changing_read_sharing(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::ShareReads(..)))
    }
    pub fn set_read_sharing(&mut self, value: bool) {
        if !self.verified
            || !self.connected
            || !self.logged_in
            || self.changing_read_sharing()
            || self.read_sharing.public == value
        {
            return;
        }
        self.rpc(
            "set_account_data",
            json!({"type":PREF,"content":{"public":value}}),
            Request::ShareReads(value, self.read_sharing.revision),
        );
    }
    pub fn reading_focused(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            web_sys::window()
                .and_then(|w| w.document())
                .is_some_and(|d| !d.hidden() && d.has_focus().unwrap_or(false))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.focused
        }
    }
    pub fn writing_read(&self, room: &str) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p,Request::ReadState(r,_) if r==room))
    }
    pub fn auto_target(&self) -> Option<(&str, &str)> {
        if !self.logged_in
            || !self.verified
            || !self.connected
            || self.closing
            || self.account
            || self.setup.is_some()
            || self.mentions.is_some()
            || self.search.is_some()
            || self.original.is_some()
            || self.reporting.is_some()
            || self.scrolled_away
            || self.writing_read(&self.selected)
        {
            return None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !self.focused {
            return None;
        }
        let r = self.live.list.get(&self.selected)?;
        let id = r.read.target()?;
        if !r.unread || r.read.marked || r.read.attempted.as_deref() == Some(id) {
            return None;
        }
        let visible = r.read.timeline.values().rev().find_map(|id| r.by_id(id))?;
        visible
            .reactable
            .then_some((id, visible.event_id.as_deref()?))
    }
    pub fn check_reading(&self) -> Task<Msg> {
        if !self.reading_focused() {
            return Task::none();
        }
        let Some((id, visible)) = self.auto_target() else {
            return Task::none();
        };
        let (room, id, visible) = (self.selected.clone(), id.to_owned(), visible.to_owned());
        crate::search::bookmark::visible_at_tail(self.timeline_id(), &visible).map(move |at_tail| {
            Msg::ReadVisible(room.clone(), id.clone(), visible.clone(), at_tail)
        })
    }
    pub fn mark_room(&mut self, room: String, event: Option<String>) {
        if self.selected != room
            || !self.logged_in
            || !self.verified
            || !self.connected
            || self.writing_read(&room)
        {
            return;
        }
        let public = self.read_sharing.public && !self.changing_read_sharing();
        let Some(r) = self.live.list.get_mut(&room) else {
            return;
        };
        if let Some(id) = &event {
            if !r.unread || r.read.target() != Some(id.as_str()) {
                return;
            }
            r.read.attempted = Some(id.clone());
            self.rpc(
                "mark_read",
                json!({"room_id":room,"event_id":id,"receipt_type":if public{"m.read"}else{"m.read.private"}}),
                Request::ReadState(room, event),
            );
        } else if !r.unread {
            self.rpc(
                "set_account_data",
                json!({"room_id":room,"type":"m.marked_unread","content":{"unread":true}}),
                Request::ReadState(room, None),
            );
        }
    }
    pub fn read_result(&mut self, room: String, event: Option<String>, ok: bool) {
        if self.selected == room && !ok {
            self.status = if event.is_some() {
                "Could not mark this room read. Use Mark read to retry."
            } else {
                "Could not mark this room unread. Try again."
            }
            .into();
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn stale_viewport_or_pagination_must_not_acknowledge_a_newer_arrival() {
        let (tx, rx) = async_channel::bounded(8);
        let mut a = App::new(tx);
        a.logged_in = true;
        a.verified = true;
        a.connected = true;
        a.focused = true;
        a.selected = "!r".into();
        let event = |id: &str, row: i64, t: i64| {
            json!({"room_id":"!r","event_id":id,"rowid":row,"type":"m.room.message",
            "sender":"@peer:h","content":{"msgtype":"m.text","body":"hello"},"timeline_rowid":t})
        };
        let r = a.live.list.entry("!r".into()).or_default();
        r.unread = true;
        r.page("!r", &json!({"events":[event("$one",1,100)]}));
        let old = Msg::ReadVisible("!r".into(), "$one".into(), "$one".into(), true);
        r.page(
            "!r",
            &json!({"events":[event("$two",2,101),event("$backfill",7,-1)]}),
        );
        let _ = a.update(old);
        assert!(rx.is_empty());
        assert_eq!(a.auto_target(), Some(("$two", "$two")));
        let _ = a.update(Msg::ReadingScroll("!r".into(), true));
        let _ = a.update(Msg::ReadVisible(
            "!r".into(),
            "$two".into(),
            "$two".into(),
            true,
        ));
        assert!(rx.is_empty());
    }
    #[test]
    fn private_threaded_or_old_receipts_cannot_move_another_readers_badge() {
        let mut s = State::default();
        let receipt = |id: &str, kind: &str, thread: &str, time: i64| {
            json!({"receipts":{id:[{
                "event_id":id,"room_id":"!r","user_id":"@peer:h","receipt_type":kind,"thread_id":thread,"timestamp":time
            }]}})
        };
        s.receipts(&receipt("$new", "m.read", "", 30), "!r");
        for bad in [
            receipt("$old", "m.read", "", 20),
            receipt("$private", "m.read.private", "", 40),
            receipt("$thread", "m.read", "$root", 50),
        ] {
            s.receipts(&bad, "!r");
        }
        s.receipts(&receipt("$wrong", "m.read", "", 80), "!other");
        assert_eq!(s.seen("@me:h").get("$new").unwrap(), &["@peer:h"]);
        assert!(s.seen("@peer:h").is_empty());
    }
}
