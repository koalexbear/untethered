//! Search (search.rs) on screen: the scope, the query with its filters, what
//! was searched and how far back, and the hits, each opening at its message.
use super::{
    ACTIVE, App, BODY, BOLD, CHANNEL, El, Icon, LINK, MUTED, Msg, Room, SIDE, TEXT, face, icon,
    input_style, plain, primary, scroll_style, thin_scrollbar,
};
use crate::search::{Scope, Search};
use chrono::{DateTime, Local};
use iced::{
    Alignment::Center,
    Border, Color, Fill, color,
    widget::{Column, button, column, container, row, scrollable, text, text_input},
};

const WARN: Color = color!(0xffc785);

fn date(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .with_timezone(&Local)
        .format("%b %-d, %Y")
        .to_string()
}

impl App {
    pub(super) fn search_view<'a>(
        &'a self,
        s: &'a Search,
        room: &'a Room,
        opening: bool,
    ) -> El<'a> {
        let rooms = &self.rooms().list;
        let link = |label: String, msg: Option<Msg>| {
            button(text(label).size(14))
                .padding([4, 8])
                .style(plain(LINK, color!(0x5fc5ff)))
                .on_press_maybe(msg)
        };
        let chip = |label: String, scope: Scope| {
            let on = s.scope == scope;
            button(text(label).size(13).font(BOLD))
                .padding([5, 12])
                .on_press(Msg::SearchScope(scope))
                .style(move |_, st| button::Style {
                    background: Some(
                        if on {
                            color!(0x5865f2)
                        } else if st == button::Status::Hovered {
                            ACTIVE
                        } else {
                            SIDE
                        }
                        .into(),
                    ),
                    text_color: if on { Color::WHITE } else { BODY },
                    border: Border::default().rounded(12),
                    ..Default::default()
                })
        };
        let mut scopes = row![chip(
            if room.dm {
                room.name.clone()
            } else {
                format!("#{}", room.name)
            },
            Scope::Room
        )]
        .spacing(6);
        if let Some(space) = s.space.as_ref().and_then(|id| rooms.get(id)) {
            scopes = scopes.push(chip(space.name.clone(), Scope::Space));
        }
        scopes = scopes.push(chip("All rooms".into(), Scope::All));
        let mut panel = column![
            row![
                icon(Icon::Search, 20., color!(0x80848e)),
                text("Search").size(16).font(BOLD).color(TEXT),
                scopes.width(Fill),
                link("Return to timeline".into(), Some(Msg::ReturnToReading))
            ]
            .spacing(10)
            .align_y(Center),
            row![
                text_input("Search  ·  from:  in:  has:  before:  after:", &s.input)
                    .id("room-search-input")
                    .on_input(Msg::SearchInput)
                    .on_submit(Msg::RunSearch)
                    .size(16)
                    .padding(10)
                    .style(input_style),
                button(text("Find").size(14).font(BOLD))
                    .padding([10, 18])
                    .style(primary)
                    .on_press_maybe((!s.input.trim().is_empty()).then_some(Msg::RunSearch))
            ]
            .spacing(8)
            .align_y(Center),
            text(
                "from:name  ·  in:channel  ·  has:file, image, video, sound or link  ·  \
                 before: or after:2026-11-20"
            )
            .size(13)
            .color(MUTED),
        ]
        .spacing(12);
        for bad in &s.unknown {
            panel = panel.push(
                text(format!(
                    "“{bad}” isn't a filter Untethered knows, so it was left out."
                ))
                .size(13)
                .color(WARN),
            );
        }
        for who in &s.nobody {
            panel = panel.push(
                text(format!("No one in these rooms is called “{who}”."))
                    .size(13)
                    .color(WARN),
            );
        }
        if !s.term.is_empty() {
            let count = s.hits.len();
            panel = panel.push(
                text(if opening {
                    "Opening the message…".into()
                } else if s.indexing {
                    "Loading older encrypted history onto this device…".into()
                } else if s.failed {
                    "Search failed. Try Find again.".into()
                } else if s.busy {
                    "Searching…".into()
                } else if s.nowhere {
                    "No room here matches that in: filter.".into()
                } else if count == 0 {
                    format!("No matches for “{}”.", s.term)
                } else {
                    format!(
                        "{count} {} for “{}”{}",
                        if count == 1 { "match" } else { "matches" },
                        s.term,
                        if s.more { ", the newest shown" } else { "" }
                    )
                })
                .size(14)
                .font(BOLD)
                .color(BODY),
            );
        }
        if !s.busy && !s.reach.is_empty() {
            panel = panel.push(self.search_reach(s));
        }
        let many = s.scope != Scope::Room;
        let mut list = Column::new().spacing(8);
        for hit in &s.hits {
            let Some(r) = rooms.get(&hit.room) else {
                continue;
            };
            let sender = r.display(&hit.sender);
            let mut heading = row![text(sender).size(16).font(BOLD).color(TEXT)]
                .spacing(8)
                .align_y(Center);
            if many {
                heading = heading.push(
                    text(if r.dm {
                        r.name.clone()
                    } else {
                        format!("#{}", r.name)
                    })
                    .size(13)
                    .color(BODY),
                );
            }
            heading = heading.push(
                text(
                    DateTime::from_timestamp_millis(hit.time)
                        .unwrap_or_default()
                        .with_timezone(&Local)
                        .format("%b %-d, %Y %H:%M")
                        .to_string(),
                )
                .size(12)
                .color(CHANNEL),
            );
            list = list.push(
                button(
                    row![
                        face(
                            sender,
                            self.avatars
                                .get(r.avatars.get(&hit.sender).map(String::as_str)),
                            40.,
                        ),
                        column![heading, text(&hit.snippet).size(16).color(BODY)]
                            .spacing(3)
                            .width(Fill)
                    ]
                    .spacing(16),
                )
                .padding([12, 16])
                .width(Fill)
                .style(|_, st| button::Style {
                    background: Some(
                        if st == button::Status::Hovered {
                            ACTIVE
                        } else {
                            SIDE
                        }
                        .into(),
                    ),
                    border: Border::default().rounded(8),
                    ..Default::default()
                })
                .on_press(Msg::SearchHit(hit.room.clone(), hit.id.clone())),
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
        .width(Fill)
        .height(Fill)
        .padding(16)
        .into()
    }

    /// Which rooms were searched where, and how far back on this device.
    fn search_reach<'a>(&'a self, s: &'a Search) -> El<'a> {
        let rooms = &self.rooms().list;
        let name = |id: &str| {
            rooms.get(id).map_or_else(
                || id.to_owned(),
                |r| {
                    if r.dm {
                        r.name.clone()
                    } else {
                        format!("#{}", r.name)
                    }
                },
            )
        };
        let mut lines = Column::new().spacing(4);
        let server: Vec<String> = s
            .reach
            .iter()
            .filter(|r| r.server)
            .map(|r| name(&r.room))
            .collect();
        if !server.is_empty() {
            lines = lines.push(
                text(format!(
                    "On the homeserver, all history: {}",
                    server.join(", ")
                ))
                .size(13)
                .color(MUTED),
            );
        }
        let local: Vec<String> = s
            .reach
            .iter()
            .filter(|r| !r.server)
            .map(|r| match r.oldest {
                Some(t) => format!("{} back to {}", name(&r.room), date(t)),
                None => format!("{} (nothing loaded yet)", name(&r.room)),
            })
            .collect();
        if !local.is_empty() {
            lines = lines.push(
                row![
                    text(format!(
                        "Encrypted, on this device only: {}",
                        local.join(", ")
                    ))
                    .size(13)
                    .color(MUTED)
                    .width(Fill),
                    button(text("Load older history").size(13))
                        .padding([2, 8])
                        .style(plain(LINK, color!(0x5fc5ff)))
                        .on_press_maybe(
                            (self.connected && !s.indexing).then_some(Msg::IndexSearch)
                        ),
                ]
                .spacing(8)
                .align_y(Center),
            );
        }
        lines.into()
    }
}
