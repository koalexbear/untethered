//! Linux MatrixRTC voice. Matrix state/Olm stay in the existing gomuks device;
//! only the media and audio streams live here, for the lifetime of a call.
use crate::call_devices::{Catalog, Port, Route};
use async_channel::{Receiver, Sender};
use futures_util::StreamExt;
use livekit::{
    e2ee::{E2eeOptions, EncryptionType, key_provider::KeyProvider},
    prelude::*,
    webrtc::{
        audio_source::native::NativeAudioSource,
        prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
    },
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::watch,
    task::JoinSet,
    time::{self, Instant},
};
use untethered_iced::rtc_keys::{self, EVENT as KEY, SenderKey};
use untethered_iced::rtc_membership::{CallKey, CallSignal, CallUpdate, EVENT_TYPE, Membership};
use url::Url;
mod audio;
mod cameras;
mod channel;
mod device_test;
mod recovery;
mod room_video;
mod share_audio;
mod voice;
use audio::{AudioIo, Kind, Mixer};
pub(crate) use cameras::{CameraChoice, list_cameras};
pub(crate) use device_test::mic_test;
use recovery::{LeaseWatch, Lost, Rejoin, Step, close, encrypted_tracks, select_key, select_one};
pub(super) use room_video::{Feed, Layer, Previews, VideoInput};
pub(crate) use room_video::{Preview, preview};
pub(crate) use voice::InputLevel;
use voice::Voice;

/// The microphones, speakers and shareable app streams of this platform.
pub(super) fn list_devices() -> futures_util::future::BoxFuture<'static, Result<Catalog>> {
    audio::platform().list_devices()
}

pub enum Target {
    Dm {
        video: bool,
        peer: String,
        incoming: Option<crate::incoming::Offer>,
    },
    Channel,
}

type Result<T> = std::result::Result<T, &'static str>;
const SLOT: &str = "org.matrix.msc4143.rtc.slot";
const RING: &str = "org.matrix.msc4075.rtc.notification";
const SLOT_ID: &str = "m.call#ROOM";
const RING_LIFETIME: Duration = Duration::from_secs(45);
const ENDED: &str = "Call ended.";
const SECOND: Duration = Duration::from_secs(1);
const LEASE: Duration = Duration::from_secs(16);
const RECOVERY_LIMIT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum TalkGate {
    #[default]
    Open,
    Waiting,
    Held,
    /// Released; capture stays open until the deadline.
    Releasing(Instant),
}
impl TalkGate {
    pub(super) fn from_ptt(enabled: bool) -> Self {
        if enabled { Self::Waiting } else { Self::Open }
    }
}

