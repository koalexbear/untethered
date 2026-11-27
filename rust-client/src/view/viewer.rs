//! The media viewer overlay (`crate::viewer`): the picture or video fitted
//! to the window over a dark backdrop, the sender and date at the top left,
//! Copy (pictures), Download and Close at the top right, and arrows to the
//! room's previous and next images and videos, with a strip of them along the
//! bottom. A loaded picture zooms with the mouse wheel and a click (zoom.rs),
//! and loads sharper once zoomed into.
use super::{App, BOLD, CHANNEL, El, Icon, Msg, TEXT, face, icon, surface};
use crate::{
    media::{Kind, Media},
    rooms::{Post, Room},
    viewer::Viewer,
};
use chrono::{DateTime, Local};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Size, Theme,
    widget::{
        Space, button, column, container, image, image::Handle, mouse_area, opaque, responsive,
        row, stack, text,
    },
};

const SHADE: Color = Color::from_rgba(0., 0., 0., 0.92);
const KNOB: Color = Color::from_rgba(0.12, 0.12, 0.14, 0.8);
const KNOB_HOVER: Color = Color::from_rgba(0.25, 0.25, 0.28, 0.9);

/// A round dark button with a white icon.
fn round<'a>(glyph: Icon, size: f32, msg: Option<Msg>) -> El<'a> {
    button(container(icon(glyph, size * 0.5, Color::WHITE)).center(size))
        .padding(0)
        .on_press_maybe(msg)
        .style(move |_: &Theme, s| button::Style {
            background: Some(
                if s == button::Status::Hovered {
                    KNOB_HOVER
                } else {
                    KNOB
                }
                .into(),
            ),
            border: Border::default().rounded(size / 2.),
            ..Default::default()
        })
        .into()
}

fn dims(h: &Handle) -> Option<(f32, f32)> {
    match h {
        Handle::Rgba { width, height, .. } => Some((*width as f32, *height as f32)),
        _ => None,
    }
}

/// `natural` scaled down (never up) to fit `room`.
fn fit(natural: (f32, f32), room: Size) -> (f32, f32) {
    let scale = (room.width / natural.0)
        .min(room.height / natural.1)
        .min(1.);
    (
        (natural.0 * scale).max(1.).floor(),
        (natural.1 * scale).max(1.).floor(),
    )
}

/// A picture at `size`, which a click doesn't pass to the backdrop.
fn picture<'a>(h: &Handle, size: (f32, f32)) -> El<'a> {
    opaque(
        image(h.clone())
            .width(size.0)
            .height(size.1)
            .content_fit(iced::ContentFit::Fill),
    )
}

/// Tells one picture's zoom from the next one's.
fn key(event: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    event.hash(&mut hasher);
    hasher.finish()
}

fn note<'a>(line: &'a str, media: &Media) -> El<'a> {
    opaque(
        column![
            text(line).size(15).color(TEXT),
            button(text("Download").size(14).font(BOLD))
                .padding([6, 14])
                .style(super::primary)
                .on_press(Msg::SaveFile(media.clone())),
        ]
        .spacing(12)
        .align_x(Center),
    )
}

fn image_stage<'a>(app: &'a App, v: &'a Viewer, media: &'a Media, room: Size) -> El<'a> {
    // While the original loads, show the timeline's preview at its size.
    let shown = v
        .sharp
        .as_ref()
        .or(v.image.as_ref())
        .or_else(|| media.preview.as_ref().and_then(|p| app.inline.ready(p)));
    match shown {
        Some(h) => {
            let natural = if v.image.is_some() {
                dims(h)
            } else {
                media
                    .dims
                    .map(|(w, h)| (w as f32, h as f32))
                    .or_else(|| dims(h))
            };
            if v.image.is_some() {
                let (w, h2) = natural.unwrap_or((room.width, room.height));
                return super::zoom::zoom(h.clone(), Size::new(w, h2), key(&v.event))
                    .on_zoom(Msg::ViewerZoomed)
                    .into();
            }
            let size = fit(natural.unwrap_or((room.width, room.height)), room);
            picture(h, size)
        }
        None if v.failed => note("This image couldn't be loaded.", media),
        None => text("Loading…").size(15).color(CHANNEL).into(),
    }
}

