//! Search (ROADMAP N7): this channel, this server (a space's channels) or all
//! rooms, with Discord's filters: `from:`, `in:`, `has:file` (or `image`,
//! `video`, `sound`, `link`), `before:` and `after:` (a date, YYYY-MM-DD).
//! The engine searches unencrypted rooms on the homeserver and encrypted ones
//! on this device (engine/search.rs); results say which, and how far back.
//! Only explicit navigation from a hit fetches network context.
pub(crate) mod bookmark;
use crate::{
    App, Msg, Request, Section,
    engine::{Command, Found, Has, Reach, RoomsFound, SearchQuery},
};
pub use bookmark::Bookmark;
use chrono::{Local, NaiveDate, TimeZone};
use iced::{Task, widget::operation};

/// Which rooms a search covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Room,
    /// The space the sidebar shows.
    Space,
    All,
}

/// A search as typed, read into its filters.
#[derive(Debug, Default, PartialEq)]
pub struct Query {
    pub text: String,
    pub from: Vec<String>,
    pub within: Vec<String>,
    pub has: Vec<Has>,
    pub before: Option<NaiveDate>,
    pub after: Option<NaiveDate>,
    /// Filters it couldn't read, as typed.
    pub unknown: Vec<String>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.from.is_empty()
            && self.within.is_empty()
            && self.has.is_empty()
            && self.before.is_none()
            && self.after.is_none()
    }
}

/// Read `from:`, `in:`, `has:`, `before:` and `after:` out of what was
/// typed; the rest is words to find.
pub fn parse(input: &str) -> Query {
    let mut q = Query::default();
    let mut words = Vec::new();
    for token in input.split_whitespace() {
        let Some((key, value)) = token.split_once(':').filter(|(k, v)| {
            !v.is_empty() && matches!(*k, "from" | "in" | "has" | "before" | "after")
        }) else {
            words.push(token);
            continue;
        };
        let date = || NaiveDate::parse_from_str(value, "%Y-%m-%d").ok();
        match key {
            "from" => q.from.push(value.to_owned()),
            "in" => q.within.push(value.trim_start_matches('#').to_lowercase()),
            "has" => match value.to_lowercase().as_str() {
                "file" | "files" | "attachment" => q.has.push(Has::File),
                "image" | "images" => q.has.push(Has::Image),
                "video" | "videos" => q.has.push(Has::Video),
                "sound" | "audio" => q.has.push(Has::Sound),
                "link" | "links" => q.has.push(Has::Link),
                _ => q.unknown.push(token.to_owned()),
            },
            "before" => match date() {
                Some(d) => q.before = Some(d),
                None => q.unknown.push(token.to_owned()),
            },
            _ => match date() {
                Some(d) => q.after = Some(d),
                None => q.unknown.push(token.to_owned()),
            },
        }
    }
    q.text = words.join(" ");
    q
}

/// The start of a day here, in milliseconds.
fn day_start(day: NaiveDate) -> Option<i64> {
    Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|t| t.timestamp_millis())
}

pub struct Hit {
    pub room: String,
    pub id: String,
    pub sender: String,
    pub snippet: String,
    pub time: i64,
}

pub struct Search {
    /// The room open when the search started, to return to.
    pub room: String,
    /// The space the sidebar showed then, for `Scope::Space`.
    pub space: Option<String>,
    pub scope: Scope,
    pub input: String,
    /// What was searched for last.
    pub term: String,
    pub hits: Vec<Hit>,
    pub reach: Vec<Reach>,
    pub nobody: Vec<String>,
    pub unknown: Vec<String>,
    /// More matched than shown.
    pub more: bool,
    /// `in:` matched no room in the scope.
    pub nowhere: bool,
    pub busy: bool,
    pub failed: bool,
    pub indexing: bool,
    bookmark: Bookmark,
}

