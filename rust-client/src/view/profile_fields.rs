//! Banners, About Me and accent colours on profile cards, and their editors
//! in My Account (profile_fields.rs). Only Untethered shows them, and the
//! editors say so.
use super::{
    ACTIVE, App, BODY, BOLD, CHANNEL, El, MUTED, Msg, TEXT, animated, face, field_style, label,
    nav, surface,
};
use crate::{
    engine::profile_fields::{BIO_LENGTH, Fields},
    profile_fields::{ACCENTS, Input},
};
use iced::{
    Alignment::Center,
    Border, Color, ContentFit, Fill, Length,
    widget::{Space, button, column, container, image, row, text, text_editor},
};

/// The id of the About Me editor.
pub const BIO: &str = "profile-bio";
const UNTETHERED_ONLY: &str = "Only Untethered shows your banner, accent colour and About Me. \
     Other Matrix apps, such as Element, show your name and picture.";

pub(super) fn accent(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// A profile picture, playing if it's animated (P1).
pub(super) fn picture(app: &App, name: &str, mxc: Option<&str>, size: f32) -> El<'static> {
    match app.avatars.moving(mxc) {
        Some(frames) => animated::animated(frames.clone())
            .width(size)
            .height(size)
            .content_fit(ContentFit::Cover)
            .into(),
        None => face(name, app.avatars.get(mxc), size),
    }
}

/// The top of a profile: the banner, playing if it's animated, or a strip
/// of the accent colour. An empty slot without either.
pub(super) fn banner<'a>(app: &'a App, fields: Option<&Fields>, height: f32) -> El<'a> {
    let tint = fields.and_then(|f| f.accent).map(accent);
    let mxc = fields.and_then(|f| f.banner.as_deref());
    let loaded = mxc.and_then(|m| app.profile_fields.banner(m));
    if let Some(picture) = loaded {
        return match crate::profile_fields::moving(picture) {
            Some(frames) => animated::animated(frames.clone())
                .width(Fill)
                .height(height)
                .content_fit(ContentFit::Cover)
                .into(),
            None => image(picture.still().clone())
                .width(Fill)
                .height(height)
                .content_fit(ContentFit::Cover)
                .into(),
        };
    }
    match (mxc, tint) {
        // Still loading: the accent, or a plain strip, at the banner's size.
        (Some(_), tint) => strip(tint.unwrap_or(ACTIVE), height),
        (None, Some(tint)) => strip(tint, height * 0.5),
        (None, None) => Space::new().height(0).into(),
    }
}

fn strip<'a>(tint: Color, height: f32) -> El<'a> {
    container(Space::new())
        .width(Fill)
        .height(height)
        .style(move |_| container::Style {
            background: Some(tint.into()),
            ..Default::default()
        })
        .into()
}

/// Someone's About Me, for their card; an empty slot without one.
pub(super) fn about<'a>(fields: Option<&'a Fields>) -> El<'a> {
    match fields.and_then(|f| f.bio.as_deref()) {
        Some(bio) => column![label("ABOUT ME"), text(bio).size(14).color(BODY)]
            .spacing(6)
            .into(),
        None => Space::new().height(0).into(),
    }
}

