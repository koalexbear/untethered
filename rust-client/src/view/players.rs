//! Playing in the timeline (inline_player.rs) on screen: a video in its
//! poster's place, at the same size, with its controls over it as on
//! Discord, and a card with play, seek and time for audio files and voice
//! messages (ROADMAP P2). Linux only, like the viewer's video.
use super::{App, BOLD, CHANNEL, El, Icon, Msg, SIDE, TEXT, icon, surface};
use crate::{media::Media, player::Control};
use iced::{
    Alignment::Center,
    Border, Color, ContentFit, Fill, Theme,
    widget::{
        Space, button, column, container, image, image::Handle, mouse_area, row, slider, stack,
        text,
    },
};
use std::time::Duration;

/// A voice message's waveform in its card, at most this many bars.
const WAVE_BARS: usize = 44;

/// A small round control.
fn control<'a>(glyph: Icon, size: f32, msg: Msg) -> El<'a> {
    button(container(icon(glyph, size * 0.5, Color::WHITE)).center(size))
        .padding(0)
        .on_press(msg)
        .style(move |_: &Theme, s| button::Style {
            background: Some(
                if s == button::Status::Hovered {
                    Color::from_rgba(1., 1., 1., 0.25)
                } else {
                    Color::from_rgba(0., 0., 0., 0.45)
                }
                .into(),
            ),
            border: Border::default().rounded(size / 2.),
            ..Default::default()
        })
        .into()
}

/// A bare glyph for the bar over a video, lit while hovered.
fn glyph<'a>(glyph: Icon, msg: Msg) -> El<'a> {
    button(container(icon(glyph, 18., Color::WHITE)).center(28))
        .padding(0)
        .on_press(msg)
        .style(|_: &Theme, s| button::Style {
            background: (s == button::Status::Hovered)
                .then(|| Color::from_rgba(1., 1., 1., 0.18).into()),
            border: Border::default().rounded(4),
            ..Default::default()
        })
        .into()
}

/// A note over the middle of a video: loading, or why it can't play.
fn pill<'a>(note: &'a str) -> El<'a> {
    container(text(note).size(13).color(Color::WHITE))
        .padding([6, 12])
        .style(|_| surface(Color::from_rgba(0., 0., 0., 0.6), 14.))
        .into()
}

/// What the controls show for the file playing at an event.
struct Progress {
    present: bool,
    playing: bool,
    muted: bool,
    /// The time shown: where it is, or where a drag on the seek bar is.
    shown: Duration,
    total: Duration,
    /// The seek bar, 0 to 1.
    at: f32,
}

impl App {
    fn progress(&self, event: &str) -> Progress {
        let Some(p) = self
            .inline_playing(Some(event))
            .and_then(|p| p.player.as_ref())
        else {
            return Progress {
                present: false,
                playing: false,
                muted: false,
                shown: Duration::ZERO,
                total: Duration::ZERO,
                at: 0.,
            };
        };
        let total = p.duration.unwrap_or_default();
        let progress = if total.is_zero() {
            0.
        } else {
            (p.position.as_secs_f32() / total.as_secs_f32()).clamp(0., 1.)
        };
        let at = p.scrub.unwrap_or(progress);
        Progress {
            present: true,
            playing: p.playing,
            muted: p.muted,
            shown: if p.scrub.is_some() {
                total.mul_f32(at)
            } else {
                p.position
            },
            total,
            at,
        }
    }

    fn seek_bar<'a>(at: f32) -> El<'a> {
        slider(0.0_f32..=1.0, at, Msg::InlineScrub)
            .step(0.001_f32)
            .on_release(Msg::InlineControl(Control::Seek(at)))
            .width(Fill)
            .into()
    }

    /// A voice message's waveform as its seek bar (ROADMAP M10): the
    /// played part brighter, with the slider's handle over it once loaded.
    fn wave_bar<'a>(bars: &[u16], present: bool, at: f32) -> El<'a> {
        let wave = container(super::voice_message::waveform(
            &crate::voice_message::heights(bars, WAVE_BARS),
            if present { at } else { 0. },
            28.,
        ))
        .center_y(28)
        .width(Fill)
        .clip(true);
        if !present {
            return wave.into();
        }
        let seek = slider(0.0_f32..=1.0, at, Msg::InlineScrub)
            .step(0.001_f32)
            .on_release(Msg::InlineControl(Control::Seek(at)))
            .width(Fill)
            .height(28)
            .style(|theme, status| {
                let mut style = slider::default(theme, status);
                style.rail.backgrounds = (Color::TRANSPARENT.into(), Color::TRANSPARENT.into());
                style.with_circular_handle(5.)
            });
        stack![wave, seek].width(Fill).into()
    }

