//! New message (dms.rs) in a modal shaped like Discord's "Select Friends":
//! a search of the user directory, the people picked, and Create DM; and
//! Settings' list of people you block.
use super::{
    App, BODY, BOLD, CHANNEL, CHAT, El, HOVER, Icon, MUTED, Msg, RAIL, SIDE, TEXT, face,
    icon_button, input_style, plain, primary, scroll_style, surface, thin_scrollbar,
};
use crate::{
    context_menu::Event as Menu,
    dms::{Input, MAX_PEOPLE, SEARCH},
    engine::Person,
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, color,
    widget::{
        Column, Space, button, checkbox, column, container, row, scrollable, text, text_input,
        tooltip,
    },
};

const WARN: Color = color!(0xffc785);

impl App {
    pub(super) fn new_message_dialog(&self) -> Option<El<'_>> {
        let form = self.new_message.as_ref()?;
        let busy = self.starting_dm();
        let close = (!busy).then_some(Msg::Dm(Input::Close));
        let left = MAX_PEOPLE - form.picked.len();
        let subtitle = match left {
            0 => "That's as many as a group DM holds.".to_owned(),
            MAX_PEOPLE => "Find people by name or Matrix ID.".to_owned(),
            1 => "You can add 1 more person.".to_owned(),
            n => format!("You can add {n} more people."),
        };
        let header = row![
            column![
                text("New Message").size(20).font(BOLD).color(TEXT),
                text(subtitle).size(12).color(MUTED)
            ]
            .spacing(4)
            .width(Fill),
            icon_button(
                Icon::Close,
                "Close",
                false,
                close.clone(),
                tooltip::Position::Left
            )
        ]
        .align_y(iced::alignment::Vertical::Top);

        // The people picked sit in the search box, as on Discord.
        let mut field = row![].spacing(4).align_y(Center);
        for p in &form.picked {
            field = field.push(
                button(
                    row![
                        text(name(p)).size(13).color(TEXT),
                        text("×").size(14).color(MUTED)
                    ]
                    .spacing(6)
                    .align_y(Center),
                )
                .padding([3, 8])
                .on_press_maybe((!busy).then(|| Msg::Dm(Input::Remove(p.id.clone()))))
                .style(|_, s| button::Style {
                    background: Some(
                        if s == button::Status::Hovered {
                            HOVER
                        } else {
                            CHAT
                        }
                        .into(),
                    ),
                    border: Border::default().rounded(4),
                    ..Default::default()
                }),
            );
        }
        let start =
            (self.connected && !busy && !form.picked.is_empty()).then_some(Msg::Dm(Input::Start));
        field = field.push(
            text_input("Type a name or @user:server", &form.query)
                .id(SEARCH)
                .size(15)
                .padding([6, 4])
                .style(|theme, status| text_input::Style {
                    background: Color::TRANSPARENT.into(),
                    border: Border::default(),
                    ..input_style(theme, status)
                })
                .on_input_maybe((!busy).then_some(|q| Msg::Dm(Input::Query(q))))
                .on_submit_maybe(start.clone()),
        );
        let field = container(field.wrap())
            .padding([4, 8])
            .width(Fill)
            .style(|_| surface(RAIL, 4.));

        let people = form.people(&self.matrix_id, &self.blocked);
        let term = form.query.trim();
        let hint = if term.is_empty() {
            Some("Search the people your homeserver knows, or enter a full Matrix ID.")
        } else if people.is_empty() && form.searching() {
            Some("Searching…")
        } else if people.is_empty() {
            Some("No one found. Enter their full Matrix ID, like @alex:example.org.")
        } else {
            None
        };
        let mut list = Column::new().spacing(2);
        for p in people {
            let picked = form.picked(&p.id);
            list = list.push(self.person_row(p, picked, busy));
        }
        let mut body = column![header, field].spacing(16);
        if let Some(hint) = hint {
            body = body.push(text(hint).size(14).color(CHANNEL));
        } else {
            body = body.push(
                scrollable(list)
                    .direction(thin_scrollbar())
                    .style(scroll_style(None))
                    .height(iced::Length::Shrink)
                    .width(Fill),
            );
        }
        if let Some(note) = form.note {
            body = body.push(text(note).size(13).color(WARN));
        }

