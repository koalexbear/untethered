use super::{BOLD, El, MUTED, Msg, Room, nav};
use crate::search::Search;
use chrono::{DateTime, Local};
use iced::{
    Fill,
    widget::{Column, button, column, container, row, scrollable, text, text_input},
};

pub(super) fn view<'a>(s: &'a Search, room: &'a Room, opening: bool) -> El<'a> {
    let mut panel = column![
        row![
            text(format!("Search · {}", room.name))
                .font(BOLD)
                .width(Fill),
            button("Return to timeline")
                .style(nav(false))
                .on_press(Msg::ReturnToReading)
        ]
        .spacing(8),
        row![
            text("Searches messages synced and readable on this device.")
                .size(13)
                .color(MUTED)
                .width(Fill),
            button(text("Check coverage").size(14))
                .style(nav(false))
                .on_press_maybe(
                    s.coverage
                        .as_ref()
                        .is_none_or(|c| c.done || c.failed)
                        .then_some(Msg::SearchCoverage)
                )
        ]
        .spacing(6),
        row![
            text_input("Words or phrase", &s.input)
                .id("room-search-input")
                .on_input(Msg::SearchInput)
                .on_submit(Msg::RunSearch(false))
                .padding(8),
            button("Find")
                .on_press_maybe((!s.input.trim().is_empty()).then_some(Msg::RunSearch(false)))
        ]
        .spacing(6)
    ]
    .spacing(8);
    if let Some(c) = &s.coverage {
        let oldest = c
            .oldest
            .and_then(DateTime::from_timestamp_millis)
            .map(|ts| {
                format!(
                    " · oldest {}",
                    ts.with_timezone(&Local).format("%b %-d, %Y")
                )
            })
            .unwrap_or_default();
        let status = if c.failed {
            "Coverage check failed".into()
        } else if c.done {
            format!("{} readable messages cached{}", c.messages, oldest)
        } else {
            format!("Counting cached messages… {}", c.messages)
        };
        panel = panel.push(text(status).color(MUTED).size(13));
    }
    if !s.term.is_empty() {
        panel = panel.push(
            text(if opening {
                "Opening message context…".into()
            } else if s.failed {
                "Local search failed. Try Find again.".into()
            } else if s.busy {
                "Searching local history…".into()
            } else if s.hits.is_empty() {
                format!("No matches for “{}” on this device.", s.term)
            } else {
                format!(
                    "{} {} for “{}”",
                    s.hits.len(),
                    if s.hits.len() == 1 {
                        "match"
                    } else {
                        "matches"
                    },
                    s.term
                )
            })
            .size(14),
        );
    }
    let mut list = Column::new().spacing(5);
    for hit in &s.hits {
        let date = DateTime::from_timestamp_millis(hit.time)
            .unwrap_or_default()
            .with_timezone(&Local);
        list = list.push(
            button(
                column![
                    text(format!(
                        "{} · {}",
                        room.display(&hit.sender),
                        date.format("%b %-d, %Y %H:%M")
                    ))
                    .size(12)
                    .color(MUTED),
                    text(&hit.snippet).size(16)
                ]
                .spacing(3),
            )
            .padding([6, 10])
            .width(Fill)
            .style(nav(false))
            .on_press(Msg::SearchHit(hit.id.clone())),
        );
    }
    if s.next.is_some() && !s.busy && !s.failed {
        list = list.push(
            button("More matches")
                .on_press_maybe((s.input.trim() == s.term).then_some(Msg::RunSearch(true))),
        );
    }
    container(panel.push(scrollable(list).height(Fill)))
        .width(Fill)
        .height(Fill)
        .padding(16)
        .into()
}
