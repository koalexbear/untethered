//! Forward To (forward.rs) in a modal shaped like Discord's: a search of
//! your channels and DMs with the rooms picked in it, the message being
//! forwarded, an optional message of your own, and Send.
use super::{
    App, BODY, BOLD, CHANNEL, CHAT, El, HOVER, Icon, MUTED, Msg, RAIL, SIDE, TEXT, face, icon,
    icon_button, input_style, plain, primary, scroll_style, surface, thin_scrollbar,
};
use crate::forward::{Input, MAX_ROOMS, SEARCH, Target};
use iced::{
    Alignment::Center,
    Border, Color, Fill, color,
    widget::{
        Column, Space, button, checkbox, column, container, mouse_area, opaque, row, scrollable,
        stack, text, text_input, tooltip,
    },
};

const WARN: Color = color!(0xffc785);

impl App {
    pub(super) fn forward_dialog(&self) -> Option<El<'_>> {
        let form = self.forwarding.as_ref()?;
        let busy = self.forward_sending();
        let close = (!busy).then_some(Msg::Forward(Input::Close));
        let left = MAX_ROOMS - form.picked.len();
        let subtitle = match left {
            0 => "That's as many places as it goes to at once.".to_owned(),
            MAX_ROOMS => "Select where you want to share this message.".to_owned(),
            1 => "You can pick 1 more place.".to_owned(),
            n => format!("You can pick {n} more places."),
        };
        let header = row![
            column![
                text("Forward To").size(20).font(BOLD).color(TEXT),
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

        // The rooms picked sit in the search box, as New Message's people do.
        let mut field = row![].spacing(4).align_y(Center);
        for id in &form.picked {
            field = field.push(
                button(
                    row![
                        text(self.place_name(id)).size(13).color(TEXT),
                        text("×").size(14).color(MUTED)
                    ]
                    .spacing(6)
                    .align_y(Center),
                )
                .padding([3, 8])
                .on_press_maybe((!busy).then(|| Msg::Forward(Input::Toggle(id.clone()))))
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
        field = field.push(
            text_input("Search", &form.query)
                .id(SEARCH)
                .size(15)
                .padding([6, 4])
                .style(|theme, status| text_input::Style {
                    background: Color::TRANSPARENT.into(),
                    border: Border::default(),
                    ..input_style(theme, status)
                })
                .on_input(|q| Msg::Forward(Input::Query(q)))
                .on_submit(Msg::Forward(Input::Enter)),
        );
        let field = container(field.wrap())
            .padding([4, 8])
            .width(Fill)
            .style(|_| surface(RAIL, 4.));

        let targets = self.forward_targets();
        let mut list = Column::new().spacing(2);
        if targets.is_empty() {
            list = list.push(
                container(
                    text("No channels or DMs by that name.")
                        .size(14)
                        .color(CHANNEL),
                )
                .padding([8, 8]),
            );
        }
        for t in targets {
            let picked = form.picked.iter().any(|p| p == t.id);
            list = list.push(self.target_row(t, picked, busy));
        }
        let list = scrollable(list)
            .direction(thin_scrollbar())
            .style(scroll_style(None))
            .height(iced::Length::Fixed(240.))
            .width(Fill);

        let note = text_input("Add an optional message…", &form.note)
            .size(15)
            .padding([10, 12])
            .style(input_style)
            .on_input_maybe((!busy).then_some(|n| Msg::Forward(Input::Note(n))))
            .on_submit(Msg::Forward(Input::Send));

        let mut body = column![header, field, list, self.forward_preview(), note].spacing(14);
        let exposed = self.forward_exposed();
        if !exposed.is_empty() {
            let (names, verb, whose) = match exposed.as_slice() {
                [one] => (one.clone(), "isn't", "its server"),
                many => (crate::forward::listed(many), "aren't", "their servers"),
            };
            body = body.push(
                text(format!(
                    "{names} {verb} end-to-end encrypted, so {whose} can read what you forward there."
                ))
                .size(13)
                .color(WARN),
            );
        }
        if let Some(problem) = &form.problem {
            body = body.push(text(problem.clone()).size(13).color(WARN));
        }

        let send = (self.connected && !busy && !form.picked.is_empty())
            .then_some(Msg::Forward(Input::Send));
        let send = button(
            text(if busy { "Sending…" } else { "Send" })
                .size(14)
                .font(BOLD),
        )
        .padding([10, 18])
        .style(primary)
        .on_press_maybe(send);
        let cancel = button(text("Cancel").size(14))
            .padding([10, 16])
            .style(plain(BODY, Color::WHITE))
            .on_press_maybe(close.clone());
        let footer = container(
            row![Space::new().width(Fill), cancel, send]
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
        let dialog = container(column![container(body).padding([20, 16]), footer])
            .width(500)
            .style(|_| surface(CHAT, 8.));
        let backdrop = mouse_area(container(Space::new()).width(Fill).height(Fill).style(|_| {
            container::Style {
                background: Some(Color::from_rgba8(0, 0, 0, 0.7).into()),
                ..Default::default()
            }
        }));
        let backdrop = match close {
            Some(close) => backdrop.on_press(close),
            None => backdrop,
        };
        Some(stack![opaque(backdrop), container(dialog).center(Fill)].into())
    }

    fn target_row(&self, t: Target<'_>, picked: bool, busy: bool) -> El<'_> {
        let toggle = (!busy).then(|| Msg::Forward(Input::Toggle(t.id.to_owned())));
        let r = self.rooms().list.get(t.id);
        let leading: El = if t.dm {
            face(
                t.name,
                r.and_then(|r| self.avatars.get(r.avatar.as_deref())),
                24.,
            )
        } else {
            container(icon(Icon::Hash, 18., MUTED)).center(24).into()
        };
        let mut line = row![
            leading,
            text(t.name.to_owned())
                .size(15)
                .color(if picked { TEXT } else { BODY })
                .wrapping(text::Wrapping::None)
                .width(Fill),
        ]
        .spacing(10)
        .align_y(Center);
        if let Some(place) = t.place {
            line = line.push(
                text(place.to_uppercase())
                    .size(12)
                    .font(BOLD)
                    .color(CHANNEL)
                    .wrapping(text::Wrapping::None),
            );
        }
        line = line.push(
            checkbox(picked)
                .size(18)
                .on_toggle_maybe(toggle.clone().map(|m| move |_| m.clone())),
        );
        button(line)
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

    /// The message being forwarded, as a quote: who sent it and its start,
    /// or the attachment's name.
    fn forward_preview(&self) -> El<'_> {
        let room = self
            .forwarding
            .as_ref()
            .and_then(|f| self.rooms().list.get(&f.room));
        let Some((r, post)) = room.zip(self.forward_post()) else {
            return Space::new().into();
        };
        let who = r.display(&post.sender).to_owned();
        let mut quote = column![
            row![
                face(&who, self.user_picture(r, &post.sender), 16.),
                text(who.clone()).size(13).font(BOLD).color(TEXT)
            ]
            .spacing(6)
            .align_y(Center)
        ]
        .spacing(4);
        let excerpt = post.quote().map(|q| q.excerpt).unwrap_or_default();
        if let Some(media) = &post.media {
            quote = quote.push(
                row![
                    icon(Icon::File, 16., MUTED),
                    text(media.name.clone())
                        .size(14)
                        .color(BODY)
                        .wrapping(text::Wrapping::None)
                ]
                .spacing(6)
                .align_y(Center),
            );
            if !excerpt.is_empty() && excerpt != media.name {
                quote = quote.push(text(excerpt).size(14).color(BODY));
            }
        } else {
            quote = quote.push(text(excerpt).size(14).color(BODY));
        }
        row![
            container(Space::new())
                .width(4)
                .height(iced::Length::Fixed(40.))
                .style(|_| surface(color!(0x4e5058), 2.)),
            container(quote).clip(true).width(Fill)
        ]
        .spacing(10)
        .align_y(Center)
        .into()
    }
}
