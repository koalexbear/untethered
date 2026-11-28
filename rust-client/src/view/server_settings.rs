//! Server Settings (server_settings.rs) on screen: Discord's layout, with
//! the tabs on the left, the page on the right, Escape to close, and the
//! unsaved-changes bar at the foot.
//!
//! Every slot of a page is emitted whether or not it shows anything, so a
//! note appearing never moves a text field (Iced keeps state by position).
use super::{
    ACTIVE, App, BLURPLE, BODY, BOLD, CHANNEL, CHAT, El, FLOAT, GREEN, HOVER, ICON, Icon, LINE,
    MUTED, Msg, RAIL, SIDE, TEXT, face, icon, icon_button, input_style, label, line_rule, plain,
    primary, scroll_style, surface, thin_scrollbar, with_tip,
};
use crate::{
    engine::space_settings::{PackRef, Picture as Kind, Usage},
    rooms::Room,
    server_settings::{Field, Input, Page, Tab, Target, picture_type, settings_pack_name},
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Length, Padding, Theme, color,
    widget::{
        Column, Space, button, column, container, image, opaque, row, scrollable, text, text_input,
        tooltip,
    },
};

fn msg(input: Input) -> Msg {
    Msg::ServerSettings(input)
}

fn caption(s: &str) -> El<'_> {
    text(s).size(12).color(CHANNEL).into()
}

