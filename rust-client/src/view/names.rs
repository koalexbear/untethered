//! Role colours and badges next to names, and Edit Server Profile: a
//! nickname and a picture per server (names.rs).
use super::{
    App, BODY, BOLD, CHANNEL, CHAT, El, MUTED, Msg, SIDE, TEXT, input_style, plain, primary,
    surface,
};
use crate::names::{FIELD, Input, LENGTH, Role};
use iced::{
    Alignment::Center,
    Border, Color, Fill, color,
    widget::{Space, button, column, container, mouse_area, opaque, row, stack, text, text_input},
};

/// Light enough to read on every surface, and apart from the red of danger,
/// the green of online and the blue of links.
fn tint(role: Role) -> Color {
    match role {
        Role::Owner => color!(0xf1c40f),
        Role::Admin => color!(0xf47067),
        Role::Moderator => color!(0x3ccfae),
        // A custom role's own colour (roles.rs).
        Role::Custom { color, .. } => {
            Color::from_rgb8((color >> 16) as u8, (color >> 8) as u8, color as u8)
        }
    }
}

/// A name's colour: its role's, or `plain` for someone without one.
pub(super) fn color(role: Option<Role>, plain: Color) -> Color {
    role.map_or(plain, tint)
}

/// A small badge, like Discord's BOT tag.
pub(super) fn tag<'a>(role: Role) -> El<'a> {
    let tint = tint(role);
    container(text(role.tag()).size(10).font(BOLD).color(tint))
        .padding([1, 4])
        .style(move |_| container::Style {
            background: Some(Color { a: 0.16, ..tint }.into()),
            border: Border::default().rounded(3),
            ..Default::default()
        })
        .into()
}

/// A name in its role's colour, with the badge after it.
pub(super) fn author<'a>(name: &'a str, role: Option<Role>, size: f32) -> El<'a> {
    let label = text(name)
        .size(size)
        .font(BOLD)
        .color(color(role, TEXT))
        .wrapping(text::Wrapping::None);
    match role {
        Some(role) => row![label, tag(role)].spacing(6).align_y(Center).into(),
        None => label.into(),
    }
}

pub(super) fn dialog(app: &App) -> Option<El<'_>> {
    let form = app.names.form.as_ref()?;
    let server = app.live.list.get(&form.space)?;
    let saving = app.nickname_saving();
    let ready = app.connected && !saving;
    let close = Msg::Names(Input::Close);
    let save = Msg::Names(Input::Save);
    let current = app.nickname(&form.space);
    let changed = form.input.trim() != current.unwrap_or_default() || form.picture.is_some();
    let global = if app.profile.is_empty() {
        app.matrix_id.as_str()
    } else {
        app.profile.as_str()
    };
    let left = LENGTH - form.input.chars().count();
    let mut field = column![
        text("SERVER NICKNAME").size(12).font(BOLD).color(CHANNEL),
        text_input(global, &form.input)
            .id(FIELD)
            .size(15)
            .padding([10, 10])
            .style(input_style)
            .on_input_maybe(ready.then_some(|t| Msg::Names(Input::Edit(t))))
            .on_submit_maybe((ready && changed).then_some(save.clone())),
    ]
    .spacing(8);
    let mut under = row![
        text(format!("{left} characters left"))
            .size(12)
            .color(CHANNEL)
            .width(Fill)
    ]
    .align_y(Center);
    // As on Discord, only a nickname that's set can be reset.
    if current.is_some() {
        under = under.push(
            button(text("Reset Nickname").size(12))
                .padding(0)
                .style(plain(super::LINK, color!(0x5fc5ff)))
                .on_press_maybe(ready.then_some(Msg::Names(Input::Reset))),
        );
    }
    field = field.push(under);
    if saving {
        field = field.push(
            text("Changing your profile in each channel…")
                .size(13)
                .color(MUTED),
        );
    } else if let Some(problem) = form.problem {
        field = field.push(text(problem).size(13).color(super::RED));
    }
    // The server picture (ROADMAP M15): one picked to save, the one set, or
    // the global one.
    let set = app.server_picture(&form.space);
    let shown: El = match form.picture.as_ref().and_then(|p| p.preview.as_ref()) {
        Some(preview) => super::face(global, Some(preview), 64.),
        None => super::profile_fields::picture(app, global, set.or(app.own_avatar.as_deref()), 64.),
    };
    let picking = form.picking.is_some();
    let mut buttons = row![
        button(text("Change Picture").size(13))
            .padding([7, 12])
            .style(primary)
            .on_press_maybe((ready && !picking).then_some(Msg::Names(Input::PickPicture))),
    ]
    .spacing(12)
    .align_y(Center);
    // As with the nickname, only a picture that's set can be reset.
    if set.is_some() {
        buttons = buttons.push(
            button(text("Reset Picture").size(12))
                .padding(0)
                .style(plain(super::LINK, color!(0x5fc5ff)))
                .on_press_maybe(ready.then_some(Msg::Names(Input::ResetPicture))),
        );
    }
    let picture = column![
        text("SERVER PICTURE").size(12).font(BOLD).color(CHANNEL),
        row![
            shown,
            column![
                buttons,
                text(if form.picture.is_some() {
                    "Save to show this picture here."
                } else {
                    "PNG, JPEG, GIF or WebP, up to 8 MiB."
                })
                .size(12)
                .color(CHANNEL),
            ]
            .spacing(8),
        ]
        .spacing(16)
        .align_y(Center),
    ]
    .spacing(8);
    let body = column![
        text("Edit Server Profile").size(20).font(BOLD).color(TEXT),
        text(format!(
            "Your nickname and picture show instead of your profile's in {} and its \
             channels only, also in other Matrix apps such as Element.",
            server.name
        ))
        .size(13)
        .color(MUTED),
        picture,
        field,
    ]
    .spacing(16);
    let footer = container(
        row![
            Space::new().width(Fill),
            button(text("Cancel").size(14))
                .padding([10, 16])
                .style(plain(BODY, Color::WHITE))
                .on_press_maybe((!saving).then_some(close.clone())),
            button(text("Save Changes").size(14).font(BOLD))
                .padding([10, 18])
                .style(primary)
                .on_press_maybe((ready && changed).then_some(save)),
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
