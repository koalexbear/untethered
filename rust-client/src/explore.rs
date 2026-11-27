//! Explore (ROADMAP J4): the homeserver's public room directory, as
//! Discord's Explore Discoverable Servers. The compass under the server
//! rail's **+** opens it; a search box and the Spaces and Rooms filters ask
//! the directory as you type (engine/explore.rs), and a result opens the
//! J2 preview (joining.rs), with **Join**.
//!
//! A space joined here would show no channels: the room list holds joined
//! rooms only. So a space's channels this account hasn't joined come from
//! its hierarchy and show in its channel list, their icons dimmed, and open the same
//! preview, rather than joining every room at once as Discord does
//! (homeservers limit how fast an account joins rooms).
use crate::{
    App, Msg, Request, Section,
    engine::{
        Command, Failure, JoinRule, RoomPreview,
        explore::{Directory, DirectoryQuery, Kind, PublicRoom, SpaceChild},
    },
    joining::RoomLink,
};
use iced::{Task, widget::operation};
use std::collections::{BTreeMap, BTreeSet};

pub const SEARCH: &str = "explore-search";

#[derive(Default)]
pub struct State {
    /// The Explore page, while it's open.
    pub page: Option<Page>,
    /// What each space contains, from its hierarchy, and the room list's
    /// hierarchy revision it was asked at.
    spaces: BTreeMap<String, (u64, Vec<SpaceChild>)>,
}

#[derive(Default)]
pub struct Page {
    pub query: String,
    pub kind: Kind,
    /// The directory's rooms for `shown`, page after page. Kept while the
    /// next search runs, so the list doesn't flicker as you type.
    pub rooms: Vec<PublicRoom>,
    /// The search `rooms` answers: its trimmed term and kind.
    pub shown: Option<(String, Kind)>,
    /// The token for the directory's next page, while there is one.
    pub next: Option<String>,
    /// The homeserver's estimate of how many rooms match.
    pub total: Option<u64>,
    /// The directory didn't answer the latest search.
    pub failed: bool,
}

#[derive(Clone, Debug)]
pub enum Input {
    Open,
    /// Close the preview open over Explore, else Explore.
    Close,
    Query(String),
    Kind(Kind),
    /// The directory's next page.
    More,
    Retry,
    /// A result: open it if this account is in it, else preview it.
    Pick(String),
    /// A channel in the open space this account hasn't joined.
    Channel(String),
}

impl Page {
    /// The search the page shows now, as the directory is asked it.
    fn search(&self) -> (String, Kind) {
        (self.query.trim().to_owned(), self.kind)
    }

    fn answers(&self, query: &DirectoryQuery) -> bool {
        (query.term.as_str(), query.kind) == (self.query.trim(), self.kind)
    }
}

/// The J2 preview's details for a room the directory or a hierarchy lists.
fn preview(room: &PublicRoom) -> (RoomLink, RoomPreview) {
    let link = RoomLink {
        target: room.room_id.clone(),
        via: room.via.clone(),
        event: None,
    };
    let preview = RoomPreview {
        room_id: room.room_id.clone(),
        alias: room.alias.clone(),
        name: room.name.clone(),
        topic: room.topic.clone(),
        avatar: room.avatar.clone(),
        members: room.members,
        space: room.space,
        joined: false,
        join: room.join,
    };
    (link, preview)
}

/// The channels a space's hierarchy lists that `joined` doesn't hold, and
/// that this account may join: in `space` itself or in a joined space under
/// it, as (text, voice), by name.
fn unjoined<'a>(
    space: &str,
    children: &'a [SpaceChild],
    joined: impl Fn(&str) -> Option<bool>,
) -> (Vec<&'a PublicRoom>, Vec<&'a PublicRoom>) {
    let mut seen = BTreeSet::new();
    let mut rooms: Vec<&PublicRoom> = children
        .iter()
        .filter(|c| c.parent == space || joined(&c.parent) == Some(true))
        .map(|c| &c.room)
        .filter(|r| !r.space && matches!(r.join, JoinRule::Open | JoinRule::Members))
        .filter(|r| joined(&r.room_id).is_none())
        .filter(|r| seen.insert(r.room_id.as_str()))
        .collect();
    rooms.sort_by_cached_key(|r| (r.title().to_lowercase(), r.room_id.clone()));
    rooms.into_iter().partition(|r| !r.voice)
}