/// A value this account can't change, where its field would be.
fn value_box(value: &str, empty: &'static str) -> El<'static> {
    let shown = if value.trim().is_empty() {
        text(empty).size(16).color(CHANNEL)
    } else {
        text(value.to_owned()).size(16).color(BODY)
    };
    container(shown)
        .padding(10)
        .width(Fill)
        .style(|_| container::Style {
            background: Some(RAIL.into()),
            border: Border {
                color: LINE,
                width: 1.,
                radius: 4.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn secondary(_: &Theme, s: button::Status) -> button::Style {
    button::Style {
        background: Some(
            match s {
                button::Status::Hovered | button::Status::Pressed => color!(0x6d6f78),
                button::Status::Disabled => color!(0x3f4147),
                button::Status::Active => color!(0x4e5058),
            }
            .into(),
        ),
        text_color: if s == button::Status::Disabled {
            CHANNEL
        } else {
            Color::WHITE
        },
        border: Border::default().rounded(4),
        ..Default::default()
    }
}

fn small_button<'a>(
    label: &'a str,
    style: fn(&Theme, button::Status) -> button::Style,
    on: Option<Msg>,
) -> El<'a> {
    button(text(label).size(14).font(BOLD))
        .padding([8, 16])
        .style(style)
        .on_press_maybe(on)
        .into()
}

fn heading(title: &str) -> El<'_> {
    text(title).size(20).font(BOLD).color(TEXT).into()
}

/// One choice of a segmented control.
fn segment(label: &'static str, chosen: bool, on: Option<Msg>) -> El<'static> {
    button(text(label).size(13).font(BOLD))
        .padding([4, 10])
        .on_press_maybe(on)
        .style(move |_, s| button::Style {
            background: Some(
                if chosen {
                    BLURPLE
                } else if s == button::Status::Hovered {
                    HOVER
                } else {
                    Color::TRANSPARENT
                }
                .into(),
            ),
            text_color: if chosen {
                Color::WHITE
            } else if s == button::Status::Disabled {
                CHANNEL
            } else {
                MUTED
            },
            border: Border::default().rounded(3),
            ..Default::default()
        })
        .into()
}

impl App {
    /// The page over the whole window, or None while it's closed.
    pub(super) fn server_settings_view(&self) -> Option<El<'_>> {
        let page = self.server_settings.page.as_ref()?;
        let space = self.live.list.get(&page.space).filter(|r| r.space)?;
        let mut nav = column![
            container(
                text(self.settings_value(Field::Name).to_uppercase())
                    .size(12)
                    .font(BOLD)
                    .color(CHANNEL)
                    .wrapping(text::Wrapping::None)
            )
            .padding([6, 10])
        ]
        .spacing(2)
        .width(218);
        for tab in [Tab::Overview, Tab::Emoji, Tab::Channels] {
            let active = page.tab == tab;
            nav = nav.push(
                button(text(tab.label()).size(16))
                    .width(Fill)
                    .padding([6, 10])
                    .on_press(msg(Input::Tab(tab)))
                    .style(move |_, s| button::Style {
                        background: Some(
                            if active {
                                ACTIVE
                            } else if s == button::Status::Hovered {
                                HOVER
                            } else {
                                Color::TRANSPARENT
                            }
                            .into(),
                        ),
                        text_color: if active { TEXT } else { MUTED },
                        border: Border::default().rounded(4),
                        ..Default::default()
                    }),
            );
        }
        let left = container(nav)
            .width(Length::FillPortion(1))
            .height(Fill)
            .align_x(iced::alignment::Horizontal::Right)
            .padding(Padding {
                top: 60.,
                right: 8.,
                bottom: 20.,
                left: 20.,
            })
            .style(|_| surface(SIDE, 0.));

        let body = match page.tab {
            Tab::Overview => self.overview(page, space),
            Tab::Emoji => self.emoji_tab(page),
            Tab::Channels => self.channels_tab(page),
        };
        // Fixed slots: the title, the read-only notice and the last note.
        let notice: El = if self.settings_read_only(page) {
            container(
                row![
                    icon(Icon::Lock, 16., MUTED),
                    text(
                        "You can view these settings. Changing them needs a role with more \
                         power in this server."
                    )
                    .size(14)
                    .color(MUTED)
                ]
                .spacing(8)
                .align_y(Center),
            )
            .padding(12)
            .width(Fill)
            .style(|_| surface(SIDE, 6.))
            .into()
        } else {
            Space::new().into()
        };
        let note: El = match (&page.note, page.load_failed) {
            (Some(n), _) => text(&n.text)
                .size(14)
                .color(if n.problem { color!(0xffc785) } else { GREEN })
                .into(),
            (None, true) => text("Couldn't load this server's settings. Showing what has synced.")
                .size(14)
                .color(color!(0xffc785))
                .into(),
            (None, false) => Space::new().into(),
        };
        let content = column![heading(page.tab.title()), notice, note, body]
            .spacing(20)
            .max_width(740);
        let close = column![
            with_tip(
                button(container(icon(Icon::Close, 18., ICON)).center(36))
                    .padding(0)
                    .on_press(msg(Input::Close))
                    .style(|_, s| button::Style {
                        background: (s == button::Status::Hovered).then(|| HOVER.into()),
                        border: Border {
                            color: if s == button::Status::Hovered {
                                BODY
                            } else {
                                ICON
                            },
                            width: 2.,
                            radius: 18.0.into(),
                        },
                        ..Default::default()
                    }),
                "Close",
                tooltip::Position::Bottom
            ),
            text("ESC").size(13).font(BOLD).color(CHANNEL)
        ]
        .spacing(8)
        .align_x(Center);
        let main = row![
            scrollable(container(content).padding(Padding {
                top: 60.,
                right: 20.,
                bottom: 40.,
                left: 40.,
            }))
            .direction(thin_scrollbar())
            .style(scroll_style(None))
            .width(Fill)
            .height(Fill),
            container(close).padding(Padding {
                top: 60.,
                right: 21.,
                ..Padding::ZERO
            })
        ]
        .height(Fill);
        let right = container(column![main, self.unsaved_bar()].height(Fill))
            .width(Length::FillPortion(2))
            .height(Fill)
            .style(|_| surface(CHAT, 0.));
        Some(opaque(
            container(row![left, right].height(Fill))
                .width(Fill)
                .height(Fill),
        ))
    }

    /// Nothing on this tab can be changed by this account.
    fn settings_read_only(&self, page: &Page) -> bool {
        let space = &page.space;
        match page.tab {
            Tab::Overview => ![
                "m.room.name",
                "m.room.topic",
                picture_type(Kind::Icon),
                picture_type(Kind::Banner),
            ]
            .iter()
            .any(|kind| self.may_set(space, kind)),
            Tab::Emoji => match self.settings_pack() {
                Some(pack) => !self.may_edit_pack(space, &pack),
                None => true,
            },
            Tab::Channels => !self.settings_channels(space).iter().any(|room| {
                self.may_set(room, "m.room.name") || self.may_set(room, "m.room.topic")
            }),
        }
    }

    /// Discord's "Careful — you have unsaved changes!" bar; an empty slot
    /// while there are none.
    fn unsaved_bar(&self) -> El<'_> {
        if !self.settings_unsaved() {
            return Space::new().into();
        }
        let saving = self.settings_saving();
        let save = button(
            text(if saving { "Saving…" } else { "Save Changes" })
                .size(14)
                .font(BOLD),
        )
        .padding([8, 16])
        .on_press_maybe((!saving && self.settings_valid()).then_some(msg(Input::Save)))
        .style(|_, s| button::Style {
            background: Some(
                match s {
                    button::Status::Hovered | button::Status::Pressed => color!(0x1a7f45),
                    button::Status::Disabled => color!(0x3f4147),
                    button::Status::Active => GREEN,
                }
                .into(),
            ),
            text_color: Color::WHITE,
            border: Border::default().rounded(4),
            ..Default::default()
        });
        let reset = button(text("Reset").size(14))
            .padding([8, 12])
            .style(plain(BODY, Color::WHITE))
            .on_press_maybe((!saving).then_some(msg(Input::Reset)));
        let words = if self.settings_valid() {
            "Careful — you have unsaved changes!"
        } else {
            "A name can't be empty."
        };
        container(
            container(
                row![text(words).size(14).color(BODY).width(Fill), reset, save]
                    .spacing(8)
                    .align_y(Center),
            )
            .padding([10, 16])
            .max_width(700)
            .style(|_| surface(FLOAT, 6.)),
        )
        .padding(Padding {
            top: 0.,
            right: 40.,
            bottom: 20.,
            left: 40.,
        })
        .width(Fill)
        .into()
    }

    fn overview<'a>(&'a self, page: &'a Page, space: &'a Room) -> El<'a> {
        let id = &page.space;
        let saving = self.settings_saving();
        let picking = self.settings_picking();
        let name = self.settings_value(Field::Name);
        let may_icon = self.may_set(id, picture_type(Kind::Icon));
        let may_banner = self.may_set(id, picture_type(Kind::Banner));
        let icon_url = self.settings_icon();
        let picture = self.avatars.get(icon_url);
        let ready = !saving && !picking;
        let mut icon_controls = column![].spacing(8).align_x(Center);
        if may_icon {
            icon_controls = icon_controls.push(small_button(
                "Upload Image",
                secondary,
                ready.then(|| msg(Input::Pick(Target::Picture(Kind::Icon)))),
            ));
            if icon_url.is_some() {
                icon_controls = icon_controls.push(
                    button(text("Remove").size(14))
                        .padding([2, 4])
                        .style(plain(MUTED, Color::WHITE))
                        .on_press_maybe(ready.then(|| msg(Input::RemovePicture(Kind::Icon)))),
                );
            }
        }
        let icon_column = column![face(&name, picture, 100.), icon_controls]
            .spacing(12)
            .align_x(Center)
            .width(140);
        let name_field: El = if self.may_set(id, "m.room.name") {
            text_input("Server name", &name)
                .id("server-name")
                .on_input_maybe((!saving).then_some(|v| msg(Input::Name(v))))
                .size(16)
                .padding(10)
                .style(input_style)
                .into()
        } else {
            value_box(&name, "No name")
        };
        let topic = self.settings_value(Field::Topic);
        let topic_field: El = if self.may_set(id, "m.room.topic") {
            text_input("What this server is about", &topic)
                .id("server-topic")
                .on_input_maybe((!saving).then_some(|v| msg(Input::Topic(v))))
                .size(16)
                .padding(10)
                .style(input_style)
                .into()
        } else {
            value_box(&topic, "No topic")
        };
        let fields = column![
            column![label("SERVER NAME"), name_field].spacing(8),
            column![
                label("TOPIC"),
                topic_field,
                caption("Every Matrix client shows this as the space's topic.")
            ]
            .spacing(8),
        ]
        .spacing(20)
        .width(Fill);
        let icon_hint = column![
            text("Server icon").size(16).font(BOLD).color(TEXT),
            caption(if may_icon {
                "We recommend an image of at least 512 × 512."
            } else {
                "The space's picture in every Matrix client."
            }),
        ]
        .spacing(4);

        // The banner: Untethered's own state event.
        let banner_url = page.snapshot.as_ref().and_then(|s| s.banner.as_deref());
        let preview: El = match banner_url.and_then(|u| self.server_settings.picture(u)) {
            Some(handle) => image(handle.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(iced::ContentFit::Cover)
                .border_radius(8.)
                .into(),
            None => container(
                text(if banner_url.is_some() {
                    "Loading…"
                } else {
                    "No banner"
                })
                .size(14)
                .color(CHANNEL),
            )
            .center(Fill)
            .into(),
        };
        let mut banner_controls = row![].spacing(12).align_y(Center);
        if may_banner {
            banner_controls = banner_controls.push(small_button(
                "Upload Banner",
                secondary,
                ready.then(|| msg(Input::Pick(Target::Picture(Kind::Banner)))),
            ));
            if banner_url.is_some() {
                banner_controls = banner_controls.push(
                    button(text("Remove").size(14))
                        .padding([2, 4])
                        .style(plain(MUTED, Color::WHITE))
                        .on_press_maybe(ready.then(|| msg(Input::RemovePicture(Kind::Banner)))),
                );
            }
        }
        let banner = column![
            label("SERVER BANNER BACKGROUND"),
            caption(
                "Untethered only: other Matrix clients don't show banners. It shows at the top \
                 of the channel list here."
            ),
            container(preview)
                .width(330)
                .height(186)
                .style(|_| surface(super::TRAY, 8.)),
            banner_controls,
        ]
        .spacing(8);
        let _ = space;
        column![
            row![
                icon_column,
                column![icon_hint, fields].spacing(20).width(Fill)
            ]
            .spacing(24),
            line_rule(LINE),
            banner,
        ]
        .spacing(24)
        .into()
    }

    fn emoji_tab<'a>(&'a self, page: &'a Page) -> El<'a> {
        let id = &page.space;
        let pack_saving = self.pack_saving();
        let picking = self.settings_picking();
        let target = self.settings_pack();
        let may = target
            .as_ref()
            .is_some_and(|pack| self.may_edit_pack(id, pack));
        let ready = !pack_saving && !picking;
        let mut out = column![
            text(
                "Custom emoji and stickers for everyone in this server, in the emoji and sticker \
                 pickers here and in Matrix clients that read image packs (Cinny, FluffyChat, \
                 Nheko). Other clients show the :shortcode:."
            )
            .size(14)
            .color(MUTED)
        ]
        .spacing(20);
        let Some(snapshot) = &page.snapshot else {
            out = out.push(caption(if page.load_failed {
                "Couldn't load the server's emoji."
            } else {
                "Loading…"
            }));
            return out.into();
        };
        let creatable = snapshot.packs.is_empty();
        if may && !creatable {
            let pack = target.clone().expect("a pack");
            out = out.push(
                column![
                    row![
                        small_button(
                            "Upload Emoji",
                            primary,
                            ready.then(|| msg(Input::Pick(Target::Image(
                                pack.clone(),
                                Usage::Emoticon
                            ))))
                        ),
                        small_button(
                            "Upload Sticker",
                            secondary,
                            ready.then(|| msg(Input::Pick(Target::Image(pack, Usage::Sticker))))
                        ),
                    ]
                    .spacing(12),
                    caption(
                        "PNG, JPEG, GIF or WebP, up to 1 MiB and 1024 × 1024. The file's name \
                         becomes its shortcode; click a name to change it."
                    ),
                ]
                .spacing(8),
            );
        }
        if creatable {
            out = out.push(
                column![
                    text("This server has no emoji yet.")
                        .size(16)
                        .font(BOLD)
                        .color(TEXT),
                    if may {
                        small_button(
                            "Create Emoji Pack",
                            primary,
                            ready.then_some(msg(Input::CreatePack)),
                        )
                    } else {
                        caption("Someone with permission to manage emoji can add a pack.")
                    }
                ]
                .spacing(12),
            );
            return out.into();
        }
        let several = snapshot.packs.len() > 1;
        for doc in &snapshot.packs {
            let pack = crate::server_settings::pack_images(id, doc);
            let may_pack = self.may_edit_pack(id, &doc.pack);
            let mut title = settings_pack_name(&pack, &self.settings_value(Field::Name));
            if !doc.pack.stable {
                title.push_str(" (Cinny's older format)");
            } else if several && !doc.pack.key.is_empty() {
                title = format!("{title} ({})", doc.pack.key);
            }
            let mut rows = column![
                row![
                    text(format!("{} — {}", title.to_uppercase(), pack.images.len()))
                        .size(12)
                        .font(BOLD)
                        .color(CHANNEL)
                        .width(Fill),
                ],
                row![
                    text("IMAGE").size(12).font(BOLD).color(CHANNEL).width(64),
                    text("NAME").size(12).font(BOLD).color(CHANNEL).width(Fill),
                    text("USE AS").size(12).font(BOLD).color(CHANNEL).width(190),
                    Space::new().width(32),
                ]
                .spacing(12),
                line_rule(LINE),
            ]
            .spacing(8);
            if pack.images.is_empty() {
                rows = rows.push(caption("No images yet."));
            }
            for img in &pack.images {
                rows = rows.push(self.emoji_row(page, &doc.pack, img, may_pack && ready));
            }
            out = out.push(rows);
        }
        out.into()
    }

    fn emoji_row<'a>(
        &'a self,
        page: &'a Page,
        pack: &PackRef,
        img: &crate::engine::packs::Image,
        editable: bool,
    ) -> El<'a> {
        let picture: El = match self.server_settings.picture(&img.url) {
            Some(handle) => image(handle.clone())
                .width(32)
                .height(32)
                .content_fit(iced::ContentFit::Contain)
                .into(),
            None => Space::new().width(32).height(32).into(),
        };
        let renaming = page
            .rename
            .as_ref()
            .filter(|r| &r.pack == pack && r.from == img.shortcode);
        let name: El = match renaming {
            Some(r) => text_input("shortcode", &r.input)
                .id(crate::server_settings::RENAME_FIELD)
                .on_input(|v| msg(Input::Shortcode(v)))
                .on_submit(msg(Input::SaveRename))
                .size(15)
                .padding([6, 8])
                .width(240)
                .style(input_style)
                .into(),
            None if editable => with_tip(
                button(text(format!(":{}:", img.shortcode)).size(15).color(BODY))
                    .padding([6, 8])
                    .on_press(msg(Input::Rename(pack.clone(), img.shortcode.clone())))
                    .style(|_, s| button::Style {
                        background: (s == button::Status::Hovered).then(|| RAIL.into()),
                        border: Border::default().rounded(4),
                        ..Default::default()
                    }),
                "Rename",
                tooltip::Position::Top,
            ),
            None => container(text(format!(":{}:", img.shortcode)).size(15).color(BODY))
                .padding([6, 8])
                .into(),
        };
        let usage = Usage::from_flags(img.emoticon, img.sticker);
        let choose = |u: Usage| {
            (editable && u != usage)
                .then(|| msg(Input::Usage(pack.clone(), img.shortcode.clone(), u)))
        };
        let usage_control: El = if editable {
            container(
                row![
                    segment("Emoji", usage == Usage::Emoticon, choose(Usage::Emoticon)),
                    segment("Sticker", usage == Usage::Sticker, choose(Usage::Sticker)),
                    segment("Both", usage == Usage::Both, choose(Usage::Both)),
                ]
                .spacing(2),
            )
            .padding(2)
            .style(|_| surface(RAIL, 4.))
            .into()
        } else {
            text(match usage {
                Usage::Emoticon => "Emoji",
                Usage::Sticker => "Sticker",
                Usage::Both => "Emoji and sticker",
            })
            .size(14)
            .color(MUTED)
            .into()
        };
        let delete: El = if editable {
            icon_button(
                Icon::Close,
                "Delete",
                false,
                Some(msg(Input::Delete(pack.clone(), img.shortcode.clone()))),
                tooltip::Position::Left,
            )
        } else {
            Space::new().width(32).into()
        };
        row![
            container(picture).width(64),
            container(name).width(Fill),
            container(usage_control).width(190),
            container(delete).width(32),
        ]
        .spacing(12)
        .align_y(Center)
        .into()
    }

    fn channels_tab<'a>(&'a self, page: &'a Page) -> El<'a> {
        let id = &page.space;
        let saving = self.settings_saving();
        let reorder: El = if self.may_arrange(id) {
            small_button(
                "Reorder in the Channel List",
                secondary,
                Some(msg(Input::Reorder)),
            )
        } else {
            Space::new().into()
        };
        let mut out = column![
            column![
                text(
                    "Rename channels and set their topics. To reorder channels and categories, \
                     drag them in the channel list."
                )
                .size(14)
                .color(MUTED),
                reorder
            ]
            .spacing(12)
        ]
        .spacing(20);
        let store = self.rooms();
        let mut groups: Vec<(Option<&str>, Vec<&str>)> = store
            .groups
            .get(id)
            .into_iter()
            .flatten()
            .map(|g| {
                let title = g
                    .category
                    .then(|| store.list.get(&g.space).map(|r| r.name.as_str()))
                    .flatten();
                (title, g.channels.iter().map(|c| c.room.as_str()).collect())
            })
            .collect();
        if groups.iter().all(|(_, rooms)| rooms.is_empty()) {
            groups = vec![(None, self.settings_channels(id))];
        }
        for (title, rooms) in groups {
            if rooms.is_empty() {
                continue;
            }
            let mut list = Column::new().spacing(8);
            list = list.push(label_owned(
                title.map_or_else(|| "CHANNELS".to_owned(), str::to_uppercase),
            ));
            for room in rooms {
                let Some(r) = store.list.get(room) else {
                    continue;
                };
                list = list.push(self.channel_card(room, r, saving));
            }
            out = out.push(list);
        }
        out.into()
    }

    fn channel_card<'a>(&'a self, room: &'a str, r: &'a Room, saving: bool) -> El<'a> {
        let name = self.channel_value(room, Field::Name);
        let topic = self.channel_value(room, Field::Topic);
        let name_field: El = if self.may_set(room, "m.room.name") {
            text_input("channel-name", &name)
                .on_input_maybe(
                    (!saving).then_some(move |v| msg(Input::ChannelName(room.to_owned(), v))),
                )
                .size(15)
                .padding(8)
                .style(input_style)
                .into()
        } else {
            value_box(&name, "No name")
        };
        let topic_field: El = if self.may_set(room, "m.room.topic") {
            text_input("Let everyone know how to use this channel", &topic)
                .on_input_maybe(
                    (!saving).then_some(move |v| msg(Input::ChannelTopic(room.to_owned(), v))),
                )
                .size(15)
                .padding(8)
                .style(input_style)
                .into()
        } else {
            value_box(&topic, "No topic")
        };
        container(
            row![
                container(icon(
                    if r.voice { Icon::Speaker } else { Icon::Hash },
                    20.,
                    ICON
                ))
                .padding(Padding {
                    top: 26.,
                    ..Padding::ZERO
                }),
                column![
                    column![label("CHANNEL NAME"), name_field].spacing(6),
                    column![label("CHANNEL TOPIC"), topic_field].spacing(6),
                ]
                .spacing(12)
                .width(Fill)
            ]
            .spacing(12),
        )
        .padding(16)
        .width(Fill)
        .style(|_| surface(SIDE, 8.))
        .into()
    }
}

fn label_owned(s: String) -> El<'static> {
    text(s).font(BOLD).size(12).color(CHANNEL).into()
}

impl App {
    /// The channel list's header with the open space's banner behind it,
    /// as Discord shows a server's banner; the header alone without one.
    pub(super) fn banner_head<'a>(&'a self, head: El<'a>) -> El<'a> {
        let Some(banner) = self.space_banner() else {
            return head;
        };
        iced::widget::stack![
            image(banner.clone())
                .width(Fill)
                .height(BANNER_HEIGHT)
                .content_fit(iced::ContentFit::Cover),
            container(head)
                .width(Fill)
                .height(48)
                .style(|_| container::Style {
                    background: Some(Color::from_rgba8(0, 0, 0, 0.45).into()),
                    ..Default::default()
                }),
        ]
        .height(BANNER_HEIGHT)
        .into()
    }
}

/// Discord's banner, 240 × 135 at the sidebar's width.
const BANNER_HEIGHT: f32 = 135.;
