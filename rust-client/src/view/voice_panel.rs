//! The call's controls as Discord lays them out (calls plan U1): a Voice
//! Connected panel above the user panel, with the call's status, a link back
//! to its stage, Disconnect, and the Camera and Screen buttons; and Mute and
//! Deafen in the user panel, each with a menu of devices. They work before
//! joining too, and the call starts muted or deafened.
//!
//! Until the Voice & Video page (U2) and participant menus (U3), a Voice
//! Settings popup keeps the rest reachable: push-to-talk, input sensitivity
//! and voice processing, the camera, shared audio and each participant's
//! volume.
//!
//! Every control is tagged for the QA control map (qa_layout.rs).
use super::{
    ACTIVE, BOLD, CHANNEL, El, FLOAT, GREEN, LINE, MUTED, Msg, RED, TEXT, TRAY, label, nav,
    popover, surface, voice_settings,
};
use crate::{
    App,
    call_devices::Port,
    desktop_call::{ActiveCall, TalkGate, VideoInput, VoiceMenu},
    qa_layout::tag,
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Length, Padding, color,
    widget::{
        Column, Space, button, checkbox, column, container, pick_list, row, scrollable, slider,
        text, tooltip,
    },
};

use super::icons::{ICON, Icon, hover_icon, icon, with_tip};

/// "Connecting" and "Reconnecting" on Discord.
const YELLOW: Color = color!(0xf0b232);
/// A Camera or Screen button: off, starting, and live.
const IDLE: Color = color!(0x383a40);
const STARTING: Color = color!(0x515468);
const LIVE: Color = color!(0x23764e);
/// A menu item under the pointer.
const PICK: Color = color!(0x4752c4);
const SETTINGS_WIDTH: f32 = 300.;

/// The call is running: not ending or ended.
fn running(call: &ActiveCall) -> bool {
    !call.cancel.is_closed()
}

fn connected(call: &ActiveCall) -> bool {
    call.label == "Connected" || call.label.starts_with("In voice ·")
}

impl App {
    /// The call's status for its panel and its colour.
    fn call_status(call: &ActiveCall) -> (&str, Color) {
        if !running(call) {
            (call.label.as_str(), MUTED)
        } else if connected(call) {
            ("Voice Connected", GREEN)
        } else {
            (call.label.as_str(), YELLOW)
        }
    }

    /// "Room / Space": the call's room and a space it's in.
    fn call_place(&self, call: &ActiveCall) -> String {
        let room = self.live.list.get(&call.room);
        let name = room.map_or(if call.group { "Voice room" } else { "DM" }, |r| {
            r.name.as_str()
        });
        let space = self
            .live
            .children
            .iter()
            .find(|(_, rooms)| rooms.contains(&call.room))
            .and_then(|(space, _)| self.live.list.get(space));
        match space {
            Some(space) if call.group => format!("{name} / {}", space.name),
            _ => name.to_owned(),
        }
    }

