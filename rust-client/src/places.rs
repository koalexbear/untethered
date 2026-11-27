//! Remember where you were (ROADMAP N2), as Discord does: the room last open
//! in each section of the server rail, the room and section open when the app
//! closed, and how far up each room you'd scrolled, restored when you come
//! back and after a restart.
//!
//! While you're scrolled up, the first message showing, and how far below the
//! top of the timeline it starts, is noted a moment after the scrolling stops.
//! Coming back, the room's timeline has started over from its newest messages
//! (paging.rs), so older pages load until that message is there again; then
//! the shown window moves to it and the timeline scrolls it back into place.
//! Back at the newest messages, the room forgets its place.
use crate::{App, Msg, Section, search::bookmark};
use iced::{Task, widget::Id};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// How many older pages coming back may load to find the message.
const PAGES: u32 = 15;
/// Posts shown below the message once it's found.
const BELOW: usize = 60;
/// How long to wait before believing a room's start is reached.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// A room's place: the first message showing and its distance below the
/// top of the timeline (negative when partly scrolled past).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub event: String,
    pub dy: f32,
}

/// What's remembered, per account and device.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Places {
    /// Rooms scrolled away from their newest messages.
    #[serde(default)]
    pub rooms: BTreeMap<String, Place>,
    /// The room last open in each section, by `section_key`.
    #[serde(default)]
    pub last: BTreeMap<String, String>,
    /// The section and room open last.
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub open: Option<String>,
}

/// Coming back to a room with a place: older pages load until it's there.
#[derive(Clone, Debug)]
pub struct Return {
    pub room: String,
    pub place: Place,
    pub pages: u32,
    /// The room's timeline has sent its posts since it opened again: what
    /// it kept from before may go once it does.
    pub fresh: bool,
    /// How many posts the room had when its start first seemed reached:
    /// a snapshot can say so before a page's older posts are all in it.
    pub ended: Option<usize>,
}

pub fn section_key(section: &Section) -> String {
    match section {
        Section::All => "all".into(),
        Section::Direct => "direct".into(),
        Section::Space(id) => id.clone(),
    }
}

impl App {
    /// The selected room became the one open in the current section.
    pub(crate) fn note_open(&mut self) {
        if !self.logged_in || self.selected.is_empty() {
            return;
        }
        let key = section_key(&self.section);
        let changed = self.places.last.get(&key) != Some(&self.selected)
            || self.places.section.as_ref() != Some(&key)
            || self.places.open.as_ref() != Some(&self.selected);
        if changed {
            self.places.last.insert(key.clone(), self.selected.clone());
            self.places.section = Some(key);
            self.places.open = Some(self.selected.clone());
            self.places_changed();
        }
    }

    /// Opening a section opens the room last open there, if it still is.
    pub(crate) fn open_last_in_section(&mut self) {
        let Some(room) = self.places.last.get(&section_key(&self.section)).cloned() else {
            return;
        };
        if self.visible().any(|(id, _)| *id == room) {
            self.open(room);
        }
    }

    /// Once the room list first arrives after signing in, open the section
    /// and room open when the app last closed.
    pub(crate) fn restore_open_room(&mut self) {
        if self.places_restored || !self.logged_in || self.live.list.is_empty() {
            return;
        }
        self.places_restored = true;
        let section = match self.places.section.as_deref() {
            Some("all") | None => Section::All,
            Some("direct") => Section::Direct,
            Some(space) if self.live.list.get(space).is_some_and(|r| r.space) => {
                Section::Space(space.to_owned())
            }
            Some(_) => Section::All,
        };
        self.section = section;
        match self.places.open.clone() {
            Some(room) if self.visible().any(|(id, _)| *id == room) => self.open(room),
            _ => self.open_last_in_section(),
        }
    }

    /// A room just opened: start going back to its place, if it has one.
    pub(crate) fn start_return(&mut self) {
        self.returning = self.places.rooms.get(&self.selected).map(|place| Return {
            room: self.selected.clone(),
            place: place.clone(),
            pages: 0,
            fresh: false,
            ended: None,
        });
    }

    /// The timeline moved: note the place a moment after it stops, or forget
    /// it back at the newest messages.
    pub(crate) fn place_moved(&mut self, room: &str, away: bool) {
        if self.returning.is_some()
            || self.timeline_restoring
            || self.search.is_some()
            || self.original.is_some()
        {
            return;
        }
        if away {
            self.place_unnoted = true;
        } else if self.places.rooms.remove(room).is_some() {
            self.places_changed();
        }
    }

