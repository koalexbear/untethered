//! The emoji picker: the quick reactions, a search field and the emoji
//! matching it by shortcode. The reaction picker and the composer's emoji
//! button both show it; the caller decides what a click does.
use super::{BOLD, CHANNEL, El, LINE, Msg, with_tip};
use crate::{emoji, reactions::QUICK};
use iced::{
    Alignment::Center,
    Border, color,
    widget::{Column, Row, button, container, text, tooltip},
};

/// Emoji the search shows at most.
const GRID: usize = 40;

/// `input` is the search field, built by the caller with its own messages;
/// `extra` sits beside it. With `browse`, an empty search shows common
/// emoji instead of nothing. `pick` is what clicking an emoji does and
/// `active` highlights one, as our own reactions are.
pub(super) fn panel<'a>(
    title: &'static str,
    search: &str,
    input: El<'a>,
    extra: Option<El<'a>>,
    browse: bool,
    pick: impl Fn(&str) -> Option<Msg>,
    active: impl Fn(&str) -> bool,
) -> El<'a> {
    let cell = |emoji: &'static str, hint: String| {
        let on = active(emoji);
        with_tip(
            button(text(emoji).size(22))
                .padding([4, 6])
                .on_press_maybe(pick(emoji))
                .style(move |_, s| button::Style {
                    background: (on || s == button::Status::Hovered).then(|| super::ACTIVE.into()),
                    border: Border::default().rounded(6),
                    ..Default::default()
                }),
            hint,
            tooltip::Position::Top,
        )
    };
    let quick = Row::with_children(QUICK.iter().map(|(key, name)| cell(key, (*name).into())))
        .spacing(2)
        .wrap();
    let query = search.trim();
    let found = if !query.is_empty() {
        emoji::search(query, GRID)
    } else if browse {
        emoji::common(GRID)
    } else {
        Vec::new()
    };
    let has_extra = extra.is_some();
    let mut body = Column::new()
        .push(text(title).size(12).font(BOLD).color(CHANNEL))
        .push(quick)
        .push(
            Row::new()
                .push(input)
                .extend(extra)
                .spacing(6)
                .align_y(Center),
        )
        .spacing(8);
    if !found.is_empty() {
        body = body.push(
            Row::with_children(
                found
                    .into_iter()
                    .map(|f| cell(f.emoji, format!(":{}:", f.shortcode))),
            )
            .spacing(2)
            .wrap(),
        );
    } else if !query.is_empty() && !has_extra {
        body = body.push(text("No emoji match.").size(13).color(CHANNEL));
    }
    container(body)
        .padding(10)
        .style(|_| container::Style {
            border: Border {
                color: LINE,
                width: 1.,
                radius: 8.0.into(),
            },
            ..super::surface(color!(0x2b2d31), 8.)
        })
        .into()
}