#[derive(Clone, Default)]
pub struct AudioMode {
    // Deafen is an overlay: lifting it must not undo a manual mute.
    pub muted: bool,
    pub deafened: bool,
    pub share_silent: bool,
    pub send_app: Option<(uuid::Uuid, crate::call_devices::AppStream)>,
    pub(super) talk: TalkGate,
    pub mic: Route,
    pub speaker: Route,
    pub camera: CameraChoice,
    // Receiver-local gain, keyed by Matrix user rather than a device/track
    // SID. Replacement microphones inherit the same setting for this call.
    volumes: std::collections::BTreeMap<String, u8>,
    pub video: [Option<uuid::Uuid>; 2],
    /// Microphone processing and the voice-activity gate (call_settings.rs).
    pub voice: crate::call_settings::Voice,
}
impl AudioMode {
    pub(super) fn volume(&self, user: &str) -> u8 {
        self.volumes.get(user).copied().unwrap_or(100)
    }
    pub(super) fn set_volume(&mut self, user: String, percent: u8) -> bool {
        let percent = percent.min(200);
        if self.volume(&user) == percent {
            return false;
        }
        if percent == 100 {
            self.volumes.remove(&user);
        } else {
            self.volumes.insert(user, percent);
        }
        true
    }
    fn mic_off(&self) -> bool {
        self.muted || self.deafened || self.talk == TalkGate::Waiting
    }
    pub(super) fn hold(&mut self, down: bool) -> bool {
        let next = match self.talk {
            TalkGate::Open => return false,
            // A manual mute or deafen overrides the key at once.
            _ if self.muted || self.deafened => TalkGate::Waiting,
            _ if down => TalkGate::Held,
            // Capture stays open for the release delay (Voice & Video).
            TalkGate::Held => TalkGate::Releasing(Instant::now() + self.voice.release()),
            waiting => waiting,
        };
        if next == self.talk {
            return false;
        }
        self.talk = next;
        true
    }
    /// When a pending release should close capture.
    pub(super) fn release_due(&self) -> Option<Instant> {
        match self.talk {
            TalkGate::Releasing(until) => Some(until),
            _ => None,
        }
    }
    /// Ends the release scheduled for `until`, unless a press, mute or
    /// deafen has changed the gate since.
    pub(super) fn finish_release(&mut self, until: Instant) -> bool {
        if self.talk != TalkGate::Releasing(until) {
            return false;
        }
        self.talk = TalkGate::Waiting;
        true
    }
}
/// Mute, deafen and the chosen microphone and speaker. They outlive calls,
/// as on Discord: set before joining, they start the next call; set during
/// one, they stay for the next (calls plan U1). The devices are saved on
/// this computer with the other Voice & Video settings (call_settings.rs).
#[derive(Default)]
pub struct LocalAudio {
    pub muted: bool,
    pub deafened: bool,
    /// PulseAudio device names; None follows the system default.
    pub mic: Option<String>,
    pub speaker: Option<String>,
    /// The devices the menus list, read when one opens.
    pub devices: Catalog,
    pub loading: bool,
    pub list_error: Option<&'static str>,
    /// The cameras the Voice & Video page lists, read when asked: listing
    /// them goes through the camera portal, which may ask for permission.
    pub cameras: Vec<CameraChoice>,
    pub camera_listing: bool,
    pub camera_note: Option<&'static str>,
}
impl LocalAudio {
    pub fn route(&self, port: Port) -> Route {
        Route {
            name: match port {
                Port::Mic => self.mic.clone(),
                Port::Speaker => self.speaker.clone(),
            },
            revision: 0,
        }
    }
}
/// The popups of the user panel, the Voice Connected panel and the Voice &
/// Video page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceMenu {
    /// Mute's chevron: input devices.
    Input,
    /// Deafen's chevron: output devices.
    Output,
    /// The call's settings that have no other home yet: shared audio (U4)
    /// and each participant's volume (U3), with a link to the page.
    Settings,
    /// The page's input and output device lists.
    PageInput,
    PageOutput,
}
pub struct ActiveCall {
    pub room: String,
    pub label: String,
    pub group: bool,
    /// A DM's other user, whose local volume the mixer sets.
    pub peer: Option<String>,
    pub video_peer: Option<String>,
    pub cancel: Sender<()>,
    pub audio: watch::Sender<AudioMode>,
    pub id: uuid::Uuid,
    pub mic_error: bool,
    pub speaker_error: bool,
    pub video_status: [room_video::Status; 2],
    pub previews: Previews,
    pub preview_hidden: bool,
    pub app_note: String,
    pub stage_open: bool,
    pub theater: bool,
    pub speaking: watch::Receiver<std::collections::BTreeSet<String>>,
    /// The microphone's processed level and the gate's verdict, for the
    /// input sensitivity meter.
    pub input: watch::Receiver<InputLevel>,
    /// The stage's scrolling thumbnail strip and grid: tiles scrolled
    /// away get no video.
    pub stage_scroll: [Scroll; 2],
}
/// The stage's two scrolling areas.
#[derive(Clone, Copy, Debug)]
pub enum StageScroll {
    Thumbnails,
    Grid,
}
/// Where a scrolling area of the stage is: scrolled this far, showing this
/// much of its length (0 until it reported its size).
#[derive(Clone, Copy, Debug, Default)]
pub struct Scroll {
    pub offset: f32,
    pub extent: f32,
}
/// What a scrolling area of the stage reported: it scrolled, or its size
/// on screen changed. It reports no scrolling while everything fits.
#[derive(Clone, Copy, Debug)]
pub enum Scrolled {
    Offset(f32),
    Extent(f32),
}
struct GroupViews {
    previews: Previews,
    speakers: watch::Sender<std::collections::BTreeSet<String>>,
    level: watch::Sender<InputLevel>,
}
pub struct Controls {
    cancel: Receiver<()>,
    audio: watch::Receiver<AudioMode>,
    views: GroupViews,
}
impl crate::App {
    /// Build the window's view, noting which call video it draws: video
    /// nobody can see pauses, and each tile gets the layer its size needs.
    /// The media viewer covers everything under it.
    pub(super) fn watch_video<T>(&self, view: impl FnOnce() -> T) -> T {
        let Some(call) = &self.call else {
            return view();
        };
        call.previews.start_view();
        let view = view();
        call.previews.finish_view(self.viewer.is_some());
        view
    }
    /// The window closed to the tray: no video is on screen.
    pub(super) fn hide_video(&self) {
        if let Some(call) = &self.call {
            call.previews.hide_all();
        }
    }
    pub(super) fn stage_scrolled(&mut self, id: uuid::Uuid, area: StageScroll, report: Scrolled) {
        if let Some(call) = &mut self.call
            && call.id == id
        {
            let scroll = &mut call.stage_scroll[area as usize];
            match report {
                Scrolled::Offset(offset) => scroll.offset = offset,
                Scrolled::Extent(extent) => scroll.extent = extent,
            }
        }
    }
    /// Upload the newest frame of each video tile on screen. A tile swaps to
    /// a frame only once it's on the GPU (`room_video::Feed`), so it never
    /// draws an empty frame while iced uploads a large image in the
    /// background. Frames nobody can see aren't uploaded.
    pub(super) fn upload_video(&mut self) -> iced::Task<crate::Msg> {
        let Some(call) = &self.call else {
            return iced::Task::none();
        };
        iced::Task::batch(call.previews.uploads().into_iter().map(|(sid, frame)| {
            iced_runtime::image::allocate(frame)
                .map(move |done| crate::Msg::VideoUploaded(sid.clone(), done))
        }))
    }
    pub(super) fn video_uploaded(
        &mut self,
        sid: String,
        frame: std::result::Result<iced::advanced::image::Allocation, iced::advanced::image::Error>,
    ) -> iced::Task<crate::Msg> {
        if let Some(call) = &self.call {
            call.previews.uploaded(&sid, frame);
        }
        self.upload_video()
    }
    /// A voice channel clicked in the room list joins its call, as on
    /// Discord. Its call opens if you're in it already; a call elsewhere
    /// ends first, and this one starts once it has.
    pub(super) fn join_channel(&mut self, room: String) {
        let joinable = self
            .live
            .list
            .get(&room)
            .is_some_and(|r| r.voice && r.encrypted);
        if !joinable || !self.verified || !self.connected || self.inbox.ringing.is_some() {
            return;
        }
        match &mut self.call {
            Some(call) if call.room == room => call.stage_open = true,
            Some(call) => {
                if !call.cancel.is_closed() {
                    call.cancel.close();
                    call.label = "Ending call…".into();
                }
                self.join_after = Some(room);
            }
            None => self.start_voice(room, Target::Channel),
        }
    }
    pub(super) fn start_voice(&mut self, room: String, target: Target) {
        self.inbox.handled = None;
        let group = matches!(target, Target::Channel);
        let peer = match &target {
            Target::Dm { peer, .. } => Some(peer.clone()),
            Target::Channel => None,
        };
        let video_peer = match &target {
            Target::Dm {
                video: true, peer, ..
            } => Some(peer.clone()),
            _ => None,
        };
        let label = match &target {
            Target::Channel => "Joining voice…",
            Target::Dm {
                incoming: Some(_), ..
            } => "Answering…",
            Target::Dm { .. } => "Starting call…",
        };
        let (cancel, rx) = async_channel::bounded(1);
        let (audio, mode) = watch::channel(AudioMode {
            video: [
                None,
                matches!(
                    target,
                    Target::Dm {
                        video: true,
                        incoming: None,
                        ..
                    }
                )
                .then(uuid::Uuid::new_v4),
            ],
            // Push to talk is the input mode for every call, as on Discord.
            talk: TalkGate::from_ptt(self.push_to_talk()),
            voice: self.voice.voice,
            camera: CameraChoice::saved(self.voice.camera.as_ref()),
            // Muted, deafened or on another device before joining: the
            // call starts that way.
            muted: self.local_audio.muted,
            deafened: self.local_audio.deafened,
            mic: Route {
                name: self.local_audio.mic.clone(),
                revision: 0,
            },
            speaker: Route {
                name: self.local_audio.speaker.clone(),
                revision: 0,
            },
            ..AudioMode::default()
        });
        let previews = Previews::default();
        let (speakers, speaking) = watch::channel(Default::default());
        let (level, input) = watch::channel(InputLevel::default());
        let controls = Controls {
            cancel: rx,
            audio: mode,
            views: GroupViews {
                previews: previews.clone(),
                speakers,
                level,
            },
        };
        // The mic test can't share the microphone with a call.
        self.voice.mic_test = None;
        if self
            .commands
            .try_send(crate::backend::Command::Voice {
                room: room.clone(),
                target,
                controls,
            })
            .is_ok()
        {
            self.status.clear();
            self.call = Some(ActiveCall {
                room,
                label: label.into(),
                group,
                peer,
                video_peer,
                cancel,
                audio,
                id: uuid::Uuid::new_v4(),
                mic_error: false,
                speaker_error: false,
                video_status: Default::default(),
                previews,
                preview_hidden: false,
                app_note: String::new(),
                stage_open: false,
                theater: false,
                speaking,
                input,
                stage_scroll: Default::default(),
            });
        } else {
            self.status = "Could not queue the voice call.".into();
        }
    }
}
impl crate::App {
    /// The call still running, if any: one that's ending takes no changes.
    fn live_call(&self) -> Option<&ActiveCall> {
        self.call.as_ref().filter(|c| !c.cancel.is_closed())
    }
    /// Mute or unmute, in a call or before one. The mic stays muted while
    /// deafened.
    pub(super) fn toggle_mute(&mut self) {
        let local = &mut self.local_audio;
        if local.deafened {
            return;
        }
        local.muted = !local.muted;
        let sound = if local.muted {
            crate::sounds::Sound::Mute
        } else {
            crate::sounds::Sound::Unmute
        };
        self.apply_mute(sound);
    }
    pub(super) fn toggle_deafen(&mut self) {
        let local = &mut self.local_audio;
        local.deafened = !local.deafened;
        let sound = if local.deafened {
            crate::sounds::Sound::Deafen
        } else {
            crate::sounds::Sound::Undeafen
        };
        self.apply_mute(sound);
    }
    fn apply_mute(&mut self, sound: crate::sounds::Sound) {
        let (muted, deafened) = (self.local_audio.muted, self.local_audio.deafened);
        match self.live_call() {
            Some(call) => {
                call.audio.send_if_modified(|s| {
                    if (s.muted, s.deafened) == (muted, deafened) {
                        return false;
                    }
                    s.muted = muted;
                    s.deafened = deafened;
                    // A manual mute or deafen closes push-to-talk until a
                    // fresh press.
                    s.hold(false);
                    true
                });
            }
            // Outside a call the sound plays here; in one, the call's own
            // change does (sounds.rs).
            None => self.local_sound(sound),
        }
    }
    /// Open or close a voice popup. Each reads the devices and the apps
    /// playing again when it opens.
    pub(super) fn voice_menu(&mut self, menu: Option<VoiceMenu>) -> iced::Task<crate::Msg> {
        self.voice_popup = menu;
        if menu.is_some() {
            return self.refresh_devices();
        }
        iced::Task::none()
    }
    pub(super) fn refresh_devices(&mut self) -> iced::Task<crate::Msg> {
        if self.local_audio.loading {
            return iced::Task::none();
        }
        self.local_audio.loading = true;
        iced::Task::perform(list_devices(), crate::Msg::DevicesListed)
    }
    pub(super) fn devices_listed(&mut self, result: Result<Catalog>) {
        let local = &mut self.local_audio;
        local.loading = false;
        match result {
            Ok(devices) => {
                local.devices = devices;
                local.list_error = None;
            }
            Err(e) => local.list_error = Some(e),
        }
    }
    /// Use `device` from now on: in the call at once, in the next one and
    /// after a restart.
    pub(super) fn select_device(
        &mut self,
        port: Port,
        device: crate::call_devices::Choice,
    ) -> iced::Task<crate::Msg> {
        if !self.local_audio.devices.list(port).contains(&device) {
            return iced::Task::none();
        }
        match port {
            Port::Mic => self.local_audio.mic = device.name.clone(),
            Port::Speaker => self.local_audio.speaker = device.name.clone(),
        }
        self.voice_popup = None;
        self.mic_test_device(port);
        if let Some(call) = self.call.as_mut().filter(|c| !c.cancel.is_closed()) {
            call.audio.send_modify(|s| {
                let route = match port {
                    Port::Mic => &mut s.mic,
                    Port::Speaker => &mut s.speaker,
                };
                route.name = device.name;
                // Choosing the same device again retries it after a failure.
                route.revision += 1;
            });
            match port {
                Port::Mic => call.mic_error = false,
                Port::Speaker => call.speaker_error = false,
            }
        }
        self.save_voice()
    }
    /// List the cameras for the Voice & Video page.
    pub(super) fn list_cameras(&mut self) -> iced::Task<crate::Msg> {
        let local = &mut self.local_audio;
        if local.camera_listing {
            return iced::Task::none();
        }
        local.camera_listing = true;
        local.camera_note = None;
        iced::Task::perform(list_cameras(), crate::Msg::CamerasListed)
    }
    pub(super) fn cameras_listed(&mut self, list: Result<Vec<CameraChoice>>) {
        let local = &mut self.local_audio;
        local.camera_listing = false;
        match list {
            Ok(v) => {
                local.camera_note = (v.len() == 1).then_some("No selectable cameras reported.");
                local.cameras = v;
            }
            Err(why) => {
                local.cameras.clear();
                local.camera_note = Some(why);
            }
        }
    }
    /// The camera chosen on the page: in the call at once (switching a live
    /// camera keeps its publication), in the preview, and saved.
    pub(super) fn select_camera(&mut self, choice: CameraChoice) -> iced::Task<crate::Msg> {
        if !self.local_audio.cameras.contains(&choice) {
            return iced::Task::none();
        }
        self.voice.camera = choice.to_saved();
        if let Some(call) = self.live_call() {
            call.audio.send_modify(|s| s.camera = choice);
        }
        self.save_voice()
    }
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// One call's signaling: the engine's [`CallSignal`], and what the call
/// collects from it while it waits.
struct Wire<'a, S> {
    signal: S,
    room: &'a str,
    dm_peer: Option<&'a str>,
    keys: Vec<(Instant, CallKey)>,
    roster_dirty: bool,
    declines: VecDeque<String>,
}
impl<S: CallSignal> Wire<'_, S> {
    /// Wait for the next signal and note what it means for this call.
    async fn pump(&mut self) -> Result<()> {
        let updates = self.signal.next(self.room).await?;
        self.note(updates);
        Ok(())
    }
    /// Note what already arrived, without waiting.
    fn absorb(&mut self) {
        let updates = self.signal.ready();
        self.note(updates);
    }
    fn note(&mut self, updates: Vec<CallUpdate>) {
        for update in updates {
            match update {
                CallUpdate::Roster => self.roster_dirty = true,
                CallUpdate::Key(key) => {
                    if self.dm_peer.is_some_and(|peer| key.sender != peer)
                        || key.content["room_id"] != self.room
                    {
                        continue;
                    }
                    if self.keys.len() >= 64 {
                        // Keep the queue bounded even if a room member
                        // sends keys without joining the voice slot.
                        self.keys.remove(0);
                    }
                    self.keys.push((Instant::now(), key));
                }
                CallUpdate::Decline { sender, refers_to } => {
                    if self.dm_peer != Some(sender.as_str())
                        || self.declines.iter().any(|t| *t == refers_to)
                    {
                        continue;
                    }
                    if self.declines.len() == 64 {
                        self.declines.pop_front();
                    }
                    self.declines.push_back(refers_to);
                }
            }
        }
    }
    async fn state(&mut self, refetch: bool) -> Result<Vec<Value>> {
        self.signal.room_state(self.room, refetch).await
    }
    async fn open_slot(&mut self, state: &[Value]) -> Result<()> {
        let previous = state
            .iter()
            .find(|e| e["type"] == SLOT && e["state_key"] == SLOT_ID)
            .map(|e| &e["content"]);
        let mut content = previous.cloned().unwrap_or_else(|| json!({}));
        if !content.is_object() {
            return Err("Invalid call slot state.");
        }
        content["application"] = json!({"type":"m.call"});
        content["status"] = json!("open");
        content["encryption"] = json!({"type":"org.matrix.msc4143.per_member"});
        if previous == Some(&content) {
            return Ok(());
        }
        self.signal
            .set_state(self.room, SLOT, SLOT_ID, content)
            .await
            .map_err(|_| "Homeserver rejected the call request.")?;
        Ok(())
    }
}
fn allowed(url: &Url, tls: &str, local: &str, allow_loopback: bool) -> bool {
    url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && (url.scheme() == tls
            || (allow_loopback
                && url.scheme() == local
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
}
fn focus(list: &Value) -> Option<Url> {
    list.as_array()?
        .iter()
        .filter(|t| t["type"] == "livekit")
        .find_map(|t| Url::parse(t["livekit_service_url"].as_str()?).ok())
}
pub(super) fn check_dm(state: &[Value], user: &str, peer: &str) -> Result<()> {
    let joined: Vec<_> = state
        .iter()
        .filter(|e| e["type"] == "m.room.member" && e["content"]["membership"] == "join")
        .filter_map(|e| e["state_key"].as_str())
        .collect();
    if state
        .iter()
        .any(|e| e["type"] == "m.room.create" && e["content"]["type"] == crate::rooms::VOICE_ROOM)
        || user == peer
        || joined.len() != 2
        || !joined.contains(&user)
        || !joined.contains(&peer)
        || !state.iter().any(|e| {
            e["type"] == "m.room.encryption" && e["content"]["algorithm"] == "m.megolm.v1.aes-sha2"
        })
    {
        return Err("Voice calls require an encrypted DM with two joined people.");
    }
    Ok(())
}
pub(super) fn active_member(e: &Value) -> bool {
    let c = &e["content"];
    e["type"] == EVENT_TYPE
        && c["application"] == "m.call"
        && c["scope"] == "m.room"
        && c["call_id"] == ""
        && c["device_id"]
            .as_str()
            .is_some_and(|d| !d.is_empty() && d != "*")
        && c["created_ts"]
            .as_u64()
            .or(e["origin_server_ts"].as_u64())
            .unwrap_or(0)
            .saturating_add(c["expires"].as_u64().unwrap_or(14_400_000))
            > now()
}
struct Session {
    user: String,
    device: String,
    focus: Url,
    sfu: String,
    token: String,
    http: reqwest::Client,
    /// A loopback homeserver may use plain HTTP and WS.
    local: bool,
}
impl Session {
    /// Our LiveKit identity, which call keys are installed for.
    fn identity(&self) -> String {
        format!("{}:{}", self.user, self.device)
    }
}
/// The call's LiveKit rooms. A parked room has lost its membership but stays
/// connected until a new room replaces its participant: closing it first
/// would tell a DM peer that we hung up.
#[derive(Default)]
struct Media {
    live: Option<Room>,
    parked: Option<Room>,
}
async fn prepare<S: CallSignal>(
    w: &mut Wire<'_, S>,
    incoming: Option<&crate::incoming::Offer>,
) -> Result<Session> {
    let identity = w.signal.identity().await?;
    if !identity.verified {
        return Err("Verify the Matrix device before calling.");
    }
    let (user, device) = (identity.user.as_str(), identity.device.as_str());
    let state = w.state(true).await?;
    if let Some(peer) = w.dm_peer {
        check_dm(&state, user, peer)?;
        if let Some(i) = incoming {
            if !i.valid_state(&state, user) {
                return Err("The caller is no longer ringing.");
            }
        } else if state.iter().any(active_member) {
            return Err("A call is already active in this DM.");
        }
    } else {
        channel::check_room(&state, user)?;
    }
    let hs = Url::parse(&identity.homeserver).map_err(|_| "Unknown homeserver.")?;
    let local = matches!(hs.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    let http = reqwest::Client::builder()
        .timeout(SECOND * 7)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not create RTC client.")?;
    let mut service = focus(&Value::Array(
        w.signal.rtc_transports().await.unwrap_or_default(),
    ));
    if service.is_none() {
        // Legacy Element Call deployments announce their OpenID service here.
        let canonical = user
            .split_once(':')
            .and_then(|(_, s)| Url::parse(&format!("https://{s}")).ok());
        for base in std::iter::once(hs).chain(canonical) {
            if !allowed(&base, "https", "http", local) {
                continue;
            }
            let Ok(response) = http
                .get(base.join("/.well-known/matrix/client").unwrap())
                .send()
                .await
            else {
                continue;
            };
            if let Ok(body) = response.json::<Value>().await {
                service = focus(&body["org.matrix.msc4143.rtc_foci"]);
            }
            if service.is_some() {
                break;
            }
        }
    }
    let focus = service
        .filter(|s| allowed(s, "https", "http", local))
        .ok_or("No supported MatrixRTC LiveKit service advertised.")?;
    if let Some(i) = incoming
        && !state.iter().filter(|e| i.matches_member(e)).any(|e| {
            crate::rooms::items(&e["content"]["foci_preferred"]).any(|f| {
                f["type"] == "livekit"
                    && f["livekit_service_url"]
                        .as_str()
                        .and_then(|s| Url::parse(s).ok())
                        .is_some_and(|u| {
                            u.as_str().trim_end_matches('/') == focus.as_str().trim_end_matches('/')
                        })
            })
        })
    {
        return Err(
            "The caller is using another call server; multi-server calls are not supported yet.",
        );
    }
    let mut session = Session {
        user: user.into(),
        device: device.into(),
        focus,
        sfu: String::new(),
        token: String::new(),
        http,
        local,
    };
    (session.sfu, session.token) = authorize(w, &session).await?;
    if incoming.is_none() {
        w.open_slot(&state).await?;
    }
    Ok(session)
}
/// The SFU and a LiveKit token for it, from the call's focus. A rejoin needs
/// a new one: the old token may be gone with the old session.
async fn authorize<S: CallSignal>(w: &mut Wire<'_, S>, s: &Session) -> Result<(String, String)> {
    let openid = w.signal.openid_token().await?;
    let auth: Value = s
        .http
        .post(format!(
            "{}/sfu/get",
            s.focus.as_str().trim_end_matches('/')
        ))
        .json(&json!({"room":w.room,"device_id":s.device,"openid_token":openid}))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| "Call server authorization failed.")?
        .json()
        .await
        .map_err(|_| "Invalid call server response.")?;
    let sfu = Url::parse(auth["url"].as_str().unwrap_or("")).map_err(|_| "Invalid SFU URL.")?;
    if !allowed(&sfu, "wss", "ws", s.local) {
        return Err("The SFU requires a secure connection.");
    }
    let token = auth["jwt"]
        .as_str()
        .ok_or("Missing SFU authorization.")?
        .to_owned();
    Ok((sfu.into(), token))
}
async fn status(events: &Sender<Value>, room: &str, label: &str) {
    let _ = events
        .send(json!({"command":"desktop_call","data":{"room":room,"label":label}}))
        .await;
}
/// A call whose Matrix signaling goes through `signal`.
pub async fn run_with<S: CallSignal>(
    signal: S,
    room: &str,
    target: Target,
    controls: Controls,
    events: &Sender<Value>,
) -> Result<&'static str> {
    let Controls {
        cancel,
        audio,
        views,
    } = controls;
    let (dm_peer, incoming, video) = match &target {
        Target::Channel => (None, None, false),
        Target::Dm {
            peer,
            incoming,
            video,
        } => (Some(peer.as_str()), incoming.as_ref(), *video),
    };
    let mut w = Wire {
        signal,
        room,
        dm_peer,
        keys: vec![],
        roster_dirty: true,
        declines: VecDeque::new(),
    };
    let mut s =
        tokio::select! {biased; _=cancel.recv()=>return Ok(ENDED),s=prepare(&mut w,incoming)=>s?};
    if cancel.is_closed() {
        return Ok(ENDED);
    }
    // Element may send its key as soon as it sees our membership.
    w.signal.start().await?;
    let mut member = match Membership::join_with_video(
        room,
        &s.user,
        &s.device,
        &s.focus,
        LEASE.as_millis() as u32,
        video,
        &mut w.signal,
    )
    .await
    {
        Ok(member) => member,
        Err(_) => {
            w.signal.stop().await;
            return Err("Could not join the MatrixRTC call.");
        }
    };
    let mut media = Media::default();
    let session = async {
        match dm_peer {
            Some(_) => {
                call(
                    &mut w,
                    &mut s,
                    &mut member,
                    events,
                    &mut media,
                    DmMedia {
                        incoming,
                        mode: audio,
                        video,
                        previews: views.previews,
                        level: views.level,
                    },
                )
                .await
            }
            None => {
                channel::joined(
                    &mut w,
                    &mut s,
                    &mut member,
                    events,
                    &mut media,
                    audio,
                    views,
                )
                .await
            }
        }
    };
    let result = tokio::select! {biased;_=cancel.recv()=>Ok(ENDED),r=session=>r};
    close(&mut media.live).await;
    close(&mut media.parked).await;
    // Dropping call() already stopped the audio jobs. Publish leave even on error/cancel.
    let leave = member.leave(&mut w.signal).await;
    w.signal.stop().await;
    leave.map_err(|_| "Disconnected; homeserver will expire call membership.")?;
    result
}
struct DmMedia<'a> {
    incoming: Option<&'a crate::incoming::Offer>,
    mode: watch::Receiver<AudioMode>,
    video: bool,
    previews: Previews,
    level: watch::Sender<InputLevel>,
}
/// Our sender key for this DM, and the provider holding both devices' keys.
struct DmKeys {
    provider: KeyProvider,
    key: SenderKey,
}
impl DmKeys {
    fn new(s: &Session, index: u8) -> Result<Self> {
        let provider = rtc_keys::provider();
        let key = SenderKey::fresh(index)?;
        if !provider.set_key(&s.identity().into(), index.into(), key.bytes.to_vec()) {
            return Err("Failed to install local key.");
        }
        recovery::trace_key(index);
        Ok(Self { provider, key })
    }
}
/// How one LiveKit session of a DM ended.
enum Talk {
    Ended(&'static str),
    Lost(Lost),
    /// A rejoin could not connect yet.
    Unready,
}
async fn call<S: CallSignal>(
    w: &mut Wire<'_, S>,
    s: &mut Session,
    m: &mut Membership,
    events: &Sender<Value>,
    media: &mut Media,
    dm: DmMedia<'_>,
) -> Result<&'static str> {
    let dm_peer = w.dm_peer.ok_or("Missing DM recipient.")?;
    let mut lease = LeaseWatch::new();
    let mut peer = answer(w, &s.user, m, events, &dm, &mut lease).await?;
    status(events, w.room, "Connecting").await;
    let mut keys = DmKeys::new(s, 0)?;
    // Send our key and wait for the peer's: on the first connection, and
    // after a rejoin with a new membership.
    let mut exchange = true;
    let mut rejoin: Option<Rejoin> = None;
    loop {
        if let Some(r) = &mut rejoin {
            r.wait(w).await?;
            let user = s.user.clone();
            let original = &peer;
            // Only back into the same call: never a new one without a ring.
            let step = r
                .attempt(w, s, m, &mut lease, dm.video, |state| {
                    check_dm(state, &user, dm_peer)?;
                    if state.iter().any(|e| original.matches(dm_peer, e)) {
                        Ok(())
                    } else {
                        Err("The call ended while you were offline.")
                    }
                })
                .await?;
            if let Step::Ready { fresh } = step {
                recovery::trace(format_args!("RTC call rejoined fresh={fresh}"));
            }
            match step {
                Step::Retry => continue,
                Step::Ready { fresh: true } => {
                    keys = DmKeys::new(s, (keys.key.index + 1) % 255)?;
                    exchange = true;
                }
                Step::Ready { fresh: false } => (),
            }
        }
        let session = DmSession {
            dm: &dm,
            keys: &keys,
            exchange,
            rejoining: rejoin.is_some(),
        };
        match talk(w, s, m, events, media, session, &mut peer, &mut lease).await? {
            Talk::Ended(label) => return Ok(label),
            Talk::Unready => close(&mut media.live).await,
            Talk::Lost(lost) => {
                recovery::trace(format_args!("RTC call lost={lost:?}"));
                close(&mut media.parked).await;
                let connected = media
                    .live
                    .as_ref()
                    .is_some_and(|lk| lk.connection_state() == ConnectionState::Connected);
                if lost == Lost::Lease && connected {
                    media.parked = media.live.take();
                } else {
                    close(&mut media.live).await;
                }
                status(events, w.room, "Reconnecting…").await;
                rejoin = Some(Rejoin::new(lost));
                exchange = false;
            }
        }
    }
}