#[cfg(target_os = "linux")]
fn video_stage<'a>(app: &'a App, v: &'a Viewer, media: &'a Media, room: Size) -> El<'a> {
    use crate::player::{Control, clock};
    const BAR: f32 = 44.;
    // The poster, until the first frame.
    let waiting = || -> El<'a> {
        let poster = media.preview.as_ref().and_then(|p| app.inline.ready(p));
        let loading = text("Loading video…").size(15).color(TEXT);
        match poster {
            Some(h) => {
                let natural = media
                    .dims
                    .map(|(w, h)| (w as f32, h as f32))
                    .or_else(|| dims(h));
                let size = fit(natural.unwrap_or((room.width, room.height)), room);
                stack![picture(h, size), container(loading).center(Fill)].into()
            }
            None => loading.into(),
        }
    };
    let Some(player) = &v.video else {
        if v.failed {
            return note("This video can't be played here.", media);
        }
        return waiting();
    };
    let Some(frame) = &player.frame else {
        return waiting();
    };
    let room = Size::new(room.width, (room.height - BAR).max(1.));
    let size = fit(dims(frame).unwrap_or((room.width, room.height)), room);
    let total = player.duration.unwrap_or_default();
    let progress = if total.is_zero() {
        0.
    } else {
        (player.position.as_secs_f32() / total.as_secs_f32()).clamp(0., 1.)
    };
    let at = player.scrub.unwrap_or(progress);
    let shown_time = if player.scrub.is_some() {
        total.mul_f32(at)
    } else {
        player.position
    };
    let controls = container(
        row![
            round(
                if player.playing {
                    Icon::Pause
                } else {
                    Icon::Play
                },
                32.,
                Some(Msg::PlayerControl(Control::Toggle)),
            ),
            text(format!("{} / {}", clock(shown_time), clock(total)))
                .size(13)
                .color(TEXT),
            iced::widget::slider(0.0_f32..=1.0, at, Msg::PlayerScrub)
                .step(0.001_f32)
                .on_release(Msg::PlayerControl(Control::Seek(at)))
                .width(Fill),
            round(
                if player.muted {
                    Icon::Muted
                } else {
                    Icon::Speaker
                },
                32.,
                Some(Msg::PlayerControl(Control::Mute)),
            ),
        ]
        .spacing(10)
        .align_y(Center),
    )
    .padding([6, 8])
    .width(size.0.max(320.))
    .height(BAR);
    opaque(
        column![
            mouse_area(
                image(frame.clone())
                    .width(size.0)
                    .height(size.1)
                    .content_fit(iced::ContentFit::Fill)
            )
            .on_press(Msg::PlayerControl(Control::Toggle)),
            controls
        ]
        .align_x(Center),
    )
}

#[cfg(not(target_os = "linux"))]
fn video_stage<'a>(app: &'a App, _: &'a Viewer, media: &'a Media, room: Size) -> El<'a> {
    match media.preview.as_ref().and_then(|p| app.inline.ready(p)) {
        Some(h) => {
            let natural = media
                .dims
                .map(|(w, h)| (w as f32, h as f32))
                .or_else(|| dims(h));
            let size = fit(natural.unwrap_or((room.width, room.height)), room);
            column![
                picture(h, size),
                note("Download the video to watch it.", media)
            ]
            .spacing(16)
            .align_x(Center)
            .into()
        }
        None => note("Download the video to watch it.", media),
    }
}

/// One of the strip's thumbnails; the shown one framed.
fn thumb<'a>(app: &'a App, p: &'a Post, m: &'a Media, current: bool) -> Option<El<'a>> {
    const SIZE: f32 = 52.;
    let event = p.event_id.clone()?;
    let picture: El = match m.preview.as_ref().and_then(|s| app.inline.ready(s)) {
        Some(h) => image(h.clone())
            .width(SIZE)
            .height(SIZE)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(4)
            .into(),
        None => container(Space::new())
            .width(SIZE)
            .height(SIZE)
            .style(|_| surface(KNOB, 4.))
            .into(),
    };
    let picture: El = if m.kind == Kind::Video {
        stack![
            picture,
            container(icon(Icon::Play, 16., Color::WHITE)).center(SIZE)
        ]
        .into()
    } else {
        picture
    };
    Some(
        button(picture)
            .padding(2)
            .on_press(Msg::OpenViewer(event))
            .style(move |_: &Theme, s| button::Style {
                border: Border {
                    color: if current {
                        Color::WHITE
                    } else if s == button::Status::Hovered {
                        Color::from_rgba(1., 1., 1., 0.4)
                    } else {
                        Color::TRANSPARENT
                    },
                    width: 2.,
                    radius: 6.0.into(),
                },
                ..Default::default()
            })
            .into(),
    )
}