    /// Measure the first message showing in the open room.
    pub(crate) fn note_place(&mut self) -> Task<Msg> {
        self.place_unnoted = false;
        if !self.scrolled_away
            || self.returning.is_some()
            || self.timeline_restoring
            || self.search.is_some()
            || self.original.is_some()
        {
            return Task::none();
        }
        let room = self.selected.clone();
        let Some(r) = self.live.list.get(&room) else {
            return Task::none();
        };
        let posts: HashMap<Id, String> = r
            .posts_in(self.timeline_range(r))
            .filter_map(|p| p.event_id.clone())
            .map(|event| (Id::from(format!("post-{event}")), event))
            .collect();
        bookmark::first_visible(self.timeline_id(), posts)
            .map(move |hit| Msg::PlaceNoted(room.clone(), hit))
    }

    pub(crate) fn place_noted(&mut self, room: String, hit: Option<(String, f32)>) {
        let Some((event, dy)) = hit else {
            return;
        };
        if room != self.selected
            || !self.scrolled_away
            || self.returning.is_some()
            || self.timeline_restoring
        {
            return;
        }
        let place = Place { event, dy };
        if self.places.rooms.get(&room) != Some(&place) {
            self.places.rooms.insert(room, place);
            self.places_changed();
        }
    }

    /// Going back to the open room's place: load older pages until its
    /// message is there, then show it where it was. Runs after every update.
    pub(crate) fn continue_return(&mut self) -> Task<Msg> {
        let Some(ret) = &self.returning else {
            return Task::none();
        };
        let room = ret.room.clone();
        if room != self.selected
            || !self.logged_in
            || self.original.is_some()
            || self.search.is_some()
        {
            self.returning = None;
            return Task::none();
        }
        let Some(r) = self.live.list.get(&room) else {
            return Task::none();
        };
        if !ret.fresh {
            return Task::none();
        }
        if let Some(index) = r.index_of(&ret.place.event) {
            let len = r.posts().len();
            let end = (index + BELOW).min(len);
            self.timeline_end = if end >= len { None } else { r.key_at(end) };
            let place = self.returning.take().unwrap().place;
            self.scrolled_away = true;
            self.timeline_restoring = true;
            return bookmark::restore_row(
                self.timeline_id(),
                bookmark::Bookmark::at(&place.event, place.dy),
            )
            .chain(Task::done(Msg::TimelineRestored));
        }
        if !self.verified
            || !self.loaded.contains(&room)
            || self.released.contains(&room)
            || self.reopening.as_ref() == Some(&room)
            || self.paging(&room)
        {
            return Task::none();
        }
        let len = r.posts().len();
        let gone = r.end && ret.ended == Some(len);
        if r.end && !gone {
            // Look again in a moment, once the page's last posts are in.
            if let Some(ret) = &mut self.returning {
                ret.ended = Some(len);
            }
            return Task::perform(crate::engine::rt::sleep(SETTLE), |()| Msg::SavePlaces);
        }
        if gone || ret.pages >= PAGES {
            // Gone, or too far back: the room stays at its newest messages.
            self.returning = None;
            if self.places.rooms.remove(&room).is_some() {
                self.places_changed();
            }
            return Task::none();
        }
        self.page();
        if self.paging(&room)
            && let Some(ret) = &mut self.returning
        {
            ret.pages += 1;
        }
        Task::none()
    }

    pub(crate) fn places_changed(&mut self) {
        if self.logged_in && !self.matrix_id.is_empty() {
            self.places_dirty = true;
            #[cfg(target_arch = "wasm32")]
            self.persist_places();
        }
    }

    pub(crate) fn load_places(&mut self) {
        self.places_dirty = false;
        self.places_restored = false;
        self.returning = None;
        self.places = Places::default();
        if !self.logged_in || self.matrix_id.is_empty() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let loaded = crate::engine::profile_root()
            .ok()
            .and_then(|root| store::load(&root, &self.matrix_id).ok());
        #[cfg(target_arch = "wasm32")]
        let loaded = store::load(&self.matrix_id).ok();
        if let Some(places) = loaded {
            self.places = places;
        }
    }

