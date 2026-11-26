//! Account screens for the matrix-sdk engine: emoji verification, the
//! one-time recovery key, recovery setup and reset, devices, sign-out and
//! browser sign-in. They use only iced widgets and the palette both the
//! current layout and the Discord-style layout share, with that layout's
//! button and field styles copied here, so they drop into either.
use super::{BLURPLE, BOLD, CHAT, El, MUTED, RAIL, SIDE, TEXT, label, surface};
use crate::{
    App, Msg,
    engine::{
        BrowserMethod, Recovery, Stage,
        account_ui::{Action, Job},
    },
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Font, Theme, color,
    widget::{Column, Space, button, column, container, row, text, text_input},
};

const BODY: Color = color!(0xdbdee1);
const CHANNEL: Color = color!(0x949ba4);
const GREEN: Color = color!(0x23a55a);
const RED: Color = color!(0xf23f43);
const LINK: Color = color!(0x00a8fc);

fn sdk(action: Action) -> Msg {
    Msg::Sdk(action)
}

fn fill(top: Color, hover: Color, disabled: Color, s: button::Status) -> button::Style {
    button::Style {
        background: Some(
            match s {
                button::Status::Hovered | button::Status::Pressed => hover,
                button::Status::Disabled => disabled,
                button::Status::Active => top,
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
fn primary(_: &Theme, s: button::Status) -> button::Style {
    fill(BLURPLE, color!(0x4752c4), color!(0x3c428a), s)
}
fn secondary(_: &Theme, s: button::Status) -> button::Style {
    fill(color!(0x4e5058), color!(0x6d6f78), color!(0x3f4147), s)
}
fn danger(_: &Theme, s: button::Status) -> button::Style {
    fill(color!(0xda373c), color!(0xa12828), color!(0x6b2b2d), s)
}
fn link(_: &Theme, s: button::Status) -> button::Style {
    button::Style {
        text_color: if s == button::Status::Hovered {
            color!(0x5fc5ff)
        } else {
            LINK
        },
        ..Default::default()
    }
}
fn field(_: &Theme, status: text_input::Status) -> text_input::Style {
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

type Style = fn(&Theme, button::Status) -> button::Style;

fn wide<'a>(label: impl text::IntoFragment<'a>, style: Style, msg: Option<Msg>) -> El<'a> {
    button(
        container(text(label).size(16).font(BOLD))
            .center_x(Fill)
            .padding([3, 0]),
    )
    .width(Fill)
    .padding([8, 16])
    .style(style)
    .on_press_maybe(msg)
    .into()
}
fn small<'a>(label: impl text::IntoFragment<'a>, style: Style, msg: Option<Msg>) -> El<'a> {
    button(text(label).size(14))
        .padding([7, 14])
        .style(style)
        .on_press_maybe(msg)
        .into()
}
fn link_button<'a>(label: impl text::IntoFragment<'a>, msg: Option<Msg>) -> El<'a> {
    button(text(label).size(14))
        .padding(0)
        .style(link)
        .on_press_maybe(msg)
        .into()
}
fn caption(s: &str) -> El<'_> {
    text(s).size(12).font(BOLD).color(MUTED).into()
}
fn note<'a>(s: impl text::IntoFragment<'a>) -> El<'a> {
    text(s).size(14).color(BODY).into()
}
fn password<'a>(app: &'a App, submit: Action) -> El<'a> {
    column![
        caption("ACCOUNT PASSWORD"),
        text_input("Account password", &app.sdk_ui.password)
            .on_input(|s| sdk(Action::Password(s)))
            .on_submit(sdk(submit))
            .secure(true)
            .padding(10)
            .size(16)
            .style(field),
    ]
    .spacing(8)
    .into()
}

/// A centered card over a blurple wash, like the Discord-style sign-in page.
fn card<'a>(content: Column<'a, Msg>) -> El<'a> {
    let backdrop = iced::Background::Gradient(
        iced::gradient::Linear::new(iced::Radians(2.4))
            .add_stop(0.0, color!(0x404eed))
            .add_stop(0.55, color!(0x2c2f8c))
            .add_stop(1.0, RAIL)
            .into(),
    );
    container(
        container(content.spacing(18).width(Fill))
            .padding(32)
            .width(480)
            .style(|_| surface(CHAT, 6.)),
    )
    .center(Fill)
    .style(move |_| iced::widget::container::Style {
        background: Some(backdrop),
        ..Default::default()
    })
    .into()
}
fn heading<'a>(title: &'a str, detail: String) -> El<'a> {
    column![
        text(title).size(24).font(BOLD).color(TEXT),
        text(detail).size(16).color(MUTED).center(),
    ]
    .spacing(8)
    .width(Fill)
    .align_x(Center)
    .into()
}

