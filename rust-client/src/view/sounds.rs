//! User Settings' sound switches (sounds.rs): one per sound, kept on this
//! computer.
use super::{El, MUTED, Msg};
use crate::{
    App,
    sounds::{Event, Switch},
};
use iced::widget::{text, toggler};

type Setting<'s> = &'s dyn Fn(&'static str, String, El<'static>) -> El<'static>;

impl App {
    pub(super) fn sound_settings(&self, setting: Setting) -> Vec<El<'static>> {
        let s = &self.sounds;
        let mut rows: Vec<El<'static>> = Switch::ALL
            .into_iter()
            .map(|switch| {
                let control = toggler(s.settings.on(switch))
                    .size(22)
                    .on_toggle_maybe(
                        (!s.saving).then_some(move |on| Msg::Sound(Event::Set(switch, on))),
                    )
                    .into();
                setting(switch.label(), switch.about().into(), control)
            })
            .collect();
        if let Some(note) = s.note {
            rows.push(text(note).size(14).color(MUTED).into());
        }
        rows
    }
}
