use super::{ACTIVE, BOLD, El, HOVER, MUTED, Msg, RAIL, Room, TEXT, TRAY, avatar, nav, surface};
use crate::desktop_call::{ActiveCall, Feed, VideoInput};
use iced::{
    Alignment::Center,
    Border, Fill, Length, color,
    widget::{Column, Row, Space, button, column, container, image, row, scrollable, stack, text},
};

struct Card {
    name: String,
    live: bool,
    speaking: bool,
    feed: Option<Feed>,
    local: bool,
}
fn card(c: &Card, id: uuid::Uuid, small: bool, selectable: bool) -> El<'static> {
    let body: El = if let Some(f) = &c.feed {
        image(f.frame.clone())
            .content_fit(iced::ContentFit::Contain)
            .width(Fill)
            .height(Fill)
            .into()
    } else {
        container(avatar(&c.name, if small { 40. } else { 76. }))
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

fn cards(call: &ActiveCall, room: &Room, me: &str) -> (Option<String>, Vec<Card>) {
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
                local,
                live,
                speaking: !live && speakers.contains(u),
                feed: Some(f.clone()),
            });
        }
        if !camera {
            out.push(Card {
                name: name.into(),
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

pub(super) fn view(call: &ActiveCall, room: &Room, me: &str) -> El<'static> {
    let (focus, cards) = cards(call, room, me);
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
        stage = stage.push(
            container(card(active, call.id, false, false))
                .width(Fill)
                .height(Fill),
        );
        if cards.len() > 1 {
            let strip =
                Row::with_children(cards.iter().enumerate().filter(|(i, _)| *i != index).map(
                    |(_, c)| {
                        container(card(c, call.id, true, true))
                            .width(162)
                            .height(94)
                            .into()
                    },
                ))
                .spacing(6);
            let strip: El = if cards.len() > 4 {
                scrollable(strip).horizontal().into()
            } else {
                strip.into()
            };
            stage = stage.push(container(strip).width(Fill).height(102).align_x(Center));
        }
        stage.into()
    } else {
        let mut grid = Column::new().spacing(8);
        let cols = cards.len().min(if cards.len() > 6 { 3 } else { 2 });
        for chunk in cards.chunks(cols.max(1)) {
            let mut cells =
                Row::with_children(chunk.iter().map(|c| card(c, call.id, false, true))).spacing(8);
            for _ in chunk.len()..cols {
                cells = cells.push(Space::new().width(Fill));
            }
            grid = grid.push(cells.height(if cards.len() > 6 {
                Length::Fixed(165.)
            } else {
                Fill
            }));
        }
        if cards.len() > 6 {
            scrollable(grid).height(Fill).into()
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
pub(super) fn popover(call: &ActiveCall, room: &Room, can_expand: bool) -> El<'static> {
    let (focus, feeds) = call.previews.snapshot();
    let feed = feeds
        .into_iter()
        .filter(|f| !f.user.is_empty() || f.source == VideoInput::Window)
        .min_by(|a, b| {
            let priority = |f: &Feed| {
                (
                    Some(&f.sid) != focus.as_ref(),
                    f.source != VideoInput::Window,
                )
            };
            priority(a)
                .cmp(&priority(b))
                .then_with(|| a.user.cmp(&b.user))
                .then_with(|| a.sid.cmp(&b.sid))
        });
    let has_feed = feed.is_some();
    let hint = text("Open call view").size(12).font(BOLD);
    let preview: El<'static> = if let Some(feed) = feed.filter(|_| !call.preview_hidden) {
        column![
            container(
                image(feed.frame)
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
            avatar(&room.name, 30.),
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
