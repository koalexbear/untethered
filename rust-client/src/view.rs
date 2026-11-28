use super::{App, Msg, Request, Section, rooms::Room};
mod animated;
mod autocomplete;
#[cfg(target_os = "linux")]
mod call_stage;
mod channel_order;
mod community;
mod context_menu;
mod dms;
mod drag;
mod emoji_picker;
mod emotes;
mod explore;
mod forward;
mod gallery;
mod hover;
mod icons;
mod invites;
mod joining;
mod link_preview;
#[cfg(target_os = "linux")]
mod players;
mod polls;
mod switcher;
mod zoom;
pub(crate) use link_preview::image_source as link_preview_image;
pub(crate) mod discussions;
mod mentions;
mod names;
mod popover;
mod presence;
mod profile;
mod reaction_bar;
mod rich;
mod room_people;
mod room_search;
mod sdk_account;
mod selectable;
mod server_rail;
mod server_settings;
mod shortcuts;
#[cfg(target_os = "linux")]
mod sounds;
#[cfg(not(target_arch = "wasm32"))]
mod tray;
mod typing;
mod upgrades;
mod viewer;
#[cfg(target_os = "linux")]
mod voice_panel;
#[cfg(target_os = "linux")]
mod voice_settings;
use iced::{
    Alignment::Center,
    Border, Color, Element, Fill, Font, Length, Padding, Theme, border, color, font, keyboard,
    widget::{
        Column, Row, Space, button, column, container, image, mouse_area, opaque, row, rule,
        scrollable, stack, text, text_editor, text_input, tooltip,
    },
};
use icons::{ICON, Icon, icon, icon_button, with_tip};

const RAIL: Color = color!(0x1e1f22);
const SIDE: Color = color!(0x2b2d31);
const CHAT: Color = color!(0x313338);
const TRAY: Color = color!(0x232428);
const HOVER: Color = color!(0x35373c);
const ACTIVE: Color = color!(0x404249);
const BLURPLE: Color = color!(0x5865f2);
const TEXT: Color = color!(0xf2f3f5);
const MUTED: Color = color!(0xb5bac1);
/// Message body text.
const BODY: Color = color!(0xdbdee1);
/// Read channel names, timestamps and secondary labels.
const CHANNEL: Color = color!(0x949ba4);
/// Tooltips, menus and other floating surfaces.
const FLOAT: Color = color!(0x111214);
const FIELD: Color = color!(0x383a40);
const LINE: Color = color!(0x3f4147);
const RED: Color = color!(0xf23f43);
const GREEN: Color = color!(0x23a55a);
const LINK: Color = color!(0x00a8fc);
const ROW_HOVER: Color = color!(0x2e3035);
const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::with_name("Liberation Sans")
};
type El<'a> = Element<'a, Msg>;

fn surface(c: Color, r: f32) -> container::Style {
    container::Style {
        background: Some(c.into()),
        border: Border::default().rounded(r),
        ..Default::default()
    }
}
fn line_rule<'a>(color: Color) -> El<'a> {
    rule::horizontal(1)
        .style(move |_| rule::Style {
            color,
            radius: 0.0.into(),
            fill_mode: rule::FillMode::Full,
            snap: true,
        })
        .into()
}
/// A red count badge, as on mentions in the server rail and channel list.
fn badge(count: u64) -> El<'static> {
    let label = if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    };
    container(text(label).size(12).font(BOLD).color(Color::WHITE))
        .padding([1, 5])
        .center_x(Length::Shrink)
        .style(|_| surface(RED, 8.))
        .into()
}
/// The white unread or selection marker on the far left of a rail or list.
fn pill(height: f32) -> El<'static> {
    container(Space::new())
        .width(4)
        .height(height)
        .style(|_| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border::default().rounded(border::right(4)),
            ..Default::default()
        })
        .into()
}
fn thin_scrollbar() -> scrollable::Direction {
    scrollable::Direction::Vertical(
        scrollable::Scrollbar::new()
            .width(4)
            .scroller_width(4)
            .margin(2),
    )
}
fn scroll_style(track: Option<Color>) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style {
    move |theme, status| {
        let mut style = scrollable::default(theme, status);
        let rail = scrollable::Rail {
            background: track.map(Into::into),
            border: Border::default().rounded(4),
            scroller: scrollable::Scroller {
                background: RAIL.into(),
                border: Border::default().rounded(4),
            },
        };
        style.vertical_rail = rail;
        style.horizontal_rail = rail;
        style
    }
}
/// Buttons that look like links or plain text until hovered.
fn plain(color: Color, hover: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| button::Style {
        background: None,
        text_color: if s == button::Status::Hovered {
            hover
        } else {
            color
        },
        ..Default::default()
    }
}
#[cfg_attr(
    not(any(target_os = "linux", target_arch = "wasm32")),
    allow(dead_code)
)]
fn nav(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| button::Style {
        background: Some(
            (if active {
                ACTIVE
            } else if s == button::Status::Hovered {
                HOVER
            } else {
                Color::TRANSPARENT
            })
            .into(),
        ),
        text_color: if active { TEXT } else { MUTED },
        border: Border::default().rounded(5),
        ..Default::default()
    }
}
/// A profile picture, or the name's initials on a colored disc while there
/// is none (or it hasn't loaded).
fn face(name: &str, picture: Option<&iced::widget::image::Handle>, size: f32) -> El<'static> {
    if let Some(picture) = picture {
        return image(picture.clone())
            .width(size)
            .height(size)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(size / 2.)
            .into();
    }
    let colors = [
        color!(0x5865a1),
        color!(0x337a79),
        color!(0x95613d),
        color!(0x865b89),
        color!(0x536bab),
    ];
    let c = colors[name.bytes().fold(0usize, |a, b| a + b as usize) % colors.len()];
    let initials = name
        .split_whitespace()
        .take(2)
        .filter_map(|s| s.chars().next())
        .collect::<String>()
        .to_uppercase();
    container(
        text(initials)
            .size(size * 0.42)
            .font(BOLD)
            .color(Color::WHITE),
    )
    .center(size)
    .style(move |_| surface(c, size / 2.))
    .into()
}
fn label(s: &str) -> El<'_> {
    text(s).font(BOLD).size(12).color(CHANNEL).into()
}
fn label_owned(s: String) -> El<'static> {
    text(s).font(BOLD).size(12).color(CHANNEL).into()
}
/// A member-list row: avatar, name and an optional status line.
fn member_row<'a>(
    name: &'a str,
    picture: Option<&iced::widget::image::Handle>,
    status: Option<&'a str>,
    action: Option<Msg>,
) -> El<'a> {
    let mut who = column![text(name).size(15).wrapping(text::Wrapping::None)];
    if let Some(s) = status {
        who = who.push(
            text(s)
                .size(12)
                .color(CHANNEL)
                .wrapping(text::Wrapping::None),
        );
    }
    button(
        row![face(name, picture, 32.), who]
            .spacing(12)
            .align_y(Center),
    )
    .width(Fill)
    .padding([5, 8])
    .on_press_maybe(action)
    .style(|_, s| button::Style {
        background: (s == button::Status::Hovered).then(|| HOVER.into()),
        text_color: if s == button::Status::Hovered {
            BODY
        } else {
            CHANNEL
        },
        border: Border::default().rounded(4),
        ..Default::default()
    })
    .into()
}
fn field_style(_: &Theme, status: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: FIELD.into(),
        border: Border::default().rounded(8),
        placeholder: color!(0x6d6f78),
        value: if matches!(status, text_editor::Status::Disabled) {
            CHANNEL
        } else {
            BODY
        },
        selection: color!(0x5865f2, 0.45),
    }
}
fn composer_editor<'a>(
    content: &'a text_editor::Content,
    id: &'static str,
    hint: String,
    on_action: Option<fn(text_editor::Action) -> Msg>,
    key_binding: impl Fn(text_editor::KeyPress) -> Option<text_editor::Binding<Msg>> + 'a,
) -> El<'a> {
    let editor = text_editor(content)
        .id(id)
        .placeholder(hint)
        .key_binding(key_binding)
        .padding([11, 0])
        .size(16)
        .height(Length::Shrink)
        .min_height(22)
        .max_height(220)
        .style(|theme, status| text_editor::Style {
            background: Color::TRANSPARENT.into(),
            border: Border::default(),
            ..field_style(theme, status)
        });
    match on_action {
        Some(action) => editor.on_action(action).into(),
        None => editor.into(),
    }
}

fn composer_send_key(press: &text_editor::KeyPress) -> bool {
    press.key == keyboard::Key::Named(keyboard::key::Named::Enter)
        && press.modifiers.is_empty()
        && matches!(press.status, text_editor::Status::Focused { .. })
}

fn input_style(_: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: RAIL.into(),
        border: Border::default().rounded(4),
        placeholder: color!(0x6d6f78),
        value: if matches!(status, text_input::Status::Disabled) {
            CHANNEL
        } else {
            BODY
        },
        selection: color!(0x5865f2, 0.45),
        icon: CHANNEL,
    }
}
/// A settings card shown between the channel header and the timeline.
fn card<'a>(content: impl Into<El<'a>>) -> El<'a> {
    container(
        container(content)
            .padding([12, 16])
            .width(Fill)
            .style(|_| container::Style {
                border: Border {
                    color: color!(0x26272b),
                    width: 1.,
                    radius: 8.0.into(),
                },
                ..surface(SIDE, 8.)
            }),
    )
    .padding(Padding {
        top: 8.,
        left: 16.,
        right: 16.,
        bottom: 0.,
    })
    .into()
}
fn panel_title<'a>(glyph: Icon, title: String, close: Msg) -> El<'a> {
    row![
        icon(glyph, 18., ICON),
        text(title).size(15).font(BOLD).color(TEXT).width(Fill),
        icon_button(
            Icon::Close,
            "Close",
            false,
            Some(close),
            tooltip::Position::Left
        )
    ]
    .spacing(8)
    .align_y(Center)
    .into()
}
/// Discord's blurple primary button.
fn primary(_: &Theme, s: button::Status) -> button::Style {
    button::Style {
        background: Some(
            match s {
                button::Status::Hovered | button::Status::Pressed => color!(0x4752c4),
                button::Status::Disabled => color!(0x3c428a),
                button::Status::Active => BLURPLE,
            }
            .into(),
        ),
        text_color: if s == button::Status::Disabled {
            color!(0xa3a6c8)
        } else {
            Color::WHITE
        },
        border: Border::default().rounded(4),
        ..Default::default()
    }
}
/// One control in a message's hover toolbar.
fn tool<'a>(glyph: Icon, hint: &'a str, active: bool, action: Option<Msg>) -> El<'a> {
    let enabled = action.is_some();
    let color = if !enabled {
        icons::ICON_OFF
    } else if active {
        TEXT
    } else {
        ICON
    };
    let control = button(icons::hover_icon(
        glyph,
        20.,
        32.,
        color,
        if enabled { TEXT } else { color },
    ))
    .padding(0)
    .on_press_maybe(action)
    .style(move |_, s| button::Style {
        background: (active || s == button::Status::Hovered).then(|| color!(0x393c41).into()),
        border: Border::default().rounded(4),
        ..Default::default()
    });
    with_tip(control, hint, tooltip::Position::Top)
}
fn toolbar<'a>(tools: impl Into<El<'a>>) -> El<'a> {
    container(tools)
        .padding(2)
        .style(|_| container::Style {
            background: Some(CHAT.into()),
            // No shadow: the overlay's damage region would not cover it.
            border: Border {
                color: color!(0x232428),
                width: 1.,
                radius: 6.0.into(),
            },
            ..Default::default()
        })
        .into()
}
fn attachment_card<'a>(room: &str, a: &'a crate::media::Attachment) -> El<'a> {
    let name = &a.source.name;
    let ext = name
        .rsplit_once('.')
        .map(|(_, s)| s)
        .filter(|s| !s.is_empty() && s.len() <= 6 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or("FILE")
        .to_ascii_uppercase();
    let tint = match ext.as_str() {
        "PDF" => color!(0xf2969d),
        "ZIP" | "7Z" | "GZ" => color!(0xe7be7c),
        "WAV" | "MP3" | "OGG" | "MP4" | "WEBM" => color!(0xc4adee),
        _ => color!(0xa7b6f6),
    };
    let visual: El = if let Some(thumb) = &a.thumbnail {
        image(thumb)
            .width(Fill)
            .height(76)
            .content_fit(iced::ContentFit::Contain)
            .into()
    } else {
        container(
            container(text(ext).size(13).font(BOLD).color(RAIL))
                .padding([5, 8])
                .style(move |_| surface(tint, 5.)),
        )
        .center_x(Fill)
        .center_y(76)
        .into()
    };
    let action = if let Some(u) = &a.upload {
        Some(Msg::CancelUpload(room.into(), a.id, u.request))
    } else {
        (!a.busy).then_some(Msg::RemoveFile(a.id))
    };
    let remove = tooltip(
        button(
            text(if a.upload.is_some() { "Cancel" } else { "×" })
                .size(14)
                .font(BOLD),
        )
        .padding([0, 6])
        .on_press_maybe(action)
        .style(|_, s| button::Style {
            background: Some(
                if s == button::Status::Hovered {
                    color!(0x943e47)
                } else {
                    TRAY
                }
                .into(),
            ),
            border: Border::default().rounded(6),
            text_color: TEXT,
            ..Default::default()
        }),
        if a.upload.is_some() {
            "Cancel upload and keep draft"
        } else {
            "Remove attachment"
        },
        tooltip::Position::Top,
    );
    let picture = stack![
        container(visual).width(Fill).style(|_| surface(RAIL, 6.)),
        container(remove).padding(3).align_right(Fill)
    ];
    let short = if name.chars().count() > 21 {
        let head: String = name.chars().take(13).collect();
        let tail: String = name
            .chars()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}…{tail}")
    } else {
        name.clone()
    };
    let state = a.status_label();
    let meta = text(format!("{}  ·  {state}", crate::media::size(a.source.size)))
        .size(11)
        .color(if a.error.is_some() {
            color!(0xffa5ad)
        } else {
            MUTED
        });
    let meta: El = if let Some(err) = &a.error {
        tooltip(meta, text(err).size(13), tooltip::Position::Top).into()
    } else {
        meta.into()
    };
    let mut card = column![
        picture,
        tooltip(
            text(short).size(13).height(17).font(BOLD),
            text(name).size(13),
            tooltip::Position::Top
        ),
        meta
    ]
    .spacing(3);
    if let Some(n) = a.upload.as_ref().and_then(|u| u.percent) {
        card = card.push(
            iced::widget::progress_bar(0.0..=100.0, n as f32)
                .length(Fill)
                .girth(3),
        );
    }
    container(card)
        .padding(7)
        .width(158)
        .style(|_| surface(SIDE, 8.))
        .into()
}
/// Unread state summarized for a server-rail entry.
#[derive(Default, Clone, Copy)]
struct Mark {
    unread: bool,
    mentions: u64,
}
impl Mark {
    fn add(&mut self, r: &Room) {
        self.unread |= r.unread;
        if r.unread {
            // Discord counts every unread direct message as a mention.
            self.mentions += if r.dm {
                r.read.messages.max(r.read.highlights)
            } else {
                r.read.highlights
            };
        }
    }
}
use server_rail::rail_button;