impl App {
    pub fn explore(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Open if self.logged_in && self.verified && self.sdk.is_some() => {
                // Open already: keep its search.
                let page = self.explore.page.take();
                self.close_mentions();
                self.cancel_setup_if_idle();
                self.room_preview = None;
                self.invite_open = None;
                self.account = false;
                self.server_menu = false;
                self.context_menu.dismiss();
                self.explore.page = page;
                if self.explore.page.is_none() {
                    self.explore.page = Some(Page::default());
                    self.search_directory(None);
                }
                return operation::focus(SEARCH);
            }
            Input::Close if self.room_preview.is_some() => self.room_preview = None,
            Input::Close => self.explore.page = None,
            Input::Query(query) => {
                let Some(page) = &mut self.explore.page else {
                    return Task::none();
                };
                let before = page.search();
                page.query = query;
                if page.search() != before {
                    self.search_directory(None);
                }
            }
            Input::Kind(kind) => {
                let Some(page) = self.explore.page.as_mut().filter(|p| p.kind != kind) else {
                    return Task::none();
                };
                page.kind = kind;
                self.search_directory(None);
            }
            Input::More => {
                let next = self.explore.page.as_ref().and_then(|p| p.next.clone());
                if next.is_some() && !self.loading_more() {
                    self.search_directory(next);
                }
            }
            Input::Retry => self.search_directory(None),
            Input::Pick(id) => {
                let Some(room) = self
                    .explore
                    .page
                    .as_ref()
                    .and_then(|p| p.rooms.iter().find(|r| r.room_id == id))
                else {
                    return Task::none();
                };
                if self.live.list.contains_key(&id) {
                    // Already in it: open it, as the room list would.
                    self.explore.page = None;
                    self.joined_invite = Some(id);
                    self.open_joined_invite();
                } else {
                    let (link, preview) = preview(room);
                    // The preview shows over Explore, which stays open
                    // behind it.
                    let page = self.explore.page.take();
                    self.preview_listed(link, preview);
                    self.explore.page = page;
                }
            }
            Input::Channel(id) => {
                let (text, voice) = self.unjoined_channels();
                let found = text.into_iter().chain(voice).find(|r| r.room_id == id);
                if let Some((link, preview)) = found.map(preview) {
                    self.preview_listed(link, preview);
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// Ask the directory for what the page's search box and filter say:
    /// from the first page, or the page after `since`.
    fn search_directory(&mut self, since: Option<String>) {
        let Some(page) = &mut self.explore.page else {
            return;
        };
        page.failed = false;
        let (term, kind) = page.search();
        let query = DirectoryQuery { term, kind, since };
        let request = Request::Explore(query.clone());
        self.command(move |id| Command::PublicRooms { id, query }, request);
    }

    /// A first page for what the page shows now is on its way.
    pub fn searching_directory(&self) -> bool {
        let Some(page) = &self.explore.page else {
            return false;
        };
        self.pending
            .values()
            .any(|r| matches!(r, Request::Explore(q) if q.since.is_none() && page.answers(q)))
    }

    pub fn loading_more(&self) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Request::Explore(q) if q.since.is_some()))
    }

    /// The directory's answer, if it's for what the page shows now.
    pub fn directory_found(&mut self, id: i64, result: Result<Directory, Failure>) {
        let Some(Request::Explore(query)) = self.pending.remove(&id) else {
            return;
        };
        let Some(page) = self.explore.page.as_mut().filter(|p| p.answers(&query)) else {
            return;
        };
        let more = query.since.is_some();
        // A later page for a search the page no longer continues.
        if more && (page.next != query.since || page.shown != Some(page.search())) {
            return;
        }
        match result {
            Ok(found) => {
                if more {
                    let known: BTreeSet<String> =
                        page.rooms.iter().map(|r| r.room_id.clone()).collect();
                    page.rooms.extend(
                        found
                            .rooms
                            .into_iter()
                            .filter(|r| !known.contains(&r.room_id)),
                    );
                } else {
                    page.rooms = found.rooms;
                    page.total = found.total;
                }
                page.next = found.next;
                page.shown = Some(page.search());
                page.failed = false;
            }
            Err(_) if more => page.failed = true,
            Err(_) => {
                page.rooms.clear();
                page.next = None;
                page.total = None;
                page.shown = Some(page.search());
                page.failed = true;
            }
        }
    }

