//! Voice settings kept on this computer (`voice.json` in the profile
//! directory), as Discord keeps its Voice & Video settings per install:
//! echo cancellation, noise suppression, automatic gain control and the
//! input sensitivity of the voice-activity gate (calls plan, step A2). They
//! apply to the call in progress at once and to every later call.
//!
//! The controls live in the call's Devices panel for now
//! (`view/voice_settings.rs`); the Voice & Video settings page (U2) takes
//! them over, with this file and these messages unchanged.
use crate::{App, Msg};
use iced::Task;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The manual threshold's range, in dBFS. At the bottom the gate never
/// closes, as Discord's slider at −100 dB sends everything.
pub const THRESHOLD_RANGE: std::ops::RangeInclusive<f32> = -100.0..=0.0;

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
}
impl Default for Voice {
    fn default() -> Self {
        // All processing on and the sensitivity automatic, as on Discord.
        Self {
            echo_cancellation: true,
            noise_suppression: true,
            gain_control: true,
            auto_sensitivity: true,
            threshold: -50.0,
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
    fn path() -> Option<PathBuf> {
        Some(crate::engine::profile_root().ok()?.join("voice.json"))
    }
    pub fn load() -> Self {
        let mut voice: Self = Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        voice.threshold = clamp_threshold(voice.threshold);
        voice
    }
    fn save(self) -> Result<(), &'static str> {
        const FAILED: &str = "Couldn’t save the voice setting in the profile directory.";
        let path = Self::path().ok_or(FAILED)?;
        let dir = path.parent().ok_or(FAILED)?;
        std::fs::create_dir_all(dir).map_err(|_| FAILED)?;
        // Replace the file whole, so a crash never leaves half a file.
        let next = path.with_extension("json.new");
        std::fs::write(&next, serde_json::to_vec_pretty(&self).map_err(|_| FAILED)?)
            .map_err(|_| FAILED)?;
        std::fs::rename(&next, &path).map_err(|_| FAILED)
    }
    fn apply(&mut self, change: Change) {
        match change {
            Change::EchoCancellation(on) => self.echo_cancellation = on,
            Change::NoiseSuppression(on) => self.noise_suppression = on,
            Change::GainControl(on) => self.gain_control = on,
            Change::AutoSensitivity(on) => self.auto_sensitivity = on,
            Change::Threshold(db) => self.threshold = clamp_threshold(db),
        }
    }
}
fn clamp_threshold(db: f32) -> f32 {
    if db.is_finite() {
        db.clamp(*THRESHOLD_RANGE.start(), *THRESHOLD_RANGE.end())
    } else {
        Voice::default().threshold
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Change {
    EchoCancellation(bool),
    NoiseSuppression(bool),
    GainControl(bool),
    AutoSensitivity(bool),
    Threshold(f32),
}

#[derive(Clone, Debug)]
pub enum Event {
    /// Applies at once and is saved.
    Set(Change),
    Saved(Voice, Result<(), &'static str>),
}

/// The settings and their saving, in `App`.
#[derive(Default)]
pub struct Settings {
    pub voice: Voice,
    /// What the file holds, or is being written with.
    saved: Voice,
    saving: bool,
    pub note: Option<&'static str>,
}
impl Settings {
    pub fn loaded() -> Self {
        let voice = Voice::load();
        Self {
            voice,
            saved: voice,
            saving: false,
            note: None,
        }
    }
}

impl App {
    pub(crate) fn voice_event(&mut self, event: Event) -> Task<Msg> {
        match event {
            Event::Set(change) => {
                let before = self.voice.voice;
                self.voice.voice.apply(change);
                if self.voice.voice != before
                    && let Some(call) = &self.call
                {
                    let voice = self.voice.voice;
                    call.audio.send_if_modified(|mode| {
                        let changed = mode.voice != voice;
                        mode.voice = voice;
                        changed
                    });
                }
                return self.save_voice();
            }
            Event::Saved(written, result) => {
                self.voice.saving = false;
                self.voice.note = result.err();
                if result.is_ok() {
                    self.voice.saved = written;
                    // Changed while that was written (a slider moving):
                    // write the newest too.
                    return self.save_voice();
                }
            }
        }
        Task::none()
    }
    /// Write the current settings unless the file has them already. One
    /// write at a time, so an older one can't land last, and a slider's
    /// many steps make a few writes.
    fn save_voice(&mut self) -> Task<Msg> {
        let next = self.voice.voice;
        if self.voice.saving || self.voice.saved == next {
            return Task::none();
        }
        self.voice.saving = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || next.save())
                    .await
                    .unwrap_or(Err("Couldn’t save the voice setting."))
            },
            move |result| Msg::Voice(Event::Saved(next, result)),
        )
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
        let partial: Voice = serde_json::from_str(r#"{"noise_suppression":false}"#).unwrap();
        assert_eq!(
            partial,
            Voice {
                noise_suppression: false,
                ..Voice::default()
            }
        );
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
}
