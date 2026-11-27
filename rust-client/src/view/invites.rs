//! Invites on screen (invites.rs): a space's in the server rail, rooms' and
//! DMs' at the top of the channel list, and the card the main area shows for
//! the one opened, with Accept and Decline.
use super::{
    ACTIVE, App, BODY, BOLD, CHANNEL, El, HOVER, Icon, MUTED, Mark, Msg, SIDE, TEXT, face, icon,
    rail_button, surface,
};
use crate::{Section, engine::Invite};
use iced::{
    Alignment::Center,
    Border, Color, Fill, color,
    widget::{Column, button, column, container, row, text},
};

/// Discord's green for accepting.
const ACCEPT: Color = color!(0x248046);
const ACCEPT_HOVER: Color = color!(0x1a6334);
const DECLINE: Color = color!(0x4e5058);
const DECLINE_HOVER: Color = color!(0x6d6f78);

fn inviter(i: &Invite) -> &str {
    i.inviter_name
        .as_deref()
        .or(i.inviter.as_deref())
        .unwrap_or("Someone")
}

impl App {
    /// Space invites, as rail entries with a mark until answered.
    pub(super) fn invite_rail(&self) -> Vec<El<'_>> {
        if !self.logged_in {
            return Vec::new();
        }
        self.live
            .invites
            .iter()
            .filter(|i| i.space)
            .map(|i| {
                let initials = i
                    .name
                    .split_whitespace()
                    .take(3)
                    .filter_map(|s| s.chars().next())
                    .collect::<String>()
                    .to_uppercase();
                rail_button(
                    initials,
                    self.avatars.get(i.avatar.as_deref()),
                    &i.name,
                    self.invite_open.as_deref() == Some(i.id.as_str()),
                    Msg::OpenInvite(i.id.clone()),
                    Mark {
                        unread: true,
                        mentions: 1,
                    },
                )
            })
            .collect()
    }

    /// The section's invites atop the channel list, under their own heading.
    pub(super) fn invite_rows(&self, section: &Section) -> Option<El<'_>> {
        let invites = self.section_invites(section);
        if invites.is_empty() {
            return None;
        }
        let key = format!(
            "{}/invites",
            match section {
                Section::Direct => "direct",
                _ => "all",
            }
        );
        let mut list = Column::new()
            .spacing(2)
            .push(self.category(&key, "INVITES", None));
        for i in invites {
            let active = self.invite_open.as_deref() == Some(i.id.as_str());
            let leading: El = if i.dm {
                face(&i.name, self.avatars.get(i.avatar.as_deref()), 32.)
            } else {
                icon(Icon::Hash, 20., color!(0x80848e))
            };
            let words = column![
                text(&i.name)
                    .size(15)
                    .font(BOLD)
                    .color(TEXT)
                    .wrapping(text::Wrapping::None),
                text(format!("Invite from {}", inviter(i)))
                    .size(12)
                    .color(CHANNEL)
                    .wrapping(text::Wrapping::None),
            ];
            list = list.push(
                button(row![leading, words].spacing(10).align_y(Center))
                    .width(Fill)
                    .padding([5, 8])
                    .on_press(Msg::OpenInvite(i.id.clone()))
                    .style(move |_, s| button::Style {
                        background: if active {
                            Some(ACTIVE.into())
                        } else if s == button::Status::Hovered {
                            Some(HOVER.into())
                        } else {
                            None
                        },
                        border: Border::default().rounded(4),
                        ..Default::default()
                    }),
            );
        }
        Some(list.into())
    }

    /// The opened invite's card, as Discord shows an invite.
    pub(super) fn invite_card<'a>(&'a self, i: &'a Invite) -> El<'a> {
        let busy = self.invite_busy(&i.id);
        let ready = self.connected && self.verified && !busy;
        let accepting = self
            .pending
            .values()
            .any(|r| matches!(r, crate::Request::AcceptInvite(room) if room == &i.id));
        let choice = |label: &'static str, msg: Msg, base: Color, hover: Color| {
            button(text(label).size(14).font(BOLD))
                .padding([10, 24])
                .on_press_maybe(ready.then_some(msg))
                .style(move |_, s| button::Style {
                    background: Some(
                        match s {
                            button::Status::Hovered => hover,
                            _ => base,
                        }
                        .into(),
                    ),
                    text_color: Color::WHITE,
                    border: Border::default().rounded(4),
                    ..Default::default()
                })
        };
        let heading = if i.dm {
            "wants to message you"
        } else if i.space {
            "invited you to join a space"
        } else {
            "invited you to join"
        };
        let mut card = column![
            face(&i.name, self.avatars.get(i.avatar.as_deref()), 96.),
            text(format!("{} {heading}", inviter(i)))
                .size(14)
                .color(MUTED),
            text(&i.name).size(28).font(BOLD).color(TEXT),
        ]
        .spacing(12)
        .align_x(Center);
        if !i.topic.trim().is_empty() {
            let topic: String = i.topic.chars().take(240).collect();
            card = card.push(text(topic).size(14).color(BODY).center());
        }
        if let Some(id) = &i.inviter
            && i.inviter_name.is_some()
        {
            card = card.push(text(id.clone()).size(12).color(CHANNEL));
        }
        card = card.push(
            row![
                choice(
                    if busy && !accepting {
                        "Declining…"
                    } else {
                        "Decline"
                    },
                    Msg::DeclineInvite(i.id.clone()),
                    DECLINE,
                    DECLINE_HOVER
                ),
                choice(
                    if accepting { "Joining…" } else { "Accept" },
                    Msg::AcceptInvite(i.id.clone()),
                    ACCEPT,
                    ACCEPT_HOVER
                ),
            ]
            .spacing(12),
        );
        container(
            container(card)
                .padding(32)
                .max_width(480)
                .style(|_| surface(SIDE, 8.)),
        )
        .center(Fill)
        .style(|_| surface(super::CHAT, 0.))
        .into()
    }
}
