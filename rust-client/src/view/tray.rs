//! User Settings' system tray switches (tray.rs): close to the tray, and
//! start at sign-in in the tray.
use super::{El, MUTED, Msg};
use crate::{
    App,
    tray::{Event, Host},
};
use iced::widget::{text, toggler};

type Setting<'s> = &'s dyn Fn(&'static str, String, El<'static>) -> El<'static>;

impl App {
    pub(super) fn tray_settings(&self, setting: Setting) -> Vec<El<'static>> {
        let t = &self.tray;
        let switch = |on: bool, change: fn(bool) -> Event| -> El<'static> {
            toggler(on)
                .size(22)
                .on_toggle_maybe((!t.saving).then_some(move |v| Msg::Tray(change(v))))
                .into()
        };
        let close = match t.host {
            Host::Missing => "This desktop has no system tray, so closing the window quits.",
            _ if t.settings.close_to_tray => {
                "Closing the window keeps Untethered running in the system tray, so messages, notifications and calls carry on. Quit from the tray’s menu."
            }
            _ => "Closing the window quits Untethered.",
        };
        let mut rows = vec![
            setting(
                "Close to tray",
                close.into(),
                switch(t.settings.close_to_tray, Event::CloseToTray),
            ),
            setting(
                "Start at sign-in",
                "Open Untethered in the system tray when you sign in to this computer.".into(),
                switch(t.start_at_login, Event::StartAtLogin),
            ),
        ];
        if let Some(note) = t.note {
            rows.push(text(note).size(14).color(MUTED).into());
        }
        rows
    }
}
