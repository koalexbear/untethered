use super::presence::with_dot;
use super::{
    ACTIVE, App, BLURPLE, BODY, BOLD, CHANNEL, El, FLOAT, HOVER, Icon, MUTED, Msg, RED, SIDE, TEXT,
    face, icon_button, input_style, label_owned, plain, scroll_style, surface, thin_scrollbar,
};
use crate::{
    moderation::{Action, Kind},
    presence::Support,
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, color,
    widget::{Column, Row, button, column, container, row, scrollable, text, text_input, tooltip},
};

fn action_button<'a>(action: Action, msg: Option<Msg>) -> El<'a> {
    let danger = matches!(action, Action::Kick | Action::Ban);
    button(text(action.label()).size(14).font(BOLD))
        .padding([6, 12])
        .on_press_maybe(msg)
        .style(move |_, s| {
            let (base, hover) = if danger {
                (color!(0xda373c), color!(0xa12828))
            } else {
                (BLURPLE, color!(0x4752c4))
            };
            button::Style {
                background: Some(
                    match s {
                        button::Status::Hovered => hover,
                        button::Status::Disabled => Color { a: 0.5, ..base },
                        _ => base,
                    }
                    .into(),
                ),
                text_color: Color::WHITE,
                border: Border::default().rounded(4),
                ..Default::default()
            }
        })
        .into()
}

/// A membership kind, its heading and the (ID, name) pairs shown under it.
type Group<'a> = (Kind, &'static str, Vec<(&'a String, &'a str)>);

impl App {
    /// A member's loaded profile picture: from their membership, or as the
    /// open room's timeline shows them.
    fn member_picture(&self, user: &str) -> Option<&iced::widget::image::Handle> {
        let mxc = self
            .directory
            .users
            .get(user)
            .and_then(|m| m.avatar.as_deref())
            .or_else(|| {
                self.live
                    .list
                    .get(&self.selected)?
                    .avatars
                    .get(user)
                    .map(String::as_str)
            });
        self.avatars.get(mxc)
    }
}

