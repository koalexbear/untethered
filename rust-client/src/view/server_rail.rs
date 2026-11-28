//! The server rail's servers and folders (ROADMAP N8, rail_order.rs).
//!
//! Every entry is a row of a marker column and an icon column, the same
//! for a server and for a folder open or closed: an open folder adds its
//! servers to both columns and tints the icon column. Each icon sits in a
//! drag slot (drag.rs) that lifts it and tells which part of it the pointer
//! is over; the drop line and the lit drop target are drawn without adding
//! rows, so Iced keeps every entry's state where it was.
use super::{
    App, BLURPLE, BODY, BOLD, CHANNEL, CHAT, El, FLOAT, MUTED, Mark, Msg, RAIL, SIDE, TEXT, badge,
    drag::{self, Look, Report},
    face,
    icons::{Icon, hover_icon, with_tip},
    input_style, pill, plain, popover, primary, surface,
};
use crate::{
    Section,
    rail_order::{self, COLORS, Entry, FIELD, Held, Input, Spot, Zone},
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Padding, color,
    widget::{
        Column, Space, button, column, container, mouse_area, opaque, row, stack, text, text_input,
        tooltip,
    },
};

/// Icons are this big, 8 px apart.
const ICON: f32 = 48.;
const GAP: f32 = 8.;

fn rgb(color: u32) -> Color {
    Color::from_rgb8((color >> 16) as u8, (color >> 8) as u8, color as u8)
}

/// How an icon looks.
#[derive(Clone, Copy, Default)]
struct Style {
    /// It's what's open.
    active: bool,
    /// A drop would make a folder of it or put the server in it.
    lit: bool,
    /// A folder's colour behind it, instead of the rail's.
    tint: Option<Color>,
}

/// A rail entry: the server's icon, or its initials while it has none.
pub(super) fn rail_button<'a>(
    glyph: String,
    picture: Option<&iced::widget::image::Handle>,
    hint: &'a str,
    active: bool,
    msg: Msg,
    mark: Mark,
) -> El<'a> {
    let marker = marker(if active {
        40.
    } else if mark.unread {
        8.
    } else {
        0.
    });
    let icon = icon(
        initials_face(glyph, picture),
        hint,
        Style {
            active,
            ..Style::default()
        },
        Some(msg),
        mark,
    );
    row![marker, icon].spacing(GAP).align_y(Center).into()
}

fn initials_face<'a>(glyph: String, picture: Option<&iced::widget::image::Handle>) -> El<'a> {
    let size = if glyph.chars().count() > 2 { 15 } else { 18 };
    match picture {
        Some(p) => iced::widget::image(p.clone())
            .width(ICON)
            .height(ICON)
            .into(),
        None => text(glyph).font(BOLD).size(size).into(),
    }
}

/// The white marker left of an icon: 40 px for what's open, 8 for unread.
fn marker<'a>(height: f32) -> El<'a> {
    container(pill(height)).center_y(ICON).into()
}

/// An icon button with its mention badge and tooltip.
fn icon<'a>(
    face: El<'a>,
    hint: impl text::IntoFragment<'a>,
    style: Style,
    msg: Option<Msg>,
    mark: Mark,
) -> El<'a> {
    let Style { active, lit, tint } = style;
    let b = button(container(face).center(ICON))
        .padding(0)
        .on_press_maybe(msg)
        .style(move |_, s| {
            let highlighted = active || lit || s == button::Status::Hovered;
            button::Style {
                background: Some(
                    match tint {
                        Some(tint) => tint,
                        None if highlighted => BLURPLE,
                        None => CHAT,
                    }
                    .into(),
                ),
                text_color: if highlighted { Color::WHITE } else { BODY },
                border: Border {
                    color: Color::WHITE,
                    width: if lit { 2. } else { 0. },
                    radius: if highlighted || tint.is_some() {
                        16
                    } else {
                        24
                    }
                    .into(),
                },
                ..Default::default()
            }
        });
    let icon: El = if mark.mentions > 0 {
        stack![
            b,
            container(
                container(badge(mark.mentions))
                    .padding(3)
                    .style(|_| surface(RAIL, 11.))
            )
            .align_right(ICON)
            .align_bottom(ICON)
        ]
        .into()
    } else {
        b.into()
    };
    with_tip(icon, hint, tooltip::Position::Right)
}