/// My Account's editors for the banner, accent colour and About Me, where
/// the homeserver lets this account set them; a line saying it doesn't
/// where it doesn't, and nothing until it said.
pub(super) fn settings(app: &App) -> El<'_> {
    let state = &app.profile_fields;
    let Some(editable) = state.editable else {
        return Space::new().height(0).into();
    };
    if !editable.any() {
        return frame(
            text(
                "Your homeserver doesn't offer profile fields, so you can't add a banner, \
                 accent colour or About Me.",
            )
            .size(13)
            .color(MUTED)
            .into(),
        );
    }
    let own = state.own.as_ref();
    let ready = app.connected && !state.busy();
    let heading = row![
        text("Profile card").size(16).font(BOLD).color(TEXT),
        container(text("UNTETHERED ONLY").size(10).font(BOLD).color(CHANNEL))
            .padding([1, 4])
            .style(|_| surface(ACTIVE, 3.)),
    ]
    .spacing(8)
    .align_y(Center);
    let mut form = column![heading, text(UNTETHERED_ONLY).size(13).color(MUTED)].spacing(12);
    // Each editor keeps its slot, so the About Me editor keeps its state.
    form = form.push(if editable.banner {
        let has = own.is_some_and(|f| f.banner.is_some());
        column![
            label("BANNER"),
            container(banner(app, own, 96.))
                .width(Fill)
                .clip(true)
                .style(|_| surface(super::RAIL, 6.)),
            row![
                button(text("Change banner").size(13))
                    .style(nav(false))
                    .on_press_maybe(ready.then_some(Msg::ProfileFields(Input::PickBanner))),
                button(text("Remove banner").size(13))
                    .style(nav(false))
                    .on_press_maybe(
                        (ready && has).then_some(Msg::ProfileFields(Input::RemoveBanner))
                    ),
            ]
            .spacing(8)
            .wrap(),
        ]
        .spacing(8)
        .into()
    } else {
        El::from(Space::new().height(0))
    });
    form = form.push(if editable.accent {
        let chosen = own.and_then(|f| f.accent);
        let swatch = |rgb: Option<u32>| -> El<'static> {
            let selected = chosen == rgb;
            let fill = rgb.map_or(Color::TRANSPARENT, accent);
            let inner: El<'static> = match rgb {
                Some(_) => Space::new().width(Fill).height(Fill).into(),
                None => container(text("Default").size(12).color(BODY))
                    .center(Fill)
                    .into(),
            };
            button(container(inner).width(Fill).height(Fill))
                .width(if rgb.is_some() { 32 } else { 72 })
                .height(32)
                .padding(0)
                .style(move |_, status| button::Style {
                    background: Some(fill.into()),
                    border: Border {
                        color: if selected {
                            Color::WHITE
                        } else if status == button::Status::Hovered {
                            MUTED
                        } else {
                            ACTIVE
                        },
                        width: if selected { 2. } else { 1. },
                        radius: 6.0.into(),
                    },
                    ..Default::default()
                })
                .on_press_maybe(
                    (ready && !selected).then_some(Msg::ProfileFields(Input::Accent(rgb))),
                )
                .into()
        };
        let swatches = ACCENTS
            .iter()
            .map(|c| swatch(Some(*c)))
            .chain([swatch(None)]);
        column![label("ACCENT COLOUR"), row(swatches).spacing(8).wrap(),]
            .spacing(8)
            .into()
    } else {
        El::from(Space::new().height(0))
    });
    form = form.push(if editable.bio {
        let typed = state.bio_text();
        let left = BIO_LENGTH.saturating_sub(typed.chars().count());
        let changed = typed.trim() != state.saved_bio();
        let editor = text_editor(&state.bio)
            .id(BIO)
            .placeholder("Tell people a little about yourself")
            .padding(10)
            .size(14)
            .height(Length::Shrink)
            .min_height(64)
            .max_height(160)
            .style(field_style);
        // Without on_action the editor is read-only.
        let editor = if app.connected && state.saving.is_none() {
            editor.on_action(|a| Msg::ProfileFields(Input::Bio(a)))
        } else {
            editor
        };
        column![
            label("ABOUT ME"),
            editor,
            row![
                text(format!("{left} characters left"))
                    .size(12)
                    .color(CHANNEL)
                    .width(Fill),
                button(text("Save About Me").size(13)).on_press_maybe(
                    (ready && changed).then_some(Msg::ProfileFields(Input::SaveBio))
                ),
            ]
            .spacing(8)
            .align_y(Center),
        ]
        .spacing(8)
        .into()
    } else {
        El::from(Space::new().height(0))
    });
    form = form.push(if state.busy() {
        El::from(text("Updating your profile card…").size(13).color(MUTED))
    } else if let Some(note) = state.note {
        text(note).size(13).color(BODY).into()
    } else {
        Space::new().height(0).into()
    });
    frame(form.into())
}

fn frame(content: El<'_>) -> El<'_> {
    container(content)
        .padding(16)
        .width(Fill)
        .style(|_| container::Style {
            border: Border {
                color: ACTIVE,
                width: 1.,
                ..Default::default()
            },
            ..surface(super::TRAY, 8.)
        })
        .into()
}
