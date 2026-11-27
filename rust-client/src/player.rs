//! Video playback for the media viewer (Linux). GStreamer's playbin decodes
//! the file and plays its sound; an appsink hands each frame over as RGBA,
//! which the viewer draws as an image. Dropping the player stops it.
use gstreamer::{self as gst, prelude::*};
use gstreamer_app as gst_app;
use gstreamer_video::{self as gst_video, prelude::*};
use iced::widget::image::Handle;
use std::{
    hash::{Hash, Hasher},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Debug)]
pub enum Event {
    Frame(Handle),
    Ended,
    Failed(String),
}

#[derive(Clone, Copy, Debug)]
pub enum Control {
    Toggle,
    /// To this fraction of the duration.
    Seek(f32),
    Mute,
}

/// The player's events, as a subscription identified by its player.
#[derive(Clone)]
struct Feed {
    id: u64,
    events: async_channel::Receiver<Event>,
}
impl Hash for Feed {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

pub struct Player {
    playbin: gst::Element,
    feed: Feed,
    stop: Arc<AtomicBool>,
    pub frame: Option<Handle>,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub playing: bool,
    pub ended: bool,
    pub muted: bool,
    /// Where a drag on the seek bar is, until it's released.
    pub scrub: Option<f32>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Start GStreamer off the UI thread while the video downloads: the first
/// start loads (or rebuilds) its plugin registry, which can take seconds.
pub fn warm_up() {
    let _ = std::thread::Builder::new()
        .name("gstreamer-init".into())
        .spawn(|| {
            let _ = gst::init();
        });
}

impl Player {
    pub fn open(path: &Path) -> Result<Self, String> {
        gst::init().map_err(|e| e.to_string())?;
        let uri = gst::glib::filename_to_uri(path, None).map_err(|e| e.to_string())?;
        let playbin = gst::ElementFactory::make("playbin")
            .property("uri", uri.as_str())
            .build()
            .map_err(|_| "GStreamer's playbin is missing (gst-plugins-base)".to_owned())?;
        let (tx, events) = async_channel::bounded(4);
        let sink = gst_app::AppSink::builder()
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .format(gst_video::VideoFormat::Rgba)
                    .build(),
            )
            .max_buffers(2)
            .drop(true)
            .build();
        let frames = tx.clone();
        let paused = tx.clone();
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if let Some(frame) = rgba(&sample) {
                        // A frame the window hasn't drawn yet is dropped.
                        let _ = frames.try_send(Event::Frame(frame));
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                // Paused (and after a seek while paused) shows this frame.
                .new_preroll(move |sink| {
                    let sample = sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                    if let Some(frame) = rgba(&sample) {
                        let _ = paused.try_send(Event::Frame(frame));
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        playbin.set_property("video-sink", &sink);
        let stop = Arc::new(AtomicBool::new(false));
        let bus = playbin.bus().ok_or("no GStreamer bus")?;
        let done = stop.clone();
        std::thread::Builder::new()
            .name("media-viewer-bus".into())
            .spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(200)) else {
                        continue;
                    };
                    let event = match msg.view() {
                        gst::MessageView::Eos(_) => Event::Ended,
                        gst::MessageView::Error(e) => Event::Failed(e.error().to_string()),
                        _ => continue,
                    };
                    if tx.send_blocking(event).is_err() {
                        return;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        playbin
            .set_state(gst::State::Playing)
            .map_err(|_| "This video can't be played here.".to_owned())?;
        Ok(Self {
            playbin,
            feed: Feed {
                id: NEXT.fetch_add(1, Ordering::Relaxed),
                events,
            },
            stop,
            frame: None,
            position: Duration::ZERO,
            duration: None,
            playing: true,
            ended: false,
            muted: false,
            scrub: None,
        })
    }

    pub fn subscription(&self) -> iced::Subscription<Event> {
        iced::Subscription::run_with(self.feed.clone(), |feed| feed.events.clone())
    }

    /// Read the position and duration, as frames arrive.
    pub fn poll(&mut self) {
        if let Some(t) = self.playbin.query_position::<gst::ClockTime>() {
            self.position = Duration::from_nanos(t.nseconds());
        }
        if self.duration.is_none() {
            self.duration = self
                .playbin
                .query_duration::<gst::ClockTime>()
                .map(|t| Duration::from_nanos(t.nseconds()));
        }
    }

    pub fn control(&mut self, control: Control) {
        match control {
            Control::Toggle if self.ended => {
                self.ended = false;
                self.seek_to(Duration::ZERO);
                self.set_playing(true);
            }
            Control::Toggle => self.set_playing(!self.playing),
            Control::Seek(fraction) => {
                self.scrub = None;
                if let Some(total) = self.duration {
                    self.ended = false;
                    self.seek_to(total.mul_f32(fraction.clamp(0., 1.)));
                }
            }
            Control::Mute => {
                self.muted = !self.muted;
                self.playbin.set_property("mute", self.muted);
            }
        }
    }

    pub fn finished(&mut self) {
        self.ended = true;
        self.playing = false;
        let _ = self.playbin.set_state(gst::State::Paused);
        if let Some(total) = self.duration {
            self.position = total;
        }
    }

    fn set_playing(&mut self, playing: bool) {
        let state = if playing {
            gst::State::Playing
        } else {
            gst::State::Paused
        };
        if self.playbin.set_state(state).is_ok() {
            self.playing = playing;
        }
    }

    fn seek_to(&mut self, at: Duration) {
        let flags = gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE;
        if self
            .playbin
            .seek_simple(flags, gst::ClockTime::from_nseconds(at.as_nanos() as u64))
            .is_ok()
        {
            self.position = at;
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.playbin.set_state(gst::State::Null);
    }
}

/// A sample's picture as tightly packed RGBA.
fn rgba(sample: &gst::Sample) -> Option<Handle> {
    let info = gst_video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(sample.buffer()?, &info).ok()?;
    let (w, h) = (info.width(), info.height());
    let stride = usize::try_from(frame.plane_stride()[0]).ok()?;
    let data = frame.plane_data(0).ok()?;
    let row = w as usize * 4;
    let mut pixels = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        pixels.extend_from_slice(data.get(y * stride..y * stride + row)?);
    }
    Some(Handle::from_rgba(w, h, pixels))
}

/// "1:05", or "1:02:05" past an hour.
pub fn clock(t: Duration) -> String {
    let s = t.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A video's first frame (RGBA, its own size) and duration, for the poster
/// and `info` of a video being sent (ROADMAP P5).
pub struct Poster {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub duration: Option<Duration>,
}

/// Read a video's first frame and duration without playing it.
pub fn poster(path: &Path) -> Result<Poster, String> {
    gst::init().map_err(|e| e.to_string())?;
    let uri = gst::glib::filename_to_uri(path, None).map_err(|e| e.to_string())?;
    let sink = gst_app::AppSink::builder()
        .caps(
            &gst_video::VideoCapsBuilder::new()
                .format(gst_video::VideoFormat::Rgba)
                .build(),
        )
        .max_buffers(1)
        .build();
    let silent = gst::ElementFactory::make("fakesink")
        .build()
        .map_err(|e| e.to_string())?;
    let playbin = gst::ElementFactory::make("playbin")
        .property("uri", uri.as_str())
        .property("video-sink", &sink)
        .property("audio-sink", &silent)
        .build()
        .map_err(|_| "GStreamer's playbin is missing (gst-plugins-base)".to_owned())?;
    let stop = |e: String| {
        let _ = playbin.set_state(gst::State::Null);
        e
    };
    playbin
        .set_state(gst::State::Paused)
        .map_err(|_| stop("This video can't be read.".into()))?;
    let bus = playbin
        .bus()
        .ok_or_else(|| stop("no GStreamer bus".into()))?;
    let ready = bus.timed_pop_filtered(
        gst::ClockTime::from_seconds(10),
        &[gst::MessageType::AsyncDone, gst::MessageType::Error],
    );
    if !matches!(
        ready.as_ref().map(|m| m.view()),
        Some(gst::MessageView::AsyncDone(_))
    ) {
        return Err(stop("This video can't be read.".into()));
    }
    let sample = sink
        .pull_preroll()
        .map_err(|_| stop("This video has no picture.".into()))?;
    let duration = playbin
        .query_duration::<gst::ClockTime>()
        .map(|t| Duration::from_nanos(t.nseconds()));
    let frame = rgba(&sample).ok_or_else(|| stop("This video has no picture.".into()))?;
    let _ = playbin.set_state(gst::State::Null);
    match frame {
        Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } => Ok(Poster {
            width,
            height,
            pixels: pixels.to_vec(),
            duration,
        }),
        _ => Err("This video has no picture.".into()),
    }
}