impl App {
    /// The servers and folders under Direct Messages and All Rooms.
    pub(super) fn server_rail<'a>(&'a self, mut icons: Column<'a, Msg>) -> Column<'a, Msg> {
        let mark = self.rail_landing().map(|plan| plan.mark);
        for entry in self.rail_entries() {
            icons = icons.push(match entry {
                Entry::Server(space) => {
                    let (marker, icon) = self.server_icon(space, None, mark.as_ref());
                    row![marker, icon].spacing(GAP).align_y(Center).into()
                }
                Entry::Folder { id, servers } => self.folder(id, servers, mark.as_ref()),
            });
        }
        icons
    }

    /// What's open is the rail's to mark: no invite card or Explore over it.
    fn shown(&self, section: &Section) -> bool {
        self.section == *section && self.invite_open.is_none() && self.explore.page.is_none()
    }

    /// A server's marker and its icon in a drag slot; `folder`: the open
    /// folder it's in.
    fn server_icon<'a>(
        &'a self,
        space: String,
        folder: Option<&str>,
        mark: Option<&rail_order::Mark>,
    ) -> (El<'a>, El<'a>) {
        let store = self.rooms();
        let Some((id, r)) = store.list.get_key_value(&space) else {
            return (marker(0.), Space::new().width(ICON).height(ICON).into());
        };
        let section = Section::Space(id.clone());
        let unread = self.rail_mark(&section);
        let active = self.shown(&section);
        let spot = Spot::Server {
            space: space.clone(),
            folder: folder.map(str::to_owned),
        };
        let initials = r
            .name
            .split_whitespace()
            .take(3)
            .filter_map(|s| s.chars().next())
            .collect::<String>()
            .to_uppercase();
        let lit = mark == Some(&rail_order::Mark::Into(spot.clone()));
        let button = icon(
            initials_face(initials, self.avatars.get(r.avatar.as_deref())),
            r.name.as_str(),
            Style {
                active,
                lit,
                tint: None,
            },
            Some(Msg::Section(section)),
            unread,
        );
        // Dropped in a server's middle, another makes a folder with it;
        // servers in folders take drops above or below.
        let zones = match self.rail_held() {
            Some(Held::Server(_)) if folder.is_none() => 3,
            _ => 2,
        };
        let held = self.rail_held() == Some(&Held::Server(space.clone()));
        let slot = self.rail_slot(button, Held::Server(space), spot.clone(), zones, held, mark);
        let height = if active {
            40.
        } else if unread.unread {
            8.
        } else {
            0.
        };
        (marker(height), slot)
    }

    /// An icon in a drag slot: lifted as `held`, dropped on as `spot`.
    fn rail_slot<'a>(
        &self,
        content: El<'a>,
        held: Held,
        spot: Spot,
        zones: u8,
        lifted: bool,
        mark: Option<&rail_order::Mark>,
    ) -> El<'a> {
        let line = match mark {
            Some(rail_order::Mark::Line(s, below)) if *s == spot => Some(*below),
            _ => None,
        };
        let report = self.rail_held().is_some().then(|| {
            let report: Report<'a> = Box::new(move |zone| {
                Msg::Rail(match zone {
                    Some(index) => Input::Over(spot.clone(), Zone::of(index, zones)),
                    None => Input::Leave(spot.clone()),
                })
            });
            (zones, report)
        });
        drag::slot(
            content,
            self.may_rearrange().then_some(Msg::Rail(Input::Lift(held))),
            report,
            Look {
                dim: lifted.then_some(Color { a: 0.7, ..RAIL }),
                line,
                inset: 0.,
                gap: GAP / 2.,
            },
        )
    }

    /// A folder: its icon, then its servers while it's open, on its colour.
    fn folder<'a>(
        &'a self,
        id: String,
        servers: Vec<String>,
        mark: Option<&rail_order::Mark>,
    ) -> El<'a> {
        let open = self.rail.open.contains(&id);
        let folder = self.rail_folders().iter().find(|f| f.id == id);
        let color = rgb(folder.map_or(crate::engine::rail::DEFAULT_COLOR, |f| f.color));
        let store = self.rooms();
        let names: Vec<&str> = servers
            .iter()
            .filter_map(|s| store.list.get(s).map(|r| r.name.as_str()))
            .collect();
        let hint = match folder.map(|f| f.name.as_str()) {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => names.join(", "),
        };
        // A closed folder counts its servers' unreads and mentions.
        let mut sum = Mark::default();
        for s in &servers {
            let m = self.rail_mark(&Section::Space(s.clone()));
            sum.unread |= m.unread;
            sum.mentions += m.mentions;
        }
        let holds_open = servers
            .iter()
            .any(|s| self.shown(&Section::Space(s.clone())));
        let face: El = if open {
            hover_icon(Icon::Folder, 26., ICON, color, color)
        } else {
            // Its first four servers, small.
            let small = |i: usize| -> El {
                servers.get(i).map_or_else(
                    || Space::new().width(18).height(18).into(),
                    |s| {
                        let r = store.list.get(s);
                        face(
                            r.map_or("", |r| r.name.as_str()),
                            self.avatars.get(r.and_then(|r| r.avatar.as_deref())),
                            18.,
                        )
                    },
                )
            };
            column![
                row![small(0), small(1)].spacing(4),
                row![small(2), small(3)].spacing(4)
            ]
            .spacing(4)
            .into()
        };
        let spot = Spot::Folder(id.clone());
        let lit = mark == Some(&rail_order::Mark::Into(spot.clone()));
        let heading = icon(
            face,
            hint,
            Style {
                active: false,
                lit,
                tint: Some(if open {
                    Color::TRANSPARENT
                } else {
                    Color { a: 0.4, ..color }
                }),
            },
            Some(Msg::Rail(Input::Toggle(id.clone()))),
            if open { Mark::default() } else { sum },
        );
        // Right-click: its menu.
        let heading = mouse_area(heading).on_right_press(Msg::Rail(Input::Menu(Some(id.clone()))));
        let menu = (self.rail.menu.as_deref() == Some(id.as_str())).then(|| self.folder_menu(&id));
        let heading = popover::popover(
            heading,
            menu,
            popover::Place::Below,
            Msg::Rail(Input::Menu(None)),
        );
        let held = self.rail_held() == Some(&Held::Folder(id.clone()));
        // A server dropped in a closed folder's middle goes in it.
        let zones = match self.rail_held() {
            Some(Held::Server(_)) => 3,
            _ => 2,
        };
        let heading = self.rail_slot(heading, Held::Folder(id.clone()), spot, zones, held, mark);
        let marker_height = if !open && holds_open {
            40.
        } else if !open && sum.unread {
            8.
        } else {
            0.
        };
        let mut markers = Column::new().spacing(GAP).push(marker(marker_height));
        let mut icons = Column::new().spacing(GAP).push(heading);
        if open {
            for space in servers {
                let (marker, icon) = self.server_icon(space, Some(&id), mark);
                markers = markers.push(marker);
                icons = icons.push(icon);
            }
        }
        let tinted = container(icons).style(move |_| container::Style {
            background: open.then(|| Color { a: 0.25, ..color }.into()),
            border: Border::default().rounded(16),
            ..Default::default()
        });
        row![markers, tinted].spacing(GAP).into()
    }

    /// A folder's right-click menu.
    fn folder_menu<'a>(&'a self, id: &str) -> El<'a> {
        let item = button(
            row![
                text("Folder Settings").size(14).width(Fill),
                hover_icon(Icon::Gear, 18., 18., MUTED, Color::WHITE)
            ]
            .align_y(Center),
        )
        .width(Fill)
        .padding([7, 8])
        .on_press(Msg::Rail(Input::Settings(Some(id.to_owned()))))
        .style(|_, s| button::Style {
            background: (s == button::Status::Hovered).then(|| color!(0x4752c4).into()),
            text_color: if s == button::Status::Hovered {
                Color::WHITE
            } else {
                MUTED
            },
            border: Border::default().rounded(3),
            ..Default::default()
        });
        container(item)
            .width(200)
            .padding(6)
            .style(|_| container::Style {
                border: Border {
                    color: color!(0x2b2d31),
                    width: 1.,
                    radius: 6.0.into(),
                },
                ..surface(FLOAT, 6.)
            })
            .into()
    }
}

