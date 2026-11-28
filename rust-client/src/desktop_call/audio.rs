//! A call's local audio. Device I/O goes through [`AudioIo`]; every remote
//! stream is mixed in-process into one output stream per call.
//!
//! Each remote track feeds a [`Source`]: a short jitter ring of 10 ms mono
//! frames. One mixer task takes a frame from every ring each 10 ms, applies
//! the listener's settings (per-user volume, Deafen, Play shared audio), sums
//! with clipping and writes the result to a single output, reopened only when
//! the speaker route changes. The frame that reached the speaker is handed to
//! the far-end tap, the echo canceller's reference.
use super::{AudioMode, DeviceNotices, Port, Result};
use crate::call_devices::Catalog;
use futures_util::{StreamExt, future::BoxFuture};
use livekit::{prelude::*, webrtc::audio_stream::native::NativeAudioStream};
use std::{
    collections::VecDeque,
    future::Future,
    process::Stdio,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Notify, watch},
    time,
};

pub(super) const RATE: u32 = 48_000;
/// 10 ms of mono audio.
pub(super) const SAMPLES: usize = RATE as usize / 100;
pub(super) type Frame = [i16; SAMPLES];
const SILENCE: Frame = [0; SAMPLES];
/// A source starts (and restarts after running dry) with this much queued,
/// so scheduling jitter between it and the mixer doesn't cut frames.
const PRIME: usize = 2 * SAMPLES;
/// Older audio is dropped beyond this, bounding the added latency.
const LIMIT: usize = 6 * SAMPLES;

/// The platform's audio devices: 48 kHz mono, 10 ms frames. `None` picks the
/// system default. Linux uses PulseAudio's command-line clients ([`Pulse`]),
/// since in-process PipeWire clashes with libwebrtc's embedded PipeWire
/// symbols (see `cameras.rs`).
pub(super) trait AudioIo: Send + Sync {
    fn open_input(&self, device: Option<&str>) -> Result<Box<dyn Input>>;
    fn open_output(&self, device: Option<&str>) -> Result<Box<dyn Output>>;
    fn list_devices(&self) -> BoxFuture<'static, Result<Catalog>>;
}
pub(super) trait Input: Send {
    /// Fill `frame` with the next 10 ms; false once the device stopped.
    fn read<'a>(&'a mut self, frame: &'a mut Frame) -> BoxFuture<'a, bool>;
}
pub(super) trait Output: Send {
    /// Queue 10 ms for playback; false once the device stopped.
    fn write<'a>(&'a mut self, frame: &'a Frame) -> BoxFuture<'a, bool>;
}

pub(super) fn platform() -> Arc<dyn AudioIo> {
    Arc::new(Pulse)
}