/// Ring, or answer a ring, until the peer's device joins the call.
async fn answer<S: CallSignal>(
    w: &mut Wire<'_, S>,
    user: &str,
    m: &mut Membership,
    events: &Sender<Value>,
    dm: &DmMedia<'_>,
    lease: &mut LeaseWatch,
) -> Result<PeerSession> {
    let DmMedia {
        incoming, video, ..
    } = *dm;
    let dm_peer = w.dm_peer.ok_or("Missing DM recipient.")?;
    // Encryption, sending the invite and waiting for its acknowledgement can
    // take seconds. They are part of the advertised ring lifetime.
    let deadline = Instant::now()
        + incoming.map_or(RING_LIFETIME, |i| {
            Duration::from_millis(i.expires.saturating_sub(now())).min(RING_LIFETIME)
        });
    let notification = if incoming.is_none() {
        let content = json!({
        "slot_id":SLOT_ID,"notification_type":"ring","m.mentions":{"user_ids":[dm_peer]},
        "m.relates_to":{"rel_type":"m.reference","event_id":m.event_id},
        "m.call.intent":if video{"video"}else{"audio"},"sender_ts":now(),"lifetime":RING_LIFETIME.as_millis() as u64,"msc4354_sticky_key":SLOT_ID});
        let n = w.signal.send_event(w.room, RING, content).await?;
        status(events, w.room, "Ringing").await;
        Some(n)
    } else {
        None
    };
    let mut heartbeat = time::interval(SECOND * 4);
    let mut poll = time::interval(SECOND);
    let answer=time::timeout_at(deadline,async {
        loop {
            tokio::select!{
                _=heartbeat.tick()=>if !lease.keep(w,m).await && lease.lost(){return Err("Lost call membership.")},
                _=poll.tick()=>{
                    let state=w.state(false).await?;check_dm(&state,user,dm_peer)?;
                    if let Some(e)=state.iter().find(|e|e["sender"]==dm_peer && active_member(e) && incoming.is_none_or(|i|i.matches_member(e))){
                        let c=&e["content"];let device=c["device_id"].as_str().unwrap();
                        let id=format!("{dm_peer}:{device}");
                        return Ok(PeerSession{device:device.to_owned(),member:c["membershipID"].as_str().unwrap_or(&id).to_owned(),
                            identity:id,created:created(e).ok_or("Unknown peer session.")?});
                    }
                    if incoming.is_some(){return Err("The caller has left.")}
                }
            }
            // Declines arrive alongside the answers to these requests.
            w.absorb();
            if w.declines.iter().any(|id|notification.as_deref()==Some(id.as_str())){return Err("Call declined.")}
        }
    }).await;
    match answer {
        Ok(result) => result,
        Err(_) => {
            // Tell the caller now, independently of server-side leave cleanup.
            status(events, w.room, "No answer.").await;
            Err("No answer.")
        }
    }
}

