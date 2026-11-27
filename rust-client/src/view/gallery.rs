//! Pictures sent together, as a gallery (ROADMAP P3). Matrix sends each
//! attachment as its own message; a run of pictures from one sender, a minute
//! apart at most, with no captions, replies, edits or reactions, lays out as a
//! grid under one header, as Discord shows a message's attachments. Each
//! picture is still its own message: its own row ID (scroll positions, jumps
//! and read markers find it), right-click menu and viewer entry.
use super::{App, BOLD, CHANNEL, El, FLOAT, Msg, TEXT, profile, surface};
use crate::{
    media::{Kind, Media},
    rooms::{Post, Room},
};
use iced::{
    Alignment::Center,
    Fill, Padding,
    widget::{Column, Space, container, image, mouse_area, row, text},
};

/// Pictures in one gallery at most; more start another.
const MOST: usize = 9;
/// The gallery's width, and the gap between pictures.
const WIDTH: f32 = 408.;
const GAP: f32 = 4.;

/// How many columns `n` pictures lay out in.
pub fn columns(n: usize) -> usize {
    if n == 2 || n == 4 { 2 } else { 3 }
}

/// A picture that can sit in a gallery.
fn alone(p: &Post) -> Option<&Media> {
    let m = p.media.as_ref()?;
    (p.event_id.is_some()
        && p.reply.is_none()
        && p.revision.is_none()
        && p.chips.is_empty()
        && m.kind == Kind::Image
        && m.preview.is_some()
        && p.text().trim() == m.name)
        .then_some(m)
}

impl App {
    /// The pictures from the first of `posts` on that make a gallery: two or
    /// more, else none. `stop` says where one can't go on (a divider, a seam,
    /// the message jumped to).
    pub(super) fn gallery_run<'p>(
        &self,
        posts: &[(usize, &'p Post)],
        stop: impl Fn(usize, &Post) -> bool,
    ) -> Vec<&'p Post> {
        let mut run: Vec<&'p Post> = Vec::new();
        for &(index, p) in posts {
            if alone(p).is_none()
                || self
                    .editing
                    .as_ref()
                    .is_some_and(|d| Some(&d.event) == p.event_id.as_ref())
            {
                break;
            }
            if let Some(last) = run.last()
                && (p.sender != last.sender
                    || p.day != last.day
                    || p.time.saturating_sub(last.time) > 60_000
                    || stop(index, p))
            {
                break;
            }
            run.push(p);
            if run.len() == MOST {
                break;
            }
        }
        if run.len() < 2 {
            run.clear();
        }
        run
    }

