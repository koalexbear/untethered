//! Playing in the timeline (inline_player.rs) on screen: a video in its own
//! place with a control bar, and a card with play, seek and time for audio
//! files and voice messages (ROADMAP P2). Linux only, like the viewer's video.
use super::{App, BOLD, CHANNEL, El, Icon, Msg, SIDE, TEXT, icon, surface};
use crate::{media::Media, player::Control};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Theme,
    widget::{button, column, container, image, mouse_area, row, slider, text},
};

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

impl App {
    /// Play/pause, the time, a seek bar and mute, for what's playing here.
    fn play_bar<'a>(&'a self, event: &str, media: &'a Media) -> El<'a> {
        use crate::player::clock;
        let player = self
            .inline_playing(Some(event))
            .and_then(|p| p.player.as_ref());
        let (playing, muted, position, total, at) = match player {
            Some(p) => {
                let total = p.duration.unwrap_or_default();
                let progress = if total.is_zero() {
                    0.
                } else {
                    (p.position.as_secs_f32() / total.as_secs_f32()).clamp(0., 1.)
                };
                let at = p.scrub.unwrap_or(progress);
                let shown = if p.scrub.is_some() {
                    total.mul_f32(at)
                } else {
                    p.position
                };
                (p.playing, p.muted, shown, total, at)
            }
            None => (false, false, Default::default(), Default::default(), 0.),
        };
        let toggle = Msg::PlayInline(event.to_owned(), media.clone());
        let mut bar = row![
            control(if playing { Icon::Pause } else { Icon::Play }, 30., toggle),
            text(format!("{} / {}", clock(position), clock(total)))
                .size(12)
                .color(TEXT),
        ]
        .spacing(8)
        .align_y(Center);
        bar = bar.push(if player.is_some() {
            El::from(
                slider(0.0_f32..=1.0, at, Msg::InlineScrub)
                    .step(0.001_f32)
                    .on_release(Msg::InlineControl(Control::Seek(at)))
                    .width(Fill),
            )
        } else {
            El::from(iced::widget::Space::new().width(Fill))
        });
        if player.is_some() {
            bar = bar.push(control(
                if muted { Icon::Muted } else { Icon::Speaker },
                30.,
                Msg::InlineControl(Control::Mute),
            ));
        }
        bar.into()
    }

    /// A video playing in the timeline, at the poster's size; None while
    /// it isn't the one playing.
    pub(super) fn inline_video<'a>(
        &'a self,
        event: &str,
        media: &'a Media,
        size: (f32, f32),
    ) -> Option<El<'a>> {
        let playing = self.inline_playing(Some(event))?;
        let picture: El = match playing.player.as_ref().and_then(|p| p.frame.as_ref()) {
            Some(frame) => mouse_area(
                image(frame.clone())
                    .width(size.0)
                    .height(size.1)
                    .content_fit(iced::ContentFit::Contain),
            )
            .on_press(Msg::InlineControl(Control::Toggle))
            .into(),
            None => container(
                text(if playing.failed {
                    "This video can't be played here."
                } else {
                    "Loading video…"
                })
                .size(13)
                .color(TEXT),
            )
            .center_x(size.0)
            .center_y(size.1)
            .style(|_| surface(Color::BLACK, 8.))
            .into(),
        };
        let open = button(text("Open in viewer").size(12).color(CHANNEL))
            .padding([2, 6])
            .style(button::text)
            .on_press(Msg::OpenViewer(event.to_owned()));
        Some(
            container(
                column![
                    picture,
                    self.play_bar(event, media),
                    container(open).align_right(Fill)
                ]
                .spacing(4)
                .width(size.0.max(280.)),
            )
            .padding(6)
            .style(|_| surface(SIDE, 8.))
            .into(),
        )
    }

    /// An audio file or voice message, with its player.
    pub(super) fn audio_card<'a>(&'a self, event: &str, media: &'a Media, tools: El<'a>) -> El<'a> {
        let size = media.size.map(crate::media::size).unwrap_or_default();
        let failed = self.inline_playing(Some(event)).is_some_and(|p| p.failed);
        let mut card = column![
            row![
                icon(Icon::Speaker, 22., Color::from_rgb8(0x94, 0x9c, 0xf7)),
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
