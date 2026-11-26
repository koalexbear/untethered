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
mod room_video;
mod share_audio;
pub(crate) use cameras::{CameraChoice, list_cameras};
pub(super) use room_video::{Feed, Previews, VideoInput};
use audio::{AudioIo, Kind, Mixer};

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
        if self.talk == TalkGate::Open {
            return false;
        }
        let next = if down && !self.muted && !self.deafened {
            TalkGate::Held
        } else {
            TalkGate::Waiting
        };
        if next == self.talk {
            return false;
        }
        self.talk = next;
        true
    }
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
    pub devices: Catalog,
    pub cameras: Vec<CameraChoice>,
    pub camera_listing: bool,
    pub camera_note: Option<&'static str>,
    pub picker: bool,
    pub mixer: bool,
    pub loading: bool,
    pub list_error: Option<&'static str>,
    pub mic_error: bool,
    pub speaker_error: bool,
    pub video_status: [room_video::Status; 2],
    pub previews: Previews,
    pub preview_hidden: bool,
    pub app_note: String,
    pub stage_open: bool,
    pub theater: bool,
    pub speaking: watch::Receiver<std::collections::BTreeSet<String>>,
}
struct GroupViews {
    previews: Previews,
    speakers: watch::Sender<std::collections::BTreeSet<String>>,
}
pub struct Controls {
    cancel: Receiver<()>,
    audio: watch::Receiver<AudioMode>,
    views: GroupViews,
}
impl crate::App {
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
            talk: TalkGate::from_ptt(group && self.room_ptt),
            ..AudioMode::default()
        });
        let previews = Previews::default();
        let (speakers, speaking) = watch::channel(Default::default());
        let controls = Controls {
            cancel: rx,
            audio: mode,
            views: GroupViews {
                previews: previews.clone(),
                speakers,
            },
        };
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
                devices: Catalog::default(),
                cameras: vec![],
                camera_listing: false,
                camera_note: None,
                picker: false,
                mixer: false,
                loading: false,
                list_error: None,
                mic_error: false,
                speaker_error: false,
                video_status: Default::default(),
                previews,
                preview_hidden: false,
                app_note: String::new(),
                stage_open: false,
                theater: false,
                speaking,
            });
        } else {
            self.status = "Could not queue the voice call.".into();
        }
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
    let openid = w.signal.openid_token().await?;
    let auth: Value = http
        .post(format!("{}/sfu/get", focus.as_str().trim_end_matches('/')))
        .json(&json!({"room":w.room,"device_id":device,"openid_token":openid}))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| "Call server authorization failed.")?
        .json()
        .await
        .map_err(|_| "Invalid call server response.")?;
    let sfu = Url::parse(auth["url"].as_str().unwrap_or("")).map_err(|_| "Invalid SFU URL.")?;
    if !allowed(&sfu, "wss", "ws", local) {
        return Err("The SFU requires a secure connection.");
    }
    let token = auth["jwt"]
        .as_str()
        .ok_or("Missing SFU authorization.")?
        .to_owned();
    if incoming.is_none() {
        w.open_slot(&state).await?;
    }
    Ok(Session {
        user: user.into(),
        device: device.into(),
        focus,
        sfu: sfu.into(),
        token,
    })
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
    let s =
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
    let mut media = None;
    let session = async {
        match dm_peer {
            Some(_) => {
                call(
                    &mut w,
                    &s,
                    &mut member,
                    events,
                    &mut media,
                    DmMedia {
                        incoming,
                        mode: audio,
                        video,
                        previews: views.previews,
                    },
                )
                .await
            }
            None => {
                channel::joined(&mut w, &s, &mut member, events, &mut media, audio, views).await
            }
        }
    };
    let result = tokio::select! {biased;_=cancel.recv()=>Ok(ENDED),r=session=>r};
    if let Some(lk) = media {
        let _ = time::timeout(SECOND * 4, lk.close()).await;
    }
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
}
async fn call<S: CallSignal>(
    w: &mut Wire<'_, S>,
    s: &Session,
    m: &mut Membership,
    events: &Sender<Value>,
    media: &mut Option<Room>,
    dm: DmMedia<'_>,
) -> Result<&'static str> {
    let DmMedia {
        incoming,
        mut mode,
        video,
        previews,
    } = dm;
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
    let lease_started = Instant::now();
    let mut heartbeat = time::interval(SECOND * 4);
    let mut poll = time::interval(SECOND);
    let answer=time::timeout_at(deadline,async {
        loop {
            tokio::select!{
                _=heartbeat.tick()=>m.heartbeat(&mut w.signal).await.map_err(|_|"Lost call membership.")?,
                _=poll.tick()=>{
                    let state=w.state(false).await?;check_dm(&state,&s.user,dm_peer)?;
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
    let answer = match answer {
        Ok(result) => result?,
        Err(_) => {
            // Tell the caller now, independently of server-side leave cleanup.
            status(events, w.room, "No answer.").await;
            return Err("No answer.");
        }
    };
    status(events, w.room, "Connecting").await;
    let peer = &answer;
    let peer_device = peer.device.as_str();
    let peer_id = peer.identity.as_str();
    let provider = rtc_keys::provider();
    let my_id = format!("{}:{}", s.user, s.device);
    let key = SenderKey::fresh(0)?;
    w.signal
        .send_to_device(
            KEY,
            key.content(w.room, &my_id, &s.device),
            &[(dm_peer.to_owned(), peer_device.to_owned())],
        )
        .await?;
    fn install<S>(
        w: &mut Wire<'_, S>,
        p: &KeyProvider,
        device: &str,
        member: &str,
        id: &str,
    ) -> Result<bool> {
        let mut installed = false;
        for (_, key) in w.keys.drain(..) {
            let Some((idx, bytes)) = rtc_keys::receive(&key.content, device, member)? else {
                continue;
            };
            if !p.set_key(&id.to_owned().into(), idx, bytes) {
                return Err("Failed to install sender key.");
            }
            installed = true;
        }
        Ok(installed)
    }
    time::timeout(SECOND*14,async {
        while !install(w,&provider,peer_device,&peer.member,peer_id)? {
            tokio::select!{r=w.pump()=>{r?;},_=heartbeat.tick()=>m.heartbeat(&mut w.signal).await.map_err(|_|"Lost call membership.")?}
        }
        Ok::<(),&'static str>(())
    }).await.map_err(|_|"The other device did not provide an encrypted audio key.")??;
    if !provider.set_key(&my_id.clone().into(), 0, key.bytes.to_vec()) {
        return Err("Failed to install local key.");
    }
    let (lk, mut rtc, src, publication) = publish_mic(s, &provider, media, true).await?;
    let notices = DeviceNotices {
        events: events.clone(),
        room: w.room.to_owned(),
    };
    let mut audio = JoinSet::new();
    let devices = mode.clone();
    let io = audio::platform();
    let mixer = Mixer::start(io.clone(), mode.clone(), notices.clone());
    let capture = |publication| {
        controlled_capture(
            io.clone(),
            src.clone(),
            publication,
            devices.clone(),
            notices.clone(),
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
    audio.spawn(capture(publication));
    let mut playing = false;
    let mut recovering: Option<Instant> = None;
    let mut rtc_up = true;
    let mut lease = LeaseWatch {
        confirmed: Instant::now(),
        renewed: lease_started,
    };
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    #[cfg(debug_assertions)]
    let trace_ice = std::env::var("UNTETHERED_QA_ICE_STATS").as_deref() == Ok("1");
    #[cfg(debug_assertions)]
    let trace_video = room_video::trace_video_enabled();
    loop {
        install(w, &provider, peer_device, &peer.member, peer_id)?;
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
            .sync(share.live_capture(), lk, key.index, &mode, events, w.room)
            .await?;
        let mut live = std::collections::BTreeSet::new();
        if recovering.is_none()
            && let Some(p) = lk.remote_participants().get(&peer_id.to_owned().into())
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
        let interrupted = tokio::select! {
            changed=mode.changed()=>{changed.map_err(|_|ENDED)?;false}
            update=camera.next()=>{camera.update(update,lk,key.index,events,w.room,&mode).await?;false}
            update=share.next()=>{share.update(update,lk,key.index,events,w.room,&mode).await?;false}
            ended=sound.ended()=>{sound.finish(ended,lk,events,w.room).await?;false}
            _=media_jobs.join_next(),if !media_jobs.is_empty()=>false,
            e=w.pump()=>{e?;false}
            _=heartbeat.tick()=>{
                if recovering.is_some_and(|since| since.elapsed()>=RECOVERY_LIMIT) {
                    return Err("Could not reconnect to the call.");
                }
                let restoring=recovering.is_some() && rtc_up && lk.connection_state()==ConnectionState::Connected;
                match lease.check(w,m,&s.user,peer,restoring).await? {
                    LeaseState::Offline=>true,
                    LeaseState::PeerLeft=>return Ok(ENDED),
                    LeaseState::Valid=>{
                        #[cfg(debug_assertions)]
                        if trace_ice && playing && recovering.is_none() {
                            trace_selected_ice(lk).await;
                        }
                        #[cfg(debug_assertions)]
                        if trace_video && recovering.is_none() {
                            room_video::trace_video_stats(lk).await;
                        }
                        if restoring && let Some((mic,track))=encrypted_tracks(lk,&my_id,peer_id)? {
                            audio.spawn(capture(mic));audio.spawn(playback(track));
                            playing=true;recovering=None;status(events,w.room,"Connected").await;
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
                Some(RoomEvent::Reconnected)=>{rtc_up=true;heartbeat.reset_immediately();false}
                Some(RoomEvent::TrackSubscribed{track:RemoteTrack::Audio(track),participant,publication,..}) if participant.identity().0==peer_id && publication.source()==TrackSource::Microphone && !playing && recovering.is_none()=>{
                    if publication.encryption_type()!=EncryptionType::Gcm{return Err("Peer did not encrypt audio.")}
                    audio.spawn(playback(track));playing=true;status(events,w.room,"Connected").await;false
                }
                Some(RoomEvent::TrackUnsubscribed{track:RemoteTrack::Audio(_),participant,publication,..}) if participant.identity().0==peer_id && publication.source()==TrackSource::Microphone=>true,
                Some(RoomEvent::ParticipantDisconnected(p)) if p.identity().0==peer_id=>true,
                None|Some(RoomEvent::Disconnected{..})=>return Err("Call connection lost."),
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
            status(events, w.room, "Reconnecting…").await;
            heartbeat.reset_immediately();
        }
    }
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
) -> Result<(
    &'a Room,
    tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
    NativeAudioSource,
    LocalTrackPublication,
)> {
    let mut options = RoomOptions::default();
    options.auto_subscribe = auto_subscribe;
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
    let publication = lk
        .local_participant()
        .publish_track(
            LocalTrack::Audio(mic),
            TrackPublishOptions {
                source: TrackSource::Microphone,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| "Could not publish microphone.")?;
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
}

struct LeaseWatch {
    confirmed: Instant,
    renewed: Instant,
}
enum LeaseState {
    Valid,
    PeerLeft,
    Offline,
}
impl LeaseWatch {
    async fn check<S: CallSignal>(
        &mut self,
        w: &mut Wire<'_, S>,
        m: &mut Membership,
        user: &str,
        peer: &PeerSession,
        restoring: bool,
    ) -> Result<LeaseState> {
        if self.confirmed.elapsed() >= LEASE {
            return Err("Call membership expired while offline.");
        }
        let started = Instant::now();
        // Never replace an expired membership to recover a call. Retry the
        // existing delayed leave first. Late RPC replies have unique request IDs.
        if !matches!(
            time::timeout(SECOND * 2, m.heartbeat(&mut w.signal)).await,
            Ok(Ok(()))
        ) {
            return Ok(LeaseState::Offline);
        }
        self.confirmed = started;
        let Ok(state) = w.state(restoring).await else {
            return Ok(LeaseState::Offline);
        };
        let dm_peer = w.dm_peer.ok_or("Missing DM recipient.")?;
        check_dm(&state, user, dm_peer)?;
        if !state.iter().any(|e| peer.matches(dm_peer, e)) {
            return Ok(LeaseState::PeerLeft);
        }
        if restoring
            && !state
                .iter()
                .any(|e| e["sender"] == user && e["event_id"] == m.event_id && active_member(e))
        {
            return Err("Our call membership is no longer active.");
        }
        if self.renewed.elapsed() > SECOND * 36 {
            m.renew(&mut w.signal)
                .await
                .map_err(|_| "Failed to renew call membership.")?;
            self.renewed = started;
        }
        Ok(LeaseState::Valid)
    }
}

/// A resume may preserve tracks; a full SDK restart may assign new SIDs.
/// Only rebind the authenticated peer's current encrypted audio in this Room.
fn encrypted_tracks(
    lk: &Room,
    me: &str,
    peer: &str,
) -> Result<Option<(LocalTrackPublication, RemoteAudioTrack)>> {
    let local = lk.local_participant();
    if local.identity().0 != me || !lk.e2ee_manager().enabled() {
        return Err("Could not restore encrypted call tracks.");
    }
    let ours = local
        .track_publications()
        .into_values()
        .find(|p| p.source() == TrackSource::Microphone && p.track().is_some());
    let theirs = lk
        .remote_participants()
        .get(&peer.to_owned().into())
        .and_then(|p| {
            p.track_publications()
                .into_values()
                .find(|p| p.source() == TrackSource::Microphone && p.is_subscribed())
        });
    let (Some(mic), Some(pubn)) = (ours, theirs) else {
        return Ok(None);
    };
    if mic.encryption_type() != EncryptionType::Gcm || pubn.encryption_type() != EncryptionType::Gcm
    {
        return Err("Could not restore encrypted call tracks.");
    }
    Ok(match pubn.track() {
        Some(RemoteTrack::Audio(track)) => Some((mic, track)),
        _ => None,
    })
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
) -> Result<()> {
    loop {
        if mode.borrow_and_update().mic_off() {
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
            result=capture(&*io,&src,&publication,&route,&notices)=>result?,
        }
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
) -> Result<()> {
    let Ok(mut input) = io.open_input(route.name.as_deref()) else {
        return Ok(());
    };
    let mut frame = [0; audio::SAMPLES];
    let mut first = true;
    while input.read(&mut frame).await {
        if first {
            first = false;
            if publication.is_muted() {
                publication.unmute();
            }
            notices.report(Port::Mic, route.revision, false).await;
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