    /// A gallery's row: the sender's header (when it starts a group) over
    /// the grid.
    pub(super) fn gallery_row<'a>(
        &'a self,
        r: &'a Room,
        run: &[&'a Post],
        group: bool,
        stamp: String,
    ) -> El<'a> {
        let first = run[0];
        let name = r.display(&first.sender);
        let grid = self.gallery_grid(run);
        let place = first.event_id.clone().unwrap_or_default();
        let block: El = if group {
            let head = row![
                self.user_area(
                    &first.sender,
                    format!("name {place}"),
                    profile::link(
                        self,
                        &first.sender,
                        text(name)
                            .size(16)
                            .font(BOLD)
                            .color(TEXT)
                            .wrapping(text::Wrapping::None),
                    )
                ),
                text(stamp).size(12).color(CHANNEL)
            ]
            .spacing(8)
            .align_y(Center);
            row![
                container(self.user_area(
                    &first.sender,
                    format!("picture {place}"),
                    profile::link(
                        self,
                        &first.sender,
                        self.user_face(r, &first.sender, name, 40.)
                    )
                ))
                .padding(Padding {
                    top: 2.,
                    ..Padding::ZERO
                }),
                Column::new().push(head).push(grid).spacing(2).width(Fill)
            ]
            .spacing(16)
            .into()
        } else {
            row![Space::new().width(40), grid].spacing(16).into()
        };
        let gutter: El = if group {
            Space::new().width(56).into()
        } else {
            container(text(&first.clock).size(11).color(CHANNEL))
                .width(56)
                .align_x(iced::alignment::Horizontal::Right)
                .padding(Padding {
                    top: 6.,
                    right: 4.,
                    ..Padding::ZERO
                })
                .into()
        };
        container(row![
            gutter,
            Space::new().width(2),
            container(block).width(Fill).padding(Padding {
                top: 2.,
                bottom: 2.,
                left: 14.,
                right: 48.,
            })
        ])
        .width(Fill)
        .padding(Padding {
            top: if group { 14. } else { 0. },
            ..Padding::ZERO
        })
        .into()
    }

    fn gallery_grid<'a>(&'a self, run: &[&'a Post]) -> El<'a> {
        let cols = columns(run.len());
        let size = ((WIDTH - GAP * (cols - 1) as f32) / cols as f32).floor();
        let mut grid = Column::new().spacing(GAP);
        for line in run.chunks(cols) {
            let mut cells = row![].spacing(GAP);
            for p in line {
                cells = cells.push(self.gallery_cell(p, size));
            }
            grid = grid.push(cells);
        }
        grid.into()
    }

    /// One picture, cropped square, opening in the viewer; its own message.
    fn gallery_cell<'a>(&'a self, p: &'a Post, size: f32) -> El<'a> {
        let (Some(m), Some(id)) = (alone(p), p.event_id.as_ref()) else {
            return Space::new().into();
        };
        let source = m.preview.as_ref();
        let moving = source.and_then(|s| self.inline.moving(s));
        let picture: El = match (moving, source.and_then(|s| self.inline.ready(s))) {
            (Some(frames), _) => super::animated::animated(frames.clone())
                .width(size)
                .height(size)
                .content_fit(iced::ContentFit::Cover)
                .border_radius(6.)
                .into(),
            (None, Some(h)) => image(h)
                .width(size)
                .height(size)
                .content_fit(iced::ContentFit::Cover)
                .border_radius(6)
                .into(),
            (None, None) => container(Space::new())
                .width(size)
                .height(size)
                .style(|_| surface(FLOAT, 6.))
                .into(),
        };
        let open = mouse_area(picture)
            .on_press(Msg::OpenViewer(id.clone()))
            .interaction(iced::mouse::Interaction::Pointer);
        container(self.message_area(p, open))
            .id(format!("post-{id}"))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::columns;

    #[test]
    fn galleries_lay_out_in_twos_or_threes() {
        assert_eq!(columns(2), 2);
        assert_eq!(columns(3), 3);
        assert_eq!(columns(4), 2);
        assert_eq!(columns(5), 3);
        assert_eq!(columns(9), 3);
    }
}

#[cfg(test)]
mod runs {
    use crate::viewer::tests::{picture, room_with};

    #[test]
    fn pictures_a_minute_apart_from_one_sender_make_a_gallery() {
        let mut pictures: Vec<_> = (1..=3).map(|n| picture(n, "@mira:h", n * 10_000)).collect();
        // Two minutes later: a new run, of one, so no gallery.
        pictures.push(picture(4, "@mira:h", 160_000));
        // Someone else's.
        pictures.push(picture(5, "@bob:h", 161_000));
        let (app, _) = room_with(pictures);
        let r = &app.live.list["!r:h"];
        let posts: Vec<_> = r.posts().enumerate().collect();
        let ids = |run: Vec<&crate::rooms::Post>| -> Vec<String> {
            run.iter().map(|p| p.event_id.clone().unwrap()).collect()
        };
        assert_eq!(
            ids(app.gallery_run(&posts, |_, _| false)),
            ["$1", "$2", "$3"]
        );
        assert!(app.gallery_run(&posts[3..], |_, _| false).is_empty());
        // A divider ends a gallery early.
        assert_eq!(
            ids(app.gallery_run(&posts, |_, q| q.event_id.as_deref() == Some("$3"))),
            ["$1", "$2"]
        );
    }
}