        let create = button(
            text(if busy {
                "Starting…"
            } else if form.picked.len() > 1 {
                "Create Group DM"
            } else {
                "Create DM"
            })
            .size(14)
            .font(BOLD),
        )
        .padding([10, 18])
        .style(primary)
        .on_press_maybe(start);
        let cancel = button(text("Cancel").size(14))
            .padding([10, 16])
            .style(plain(BODY, Color::WHITE))
            .on_press_maybe(close.clone());
        let footer = container(
            row![Space::new().width(Fill), cancel, create]
                .spacing(8)
                .align_y(Center),
        )
        .padding(16)
        .width(Fill)
        .style(|_| container::Style {
            background: Some(SIDE.into()),
            border: Border::default().rounded(iced::border::bottom(8)),
            ..Default::default()
        });
        let dialog = container(column![
            container(body).padding([20, 16]).max_height(460.),
            footer
        ])
        .width(460)
        .style(|_| surface(CHAT, 8.));
        let backdrop =
            iced::widget::mouse_area(container(Space::new()).width(Fill).height(Fill).style(
                |_| container::Style {
                    background: Some(Color::from_rgba8(0, 0, 0, 0.7).into()),
                    ..Default::default()
                },
            ));
        let backdrop = match close {
            Some(close) => backdrop.on_press(close),
            None => backdrop,
        };
        Some(
            iced::widget::stack![
                iced::widget::opaque(backdrop),
                container(dialog).center(Fill)
            ]
            .into(),
        )
    }

    fn person_row(&self, p: Person, picked: bool, busy: bool) -> El<'_> {
        let toggle = (!busy).then(|| Msg::Dm(Input::Toggle(p.clone())));
        let shown = name(&p).to_owned();
        let mut who = column![text(shown.clone()).size(15).font(BOLD).color(TEXT)].spacing(1);
        if p.name.is_some() {
            who = who.push(text(p.id.clone()).size(12).color(MUTED));
        }
        button(
            row![
                face(&shown, self.avatars.get(p.avatar.as_deref()), 32.),
                who.width(Fill),
                checkbox(picked)
                    .size(18)
                    .on_toggle_maybe(toggle.clone().map(|m| move |_| m.clone())),
            ]
            .spacing(10)
            .align_y(Center),
        )
        .width(Fill)
        .padding([6, 8])
        .on_press_maybe(toggle)
        .style(|_, s| button::Style {
            background: (s == button::Status::Hovered).then(|| HOVER.into()),
            border: Border::default().rounded(4),
            ..Default::default()
        })
        .into()
    }

    /// Settings' Privacy rows: who you block, each with Unblock.
    pub(super) fn blocked_list(&self) -> El<'static> {
        let mut rows = column![
            text("Blocked people").size(16).font(BOLD).color(TEXT),
            text(if self.blocked.is_empty() {
                "You haven't blocked anyone. Block someone from their profile or by right-clicking their name."
            } else {
                "Their messages, invites and calls stay away, here and in other Matrix apps. They aren't told."
            })
            .size(14)
            .color(MUTED)
        ]
        .spacing(4);
        for user in &self.blocked {
            let name = self.someone(user);
            let mut who = column![text(name.clone()).size(15).font(BOLD).color(TEXT)];
            if name != *user {
                who = who.push(text(user.clone()).size(12).color(MUTED));
            }
            rows = rows.push(
                container(
                    row![
                        face(&name, None, 32.),
                        who.width(Fill),
                        button(text("Unblock").size(14))
                            .padding([7, 14])
                            .style(|_, s| button::Style {
                                background: Some(
                                    match s {
                                        button::Status::Hovered | button::Status::Pressed => {
                                            color!(0x6d6f78)
                                        }
                                        _ => color!(0x4e5058),
                                    }
                                    .into(),
                                ),
                                text_color: Color::WHITE,
                                border: Border::default().rounded(4),
                                ..Default::default()
                            })
                            .on_press_maybe(
                                (self.connected && !self.blocking(user))
                                    .then(|| Msg::ContextMenu(Menu::Unblock(user.clone())))
                            ),
                    ]
                    .spacing(10)
                    .align_y(Center),
                )
                .padding([8, 0]),
            );
        }
        rows.into()
    }
}

fn name(p: &Person) -> &str {
    p.name.as_deref().unwrap_or(&p.id)
}
