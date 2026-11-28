//! Voice messages (ROADMAP M10), recorded from the composer and sent as
//! Element does, under unstable MSC3245: an `m.audio` message with an Ogg
//! Opus file, MSC1767's audio block (duration and a waveform of 0 to 1024)
//! and MSC3245's empty `voice` marker, which asks clients to draw it as a
//! voice message.
//!
//! This part is the same everywhere: the waveform, the event content and
//! reading both back from a received message. Recording is Linux's
//! (`recorder`, GStreamer); elsewhere the composer has no microphone button.
//! In the browser, MediaRecorder records WebM in Chromium, which would need
//! remuxing to Ogg before Element's mobile clients could play it.
use serde_json::{Value, json};

#[cfg(target_os = "linux")]
mod capture;
#[cfg(target_os = "linux")]
mod recorder;
#[cfg(target_os = "linux")]
pub use recorder::{Input, LIVE_BARS, Mic, Recorder, Stage};

/// Escape while recording in the open channel cancels the recording.
impl crate::App {
    pub(crate) fn escape_voice_message(&mut self) -> Option<iced::Task<crate::Msg>> {
        #[cfg(target_os = "linux")]
        if self.recording_here().is_some() {
            return Some(self.voice_message(Input::Cancel));
        }
        None
    }
}

/// What clients without voice messages show, as Element sends it.
pub const BODY: &str = "Voice message";
pub const FILE_NAME: &str = "Voice message.ogg";
pub const MIME: &str = "audio/ogg";
/// The waveform's bars, as many as Element sends.
pub const BARS: usize = 100;
/// MSC3246's loudest bar.
pub const LOUDEST: u16 = 1024;
/// Quieter than this (about -40 dB) a recording is silence: its waveform
/// stays flat rather than being scaled up to fill the bars.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const SILENCE: f32 = 0.01;

/// A finished recording: the Ogg Opus file, its length and its waveform.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Clip {
    pub ogg: Vec<u8>,
    pub duration_ms: u64,
    /// [`BARS`] values from 0 to [`LOUDEST`].
    pub waveform: Vec<u16>,
}

/// Where the uploaded file is: plain, or encrypted (an `EncryptedFile` as
/// the event carries it) in an encrypted room.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Plain(String),
    Encrypted(Value),
}

/// The waveform of a recording from its peak levels (0 to 1, one per short
/// stretch of audio, in order): [`BARS`] bars, each the loudest peak of its
/// share of the recording, scaled so the loudest bar is [`LOUDEST`], as
/// Element rescales its own. A recording shorter than [`BARS`] stretches
/// repeat their peak over several bars.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn waveform(peaks: &[f32]) -> Vec<u16> {
    if peaks.is_empty() {
        return vec![0; BARS];
    }
    let n = peaks.len();
    let bars: Vec<f32> = (0..BARS)
        .map(|i| {
            let start = i * n / BARS;
            let end = ((i + 1) * n / BARS).clamp(start + 1, n);
            peaks[start..end]
                .iter()
                .copied()
                .filter(|p| p.is_finite())
                .fold(0., f32::max)
        })
        .collect();
    let loudest = bars.iter().copied().fold(0., f32::max).max(SILENCE);
    bars.into_iter()
        .map(|b| ((b / loudest).clamp(0., 1.) * f32::from(LOUDEST)).round() as u16)
        .collect()
}

/// How tall a live bar is while recording, from a peak level (0 to 1): a
/// decibel scale from -50 dB to full scale, so speech fills the bars
/// without being normalized against what is still to come.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn live_level(peak: f32) -> f32 {
    if !(peak > 0.) {
        return 0.;
    }
    ((20. * peak.log10() + 50.) / 50.).clamp(0., 1.)
}

/// The content of the `m.room.message` that sends `clip`, uploaded to
/// `source` as `size` bytes, in Element Web's shape (its
/// `createVoiceMessageContent`).
pub fn content(source: &Source, size: u64, clip: &Clip) -> Value {
    let mut file = json!({ "name": FILE_NAME, "mimetype": MIME, "size": size });
    let mut content = json!({
        "body": BODY,
        "msgtype": "m.audio",
        "info": { "duration": clip.duration_ms, "mimetype": MIME, "size": size },
        "org.matrix.msc1767.text": BODY,
        "org.matrix.msc1767.audio": {
            "duration": clip.duration_ms,
            "waveform": clip.waveform,
        },
        "org.matrix.msc3245.voice": {},
    });
    match source {
        Source::Plain(url) => {
            content["url"] = json!(url);
            file["url"] = json!(url);
        }
        Source::Encrypted(encrypted) => {
            content["file"] = encrypted.clone();
            file["file"] = encrypted.clone();
        }
    }
    content["org.matrix.msc1767.file"] = file;
    content
}

