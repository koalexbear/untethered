//! Create a space or channel, or invite someone, in a Discord-style modal.
use super::{
    ACTIVE, App, BODY, BOLD, CHANNEL, CHAT, El, HOVER, Icon, MUTED, Msg, SIDE, TEXT, icon,
    icon_button, input_style, plain, primary,
};
use crate::community::{Form, Input, Kind};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Padding, color,
    widget::{button, column, container, row, text, text_input, tooltip},
};

fn field_label(s: &str) -> El<'_> {
    text(s).size(12).font(BOLD).color(CHANNEL).into()
}

/// One of the radio cards for choosing a channel type.
fn kind_card<'a>(
    glyph: Icon,
    title: &'a str,
    detail: &'a str,
    chosen: bool,
    msg: Option<Msg>,
) -> El<'a> {
    let dot = container(
        container(iced::widget::Space::new())
            .width(10)
            .height(10)
            .style(move |_| {
                super::surface(
                    if chosen {
                        Color::WHITE
                    } else {
                        Color::TRANSPARENT
                    },
                    5.,
                )
            }),
    )
    .center(20)
    .style(move |_| container::Style {
        border: Border {
            color: if chosen { Color::WHITE } else { super::ICON },
            width: 2.,
            radius: 10.0.into(),
        },
        ..Default::default()
    });
    button(
        row![
            icon(glyph, 24., if chosen { TEXT } else { super::ICON }),
            column![
                text(title).size(16).font(BOLD).color(TEXT),
                text(detail).size(14).color(MUTED)
            ]
            .spacing(2)
            .width(Fill),
            dot
        ]
        .spacing(12)
        .align_y(Center),
    )
    .width(Fill)
    .padding([10, 12])
    .on_press_maybe(msg)
    .style(move |_, s| button::Style {
        background: Some(
            if chosen {
                ACTIVE
            } else if s == button::Status::Hovered {
                HOVER
            } else {
                SIDE
            }
            .into(),
        ),
        border: Border::default().rounded(4),
        ..Default::default()
    })
    .into()
}

