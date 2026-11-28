//! The Voice & Video settings (calls plan, steps A2 and U2), kept on this
//! computer as `voice.json` in the profile directory, as Discord keeps them
//! per install: the input and output devices and volumes, the input mode
//! with its push-to-talk key and release delay, the voice-activity gate's
//! input sensitivity, echo cancellation, noise suppression, automatic gain
//! control and the camera. Each applies to the call in progress at once and
//! to every later call.
//!
//! The page that sets them is `view/user_settings.rs`. It can also try the
//! microphone (a local loopback with a level meter) and the camera (a
//! preview that publishes nothing) without a call; their runs live here.
use crate::{App, Msg, call_devices::Port, desktop_call};
use iced::{Subscription, Task};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The manual threshold's range, in dBFS. At the bottom the gate never
/// closes, as Discord's slider at −100 dB sends everything.
pub const THRESHOLD_RANGE: std::ops::RangeInclusive<f32> = -100.0..=0.0;
/// Input and output volume, in percent; 100 leaves the sound as it is.
pub const VOLUME_RANGE: std::ops::RangeInclusive<u8> = 0..=200;
/// Push-to-talk's release delay, in milliseconds, as Discord's slider.
pub const RELEASE_RANGE: std::ops::RangeInclusive<u16> = 0..=2000;

/// What a call reads for every 10 ms of audio: the microphone's processing,
/// gate and volume, the playback volume and push-to-talk's release delay.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Voice {
    pub echo_cancellation: bool,
    pub noise_suppression: bool,
    pub gain_control: bool,
    /// Discord's "Automatically determine input sensitivity": the gate
    /// follows the room's noise floor instead of `threshold`.
    pub auto_sensitivity: bool,
    /// The gate's threshold when not automatic, in dBFS.
    pub threshold: f32,
    /// The microphone's volume before processing, in percent.
    pub input_volume: u8,
    /// Everything the call plays, in percent, on top of each participant's
    /// own volume.
    pub output_volume: u8,
    /// How long push-to-talk keeps sending after the key is released, in
    /// milliseconds, so the end of a word isn't cut off.
    pub release_delay: u16,
}
impl Default for Voice {
    fn default() -> Self {
        // All processing on and the sensitivity automatic, as on Discord.
        // The release delay covers the 40 ms capture buffer and the last
        // 20 ms Opus frame (calls plan K1).
        Self {
            echo_cancellation: true,
            noise_suppression: true,
            gain_control: true,
            auto_sensitivity: true,
            threshold: -50.0,
            input_volume: 100,
            output_volume: 100,
            release_delay: 100,
        }
    }
}

/// What the voice-activity gate compares each 10 ms of processed microphone
/// audio with.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sensitivity {
    Auto,
    /// Send audio at or above this level, in dBFS.
    Threshold(f32),
}

impl Voice {
    pub fn sensitivity(&self) -> Sensitivity {
        if self.auto_sensitivity {
            Sensitivity::Auto
        } else {
            Sensitivity::Threshold(self.threshold)
        }
    }
    /// Whether the audio processing module runs at all: with everything off
    /// the microphone is sent as captured.
    pub fn processing(&self) -> bool {
        self.echo_cancellation || self.noise_suppression || self.gain_control
    }
    pub fn release(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.release_delay.into())
    }
    fn apply(&mut self, change: Change) {
        match change {
            Change::EchoCancellation(on) => self.echo_cancellation = on,
            Change::NoiseSuppression(on) => self.noise_suppression = on,
            Change::GainControl(on) => self.gain_control = on,
            Change::AutoSensitivity(on) => self.auto_sensitivity = on,
            Change::Threshold(db) => self.threshold = clamp_threshold(db),
            Change::InputVolume(v) => self.input_volume = v.min(*VOLUME_RANGE.end()),
            Change::OutputVolume(v) => self.output_volume = v.min(*VOLUME_RANGE.end()),
            Change::ReleaseDelay(ms) => self.release_delay = ms.min(*RELEASE_RANGE.end()),
        }
    }
    /// Values an older or hand-edited file may hold, in range.
    fn clamped(mut self) -> Self {
        self.threshold = clamp_threshold(self.threshold);
        self.input_volume = self.input_volume.min(*VOLUME_RANGE.end());
        self.output_volume = self.output_volume.min(*VOLUME_RANGE.end());
        self.release_delay = self.release_delay.min(*RELEASE_RANGE.end());
        self
    }
}
fn clamp_threshold(db: f32) -> f32 {
    if db.is_finite() {
        db.clamp(*THRESHOLD_RANGE.start(), *THRESHOLD_RANGE.end())
    } else {
        Voice::default().threshold
    }
}