pub(super) fn view(app: &App) -> El<'_> {
    let d = &app.directory;
    // Badges come from the open room's power levels (names.rs).
    let role = |user: &str| app.live.list.get(&app.selected)?.role(user);
    let busy = app.membership_busy();
    let mut panel = column![
        row![
            text_input("Search members", &d.filter)
                .on_input(Msg::MemberFilter)
                .size(14)
                .padding([6, 8])
                .style(input_style)
                .width(Fill),
            button(text("Refresh").size(12))
                .padding([2, 4])
                .style(plain(CHANNEL, BODY))
                .on_press_maybe((app.connected && !busy).then_some(Msg::RefreshMembers))
        ]
        .spacing(6)
        .align_y(Center),
    ]
    .spacing(10)
    .padding([12, 8]);
    if let Some(id) = &d.selected
        && let Some(m) = d.users.get(id)
    {
        let name = if m.name.is_empty() {
            crate::rooms::short(id)
        } else {
            &m.name
        };
        let presence = app.presence_of(id);
        let mut who = column![
            super::names::author(name, role(id), 16.),
            text(id).size(12).color(MUTED),
        ]
        .spacing(2)
        .width(Fill);
        if let Some(p) = &presence {
            let line = match p.message.as_ref().filter(|_| p.status.here()) {
                Some(message) => format!("{} · {message}", p.status.label()),
                None => p.status.label().to_owned(),
            };
            who = who.push(text(line).size(12).color(BODY));
        }
        let mut card = column![
            row![
                with_dot(
                    face(name, app.member_picture(id), 48.),
                    48.,
                    presence.as_ref().map(|p| p.status),
                    FLOAT
                ),
                who,
                icon_button(
                    Icon::Close,
                    "Close",
                    false,
                    Some(Msg::MemberFilter(d.filter.clone())),
                    tooltip::Position::Left
                )
            ]
            .spacing(10)
            .align_y(Center),
            text(format!("MEMBERSHIP · {}", m.kind.label().to_uppercase()))
                .size(12)
                .font(BOLD)
                .color(CHANNEL),
        ]
        .spacing(10);
        let mut actions = Row::new().spacing(6);
        let mut available = false;
        for action in Action::ALL {
            if let Some(change) = app.member_change(action) {
                available = true;
                actions = actions.push(action_button(
                    action,
                    (!busy).then_some(Msg::Moderate(change)),
                ));
            }
        }
        if available {
            card = card
                .push(
                    text_input("Reason (optional)", &d.reason)
                        .on_input_maybe((!busy).then_some(Msg::MemberReason))
                        .size(14)
                        .padding([6, 8])
                        .style(input_style),
                )
                .push(actions.wrap());
        }
        if busy {
            card = card.push(text("Updating room membership…").size(12).color(MUTED));
        }
        if !d.note.is_empty() {
            card = card.push(text(&d.note).size(12).color(BODY));
        }
        panel = panel.push(
            container(card)
                .padding(12)
                .width(Fill)
                .style(|_| surface(FLOAT, 8.)),
        );
    } else {
        if busy {
            panel = panel.push(text("Updating room membership…").size(12).color(MUTED));
        }
        if !d.note.is_empty() {
            panel = panel.push(text(&d.note).size(12).color(BODY));
        }
    }
    let query = d.filter.trim().to_lowercase();
    // Where the homeserver shares presence, members are Online or Offline
    // as on Discord (presence.rs).
    let presence = app.presence.support == Support::On;
    let here = |id: &str| app.status_of(id).is_some_and(|s| s.here());
    let mut groups: [Group; 6] = [
        (
            Kind::Joined,
            if presence { "ONLINE" } else { "MEMBERS" },
            vec![],
        ),
        (Kind::Joined, "OFFLINE", vec![]),
        (Kind::Knocking, "REQUESTED", vec![]),
        (Kind::Invited, "INVITED", vec![]),
        (Kind::Banned, "BANNED", vec![]),
        (Kind::Left, "LEFT", vec![]),
    ];
    let mut total = 0;
    for (id, m) in &d.users {
        if query.is_empty() && m.kind == Kind::Left && d.selected.as_ref() != Some(id) {
            continue;
        }
        if !query.is_empty()
            && !id.to_lowercase().contains(&query)
            && !m.name.to_lowercase().contains(&query)
        {
            continue;
        }
        total += 1;
        if total > 80 {
            continue;
        }
        let name = if m.name.is_empty() {
            crate::rooms::short(id)
        } else {
            &m.name
        };
        let offline = presence && m.kind == Kind::Joined && !here(id);
        if let Some(group) = groups
            .iter_mut()
            .skip(usize::from(offline))
            .find(|(k, ..)| *k == m.kind)
        {
            group.2.push((id, name));
        }
    }
    let mut list = Column::new().spacing(2);
    for (kind, title, members) in groups {
        if members.is_empty() {
            continue;
        }
        list = list.push(
            container(label_owned(format!("{title} — {}", members.len()))).padding(iced::Padding {
                top: 16.,
                bottom: 4.,
                left: 8.,
                right: 8.,
            }),
        );
        for (id, name) in members {
            let selected = d.selected.as_ref() == Some(id);
            // Offline members have no dot and are dimmed, as on Discord.
            let presence = (kind == Kind::Joined)
                .then(|| app.presence_of(id))
                .flatten()
                .filter(|p| p.status.here());
            let dim = kind != Kind::Joined || title == "OFFLINE";
            let second = presence
                .as_ref()
                .and_then(|p| p.message.clone())
                .unwrap_or_else(|| id.clone());
            let mut label = text(name).size(15).wrapping(text::Wrapping::None);
            let title: El = match role(id) {
                Some(role) => {
                    let tint = super::names::color(Some(role), TEXT);
                    label = label.color(Color {
                        a: if dim { 0.55 } else { 1. },
                        ..tint
                    });
                    row![label, super::names::tag(role)]
                        .spacing(6)
                        .align_y(Center)
                        .into()
                }
                None => label.into(),
            };
            let entry = button(
                row![
                    with_dot(
                        face(name, app.member_picture(id), 32.),
                        32.,
                        presence.map(|p| p.status),
                        if selected { ACTIVE } else { SIDE }
                    ),
                    column![
                        title,
                        text(second)
                            .size(12)
                            .color(CHANNEL)
                            .wrapping(text::Wrapping::None)
                    ]
                    .spacing(1)
                ]
                .spacing(12)
                .align_y(Center),
            )
            .width(Fill)
            .padding([5, 8])
            .on_press(Msg::SelectMember(id.clone()))
            .style(move |_, s| button::Style {
                background: if selected {
                    Some(ACTIVE.into())
                } else if s == button::Status::Hovered {
                    Some(HOVER.into())
                } else {
                    None
                },
                text_color: if selected || s == button::Status::Hovered {
                    TEXT
                } else if dim {
                    color!(0x80848e)
                } else {
                    BODY
                },
                border: Border::default().rounded(4),
                ..Default::default()
            });
            list = list.push(app.user_area(id, "member".into(), entry));
        }
    }
    if total == 0 && !busy {
        list = list
            .push(container(text("No matching members.").size(13).color(MUTED)).padding([12, 8]));
    }
    if total > 80 {
        panel = panel.push(
            text(format!("Showing 80 of {total}. Type to narrow the list."))
                .size(12)
                .color(RED),
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
    .width(240)
    .height(Fill)
    .style(|_| surface(SIDE, 0.))
    .into()
}
