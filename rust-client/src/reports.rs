//! An event report is private feedback to the reporter's homeserver. Send
//! only the chosen event reference and the reason the user actually enters.
use crate::{App, Request, rooms::Post};
use serde_json::json;

pub struct Draft {
    pub room: String,
    pub event: String,
    pub preview: String,
    pub reason: String,
}
impl Post {
    pub fn report_id(&self, me: &str) -> Option<&str> {
        if !self.reactable || self.sender == me || self.sender.is_empty() {
            return None;
        }
        self.revision
            .as_ref()
            .map(|r| r.event_id.as_str())
            .or(self.event_id.as_deref())
    }
}
impl App {
    pub fn report_pending(&self) -> bool {
        self.pending.values().any(|r| matches!(r, Request::Report))
    }
    pub fn begin_report(&mut self, room: &str, base: &str) -> bool {
        if !self.logged_in
            || !self.connected
            || !self.verified
            || self.selected != room
            || self.reporting.is_some()
            || self.report_pending()
        {
            return false;
        }
        let Some(p) = self.visible_post(base) else {
            return false;
        };
        let Some(event) = p.report_id(&self.matrix_id) else {
            return false;
        };
        let Some(q) = p.quote() else { return false };
        self.reporting = Some(Draft {
            room: room.into(),
            event: event.into(),
            preview: format!("{}: {}", q.sender, q.excerpt),
            reason: String::new(),
        });
        self.status.clear();
        true
    }
    pub fn submit_report(&mut self) {
        let Some(d) = &self.reporting else { return };
        if !self.logged_in
            || !self.verified
            || !self.connected
            || d.room != self.selected
            || !self.live.list.contains_key(&d.room)
            || d.reason.trim().is_empty()
            || self.report_pending()
        {
            return;
        }
        // An edit arriving after the form opened must not change the target.
        self.rpc(
            "report_event",
            json!({"room_id":d.room,"event_id":d.event,"reason":d.reason}),
            Request::Report,
        );
    }
    pub fn report_result(&mut self, ok: bool) {
        if self.reporting.is_none() {
            return;
        }
        if ok {
            self.reporting = None;
            self.status = "Report sent to your homeserver.".into();
        } else {
            self.status = "Could not submit the report. Your reason has been kept.".into();
        }
    }
}