/// What a received audio message says about itself beyond the file: whether
/// it is a voice message, its length and its waveform.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct Shape {
    /// MSC3245's marker: draw it as a voice message.
    pub voice: bool,
    pub duration_ms: Option<u64>,
    /// From 0 to [`LOUDEST`]; empty when the sender gave none.
    pub waveform: Vec<u16>,
}

/// Read an `m.audio` message's [`Shape`], when it has more than the file:
/// MSC3245's marker, or MSC1767's audio block, or a duration in `info`.
pub fn shape(content: &Value) -> Option<Shape> {
    if content["msgtype"] != "m.audio" {
        return None;
    }
    let audio = &content["org.matrix.msc1767.audio"];
    let voice = content["org.matrix.msc3245.voice"].is_object();
    let duration_ms = audio["duration"]
        .as_u64()
        .or_else(|| content["info"]["duration"].as_u64());
    let waveform: Vec<u16> = audio["waveform"]
        .as_array()
        .map(|bars| {
            bars.iter()
                .filter_map(Value::as_f64)
                .map(|v| v.round().clamp(0., f64::from(LOUDEST)) as u16)
                .collect()
        })
        .unwrap_or_default();
    (voice || duration_ms.is_some() || !waveform.is_empty()).then_some(Shape {
        voice,
        duration_ms,
        waveform,
    })
}

/// `bars` resampled to `n` heights from 0 to 1, for drawing: each the
/// loudest of its share, or a repeat when there are fewer bars than `n`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn heights(bars: &[u16], n: usize) -> Vec<f32> {
    if bars.is_empty() || n == 0 {
        return vec![0.; n];
    }
    let len = bars.len();
    (0..n)
        .map(|i| {
            let start = i * len / n;
            let end = ((i + 1) * len / n).clamp(start + 1, len);
            let loudest = bars[start..end].iter().copied().max().unwrap_or(0);
            f32::from(loudest.min(LOUDEST)) / f32::from(LOUDEST)
        })
        .collect()
}

