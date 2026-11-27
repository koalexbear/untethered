//! The quick switcher (switcher.rs) on screen, shaped like Discord's: a
//! large search field, the rooms it matches with the highlighted one marked,
//! and a tip about narrowing the list.
use super::{
    ACTIVE, App, BOLD, CHANNEL, El, HOVER, Icon, MUTED, Msg, SIDE, TEXT, badge, face, icon,
    input_style, surface,
};
use crate::switcher::{Hit, Input, Kind, SEARCH};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Padding,
    widget::{
        Column, Space, button, column, container, mouse_area, opaque, row, stack, text, text_input,
    },
};

impl App {
    pub(super) fn switcher_view(&self) -> Option<El<'_>> {
        let switcher = self.switcher.as_ref()?;
        let search = text_input("Where would you like to go?", &switcher.query)
            .id(SEARCH)
            .size(20)
            .padding([12, 14])
            .style(input_style)
            .on_input(|q| Msg::Switcher(Input::Query(q)))
            .on_submit(Msg::Switcher(Input::Go(None)));
        let hits = self.switcher_hits();
        let mut list = Column::new().spacing(2);
        if hits.is_empty() {
            list = list.push(
                container(
                    text("No rooms, DMs or spaces by that name.")
                        .size(14)
                        .color(CHANNEL),
                )
                .padding([12, 8]),
            );
        }
        for (i, hit) in hits.into_iter().enumerate() {
            list = list.push(self.switcher_row(hit, i == switcher.chosen));
        }
        let tip = text("Start with # for text channels, ! for voice, @ for DMs or * for spaces.")
            .size(12)
            .color(CHANNEL);
        let body = column![search, list, tip].spacing(12);
        let dialog = container(body)
            .padding(16)
            .width(570)
            .style(|_| surface(SIDE, 8.));
        let backdrop = mouse_area(container(Space::new()).width(Fill).height(Fill).style(|_| {
            container::Style {
                background: Some(Color::from_rgba8(0, 0, 0, 0.7).into()),
                ..Default::default()
            }
        }))
        .on_press(Msg::Switcher(Input::Close));
        // A little above the middle, as Discord places it.
        let placed = container(dialog).center_x(Fill).padding(Padding {
            top: 120.,
            ..Padding::ZERO
        });
        Some(stack![opaque(backdrop), placed].into())
    }

    fn switcher_row(&self, hit: Hit<'_>, chosen: bool) -> El<'_> {
        let r = self.rooms().list.get(hit.id);
        let picture = r.and_then(|r| self.avatars.get(r.avatar.as_deref()));
        let leading: El = match hit.kind {
            Kind::Dm | Kind::Space => face(hit.name, picture, 20.),
            Kind::Voice => icon(Icon::Speaker, 18., MUTED),
            Kind::Channel => icon(Icon::Hash, 18., MUTED),
        };
        let mut line = row![
            leading,
            text(hit.name.to_owned())
                .size(16)
                .font_maybe(hit.unread.then_some(BOLD))
                .color(if hit.unread || chosen { TEXT } else { MUTED })
                .wrapping(text::Wrapping::None)
                .width(Fill),
        ]
        .spacing(10)
        .align_y(Center);
        if hit.mentions > 0 {
            line = line.push(badge(hit.mentions));
        }
        if let Some(place) = hit.place {
            line = line.push(
                text(place.to_uppercase())
                    .size(12)
                    .font(BOLD)
                    .color(CHANNEL),
            );
        }
        button(line)
            .width(Fill)
            .padding([8, 10])
            .on_press(Msg::Switcher(Input::Go(Some(hit.id.to_owned()))))
            .style(move |_, s| button::Style {
                background: if chosen {
                    Some(ACTIVE.into())
                } else {
                    (s == button::Status::Hovered).then(|| HOVER.into())
                },
                border: Border::default().rounded(4),
                ..Default::default()
            })
            .into()
    }
}