/// Install every key the peer's device sent; whether there was one.
fn install<S>(w: &mut Wire<'_, S>, p: &KeyProvider, peer: &PeerSession) -> Result<bool> {
    let mut installed = false;
    for (_, key) in w.keys.drain(..) {
        let Some((idx, bytes)) = rtc_keys::receive(&key.content, &peer.device, &peer.member)?
        else {
            continue;
        };
        if !p.set_key(&peer.identity.clone().into(), idx, bytes) {
            return Err("Failed to install sender key.");
        }
        installed = true;
    }
    Ok(installed)
}

/// What [`talk`] runs with.
struct DmSession<'a, 'b> {
    dm: &'a DmMedia<'b>,
    keys: &'a DmKeys,
    exchange: bool,
    /// Connection failures are retried, not the end of the call.
    rejoining: bool,
}

/// One LiveKit session of a DM: from the key exchange until the call ends or
/// the session is lost.
#[allow(clippy::too_many_arguments)]
async fn talk<S: CallSignal>(
    w: &mut Wire<'_, S>,
    s: &Session,
    m: &mut Membership,
    events: &Sender<Value>,
    media: &mut Media,
    session: DmSession<'_, '_>,
    peer: &mut PeerSession,
    lease: &mut LeaseWatch,
) -> Result<Talk> {
    let DmSession {
        dm,
        keys,
        exchange,
        rejoining,
    } = session;
    let dm_peer = w.dm_peer.ok_or("Missing DM recipient.")?;
    let peer_id = peer.identity.clone();
    let my_id = s.identity();
    let unready = |why| {
        if rejoining {
            Ok(Talk::Unready)
        } else {
            Err(why)
        }
    };
    let mut heartbeat = time::interval(SECOND * 4);
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    if exchange {
        let target = [(dm_peer.to_owned(), peer.device.clone())];
        let content = keys.key.content(w.room, &my_id, &s.device);
        if let Err(why) = w.signal.send_to_device(KEY, content, &target).await {
            return unready(why);
        }
        let waited = time::timeout(SECOND * 14, async {
            loop {
                if install(w, &keys.provider, peer)? {
                    return Ok(true);
                }
                tokio::select! {
                    r=w.pump()=>r?,
                    _=heartbeat.tick()=>if !lease.keep(w,m).await && lease.lost() {return Ok(false)},
                }
            }
        })
        .await;
        match waited {
            Ok(Ok(true)) => (),
            Ok(Ok(false)) => return unready("Lost call membership."),
            Ok(Err(why)) => return Err(why),
            Err(_) => return unready("The other device did not provide an encrypted audio key."),
        }
    }
    let muted = dm.mode.borrow().mic_off();
    let (lk, mut rtc, src, publication) =
        match publish_mic(s, &keys.provider, &mut media.live, true, muted).await {
            Ok(link) => link,
            Err(why) => return unready(why),
        };
    // Our new participant replaced the parked one at the SFU.
    close(&mut media.parked).await;
    select_key(lk, keys.key.index)?;
    let mut mode = dm.mode.clone();
    let previews = &dm.previews;
    let notices = DeviceNotices {
        events: events.clone(),
        room: w.room.to_owned(),
    };
    let mut audio = JoinSet::new();
    let devices = mode.clone();
    let io = audio::platform();
    let mixer = Mixer::start(io.clone(), mode.clone(), notices.clone());
    let voice = Voice::new(dm.level.clone());
    mixer.far_end(voice.reference());
    let capture = |publication| {
        controlled_capture(
            io.clone(),
            src.clone(),
            publication,
            devices.clone(),
            notices.clone(),
            voice.clone(),
        )
    };
    let playback = |track| mixer.play(track, Kind::Voice, Some(dm_peer.to_owned()));
    let mut camera = room_video::Publisher::new(VideoInput::Camera);
    let mut share = room_video::Publisher::new(VideoInput::Window);
    let mut sound = share_audio::Publisher::default();
    share.local_preview = Some(previews.clone());
    camera.local_preview = Some(previews.clone());
    let mut media_jobs = JoinSet::new();
    let mut subscriptions = std::collections::BTreeMap::<String, tokio::task::AbortHandle>::new();
    // What the window shows: hidden video pauses, each tile gets its layer.
    let mut viewer = room_video::Viewer::default();
    let mut sight = previews.watch();
    audio.spawn(capture(publication));
    let mut playing = false;
    // Whether the dock says "Connected" for this connection.
    let mut connected = false;
    let mut recovering: Option<Instant> = None;
    let mut rtc_up = true;
    // The peer's membership is gone, but it did not hang up: it may rejoin.
    let mut away: Option<Instant> = None;
    // The peer's LiveKit participant left on its own request.
    let mut hung_up = false;
    // Our key, for a peer that rejoined with a new membership.
    let mut owed = false;
    #[cfg(debug_assertions)]
    let trace_ice = std::env::var("UNTETHERED_QA_ICE_STATS").as_deref() == Ok("1");
    #[cfg(debug_assertions)]
    let trace_video = room_video::trace_video_enabled();
    loop {
        install(w, &keys.provider, peer)?;
        if lease.lost() {
            return Ok(Talk::Lost(Lost::Lease));
        }
        // The call is up once the authenticated peer is in this Room, whether
        // or not it publishes a microphone: it may have joined muted or have
        // none. Their audio plays whenever its track is subscribed.
        if recovering.is_none()
            && !connected
            && lk
                .remote_participants()
                .contains_key(&peer_id.clone().into())
        {
            connected = true;
            status(events, w.room, "Connected").await;
        }
        let (requested, route) = {
            let state = mode.borrow_and_update();
            (
                if recovering.is_some() {
                    [None; 2]
                } else {
                    state.video
                },
                state.camera.clone(),
            )
        };
        camera
            .select(requested[VideoInput::Camera as usize], lk, route)
            .await?;
        share
            .select(
                requested[VideoInput::Window as usize],
                lk,
                CameraChoice::default(),
            )
            .await?;
        sound
            .sync(
                share.live_capture(),
                lk,
                keys.key.index,
                &mode,
                events,
                w.room,
            )
            .await?;
        let mut live = std::collections::BTreeSet::new();
        let shown = sight.borrow_and_update().clone();
        if recovering.is_none()
            && let Some(p) = lk.remote_participants().get(&peer_id.clone().into())
        {
            for pubn in p.track_publications().into_values() {
                if pubn.encryption_type() != EncryptionType::Gcm || pubn.is_muted() {
                    continue;
                }
                let sid = pubn.sid().to_string();
                match (pubn.source(), pubn.track()) {
                    (
                        source @ (TrackSource::Camera | TrackSource::Screenshare),
                        Some(RemoteTrack::Video(track)),
                    ) => {
                        viewer.ask(&pubn, &shown);
                        live.insert(sid.clone());
                        subscriptions.entry(sid).or_insert_with(|| {
                            media_jobs.spawn(room_video::display(
                                track,
                                dm_peer.into(),
                                if source == TrackSource::Camera {
                                    VideoInput::Camera
                                } else {
                                    VideoInput::Window
                                },
                                previews.clone(),
                                events.clone(),
                            ))
                        });
                    }
                    (TrackSource::ScreenshareAudio, Some(RemoteTrack::Audio(track))) => {
                        // A tab may stop or be replaced while the microphone stays live.
                        // Its worker follows the publication SID, not the call recovery path.
                        live.insert(sid.clone());
                        subscriptions.entry(sid).or_insert_with(|| {
                            media_jobs.spawn(mixer.play(
                                track,
                                Kind::Share,
                                Some(dm_peer.to_owned()),
                            ))
                        });
                    }
                    _ => (),
                }
            }
        }
        subscriptions.retain(|sid, job| {
            if live.contains(sid) {
                true
            } else {
                job.abort();
                false
            }
        });
        viewer.retain(|sid| live.contains(sid.as_str()));
        let interrupted = tokio::select! {
            changed=mode.changed()=>{changed.map_err(|_|ENDED)?;false}
            changed=sight.changed()=>{changed.map_err(|_|ENDED)?;false}
            _=viewer.settled()=>{viewer.repeat(lk);false}
            update=camera.next()=>{camera.update(update,lk,keys.key.index,events,w.room,&mode).await?;false}
            update=share.next()=>{share.update(update,lk,keys.key.index,events,w.room,&mode).await?;false}
            ended=sound.ended()=>{sound.finish(ended,lk,events,w.room).await?;false}
            _=media_jobs.join_next(),if !media_jobs.is_empty()=>false,
            e=w.pump()=>{
                e?;
                // A peer that is away comes back with a new membership.
                if away.is_some() && std::mem::take(&mut w.roster_dirty) {heartbeat.reset_immediately();}
                false
            }
            // Notice the end of our lease as it happens, not at the next tick.
            _=time::sleep_until(lease.deadline())=>false,
            _=heartbeat.tick()=>{
                if away.is_none() && recovering.is_some_and(|since| since.elapsed()>=RECOVERY_LIMIT) {
                    return Ok(Talk::Lost(Lost::Media));
                }
                if away.is_some_and(|since| since.elapsed()>=recovery::REJOIN_WINDOW) {
                    return Ok(Talk::Ended("The other person did not reconnect."));
                }
                let restoring=recovering.is_some() && rtc_up && lk.connection_state()==ConnectionState::Connected;
                match check_peer(w,m,lease,&s.user,peer,restoring,hung_up).await? {
                    PeerState::Offline=>true,
                    PeerState::Expired=>return Ok(Talk::Lost(Lost::Lease)),
                    PeerState::Left=>return Ok(Talk::Ended(ENDED)),
                    PeerState::Away=>{away.get_or_insert_with(Instant::now);true}
                    PeerState::Rejoined(next)=>{
                        // Its new LiveKit session starts now, not at the outage.
                        *peer=next;away=None;owed=true;recovering=Some(Instant::now());
                        heartbeat.reset_immediately();
                        true
                    }
                    PeerState::Valid=>{
                        away=None;
                        if owed {
                            // The rejoined device has none of our keys yet.
                            let target=[(dm_peer.to_owned(),peer.device.clone())];
                            let content=keys.key.content(w.room,&my_id,&s.device);
                            let sent=matches!(time::timeout(SECOND*5,w.signal.send_to_device(KEY,content,&target)).await,Ok(Ok(())));
                            owed = !sent;
                        }
                        #[cfg(debug_assertions)]
                        if trace_ice && playing && recovering.is_none() {
                            trace_selected_ice(lk).await;
                        }
                        #[cfg(debug_assertions)]
                        if trace_video && recovering.is_none() {
                            room_video::trace_video_stats(lk).await;
                        }
                        if restoring && let Some((mic,track))=encrypted_tracks(lk,&my_id,&peer_id)? {
                            audio.spawn(capture(mic));
                            if let Some(track)=track {audio.spawn(playback(track));playing=true;}
                            recovering=None;
                        }
                        false
                    }
                }
            }
            job=audio.join_next(),if !audio.is_empty()=>match job {
                Some(Ok(Ok(())))=>true, // stream ended: membership distinguishes a leave from a restart
                Some(Ok(Err(why)))=>return Err(why),
                _=>return Err("Audio device stopped.")
            },
            event=rtc.recv()=>match event {
                Some(RoomEvent::Reconnecting)=>{rtc_up=false;true}
                Some(RoomEvent::Reconnected)=>{
                    // A restart republished our tracks with cryptors at index 0.
                    if select_key(lk,keys.key.index).is_err(){return Ok(Talk::Lost(Lost::Media))}
                    rtc_up=true;heartbeat.reset_immediately();false
                }
                Some(RoomEvent::LocalTrackRepublished{previous_sid,publication,..})=>{
                    camera.rebind(&previous_sid,&publication);share.rebind(&previous_sid,&publication);sound.rebind(&previous_sid,&publication);
                    if select_one(lk,&publication,keys.key.index).is_err(){return Ok(Talk::Lost(Lost::Media))}
                    false
                }
                Some(RoomEvent::TrackSubscribed{track:RemoteTrack::Audio(track),participant,publication,..}) if participant.identity().0==peer_id && publication.source()==TrackSource::Microphone && !playing && recovering.is_none()=>{
                    if publication.encryption_type()!=EncryptionType::Gcm{return Err("Peer did not encrypt audio.")}
                    audio.spawn(playback(track));playing=true;false
                }
                Some(RoomEvent::TrackUnsubscribed{track:RemoteTrack::Audio(_),participant,publication,..}) if participant.identity().0==peer_id && publication.source()==TrackSource::Microphone=>true,
                // A video track subscribed anew starts at the SFU's defaults.
                Some(RoomEvent::TrackSubscribed{track:RemoteTrack::Video(_),publication,..})=>{viewer.forget(&publication.sid());false}
                Some(RoomEvent::ParticipantConnected(p)) if p.identity().0==peer_id=>{hung_up=false;false}
                Some(RoomEvent::ParticipantDisconnected(p)) if p.identity().0==peer_id=>{
                    // A hangup leaves LiveKit on request before the membership goes.
                    hung_up=p.disconnect_reason()==DisconnectReason::ClientInitiated;
                    heartbeat.reset_immediately();
                    true
                }
                None|Some(RoomEvent::Disconnected{..})=>return Ok(Talk::Lost(Lost::Media)),
                _=>false
            }
        };
        if interrupted && recovering.is_none() {
            recovering = Some(Instant::now());
            // Cancel and drain both device tasks before any replacement is
            // started. The microphone's pacat child is kill-on-drop, and the
            // mixer closes its output once no source is left. Keep user routes,
            // mute/deafen, the E2EE provider and the SDK reconnect loop alive.
            audio.shutdown().await;
            playing = false;
            connected = false;
            status(events, w.room, "Reconnecting…").await;
            heartbeat.reset_immediately();
        }
    }
}

