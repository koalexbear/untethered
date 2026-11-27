use super::{App, BLURPLE, BODY, CHANNEL, El, Icon, Msg, SIDE, TEXT, icons, input_style, with_tip};
use crate::{
    reactions::{Chip, Toggle, label},
    rooms::Post,
};
use iced::{
    Alignment::Center,
    Border, Color, color,
    widget::{Row, button, container, row, text, text_input, tooltip},
};

/// Reaction pills, like Discord's: emoji and count, outlined in blurple when
/// one of them is yours.
fn pill<'a>(caption: El<'a>, own: bool, action: Option<Msg>) -> button::Button<'a, Msg> {
    button(caption)
        .padding([2, 7])
        .on_press_maybe(action)
        .style(move |_, s| {
            let hovered = s == button::Status::Hovered;
            button::Style {
                background: Some(if own { color!(0x5865f2, 0.15) } else { SIDE }.into()),
                text_color: if own { color!(0xdee0fc) } else { BODY },
                border: Border {
                    color: if own {
                        BLURPLE
                    } else if hovered {
                        color!(0x4e5058)
                    } else {
                        Color::TRANSPARENT
                    },
                    width: 1.,
                    radius: 8.0.into(),
                },
                ..Default::default()
            }
        })
}

/// The message's reactions with the smile to add one; the picker floats
/// under that smile when it was opened there.
pub(super) fn view<'a>(app: &'a App, post: &'a Post) -> Option<El<'a>> {
    let id = post.event_id.as_ref()?;
    // An older room's reactions (upgrades.rs) show, but don't change.
    if !(post.reactable || post.from.is_some()) || post.chips.is_empty() {
        return None;
    }
    let picker = app.reaction_picker.as_ref().filter(|p| &p.target == id);
    let action = |key: &str| action(app, post, key);
    let own = |key: &str| own(post, key);
    let mut chips = Row::new().spacing(4);
    for (key, chip) in &post.chips {
        let mine = own(key);
        // A custom emoji's key is its picture's mxc URI (emotes.rs).
        let custom = key.starts_with("mxc://");
        let face: El = if custom {
            app.emote_image(key, 18.)
        } else {
            text(key.to_string()).size(15).into()
        };
        let caption = row![
            face,
            text(chip.count.to_string()).size(14).font(super::BOLD)
        ]
        .spacing(6)
        .align_y(Center);
        let name = if custom {
            app.emote_name(key)
                .map_or_else(|| "Custom emoji".to_owned(), |n| format!(":{n}:"))
        } else {
            label(key)
        };
        let hint = format!(
            "{} · {} · {}",
            name,
            chip.count,
            if mine {
                "Remove your reaction"
            } else {
                "Add your reaction"
            }
        );
        chips = chips.push(with_tip(
            pill(caption.into(), mine, action(key)),
            hint,
            tooltip::Position::Top,
        ));
    }
    let open = picker.is_some();
    let smile = pill(
        icons::icon(Icon::Smile, 18., if open { TEXT } else { CHANNEL }),
        false,
        (app.verified && app.connected && post.reactable)
            .then(|| Msg::ReactionPickerChips(id.clone())),
    );
    // Its tooltip would cover the open picker.
    let smile: El = if open {
        smile.into()
    } else {
        with_tip(smile, "Add Reaction", tooltip::Position::Top)
    };
    let popup = picker
        .filter(|p| p.beside_chips)
        .and_then(|_| panel(app, post));
    chips = chips.push(super::popover::popover(
        smile,
        popup,
        super::popover::Place::Below,
        Msg::ReactionPicker(id.clone()),
    ));
    Some(chips.wrap().into())
}

fn action(app: &App, post: &Post, key: &str) -> Option<Msg> {
    let id = post.event_id.as_ref()?;
    if app.connected && app.verified && !app.reaction_pending(id, key) {
        Toggle::for_post(&app.selected, post, key).map(Msg::ToggleReaction)
    } else {
        None
    }
}

fn own(post: &Post, key: &str) -> bool {
    post.chips.get(key).and_then(Chip::mine).is_some()
}

/// The open reaction picker for `post`: the quick reactions, a search field
/// and what it finds, or a reaction typed as text.
pub(super) fn panel<'a>(app: &'a App, post: &'a Post) -> Option<El<'a>> {
    let id = post.event_id.as_ref()?;
    let p = app.reaction_picker.as_ref().filter(|p| &p.target == id)?;
    let key = p.custom.trim();
    let act = action(app, post, key);
    let input = text_input("Search emoji, or react with text", &p.custom)
        .id(crate::reactions::SEARCH)
        .width(220)
        .size(14)
        .padding(7)
        .style(input_style)
        .on_input(Msg::ReactionInput)
        .on_submit_maybe(act.clone());
    let add = button(
        text(if own(post, key) { "Remove" } else { "Add" })
            .size(14)
            .font(super::BOLD),
    )
    .padding([6, 12])
    .style(|_, s| button::Style {
        background: Some(
            match s {
                button::Status::Hovered => color!(0x4752c4),
                button::Status::Disabled => color!(0x3c428a),
                _ => BLURPLE,
            }
            .into(),
        ),
        text_color: Color::WHITE,
        border: Border::default().rounded(4),
        ..Default::default()
    })
    .on_press_maybe(act);
    let panel = super::emoji_picker::panel(
        "QUICK REACTIONS",
        &p.custom,
        input.into(),
        Some(add.into()),
        app.emote_sections(
            &app.selected,
            &p.custom,
            |i| {
                (app.connected && app.verified && !app.reaction_pending(id, &i.url)).then(|| {
                    Msg::Emote(crate::emotes::Input::React {
                        event: id.clone(),
                        url: i.url.clone(),
                        shortcode: i.shortcode.clone(),
                    })
                })
            },
            |i| own(post, &i.url),
        ),
        |key| action(app, post, key),
        |key| own(post, key),
    );
    Some(container(panel).width(340).into())
}
