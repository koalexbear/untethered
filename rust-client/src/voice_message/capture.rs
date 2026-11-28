//! Recording a voice message on Linux with GStreamer: the microphone through
//! PulseAudio (PipeWire's pulse server on most desktops, the device the call
//! settings chose or the default) encoded as Opus in Ogg, as Element records
//! them: 48 kHz mono at 24 kbit/s, tuned for voice. The file stays in memory.
//!
//! The microphone is shared: PulseAudio lets the call and a recording read
//! one device at once, so a recording started before a call goes on.
use super::{Clip, waveform};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app as gst_app;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const RATE: u64 = 48_000;
/// One live bar, and one waveform peak, per 50 ms.
const STRETCH: usize = RATE as usize / 20;
/// How long finishing the file may take before the recording is lost.
const FINISH: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Captured {
    /// The Ogg file so far.
    ogg: Vec<u8>,
    /// Samples encoded so far.
    samples: u64,
    /// Each finished stretch's peak, 0 to 1.
    peaks: Vec<f32>,
    stretch_peak: f32,
    stretch_len: usize,
}

/// A recording in progress. Dropping it without [`Capture::finish`] stops
/// it and throws it away.
pub struct Capture {
    pipeline: gst::Pipeline,
    captured: Arc<Mutex<Captured>>,
}

impl Capture {
    /// Start recording from the PulseAudio source `device`, or the default.
    /// Blocks while GStreamer loads (seconds when its registry is rebuilt),
    /// so call it off the UI thread.
    pub fn start(device: Option<&str>) -> Result<Self, String> {
        gst::init().map_err(|e| e.to_string())?;
        let missing = |what: &str| format!("Can't record: GStreamer's {what} is missing.");
        let source = gst::ElementFactory::make("pulsesrc")
            .property("client-name", "Untethered")
            .property(
                "stream-properties",
                gst::Structure::builder("props")
                    .field("media.name", "Voice message")
                    .field("media.role", "communication")
                    .build(),
            )
            .build()
            .map_err(|_| missing("pulsesrc (gst-plugins-good)"))?;
        if let Some(device) = device {
            source.set_property("device", device);
        }
        let make = |factory: &str, plugins: &str| {
            gst::ElementFactory::make(factory)
                .build()
                .map_err(|_| missing(&format!("{factory} ({plugins})")))
        };
        let convert = make("audioconvert", "gst-plugins-base")?;
        let resample = make("audioresample", "gst-plugins-base")?;
        let caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("audio/x-raw")
                    .field("format", "S16LE")
                    .field("rate", RATE as i32)
                    .field("channels", 1)
                    .build(),
            )
            .build()
            .map_err(|_| missing("capsfilter"))?;
        let encoder = make("opusenc", "gst-plugins-base")?;
        encoder.set_property("bitrate", 24_000);
        encoder.set_property_from_str("audio-type", "voice");
        let mux = make("oggmux", "gst-plugins-base")?;
        let sink = gst_app::AppSink::builder().sync(false).build();
        let pipeline = gst::Pipeline::new();
        let chain = [
            &source,
            &convert,
            &resample,
            &caps,
            &encoder,
            &mux,
            sink.upcast_ref(),
        ];
        pipeline
            .add_many(chain)
            .and_then(|_| gst::Element::link_many(chain))
            .map_err(|e| e.to_string())?;

        let captured = Arc::new(Mutex::new(Captured::default()));
        // The levels, from the samples on their way into the encoder.
        let levels = captured.clone();
        encoder
            .static_pad("sink")
            .ok_or("opusenc has no sink pad")?
            .add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                if let Some(buffer) = info.buffer()
                    && let Ok(map) = buffer.map_readable()
                {
                    let mut c = levels.lock().unwrap();
                    measure(&mut c, &map);
                }
                gst::PadProbeReturn::Ok
            });
        let file = captured.clone();
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if let Some(buffer) = sample.buffer()
                        && let Ok(map) = buffer.map_readable()
                    {
                        file.lock().unwrap().ogg.extend_from_slice(&map);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        let capture = Self { pipeline, captured };
        capture
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| capture.failure().unwrap_or_else(cannot_open))?;
        Ok(capture)
    }

    /// How long it has recorded, and the peaks of its last `n` stretches,
    /// oldest first.
    pub fn progress(&self, n: usize) -> (Duration, Vec<f32>) {
        let c = self.captured.lock().unwrap();
        let from = c.peaks.len().saturating_sub(n);
        (
            Duration::from_millis(c.samples * 1000 / RATE),
            c.peaks[from..].to_vec(),
        )
    }

    /// Why the recording stopped, once the device failed or went away.
    pub fn failure(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        let message = bus.pop_filtered(&[gst::MessageType::Error])?;
        let gst::MessageView::Error(error) = message.view() else {
            return None;
        };
        Some(format!("Recording stopped: {}", error.error()))
    }

    /// Stop recording and finish the file. Blocks until the encoder has
    /// written its last page, so call it off the UI thread.
    pub fn finish(self) -> Result<Clip, String> {
        self.pipeline.send_event(gst::event::Eos::new());
        let ended = self.pipeline.bus().and_then(|bus| {
            bus.timed_pop_filtered(
                gst::ClockTime::from_mseconds(FINISH.as_millis() as u64),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
        });
        let _ = self.pipeline.set_state(gst::State::Null);
        match ended.as_ref().map(|m| m.view()) {
            Some(gst::MessageView::Eos(_)) => {}
            Some(gst::MessageView::Error(e)) => {
                return Err(format!("Recording failed: {}", e.error()));
            }
            _ => return Err("Recording failed: the file didn't finish.".into()),
        }
        let mut c = self.captured.lock().unwrap();
        if c.stretch_len > 0 {
            let last = c.stretch_peak;
            c.peaks.push(last);
        }
        if c.ogg.is_empty() || c.samples == 0 {
            return Err("Nothing was recorded. Check the microphone.".into());
        }
        Ok(Clip {
            ogg: std::mem::take(&mut c.ogg),
            duration_ms: c.samples * 1000 / RATE,
            waveform: waveform(&c.peaks),
        })
    }
}

/// A recording dropped unfinished stops at once, closing the microphone.
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn cannot_open() -> String {
    "Can't open the microphone.".into()
}

/// Count S16LE mono samples into the duration and the stretches' peaks.
fn measure(c: &mut Captured, bytes: &[u8]) {
    for sample in bytes.as_chunks::<2>().0 {
        let level = f32::from(i16::from_le_bytes(*sample)).abs() / 32768.;
        c.stretch_peak = c.stretch_peak.max(level);
        c.stretch_len += 1;
        if c.stretch_len == STRETCH {
            let peak = c.stretch_peak;
            c.peaks.push(peak);
            c.stretch_peak = 0.;
            c.stretch_len = 0;
        }
    }
    c.samples += (bytes.len() / 2) as u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_levels_per_stretch() {
        let mut c = Captured::default();
        // 120 ms: 50 ms at half scale, 50 ms of silence, 20 ms at full.
        let mut bytes = Vec::new();
        for i in 0..5760 {
            let s: i16 = match i {
                0..2400 => {
                    if i % 2 == 0 {
                        16384
                    } else {
                        -16384
                    }
                }
                2400..4800 => 0,
                _ => i16::MIN,
            };
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        // In two buffers split mid-stretch.
        measure(&mut c, &bytes[..3000]);
        measure(&mut c, &bytes[3000..]);
        assert_eq!(c.samples, 5760);
        assert_eq!(c.peaks, vec![0.5, 0.]);
        assert_eq!((c.stretch_len, c.stretch_peak), (960, 1.));
    }
}