/// A camera chosen on the page: what PipeWire connects to (its node name,
/// which survives restarts) and the name the list shows.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Camera {
    pub target: String,
    pub label: String,
}

/// The rest of the page's choices, saved beside [`Voice`] in the same file.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Choices {
    /// PulseAudio device names; None follows the system default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// Discord's Input Mode: push to talk instead of voice activity.
    pub push_to_talk: bool,
    /// The push-to-talk key as an XDG shortcuts trigger ("Control_R",
    /// "CTRL+SHIFT+t"); None uses the default (hotkeys.rs).
    pub talk_key: Option<String>,
    /// None uses the system's default camera.
    pub camera: Option<Camera>,
}

/// The file: one flat JSON object, so files from before U2 still load.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Saved {
    #[serde(flatten)]
    pub voice: Voice,
    #[serde(flatten)]
    pub choices: Choices,
}
impl Saved {
    fn path() -> Option<PathBuf> {
        Some(crate::engine::profile_root().ok()?.join("voice.json"))
    }
    pub fn load() -> Self {
        let mut saved: Self = Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        saved.voice = saved.voice.clamped();
        saved
    }
    fn save(&self) -> Result<(), &'static str> {
        const FAILED: &str = "Couldn’t save the voice setting in the profile directory.";
        let path = Self::path().ok_or(FAILED)?;
        let dir = path.parent().ok_or(FAILED)?;
        std::fs::create_dir_all(dir).map_err(|_| FAILED)?;
        // Replace the file whole, so a crash never leaves half a file.
        let next = path.with_extension("json.new");
        std::fs::write(&next, serde_json::to_vec_pretty(self).map_err(|_| FAILED)?)
            .map_err(|_| FAILED)?;
        std::fs::rename(&next, &path).map_err(|_| FAILED)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Change {
    EchoCancellation(bool),
    NoiseSuppression(bool),
    GainControl(bool),
    AutoSensitivity(bool),
    Threshold(f32),
    InputVolume(u8),
    OutputVolume(u8),
    ReleaseDelay(u16),
}

#[derive(Clone, Debug)]
pub enum Event {
    /// A [`Voice`] value: applies at once and is saved.
    Set(Change),
    /// Input mode: push to talk (true) or voice activity.
    PushToTalk(bool),
    /// Open or close the push-to-talk key's binding dialog.
    RecordKey(bool),
    /// A key went down or up while the dialog listens: its keysym name
    /// (None for a key push-to-talk can't use) and the modifiers held.
    Key(bool, Option<String>, iced::keyboard::Modifiers),
    /// Back to the default push-to-talk key.
    DefaultKey,
    /// Start or stop the mic test.
    MicTest(bool),
    MicTestEnded(u64, Result<(), &'static str>),
    /// Start or stop the camera preview.
    Preview(bool),
    Previewed(u64, desktop_call::Preview),
    Saved(Saved, Result<(), &'static str>),
}

/// The mic test: a loopback of the microphone through the call's
/// processing and mixer to the speaker. It runs as a subscription while the
/// page shows it, so leaving the page stops it.
pub struct MicTest {
    id: u64,
    /// The devices and settings it uses, changed as the page changes them.
    pub audio: tokio::sync::watch::Sender<desktop_call::AudioMode>,
    pub level: tokio::sync::watch::Receiver<desktop_call::InputLevel>,
    run: TestRun,
}
/// What the mic test's subscription runs with; one run per id.
#[derive(Clone)]
struct TestRun {
    id: u64,
    mode: tokio::sync::watch::Receiver<desktop_call::AudioMode>,
    level: tokio::sync::watch::Sender<desktop_call::InputLevel>,
}
impl std::hash::Hash for TestRun {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// The settings, their saving, and the page's tests, in `App`. The chosen
/// microphone and speaker live in `App::local_audio`, as before U2, and are
/// saved from there.
#[derive(Default)]
pub struct Settings {
    pub voice: Voice,
    pub push_to_talk: bool,
    pub talk_key: Option<String>,
    pub camera: Option<Camera>,
    /// What the file holds, or is being written with.
    saved: Saved,
    saving: bool,
    pub note: Option<&'static str>,
    /// The binding dialog is open; a modifier pressed alone so far, which
    /// becomes the key if it's released before another key goes down.
    pub recording: bool,
    held_modifier: Option<String>,
    pub key_note: Option<&'static str>,
    pub mic_test: Option<MicTest>,
    pub mic_note: Option<&'static str>,
    /// The camera preview: its run, its newest frame, and why it stopped.
    pub preview: Option<u64>,
    pub frame: Option<iced::widget::image::Handle>,
    pub preview_note: Option<&'static str>,
    runs: u64,
}

impl App {
    /// Load the saved settings, at start.
    pub(crate) fn load_voice(&mut self) {
        let saved = Saved::load();
        self.local_audio.mic = saved.choices.input_device.clone();
        self.local_audio.speaker = saved.choices.output_device.clone();
        self.voice = Settings {
            voice: saved.voice,
            push_to_talk: saved.choices.push_to_talk,
            talk_key: saved.choices.talk_key.clone(),
            camera: saved.choices.camera.clone(),
            saved,
            ..Settings::default()
        };
    }
    /// Push-to-talk is the input mode.
    pub(crate) fn push_to_talk(&self) -> bool {
        self.voice.push_to_talk
    }
    fn current_voice(&self) -> Saved {
        Saved {
            voice: self.voice.voice,
            choices: Choices {
                input_device: self.local_audio.mic.clone(),
                output_device: self.local_audio.speaker.clone(),
                push_to_talk: self.voice.push_to_talk,
                talk_key: self.voice.talk_key.clone(),
                camera: self.voice.camera.clone(),
            },
        }
    }
    pub(crate) fn voice_event(&mut self, event: Event) -> Task<Msg> {
        match event {
            Event::Set(change) => {
                let before = self.voice.voice;
                self.voice.voice.apply(change);
                let voice = self.voice.voice;
                if voice != before {
                    let apply = |audio: &tokio::sync::watch::Sender<desktop_call::AudioMode>| {
                        audio.send_if_modified(|mode| {
                            let changed = mode.voice != voice;
                            mode.voice = voice;
                            changed
                        });
                    };
                    if let Some(call) = &self.call {
                        apply(&call.audio);
                    }
                    if let Some(test) = &self.voice.mic_test {
                        apply(&test.audio);
                    }
                }
            }
            Event::PushToTalk(on) => self.set_push_to_talk(on),
            Event::RecordKey(open) => {
                self.voice.recording = open;
                self.voice.held_modifier = None;
                self.voice.key_note = None;
            }
            Event::Key(down, name, modifiers) => self.recorded_key(down, name, modifiers),
            Event::DefaultKey => self.set_talk_key(None),
            Event::MicTest(on) => return self.mic_test(on),
            Event::MicTestEnded(id, result) => {
                if self.voice.mic_test.as_ref().is_some_and(|t| t.id == id) {
                    self.voice.mic_test = None;
                    self.voice.mic_note = result.err();
                }
            }
            Event::Preview(on) => {
                self.voice.frame = None;
                self.voice.preview_note = None;
                self.voice.preview = on.then(|| {
                    self.voice.runs += 1;
                    self.voice.runs
                });
            }
            Event::Previewed(id, preview) => {
                if self.voice.preview == Some(id) {
                    match preview {
                        desktop_call::Preview::Frame(frame) => self.voice.frame = Some(frame),
                        desktop_call::Preview::Ended(result) => {
                            self.voice.preview = None;
                            self.voice.frame = None;
                            self.voice.preview_note = result.err();
                        }
                    }
                }
            }
            Event::Saved(written, result) => {
                self.voice.saving = false;
                self.voice.note = result.err();
                match result {
                    Ok(()) => self.voice.saved = written,
                    // The next change tries again.
                    Err(_) => return Task::none(),
                }
                // Changed while that was written (a slider moving): write
                // the newest too, below.
            }
        }
        self.save_voice()
    }
    /// Write the current settings unless the file has them already. One
    /// write at a time, so an older one can't land last, and a slider's
    /// many steps make a few writes. The device choices call this too.
    pub(crate) fn save_voice(&mut self) -> Task<Msg> {
        let next = self.current_voice();
        if self.voice.saving || self.voice.saved == next {
            return Task::none();
        }
        self.voice.saving = true;
        let write = next.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || write.save())
                    .await
                    .unwrap_or(Err("Couldn’t save the voice setting."))
            },
            move |result| Msg::Voice(Event::Saved(next, result)),
        )
    }
    /// Voice activity or push to talk, from the page or the voice channel's
    /// header: in the call at once, and saved.
    pub(crate) fn set_push_to_talk(&mut self, on: bool) {
        self.voice.push_to_talk = on;
        self.hotkeys.reset();
        let talk = desktop_call::TalkGate::from_ptt(on);
        if let Some(call) = &self.call
            && !call.cancel.is_closed()
        {
            call.audio.send_modify(|s| s.talk = talk);
        }
        // The mic test skips the gate with push to talk.
        if let Some(test) = &self.voice.mic_test {
            test.audio.send_modify(|s| s.talk = talk);
        }
    }
    fn set_talk_key(&mut self, key: Option<String>) {
        self.voice.recording = false;
        self.voice.held_modifier = None;
        if self.voice.talk_key != key {
            self.voice.talk_key = key;
            // The shortcut listeners start again with the new key; a hold
            // of the old one is forgotten.
            let _ = self.talk_key(crate::hotkeys::Source::Window, false);
            let _ = self.talk_key(crate::hotkeys::Source::Global, false);
            self.hotkeys.reset();
        }
    }
    /// The binding dialog heard a key. An ordinary key binds at once with
    /// the modifiers held; a modifier binds alone when it's released before
    /// any other key goes down (Right Ctrl, Discord's usual choice).
    fn recorded_key(
        &mut self,
        down: bool,
        name: Option<String>,
        modifiers: iced::keyboard::Modifiers,
    ) {
        if !self.voice.recording {
            return;
        }
        let Some(name) = name else {
            if down {
                self.voice.key_note = Some("That key can’t be used for push to talk.");
            }
            return;
        };
        let modifier = crate::hotkeys::is_modifier(&name);
        match (down, modifier) {
            (true, true) => self.voice.held_modifier = Some(name),
            (true, false) => {
                self.set_talk_key(Some(crate::hotkeys::combine(&name, modifiers)));
            }
            (false, true) if self.voice.held_modifier.as_deref() == Some(name.as_str()) => {
                self.set_talk_key(Some(name));
            }
            (false, _) => {}
        }
    }
    /// Start or stop the mic test. It can't run during a call, whose meter
    /// shows the microphone instead.
    fn mic_test(&mut self, on: bool) -> Task<Msg> {
        self.voice.mic_test = None;
        self.voice.mic_note = None;
        if !on {
            return Task::none();
        }
        if self.call.is_some() {
            self.voice.mic_note = Some("Leave the call to test your microphone.");
            return Task::none();
        }
        self.voice.runs += 1;
        let id = self.voice.runs;
        let mut start = desktop_call::AudioMode::default();
        start.mic = self.local_audio.route(Port::Mic);
        start.speaker = self.local_audio.route(Port::Speaker);
        start.voice = self.voice.voice;
        start.talk = desktop_call::TalkGate::from_ptt(self.voice.push_to_talk);
        let (audio, mode) = tokio::sync::watch::channel(start);
        let (level, meter) = tokio::sync::watch::channel(desktop_call::InputLevel::default());
        self.voice.mic_test = Some(MicTest {
            id,
            audio,
            level: meter,
            run: TestRun { id, mode, level },
        });
        Task::none()
    }
    /// The page was opened or changed: tests left running on it stop, as
    /// they did when it was left.
    pub(crate) fn reset_voice_page(&mut self) {
        self.voice.mic_test = None;
        self.voice.mic_note = None;
        self.voice.preview = None;
        self.voice.frame = None;
        self.voice.preview_note = None;
        self.voice.recording = false;
    }
    /// A device chosen on the page or in a device menu: the mic test
    /// follows it too.
    pub(crate) fn mic_test_device(&self, port: Port) {
        if let Some(test) = &self.voice.mic_test {
            let route = self.local_audio.route(port);
            test.audio.send_modify(|s| {
                let current = match port {
                    Port::Mic => &mut s.mic,
                    Port::Speaker => &mut s.speaker,
                };
                current.name = route.name;
                current.revision += 1;
            });
        }
    }
    /// The page's meter: the call's microphone during a call, the mic
    /// test's otherwise.
    pub(crate) fn input_level(&self) -> Option<desktop_call::InputLevel> {
        if let Some(call) = self.call.as_ref().filter(|c| !c.cancel.is_closed()) {
            return Some(*call.input.borrow());
        }
        self.voice.mic_test.as_ref().map(|t| *t.level.borrow())
    }
    /// The Voice & Video page is on screen.
    pub(crate) fn voice_page_open(&self) -> bool {
        self.account
            && self.logged_in
            && self.verified
            && self.settings_page == crate::view::user_settings::Page::Voice
    }
    /// What the page needs while it's open: its meter redrawn, the camera
    /// preview's frames and the binding dialog's keys.
    pub(crate) fn voice_subscription(&self) -> Subscription<Msg> {
        let open = self.voice_page_open();
        let meter = if open && self.input_level().is_some() {
            iced::time::every(std::time::Duration::from_millis(50)).map(|_| Msg::VoiceMeter)
        } else {
            Subscription::none()
        };
        let test = match &self.voice.mic_test {
            Some(test) if open => Subscription::run_with(test.run.clone(), |run| {
                let run = run.clone();
                iced::stream::channel(1, async move |mut out| {
                    use iced::futures::SinkExt;
                    let result = desktop_call::mic_test(run.mode, run.level).await;
                    let _ = out
                        .send(Msg::Voice(Event::MicTestEnded(run.id, result)))
                        .await;
                })
            }),
            _ => Subscription::none(),
        };
        let preview = match self.voice.preview {
            Some(id) if open => Subscription::run_with(
                (
                    id,
                    desktop_call::CameraChoice::saved(self.voice.camera.as_ref())
                        .target()
                        .map(str::to_owned),
                ),
                |(id, target)| {
                    use iced::futures::StreamExt;
                    let id = *id;
                    desktop_call::preview(target.clone())
                        .map(move |p| Msg::Voice(Event::Previewed(id, p)))
                },
            ),
            _ => Subscription::none(),
        };
        let keys = if open && self.voice.recording {
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key,
                    physical_key,
                    modifiers,
                    repeat: false,
                    ..
                }) if key != iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                    Some(Msg::Voice(Event::Key(
                        true,
                        crate::hotkeys::name(&key, physical_key),
                        modifiers,
                    )))
                }
                iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
                    key,
                    physical_key,
                    modifiers,
                    ..
                }) => Some(Msg::Voice(Event::Key(
                    false,
                    crate::hotkeys::name(&key, physical_key),
                    modifiers,
                ))),
                _ => None,
            })
        } else {
            Subscription::none()
        };
        Subscription::batch([meter, test, preview, keys])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_discord_and_old_files_fill_in_the_rest() {
        let voice = Voice::default();
        assert!(voice.echo_cancellation && voice.noise_suppression && voice.gain_control);
        assert_eq!(voice.sensitivity(), Sensitivity::Auto);
        assert_eq!((voice.input_volume, voice.output_volume), (100, 100));
        // A2's file, before U2 saved devices, keys and volumes.
        let partial: Saved = serde_json::from_str(r#"{"noise_suppression":false}"#).unwrap();
        assert_eq!(
            partial,
            Saved {
                voice: Voice {
                    noise_suppression: false,
                    ..Voice::default()
                },
                choices: Choices::default(),
            }
        );
    }

    #[test]
    fn every_choice_round_trips_through_one_flat_file() {
        let saved = Saved {
            voice: Voice {
                auto_sensitivity: false,
                threshold: -42.0,
                input_volume: 150,
                output_volume: 40,
                release_delay: 300,
                ..Voice::default()
            },
            choices: Choices {
                input_device: Some("QAMicB.monitor".into()),
                output_device: Some("QASpeakerB".into()),
                push_to_talk: true,
                talk_key: Some("F9".into()),
                camera: Some(Camera {
                    target: "qa-second-camera".into(),
                    label: "Cobalt".into(),
                }),
            },
        };
        let json = serde_json::to_value(&saved).unwrap();
        assert_eq!(json["input_device"], "QAMicB.monitor");
        assert_eq!(json["threshold"], -42.0);
        assert_eq!(json["camera"]["target"], "qa-second-camera");
        assert_eq!(serde_json::from_value::<Saved>(json).unwrap(), saved);
    }

    #[test]
    fn manual_sensitivity_uses_the_threshold_kept_in_range() {
        let mut voice = Voice::default();
        voice.apply(Change::AutoSensitivity(false));
        voice.apply(Change::Threshold(-140.0));
        assert_eq!(voice.sensitivity(), Sensitivity::Threshold(-100.0));
        voice.apply(Change::Threshold(-42.0));
        assert_eq!(voice.sensitivity(), Sensitivity::Threshold(-42.0));
        voice.apply(Change::Threshold(f32::NAN));
        assert_eq!(voice.threshold, Voice::default().threshold);
        voice.apply(Change::InputVolume(250));
        voice.apply(Change::ReleaseDelay(9000));
        assert_eq!((voice.input_volume, voice.release_delay), (200, 2000));
        for off in [
            Change::EchoCancellation(false),
            Change::NoiseSuppression(false),
            Change::GainControl(false),
        ] {
            assert!(voice.processing());
            voice.apply(off);
        }
        assert!(
            !voice.processing(),
            "all off sends the microphone as captured"
        );
    }

    #[test]
    fn out_of_range_values_in_the_file_load_in_range() {
        let saved: Saved =
            serde_json::from_str(r#"{"output_volume":255,"release_delay":60000,"threshold":12}"#)
                .unwrap();
        let voice = saved.voice.clamped();
        assert_eq!(
            (voice.output_volume, voice.release_delay, voice.threshold),
            (200, 2000, 0.0)
        );
    }

    /// An app, and its engine's end of the command queue, which must
    /// stay open for a call to start.
    fn app() -> (App, async_channel::Receiver<crate::backend::Command>) {
        let (commands, backend) = async_channel::bounded(8);
        (App::new(commands), backend)
    }

    #[test]
    fn the_binding_dialog_takes_a_key_with_modifiers_or_a_modifier_alone() {
        use iced::keyboard::Modifiers;
        let (mut app, _backend) = app();
        let _ = app.voice_event(Event::RecordKey(true));
        // Ctrl held, then T: the combination.
        let _ = app.voice_event(Event::Key(true, Some("Control_L".into()), Modifiers::CTRL));
        let _ = app.voice_event(Event::Key(true, Some("t".into()), Modifiers::CTRL));
        assert_eq!(app.voice.talk_key.as_deref(), Some("CTRL+t"));
        assert!(!app.voice.recording);
        // Right Ctrl pressed and released alone.
        let _ = app.voice_event(Event::RecordKey(true));
        let _ = app.voice_event(Event::Key(true, Some("Control_R".into()), Modifiers::CTRL));
        let _ = app.voice_event(Event::Key(
            false,
            Some("Control_R".into()),
            Modifiers::empty(),
        ));
        assert_eq!(app.voice.talk_key.as_deref(), Some("Control_R"));
        // A key it can't use leaves the dialog open with a note.
        let _ = app.voice_event(Event::RecordKey(true));
        let _ = app.voice_event(Event::Key(true, None, Modifiers::empty()));
        assert!(app.voice.recording && app.voice.key_note.is_some());
        let _ = app.voice_event(Event::Key(true, Some("F9".into()), Modifiers::empty()));
        assert_eq!(app.voice.talk_key.as_deref(), Some("F9"));
        let _ = app.voice_event(Event::DefaultKey);
        assert_eq!(app.voice.talk_key, None);
    }

    #[test]
    fn settings_apply_to_the_call_in_progress_and_the_next_one() {
        let (mut app, _backend) = app();
        let _ = app.voice_event(Event::PushToTalk(true));
        let _ = app.voice_event(Event::Set(Change::OutputVolume(50)));
        app.start_voice("!dm".into(), desktop_call::Target::Channel);
        {
            let mode = app.call.as_ref().unwrap().audio.borrow();
            assert_eq!(mode.voice.output_volume, 50);
            assert!(mode.talk == desktop_call::TalkGate::Waiting);
        }
        let _ = app.voice_event(Event::Set(Change::InputVolume(150)));
        let _ = app.voice_event(Event::PushToTalk(false));
        let mode = app.call.as_ref().unwrap().audio.borrow();
        assert_eq!(mode.voice.input_volume, 150);
        assert!(mode.talk == desktop_call::TalkGate::Open);
    }
}
