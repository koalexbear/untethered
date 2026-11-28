//! Member cards and account profile editing.
use super::{
    ACTIVE, App, BODY, BOLD, El, FLOAT, MUTED, Msg, TRAY, input_style, nav, profile_fields, surface,
};
use crate::{
    context_menu::Event as Menu,
    engine::profile::{Action, msg},
};
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
        // Your name in this room: a server nickname, or the global name.
        r.me.as_ref()
            .and_then(|me| me.name.as_deref())
            .unwrap_or(&app.profile)
    } else {
        member
            .map(|m| m.name.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| r.display(user))
    };
    // Their picture in this room: a server picture where they set one.
    let picture = if user == &app.matrix_id {
        app.own_picture(r)
    } else {
        member
            .and_then(|m| m.avatar.as_deref())
            .or_else(|| r.avatars.get(user).map(String::as_str))
    };
    // Their banner, About Me and accent colour (profile_fields.rs).
    let fields = app.profile_fields.of(user, &app.matrix_id);
    let tint = fields.and_then(|f| f.accent).map(profile_fields::accent);
    let role = r.role(user);
    let power = match (r.posting.actor(user), role) {
        (None, _) => "Room power level unavailable".into(),
        (Some(level), None) => format!("Room power level {level}"),
        (Some(level), Some(role)) => format!("{} · room power level {level}", role.label()),
    };
    let close = msg(Action::Close);
    // Their status dot, and status with any custom status (presence.rs).
    let presence = app.presence_of(user);
    let mut who = column![
        super::names::author(name, role, 23.),
        text(user).size(13).color(MUTED)
    ]
    .spacing(3)
    .width(Fill);
    if let Some(p) = &presence {
        let line = match p.message.as_ref().filter(|_| p.status.here()) {
            Some(message) => format!("{} · {message}", p.status.label()),
            None => p.status.label().to_owned(),
        };
        who = who.push(text(line).size(13).color(BODY));
    }
    let mut details = column![
        row![
            super::presence::with_dot(
                profile_fields::picture(app, name, picture, 64.),
                64.,
                presence.as_ref().map(|p| p.status),
                FLOAT
            ),
            who,
            button(text("Close").size(12))
                .style(nav(false))
                .on_press(close.clone()),
        ]
        .spacing(12)
        .align_y(Center),
        profile_fields::about(fields),
        text(power).size(13).color(BODY),
    ]
    .spacing(14);
    if user != &app.matrix_id {
        let message = button(
            text(if ui.opening_dm() {
                "Opening conversation…"
            } else {
                "Message"
            })
            .size(14),
        )
        .on_press_maybe((app.connected && !ui.opening_dm()).then_some(msg(Action::Message)));
        // Blocking asks first (context_menu.rs); unblocking doesn't.
        let blocked = app.is_blocked(user);
        let block = button(text(if blocked { "Unblock" } else { "Block" }).size(14))
            .style(nav(false))
            .on_press_maybe((app.connected && !app.blocking(user)).then(|| {
                Msg::ContextMenu(if blocked {
                    Menu::Unblock(user.clone())
                } else {
                    Menu::Block(user.clone())
                })
            }));
        details = details.push(row![message, block].spacing(8));
    } else if app.may_edit_server_profile()
        && let Some(space) = app.server_of(room)
    {
        // Your server nickname (names.rs).
        details = details.push(
            button(text("Edit Server Profile").size(14)).on_press_maybe(
                app.connected
                    .then(|| Msg::Names(crate::names::Input::Open(space))),
            ),
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
        container(column![
            profile_fields::banner(app, fields, 120.),
            container(details).padding(20)
        ])
        .width(Fill)
        .max_width(392)
        .clip(true)
        .style(move |_| container::Style {
            border: iced::Border {
                color: tint.unwrap_or(Color::TRANSPARENT),
                width: if tint.is_some() { 2. } else { 0. },
                radius: 12.0.into(),
            },
            ..surface(FLOAT, 12.)
        }),
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
    let form = container(form)
        .padding(16)
        .width(Fill)
        .style(|_| container::Style {
            border: iced::Border {
                color: ACTIVE,
                width: 1.,
                ..Default::default()
            },
            ..surface(TRAY, 8.)
        });
    // The banner, accent colour and About Me in their own slot after the
    // form, which grows and shrinks (profile_fields.rs).
    column![form, profile_fields::settings(app)]
        .spacing(16)
        .into()
}
