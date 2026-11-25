use super::{App, BOLD, El, MUTED, Msg, nav};
use crate::mentions::Inbox;
use chrono::{DateTime, Local};
use iced::{
    Fill,
    widget::{Column, button, column, container, row, scrollable, text},
};

pub(super) fn view<'a>(app: &'a App, inbox: &'a Inbox) -> El<'a> {
    let header = row![
        text("Mentions inbox").size(22).font(BOLD).width(Fill),
        button("Refresh")
            .style(nav(false))
            .on_press_maybe((app.connected && !inbox.loading()).then_some(Msg::OpenMentions))
    ];
    let status = if inbox.failed {
        "Could not load mentions. Refresh to retry.".into()
    } else if inbox.loading() {
        "Checking local history…".into()
    } else if inbox.hits.is_empty() {
        "No direct mentions on this device.".into()
    } else {
        format!(
            "{} direct {}",
            inbox.hits.len(),
            if inbox.hits.len() == 1 {
                "mention"
            } else {
                "mentions"
            }
        )
    };
    let mut list = Column::new().spacing(6);
    for h in inbox.hits.iter().take(inbox.shown) {
        let Some(r) = app.live.list.get(&h.room) else {
            continue;
        };
        let stamp = DateTime::from_timestamp_millis(h.time)
            .unwrap_or_default()
            .with_timezone(&Local);
        let byline = format!(
            "{} · {} · {}",
            r.name,
            r.display(&h.sender),
            stamp.format("%b %-d, %Y %H:%M")
        );
        list = list.push(
            button(
                column![
                    text(byline).size(13).color(MUTED),
                    text(&h.preview).size(16)
                ]
                .spacing(3),
            )
            .style(nav(false))
            .width(Fill)
            .padding([7, 10])
            .on_press(Msg::MentionHit(h.room.clone(), h.event.clone())),
        );
    }
    if inbox.hits.len() > inbox.shown {
        list = list.push(button("Show older mentions").on_press(Msg::MoreMentions));
    }
    let description =
        text("Direct mentions in joined rooms, from messages readable on this device.")
            .size(14)
            .color(MUTED);
    container(
        column![
            header,
            description,
            text(status).size(14),
            scrollable(list).height(Fill)
        ]
        .spacing(10),
    )
    .padding(18)
    .width(Fill)
    .height(Fill)
    .into()
}