pub(super) fn view<'a>(app: &'a App, f: &'a Form) -> El<'a> {
    let busy = app.setup_busy();
    let unlocked = !busy && !f.done && f.created.is_none();
    let space = f.parent.as_ref().map(|parent| {
        app.live
            .list
            .get(parent)
            .map_or(parent.as_str(), |r| r.name.as_str())
    });
    let (title, subtitle) = match (f.kind, space) {
        (Kind::Invite, Some(space)) => ("Invite friends".to_owned(), format!("to {space}")),
        (_, Some(space)) => (
            format!(
                "Create {} Channel",
                if f.kind == Kind::Voice {
                    "Voice"
                } else {
                    "Text"
                }
            ),
            format!("in {space}"),
        ),
        _ => (
            "Create a space".to_owned(),
            "An invite-only Matrix space for your community. Add channels after you create it."
                .to_owned(),
        ),
    };
    let close = (!busy).then_some(Msg::Setup(Input::Close));
    let mut body = column![
        row![
            column![
                text(title).size(20).font(BOLD).color(TEXT),
                text(subtitle).size(12).color(MUTED)
            ]
            .spacing(4)
            .width(Fill),
            icon_button(
                Icon::Close,
                "Close",
                false,
                close.clone(),
                tooltip::Position::Left
            )
        ]
        .align_y(iced::alignment::Vertical::Top)
    ]
    .spacing(16);
    if f.parent.is_some() && f.kind == Kind::Invite {
        body = body.push(
            text("Once they join this space, they can join its members-only channels.")
                .size(14)
                .color(BODY),
        );
    } else if f.parent.is_some() {
        body = body.push(
            column![
                field_label("CHANNEL TYPE"),
                kind_card(
                    Icon::Hash,
                    "Text",
                    "Send messages, images, files and replies",
                    f.kind == Kind::Text,
                    unlocked.then_some(Msg::Setup(Input::Kind(Kind::Text)))
                ),
                kind_card(
                    Icon::Speaker,
                    "Voice",
                    "Hang out together with voice, video and screen share",
                    f.kind == Kind::Voice,
                    unlocked.then_some(Msg::Setup(Input::Kind(Kind::Voice)))
                ),
            ]
            .spacing(8),
        );
    }
    if f.kind != Kind::Invite {
        let input = text_input(
            if f.parent.is_some() {
                "new-channel"
            } else {
                "My community"
            },
            &f.name,
        )
        .id("setup-name")
        .on_input_maybe(unlocked.then_some(|s| Msg::Setup(Input::Name(s))))
        .size(16)
        .padding(10)
        .style(input_style);
        let named: El = if f.parent.is_some() {
            container(
                row![
                    icon(
                        if f.kind == Kind::Voice {
                            Icon::Speaker
                        } else {
                            Icon::Hash
                        },
                        18.,
                        super::ICON
                    ),
                    input
                ]
                .spacing(2)
                .align_y(Center),
            )
            .padding(Padding {
                left: 10.,
                ..Padding::ZERO
            })
            .style(|_| super::surface(super::RAIL, 4.))
            .into()
        } else {
            input.into()
        };
        body = body.push(
            column![
                field_label(if f.parent.is_some() {
                    "CHANNEL NAME"
                } else {
                    "SPACE NAME"
                }),
                named
            ]
            .spacing(8),
        );
        if f.parent.is_some() {
            body = body.push(
                row![
                    icon(Icon::Lock, 16., CHANNEL),
                    text("Encrypted. Space members can join; others need a separate invitation.")
                        .size(13)
                        .color(CHANNEL)
                ]
                .spacing(6)
                .align_y(Center),
            );
        }
    }
    if matches!(f.kind, Kind::Space | Kind::Invite) {
        body = body.push(
            column![
                field_label(if f.kind == Kind::Space {
                    "ADD A MEMBER (OPTIONAL)"
                } else {
                    "MATRIX USER ID"
                }),
                text_input(
                    if f.kind == Kind::Space {
                        "Invite a Matrix user (optional)"
                    } else {
                        "@alex:example.org"
                    },
                    &f.invite,
                )
                .id("setup-invite")
                .on_input_maybe(unlocked.then_some(|s| Msg::Setup(Input::Invite(s))))
                .size(16)
                .padding(10)
                .style(input_style),
                text("Use the full ID, e.g. @alex:example.org.")
                    .size(12)
                    .color(CHANNEL),
            ]
            .spacing(8),
        );
    }
    if !f.note.is_empty() {
        body = body.push(text(&f.note).size(14).color(color!(0xffc785)));
    }
    if let Some(id) = &f.created {
        body = body.push(text(id).size(12).color(CHANNEL));
    }
    let submit = button(
        text(if f.kind == Kind::Invite {
            "Send invite"
        } else if f.created.is_some() {
            "Retry link"
        } else if f.parent.is_some() {
            "Create Channel"
        } else {
            "Create"
        })
        .size(14)
        .font(BOLD),
    )
    .padding([10, 18])
    .style(primary)
    .on_press_maybe(
        (app.connected && !busy && !f.done && f.valid(&app.matrix_id))
            .then_some(Msg::Setup(Input::Submit)),
    );
    let cancel = button(
        text(if f.created.is_some() || f.done {
            "Close"
        } else {
            "Cancel"
        })
        .size(14),
    )
    .padding([10, 16])
    .style(plain(BODY, Color::WHITE))
    .on_press_maybe(close);
    let footer = container(
        row![iced::widget::Space::new().width(Fill), cancel, submit]
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
    container(column![container(body).padding([20, 16]), footer])
        .width(460)
        .style(|_| super::surface(CHAT, 8.))
        .into()
}
