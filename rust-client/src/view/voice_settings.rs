//! Input sensitivity and voice processing (call_settings.rs, calls plan step
//! A2), as on Discord's Voice & Video page: an automatic sensitivity switch,
//! a live input meter with the manual threshold's slider over it, and the
//! echo cancellation, noise suppression and gain control switches. It shows
//! in the call's Devices panel until the Voice & Video page (U2) takes it.
use super::{BOLD, CHANNEL, El, GREEN, MUTED, Msg, label};
use crate::{
    call_settings::{Change, Event, THRESHOLD_RANGE, Voice},
    desktop_call::InputLevel,
};
use iced::{
    Border, Color, Fill, Point, Rectangle, Renderer, Size, Theme, color, mouse,
    widget::{
        canvas::{self, Canvas, Frame, Geometry, Path},
        checkbox, column, container, row, slider, stack, text,
    },
};

/// Discord's colours: below the threshold, above it, and the empty track.
const BELOW: Color = color!(0xf0b232);
const ABOVE: Color = GREEN;
const TRACK: Color = color!(0x4e5058);
/// The slider's handle; the meter is inset by it at both ends, so the
/// handle sits on the level it sets.
const HANDLE: f32 = 7.0;

/// The input meter: the processed microphone level on a −100…0 dB scale.
struct Meter {
    level: InputLevel,
    /// The manual threshold, splitting the meter's colours.
    threshold: Option<f32>,
}
fn x(db: f32, width: f32) -> f32 {
    let (low, high) = (*THRESHOLD_RANGE.start(), *THRESHOLD_RANGE.end());
    HANDLE + (width - 2.0 * HANDLE) * ((db.clamp(low, high) - low) / (high - low))
}
impl canvas::Program<Msg> for Meter {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let bar = |f: &mut Frame, from: f32, to: f32, color: Color| {
            if to > from {
                f.fill(
                    &Path::rounded_rectangle(
                        Point::new(from, 0.),
                        Size::new(to - from, h),
                        (h / 2.).into(),
                    ),
                    color,
                );
            }
        };
        let (start, end, level) = (x(-100.0, w), x(0.0, w), x(self.level.db, w));
        match self.threshold {
            Some(t) => {
                let split = x(t, w);
                bar(&mut f, start, split, Color { a: 0.3, ..BELOW });
                bar(&mut f, split, end, Color { a: 0.3, ..ABOVE });
                bar(&mut f, start, level.min(split), BELOW);
                bar(&mut f, split, level, ABOVE);
            }
            None => {
                bar(&mut f, start, end, TRACK);
                bar(
                    &mut f,
                    start,
                    level,
                    if self.level.voice { ABOVE } else { BELOW },
                );
            }
        }
        vec![f.into_geometry()]
    }
}

/// The section, for a call whose microphone reports `level`. `enabled` is
/// false while the call is ending.
pub(super) fn section(
    voice: Voice,
    level: InputLevel,
    note: Option<&'static str>,
    enabled: bool,
) -> El<'static> {
    let switch = |on: bool, name: &'static str, change: fn(bool) -> Change| -> El<'static> {
        checkbox(on)
            .label(name)
            .size(16)
            .text_size(13)
            .on_toggle_maybe(enabled.then_some(move |on| Msg::Voice(Event::Set(change(on)))))
            .into()
    };
    let meter = |threshold| {
        Canvas::new(Meter { level, threshold })
            .width(Fill)
            .height(8)
    };
    let mut sensitivity = column![
        label("INPUT SENSITIVITY"),
        switch(
            voice.auto_sensitivity,
            "Automatically determine input sensitivity",
            Change::AutoSensitivity
        ),
    ]
    .spacing(5);
    if voice.auto_sensitivity {
        sensitivity = sensitivity.push(meter(None));
    } else {
        // Arrow keys move it a decibel while the pointer is over it.
        let threshold = slider(THRESHOLD_RANGE, voice.threshold, move |db| {
            if enabled {
                Msg::Voice(Event::Set(Change::Threshold(db)))
            } else {
                Msg::VoiceMeter
            }
        })
        .step(1.0_f32)
        .shift_step(5.0_f32)
        .height(18)
        .style(|_, status| slider::Style {
            rail: slider::Rail {
                backgrounds: (Color::TRANSPARENT.into(), Color::TRANSPARENT.into()),
                width: 0.,
                border: Border::default(),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: HANDLE },
                background: match status {
                    slider::Status::Hovered | slider::Status::Dragged => Color::WHITE,
                    slider::Status::Active => color!(0xe3e5e8),
                }
                .into(),
                border_width: 1.,
                border_color: color!(0x1e1f22),
            },
        });
        sensitivity = sensitivity.push(stack![
            container(meter(Some(voice.threshold))).center_y(18),
            threshold,
        ]);
        sensitivity = sensitivity.push(
            row![
                text("Threshold").size(12).color(MUTED),
                text(format!("{:.0} dB", voice.threshold))
                    .size(12)
                    .font(BOLD)
                    .color(CHANNEL),
            ]
            .spacing(6),
        );
    }
    let mut section = column![
        sensitivity,
        label("VOICE PROCESSING"),
        switch(
            voice.echo_cancellation,
            "Echo cancellation",
            Change::EchoCancellation
        ),
        switch(
            voice.noise_suppression,
            "Noise suppression",
            Change::NoiseSuppression
        ),
        switch(
            voice.gain_control,
            "Automatic gain control",
            Change::GainControl
        ),
    ]
    .spacing(5)
    .width(Fill);
    if let Some(note) = note {
        section = section.push(text(note).size(12).color(MUTED));
    }
    section.into()
}
