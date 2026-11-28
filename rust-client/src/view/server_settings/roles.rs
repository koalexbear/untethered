//! The Roles tab of Server Settings (roles.rs): the roles, who has them,
//! and what each role may do in the server and in each channel.
//!
//! Every slot is emitted whether or not it shows anything, and the role
//! form sits above the lists that grow, so a text field never moves
//! (Iced keeps state by position).
use super::{caption, secondary, segment, small_button};
use crate::{
    roles::{
        COLORS, Choice, FILTER_FIELD, Input, LEVEL_FIELD, MODERATOR, NAME_FIELD, Perm, Place,
        ROLES, SHOWN, View, assign_msg, msg, perm_msg,
    },
    server_settings::Page,
    view::{
        App, BLURPLE, BODY, BOLD, CHANNEL, El, Icon, LINE, MUTED, RAIL, SIDE, TEXT, icon,
        icon_button, input_style, label, label_owned, line_rule, plain, primary, surface,
    },
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Theme, color,
    widget::{
        Column, Space, button, column, container, overlay::menu, pick_list, row, text, text_input,
        tooltip,
    },
};

const WARN: Color = color!(0xffc785);
const ADMIN_TINT: Color = color!(0xf47067);
const MOD_TINT: Color = color!(0x3ccfae);

fn rgb(c: u32) -> Color {
    Color::from_rgb8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn picker_style(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    let open = !matches!(status, pick_list::Status::Active);
    pick_list::Style {
        text_color: TEXT,
        placeholder_color: CHANNEL,
        handle_color: MUTED,
        background: RAIL.into(),
        border: Border {
            color: if open { BLURPLE } else { LINE },
            width: 1.,
            radius: 4.0.into(),
        },
    }
}

fn menu_style(_: &Theme) -> menu::Style {
    menu::Style {
        background: color!(0x111214).into(),
        border: Border {
            color: LINE,
            width: 1.,
            radius: 4.0.into(),
        },
        text_color: BODY,
        selected_text_color: Color::WHITE,
        selected_background: BLURPLE.into(),
        shadow: Default::default(),
    }
}

/// A role picker, or the role as text where it can't be changed here.
fn picker<'a>(
    choices: Vec<Choice>,
    current: Choice,
    on: Option<Box<dyn Fn(Choice) -> crate::Msg + 'a>>,
) -> El<'a> {
    match on {
        Some(on) => pick_list(choices, Some(current), on)
            .text_size(14)
            .padding([6, 10])
            .width(190)
            .style(picker_style)
            .menu_style(menu_style)
            .into(),
        None => container(text(current.label).size(14).color(MUTED))
            .padding([6, 10])
            .width(190)
            .into(),
    }
}

fn dot<'a>(tint: Color) -> El<'a> {
    container(Space::new())
        .width(12)
        .height(12)
        .style(move |_| container::Style {
            background: Some(tint.into()),
            border: Border::default().rounded(6),
            ..Default::default()
        })
        .into()
}

impl App {
    pub(in crate::view) fn roles_tab<'a>(&'a self, page: &'a Page) -> El<'a> {
        let intro = text(
            "Roles are Matrix power levels, which every client shows and the server enforces: \
             Admin (100), Moderator (50), and Member for everyone. Custom roles name the levels \
             between; only Untethered shows their names and colours.",
        )
        .size(14)
        .color(MUTED);
        let mut views = row![].spacing(2);
        for view in View::ALL {
            let chosen = page.roles.view == view;
            views = views.push(segment(
                view.label(),
                chosen,
                (!chosen).then(|| msg(Input::View(view))),
            ));
        }
        let views = container(views).padding(2).style(|_| surface(RAIL, 4.));
        let body = match page.roles.view {
            View::Roles => self.role_list(page),
            View::Members => self.role_member_list(page),
            View::Permissions => self.permission_list(page),
        };
        column![intro, views, body].spacing(20).into()
    }

