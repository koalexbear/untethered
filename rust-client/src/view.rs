use super::{App, Msg, Request, Section, rooms::Room};
#[cfg(target_os = "linux")]
mod call_controls;
#[cfg(target_os = "linux")]
mod call_stage;
mod community;
mod mentions;
mod reaction_bar;
mod room_people;
mod room_search;
use chrono::{DateTime, Local};
use iced::{
    Alignment::Center,
    Border, Color, Element, Fill, Font, Length, Theme, color, font, keyboard,
    widget::{
        Column, Row, Space, button, column, container, image, row, rule, scrollable, stack, text,
        text_editor, text_input, tooltip,
    },
};

const RAIL: Color = color!(0x1e1f22);
const SIDE: Color = color!(0x2b2d31);
const CHAT: Color = color!(0x313338);
const TRAY: Color = color!(0x232428);
const HOVER: Color = color!(0x35373c);
const ACTIVE: Color = color!(0x404249);
const BLURPLE: Color = color!(0x5865f2);
const TEXT: Color = color!(0xf2f3f5);
const MUTED: Color = color!(0xb5bac1);
const DIM: Color = color!(0x969ba4);
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
fn avatar(name: &str, size: f32) -> El<'static> {
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
    text(s).font(BOLD).size(11).color(MUTED).into()
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
fn rail_button<'a>(glyph: String, hint: &'a str, active: bool, msg: Msg) -> El<'a> {
    let b = button(container(text(glyph).font(BOLD).color(TEXT).size(19)).center(44))
        .padding(0)
        .on_press(msg)
        .style(move |_, s| {
            let highlighted = active || s == button::Status::Hovered;
            button::Style {
                background: Some(if highlighted {
                    BLURPLE.into()
                } else {
                    CHAT.into()
                }),
                border: Border::default().rounded(if highlighted { 14 } else { 22 }),
                ..Default::default()
            }
        });
    let marker = container(Space::new())
        .width(4)
        .height(if active { 30 } else { 0 })
        .style(|_| surface(TEXT, 3.));
    tooltip(
        row![marker, b].spacing(7).align_y(Center),
        container(text(hint).size(14)).padding(6),
        tooltip::Position::Right,
    )
    .style(|_| surface(RAIL, 5.))
    .gap(8)
    .into()
}

