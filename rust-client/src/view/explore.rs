//! Explore (explore.rs), shaped like Discord's Explore Discoverable
//! Servers: a Discover sidebar with the kinds to list, a banner with the
//! search box, and the directory's spaces and rooms as cards. A card opens
//! the J2 preview over it (view/joining.rs). Also the rows, icons dimmed, for a
//! space's channels this account hasn't joined.
use super::{
    App, BLURPLE, BODY, BOLD, CHANNEL, CHAT, El, HOVER, Icon, LINE, MUTED, Msg, RAIL, SIDE, TEXT,
    face, icon, input_style, line_rule, nav, plain, primary, scroll_style, surface, thin_scrollbar,
    with_tip,
};
use crate::{
    engine::explore::{Kind, PublicRoom},
    explore::{Input, Page, SEARCH},
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Length, Padding, color,
    widget::{
        Column, Space, button, column, container, grid, row, scrollable, text, text_input, tooltip,
    },
};

const WARN: Color = color!(0xffc785);
/// The widest a card gets before the grid adds a column.
const CARD_WIDTH: f32 = 340.;
const CARD_HEIGHT: f32 = 196.;
/// Characters of a topic a card shows.
const TOPIC: usize = 110;

fn members(n: u64) -> String {
    format!("{n} {}", if n == 1 { "member" } else { "members" })
}

fn clipped(topic: &str) -> String {
    let flat = topic.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= TOPIC {
        return flat;
    }
    let cut: String = flat.chars().take(TOPIC).collect();
    format!("{}…", cut.trim_end())
}

impl App {
    /// The sidebar while Explore is open, in place of the channel list.
    pub(super) fn explore_side<'a>(&'a self, page: &'a Page) -> El<'a> {
        let head = container(text("Discover").font(BOLD).size(16).color(TEXT))
            .padding([0, 16])
            .center_y(48)
            .width(Fill);
        let mut kinds = Column::new().spacing(2).padding([8, 8]);
        for (kind, glyph, label) in [
            (Kind::All, Icon::Compass, "Everything"),
            (Kind::Spaces, Icon::Members, "Spaces"),
            (Kind::Rooms, Icon::Hash, "Rooms"),
        ] {
            let active = page.kind == kind;
            kinds = kinds.push(
                button(
                    row![
                        icon(glyph, 20., if active { TEXT } else { CHANNEL }),
                        text(label).size(16)
                    ]
                    .spacing(8)
                    .align_y(Center),
                )
                .width(Fill)
                .padding([8, 10])
                .style(nav(active))
                .on_press(Msg::Explore(Input::Kind(kind))),
            );
        }
        let sidebar = column![
            head,
            line_rule(color!(0x1f2023)),
            kinds,
            Space::new().height(Fill)
        ];
        #[cfg(target_os = "linux")]
        let sidebar = sidebar.extend(self.call_dock());
        container(sidebar.push(self.user_panel()))
            .width(240)
            .height(Fill)
            .style(|_| surface(SIDE, 0.))
            .into()
    }

    /// The main area while Explore is open and nothing is previewed.
    pub(super) fn explore_page<'a>(&'a self, page: &'a Page) -> El<'a> {
        let server = self
            .matrix_id
            .split_once(':')
            .map_or("your homeserver", |(_, s)| s);
        let (title, subtitle) = match page.kind {
            Kind::All => (
                "Find your community on Matrix",
                format!("Public spaces and rooms on {server}."),
            ),
            Kind::Spaces => (
                "Find a space to join",
                format!("Public spaces on {server}: communities with channels of their own."),
            ),
            Kind::Rooms => ("Find a room to join", format!("Public rooms on {server}.")),
        };
        let search = text_input("Explore spaces and rooms", &page.query)
            .id(SEARCH)
            .size(16)
            .padding([12, 14])
            .style(|theme, status| text_input::Style {
                background: Color::WHITE.into(),
                value: color!(0x060607),
                placeholder: color!(0x5c5e66),
                border: Border::default().rounded(6),
                ..input_style(theme, status)
            })
            .on_input(|q| Msg::Explore(Input::Query(q)));
        let banner = container(
            column![
                text(title).size(26).font(BOLD).color(Color::WHITE),
                text(subtitle).size(15).color(color!(0xe0e3ff)),
                container(search).max_width(560).padding(Padding {
                    top: 8.,
                    ..Padding::ZERO
                }),
            ]
            .spacing(6)
            .align_x(Center),
        )
        .padding([32, 24])
        .width(Fill)
        .center_x(Fill)
        .style(|_| surface(color!(0x404eed), 8.));

        let searching = self.searching_directory();
        let term = page.query.trim();
        let current = page
            .shown
            .as_ref()
            .is_some_and(|(t, k)| t == term && *k == page.kind);
        let mut body = Column::new().spacing(16);
        let heading = match (&page.shown, page.total) {
            (None, _) => None,
            _ if page.rooms.is_empty() => None,
            // The homeserver's estimate counts its whole directory, so
            // it only says how many more pages may hold.
            (_, Some(total)) if page.next.is_some() && total as usize > page.rooms.len() => {
                Some(format!("{} of about {total}", page.rooms.len()))
            }
            _ => Some(match page.rooms.len() {
                1 => "1 result".to_owned(),
                n => format!("{n} results"),
            }),
        };
        let heading = if term.is_empty() {
            heading.map(|h| format!("Popular on {server} · {h}"))
        } else {
            heading
        };
        if let Some(h) = heading {
            body = body.push(text(h).size(13).font(BOLD).color(CHANNEL));
        }
        if page.rooms.is_empty() {
            let hint = if page.failed {
                "Couldn't reach your homeserver's room directory."
            } else if searching || page.shown.is_none() {
                "Searching…"
            } else if term.is_empty() {
                "Your homeserver doesn't list any public spaces or rooms yet."
            } else {
                "Nothing public matches that. Check the spelling, or try another word."
            };
            body = body.push(
                text(hint)
                    .size(15)
                    .color(if page.failed { WARN } else { BODY }),
            );
        } else {
            let cards = page.rooms.iter().map(|r| self.explore_card(r));
            body = body.push(
                grid(cards)
                    .fluid(CARD_WIDTH)
                    .spacing(16)
                    .height(Length::Shrink),
            );
        }
        if page.failed {
            body = body.push(
                button(text("Try again").size(14).font(BOLD))
                    .padding([8, 16])
                    .style(primary)
                    .on_press(Msg::Explore(Input::Retry)),
            );
        } else if current && page.next.is_some() {
            let more = self.loading_more();
            body = body.push(
                container(
                    button(
                        text(if more { "Loading…" } else { "Show more" })
                            .size(14)
                            .font(BOLD),
                    )
                    .padding([8, 16])
                    .style(primary)
                    .on_press_maybe((!more).then_some(Msg::Explore(Input::More))),
                )
                .center_x(Fill),
            );
        } else if searching && !page.rooms.is_empty() {
            body = body.push(text("Searching…").size(13).color(MUTED));
        }
        let content = column![banner, body].spacing(24).padding([24, 24]);
        container(
            scrollable(content)
                .direction(thin_scrollbar())
                .style(scroll_style(None))
                .height(Fill)
                .width(Fill),
        )
        .width(Fill)
        .height(Fill)
        .style(|_| surface(CHAT, 0.))
        .into()
    }