    fn role_list<'a>(&'a self, page: &'a Page) -> El<'a> {
        let space = &page.space;
        let busy = self.roles_busy();
        let everyone = self.everyone_in(space);
        let members = page.snapshot.as_ref().map(|s| &s.members);
        let levels: Vec<i64> = members
            .into_iter()
            .flatten()
            .map(|m| self.level_in(space, &m.id).unwrap_or(everyone))
            .collect();
        // (level, name, colour, custom)
        let mut roles: Vec<(i64, String, Color, bool)> = vec![
            (100, "Admin".into(), ADMIN_TINT, false),
            (MODERATOR, "Moderator".into(), MOD_TINT, false),
        ];
        roles.extend(
            self.titles(space)
                .into_iter()
                .flat_map(|t| t.iter())
                .map(|t| (t.level, t.name.clone(), rgb(t.color), true)),
        );
        roles.sort_by_key(|r| std::cmp::Reverse(r.0));
        if levels.iter().any(|l| *l >= crate::permissions::MAX) {
            roles.insert(
                0,
                (
                    crate::permissions::MAX,
                    "Owner".into(),
                    color!(0xf1c40f),
                    false,
                ),
            );
        }
        roles.push((everyone, "Member".into(), BODY, false));

        let form: El = match &page.roles.form {
            Some(f) => self.role_form(f, busy),
            None if self.may_set(space, ROLES) => small_button(
                "Create Role",
                primary,
                (!busy).then(|| msg(Input::Edit(None))),
            ),
            None => caption("Someone with permission to manage roles can add custom roles."),
        };
        let mut list = column![
            label_owned(format!("ROLES — {}", roles.len())),
            form,
            row![
                text("ROLE").size(12).font(BOLD).color(CHANNEL).width(Fill),
                text("POWER LEVEL")
                    .size(12)
                    .font(BOLD)
                    .color(CHANNEL)
                    .width(110),
                text("MEMBERS").size(12).font(BOLD).color(CHANNEL).width(80),
                Space::new().width(96),
            ]
            .spacing(12),
            line_rule(LINE),
        ]
        .spacing(10);
        for (level, name, tint, custom) in roles {
            let owner = level >= crate::permissions::MAX;
            let count = if level == everyone {
                levels.iter().filter(|l| **l <= everyone).count()
            } else {
                levels.iter().filter(|l| **l == level).count()
            };
            let source = if custom {
                "Untethered name"
            } else if owner {
                "Creators of this server: above every role"
            } else if level == everyone {
                "Everyone"
            } else {
                "Matrix role"
            };
            let editable = custom && !busy && self.may_title(space, level);
            let actions: El = if editable {
                row![
                    button(text("Edit").size(13))
                        .padding([4, 8])
                        .style(plain(MUTED, Color::WHITE))
                        .on_press(msg(Input::Edit(Some(level)))),
                    icon_button(
                        Icon::Close,
                        "Remove role",
                        false,
                        Some(msg(Input::Remove(level))),
                        tooltip::Position::Left,
                    ),
                ]
                .spacing(8)
                .align_y(Center)
                .into()
            } else {
                Space::new().into()
            };
            list = list.push(
                row![
                    row![
                        dot(tint),
                        column![
                            text(name).size(15).font(BOLD).color(tint),
                            text(source).size(12).color(CHANNEL),
                        ]
                        .spacing(2)
                    ]
                    .spacing(10)
                    .align_y(Center)
                    .width(Fill),
                    text(if owner {
                        "—".to_owned()
                    } else {
                        level.to_string()
                    })
                    .size(14)
                    .color(BODY)
                    .width(110),
                    text(if members.is_some() {
                        count.to_string()
                    } else {
                        String::new()
                    })
                    .size(14)
                    .color(BODY)
                    .width(80),
                    container(actions).width(96),
                ]
                .spacing(12)
                .align_y(Center),
            );
        }
        list.into()
    }