impl App {
    #[cfg(target_os = "linux")]
    fn ptt_control(&self, active: bool) -> El<'_> {
        iced::widget::checkbox(self.room_ptt)
            .label("Push-to-talk")
            .size(16)
            .text_size(14)
            .on_toggle_maybe(active.then_some(Msg::RoomPtt))
            .into()
    }
    #[cfg(target_os = "linux")]
    fn call_bar(&self) -> Option<El<'_>> {
        use crate::call_devices::Port;
        use call_controls::{Icon, control};
        use iced::widget::pick_list;
        let call = self.call.as_ref()?;
        let mode = call.audio.borrow();
        let name = self
            .live
            .list
            .get(&call.room)
            .map_or("DM", |r| r.name.as_str());
        let active = !call.cancel.is_closed();
        let mic = if mode.deafened {
            "Mic muted"
        } else if mode.muted {
            "Unmute"
        } else {
            "Mute"
        };
        let controls = row![
            control(
                Icon::Mic,
                mic,
                mode.muted || mode.deafened,
                mode.muted || mode.deafened,
                (active && !mode.deafened).then_some(Msg::ToggleMic)
            ),
            control(
                Icon::Headphones,
                if mode.deafened { "Undeafen" } else { "Deafen" },
                mode.deafened,
                mode.deafened,
                active.then_some(Msg::ToggleDeafen)
            ),
        ]
        .spacing(8)
        .align_y(Center);
        let mut controls = controls;
        for input in crate::desktop_call::VideoInput::ALL.into_iter().rev() {
            let slot = input as usize;
            let requested = mode.video[slot].is_some();
            let label = input.control(requested, call.video_status[slot].live);
            let cam = input == crate::desktop_call::VideoInput::Camera;
            controls = controls.push(control(
                if cam { Icon::Camera } else { Icon::Screen },
                label,
                requested,
                cam && !requested,
                active.then_some(Msg::ToggleVideo(input)),
            ));
        }
        controls = controls.push(control(
            Icon::Devices,
            "Devices",
            call.picker,
            false,
            active.then_some(Msg::AudioDevices),
        ));
        if call.group {
            controls = controls.push(self.ptt_control(active)).push(
                button(text("Mixer").size(14))
                    .style(nav(call.mixer))
                    .on_press_maybe(active.then_some(Msg::ToggleMixer)),
            );
        }
        controls = controls.push(control(
            Icon::Hangup,
            if call.group { "Leave voice" } else { "Hang up" },
            false,
            false,
            active.then_some(Msg::EndCall),
        ));
        let browsing = call.chat_open
            || self.selected != call.room
            || self.mentions.is_some()
            || self.setup.is_some()
            || self.account;
        let mut bar = Column::new().spacing(5);
        if browsing {
            bar = bar.push(
                text(format!("{} · {name}", call.label))
                    .size(13)
                    .color(TEXT),
            );
        }
        bar = bar.push(container(controls.wrap()).center_x(Fill));
        let screen = crate::desktop_call::VideoInput::Window as usize;
        if call.video_status[screen].live
            && let Some(window) = mode.video[screen]
        {
            let id = call.id;
            let label = if !call.app_note.is_empty() {
                call.app_note.as_str()
            } else {
                "Window audio is off"
            };
            let mut line = row![text(label).size(13)].spacing(8).align_y(Center);
            if mode.send_app.is_some() {
                line = line.push(
                    button(text("Stop app audio").size(13))
                        .style(nav(false))
                        .on_press(Msg::ShareApp(id, window, None)),
                );
            }
            if !call.picker {
                line = line.push(
                    button(text("Choose app audio").size(13))
                        .style(nav(false))
                        .on_press(Msg::AudioDevices),
                );
            }
            bar = bar.push(line);
            if call.picker {
                if call.devices.streams.is_empty() {
                    bar = bar.push(
                        text("Play sound in the app, then refresh devices.")
                            .size(13)
                            .color(MUTED),
                    );
                } else {
                    bar = bar.push(
                        row![
                            text("Send sound from").size(14),
                            pick_list(
                                call.devices.streams.as_slice(),
                                mode.send_app.as_ref().map(|(_, s)| s.clone()),
                                move |s| Msg::ShareApp(id, window, Some(s))
                            )
                            .placeholder("Choose a playback stream")
                            .text_size(14)
                        ]
                        .align_y(Center)
                        .spacing(8),
                    );
                }
            }
        }
        if call.picker || call.previews.remote_share() {
            let id = call.id;
            bar = bar.push(
                iced::widget::checkbox(!mode.share_silent)
                    .label("Play shared audio")
                    .size(16)
                    .text_size(14)
                    .on_toggle_maybe(
                        active.then_some(move |enabled| Msg::SetShareSound(id, enabled)),
                    ),
            );
        }
        if self.verified
            && active
            && browsing
            && let Some(room) = self.live.list.get(&call.room)
        {
            bar = bar.push(call_stage::compact(call, room, !self.setup_busy()));
        }
        if mode.talk != crate::desktop_call::TalkGate::Open {
            let hint = if mode.deafened {
                "Undeafen to use push-to-talk."
            } else if mode.muted {
                "Unmute to use push-to-talk."
            } else if mode.talk == crate::desktop_call::TalkGate::Held {
                "Right Ctrl held · release to stop talking."
            } else {
                "Hold Right Ctrl to talk while Untethered is focused."
            };
            bar = bar.push(text(hint).size(13));
        }
        for status in &call.video_status {
            if !status.label.is_empty() && (browsing || !status.live) {
                bar = bar.push(text(&status.label).size(13));
            }
        }
        if call.mic_error {
            bar =
                bar.push(text("Microphone unavailable · choose an input under Devices.").size(13));
        }
        if call.speaker_error {
            bar = bar.push(text("Speaker unavailable · choose an output under Devices.").size(13));
        }
        if call.mixer
            && let Some(room) = self.live.list.get(&call.room)
        {
            let id = call.id;
            let mut rows = Column::new().spacing(4);
            let mut peers = room.voice_users();
            peers.remove(self.matrix_id.as_str());
            for &user in &peers {
                let gain = mode.volume(user);
                rows = rows.push(
                    row![
                        container(tooltip(
                            text(room.display(user)).size(14),
                            text(user).size(13),
                            tooltip::Position::Bottom
                        ))
                        .width(Length::FillPortion(1)),
                        text(format!("{gain}%")).size(13).width(44),
                        iced::widget::slider(0..=200u8, gain, move |v| Msg::PeerVolume(
                            id,
                            user.into(),
                            v
                        ))
                        .default(100u8)
                        .step(5u8)
                        .shift_step(1u8)
                        .width(Length::FillPortion(2)),
                        button(text("Reset").size(13))
                            .padding([2, 8])
                            .style(nav(false))
                            .on_press(Msg::PeerVolume(id, user.into(), 100))
                    ]
                    .spacing(8)
                    .align_y(Center),
                );
            }
            bar = bar.push(label("LOCAL PLAYBACK VOLUME"));
            if peers.is_empty() {
                bar = bar.push(text("No other participants in voice.").size(13));
            } else {
                bar = bar.push(container(scrollable(rows)).max_height(132).width(Fill));
            }
        }
        if call.picker {
            for (label, port, route) in [
                ("Microphone", Port::Mic, &mode.mic),
                ("Speaker", Port::Speaker, &mode.speaker),
            ] {
                bar = bar.push(
                    row![
                        text(label).width(90).size(14),
                        pick_list(
                            call.devices.list(port),
                            Some(call.devices.selected(port, route)),
                            move |choice| Msg::SelectDevice(port, choice)
                        )
                        .width(Fill)
                        .text_size(14)
                        .padding([5, 9])
                    ]
                    .align_y(Center)
                    .spacing(8),
                );
            }
            let refresh = button(
                text(if call.loading {
                    "Loading…"
                } else {
                    "Refresh devices"
                })
                .size(14)
                .font(BOLD),
            )
            .style(nav(false))
            .on_press_maybe((active && !call.loading).then_some(Msg::RefreshDevices));
            bar = bar.push(
                row![refresh, text(call.list_error.unwrap_or("")).size(13)]
                    .align_y(Center)
                    .spacing(8),
            );
            let mut cams = row![text("Camera").width(90).size(14)]
                .align_y(Center)
                .spacing(8);
            if !call.cameras.is_empty() {
                let id = call.id;
                cams = cams.push(
                    pick_list(
                        call.cameras.as_slice(),
                        Some(mode.camera.clone()),
                        move |choice| Msg::SelectCamera(id, choice),
                    )
                    .width(Fill)
                    .text_size(14)
                    .padding([5, 9]),
                );
            }
            cams = cams.push(
                button(
                    text(if call.camera_listing {
                        "Looking…"
                    } else if call.cameras.is_empty() {
                        "List cameras"
                    } else {
                        "Refresh cameras"
                    })
                    .size(14),
                )
                .style(nav(false))
                .on_press_maybe((active && !call.camera_listing).then_some(Msg::ListCameras)),
            );
            bar = bar.push(cams);
            if let Some(note) = call.camera_note {
                bar = bar.push(text(note).size(13));
            }
        }
        Some(
            container(bar)
                .width(Fill)
                .padding([10, 12])
                .style(|_| surface(TRAY, 4.))
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
        if self.account || self.logged_in && !self.verified {
            return self.account_view();
        }
        let store = self.rooms();
        let mut icons = column![
            rail_button(
                "U".into(),
                "Home · direct messages",
                self.section == Section::Direct,
                Msg::Section(Section::Direct)
            ),
            rail_button(
                "#".into(),
                "All rooms",
                self.section == Section::All,
                Msg::Section(Section::All)
            ),
            container(rule::horizontal(1)).width(36).padding([4, 0])
        ]
        .spacing(8)
        .align_x(Center);
        for (id, r) in &store.list {
            if r.space {
                let s = Section::Space(id.clone());
                let initials = r
                    .name
                    .split_whitespace()
                    .take(2)
                    .filter_map(|s| s.chars().next())
                    .collect::<String>()
                    .to_uppercase();
                icons = icons.push(rail_button(
                    initials,
                    &r.name,
                    self.section == s,
                    Msg::Section(s),
                ));
            }
        }
        let rail = container(scrollable(icons))
            .padding([14, 0])
            .width(70)
            .height(Fill)
            .style(|_| surface(RAIL, 0.));
        let section_name = match &self.section {
            Section::All => "All rooms",
            Section::Direct => "Direct messages",
            Section::Space(id) => store
                .list
                .get(id)
                .map(|r| r.name.as_str())
                .unwrap_or("Space"),
        };
        let head = container(text(section_name).font(BOLD).size(17).color(TEXT))
            .padding([0, 16])
            .center_y(52)
            .width(Fill);
        let mut links = Column::new().spacing(2).padding([0, 8]);
        for (id, r) in self.visible() {
            let prefix = if r.voice {
                "♪"
            } else if r.dm {
                "@"
            } else {
                "#"
            };
            let active = self.mentions.is_none() && id == &self.selected;
            let entry = row![
                text(prefix).size(22).color(DIM),
                text(&r.name)
                    .size(15)
                    .width(Fill)
                    .color(if active || r.unread { TEXT } else { MUTED }),
                text(if !r.unread {
                    String::new()
                } else if r.read.highlights > 0 {
                    format!("@{}", r.read.highlights)
                } else if r.read.messages > 0 {
                    format!("{}", r.read.messages)
                } else {
                    "•".into()
                })
                .font(BOLD)
                .size(13)
                .color(if r.read.highlights > 0 {
                    color!(0xffb6bc)
                } else {
                    TEXT
                })
            ]
            .spacing(9)
            .align_y(Center);
            links = links.push(
                button(entry)
                    .width(Fill)
                    .padding([5, 9])
                    .on_press(Msg::Open(id.clone()))
                    .style(nav(active)),
            );
            #[cfg(target_os = "linux")]
            if r.voice {
                let speakers = self
                    .call
                    .as_ref()
                    .filter(|c| c.group && c.room == *id)
                    .map(|c| c.speaking.borrow());
                for user in r.voice_users() {
                    let speaking = speakers.as_ref().is_some_and(|s| s.contains(user));
                    let mut line = row![
                        text(r.display(user))
                            .width(Fill)
                            .size(13)
                            .color(if speaking { TEXT } else { MUTED })
                    ]
                    .spacing(4)
                    .align_y(Center);
                    if speaking {
                        line =
                            line.push(text("Speaking").size(12).font(BOLD).color(color!(0x65d99c)));
                    }
                    links = links.push(container(line).width(Fill).padding(iced::Padding {
                        left: 28.,
                        right: 10.,
                        ..Default::default()
                    }));
                }
            }
        }
        let name = if self.logged_in {
            self.profile.as_str()
        } else {
            "Preview user"
        };
        let account_label = if self.logged_in {
            "Matrix account"
        } else {
            "Local preview"
        };
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        let account_label = if self.logged_in && self.alerts.dnd == Some(true) {
            "Do Not Disturb"
        } else {
            account_label
        };
        let user = button(
            row![
                avatar(name, 31.),
                column![
                    text(name).size(14).font(BOLD),
                    text(account_label).size(11).color(MUTED)
                ]
            ]
            .spacing(10)
            .align_y(Center),
        )
        .padding(9)
        .width(Fill)
        .on_press(Msg::Account)
        .style(nav(false));
        let mut sidebar = column![head, rule::horizontal(1)];
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        if self.logged_in
            && self.verified
            && let Section::Space(id) = &self.section
            && let Some(space) = store.list.get(id)
        {
            sidebar = sidebar.push(
                button(text("Space alerts").size(15))
                    .style(nav(self.alerts.spaces.panel))
                    .width(Fill)
                    .padding([7, 16])
                    .on_press(Msg::SpaceAlerts),
            );
            if self.alerts.spaces.panel {
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
                let mut panel =
                    column![text(format!("Space default: {state}")).size(13).font(BOLD)].spacing(3);
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
                    panel =
                        panel.push(button("Retry syncing rules").on_press(Msg::RetrySpaceRules));
                }
                if self.alerts.spaces.conflict() {
                    panel =
                        panel.push(text("A customized Matrix rule was left unchanged.").size(12));
                }
                sidebar = sidebar.push(container(panel).padding([4, 10]));
            }
        }
        if self.logged_in && self.verified {
            sidebar = sidebar.push(
                button(text("Mentions").size(15))
                    .width(Fill)
                    .padding([7, 16])
                    .style(nav(self.mentions.is_some()))
                    .on_press(Msg::OpenMentions),
            );
        }
        let mut sidebar = sidebar
            .push(
                container(label(if self.section == Section::Direct {
                    "DIRECT MESSAGES"
                } else {
                    "CHANNELS"
                }))
                .padding([14, 16]),
            )
            .push(scrollable(links).height(Fill));
        if self.logged_in && self.verified {
            let (label, parent) = match &self.section {
                Section::Space(id) => ("Add channel", Some(id.clone())),
                _ => ("New space", None),
            };
            if let Some(space) = &parent {
                sidebar = sidebar.push(
                    button(text("Invite member").size(14))
                        .padding([6, 16])
                        .width(Fill)
                        .style(nav(false))
                        .on_press_maybe((self.connected && self.setup.is_none()).then(|| {
                            Msg::Setup(crate::community::Input::InviteMember(space.clone()))
                        })),
                );
            }
            sidebar = sidebar.push(
                button(text(label).size(14))
                    .width(Fill)
                    .padding([6, 16])
                    .style(nav(false))
                    .on_press_maybe(
                        (self.connected && self.setup.is_none())
                            .then_some(Msg::Setup(crate::community::Input::Start(parent))),
                    ),
            );
        }
        let side =
            container(sidebar.push(container(user).width(Fill).style(|_| surface(TRAY, 0.))))
                .width(211)
                .height(Fill)
                .style(|_| surface(SIDE, 0.));
        if self.setup.is_some() || self.mentions.is_some() {
            let panel = Column::new();
            #[cfg(target_os = "linux")]
            let panel = panel.extend(self.incoming_banner()).extend(self.call_bar());
            let content = if let Some(f) = &self.setup {
                community::view(self, f)
            } else {
                mentions::view(self, self.mentions.as_ref().unwrap())
            };
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
        let room = store.list.get(&self.selected);
        #[cfg(target_os = "linux")]
        if let Some(c) = &self.call
            && !c.chat_open
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
            let mut panel = column![container(header).padding([10, 16]).width(Fill)]
                .extend(self.incoming_banner())
                .push(call_stage::view(c, r, &self.matrix_id));
            if let Some(bar) = self.call_bar() {
                panel = panel.push(bar);
            }
            let panel = container(panel)
                .style(|_| surface(color!(0x111214), 0.))
                .width(Fill)
                .height(Fill);
            return if c.theater {
                panel.into()
            } else {
                row![rail, side, panel].height(Fill).into()
            };
        }
        let channel = room.map(|r| r.name.as_str()).unwrap_or("Rooms");
        let header = row![
            text(if room.is_some_and(|r| r.voice) {
                "♪"
            } else if room.is_some_and(|r| r.dm) {
                "@"
            } else {
                "#"
            })
            .size(25)
            .color(DIM),
            text(channel).font(BOLD).size(17),
            Space::new().width(Fill),
            button(text("People").size(13))
                .on_press(Msg::TogglePeople)
                .style(nav(self.people)),
            button(text("Copy chat").size(13))
                .on_press_maybe(room.map(|_| Msg::Copy))
                .style(nav(false))
        ]
        .spacing(9)
        .align_y(Center);
        let header = if self.logged_in && self.verified && room.is_some() {
            header
                .push(
                    button(text("Permissions").size(13))
                        .style(nav(self.permission_panel))
                        .on_press(Msg::Permissions),
                )
                .push(
                    button(text("Search").size(13))
                        .style(nav(self.search.is_some()))
                        .on_press_maybe(
                            (self.search.is_none() && self.original.is_none())
                                .then_some(Msg::OpenSearch),
                        ),
                )
        } else {
            header
        };
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        let header = if self.logged_in && self.verified && room.is_some() {
            header.push(
                button(text("Alerts").size(13))
                    .style(nav(self.alerts.panel))
                    .on_press(Msg::NotificationPanel),
            )
        } else {
            header
        };
        #[cfg(target_os = "linux")]
        let header = if self.call.is_none()
            && self.verified
            && self.connected
            && self.inbox.ringing.is_none()
            && room.is_some_and(|r| r.encrypted && (r.voice || r.dm_peer.is_some()))
        {
            if room.is_some_and(|r| r.voice) {
                header.push(self.ptt_control(true)).push(
                    button(text("Join voice").size(16).font(BOLD))
                        .padding([5, 12])
                        .style(nav(false))
                        .on_press(Msg::JoinVoice),
                )
            } else {
                header
                    .push(button("Voice call").on_press(Msg::DialDm(false)))
                    .push(button("Video call").on_press(Msg::DialDm(true)))
            }
        } else {
            header
        };
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
            container(header).padding([0, 17]).center_y(52).width(Fill),
            rule::horizontal(1)
        ];
        if let Some(banner) = banner {
            chat = chat.push(banner)
        }
        if self.permission_panel
            && let Some(r) = room
        {
            let policy = &r.posting;
            let mut panel = column![text("Room permissions · Posting").font(BOLD)].spacing(4);
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
                    .size(14),
                );
                if policy.patch(&self.matrix_id, action).is_some() {
                    panel = panel.push(
                        button(text(if busy { "Updating…" } else { action.label() }).size(14))
                            .style(nav(false))
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
                }).size(13).color(MUTED));
            } else {
                panel = panel.push(
                    text("Posting permissions unavailable until room state is loaded.").size(13),
                );
            }
            chat = chat.push(
                container(panel)
                    .padding([6, 14])
                    .width(Fill)
                    .style(|_| surface(TRAY, 5.)),
            );
        }
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        if self.alerts.panel
            && let Some(r) = room
        {
            let selected = self.alerts.mode(&self.selected);
            let inherited = self.alerts.spaces.inherited(&self.selected);
            let pending = self.notifications_pending(&self.selected);
            let mut options = Row::new().spacing(5);
            for mode in crate::notifications::Mode::CHOICES {
                options = options.push(
                    button(
                        text(
                            if mode == crate::notifications::Mode::Default && inherited != mode {
                                "Use space default"
                            } else {
                                mode.label()
                            },
                        )
                        .size(14),
                    )
                    .style(nav(mode == selected))
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
                text(format!("Room alerts · {} · {state}", r.name)).font(BOLD),
                options.wrap(),
                text("Mute ordinary messages; higher-priority mention rules still apply.")
                    .size(13)
                    .color(MUTED)
            ]
            .spacing(4);
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
            chat = chat.push(
                container(settings)
                    .padding([6, 14])
                    .width(Fill)
                    .style(|_| surface(TRAY, 5.)),
            );
        }
        #[cfg(target_os = "linux")]
        if let Some(b) = self.incoming_banner() {
            chat = chat.push(b);
        }
        #[cfg(target_os = "linux")]
        if let Some(bar) = self.call_bar() {
            chat = chat.push(bar);
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
            room_search::view(
                s,
                r,
                self.pending
                    .values()
                    .any(|p| matches!(p, Request::Context(..))),
            )
        } else if let Some(r) = room {
            self.conversation(r)
        } else {
            container(
                column![
                    text("No rooms here yet").font(BOLD).size(24),
                    text("Choose another space or open All rooms.").color(MUTED)
                ]
                .spacing(6),
            )
            .center(Fill)
            .into()
        };
        let mut middle = Row::new().push(body).height(Fill);
        if self.people && self.logged_in && room.is_some() {
            middle = middle.push(room_people::view(self));
        } else if self.people
            && let Some(r) = room
        {
            let people: std::collections::BTreeSet<_> =
                r.history.values().map(|p| p.sender.as_str()).collect();
            let mut list = column![label("IN THIS CONVERSATION")]
                .spacing(13)
                .padding(14);
            for sender in people {
                let n = r.display(sender);
                list = list.push(
                    row![avatar(n, 28.), text(n).size(14)]
                        .spacing(8)
                        .align_y(Center),
                );
            }
            middle = middle.push(
                container(scrollable(list))
                    .width(178)
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
        let mut feed = Column::new().padding([16, 18]).spacing(2);
        let original = self.original.as_ref().filter(|c| c.room == self.selected);
        let history = original.and_then(|c| c.slice.as_ref()).unwrap_or(r);
        if original.is_none() && self.logged_in && !r.end {
            feed = feed.push(
                button(text("Earlier messages").size(12))
                    .on_press_maybe(self.connected.then_some(Msg::More))
                    .style(nav(false)),
            );
        }
        if original.is_none() && (r.end || !self.logged_in) {
            feed = feed
                .push(
                    container(text(if r.dm { "@" } else { "#" }).font(BOLD).size(30))
                        .center(52)
                        .style(|_| surface(ACTIVE, 26.)),
                )
                .push(
                    text(format!(
                        "{}{}",
                        if r.dm { "" } else { "Welcome to #" },
                        r.name
                    ))
                    .font(BOLD)
                    .size(28)
                    .color(TEXT),
                )
                .push(
                    text(if r.topic.is_empty() {
                        "This is the start of the conversation."
                    } else {
                        &r.topic
                    })
                    .size(14)
                    .color(MUTED),
                );
        }
        let mut prev: Option<(&str, i64, String)> = None;
        let sorted = history.posts();
        let anchor = original.and_then(|c| {
            sorted
                .iter()
                .position(|p| p.event_id.as_ref() == Some(&c.event))
        });
        let posts = if let Some(c) = original {
            let i = anchor.unwrap_or(sorted.len());
            &sorted[if c.preceding { i.saturating_sub(6) } else { i }..sorted.len().min(i + 13)]
        } else {
            &sorted[..]
        };
        let divider = if original.is_none() && r.unread {
            r.read.divider(r)
        } else {
            None
        };
        let seen = r.read.seen(&self.matrix_id);
        for &cached in posts {
            let p = if original.is_some() {
                cached
                    .event_id
                    .as_deref()
                    .and_then(|id| r.by_id(id))
                    .unwrap_or(cached)
            } else {
                cached
            };
            let stamp = DateTime::from_timestamp_millis(p.time)
                .unwrap_or_default()
                .with_timezone(&Local);
            let day = stamp.format("%B %-d, %Y").to_string();
            let date_change = prev.as_ref().is_none_or(|(_, _, d)| d != &day);
            let group = date_change
                || prev
                    .as_ref()
                    .is_none_or(|(s, t, _)| *s != p.sender || p.time - *t > 300_000);
            if date_change {
                feed = feed.push(
                    container(
                        row![
                            rule::horizontal(1),
                            text(day.clone()).size(11).color(MUTED),
                            rule::horizontal(1)
                        ]
                        .spacing(8)
                        .align_y(Center),
                    )
                    .padding([9, 0]),
                );
            }
            if divider.is_some() && p.event_id.as_deref() == divider {
                feed = feed.push(
                    row![
                        rule::horizontal(1),
                        text("New messages")
                            .size(12)
                            .font(BOLD)
                            .color(color!(0xffb6bc)),
                        rule::horizontal(1)
                    ]
                    .spacing(8)
                    .align_y(Center),
                );
            }
            let name = r.display(&p.sender);
            let mut message = Column::new().spacing(2);
            if let Some(target) = &p.reply {
                let q = r.quote(target).or_else(|| history.quote(target));
                let preview = q
                    .map(|q| format!("{}: {}", r.display(&q.sender), q.excerpt))
                    .unwrap_or_else(|| "Earlier message".into());
                message = message.push(
                    button(text(format!("View original · {preview}")).size(13))
                        .padding([2, 5])
                        .style(nav(false))
                        .on_press(Msg::Jump(target.clone())),
                );
            }
            let mut line = row![text(p.text()).size(16).width(Fill)]
                .align_y(Center)
                .spacing(6);
            if p.revision.is_some() {
                line = line.push(text("edited").size(11).color(DIM));
            }
            if self.logged_in
                && let Some(id) = &p.event_id
            {
                if p.may_edit(&self.matrix_id) && r.has_metadata && r.encrypted == p.encrypted {
                    line = line.push(
                        button(text("Edit").size(12))
                            .padding([1, 6])
                            .style(nav(false))
                            .on_press_maybe(
                                (self.connected && self.editing.is_none())
                                    .then(|| Msg::Edit(id.clone())),
                            ),
                    );
                }
                line = line.push(
                    button(text("Reply").size(12))
                        .padding([1, 6])
                        .style(nav(false))
                        .on_press(Msg::Reply(id.clone())),
                );
                if p.reactable {
                    line = line.push(
                        button(text("React").size(12))
                            .padding([1, 6])
                            .style(nav(self
                                .reaction_picker
                                .as_ref()
                                .is_some_and(|p| &p.target == id)))
                            .on_press_maybe(
                                (self.verified && self.connected)
                                    .then(|| Msg::ReactionPicker(id.clone())),
                            ),
                    );
                }
                if self.verified && p.report_id(&self.matrix_id).is_some() {
                    line = line.push(
                        button(text("Report").size(12))
                            .padding([1, 6])
                            .style(nav(false))
                            .on_press_maybe(
                                (self.connected
                                    && self.reporting.is_none()
                                    && !self.report_pending())
                                .then(|| Msg::Report(self.selected.clone(), id.clone())),
                            ),
                    );
                }
            }
            if let Some(edit) = self
                .editing
                .as_ref()
                .filter(|d| d.room == self.selected && Some(&d.event) == p.event_id.as_ref())
            {
                let input = text_editor(&edit.input)
                    .id("message-edit")
                    .on_action(Msg::ChangeEdit)
                    .key_binding(|press| {
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
                    .padding(7)
                    .height(Length::Shrink)
                    .max_height(120);
                let draft = edit.input.text();
                message = message
                    .push(text("Editing message").size(13).font(BOLD).color(MUTED))
                    .push(input)
                    .push(
                        row![
                            button(if edit.pending.is_some() {
                                "Saving…"
                            } else {
                                "Save edit"
                            })
                            .on_press_maybe(
                                (self.connected
                                    && edit.pending.is_none()
                                    && !draft.trim().is_empty()
                                    && draft != p.text())
                                .then_some(Msg::SaveEdit)
                            ),
                            button("Cancel")
                                .on_press_maybe(edit.pending.is_none().then_some(Msg::CancelEdit))
                        ]
                        .spacing(6),
                    );
            } else {
                message = message.push(line);
            }
            if let Some(bar) = reaction_bar::view(self, p) {
                message = message.push(bar);
            }
            if original.is_some_and(|c| Some(&c.event) == p.event_id.as_ref()) {
                feed = feed.push(
                    text(
                        if self.search.as_ref().is_some_and(|s| {
                            s.hits.iter().any(|h| Some(&h.id) == p.event_id.as_ref())
                        }) {
                            "Search match"
                        } else {
                            "Original message"
                        },
                    )
                    .size(13)
                    .color(color!(0xa8b1ff))
                    .font(BOLD),
                );
            }
            let mut entry = Column::new().spacing(2);
            if group {
                let info = row![
                    text(name).size(16).font(BOLD).color(TEXT),
                    text(stamp.format("%H:%M").to_string())
                        .size(11)
                        .color(MUTED)
                ]
                .spacing(9)
                .align_y(Center);
                entry = entry.push(
                    container(
                        row![
                            avatar(name, 36.),
                            column![info, message].spacing(2).width(Fill)
                        ]
                        .spacing(13),
                    )
                    .padding([5, 0]),
                );
            } else {
                entry = entry.push(row![Space::new().width(36), message.width(Fill)].spacing(13));
            }
            prev = Some((&p.sender, p.time, day));
            if let Some(file) = &p.media {
                if let Some(source) = &file.preview {
                    let preview: El = if let Some(handle) = self.inline.ready(source) {
                        image(handle)
                            .width(320)
                            .height(208)
                            .content_fit(iced::ContentFit::Contain)
                            .border_radius(8)
                            .into()
                    } else if self.inline.pending(source) {
                        text("Loading image…").size(12).color(DIM).into()
                    } else {
                        button(
                            text(if self.inline.failed(source) {
                                "Retry image preview"
                            } else {
                                "Show image preview"
                            })
                            .size(12),
                        )
                        .on_press(Msg::ShowInline(source.clone()))
                        .style(nav(false))
                        .into()
                    };
                    entry = entry.push(row![Space::new().width(36), preview].spacing(13));
                }
                entry = entry.push(
                    row![
                        Space::new().width(36),
                        button(
                            text(format!(
                                "Save {}  {}",
                                file.name,
                                file.size.map(crate::media::size).unwrap_or_default()
                            ))
                            .size(14)
                        )
                        .on_press(Msg::SaveFile(file.clone()))
                        .style(button::secondary)
                    ]
                    .spacing(13),
                );
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
                    entry = entry.push(
                        row![
                            Space::new().width(36),
                            text(format!("Seen by {}{suffix}", head.join(", ")))
                                .size(12)
                                .color(MUTED)
                        ]
                        .spacing(13),
                    );
                }
            }
            let entry = container(entry).width(Fill);
            feed = feed.push(if let Some(id) = &p.event_id {
                entry.id(iced::widget::Id::from(format!("post-{id}")))
            } else {
                entry
            });
        }
        let feed = scrollable(feed).height(Fill).id(self.timeline_id());
        let feed = if original.is_some() {
            feed
        } else {
            feed.anchor_bottom().on_scroll(|v| {
                Msg::ReadingScroll(self.selected.clone(), v.absolute_offset().y > 1.)
            })
        };
        let mut viewport = Column::new();
        if let Some(c) = original {
            let mut header = row![text("Message history").width(Fill).font(BOLD)];
            if anchor.is_some_and(|i| i > 0) {
                header = header.push(
                    button(if c.preceding {
                        "Back to original"
                    } else {
                        "Preceding messages"
                    })
                    .on_press(Msg::Preceding),
                );
            }
            let header = if self.search.is_some() {
                header
                    .push(
                        button("Back to results")
                            .style(nav(false))
                            .on_press(Msg::SearchResults),
                    )
                    .push(
                        button("Return to timeline")
                            .style(nav(false))
                            .on_press(Msg::ReturnToReading),
                    )
            } else {
                header.push(button("Back to latest").on_press(Msg::Latest))
            };
            let header = header.spacing(5).align_y(Center);
            viewport = viewport.push(container(header).padding([4, 16]));
        }
        if self
            .pending
            .values()
            .any(|p| matches!(p,Request::Context(room,_) if room==&self.selected))
        {
            viewport = viewport.push(text("Loading original message…").size(13));
        }
        let restricted =
            self.logged_in && r.posting.can_post(&self.matrix_id, r.encrypted) == Some(false);
        let hint = if restricted {
            "Read-only for your account".into()
        } else {
            format!("Message {}{}", if r.dm { "@" } else { "#" }, r.name)
        };
        let editor = text_editor(&self.editor)
            .id("composer")
            .placeholder(hint)
            .on_action(Msg::Draft)
            .key_binding(|press| {
                #[cfg(target_arch = "wasm32")]
                if press.modifiers.command()
                    && press.key.as_ref() == keyboard::Key::Character("v")
                    && matches!(press.status, text_editor::Status::Focused { .. })
                {
                    return Some(text_editor::Binding::Custom(Msg::WebPaste(
                        super::Field::Composer(self.selected.clone()),
                    )));
                }
                if press.key == keyboard::Key::Named(keyboard::key::Named::Enter)
                    && press.modifiers.is_empty()
                    && matches!(press.status, text_editor::Status::Focused { .. })
                {
                    Some(text_editor::Binding::Custom(Msg::Send))
                } else {
                    let binding = text_editor::Binding::from_key_press(press);
                    #[cfg(not(target_arch = "wasm32"))]
                    if matches!(binding, Some(text_editor::Binding::Paste)) {
                        return Some(text_editor::Binding::Custom(Msg::PasteComposer));
                    }
                    binding
                }
            })
            .padding(11)
            .height(Length::Shrink)
            .min_height(42)
            .max_height(100);
        let sending = self
            .pending
            .values()
            .any(|p| matches!(p,Request::Send(id,..) if id==&self.selected));
        let files = self.outbox.rooms.get(&self.selected);
        let has_files = files.is_some_and(|q| !q.is_empty());
        let skipped = self.outbox.skipped.get(&self.selected);
        let uploading = self.batch.as_ref().is_some_and(|b| b.room == self.selected);
        let mut tray = Column::new().spacing(4);
        if let Some(d) = &self.reporting {
            let busy = self.report_pending();
            let ready = !busy && self.connected && self.verified && !d.reason.trim().is_empty();
            let input = text_input("Reason for reporting", &d.reason)
                .id("report-reason")
                .on_input_maybe((!busy).then_some(Msg::ReportReason))
                .on_submit_maybe(ready.then_some(Msg::SubmitReport));
            let submit = button(if busy {
                "Submitting…"
            } else {
                "Submit report"
            })
            .style(if ready { nav(true) } else { nav(false) })
            .on_press_maybe(ready.then_some(Msg::SubmitReport));
            let cancel = button("Cancel")
                .style(nav(false))
                .on_press_maybe((!busy).then_some(Msg::CancelReport));
            let panel=column![
                text(format!("Report message · {}",d.preview)).font(BOLD).size(14),
                text("Your homeserver admin receives the event ID and your reason. The message text is not attached.").size(13).color(MUTED),
                row![input,submit,cancel].align_y(Center).spacing(6),
            ].spacing(5);
            return column![
                viewport.push(feed).height(Fill),
                container(panel).padding([10, 16])
            ]
            .width(Fill)
            .height(Fill)
            .into();
        }
        if let Some(q) = self.reply_drafts.get(&self.selected) {
            tray = tray.push(
                row![
                    text(format!(
                        "Replying to {} · {}",
                        r.display(&q.sender),
                        q.excerpt
                    ))
                    .size(14)
                    .width(Fill),
                    button("Cancel reply")
                        .padding([2, 6])
                        .on_press(Msg::CancelReply)
                ]
                .align_y(Center),
            );
        }
        if let Some(reason) = skipped {
            tray = tray.push(
                row![
                    text(reason).size(13).color(color!(0xffc785)).width(Fill),
                    button(text("Dismiss").size(13))
                        .style(nav(false))
                        .on_press(Msg::DismissSkipped(self.selected.clone()))
                ]
                .spacing(6)
                .align_y(Center),
            );
            if !has_files && !self.editor.text().trim().is_empty() {
                tray = tray.push(
                    text("Choose another file or dismiss to send just the text.")
                        .size(12)
                        .color(MUTED),
                );
            }
        }
        if self.hover_files {
            tray = tray.push(
                text(format!("Drop files to attach to {}", r.name))
                    .color(TEXT)
                    .font(BOLD),
            );
        }
        if let Some(files) = files.filter(|q| !q.is_empty()) {
            let cards =
                Row::with_children(files.iter().map(|a| attachment_card(&self.selected, a)))
                    .spacing(8)
                    .padding([2, 0]);
            tray = tray.push(scrollable(cards).horizontal().width(Fill).height(138));
        }
        let composer = row![
            tooltip(
                button(text("+").size(20))
                    .on_press(Msg::PickFiles)
                    .style(button::secondary),
                "Attach files",
                tooltip::Position::Top
            ),
            editor,
            button(
                text(if sending || uploading {
                    "Sending…"
                } else if files.is_some_and(|q| q.iter().any(|a| a.cancelled || a.error.is_some()))
                {
                    "Retry"
                } else {
                    "Send"
                })
                .size(14)
                .font(BOLD)
            )
            .on_press_maybe(
                (!sending
                    && !restricted
                    && (!self.logged_in || self.connected)
                    && (has_files && self.batch.is_none() && !self.media_config_pending()
                        || !has_files
                            && skipped.is_none()
                            && !self.editor.text().trim().is_empty()))
                .then_some(Msg::Send)
            )
        ]
        .spacing(8)
        .align_y(Center);
        let foot = if restricted {
            "Read-only for your account"
        } else if !self.logged_in {
            "Local preview · Messages stay here"
        } else if r.encrypted {
            "End-to-end encrypted"
        } else {
            "Matrix room"
        };
        let mut footer =
            row![text(foot).size(11).color(DIM), Space::new().width(Fill)].align_y(Center);
        if self.logged_in && self.verified {
            let id = r.read.target().map(str::to_owned);
            let available = self.connected && !self.writing_read(&self.selected) && id.is_some();
            footer = footer.push(
                button(text(if r.unread { "Mark read" } else { "Mark unread" }).size(12))
                    .style(nav(false))
                    .padding([1, 6])
                    .on_press_maybe(available.then(|| {
                        Msg::MarkRoom(self.selected.clone(), if r.unread { id } else { None })
                    })),
            );
        }
        column![
            viewport.push(feed).height(Fill),
            container(
                column![
                    tray,
                    composer,
                    footer
                        .push(text("Shift + Enter for a new line").size(11).color(DIM))
                        .spacing(8)
                ]
                .spacing(5)
            )
            .padding([10, 16])
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }
    fn account_view(&self) -> El<'_> {
        macro_rules! field {
            ($label:expr,$value:expr,$kind:ident) => {{
                let input = text_input($label, $value).on_input(Msg::$kind).padding(11);
                #[cfg(target_arch = "wasm32")]
                let input = input.on_paste(|_| Msg::WebPaste(super::Field::$kind));
                input
            }};
        }
        let mut form = column![label("UNTETHERED  /  ACCOUNT")]
            .spacing(14)
            .width(362);
        #[cfg(target_os = "linux")]
        if let Some(b) = self.incoming_banner() {
            form = form.push(b);
        }
        #[cfg(target_os = "linux")]
        if let Some(bar) = self.call_bar() {
            form = form.push(bar);
        }
        if !self.logged_in {
            form = form
                .push(text("Welcome back").size(32).font(BOLD))
                .push(text("Sign in to your Matrix homeserver.").color(MUTED))
                .push(field!("Homeserver · matrix.org", &self.server, Server))
                .push(field!("Matrix username", &self.username, Username))
                .push(
                    field!("Password", &self.password, Password)
                        .secure(true)
                        .on_submit(Msg::Login),
                )
                .push(
                    button("Sign in").padding([9, 16]).on_press_maybe(
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
                    .push(text("Verify this device").size(28).font(BOLD))
                    .push(text(&self.matrix_id))
                    .push(
                        text_input("Recovery key or passphrase", &self.recovery)
                            .on_input(Msg::Recovery)
                            .padding(11)
                            .secure(true)
                            .on_submit(Msg::Verify),
                    )
                    .push(
                        button(if self.verifying() {
                            "Verifying…"
                        } else {
                            "Verify device"
                        })
                        .on_press_maybe(
                            (self.connected
                                && !self.verifying()
                                && !self.recovery_unavailable
                                && !self.recovery.trim().is_empty())
                            .then_some(Msg::Verify),
                        ),
                    )
                    .push(text(self.verification_feedback()).size(14))
                    .push(
                        text("Emoji comparison isn’t available with this version of gomuks.")
                            .size(13)
                            .color(MUTED),
                    );
            }
        } else {
            form = form
                .push(text(&self.profile).size(25).font(BOLD))
                .push(text("Verified Matrix device").color(MUTED))
                .push(
                    button("Check call server support")
                        .on_press_maybe(self.connected.then_some(Msg::CheckCalls)),
                )
                .push(text(&self.rtc_status).size(13));
            let public = self.read_sharing.public;
            form = form.push(
                column![
                    text("Read receipts").font(BOLD),
                    text(if public {
                        "Room members can see what you’ve read."
                    } else {
                        "Read receipts are private to your account."
                    })
                    .size(13)
                    .color(MUTED),
                    button(
                        text(if public {
                            "Keep receipts private"
                        } else {
                            "Share receipts with room members"
                        })
                        .size(14)
                    )
                    .style(nav(public))
                    .on_press_maybe(
                        (self.connected && !self.changing_read_sharing())
                            .then_some(Msg::ShareReads(!public))
                    )
                ]
                .spacing(4),
            );
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
                    .style(nav(dnd == Some(true)))
                    .on_press_maybe(
                        dnd.filter(|_| self.connected && !self.changing_dnd())
                            .map(|on| Msg::SetDnd(!on)),
                    );
                form=form.push(column![
                    row![text("Do Not Disturb").font(BOLD).width(Fill), control].align_y(Center),
                    text("Pauses Matrix message notifications across your account and silences Untethered’s incoming ringtone. Calls can still be answered.")
                        .size(13).color(MUTED)
                ].spacing(4));
            }
        }
        if (!self.logged_in || self.verified) && !self.status.is_empty() {
            form = form.push(text(&self.status).size(14).color(MUTED));
        }
        if !self.logged_in || self.verified {
            form = form.push(
                button(if self.logged_in {
                    "Back to rooms"
                } else {
                    "Back to preview"
                })
                .on_press(Msg::Back)
                .style(button::secondary),
            );
        }
        container(container(form).padding(30).style(|_| surface(SIDE, 12.)))
            .center(Fill)
            .style(|_| surface(RAIL, 0.))
            .into()
    }
}
