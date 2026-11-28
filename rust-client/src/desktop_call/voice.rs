//! Microphone processing (calls plan, step A2): libwebrtc's audio processing
//! module for echo cancellation, noise suppression and automatic gain
//! control, then a voice-activity gate with Discord's input sensitivity.
//!
//! One [`Voice`] serves a call's LiveKit session. The mixer hands it every
//! frame it plays (the echo canceller's far-end reference) and capture runs
//! every 10 ms microphone frame through it before sending. Below the gate
//! capture sends silence (Opus DTX makes that nearly free) and leaves the
//! publication unmuted, so peers' Speaking and mute icons stay truthful. The
//! gate's decision is also the local Speaking state, without waiting for the
//! SFU's audio levels to come back.
use super::audio::{Frame, RATE, SAMPLES};
use crate::call_settings::{Sensitivity, Voice as Settings};
use livekit::webrtc::native::apm::AudioProcessingModule;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::watch;

/// The quietest level reported, in dBFS; digital silence reads as this.
pub(super) const QUIET: f32 = -100.0;
/// How long the gate stays open after the last frame above it, so word ends
/// and short pauses aren't cut.
const HANGOVER: u32 = 20;
/// The automatic gate opens this far above the noise floor.
const AUTO_MARGIN: f32 = 12.0;
/// Where the automatic noise floor starts, and how fast it follows the
/// level down and up (dB per frame). It falls within a few frames, in any
/// pause between syllables, and rises 5 dB a second, so speech never lifts
/// it while any steady sound, however loud, is learned within seconds. That
/// matters even with noise suppression on: with all three modules running,
/// libwebrtc's gain control lifts steady noise back up for the first ten
/// seconds or so.
const FLOOR_START: f32 = -60.0;
const FLOOR_FALL: f32 = 0.25;
const FLOOR_RISE: f32 = 0.05;
/// Delay between a frame reaching the speaker and its echo reaching the
/// microphone, as a starting hint: the 40 ms playback and capture buffers.
/// AEC3 measures the real delay itself.
const STREAM_DELAY_MS: i32 = 80;

/// The microphone's level after processing, for the sensitivity meter.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InputLevel {
    /// dBFS, from [`QUIET`] to 0, with a meter's fall-back.
    pub db: f32,
    /// The gate hears voice: above the threshold or in its hangover.
    pub voice: bool,
}
impl Default for InputLevel {
    fn default() -> Self {
        Self {
            db: QUIET,
            voice: false,
        }
    }
}

/// The processing settings one module was built with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Config {
    echo: bool,
    gain: bool,
    noise: bool,
}
impl Config {
    fn of(settings: &Settings) -> Option<Self> {
        settings.processing().then_some(Self {
            echo: settings.echo_cancellation,
            gain: settings.gain_control,
            noise: settings.noise_suppression,
        })
    }
}

#[derive(Default)]
struct Apm {
    config: Option<Config>,
    module: Option<AudioProcessingModule>,
}
impl Apm {
    /// libwebrtc fixes a module's settings when it is built, so a change
    /// builds a new one; the echo canceller converges again in a second or
    /// two.
    fn configure(&mut self, settings: &Settings) {
        let config = Config::of(settings);
        if config == self.config {
            return;
        }
        self.config = config;
        self.module = config.map(|c| AudioProcessingModule::new(c.echo, c.gain, true, c.noise));
    }
}

