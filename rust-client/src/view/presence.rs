//! Presence dots on people's pictures, the user panel's status menu and the
//! custom status dialog (presence.rs).
use super::{
    App, BODY, BOLD, CHANNEL, CHAT, El, FLOAT, MUTED, Msg, SIDE, TEXT, input_style, plain, primary,
    surface,
};
use crate::presence::{Choice, FIELD, Input, MAX_MESSAGE, Status};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Point, Rectangle, Renderer, Theme, Vector, color, mouse,
    widget::{
        Column, Space, button,
        canvas::{self, Cache, Canvas, Geometry, LineCap, Path, Stroke},
        column, container, mouse_area, opaque, row, stack, text, text_input,
    },
};
use std::cell::Cell;

pub(super) const IDLE: Color = color!(0xf0b232);
const OFFLINE: Color = color!(0x80848e);

pub(super) fn color(status: Status) -> Color {
    match status {
        Status::Online => super::GREEN,
        Status::Idle => IDLE,
        Status::Busy => super::RED,
        Status::Offline => OFFLINE,
    }
}

/// A status dot drawn as Discord does: green, an amber moon, red with a
/// bar, a grey ring. `ring` is the surface behind the picture: it parts the
/// dot from the picture and fills the dot's cut-outs.
struct Dot {
    status: Status,
    /// The dot's diameter and the ring around it.
    dot: f32,
    ring: f32,
    background: Color,
}

#[derive(Default)]
struct DotState {
    cache: Cache,
    key: Cell<u64>,
}

impl canvas::Program<Msg> for Dot {
    type State = DotState;
    fn draw(
        &self,
        state: &DotState,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let [r, g, b, a] = self.background.into_rgba8();
        let key = (self.status as u64) << 56
            | u64::from(u32::from_be_bytes([r, g, b, a])) << 24
            | (self.dot as u64 & 0xff) << 16
            | (bounds.width as u64 & 0xff) << 8
            | (bounds.height as u64 & 0xff);
        if state.key.get() != key {
            state.cache.clear();
            state.key.set(key);
        }
        vec![state.cache.draw(renderer, bounds.size(), |f| {
            // The dot sits in the bottom-right corner; its ring reaches past
            // the top-left, over the picture.
            let radius = self.dot / 2.;
            let c = Point::new(bounds.width - radius, bounds.height - radius);
            f.fill(&Path::circle(c, radius + self.ring), self.background);
            f.fill(&Path::circle(c, radius), color(self.status));
            match self.status {
                Status::Online => {}
                Status::Idle => f.fill(
                    &Path::circle(c - Vector::new(radius * 0.45, radius * 0.45), radius * 0.62),
                    self.background,
                ),
                Status::Busy => f.stroke(
                    &Path::line(
                        c - Vector::new(radius * 0.45, 0.),
                        c + Vector::new(radius * 0.45, 0.),
                    ),
                    Stroke::default()
                        .with_color(self.background)
                        .with_width(radius * 0.5)
                        .with_line_cap(LineCap::Round),
                ),
                Status::Offline => f.fill(&Path::circle(c, radius * 0.45), self.background),
            }
        })]
    }
}

/// A picture of `size` pixels with someone's status dot in its corner over
/// `background`. Without a status (presence off, or not known yet) the
/// picture shows alone, in the same slot.
pub(super) fn with_dot<'a>(
    picture: El<'a>,
    size: f32,
    status: Option<Status>,
    background: Color,
) -> El<'a> {
    let dot = (size * 10. / 32.).round().max(6.);
    let ring = (size * 3. / 32.).round().max(2.);
    let corner: El = match status {
        Some(status) => Canvas::new(Dot {
            status,
            dot,
            ring,
            background,
        })
        .width(dot + ring)
        .height(dot + ring)
        .into(),
        None => Space::new().into(),
    };
    stack![
        container(picture).width(size).height(size),
        container(corner).align_right(Fill).align_bottom(Fill)
    ]
    .into()
}

/// The user panel's text under our name: the custom status, else the status.
pub(super) fn panel_line(app: &App) -> Option<String> {
    let status = app.own_status();
    let shown = app.presence.support == crate::presence::Support::On;
    if shown && status != Status::Offline && !app.presence.message.is_empty() {
        return Some(app.presence.message.clone());
    }
    let label = match status {
        Status::Offline => "Invisible",
        status => status.label(),
    };
    (shown || status == Status::Busy).then(|| label.to_owned())
}

fn item<'a>(status: Status, label: &'a str, note: Option<&'a str>, msg: Msg) -> El<'a> {
    let mut words = column![text(label).size(14)].spacing(2);
    if let Some(note) = note {
        words = words.push(text(note).size(12).color(CHANNEL));
    }
    let dot = container(
        container(Space::new().width(10).height(10)).style(move |_| surface(color(status), 5.)),
    )
    .padding([4, 0]);
    button(row![dot, words].spacing(10))
        .width(Fill)
        .padding([7, 8])
        .on_press(msg)
        .style(menu_style)
        .into()
}

