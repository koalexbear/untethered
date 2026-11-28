//! User Settings, as on Discord: a sidebar of pages on the left, the page on
//! the right with a close control that returns to the conversation. My
//! Account is `settings_view` in view.rs; Voice & Video (calls plan U2) is
//! here. S1 adds Appearance, Notifications, Keybinds and Privacy as more
//! pages.
use super::{
    ACTIVE, BODY, CHAT, El, HOVER, ICON, Icon, Msg, SIDE, TEXT, icon, label, scroll_style, surface,
    thin_scrollbar, with_tip,
};
use crate::App;
use iced::{
    Alignment::Center,
    Border, Fill, Length, Padding,
    widget::{Column, button, column, container, row, scrollable, stack, text, tooltip},
};

/// A page of User Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Account,
    /// Voice & Video: devices, input mode, processing, camera. Calls exist
    /// only on Linux so far.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Voice,
}

/// The page's scrolling area, scrolled back to its top when the page
/// changes.
pub(crate) const SCROLL: &str = "user-settings-page";

/// `content` as a control of the QA control map (calls only exist on Linux).
#[cfg(target_os = "linux")]
fn tagged<'a>(name: &'static str, state: &'static str, content: impl Into<El<'a>>) -> El<'a> {
    crate::qa_layout::tag(name, state, "", content)
}
#[cfg(not(target_os = "linux"))]
fn tagged<'a>(_: &'static str, _: &'static str, content: impl Into<El<'a>>) -> El<'a> {
    content.into()
}

impl App {
    /// The sidebar's pages.
    fn settings_nav(&self) -> El<'_> {
        let item = |name: &'static str, title: &'static str, page: Page| -> El<'static> {
            let selected = self.settings_page == page;
            tagged(
                name,
                if selected { "selected" } else { "" },
                button(text(title).size(16))
                    .width(Fill)
                    .padding([6, 10])
                    .style(move |_, s| button::Style {
                        background: if selected {
                            Some(ACTIVE.into())
                        } else {
                            matches!(s, button::Status::Hovered | button::Status::Pressed)
                                .then(|| HOVER.into())
                        },
                        text_color: if selected { TEXT } else { BODY },
                        border: Border::default().rounded(4),
                        ..Default::default()
                    })
                    .on_press(Msg::SettingsPage(page)),
            )
        };
        let nav = column![
            container(label("USER SETTINGS")).padding([6, 10]),
            item("settings-account", "My Account", Page::Account),
        ];
        #[cfg(target_os = "linux")]
        let nav = nav.push(item("settings-voice", "Voice & Video", Page::Voice));
        nav.spacing(2).width(218).into()
    }

    /// User Settings around `page`: the sidebar, the page scrolling, the
    /// close control, and a dialog over them while one is open.
    pub(super) fn settings_frame<'a>(&'a self, page: Column<'a, Msg>) -> El<'a> {
        let left = container(self.settings_nav())
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
        let close = column![
            tagged(
                "settings-close",
                "",
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
                )
            ),
            text("ESC").size(13).font(super::BOLD).color(super::CHANNEL)
        ]
        .spacing(8)
        .align_x(Center);
        let right = container(
            row![
                tagged(
                    "settings-page",
                    "",
                    scrollable(container(page.max_width(740)).padding(Padding {
                        top: 60.,
                        right: 20.,
                        bottom: 60.,
                        left: 40.,
                    }))
                    .id(SCROLL)
                    .direction(thin_scrollbar())
                    .style(scroll_style(None))
                    .width(Fill)
                    .height(Fill),
                ),
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
        // The dialog's slot is always there, so the page keeps its widgets'
        // state as the dialog comes and goes.
        #[cfg(target_os = "linux")]
        let dialog = self.key_dialog();
        #[cfg(not(target_os = "linux"))]
        let dialog: El<'_> = iced::widget::Space::new().into();
        stack![row![left, right].height(Fill), dialog].into()
    }
}

