//! Custom emoji and stickers (emotes.rs) on screen: their pictures, the
//! packs' sections in the emoji pickers, and the composer's sticker picker.
use super::{
    ACTIVE, App, BOLD, CHANNEL, El, FLOAT, Icon, LINE, Msg, icon_button, scroll_style, surface,
    thin_scrollbar, with_tip,
};
use crate::{
    emotes::{FOUND, Input},
    engine::packs::{Image, Pack},
};
use iced::{
    Alignment::Center,
    Border, Fill, Length,
    widget::{
        Column, Row, Space, button, checkbox, column, container, image, row, scrollable, text,
        tooltip,
    },
};

impl App {
    /// A custom emoji's picture at `size`, or room for it until it's here.
    pub(super) fn emote_image<'a>(&'a self, url: &str, size: f32) -> El<'a> {
        match self.emotes.get(url) {
            Some(picture) => image(picture.clone())
                .width(size)
                .height(size)
                .content_fit(iced::ContentFit::Contain)
                .into(),
            None => Space::new().width(size).height(size).into(),
        }
    }

    /// The image packs' emoji for a picker in `room`: by pack, each with
    /// Use everywhere for a room's pack, or what `query` finds. `pick` is
    /// what a click does and `active` marks one, as our own reactions are.
    /// None when there's nothing to show.
    pub(super) fn emote_sections<'a>(
        &'a self,
        room: &str,
        query: &str,
        pick: impl Fn(&Image) -> Option<Msg>,
        active: impl Fn(&Image) -> bool,
    ) -> Option<El<'a>> {
        let cell = |image: &'a Image| {
            let on = active(image);
            with_tip(
                button(self.emote_image(&image.url, 28.))
                    .padding(4)
                    .on_press_maybe(pick(image))
                    .style(move |_, s| button::Style {
                        background: (on || s == button::Status::Hovered).then(|| ACTIVE.into()),
                        border: Border::default().rounded(6),
                        ..Default::default()
                    }),
                format!(":{}:", image.shortcode),
                tooltip::Position::Top,
            )
        };
        let query = query.trim();
        let body: El = if !query.is_empty() {
            let found = self.emote_search(room, query, FOUND);
            if found.is_empty() {
                return None;
            }
            Row::with_children(found.into_iter().map(|(_, i)| cell(i)))
                .spacing(2)
                .wrap()
                .into()
        } else {
            let mut sections = Column::new().spacing(8);
            let mut seen = std::collections::BTreeSet::new();
            let mut any = false;
            for pack in self.room_packs(room) {
                let images: Vec<&Image> = pack
                    .images
                    .iter()
                    .filter(|i| i.emoticon && seen.insert(i.shortcode.as_str()))
                    .collect();
                if images.is_empty() {
                    continue;
                }
                any = true;
                sections = sections.push(
                    column![
                        self.pack_header(pack),
                        Row::with_children(images.into_iter().map(cell))
                            .spacing(2)
                            .wrap()
                    ]
                    .spacing(4),
                );
            }
            if !any {
                return None;
            }
            scrollable(sections)
                .direction(thin_scrollbar())
                .style(scroll_style(None))
                .height(Length::Shrink)
                .into()
        };
        Some(container(body).max_height(240).into())
    }

    /// A pack's name, and for a room's pack whether you use it everywhere.
    fn pack_header<'a>(&'a self, pack: &'a Pack) -> El<'a> {
        let mut header = row![
            text(pack.name.to_uppercase())
                .size(12)
                .font(BOLD)
                .color(CHANNEL)
                .width(Fill)
        ]
        .spacing(8)
        .align_y(Center);
        if let Some(room) = &pack.room {
            let on = self
                .emotes
                .packs
                .everywhere
                .contains(&(room.clone(), pack.key.clone()));
            let (room, key) = (room.clone(), pack.key.clone());
            header = header.push(
                checkbox(on)
                    .label("Use everywhere")
                    .size(14)
                    .text_size(12)
                    .on_toggle_maybe(
                        (self.verified && self.connected && !self.pack_pending()).then_some(
                            move |on| {
                                Msg::Emote(Input::Everywhere {
                                    room: room.clone(),
                                    key: key.clone(),
                                    on,
                                })
                            },
                        ),
                    ),
            );
        }
        header.into()
    }

    /// The composer's sticker button, with its picker over the composer.
    pub(super) fn sticker_button(&self, restricted: bool) -> El<'_> {
        let open = self.emotes.stickers;
        super::popover::popover(
            icon_button(
                Icon::Sticker,
                "Stickers",
                open,
                (!restricted && self.can_send_sticker())
                    .then_some(Msg::Emote(Input::ToggleStickers)),
                tooltip::Position::Top,
            ),
            open.then(|| self.sticker_picker()),
            super::popover::Place::Below,
            Msg::Emote(Input::CloseStickers),
        )
    }

    /// Stickers by pack; a click sends one.
    fn sticker_picker(&self) -> El<'_> {
        let packs = self.room_stickers(&self.selected);
        let body: El = if packs.is_empty() {
            text(
                "No stickers here yet. They come from image packs in this room, its \
                 space, your account, or packs you use everywhere.",
            )
            .size(13)
            .color(CHANNEL)
            .into()
        } else {
            let mut sections = Column::new().spacing(10);
            for (pack, images) in packs {
                sections = sections.push(
                    column![
                        self.pack_header(pack),
                        Row::with_children(images.into_iter().map(|image| {
                            let hint = image
                                .body
                                .clone()
                                .filter(|b| !b.trim().is_empty())
                                .unwrap_or_else(|| format!(":{}:", image.shortcode));
                            with_tip(
                                button(self.emote_image(&image.url, 72.))
                                    .padding(4)
                                    .on_press(Msg::Emote(Input::Sticker(image.clone())))
                                    .style(|_, s| button::Style {
                                        background: (s == button::Status::Hovered)
                                            .then(|| ACTIVE.into()),
                                        border: Border::default().rounded(8),
                                        ..Default::default()
                                    }),
                                hint,
                                tooltip::Position::Top,
                            )
                        }))
                        .spacing(4)
                        .wrap()
                    ]
                    .spacing(6),
                );
            }
            scrollable(sections)
                .direction(thin_scrollbar())
                .style(scroll_style(None))
                .height(Length::Shrink)
                .into()
        };
        container(column![text("STICKERS").size(12).font(BOLD).color(CHANNEL), body].spacing(8))
            .padding(10)
            .width(360)
            .max_height(360)
            .style(|_| iced::widget::container::Style {
                border: Border {
                    color: LINE,
                    width: 1.,
                    radius: 8.0.into(),
                },
                ..surface(FLOAT, 8.)
            })
            .into()
    }
}