/// A labelled settings group, like the Discord-style settings page.
fn group<'a>(heading: &'static str, rows: Column<'a, Msg>) -> El<'a> {
    column![
        label(heading),
        container(rows.spacing(16))
            .padding(16)
            .width(Fill)
            .style(|_| surface(SIDE, 8.))
    ]
    .spacing(8)
    .into()
}

fn when(stamp: i64) -> String {
    chrono::DateTime::from_timestamp_millis(stamp)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%b %-d, %Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

impl App {
    /// Full-window screens that wait for the user: the one-time recovery key,
    /// then a running or finished verification.
    pub(super) fn sdk_screen(&self) -> Option<El<'_>> {
        self.sdk.as_ref()?;
        if let Some(key) = &self.sdk_ui.recovery_key {
            return Some(card(column![
                heading(
                    "Save your recovery key",
                    "Keep it somewhere safe, like a password manager. You need it to verify a new device and read encrypted history. Untethered won’t show it again.".into()
                ),
                container(text(key).font(Font::MONOSPACE).size(18).color(TEXT))
                    .padding(14)
                    .width(Fill)
                    .style(|_| surface(RAIL, 4.)),
                row![
                    wide(
                        if self.sdk_ui.key_copied { "Copied" } else { "Copy" },
                        secondary,
                        Some(sdk(Action::CopyRecoveryKey))
                    ),
                    wide("I’ve saved it", primary, Some(sdk(Action::SavedRecoveryKey))),
                ]
                .spacing(12),
            ]));
        }
        let v = self.sdk_ui.verification.as_ref()?;
        let device = v
            .device
            .clone()
            .unwrap_or_else(|| "your other device".into());
        let cancel = || wide("Cancel", secondary, Some(sdk(Action::CancelVerification)));
        let content = match &v.stage {
            Stage::Requested => column![
                heading(
                    "Verify a new device?",
                    format!("{device} is asking to be verified. Accept only if you just signed in there.")
                ),
                row![
                    wide("Decline", secondary, Some(sdk(Action::CancelVerification))),
                    wide("Accept", primary, Some(sdk(Action::AcceptVerification))),
                ]
                .spacing(12),
            ],
            Stage::Waiting => column![
                heading(
                    "Verify this device",
                    "Accept the request on a device that’s already verified, in Untethered or Element.".into()
                ),
                cancel(),
            ],
            Stage::Starting => column![
                heading("Verify this device", format!("Connecting to {device}…")),
                cancel(),
            ],
            Stage::Emoji(emoji) => {
                let tiles = emoji.iter().map(|(symbol, name)| {
                    column![
                        text(symbol.as_str()).size(36),
                        text(name.as_str()).size(11).color(MUTED).center(),
                    ]
                    .spacing(4)
                    .width(Fill)
                    .align_x(Center)
                    .into()
                });
                column![
                    heading(
                        "Compare emoji",
                        format!("Check that {device} shows these emoji in the same order.")
                    ),
                    container(row(tiles).spacing(4))
                        .padding([14, 8])
                        .width(Fill)
                        .style(|_| surface(SIDE, 8.)),
                    row![
                        wide("They don’t match", danger, Some(sdk(Action::EmojiMatch(false)))),
                        wide("They match", primary, Some(sdk(Action::EmojiMatch(true)))),
                    ]
                    .spacing(12),
                ]
            }
            Stage::Confirmed => column![
                heading("Compare emoji", format!("Waiting for {device} to confirm…")),
                cancel(),
            ],
            Stage::Done => column![
                heading(
                    "Verified",
                    if v.incoming {
                        format!("{device} is verified and can read your encrypted messages.")
                    } else {
                        "This device is verified. Your encrypted messages are unlocking.".into()
                    }
                ),
                text("✓").size(48).color(GREEN).width(Fill).center(),
                wide("Continue", primary, Some(sdk(Action::CloseVerification))),
            ],
            Stage::Cancelled(reason) => column![
                heading("Verification stopped", reason.clone()),
                wide("Close", secondary, Some(sdk(Action::CloseVerification))),
            ],
        };
        Some(card(content))
    }

    /// Under the recovery form on "Verify this device": emoji verification,
    /// recovery setup for accounts without keys, and starting over.
    pub(super) fn sdk_verify_choices(&self) -> El<'_> {
        let ui = &self.sdk_ui;
        let mut choices = column![
            row![
                container(Space::new())
                    .height(1)
                    .width(Fill)
                    .style(|_| surface(CHANNEL, 0.)),
                text("OR").size(12).font(BOLD).color(CHANNEL),
                container(Space::new())
                    .height(1)
                    .width(Fill)
                    .style(|_| surface(CHANNEL, 0.)),
            ]
            .spacing(10)
            .align_y(Center)
        ]
        .spacing(14);
        if ui.identity {
            choices = choices
                .push(wide(
                    if ui.busy(&Job::VerifyDevice) {
                        "Asking your other devices…"
                    } else {
                        "Verify with another device"
                    },
                    secondary,
                    (self.connected && !ui.busy(&Job::VerifyDevice))
                        .then(|| sdk(Action::VerifyWithDevice)),
                ))
                .push(
                    text("Compare emoji with a device that’s already verified.")
                        .size(13)
                        .color(CHANNEL),
                );
            if ui.resetting {
                choices = choices
                    .push(note(
                        "Resetting creates new encryption keys and a new recovery key. Your other devices and the people you talk to will need to verify you again, and messages you can’t read now stay unreadable.",
                    ))
                    .push(password(self, Action::ResetEncryption))
                    .push(row![
                        wide("Cancel", secondary, Some(sdk(Action::ShowReset(false)))),
                        wide(
                            "Reset encryption",
                            danger,
                            (self.connected && !ui.busy(&Job::ResetEncryption) && !ui.password.is_empty())
                                .then(|| sdk(Action::ResetEncryption)),
                        ),
                    ].spacing(12));
            } else {
                choices = choices.push(link_button(
                    "Lost your recovery key and every other device?",
                    Some(sdk(Action::ShowReset(true))),
                ));
            }
        } else if ui.recovery == Recovery::Unknown {
            choices = choices.push(note("Checking this account’s encryption…"));
        } else {
            choices = choices
                .push(note(
                    "Set up recovery to create this account’s encryption keys and a recovery key to save.",
                ))
                .push(password(self, Action::SetUpRecovery))
                .push(wide(
                    if ui.busy(&Job::SetUpRecovery) {
                        "Setting up recovery…"
                    } else {
                        "Set up recovery"
                    },
                    primary,
                    (self.connected && !ui.busy(&Job::SetUpRecovery) && !ui.password.is_empty())
                        .then(|| sdk(Action::SetUpRecovery)),
                ));
        }
        if !ui.note.is_empty() {
            choices = choices.push(note(&ui.note));
        }
        choices.into()
    }

    /// Recovery, devices and sign-out on a verified device's account page.
    pub(super) fn sdk_account_settings(&self) -> El<'_> {
        let ui = &self.sdk_ui;
        let set_up = matches!(ui.recovery, Recovery::Enabled | Recovery::Incomplete);
        let recovery = column![
            text("Recovery").size(16).font(BOLD).color(TEXT),
            text(match ui.recovery {
                Recovery::Enabled =>
                    "Your recovery key can verify a new device and restore encrypted history.",
                Recovery::Incomplete =>
                    "Recovery is missing some keys. Create a new recovery key to repair it.",
                _ => "Not set up. Without it, only this device can verify a new one.",
            })
            .size(14)
            .color(MUTED),
            small(
                if ui.busy(&Job::ChangeRecoveryKey) {
                    "Creating a key…"
                } else if set_up {
                    "Change recovery key"
                } else {
                    "Set up recovery"
                },
                secondary,
                (self.connected && !ui.busy(&Job::ChangeRecoveryKey))
                    .then(|| sdk(Action::ChangeRecoveryKey)),
            ),
        ]
        .spacing(6);
        let mut devices = Column::new().push(
            row![
                text("Devices").size(16).font(BOLD).color(TEXT).width(Fill),
                link_button(
                    if ui.busy(&Job::Devices) {
                        "Loading…"
                    } else {
                        "Refresh"
                    },
                    (self.connected && !ui.busy(&Job::Devices))
                        .then(|| sdk(Action::RefreshDevices)),
                ),
            ]
            .align_y(Center),
        );
        for d in ui.devices.iter().flatten() {
            let name = d.name.clone().unwrap_or_else(|| d.id.clone());
            let mut tags = row![
                text(format!("{name} · {}", d.id))
                    .size(14)
                    .color(BODY)
                    .width(Fill)
            ]
            .spacing(6)
            .align_y(Center);
            if d.current {
                tags = tags.push(text("This device").size(12).font(BOLD).color(BLURPLE));
            }
            tags = tags.push(
                text(if d.verified {
                    "Verified"
                } else {
                    "Not verified"
                })
                .size(12)
                .font(BOLD)
                .color(if d.verified { GREEN } else { RED }),
            );
            let mut entry = column![tags].spacing(4);
            if let Some(seen) = d.last_seen {
                entry = entry.push(
                    text(format!("Last active {}", when(seen)))
                        .size(12)
                        .color(CHANNEL),
                );
            }
            if !d.current {
                if ui.removing.as_deref() == Some(d.id.as_str()) {
                    entry = entry.push(password(self, Action::RemoveDevice)).push(
                        row![
                            small("Cancel", secondary, Some(sdk(Action::AskRemove(None)))),
                            small(
                                "Sign out device",
                                danger,
                                (self.connected
                                    && !ui.password.is_empty()
                                    && !ui.busy(&Job::DeleteDevice(d.id.clone())))
                                .then(|| sdk(Action::RemoveDevice)),
                            ),
                        ]
                        .spacing(8),
                    );
                } else {
                    entry = entry.push(link_button(
                        "Sign out",
                        self.connected
                            .then(|| sdk(Action::AskRemove(Some(d.id.clone())))),
                    ));
                }
            }
            devices = devices.push(entry);
        }
        let mut leave = column![
            text("Sign out").size(16).font(BOLD).color(TEXT),
            text("Signs this device out and removes its Matrix data from this profile. Keep your recovery key: you’ll need it to read encrypted history after signing in again.")
                .size(14)
                .color(MUTED),
        ]
        .spacing(6);
        leave = if ui.signing_out {
            leave.push(
                row![
                    small("Cancel", secondary, Some(sdk(Action::AskSignOut(false)))),
                    small(
                        if ui.busy(&Job::Logout) {
                            "Signing out…"
                        } else {
                            "Sign out of this device"
                        },
                        danger,
                        (!ui.busy(&Job::Logout)).then(|| sdk(Action::SignOut)),
                    ),
                ]
                .spacing(8),
            )
        } else {
            leave.push(small(
                "Sign out",
                danger,
                Some(sdk(Action::AskSignOut(true))),
            ))
        };
        let mut page = column![
            group("ENCRYPTION", column![recovery]),
            group("DEVICES", devices),
            group("SESSION", leave),
        ]
        .spacing(20)
        .width(Fill);
        if !ui.note.is_empty() {
            page = page.push(note(&ui.note));
        }
        page.into()
    }

    /// Under the sign-in form: single sign-on and OAuth 2.0 when the
    /// homeserver offers them.
    pub(super) fn sdk_login_choices(&self) -> El<'_> {
        let ui = &self.sdk_ui;
        if ui.browser {
            return column![
                note(
                    "Finish signing in in your browser. Untethered is waiting for the homeserver."
                ),
                link_button("Cancel", Some(sdk(Action::CancelBrowserLogin))),
            ]
            .spacing(8)
            .into();
        }
        let Some(options) = self.sdk_current_options() else {
            let checking = ui.jobs.values().any(|j| matches!(j, Job::LoginOptions(_)));
            return link_button(
                if checking {
                    "Checking sign-in options…"
                } else {
                    "Other ways to sign in"
                },
                (self.connected && !checking).then(|| sdk(Action::LoginOptions)),
            );
        };
        let mut choices = column![].spacing(10);
        if options.oauth {
            choices = choices.push(wide(
                "Continue with your homeserver account",
                secondary,
                self.connected
                    .then(|| sdk(Action::BrowserLogin(BrowserMethod::OAuth))),
            ));
        } else if options.sso && options.providers.is_empty() {
            choices = choices.push(wide(
                "Continue with single sign-on",
                secondary,
                self.connected
                    .then(|| sdk(Action::BrowserLogin(BrowserMethod::Sso { provider: None }))),
            ));
        } else if options.sso {
            for (id, name) in &options.providers {
                choices = choices.push(wide(
                    format!("Continue with {name}"),
                    secondary,
                    self.connected.then(|| {
                        sdk(Action::BrowserLogin(BrowserMethod::Sso {
                            provider: Some(id.clone()),
                        }))
                    }),
                ));
            }
        } else {
            choices = choices.push(note("This homeserver only offers password sign-in."));
        }
        choices.into()
    }
}
