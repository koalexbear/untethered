//! The composer's voice recorder on Linux: the microphone button starts it,
//! the bar that takes the composer's place shows a timer and live levels,
//! Cancel (Escape) throws the recording away and Send (Enter) finishes and
//! sends it.
//!
//! During a call the button is off: the call has the microphone, and what
//! you said for the message would go to the call too. A call joined while
//! recording doesn't stop the recording; PulseAudio shares the device.
use super::{Clip, capture::Capture};
use crate::engine::{Command, Failure};
use crate::{App, Msg, Request};
use iced::{Subscription, Task};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Element's longest voice message; recording stops there, ready to send.
const LONGEST: Duration = Duration::from_secs(15 * 60);
/// Shorter than this, Send is taken as a slip and nothing is sent.
const SHORTEST_MS: u64 = 500;
/// The live levels the recording bar shows, newest last.
pub const LIVE_BARS: usize = 48;
/// The composer's editor, focused again once the recording bar goes.
const COMPOSER: &str = "composer";

#[derive(Clone)]
pub enum Input {
    /// The composer's microphone button.
    Record,
    Started(u64, Handoff<Result<Capture, String>>),
    /// The timer and levels move; a change of channel stops the recording.
    Tick,
    Cancel,
    Send,
    Finished(u64, Result<Clip, String>),
}

/// A value carried to `update` once: messages are cloned, a recording
/// isn't. Dropped untaken, it drops the value (and stops a recording).
pub struct Handoff<T>(Arc<Mutex<Option<T>>>);
impl<T> Clone for Handoff<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Handoff<T> {
    fn new(value: T) -> Self {
        Self(Arc::new(Mutex::new(Some(value))))
    }
    fn take(&self) -> Option<T> {
        self.0.lock().unwrap().take()
    }
}

#[derive(Default)]
pub struct Recorder {
    takes: u64,
    pub current: Option<Recording>,
    refocus: bool,
}

pub struct Recording {
    /// The channel it is for; recording stops if you leave it.
    pub room: String,
    /// Which start this is, so a late answer for one cancelled can't land.
    take: u64,
    pub stage: Stage,
}

pub enum Stage {
    /// GStreamer is starting (seconds, the first time).
    Starting,
    Live {
        capture: Capture,
        length: Duration,
        /// The last [`LIVE_BARS`] levels, 0 to 1, oldest first.
        levels: Vec<f32>,
    },
    /// The encoder is writing its last page.
    Finishing {
        send: bool,
        length: Duration,
        levels: Vec<f32>,
    },
    /// Stopped at [`LONGEST`], or sending failed (`note` says so): Send
    /// sends it.
    Ready { clip: Clip, note: Option<String> },
    Sending {
        clip: Clip,
        reply: Option<crate::rooms::Quote>,
    },
}

/// Whether the composer offers the microphone, and why not when it's off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mic {
    Hidden,
    Off(&'static str),
    On,
}

