//! The autocomplete popup and the composer's emoji picker, floating above
//! the field as on Discord.
use super::{
    ACTIVE, App, BOLD, CHANNEL, El, Icon, LINE, Msg, ROW_HOVER, SIDE, TEXT, face, icon, input_style,
};
use crate::autocomplete::{Candidate, Field, Input, Trigger};
use iced::{
    Alignment::Center,
    Border, Fill, Length, Padding, alignment, keyboard,
    widget::{Column, Space, Stack, button, container, opaque, row, text, text_editor, text_input},
};

impl App {
    /// Popup keys while `field`'s popup is open: the arrows move, Tab and
    /// Enter insert, Escape closes it and keeps the editor focused.
    pub(super) fn completion_key(
        &self,
        field: Field,
        press: &text_editor::KeyPress,
    ) -> Option<text_editor::Binding<Msg>> {
        use keyboard::key::Named;
        if !matches!(press.status, text_editor::Status::Focused { .. }) {
            return None;
        }
        let keyboard::Key::Named(key) = press.key.as_ref() else {
            return None;
        };
        let plain = press.modifiers.is_empty();
        let input = match key {
            Named::ArrowUp if plain => Input::Up,
            Named::ArrowDown if plain => Input::Down,
            Named::Tab | Named::Enter if plain => Input::Accept,
            Named::Escape => Input::Close,
            _ => return None,
        };
        self.completion().filter(|p| p.field == field)?;
        Some(text_editor::Binding::Custom(Msg::Autocomplete(input)))
    }