impl App {
    /// Reactions, reply, edit and report, shown while a message is hovered:
    /// the entries of its right-click menu that have a toolbar icon.
    /// A message row, with the reaction picker floating under its hover
    /// toolbar while it's open there (opened from the toolbar or the
    /// right-click menu); the smile beside its reactions anchors its own.
    fn reaction_popover<'a>(&'a self, p: &'a crate::rooms::Post, row: El<'a>) -> El<'a> {
        let popup = self
            .reaction_picker
            .as_ref()
            .filter(|picker| !picker.beside_chips && p.event_id.as_ref() == Some(&picker.target))
            .and_then(|_| reaction_bar::panel(self, p));
        let Some(id) = p.event_id.clone() else {
            return row;
        };
        // The toolbar rises 16 px above the row's top-right corner, 16 px in,
        // and is about 36 px tall (view/hover.rs).
        popover::popover(
            row,
            popup,
            popover::Place::UnderCorner {
                inset: 16.,
                drop: 20.,
            },
            Msg::ReactionPicker(id),
        )
    }
    fn message_tools<'a>(&'a self, p: &'a crate::rooms::Post) -> Option<El<'a>> {
        // An open menu covers the window; a toolbar would catch its clicks.
        if self.context_menu.open.is_some() {
            return None;
        }
        let id = p.event_id.as_ref()?;
        // Nor do other messages raise theirs over an open reaction picker.
        if self
            .reaction_picker
            .as_ref()
            .is_some_and(|picker| &picker.target != id)
        {
            return None;
        }
        let items = self.message_items(&self.selected, p);
        if items.is_empty() {
            return None;
        }
        let mut tools = Row::new();
        for item in items {
            let (Some(hint), Some(glyph)) = (
                item.entry.toolbar_hint(),
                context_menu::tool_glyph(item.entry),
            ) else {
                continue;
            };
            let open = item.entry == crate::context_menu::Entry::AddReaction
                && self
                    .reaction_picker
                    .as_ref()
                    .is_some_and(|picker| &picker.target == id);
            tools = tools.push(tool(glyph, hint, open, item.msg));
        }
        Some(toolbar(tools))
    }
    /// The quoted line above a reply. Clicking it jumps to the original.
    fn reply_reference<'a>(&'a self, r: &'a Room, history: &'a Room, target: &'a str) -> El<'a> {
        let mut line = row![icons::spine()].spacing(4).align_y(Center);
        line = match r.quote(target).or_else(|| history.quote(target)) {
            Some(q) => {
                let who = r.display(&q.sender).to_owned();
                line.push(face(&who, self.user_picture(r, &q.sender), 16.))
                    .push(
                        text(format!("@{who}"))
                            .size(14)
                            .font(BOLD)
                            .color(names::color(r.role(&q.sender), BODY))
                            .wrapping(text::Wrapping::None),
                    )
                    .push(
                        container(text(q.excerpt).size(14).wrapping(text::Wrapping::None))
                            .clip(true)
                            .width(Fill),
                    )
            }
            None => line.push(text("View original · Earlier message").size(14)),
        };
        with_tip(
            button(line)
                .padding(0)
                .width(Fill)
                .on_press(Msg::Jump(target.to_owned()))
                .style(plain(CHANNEL, BODY)),
            "View original",
            tooltip::Position::Top,
        )
    }
    /// An inline image or video (opening the media viewer) with a hover
    /// download control, or a Discord-style file card.
    fn attachment<'a>(&'a self, event: Option<&str>, file: &'a crate::media::Media) -> El<'a> {
        use crate::media::Kind;
        let save = Msg::SaveFile(file.clone());
        let open = event
            .filter(|_| file.viewable())
            .map(|e| Msg::OpenViewer(e.to_owned()));
        let tools = || {
            container(toolbar(tool(
                Icon::Download,
                "Download",
                false,
                Some(save.clone()),
            )))
            .width(Fill)
            .align_right(Fill)
            .padding(6)
        };
        let clickable = |content: El<'a>| -> El<'a> {
            match open.clone() {
                Some(msg) => mouse_area(content)
                    .on_press(msg)
                    .interaction(iced::mouse::Interaction::Pointer)
                    .into(),
                None => content,
            }
        };
        // Known dimensions fix the picture's size before it loads, so the
        // timeline doesn't move when it arrives.
        let shown = file.inline_size();
        let picture = |handle: &'a iced::widget::image::Handle| {
            let picture = image(handle)
                .content_fit(iced::ContentFit::Contain)
                .border_radius(8);
            match shown {
                Some((w, h)) => picture.width(w).height(h),
                None => picture,
            }
        };
        let placeholder = |(w, h): (f32, f32)| -> El<'a> {
            container(Space::new())
                .width(w)
                .height(h)
                .style(|_| surface(FLOAT, 8.))
                .into()
        };
        if file.kind == Kind::Video {
            let still = file.preview.as_ref().and_then(|s| self.inline.ready(s));
            // The poster and the video playing in its place take the same
            // space, so starting it moves nothing.
            let size = shown
                .or_else(|| match still {
                    Some(iced::widget::image::Handle::Rgba { width, height, .. }) => {
                        Some(crate::media::fit_inline(*width, *height))
                    }
                    _ => None,
                })
                .unwrap_or((400., 225.));
            // Linux plays it here, as Discord does (inline_player.rs).
            #[cfg(target_os = "linux")]
            if let Some(event) = event
                && let Some(playing) = self.inline_video(event, size, still)
            {
                return iced::widget::hover(playing, tools());
            }
            let poster: El = match still {
                Some(handle) => image(handle)
                    .content_fit(iced::ContentFit::Contain)
                    .border_radius(8)
                    .width(size.0)
                    .height(size.1)
                    .into(),
                None => placeholder(size),
            };
            let play = container(
                container(icon(Icon::Play, 24., Color::WHITE))
                    .center(56)
                    .style(|_| surface(Color::from_rgba(0., 0., 0., 0.6), 28.)),
            )
            .center(Fill);
            let video = container(stack![poster, play])
                .max_width(400)
                .max_height(300);
            #[cfg(target_os = "linux")]
            if let Some(event) = event {
                let play = mouse_area(video)
                    .on_press(Msg::PlayInline(event.to_owned(), file.clone()))
                    .interaction(iced::mouse::Interaction::Pointer);
                return iced::widget::hover(play, tools());
            }
            return iced::widget::hover(clickable(video.into()), tools());
        }
        // Audio files and voice messages play here (inline_player.rs).
        #[cfg(target_os = "linux")]
        if file.kind == Kind::Audio
            && let Some(event) = event
        {
            return self.audio_card(
                event,
                file,
                tool(Icon::Download, "Download", false, Some(save.clone())),
            );
        }
        if let Some(source) = &file.preview {
            // An animated GIF or WebP plays, while it's on screen (P1).
            if let Some(frames) = self.inline.moving(source) {
                let mut playing = animated::animated(frames.clone()).border_radius(8.);
                if let Some((w, h)) = shown {
                    playing = playing.width(w).height(h);
                }
                let playing = container(playing).max_width(400).max_height(300);
                return iced::widget::hover(clickable(playing.into()), tools());
            }
            if let Some(handle) = self.inline.ready(source) {
                let picture = container(picture(handle)).max_width(400).max_height(300);
                return iced::widget::hover(clickable(picture.into()), tools());
            }
            if self.inline.pending(source) {
                let loading = placeholder(shown.unwrap_or((320., 180.)));
                return iced::widget::hover(clickable(loading), tools());
            }
            return container(
                column![
                    button(text("Retry image preview").size(14))
                        .padding(0)
                        .style(plain(LINK, color!(0x5fc5ff)))
                        .on_press(Msg::ShowInline(source.clone())),
                    button(text(format!("Download {}", file.name)).size(12))
                        .padding(0)
                        .style(plain(CHANNEL, BODY))
                        .on_press(save),
                ]
                .spacing(6),
            )
            .padding(16)
            .width(320)
            .height(180)
            .center_y(180)
            .style(|_| surface(SIDE, 8.))
            .into();
        }
        let size = file.size.map(crate::media::size).unwrap_or_default();
        container(
            row![
                icon(Icon::File, 34., color!(0x949cf7)),
                column![
                    button(
                        text(&file.name)
                            .size(16)
                            .wrapping(text::Wrapping::WordOrGlyph)
                    )
                    .padding(0)
                    .style(plain(LINK, color!(0x5fc5ff)))
                    .on_press(save.clone()),
                    text(size).size(12).color(CHANNEL)
                ]
                .spacing(2)
                .width(Fill),
                tool(Icon::Download, "Download", false, Some(save))
            ]
            .spacing(10)
            .align_y(Center),
        )
        .padding([10, 12])
        .max_width(432)
        .style(|_| container::Style {
            border: Border {
                color: color!(0x26272b),
                width: 1.,
                radius: 8.0.into(),
            },
            ..surface(SIDE, 8.)
        })
        .into()
    }
    /// Push-to-talk beside Join voice, armed before joining.
    #[cfg(target_os = "linux")]
    fn ptt_control(&self) -> El<'_> {
        crate::qa_layout::tag(
            "ptt-header",
            if self.room_ptt { "on" } else { "off" },
            "",
            iced::widget::checkbox(self.room_ptt)
                .label("Push-to-talk")
                .size(16)
                .text_size(14)
                .on_toggle(Msg::RoomPtt),
        )
    }
    /// The call's video, above the Voice Connected panel while its stage
    /// is closed.
    #[cfg(target_os = "linux")]
    fn call_preview(&self) -> Option<El<'_>> {
        let call = self.call.as_ref()?;
        let room = self
            .live
            .list
            .get(&call.room)
            .filter(|_| self.verified && !call.stage_open && !call.cancel.is_closed())?;
        let picture = self.avatars.get(room.avatar.as_deref());
        Some(
            container(call_stage::popover(call, room, picture, !self.setup_busy()))
                .width(Fill)
                .padding(8)
                .into(),
        )
    }
    #[cfg(target_os = "linux")]
    fn incoming_banner(&self) -> Option<El<'_>> {
        if let Some((room, note)) = &self.inbox.handled
            && *room == self.selected
            && self.mentions.is_none()
            && self.inbox.ringing.is_none()
        {
            return Some(
                container(
                    row![
                        text(*note).width(Fill).size(14),
                        button(text("Dismiss").size(13))
                            .style(nav(false))
                            .on_press(Msg::DismissCallNotice)
                    ]
                    .align_y(Center),
                )
                .width(Fill)
                .padding([6, 12])
                .style(|_| surface(TRAY, 4.))
                .into(),
            );
        }
        let (i, _) = self.inbox.ringing.as_ref()?;
        Some(
            container(
                row![
                    text(format!(
                        "Incoming {} call · {}",
                        if i.video { "video" } else { "voice" },
                        i.peer
                    ))
                    .width(Fill)
                    .size(15),
                    button(text("Decline").size(16).font(BOLD))
                        .padding([5, 14])
                        .style(nav(false))
                        .on_press(Msg::DecideRing(i.event.clone(), false)),
                ]
                .extend(
                    i.video
                        .then(|| text("Your camera is off").size(13).color(MUTED).into()),
                )
                .push(
                    button(text("Accept").size(16).font(BOLD))
                        .padding([5, 14])
                        .style(nav(true))
                        .on_press(Msg::DecideRing(i.event.clone(), true)),
                )
                .align_y(Center)
                .spacing(9),
            )
            .padding([7, 12])
            .style(|_| surface(TRAY, 4.))
            .into(),
        )
    }
    pub(super) fn view(&self) -> El<'_> {
        if let Some(screen) = self.sdk_screen() {
            return screen;
        }
        // The screen always sits first in this stack, under one fixed slot
        // per overlay. Wrapping it in a stack only while an overlay shows
        // would rebuild it, and the timeline would lose its scroll position
        // whenever the media viewer, a dialog or the server menu opened or
        // closed.
        let none = || -> El<'_> { Space::new().into() };
        let setup = match &self.setup {
            Some(form) if !self.account && (!self.logged_in || self.verified) => {
                let backdrop =
                    mouse_area(container(Space::new()).width(Fill).height(Fill).style(|_| {
                        container::Style {
                            background: Some(Color::from_rgba8(0, 0, 0, 0.7).into()),
                            ..Default::default()
                        }
                    }));
                let backdrop = if self.setup_busy() {
                    backdrop
                } else {
                    backdrop.on_press(Msg::Setup(crate::community::Input::Close))
                };
                stack![
                    opaque(backdrop),
                    container(community::view(self, form)).center(Fill)
                ]
                .into()
            }
            _ => none(),
        };
        // Server Settings covers the screen, which keeps its state below.
        let server_settings = self.server_settings_view().unwrap_or_else(none);
        let dialog = self
            .switcher_view()
            .or_else(|| self.delete_dialog())
            .or_else(|| self.leave_dialog())
            .or_else(|| self.block_dialog())
            .or_else(|| self.new_message_dialog())
            .or_else(|| self.forward_dialog())
            .or_else(|| names::dialog(self))
            .or_else(|| server_rail::folder_dialog(self))
            .or_else(|| self.poll_dialog())
            .or_else(|| self.end_poll_dialog())
            .or_else(|| self.shortcuts_view())
            .or_else(|| presence::dialog(self))
            .unwrap_or_else(none);
        let viewer = self
            .viewer
            .as_ref()
            .map_or_else(none, |v| viewer::view(self, v));
        let profile = self.viewer.is_none().then(|| profile::card(self)).flatten();
        let menu = match (self.server_menu && self.viewer.is_none() && profile.is_none())
            .then(|| self.server_menu_view())
            .flatten()
        {
            Some(menu) => {
                // Clicking anywhere outside the menu only closes it, as in Discord.
                let dismiss = opaque(
                    mouse_area(container(Space::new()).width(Fill).height(Fill))
                        .on_press(Msg::ServerMenu(false)),
                );
                stack![
                    dismiss,
                    container(menu).padding(Padding {
                        top: 52.,
                        left: 80.,
                        ..Padding::ZERO
                    })
                ]
                .into()
            }
            _ => none(),
        };
        stack![
            self.screen(),
            setup,
            server_settings,
            dialog,
            viewer,
            menu,
            profile.unwrap_or_else(none)
        ]
        .into()
    }
    /// Discord's server dropdown: invites, channel creation and notification
    /// defaults for the selected space.
    fn server_menu_view(&self) -> Option<El<'_>> {
        let Section::Space(space) = &self.section else {
            return None;
        };
        if !self.logged_in || !self.verified {
            return None;
        }
        let ready = self.connected && self.setup.is_none();
        let item = |glyph: Icon, label: &'static str, accent: Color, msg: Option<Msg>| {
            button(
                row![
                    text(label).size(14).width(Fill),
                    icon(
                        glyph,
                        18.,
                        if msg.is_some() {
                            accent
                        } else {
                            icons::ICON_OFF
                        }
                    )
                ]
                .align_y(Center),
            )
            .width(Fill)
            .padding([7, 8])
            .on_press_maybe(msg)
            .style(move |_, s| button::Style {
                background: (s == button::Status::Hovered).then(|| color!(0x4752c4).into()),
                text_color: match s {
                    button::Status::Hovered => Color::WHITE,
                    button::Status::Disabled => icons::ICON_OFF,
                    _ => accent,
                },
                border: Border::default().rounded(3),
                ..Default::default()
            })
        };
        let mut menu = column![
            // Read-only for members who can't change anything (server_settings.rs).
            item(
                Icon::Gear,
                "Server Settings",
                MUTED,
                (ready && self.may_open_server_settings()).then(|| Msg::ServerSettings(
                    crate::server_settings::Input::Open(space.clone())
                ))
            ),
            item(
                Icon::UserPlus,
                "Invite People",
                color!(0x949cf7),
                ready.then(|| Msg::Setup(crate::community::Input::InviteMember(space.clone())))
            ),
            item(
                Icon::Plus,
                "Create Channel",
                MUTED,
                ready.then(|| Msg::Setup(crate::community::Input::Start(Some(space.clone()))))
            ),
        ]
        .spacing(2);
        // Your nickname in this space (names.rs).
        if self.may_edit_server_profile() {
            menu = menu.push(item(
                Icon::Pencil,
                "Edit Server Profile",
                MUTED,
                ready.then(|| Msg::Names(crate::names::Input::Open(space.clone()))),
            ));
        }
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        {
            menu = menu
                .push(container(line_rule(color!(0x2e2f34))).padding([4, 8]))
                .push(item(
                    Icon::Bell,
                    "Notification Settings",
                    MUTED,
                    Some(Msg::SpaceAlerts),
                ));
        }
        Some(
            container(menu)
                .width(220)
                .padding(6)
                .style(|_| container::Style {
                    border: Border {
                        color: color!(0x2b2d31),
                        width: 1.,
                        radius: 6.0.into(),
                    },
                    ..surface(FLOAT, 6.)
                })
                .into(),
        )
    }
    fn rail_mark(&self, section: &Section) -> Mark {
        let store = self.rooms();
        let mut mark = Mark::default();
        match section {
            Section::Direct => store
                .ordered_direct
                .iter()
                .filter_map(|id| store.list.get(id))
                .for_each(|r| mark.add(r)),
            Section::All => store
                .list
                .values()
                .filter(|r| !r.space && !r.dm)
                .for_each(|r| mark.add(r)),
            Section::Space(space) => store
                .channels(space)
                .keys()
                .filter_map(|id| store.list.get(*id))
                .filter(|r| !r.space)
                .for_each(|r| mark.add(r)),
        }
        // Invites waiting count like mentions, as Discord's requests do.
        let invites = self.invites_in(section);
        if invites > 0 {
            mark.unread = true;
            mark.mentions += invites;
        }
        mark
    }
    fn channel_row<'a>(&'a self, id: &'a String, r: &'a Room) -> El<'a> {
        let active = self.mentions.is_none() && id == &self.selected;
        let unread = r.unread && !active;
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        let muted = !unread && self.alerts.mode(id) == crate::notifications::Mode::Quiet;
        #[cfg(not(any(target_os = "linux", target_arch = "wasm32")))]
        let muted = false;
        // A DM's peer has a status dot and their custom status (presence.rs).
        let peer = r.dm_peer.as_deref().and_then(|peer| self.presence_of(peer));
        let leading: El = if r.dm {
            presence::with_dot(
                self.room_face(r, 32.),
                32.,
                peer.as_ref().map(|p| p.status),
                if active { ACTIVE } else { SIDE },
            )
        } else {
            icon(
                if r.voice { Icon::Speaker } else { Icon::Hash },
                20.,
                if muted {
                    color!(0x4e5058)
                } else {
                    color!(0x80848e)
                },
            )
        };
        let name = text(&r.name)
            .size(if r.dm { 15 } else { 16 })
            .font_maybe(unread.then_some(BOLD))
            .wrapping(text::Wrapping::None)
            .width(Fill);
        let custom = peer.filter(|p| p.status.here()).and_then(|p| p.message);
        let label: El = match custom {
            Some(message) => column![
                name,
                text(message)
                    .size(12)
                    .color(CHANNEL)
                    .wrapping(text::Wrapping::None)
            ]
            .width(Fill)
            .into(),
            None => name.into(),
        };
        let mut entry = row![leading, label]
            .spacing(if r.dm { 12 } else { 6 })
            .align_y(Center);
        let mentions = if !r.unread {
            0
        } else if r.dm {
            r.read.messages.max(r.read.highlights)
        } else {
            r.read.highlights
        };
        if mentions > 0 {
            entry = entry.push(badge(mentions));
        }
        let link = button(entry)
            .width(Fill)
            .padding(if r.dm { [5, 8] } else { [6, 8] })
            .on_press(if r.voice {
                Msg::OpenVoice(id.clone())
            } else {
                Msg::Open(id.clone())
            })
            .style(move |_, s| button::Style {
                background: if active {
                    Some(ACTIVE.into())
                } else if s == button::Status::Hovered {
                    Some(HOVER.into())
                } else {
                    None
                },
                text_color: if active || unread {
                    TEXT
                } else if s == button::Status::Hovered {
                    BODY
                } else if muted {
                    color!(0x5c5e66)
                } else {
                    CHANNEL
                },
                border: Border::default().rounded(4),
                ..Default::default()
            });
        let link = self.menu_area(
            link,
            crate::context_menu::Target::Channel(id.clone()),
            String::new(),
            None,
        );
        row![container(pill(if unread { 8. } else { 0. })).width(8), link]
            .align_y(Center)
            .into()
    }
    #[cfg(target_os = "linux")]
    fn voice_people<'a>(&'a self, id: &'a String, r: &'a Room) -> Column<'a, Msg> {
        let speakers = self
            .call
            .as_ref()
            .filter(|c| c.group && c.room == *id)
            .map(|c| c.speaking.borrow());
        let mut people = Column::new().spacing(2);
        for user in r.voice_users() {
            let speaking = speakers.as_ref().is_some_and(|s| s.contains(user));
            let name = r.display(user);
            let face = container(face(name, self.user_picture(r, user), 24.))
                .padding(2)
                .style(move |_| container::Style {
                    border: Border {
                        color: if speaking { GREEN } else { Color::TRANSPARENT },
                        width: 2.,
                        radius: 14.0.into(),
                    },
                    ..Default::default()
                });
            let label = text(name)
                .size(14)
                .wrapping(text::Wrapping::None)
                .color(if speaking { TEXT } else { CHANNEL });
            people = people.push(
                container(
                    row![face, label]
                        .spacing(6)
                        .align_y(Center)
                        .extend(speaking.then(|| {
                            with_tip(
                                icon(Icon::Speaker, 14., GREEN),
                                "Speaking",
                                tooltip::Position::Right,
                            )
                        })),
                )
                .width(Fill)
                .padding(Padding {
                    left: 36.,
                    right: 8.,
                    top: 1.,
                    bottom: 1.,
                }),
            );
        }
        people
    }
    /// A category's heading, with a **+** that does `create` (and says
    /// what in its tooltip).
    fn category<'a>(
        &self,
        key: &str,
        title: impl text::IntoFragment<'a>,
        create: Option<(Msg, &'static str)>,
    ) -> El<'a> {
        let open = !self.collapsed.contains(key);
        let head = button(
            row![
                icon(
                    if open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    12.,
                    CHANNEL
                ),
                text(title).size(12).font(BOLD)
            ]
            .spacing(2)
            .align_y(Center),
        )
        .padding(0)
        .width(Fill)
        .on_press(Msg::Collapse(key.into()))
        .style(plain(CHANNEL, BODY));
        let mut line = row![head].align_y(Center);
        if let Some((msg, tip)) = create {
            line = line.push(icon_button(
                Icon::Plus,
                tip,
                false,
                Some(msg),
                tooltip::Position::Top,
            ));
        }
        container(line)
            .height(40)
            .align_y(iced::alignment::Vertical::Bottom)
            .padding(Padding {
                left: 10.,
                right: 4.,
                bottom: 0.,
                top: 12.,
            })
            .into()
    }
    fn channel_list(&self) -> Column<'_, Msg> {
        let mut text_rooms = Vec::new();
        let mut voice = Vec::new();
        let mut direct = Vec::new();
        for (id, r) in self.visible() {
            if r.dm {
                direct.push((id, r));
            } else if r.voice {
                voice.push((id, r));
            } else {
                text_rooms.push((id, r));
            }
        }
        let create = match &self.section {
            Section::Space(space) if self.logged_in && self.verified => (self.connected
                && self.setup.is_none())
            .then(|| Msg::Setup(crate::community::Input::Start(Some(space.clone())))),
            _ => None,
        };
        let section = match &self.section {
            Section::All => "all".to_owned(),
            Section::Direct => "direct".to_owned(),
            Section::Space(id) => id.clone(),
        };
        let mut list = Column::new().spacing(2).padding(Padding {
            right: 8.,
            bottom: 12.,
            ..Padding::ZERO
        });
        list = list.extend(self.invite_rows(&self.section));
        // A space's channels by category, to drag where allowed (N5).
        if let Some((_, groups)) = self.open_groups() {
            return self.space_channel_list(list, groups, create);
        }
        let groups = [
            (
                "text",
                if matches!(self.section, Section::Space(_)) {
                    "TEXT CHANNELS"
                } else {
                    "CHANNELS"
                },
                text_rooms,
            ),
            ("voice", "VOICE CHANNELS", voice),
            ("direct", "DIRECT MESSAGES", direct),
        ];
        // A space's channels this account hasn't joined, after its own
        // (explore.rs).
        let (unjoined_text, unjoined_voice) = self.unjoined_channels();
        // Direct Messages always has its heading, whose + starts a DM.
        let direct_home = self.section == Section::Direct;
        let new_message = (self.logged_in && self.verified && self.new_message.is_none())
            .then_some((Msg::Dm(crate::dms::Input::Open), "Create DM"));
        for (kind, title, rooms) in groups {
            let unjoined = match kind {
                "text" => unjoined_text.as_slice(),
                "voice" => unjoined_voice.as_slice(),
                _ => &[],
            };
            if rooms.is_empty() && unjoined.is_empty() && !(kind == "direct" && direct_home) {
                continue;
            }
            let key = format!("{section}/{kind}");
            let open = !self.collapsed.contains(&key);
            let plus = if kind == "direct" {
                new_message.clone()
            } else {
                create.clone().map(|msg| (msg, "Create Channel"))
            };
            list = list.push(self.category(&key, title, plus));
            for (id, r) in rooms {
                // A collapsed category still shows unread and selected channels.
                if !open && !r.unread && !(self.mentions.is_none() && id == &self.selected) {
                    continue;
                }
                list = list.push(self.channel_row(id, r));
                #[cfg(target_os = "linux")]
                if r.voice {
                    list = list.push(self.voice_people(id, r));
                }
            }
            if open {
                list = list.extend(unjoined.iter().map(|r| self.unjoined_row(r)));
            }
        }
        list
    }
    /// A member's loaded profile picture in `r`.
    pub(super) fn user_picture(
        &self,
        r: &Room,
        user: &str,
    ) -> Option<&iced::widget::image::Handle> {
        self.avatars.get(r.avatars.get(user).map(String::as_str))
    }
    /// A member's profile picture in `r`, playing if it's animated (P1).
    pub(super) fn user_face(&self, r: &Room, user: &str, name: &str, size: f32) -> El<'static> {
        let mxc = r.avatars.get(user).map(String::as_str);
        match self.avatars.moving(mxc) {
            Some(frames) => animated::animated(frames.clone())
                .width(size)
                .height(size)
                .content_fit(iced::ContentFit::Cover)
                .into(),
            None => face(name, self.avatars.get(mxc), size),
        }
    }
    fn room_face(&self, r: &Room, size: f32) -> El<'static> {
        face(&r.name, self.avatars.get(r.avatar.as_deref()), size)
    }
    fn user_panel(&self) -> El<'_> {
        let name = if self.logged_in {
            self.profile.as_str()
        } else {
            "Preview user"
        };
        // Our status where the homeserver shares presence, or Do Not
        // Disturb, which works without it (presence.rs).
        let (status, dot) = if self.logged_in {
            let line = presence::panel_line(self);
            let dot = line.as_ref().map(|_| self.own_status());
            (line.unwrap_or_else(|| self.matrix_id.clone()), dot)
        } else {
            (
                "Local preview".to_owned(),
                Some(crate::presence::Status::Offline),
            )
        };
        let face = presence::with_dot(
            face(name, self.avatars.get(self.own_avatar.as_deref()), 32.),
            32.,
            dot,
            TRAY,
        );
        let who = button(
            row![
                container(face).width(36).height(36),
                column![
                    text(name)
                        .size(14)
                        .font(BOLD)
                        .color(TEXT)
                        .wrapping(text::Wrapping::None),
                    text(status)
                        .size(12)
                        .color(MUTED)
                        .wrapping(text::Wrapping::None)
                ]
                .spacing(1)
            ]
            .spacing(8)
            .align_y(Center),
        )
        .width(Fill)
        .padding([2, 4])
        // A long name or Matrix ID stops at the gear.
        .clip(true)
        // The status menu, as on Discord; settings where there's no status
        // to pick.
        .on_press(
            if self.logged_in
                && (self.presence.support == crate::presence::Support::On || self.dnd_available())
            {
                Msg::Presence(crate::presence::Input::Menu(!self.presence.menu))
            } else {
                Msg::Account
            },
        )
        .style(|_, s| button::Style {
            background: (s == button::Status::Hovered).then(|| ACTIVE.into()),
            border: Border::default().rounded(4),
            ..Default::default()
        });
        let menu = (self.presence.menu && self.logged_in).then(|| presence::menu(self));
        let who = popover::popover(
            who,
            menu,
            popover::Place::Below,
            Msg::Presence(crate::presence::Input::Menu(false)),
        );
        let controls = row![who].spacing(2).align_y(Center);
        // Mute and Deafen, with or without a call (view/voice_panel.rs).
        #[cfg(target_os = "linux")]
        let controls = controls.push(self.voice_buttons());
        container(controls.push(icon_button(
            Icon::Gear,
            "User Settings",
            self.account,
            Some(Msg::Account),
            tooltip::Position::Top,
        )))
        .width(Fill)
        .padding([8, 8])
        .style(|_| surface(TRAY, 0.))
        .into()
    }
    fn screen(&self) -> El<'_> {
        if self.account || self.logged_in && !self.verified {
            return self.account_view();
        }
        let store = self.rooms();
        let exploring = self.explore.page.is_some();
        let mut icons = column![
            rail_button(
                "U".into(),
                None,
                "Direct Messages",
                self.section == Section::Direct && self.invite_open.is_none() && !exploring,
                Msg::Section(Section::Direct),
                self.rail_mark(&Section::Direct),
            ),
            rail_button(
                "#".into(),
                None,
                "All Rooms",
                self.section == Section::All && self.invite_open.is_none() && !exploring,
                Msg::Section(Section::All),
                self.rail_mark(&Section::All),
            ),
            container(
                container(Space::new())
                    .width(32)
                    .height(2)
                    .style(|_| surface(color!(0x35363c), 1.))
            )
            .padding(Padding {
                left: 12.,
                ..Padding::ZERO
            })
        ]
        .spacing(8);
        // Servers and folders (server_rail.rs).
        icons = self.server_rail(icons);
        for invite in self.invite_rail() {
            icons = icons.push(invite);
        }
        if self.logged_in && self.verified {
            let add = button(icons::hover_icon(Icon::Plus, 24., 48., GREEN, Color::WHITE))
                .padding(0)
                .on_press_maybe(
                    (self.connected && self.setup.is_none())
                        .then_some(Msg::Setup(crate::community::Input::Start(None))),
                )
                .style(|_, s| {
                    let hovered = s == button::Status::Hovered;
                    button::Style {
                        background: Some(if hovered { GREEN } else { CHAT }.into()),
                        border: Border::default().rounded(if hovered { 16 } else { 24 }),
                        ..Default::default()
                    }
                });
            icons = icons.push(
                container(with_tip(add, "Add a Space", tooltip::Position::Right)).padding(
                    Padding {
                        left: 12.,
                        ..Padding::ZERO
                    },
                ),
            );
            // Explore, under +, as Discord's Explore Discoverable Servers
            // (explore.rs).
            let explore = button(icons::hover_icon(
                Icon::Compass,
                24.,
                48.,
                if exploring { Color::WHITE } else { GREEN },
                Color::WHITE,
            ))
            .padding(0)
            .on_press_maybe(
                self.connected
                    .then_some(Msg::Explore(crate::explore::Input::Open)),
            )
            .style(move |_, s| {
                let lit = exploring || s == button::Status::Hovered;
                button::Style {
                    background: Some(if lit { GREEN } else { CHAT }.into()),
                    border: Border::default().rounded(if lit { 16 } else { 24 }),
                    ..Default::default()
                }
            });
            let marker = container(pill(if exploring { 40. } else { 0. })).center_y(48);
            // The directory needs the matrix-sdk engine.
            if self.sdk.is_some() {
                icons = icons.push(with_tip(
                    row![marker, explore].spacing(8).align_y(Center),
                    "Explore Discoverable Spaces",
                    tooltip::Position::Right,
                ));
            }
        }
        let rail = container(
            scrollable(icons)
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::new().width(0).scroller_width(0),
                ))
                .height(Fill),
        )
        .padding([12, 0])
        .width(72)
        .height(Fill)
        .style(|_| surface(RAIL, 0.));
        // Explore takes the sidebar and the main area, with a result's
        // preview over its page (explore.rs).
        if let Some(page) = &self.explore.page {
            let main = match &self.room_preview {
                Some(preview) => self.preview_card(preview),
                None => self.explore_page(page),
            };
            return row![rail, self.explore_side(page), main]
                .height(Fill)
                .into();
        }
        let section_name = match &self.section {
            Section::All => "All Rooms",
            Section::Direct => "Direct Messages",
            Section::Space(id) => store
                .list
                .get(id)
                .map(|r| r.name.as_str())
                .unwrap_or("Space"),
        };
        let has_menu = matches!(self.section, Section::Space(_)) && self.logged_in && self.verified;
        let title = text(section_name)
            .font(BOLD)
            .size(16)
            .color(TEXT)
            .wrapping(text::Wrapping::None)
            .width(Fill);
        let head: El = if has_menu {
            let open = self.server_menu;
            button(
                row![
                    title,
                    icon(
                        if open { Icon::Close } else { Icon::ChevronDown },
                        18.,
                        BODY
                    )
                ]
                .height(Fill)
                .align_y(Center),
            )
            .width(Fill)
            .height(48)
            .padding([0, 16])
            .on_press(Msg::ServerMenu(!open))
            .style(move |_, s| button::Style {
                background: (open || s == button::Status::Hovered).then(|| color!(0x35373c).into()),
                ..Default::default()
            })
            .into()
        } else {
            container(title)
                .padding([0, 16])
                .center_y(48)
                .width(Fill)
                .into()
        };
        // A space's banner behind its name (server_settings.rs).
        let head = self.banner_head(head);
        #[cfg_attr(
            not(any(target_os = "linux", target_arch = "wasm32")),
            allow(unused_mut)
        )]
        let mut sidebar = column![head, line_rule(color!(0x1f2023))];
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        if self.logged_in
            && self.verified
            && let Section::Space(id) = &self.section
            && let Some(space) = store.list.get(id)
            && self.alerts.spaces.panel
        {
            let busy = self
                .pending
                .values()
                .any(|p| matches!(p,Request::SpacePref(s) if s==id));
            let state = if busy {
                "Saving…"
            } else if space.space_alerts == crate::notifications::Mode::Default {
                "Inherit"
            } else {
                space.space_alerts.label()
            };
            let mut panel = column![
                row![
                    text("SPACE NOTIFICATIONS")
                        .size(12)
                        .font(BOLD)
                        .color(CHANNEL)
                        .width(Fill),
                    icon_button(
                        Icon::Close,
                        "Close",
                        false,
                        Some(Msg::SpaceAlerts),
                        tooltip::Position::Top
                    )
                ]
                .align_y(Center),
                text(format!("Space default: {state}"))
                    .size(13)
                    .font(BOLD)
                    .color(TEXT)
            ]
            .spacing(3);
            for m in crate::notifications::Mode::CHOICES {
                panel = panel.push(
                    button(
                        text(if m == crate::notifications::Mode::Default {
                            "Inherit"
                        } else {
                            m.label()
                        })
                        .size(13),
                    )
                    .style(nav(m == space.space_alerts))
                    .width(Fill)
                    .on_press_maybe(
                        (self.connected
                            && self.alerts.ready
                            && self.alerts.spaces.initialized
                            && !busy
                            && m != space.space_alerts)
                            .then(|| Msg::SpaceMode(id.clone(), m)),
                    ),
                );
            }
            panel = panel.push(
                text("Includes nested channels. Room settings take priority.")
                    .size(12)
                    .color(MUTED),
            );
            if self
                .pending
                .values()
                .any(|p| matches!(p, Request::SpaceRule(_)))
            {
                panel = panel.push(text("Updating channel rules…").size(12));
            }
            if self.alerts.spaces.failed() {
                panel = panel.push(button("Retry syncing rules").on_press(Msg::RetrySpaceRules));
            }
            if self.alerts.spaces.conflict() {
                panel = panel.push(text("A customized Matrix rule was left unchanged.").size(12));
            }
            sidebar = sidebar.push(
                container(
                    container(panel)
                        .padding([8, 10])
                        .width(Fill)
                        .style(|_| surface(TRAY, 6.)),
                )
                .padding([8, 8]),
            );
        }
        let mut links = Column::new().spacing(2).padding(Padding {
            top: 8.,
            ..Padding::ZERO
        });
        if self.logged_in && self.verified {
            let open = self.mentions.is_some();
            let mentions = button(
                row![
                    icon(Icon::At, 20., color!(0x80848e)),
                    text("Mentions").size(16)
                ]
                .spacing(6)
                .align_y(Center),
            )
            .width(Fill)
            .padding([6, 8])
            .on_press(Msg::OpenMentions)
            .style(move |_, s| button::Style {
                background: if open {
                    Some(ACTIVE.into())
                } else if s == button::Status::Hovered {
                    Some(HOVER.into())
                } else {
                    None
                },
                text_color: if open || s == button::Status::Hovered {
                    TEXT
                } else {
                    CHANNEL
                },
                border: Border::default().rounded(4),
                ..Default::default()
            });
            links = links.push(container(mentions).padding(Padding {
                left: 8.,
                right: 8.,
                ..Padding::ZERO
            }));
        }
        let sidebar = sidebar.push(
            scrollable(links.push(self.channel_list()))
                .direction(thin_scrollbar())
                .style(scroll_style(None))
                .height(Fill),
        );
        #[cfg(target_os = "linux")]
        let sidebar = sidebar
            .extend(self.call_preview())
            .push(self.call_corner(self.user_panel()));
        #[cfg(not(target_os = "linux"))]
        let sidebar = sidebar.push(self.user_panel());
        let side = container(sidebar)
            .width(240)
            .height(Fill)
            .style(|_| surface(SIDE, 0.));
        if let Some(inbox) = &self.mentions {
            let panel = Column::new();
            #[cfg(target_os = "linux")]
            let panel = panel.extend(self.incoming_banner());
            let content = mentions::view(self, inbox);
            return row![
                rail,
                side,
                container(panel.push(content))
                    .width(Fill)
                    .height(Fill)
                    .style(|_| surface(CHAT, 0.))
            ]
            .height(Fill)
            .into();
        }
        if let Some(preview) = &self.room_preview {
            return row![rail, side, self.preview_card(preview)]
                .height(Fill)
                .into();
        }
        if let Some(invite) = self.invite_open.as_deref().and_then(|id| self.invite(id)) {
            return row![rail, side, self.invite_card(invite)]
                .height(Fill)
                .into();
        }
        let room = store.list.get(&self.selected);
        #[cfg(target_os = "linux")]
        if let Some(c) = &self.call
            && c.stage_open
            && c.room == self.selected
            && let Some(r) = room
        {
            let header = row![
                column![
                    text(if c.group {
                        "VOICE CHANNEL"
                    } else {
                        "DIRECT CALL"
                    })
                    .size(11)
                    .color(MUTED)
                    .font(BOLD),
                    text(r.name.as_str()).size(20).font(BOLD).color(TEXT)
                ]
                .spacing(2),
                container(text(c.label.as_str()).size(12).font(BOLD).color(
                    if c.label == "Connected" || c.label.starts_with("In voice") {
                        color!(0x9be8b6)
                    } else {
                        MUTED
                    }
                ))
                .padding([4, 8])
                .style(|_| surface(TRAY, 9.)),
                Space::new().width(Fill),
                button(text("Open chat").size(14))
                    .style(nav(false))
                    .on_press(Msg::CallLayout(c.id, true)),
                button(text(if c.theater { "Exit theater" } else { "Theater" }).size(14))
                    .style(nav(c.theater))
                    .on_press(Msg::CallLayout(c.id, false))
            ]
            .align_y(Center)
            .spacing(10);
            let panel = column![container(header).padding([10, 16]).width(Fill)]
                .extend(self.incoming_banner())
                .push(call_stage::view(
                    c,
                    r,
                    &self.matrix_id,
                    &call_stage::Pictures {
                        avatars: &self.avatars,
                        own: self.own_avatar.as_deref(),
                    },
                ));
            let panel = container(panel)
                .style(|_| surface(color!(0x111214), 0.))
                .width(Fill)
                .height(Fill);
            return if c.theater {
                // The same panel and user panel as in the sidebar, over the
                // stage's bottom-left corner.
                let dock = container(
                    container(self.call_corner(self.user_panel()))
                        .width(240)
                        .style(|_| container::Style {
                            border: Border::default().rounded(border::top_right(8)),
                            ..surface(TRAY, 0.)
                        })
                        .clip(true),
                )
                .align_y(iced::alignment::Vertical::Bottom)
                .height(Fill);
                stack![panel, dock].into()
            } else {
                row![rail, side, panel].height(Fill).into()
            };
        }
        let channel = room.map(|r| r.name.as_str()).unwrap_or("Rooms");
        let lead: El = match room {
            Some(r) if r.dm => self.room_face(r, 24.),
            Some(r) if r.voice => icon(Icon::Speaker, 24., color!(0x80848e)),
            _ => icon(Icon::Hash, 24., color!(0x80848e)),
        };
        let mut header = row![
            lead,
            text(channel)
                .font(BOLD)
                .size(16)
                .color(TEXT)
                .wrapping(text::Wrapping::None)
        ]
        .spacing(8)
        .align_y(Center);
        match room.map(|r| r.topic.trim()).filter(|t| !t.is_empty()) {
            Some(topic) => {
                let mut short: String = topic.lines().next().unwrap_or_default().into();
                if short.chars().count() > 140 {
                    short = short.chars().take(139).collect::<String>() + "…";
                }
                header = header
                    .push(
                        container(Space::new())
                            .width(1)
                            .height(24)
                            .style(|_| surface(LINE, 0.)),
                    )
                    .push(
                        container(
                            text(short)
                                .size(14)
                                .color(CHANNEL)
                                .wrapping(text::Wrapping::None),
                        )
                        .clip(true)
                        .width(Fill),
                    );
            }
            None => header = header.push(Space::new().width(Fill)),
        }
        let full = self.logged_in && self.verified && room.is_some();
        let mut tools = Row::new().spacing(8).align_y(Center);
        #[cfg(target_os = "linux")]
        if self.call.is_none()
            && self.verified
            && self.connected
            && self.inbox.ringing.is_none()
            && let Some(r) = room.filter(|r| r.encrypted && (r.voice || r.dm_peer.is_some()))
        {
            if r.voice {
                tools = tools.push(self.ptt_control()).push(
                    button(text("Join voice").size(14).font(BOLD))
                        .padding([6, 14])
                        .style(|_, s| button::Style {
                            background: Some(
                                if s == button::Status::Hovered {
                                    color!(0x1a8b4b)
                                } else {
                                    color!(0x248046)
                                }
                                .into(),
                            ),
                            text_color: Color::WHITE,
                            border: Border::default().rounded(4),
                            ..Default::default()
                        })
                        .on_press(Msg::JoinVoice),
                );
            } else {
                tools = tools
                    .push(icon_button(
                        Icon::Phone,
                        "Start Voice Call",
                        false,
                        Some(Msg::DialDm(false)),
                        tooltip::Position::Bottom,
                    ))
                    .push(icon_button(
                        Icon::Video,
                        "Start Video Call",
                        false,
                        Some(Msg::DialDm(true)),
                        tooltip::Position::Bottom,
                    ));
            }
        }
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        if full {
            tools = tools.push(icon_button(
                Icon::Bell,
                "Notification Settings",
                self.alerts.panel,
                Some(Msg::NotificationPanel),
                tooltip::Position::Bottom,
            ));
        }
        if full {
            tools = tools.push(icon_button(
                Icon::Shield,
                "Posting Permissions",
                self.permission_panel,
                Some(Msg::Permissions),
                tooltip::Position::Bottom,
            ));
        }
        if full {
            use discussions::{Action, Panel};

            for (glyph, label, panel) in [
                (Icon::Pin, "Pinned Messages", Panel::Pins),
                (Icon::Threads, "Threads", Panel::Threads),
            ] {
                let active = !self.people
                    && (self.discussions.panel.as_ref() == Some(&panel)
                        || matches!(
                            (&panel, &self.discussions.panel),
                            (Panel::Threads, Some(Panel::Replies(_)))
                        ));
                let action = if active {
                    Action::Close
                } else {
                    Action::Open(panel)
                };
                tools = tools.push(icon_button(
                    glyph,
                    label,
                    active,
                    (active || self.connected).then_some(Msg::Discussion(action)),
                    tooltip::Position::Bottom,
                ));
            }
        }
        tools = tools
            .push(icon_button(
                Icon::Members,
                if self.people {
                    "Hide Member List"
                } else {
                    "Show Member List"
                },
                self.people,
                Some(Msg::TogglePeople),
                tooltip::Position::Bottom,
            ))
            .push(icon_button(
                Icon::Copy,
                "Copy Conversation",
                false,
                room.map(|_| Msg::Copy),
                tooltip::Position::Bottom,
            ));
        if full {
            let open = self.search.is_some();
            tools = tools.push(
                button(
                    row![
                        text("Search")
                            .size(14)
                            .color(if open { BODY } else { CHANNEL })
                            .width(Fill),
                        icon(Icon::Search, 16., if open { BODY } else { CHANNEL })
                    ]
                    .height(Fill)
                    .align_y(Center),
                )
                .width(if open { 220 } else { 150 })
                .height(26)
                .padding([0, 8])
                .on_press_maybe((!open && self.original.is_none()).then_some(Msg::OpenSearch))
                .style(|_, _| button::Style {
                    background: Some(RAIL.into()),
                    border: Border::default().rounded(4),
                    ..Default::default()
                }),
            );
        }
        let header = header.push(tools);
        let banner = if !self.logged_in {
            Some(
                container(
                    row![
                        text("PREVIEW").size(11).font(BOLD).color(color!(0xb7bcff)),
                        text("Explore a local room").size(13).width(Fill),
                        button(text("Sign in to Matrix").size(13).font(BOLD))
                            .on_press(Msg::Account)
                            .style(button::primary)
                    ]
                    .spacing(10)
                    .align_y(Center),
                )
                .width(Fill)
                .padding([7, 16])
                .style(|_| surface(color!(0x30334a), 0.)),
            )
        } else {
            None
        };
        let mut chat = column![
            container(header).padding([0, 16]).center_y(48).width(Fill),
            line_rule(color!(0x1f2023))
        ];
        if let Some(banner) = banner {
            chat = chat.push(banner)
        }
        if self.permission_panel
            && let Some(r) = room
        {
            let policy = &r.posting;
            let mut panel = column![panel_title(
                Icon::Shield,
                "Posting permissions".into(),
                Msg::Permissions
            )]
            .spacing(8);
            if let Some((member, plain, encrypted)) = policy.summary()
                && let Some(base) = policy.id()
                && let Some(action) = policy.action()
            {
                let busy = self.posting_busy(&self.selected);
                let status = if plain > member && encrypted > member {
                    "Read-only for ordinary members"
                } else if plain <= member && encrypted <= member {
                    "Members can post"
                } else {
                    "Custom posting levels"
                };
                panel = panel.push(
                    text(format!(
                        "{status} · member level {member}, message {plain}, encrypted {encrypted}"
                    ))
                    .size(14)
                    .color(BODY),
                );
                if policy.patch(&self.matrix_id, action).is_some() {
                    panel = panel.push(
                        button(
                            text(if busy { "Updating…" } else { action.label() })
                                .size(14)
                                .font(BOLD),
                        )
                        .padding([7, 14])
                        .style(primary)
                        .on_press_maybe((self.connected && !busy).then(|| {
                            Msg::SetPosting(crate::permissions::Change {
                                room: self.selected.clone(),
                                base: base.into(),
                                action,
                            })
                        })),
                    );
                } else {
                    panel =
                        panel.push(text("Your role cannot change these posting levels.").size(13));
                }
                panel=panel.push(text(match action{
                    crate::permissions::Action::Restrict=>"Read-only raises the default event level and chat overrides. Higher-level members can still post.",
                    crate::permissions::Action::Restore=>"Restore the previous posting levels, keeping subsequent rule changes.",
                    crate::permissions::Action::Allow=>"Open plain and encrypted messages to ordinary members. Other event levels stay in place.",
                }).size(13).color(CHANNEL));
            } else {
                panel = panel.push(
                    text("Posting permissions unavailable until room state is loaded.").size(13),
                );
            }
            chat = chat.push(card(panel));
        }
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        if self.alerts.panel
            && let Some(r) = room
        {
            let selected = self.alerts.mode(&self.selected);
            let inherited = self.alerts.spaces.inherited(&self.selected);
            let pending = self.notifications_pending(&self.selected);
            let mut options = Row::new().spacing(6);
            for mode in crate::notifications::Mode::CHOICES {
                let chosen = mode == selected;
                options = options.push(
                    button(
                        row![
                            container(container(Space::new()).width(8).height(8).style(
                                move |_| surface(
                                    if chosen {
                                        Color::WHITE
                                    } else {
                                        Color::TRANSPARENT
                                    },
                                    4.
                                )
                            ))
                            .center(16)
                            .style(move |_| container::Style {
                                border: Border {
                                    color: if chosen { Color::WHITE } else { ICON },
                                    width: 2.,
                                    radius: 8.0.into(),
                                },
                                ..Default::default()
                            }),
                            text(
                                if mode == crate::notifications::Mode::Default && inherited != mode
                                {
                                    "Use space default"
                                } else {
                                    mode.label()
                                },
                            )
                            .size(14)
                        ]
                        .spacing(8)
                        .align_y(Center),
                    )
                    .padding([8, 12])
                    .style(move |_, s| button::Style {
                        background: Some(
                            if chosen {
                                color!(0x5865f2, 0.25)
                            } else if s == button::Status::Hovered {
                                ACTIVE
                            } else {
                                HOVER
                            }
                            .into(),
                        ),
                        text_color: if chosen { Color::WHITE } else { BODY },
                        border: Border::default().rounded(4),
                        ..Default::default()
                    })
                    .on_press_maybe(
                        (self.alerts.ready && self.connected && !pending && selected != mode)
                            .then(|| Msg::NotifyMode(self.selected.clone(), mode)),
                    ),
                );
            }
            let state = if pending {
                "Saving…"
            } else if !self.alerts.ready {
                "Loading push rules…"
            } else if selected == crate::notifications::Mode::Default && inherited != selected {
                match inherited {
                    crate::notifications::Mode::Quiet => "Space default · Mute ordinary",
                    _ => "Space default · All messages",
                }
            } else {
                selected.label()
            };
            let mut settings = column![
                panel_title(
                    Icon::Bell,
                    format!("Room alerts · {} · {state}", r.name),
                    Msg::NotificationPanel
                ),
                options.wrap(),
                text("Mute ordinary messages; higher-priority mention rules still apply.")
                    .size(13)
                    .color(CHANNEL)
            ]
            .spacing(8);
            #[cfg(target_arch = "wasm32")]
            {
                let (label, can_ask) = match crate::notifications::browser::permission().as_str() {
                    "granted" => ("Browser notifications allowed", false),
                    "denied" => (
                        "Notifications blocked · change this site's browser permissions",
                        false,
                    ),
                    "default" if self.alerts.asking => ("Waiting for browser permission…", false),
                    "default" => ("Enable browser notifications", true),
                    _ => (
                        "Browser notifications unavailable · use a supported browser on HTTPS or localhost",
                        false,
                    ),
                };
                settings = if can_ask {
                    settings.push(
                        button(text(label).size(14))
                            .style(nav(false))
                            .on_press(Msg::AllowBrowserAlerts),
                    )
                } else {
                    settings.push(text(label).size(13))
                };
            }
            if self.alerts.delivery_failed {
                settings =
                    settings.push(text("Desktop notification service unavailable.").size(13));
            }
            chat = chat.push(card(settings));
        }
        #[cfg(target_os = "linux")]
        if let Some(b) = self.incoming_banner() {
            chat = chat.push(b);
        }
        if !self.status.is_empty() && !self.status.starts_with("Starting") {
            chat = chat.push(
                container(text(&self.status).size(13).color(color!(0xffc785))).padding([5, 16]),
            );
        }
        let body: El = if let Some(s) = &self.search
            && self.original.is_none()
            && let Some(r) = room
        {
            self.search_view(
                s,
                r,
                self.pending
                    .values()
                    .any(|p| matches!(p, Request::Context(..))),
            )
        } else if let Some(r) = room {
            self.conversation(r)
        } else {
            // A space just joined lists channels to join (explore.rs).
            let (to_join, voice_to_join) = self.unjoined_channels();
            let (title, hint) = if to_join.is_empty() && voice_to_join.is_empty() {
                (
                    "No rooms here yet",
                    "Choose another space or open All rooms.",
                )
            } else {
                (
                    "Pick a channel to join",
                    "Its channels are in the list: click one for its preview, then Join.",
                )
            };
            container(column![text(title).font(BOLD).size(24), text(hint).color(MUTED)].spacing(6))
                .center(Fill)
                .into()
        };
        let mut middle = Row::new().push(body).height(Fill);
        if let Some(panel) = discussions::view(self).filter(|_| room.is_some() && !self.people) {
            middle = middle.push(panel);
        } else if self.people && self.logged_in && room.is_some() {
            middle = middle.push(room_people::view(self));
        } else if self.people
            && let Some(r) = room
        {
            let people: std::collections::BTreeSet<_> =
                r.history.values().map(|p| p.sender.as_str()).collect();
            let mut list = column![
                container(label_owned(format!(
                    "IN THIS CONVERSATION — {}",
                    people.len()
                )))
                .padding([0, 8])
            ]
            .spacing(2)
            .padding([20, 8]);
            for sender in people {
                let n = r.display(sender);
                list = list.push(member_row(n, self.user_picture(r, sender), None, None));
            }
            middle = middle.push(
                container(
                    scrollable(list)
                        .direction(thin_scrollbar())
                        .style(scroll_style(None)),
                )
                .width(240)
                .height(Fill)
                .style(|_| surface(SIDE, 0.)),
            );
        }
        chat = chat.push(middle);
        row![
            rail,
            side,
            container(chat)
                .height(Fill)
                .width(Fill)
                .style(|_| surface(CHAT, 0.))
        ]
        .height(Fill)
        .into()
    }
    fn conversation<'a>(&'a self, r: &'a Room) -> El<'a> {
        let mut feed = Column::new().padding([16, 0]).spacing(0);
        let original = self.original.as_ref().filter(|c| c.room == self.selected);
        let history = original.and_then(|c| c.slice.as_ref()).unwrap_or(r);
        let timeline = self.timeline_range(history);
        let edge = |label: &'static str, msg: Option<Msg>| {
            container(
                button(text(label).size(13).font(BOLD))
                    .padding([4, 10])
                    .on_press_maybe(msg)
                    .style(|_, s| button::Style {
                        background: Some(
                            if s == button::Status::Hovered {
                                ACTIVE
                            } else {
                                SIDE
                            }
                            .into(),
                        ),
                        text_color: BODY,
                        border: Border::default().rounded(12),
                        ..Default::default()
                    }),
            )
            .center_x(Fill)
            .padding([4, 0])
        };
        let paging = self.paging(&self.selected);
        if original.is_none() && timeline.start > 0 {
            feed = feed.push(edge("Earlier messages", Some(Msg::TimelineEdge(true))));
        } else if original.is_none() && self.logged_in && !r.end && paging {
            feed = feed.push(
                container(text("Loading earlier messages…").size(13).color(CHANNEL))
                    .center_x(Fill)
                    .padding([8, 0]),
            );
        } else if original.is_none() && self.logged_in && !r.end {
            feed = feed.push(edge(
                "Earlier messages",
                self.connected.then_some(Msg::More),
            ));
        }
        if original.is_none() && timeline.start == 0 && (r.end || !self.logged_in) {
            let (badge, title, intro): (El, String, String) = if r.dm {
                (
                    self.room_face(r, 80.),
                    r.name.clone(),
                    format!(
                        "This is the beginning of your direct message history with {}.",
                        r.name
                    ),
                )
            } else {
                (
                    container(icon(
                        if r.voice { Icon::Speaker } else { Icon::Hash },
                        42.,
                        Color::WHITE,
                    ))
                    .center(68)
                    .style(|_| surface(color!(0x41434a), 34.))
                    .into(),
                    format!("Welcome to #{}!", r.name),
                    format!("This is the start of the #{} channel.", r.name),
                )
            };
            let mut welcome = column![
                badge,
                text(title).font(BOLD).size(32).color(TEXT),
                text(intro).size(16).color(MUTED)
            ]
            .spacing(8);
            if !r.dm && !r.topic.trim().is_empty() {
                welcome = welcome.push(text(&r.topic).size(14).color(CHANNEL));
            }
            feed = feed.push(container(welcome).padding([8, 16]));
        }
        let today = chrono::Local::now().date_naive();
        let stamp = |p: &'a crate::rooms::Post| -> String {
            if p.day == today {
                format!("Today at {}", p.clock)
            } else if today.pred_opt() == Some(p.day) {
                format!("Yesterday at {}", p.clock)
            } else {
                format!("{} {}", p.day.format("%-d %b %Y"), p.clock)
            }
        };
        let mut prev: Option<(&str, i64, chrono::NaiveDate)> = None;
        let count = history.posts().len();
        let anchor = original.and_then(|c| history.position(&c.event));
        let range = if let Some(c) = original {
            let i = anchor.unwrap_or(count);
            (if c.preceding { i.saturating_sub(6) } else { i })..count.min(i + 13)
        } else {
            timeline.clone()
        };
        let divider = if original.is_none() && r.unread {
            r.read.divider(r)
        } else {
            None
        };
        let seen = r.read.seen(&self.matrix_id);
        let posts: Vec<(usize, &crate::rooms::Post)> = range
            .clone()
            .zip(history.posts_in(range))
            .map(|(index, cached)| {
                let p = if original.is_some() {
                    cached
                        .event_id
                        .as_deref()
                        .and_then(|id| r.by_id(id))
                        .unwrap_or(cached)
                } else {
                    cached
                };
                (index, p)
            })
            .collect();
        // Messages a gallery (gallery.rs) already shows.
        let mut shown_in_gallery = 0;
        for (k, &(index, p)) in posts.iter().enumerate() {
            if shown_in_gallery > 0 {
                shown_in_gallery -= 1;
                continue;
            }
            let date_change = prev.is_none_or(|(_, _, day)| day != p.day);
            // An upgraded room's history meets the older room's (upgrades.rs).
            let seam = self.seams(history, index);
            // Like Discord, a reply always starts a new group with its own header.
            let group = date_change
                || seam.is_some()
                || p.reply.is_some()
                || prev.is_none_or(|(sender, time, _)| {
                    sender != p.sender || p.time.saturating_sub(time) > 300_000
                });
            feed = feed.extend(seam);
            if date_change {
                feed = feed.push(
                    container(
                        row![
                            line_rule(LINE),
                            text(&p.day_label).size(12).font(BOLD).color(CHANNEL),
                            line_rule(LINE)
                        ]
                        .spacing(8)
                        .align_y(Center),
                    )
                    .padding(Padding {
                        top: 20.,
                        bottom: 4.,
                        left: 16.,
                        right: 16.,
                    }),
                );
            }
            if divider.is_some() && p.event_id.as_deref() == divider {
                feed = feed.push(
                    container(
                        row![
                            line_rule(RED),
                            container(text("NEW").size(12).font(BOLD).color(Color::WHITE))
                                .padding([1, 5])
                                .style(|_| surface(RED, 4.))
                        ]
                        .align_y(Center),
                    )
                    .padding(Padding {
                        top: 8.,
                        bottom: 4.,
                        left: 16.,
                        right: 16.,
                    }),
                );
            }
            // Pictures sent together lay out as a gallery, each still its
            // own message; a divider, a seam or the message jumped to ends one.
            let run = self.gallery_run(&posts[k..], |i, q| {
                (divider.is_some() && q.event_id.as_deref() == divider)
                    || self.seams(history, i).is_some()
                    || original.is_some_and(|c| Some(&c.event) == q.event_id.as_ref())
            });
            if !original.is_some_and(|c| Some(&c.event) == p.event_id.as_ref())
                && let Some(last) = run.last()
            {
                feed = feed.push(self.gallery_row(r, &run, group, stamp(p)));
                prev = Some((last.sender.as_str(), last.time, last.day));
                shown_in_gallery = run.len() - 1;
                continue;
            }
            let name = r.display(&p.sender);
            let editing = self
                .editing
                .as_ref()
                .filter(|d| d.room == self.selected && Some(&d.event) == p.event_id.as_ref());
            let mut message = Column::new().spacing(4);
            // An attachment's body is usually just its filename; Discord shows the file alone.
            let caption = p.media.as_ref().is_none_or(|f| p.text().trim() != f.name);
            if let Some(edit) = editing {
                let input = text_editor(&edit.input)
                    .id("message-edit")
                    .on_action(Msg::ChangeEdit)
                    .key_binding(|press| {
                        if let Some(popup) =
                            self.completion_key(crate::autocomplete::Field::Edit, &press)
                        {
                            return Some(popup);
                        }
                        if matches!(press.status, text_editor::Status::Focused { .. }) {
                            if press.key == keyboard::Key::Named(keyboard::key::Named::Escape) {
                                return Some(text_editor::Binding::Custom(Msg::CancelEdit));
                            }
                            if press.key == keyboard::Key::Named(keyboard::key::Named::Enter)
                                && press.modifiers.is_empty()
                            {
                                return Some(text_editor::Binding::Custom(Msg::SaveEdit));
                            }
                        }
                        text_editor::Binding::from_key_press(press)
                    })
                    .padding([10, 14])
                    .height(Length::Shrink)
                    .max_height(160)
                    .style(field_style);
                let draft = edit.input.text();
                let can_save = self.connected
                    && edit.pending.is_none()
                    && !draft.trim().is_empty()
                    && draft != p.text();
                let input =
                    self.with_completion_slot(crate::autocomplete::Field::Edit, input.into());
                message = message.push(input).push(
                    row![
                        text("Escape to").size(12).color(BODY),
                        button(text("cancel").size(12))
                            .padding(0)
                            .style(plain(LINK, color!(0x5fc5ff)))
                            .on_press_maybe(edit.pending.is_none().then_some(Msg::CancelEdit)),
                        text("·").size(12).color(BODY),
                        text("Enter to").size(12).color(BODY),
                        button(
                            text(if edit.pending.is_some() {
                                "saving…"
                            } else {
                                "save edit"
                            })
                            .size(12)
                        )
                        .padding(0)
                        .style(plain(LINK, color!(0x5fc5ff)))
                        .on_press_maybe(can_save.then_some(Msg::SaveEdit))
                    ]
                    .spacing(4)
                    .align_y(Center),
                );
            } else if let Some(poll) = &p.poll {
                message = message.push(self.poll_card(p, poll));
            } else if caption {
                // Formatted or plain, `/me` and jumbo emoji (view/rich.rs).
                message = message.push(self.message_body(r, p));
            } else if p.revision.is_some() {
                message = message.push(text("(edited)").size(11).color(CHANNEL));
            }
            if let Some(file) = &p.media {
                message = message.push(self.attachment(p.event_id.as_deref(), file));
            }
            if r.link_previews
                && let Some(previews) = link_preview::view(self, p)
            {
                message = message.push(previews);
            }
            if let Some(bar) = reaction_bar::view(self, p) {
                message = message.push(bar);
            }
            if self.logged_in
                && let Some(id) = p.event_id.as_deref()
                && let Some(names) = seen.get(id)
            {
                let mut readers = names.iter().copied().filter(|u| *u != p.sender);
                let head: Vec<_> = readers.by_ref().take(3).map(|u| r.display(u)).collect();
                if !head.is_empty() {
                    let more = readers.count();
                    let suffix = if more == 0 {
                        String::new()
                    } else {
                        format!(" +{more}")
                    };
                    message = message.push(
                        text(format!("Seen by {}{suffix}", head.join(", ")))
                            .size(12)
                            .color(CHANNEL),
                    );
                }
            }
            let target = original.is_some_and(|c| Some(&c.event) == p.event_id.as_ref());
            let mut block = Column::new().spacing(2);
            if target {
                block = block.push(
                    text(
                        if self.search.as_ref().is_some_and(|s| {
                            s.hits.iter().any(|h| Some(&h.id) == p.event_id.as_ref())
                        }) {
                            "Search match"
                        } else {
                            "Original message"
                        },
                    )
                    .size(12)
                    .color(color!(0xa8b1ff))
                    .font(BOLD),
                );
            }
            if let Some(quoted) = &p.reply {
                block = block.push(self.reply_reference(r, history, quoted));
            }
            block = block.push(if group {
                // The author's picture and name are separate places to click.
                let place = p.event_id.clone().unwrap_or_else(|| index.to_string());
                let head = row![
                    self.user_area(
                        &p.sender,
                        format!("name {place}"),
                        profile::link(self, &p.sender, names::author(name, r.role(&p.sender), 16.))
                    ),
                    text(stamp(p)).size(12).color(CHANNEL)
                ]
                .spacing(8)
                .align_y(Center);
                row![
                    container(self.user_area(
                        &p.sender,
                        format!("picture {place}"),
                        profile::link(self, &p.sender, self.user_face(r, &p.sender, name, 40.))
                    ))
                    .padding(Padding {
                        top: 2.,
                        ..Padding::ZERO
                    }),
                    column![head, message].spacing(2).width(Fill)
                ]
                .spacing(16)
            } else {
                row![Space::new().width(40), message.width(Fill)].spacing(16)
            });
            let mentioned = crate::rooms::items(&p.mentions["user_ids"])
                .any(|u| u.as_str() == Some(self.matrix_id.as_str()));
            let (tint, bar) = if target {
                (Some(color!(0x5865f2, 0.1)), BLURPLE)
            } else if mentioned {
                (Some(color!(0xf0b232, 0.08)), color!(0xf0b232))
            } else {
                (None, Color::TRANSPARENT)
            };
            let body = container(
                row![
                    container(Space::new())
                        .width(2)
                        .height(Fill)
                        .style(move |_| surface(bar, 0.)),
                    container(block).width(Fill).padding(Padding {
                        top: 2.,
                        bottom: 2.,
                        left: 14.,
                        right: 48.,
                    })
                ]
                .height(Length::Shrink),
            )
            .width(Fill)
            .style(move |_| container::Style {
                background: tint.map(Into::into),
                ..Default::default()
            });
            let gutter: El = if group || editing.is_some() {
                Space::new().width(56).into()
            } else {
                container(text(&p.clock).size(11).color(CHANNEL))
                    .width(56)
                    .align_x(iced::alignment::Horizontal::Right)
                    .padding(Padding {
                        top: 6.,
                        right: 4.,
                        ..Padding::ZERO
                    })
                    .into()
            };
            let tools = editing.is_none().then(|| self.message_tools(p)).flatten();
            prev = Some((p.sender.as_str(), p.time, p.day));
            let (_, row) = history.key_at(index).expect("timeline index is in range");
            let id = p
                .event_id
                .as_ref()
                .map_or_else(|| format!("post-row-{row}"), |id| format!("post-{id}"));
            feed = feed.push(
                container(self.message_area(
                    p,
                    self.reaction_popover(p, hover::row(body, gutter, tools, ROW_HOVER)),
                ))
                .width(Fill)
                .padding(Padding {
                    top: if group { 14. } else { 0. },
                    ..Padding::ZERO
                })
                .id(id),
            );
        }
        if original.is_none() && timeline.end == count {
            feed = feed.extend(self.seams(history, count));
        }
        let has_newer = timeline.end < count;
        if original.is_none() && has_newer {
            feed = feed.push(edge("Newer messages", Some(Msg::TimelineEdge(false))));
        }
        let feed = scrollable(feed)
            .height(Fill)
            .id(self.timeline_id())
            .direction(scrollable::Direction::Vertical(
                scrollable::Scrollbar::new()
                    .width(8)
                    .scroller_width(8)
                    .margin(4),
            ))
            .style(scroll_style(Some(SIDE)));
        let feed = if original.is_some() {
            feed
        } else {
            // Where it is; paging.rs loads more at the edges (Discord offers
            // Jump to present once you're a screen or so up).
            feed.anchor_bottom().on_scroll(move |v| {
                Msg::ReadingScroll(
                    self.selected.clone(),
                    crate::paging::Scrolled::of(v, has_newer),
                )
            })
        };
        // The feed always sits in this stack: adding the layer only while
        // scrolled up would rebuild the scrollable and lose its position.
        let jump: El = if original.is_none() && self.scrolled_far {
            // The whole bar is the button, as on Discord.
            let bar = button(
                row![
                    text("You're viewing older messages")
                        .size(14)
                        .color(Color::WHITE)
                        .width(Fill),
                    row![
                        text("Jump to present").size(14).font(BOLD),
                        icon(Icon::ChevronDown, 14., Color::WHITE)
                    ]
                    .spacing(6)
                    .align_y(Center)
                ]
                .align_y(Center),
            )
            .padding([8, 16])
            .width(Fill)
            .on_press(Msg::Latest)
            .style(|_, s| button::Style {
                background: Some(
                    if s == button::Status::Hovered {
                        color!(0x4752c4)
                    } else {
                        color!(0x5865f2)
                    }
                    .into(),
                ),
                text_color: Color::WHITE,
                border: Border::default().rounded(border::top(8)),
                ..Default::default()
            });
            container(bar)
                .align_bottom(Fill)
                .padding(Padding {
                    left: 16.,
                    right: 16.,
                    ..Padding::ZERO
                })
                .into()
        } else {
            Space::new().into()
        };
        let feed: El = stack![feed, jump].into();
        let mut viewport = Column::new();
        let unread_banner = if original.is_none() && r.unread {
            r.read.unread_summary(r)
        } else {
            None
        };
        // The banner's slot is always there, empty while the room is read:
        // a banner appearing above the feed as a message arrives would move
        // the scrollable, which loses its place and jumps to the newest
        // message, reading it.
        let banner: El = if let Some((count, since)) = unread_banner {
            let noun = if count == 1 { "message" } else { "messages" };
            let caption = since.map_or_else(
                || format!("{count} new {noun}"),
                |time| format!("{count} new {noun} since {time}"),
            );
            let mark = crate::context_menu::Event::MarkLatestRead(self.selected.clone());
            container(
                row![
                    text(caption).size(14).color(Color::WHITE).width(Fill),
                    button(text("Mark as Read").size(13).font(BOLD))
                        .style(plain(Color::WHITE, color!(0xdee0fc)))
                        .on_press_maybe(
                            (self.verified && self.connected && !self.writing_read(&self.selected))
                                .then_some(Msg::ContextMenu(mark))
                        ),
                ]
                .align_y(Center)
                .spacing(8),
            )
            .padding([4, 16])
            .width(Fill)
            .style(|_| surface(color!(0x4752c4), 0.))
            .into()
        } else {
            Space::new().into()
        };
        viewport = viewport.push(banner);
        if let Some(c) = original {
            let jump = |label: &'static str, msg: Msg| {
                button(text(label).size(14).font(BOLD))
                    .padding([2, 8])
                    .style(plain(Color::WHITE, color!(0xdee0fc)))
                    .on_press(msg)
            };
            let mut header = row![
                text("You're viewing older messages")
                    .size(14)
                    .color(Color::WHITE)
                    .width(Fill)
            ];
            if anchor.is_some_and(|i| i > 0) {
                header = header.push(jump(
                    if c.preceding {
                        "Back to original"
                    } else {
                        "Preceding messages"
                    },
                    Msg::Preceding,
                ));
            }
            let header = if self.search.is_some() {
                header
                    .push(jump("Back to results", Msg::SearchResults))
                    .push(jump("Return to timeline", Msg::ReturnToReading))
            } else {
                header.push(jump("Back to latest", Msg::Latest))
            };
            let header = header.spacing(5).align_y(Center);
            viewport = viewport.push(
                container(header)
                    .padding([6, 16])
                    .width(Fill)
                    .style(|_| surface(color!(0x4752c4), 0.)),
            );
        }
        if self
            .pending
            .values()
            .any(|p| matches!(p,Request::Context(room,_) if room==&self.selected))
        {
            viewport = viewport.push(text("Loading original message…").size(13));
        }
        let upgraded = r.successor.is_some();
        let restricted = self.logged_in
            && (upgraded || r.posting.can_post(&self.matrix_id, r.encrypted) == Some(false));
        let hint = if upgraded {
            "This room was upgraded".into()
        } else if restricted {
            "Read-only for your account".into()
        } else {
            // A group DM is named for its people, with no one to @.
            let sign = match (r.dm, &r.dm_peer) {
                (true, Some(_)) => "@",
                (true, None) => "",
                (false, _) => "#",
            };
            format!("Message {sign}{}", r.name)
        };
        let editor = composer_editor(&self.editor, "composer", hint, Some(Msg::Draft), |press| {
            if let Some(popup) = self.completion_key(crate::autocomplete::Field::Composer, &press) {
                return Some(popup);
            }
            // ↑ in an empty composer edits our last message; Escape keeps
            // the focus and goes on to the app (composer.rs).
            if let Some(edit) = self.edit_last_key(&press) {
                return Some(text_editor::Binding::Custom(edit));
            }
            if crate::composer::lets_escape_through(&press) {
                return None;
            }
            #[cfg(target_arch = "wasm32")]
            if press.modifiers.command()
                && press.key.as_ref() == keyboard::Key::Character("v")
                && matches!(press.status, text_editor::Status::Focused { .. })
            {
                return Some(text_editor::Binding::Custom(Msg::WebPaste(
                    super::Field::Composer(self.selected.clone()),
                )));
            }
            if composer_send_key(&press) {
                Some(text_editor::Binding::Custom(Msg::Send))
            } else {
                let binding = text_editor::Binding::from_key_press(press);
                #[cfg(not(target_arch = "wasm32"))]
                if matches!(binding, Some(text_editor::Binding::Paste)) {
                    return Some(text_editor::Binding::Custom(Msg::PasteComposer));
                }
                binding
            }
        });
        let sending = self
            .pending
            .values()
            .any(|p| matches!(p,Request::Send(id,..) if id==&self.selected));
        let files = self.outbox.rooms.get(&self.selected);
        let has_files = files.is_some_and(|q| !q.is_empty());
        let skipped = self.outbox.skipped.get(&self.selected);
        let uploading = self.batch.as_ref().is_some_and(|b| b.room == self.selected);
        let mut tray = Column::new().spacing(4).extend(self.upgrade_offer(r));
        if let Some(d) = &self.reporting {
            let busy = self.report_pending();
            let ready = !busy && self.connected && self.verified && !d.reason.trim().is_empty();
            let input = text_input("Reason for reporting", &d.reason)
                .id("report-reason")
                .on_input_maybe((!busy).then_some(Msg::ReportReason))
                .on_submit_maybe(ready.then_some(Msg::SubmitReport))
                .padding(10)
                .style(input_style);
            let submit = button(
                text(if busy {
                    "Submitting…"
                } else {
                    "Submit report"
                })
                .size(14)
                .font(BOLD),
            )
            .padding([8, 14])
            .style(move |_, s| button::Style {
                background: Some(
                    match s {
                        button::Status::Disabled => color!(0xa12d2f),
                        button::Status::Hovered => color!(0xc22d31),
                        _ => color!(0xda373c),
                    }
                    .into(),
                ),
                text_color: if ready {
                    Color::WHITE
                } else {
                    color!(0xd9a2a3)
                },
                border: Border::default().rounded(4),
                ..Default::default()
            })
            .on_press_maybe(ready.then_some(Msg::SubmitReport));
            let cancel = button(text("Cancel").size(14))
                .padding([8, 14])
                .style(plain(BODY, Color::WHITE))
                .on_press_maybe((!busy).then_some(Msg::CancelReport));
            let panel=column![
                row![icon(Icon::Flag, 20., RED), text("Report message").font(BOLD).size(16).color(TEXT)].spacing(8).align_y(Center),
                text(&d.preview).size(14).color(BODY),
                text("Your homeserver admin receives the event ID and your reason. The message text is not attached.").size(13).color(CHANNEL),
                row![input,cancel,submit].align_y(Center).spacing(8),
            ].spacing(8);
            return column![
                viewport.push(feed).height(Fill),
                container(
                    container(panel)
                        .padding(16)
                        .width(Fill)
                        .style(|_| container::Style {
                            border: Border {
                                color: color!(0x26272b),
                                width: 1.,
                                radius: 8.0.into()
                            },
                            ..surface(SIDE, 8.)
                        })
                )
                .padding(Padding {
                    left: 16.,
                    right: 16.,
                    bottom: 24.,
                    top: 0.
                })
            ]
            .width(Fill)
            .height(Fill)
            .into();
        }
        if let Some(reason) = skipped {
            tray = tray.push(
                row![
                    text(reason).size(13).color(color!(0xffc785)).width(Fill),
                    button(text("Dismiss").size(13))
                        .style(plain(LINK, color!(0x5fc5ff)))
                        .on_press(Msg::DismissSkipped(self.selected.clone()))
                ]
                .spacing(6)
                .align_y(Center),
            );
            if !has_files && !self.editor.text().trim().is_empty() {
                tray = tray.push(
                    text("Choose another file or dismiss to send just the text.")
                        .size(12)
                        .color(CHANNEL),
                );
            }
        }
        if self.hover_files {
            tray = tray.push(
                container(
                    text(format!("Drop files to attach to {}", r.name))
                        .color(Color::WHITE)
                        .font(BOLD),
                )
                .padding([10, 14])
                .width(Fill)
                .style(|_| surface(BLURPLE, 8.)),
            );
        }
        let reply = self.reply_drafts.get(&self.selected).map(|q| {
            let close = button(container(icon(Icon::Close, 12., SIDE)).center(16))
                .padding(0)
                .on_press(Msg::CancelReply)
                .style(|_, s| button::Style {
                    background: Some(
                        if s == button::Status::Hovered {
                            BODY
                        } else {
                            ICON
                        }
                        .into(),
                    ),
                    border: Border::default().rounded(8),
                    ..Default::default()
                });
            container(
                row![
                    text("Replying to").size(14).color(CHANNEL),
                    text(r.display(&q.sender).to_owned())
                        .size(14)
                        .font(BOLD)
                        .color(BODY)
                        .wrapping(text::Wrapping::None),
                    container(
                        text(q.excerpt.clone())
                            .size(14)
                            .color(CHANNEL)
                            .wrapping(text::Wrapping::None)
                    )
                    .clip(true)
                    .width(Fill),
                    with_tip(close, "Cancel reply", tooltip::Position::Top)
                ]
                .spacing(5)
                .align_y(Center),
            )
            .padding([9, 16])
            .width(Fill)
            .style(|_| container::Style {
                background: Some(SIDE.into()),
                border: Border::default().rounded(border::top(8)),
                ..Default::default()
            })
        });
        // Discord's +: a menu with Upload a File and Create Poll (polls.rs),
        // over the composer.
        let open = self.composer_menu;
        let attach = popover::popover(
            button(container(icon(Icon::Plus, 16., FIELD)).center(24))
                .padding(0)
                .on_press(Msg::ComposerMenu(!open))
                .style(move |_, s| button::Style {
                    background: Some(
                        if open || s == button::Status::Hovered {
                            BODY
                        } else {
                            ICON
                        }
                        .into(),
                    ),
                    border: Border::default().rounded(12),
                    ..Default::default()
                }),
            self.composer_menu(),
            popover::Place::Below,
            Msg::ComposerMenu(false),
        );
        // Discord's emoji button: the emoji picker, inserting at the cursor.
        let emoji = icon_button(
            Icon::Smile,
            "Emoji",
            self.completion.picker.is_some(),
            (!restricted).then_some(Msg::Autocomplete(crate::autocomplete::Input::TogglePicker)),
            tooltip::Position::Top,
        );
        let can_send = !sending
            && !restricted
            && (!self.logged_in || self.connected)
            && (has_files && self.batch.is_none() && !self.media_config_pending()
                || !has_files && skipped.is_none() && !self.editor.text().trim().is_empty());
        let retry = files.is_some_and(|q| q.iter().any(|a| a.cancelled || a.error.is_some()));
        let send: El = if retry && !(sending || uploading) {
            button(text("Retry").size(14).font(BOLD))
                .padding([5, 12])
                .on_press_maybe(can_send.then_some(Msg::Send))
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
                })
                .into()
        } else {
            icon_button(
                Icon::Send,
                if sending || uploading {
                    "Sending…"
                } else {
                    "Send"
                },
                false,
                can_send.then_some(Msg::Send),
                tooltip::Position::Top,
            )
        };
        // The attachments sit in one fixed slot above the editor: inserting
        // them before it only while files are staged would rebuild the
        // editor, and a drop or paste would take its focus, so Enter
        // wouldn't send.
        let staged: El = match files.filter(|q| !q.is_empty()) {
            Some(files) => {
                let cards =
                    Row::with_children(files.iter().map(|a| attachment_card(&self.selected, a)))
                        .spacing(8)
                        .padding([2, 0]);
                column![
                    container(scrollable(cards).horizontal().width(Fill).height(140)).padding(
                        Padding {
                            top: 12.,
                            left: 16.,
                            right: 16.,
                            bottom: 6.,
                        },
                    ),
                    line_rule(color!(0x2e2f34))
                ]
                .into()
            }
            None => Space::new().into(),
        };
        let field = column![staged].push(
            row![attach, editor, emoji, self.sticker_button(restricted), send]
                .spacing(14)
                .align_y(Center)
                .padding(Padding {
                    left: 16.,
                    right: 8.,
                    ..Padding::ZERO
                }),
        );
        let attached = reply.is_some();
        let composer = container(field)
            .width(Fill)
            .style(move |_| container::Style {
                background: Some(FIELD.into()),
                border: Border::default().rounded(if attached {
                    border::bottom(8)
                } else {
                    border::radius(8)
                }),
                ..Default::default()
            });
        let foot = if upgraded {
            "Read-only: this room was upgraded"
        } else if restricted {
            "Read-only for your account"
        } else if !self.logged_in {
            "Local preview · Messages stay here"
        } else if r.encrypted {
            "End-to-end encrypted"
        } else {
            "Matrix room"
        };
        // Who is typing takes the note's place, in one slot either way.
        let note = self.typing_footer().unwrap_or_else(|| {
            let mut note = row![].spacing(4).align_y(Center);
            if self.logged_in && r.encrypted && !restricted {
                note = note.push(icon(Icon::Lock, 12., CHANNEL));
            }
            note.push(text(foot).size(12).color(CHANNEL)).into()
        });
        let mut footer = row![note, Space::new().width(Fill)]
            .spacing(4)
            .align_y(Center)
            .height(24);
        if self.logged_in && self.verified && unread_banner.is_none() {
            let id = r.read.target().map(str::to_owned);
            let available = self.connected && !self.writing_read(&self.selected) && id.is_some();
            footer = footer.push(
                button(text(if r.unread { "Mark read" } else { "Mark unread" }).size(12))
                    .padding([1, 4])
                    .style(plain(CHANNEL, BODY))
                    .on_press_maybe(available.then(|| {
                        Msg::MarkRoom(self.selected.clone(), if r.unread { id } else { None })
                    })),
            );
        }
        column![
            self.with_composer_overlays(viewport.push(feed).height(Fill).into()),
            container(
                column![
                    tray,
                    // The reply bar's slot stays put too, for the same reason.
                    column![
                        reply.map_or_else(|| Space::new().into(), Into::<El>::into),
                        composer
                    ],
                    footer
                ]
                .spacing(4)
            )
            .padding(Padding {
                left: 16.,
                right: 16.,
                top: 0.,
                bottom: 2.,
            })
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }
    fn account_view(&self) -> El<'_> {
        if self.logged_in && self.verified {
            return self.settings_view();
        }
        macro_rules! field {
            ($label:expr,$value:expr,$kind:ident) => {{
                let input = text_input($label, $value)
                    .on_input(Msg::$kind)
                    .padding(10)
                    .size(16)
                    .style(input_style);
                #[cfg(target_arch = "wasm32")]
                let input = input.on_paste(|_| Msg::WebPaste(super::Field::$kind));
                input
            }};
        }
        let caption = |s: &'static str| text(s).size(12).font(BOLD).color(MUTED);
        let mut form = column![].spacing(18).width(Fill);
        #[cfg(target_os = "linux")]
        if let Some(b) = self.incoming_banner() {
            form = form.push(b);
        }
        #[cfg(target_os = "linux")]
        if let Some(dock) = self.call_dock_compact() {
            form = form.push(dock);
        }
        if !self.logged_in {
            form = form
                .push(
                    column![
                        text("Welcome back").size(24).font(BOLD).color(TEXT),
                        text("Sign in to your Matrix homeserver.")
                            .size(16)
                            .color(MUTED)
                    ]
                    .spacing(8)
                    .width(Fill)
                    .align_x(Center),
                )
                .push(
                    column![
                        caption("HOMESERVER"),
                        field!("Homeserver · matrix.org", &self.server, Server)
                    ]
                    .spacing(8),
                )
                .push(
                    column![
                        caption("USERNAME"),
                        field!("Matrix username", &self.username, Username)
                    ]
                    .spacing(8),
                )
                .push(
                    column![
                        caption("PASSWORD"),
                        field!("Password", &self.password, Password)
                            .secure(true)
                            .on_submit(Msg::Login)
                    ]
                    .spacing(8),
                )
                .push(
                    button(
                        container(text("Sign in").size(16).font(BOLD))
                            .center_x(Fill)
                            .padding([3, 0]),
                    )
                    .width(Fill)
                    .padding([8, 16])
                    .style(primary)
                    .on_press_maybe(
                        (self.connected
                            && !self.pending.values().any(|p| matches!(p, Request::Login)))
                        .then_some(Msg::Login),
                    ),
                );
        } else if !self.verified {
            // The web build uses a real password input in a browser dialog,
            // so keyboard, context-menu paste and password managers work normally.
            #[cfg(not(target_arch = "wasm32"))]
            {
                form = form
                    .push(
                        column![
                            text("Verify this device").size(24).font(BOLD).color(TEXT),
                            text(&self.matrix_id).size(14).color(MUTED)
                        ]
                        .spacing(8)
                        .width(Fill)
                        .align_x(Center),
                    )
                    .push(
                        column![
                            // Not the placeholder's words: tests tap "passphrase" to focus the field.
                            caption("RECOVERY KEY"),
                            text_input("Recovery key or passphrase", &self.recovery)
                                .on_input(Msg::Recovery)
                                .padding(10)
                                .size(16)
                                .style(input_style)
                                .secure(true)
                                .on_submit(Msg::Verify)
                        ]
                        .spacing(8),
                    )
                    .push(
                        button(
                            container(
                                text(if self.verifying() {
                                    "Verifying…"
                                } else {
                                    "Verify device"
                                })
                                .size(16)
                                .font(BOLD),
                            )
                            .center_x(Fill)
                            .padding([3, 0]),
                        )
                        .width(Fill)
                        .padding([8, 16])
                        .style(primary)
                        .on_press_maybe(
                            (self.connected
                                && !self.verifying()
                                && !self.recovery_unavailable
                                && !self.recovery.trim().is_empty())
                            .then_some(Msg::Verify),
                        ),
                    )
                    .push(text(self.verification_feedback()).size(14).color(BODY));
            }
        }
        form = form.push(if self.logged_in {
            self.sdk_verify_choices()
        } else {
            self.sdk_login_choices()
        });
        if !self.logged_in && !self.status.is_empty() {
            form = form.push(text(&self.status).size(14).color(MUTED));
        }
        if !self.logged_in {
            form = form.push(
                button(text("Back to preview").size(14))
                    .padding(0)
                    .style(plain(LINK, color!(0x5fc5ff)))
                    .on_press(Msg::Back),
            );
        }
        let backdrop = iced::Background::Gradient(
            iced::gradient::Linear::new(iced::Radians(2.4))
                .add_stop(0.0, color!(0x404eed))
                .add_stop(0.55, color!(0x2c2f8c))
                .add_stop(1.0, RAIL)
                .into(),
        );
        container(
            container(form)
                .padding(32)
                .width(480)
                .style(|_| surface(CHAT, 6.)),
        )
        .center(Fill)
        .style(move |_| container::Style {
            background: Some(backdrop),
            ..Default::default()
        })
        .into()
    }
    /// Discord-style User Settings: a category list on the left, the page on
    /// the right and a close control that returns to the conversation.
    fn settings_view(&self) -> El<'_> {
        let nav = column![
            container(label("USER SETTINGS")).padding([6, 10]),
            button(text("My Account").size(16))
                .width(Fill)
                .padding([6, 10])
                .style(|_, _| button::Style {
                    background: Some(ACTIVE.into()),
                    text_color: TEXT,
                    border: Border::default().rounded(4),
                    ..Default::default()
                }),
        ]
        .spacing(2)
        .width(218);
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
        let secondary = |_: &Theme, s: button::Status| button::Style {
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
        };
        let setting = |title: &'static str, detail: String, control: El<'static>| -> El<'static> {
            row![
                column![
                    text(title).size(16).font(BOLD).color(TEXT),
                    text(detail).size(14).color(MUTED)
                ]
                .spacing(4)
                .width(Fill),
                control
            ]
            .spacing(16)
            .align_y(Center)
            .into()
        };
        let mut page = column![text("My Account").size(20).font(BOLD).color(TEXT)].spacing(20);
        #[cfg(target_os = "linux")]
        if let Some(b) = self.incoming_banner() {
            page = page.push(b);
        }
        #[cfg(target_os = "linux")]
        if let Some(dock) = self.call_dock_compact() {
            page = page.push(dock);
        }
        let profile = column![
            container(Space::new())
                .height(72)
                .width(Fill)
                .style(|_| container::Style {
                    background: Some(BLURPLE.into()),
                    border: Border::default().rounded(border::top(8)),
                    ..Default::default()
                }),
            container(
                row![
                    container(face(
                        &self.profile,
                        self.avatars.get(self.own_avatar.as_deref()),
                        72.
                    ))
                    .padding(5)
                    .style(|_| surface(TRAY, 41.)),
                    column![
                        text(&self.profile).size(20).font(BOLD).color(TEXT),
                        text(&self.matrix_id).size(14).color(MUTED)
                    ]
                    .spacing(2)
                    .width(Fill),
                    container(
                        row![
                            icon(Icon::Shield, 14., GREEN),
                            text("Verified device").size(12).font(BOLD).color(GREEN)
                        ]
                        .spacing(4)
                        .align_y(Center)
                    )
                    .padding([4, 8])
                    .style(|_| surface(color!(0x23a55a, 0.12), 4.))
                ]
                .spacing(16)
                .align_y(Center)
            )
            .padding(16)
        ];
        page = page
            .push(container(profile).width(Fill).style(|_| surface(TRAY, 8.)))
            .push(profile::settings(self));
        let group = |heading: &'static str, rows: Vec<El<'static>>| -> El<'static> {
            column![
                label(heading),
                container(Column::with_children(rows).spacing(20))
                    .padding(16)
                    .width(Fill)
                    .style(|_| surface(SIDE, 8.))
            ]
            .spacing(8)
            .into()
        };
        let calls = setting(
            "Call server",
            if self.rtc_status.is_empty() {
                "Check whether your homeserver advertises MatrixRTC for calls.".into()
            } else {
                self.rtc_status.clone()
            },
            button(text("Check call server support").size(14))
                .padding([7, 14])
                .style(secondary)
                .on_press_maybe(self.connected.then_some(Msg::CheckCalls))
                .into(),
        );
        page = page.push(group("VOICE & VIDEO", vec![calls]));
        let public = self.read_sharing.public;
        let receipts = setting(
            "Read receipts",
            if public {
                "Room members can see what you’ve read.".into()
            } else {
                "Read receipts are private to your account.".into()
            },
            button(
                text(if public {
                    "Keep receipts private"
                } else {
                    "Share receipts with room members"
                })
                .size(14),
            )
            .padding([7, 14])
            .style(secondary)
            .on_press_maybe(
                (self.connected && !self.changing_read_sharing())
                    .then_some(Msg::ShareReads(!public)),
            )
            .into(),
        );
        let typing = self.typing_setting(&setting);
        page = page.push(group(
            "PRIVACY",
            vec![receipts, typing, self.blocked_list()],
        ));
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        {
            let dnd = self.alerts.dnd;
            let label = if self.changing_dnd() {
                "Saving…"
            } else {
                match dnd {
                    Some(false) => "Pause alerts",
                    Some(true) => "Resume alerts",
                    None => "Unavailable",
                }
            };
            let control = button(text(label).size(14))
                .padding([7, 14])
                .style(if dnd == Some(true) {
                    primary
                } else {
                    secondary
                })
                .on_press_maybe(
                    dnd.filter(|_| self.connected && !self.changing_dnd())
                        .map(|on| Msg::SetDnd(!on)),
                );
            page = page.push(group(
                "NOTIFICATIONS",
                vec![setting(
                    "Do Not Disturb",
                    "Pauses Matrix message notifications across your account and silences Untethered’s incoming ringtone. Calls can still be answered.".into(),
                    control.into(),
                )],
            ));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            page = page.push(group("SYSTEM TRAY", self.tray_settings(&setting)));
        }
        #[cfg(target_os = "linux")]
        {
            page = page.push(group("SOUNDS", self.sound_settings(&setting)));
        }
        page = page.push(self.sdk_account_settings());
        if !self.status.is_empty() {
            page = page.push(text(&self.status).size(14).color(MUTED));
        }
        let close = column![
            with_tip(
                button(container(icon(Icon::Close, 18., ICON)).center(36))
                    .padding(0)
                    .on_press(Msg::Back)
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
                "Back to rooms",
                tooltip::Position::Bottom
            ),
            text("ESC").size(13).font(BOLD).color(CHANNEL)
        ]
        .spacing(8)
        .align_x(Center);
        let right = container(
            row![
                scrollable(container(page.max_width(740)).padding(Padding {
                    top: 60.,
                    right: 20.,
                    bottom: 60.,
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
            .height(Fill),
        )
        .width(Length::FillPortion(2))
        .height(Fill)
        .style(|_| surface(CHAT, 0.));
        row![left, right].height(Fill).into()
    }
}
