//! Search the device's gomuks database. Hits are canonical messages, not edit
//! events. Only explicit navigation from a hit may fetch network context.
pub(crate) mod bookmark;
use crate::{
    App, Msg, Request, edits,
    rooms::{self, Room, items},
};
pub use bookmark::Bookmark;
use iced::{Task, widget::operation};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub enum Page {
    Matches,
    Bases,
    Coverage,
}
#[derive(Default)]
pub struct Coverage {
    pub messages: usize,
    pub oldest: Option<i64>,
    pub done: bool,
    pub failed: bool,
    as_of: i64,
}
pub struct Hit {
    pub id: String,
    pub sender: String,
    pub snippet: String,
    pub time: i64,
}
pub struct Search {
    pub room: String,
    pub input: String,
    pub term: String,
    pub hits: Vec<Hit>,
    pub next: Option<String>,
    pub busy: bool,
    pub failed: bool,
    pub coverage: Option<Coverage>,
    bookmark: Bookmark,
    unresolved: BTreeMap<String, BTreeSet<String>>,
    senders: BTreeSet<String>,
}
impl Search {
    fn new(room: String, bookmark: Bookmark) -> Self {
        Self {
            room,
            bookmark,
            input: String::new(),
            term: String::new(),
            hits: Vec::new(),
            next: None,
            busy: false,
            failed: false,
            coverage: None,
            unresolved: Default::default(),
            senders: Default::default(),
        }
    }
    fn params(&self, kind: Page, next: Option<String>) -> Value {
        let mut p = json!({"room_ids":[self.room],"sort_by_time":true});
        match kind {
            Page::Coverage => {
                p["limit"] = 256.into();
                p["max_timestamp"] = self.coverage.as_ref().unwrap().as_of.into();
            }
            Page::Matches => {
                // Escape the FTS5 phrase, including embedded double quotes.
                p["search_term"] = format!("body : \"{}\"", self.term.replace('"', "\"\"")).into();
                p["limit"] = 32.into();
            }
            Page::Bases => {
                // get_event fetches from the server on a cache miss. Resolve
                // matching edits against *locally stored* bases instead.
                p["senders"] = json!(self.senders);
                p["limit"] = 128.into();
            }
        }
        if let Some(next) = next {
            p["next_batch"] = next.into();
        }
        p
    }
    fn accept_page(&mut self, kind: Page, data: &Value) -> Option<(Page, Option<String>)> {
        let token = data["next_batch"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if matches!(kind, Page::Coverage) {
            let c = self.coverage.as_mut()?;
            for e in items(&data["events"])
                .filter(|e| e["room_id"] == self.room && searchable(e) && !edits::is_replacement(e))
            {
                c.messages += 1;
                if let Some(ts) = e["timestamp"].as_i64() {
                    c.oldest = Some(c.oldest.map_or(ts, |n| n.min(ts)));
                }
            }
            c.done = token.is_none();
            return token.map(|t| (Page::Coverage, Some(t)));
        }
        let mut page = Room::default();
        for e in items(&data["events"]).filter(|e| e["room_id"] == self.room) {
            if !searchable(e) {
                continue;
            }
            page.insert(e);
            if matches!(kind, Page::Matches)
                && rooms::server_confirmed(e)
                && edits::is_replacement(e)
            {
                let c = if e["type"] == "m.room.encrypted" {
                    &e["decrypted"]
                } else {
                    &e["content"]
                };
                if let (Some(id), Some(sender)) =
                    (c["m.relates_to"]["event_id"].as_str(), e["sender"].as_str())
                    && !self.hits.iter().any(|h| h.id == id)
                {
                    self.unresolved
                        .entry(id.into())
                        .or_default()
                        .insert(sender.into());
                }
            }
        }
        page.related_events(&self.room, data);
        let term = self.term.to_lowercase();
        for p in page.history.values() {
            let Some(id) = &p.event_id else { continue };
            let wanted = self.unresolved.remove(id);
            if matches!(kind, Page::Bases) && !wanted.is_some_and(|users| users.contains(&p.sender))
            {
                continue;
            }
            if self.hits.iter().any(|h| &h.id == id) {
                continue;
            }
            // The index includes a revision's fallback body and old bodies.
            // Display only the currently valid, decrypted message text.
            if let Some(snippet) = excerpt(p.text(), &term) {
                self.hits.push(Hit {
                    id: id.clone(),
                    sender: p.sender.clone(),
                    snippet,
                    time: p.time,
                });
            }
        }
        match kind {
            Page::Matches => {
                self.next = token;
                self.senders = self.unresolved.values().flatten().cloned().collect();
                if !self.unresolved.is_empty() {
                    return Some((Page::Bases, None));
                }
            }
            Page::Bases if !self.unresolved.is_empty() && token.is_some() => {
                return Some((Page::Bases, token));
            }
            _ => {}
        }
        self.unresolved.clear();
        self.senders.clear();
        if self.hits.is_empty()
            && let Some(t) = self.next.take()
        {
            return Some((Page::Matches, Some(t)));
        }
        self.hits
            .sort_by(|a, b| b.time.cmp(&a.time).then(a.id.cmp(&b.id)));
        self.busy = false;
        None
    }
}
fn searchable(e: &Value) -> bool {
    let (ty, c) = if e["type"] == "m.room.encrypted" {
        (&e["decrypted_type"], &e["decrypted"])
    } else {
        (&e["type"], &e["content"])
    };
    rooms::server_confirmed(e)
        && e["state_key"].is_null()
        && !e["redacted_by"].as_str().is_some_and(|id| !id.is_empty())
        && matches!(ty.as_str(), Some("m.room.message" | "m.sticker"))
        && c["body"].as_str().is_some_and(|body| !body.is_empty())
}
// Fold for matching without using folded byte offsets to slice original UTF-8.
fn excerpt(body: &str, term: &str) -> Option<String> {
    let folded = body.to_lowercase();
    let pos = folded.find(term)?;
    // Lowercasing can expand a scalar (e.g. İ); translate back to the
    // original character index rather than slicing on the folded offset.
    let mut bytes = 0;
    let start = body
        .chars()
        .take_while(|c| {
            if bytes >= pos {
                return false;
            }
            bytes += c.to_lowercase().map(char::len_utf8).sum::<usize>();
            true
        })
        .count()
        .saturating_sub(36);
    let mut chars = body.chars().skip(start);
    let mut snippet = if start > 0 {
        "…".into()
    } else {
        String::new()
    };
    snippet.extend(
        chars
            .by_ref()
            .take(150)
            .map(|c| if c.is_whitespace() { ' ' } else { c }),
    );
    if chars.next().is_some() {
        snippet.push('…');
    }
    Some(snippet)
}
impl App {
    pub fn begin_search(&self) -> Task<Msg> {
        if !self.logged_in
            || !self.verified
            || self.selected.is_empty()
            || self.original.is_some()
            || self.search.is_some()
        {
            return Task::none();
        }
        let room = self.selected.clone();
        bookmark::capture(self.timeline_id()).map(move |b| Msg::SearchReady(room.clone(), b))
    }
    pub fn search_ready(&mut self, room: String, b: Bookmark) -> Task<Msg> {
        if self.selected != room
            || self.search.is_some()
            || self.original.is_some()
            || !self.verified
        {
            return Task::none();
        }
        self.search = Some(Search::new(room, b));
        operation::focus("room-search-input")
    }
    pub fn clear_search(&mut self) {
        self.search = None;
        self.pending
            .retain(|_, p| !matches!(p, Request::Search(..)));
    }
    pub fn start_coverage(&mut self) {
        let Some(s) = &mut self.search else { return };
        if s.coverage.as_ref().is_some_and(|c| !c.done && !c.failed) {
            return;
        }
        s.coverage = Some(Coverage {
            as_of: chrono::Utc::now().timestamp_millis(),
            ..Default::default()
        });
        self.search_page(Page::Coverage, None);
    }
    pub fn fail_search(&mut self, room: &str, kind: Page) {
        if let Some(s) = self.search.as_mut().filter(|s| s.room == room) {
            if matches!(kind, Page::Coverage) {
                if let Some(c) = &mut s.coverage {
                    c.failed = true;
                }
            } else {
                s.busy = false;
                s.failed = true;
            }
        }
    }
    pub fn return_to_reading(&mut self) -> Task<Msg> {
        self.close_original();
        let Some(s) = self.search.take() else {
            return self.anchor_history();
        };
        self.clear_search();
        bookmark::restore(self.timeline_id(), s.bookmark)
    }
    pub fn search_page(&mut self, kind: Page, next: Option<String>) {
        if let Some(s) = &mut self.search
            && self.selected == s.room
        {
            if !matches!(kind, Page::Coverage) {
                s.busy = true;
                s.failed = false;
            }
            let params = s.params(kind, next);
            let room = s.room.clone();
            self.rpc("search_local", params, Request::Search(room.clone(), kind));
            if !self.pending.contains_key(&self.seq) {
                self.fail_search(&room, kind);
            }
        }
    }
    pub fn submit_search(&mut self, more: bool) {
        let Some(s) = &mut self.search else { return };
        if s.input.trim().is_empty() || !self.verified {
            return;
        }
        let next = if more {
            if s.busy || s.term != s.input.trim() || s.next.is_none() {
                return;
            }
            s.next.clone()
        } else {
            self.pending
                .retain(|_, p| !matches!(p, Request::Search(_, Page::Matches | Page::Bases)));
            s.term = s.input.trim().into();
            s.hits.clear();
            s.unresolved.clear();
            s.next = None;
            None
        };
        self.search_page(Page::Matches, next);
    }
    pub fn search_response(&mut self, room: String, kind: Page, data: &Value) {
        if let Some(s) = &mut self.search
            && s.room == room
            && self.selected == room
        {
            if !data.is_object() || (!data["events"].is_array() && !data["events"].is_null()) {
                self.fail_search(&room, kind);
                return;
            }
            if let Some((kind, next)) = s.accept_page(kind, data) {
                self.search_page(kind, next);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resubmission_invalidates_old_pages_and_quotes_fts_syntax() {
        let (tx, rx) = async_channel::bounded(8);
        let mut app = App::new(tx);
        app.verified = true;
        app.connected = true;
        app.selected = "!here".into();
        app.search = Some(Search::new(app.selected.clone(), Bookmark::default()));
        app.search.as_mut().unwrap().input = r#"blue" OR "green"#.into();
        app.submit_search(false);
        let first = rx.try_recv().unwrap().into_rpc();
        assert_eq!(first["command"], "search_local");
        assert_eq!(first["data"]["room_ids"], json!(["!here"]));
        assert_eq!(
            first["data"]["search_term"],
            r#"body : "blue"" OR ""green""#
        );
        app.search.as_mut().unwrap().input = "new".into();
        app.submit_search(false);
        let second = rx.try_recv().unwrap().into_rpc();
        let event = |room: &str| json!({"room_id":room,"rowid":4,"event_id":"$new","type":"m.room.encrypted","decrypted_type":"m.room.message","decrypted":{"body":"new"}});
        app.frame(json!({"command":"response","request_id":first["request_id"],"data":{"events":[event("!here")]}}));
        assert!(app.search.as_ref().unwrap().busy && app.search.as_ref().unwrap().hits.is_empty());
        app.frame(json!({"command":"response","request_id":second["request_id"],"data":{"events":[event("!elsewhere")]}}));
        assert!(!app.search.as_ref().unwrap().busy && app.search.as_ref().unwrap().hits.is_empty());
    }
}
