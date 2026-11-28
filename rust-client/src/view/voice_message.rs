//! Voice messages on screen (ROADMAP M10): the composer's microphone button,
//! the recording bar that takes the composer's place while recording, and
//! waveforms, also in received voice messages' players (players.rs).
//!
//! The microphone takes the Send button's slot while the composer is empty,
//! as in Element, so the editor keeps its place in the row. Recording is
//! Linux's; elsewhere the slot stays Send.
use super::El;
#[cfg(target_os = "linux")]
use super::{App, BOLD, CHANNEL, Icon, Msg, RED, TEXT, icon_button};
#[cfg(target_os = "linux")]
use iced::{
    Alignment::Center,
    Color, Fill,
    widget::{Row, Space, container},
};

/// A waveform's played part and the rest.
#[cfg(target_os = "linux")]
const PLAYED: Color = iced::color!(0xdbdee1);
#[cfg(target_os = "linux")]
const UNPLAYED: Color = iced::color!(0x6d6f78);

/// Bars `height` tall at most, one per value in `heights` (0 to 1), the
/// first `played` share of them in the brighter color.
#[cfg(target_os = "linux")]
pub(super) fn waveform<'a>(heights: &[f32], played: f32, height: f32) -> El<'a> {
    let n = heights.len().max(1) as f32;
    Row::with_children(heights.iter().enumerate().map(|(i, &h)| {
        let color = if (i as f32 + 0.5) / n <= played {
            PLAYED
        } else {
            UNPLAYED
        };
        container(Space::new())
            .width(3)
            .height((h.clamp(0., 1.) * height).max(2.))
            .style(move |_| container::Style {
                background: Some(color.into()),
                border: iced::Border::default().rounded(1.5),
                ..Default::default()
            })
            .into()
    }))
    .spacing(2)
    .align_y(Center)
    .height(height)
    .into()
}

#[cfg(target_os = "linux")]
impl App {
    /// The composer's last button: Send, or the microphone while the
    /// composer is empty and the channel takes messages from you.
    pub(super) fn send_or_record<'a>(&'a self, send: El<'a>, empty: bool) -> El<'a> {
        use crate::voice_message::{Input, Mic};
        use iced::widget::tooltip::Position::Top;
        let (tip, action) = match self.mic() {
            Mic::On if empty => (
                "Record a voice message",
                Some(Msg::VoiceMessage(Input::Record)),
            ),
            Mic::Off(why) if empty => (why, None),
            _ => return send,
        };
        let state = if action.is_some() { "on" } else { "off" };
        crate::qa_layout::tag(
            "voice-record",
            state,
            tip,
            icon_button(Icon::Mic, tip, false, action, Top),
        )
    }

    /// While recording in the open channel, the bar in the composer row's
    /// place: Cancel, a red dot, the time, the live levels and Send.
    pub(super) fn voice_bar(&self) -> Option<El<'_>> {
        use crate::voice_message::{Input, LIVE_BARS, Stage, clock, heights, live_level};
        use iced::widget::{row, text, tooltip::Position::Top};
        let rec = self.recording_here()?;
        let live = |levels: &[f32]| {
            // Newest on the right; the bar fills from there.
            let mut bars = vec![0.; LIVE_BARS.saturating_sub(levels.len())];
            bars.extend(levels.iter().map(|&p| live_level(p)));
            bars
        };
        let (ms, bars, note, recording) = match &rec.stage {
            Stage::Starting => (0, live(&[]), Some("Starting the microphone…"), false),
            Stage::Live { length, levels, .. } => {
                (length.as_millis() as u64, live(levels), None, true)
            }
            Stage::Finishing { length, levels, .. } => (
                length.as_millis() as u64,
                live(levels),
                Some("Finishing…"),
                false,
            ),
            Stage::Ready { clip, note } => (
                clip.duration_ms,
                heights(&clip.waveform, LIVE_BARS),
                note.as_deref(),
                false,
            ),
            Stage::Sending { clip, .. } => (
                clip.duration_ms,
                heights(&clip.waveform, LIVE_BARS),
                Some("Sending…"),
                false,
            ),
        };
        let sending = matches!(rec.stage, Stage::Sending { .. });
        let ready = matches!(rec.stage, Stage::Live { .. } | Stage::Ready { .. });
        let cancel = crate::qa_layout::tag(
            "voice-cancel",
            if sending { "off" } else { "on" },
            "Cancel",
            icon_button(
                Icon::Close,
                "Cancel (Esc)",
                false,
                (!sending).then_some(Msg::VoiceMessage(Input::Cancel)),
                Top,
            ),
        );
        let dot = container(Space::new())
            .width(10)
            .height(10)
            .style(move |_| container::Style {
                background: Some(if recording { RED } else { CHANNEL }.into()),
                border: iced::Border::default().rounded(5),
                ..Default::default()
            });
        let timer = crate::qa_layout::tag(
            "voice-timer",
            if recording { "on" } else { "off" },
            "",
            text(clock(ms)).size(15).font(BOLD).color(TEXT),
        );
        let wave = crate::qa_layout::tag(
            "voice-wave",
            "",
            "",
            container(waveform(&bars, 1., 24.)).width(Fill).clip(true),
        );
        let note: El = match note {
            Some(note) => text(note.to_owned())
                .size(13)
                .color(if matches!(rec.stage, Stage::Ready { .. }) {
                    RED
                } else {
                    CHANNEL
                })
                .into(),
            None => Space::new().into(),
        };
        let send = crate::qa_layout::tag(
            "voice-send",
            if ready { "on" } else { "off" },
            "Send",
            icon_button(
                Icon::Send,
                if sending { "Sending…" } else { "Send (Enter)" },
                false,
                ready.then_some(Msg::VoiceMessage(Input::Send)),
                Top,
            ),
        );
        Some(
            row![cancel, dot, timer, wave, note, send]
                .spacing(12)
                .align_y(Center)
                .height(44)
                .padding(iced::Padding {
                    left: 16.,
                    right: 8.,
                    ..iced::Padding::ZERO
                })
                .into(),
        )
    }
}

#[cfg(not(target_os = "linux"))]
impl super::App {
    /// No recording on this platform: the slot stays Send.
    pub(super) fn send_or_record<'a>(&'a self, send: El<'a>, _empty: bool) -> El<'a> {
        send
    }

    pub(super) fn voice_bar(&self) -> Option<El<'_>> {
        None
    }
}
