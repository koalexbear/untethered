//! Use gomuks timeline order (not sender timestamps, DB event IDs, or context
//! rows) for read markers. Private unless the account opts into public receipts.
use crate::{
    App, Msg, Request,
    rooms::{self},
};
use iced::Task;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct State {
    pub marked: bool,
    pub messages: u64,
    pub highlights: u64,
    pub fully_read: Option<String>,
    timeline: BTreeMap<i64, String>,
    /// The engine's latest remote event, which may be an edit or reaction
    /// that has no message of its own.
    latest: Option<String>,
    preview: Option<String>,
    attempted: Option<String>,
    // One current main-timeline receipt per reader, even across re-pagination.
    readers: BTreeMap<String, (String, i64)>,
}
impl State {
    /// Timeline order and receipts from the matrix-sdk engine's snapshot:
    /// `events` are sent events in timeline order, `receipts` are
    /// (event, reader, timestamp).
    pub fn apply_engine<'a>(
        &mut self,
        events: impl Iterator<Item = &'a str>,
        receipts: impl Iterator<Item = (&'a str, &'a str, i64)>,
    ) {
        self.timeline = events.zip(1..).map(|(id, n)| (n, id.to_owned())).collect();
        self.preview = None;
        for (event, user, ts) in receipts {
            if self.readers.get(user).is_none_or(|(_, old)| ts >= *old) {
                self.readers.insert(user.to_owned(), (event.to_owned(), ts));
            }
        }
    }
    pub fn set_latest(&mut self, latest: Option<String>) {
        self.latest = latest;
    }
    /// What reading the room marks read: its latest event, also when that is
    /// an edit of the last message (which has no row of its own).
    pub fn target(&self) -> Option<&str> {
        self.latest
            .as_deref()
            .or_else(|| self.timeline.last_key_value().map(|(_, e)| e.as_str()))
            .or(self.preview.as_deref())
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
    /// The preference as synced by the matrix-sdk engine.
    pub fn set_from_engine(&mut self, public: bool) {
        self.update(public)
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
mod tests {}