/// A length as Element and Discord show it: 0:07, 1:05, 12:00.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip() -> Clip {
        Clip {
            ogg: b"OggS".to_vec(),
            duration_ms: 4250,
            waveform: vec![0, 512, 1024],
        }
    }

    #[test]
    fn waveform_has_bars_from_zero_to_loudest() {
        // Five seconds of 50 ms peaks: quiet, then loud, then silent.
        let peaks: Vec<f32> = (0..100)
            .map(|i| match i {
                0..40 => 0.1,
                40..70 => 0.5,
                _ => 0.,
            })
            .collect();
        let bars = waveform(&peaks);
        assert_eq!(bars.len(), BARS);
        assert!(bars.iter().all(|&b| b <= LOUDEST));
        // Scaled so the loudest bar is the top.
        assert_eq!(bars[50], LOUDEST);
        assert_eq!(bars[10], 205); // 0.1 / 0.5 of 1024
        assert_eq!(bars[90], 0);
    }

    #[test]
    fn waveform_takes_each_shares_loudest_peak() {
        let mut peaks = vec![0.2; 1000];
        peaks[505] = 0.8; // one loud moment, in the 51st bar's share
        let bars = waveform(&peaks);
        assert_eq!(bars[50], LOUDEST);
        assert_eq!(bars[49], 256);
        assert_eq!(bars[51], 256);
    }

    #[test]
    fn short_recordings_spread_over_every_bar() {
        let bars = waveform(&[0.25, 1.0]);
        assert_eq!(bars.len(), BARS);
        assert!(bars[..50].iter().all(|&b| b == 256));
        assert!(bars[50..].iter().all(|&b| b == LOUDEST));
    }

    #[test]
    fn silence_stays_flat() {
        assert_eq!(waveform(&[]), vec![0; BARS]);
        let hum = waveform(&[0.001, 0.002, 0.001]);
        assert!(hum.iter().all(|&b| b <= 205), "{hum:?}");
        // Nothing breaks on a bad level.
        assert!(waveform(&[f32::NAN, 0.5]).iter().all(|&b| b <= LOUDEST));
    }

    #[test]
    fn live_bars_use_decibels() {
        assert_eq!(live_level(0.), 0.);
        assert_eq!(live_level(1.), 1.);
        assert!((live_level(0.1) - 0.6).abs() < 0.001); // -20 dB
        assert_eq!(live_level(0.001), 0.); // -60 dB
        assert_eq!(live_level(f32::NAN), 0.);
    }

    #[test]
    fn content_is_elements_voice_message() {
        let c = content(
            &Source::Plain("mxc://example.org/abc".into()),
            6800,
            &clip(),
        );
        assert_eq!(
            c,
            json!({
                "body": "Voice message",
                "msgtype": "m.audio",
                "url": "mxc://example.org/abc",
                "info": { "duration": 4250, "mimetype": "audio/ogg", "size": 6800 },
                "org.matrix.msc1767.text": "Voice message",
                "org.matrix.msc1767.file": {
                    "url": "mxc://example.org/abc",
                    "name": "Voice message.ogg",
                    "mimetype": "audio/ogg",
                    "size": 6800,
                },
                "org.matrix.msc1767.audio": { "duration": 4250, "waveform": [0, 512, 1024] },
                "org.matrix.msc3245.voice": {},
            })
        );
        // It reads back as one, and as an attachment.
        let shape = shape(&c).unwrap();
        assert!(shape.voice);
        assert_eq!(shape.duration_ms, Some(4250));
        assert_eq!(shape.waveform, vec![0, 512, 1024]);
        let media = crate::media::Media::parse(&c).unwrap();
        assert_eq!(media.kind, crate::media::Kind::Audio);
        assert_eq!(media.voice, Some(shape));
    }

    #[test]
    fn encrypted_content_carries_the_file_not_a_url() {
        let file = json!({
            "url": "mxc://example.org/enc",
            "key": { "kty": "oct", "alg": "A256CTR", "k": "secret", "ext": true, "key_ops": ["encrypt", "decrypt"] },
            "iv": "iv", "hashes": { "sha256": "hash" }, "v": "v2",
        });
        let c = content(&Source::Encrypted(file.clone()), 100, &clip());
        assert_eq!(c["file"], file);
        assert_eq!(c["org.matrix.msc1767.file"]["file"], file);
        assert!(c.get("url").is_none());
        assert!(c["org.matrix.msc1767.file"].get("url").is_none());
        let media = crate::media::Media::parse(&c).unwrap();
        assert!(media.encrypted);
        assert_eq!(media.mxc, "mxc://example.org/enc");
    }

    #[test]
    fn received_shapes() {
        // Element X's: no text fallback, floats are tolerated, too loud is clamped.
        let x = json!({
            "msgtype": "m.audio", "body": "voice.ogg", "url": "mxc://s/a",
            "org.matrix.msc1767.audio": { "duration": 1200, "waveform": [3.0, 2000, -1] },
            "org.matrix.msc3245.voice": {},
        });
        assert_eq!(
            shape(&x),
            Some(Shape {
                voice: true,
                duration_ms: Some(1200),
                waveform: vec![3, LOUDEST, 0]
            })
        );
        // A plain audio file with only info.duration.
        let file = json!({"msgtype":"m.audio","body":"song.mp3","url":"mxc://s/b","info":{"duration":61000}});
        let s = shape(&file).unwrap();
        assert!(!s.voice && s.waveform.is_empty());
        assert_eq!(s.duration_ms, Some(61000));
        // Nothing beyond the file, or not audio.
        assert_eq!(shape(&json!({"msgtype":"m.audio","url":"mxc://s/c"})), None);
        assert_eq!(
            shape(&json!({"msgtype":"m.file","info":{"duration":5}})),
            None
        );
    }

    #[test]
    fn heights_for_drawing() {
        assert_eq!(heights(&[0, 1024], 4), vec![0., 0., 1., 1.]);
        assert_eq!(heights(&[0, 512, 256, 1024], 2), vec![0.5, 1.]);
        assert_eq!(heights(&[], 3), vec![0.; 3]);
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_400), "1:05");
        assert_eq!(clock(720_000), "12:00");
    }
}