/// Folder Settings: the folder's name and colour.
pub(super) fn folder_dialog(app: &App) -> Option<El<'_>> {
    let settings = app.rail.settings.as_ref()?;
    let close = Msg::Rail(Input::Settings(None));
    let save = Msg::Rail(Input::Save);
    let store = app.rooms();
    let names: Vec<&str> = app
        .rail_entries()
        .into_iter()
        .find_map(|e| match e {
            Entry::Folder { id, servers } if id == settings.folder => Some(servers),
            _ => None,
        })
        .unwrap_or_default()
        .iter()
        .filter_map(|s| store.list.get(s).map(|r| r.name.as_str()))
        .collect();
    let placeholder = names.join(", ");
    let swatch = |c: u32| {
        let chosen = settings.color == c;
        let tint = rgb(c);
        button(
            container(if chosen {
                hover_icon(Icon::Check, 16., 28., Color::WHITE, Color::WHITE)
            } else {
                Space::new().width(28).height(28).into()
            })
            .center(28),
        )
        .padding(0)
        .on_press(Msg::Rail(Input::Color(c)))
        .style(move |_, s| button::Style {
            background: Some(tint.into()),
            border: Border {
                color: Color::WHITE,
                width: if chosen || s == button::Status::Hovered {
                    2.
                } else {
                    0.
                },
                radius: 6.0.into(),
            },
            ..Default::default()
        })
    };
    let swatches = COLORS
        .iter()
        .fold(row![].spacing(8), |r, c| r.push(swatch(*c)))
        .wrap();
    let body = column![
        text("Folder Settings").size(20).font(BOLD).color(TEXT),
        column![
            text("FOLDER NAME").size(12).font(BOLD).color(CHANNEL),
            text_input(&placeholder, &settings.name)
                .id(FIELD)
                .size(15)
                .padding([10, 10])
                .style(input_style)
                .on_input(|t| Msg::Rail(Input::Name(t)))
                .on_submit(save.clone()),
        ]
        .spacing(8),
        column![
            text("FOLDER COLOR").size(12).font(BOLD).color(CHANNEL),
            swatches,
        ]
        .spacing(8),
        text("Only Untethered shows folders. Other Matrix apps such as Element show these servers in the same order.")
            .size(13)
            .color(MUTED),
    ]
    .spacing(16);
    let footer = container(
        row![
            Space::new().width(Fill),
            button(text("Cancel").size(14))
                .padding([10, 16])
                .style(plain(BODY, Color::WHITE))
                .on_press(close.clone()),
            button(text("Done").size(14).font(BOLD))
                .padding([10, 18])
                .style(primary)
                .on_press_maybe(app.may_rearrange().then_some(save)),
        ]
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
        .width(440)
        .style(|_| surface(CHAT, 8.));
    let backdrop =
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Color::from_rgba8(0, 0, 0, 0.7).into()),
                    ..Default::default()
                }),
        )
        .on_press(close);
    Some(
        stack![
            opaque(backdrop),
            container(dialog).center(Fill).padding(Padding::ZERO)
        ]
        .into(),
    )
}
