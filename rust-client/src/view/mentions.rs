use super::{
    ACTIVE, App, BODY, BOLD, CHANNEL, El, Icon, LINK, MUTED, Msg, SIDE, TEXT, avatar, icon,
    line_rule, plain, scroll_style, thin_scrollbar,
};
use crate::mentions::Inbox;
use chrono::{DateTime, Local};
use iced::{
    Alignment::Center,
    Border, Fill, color,
    widget::{Column, button, column, container, row, scrollable, text},
};

pub(super) fn view<'a>(app: &'a App, inbox: &'a Inbox) -> El<'a> {
    let header = container(
        row![
            icon(Icon::At, 24., color!(0x80848e)),
            text("Mentions inbox")
                .size(16)
                .font(BOLD)
                .color(TEXT)
                .width(Fill),
            button(text("Refresh").size(14))
                .padding([4, 8])
                .style(plain(LINK, color!(0x5fc5ff)))
                .on_press_maybe((app.connected && !inbox.loading()).then_some(Msg::OpenMentions))
        ]
        .spacing(8)
        .align_y(Center),
    )
    .padding([0, 16])
    .center_y(48)
    .width(Fill);
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
    let mut list = Column::new().spacing(8);
    for h in inbox.hits.iter().take(inbox.shown) {
        let Some(r) = app.live.list.get(&h.room) else {
            continue;
        };
        let stamp = DateTime::from_timestamp_millis(h.time)
            .unwrap_or_default()
            .with_timezone(&Local);
        let sender = r.display(&h.sender);
        list = list.push(
            button(
                row![
                    avatar(sender, 40.),
                    column![
                        row![
                            text(sender).size(16).font(BOLD).color(TEXT),
                            text(stamp.format("%b %-d, %Y %H:%M").to_string())
                                .size(12)
                                .color(CHANNEL),
                        ]
                        .spacing(8)
                        .align_y(Center),
                        text(format!("#{}", r.name)).size(12).color(MUTED),
                        text(&h.preview).size(16).color(BODY)
                    ]
                    .spacing(3)
                    .width(Fill)
                ]
                .spacing(16),
            )
            .width(Fill)
            .padding([12, 16])
            .on_press(Msg::MentionHit(h.room.clone(), h.event.clone()))
            .style(|_, s| button::Style {
                background: Some(
                    if s == button::Status::Hovered {
                        ACTIVE
                    } else {
                        SIDE
                    }
                    .into(),
                ),
                border: Border::default().rounded(8),
                ..Default::default()
            }),
        );
    }
    if inbox.hits.len() > inbox.shown {
        list = list.push(
            container(
                button(text("Show older mentions").size(14))
                    .padding([6, 12])
                    .style(plain(LINK, color!(0x5fc5ff)))
                    .on_press(Msg::MoreMentions),
            )
            .center_x(Fill),
        );
    }
    column![
        header,
        line_rule(color!(0x1f2023)),
        container(
            column![
                text("Direct mentions in joined rooms, from messages readable on this device.")
                    .size(14)
                    .color(MUTED),
                text(status).size(14).font(BOLD).color(BODY),
                scrollable(list)
                    .direction(thin_scrollbar())
                    .style(scroll_style(None))
                    .height(Fill)
            ]
            .spacing(12)
        )
        .padding([16, 16])
        .width(Fill)
        .height(Fill)
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