#[cfg(target_os = "linux")]
mod voice {
    use super::super::{
        ACTIVE, BODY, BOLD, CHANNEL, CHAT, El, FLOAT, ICON, Icon, LINE, MUTED, Msg, SIDE, TEXT,
        icon, label, popover, primary, surface, voice_settings,
    };
    use crate::{
        App,
        call_devices::Port,
        call_settings::{Change, Event, RELEASE_RANGE, VOLUME_RANGE},
        desktop_call::{CameraChoice, VideoInput, VoiceMenu},
        qa_layout::tag,
    };
    use iced::{
        Alignment::Center,
        Border, Color, Fill, Length, Padding, Theme, color,
        widget::{
            Column, Space, button, column, container, image, opaque, radio, row, scrollable,
            slider, text,
        },
    };

    /// A group of the page: its heading over its rows.
    fn group<'a>(heading: &'static str, body: impl Into<El<'a>>) -> El<'a> {
        column![label(heading), body.into()].spacing(8).into()
    }

    fn note<'a>(s: impl text::IntoFragment<'a>) -> El<'a> {
        text(s).size(13).color(MUTED).into()
    }

    fn divider() -> El<'static> {
        container(Space::new())
            .width(Fill)
            .height(1)
            .style(|_| surface(LINE, 0.))
            .into()
    }

    /// A grey button, as Discord's secondary buttons.
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

    /// A row of a list: its text, ticked when chosen.
    fn choice<'a>(label: String, chosen: bool, action: Option<Msg>) -> El<'a> {
        let line = row![
            text(label)
                .size(14)
                .width(Fill)
                .wrapping(text::Wrapping::WordOrGlyph)
        ]
        .extend(chosen.then(|| icon(Icon::Check, 16., TEXT)))
        .spacing(6)
        .align_y(Center);
        button(line)
            .width(Fill)
            .padding([6, 8])
            .style(move |_, s| {
                let hovered = matches!(s, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: if chosen {
                        Some(ACTIVE.into())
                    } else {
                        hovered.then(|| color!(0x4752c4).into())
                    },
                    text_color: if chosen || hovered {
                        Color::WHITE
                    } else {
                        BODY
                    },
                    border: Border::default().rounded(4),
                    ..Default::default()
                }
            })
            .on_press_maybe(action)
            .into()
    }

    /// A volume: its percent, and a slider from 0 to 200 % that arrow keys
    /// move by 5 while the pointer is over it.
    fn volume(
        name: &'static str,
        title: &'static str,
        value: u8,
        set: fn(u8) -> Change,
    ) -> El<'static> {
        let level = slider(VOLUME_RANGE, value, move |v| Msg::Voice(Event::Set(set(v))))
            .default(100u8)
            .step(5u8)
            .shift_step(1u8)
            .width(Fill);
        let slider_name = match name {
            "input-volume" => "input-volume-slider",
            _ => "output-volume-slider",
        };
        tag(
            name,
            "",
            format!("{value}%"),
            column![
                row![
                    label(title),
                    Space::new().width(Fill),
                    text(format!("{value}%")).size(12).color(CHANNEL),
                ]
                .align_y(Center),
                tag(slider_name, "", "", level),
            ]
            .spacing(6),
        )
    }

    impl App {
        /// A device list's dropdown on the page: the chosen device, and its
        /// list below it while open.
        fn device_picker(&self, port: Port) -> El<'_> {
            let local = &self.local_audio;
            let (name, menu, title) = match port {
                Port::Mic => ("settings-input", VoiceMenu::PageInput, "INPUT DEVICE"),
                Port::Speaker => ("settings-output", VoiceMenu::PageOutput, "OUTPUT DEVICE"),
            };
            let route = local.route(port);
            let listed = local.devices.list(port);
            let chosen = local.devices.selected(port, &route);
            // Until the list has been read, a saved device shows by name.
            let shown = if listed.contains(&chosen) || !local.loading {
                chosen.to_string()
            } else {
                route
                    .name
                    .clone()
                    .unwrap_or_else(|| "System default".into())
            };
            let open = self.voice_popup == Some(menu);
            let field = button(
                row![
                    text(shown)
                        .size(14)
                        .color(TEXT)
                        .width(Fill)
                        .wrapping(text::Wrapping::None),
                    icon(Icon::ChevronDown, 14., if open { TEXT } else { ICON }),
                ]
                .spacing(6)
                .align_y(Center),
            )
            .width(Fill)
            .padding([9, 10])
            .style(|_, _| button::Style {
                background: Some(color!(0x1e1f22).into()),
                border: Border::default().rounded(4),
                ..Default::default()
            })
            .on_press(Msg::VoiceMenu((!open).then_some(menu)));
            let list = open.then(|| {
                let unavailable = (!listed.contains(&chosen)).then(|| chosen.clone());
                let mut items = column![].spacing(1);
                for (i, device) in unavailable.iter().chain(listed).enumerate() {
                    let selected = *device == chosen;
                    items = items.push(tag(
                        format!("{name}-{i}"),
                        if selected { "selected" } else { "" },
                        "",
                        choice(
                            device.to_string(),
                            selected,
                            listed
                                .contains(device)
                                .then(|| Msg::SelectDevice(port, device.clone())),
                        ),
                    ));
                }
                if let Some(error) = local.list_error {
                    items = items.push(container(note(error)).padding([4, 8]));
                }
                items = items.push(tag(
                    format!("{name}-refresh"),
                    "",
                    "",
                    choice(
                        if local.loading {
                            "Looking for devices…".into()
                        } else {
                            "Refresh devices".into()
                        },
                        false,
                        (!local.loading).then_some(Msg::RefreshDevices),
                    ),
                ));
                let menu = match port {
                    Port::Mic => "settings-input-menu",
                    Port::Speaker => "settings-output-menu",
                };
                tag(
                    menu,
                    "",
                    "",
                    container(scrollable(items).height(Length::Shrink))
                        .width(330)
                        .max_height(320)
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
            });
            column![
                label(title),
                popover::popover(
                    tag(name, if open { "on" } else { "off" }, "", field),
                    list,
                    popover::Place::Below,
                    Msg::VoiceMenu(None),
                ),
            ]
            .spacing(8)
            .width(Fill)
            .into()
        }

        /// The push-to-talk key: its name, the binding dialog's button, the
        /// default, and what the desktop bound.
        fn talk_key_settings(&self) -> El<'_> {
            let key = crate::hotkeys::window_key(self.voice.talk_key.as_deref());
            let name = crate::hotkeys::label(&key);
            let field = container(text(name.clone()).size(14).font(BOLD).color(TEXT))
                .width(Fill)
                .padding([9, 10])
                .style(|_| surface(color!(0x1e1f22), 4.));
            let mut controls = row![
                tag("ptt-key", "", name, field),
                tag(
                    "ptt-key-record",
                    if self.voice.recording { "on" } else { "off" },
                    "",
                    button(text("Record Keybind").size(14))
                        .padding([9, 14])
                        .style(secondary)
                        .on_press(Msg::Voice(Event::RecordKey(true))),
                ),
            ]
            .spacing(8)
            .align_y(Center);
            if self.voice.talk_key.is_some() {
                controls = controls.push(tag(
                    "ptt-key-default",
                    "",
                    "",
                    button(text("Default").size(14))
                        .padding([9, 14])
                        .style(secondary)
                        .on_press(Msg::Voice(Event::DefaultKey)),
                ));
            }
            let mut body = column![label("SHORTCUT"), controls].spacing(8);
            // On Wayland the compositor owns desktop-wide keys: it confirms
            // the key the first time and keeps the one confirmed.
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                body = body.push(note(match &self.hotkeys.talk {
                    Some(bound) => format!(
                        "Your desktop holds push to talk on {bound} in other apps. Change it in its shortcut settings."
                    ),
                    None => "Your desktop asks to confirm this key for other apps the first time push to talk is on, and keeps the key you confirm.".into(),
                }));
            }
            body.into()
        }

        /// The binding dialog, over the page while it listens.
        pub(in crate::view) fn key_dialog(&self) -> El<'_> {
            if !(self.voice.recording && self.voice_page_open()) {
                return Space::new().into();
            }
            let mut card = column![
                text("Push to Talk").size(20).font(BOLD).color(TEXT),
                text("Press the key you want to hold to talk. A modifier such as Right Ctrl works on its own.")
                    .size(14)
                    .color(BODY),
            ]
            .spacing(12)
            .width(400);
            if let Some(why) = self.voice.key_note {
                card = card.push(note(why));
            }
            card = card.push(row![
                Space::new().width(Fill),
                tag(
                    "ptt-key-cancel",
                    "",
                    "",
                    button(text("Cancel").size(14))
                        .padding([9, 16])
                        .style(secondary)
                        .on_press(Msg::Voice(Event::RecordKey(false))),
                )
            ]);
            opaque(
                container(tag(
                    "ptt-key-dialog",
                    "",
                    "",
                    container(card).padding(20).style(|_| surface(CHAT, 8.)),
                ))
                .center(Fill)
                .style(|_| surface(Color::from_rgba8(0, 0, 0, 0.7), 0.)),
            )
        }

        /// The camera: the list (the saved one ticked before any list is
        /// read), and a preview that publishes nothing.
        fn camera_settings(&self) -> El<'_> {
            let local = &self.local_audio;
            let chosen = CameraChoice::saved(self.voice.camera.as_ref());
            let saved = [chosen.clone()];
            let listed: &[CameraChoice] = if local.cameras.is_empty() {
                &saved
            } else {
                &local.cameras
            };
            let mut list = column![].spacing(1);
            for (i, camera) in listed.iter().enumerate() {
                let selected = *camera == chosen;
                list = list.push(tag(
                    format!("camera-item-{i}"),
                    if selected { "selected" } else { "" },
                    "",
                    choice(
                        camera.to_string(),
                        selected,
                        (!selected && !local.cameras.is_empty())
                            .then(|| Msg::SelectCamera(camera.clone())),
                    ),
                ));
            }
            let refresh = tag(
                "cameras-refresh",
                "",
                "",
                button(
                    text(if local.camera_listing {
                        "Looking…"
                    } else if local.cameras.is_empty() {
                        "List cameras"
                    } else {
                        "Refresh cameras"
                    })
                    .size(14),
                )
                .padding([7, 14])
                .style(secondary)
                .on_press_maybe((!local.camera_listing).then_some(Msg::ListCameras)),
            );
            let mut left = column![label("CAMERA"), list, refresh]
                .spacing(8)
                .width(Fill);
            if let Some(why) = local.camera_note {
                left = left.push(note(why));
            }
            // The preview: the camera's pictures, or why there are none.
            let in_call = self
                .call
                .as_ref()
                .is_some_and(|c| c.audio.borrow().video[VideoInput::Camera as usize].is_some());
            let on = self.voice.preview.is_some();
            let state = match (on, &self.voice.frame) {
                (true, Some(_)) => "live",
                (true, None) => "starting",
                _ => "off",
            };
            let picture: El<'_> = match &self.voice.frame {
                Some(frame) if on => image(frame.clone())
                    .width(Fill)
                    .height(Fill)
                    .content_fit(iced::ContentFit::Contain)
                    .into(),
                _ => container(
                    column![
                        icon(Icon::Video, 28., ICON),
                        text(if on {
                            "Starting the camera…"
                        } else if in_call {
                            "Your camera is on in the call."
                        } else {
                            "Test your camera here. Nothing is sent."
                        })
                        .size(13)
                        .color(MUTED)
                    ]
                    .spacing(8)
                    .align_x(Center),
                )
                .center(Fill)
                .into(),
            };
            let preview = tag(
                "camera-preview",
                state,
                "",
                container(picture)
                    .width(320)
                    .height(180)
                    .style(|_| surface(color!(0x1e1f22), 8.))
                    .clip(true),
            );
            let test = tag(
                "camera-test",
                if on { "on" } else { "off" },
                "",
                button(text(if on { "Stop Video" } else { "Test Video" }).size(14))
                    .padding([7, 14])
                    .style(if on { primary } else { secondary })
                    .on_press_maybe((on || !in_call).then_some(Msg::Voice(Event::Preview(!on)))),
            );
            let mut right = column![preview, test].spacing(8).align_x(Center);
            if let Some(why) = self.voice.preview_note {
                right = right.push(note(why));
            }
            row![left, right].spacing(20).into()
        }

        /// The Voice & Video page.
        pub(in crate::view) fn voice_video_page<'a>(&'a self) -> Column<'a, Msg> {
            let voice = self.voice.voice;
            let running = self.call.as_ref().is_some_and(|c| !c.cancel.is_closed());
            let level = self.input_level().unwrap_or_default();
            // The incoming call and the call's panel keep their slots, empty
            // without a call, so the rows below keep their widgets' state.
            // Each part brings its own space above it.
            let part = |el: Option<El<'a>>| -> El<'a> {
                match el {
                    Some(el) => container(el)
                        .padding(Padding {
                            top: 20.,
                            ..Padding::ZERO
                        })
                        .into(),
                    None => Space::new().height(0).into(),
                }
            };
            let page = column![
                text("Voice & Video").size(20).font(BOLD).color(TEXT),
                part(self.incoming_banner()),
                part(self.call_dock_compact()),
            ];
            let devices = row![
                self.device_picker(Port::Mic),
                self.device_picker(Port::Speaker)
            ]
            .spacing(16);
            let volumes = row![
                volume(
                    "input-volume",
                    "INPUT VOLUME",
                    voice.input_volume,
                    Change::InputVolume
                ),
                volume(
                    "output-volume",
                    "OUTPUT VOLUME",
                    voice.output_volume,
                    Change::OutputVolume
                ),
            ]
            .spacing(16);
            // The mic test: Discord's "Let's Check".
            let testing = self.voice.mic_test.is_some();
            let mut mic = column![
                label("MIC TEST"),
                text(if running {
                    "During a call the meter shows your microphone as the call sends it."
                } else {
                    "Having mic issues? Start a test and say something: you’ll hear yourself as others would."
                })
                .size(14)
                .color(MUTED),
                row![
                    tag(
                        "mic-test",
                        if testing { "on" } else { "off" },
                        "",
                        button(text(if testing { "Stop Testing" } else { "Let’s Check" }).size(14))
                            .padding([7, 16])
                            .style(if testing { primary } else { secondary })
                            .on_press_maybe(
                                (testing || !running)
                                    .then_some(Msg::Voice(Event::MicTest(!testing))),
                            ),
                    ),
                    container(voice_settings::meter("mic-meter", level, None))
                        .width(Fill)
                        .center_y(32),
                ]
                .spacing(16)
                .align_y(Center),
            ]
            .spacing(8);
            if let Some(why) = self.voice.mic_note {
                mic = mic.push(note(why));
            }
            // Input mode, then what it needs.
            let ptt = self.voice.push_to_talk;
            let mode = |name: &'static str, title: &'static str, value: bool| -> El<'static> {
                tag(
                    name,
                    if ptt == value { "selected" } else { "" },
                    "",
                    radio(title, value, Some(ptt), |on| {
                        Msg::Voice(Event::PushToTalk(on))
                    })
                    .size(18)
                    .text_size(15)
                    .spacing(10),
                )
            };
            let input_mode = column![
                label("INPUT MODE"),
                mode("input-mode-voice", "Voice Activity", false),
                mode("input-mode-ptt", "Push to Talk", true),
            ]
            .spacing(10);
            let release = tag(
                "ptt-release",
                "",
                format!("{} ms", voice.release_delay),
                column![
                    row![
                        label("PUSH TO TALK RELEASE DELAY"),
                        Space::new().width(Fill),
                        text(format!("{} ms", voice.release_delay))
                            .size(12)
                            .color(CHANNEL),
                    ]
                    .align_y(Center),
                    tag(
                        "ptt-release-slider",
                        "",
                        "",
                        slider(RELEASE_RANGE, voice.release_delay, |ms| {
                            Msg::Voice(Event::Set(Change::ReleaseDelay(ms)))
                        })
                        .default(100u16)
                        .step(20u16)
                        .shift_step(100u16)
                        .width(Fill),
                    ),
                ]
                .spacing(6),
            );
            // What the input mode needs, in one slot, so the rows after it
            // keep their place either way.
            let details: El<'_> = if ptt {
                column![
                    self.talk_key_settings(),
                    release,
                    note("Push to Talk applies to voice channels and calls alike."),
                ]
                .spacing(16)
                .into()
            } else {
                voice_settings::sensitivity(voice, level, true)
            };
            let mut voice_rows = column![
                devices,
                volumes,
                mic,
                divider(),
                input_mode,
                details,
                divider(),
                voice_settings::processing(voice, true),
            ]
            .spacing(20);
            if let Some(why) = self.voice.note {
                voice_rows = voice_rows.push(note(why));
            }
            page.push(part(Some(group(
                "VOICE SETTINGS",
                container(voice_rows)
                    .padding(16)
                    .width(Fill)
                    .style(|_| surface(SIDE, 8.)),
            ))))
            .push(part(Some(group(
                "VIDEO SETTINGS",
                container(self.camera_settings())
                    .padding(16)
                    .width(Fill)
                    .style(|_| surface(SIDE, 8.)),
            ))))
        }
    }
}