    /// A result: picture, name, kind, topic and members; a click opens it
    /// or its preview.
    fn explore_card<'a>(&'a self, r: &'a PublicRoom) -> El<'a> {
        let joined = self.live.list.contains_key(&r.room_id);
        let title = r.title();
        let kind = match (r.space, r.voice) {
            (true, _) => "Space",
            (_, true) => "Voice room",
            _ => "Room",
        };
        let sub = match r
            .alias
            .as_deref()
            .filter(|a| r.name.is_some() && *a != title)
        {
            Some(alias) => format!("{kind} · {alias}"),
            None => kind.to_owned(),
        };
        let head = row![
            face(title, self.avatars.get(r.avatar.as_deref()), 48.),
            column![
                text(title)
                    .size(16)
                    .font(BOLD)
                    .color(TEXT)
                    .wrapping(text::Wrapping::None),
                text(sub)
                    .size(12)
                    .color(CHANNEL)
                    .wrapping(text::Wrapping::None),
            ]
            .spacing(2)
            .width(Fill)
            .clip(true),
        ]
        .spacing(12)
        .align_y(Center);
        let topic = text(r.topic.as_deref().map(clipped).unwrap_or_default())
            .size(14)
            .color(BODY)
            .height(Fill);
        let mut foot = row![
            container(Space::new())
                .width(8)
                .height(8)
                .style(|_| surface(CHANNEL, 4.)),
            text(members(r.members)).size(12).color(CHANNEL).width(Fill),
        ]
        .spacing(6)
        .align_y(Center);
        if joined {
            foot = foot.push(
                container(text("Joined").size(12).font(BOLD).color(TEXT))
                    .padding([2, 8])
                    .style(|_| surface(color!(0x248046), 9.)),
            );
        }
        let card = button(column![head, topic, foot].spacing(12))
            .padding(16)
            .width(Fill)
            .height(CARD_HEIGHT)
            .on_press(Msg::Explore(Input::Pick(r.room_id.clone())))
            .style(|_, s| button::Style {
                background: Some(
                    if s == button::Status::Hovered {
                        RAIL
                    } else {
                        SIDE
                    }
                    .into(),
                ),
                text_color: TEXT,
                border: Border {
                    color: if s == button::Status::Hovered {
                        BLURPLE
                    } else {
                        LINE
                    },
                    width: 1.,
                    radius: 8.0.into(),
                },
                ..Default::default()
            });
        card.into()
    }

    /// A channel in the open space this account hasn't joined: its icon dimmed
    /// (its name stays as readable as a joined channel's), and
    /// a click previews it with Join.
    pub(super) fn unjoined_row<'a>(&self, r: &'a PublicRoom) -> El<'a> {
        let entry = row![
            icon(
                if r.voice { Icon::Speaker } else { Icon::Hash },
                20.,
                color!(0x4e5058)
            ),
            text(r.title())
                .size(16)
                .wrapping(text::Wrapping::None)
                .width(Fill),
        ]
        .spacing(6)
        .align_y(Center);
        let link = button(entry)
            .width(Fill)
            .padding([6, 8])
            .on_press(Msg::Explore(Input::Channel(r.room_id.clone())))
            .style(|theme, s| button::Style {
                background: (s == button::Status::Hovered).then(|| HOVER.into()),
                border: Border::default().rounded(4),
                ..plain(CHANNEL, BODY)(theme, s)
            });
        row![
            Space::new().width(8),
            with_tip(
                link,
                "Not joined yet: preview and join",
                tooltip::Position::Right
            )
        ]
        .align_y(Center)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_topics_are_cut_with_an_ellipsis() {
        assert_eq!(clipped("A  short\ntopic"), "A short topic");
        let long = "word ".repeat(40);
        let cut = clipped(&long);
        assert!(cut.ends_with('…'));
        assert!(cut.chars().count() <= TOPIC + 1);
        assert_eq!(members(1), "1 member");
        assert_eq!(members(12), "12 members");
    }
}