enum PeerState {
    Valid,
    Offline,
    /// Our lease is gone: rejoin.
    Expired,
    /// The peer hung up.
    Left,
    /// The peer's membership is gone, but not by a hangup.
    Away,
    /// The peer's device is back with a new membership.
    Rejoined(PeerSession),
}
/// Keep our lease and look at both memberships.
#[allow(clippy::too_many_arguments)]
async fn check_peer<S: CallSignal>(
    w: &mut Wire<'_, S>,
    m: &mut Membership,
    lease: &mut LeaseWatch,
    user: &str,
    peer: &PeerSession,
    restoring: bool,
    hung_up: bool,
) -> Result<PeerState> {
    if !lease.keep(w, m).await {
        return Ok(if lease.lost() {
            PeerState::Expired
        } else {
            PeerState::Offline
        });
    }
    let Ok(Ok(state)) = time::timeout(SECOND * 5, w.state(restoring)).await else {
        return Ok(PeerState::Offline);
    };
    let dm_peer = w.dm_peer.ok_or("Missing DM recipient.")?;
    check_dm(&state, user, dm_peer)?;
    if !recovery::ours(&state, user, &m.event_id) {
        return Ok(PeerState::Expired);
    }
    if state.iter().any(|e| peer.matches(dm_peer, e)) {
        return Ok(PeerState::Valid);
    }
    if hung_up {
        return Ok(PeerState::Left);
    }
    Ok(state
        .iter()
        .find_map(|e| peer.successor(dm_peer, e))
        .map_or(PeerState::Away, PeerState::Rejoined))
}

