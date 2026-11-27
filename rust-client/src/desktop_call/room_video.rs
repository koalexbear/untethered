//! Group camera and screen/window video use distinct portal grants and tracks.
use super::*;
use ashpd::desktop::{
    PersistMode, Session as PortalSession,
    camera::Camera,
    screencast::{CursorMode, Screencast, SourceType},
};
use ashpd::{zbus, zvariant};
use iced::advanced::image::{self, Allocation};
use iced::widget::image::Handle;
use livekit::options::VideoEncoding;
use livekit::webrtc::{
    prelude::{
        I420Buffer, RtcVideoSource, VideoBuffer, VideoFormatType, VideoFrame, VideoResolution,
        VideoRotation,
    },
    video_source::native::NativeVideoSource,
    video_stream::native::NativeVideoStream,
};
use std::{
    collections::BTreeMap,
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
};
use tokio::process::{Child, ChildStdout};
use uuid::Uuid;

/// Upper bounds for one published video track. A capture never upscales its
/// source past `width`×`height`; Go Live (U4) will choose these per share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Quality {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub max_bitrate: u64,
}
impl Quality {
    pub(crate) const SCREEN: Self = Self {
        width: 1920,
        height: 1080,
        fps: 30,
        max_bitrate: 2_500_000,
    };
    pub(crate) const CAMERA: Self = Self {
        width: 1280,
        height: 720,
        fps: 30,
        max_bitrate: 1_700_000,
    };
    fn encoding(self) -> VideoEncoding {
        VideoEncoding {
            max_bitrate: self.max_bitrate,
            max_framerate: f64::from(self.fps),
        }
    }
    /// Fit a source into these bounds, keeping its aspect ratio. I420 needs
    /// even sides. An unknown source size uses the full bounds (letterboxed).
    fn fit(self, source: Option<(i32, i32)>) -> (u32, u32) {
        let (w, h) = source
            .filter(|&(w, h)| w > 0 && h > 0)
            .map_or((self.width, self.height), |(w, h)| (w as u32, h as u32));
        let scale = (f64::from(self.width) / f64::from(w))
            .min(f64::from(self.height) / f64::from(h))
            .min(1.);
        let even = |v: u32| ((f64::from(v) * scale) as u32 & !1).max(2);
        (even(w), even(h))
    }
}
/// Largest frame a receiver converts for display, and its fastest redraw.
const SHOW_W: u32 = 1920;
const SHOW_H: u32 = 1080;
const SHOW_EVERY: Duration = Duration::from_millis(33);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum VideoInput {
    Window,
    Camera,
}
impl VideoInput {
    pub(crate) const ALL: [Self; 2] = [Self::Window, Self::Camera];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Window => "Shared window",
            Self::Camera => "Camera",
        }
    }
    pub(crate) fn from_name(name: Option<&str>) -> Option<Self> {
        Self::ALL.into_iter().find(|i| Some(i.name()) == name)
    }
    pub(crate) fn starting(self) -> &'static str {
        match self {
            Self::Window => "Choose a screen or window in the desktop picker…",
            Self::Camera => "Opening camera…",
        }
    }
    pub(crate) fn control(self, requested: bool, live: bool) -> &'static str {
        match (self, requested, live) {
            (Self::Window, false, _) => "Share screen",
            (Self::Window, true, true) => "Stop sharing",
            (Self::Window, true, false) => "Cancel share",
            (Self::Camera, false, _) => "Start camera",
            (Self::Camera, true, true) => "Stop camera",
            (Self::Camera, true, false) => "Cancel camera",
        }
    }
    fn source(self) -> TrackSource {
        match self {
            Self::Window => TrackSource::Screenshare,
            Self::Camera => TrackSource::Camera,
        }
    }
    fn request(self, mode: &AudioMode) -> Option<Uuid> {
        mode.video[self as usize]
    }
    fn default_quality(self) -> Quality {
        match self {
            Self::Window => Quality::SCREEN,
            Self::Camera => Quality::CAMERA,
        }
    }
}
#[derive(Default)]
pub(crate) struct Status {
    pub label: String,
    pub live: bool,
}

