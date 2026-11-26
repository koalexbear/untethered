//! Search the device's gomuks database. Hits are canonical messages, not edit
//! events. Only explicit navigation from a hit may fetch network context.
pub(crate) mod bookmark;
use crate::{App, Msg, Request};
pub use bookmark::Bookmark;
use iced::{Task, widget::operation};

#[derive(Clone, Copy)]
pub enum Page {
    Matches,
    Coverage,
}
#[derive(Default)]
pub struct Coverage {
    pub messages: usize,
    pub oldest: Option<i64>,
    pub done: bool,
    pub failed: bool,
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
        }
    }
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
            use crate::engine::Command;
            let (room, term) = (s.room.clone(), s.term.clone());
            let request = Request::Search(room.clone(), kind);
            let target = room.clone();
            match kind {
                Page::Coverage => {
                    self.command(|id| Command::SearchCoverage { id, room: target }, request)
                }
                Page::Matches => self.command(
                    |id| Command::Search {
                        id,
                        room: target,
                        term,
                        before: next,
                    },
                    request,
                ),
            }
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
                .retain(|_, p| !matches!(p, Request::Search(_, Page::Matches)));
            s.term = s.input.trim().into();
            s.hits.clear();
            s.next = None;
            None
        };
        self.search_page(Page::Matches, next);
    }
    /// A page of matches from the engine, with current text.
    pub fn search_found(
        &mut self,
        room: String,
        found: Vec<crate::engine::Found>,
        next: Option<String>,
    ) {
        let selected = &self.selected;
        let Some(s) = self
            .search
            .as_mut()
            .filter(|s| s.room == room && *selected == room)
        else {
            return;
        };
        let term = s.term.to_lowercase();
        for f in found {
            if s.hits.iter().any(|h| h.id == f.event_id) {
                continue;
            }
            if let Some(snippet) = excerpt(&f.body, &term) {
                s.hits.push(Hit {
                    id: f.event_id,
                    sender: f.sender,
                    snippet,
                    time: f.time,
                });
            }
        }
        s.next = next;
        s.hits
            .sort_by(|a, b| b.time.cmp(&a.time).then(a.id.cmp(&b.id)));
        s.busy = false;
    }
    /// How much of the room's history the matrix-sdk engine has stored.
    pub fn search_counted(&mut self, room: String, messages: usize, oldest: Option<i64>) {
        if let Some(c) = self
            .search
            .as_mut()
            .filter(|s| s.room == room)
            .and_then(|s| s.coverage.as_mut())
        {
            c.messages = messages;
            c.oldest = oldest;
            c.done = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn engine_matches_become_snippets_once_newest_first() {
        let (tx, _) = async_channel::bounded(4);
        let mut app = App::new(tx);
        app.selected = "!r".into();
        let mut s = Search::new("!r".into(), Bookmark::default());
        s.term = "Lunch".into();
        s.busy = true;
        app.search = Some(s);
        let found = |id: &str, time, body: &str| crate::engine::Found {
            room: "!r".into(),
            event_id: id.into(),
            sender: "@a:h".into(),
            body: body.into(),
            time,
        };
        app.search_found(
            "!r".into(),
            vec![
                found("$old", 1, "lunch at noon"),
                found("$new", 5, "Lunch\nmoved"),
            ],
            Some("cursor".into()),
        );
        app.search_found("!r".into(), vec![found("$new", 5, "Lunch moved")], None);
        let s = app.search.as_ref().unwrap();
        let hits: Vec<_> = s
            .hits
            .iter()
            .map(|h| (h.id.as_str(), h.snippet.as_str()))
            .collect();
        assert_eq!(hits, [("$new", "Lunch moved"), ("$old", "lunch at noon")]);
        assert!(!s.busy && s.next.is_none());
    }
    #[test]
    fn resubmission_invalidates_old_pages() {
        use crate::engine::{Command, Event, Found};
        let (tx, _) = async_channel::bounded(8);
        let mut app = App::new(tx);
        let (sdk, commands) = async_channel::bounded(8);
        app.sdk = Some(sdk);
        app.verified = true;
        app.connected = true;
        app.selected = "!here".into();
        app.search = Some(Search::new(app.selected.clone(), Bookmark::default()));
        app.search.as_mut().unwrap().input = "blue".into();
        app.submit_search(false);
        let Ok(Command::Search {
            id: first, term, ..
        }) = commands.try_recv()
        else {
            panic!("expected a search")
        };
        assert_eq!(term, "blue");
        app.search.as_mut().unwrap().input = "new".into();
        app.submit_search(false);
        let Ok(Command::Search { id: second, .. }) = commands.try_recv() else {
            panic!("expected a second search")
        };
        let found = |id: &str| Found {
            room: "!here".into(),
            event_id: id.into(),
            sender: "@a:example".into(),
            body: "new".into(),
            time: 1,
        };
        app.engine_event(Event::SearchResults {
            id: first,
            result: Ok((vec![found("$old")], None)),
        });
        let s = app.search.as_ref().unwrap();
        assert!(s.busy && s.hits.is_empty(), "the old page is dropped");
        app.engine_event(Event::SearchResults {
            id: second,
            result: Ok((vec![found("$new")], None)),
        });
        let s = app.search.as_ref().unwrap();
        assert!(!s.busy && s.hits.len() == 1 && s.hits[0].id == "$new");
    }
}