/// One call session's microphone processing, shared by its capture and its
/// mixer.
#[derive(Clone)]
pub(super) struct Voice {
    apm: Arc<Mutex<Apm>>,
    level: watch::Sender<InputLevel>,
    /// Whether you're speaking by the gate, for the voice roster; changes
    /// only when that does.
    talking: watch::Sender<bool>,
}
impl Voice {
    /// `level` is the call's meter, kept across sessions of the call.
    pub(super) fn new(level: watch::Sender<InputLevel>) -> Self {
        Self {
            apm: Arc::default(),
            level,
            talking: watch::Sender::new(false),
        }
    }
    fn apm(&self) -> std::sync::MutexGuard<'_, Apm> {
        self.apm.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// The mixer's far-end tap: every frame that reached the speaker.
    pub(super) fn reference(&self) -> impl FnMut(&Frame) + Send + 'static {
        let voice = self.clone();
        move |frame| voice.played(frame)
    }
    /// A frame that reached the speaker: the echo canceller's reference.
    fn played(&self, frame: &Frame) {
        let mut apm = self.apm();
        if apm.config.is_some_and(|c| c.echo)
            && let Some(module) = &mut apm.module
        {
            let mut frame = *frame;
            let _ = module.process_reverse_stream(&mut frame, RATE as i32, 1);
        }
    }
    /// Run one microphone frame through processing in place.
    fn process(&self, frame: &mut Frame, settings: &Settings) {
        let mut apm = self.apm();
        apm.configure(settings);
        if let Some(module) = &mut apm.module {
            let _ = module.set_stream_delay_ms(STREAM_DELAY_MS);
            let _ = module.process_stream(frame, RATE as i32, 1);
        }
    }
    /// Start a capture: the gate starts closed, its noise floor afresh.
    pub(super) fn capture(&self) -> Capture {
        Capture {
            voice: self.clone(),
            gate: Gate::default(),
            meter: QUIET,
        }
    }
    /// The microphone is closed (muted, deafened, waiting for the talk key or
    /// failed): no level and no speaking.
    pub(super) fn closed(&self) {
        self.level.send_if_modified(|level| {
            let changed = *level != InputLevel::default();
            *level = InputLevel::default();
            changed
        });
        self.talking
            .send_if_modified(|t| std::mem::replace(t, false));
    }
    pub(super) fn talking(&self) -> watch::Receiver<bool> {
        self.talking.subscribe()
    }
}

/// One open microphone's processing state.
pub(super) struct Capture {
    voice: Voice,
    gate: Gate,
    meter: f32,
}
impl Capture {
    /// Process one captured frame in place and gate it. With push-to-talk
    /// the key is the gate: a held key always sends.
    pub(super) fn frame(&mut self, frame: &mut Frame, settings: &Settings, push_to_talk: bool) {
        self.voice.process(frame, settings);
        let db = level(frame);
        let voice = self.gate.decide(db, settings.sensitivity());
        self.gate.shape(frame, voice || push_to_talk);
        // The meter jumps up and falls back at 30 dB a second, so a 20 Hz
        // redraw still shows each syllable's peak.
        self.meter = db.max(self.meter - 0.3);
        self.voice.level.send_replace(InputLevel {
            db: self.meter,
            voice,
        });
        self.voice
            .talking
            .send_if_modified(|t| std::mem::replace(t, voice) != voice);
    }
}

/// A frame's RMS level in dBFS, at least [`QUIET`].
pub(super) fn level(frame: &Frame) -> f32 {
    let power = frame.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / SAMPLES as f64;
    let db = 10.0 * (power / f64::from(i16::MAX).powi(2)).log10();
    (db as f32).max(QUIET)
}