    /// Ask for what the open space contains, once per change to the room
    /// list's spaces (main.rs calls it whenever the section may change).
    pub fn ask_space_rooms(&mut self) {
        let Section::Space(space) = &self.section else {
            return;
        };
        let revision = self.live.hierarchy_revision;
        if !self.logged_in
            || !self.verified
            || !self.connected
            || self
                .explore
                .spaces
                .get(space)
                .is_some_and(|(r, _)| *r == revision)
        {
            return;
        }
        let Some(sdk) = &self.sdk else {
            return;
        };
        if sdk
            .try_send(Command::SpaceRooms {
                space: space.clone(),
            })
            .is_ok()
        {
            // Keep what it held until the answer comes.
            let children = self
                .explore
                .spaces
                .remove(space)
                .map(|(_, c)| c)
                .unwrap_or_default();
            self.explore
                .spaces
                .insert(space.clone(), (revision, children));
        }
    }

    pub fn space_rooms_found(&mut self, space: String, result: Result<Vec<SpaceChild>, Failure>) {
        // A homeserver without the hierarchy lists nothing more.
        let Ok(children) = result else {
            return;
        };
        let revision = self
            .explore
            .spaces
            .get(&space)
            .map_or(self.live.hierarchy_revision, |(r, _)| *r);
        self.explore.spaces.insert(space, (revision, children));
    }

    /// The open space's channels this account hasn't joined, as (text,
    /// voice), by name. Invites show as invites instead.
    pub fn unjoined_channels(&self) -> (Vec<&PublicRoom>, Vec<&PublicRoom>) {
        let Section::Space(space) = &self.section else {
            return Default::default();
        };
        let Some((_, children)) = self.explore.spaces.get(space).filter(|_| self.logged_in) else {
            return Default::default();
        };
        unjoined(space, children, |id| {
            self.live.list.get(id).map(|r| r.space).or_else(|| {
                self.live
                    .invites
                    .iter()
                    .any(|i| i.id == id)
                    .then_some(false)
            })
        })
    }