    /// The popup for `field`'s editor, if it has one open.
    pub(super) fn completion_popup(&self, field: Field) -> Option<El<'_>> {
        let p = self.completion().filter(|p| p.field == field)?;
        let title = match p.token.trigger {
            Trigger::Person => "MEMBERS".to_owned(),
            Trigger::Room => "CHANNELS".to_owned(),
            Trigger::Emoji => format!("EMOJI MATCHING :{}", p.token.query),
            Trigger::Command => "COMMANDS".to_owned(),
        };
        let mut rows = Column::new().spacing(2);
        for (i, c) in p.candidates.into_iter().enumerate() {
            let chosen = i == p.selected;
            let content: El = match c {
                Candidate::Person {
                    id, name, avatar, ..
                } => row![
                    face(&name, self.avatars.get(avatar.as_deref()), 24.),
                    text(name)
                        .size(15)
                        .font(BOLD)
                        .color(TEXT)
                        .wrapping(text::Wrapping::None),
                    container(
                        text(id)
                            .size(13)
                            .color(CHANNEL)
                            .wrapping(text::Wrapping::None)
                    )
                    .clip(true)
                    .width(Fill),
                ]
                .spacing(10)
                .align_y(Center)
                .into(),
                Candidate::Room { name, voice, .. } => row![
                    icon(if voice { Icon::Speaker } else { Icon::Hash }, 18., CHANNEL),
                    container(
                        text(name)
                            .size(15)
                            .color(TEXT)
                            .wrapping(text::Wrapping::None)
                    )
                    .clip(true)
                    .width(Fill),
                ]
                .spacing(8)
                .align_y(Center)
                .height(24)
                .into(),
                Candidate::Emoji(f) => row![
                    text(f.emoji).size(20),
                    text(format!(":{}:", f.shortcode)).size(15).color(TEXT),
                ]
                .spacing(10)
                .align_y(Center)
                .height(24)
                .into(),
                Candidate::Command(spec) => row![
                    text(format!("/{}", spec.name))
                        .size(15)
                        .font(BOLD)
                        .color(TEXT)
                        .wrapping(text::Wrapping::None),
                    container(
                        text(spec.about)
                            .size(13)
                            .color(CHANNEL)
                            .wrapping(text::Wrapping::None)
                    )
                    .clip(true)
                    .width(Fill),
                ]
                .spacing(10)
                .align_y(Center)
                .height(24)
                .into(),
                Candidate::Emote {
                    shortcode,
                    url,
                    pack,
                } => row![
                    self.emote_image(&url, 22.),
                    text(format!(":{shortcode}:"))
                        .size(15)
                        .color(TEXT)
                        .width(Fill),
                    text(pack.clone()).size(12).color(super::CHANNEL),
                ]
                .spacing(10)
                .align_y(Center)
                .height(24)
                .into(),
            };
            rows = rows.push(
                button(content)
                    .width(Fill)
                    .padding([5, 8])
                    .on_press(Msg::Autocomplete(Input::Pick(i)))
                    .style(move |_, s| button::Style {
                        background: if chosen {
                            Some(ACTIVE.into())
                        } else if s == button::Status::Hovered {
                            Some(ROW_HOVER.into())
                        } else {
                            None
                        },
                        border: Border::default().rounded(4),
                        ..Default::default()
                    }),
            );
        }
        let panel = Column::new()
            .push(
                row![
                    text(title).size(12).font(BOLD).color(CHANNEL),
                    Space::new().width(Fill),
                    text("Tab or Enter to insert · Esc to close")
                        .size(12)
                        .color(CHANNEL),
                ]
                .align_y(Center)
                .padding([0, 8]),
            )
            .push(rows)
            .spacing(6);
        Some(opaque(container(panel).padding(8).width(Fill).style(
            |_| container::Style {
                border: Border {
                    color: LINE,
                    width: 1.,
                    radius: 8.0.into(),
                },
                ..super::surface(SIDE, 8.)
            },
        )))
    }

    /// The emoji button's picker: picking inserts at the composer's cursor.
    fn composer_picker(&self) -> Option<El<'_>> {
        let search = self.completion.picker.as_deref()?;
        // Enter picks the best match: the room's custom emoji first.
        let first = self
            .emote_search(&self.selected, search, 1)
            .first()
            .map(|(_, i)| format!(":{}: ", i.shortcode))
            .filter(|_| !search.trim().is_empty())
            .or_else(|| {
                crate::emoji::search(search, 1)
                    .first()
                    .map(|f| f.emoji.to_owned())
            });
        let input = text_input("Search emoji", search)
            .id("emoji-search")
            .on_input(|s| Msg::Autocomplete(Input::PickerSearch(s)))
            .on_submit_maybe(first.map(|e| Msg::Autocomplete(Input::PickEmoji(e))))
            .size(14)
            .padding(7)
            .style(input_style);
        let panel = super::emoji_picker::panel(
            "EMOJI",
            search,
            input.into(),
            None,
            // A custom emoji stands as its shortcode, sent as its picture.
            self.emote_sections(
                &self.selected,
                search,
                |i| {
                    Some(Msg::Autocomplete(Input::PickEmoji(format!(
                        ":{}: ",
                        i.shortcode
                    ))))
                },
                |_| false,
            ),
            |e| Some(Msg::Autocomplete(Input::PickEmoji(e.into()))),
            |_| false,
        );
        Some(opaque(container(panel).width(360)))
    }

    /// The timeline with the composer's popup and emoji picker floating over
    /// its bottom edge, so opening them doesn't move the messages. The layers
    /// are always there, empty when closed: Iced keeps widget state by
    /// position, and a timeline that moved into a new parent would lose its
    /// scroll position, the picker its focused search field.
    pub(super) fn with_composer_overlays<'a>(&'a self, timeline: El<'a>) -> El<'a> {
        let float = |content: Option<El<'a>>, x: alignment::Horizontal| -> El<'a> {
            let Some(content) = content else {
                return Space::new().into();
            };
            container(content)
                .width(Fill)
                .height(Fill)
                .align_x(x)
                .align_y(alignment::Vertical::Bottom)
                .padding(Padding {
                    left: 16.,
                    right: 16.,
                    bottom: 8.,
                    top: 0.,
                })
                .into()
        };
        Stack::new()
            .push(timeline)
            .push(float(
                self.completion_popup(Field::Composer),
                alignment::Horizontal::Left,
            ))
            .push(float(self.composer_picker(), alignment::Horizontal::Right))
            .width(Fill)
            .height(Length::Fill)
            .into()
    }

    /// A message editor with the slot for its popup above it. The slot is
    /// always there so the editor keeps its place, and with it its focus.
    pub(super) fn with_completion_slot<'a>(&'a self, field: Field, editor: El<'a>) -> El<'a> {
        let slot: El = match self.completion_popup(field) {
            Some(popup) => container(popup).padding(Padding::ZERO.bottom(4)).into(),
            None => Space::new().into(),
        };
        Column::new().push(slot).push(editor).into()
    }
}