/// `pacat` children, killed when their stream is dropped.
struct Pulse;
impl Pulse {
    fn spawn(record: bool, device: Option<&str>) -> Result<Child> {
        let mut p = Command::new("pacat");
        p.args([
            if record { "--record" } else { "--playback" },
            "--raw",
            "--format=s16le",
            "--rate=48000",
            "--channels=1",
            "--latency-msec=40",
            "--client-name=Untethered",
            "--stream-name=Voice call",
        ])
        .kill_on_drop(true)
        .stderr(Stdio::null());
        if let Some(device) = device {
            p.arg("--device").arg(device);
        }
        if record {
            p.stdin(Stdio::null()).stdout(Stdio::piped());
        } else {
            p.stdin(Stdio::piped()).stdout(Stdio::null());
        }
        p.spawn()
            .map_err(|_| "Cannot open audio. Install the PulseAudio client tools.")
    }
}
impl AudioIo for Pulse {
    fn open_input(&self, device: Option<&str>) -> Result<Box<dyn Input>> {
        let mut child = Self::spawn(true, device)?;
        let pipe = child.stdout.take().ok_or("Cannot open audio.")?;
        Ok(Box::new(PacatInput {
            _child: child,
            pipe,
            bytes: [0; SAMPLES * 2],
        }))
    }
    fn open_output(&self, device: Option<&str>) -> Result<Box<dyn Output>> {
        let mut child = Self::spawn(false, device)?;
        let pipe = child.stdin.take().ok_or("Cannot open audio.")?;
        bound(&pipe);
        Ok(Box::new(PacatOutput {
            _child: child,
            pipe,
            bytes: [0; SAMPLES * 2],
        }))
    }
    fn list_devices(&self) -> BoxFuture<'static, Result<Catalog>> {
        Box::pin(Catalog::read())
    }
}
/// Let at most a page (about 43 ms of audio) wait in the pipe to a playback
/// `pacat`, so `pacat` paces the mixer. The mixer runs on a timer: whatever
/// it wrote while `pacat` started, or in a burst after a stall, would
/// otherwise wait in the 64 KiB pipe for good, up to 680 ms of delay that a
/// listener hears and the echo canceller has to find between its reference
/// and the microphone.
fn bound(pipe: &ChildStdin) {
    use std::os::fd::AsRawFd;
    // SAFETY: fcntl on a pipe descriptor this process owns; on failure the
    // pipe keeps its default size.
    unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETPIPE_SZ, 4096) };
}
struct PacatInput {
    _child: Child,
    pipe: ChildStdout,
    bytes: [u8; SAMPLES * 2],
}
impl Input for PacatInput {
    fn read<'a>(&'a mut self, frame: &'a mut Frame) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            if self.pipe.read_exact(&mut self.bytes).await.is_err() {
                return false;
            }
            for (sample, bytes) in frame.iter_mut().zip(self.bytes.as_chunks::<2>().0) {
                *sample = i16::from_le_bytes(*bytes);
            }
            true
        })
    }
}
struct PacatOutput {
    _child: Child,
    pipe: ChildStdin,
    bytes: [u8; SAMPLES * 2],
}
impl Output for PacatOutput {
    fn write<'a>(&'a mut self, frame: &'a Frame) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            for (bytes, sample) in self.bytes.as_chunks_mut::<2>().0.iter_mut().zip(frame) {
                *bytes = sample.to_le_bytes();
            }
            self.pipe.write_all(&self.bytes).await.is_ok()
        })
    }
}

/// What a remote track carries, for the listener's settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Voice,
    /// Muted by unchecking Play shared audio.
    Share,
}
impl From<TrackSource> for Kind {
    fn from(source: TrackSource) -> Self {
        if source == TrackSource::ScreenshareAudio {
            Self::Share
        } else {
            Self::Voice
        }
    }
}

#[derive(Default)]
struct Ring {
    samples: VecDeque<i16>,
    primed: bool,
}
impl Ring {
    fn push(&mut self, samples: &[i16]) {
        self.samples.extend(samples);
        let excess = self.samples.len().saturating_sub(LIMIT);
        self.samples.drain(..excess);
    }
    fn pop(&mut self, frame: &mut Frame) -> bool {
        if self.samples.len() < if self.primed { SAMPLES } else { PRIME } {
            self.primed = false;
            return false;
        }
        self.primed = true;
        for (out, sample) in frame.iter_mut().zip(self.samples.drain(..SAMPLES)) {
            *out = sample;
        }
        true
    }
}

struct Entry {
    id: u64,
    kind: Kind,
    // Volume follows the Matrix user, so a replaced track keeps it.
    user: Option<String>,
    ring: Ring,
}
#[derive(Default)]
struct Sources {
    next: u64,
    list: Vec<Entry>,
}
impl Sources {
    /// Mix one frame from every source. Every ring is drained at the same
    /// pace, including muted ones, so unmuting resumes with current audio.
    fn mix(&mut self, mode: &AudioMode, out: &mut Frame) {
        let mut sum = [0i32; SAMPLES];
        let mut frame = SILENCE;
        for entry in &mut self.list {
            if !entry.ring.pop(&mut frame) {
                continue;
            }
            let gain = i32::from(gain(mode, entry.kind, entry.user.as_deref()));
            if gain == 0 {
                continue;
            }
            for (sum, &sample) in sum.iter_mut().zip(&frame) {
                *sum += i32::from(sample) * gain / 100;
            }
        }
        for (out, sum) in out.iter_mut().zip(sum) {
            *out = clip(sum);
        }
    }
}
fn gain(mode: &AudioMode, kind: Kind, user: Option<&str>) -> u8 {
    if mode.deafened || kind == Kind::Share && mode.share_silent {
        0
    } else {
        user.map_or(100, |u| mode.volume(u))
    }
}
/// Hard clip: an overdriven peak flattens instead of wrapping around.
fn clip(sample: i32) -> i16 {
    sample.clamp(i16::MIN.into(), i16::MAX.into()) as i16
}