/// Opt-in acceptance diagnostics. Never log the entire stats report: candidate
/// addresses, TURN URLs, ICE fragments and certificates do not belong in logs.
#[cfg(debug_assertions)]
async fn trace_selected_ice(lk: &Room) {
    use livekit::webrtc::stats::{IceCandidatePairState, RtcStats};
    let Ok(Ok(stats)) = time::timeout(SECOND, lk.get_stats()).await else {
        return;
    };
    for report in [&stats.publisher_stats, &stats.subscriber_stats] {
        for r in report {
            let (direction, stream, packets) = match r {
                RtcStats::InboundRtp(r) => ("inbound", &r.stream, r.received.packets_received),
                RtcStats::OutboundRtp(r) => ("outbound", &r.stream, r.sent.packets_sent),
                _ => continue,
            };
            if stream.kind != "audio" || packets == 0 {
                continue;
            }
            let Some(pair_id) = report.iter().find_map(|v| match v {
                RtcStats::Transport(t) if t.rtc.id == stream.transport_id => {
                    Some(&t.transport.selected_candidate_pair_id)
                }
                _ => None,
            }) else {
                continue;
            };
            let Some(local_id) = report.iter().find_map(|v| match v {
                RtcStats::CandidatePair(p)
                    if &p.rtc.id == pair_id
                        && p.candidate_pair.state == Some(IceCandidatePairState::Succeeded) =>
                {
                    Some(&p.candidate_pair.local_candidate_id)
                }
                _ => None,
            }) else {
                continue;
            };
            if let Some(c) = report.iter().find_map(|v| match v {
                RtcStats::LocalCandidate(c) if &c.rtc.id == local_id => Some(&c.local_candidate),
                _ => None,
            }) && let Some(kind) = c.candidate_type
            {
                let proto = match c.relay_protocol {
                    Some(livekit::webrtc::stats::IceServerTransportProtocol::Udp) => "Udp",
                    Some(livekit::webrtc::stats::IceServerTransportProtocol::Tcp) => "Tcp",
                    Some(livekit::webrtc::stats::IceServerTransportProtocol::Tls) => "Tls",
                    None => "None",
                };
                eprintln!(
                    "RTC selected audio {direction} local={kind:?} relay_protocol={proto} packets={packets}"
                );
            }
        }
    }
}

