use super::{
    ACTIVE, BODY, BOLD, CHANNEL, El, Icon, LINK, MUTED, Msg, Room, SIDE, TEXT, face, icon,
    input_style, plain, primary, scroll_style, thin_scrollbar,
};
use crate::search::Search;
use chrono::{DateTime, Local};
use iced::{
    Alignment::Center,
    Border, Fill, color,
    widget::{Column, button, column, container, row, scrollable, text, text_input},
};

pub(super) fn view<'a>(
    s: &'a Search,
    room: &'a Room,
    avatars: &'a crate::avatars::Avatars,
    opening: bool,
) -> El<'a> {
    let link = |label: &'static str, msg: Option<Msg>| {
        button(text(label).size(14))
            .padding([4, 8])
            .style(plain(LINK, color!(0x5fc5ff)))
            .on_press_maybe(msg)
    };
    let mut panel = column![
        row![
            icon(Icon::Search, 20., color!(0x80848e)),
            text(format!("Search · {}", room.name))
                .size(16)
                .font(BOLD)
                .color(TEXT)
                .width(Fill),
            link("Return to timeline", Some(Msg::ReturnToReading))
        ]
        .spacing(8)
        .align_y(Center),
        row![
            text_input("Words or phrase", &s.input)
                .id("room-search-input")
                .on_input(Msg::SearchInput)
                .on_submit(Msg::RunSearch(false))
                .size(16)
                .padding(10)
                .style(input_style),
            button(text("Find").size(14).font(BOLD))
                .padding([10, 18])
                .style(primary)
                .on_press_maybe((!s.input.trim().is_empty()).then_some(Msg::RunSearch(false)))
        ]
        .spacing(8)
        .align_y(Center),
        row![
            text("Searches messages synced and readable on this device.")
                .size(13)
                .color(MUTED)
                .width(Fill),
            link(
                "Check coverage",
                s.coverage
                    .as_ref()
                    .is_none_or(|c| c.done || c.failed)
                    .then_some(Msg::SearchCoverage)
            )
        ]
        .spacing(6)
        .align_y(Center)
    ]
    .spacing(12);
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
            .size(14)
            .font(BOLD)
            .color(BODY),
        );
    }
    let mut list = Column::new().spacing(8);
    for hit in &s.hits {
        let date = DateTime::from_timestamp_millis(hit.time)
            .unwrap_or_default()
            .with_timezone(&Local);
        let sender = room.display(&hit.sender);
        list = list.push(
            button(
                row![
                    face(
                        sender,
                        avatars.get(room.avatars.get(&hit.sender).map(String::as_str)),
                        40.,
                    ),
                    column![
                        row![
                            text(sender).size(16).font(BOLD).color(TEXT),
                            text(date.format("%b %-d, %Y %H:%M").to_string())
                                .size(12)
                                .color(CHANNEL)
                        ]
                        .spacing(8)
                        .align_y(Center),
                        text(&hit.snippet).size(16).color(BODY)
                    ]
                    .spacing(3)
                    .width(Fill)
                ]
                .spacing(16),
            )
            .padding([12, 16])
            .width(Fill)
            .style(|_, st| button::Style {
                background: Some(
                    if st == button::Status::Hovered {
                        ACTIVE
                    } else {
                        SIDE
                    }
                    .into(),
                ),
                border: Border::default().rounded(8),
                ..Default::default()
            })
            .on_press(Msg::SearchHit(hit.id.clone())),
        );
    }
    if s.next.is_some() && !s.busy && !s.failed {
        list = list.push(
            container(link(
                "More matches",
                (s.input.trim() == s.term).then_some(Msg::RunSearch(true)),
            ))
            .center_x(Fill),
        );
    }
    container(
        panel.push(
            scrollable(list)
                .direction(thin_scrollbar())
                .style(scroll_style(None))
                .height(Fill),
        ),
    )
    .width(Fill)
    .height(Fill)
    .padding(16)
    .into()
}