/// How long an upload may stay unanswered before the frame is asked again
/// (the renderer drops the request while the window is closed).
const UPLOAD_PATIENCE: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub(crate) struct Feed {
    pub sid: String,
    pub user: String,
    pub source: VideoInput,
    /// The newest frame decoded or captured.
    frame: Handle,
    /// The newest frame already on the GPU: the one tiles draw. iced uploads
    /// images of 2 MiB or more on a worker and draws nothing until they're
    /// there, so a tile drawing each new frame at once flashes empty.
    shown: Option<Allocation>,
    /// The frame being uploaded, and since when; one at a time, so a slow
    /// upload skips frames instead of queueing them.
    uploading: Option<(image::Id, Instant)>,
}
impl Feed {
    /// What the tile draws: the newest uploaded frame, if any is yet.
    pub(crate) fn picture(&self) -> Option<&Handle> {
        self.shown.as_ref().map(Allocation::handle)
    }
    fn waiting(&self) -> bool {
        self.picture().is_none_or(|h| h.id() != self.frame.id())
    }
}
#[derive(Default)]
struct Wall {
    feeds: BTreeMap<String, Feed>,
    focus: Option<String>,
}
#[derive(Clone, Default)]
pub(crate) struct Previews(Arc<Mutex<Wall>>);
impl Previews {
    pub fn remote_share(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .feeds
            .values()
            .any(|f| f.source == VideoInput::Window && !f.user.is_empty())
    }
    pub(crate) fn snapshot(&self) -> (Option<String>, Vec<Feed>) {
        let wall = self.0.lock().unwrap();
        (wall.focus.clone(), wall.feeds.values().cloned().collect())
    }
    /// The feed the call dock previews while the stage is closed: the
    /// focused one, else a screen share, else a remote camera.
    pub(crate) fn docked(&self) -> Option<Feed> {
        let (focus, feeds) = self.snapshot();
        feeds
            .into_iter()
            .filter(|f| !f.user.is_empty() || f.source == VideoInput::Window)
            .min_by(|a, b| {
                let priority = |f: &Feed| {
                    (
                        Some(&f.sid) != focus.as_ref(),
                        f.source != VideoInput::Window,
                    )
                };
                priority(a)
                    .cmp(&priority(b))
                    .then_with(|| a.user.cmp(&b.user))
                    .then_with(|| a.sid.cmp(&b.sid))
            })
    }
    pub(crate) fn focus(&self, sid: Option<String>) {
        let mut wall = self.0.lock().unwrap();
        if sid.as_ref().is_none_or(|s| wall.feeds.contains_key(s)) {
            wall.focus = sid;
        }
    }
    /// Newest frames to upload for the feeds on screen (`only` one, or all),
    /// each feed's next once its last upload has answered.
    pub(crate) fn uploads(&self, only: Option<&str>) -> Vec<(String, Handle)> {
        let mut wall = self.0.lock().unwrap();
        let now = Instant::now();
        wall.feeds
            .values_mut()
            .filter(|f| only.is_none_or(|sid| sid == f.sid))
            .filter(|f| f.waiting())
            .filter(|f| {
                f.uploading
                    .is_none_or(|(_, since)| now.duration_since(since) >= UPLOAD_PATIENCE)
            })
            .map(|f| {
                f.uploading = Some((f.frame.id(), now));
                (f.sid.clone(), f.frame.clone())
            })
            .collect()
    }
    /// A frame is on the GPU: tiles draw it from now on.
    pub(crate) fn uploaded(&self, sid: &str, frame: std::result::Result<Allocation, image::Error>) {
        let mut wall = self.0.lock().unwrap();
        let Some(feed) = wall.feeds.get_mut(sid) else {
            return;
        };
        let Ok(frame) = frame else {
            feed.uploading = None;
            return;
        };
        // An answer to a request given up on is older than what's asked now.
        let current = feed
            .uploading
            .is_some_and(|(id, _)| id == frame.handle().id());
        if current {
            feed.uploading = None;
        }
        if current || feed.shown.is_none() {
            feed.shown = Some(frame);
        }
    }
    /// Keep a feed's newest frame. True when the UI should look at it now:
    /// while an upload is out, its answer picks up the newest frame instead,
    /// so a steady stream costs the UI one update per frame, not two.
    fn frame(&self, sid: &str, user: &str, source: VideoInput, frame: Handle) -> bool {
        let mut wall = self.0.lock().unwrap();
        if let Some(feed) = wall.feeds.get_mut(sid) {
            feed.frame = frame;
            feed.uploading
                .is_none_or(|(_, since)| since.elapsed() >= UPLOAD_PATIENCE)
        } else {
            if source == VideoInput::Window {
                wall.focus = Some(sid.into());
            }
            wall.feeds.insert(
                sid.into(),
                Feed {
                    sid: sid.into(),
                    user: user.into(),
                    source,
                    frame,
                    shown: None,
                    uploading: None,
                },
            );
            true
        }
    }
}
struct Tile {
    sid: String,
    views: Previews,
    events: Sender<Value>,
}
impl Drop for Tile {
    fn drop(&mut self) {
        let mut wall = self.views.0.lock().unwrap();
        wall.feeds.remove(&self.sid);
        if wall.focus.as_ref() == Some(&self.sid) {
            wall.focus = None;
        }
        drop(wall);
        let _ = self.events.try_send(json!({"command":"video_redraw"}));
    }
}
pub(super) async fn display(
    track: RemoteVideoTrack,
    user: String,
    source: VideoInput,
    views: Previews,
    events: Sender<Value>,
) -> Result<()> {
    let tile = Tile {
        sid: track.sid().to_string(),
        views,
        events,
    };
    let mut stream = NativeVideoStream::new(track.rtc_track());
    // The stream holds only the newest decoded frame. Waking at most every
    // SHOW_EVERY and then waiting for that frame follows the publisher's own
    // rate up to 30 fps without queueing stale frames.
    let mut tick = time::interval(SHOW_EVERY);
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    #[cfg(debug_assertions)]
    let mut probe = trace_video_enabled().then(|| RenderProbe::new(&tile.sid));
    loop {
        tick.tick().await;
        let Some(frame) = stream.next().await else {
            return Ok(());
        };
        let (w, h) = (frame.buffer.width(), frame.buffer.height());
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 8_294_400 {
            continue;
        }
        #[cfg(debug_assertions)]
        let started = Instant::now();
        let frame = rgba(&mut frame.buffer.to_i420(), SHOW_W, SHOW_H);
        #[cfg(debug_assertions)]
        if let Some(probe) = &mut probe {
            probe.frame(started.elapsed(), w, h);
        }
        if tile.views.frame(&tile.sid, &user, source, frame) {
            let _ = tile.events.try_send(json!({"command":"video_redraw"}));
        }
    }
}