struct PeerSession {
    device: String,
    member: String,
    identity: String,
    created: u64,
}

async fn publish_mic<'a>(
    s: &Session,
    provider: &KeyProvider,
    media: &'a mut Option<Room>,
    auto_subscribe: bool,
    muted: bool,
) -> Result<(
    &'a Room,
    tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
    NativeAudioSource,
    LocalTrackPublication,
)> {
    let mut options = RoomOptions::default();
    options.auto_subscribe = auto_subscribe;
    // Publishers stop encoding simulcast layers no subscriber asked for.
    // Layers are asked for explicitly (`room_video::Viewer`), not by
    // adaptive stream, which relies on a browser's element sizes.
    options.dynacast = true;
    options.encryption = Some(E2eeOptions {
        encryption_type: EncryptionType::Gcm,
        key_provider: provider.clone(),
    });
    let (lk, rtc) = time::timeout(SECOND * 12, Room::connect(&s.sfu, &s.token, options))
        .await
        .map_err(|_| "SFU connection timed out.")?
        .map_err(|_| "Could not connect to SFU.")?;
    let lk = media.insert(lk);
    if lk.local_participant().identity().0 != format!("{}:{}", s.user, s.device) {
        return Err("SFU assigned an unexpected identity.");
    }
    let src = NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 50);
    let mic =
        LocalAudioTrack::create_audio_track("Microphone", RtcAudioSource::Native(src.clone()));
    // Muted before joining: the SFU learns of the microphone as muted, and
    // nobody gets a frame of it until it's unmuted.
    if muted {
        mic.mute();
    }
    let publication = time::timeout(
        SECOND * 10,
        lk.local_participant().publish_track(
            LocalTrack::Audio(mic),
            TrackPublishOptions {
                source: TrackSource::Microphone,
                ..Default::default()
            },
        ),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .ok_or("Could not publish microphone.")?;
    if publication.encryption_type() != EncryptionType::Gcm {
        return Err("Microphone encryption unavailable.");
    }
    Ok((lk, rtc, src, publication))
}
fn created(event: &Value) -> Option<u64> {
    event["content"]["created_ts"]
        .as_u64()
        .or_else(|| event["origin_server_ts"].as_u64())
}
impl PeerSession {
    // User/device IDs alone could also describe a *new* call after hangup.
    fn matches(&self, user: &str, event: &Value) -> bool {
        let c = &event["content"];
        event["sender"] == user
            && active_member(event)
            && c["device_id"] == self.device
            && c["membershipID"].as_str().unwrap_or(&self.identity) == self.member
            && created(event) == Some(self.created)
    }
    /// The same device's next session in this call: it rejoined after its
    /// lease expired, with a new membership and no new ring.
    fn successor(&self, user: &str, event: &Value) -> Option<Self> {
        let c = &event["content"];
        let created = created(event)?;
        (event["sender"] == user
            && active_member(event)
            && c["device_id"] == self.device
            && created > self.created)
            .then(|| Self {
                device: self.device.clone(),
                member: c["membershipID"]
                    .as_str()
                    .unwrap_or(&self.identity)
                    .to_owned(),
                identity: self.identity.clone(),
                created,
            })
    }
}

#[derive(Clone)]
struct DeviceNotices {
    events: Sender<Value>,
    room: String,
}
impl DeviceNotices {
    async fn report(&self, port: Port, revision: u64, failed: bool) {
        let _ = self
            .events
            .send(json!({"command":"call_device_status","data":{
                "room":self.room,"mic":port==Port::Mic,"revision":revision,"failed":failed
            }}))
            .await;
    }
}

async fn controlled_capture(
    io: Arc<dyn AudioIo>,
    src: NativeAudioSource,
    publication: LocalTrackPublication,
    mut mode: watch::Receiver<AudioMode>,
    notices: DeviceNotices,
    voice: Voice,
) -> Result<()> {
    let settings = mode.clone();
    loop {
        if mode.borrow_and_update().mic_off() {
            voice.closed();
            if !publication.is_muted() {
                publication.mute();
            }
            // No queued audio from the previous hold should survive a closed gate.
            src.clear_buffer();
            mode.wait_for(|s| !s.mic_off()).await.map_err(|_| ENDED)?;
            continue;
        }
        let route = mode.borrow().mic.clone();
        let changed = |s: &AudioMode| s.mic_off() || s.mic.revision != route.revision;
        // Drop only this local input on mute or a mic switch. The LiveKit
        // publication, its source and frame cryptor outlive every pacat child.
        tokio::select! {
            biased;
            change=mode.wait_for(changed)=>{change.map_err(|_| ENDED)?;continue;}
            result=capture(&*io,&src,&publication,&route,&notices,&settings,&voice)=>result?,
        }
        voice.closed();
        if !publication.is_muted() {
            publication.mute();
        }
        notices.report(Port::Mic, route.revision, true).await;
        mode.wait_for(changed).await.map_err(|_| ENDED)?;
    }
}

