//! Ctrl+/'s list of keyboard shortcuts (shortcuts.rs), shaped like
//! Discord's: each thing you can do, with its keys drawn as keycaps.
use super::{
    BOLD, CHANNEL, CHAT, El, FIELD, Icon, MUTED, Msg, SIDE, TEXT, icon_button, scroll_style,
    surface, thin_scrollbar,
};
use crate::{
    App,
    shortcuts::{Input, list},
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Padding,
    widget::{
        Column, Row, Space, column, container, mouse_area, opaque, row, scrollable, stack, text,
        tooltip,
    },
};

/// One key, as a keycap.
fn keycap(key: &'static str) -> El<'static> {
    container(text(key).size(13).font(BOLD).color(TEXT))
        .padding([3, 8])
        .style(|_| container::Style {
            background: Some(FIELD.into()),
            border: Border {
                color: Color::from_rgba8(0, 0, 0, 0.5),
                width: 1.,
                radius: 4.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// "Ctrl+Alt+↓" as keycaps, and each other way of doing it after a slash.
fn combos(ways: &'static [&'static str]) -> El<'static> {
    let mut line = Row::new().spacing(4).align_y(Center);
    for (i, way) in ways.iter().enumerate() {
        if i > 0 {
            line = line.push(text("/").size(14).color(CHANNEL));
        }
        for key in way.split('+') {
            line = line.push(keycap(key));
        }
    }
    line.into()
}

impl App {
    pub(super) fn shortcuts_view(&self) -> Option<El<'_>> {
        if !self.shortcuts.open {
            return None;
        }
        let close = icon_button(
            Icon::Close,
            "Close",
            false,
            Some(Msg::Shortcut(Input::Close)),
            tooltip::Position::Bottom,
        );
        let title = row![
            text("Keyboard Shortcuts")
                .size(20)
                .font(BOLD)
                .color(TEXT)
                .width(Fill),
            close
        ]
        .align_y(Center);
        let mut groups = Column::new().spacing(18);
        for (heading, rows) in list() {
            let mut group = Column::new()
                .spacing(6)
                .push(text(heading).size(12).font(BOLD).color(CHANNEL));
            for (what, ways) in rows {
                group = group.push(
                    container(
                        row![text(what).size(15).color(MUTED).width(Fill), combos(ways)]
                            .spacing(12)
                            .align_y(Center),
                    )
                    .padding([6, 10])
                    .style(|_| surface(CHAT, 4.)),
                );
            }
            groups = groups.push(group);
        }
        let dialog = container(
            column![
                title,
                scrollable(groups.padding(Padding {
                    right: 12.,
                    ..Padding::ZERO
                }))
                .direction(thin_scrollbar())
                .style(scroll_style(None))
                .height(iced::Length::Shrink)
            ]
            .spacing(16),
        )
        .padding(16)
        .width(620)
        .max_height(640)
        .style(|_| surface(SIDE, 8.));
        let backdrop = mouse_area(container(Space::new()).width(Fill).height(Fill).style(|_| {
            container::Style {
                background: Some(Color::from_rgba8(0, 0, 0, 0.7).into()),
                ..Default::default()
            }
        }))
        .on_press(Msg::Shortcut(Input::Close));
        let placed = container(dialog).center_x(Fill).padding(Padding {
            top: 80.,
            bottom: 40.,
            ..Padding::ZERO
        });
        Some(stack![opaque(backdrop), placed].into())
    }
}