    pub(crate) fn persist_places(&mut self) {
        if !std::mem::take(&mut self.places_dirty) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = crate::engine::profile_root()
            .ok()
            .map(|root| store::save(&root, &self.matrix_id, &self.places));
        #[cfg(target_arch = "wasm32")]
        let _ = store::save(&self.matrix_id, &self.places);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    use super::Places;
    use std::{fs, io, path::Path};

    fn path(root: &Path, account: &str) -> std::path::PathBuf {
        root.join("places")
            .join(crate::drafts::account_key(account))
    }

    pub fn load(root: &Path, account: &str) -> io::Result<Places> {
        match fs::read(path(root, account)) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Places::default()),
            Err(err) => Err(err),
        }
    }

    pub fn save(root: &Path, account: &str, places: &Places) -> io::Result<()> {
        let path = path(root, account);
        let dir = path.parent().expect("a places file has a folder");
        fs::create_dir_all(dir)?;
        let mut pending = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer(&mut pending, places)?;
        pending.persist(path)?;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod store {
    use super::Places;
    use wasm_bindgen::JsValue;

    fn key(account: &str) -> String {
        format!(
            "untethered:places:v1:{}",
            crate::drafts::account_key(account)
        )
    }

    fn storage() -> Result<web_sys::Storage, JsValue> {
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("No browser window"))?
            .local_storage()?
            .ok_or_else(|| JsValue::from_str("Local storage unavailable"))
    }

    pub fn load(account: &str) -> Result<Places, JsValue> {
        storage()?
            .get_item(&key(account))?
            .map(|value| {
                serde_json::from_str(&value).map_err(|e| JsValue::from_str(&e.to_string()))
            })
            .unwrap_or_else(|| Ok(Places::default()))
    }

    pub fn save(account: &str, places: &Places) -> Result<(), JsValue> {
        let value = serde_json::to_string(places).map_err(|e| JsValue::from_str(&e.to_string()))?;
        storage()?.set_item(&key(account), &value)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::{
        Request,
        engine::{Command, Event, Message, RoomList, RoomSummary, TimelineSnapshot},
    };

    fn app() -> (App, async_channel::Receiver<Command>) {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, rx) = async_channel::bounded(256);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.matrix_id = "@me:h".into();
        (app, rx)
    }

    fn rooms(app: &mut App) {
        let room = |id: &str, name: &str| RoomSummary {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let mut children = BTreeMap::new();
        children.insert(
            "!space:h".to_owned(),
            std::collections::BTreeSet::from(["!b:h".to_owned(), "!c:h".to_owned()]),
        );
        app.engine_event(Event::Rooms(RoomList {
            invites: Vec::new(),
            rooms: vec![
                room("!a:h", "alpha"),
                room("!b:h", "bravo"),
                room("!c:h", "charlie"),
                RoomSummary {
                    space: true,
                    ..room("!space:h", "Space")
                },
                RoomSummary {
                    dm: true,
                    dm_peer: Some("@mira:h".into()),
                    ..room("!dm:h", "Mira")
                },
            ],
            children,
        }));
    }

    fn message(row: i64) -> Message {
        Message {
            row,
            event_id: Some(format!("${row}")),
            sender: "@mira:h".into(),
            sender_name: None,
            sender_avatar: None,
            time: row,
            body: format!("message {row}"),
            formatted: None,
            kind: crate::engine::Kind::Text,
            encrypted: false,
            delivery: crate::engine::Delivery::Sent,
            edited: false,
            reply_to: None,
            reactions: Vec::new(),
            text: true,
            editable: false,
            mentions: None,
            edit_id: None,
            edit_time: None,
            receipts: Vec::new(),
            media: None,
            from: None,
        }
    }

    fn snapshot(room: &str, rows: std::ops::RangeInclusive<i64>, start: bool) -> Event {
        Event::Timeline(TimelineSnapshot {
            room: room.into(),
            messages: rows.map(message).collect(),
            start_reached: start,
            latest_event: None,
            seams: Vec::new(),
        })
    }

    /// Answer the open room's history request, as the engine would.
    fn page_done(app: &mut App) -> bool {
        let Some(id) = app
            .pending
            .iter()
            .find(|(_, r)| matches!(r, Request::Page(_)))
            .map(|(id, _)| *id)
        else {
            return false;
        };
        app.engine_event(Event::Done { id, result: Ok(()) });
        true
    }

    #[test]
    fn each_section_opens_the_room_last_open_there() {
        let (mut app, _rx) = app();
        rooms(&mut app);
        let _ = app.update(Msg::Section(Section::Space("!space:h".into())));
        let _ = app.update(Msg::Open("!c:h".into()));
        let _ = app.update(Msg::Section(Section::All));
        let _ = app.update(Msg::Open("!a:h".into()));
        let _ = app.update(Msg::Section(Section::Space("!space:h".into())));
        assert_eq!(app.selected, "!c:h");
        let _ = app.update(Msg::Section(Section::All));
        assert_eq!(app.selected, "!a:h");
        assert_eq!(app.places.open.as_deref(), Some("!a:h"));
        assert_eq!(app.places.section.as_deref(), Some("all"));
    }

    #[test]
    fn signing_in_opens_the_room_open_when_the_app_closed() {
        let (mut app, _rx) = app();
        app.places.section = Some("direct".into());
        app.places.open = Some("!dm:h".into());
        rooms(&mut app);
        assert!(app.section == Section::Direct);
        assert_eq!(app.selected, "!dm:h");
        // Only once: later room lists leave you where you are.
        let _ = app.update(Msg::Section(Section::All));
        let _ = app.update(Msg::Open("!b:h".into()));
        rooms(&mut app);
        assert_eq!(app.selected, "!b:h");
    }

    #[test]
    fn coming_back_loads_older_pages_until_the_place_is_there() {
        let (mut app, rx) = app();
        app.places.open = Some("!b:h".into());
        app.places.rooms.insert(
            "!b:h".into(),
            Place {
                event: "$20".into(),
                dy: 8.,
            },
        );
        rooms(&mut app);
        assert_eq!(app.selected, "!b:h");
        assert!(app.returning.is_some());
        // The first page brings the newest posts, not yet the place.
        assert!(page_done(&mut app));
        app.engine_event(snapshot("!b:h", 200..=300, false));
        while rx.try_recv().is_ok() {}
        let _ = app.update(Msg::SavePlaces);
        assert!(
            std::iter::from_fn(|| rx.try_recv().ok())
                .any(|c| matches!(c, Command::Paginate { room, .. } if room == "!b:h")),
            "an older page"
        );
        // While it pages, the room doesn't start over from its newest.
        let _ = app.bound_history();
        assert!(app.reopening.is_none());
        assert!(page_done(&mut app));
        app.engine_event(snapshot("!b:h", 1..=300, false));
        let _ = app.update(Msg::SavePlaces);
        assert!(app.returning.is_none());
        assert!(app.timeline_restoring && app.scrolled_away);
        // The shown window has the message, with posts below it.
        let r = &app.live.list["!b:h"];
        let range = app.timeline_range(r);
        let index = r.index_of("$20").unwrap();
        assert!(range.contains(&index) && range.end > index + 30);
    }

    #[test]
    fn a_snapshot_that_reaches_the_start_too_soon_is_looked_at_again() {
        let (mut app, _rx) = app();
        app.places.open = Some("!b:h".into());
        app.places.rooms.insert(
            "!b:h".into(),
            Place {
                event: "$20".into(),
                dy: 0.,
            },
        );
        rooms(&mut app);
        assert!(page_done(&mut app));
        // The start seems reached, but the page's older posts aren't in yet.
        app.engine_event(snapshot("!b:h", 100..=300, true));
        let _ = app.update(Msg::SavePlaces);
        assert!(app.returning.as_ref().is_some_and(|r| r.ended == Some(201)));
        app.engine_event(snapshot("!b:h", 1..=300, true));
        let _ = app.update(Msg::SavePlaces);
        assert!(app.returning.is_none());
        assert!(app.places.rooms.contains_key("!b:h"), "found, not given up");
        // Truly gone: the same count twice gives up, and forgets the place.
        app.places.rooms.insert(
            "!c:h".into(),
            Place {
                event: "$999".into(),
                dy: 0.,
            },
        );
        let _ = app.update(Msg::Open("!c:h".into()));
        assert!(page_done(&mut app));
        app.engine_event(snapshot("!c:h", 1..=50, true));
        let _ = app.update(Msg::SavePlaces);
        assert!(app.returning.is_some());
        let _ = app.update(Msg::SavePlaces);
        assert!(app.returning.is_none());
        assert!(!app.places.rooms.contains_key("!c:h"));
    }

    #[test]
    fn a_place_is_noted_while_away_and_forgotten_at_the_newest() {
        let (mut app, _rx) = app();
        rooms(&mut app);
        let _ = app.update(Msg::Open("!b:h".into()));
        let away = crate::paging::Scrolled {
            away: true,
            ..Default::default()
        };
        let _ = app.update(Msg::ReadingScroll("!b:h".into(), away));
        assert!(app.place_unnoted);
        let _ = app.update(Msg::PlaceNoted("!b:h".into(), Some(("$5".into(), -3.))));
        assert_eq!(app.places.rooms["!b:h"].event, "$5");
        assert!(app.places_dirty);
        let _ = app.update(Msg::ReadingScroll("!b:h".into(), Default::default()));
        assert!(!app.places.rooms.contains_key("!b:h"));
        // A measurement that arrives once you're back at the newest is dropped.
        let _ = app.update(Msg::PlaceNoted("!b:h".into(), Some(("$5".into(), -3.))));
        assert!(!app.places.rooms.contains_key("!b:h"));
    }

    #[test]
    fn places_round_trip_by_account() {
        let root = tempfile::tempdir().unwrap();
        let mut places = Places::default();
        places.rooms.insert(
            "!a:h".into(),
            Place {
                event: "$e".into(),
                dy: -12.5,
            },
        );
        places.last.insert("all".into(), "!a:h".into());
        places.section = Some("all".into());
        places.open = Some("!a:h".into());
        store::save(root.path(), "@me:h", &places).unwrap();
        assert_eq!(store::load(root.path(), "@me:h").unwrap(), places);
        assert_eq!(
            store::load(root.path(), "@you:h").unwrap(),
            Places::default()
        );
    }
}