    /// Play/pause, the time, a seek bar and mute, for an audio card.
    fn play_bar<'a>(&'a self, event: &str, media: &'a Media) -> El<'a> {
        use crate::player::clock;
        let mut p = self.progress(event);
        let voice = media.voice.as_ref();
        // Its length before it plays, from the event.
        if p.total.is_zero()
            && let Some(ms) = voice.and_then(|v| v.duration_ms)
        {
            p.total = Duration::from_millis(ms);
        }
        let toggle = Msg::PlayInline(event.to_owned(), media.clone());
        let mut bar = row![
            control(
                if p.playing { Icon::Pause } else { Icon::Play },
                30.,
                toggle
            ),
            text(format!("{} / {}", clock(p.shown), clock(p.total)))
                .size(12)
                .color(TEXT),
        ]
        .spacing(8)
        .align_y(Center);
        bar = bar.push(match voice.filter(|v| !v.waveform.is_empty()) {
            Some(v) => Self::wave_bar(&v.waveform, p.present, p.at),
            None if p.present => Self::seek_bar(p.at),
            None => Space::new().width(Fill).into(),
        });
        if p.present {
            bar = bar.push(control(
                if p.muted { Icon::Muted } else { Icon::Speaker },
                30.,
                Msg::InlineControl(Control::Mute),
            ));
        }
        bar.into()
    }

    /// The bar along a playing video's bottom edge: play/pause, the time
    /// (when there's room), a seek bar, mute and Open in viewer.
    fn video_bar<'a>(&'a self, event: &str, width: f32) -> El<'a> {
        use crate::player::clock;
        let p = self.progress(event);
        let mut bar = row![glyph(
            if p.playing { Icon::Pause } else { Icon::Play },
            Msg::InlineControl(Control::Toggle)
        )]
        .spacing(6)
        .align_y(Center);
        if width >= 260. {
            bar = bar.push(
                text(format!("{} / {}", clock(p.shown), clock(p.total)))
                    .size(12)
                    .color(Color::WHITE)
                    .wrapping(text::Wrapping::None),
            );
        }
        bar = bar
            .push(Self::seek_bar(p.at))
            .push(glyph(
                if p.muted { Icon::Muted } else { Icon::Speaker },
                Msg::InlineControl(Control::Mute),
            ))
            .push(glyph(Icon::Expand, Msg::OpenViewer(event.to_owned())));
        container(bar)
            .padding([4, 8])
            .width(Fill)
            .style(|_| container::Style {
                background: Some(Color::from_rgba(0., 0., 0., 0.6).into()),
                border: Border::default().rounded(iced::border::bottom(8)),
                ..Default::default()
            })
            .into()
    }

    /// A video playing in the timeline, in its poster's place at `size`,
    /// so starting it moves nothing. Its controls lie over it: shown while
    /// it's paused, and for a moment after the pointer moves over it while
    /// it plays. A click on the picture pauses and plays. None while it
    /// isn't the one playing.
    pub(super) fn inline_video<'a>(
        &'a self,
        event: &str,
        size: (f32, f32),
        poster: Option<&'a Handle>,
    ) -> Option<El<'a>> {
        let playing = self.inline_playing(Some(event))?;
        let player = playing.player.as_ref();
        let (w, h) = size;
        let picture: El = match player.and_then(|p| p.frame.as_ref()).or(poster) {
            Some(frame) => image(frame.clone())
                .width(w)
                .height(h)
                .content_fit(ContentFit::Contain)
                .border_radius(8)
                .into(),
            None => Space::new().width(w).height(h).into(),
        };
        let mut picture = mouse_area(container(picture).style(|_| surface(Color::BLACK, 8.)));
        if player.is_some() {
            picture = picture.on_press(Msg::InlineControl(Control::Toggle));
        }
        // The overlay's two slots stay put whatever they show, so the seek
        // bar keeps its state while it's on screen.
        let middle: El = if playing.failed {
            pill("This video can't be played here.")
        } else if player.is_some_and(|p| !p.playing) {
            container(icon(Icon::Play, 24., Color::WHITE))
                .center(56)
                .style(|_| surface(Color::from_rgba(0., 0., 0., 0.6), 28.))
                .into()
        } else if player.is_none() {
            pill("Loading video…")
        } else {
            Space::new().into()
        };
        let bar: El = if self.inline_controls(event) {
            self.video_bar(event, w)
        } else {
            Space::new().into()
        };
        let overlay = column![container(middle).center(Fill), bar]
            .width(w)
            .height(h);
        Some(
            mouse_area(stack![picture, overlay])
                .on_move(|_| Msg::InlinePointer(true))
                .on_exit(Msg::InlinePointer(false))
                .into(),
        )
    }

    /// An audio file or voice message, with its player.
    pub(super) fn audio_card<'a>(&'a self, event: &str, media: &'a Media, tools: El<'a>) -> El<'a> {
        let size = media.size.map(crate::media::size).unwrap_or_default();
        let failed = self.inline_playing(Some(event)).is_some_and(|p| p.failed);
        let mut card = column![
            row![
                icon(
                    if media.voice.as_ref().is_some_and(|v| v.voice) {
                        Icon::Mic
                    } else {
                        Icon::Speaker
                    },
                    22.,
                    Color::from_rgb8(0x94, 0x9c, 0xf7)
                ),
                column![
                    text(&media.name).size(15).font(BOLD).color(TEXT),
                    text(if failed {
                        "This can't be played here.".to_owned()
                    } else {
                        size
                    })
                    .size(12)
                    .color(CHANNEL)
                ]
                .spacing(2)
                .width(Fill),
                tools
            ]
            .spacing(10)
            .align_y(Center),
        ]
        .spacing(8);
        card = card.push(self.play_bar(event, media));
        container(card)
            .padding([10, 12])
            .width(420)
            .style(|_| surface(SIDE, 8.))
            .into()
    }
}
