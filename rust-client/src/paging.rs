//! Loading older (and newer) messages as the timeline scrolls, as Discord
//! does: within a screen of the top, older messages come from the server,
//! and at the very top the shown window moves back through what's loaded.
//! The timeline reports where it is whenever that changes, and the check
//! runs after every update, so loading carries on while you stay at the top
//! even when nothing moves: a page that lands outside the shown window
//! changes no scroll position to report, and scrolling up at the top
//! reports nothing new either.
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

impl App {
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