// A local device failure is recoverable by choosing another input. An error
// submitting PCM to the network source remains a call-level error.
async fn capture(
    io: &dyn AudioIo,
    src: &NativeAudioSource,
    publication: &LocalTrackPublication,
    route: &Route,
    notices: &DeviceNotices,
    settings: &watch::Receiver<AudioMode>,
    voice: &Voice,
) -> Result<()> {
    let Ok(mut input) = io.open_input(route.name.as_deref()) else {
        return Ok(());
    };
    let mut frame = [0; audio::SAMPLES];
    let mut first = true;
    let mut processing = voice.capture();
    while input.read(&mut frame).await {
        if first {
            first = false;
            if publication.is_muted() {
                publication.unmute();
            }
            notices.report(Port::Mic, route.revision, false).await;
        }
        {
            let mode = settings.borrow();
            processing.frame(&mut frame, &mode.voice, mode.talk != TalkGate::Open);
        }
        src.capture_frame(&AudioFrame {
            data: frame.as_slice().into(),
            sample_rate: audio::RATE,
            num_channels: 1,
            samples_per_channel: audio::SAMPLES as u32,
        })
        .await
        .map_err(|_| "Microphone capture failed.")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_and_deafen_before_joining_start_the_call_that_way() {
        let (commands, _backend) = async_channel::bounded(8);
        let mut app = crate::App::new(commands);
        let _ = app.update(crate::Msg::ToggleMic);
        let _ = app.update(crate::Msg::ToggleDeafen);
        // The mic stays muted while deafened, as in a call.
        let _ = app.update(crate::Msg::ToggleMic);
        assert!(app.local_audio.muted && app.local_audio.deafened);
        app.local_audio.mic = Some("QAMicB.monitor".into());
        app.start_voice("!room".into(), Target::Channel);
        let mode = app.call.as_ref().unwrap().audio.borrow().clone();
        assert!(mode.muted && mode.deafened && mode.mic_off());
        assert_eq!(mode.mic.name.as_deref(), Some("QAMicB.monitor"));

        // In the call: undeafening keeps the manual mute, and both stay
        // set after it ends.
        let _ = app.update(crate::Msg::ToggleDeafen);
        let mode = app.call.as_ref().unwrap().audio.borrow().clone();
        assert!(mode.muted && !mode.deafened);
        let _ = app.update(crate::Msg::EndCall);
        app.call = None;
        assert!(app.local_audio.muted && !app.local_audio.deafened);
        app.start_voice("!room".into(), Target::Channel);
        assert!(app.call.as_ref().unwrap().audio.borrow().muted);
    }

    #[test]
    fn a_chosen_device_applies_now_and_to_the_next_call() {
        let (commands, _backend) = async_channel::bounded(8);
        let mut app = crate::App::new(commands);
        let cobalt = crate::call_devices::Choice::for_test("QASpeakerB", "Grove");
        app.local_audio.devices.speakers.push(cobalt.clone());
        app.start_voice("!room".into(), Target::Channel);
        let _ = app.update(crate::Msg::SelectDevice(Port::Speaker, cobalt));
        let speaker = app.call.as_ref().unwrap().audio.borrow().speaker.clone();
        assert_eq!(speaker.name.as_deref(), Some("QASpeakerB"));
        assert_eq!(speaker.revision, 1);
        assert_eq!(app.local_audio.speaker.as_deref(), Some("QASpeakerB"));
        // A device no list offers is refused.
        let gone = crate::call_devices::Choice::for_test("Gone", "Gone");
        let _ = app.update(crate::Msg::SelectDevice(Port::Speaker, gone));
        assert_eq!(app.local_audio.speaker.as_deref(), Some("QASpeakerB"));
    }

    #[test]
    fn call_stage_opens_from_preview_and_collapses_when_browsing() {
        let (commands, _backend) = async_channel::bounded(8);
        let mut app = crate::App::new(commands);
        app.logged_in = true;
        app.verified = true;
        for (id, name) in [("!call", "Call"), ("!other", "Other")] {
            app.live.list.insert(
                id.into(),
                crate::rooms::Room {
                    name: name.into(),
                    has_metadata: true,
                    ..Default::default()
                },
            );
        }
        app.open("!call".into());
        app.start_voice("!call".into(), Target::Channel);
        let call = app.call.as_ref().unwrap();
        assert!(!call.stage_open);
        let id = call.id;

        let _ = app.update(crate::Msg::ReturnToCall(id));
        assert!(app.call.as_ref().unwrap().stage_open);

        let _ = app.update(crate::Msg::CallLayout(id, true));
        assert!(!app.call.as_ref().unwrap().stage_open);
        let _ = app.update(crate::Msg::ReturnToCall(id));
        let _ = app.update(crate::Msg::Open("!other".into()));
        let _ = app.update(crate::Msg::Open("!call".into()));
        assert!(!app.call.as_ref().unwrap().stage_open);
    }

    #[test]
    fn clicking_a_voice_channel_joins_it_and_moves_over_from_another_call() {
        let (commands, _backend) = async_channel::bounded(8);
        let mut app = crate::App::new(commands);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        for (id, voice) in [("!lounge", true), ("!stage", true), ("!chat", false)] {
            app.live.list.insert(
                id.into(),
                crate::rooms::Room {
                    name: id.into(),
                    has_metadata: true,
                    voice,
                    encrypted: true,
                    ..Default::default()
                },
            );
        }
        let in_call = |app: &crate::App| app.call.as_ref().map(|c| c.room.clone());
        let _ = app.update(crate::Msg::OpenVoice("!lounge".into()));
        assert_eq!(app.selected, "!lounge");
        assert_eq!(in_call(&app).as_deref(), Some("!lounge"));
        // Its own channel again: the call opens.
        let _ = app.update(crate::Msg::Open("!chat".into()));
        let _ = app.update(crate::Msg::OpenVoice("!lounge".into()));
        assert!(app.call.as_ref().unwrap().stage_open);
        // Another channel: this call ends first, then that one starts.
        let _ = app.update(crate::Msg::OpenVoice("!stage".into()));
        assert_eq!(app.selected, "!stage");
        assert!(app.call.as_ref().unwrap().cancel.is_closed());
        assert_eq!(app.join_after.as_deref(), Some("!stage"));
        app.frame(json!({"command":"desktop_call","data":{"room":"!lounge","label":"Call ended","ended":true}}));
        assert_eq!(in_call(&app).as_deref(), Some("!stage"));
        assert_eq!(app.join_after, None);
        // Hanging up while one is pending joins nothing afterwards.
        let _ = app.update(crate::Msg::OpenVoice("!lounge".into()));
        let _ = app.update(crate::Msg::EndCall);
        app.frame(json!({"command":"desktop_call","data":{"room":"!stage","label":"Call ended","ended":true}}));
        assert_eq!(in_call(&app), None);
    }

    #[test]
    fn recovery_accepts_lease_renewal_but_not_a_new_call_from_the_same_device() {
        let stamp = now();
        let peer = PeerSession {
            device: "D".into(),
            member: "@b:chat:D".into(),
            identity: "@b:chat:D".into(),
            created: stamp,
        };
        let mut e = json!({"event_id":"$original","type":EVENT_TYPE,"sender":"@b:chat","origin_server_ts":stamp,
            "content":{"application":"m.call","scope":"m.room","call_id":"","device_id":"D","expires":60000}});
        assert!(peer.matches("@b:chat", &e));
        e["event_id"] = "$renewed".into();
        e["origin_server_ts"] = (stamp + 1).into();
        e["content"]["created_ts"] = stamp.into();
        assert!(peer.matches("@b:chat", &e));
        e["content"]["created_ts"] = (stamp + 1).into();
        assert!(!peer.matches("@b:chat", &e));
        e["content"] = json!({});
        assert!(!peer.matches("@b:chat", &e));
    }

    #[test]
    fn a_rejoin_is_the_same_device_with_a_newer_membership() {
        let stamp = now();
        let peer = PeerSession {
            device: "D".into(),
            member: "@b:chat:D".into(),
            identity: "@b:chat:D".into(),
            created: stamp,
        };
        let member = |device: &str, created: u64| {
            json!({"event_id":"$next","type":EVENT_TYPE,"sender":"@b:chat","origin_server_ts":created,
                "content":{"application":"m.call","scope":"m.room","call_id":"","device_id":device,
                    "created_ts":created,"expires":60000}})
        };
        let next = peer
            .successor("@b:chat", &member("D", stamp + 20_000))
            .unwrap();
        assert_eq!(
            (next.created, next.identity.as_str()),
            (stamp + 20_000, "@b:chat:D")
        );
        // The original session itself, or an older one, is not a rejoin.
        assert!(peer.successor("@b:chat", &member("D", stamp)).is_none());
        assert!(peer.successor("@b:chat", &member("D", stamp - 1)).is_none());
        // Another device, or another sender, is a different call.
        assert!(peer.successor("@b:chat", &member("E", stamp + 1)).is_none());
        assert!(peer.successor("@c:chat", &member("D", stamp + 1)).is_none());
        let mut left = member("D", stamp + 1);
        left["content"] = json!({});
        assert!(peer.successor("@b:chat", &left).is_none());
    }

    #[test]
    fn stale_direct_chat_marker_cannot_ring_a_group_or_clear_room() {
        let member = |id: &str| json!({"type":"m.room.member","state_key":id,"content":{"membership":"join"}});
        let mut state = vec![member("@a:chat"), member("@b:chat")];
        assert!(check_dm(&state, "@a:chat", "@b:chat").is_err());
        state.push(
            json!({"type":"m.room.encryption","content":{"algorithm":"m.megolm.v1.aes-sha2"}}),
        );
        assert!(check_dm(&state, "@a:chat", "@b:chat").is_ok());
        assert!(check_dm(&state, "@a:chat", "@c:chat").is_err());
        state.push(member("@c:chat"));
        assert!(check_dm(&state, "@a:chat", "@b:chat").is_err());
    }
}