impl Search {
    fn new(room: String, space: Option<String>, bookmark: Bookmark) -> Self {
        Self {
            scope: if space.is_some() {
                Scope::Space
            } else {
                Scope::Room
            },
            room,
            space,
            bookmark,
            input: String::new(),
            term: String::new(),
            hits: Vec::new(),
            reach: Vec::new(),
            nobody: Vec::new(),
            unknown: Vec::new(),
            more: false,
            nowhere: false,
            busy: false,
            failed: false,
            indexing: false,
        }
    }
    /// Some rooms were searched on this device only.
    pub fn local(&self) -> bool {
        self.reach.iter().any(|r| !r.server)
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

/// A hit's text around what was searched for (or its first word, as the
/// homeserver matches words), or from its start.
fn snippet(body: &str, text: &str) -> String {
    let text = text.trim().to_lowercase();
    excerpt(body, &text)
        .or_else(|| text.split_whitespace().find_map(|w| excerpt(body, w)))
        .or_else(|| excerpt(body, ""))
        .unwrap_or_default()
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
        let space = match &self.section {
            Section::Space(id) => Some(id.clone()),
            _ => None,
        };
        self.search = Some(Search::new(room, space, b));
        operation::focus("room-search-input")
    }
    pub fn clear_search(&mut self) {
        self.search = None;
        self.pending
            .retain(|_, p| !matches!(p, Request::Search | Request::IndexSearch));
    }
    pub fn return_to_reading(&mut self) -> Task<Msg> {
        self.close_original();
        let Some(s) = self.search.take() else {
            return self.anchor_history();
        };
        self.clear_search();
        bookmark::restore(self.timeline_id(), s.bookmark)
    }

    /// The rooms a scope covers, before `in:`.
    fn scope_rooms(&self, s: &Search) -> Vec<String> {
        let rooms = self.rooms();
        match (s.scope, &s.space) {
            (Scope::Space, Some(space)) => {
                rooms.ordered_spaces.get(space).cloned().unwrap_or_default()
            }
            (Scope::All, _) => rooms
                .list
                .iter()
                .filter(|(_, r)| !r.space)
                .map(|(id, _)| id.clone())
                .collect(),
            _ => vec![s.room.clone()],
        }
    }

    pub fn search_scope(&mut self, scope: Scope) {
        if let Some(s) = &mut self.search
            && s.scope != scope
            && (scope != Scope::Space || s.space.is_some())
        {
            s.scope = scope;
            if !s.term.is_empty() {
                self.run_search();
            }
        }
    }

    pub fn submit_search(&mut self) {
        let Some(s) = &mut self.search else { return };
        if s.input.trim().is_empty() || !self.verified {
            return;
        }
        s.term = s.input.trim().to_owned();
        self.run_search();
    }

    /// Search for `term` over the scope, from scratch.
    fn run_search(&mut self) {
        let Some(s) = &self.search else { return };
        let query = parse(&s.term);
        let mut rooms = self.scope_rooms(s);
        if !query.within.is_empty() {
            let list = &self.rooms().list;
            rooms.retain(|id| {
                list.get(id).is_some_and(|r| {
                    let name = r.name.to_lowercase();
                    query.within.iter().any(|w| name.contains(w.as_str()))
                })
            });
        }
        let nowhere = rooms.is_empty();
        let request = SearchQuery {
            rooms,
            text: query.text.clone(),
            from: query.from.clone(),
            has: query.has.clone(),
            before: query.before.and_then(day_start),
            // After a day: from the next one on.
            after: query
                .after
                .and_then(|d| d.succ_opt())
                .and_then(day_start)
                .map(|t| t - 1),
        };
        self.pending.retain(|_, p| !matches!(p, Request::Search));
        let s = self.search.as_mut().unwrap();
        s.hits.clear();
        s.reach.clear();
        s.nobody.clear();
        s.more = false;
        s.failed = false;
        let empty = query.is_empty();
        s.unknown = query.unknown;
        s.nowhere = nowhere;
        if nowhere || empty {
            s.busy = false;
            return;
        }
        s.busy = true;
        self.command(
            move |id| Command::SearchRooms { id, query: request },
            Request::Search,
        );
        if !self.pending.values().any(|p| matches!(p, Request::Search))
            && let Some(s) = &mut self.search
        {
            s.busy = false;
            s.failed = true;
        }
    }

    /// The engine's matches for the search as it is now.
    pub fn search_found(&mut self, result: Result<RoomsFound, crate::engine::Failure>) {
        let Some(s) = &mut self.search else { return };
        s.busy = false;
        let found = match result {
            Ok(found) => found,
            Err(_) => {
                s.failed = true;
                return;
            }
        };
        let words = parse(&s.term).text;
        s.hits = found
            .found
            .into_iter()
            .map(|f: Found| Hit {
                snippet: snippet(&f.body, &words),
                room: f.room,
                id: f.event_id,
                sender: f.sender,
                time: f.time,
            })
            .collect();
        s.reach = found.reach;
        s.nobody = found.nobody;
        s.more = found.more;
    }

    /// Load older history of the scope's encrypted rooms onto this device,
    /// then search again.
    pub fn index_search(&mut self) {
        let Some(s) = &self.search else { return };
        if s.indexing || !s.local() {
            return;
        }
        let rooms: Vec<String> = s
            .reach
            .iter()
            .filter(|r| !r.server)
            .map(|r| r.room.clone())
            .collect();
        self.command(
            move |id| Command::IndexRooms { id, rooms },
            Request::IndexSearch,
        );
        if self
            .pending
            .values()
            .any(|p| matches!(p, Request::IndexSearch))
            && let Some(s) = &mut self.search
        {
            s.indexing = true;
        }
    }

    pub fn search_indexed(&mut self, ok: bool) {
        let Some(s) = &mut self.search else { return };
        s.indexing = false;
        if ok {
            self.run_search();
        } else {
            s.failed = true;
        }
    }

    /// Open a hit: in this room, its context; in another, that room at it.
    pub fn open_hit(&mut self, room: String, id: String) -> Task<Msg> {
        if !self
            .search
            .as_ref()
            .is_some_and(|s| s.hits.iter().any(|h| h.room == room && h.id == id))
        {
            return Task::none();
        }
        if room == self.selected {
            self.jump(id);
            return if self.original.is_some() {
                self.anchor_history()
            } else {
                Task::none()
            };
        }
        let Some(r) = self.live.list.get(&room) else {
            return Task::none();
        };
        let dm = r.dm;
        self.clear_search();
        if !self.visible().any(|(shown, _)| *shown == room) {
            self.section = if dm {
                Section::Direct
            } else {
                self.rooms()
                    .ordered_spaces
                    .iter()
                    .find(|(_, rooms)| rooms.contains(&room))
                    .map_or(Section::All, |(space, _)| Section::Space(space.clone()))
            };
        }
        self.open(room);
        self.jump(id);
        self.reconcile();
        if self.original.is_some() {
            self.anchor_history()
        } else {
            Task::none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Event, RoomList, RoomSummary};

    #[test]
    fn filters_are_read_out_of_what_was_typed() {
        let q = parse("budget from:mira in:#Design has:file before:2026-11-20 after:2026-11-01 v2");
        assert_eq!(q.text, "budget v2");
        assert_eq!(q.from, ["mira"]);
        assert_eq!(q.within, ["design"]);
        assert_eq!(q.has, [Has::File]);
        assert_eq!(q.before, NaiveDate::from_ymd_opt(2026, 11, 20));
        assert_eq!(q.after, NaiveDate::from_ymd_opt(2026, 11, 1));
        assert!(q.unknown.is_empty());
        let q = parse("has:gifs before:soon time:now from:");
        assert_eq!(q.unknown, ["has:gifs", "before:soon"]);
        // Not a filter: words, like a time or an empty value.
        assert_eq!(q.text, "time:now from:");
        assert!(parse("   ").is_empty());
    }

    #[test]
    fn snippets_start_near_the_first_word_found() {
        assert_eq!(snippet("the plan is set", "plan"), "the plan is set");
        assert_eq!(snippet("report.pdf", ""), "report.pdf");
        let long = format!("{} needle", "x".repeat(80));
        assert!(snippet(&long, "needle").starts_with('…'));
    }

    fn app() -> (App, async_channel::Receiver<Command>) {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, rx) = async_channel::bounded(16);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        let room = |id: &str, name: &str| RoomSummary {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let mut children = std::collections::BTreeMap::new();
        children.insert(
            "!space:h".to_owned(),
            std::collections::BTreeSet::from(["!design:h".to_owned(), "!archive:h".to_owned()]),
        );
        app.engine_event(Event::Rooms(RoomList {
            invites: Vec::new(),
            rooms: vec![
                room("!design:h", "design"),
                room("!archive:h", "archive"),
                room("!lounge:h", "lounge"),
                RoomSummary {
                    space: true,
                    ..room("!space:h", "Workshop")
                },
            ],
            children,
        }));
        app.section = Section::Space("!space:h".into());
        let _ = app.update(Msg::Open("!design:h".into()));
        while rx.try_recv().is_ok() {}
        (app, rx)
    }

    fn searched(rx: &async_channel::Receiver<Command>) -> SearchQuery {
        std::iter::from_fn(|| rx.try_recv().ok())
            .find_map(|c| match c {
                Command::SearchRooms { query, .. } => Some(query),
                _ => None,
            })
            .expect("a search")
    }

    #[test]
    fn a_search_covers_the_server_by_default_and_narrows_with_in() {
        let (mut app, rx) = app();
        let _ = app.search_ready("!design:h".into(), Bookmark::default());
        let s = app.search.as_mut().unwrap();
        assert_eq!(s.scope, Scope::Space);
        s.input = "from:mira has:file".into();
        app.submit_search();
        let q = searched(&rx);
        assert_eq!(q.rooms, ["!archive:h", "!design:h"]);
        assert_eq!(
            (q.from, q.has, q.text),
            (vec!["mira".to_owned()], vec![Has::File], String::new())
        );
        app.search_scope(Scope::All);
        assert_eq!(searched(&rx).rooms.len(), 3);
        app.search.as_mut().unwrap().input = "in:arch plan".into();
        app.submit_search();
        assert_eq!(searched(&rx).rooms, ["!archive:h"]);
        // `in:` naming no room searches nothing.
        app.search.as_mut().unwrap().input = "in:nowhere plan".into();
        app.submit_search();
        assert!(rx.try_recv().is_err());
        assert!(app.search.as_ref().unwrap().nowhere);
    }

    #[test]
    fn a_hit_in_another_room_opens_that_room() {
        let (mut app, rx) = app();
        let _ = app.search_ready("!design:h".into(), Bookmark::default());
        app.search.as_mut().unwrap().input = "plan".into();
        app.submit_search();
        let _ = searched(&rx);
        let id = *app
            .pending
            .iter()
            .find(|(_, r)| matches!(r, Request::Search))
            .unwrap()
            .0;
        app.engine_event(Event::RoomsFound {
            id,
            result: Ok(RoomsFound {
                found: vec![Found {
                    room: "!archive:h".into(),
                    event_id: "$plan".into(),
                    sender: "@mira:h".into(),
                    body: "the plan.pdf".into(),
                    time: 5,
                }],
                reach: vec![Reach {
                    room: "!archive:h".into(),
                    server: true,
                    oldest: None,
                }],
                ..Default::default()
            }),
        });
        let s = app.search.as_ref().unwrap();
        assert_eq!(s.hits[0].snippet, "the plan.pdf");
        assert!(!s.busy && !s.local());
        let _ = app.update(Msg::SearchHit("!archive:h".into(), "$plan".into()));
        assert_eq!(app.selected, "!archive:h");
        assert!(app.search.is_none());
    }
}