/// The voice-activity gate.
struct Gate {
    /// The automatic threshold's noise floor, dBFS.
    floor: f32,
    /// Frames left before a gate with nothing above it closes.
    hold: u32,
    /// Whether the last frame was sent.
    sent: bool,
}
impl Default for Gate {
    fn default() -> Self {
        Self {
            floor: FLOOR_START,
            hold: 0,
            sent: false,
        }
    }
}
impl Gate {
    fn threshold(&self, sensitivity: Sensitivity) -> f32 {
        match sensitivity {
            Sensitivity::Auto => self.floor + AUTO_MARGIN,
            Sensitivity::Threshold(db) => db,
        }
    }
    /// Whether a frame at `db` is voice, counting the hangover.
    fn decide(&mut self, db: f32, sensitivity: Sensitivity) -> bool {
        let above = match sensitivity {
            // −100 dB sends everything, digital silence included.
            Sensitivity::Threshold(t) if t <= QUIET => true,
            _ => db >= self.threshold(sensitivity),
        };
        if db < self.floor {
            self.floor = (self.floor - FLOOR_FALL * (self.floor - db)).max(db);
        } else {
            self.floor = (self.floor + FLOOR_RISE).min(db);
        }
        if above {
            self.hold = HANGOVER;
            true
        } else if self.hold > 0 {
            self.hold -= 1;
            true
        } else {
            false
        }
    }
    /// Silence a frame below the gate, ramping across the frame where it
    /// opens or closes so the edge doesn't click.
    fn shape(&mut self, frame: &mut Frame, send: bool) {
        let was = std::mem::replace(&mut self.sent, send);
        match (was, send) {
            (true, true) => {}
            (false, false) => *frame = [0; SAMPLES],
            (false, true) => ramp(frame, |n| n),
            (true, false) => ramp(frame, |n| SAMPLES - n),
        }
    }
}
fn ramp(frame: &mut Frame, gain: impl Fn(usize) -> usize) {
    for (n, sample) in frame.iter_mut().enumerate() {
        *sample = (i32::from(*sample) * gain(n) as i32 / SAMPLES as i32) as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    fn tone(hz: f64, amplitude: f64, start: usize) -> Frame {
        std::array::from_fn(|n| {
            (amplitude * (TAU * hz * (start + n) as f64 / f64::from(RATE)).sin()).round() as i16
        })
    }
    /// The amplitude of `hz` in `samples`, by correlation.
    fn amplitude(samples: &[i16], hz: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (n, &s) in samples.iter().enumerate() {
            let phase = TAU * hz * n as f64 / f64::from(RATE);
            re += f64::from(s) * phase.cos();
            im += f64::from(s) * phase.sin();
        }
        2.0 * re.hypot(im) / samples.len() as f64
    }
    fn db(ratio: f64) -> f64 {
        20.0 * ratio.log10()
    }
    /// A repeatable noise generator (xorshift), white or pink.
    struct Noise(u64, [f64; 3]);
    impl Noise {
        fn white(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        }
        /// Paul Kellet's economy pink filter.
        fn pink(&mut self) -> f64 {
            let w = self.white();
            let b = &mut self.1;
            b[0] = 0.99765 * b[0] + w * 0.0990460;
            b[1] = 0.96300 * b[1] + w * 0.2965164;
            b[2] = 0.57000 * b[2] + w * 1.0526913;
            (b[0] + b[1] + b[2] + w * 0.1848) / 4.0
        }
    }
    fn settings(echo: bool, noise: bool, gain: bool) -> Settings {
        Settings {
            echo_cancellation: echo,
            noise_suppression: noise,
            gain_control: gain,
            ..Settings::default()
        }
    }
    fn open(threshold: f32) -> Settings {
        Settings {
            auto_sensitivity: false,
            threshold,
            ..settings(false, false, false)
        }
    }

    #[test]
    fn levels_are_rms_dbfs() {
        assert_eq!(level(&[0; SAMPLES]), QUIET);
        assert!((level(&[i16::MAX; SAMPLES])).abs() < 0.01);
        // A full-scale sine is 3 dB below a full-scale square.
        assert!((level(&tone(1000.0, 32767.0, 0)) + 3.01).abs() < 0.05);
    }

    #[test]
    fn manual_gate_opens_at_once_holds_200_ms_and_closes_quietly() {
        let mut gate = Gate::default();
        let at = Sensitivity::Threshold(-40.0);
        assert!(!gate.decide(-45.0, at));
        assert!(gate.decide(-39.0, at), "opens on the first loud frame");
        for _ in 0..HANGOVER {
            assert!(gate.decide(-60.0, at), "word ends aren't cut");
        }
        assert!(!gate.decide(-60.0, at));
        assert!(gate.decide(-80.0, Sensitivity::Threshold(-100.0)));
        assert!(gate.decide(QUIET, Sensitivity::Threshold(-100.0)));
    }

    #[test]
    fn gate_edges_ramp_and_closed_frames_are_silent() {
        let mut gate = Gate::default();
        let mut frame = [1000; SAMPLES];
        gate.shape(&mut frame, false);
        assert_eq!(frame, [0; SAMPLES]);
        let mut frame = [1000; SAMPLES];
        gate.shape(&mut frame, true);
        assert_eq!((frame[0], frame[SAMPLES / 2]), (0, 500));
        let mut frame = [1000; SAMPLES];
        gate.shape(&mut frame, true);
        assert_eq!(frame, [1000; SAMPLES]);
        gate.shape(&mut frame, false);
        assert_eq!(
            (frame[0], frame[SAMPLES / 2], frame[SAMPLES - 1]),
            (1000, 500, 2)
        );
    }

    #[test]
    fn auto_gate_learns_steady_sound_and_still_hears_speech_over_it() {
        let mut gate = Gate::default();
        let mut noise = Noise(7, [0.0; 3]);
        let mut wobble = |base: f32| base + noise.white() as f32 * 3.0;
        // A quiet room: the floor drops to it within a few frames.
        for _ in 0..20 {
            gate.decide(wobble(-75.0), Sensitivity::Auto);
        }
        assert!(gate.floor < -72.0, "floor {}", gate.floor);
        // A fan starts 25 dB louder: sent at first, learned within seconds.
        let sent = (0..1000)
            .filter(|_| gate.decide(wobble(-50.0), Sensitivity::Auto))
            .count();
        assert!(!gate.decide(wobble(-50.0), Sensitivity::Auto));
        assert!(sent < 500, "sent {sent} frames of fan");
        // Speech: syllables 25 dB over the fan, with short dips between.
        let sent = (0..300)
            .filter(|n| {
                let db = if n % 25 < 15 { -25.0 } else { -50.0 };
                gate.decide(wobble(db), Sensitivity::Auto)
            })
            .count();
        assert_eq!(sent, 300, "every syllable and gap sent");
        assert!(
            gate.floor < -45.0,
            "speech left the floor at {}",
            gate.floor
        );
        // A steady sound is noise however loud: a held tone stops within
        // seconds.
        let sent = (0..1000)
            .filter(|_| gate.decide(wobble(-15.0), Sensitivity::Auto))
            .count();
        assert!(sent < 1000 && !gate.decide(wobble(-15.0), Sensitivity::Auto));
    }

    #[test]
    fn defaults_silence_steady_noise_and_send_speech_over_it() {
        // Real pink noise at −35 dBFS through all three modules and the
        // automatic gate, as a fan next to a microphone.
        let (level, _meter) = watch::channel(InputLevel::default());
        let voice = Voice::new(level);
        let mut capture = voice.capture();
        let defaults = Settings::default();
        let mut noise = Noise(5, [0.0; 3]);
        let mut sent = Vec::new();
        for i in 0..1600 {
            let talker = if i >= 1200 {
                voiced(410.0, 6000.0, i * SAMPLES)
            } else {
                [0; SAMPLES]
            };
            let mut frame: Frame =
                std::array::from_fn(|n| talker[n].saturating_add((noise.pink() * 2600.0) as i16));
            capture.frame(&mut frame, &defaults, false);
            sent.push(frame != [0; SAMPLES]);
        }
        let seconds: Vec<usize> = sent
            .chunks(100)
            .map(|s| s.iter().filter(|&&s| s).count())
            .collect();
        assert!(
            seconds[5..12].iter().all(|&n| n == 0),
            "steady noise still sent after 5 s: {seconds:?}"
        );
        // Syllables over it go out whole, but for the first 10 ms of the
        // first one, while the gate opens.
        let sounding: Vec<usize> = sent[1200..]
            .as_chunks::<25>()
            .0
            .iter()
            .map(|s| s[..15].iter().filter(|&&s| s).count())
            .collect();
        assert!(
            sounding[0] >= 14 && sounding[1..].iter().all(|&n| n == 15),
            "sounding frames sent per syllable: {sounding:?}"
        );
    }

    #[test]
    fn capture_meters_gates_and_reports_speaking_once_per_change() {
        let (level, mut meter) = watch::channel(InputLevel::default());
        let voice = Voice::new(level);
        let mut talking = voice.talking();
        let mut capture = voice.capture();
        let quiet = open(-30.0);
        let mut frame = tone(410.0, 1000.0, 0);
        capture.frame(&mut frame, &quiet, false);
        assert_eq!(frame, [0; SAMPLES], "below the threshold");
        assert!(!meter.borrow_and_update().voice);
        assert!((meter.borrow().db + 33.3).abs() < 0.2);
        assert!(!talking.has_changed().unwrap());
        let mut frame = tone(410.0, 10_000.0, SAMPLES);
        capture.frame(&mut frame, &quiet, false);
        assert!(meter.borrow().voice && *talking.borrow_and_update());
        // Push-to-talk sends a quiet frame without calling it speech.
        let mut capture = voice.capture();
        let mut frame = tone(410.0, 1000.0, 0);
        capture.frame(&mut frame, &quiet, true);
        assert_ne!(frame, [0; SAMPLES]);
        assert!(!meter.borrow().voice && !*talking.borrow_and_update());
        voice.closed();
        assert_eq!(*meter.borrow(), InputLevel::default());
        assert!(!*talking.borrow());
    }

    /// Run `frames` 10 ms frames through a fresh module; `far` makes the
    /// frame played on the speaker, `near` the microphone's given it.
    fn run(
        settings: &Settings,
        frames: usize,
        mut far: impl FnMut(usize) -> Frame,
        mut near: impl FnMut(usize, &Frame) -> Frame,
    ) -> Vec<i16> {
        let (level, _meter) = watch::channel(InputLevel::default());
        let voice = Voice::new(level);
        let mut out = Vec::with_capacity(frames * SAMPLES);
        for i in 0..frames {
            let played = far(i);
            voice.played(&played);
            let mut mic = near(i, &played);
            voice.process(&mut mic, settings);
            out.extend_from_slice(&mic);
        }
        out
    }
    /// The last `seconds` of `out`.
    fn tail(out: &[i16], seconds: usize) -> &[i16] {
        &out[out.len() - seconds * RATE as usize..]
    }

    /// Bruno's syllables: 150 ms of `hz`, 100 ms of silence.
    fn syllables(hz: f64, amplitude: f64, start: usize) -> Frame {
        let mut frame = tone(hz, amplitude, start);
        for (n, sample) in frame.iter_mut().enumerate() {
            if (start + n) % (RATE as usize / 4) >= RATE as usize * 3 / 20 {
                *sample = 0;
            }
        }
        frame
    }
    /// Voiced syllables: `hz` and its next three harmonics, falling off
    /// like a voice's.
    fn voiced(hz: f64, amplitude: f64, start: usize) -> Frame {
        let mut frame = [0i16; SAMPLES];
        for h in 1..=4 {
            let part = syllables(hz * f64::from(h), amplitude / f64::from(h), start);
            for (out, part) in frame.iter_mut().zip(part) {
                *out = out.saturating_add(part);
            }
        }
        frame
    }
    /// The mean amplitude of `hz` over the sounding part of each syllable
    /// in `samples` (which starts on a syllable).
    fn syllable_amplitude(samples: &[i16], hz: f64) -> f64 {
        const SYLLABLE: usize = RATE as usize / 4;
        let chunks: Vec<_> = samples
            .as_chunks::<SYLLABLE>()
            .0
            .iter()
            .map(|c| amplitude(&c[RATE as usize / 50..RATE as usize * 13 / 100], hz))
            .collect();
        chunks.iter().sum::<f64>() / chunks.len() as f64
    }

    #[test]
    fn echo_canceller_removes_the_speakers_echo_and_keeps_the_talker() {
        // Bruno's syllables at 670 Hz play here and return 60 ms later at
        // −6 dB, under Alice's own steady 410 Hz, as in the A2 fixture.
        let echo = |s: &Settings| {
            let mut history = Vec::<Frame>::new();
            run(
                s,
                600,
                |i| voiced(670.0, 7000.0, i * SAMPLES),
                move |i, played| {
                    history.push(*played);
                    let back = history.len().checked_sub(7).map(|n| history[n]);
                    let talker = tone(410.0, 7000.0, i * SAMPLES);
                    std::array::from_fn(|n| talker[n] + back.map_or(0, |b| b[n] / 2))
                },
            )
        };
        let raw = echo(&settings(false, false, false));
        let cancelled = echo(&settings(true, false, false));
        let (raw, cancelled) = (tail(&raw, 2), tail(&cancelled, 2));
        let drop = db(amplitude(raw, 670.0) / amplitude(cancelled, 670.0));
        let kept = db(amplitude(cancelled, 410.0) / amplitude(raw, 410.0));
        assert!(drop >= 25.0, "echo dropped only {drop:.1} dB");
        assert!(kept.abs() <= 6.0, "talker changed {kept:+.1} dB");
    }

    #[test]
    fn noise_suppression_lowers_pink_noise_and_keeps_speech() {
        let noisy = |s: &Settings, talker: fn(f64, f64, usize) -> Frame| {
            let mut noise = Noise(11, [0.0; 3]);
            run(
                s,
                800,
                |_| [0; SAMPLES],
                move |i, _| {
                    let talker = talker(410.0, 3000.0, i * SAMPLES);
                    std::array::from_fn(|n| talker[n] + (noise.pink() * 2500.0) as i16)
                },
            )
        };
        let raw = noisy(&settings(false, false, false), syllables);
        let clean = noisy(&settings(false, true, false), syllables);
        let (raw, clean) = (tail(&raw, 3), tail(&clean, 3));
        // The noise floor away from the voice: bins from 1 to 4 kHz.
        let floor = |s: &[i16]| {
            (0..12)
                .map(|k| amplitude(s, 1000.0 + 250.0 * k as f64).powi(2))
                .sum::<f64>()
                .sqrt()
        };
        let lowered = db(floor(raw) / floor(clean));
        let kept = db(syllable_amplitude(clean, 410.0) / syllable_amplitude(raw, 410.0));
        assert!(lowered >= 10.0, "noise lowered only {lowered:.1} dB");
        assert!(kept.abs() <= 3.0, "speech changed {kept:+.1} dB");
        // A tone that never pauses looks like noise: it is turned down by
        // the suppressor's limit, 12 dB or so, and no further. This is why
        // the fixtures measure processing with syllables.
        let raw = noisy(&settings(false, false, false), tone);
        let clean = noisy(&settings(false, true, false), tone);
        let steady = db(amplitude(tail(&clean, 3), 410.0) / amplitude(tail(&raw, 3), 410.0));
        assert!(
            (-16.0..=-8.0).contains(&steady),
            "steady tone {steady:+.1} dB"
        );
    }

    #[test]
    fn gain_control_settles_on_a_steady_tone_within_3_db() {
        let steady = |s: &Settings| {
            run(
                s,
                2000,
                |_| [0; SAMPLES],
                |i, _| tone(410.0, 7000.0, i * SAMPLES),
            )
        };
        let raw = steady(&settings(false, false, false));
        let out = steady(&settings(false, false, true));
        let reference = amplitude(tail(&raw, 1), 410.0);
        let second = |n: usize| {
            let window = &out[n * RATE as usize..(n + 1) * RATE as usize];
            db(amplitude(window, 410.0) / reference)
        };
        // It starts with some gain and backs off within two seconds.
        assert!(second(0) > 3.0, "first second {:+.1} dB", second(0));
        for n in 2..20 {
            assert!(second(n).abs() <= 3.0, "second {n}: {:+.1} dB", second(n));
        }
    }
}
