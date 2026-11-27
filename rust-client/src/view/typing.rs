//! The typing line under the composer and the Privacy switch (typing.rs).
use super::{BODY, BOLD, CHANNEL, El, Msg};
use crate::{
    App,
    typing::{Input, Part},
};
use iced::widget::{rich_text, span, text::Span, toggler};

type Setting<'s> = &'s dyn Fn(&'static str, String, El<'static>) -> El<'static>;

impl App {
    /// "Mira is typing…" in the composer's footer, where it replaces the
    /// room's note while anyone is.
    pub(super) fn typing_footer(&self) -> Option<El<'_>> {
        let parts = self.typing_line();
        if parts.is_empty() {
            return None;
        }
        let spans: Vec<Span<'_, Msg>> = parts
            .into_iter()
            .map(|part| match part {
                Part::Name(name) => span(name).font(BOLD).color(BODY),
                Part::Text(text) => span(text).color(CHANNEL),
            })
            .collect();
        Some(rich_text(spans).size(12).into())
    }

    pub(super) fn typing_setting(&self, setting: Setting) -> El<'static> {
        let send = self.typing.sharing.send;
        let detail = if send {
            "Room members see when you’re writing a message."
        } else {
            "Nobody sees when you’re writing a message."
        };
        let change = (self.connected && !self.changing_typing_sharing())
            .then_some(|on| Msg::Typing(Input::Share(on)));
        setting(
            "Send typing indicators",
            detail.into(),
            toggler(send).size(22).on_toggle_maybe(change).into(),
        )
    }
}
