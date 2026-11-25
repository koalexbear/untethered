use super::{
    App, BLURPLE, BODY, CHANNEL, El, Icon, LINE, Msg, SIDE, TEXT, icons, input_style, with_tip,
};
use crate::{
    reactions::{Chip, QUICK, Toggle, label},
    rooms::Post,
};
use iced::{
    Alignment::Center,
    Border, Color, color,
    widget::{Column, Row, button, container, row, text, text_input, tooltip},
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

pub(super) fn view<'a>(app: &'a App, post: &Post) -> Option<El<'a>> {
    let id = post.event_id.as_ref()?;
    let picker = app.reaction_picker.as_ref().filter(|p| &p.target == id);
    if !post.reactable || post.chips.is_empty() && picker.is_none() {
        return None;
    }
    let action = |key: &str| {
        if app.connected && app.verified && !app.reaction_pending(id, key) {
            Toggle::for_post(&app.selected, post, key).map(Msg::ToggleReaction)
        } else {
            None
        }
    };
    let own = |key: &str| post.chips.get(key).and_then(Chip::mine).is_some();
    let mut bar = Column::new().spacing(6);
    if !post.chips.is_empty() {
        let mut chips = Row::new().spacing(4);
        for (key, chip) in &post.chips {
            let mine = own(key);
            let caption = row![
                text(key.to_string()).size(15),
                text(chip.count.to_string()).size(14).font(super::BOLD)
            ]
            .spacing(6)
            .align_y(Center);
            let hint = format!(
                "{} · {} · {}",
                label(key),
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
        chips = chips.push(with_tip(
            pill(
                icons::icon(Icon::Smile, 18., if open { TEXT } else { CHANNEL }),
                false,
                (app.verified && app.connected).then(|| Msg::ReactionPicker(id.clone())),
            ),
            "Add Reaction",
            tooltip::Position::Top,
        ));
        bar = bar.push(chips.wrap());
    }
    if let Some(p) = picker {
        let mut choices = Row::new().spacing(2);
        for (key, name) in QUICK {
            let mine = own(key);
            choices = choices.push(with_tip(
                button(text(key).size(22))
                    .padding([4, 6])
                    .on_press_maybe(action(key))
                    .style(move |_, s| button::Style {
                        background: (mine || s == button::Status::Hovered)
                            .then(|| super::ACTIVE.into()),
                        border: Border::default().rounded(6),
                        ..Default::default()
                    }),
                name,
                tooltip::Position::Top,
            ));
        }
        let key = p.custom.trim();
        let act = action(key);
        let custom = row![
            text_input("Other emoji or text", &p.custom)
                .width(200)
                .size(14)
                .padding(7)
                .style(input_style)
                .on_input(Msg::ReactionInput)
                .on_submit_maybe(act.clone()),
            button(
                text(if own(key) { "Remove" } else { "Add" })
                    .size(14)
                    .font(super::BOLD)
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
            .on_press_maybe(act)
        ]
        .spacing(6)
        .align_y(Center);
        bar = bar.push(
            container(
                Column::new()
                    .push(
                        text("QUICK REACTIONS")
                            .size(12)
                            .font(super::BOLD)
                            .color(CHANNEL),
                    )
                    .push(choices.wrap())
                    .push(custom)
                    .spacing(8),
            )
            .padding(10)
            .style(|_| container::Style {
                border: Border {
                    color: LINE,
                    width: 1.,
                    radius: 8.0.into(),
                },
                ..super::surface(color!(0x2b2d31), 8.)
            }),
        );
    }
    Some(bar.into())
}
