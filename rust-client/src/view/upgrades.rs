//! Upgraded rooms in the conversation (upgrades.rs): the divider where the
//! newest room's history continues into an older room's, and the offer an old
//! room makes to join the room that replaced it.
use super::{
    App, BLURPLE, BODY, BOLD, CHANNEL, El, Icon, LINE, Msg, SIDE, icon, line_rule, surface,
};
use crate::{engine::Seam, rooms::Room};
use chrono::{DateTime, Local};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Padding, color,
    widget::{Column, button, column, container, row, text},
};

impl App {
    /// The dividers before the message at `index` of `history`'s order, or
    /// after the last message when `index` is its length.
    pub(super) fn seams<'a>(&self, history: &Room, index: usize) -> Option<El<'a>> {
        let mut seams = history.seams.iter().filter(|s| s.index == index).peekable();
        seams.peek()?;
        Some(Column::new().extend(seams.map(seam)).into())
    }

    /// An old room whose successor this account hasn't joined: say so, and
    /// offer to join it, where the composer would send.
    pub(super) fn upgrade_offer<'a>(&self, r: &Room) -> Option<El<'a>> {
        r.successor.as_ref()?;
        let busy = self.following(&self.selected);
        let ready = self.logged_in && self.verified && self.connected && !busy;
        let join = button(
            text(if busy {
                "Joining…"
            } else {
                "Join the new room"
            })
            .size(14)
            .font(BOLD),
        )
        .padding([6, 14])
        .on_press_maybe(ready.then(|| Msg::FollowUpgrade(self.selected.clone())))
        .style(|_, s| button::Style {
            background: Some(
                if s == button::Status::Hovered {
                    color!(0x4752c4)
                } else {
                    BLURPLE
                }
                .into(),
            ),
            text_color: Color::WHITE,
            border: Border::default().rounded(4),
            ..Default::default()
        });
        Some(
            container(
                row![
                    icon(Icon::Hash, 20., CHANNEL),
                    column![
                        text("This room was upgraded").size(14).font(BOLD).color(BODY),
                        text("It’s read-only now. The conversation continues in the new room, with this room’s history above it.")
                            .size(13)
                            .color(CHANNEL),
                    ]
                    .spacing(2)
                    .width(Fill),
                    join
                ]
                .spacing(12)
                .align_y(Center),
            )
            .padding([10, 14])
            .width(Fill)
            .style(|_| surface(SIDE, 8.))
            .into(),
        )
    }
}

fn seam<'a>(s: &Seam) -> El<'a> {
    let label = match s.time.and_then(DateTime::from_timestamp_millis) {
        Some(at) => format!(
            "Room upgraded on {}",
            at.with_timezone(&Local).format("%B %-d, %Y")
        ),
        None => "Room upgraded".into(),
    };
    let note = if s.readable {
        "Messages above are from before the upgrade and are read-only."
    } else {
        "Older messages from before the upgrade aren’t available to you."
    };
    container(
        column![
            row![
                line_rule(LINE),
                text(label).size(12).font(BOLD).color(CHANNEL),
                line_rule(LINE)
            ]
            .spacing(8)
            .align_y(Center),
            container(text(note).size(12).color(CHANNEL)).center_x(Fill),
        ]
        .spacing(4),
    )
    .padding(Padding {
        top: 20.,
        bottom: 4.,
        left: 16.,
        right: 16.,
    })
    .into()
}
