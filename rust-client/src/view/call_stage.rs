use super::{ACTIVE, BOLD, El, HOVER, MUTED, Msg, RAIL, Room, TEXT, TRAY, face, nav, surface};
use crate::avatars::Avatars;
use crate::desktop_call::{ActiveCall, Feed, Layer, StageScroll, VideoInput};
use iced::widget::image::Handle;
use iced::{
    Alignment::Center,
    Border, Fill, Length, color,
    widget::{Column, Row, Space, button, column, container, image, row, scrollable, stack, text},
};

/// Thumbnails under the focused tile, and the grid's rows once it scrolls.
const THUMB_W: f32 = 162.;
const THUMB_GAP: f32 = 6.;
const ROW_H: f32 = 165.;
const ROW_GAP: f32 = 8.;

/// Where a scrolling row or column is: its offset, the length it shows,
/// and its content's length, along the direction it scrolls.
fn span(viewport: &scrollable::Viewport, horizontal: bool) -> (f32, f32, f32) {
    let (offset, bounds, content) = (
        viewport.absolute_offset(),
        viewport.bounds(),
        viewport.content_bounds(),
    );
    if horizontal {
        (offset.x, bounds.width, content.width)
    } else {
        (offset.y, bounds.height, content.height)
    }
}

/// Whether item `i` of `count` in a scrolling row or column (items `size`
/// long, `gap` apart) shows in its `span`. Until the first viewport, or
/// with one left from another layout, every item counts as shown.
fn in_view(span: Option<(f32, f32, f32)>, (i, count): (usize, usize), size: f32, gap: f32) -> bool {
    let Some((offset, extent, content)) = span else {
        return true;
    };
    if (content - (count as f32 * (size + gap) - gap)).abs() > 1.5 {
        return true;
    }
    let start = i as f32 * (size + gap);
    start < offset + extent && start + size > offset
}

/// The layer a grid tile needs: one fills the stage, up to six share it
/// two to a row, more sit in short scrolling rows.
fn grid_layer(cards: usize) -> Layer {
    match cards {
        0 | 1 => Layer::High,
        2..=6 => Layer::Medium,
        _ => Layer::Low,
    }
}

struct Card {
    name: String,
    picture: Option<Handle>,
    live: bool,
    speaking: bool,
    feed: Option<Feed>,
    local: bool,
}
fn card(c: &Card, id: uuid::Uuid, small: bool, selectable: bool) -> El<'static> {
    let body: El = if let Some(frame) = c.feed.as_ref().and_then(Feed::picture) {
        image(frame.clone())
            .content_fit(iced::ContentFit::Contain)
            .width(Fill)
            .height(Fill)
            .into()
    } else {
        container(face(
            &c.name,
            c.picture.as_ref(),
            if small { 40. } else { 76. },
        ))
        .center(Fill)
        .into()
    };
    let mut badge = row![
        text(c.name.clone())
            .size(if small { 12 } else { 14 })
            .font(BOLD)
            .color(TEXT)
    ]
    .align_y(Center)
    .spacing(6);
    if c.live {
        badge = badge.push(
            container(text("LIVE").size(11).font(BOLD).color(iced::Color::WHITE))
                .padding([2, 5])
                .style(|_| surface(color!(0xb62941), 4.)),
        );
    }
    let caption = container(
        container(badge)
            .padding([3, 7])
            .style(|_| surface(color!(0x202125), 5.)),
    )
    .padding(7)
    .width(Fill)
    .height(Fill)
    .align_y(iced::alignment::Vertical::Bottom);
    let speaking = c.speaking;
    let visual = container(stack![body, caption])
        .padding(2)
        .width(Fill)
        .height(Fill)
        .style(move |_| {
            let mut s = surface(
                if speaking {
                    color!(0x202e28)
                } else {
                    color!(0x25262b)
                },
                10.,
            );
            s.border = Border::default()
                .rounded(10)
                .color(if speaking {
                    color!(0x42bd79)
                } else {
                    color!(0x303136)
                })
                .width(if speaking { 2. } else { 1. });
            s
        });
    if selectable && let Some(f) = &c.feed {
        button(visual)
            .padding(1)
            .style(nav(false))
            .width(Fill)
            .height(Fill)
            .on_press(Msg::FocusVideo(id, Some(f.sid.clone())))
            .into()
    } else {
        visual.into()
    }
}

/// Whose pictures the cards show: the loaded ones and the account's own.
pub(super) struct Pictures<'a> {
    pub avatars: &'a Avatars,
    pub own: Option<&'a str>,
}

