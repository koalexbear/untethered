//! A room reached by a link or address, before joining (joining.rs): its
//! picture, name, topic and members, with Join, as Discord previews a
//! server from an invite link.
use super::{App, BODY, BOLD, CHANNEL, El, MUTED, Msg, SIDE, TEXT, face, surface};
use crate::{
    engine::JoinRule,
    joining::{Preview, PreviewState, REFUSED},
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, color,
    widget::{button, column, container, row, text},
};

const JOIN: Color = color!(0x248046);
const JOIN_HOVER: Color = color!(0x1a6334);
const CLOSE: Color = color!(0x4e5058);
const CLOSE_HOVER: Color = color!(0x6d6f78);
const WARN: Color = color!(0xffc785);

fn choice<'a>(label: &'a str, msg: Option<Msg>, base: Color, hover: Color) -> El<'a> {
    button(text(label).size(14).font(BOLD))
        .padding([10, 24])
        .on_press_maybe(msg)
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
        .into()
}

impl App {
    pub(super) fn preview_card<'a>(&'a self, preview: &'a Preview) -> El<'a> {
        let target = preview.link.target.as_str();
        let joining = self.joining(preview.join_target());
        let ready = self.connected && self.verified && !joining;
        let close = choice("Close", Some(Msg::ClosePreview), CLOSE, CLOSE_HOVER);
        let join_label = if joining { "Joining…" } else { "Join" };
        let card = match &preview.state {
            PreviewState::Loading => column![
                text("Looking up").size(14).color(MUTED),
                text(target).size(24).font(BOLD).color(TEXT),
                close,
            ],
            PreviewState::Failed => column![
                text(target).size(24).font(BOLD).color(TEXT),
                text("Couldn't preview this room. It may be private, or the link may be wrong.")
                    .size(14)
                    .color(BODY)
                    .center(),
                row![
                    close,
                    choice(
                        if joining { "Joining…" } else { "Try to join" },
                        ready.then_some(Msg::JoinPreviewed),
                        JOIN,
                        JOIN_HOVER
                    )
                ]
                .spacing(12),
            ],
            PreviewState::Ready(p) => {
                let name = p
                    .name
                    .as_deref()
                    .or(p.alias.as_deref())
                    .unwrap_or(p.room_id.as_str());
                let mut card = column![
                    face(name, self.avatars.get(p.avatar.as_deref()), 96.),
                    text(if p.space {
                        "A Matrix space"
                    } else {
                        "A Matrix room"
                    })
                    .size(14)
                    .color(MUTED),
                    text(name).size(28).font(BOLD).color(TEXT),
                ];
                if let Some(alias) = p
                    .alias
                    .as_deref()
                    .filter(|a| p.name.is_some() && *a != name)
                {
                    card = card.push(text(alias).size(13).color(CHANNEL));
                }
                if let Some(topic) = &p.topic {
                    let topic: String = topic.chars().take(240).collect();
                    card = card.push(text(topic).size(14).color(BODY).center());
                }
                card = card.push(
                    text(format!(
                        "{} {}",
                        p.members,
                        if p.members == 1 { "member" } else { "members" }
                    ))
                    .size(13)
                    .color(CHANNEL),
                );
                let blocked = match p.join {
                    JoinRule::Open | JoinRule::Members => None,
                    JoinRule::Knock => {
                        Some("This room asks people to knock first, which Untethered can't do yet.")
                    }
                    JoinRule::Invite => Some("Invite-only: ask a member to invite you."),
                };
                if let Some(why) = blocked {
                    card = card.push(text(why).size(13).color(color!(0xffc785)).center());
                }
                card.push(
                    row![
                        close,
                        choice(
                            join_label,
                            (ready && blocked.is_none()).then_some(Msg::JoinPreviewed),
                            JOIN,
                            JOIN_HOVER
                        )
                    ]
                    .spacing(12),
                )
            }
        };
        let card = if preview.refused {
            card.push(text(REFUSED).size(13).color(WARN).center())
        } else {
            card
        }
        .spacing(12)
        .align_x(Center);
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
