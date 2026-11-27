//! Loading older (and newer) messages as the timeline scrolls, as Discord
//! does: within a screen of the top, older messages come from the server,
//! and at the very top the shown window moves back through what's loaded.
//! The timeline reports where it is whenever that changes, and the check
//! runs after every update, so loading carries on while you stay at the top
//! even when nothing moves: a page that lands outside the shown window
//! changes no scroll position to report, and scrolling up at the top
//! reports nothing new either.
//!
//! Loaded history is let go again: a room's timeline closes when you leave
//! it, keeping its newest posts, and the open room starts over from its
//! newest messages once you're back there with a long history loaded. What
//! goes stays in the engine's event cache on disk and loads again quickly.
use crate::{App, Msg};
use iced::{Task, widget::scrollable::Viewport};

/// Where the timeline is scrolled to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scrolled {
    /// Away from the newest message at all, and by a screen or more.
    pub away: bool,
    pub far: bool,
    /// How far up its shown window: 0 at the newest message, 1 at the oldest.
    pub depth: f32,
    /// Within a screen of the top, within a few pixels of it, and within a
    /// few pixels of the bottom.
    pub near_top: bool,
    pub at_top: bool,
    pub at_bottom: bool,
}

/// How close to an edge counts as at it.
const EDGE: f32 = 24.;

impl Scrolled {
    /// The timeline is anchored at the bottom: its offset counts up from the
    /// newest message. `has_newer`: the shown window ends before the newest
    /// loaded message.
    pub fn of(v: Viewport, has_newer: bool) -> Self {
        let from_bottom = v.absolute_offset().y;
        let from_top = v.absolute_offset_reversed().y;
        let screen = v.bounds().height;
        let travel = (v.content_bounds().height - screen).max(1.);
        Self {
            away: has_newer || from_bottom > 1.,
            far: has_newer || from_bottom > screen * 1.5,
            depth: (from_bottom / travel).clamp(0., 1.),
            near_top: from_top < screen,
            at_top: from_top < EDGE,
            at_bottom: from_bottom < EDGE,
        }
    }
}

/// Posts a room keeps once its timeline closes.
const KEEP: usize = 60;
/// Posts loaded in the open room before it starts over from its newest,
/// once you're back there.
const LIMIT: usize = 400;
/// Between closing the open room's timeline and opening it again.
const REOPEN: std::time::Duration = std::time::Duration::from_millis(600);

impl App {
    /// `room` is no longer shown: its timeline closes, and it keeps its
    /// newest posts until it opens again (`load_selected`).
    pub fn release_timeline(&mut self, room: String) {
        if !self.logged_in || room.is_empty() {
            return;
        }
        if let Some(sdk) = &self.sdk {
            let _ = sdk.try_send(crate::engine::Command::CloseTimeline { room: room.clone() });
        }
        if let Some(r) = self.live.list.get_mut(&room) {
            r.keep_newest(KEEP);
        }
        self.released.insert(room);
    }

    /// Back at the newest message after loading a long history, start the
    /// open room over from there. Also open the room's timeline again if it
    /// closed and couldn't reopen yet (offline).
    pub fn bound_history(&mut self) -> Task<Msg> {
        let room = self.selected.clone();
        if self.released.contains(&room) {
            if self.reopening.as_ref() != Some(&room) {
                self.load_selected();
            }
            return Task::none();
        }
        if !self.logged_in
            || self.scrolled_away
            || self.restarted.as_ref() == Some(&room)
            || self.timeline_end.is_some()
            || self.original.is_some()
            || self.search.is_some()
            || self.editing.is_some()
            || self.timeline_restoring
            || self.paging(&room)
            || self
                .live
                .list
                .get(&room)
                .is_none_or(|r| r.posts().len() <= LIMIT)
        {
            return Task::none();
        }
        // Once per visit to the newest messages: a timeline that opens with
        // more (the SDK still holding them) mustn't start over again.
        self.restarted = Some(room.clone());
        self.release_timeline(room.clone());
        // The SDK lets go of a room's events once its last timeline is gone,
        // which takes a moment; opening again at once would get them back.
        self.reopening = Some(room.clone());
        Task::perform(crate::engine::rt::sleep(REOPEN), move |()| {
            Msg::ReopenTimeline(room)
        })
    }

    /// The pause after closing the open room's timeline is over.
    pub fn reopen_timeline(&mut self, room: String) {
        if self.reopening.as_ref() == Some(&room) {
            self.reopening = None;
            if room == self.selected {
                self.load_selected();
            }
        }
    }

    /// Load more while the timeline sits at an edge with more beyond it.
    pub fn keep_paging(&mut self) -> Task<Msg> {
        let Some((room, at)) = self.scrolled.clone() else {
            return Task::none();
        };
        if room != self.selected {
            self.scrolled = None;
            return Task::none();
        }
        if self.original.is_some() || self.search.is_some() || self.timeline_restoring {
            return Task::none();
        }
        let Some(r) = self.rooms().list.get(&room) else {
            return Task::none();
        };
        let range = self.timeline_range(r);
        let count = r.posts().len();
        let beginning = r.end;
        // Moving the window changes what's shown, and the timeline reports
        // its new place; until it does, this place is out of date.
        if at.at_top && range.start > 0 {
            self.scrolled = Some((
                room,
                Scrolled {
                    at_top: false,
                    ..at
                },
            ));
            return Task::done(Msg::TimelineEdge(true));
        }
        if at.at_bottom && range.end < count {
            self.scrolled = Some((
                room,
                Scrolled {
                    at_bottom: false,
                    ..at
                },
            ));
            return Task::done(Msg::TimelineEdge(false));
        }
        if at.near_top
            && range.start == 0
            && self.logged_in
            && self.connected
            && !beginning
            && !self.paging(&room)
            && self.auto_page_failed.as_ref() != Some(&room)
        {
            self.scrolled_away = true;
            self.page();
        }
        Task::none()
    }
}