    /// The Voice Connected panel, while there's a call.
    pub(super) fn voice_panel(&self) -> Option<El<'_>> {
        let call = self.call.as_ref()?;
        let active = running(call);
        let mode = call.audio.borrow();
        let (status, color) = Self::call_status(call);
        let disconnect = button(hover_icon(Icon::PhoneOff, 20., 32., ICON, TEXT))
            .padding(0)
            .style(plain)
            .on_press_maybe(active.then_some(Msg::EndCall));
        let settings_open = self.voice_popup == Some(VoiceMenu::Settings);
        let settings = button(hover_icon(
            Icon::Gear,
            18.,
            32.,
            if settings_open { TEXT } else { ICON },
            TEXT,
        ))
        .padding(0)
        .style(plain)
        .on_press(Msg::VoiceMenu(
            (!settings_open).then_some(VoiceMenu::Settings),
        ));
        let header = row![
            icon(Icon::Signal, 18., color),
            tag(
                "call-status",
                if active && connected(call) {
                    "connected"
                } else {
                    ""
                },
                "",
                text(status)
                    .size(14)
                    .font(BOLD)
                    .color(color)
                    .wrapping(text::Wrapping::None),
            ),
            Space::new().width(Fill),
            tag(
                "voice-settings-open",
                if settings_open { "on" } else { "off" },
                "Voice Settings",
                with_tip(settings, "Voice Settings", tooltip::Position::Top),
            ),
            tag(
                "disconnect",
                "",
                "Disconnect",
                with_tip(disconnect, "Disconnect", tooltip::Position::Top),
            ),
        ]
        .spacing(6)
        .align_y(Center);
        // The room's name opens its stage, as Discord's link does.
        let place = button(
            text(self.call_place(call))
                .size(12)
                .color(MUTED)
                .wrapping(text::Wrapping::None),
        )
        .padding(0)
        .style(|_, s| button::Style {
            text_color: if s == button::Status::Hovered {
                TEXT
            } else {
                MUTED
            },
            ..Default::default()
        })
        .on_press_maybe(active.then_some(Msg::ReturnToCall(call.id)));
        let place = container(tag("call-room", "", "Open the call", place))
            .padding(Padding {
                left: 24.,
                ..Padding::ZERO
            })
            .clip(true);
        let mut media = row![].spacing(8);
        for (input, glyph, name) in [
            (VideoInput::Camera, Icon::Video, "camera"),
            (VideoInput::Window, Icon::ScreenShare, "screen"),
        ] {
            let slot = input as usize;
            let requested = mode.video[slot].is_some();
            let live = call.video_status[slot].live;
            let fill = match (requested, live) {
                (true, true) => LIVE,
                (true, false) => STARTING,
                _ => IDLE,
            };
            let tip = input.control(requested, live);
            let control = button(
                container(icon(glyph, 20., TEXT))
                    .center_x(Fill)
                    .center_y(32),
            )
            .width(Fill)
            .padding(0)
            .style(move |_, status| {
                let background = match status {
                    button::Status::Hovered | button::Status::Pressed => Color {
                        r: fill.r + 0.06,
                        g: fill.g + 0.06,
                        b: fill.b + 0.06,
                        ..fill
                    },
                    button::Status::Disabled => Color { a: 0.45, ..fill },
                    _ => fill,
                };
                button::Style {
                    background: Some(background.into()),
                    border: Border::default().rounded(8),
                    ..Default::default()
                }
            })
            .on_press_maybe(active.then_some(Msg::ToggleVideo(input)));
            let state = match (requested, live) {
                (true, true) => "live",
                (true, false) => "starting",
                _ => "off",
            };
            media = media.push(
                container(tag(
                    name,
                    state,
                    tip,
                    with_tip(control, tip, tooltip::Position::Top),
                ))
                .width(Length::FillPortion(1)),
            );
        }
        let notes = self.call_notes(call, &mode);
        let panel = column![header, place, media]
            .push(tag("call-notes", "", "", notes))
            .spacing(6);
        Some(
            container(panel)
                .width(Fill)
                .padding(Padding {
                    top: 9.,
                    bottom: 8.,
                    left: 10.,
                    right: 8.,
                })
                .style(|_| surface(TRAY, 0.))
                .into(),
        )
    }

    /// The call's status lines under the panel's controls, one per line:
    /// push-to-talk, camera and share errors, device failures, app audio.
    fn call_notes<'a>(
        &'a self,
        call: &'a ActiveCall,
        mode: &crate::desktop_call::AudioMode,
    ) -> El<'a> {
        let note = |s: String| -> El<'a> { text(s).size(12).color(MUTED).into() };
        let mut notes = Column::new().spacing(2);
        if mode.talk != TalkGate::Open {
            notes = notes.push(note(if mode.deafened {
                "Undeafen to use push-to-talk.".into()
            } else if mode.muted {
                "Unmute to use push-to-talk.".into()
            } else {
                self.hotkeys.hint(mode.talk != TalkGate::Waiting)
            }));
        }
        for status in &call.video_status {
            if !status.label.is_empty() {
                notes = notes.push(note(status.label.clone()));
            }
        }
        let screen = VideoInput::Window as usize;
        if call.video_status[screen].live && mode.video[screen].is_some() {
            notes = notes.push(note(if call.app_note.is_empty() {
                "Window audio is off · choose it in Voice Settings.".into()
            } else {
                call.app_note.clone()
            }));
        }
        // Someone shares their screen: whether its sound plays, at hand.
        if call.previews.remote_share() {
            let id = call.id;
            notes = notes.push(tag(
                "share-sound-panel",
                if mode.share_silent { "off" } else { "on" },
                "",
                checkbox(!mode.share_silent)
                    .label("Play shared audio")
                    .size(14)
                    .text_size(12)
                    .on_toggle_maybe(
                        running(call).then_some(move |enabled| Msg::SetShareSound(id, enabled)),
                    ),
            ));
        }
        if call.mic_error {
            notes = notes.push(note(
                "Microphone unavailable · choose an input under Mute's menu.".into(),
            ));
        }
        if call.speaker_error {
            notes = notes.push(note(
                "Speaker unavailable · choose an output under Deafen's menu.".into(),
            ));
        }
        notes.into()
    }

    /// Mute and Deafen, each with its device menu's chevron, for the user
    /// panel. They work with or without a call.
    pub(super) fn voice_buttons(&self) -> El<'_> {
        let local = &self.local_audio;
        let (muted, deafened) = (local.muted, local.deafened);
        let mic_off = muted || deafened;
        let mic_tip = if deafened {
            "Mic muted while deafened"
        } else if muted {
            "Unmute"
        } else {
            "Mute"
        };
        let mute = self.voice_toggle(
            "mute",
            if deafened {
                "deafened"
            } else if muted {
                "on"
            } else {
                "off"
            },
            if mic_off { Icon::MicOff } else { Icon::Mic },
            mic_off,
            mic_tip,
            (!deafened).then_some(Msg::ToggleMic),
            VoiceMenu::Input,
        );
        let deafen = self.voice_toggle(
            "deafen",
            if deafened { "on" } else { "off" },
            if deafened {
                Icon::HeadphonesOff
            } else {
                Icon::Headphones
            },
            deafened,
            if deafened { "Undeafen" } else { "Deafen" },
            Some(Msg::ToggleDeafen),
            VoiceMenu::Output,
        );
        row![mute, deafen].align_y(Center).into()
    }

    /// One of Mute and Deafen: its button and its menu's chevron.
    #[allow(clippy::too_many_arguments)]
    fn voice_toggle(
        &self,
        name: &'static str,
        state: &'static str,
        glyph: Icon,
        off: bool,
        tip: &'static str,
        action: Option<Msg>,
        menu: VoiceMenu,
    ) -> El<'_> {
        let color = if off { RED } else { ICON };
        let hover = if off { RED } else { TEXT };
        let main = button(hover_icon(glyph, 20., 30., color, hover))
            .padding(0)
            .style(hovered)
            .on_press_maybe(action);
        let open = self.voice_popup == Some(menu);
        let chevron = button(
            container(icon(Icon::ChevronDown, 10., if open { TEXT } else { ICON }))
                .center_x(12)
                .center_y(30),
        )
        .padding(0)
        .style(hovered)
        .on_press(Msg::VoiceMenu((!open).then_some(menu)));
        let (menu_name, menu_tip) = match menu {
            VoiceMenu::Input => ("mute-menu", "Input Options"),
            _ => ("deafen-menu", "Output Options"),
        };
        // No tooltip over its own open menu.
        let chevron: El<'_> = if open {
            chevron.into()
        } else {
            with_tip(chevron, menu_tip, tooltip::Position::Top)
        };
        let chevron = popover::popover(
            tag(
                menu_name,
                if open { "on" } else { "off" },
                menu_tip,
                chevron,
            ),
            open.then(|| self.device_menu(menu)),
            popover::Place::Below,
            Msg::VoiceMenu(None),
        );
        row![
            tag(
                name,
                state,
                tip,
                with_tip(main, tip, tooltip::Position::Top)
            ),
            chevron
        ]
        .align_y(Center)
        .into()
    }

    /// A device menu: the inputs or outputs, the chosen one ticked, then
    /// Refresh and Voice Settings.
    fn device_menu(&self, menu: VoiceMenu) -> El<'_> {
        let local = &self.local_audio;
        let (port, title, prefix) = match menu {
            VoiceMenu::Input => (Port::Mic, "INPUT DEVICE", "input"),
            _ => (Port::Speaker, "OUTPUT DEVICE", "output"),
        };
        let route = local.route(port);
        let chosen = local.devices.selected(port, &route);
        let mut items = column![container(label(title)).padding([4, 8])].spacing(1);
        // A chosen device that's gone shows as unavailable, first.
        let listed = local.devices.list(port);
        let unavailable = (!listed.contains(&chosen)).then(|| chosen.clone());
        for (i, choice) in unavailable.iter().chain(listed).enumerate() {
            let selected = *choice == chosen;
            let line = row![
                text(choice.to_string())
                    .size(14)
                    .width(Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
            ]
            .extend(selected.then(|| icon(Icon::Check, 16., TEXT)))
            .spacing(6)
            .align_y(Center);
            let item = button(line)
                .width(Fill)
                .padding([6, 8])
                .style(menu_item)
                .on_press_maybe(
                    listed
                        .contains(choice)
                        .then(|| Msg::SelectDevice(port, choice.clone())),
                );
            items = items.push(tag(
                format!("{prefix}-device-{i}"),
                if selected { "selected" } else { "" },
                "",
                item,
            ));
        }
        if let Some(error) = local.list_error {
            items = items.push(container(text(error).size(12).color(MUTED)).padding([4, 8]));
        }
        let refresh = button(
            text(if local.loading {
                "Looking for devices…"
            } else {
                "Refresh devices"
            })
            .size(14),
        )
        .width(Fill)
        .padding([6, 8])
        .style(menu_item)
        .on_press_maybe((!local.loading).then_some(Msg::RefreshDevices));
        let settings = button(
            row![
                text("Voice Settings").size(14).width(Fill),
                icon(Icon::Gear, 16., ICON)
            ]
            .align_y(Center),
        )
        .width(Fill)
        .padding([6, 8])
        .style(menu_item)
        .on_press(Msg::VoiceMenu(Some(VoiceMenu::Settings)));
        items = items
            .push(separator())
            .push(tag(format!("{prefix}-refresh"), "", "", refresh))
            .push(tag(format!("{prefix}-settings"), "", "", settings));
        tag(
            format!("{prefix}-menu"),
            "",
            "",
            container(items)
                .width(240)
                .padding(6)
                .style(|_| container::Style {
                    border: Border {
                        color: color!(0x26272b),
                        width: 1.,
                        radius: 8.0.into(),
                    },
                    ..surface(FLOAT, 8.)
                }),
        )
    }

    /// The bottom of the sidebar: the Voice Connected panel during a call,
    /// then the user panel, with the voice settings popup beside them.
    pub(super) fn call_corner<'a>(&'a self, user_panel: El<'a>) -> El<'a> {
        // The panel's slot is always there, so the user panel keeps its
        // place, and its widgets their state, as calls come and go.
        let panel = self
            .voice_panel()
            .unwrap_or_else(|| Space::new().height(0).into());
        let corner = column![panel, user_panel];
        self.with_voice_settings(corner)
    }

    /// `anchor` with the voice settings popup beside it while it's open:
    /// right of it, at the window's foot, so the voice roster and Mute and
    /// Deafen stay in sight.
    pub(super) fn with_voice_settings<'a>(&'a self, anchor: impl Into<El<'a>>) -> El<'a> {
        let open = self.voice_popup == Some(VoiceMenu::Settings);
        popover::popover(
            anchor,
            open.then(|| self.voice_settings()),
            // The popup is SETTINGS_WIDTH wide: its left edge 8 px right of
            // the anchor's. Dropped far below, it's moved up to fit.
            popover::Place::UnderCorner {
                inset: -(SETTINGS_WIDTH + 8.),
                drop: 10_000.,
            },
            Msg::VoiceMenu(None),
        )
    }

    /// Where the call is shown without the sidebar (theater, account
    /// settings, the sign-in page): the panel and Mute and Deafen.
    pub(super) fn call_dock_compact(&self) -> Option<El<'_>> {
        let panel = self.voice_panel()?;
        let buttons = container(
            row![Space::new().width(Fill), self.voice_buttons()]
                .align_y(Center)
                .spacing(4),
        )
        .width(Fill)
        .padding([4, 8])
        .style(|_| surface(TRAY, 0.));
        Some(
            self.with_voice_settings(
                container(column![panel, buttons])
                    .width(240)
                    .style(|_| container::Style {
                        border: Border::default().rounded(8),
                        ..surface(TRAY, 8.)
                    })
                    .clip(true),
            ),
        )
    }

    /// The voice settings that have no page yet.
    fn voice_settings(&self) -> El<'_> {
        let call = self.call.as_ref().filter(|c| running(c));
        let mut body = column![
            text("Voice Settings").size(16).font(BOLD).color(TEXT),
            tag(
                "ptt",
                if self.room_ptt { "on" } else { "off" },
                "",
                checkbox(self.room_ptt)
                    .label("Push-to-talk in voice channels")
                    .size(16)
                    .text_size(13)
                    .on_toggle(Msg::RoomPtt),
            ),
            voice_settings::section(
                self.voice.voice,
                call.map(|c| *c.input.borrow()).unwrap_or_default(),
                self.voice.note,
                true,
            ),
        ]
        .spacing(10)
        .width(Fill);
        if let Some(call) = call {
            body = body.push(self.camera_settings(call));
            body = body.push(self.share_settings(call));
            body = body.push(self.volume_settings(call));
        }
        let scrollbar = scrollable::Scrollbar::new()
            .width(4)
            .scroller_width(4)
            .spacing(4);
        tag(
            "voice-settings",
            "",
            "",
            container(
                scrollable(container(body).padding(Padding {
                    right: 10.,
                    ..Padding::ZERO
                }))
                .direction(scrollable::Direction::Vertical(scrollbar)),
            )
            .width(SETTINGS_WIDTH)
            .max_height(560)
            .padding(Padding {
                top: 12.,
                bottom: 12.,
                left: 14.,
                right: 4.,
            })
            .style(|_| container::Style {
                border: Border {
                    color: color!(0x26272b),
                    width: 1.,
                    radius: 8.0.into(),
                },
                ..surface(FLOAT, 8.)
            }),
        )
    }

    fn camera_settings<'a>(&'a self, call: &'a ActiveCall) -> El<'a> {
        let mode = call.audio.borrow();
        let id = call.id;
        let mut cams = column![label("CAMERA")].spacing(5);
        if !call.cameras.is_empty() {
            cams = cams.push(tag(
                "camera-list",
                "",
                "",
                pick_list(
                    call.cameras.as_slice(),
                    Some(mode.camera.clone()),
                    move |choice| Msg::SelectCamera(id, choice),
                )
                .width(Fill)
                .text_size(13)
                .padding([5, 9]),
            ));
        }
        cams = cams.push(tag(
            "cameras-refresh",
            "",
            "",
            button(
                text(if call.camera_listing {
                    "Looking…"
                } else if call.cameras.is_empty() {
                    "List cameras"
                } else {
                    "Refresh cameras"
                })
                .size(13),
            )
            .style(nav(false))
            .on_press_maybe((!call.camera_listing).then_some(Msg::ListCameras)),
        ));
        if let Some(note) = call.camera_note {
            cams = cams.push(text(note).size(12).color(MUTED));
        }
        cams.into()
    }

    /// The window share's app audio, and whether shared audio plays.
    fn share_settings<'a>(&'a self, call: &'a ActiveCall) -> El<'a> {
        let mode = call.audio.borrow();
        let id = call.id;
        let mut share = column![label("SCREEN SHARE AUDIO")].spacing(5);
        let screen = VideoInput::Window as usize;
        if call.video_status[screen].live
            && let Some(window) = mode.video[screen]
        {
            let streams = &self.local_audio.devices.streams;
            if streams.is_empty() {
                share = share.push(
                    text("Play sound in the app, then refresh devices.")
                        .size(12)
                        .color(MUTED),
                );
            } else {
                share = share.push(tag(
                    "app-audio",
                    if mode.send_app.is_some() { "on" } else { "off" },
                    "",
                    pick_list(
                        streams.as_slice(),
                        mode.send_app.as_ref().map(|(_, s)| s.clone()),
                        move |s| Msg::ShareApp(id, window, Some(s)),
                    )
                    .placeholder("Send sound from…")
                    .width(Fill)
                    .text_size(13),
                ));
            }
            let mut buttons = row![tag(
                "devices-refresh",
                "",
                "",
                button(text("Refresh devices").size(13))
                    .style(nav(false))
                    .on_press_maybe((!self.local_audio.loading).then_some(Msg::RefreshDevices)),
            )]
            .spacing(6);
            if mode.send_app.is_some() {
                buttons = buttons.push(tag(
                    "app-audio-stop",
                    "",
                    "",
                    button(text("Stop app audio").size(13))
                        .style(nav(false))
                        .on_press(Msg::ShareApp(id, window, None)),
                ));
            }
            share = share.push(buttons);
        }
        share
            .push(tag(
                "share-sound",
                if mode.share_silent { "off" } else { "on" },
                "",
                checkbox(!mode.share_silent)
                    .label("Play shared audio")
                    .size(16)
                    .text_size(13)
                    .on_toggle(move |enabled| Msg::SetShareSound(id, enabled)),
            ))
            .into()
    }

    /// Each participant's local volume: Discord's User Volume (U3 moves it
    /// to their menu).
    fn volume_settings<'a>(&'a self, call: &'a ActiveCall) -> El<'a> {
        let mode = call.audio.borrow();
        let id = call.id;
        let mut rows = column![label("USER VOLUME")].spacing(5);
        let Some(room) = self.live.list.get(&call.room) else {
            return rows.into();
        };
        let mut peers = match &call.peer {
            Some(peer) => std::collections::BTreeSet::from([peer.as_str()]),
            None => room.voice_users(),
        };
        peers.remove(self.matrix_id.as_str());
        if peers.is_empty() {
            return rows
                .push(
                    text("No other participants in voice.")
                        .size(12)
                        .color(MUTED),
                )
                .into();
        }
        for (i, &user) in peers.iter().enumerate() {
            let gain = mode.volume(user);
            let line = row![
                container(tooltip(
                    text(room.display(user)).size(13).color(TEXT),
                    container(text(user).size(13)).padding([4, 7]),
                    tooltip::Position::Bottom
                ))
                .width(Fill)
                .clip(true),
                text(format!("{gain}%")).size(12).color(CHANNEL).width(38),
                tag(
                    format!("volume-reset-{i}"),
                    "",
                    "",
                    button(text("Reset").size(12))
                        .padding([2, 6])
                        .style(nav(false))
                        .on_press(Msg::PeerVolume(id, user.into(), 100)),
                ),
            ]
            .spacing(4)
            .align_y(Center);
            let level = slider(0..=200u8, gain, move |v| {
                Msg::PeerVolume(id, user.into(), v)
            })
            .default(100u8)
            .step(5u8)
            .shift_step(1u8)
            .width(Fill);
            rows = rows.push(tag(
                format!("volume-{i}"),
                "",
                "",
                column![line, tag(format!("volume-slider-{i}"), "", "", level)].spacing(2),
            ));
        }
        rows.into()
    }
}

fn separator() -> El<'static> {
    container(
        container(Space::new())
            .width(Fill)
            .height(1)
            .style(|_| surface(LINE, 0.)),
    )
    .padding([4, 6])
    .into()
}

fn plain(_: &iced::Theme, _: button::Status) -> button::Style {
    button::Style::default()
}

fn hovered(_: &iced::Theme, s: button::Status) -> button::Style {
    button::Style {
        background: matches!(s, button::Status::Hovered | button::Status::Pressed)
            .then(|| ACTIVE.into()),
        border: Border::default().rounded(4),
        ..Default::default()
    }
}

fn menu_item(_: &iced::Theme, s: button::Status) -> button::Style {
    let hovered = matches!(s, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then(|| PICK.into()),
        text_color: if hovered {
            Color::WHITE
        } else if s == button::Status::Disabled {
            MUTED
        } else {
            color!(0xdbdee1)
        },
        border: Border::default().rounded(4),
        ..Default::default()
    }
}