    /// The pictures Explore's results show (avatars.rs).
    pub fn explore_avatars(&self) -> impl Iterator<Item = &str> {
        let page = self.explore.page.iter().flat_map(|p| &p.rooms);
        page.filter_map(|r| r.avatar.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: &str, name: &str, space: bool, join: JoinRule) -> PublicRoom {
        PublicRoom {
            room_id: id.into(),
            alias: None,
            name: Some(name.into()),
            topic: None,
            avatar: None,
            members: 3,
            space,
            voice: false,
            join,
            via: vec!["example.org".into()],
        }
    }

    fn child(parent: &str, room: PublicRoom) -> SpaceChild {
        SpaceChild {
            parent: parent.into(),
            room,
        }
    }

    fn page(query: &str, kind: Kind) -> Page {
        Page {
            query: query.into(),
            kind,
            ..Default::default()
        }
    }

    #[test]
    fn unjoined_channels_are_the_joinable_rooms_not_joined_yet() {
        let mut voice = room("!v", "Voice", false, JoinRule::Open);
        voice.voice = true;
        let children = vec![
            child("!space", room("!b", "lounge", false, JoinRule::Open)),
            child(
                "!space",
                room("!a", "Announcements", false, JoinRule::Members),
            ),
            child("!space", room("!joined", "General", false, JoinRule::Open)),
            child("!space", room("!invite", "Staff", false, JoinRule::Invite)),
            child("!space", room("!knock", "Door", false, JoinRule::Knock)),
            child("!space", room("!sub", "Sub", true, JoinRule::Open)),
            child("!space", voice),
            // Under a subspace this account is in, and under one it isn't.
            child("!sub", room("!c", "Crafts", false, JoinRule::Open)),
            child("!other", room("!d", "Elsewhere", false, JoinRule::Open)),
            // Listed by two spaces, shown once.
            child("!sub", room("!b", "lounge", false, JoinRule::Open)),
        ];
        let joined = |id: &str| match id {
            "!joined" => Some(false),
            "!sub" => Some(true),
            _ => None,
        };
        let (text, voice) = unjoined("!space", &children, joined);
        let ids: Vec<&str> = text.iter().map(|r| r.room_id.as_str()).collect();
        assert_eq!(ids, ["!a", "!c", "!b"]);
        assert_eq!(voice.len(), 1);
        assert_eq!(voice[0].room_id, "!v");
    }

    #[test]
    fn a_page_answers_its_own_search_only() {
        let p = page("  lounge ", Kind::Spaces);
        let asked = |term: &str, kind| DirectoryQuery {
            term: term.into(),
            kind,
            since: None,
        };
        assert!(p.answers(&asked("lounge", Kind::Spaces)));
        assert!(!p.answers(&asked("lounge", Kind::All)));
        assert!(!p.answers(&asked("loung", Kind::Spaces)));
    }

    #[test]
    fn a_listed_room_previews_with_its_details_and_servers() {
        let mut r = room("!a:example.org", "Lounge", true, JoinRule::Members);
        r.topic = Some("Chat".into());
        r.alias = Some("#lounge:example.org".into());
        let (link, p) = preview(&r);
        assert_eq!(link.target, "!a:example.org");
        assert_eq!(link.via, ["example.org"]);
        assert_eq!(link.event, None);
        assert_eq!(p.room_id, "!a:example.org");
        assert_eq!(p.name.as_deref(), Some("Lounge"));
        assert_eq!(p.alias.as_deref(), Some("#lounge:example.org"));
        assert_eq!(p.topic.as_deref(), Some("Chat"));
        assert!(p.space && !p.joined);
        assert_eq!(p.join, JoinRule::Members);
    }

    mod app {
        use super::*;
        use crate::engine::{Event, RoomList, RoomSummary};
        use async_channel::Receiver;

        fn app() -> (App, Receiver<Command>) {
            let (gomuks, _) = async_channel::bounded(1);
            let (tx, rx) = async_channel::bounded(64);
            let mut app = App::new(gomuks);
            app.sdk = Some(tx);
            app.logged_in = true;
            app.verified = true;
            app.connected = true;
            app.matrix_id = "@me:h".into();
            (app, rx)
        }

        /// The next directory search the App asked for, skipping others.
        fn asked(rx: &Receiver<Command>) -> (i64, DirectoryQuery) {
            while let Ok(c) = rx.try_recv() {
                if let Command::PublicRooms { id, query } = c {
                    return (id, query);
                }
            }
            panic!("no directory search");
        }

        fn found(rooms: Vec<PublicRoom>, next: Option<&str>) -> Result<Directory, Failure> {
            Ok(Directory {
                rooms,
                next: next.map(Into::into),
                total: Some(40),
            })
        }

        fn shown(app: &App) -> Vec<&str> {
            let page = app.explore.page.as_ref().unwrap();
            page.rooms.iter().map(|r| r.room_id.as_str()).collect()
        }

        #[test]
        fn explore_searches_as_you_type_and_pages_on() {
            let (mut app, rx) = app();
            let _ = app.update(Msg::Explore(Input::Open));
            let (first, q) = asked(&rx);
            assert_eq!(q, DirectoryQuery::default());
            assert!(app.searching_directory());
            let _ = app.update(Msg::Explore(Input::Query(" lou".into())));
            let (typed, q) = asked(&rx);
            assert_eq!((q.term.as_str(), q.kind, q.since), ("lou", Kind::All, None));
            // A space typed after it changes nothing to search for.
            let _ = app.update(Msg::Explore(Input::Query(" lou ".into())));
            assert!(rx.try_recv().is_err());
            // The first search's answer comes late: dropped.
            let lounge = room("!lounge:h", "Lounge", true, JoinRule::Open);
            app.engine_event(Event::PublicRooms {
                id: first,
                result: found(vec![room("!old:h", "Old", false, JoinRule::Open)], None),
            });
            assert!(shown(&app).is_empty());
            app.engine_event(Event::PublicRooms {
                id: typed,
                result: found(vec![lounge.clone()], Some("t1")),
            });
            assert_eq!(shown(&app), ["!lounge:h"]);
            assert!(!app.searching_directory());
            let _ = app.update(Msg::Explore(Input::More));
            let (more, q) = asked(&rx);
            assert_eq!(q.since.as_deref(), Some("t1"));
            assert!(app.loading_more());
            // Asking again while it loads asks nothing more.
            let _ = app.update(Msg::Explore(Input::More));
            assert!(rx.try_recv().is_err());
            // The next page adds what's new, once.
            app.engine_event(Event::PublicRooms {
                id: more,
                result: found(
                    vec![lounge, room("!crafts:h", "Crafts", false, JoinRule::Open)],
                    None,
                ),
            });
            assert_eq!(shown(&app), ["!lounge:h", "!crafts:h"]);
            assert_eq!(app.explore.page.as_ref().unwrap().next, None);
            // Spaces only: a new search; its failure says so.
            let _ = app.update(Msg::Explore(Input::Kind(Kind::Spaces)));
            let (spaces, q) = asked(&rx);
            assert_eq!(q.kind, Kind::Spaces);
            app.engine_event(Event::PublicRooms {
                id: spaces,
                result: Err(Failure::Unreachable),
            });
            let page = app.explore.page.as_ref().unwrap();
            assert!(page.failed && page.rooms.is_empty());
            let _ = app.update(Msg::Explore(Input::Retry));
            assert_eq!(asked(&rx).1.kind, Kind::Spaces);
            let _ = app.update(Msg::Escape);
            assert!(app.explore.page.is_none());
        }

        #[test]
        fn a_space_found_previews_joins_and_lists_its_channels() {
            let (mut app, rx) = app();
            let _ = app.update(Msg::Explore(Input::Open));
            let (id, _) = asked(&rx);
            let mut space = room("!space:h", "Lounge", true, JoinRule::Open);
            space.via = vec!["h".into()];
            app.engine_event(Event::PublicRooms {
                id,
                result: found(vec![space], None),
            });
            // The preview shows over Explore; Escape goes back to it.
            let _ = app.update(Msg::Explore(Input::Pick("!space:h".into())));
            assert!(matches!(
                app.room_preview.as_ref().map(|p| &p.state),
                Some(crate::joining::PreviewState::Ready(p)) if p.space && p.name.as_deref() == Some("Lounge")
            ));
            assert!(app.explore.page.is_some());
            let _ = app.update(Msg::Escape);
            assert!(app.room_preview.is_none() && app.explore.page.is_some());
            let _ = app.update(Msg::Explore(Input::Pick("!space:h".into())));
            let _ = app.update(Msg::JoinPreviewed);
            let join = loop {
                match rx.try_recv() {
                    Ok(Command::JoinRoom { id, target, via }) => break (id, target, via),
                    Ok(_) => {}
                    Err(_) => panic!("no join"),
                }
            };
            assert_eq!(
                (join.1.as_str(), join.2.as_slice()),
                ("!space:h", ["h".to_owned()].as_slice())
            );
            app.engine_event(Event::Done {
                id: join.0,
                result: Ok(()),
            });
            assert!(app.explore.page.is_none());
            // Listed, the space opens, and its contents are asked for.
            app.engine_event(Event::Rooms(RoomList {
                rooms: vec![RoomSummary {
                    id: "!space:h".into(),
                    name: "Lounge".into(),
                    space: true,
                    ..Default::default()
                }],
                ..Default::default()
            }));
            assert!(app.section == Section::Space("!space:h".into()));
            let asked_space = std::iter::from_fn(|| rx.try_recv().ok())
                .any(|c| matches!(c, Command::SpaceRooms { space } if space == "!space:h"));
            assert!(asked_space);
            let mut voice = room("!voice:h", "Hangout", false, JoinRule::Members);
            voice.voice = true;
            app.engine_event(Event::SpaceRooms {
                space: "!space:h".into(),
                result: Ok(vec![
                    child(
                        "!space:h",
                        room("!general:h", "general", false, JoinRule::Open),
                    ),
                    child("!space:h", voice),
                ]),
            });
            let (text, voice) = app.unjoined_channels();
            assert_eq!(text[0].room_id, "!general:h");
            assert_eq!(voice[0].room_id, "!voice:h");
            // Nothing changed in the room list: not asked again.
            app.ask_space_rooms();
            assert!(rx.try_recv().is_err());
            // A channel previews with its space's servers.
            let _ = app.update(Msg::Explore(Input::Channel("!general:h".into())));
            let preview = app.room_preview.as_ref().unwrap();
            assert_eq!(preview.link.target, "!general:h");
            assert_eq!(preview.link.via, ["example.org"]);
        }
    }
}