/// Frames handed to the UI per remote track, and the CPU conversion cost of
/// each (I420 copy, scale, RGBA and the new `image::Handle`).
#[cfg(debug_assertions)]
struct RenderProbe {
    sid: String,
    since: Instant,
    frames: u32,
    busy: Duration,
}
#[cfg(debug_assertions)]
impl RenderProbe {
    fn new(sid: &str) -> Self {
        Self {
            sid: sid.into(),
            since: Instant::now(),
            frames: 0,
            busy: Duration::ZERO,
        }
    }
    /// Frames read from the capture pipe, after GStreamer's rate limit.
    fn capture(&mut self, w: u32, h: u32) {
        self.frames += 1;
        let elapsed = self.since.elapsed();
        if elapsed >= SECOND * 4 {
            eprintln!(
                "RTC video capture source={:?} width={w} height={h} fps={:.1}",
                self.sid,
                f64::from(self.frames) / elapsed.as_secs_f64(),
            );
            *self = Self::new(&self.sid);
        }
    }
    fn frame(&mut self, busy: Duration, w: u32, h: u32) {
        self.frames += 1;
        self.busy += busy;
        let elapsed = self.since.elapsed();
        if elapsed >= SECOND * 4 {
            eprintln!(
                "RTC video render sid={} width={w} height={h} fps={:.1} convert_ms={:.2}",
                self.sid,
                f64::from(self.frames) / elapsed.as_secs_f64(),
                self.busy.as_secs_f64() * 1000. / f64::from(self.frames),
            );
            *self = Self::new(&self.sid);
        }
    }
}

/// Convert to an RGBA image no larger than `max_w`×`max_h`, never upscaling.
fn rgba(buf: &mut I420Buffer, max_w: u32, max_h: u32) -> Handle {
    use livekit::webrtc::video_frame::native::VideoFrameBufferExt;
    let (w, h) = (buf.width(), buf.height());
    let scale = (f64::from(max_w) / f64::from(w))
        .min(f64::from(max_h) / f64::from(h))
        .min(1.);
    let (ow, oh) = (
        (f64::from(w) * scale).round().max(1.) as u32,
        (f64::from(h) * scale).round().max(1.) as u32,
    );
    let mut pixels = vec![0; (ow * oh * 4) as usize];
    // libyuv's ABGR is R, G, B, A in memory.
    if (ow, oh) == (w, h) {
        buf.to_argb(
            VideoFormatType::ABGR,
            &mut pixels,
            ow * 4,
            ow as i32,
            oh as i32,
        );
    } else {
        buf.scale(ow as i32, oh as i32).to_argb(
            VideoFormatType::ABGR,
            &mut pixels,
            ow * 4,
            ow as i32,
            oh as i32,
        );
    }
    Handle::from_rgba(ow, oh, pixels)
}

