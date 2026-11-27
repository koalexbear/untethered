//! Member cards and account profile editing.
use super::{
    ACTIVE, App, BODY, BOLD, El, FLOAT, MUTED, TEXT, TRAY, face, input_style, nav, surface,
};
use crate::engine::profile::{Action, msg};
use iced::{
    Alignment::Center,
    Color, Fill,
    widget::{Space, button, column, container, mouse_area, opaque, row, stack, text, text_input},
};

pub(super) fn link<'a>(app: &App, user: &str, content: impl Into<El<'a>>) -> El<'a> {
    button(content)
        .padding(0)
        .style(button::text)
        .on_press_maybe(app.verified.then(|| {
            msg(Action::Open {
                room: app.selected.clone(),
                user: user.to_owned(),
            })
        }))
        .into()
}

pub(super) fn card(app: &App) -> Option<El<'_>> {
    let ui = &app.sdk_ui.profile;
    let (room, user) = ui.card.as_ref()?;
    let r = app.live.list.get(room)?;
    let member = (room == &app.selected)
        .then(|| app.directory.users.get(user))
        .flatten();
    let name = if user == &app.matrix_id {
        &app.profile
    } else {
        member
            .map(|m| m.name.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| r.display(user))
    };
    let picture = app.avatars.get(if user == &app.matrix_id {
        app.own_avatar.as_deref()
    } else {
        member
            .and_then(|m| m.avatar.as_deref())
            .or_else(|| r.avatars.get(user).map(String::as_str))
    });
    let power = r.posting.actor(user).map_or_else(
        || "Room power level unavailable".into(),
        |level| format!("Room power level {level}"),
    );
    let close = msg(Action::Close);
    let mut details = column![
        row![
            face(name, picture, 64.),
            column![
                text(name).size(23).font(BOLD).color(TEXT),
                text(user).size(13).color(MUTED)
            ]
            .spacing(3)
            .width(Fill),
            button(text("Close").size(12))
                .style(nav(false))
                .on_press(close.clone()),
        ]
        .spacing(12)
        .align_y(Center),
        text(power).size(13).color(BODY),
    ]
    .spacing(14);
    if user != &app.matrix_id {
        details = details.push(
            button(
                text(if ui.opening_dm() {
                    "Opening conversation…"
                } else {
                    "Message"
                })
                .size(14),
            )
            .on_press_maybe((app.connected && !ui.opening_dm()).then_some(msg(Action::Message))),
        );
    }
    if let Some(note) = ui.card_error {
        details = details.push(text(note).size(13).color(BODY));
    }
    let backdrop =
        opaque(
            mouse_area(container(Space::new()).width(Fill).height(Fill).style(|_| {
                container::Style {
                    background: Some(Color::from_rgba8(0, 0, 0, 0.55).into()),
                    ..Default::default()
                }
            }))
            .on_press(close),
        );
    let panel = opaque(
        container(details)
            .padding(20)
            .width(Fill)
            .max_width(392)
            .style(|_| surface(FLOAT, 12.)),
    );
    Some(stack![backdrop, container(panel).padding(16).center(Fill)].into())
}

pub(super) fn settings(app: &App) -> El<'_> {
    let ui = &app.sdk_ui.profile;
    let ready = app.verified && app.connected && !ui.busy();
    let name = ui.name.as_deref().unwrap_or(&app.profile);
    let save = ready && ui.name.as_ref().is_some_and(|n| n.trim() != app.profile);
    let mut form = column![
        text("Display name").size(12).font(BOLD).color(BODY),
        row![
            text_input("Display name", name)
                .style(input_style)
                .padding([7, 9])
                .on_input_maybe(ready.then_some(|name| msg(Action::Name(name))))
                .on_submit_maybe(save.then_some(msg(Action::SaveName))),
            button(text("Save name").size(13))
                .on_press_maybe(save.then_some(msg(Action::SaveName))),
        ]
        .spacing(8)
        .align_y(Center),
        row![
            button(text("Change picture").size(13))
                .style(nav(false))
                .on_press_maybe(ready.then_some(msg(Action::PickAvatar))),
            button(text("Remove picture").size(13))
                .style(nav(false))
                .on_press_maybe(
                    (ready && app.own_avatar.is_some()).then_some(msg(Action::RemoveAvatar))
                ),
        ]
        .spacing(8)
        .wrap(),
    ]
    .spacing(8);
    if ui.busy() {
        form = form.push(text("Updating profile…").size(13).color(MUTED));
    } else if let Some(note) = ui.note {
        form = form.push(text(note).size(13).color(BODY));
    }
    container(form)
        .padding(16)
        .width(Fill)
        .style(|_| container::Style {
            border: iced::Border {
                color: ACTIVE,
                width: 1.,
                ..Default::default()
            },
            ..surface(TRAY, 8.)
        })
        .into()
}