fn cards(
    call: &ActiveCall,
    room: &Room,
    me: &str,
    pictures: &Pictures,
) -> (Option<String>, Vec<Card>) {
    let (focus, feeds) = call.previews.snapshot();
    let speakers = call.speaking.borrow();
    let mut out = Vec::new();
    let mut users = room.voice_users();
    users.insert(me);
    if !call.group
        && let Some(peer) = room.dm_peer.as_deref()
    {
        users.insert(peer);
    }
    for f in &feeds {
        if !f.user.is_empty() {
            users.insert(&f.user);
        }
    }
    for u in users {
        let local = u == me;
        let name = if local { "You" } else { room.display(u) };
        let theirs = pictures
            .avatars
            .get(room.avatars.get(u).map(String::as_str));
        let picture = if local {
            pictures.avatars.get(pictures.own).or(theirs)
        } else {
            theirs
        }
        .cloned();
        let mut camera = false;
        for f in feeds.iter().filter(|f| {
            if local {
                f.user.is_empty()
            } else {
                f.user == u
            }
        }) {
            let live = f.source == VideoInput::Window;
            camera |= !live;
            out.push(Card {
                name: if live {
                    if local {
                        "Your screen".into()
                    } else {
                        format!("{name}’s screen")
                    }
                } else {
                    name.into()
                },
                picture: picture.clone(),
                local,
                live,
                speaking: !live && speakers.contains(u),
                feed: Some(f.clone()),
            });
        }
        if !camera {
            out.push(Card {
                name: name.into(),
                picture,
                local,
                live: false,
                speaking: speakers.contains(u),
                feed: None,
            });
        }
    }
    out.sort_by_key(|c| (!c.live, c.feed.is_none(), c.local, c.name.clone()));
    (focus, out)
}

pub(super) fn view(call: &ActiveCall, room: &Room, me: &str, pictures: &Pictures) -> El<'static> {
    let (focus, cards) = cards(call, room, me, pictures);
    let pinned = cards.iter().position(|c| {
        c.feed
            .as_ref()
            .is_some_and(|f| Some(&f.sid) == focus.as_ref())
    });
    let selected = pinned.or_else(|| {
        if call.group {
            None
        } else {
            cards
                .iter()
                .position(|c| c.live)
                .or_else(|| cards.iter().position(|c| !c.local))
        }
    });
    let content: El = if let Some(index) = selected {
        let active = &cards[index];
        let mut stage = column![].spacing(8);
        if active.live || pinned.is_some() && call.group {
            let mut label = row![
                text(if active.live {
                    "SCREEN SHARE"
                } else {
                    "FOCUS VIEW"
                })
                .font(BOLD)
                .size(11)
                .color(MUTED),
                text(active.name.clone()).size(14).color(TEXT),
                Space::new().width(Fill)
            ]
            .align_y(Center)
            .spacing(8);
            if call.group {
                label = label.push(
                    button("Back to grid")
                        .style(nav(false))
                        .on_press(Msg::FocusVideo(call.id, None)),
                );
            }
            stage = stage.push(label);
        } else if !active.local
            && active.feed.is_none()
            && call.video_peer.is_some()
            && call.label == "Connected"
        {
            stage = stage.push(text("No incoming video").size(13).color(MUTED));
        }
        if let Some(f) = &active.feed {
            call.previews.draws(&f.sid, Layer::High);
        }
        stage = stage.push(
            container(card(active, call.id, false, false))
                .width(Fill)
                .height(Fill),
        );
        if cards.len() > 1 {
            let thumbs: Vec<&Card> = cards
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, c)| c)
                .collect();
            let scrolls = thumbs.len() > 3;
            let shown = call.stage_scroll[StageScroll::Thumbnails as usize].map(|v| span(&v, true));
            for (i, c) in thumbs.iter().enumerate() {
                if let Some(f) = &c.feed
                    && (!scrolls || in_view(shown, (i, thumbs.len()), THUMB_W, THUMB_GAP))
                {
                    call.previews.draws(&f.sid, Layer::Low);
                }
            }
            let strip = Row::with_children(thumbs.into_iter().map(|c| {
                container(card(c, call.id, true, true))
                    .width(THUMB_W)
                    .height(94)
                    .into()
            }))
            .spacing(THUMB_GAP);
            let strip: El = if scrolls {
                let id = call.id;
                scrollable(strip)
                    .horizontal()
                    .on_scroll(move |v| Msg::StageScroll(id, StageScroll::Thumbnails, v))
                    .into()
            } else {
                strip.into()
            };
            stage = stage.push(container(strip).width(Fill).height(102).align_x(Center));
        }
        stage.into()
    } else {
        let mut grid = Column::new().spacing(ROW_GAP);
        let scrolls = cards.len() > 6;
        let cols = cards.len().min(if scrolls { 3 } else { 2 });
        let rows = cards.len().div_ceil(cols.max(1));
        let shown = call.stage_scroll[StageScroll::Grid as usize].map(|v| span(&v, false));
        for (r, chunk) in cards.chunks(cols.max(1)).enumerate() {
            if !scrolls || in_view(shown, (r, rows), ROW_H, ROW_GAP) {
                for f in chunk.iter().filter_map(|c| c.feed.as_ref()) {
                    call.previews.draws(&f.sid, grid_layer(cards.len()));
                }
            }
            let mut cells =
                Row::with_children(chunk.iter().map(|c| card(c, call.id, false, true))).spacing(8);
            for _ in chunk.len()..cols {
                cells = cells.push(Space::new().width(Fill));
            }
            grid = grid.push(cells.height(if scrolls { Length::Fixed(ROW_H) } else { Fill }));
        }
        if scrolls {
            let id = call.id;
            scrollable(grid)
                .height(Fill)
                .on_scroll(move |v| Msg::StageScroll(id, StageScroll::Grid, v))
                .into()
        } else {
            grid.height(Fill).into()
        }
    };
    container(content)
        .width(Fill)
        .height(Fill)
        .padding([6, 12])
        .into()
}