type Session = PortalSession<'static, Screencast<'static>>;
struct Lease(Option<Session>);
impl Lease {
    async fn close(&mut self) {
        if let Some(session) = &self.0 {
            let _ = session.close().await;
            self.0 = None;
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(session) = self.0.take() {
            tokio::spawn(async move {
                let _ = session.close().await;
            });
        }
    }
}

// Camera.AccessCamera has a Request but no Session. Keep its handle so cancelling
// from the call bar (or leaving voice) also dismisses the desktop prompt.
struct PermissionPrompt(Option<zbus::Proxy<'static>>);
impl Drop for PermissionPrompt {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            tokio::spawn(async move {
                let _: zbus::Result<()> = p.call("Close", &()).await;
            });
        }
    }
}
async fn authorize_camera(portal: &Camera<'_>) -> Result<()> {
    let conn = portal.connection();
    let sender = conn
        .unique_name()
        .ok_or("Camera portal disconnected.")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let token = format!("untethered_{}", Uuid::new_v4().simple());
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let proxy = zbus::Proxy::new_owned(
        conn.clone(),
        "org.freedesktop.portal.Desktop",
        path.clone(),
        "org.freedesktop.portal.Request",
    )
    .await
    .map_err(|_| "Camera permission request unavailable.")?;
    let mut response = proxy
        .receive_signal("Response")
        .await
        .map_err(|_| "Cannot receive camera permission.")?;
    let mut prompt = PermissionPrompt(Some(proxy));
    let options =
        std::collections::HashMap::from([("handle_token", zvariant::Value::from(token.as_str()))]);
    let handle: zvariant::OwnedObjectPath = portal
        .call("AccessCamera", &(options,))
        .await
        .map_err(|_| "Camera permission request failed.")?;
    if handle.as_str() != path {
        return Err("Unexpected camera permission handle.");
    }
    let msg = response
        .next()
        .await
        .ok_or("Camera permission request closed.")?;
    let (code, _): (u32, std::collections::HashMap<String, zvariant::OwnedValue>) = msg
        .body()
        .deserialize()
        .map_err(|_| "Invalid camera permission response.")?;
    prompt.0 = None;
    if code != 0 {
        return Err("Camera access cancelled or denied.");
    }
    Ok(())
}
/// One I420 frame as GStreamer writes it to `fdsink`. Its default video
/// layout pads every row to 4 bytes, so a width that isn't a multiple of 8
/// has wider luma or chroma rows than pixels. Read as packed planes, such a
/// frame shears and runs into the next one: colour errors and tearing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    width: u32,
    height: u32,
    stride_y: u32,
    stride_uv: u32,
}
impl Layout {
    /// Sides are even, as `Quality::fit` and the camera size make them.
    fn i420(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            stride_y: width.next_multiple_of(4),
            stride_uv: (width / 2).next_multiple_of(4),
        }
    }
    fn luma(self) -> usize {
        (self.stride_y * self.height) as usize
    }
    fn chroma(self) -> usize {
        (self.stride_uv * self.height / 2) as usize
    }
    fn size(self) -> usize {
        self.luma() + 2 * self.chroma()
    }
    /// A WebRTC buffer with GStreamer's strides, so each plane copies whole.
    fn buffer(self, frame: &[u8]) -> I420Buffer {
        let (luma, chroma) = (self.luma(), self.chroma());
        let mut buf = I420Buffer::with_strides(
            self.width,
            self.height,
            self.stride_y,
            self.stride_uv,
            self.stride_uv,
        );
        let (y, u, v) = buf.data_mut();
        y.copy_from_slice(&frame[..luma]);
        u.copy_from_slice(&frame[luma..luma + chroma]);
        v.copy_from_slice(&frame[luma + chroma..]);
        buf
    }
}
pub(super) struct Capture {
    route: CameraChoice,
    quality: Quality,
    lease: Option<Lease>,
    child: Child,
    pipe: ChildStdout,
    /// Even I420 frame size chosen when the source opened. A source that
    /// later resizes is letterboxed into it by `videoscale add-borders`.
    layout: Layout,
    pixels: Vec<u8>,
}
impl Capture {
    async fn open(input: VideoInput, route: CameraChoice, quality: Quality) -> Result<Self> {
        let (fd, node, lease, size) = match input {
            VideoInput::Window => {
                let (fd, node, lease, size) = Self::screen().await?;
                (fd, Some(node), Some(lease), quality.fit(size))
            }
            VideoInput::Camera => (
                camera_remote().await?,
                None,
                None,
                (quality.width & !1, quality.height & !1),
            ),
        };
        Self::reader(fd, node, lease, route, quality, size).await
    }
    async fn screen() -> Result<(OwnedFd, u32, Lease, Option<(i32, i32)>)> {
        crate::app_id::portal_ready().await;
        let proxy = Screencast::new()
            .await
            .map_err(|_| "Screen capture portal unavailable.")?;
        let wanted = SourceType::Window | SourceType::Monitor;
        let types = match proxy.available_source_types().await {
            Ok(offered) if offered.intersects(wanted) => offered & wanted,
            _ => wanted,
        };
        let cursor = match proxy.available_cursor_modes().await {
            Ok(modes) if modes.contains(CursorMode::Embedded) => CursorMode::Embedded,
            _ => CursorMode::Hidden,
        };
        let lease = Lease(Some(
            proxy
                .create_session()
                .await
                .map_err(|_| "Could not open the screen picker.")?,
        ));
        let session = lease.0.as_ref().unwrap();
        proxy
            .select_sources(session, cursor, types, false, None, PersistMode::DoNot)
            .await
            .and_then(|r| r.response())
            .map_err(|_| "The desktop could not select a screen or window.")?;
        let selection = proxy
            .start(session, None)
            .await
            .and_then(|r| r.response())
            .map_err(|_| "Screen sharing was cancelled or denied.")?;
        let [stream] = selection.streams() else {
            return Err("Select exactly one screen or window.");
        };
        if stream.source_type().is_some_and(|t| !wanted.contains(t)) {
            return Err("The selection was not a screen or window.");
        }
        let node = stream.pipe_wire_node_id();
        let mut size = stream.size();
        if size.is_none()
            && let Ok(fd) = proxy.open_pipe_wire_remote(session).await
        {
            size = probe_size(fd, node).await;
        }
        #[cfg(debug_assertions)]
        if trace_video_enabled() {
            eprintln!(
                "RTC video portal source={:?} size={:?} measured={size:?}",
                stream.source_type(),
                stream.size(),
            );
        }
        let fd = proxy
            .open_pipe_wire_remote(session)
            .await
            .map_err(|_| "Could not open the selected PipeWire stream.")?;
        Ok((fd, node, lease, size))
    }
    async fn reader(
        fd: OwnedFd,
        node: Option<u32>,
        lease: Option<Lease>,
        route: CameraChoice,
        quality: Quality,
        (width, height): (u32, u32),
    ) -> Result<Self> {
        // Stdio maps the already-authorized FD to descriptor 0 in the child;
        // no inheritance of arbitrary FDs, temporary recording or shell needed.
        let mut pipeline = Command::new("gst-launch-1.0");
        pipeline.args(["-q", "pipewiresrc", "fd=0"]);
        if let Some(node) = node {
            pipeline.args([format!("path={node}"), "keepalive-time=300".into()]);
        } else if let CameraChoice::Source { serial, .. } = &route {
            pipeline.args([format!("target-object={serial}"),"stream-properties=props,media.role=Camera,node.dont-fallback=true,node.dont-reconnect=true,node.name=untethered-camera-capture".into()]);
        } else {
            // Autoconnect on the camera-only portal remote; the media session
            // manager chooses the camera. Never fall back to a host device.
            pipeline.arg(
                "stream-properties=props,media.role=Camera,node.name=untethered-camera-capture",
            );
        }
        let fps = quality.fps.max(1);
        let mut child = pipeline
            .args([
                "do-timestamp=true",
                "!",
                "queue",
                "max-size-buffers=2",
                "leaky=downstream",
                "!",
                // PipeWire screen streams are variable-rate (0/1), and
                // videorate only enforces max-rate against a fixed output
                // rate. Prefer exactly fps; a slower fixed-rate camera falls
                // through to the range (drop-only cannot raise its rate).
                "videorate",
                "drop-only=true",
                &format!("max-rate={fps}"),
                "!",
                &format!("video/x-raw,framerate={fps}/1;video/x-raw,framerate=[1/1,{fps}/1]"),
                "!",
                "videoconvert",
                "n-threads=2",
                "!",
                "videoscale",
                "add-borders=true",
                "n-threads=2",
                "!",
                &format!(
                    "video/x-raw,format=I420,width={width},height={height},pixel-aspect-ratio=1/1"
                ),
                "!",
                "fdsink",
                "fd=1",
                "sync=false",
            ])
            .stdin(Stdio::from(fd))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "Install GStreamer and its PipeWire plugin to capture video.")?;
        let mut pipe = child.stdout.take().unwrap();
        let layout = Layout::i420(width, height);
        let mut pixels = vec![0; layout.size()];
        time::timeout(SECOND * 12, pipe.read_exact(&mut pixels))
            .await
            .map_err(|_| "Video capture timed out.")?
            .map_err(|_| "Could not read video from the selected source.")?;
        Ok(Self {
            route,
            quality,
            lease,
            child,
            pipe,
            layout,
            pixels,
        })
    }
    async fn feed(
        mut self,
        src: NativeVideoSource,
        mut mode: watch::Receiver<AudioMode>,
        id: Uuid,
        input: VideoInput,
        preview: Option<Tile>,
    ) -> Result<()> {
        // Stop capture independently of room-state and key-distribution RPCs.
        loop {
            let route = self.route.clone();
            let result = tokio::select! {
                biased;
                _=mode.wait_for(|m|input.request(m)!=Some(id) || input==VideoInput::Camera && m.camera!=route)=>None,
                f=self.pump(src.clone(),input,preview.as_ref())=>Some(f),
            };
            let _ = self.child.kill().await;
            if let Some(lease) = &mut self.lease {
                lease.close().await;
            }
            if let Some(result) = result {
                return if matches!(route, CameraChoice::Source { .. }) && result.is_err() {
                    Err("Selected camera stopped. Choose a camera in Devices and start it again.")
                } else {
                    result
                };
            }
            // Keep the publication and cryptor. Close the old camera before
            // opening the new source; cancellation wins even inside the picker.
            loop {
                if input != VideoInput::Camera
                    || input.request(&mode.borrow()) != Some(id)
                    || mode.has_changed().is_err()
                {
                    return Ok(());
                }
                let next = mode.borrow_and_update().camera.clone();
                tokio::select! {biased;
                    r=mode.wait_for(|m|input.request(m)!=Some(id) || m.camera!=next)=>{if r.is_err(){return Ok(())}}
                    cap=Self::open(input,next.clone(),self.quality)=>{self=cap?;break;}
                }
            }
        }
    }
    async fn pump(
        &mut self,
        src: NativeVideoSource,
        input: VideoInput,
        preview: Option<&Tile>,
    ) -> Result<()> {
        let lease = &self.lease;
        let closed = async {
            if let Some(lease) = lease
                && let Some(session) = &lease.0
            {
                if let Ok(closed) = session.receive_closed().await {
                    tokio::pin!(closed);
                    closed.next().await;
                }
            } else {
                std::future::pending::<()>().await
            }
        };
        tokio::pin!(closed);
        let layout = self.layout;
        let (pw, ph) = if input == VideoInput::Window {
            (1280, 720)
        } else {
            (320, 180)
        };
        #[cfg(debug_assertions)]
        let mut probe = trace_video_enabled().then(|| RenderProbe::new(input.name()));
        // The sharer's own tile, logged like a remote track's render rate.
        #[cfg(debug_assertions)]
        let mut shown = (trace_video_enabled() && preview.is_some()).then(|| {
            RenderProbe::new(match input {
                VideoInput::Window => "local-window",
                VideoInput::Camera => "local-camera",
            })
        });
        loop {
            #[cfg(debug_assertions)]
            if let Some(probe) = &mut probe {
                probe.capture(layout.width, layout.height);
            }
            let mut frame =
                VideoFrame::new(VideoRotation::VideoRotation0, layout.buffer(&self.pixels));
            src.capture_frame(&frame);
            // The encoder has the frame; the sharer's own tile follows the
            // capture rate (at most `quality.fps`), as remote viewers' do.
            if let Some(tile) = preview {
                #[cfg(debug_assertions)]
                let started = Instant::now();
                let shown_frame = rgba(&mut frame.buffer, pw, ph);
                #[cfg(debug_assertions)]
                if let Some(probe) = &mut shown {
                    probe.frame(started.elapsed(), layout.width, layout.height);
                }
                if tile.views.frame(&tile.sid, "", input, shown_frame) {
                    let _ = tile.events.try_send(json!({"command":"video_redraw"}));
                }
            }
            tokio::select! {
                _=&mut closed=>return Ok(()),
                f=time::timeout(SECOND*12,self.pipe.read_exact(&mut self.pixels))=>{f.map_err(|_|"Video capture timed out.")?.map_err(|_|"Video capture ended.")?;}
            }
        }
    }
}
/// KDE's portal reports no size for window streams. Read the negotiated
/// buffer size from one frame on a separate PipeWire connection instead.
async fn probe_size(fd: OwnedFd, node: u32) -> Option<(i32, i32)> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut child = Command::new("gst-launch-1.0")
        .args(["-v", "pipewiresrc", "fd=0"])
        .arg(format!("path={node}"))
        .args(["num-buffers=1", "!", "video/x-raw", "!", "fakesink"])
        .stdin(Stdio::from(fd))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut lines = BufReader::new(child.stdout.take()?).lines();
    let size = time::timeout(SECOND * 5, async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(size) = negotiated_size(&line) {
                return Some(size);
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    let _ = child.kill().await;
    size
}
/// `width=(int)W, height=(int)H` from a `gst-launch-1.0 -v` source caps line.
fn negotiated_size(line: &str) -> Option<(i32, i32)> {
    if !line.contains("GstPipeWireSrc") || !line.contains("caps = video/x-raw") {
        return None;
    }
    let field = |name: &str| -> Option<i32> {
        let rest = &line[line.find(&format!("{name}=(int)"))? + name.len() + 6..];
        rest[..rest.find(|c: char| !c.is_ascii_digit())?]
            .parse()
            .ok()
    };
    Some((field("width")?, field("height")?))
}
pub(super) async fn camera_remote() -> Result<OwnedFd> {
    crate::app_id::portal_ready().await;
    let portal = Camera::new()
        .await
        .map_err(|_| "Camera portal unavailable.")?;
    if !portal
        .is_present()
        .await
        .map_err(|_| "Could not check for a camera.")?
    {
        return Err("No camera available.");
    }
    authorize_camera(&portal).await?;
    portal
        .open_pipe_wire_remote()
        .await
        .map_err(|_| "Could not open the camera connection.")
}

pub(super) struct Publisher {
    pub(super) local_preview: Option<Previews>,
    /// Applies to the next capture this publisher opens.
    pub(super) quality: Quality,
    input: VideoInput,
    request: Option<Uuid>,
    picker: JoinSet<Result<Capture>>,
    frames: JoinSet<Result<()>>,
    published: Option<TrackSid>,
    capture_lifetime: Option<watch::Receiver<()>>,
}
pub(super) enum Update {
    Chosen(Result<Box<Capture>>),
    Ended(&'static str),
}
impl Publisher {
    pub(super) fn live_capture(&self) -> Option<(Uuid, watch::Receiver<()>)> {
        let alive = self.capture_lifetime.as_ref()?;
        (alive.has_changed().is_ok()).then_some((self.request?, alive.clone()))
    }
    pub(super) fn new(input: VideoInput) -> Self {
        Self {
            local_preview: None,
            quality: input.default_quality(),
            input,
            request: None,
            picker: JoinSet::new(),
            frames: JoinSet::new(),
            published: None,
            capture_lifetime: None,
        }
    }
    pub(super) async fn select(
        &mut self,
        request: Option<Uuid>,
        lk: &Room,
        route: CameraChoice,
    ) -> Result<()> {
        if self.request == request {
            return Ok(());
        }
        self.stop(lk).await?;
        self.request = request;
        if request.is_some() {
            self.picker
                .spawn(Capture::open(self.input, route, self.quality));
        }
        Ok(())
    }
    async fn stop(&mut self, lk: &Room) -> Result<()> {
        self.capture_lifetime = None;
        self.picker.shutdown().await;
        // On an explicit stop the worker observes the same watch change and
        // closes the portal session itself. Abort on timeout or call teardown;
        // Lease::drop and Child::kill_on_drop cover those paths.
        if !self.frames.is_empty() {
            let _ = time::timeout(SECOND, self.frames.join_next()).await;
        }
        self.frames.shutdown().await;
        if let Some(sid) = self.published.take() {
            lk.local_participant()
                .unpublish_track(&sid)
                .await
                .map_err(|_| "Could not unpublish video track.")?;
        }
        Ok(())
    }
    /// A LiveKit restart republished our track under a new SID.
    pub(super) fn rebind(&mut self, previous: &TrackSid, now: &LocalTrackPublication) {
        if self.published.as_ref() == Some(previous) {
            self.published = Some(now.sid());
        }
    }
    pub(super) async fn next(&mut self) -> Update {
        tokio::select! {
            p=self.picker.join_next(),if !self.picker.is_empty()=>Update::Chosen(p.and_then(|r|r.ok()).unwrap_or(Err("Video start interrupted.")).map(Box::new)),
            f=self.frames.join_next(),if !self.frames.is_empty()=>Update::Ended(f.and_then(|r|r.ok()).and_then(|r|r.err()).unwrap_or("Video stopped.")),
            else=>std::future::pending().await,
        }
    }
    pub(super) async fn update(
        &mut self,
        update: Update,
        lk: &Room,
        index: u8,
        events: &Sender<Value>,
        room: &str,
        mode: &watch::Receiver<AudioMode>,
    ) -> Result<()> {
        let Some(id) = self.request else {
            return Ok(());
        };
        if self.input.request(&mode.borrow()) != Some(id) {
            return Ok(());
        }
        let outcome = match update {
            Update::Chosen(Ok(cap)) => {
                let src = NativeVideoSource::new(
                    VideoResolution {
                        width: cap.layout.width,
                        height: cap.layout.height,
                    },
                    self.input == VideoInput::Window,
                );
                let video = LocalVideoTrack::create_video_track(
                    self.input.name(),
                    RtcVideoSource::Native(src.clone()),
                );
                match lk
                    .local_participant()
                    .publish_track(
                        LocalTrack::Video(video),
                        // Degradation stays at the SDK default per source:
                        // screens keep resolution, cameras keep frame rate.
                        TrackPublishOptions {
                            source: self.input.source(),
                            simulcast: false,
                            video_encoding: Some(cap.quality.encoding()),
                            ..Default::default()
                        },
                    )
                    .await
                {
                    Ok(p) => {
                        self.published = Some(p.sid());
                        let cryptor = lk
                            .e2ee_manager()
                            .frame_cryptors()
                            .remove(&(lk.local_participant().identity(), p.sid()));
                        if p.encryption_type() == EncryptionType::Gcm
                            && let Some(fc) = cryptor
                            && fc.enabled()
                        {
                            fc.set_key_index(i32::from(index));
                            if fc.key_index() != i32::from(index) {
                                return Err("Could not encrypt video.");
                            }
                            let preview = self.local_preview.clone().map(|views| Tile {
                                sid: format!("local-{}", self.input.name()),
                                views,
                                events: events.clone(),
                            });
                            let (guard, lifetime) = watch::channel(());
                            self.capture_lifetime = Some(lifetime);
                            let mode = mode.clone();
                            let input = self.input;
                            self.frames.spawn(async move {
                                let _guard = guard;
                                cap.feed(src, mode, id, input, preview).await
                            });
                            Ok(match self.input {
                                VideoInput::Window => "Sharing your screen",
                                VideoInput::Camera => "Camera is on",
                            })
                        } else {
                            Err("Video encryption unavailable.")
                        }
                    }
                    Err(_) => Err("Could not publish video."),
                }
            }
            Update::Chosen(Err(e)) | Update::Ended(e) => Err(e),
        };
        if outcome.is_err() {
            self.stop(lk).await?;
        }
        let _ = events
            .send(
                json!({"command":"room_video","data":{"room":room,"source":self.input.name(),"request":id.to_string(),
            "live":outcome.is_ok(),"label":outcome.unwrap_or_else(|e|e)}}),
            )
            .await;
        Ok(())
    }
}

/// Whether `UNTETHERED_QA_VIDEO_STATS=1` asked for [`trace_video_stats`].
#[cfg(debug_assertions)]
pub(super) fn trace_video_enabled() -> bool {
    std::env::var("UNTETHERED_QA_VIDEO_STATS").as_deref() == Ok("1")
}
/// Opt-in acceptance diagnostics, one line per sent or received video track:
/// direction, source, frame size, frame rate and byte count. Never log the
/// stats report itself: it carries candidate addresses, ICE and certificates.
#[cfg(debug_assertions)]
pub(super) async fn trace_video_stats(lk: &Room) {
    use livekit::webrtc::stats::RtcStats;
    let Ok(Ok(stats)) = time::timeout(SECOND * 3, lk.get_stats()).await else {
        eprintln!("RTC video stats unavailable");
        return;
    };
    let source = |s: TrackSource| match s {
        TrackSource::Camera => "camera",
        TrackSource::Screenshare => "screenshare",
        _ => "other",
    };
    let mut tracks = BTreeMap::new();
    for p in lk.local_participant().track_publications().into_values() {
        if let Some(LocalTrack::Video(t)) = p.track() {
            tracks.insert(
                t.rtc_track().id(),
                (p.sid().to_string(), source(p.source())),
            );
        }
    }
    for peer in lk.remote_participants().into_values() {
        for p in peer.track_publications().into_values() {
            if let Some(RemoteTrack::Video(t)) = p.track() {
                tracks.insert(
                    t.rtc_track().id(),
                    (p.sid().to_string(), source(p.source())),
                );
            }
        }
    }
    // With LiveKit's single peer connection both directions are in one report.
    for report in [&stats.publisher_stats, &stats.subscriber_stats] {
        for r in report {
            match r {
                RtcStats::OutboundRtp(o) if o.stream.kind == "video" => {
                    let track = report.iter().find_map(|m| match m {
                        RtcStats::MediaSource(m) if m.rtc.id == o.outbound.media_source_id => {
                            tracks.get(&m.source.track_identifier)
                        }
                        _ => None,
                    });
                    let (sid, kind) = track.map_or(("?", "unknown"), |(s, k)| (s.as_str(), *k));
                    let x = &o.outbound;
                    eprintln!(
                        "RTC video outbound sid={sid} source={kind} rid={} width={} height={} fps={:.1} bytes={} limit={:?} encoder={}",
                        if x.rid.is_empty() { "-" } else { &x.rid },
                        x.frame_width,
                        x.frame_height,
                        x.frames_per_second,
                        o.sent.bytes_sent,
                        x.quality_limitation_reason,
                        if x.encoder_implementation.is_empty() {
                            "-"
                        } else {
                            &x.encoder_implementation
                        },
                    );
                }
                RtcStats::InboundRtp(i) if i.stream.kind == "video" => {
                    let x = &i.inbound;
                    let (sid, kind) = tracks
                        .get(&x.track_identifier)
                        .map_or(("?", "unknown"), |(s, k)| (s.as_str(), *k));
                    eprintln!(
                        "RTC video inbound sid={sid} source={kind} width={} height={} fps={:.1} bytes={} decoded={} dropped={}",
                        x.frame_width,
                        x.frame_height,
                        x.frames_per_second,
                        x.bytes_received,
                        x.frames_decoded,
                        x.frames_dropped,
                    );
                }
                _ => (),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_swap_only_to_uploaded_frames() {
        let views = Previews::default();
        let frame = |v: u8| Handle::from_rgba(1, 1, vec![v; 4]);
        let on_gpu = |h: &Handle| -> std::result::Result<Allocation, image::Error> {
            // SAFETY: no renderer reads this one; it only marks the frame uploaded.
            #[allow(unsafe_code)]
            Ok(unsafe { image::allocate(h, iced::Size::new(1, 1)) })
        };
        let shown = || views.snapshot().1[0].picture().map(Handle::id);
        let (a, b, c) = (frame(1), frame(2), frame(3));
        assert!(views.frame("cam", "@b:h", VideoInput::Camera, a.clone()));
        let asked = views.uploads(None);
        assert_eq!((asked.len(), asked[0].1.id()), (1, a.id()));
        // Nothing to draw until it's uploaded; one upload at a time.
        assert_eq!(shown(), None);
        // The upload's answer looks for newer frames; no need to wake the UI.
        assert!(!views.frame("cam", "@b:h", VideoInput::Camera, b.clone()));
        assert!(views.uploads(None).is_empty());
        views.uploaded("cam", on_gpu(&a));
        assert_eq!(shown(), Some(a.id()));
        // Then the newest frame, while the tile keeps drawing the last one.
        views.frame("cam", "@b:h", VideoInput::Camera, c.clone());
        assert_eq!(views.uploads(None)[0].1.id(), c.id());
        assert_eq!(shown(), Some(a.id()));
        // A failed upload keeps the picture and asks again.
        views.uploaded("cam", Err(image::Error::OutOfMemory));
        assert_eq!(shown(), Some(a.id()));
        assert_eq!(views.uploads(None)[0].1.id(), c.id());
        // An answer to a request given up on doesn't replace a newer picture.
        views.uploaded("cam", on_gpu(&c));
        views.uploaded("cam", on_gpu(&b));
        assert_eq!(shown(), Some(c.id()));
        assert!(views.uploads(None).is_empty());
        // Only feeds on screen are asked for.
        views.frame("share", "", VideoInput::Window, frame(4));
        views.frame("cam", "@b:h", VideoInput::Camera, frame(5));
        let asked = views.uploads(Some("share"));
        assert_eq!((asked.len(), asked[0].0.as_str()), (1, "share"));
    }

    #[test]
    fn reads_gstreamer_padded_i420_rows() {
        // Bytes per 900-row frame that `videoconvert ! videoscale !
        // video/x-raw,format=I420,width=W ! fdsink` wrote on GStreamer 1.28.
        for (width, written) in [(1600, 2_160_000), (1604, 2_167_200), (1598, 2_160_000)] {
            assert_eq!(Layout::i420(width, 900).size(), written, "{width}");
        }
        let odd = Layout::i420(1598, 900);
        assert_eq!((odd.stride_y, odd.stride_uv), (1600, 800));
        // Each plane lands whole, padding included, in a buffer with the same strides.
        let frame: Vec<u8> = (0..odd.size()).map(|i| (i % 251) as u8).collect();
        let buf = odd.buffer(&frame);
        assert_eq!(buf.strides(), (1600, 800, 800));
        let (y, u, v) = buf.data();
        assert_eq!([y, u, v].concat(), frame);
    }

    #[test]
    fn screen_capture_keeps_source_size_up_to_1080p() {
        let q = Quality::SCREEN;
        assert_eq!(q.fit(Some((1600, 900))), (1600, 900));
        assert_eq!(q.fit(Some((2560, 1440))), (1920, 1080));
        assert_eq!(q.fit(Some((3840, 1600))), (1920, 800));
        assert_eq!(q.fit(Some((1080, 1920))), (606, 1080));
        assert_eq!(q.fit(Some((1001, 777))), (1000, 776));
        assert_eq!(q.fit(Some((1, 1))), (2, 2));
        assert_eq!(q.fit(None), (1920, 1080));
        assert_eq!(q.fit(Some((0, 720))), (1920, 1080));
    }

    #[test]
    fn reads_the_negotiated_window_size() {
        let line = "/GstPipeline:pipeline0/GstPipeWireSrc:pipewiresrc0.GstPad:src: caps = video/x-raw, format=(string)BGRx, width=(int)1600, height=(int)930, framerate=(fraction)0/1";
        assert_eq!(negotiated_size(line), Some((1600, 930)));
        let filter = "/GstPipeline:pipeline0/GstCapsFilter:capsfilter0.GstPad:src: caps = video/x-raw, width=(int)8, height=(int)8";
        assert_eq!(negotiated_size(filter), None);
        assert_eq!(
            negotiated_size("GstPipeWireSrc caps = video/x-raw, width=(int)1600"),
            None
        );
    }

    #[test]
    fn default_encodings_match_the_go_live_targets() {
        let screen = VideoInput::Window.default_quality().encoding();
        assert_eq!((screen.max_bitrate, screen.max_framerate), (2_500_000, 30.));
        let camera = VideoInput::Camera.default_quality();
        assert_eq!((camera.width, camera.height, camera.fps), (1280, 720, 30));
        assert_eq!(camera.encoding().max_bitrate, 1_700_000);
    }
}
