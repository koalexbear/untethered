use super::{App, El, Msg, RAIL, nav, surface};
use crate::{
    reactions::{Chip, QUICK, Toggle, label},
    rooms::Post,
};
use iced::widget::{Column, Row, button, row, text, text_input, tooltip};

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
    let control = |key: &str, caption: String| {
        button(text(caption).size(13))
            .padding([2, 7])
            .style(nav(own(key)))
            .on_press_maybe(action(key))
    };
    let mut bar = Column::new().spacing(3);
    if !post.chips.is_empty() {
        let mut chips = Row::new().spacing(4);
        for (key, chip) in &post.chips {
            chips = chips.push(
                tooltip(
                    control(key, format!("{}  {}", label(key), chip.count)),
                    if own(key) {
                        "Remove your reaction"
                    } else {
                        "Add your reaction"
                    },
                    tooltip::Position::Top,
                )
                .style(|_| surface(RAIL, 5.))
                .gap(3),
            );
        }
        bar = bar.push(chips.wrap());
    }
    if let Some(p) = picker {
        let mut choices = Row::new().spacing(4);
        for (key, _) in QUICK {
            choices = choices.push(control(key, label(key)));
        }
        let key = p.custom.trim();
        let act = action(key);
        bar = bar.push(choices.wrap()).push(
            row![
                text_input("Other emoji or text", &p.custom)
                    .width(180)
                    .size(13)
                    .on_input(Msg::ReactionInput)
                    .on_submit_maybe(act.clone()),
                button(text(if own(key) { "Remove" } else { "Add" }).size(13)).on_press_maybe(act)
            ]
            .spacing(4),
        );
    }
    Some(bar.into())
}