/// The room's images and videos around the shown one, to jump to.
fn strip<'a>(app: &'a App, v: &'a Viewer, r: &'a Room) -> Option<El<'a>> {
    let items = v.strip(r);
    if items.len() < 2 {
        return None;
    }
    let mut thumbs = row![].spacing(4).align_y(Center);
    for (p, m) in items {
        let current = p.event_id.as_deref() == Some(v.event.as_str());
        if let Some(t) = thumb(app, p, m, current) {
            thumbs = thumbs.push(t);
        }
    }
    Some(
        container(opaque(thumbs))
            .center_x(Fill)
            .padding([6, 16])
            .into(),
    )
}

fn header<'a>(app: &'a App, r: &'a Room, post: &'a crate::rooms::Post, media: &'a Media) -> El<'a> {
    let name = r.display(&post.sender);
    let date = DateTime::from_timestamp_millis(post.time)
        .unwrap_or_default()
        .with_timezone(&Local)
        .format("%b %-d, %Y %H:%M")
        .to_string();
    let who = row![
        face(name, app.user_picture(r, &post.sender), 36.),
        column![
            text(name).size(15).font(BOLD).color(TEXT),
            text(date).size(12).color(CHANNEL)
        ]
        .spacing(2)
    ]
    .spacing(10)
    .align_y(Center);
    let mut buttons = row![].spacing(8);
    #[cfg(not(target_arch = "wasm32"))]
    if media.kind == Kind::Image {
        buttons = buttons.push(round(
            Icon::Copy,
            36.,
            (app.connected && !app.copying_image()).then(|| Msg::CopyImage(media.clone())),
        ));
    }
    buttons = buttons
        .push(round(
            Icon::Download,
            36.,
            Some(Msg::SaveFile(media.clone())),
        ))
        .push(round(Icon::Close, 36., Some(Msg::CloseViewer)));
    row![opaque(who), Space::new().width(Fill), opaque(buttons)]
        .align_y(Center)
        .padding([12, 16])
        .into()
}

pub(super) fn view<'a>(app: &'a App, v: &'a Viewer) -> El<'a> {
    let backdrop =
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(SHADE.into()),
                    ..Default::default()
                }),
        )
        .on_press(Msg::CloseViewer);
    let Some((r, (post, media, index, total))) = app
        .live
        .list
        .get(&v.room)
        .and_then(|r| Some((r, v.current(r)?)))
    else {
        return opaque(backdrop);
    };
    let arrow = |glyph, show: bool, step: i32| -> El<'a> {
        let slot = container(if show {
            opaque(round(glyph, 44., Some(Msg::ViewerStep(step))))
        } else {
            Space::new().into()
        });
        slot.width(60).center_y(Fill).into()
    };
    let stage = responsive(move |room| {
        let content = match media.kind {
            Kind::Video => video_stage(app, v, media, room),
            _ => image_stage(app, v, media, room),
        };
        container(content).center(Fill).into()
    });
    let middle = row![
        arrow(Icon::ChevronLeft, index > 1 || !r.end, -1),
        container(stage).width(Fill).height(Fill),
        arrow(Icon::ChevronRight, index < total, 1),
    ]
    .height(Fill)
    .padding([0, 12]);
    let copied = app.copy_note.map(|n| format!(" · {n}")).unwrap_or_default();
    let caption = container(
        text(format!("{} · {index} of {total}{copied}", media.name))
            .size(13)
            .color(CHANNEL),
    )
    .center_x(Fill)
    .padding([10, 16]);
    let mut layers = column![header(app, r, post, media), middle];
    if let Some(strip) = strip(app, v, r) {
        layers = layers.push(strip);
    }
    stack![
        opaque(backdrop),
        layers.push(caption).width(Fill).height(Fill)
    ]
    .into()
}
