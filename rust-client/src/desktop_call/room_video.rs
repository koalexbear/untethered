//! Group camera and single-window video use distinct portal grants and tracks.
use super::*;
use ashpd::desktop::{
    PersistMode, Session as PortalSession,
    camera::Camera,
    screencast::{CursorMode, Screencast, SourceType},
};
use ashpd::{zbus, zvariant};
use iced::widget::image::Handle;
use livekit::webrtc::{
    prelude::{
        I420Buffer, RtcVideoSource, VideoFormatType, VideoFrame, VideoResolution, VideoRotation,
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

const W: u32 = 960;
const H: u32 = 540;
const Y: usize = (W * H) as usize;
const FRAME: usize = Y * 3 / 2;

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
            Self::Window => "Choose a window in the desktop picker…",
            Self::Camera => "Opening camera…",
        }
    }
    pub(crate) fn control(self, requested: bool, live: bool) -> &'static str {
        match (self, requested, live) {
            (Self::Window, false, _) => "Share window",
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
}
#[derive(Default)]
pub(crate) struct Status {
    pub label: String,
    pub live: bool,
}

#[derive(Clone)]
pub(crate) struct Feed {
    pub sid: String,
    pub user: String,
    pub source: VideoInput,
    pub frame: Handle,
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
    pub(crate) fn focus(&self, sid: Option<String>) {
        let mut wall = self.0.lock().unwrap();
        if sid.as_ref().is_none_or(|s| wall.feeds.contains_key(s)) {
            wall.focus = sid;
        }
    }
    fn frame(&self, sid: &str, user: &str, source: VideoInput, frame: Handle) {
        let mut wall = self.0.lock().unwrap();
        if let Some(feed) = wall.feeds.get_mut(sid) {
            feed.frame = frame;
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
                },
            );
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
    let mut tick = time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let Some(frame) = stream.next().await else {
            return Ok(());
        };
        let (w, h) = (frame.buffer.width(), frame.buffer.height());
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 8_294_400 {
            continue;
        }
        let mut buf = frame.buffer.to_i420();
        let scale = (W as f64 / w as f64).min(H as f64 / h as f64).min(1.);
        let (ow, oh) = (
            (f64::from(w) * scale).round().max(1.) as u32,
            (f64::from(h) * scale).round().max(1.) as u32,
        );
        if (ow, oh) != (w, h) {
            buf = buf.scale(ow as i32, oh as i32);
        }
        let mut rgba = vec![0; (ow * oh * 4) as usize];
        use livekit::webrtc::video_frame::native::VideoFrameBufferExt;
        buf.to_argb(
            VideoFormatType::ABGR,
            &mut rgba,
            ow * 4,
            ow as i32,
            oh as i32,
        );
        let frame = Handle::from_rgba(ow, oh, rgba);
        tile.views.frame(&tile.sid, &user, source, frame);
        let _ = tile.events.try_send(json!({"command":"video_redraw"}));
    }
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
pub(super) struct Capture {
    route: CameraChoice,
    lease: Option<Lease>,
    child: Child,
    pipe: ChildStdout,
    pixels: Vec<u8>,
}
impl Capture {
    async fn open(input: VideoInput, route: CameraChoice) -> Result<Self> {
        let (fd, node, lease) = match input {
            VideoInput::Window => {
                let (fd, node, lease) = Self::window().await?;
                (fd, Some(node), Some(lease))
            }
            VideoInput::Camera => (camera_remote().await?, None, None),
        };
        Self::reader(fd, node, lease, route).await
    }
    async fn window() -> Result<(OwnedFd, u32, Lease)> {
        let proxy = Screencast::new()
            .await
            .map_err(|_| "Window capture portal unavailable.")?;
        let lease = Lease(Some(
            proxy
                .create_session()
                .await
                .map_err(|_| "Could not open window picker.")?,
        ));
        let session = lease.0.as_ref().unwrap();
        proxy
            .select_sources(
                session,
                CursorMode::Hidden,
                SourceType::Window.into(),
                false,
                None,
                PersistMode::DoNot,
            )
            .await
            .and_then(|r| r.response())
            .map_err(|_| "The desktop could not select a window.")?;
        let selection = proxy
            .start(session, None)
            .await
            .and_then(|r| r.response())
            .map_err(|_| "Window sharing was cancelled or denied.")?;
        let [stream] = selection.streams() else {
            return Err("Select exactly one window.");
        };
        if stream
            .source_type()
            .is_some_and(|t| t != SourceType::Window)
        {
            return Err("The selection was not a window.");
        }
        let fd = proxy
            .open_pipe_wire_remote(session)
            .await
            .map_err(|_| "Could not open the selected PipeWire stream.")?;
        Ok((fd, stream.pipe_wire_node_id(), lease))
    }
    async fn reader(
        fd: OwnedFd,
        node: Option<u32>,
        lease: Option<Lease>,
        route: CameraChoice,
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
        let mut child = pipeline
            .args([
                "do-timestamp=true",
                "!",
                "queue",
                "max-size-buffers=2",
                "leaky=downstream",
                "!",
                "videorate",
                "drop-only=true",
                "max-rate=12",
                "!",
                "videoconvert",
                "!",
                "videoscale",
                "add-borders=true",
                "!",
                "video/x-raw,format=I420,width=960,height=540,pixel-aspect-ratio=1/1",
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
        let mut pixels = vec![0; FRAME];
        time::timeout(SECOND * 12, pipe.read_exact(&mut pixels))
            .await
            .map_err(|_| "Video capture timed out.")?
            .map_err(|_| "Could not read video from the selected source.")?;
        Ok(Self {
            route,
            lease,
            child,
            pipe,
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
                    cap=Self::open(input,next.clone())=>{self=cap?;break;}
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
        let mut last = None;
        loop {
            let mut buf = I420Buffer::new(W, H);
            let (y, u, v) = buf.data_mut();
            y.copy_from_slice(&self.pixels[..Y]);
            u.copy_from_slice(&self.pixels[Y..Y + Y / 4]);
            v.copy_from_slice(&self.pixels[Y + Y / 4..]);
            if let Some(tile) = preview
                && last.is_none_or(|at: Instant| at.elapsed() >= Duration::from_millis(100))
            {
                use livekit::webrtc::video_frame::native::VideoFrameBufferExt;
                let (w, h) = if input == VideoInput::Window {
                    (W, H)
                } else {
                    (320, 180)
                };
                let mut rgba = vec![0; (w * h * 4) as usize];
                if input == VideoInput::Window {
                    buf.to_argb(VideoFormatType::ABGR, &mut rgba, w * 4, w as i32, h as i32);
                } else {
                    buf.scale(w as i32, h as i32).to_argb(
                        VideoFormatType::ABGR,
                        &mut rgba,
                        w * 4,
                        w as i32,
                        h as i32,
                    );
                }
                tile.views
                    .frame(&tile.sid, "", input, Handle::from_rgba(w, h, rgba));
                let _ = tile.events.try_send(json!({"command":"video_redraw"}));
                last = Some(Instant::now());
            }
            src.capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, buf));
            tokio::select! {
                _=&mut closed=>return Ok(()),
                f=time::timeout(SECOND*12,self.pipe.read_exact(&mut self.pixels))=>{f.map_err(|_|"Video capture timed out.")?.map_err(|_|"Video capture ended.")?;}
            }
        }
    }
}
pub(super) async fn camera_remote() -> Result<OwnedFd> {
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
    input: VideoInput,
    request: Option<Uuid>,
    picker: JoinSet<Result<Capture>>,
    frames: JoinSet<Result<()>>,
    published: Option<TrackSid>,
}
pub(super) enum Update {
    Chosen(Result<Box<Capture>>),
    Ended(&'static str),
}
impl Publisher {
    pub(super) fn new(input: VideoInput) -> Self {
        Self {
            local_preview: None,
            input,
            request: None,
            picker: JoinSet::new(),
            frames: JoinSet::new(),
            published: None,
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
            self.picker.spawn(Capture::open(self.input, route));
        }
        Ok(())
    }
    async fn stop(&mut self, lk: &Room) -> Result<()> {
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
                        width: W,
                        height: H,
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
                        TrackPublishOptions {
                            source: self.input.source(),
                            simulcast: false,
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
                            self.frames
                                .spawn(cap.feed(src, mode.clone(), id, self.input, preview));
                            Ok(match self.input {
                                VideoInput::Window => "Sharing the selected window",
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