impl App {
    pub(crate) fn voice_message(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Record => return self.start_recording(),
            Input::Started(take, handoff) => {
                let Some(result) = handoff.take() else {
                    return Task::none();
                };
                let Some(rec) = self.recording_at(take) else {
                    return Task::none(); // Cancelled meanwhile: dropping it stops it.
                };
                if !matches!(rec.stage, Stage::Starting) {
                    return Task::none();
                }
                match result {
                    Ok(capture) => {
                        rec.stage = Stage::Live {
                            capture,
                            length: Duration::ZERO,
                            levels: Vec::new(),
                        }
                    }
                    Err(why) => {
                        self.voice_recorder.current = None;
                        self.status = why;
                        return refocus();
                    }
                }
            }
            Input::Tick => return self.recording_tick(),
            Input::Cancel => {
                if self
                    .voice_recorder
                    .current
                    .as_ref()
                    .is_some_and(|r| !matches!(r.stage, Stage::Sending { .. }))
                {
                    // Dropping a live capture closes the microphone.
                    self.voice_recorder.current = None;
                    return refocus();
                }
            }
            Input::Send => return self.finish_recording(true),
            Input::Finished(take, result) => {
                let Some(rec) = self.recording_at(take) else {
                    return Task::none();
                };
                let Stage::Finishing { send, .. } = rec.stage else {
                    return Task::none();
                };
                match result {
                    Ok(clip) if clip.duration_ms < SHORTEST_MS => {
                        self.voice_recorder.current = None;
                        self.status = "Too short to send. Hold on a moment longer.".into();
                        return refocus();
                    }
                    Ok(clip) if send => {
                        let room = rec.room.clone();
                        self.send_clip(room, clip);
                    }
                    Ok(clip) => rec.stage = Stage::Ready { clip, note: None },
                    Err(why) => {
                        self.voice_recorder.current = None;
                        self.status = why;
                        return refocus();
                    }
                }
            }
        }
        Task::none()
    }

    /// The microphone button's state in the open channel: hidden where you
    /// can't post or this build can't record.
    pub(crate) fn mic(&self) -> Mic {
        let postable = self.logged_in
            && self.verified
            && self.sdk.is_some()
            && self.live.list.get(&self.selected).is_some_and(|r| {
                !r.space
                    && r.successor.is_none()
                    && r.posting.can_post(&self.matrix_id, r.encrypted) != Some(false)
            });
        if !postable {
            Mic::Hidden
        } else if self.call.as_ref().is_some_and(|c| !c.cancel.is_closed()) {
            Mic::Off("Leave the call to record a voice message")
        } else if !self.connected {
            Mic::Off("Offline")
        } else {
            Mic::On
        }
    }

    /// The recording to show in the open channel's composer.
    pub(crate) fn recording_here(&self) -> Option<&Recording> {
        self.voice_recorder
            .current
            .as_ref()
            .filter(|r| r.room == self.selected)
    }

    /// The timer and levels while recording, Enter while the bar shows.
    pub(crate) fn recorder_subscription(&self) -> Subscription<Msg> {
        let Some(rec) = &self.voice_recorder.current else {
            return Subscription::none();
        };
        let ticking = matches!(rec.stage, Stage::Starting | Stage::Live { .. });
        Subscription::batch([
            if ticking {
                iced::time::every(Duration::from_millis(50)).map(|_| Msg::VoiceMessage(Input::Tick))
            } else {
                Subscription::none()
            },
            // The editor is gone while the bar shows, so no field takes Enter.
            iced::event::listen_with(|event, status, _| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
                    modifiers,
                    ..
                }) if status == iced::event::Status::Ignored && !modifiers.shift() => {
                    Some(Msg::VoiceMessage(Input::Send))
                }
                _ => None,
            }),
        ])
    }

    /// The engine's answer to a [`Command::SendVoice`].
    pub(crate) fn voice_sent(&mut self, room: String, result: Result<(), Failure>) {
        let Some(rec) = self
            .voice_recorder
            .current
            .as_mut()
            .filter(|r| r.room == room && matches!(r.stage, Stage::Sending { .. }))
        else {
            return;
        };
        let Stage::Sending { clip, reply } = std::mem::replace(&mut rec.stage, Stage::Starting)
        else {
            unreachable!()
        };
        match result {
            Ok(()) => {
                self.voice_recorder.current = None;
                self.voice_recorder.refocus = true;
                if reply.is_some() && self.reply_drafts.get(&room) == reply.as_ref() {
                    self.reply_drafts.remove(&room);
                }
            }
            Err(_) => {
                rec.stage = Stage::Ready {
                    clip,
                    note: Some("Couldn't send. Send again to retry.".into()),
                }
            }
        }
    }

    /// Focus the composer again once a sent recording's bar has gone; the
    /// engine's answers can't return a task themselves.
    pub(crate) fn voice_refocus(&mut self) -> Option<Task<Msg>> {
        std::mem::take(&mut self.voice_recorder.refocus).then(refocus)
    }

    fn start_recording(&mut self) -> Task<Msg> {
        if self
            .voice_recorder
            .current
            .as_ref()
            .is_some_and(|r| r.room != self.selected)
        {
            self.status = "Still sending your last voice message…".into();
            return Task::none();
        }
        if self.voice_recorder.current.is_some()
            || self.mic() != Mic::On
            || !self.editor.text().trim().is_empty()
            || self
                .outbox
                .rooms
                .get(&self.selected)
                .is_some_and(|q| !q.is_empty())
        {
            return Task::none();
        }
        self.composer_menu = false;
        self.emotes.stickers = false;
        self.completion.picker = None;
        self.voice_recorder.takes += 1;
        let take = self.voice_recorder.takes;
        self.voice_recorder.current = Some(Recording {
            room: self.selected.clone(),
            take,
            stage: Stage::Starting,
        });
        // The input device the voice settings chose, or the default.
        let device = self.local_audio.mic.clone();
        Task::perform(
            async move {
                crate::engine::rt::blocking(move || Capture::start(device.as_deref()))
                    .await
                    .unwrap_or_else(|| Err("Can't open the microphone.".into()))
            },
            move |result| Msg::VoiceMessage(Input::Started(take, Handoff::new(result))),
        )
    }

    fn recording_tick(&mut self) -> Task<Msg> {
        let selected = self.selected.clone();
        let Some(rec) = self.voice_recorder.current.as_mut() else {
            return Task::none();
        };
        if rec.room != selected && !matches!(rec.stage, Stage::Sending { .. }) {
            self.voice_recorder.current = None;
            self.status = "Voice message discarded: you left the channel.".into();
            return Task::none();
        }
        let Stage::Live {
            capture,
            length,
            levels,
        } = &mut rec.stage
        else {
            return Task::none();
        };
        if let Some(why) = capture.failure() {
            self.voice_recorder.current = None;
            self.status = why;
            return refocus();
        }
        (*length, *levels) = capture.progress(LIVE_BARS);
        if *length >= LONGEST {
            return self.finish_recording(false);
        }
        Task::none()
    }

    /// Stop recording and finish the file; `send` sends it as soon as it's
    /// done. Send on a finished recording sends it.
    fn finish_recording(&mut self, send: bool) -> Task<Msg> {
        let Some(rec) = self.voice_recorder.current.as_mut() else {
            return Task::none();
        };
        match &rec.stage {
            Stage::Live { length, levels, .. } => {
                let finishing = Stage::Finishing {
                    send,
                    length: *length,
                    levels: levels.clone(),
                };
                let Stage::Live { capture, .. } = std::mem::replace(&mut rec.stage, finishing)
                else {
                    unreachable!()
                };
                let take = rec.take;
                Task::perform(
                    async move {
                        crate::engine::rt::blocking(move || capture.finish())
                            .await
                            .unwrap_or_else(|| Err("Recording failed.".into()))
                    },
                    move |result| Msg::VoiceMessage(Input::Finished(take, result)),
                )
            }
            Stage::Ready { clip, .. } if send => {
                let (room, clip) = (rec.room.clone(), clip.clone());
                self.send_clip(room, clip);
                Task::none()
            }
            Stage::Finishing { .. } if send => {
                if let Some(Stage::Finishing { send, .. }) =
                    self.voice_recorder.current.as_mut().map(|r| &mut r.stage)
                {
                    *send = true;
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// Hand the finished clip to the engine, as a reply when one is drafted.
    fn send_clip(&mut self, room: String, clip: Clip) {
        let reply = self.reply_drafts.get(&room).cloned();
        let reply_to = reply.as_ref().map(|q| q.id.clone());
        let sent = clip.clone();
        let for_engine = room.clone();
        self.command(
            move |id| Command::SendVoice {
                id,
                room: for_engine,
                clip: sent,
                reply_to,
            },
            Request::Voice(room.clone()),
        );
        let queued = matches!(self.pending.get(&self.seq), Some(Request::Voice(r)) if *r == room);
        if let Some(rec) = self.voice_recorder.current.as_mut() {
            rec.stage = if queued {
                Stage::Sending { clip, reply }
            } else {
                Stage::Ready {
                    clip,
                    note: Some("Couldn't send. Send again to retry.".into()),
                }
            };
        }
    }

    fn recording_at(&mut self, take: u64) -> Option<&mut Recording> {
        self.voice_recorder
            .current
            .as_mut()
            .filter(|r| r.take == take)
    }
}

fn refocus() -> Task<Msg> {
    iced::widget::operation::focus(COMPOSER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rooms::Room;

    fn app() -> (App, async_channel::Receiver<Command>) {
        let (tx, _) = async_channel::bounded(8);
        let mut app = App::new(tx);
        let (sdk, commands) = async_channel::bounded(8);
        app.sdk = Some(sdk);
        app.logged_in = true;
        app.connected = true;
        app.verified = true;
        app.selected = "!a".into();
        app.live.list.insert(
            "!a".into(),
            Room {
                has_metadata: true,
                ..Default::default()
            },
        );
        (app, commands)
    }

    fn clip(ms: u64) -> Clip {
        Clip {
            ogg: b"OggS".to_vec(),
            duration_ms: ms,
            waveform: vec![512; super::super::BARS],
        }
    }

    /// A recording at the Finishing stage, as Send leaves a live one.
    fn finishing(app: &mut App, send: bool) -> u64 {
        app.voice_recorder.takes += 1;
        let take = app.voice_recorder.takes;
        app.voice_recorder.current = Some(Recording {
            room: "!a".into(),
            take,
            stage: Stage::Finishing {
                send,
                length: Duration::from_secs(3),
                levels: vec![],
            },
        });
        take
    }

    #[test]
    fn mic_follows_permissions() {
        let (mut app, _) = app();
        assert_eq!(app.mic(), Mic::On);
        app.connected = false;
        assert!(matches!(app.mic(), Mic::Off(_)));
        app.connected = true;
        app.live.list.get_mut("!a").unwrap().successor = Some(Default::default());
        assert_eq!(app.mic(), Mic::Hidden);
        app.selected = "!unknown".into();
        assert_eq!(app.mic(), Mic::Hidden);
    }

    #[test]
    fn finished_recording_is_sent_as_a_reply() {
        let (mut app, commands) = app();
        let quote = crate::rooms::Quote {
            id: "$parent".into(),
            sender: "@b:s".into(),
            excerpt: "hi".into(),
        };
        app.reply_drafts.insert("!a".into(), quote.clone());
        let take = finishing(&mut app, true);
        let _ = app.voice_message(Input::Finished(take, Ok(clip(3000))));
        let Ok(Command::SendVoice {
            id,
            room,
            clip: sent,
            reply_to,
        }) = commands.try_recv()
        else {
            panic!("expected SendVoice")
        };
        assert_eq!(
            (room.as_str(), reply_to.as_deref()),
            ("!a", Some("$parent"))
        );
        assert_eq!(sent, clip(3000));
        assert!(matches!(
            app.recording_here().unwrap().stage,
            Stage::Sending { .. }
        ));
        // Cancel can't take it back now.
        let _ = app.voice_message(Input::Cancel);
        assert!(app.recording_here().is_some());
        // Done: the bar goes, and so does the reply.
        app.engine_event(crate::engine::Event::Done { id, result: Ok(()) });
        assert!(app.voice_recorder.current.is_none());
        assert!(!app.reply_drafts.contains_key("!a"));
    }

    #[test]
    fn failed_send_keeps_the_clip_to_retry() {
        let (mut app, commands) = app();
        let take = finishing(&mut app, true);
        let _ = app.voice_message(Input::Finished(take, Ok(clip(2000))));
        let Ok(Command::SendVoice { id, .. }) = commands.try_recv() else {
            panic!("expected SendVoice")
        };
        app.engine_event(crate::engine::Event::Done {
            id,
            result: Err(Failure::Unreachable),
        });
        let Some(Recording {
            stage: Stage::Ready { clip: kept, note },
            ..
        }) = &app.voice_recorder.current
        else {
            panic!("expected the clip kept")
        };
        assert_eq!(*kept, clip(2000));
        assert!(note.is_some());
        let _ = app.voice_message(Input::Send);
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::SendVoice { reply_to: None, .. })
        ));
    }

    #[test]
    fn stale_or_short_recordings_send_nothing() {
        let (mut app, commands) = app();
        let take = finishing(&mut app, true);
        // An answer for an earlier take changes nothing.
        let _ = app.voice_message(Input::Finished(take - 1, Ok(clip(3000))));
        assert!(commands.try_recv().is_err());
        assert!(app.recording_here().is_some());
        // A slip of the finger is dropped with a note.
        let _ = app.voice_message(Input::Finished(take, Ok(clip(200))));
        assert!(commands.try_recv().is_err());
        assert!(app.voice_recorder.current.is_none());
        assert!(!app.status.is_empty());
        // Cancelled while finishing: the late file is ignored.
        let take = finishing(&mut app, true);
        let _ = app.voice_message(Input::Cancel);
        let _ = app.voice_message(Input::Finished(take, Ok(clip(3000))));
        assert!(commands.try_recv().is_err());
        assert!(app.voice_recorder.current.is_none());
    }

    #[test]
    fn leaving_the_channel_discards_the_recording() {
        let (mut app, _) = app();
        finishing(&mut app, false);
        app.voice_recorder.current.as_mut().unwrap().stage = Stage::Starting;
        app.selected = "!b".into();
        let _ = app.voice_message(Input::Tick);
        assert!(app.voice_recorder.current.is_none());
    }

    #[test]
    fn record_needs_an_empty_composer() {
        let (mut app, _) = app();
        app.editor = iced::widget::text_editor::Content::with_text("typed");
        let _ = app.voice_message(Input::Record);
        assert!(app.voice_recorder.current.is_none());
    }
}