fn menu_style(_: &Theme, s: button::Status) -> button::Style {
    let hovered = matches!(s, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then(|| color!(0x4752c4).into()),
        text_color: if hovered { Color::WHITE } else { BODY },
        border: Border::default().rounded(3),
        ..Default::default()
    }
}

fn line<'a>() -> El<'a> {
    container(
        container(Space::new().height(1).width(Fill)).style(|_| surface(color!(0x2e2f34), 0.)),
    )
    .padding([4, 8])
    .into()
}

/// The status menu over the user panel, as Discord's: the statuses, the
/// custom status, User Settings. Where the homeserver doesn't share
/// presence only Do Not Disturb (which pauses alerts) is left.
pub(super) fn menu(app: &App) -> El<'_> {
    let shown = app.presence.support == crate::presence::Support::On;
    let mut list = Column::new().spacing(2);
    if shown {
        list = list
            .push(item(
                Status::Online,
                "Online",
                None,
                Msg::Presence(Input::Choose(Choice::Online)),
            ))
            .push(item(
                Status::Idle,
                "Idle",
                None,
                Msg::Presence(Input::Choose(Choice::Idle)),
            ));
    }
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    if app.dnd_available() {
        list = list.push(item(
            Status::Busy,
            "Do Not Disturb",
            Some("You won't get desktop notifications"),
            Msg::Presence(Input::Dnd),
        ));
    }
    if shown {
        list = list
            .push(item(
                Status::Offline,
                "Invisible",
                Some("You'll appear offline"),
                Msg::Presence(Input::Choose(Choice::Invisible)),
            ))
            .push(line());
        let edit = if app.presence.message.is_empty() {
            "Set Custom Status"
        } else {
            "Edit Custom Status"
        };
        let mut custom = row![
            button(text(edit).size(14))
                .width(Fill)
                .padding([7, 8])
                .on_press(Msg::Presence(Input::Edit))
                .style(menu_style)
        ]
        .align_y(Center);
        if !app.presence.message.is_empty() {
            custom = custom.push(
                button(text("Clear").size(12))
                    .padding([7, 8])
                    .on_press(Msg::Presence(Input::Clear))
                    .style(menu_style),
            );
        }
        list = list.push(custom);
    } else {
        list = list.push(
            container(
                text("Your homeserver doesn't share who's online.")
                    .size(12)
                    .color(CHANNEL),
            )
            .padding([4, 8]),
        );
    }
    list = list.push(line()).push(
        button(text("User Settings").size(14))
            .width(Fill)
            .padding([7, 8])
            .on_press(Msg::Account)
            .style(menu_style),
    );
    container(list)
        .width(232)
        .padding(6)
        .style(|_| container::Style {
            border: Border {
                color: color!(0x2b2d31),
                width: 1.,
                radius: 6.0.into(),
            },
            ..surface(FLOAT, 6.)
        })
        .into()
}

/// Discord's "Set a custom status" dialog.
pub(super) fn dialog(app: &App) -> Option<El<'_>> {
    let draft = app.presence.dialog.as_ref()?;
    let close = Msg::Presence(Input::Close);
    let save = Msg::Presence(Input::Save);
    let left = MAX_MESSAGE - draft.chars().count();
    let body = column![
        text("Set a custom status").size(20).font(BOLD).color(TEXT),
        text("Everyone who shares a room with you sees it while you're online.")
            .size(13)
            .color(MUTED),
        column![
            text("WHAT'S COOKING?").size(12).font(BOLD).color(CHANNEL),
            text_input("Support has arrived!", draft)
                .id(FIELD)
                .size(15)
                .padding([10, 10])
                .style(input_style)
                .on_input(|t| Msg::Presence(Input::Draft(t)))
                .on_submit(save.clone()),
            text(format!("{left} characters left"))
                .size(12)
                .color(CHANNEL),
        ]
        .spacing(8),
    ]
    .spacing(12);
    let footer = container(
        row![
            Space::new().width(Fill),
            button(text("Cancel").size(14))
                .padding([10, 16])
                .style(plain(BODY, Color::WHITE))
                .on_press(close.clone()),
            button(text("Save").size(14).font(BOLD))
                .padding([10, 18])
                .style(primary)
                .on_press(save),
        ]
        .spacing(8)
        .align_y(Center),
    )
    .padding(16)
    .width(Fill)
    .style(|_| container::Style {
        background: Some(SIDE.into()),
        border: Border::default().rounded(iced::border::bottom(8)),
        ..Default::default()
    });
    let dialog = container(column![container(body).padding([20, 16]), footer])
        .width(440)
        .style(|_| surface(CHAT, 8.));
    let backdrop =
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Color::from_rgba8(0, 0, 0, 0.7).into()),
                    ..Default::default()
                }),
        )
        .on_press(close);
    Some(stack![opaque(backdrop), container(dialog).center(Fill)].into())
}