    fn role_form<'a>(&'a self, f: &'a crate::roles::Form, busy: bool) -> El<'a> {
        let mut swatches = row![].spacing(6);
        for c in COLORS {
            let chosen = f.color == c;
            swatches = swatches.push(
                button(Space::new().width(24).height(24))
                    .padding(0)
                    .on_press(msg(Input::Color(c)))
                    .style(move |_, _| button::Style {
                        background: Some(rgb(c).into()),
                        border: Border {
                            color: if chosen { Color::WHITE } else { rgb(c) },
                            width: 2.,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    }),
            );
        }
        let title = if f.from.is_some() {
            "EDIT ROLE"
        } else {
            "CREATE ROLE"
        };
        container(
            column![
                label(title),
                row![
                    column![
                        label("ROLE NAME"),
                        text_input("New role", &f.name)
                            .id(NAME_FIELD)
                            .on_input(|v| msg(Input::Name(v)))
                            .on_submit(msg(Input::Save))
                            .size(15)
                            .padding(8)
                            .style(input_style),
                    ]
                    .spacing(6)
                    .width(Fill),
                    column![
                        label("POWER LEVEL"),
                        text_input("1–99", &f.level)
                            .id(LEVEL_FIELD)
                            .on_input(|v| msg(Input::Level(v)))
                            .on_submit(msg(Input::Save))
                            .size(15)
                            .padding(8)
                            .width(110)
                            .style(input_style),
                    ]
                    .spacing(6),
                ]
                .spacing(12),
                column![label("ROLE COLOUR"), swatches].spacing(6),
                caption(
                    "Members with at least this power level show the role, unless a higher one \
                     applies. Only Untethered shows the name and colour.",
                ),
                row![
                    Space::new().width(Fill),
                    button(text("Cancel").size(14))
                        .padding([8, 12])
                        .style(plain(BODY, Color::WHITE))
                        .on_press(msg(Input::Cancel)),
                    small_button(
                        if busy { "Saving…" } else { "Save Role" },
                        primary,
                        (!busy).then(|| msg(Input::Save)),
                    ),
                ]
                .spacing(8)
                .align_y(Center),
            ]
            .spacing(12),
        )
        .padding(16)
        .width(Fill)
        .style(|_| surface(SIDE, 8.))
        .into()
    }

    fn role_member_list<'a>(&'a self, page: &'a Page) -> El<'a> {
        let space = &page.space;
        let busy = self.roles_busy();
        let everyone = self.everyone_in(space);
        let total = page.snapshot.as_ref().map_or(0, |s| s.members.len());
        let mut list = column![
            label_owned(format!("MEMBERS — {total}")),
            text_input("Search members", &page.roles.filter)
                .id(FILTER_FIELD)
                .on_input(|v| msg(Input::Filter(v)))
                .size(14)
                .padding([6, 8])
                .style(input_style),
        ]
        .spacing(10);
        if page.snapshot.is_none() {
            list = list.push(caption(if page.load_failed {
                "Couldn't load the server's members."
            } else {
                "Loading…"
            }));
            return list.into();
        }
        let room = self.live.list.get(space);
        let members = self.role_members();
        let more = members.len().saturating_sub(SHOWN);
        if members.is_empty() {
            list = list.push(caption("No members match."));
        }
        for m in members.into_iter().take(SHOWN) {
            let name = if m.name.is_empty() {
                crate::rooms::short(&m.id)
            } else {
                m.name.as_str()
            };
            let role = room.and_then(|r| r.role(&m.id));
            let level = self.level_in(space, &m.id).unwrap_or(everyone);
            let current = Choice {
                level: (level > everyone).then_some(level),
                label: self.level_label(space, space, level),
            };
            let on: Option<Box<dyn Fn(Choice) -> crate::Msg + 'a>> = (!busy
                && self.may_assign(space, &m.id))
            .then(|| Box::new(assign_msg(&m.id)) as Box<dyn Fn(Choice) -> crate::Msg + 'a>);
            let differs = self.member_differs(space, &m.id);
            let second = if m.id == self.matrix_id {
                format!("{} · you", m.id)
            } else if differs.is_empty() {
                m.id.clone()
            } else {
                let rooms: Vec<String> = differs
                    .iter()
                    .take(3)
                    .map(|(room, l)| format!("{} in #{room}", self.level_label(space, space, *l)))
                    .collect();
                format!("{} · {}", m.id, rooms.join(", "))
            };
            list = list.push(
                row![
                    column![
                        crate::view::names::author(name, role, 15.),
                        text(second).size(12).color(CHANNEL),
                    ]
                    .spacing(2)
                    .width(Fill),
                    picker(self.role_choices(space, space, false), current, on),
                ]
                .spacing(12)
                .align_y(Center),
            );
        }
        if more > 0 {
            list = list.push(caption_owned(format!("{more} more. Search to find them.")));
        }
        list.into()
    }

    fn permission_list<'a>(&'a self, page: &'a Page) -> El<'a> {
        let space = &page.space;
        let busy = self.roles_busy();
        let channels = self.role_channels(space);
        let target = page
            .roles
            .channel
            .as_deref()
            .filter(|room| channels.iter().any(|(id, _)| id == room));
        let room = target.unwrap_or(space);
        let mut places = vec![Place {
            room: None,
            label: "Server defaults".into(),
        }];
        places.extend(channels.iter().map(|(id, r)| Place {
            room: Some((*id).to_owned()),
            label: format!("#{}", r.name),
        }));
        let shown = places.iter().find(|p| p.room.as_deref() == target).cloned();
        let chooser = pick_list(places, shown, |p: Place| msg(Input::Channel(p.room)))
            .text_size(14)
            .padding([6, 10])
            .width(260)
            .style(picker_style)
            .menu_style(menu_style);
        let may = self.may_roles(room);
        let mine = self.my_level(room).unwrap_or(i64::MIN);
        let everyone = self.everyone_in(room);
        let notice: El = if may {
            caption(if target.is_some() {
                "Where a channel differs from the server's defaults, it says so. Changing it \
                 here changes this channel only."
            } else {
                "The space's own levels, and every channel's that has the same. Changing one \
                 here changes it in each of those channels too."
            })
        } else {
            row![
                icon(Icon::Lock, 14., MUTED),
                text("Read-only: you can't change permissions here.")
                    .size(13)
                    .color(MUTED)
            ]
            .spacing(6)
            .align_y(Center)
            .into()
        };
        let mut list = column![
            label("PERMISSIONS"),
            row![text("Show").size(14).color(BODY), chooser]
                .spacing(10)
                .align_y(Center),
            notice,
        ]
        .spacing(10);
        for perm in Perm::ALL {
            let level = self.perm_level(room, perm);
            let server = self.perm_level(space, perm);
            let current = Choice {
                level,
                label: level.map_or_else(|| "Unknown".into(), |l| self.level_label(space, room, l)),
            };
            let changeable = may && !busy && level.is_some_and(|l| l <= mine);
            let on: Option<Box<dyn Fn(Choice) -> crate::Msg + 'a>> = changeable.then(|| {
                Box::new(perm_msg(room, perm, everyone)) as Box<dyn Fn(Choice) -> crate::Msg + 'a>
            });
            let differs: El = match (target, level, server) {
                (Some(_), Some(here), Some(server)) if here != server => text(format!(
                    "Differs from the server: {}",
                    self.level_label(space, space, server)
                ))
                .size(12)
                .color(WARN)
                .into(),
                _ => Space::new().into(),
            };
            list = list.push(
                container(
                    row![
                        column![
                            text(perm.label()).size(15).font(BOLD).color(TEXT),
                            text(perm.hint()).size(12).color(CHANNEL),
                            differs,
                        ]
                        .spacing(2)
                        .width(Fill),
                        picker(self.role_choices(space, room, true), current, on),
                    ]
                    .spacing(12)
                    .align_y(Center),
                )
                .padding([6, 0]),
            );
        }
        let limits = target.map(|t| self.channel_limits(space, t));
        let sync: El = match (target, &limits) {
            (Some(t), Some(l)) if !l.is_empty() && may => small_button(
                "Use the Server's Permissions",
                secondary,
                (!busy).then(|| msg(Input::Sync(t.to_owned()))),
            ),
            _ => Space::new().into(),
        };
        list = list.push(sync);
        // Which channels have permissions of their own.
        let mut own = Column::new().spacing(4);
        own = own.push(label("CHANNELS WITH THEIR OWN PERMISSIONS"));
        let mut any = false;
        for (id, r) in &channels {
            let limits = self.channel_limits(space, id);
            if limits.is_empty() {
                continue;
            }
            any = true;
            let names: Vec<&str> = limits.iter().map(|p| p.label()).collect();
            own = own.push(
                button(
                    row![
                        text(format!("#{}", r.name)).size(14).font(BOLD).color(BODY),
                        text(names.join(", ")).size(13).color(MUTED),
                    ]
                    .spacing(10),
                )
                .padding([4, 6])
                .style(plain(BODY, Color::WHITE))
                .on_press(msg(Input::Channel(Some((*id).to_owned())))),
            );
        }
        if !any {
            own = own.push(caption("Every channel has the server's permissions."));
        }
        list.push(Space::new().height(8)).push(own).into()
    }
}

fn caption_owned<'a>(s: String) -> El<'a> {
    text(s).size(12).color(CHANNEL).into()
}