/// Keep the call preview above the controls until the user opens the stage.
pub(super) fn popover(
    call: &ActiveCall,
    room: &Room,
    picture: Option<&Handle>,
    can_expand: bool,
) -> El<'static> {
    let feed = call.previews.docked();
    let has_feed = feed.is_some();
    if let Some(f) = feed.as_ref().filter(|_| !call.preview_hidden) {
        call.previews.draws(&f.sid, Layer::Low);
    }
    let hint = text("Open call view").size(12).font(BOLD);
    let frame = feed
        .as_ref()
        .filter(|_| !call.preview_hidden)
        .and_then(|f| f.picture().cloned());
    let preview: El<'static> = if let Some(frame) = frame {
        column![
            container(
                image(frame)
                    .width(Fill)
                    .height(Fill)
                    .content_fit(iced::ContentFit::Contain)
            )
            .width(Fill)
            .height(56)
            .style(|_| surface(RAIL, 5.)),
            hint
        ]
        .spacing(4)
        .into()
    } else {
        row![
            face(&room.name, picture, 30.),
            column![
                text(room.name.clone()).size(13).color(TEXT).height(17),
                hint.color(MUTED),
            ]
            .spacing(2)
        ]
        .spacing(8)
        .align_y(Center)
        .into()
    };
    let preview = button(preview)
        .padding(7)
        .width(Fill)
        .on_press_maybe(can_expand.then_some(Msg::ReturnToCall(call.id)))
        .style(|_, status| button::Style {
            background: Some(
                if status == button::Status::Hovered {
                    HOVER
                } else {
                    TRAY
                }
                .into(),
            ),
            border: Border {
                color: ACTIVE,
                width: 1.,
                ..Border::default().rounded(10)
            },
            text_color: TEXT,
            ..Default::default()
        });
    let mut card = column![preview].spacing(2);
    if has_feed {
        card = card.push(
            button(
                text(if call.preview_hidden {
                    "Show video preview"
                } else {
                    "Hide video preview"
                })
                .size(11),
            )
            .padding([2, 6])
            .style(nav(false))
            .on_press(Msg::HideCallPreview(call.id, !call.preview_hidden)),
        );
    }
    card.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolled_away_tiles_are_not_in_view() {
        // Seven thumbnails (1176 px) in a 500-px strip scrolled by 340 px:
        // the third (336-498) through the fifth (672-834) show.
        let strip = |offset| Some((offset, 500., 7. * 168. - 6.));
        let shown = |offset| {
            (0..7)
                .filter(|&i| in_view(strip(offset), (i, 7), THUMB_W, THUMB_GAP))
                .collect::<Vec<_>>()
        };
        assert_eq!(shown(0.), [0, 1, 2]);
        assert_eq!(shown(340.), [2, 3, 4]);
        assert_eq!(shown(676.), [4, 5, 6]);
        // No viewport yet, or one from another count of tiles: all show.
        assert!(in_view(None, (6, 7), THUMB_W, THUMB_GAP));
        assert!(in_view(strip(0.), (6, 8), THUMB_W, THUMB_GAP));
        // A grid of four 165-px rows in 400 px, scrolled to the bottom.
        let grid = Some((292., 400., 4. * 173. - 8.));
        let rows: Vec<_> = (0..4)
            .filter(|&r| in_view(grid, (r, 4), ROW_H, ROW_GAP))
            .collect();
        assert_eq!(rows, [1, 2, 3]);
    }

    #[test]
    fn grid_tiles_need_less_picture_as_they_shrink() {
        assert_eq!(grid_layer(1), Layer::High);
        assert_eq!(grid_layer(2), Layer::Medium);
        assert_eq!(grid_layer(6), Layer::Medium);
        assert_eq!(grid_layer(7), Layer::Low);
    }
}