type FarEnd = Box<dyn FnMut(&Frame) + Send>;
struct Shared {
    sources: Mutex<Sources>,
    added: Notify,
    far_end: Mutex<Option<FarEnd>>,
}
impl Shared {
    fn sources(&self) -> std::sync::MutexGuard<'_, Sources> {
        self.sources.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One call's playback. Dropping it stops the mixer and closes the output.
pub(super) struct Mixer {
    shared: Arc<Shared>,
    task: tokio::task::AbortHandle,
}
impl Drop for Mixer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Mixer {
    pub(super) fn start(
        io: Arc<dyn AudioIo>,
        mode: watch::Receiver<AudioMode>,
        notices: DeviceNotices,
    ) -> Self {
        let shared = Arc::new(Shared {
            sources: Mutex::default(),
            added: Notify::new(),
            far_end: Mutex::default(),
        });
        let task = tokio::spawn(run(shared.clone(), io, mode, notices)).abort_handle();
        Self { shared, task }
    }
    /// Register a source now; it plays until the returned handle is dropped.
    pub(super) fn add(&self, kind: Kind, user: Option<String>) -> Source {
        let mut sources = self.shared.sources();
        let id = sources.next;
        sources.next += 1;
        sources.list.push(Entry {
            id,
            kind,
            user,
            ring: Ring::default(),
        });
        drop(sources);
        self.shared.added.notify_one();
        Source {
            shared: self.shared.clone(),
            id,
        }
    }
    /// Play a remote track until it ends or the returned future is dropped.
    pub(super) fn play(
        &self,
        track: RemoteAudioTrack,
        kind: Kind,
        user: Option<String>,
    ) -> impl Future<Output = Result<()>> + Send + 'static {
        let source = self.add(kind, user);
        async move {
            let mut stream = NativeAudioStream::new(track.rtc_track(), RATE as i32, 1);
            while let Some(frame) = stream.next().await {
                source.push(&frame.data);
            }
            Ok(())
        }
    }
    /// Receive every mixed frame as it is played, silence included, for the
    /// echo canceller's far-end reference.
    pub(super) fn far_end(&self, tap: impl FnMut(&Frame) + Send + 'static) {
        *self
            .shared
            .far_end
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(tap));
    }
}

pub(super) struct Source {
    shared: Arc<Shared>,
    id: u64,
}
impl Source {
    pub(super) fn push(&self, samples: &[i16]) {
        if let Some(entry) = self
            .shared
            .sources()
            .list
            .iter_mut()
            .find(|e| e.id == self.id)
        {
            entry.ring.push(samples);
        }
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.shared.sources().list.retain(|e| e.id != self.id);
    }
}

/// The output is open while at least one source is registered, so a call
/// that is ringing, reconnecting or alone in a room holds no playback stream.
async fn run(
    shared: Arc<Shared>,
    io: Arc<dyn AudioIo>,
    mut mode: watch::Receiver<AudioMode>,
    notices: DeviceNotices,
) {
    let mut tick = time::interval(Duration::from_millis(10));
    let mut output: Option<(u64, Box<dyn Output>)> = None;
    // Like a single failed stream before, a failed speaker stays silent
    // until the user picks a route again or playback restarts.
    let mut failed = None;
    let mut frame = SILENCE;
    loop {
        if shared.sources().list.is_empty() {
            output = None;
            failed = None;
            shared.added.notified().await;
            tick.reset();
            continue;
        }
        tick.tick().await;
        let route = {
            let mut sources = shared.sources();
            let mode = mode.borrow();
            sources.mix(&mode, &mut frame);
            mode.speaker.clone()
        };
        if output.as_ref().is_some_and(|(r, _)| *r != route.revision) {
            output = None;
        }
        if output.is_none() && failed != Some(route.revision) {
            match io.open_output(route.name.as_deref()) {
                Ok(out) => output = Some((route.revision, out)),
                Err(_) => {
                    failed = Some(route.revision);
                    notices.report(Port::Speaker, route.revision, true).await;
                }
            }
        }
        let mut played = false;
        if let Some((revision, out)) = &mut output {
            let revision = *revision;
            let written = tokio::select! {
                biased;
                changed = mode.wait_for(|s| s.speaker.revision != revision) => {
                    if changed.is_err() {
                        return;
                    }
                    None
                }
                written = out.write(&frame) => Some(written),
            };
            match written {
                Some(true) => played = true,
                Some(false) => {
                    output = None;
                    failed = Some(revision);
                    notices.report(Port::Speaker, revision, true).await;
                }
                None => output = None,
            }
        }
        if let Some(tap) = shared
            .far_end
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            tap(if played { &frame } else { &SILENCE });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_devices::Route;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn mode() -> AudioMode {
        AudioMode::default()
    }
    fn mixed(sources: &mut Sources, mode: &AudioMode) -> Frame {
        let mut out = SILENCE;
        sources.mix(mode, &mut out);
        out
    }
    fn source(sources: &mut Sources, kind: Kind, user: Option<&str>, samples: &[i16]) {
        let mut ring = Ring::default();
        for _ in 0..2 {
            let mut frame = SILENCE;
            frame[..samples.len()].copy_from_slice(samples);
            ring.push(&frame);
        }
        sources.list.push(Entry {
            id: sources.list.len() as u64,
            kind,
            user: user.map(str::to_owned),
            ring,
        });
    }

    #[test]
    fn boosted_playback_clips_peaks_without_inverting_them() {
        let mut mode = mode();
        mode.set_volume("@b:x".into(), 200);
        let mut sources = Sources::default();
        let peaks = [-20_000, -10_000, 0, 10_000, 20_000];
        source(&mut sources, Kind::Voice, Some("@b:x"), &peaks);
        let out = mixed(&mut sources, &mode);
        assert_eq!(out[..5], [i16::MIN, -20_000, 0, 20_000, i16::MAX]);
    }

    #[test]
    fn sources_sum_with_their_own_gain_and_clip_together() {
        let mut mode = mode();
        mode.set_volume("@b:x".into(), 25);
        mode.set_volume("@c:x".into(), 150);
        let mut sources = Sources::default();
        source(&mut sources, Kind::Voice, Some("@b:x"), &[8_000, 30_000]);
        source(&mut sources, Kind::Voice, Some("@c:x"), &[1_000, 30_000]);
        source(&mut sources, Kind::Share, Some("@c:x"), &[-100, 0]);
        let out = mixed(&mut sources, &mode);
        assert_eq!(out[..2], [2_000 + 1_500 - 150, i16::MAX]);
        // The other frame queued in each ring follows the same rules.
        mode.share_silent = true;
        assert_eq!(mixed(&mut sources, &mode)[0], 2_000 + 1_500);
    }

    #[test]
    fn deafen_and_zero_volume_still_drain_the_rings() {
        let mut mode = mode();
        mode.deafened = true;
        let mut sources = Sources::default();
        source(&mut sources, Kind::Voice, None, &[5_000]);
        assert_eq!(mixed(&mut sources, &mode), SILENCE);
        mode.deafened = false;
        assert_eq!(mixed(&mut sources, &mode)[0], 5_000);
        // Both queued frames are used up: no stale speech after undeafening.
        assert_eq!(mixed(&mut sources, &mode), SILENCE);
    }

    #[test]
    fn ring_primes_before_playing_and_bounds_its_latency() {
        let mut ring = Ring::default();
        let mut frame = SILENCE;
        ring.push(&[1; SAMPLES]);
        assert!(!ring.pop(&mut frame), "one frame is not enough to start");
        ring.push(&[2; SAMPLES]);
        assert!(ring.pop(&mut frame) && frame[0] == 1);
        assert!(ring.pop(&mut frame) && frame[0] == 2);
        // Ran dry: wait for two frames again instead of playing every other one.
        assert!(!ring.pop(&mut frame));
        ring.push(&[3; SAMPLES]);
        assert!(!ring.pop(&mut frame));
        for n in 4..20 {
            ring.push(&[n; SAMPLES]);
        }
        assert_eq!(ring.samples.len(), LIMIT);
        assert!(
            ring.pop(&mut frame) && frame[0] == 14,
            "oldest audio dropped"
        );
    }

    /// Records what reaches each opened output.
    #[derive(Default)]
    struct Fake {
        opened: Mutex<Vec<Option<String>>>,
        live: Arc<AtomicUsize>,
        played: Arc<Mutex<Vec<i16>>>,
    }
    struct FakeOutput {
        live: Arc<AtomicUsize>,
        played: Arc<Mutex<Vec<i16>>>,
    }
    impl Drop for FakeOutput {
        fn drop(&mut self) {
            self.live.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl Output for FakeOutput {
        fn write<'a>(&'a mut self, frame: &'a Frame) -> BoxFuture<'a, bool> {
            self.played.lock().unwrap().push(frame[0]);
            Box::pin(async { true })
        }
    }
    impl AudioIo for Fake {
        fn open_input(&self, _: Option<&str>) -> Result<Box<dyn Input>> {
            Err("no input")
        }
        fn open_output(&self, device: Option<&str>) -> Result<Box<dyn Output>> {
            self.opened.lock().unwrap().push(device.map(str::to_owned));
            self.live.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeOutput {
                live: self.live.clone(),
                played: self.played.clone(),
            }))
        }
        fn list_devices(&self) -> BoxFuture<'static, Result<Catalog>> {
            Box::pin(async { Ok(Catalog::default()) })
        }
    }
    async fn until(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..200 {
            if ok() {
                return;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn one_output_follows_sources_and_reopens_only_on_a_speaker_change() {
        let fake = Arc::new(Fake::default());
        let (audio, mode) = watch::channel(AudioMode::default());
        let (events, _rx) = async_channel::unbounded();
        let notices = DeviceNotices {
            events,
            room: "!r".into(),
        };
        let mixer = Mixer::start(fake.clone(), mode, notices);
        let heard = Arc::new(Mutex::new(Vec::new()));
        let tap = heard.clone();
        mixer.far_end(move |f| tap.lock().unwrap().push(f[0]));
        let live = || fake.live.load(Ordering::SeqCst);
        let opened = || fake.opened.lock().unwrap().clone();

        // Nothing registered: no playback stream, as while a DM rings.
        time::sleep(Duration::from_millis(50)).await;
        assert_eq!(live(), 0);

        let voice = mixer.add(Kind::Voice, Some("@b:x".into()));
        let share = mixer.add(Kind::Share, Some("@b:x".into()));
        for _ in 0..4 {
            voice.push(&[1_000; SAMPLES]);
            share.push(&[200; SAMPLES]);
        }
        until("mixed audio", || {
            fake.played.lock().unwrap().contains(&1_200)
        })
        .await;
        until("the reference", || heard.lock().unwrap().contains(&1_200)).await;
        assert_eq!((live(), opened()), (1, vec![None]));

        audio.send_modify(|s| {
            s.speaker = Route {
                name: Some("Grove".into()),
                revision: 1,
            }
        });
        until("the new speaker", || opened().len() == 2).await;
        assert_eq!(opened()[1].as_deref(), Some("Grove"));
        assert_eq!(live(), 1, "the old output closed");

        drop(voice);
        time::sleep(Duration::from_millis(50)).await;
        assert_eq!((live(), opened().len()), (1, 2), "one source still plays");
        drop(share);
        until("output closed", || live() == 0).await;
        drop(mixer);
    }
}
